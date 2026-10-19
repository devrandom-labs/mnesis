//! Verified native TLS, authentication and runtime permissions.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "native security assertions"
)]

#[path = "support/cluster.rs"]
mod cluster;

use std::io::ErrorKind;
use std::net::TcpListener;
#[cfg(all(feature = "projection", feature = "snapshot"))]
use std::num::{NonZeroU32, NonZeroU64};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use futures::TryStreamExt;
use mnesis::{ErrorId, Version};
use mnesis_postgres::{PostgresError, PostgresStore};
#[cfg(all(feature = "projection", feature = "snapshot"))]
use mnesis_store::checkpoint::{
    CheckpointHydrated, CheckpointMode, CheckpointStore, CheckpointWrite,
};
#[cfg(all(feature = "projection", feature = "snapshot"))]
use mnesis_store::state::{Hydrated, SnapshotStore};
use mnesis_store::{AppendError, PendingBatch, RawEventStore, StreamKey, pending_envelope};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};
use sqlx::{AssertSqlSafe, ConnectOptions, PgPool};
use tokio::sync::Barrier;
use tokio::time::timeout;

use cluster::Cluster;

const PASSWORD: &str = "pg8-test-password-must-stay-redacted";
const WRONG_PASSWORD: &str = "pg8-wrong-password-must-stay-redacted";

struct Secure {
    cluster: Cluster,
    port: u16,
}

impl Secure {
    fn authority(cluster: &Cluster, name: &str) {
        let subject = format!("/CN=Mnesis {name} test CA");
        let key = format!("{name}.key");
        let certificate = format!("{name}.crt");
        openssl(
            cluster,
            &[
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-sha256",
                "-days",
                "1",
                "-subj",
                &subject,
                "-addext",
                "basicConstraints=critical,CA:TRUE",
                "-addext",
                "keyUsage=critical,keyCertSign,cRLSign",
                "-keyout",
                &key,
                "-out",
                &certificate,
            ],
        );
    }

    fn certificates(cluster: &Cluster) {
        Self::authority(cluster, "ca");
        openssl(
            cluster,
            &[
                "req",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-sha256",
                "-subj",
                "/CN=localhost",
                "-addext",
                "subjectAltName=DNS:localhost",
                "-keyout",
                "server.key",
                "-out",
                "server.csr",
            ],
        );
        openssl(
            cluster,
            &[
                "x509",
                "-req",
                "-in",
                "server.csr",
                "-CA",
                "ca.crt",
                "-CAkey",
                "ca.key",
                "-CAcreateserial",
                "-days",
                "1",
                "-copy_extensions",
                "copy",
                "-out",
                "server.crt",
            ],
        );
        openssl(
            cluster,
            &[
                "verify",
                "-CAfile",
                "ca.crt",
                "-verify_hostname",
                "localhost",
                "server.crt",
            ],
        );
    }

    fn start() -> Self {
        let cluster = Cluster::initialize();
        Self::certificates(&cluster);
        std::fs::write(cluster.data().join("pg_hba.conf"),
            "local all all trust\nhostssl all all 127.0.0.1/32 scram-sha-256\nhostnossl all all 127.0.0.1/32 reject\n"
        ).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        cluster.launch(&format!(
            "-h 127.0.0.1 -k {} -p {port} -c ssl=on -c ssl_cert_file=server.crt -c ssl_key_file=server.key",
            cluster.socket().display()
        ));
        Self { cluster, port }
    }

    fn connection(&self) -> PgConnectOptions {
        PgConnectOptions::new_without_pgpass()
            .host("localhost")
            .port(self.port)
            .database("postgres")
            .username("pg8_runtime")
            .password(PASSWORD)
            .ssl_mode(PgSslMode::VerifyFull)
    }

    fn options(&self) -> PgConnectOptions {
        self.connection()
            .ssl_root_cert(self.cluster.data().join("ca.crt"))
    }

    fn address(&self, options: &PgConnectOptions, certificate: &str) -> String {
        let mut url = options.to_url_lossy();
        url.query_pairs_mut().append_pair(
            "sslrootcert",
            self.cluster.data().join(certificate).to_str().unwrap(),
        );
        url.to_string()
    }

    async fn open(&self) -> (PostgresStore, PgPool) {
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect_with(self.options())
            .await
            .unwrap();
        let store = PostgresStore::from_pool(pool.clone()).await.unwrap();
        (store, pool)
    }

    async fn prepare(&self) -> PgPool {
        let options = PgConnectOptions::new_without_pgpass()
            .host(self.cluster.socket().to_str().unwrap())
            .port(self.port)
            .database("postgres")
            .username("mnesis")
            .ssl_mode(PgSslMode::Disable);
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await
            .unwrap();
        let enabled: String = sqlx::query_scalar("SHOW ssl")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(enabled, "on");
        PostgresStore::migrate(&pool).await.unwrap();
        // PostgreSQL utility statements cannot bind; only our fixed test constant is interpolated.
        sqlx::raw_sql(AssertSqlSafe(format!(
            "CREATE ROLE pg8_runtime LOGIN PASSWORD '{PASSWORD}'"
        )))
        .execute(&pool)
        .await
        .unwrap();
        sqlx::raw_sql("GRANT USAGE ON SCHEMA mnesis TO pg8_runtime; GRANT SELECT ON ALL TABLES IN SCHEMA mnesis TO pg8_runtime; GRANT INSERT ON mnesis.keys, mnesis.events TO pg8_runtime; GRANT INSERT, UPDATE ON mnesis.snapshots, mnesis.checkpoints TO pg8_runtime; GRANT USAGE ON ALL SEQUENCES IN SCHEMA mnesis TO pg8_runtime")
            .execute(&pool).await.unwrap();
        let native = Command::new("psql")
            .args([
                "-X",
                "-v",
                "ON_ERROR_STOP=1",
                "-Atc",
                "SELECT ssl FROM pg_catalog.pg_stat_ssl WHERE pid = pg_catalog.pg_backend_pid()",
            ])
            .env("PGHOST", "localhost")
            .env("PGPORT", self.port.to_string())
            .env("PGDATABASE", "postgres")
            .env("PGUSER", "pg8_runtime")
            .env("PGPASSWORD", PASSWORD)
            .env("PGSSLMODE", "verify-full")
            .env("PGSSLROOTCERT", self.cluster.data().join("ca.crt"))
            .output()
            .unwrap();
        assert!(native.status.success(), "{native:?}");
        assert_eq!(String::from_utf8(native.stdout).unwrap(), "t\n");
        pool
    }
}

fn openssl(cluster: &Cluster, args: &[&str]) {
    let result = Command::new("openssl")
        .current_dir(cluster.data())
        .args(args)
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
}

#[tokio::test]
async fn verified_tls_runtime_can_open_append_read_close_and_reopen() {
    if std::env::var_os("DATABASE_URL").is_none() {
        return;
    }
    let server = Secure::start();
    let admin = server.prepare().await;
    let (store, pool) = server.open().await;
    let encrypted: bool = sqlx::query_scalar(
        "SELECT ssl FROM pg_catalog.pg_stat_ssl WHERE pid = pg_catalog.pg_backend_pid()",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(encrypted);
    let event = pending_envelope(Version::INITIAL)
        .event_type("Secured")
        .payload(b"verified TLS event".as_slice())
        .build()
        .unwrap();
    let position = store
        .append(
            &StreamKey::from_bytes("secured"),
            None,
            PendingBatch::of(&event),
        )
        .await
        .unwrap();
    let rows: Vec<_> = store
        .read_all(None)
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows.first().unwrap().0, position);
    assert_eq!(rows.first().unwrap().1, StreamKey::from_bytes("secured"));
    assert_eq!(rows.first().unwrap().2.payload(), b"verified TLS event");
    drop(store);
    assert_eq!(
        sqlx::query_scalar::<_, i32>("SELECT 1")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    pool.close().await;
    let (reopened, reopened_pool) = server.open().await;
    let recovered: Vec<_> = reopened
        .read_all(None)
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(recovered.first().unwrap().0, position);
    assert_eq!(
        recovered.first().unwrap().2.payload(),
        b"verified TLS event"
    );
    drop(reopened);
    reopened_pool.close().await;
    admin.close().await;
}

fn redacted(error: &PostgresError) {
    let text = format!("{error}\n{error:?}");
    for secret in [PASSWORD, WRONG_PASSWORD] {
        assert!(!text.contains(secret), "{text}");
    }
}

#[tokio::test]
async fn verified_tls_rejects_a_wrong_authority_and_hostname_without_losing_the_cause() {
    if std::env::var_os("DATABASE_URL").is_none() {
        return;
    }
    let server = Secure::start();
    let admin = server.prepare().await;
    Secure::authority(&server.cluster, "other");
    let healthy = PostgresStore::connect(&server.address(&server.connection(), "ca.crt"))
        .await
        .unwrap();
    drop(healthy);
    let wrong_authority = server.address(&server.connection(), "other.crt");
    let authority_error = timeout(
        Duration::from_secs(5),
        PostgresStore::connect(&wrong_authority),
    )
    .await
    .unwrap()
    .err()
    .expect("wrong CA must reject");
    redacted(&authority_error);
    assert!(
        matches!(&authority_error, PostgresError::Sqlx(sqlx::Error::Io(cause))
        if cause.kind() == ErrorKind::InvalidData && cause.get_ref().map(ToString::to_string).as_deref() == Some("invalid peer certificate: UnknownIssuer")),
        "{authority_error:?}"
    );
    let wrong_hostname = server.address(&server.connection().host("127.0.0.1"), "ca.crt");
    let hostname_error = timeout(
        Duration::from_secs(5),
        PostgresStore::connect(&wrong_hostname),
    )
    .await
    .unwrap()
    .err()
    .expect("certificate without IP SAN must reject");
    redacted(&hostname_error);
    assert!(
        matches!(&hostname_error, PostgresError::Sqlx(sqlx::Error::Io(cause))
        if cause.kind() == ErrorKind::InvalidData && cause.get_ref().map(ToString::to_string).as_deref() == Some("invalid peer certificate: certificate not valid for name \"127.0.0.1\"; certificate is only valid for DnsName(\"localhost\")")),
        "{hostname_error:?}"
    );
    admin.close().await;
}

#[tokio::test]
async fn verified_tls_preserves_authentication_errors_and_rejects_plaintext() {
    if std::env::var_os("DATABASE_URL").is_none() {
        return;
    }
    let server = Secure::start();
    let admin = server.prepare().await;
    let wrong = server.address(&server.connection().password(WRONG_PASSWORD), "ca.crt");
    let error = timeout(Duration::from_secs(5), PostgresStore::connect(&wrong))
        .await
        .unwrap()
        .err()
        .expect("wrong password must reject");
    redacted(&error);
    assert!(
        matches!(&error, PostgresError::Sqlx(sqlx::Error::Database(cause))
        if cause.code().as_deref() == Some("28P01")
        && cause.message() == "password authentication failed for user \"pg8_runtime\""),
        "{error:?}"
    );
    let plaintext = server.address(&server.connection().ssl_mode(PgSslMode::Disable), "ca.crt");
    let rejected = timeout(Duration::from_secs(5), PostgresStore::connect(&plaintext))
        .await
        .unwrap()
        .err()
        .expect("hostnossl reject must refuse plaintext");
    redacted(&rejected);
    assert!(
        matches!(&rejected, PostgresError::Sqlx(sqlx::Error::Database(cause))
        if cause.code().as_deref() == Some("28000")
        && cause.message() == "pg_hba.conf rejects connection for host \"127.0.0.1\", user \"pg8_runtime\", database \"postgres\", no encryption"),
        "{rejected:?}"
    );
    admin.close().await;
}

#[tokio::test]
async fn verified_tls_runtime_competes_atomically_and_cannot_rewrite_events_or_schema() {
    if std::env::var_os("DATABASE_URL").is_none() {
        return;
    }
    let server = Secure::start();
    let admin = server.prepare().await;
    let (store, pool) = server.open().await;
    let barrier = Arc::new(Barrier::new(3));
    let [first, second] = [store.clone(), store.clone()].map(|worker| {
        let start = Arc::clone(&barrier);
        tokio::spawn(async move {
            let event = pending_envelope(Version::INITIAL)
                .event_type("Contended")
                .payload(b"committed over TLS".as_slice())
                .build()
                .unwrap();
            start.wait().await;
            worker
                .append(
                    &StreamKey::from_bytes("contended"),
                    None,
                    PendingBatch::of(&event),
                )
                .await
        })
    });
    barrier.wait().await;
    let results = [first.await.unwrap(), second.await.unwrap()];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    let errors: Vec<_> = results.into_iter().filter_map(Result::err).collect();
    assert!(
        matches!(
        errors.as_slice(),
        [AppendError::Conflict {
            stream_id,
            expected: None,
            actual,
        }] if stream_id == &ErrorId::from_display(&StreamKey::from_bytes("contended"))
            && matches!(actual, None | Some(Version::INITIAL))
        ),
        "{errors:?}"
    );
    for statement in [
        "UPDATE mnesis.events SET payload = 'forbidden'::bytea",
        "DELETE FROM mnesis.events",
        "TRUNCATE mnesis.events",
        "ALTER TABLE mnesis.events ADD COLUMN forbidden integer",
        "CREATE TABLE mnesis.forbidden (value integer)",
    ] {
        let error = sqlx::raw_sql(statement).execute(&pool).await.unwrap_err();
        assert!(
            matches!(&error, sqlx::Error::Database(cause) if cause.code().as_deref() == Some("42501")),
            "{statement}: {error:?}"
        );
    }
    let rows: Vec<_> = store
        .read_all(None)
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows.first().unwrap().2.version(), Version::INITIAL);
    assert_eq!(rows.first().unwrap().2.payload(), b"committed over TLS");
    let encrypted: Option<bool> = sqlx::query_scalar("SELECT bool_and(s.ssl) FROM pg_catalog.pg_stat_ssl s JOIN pg_catalog.pg_stat_activity a ON a.pid = s.pid WHERE a.usename = 'pg8_runtime'")
        .fetch_one(&admin).await.unwrap();
    assert_eq!(encrypted, Some(true));
    drop(store);
    pool.close().await;
    admin.close().await;
}

#[cfg(all(feature = "projection", feature = "snapshot"))]
#[tokio::test]
async fn verified_tls_runtime_can_update_snapshots_and_checkpoints_then_reopen_exact_pairs() {
    if std::env::var_os("DATABASE_URL").is_none() {
        return;
    }
    let server = Secure::start();
    let admin = server.prepare().await;
    let (store, pool) = server.open().await;
    let events = StreamKey::from_bytes("tls-state-events");
    let state_id = StreamKey::from_bytes("tls-state");
    let mut revision = None;
    let mut final_position = None;
    for number in [1_u64, 2] {
        let version = Version::new(number).unwrap();
        let state = format!("tls-state-{number}").into_bytes();
        let event = pending_envelope(version)
            .event_type("Saved")
            .payload(state.clone())
            .build()
            .unwrap();
        let position = store
            .append(
                &events,
                number.checked_sub(1).and_then(Version::new),
                PendingBatch::of(&event),
            )
            .await
            .unwrap();
        store
            .commit(&state_id, NonZeroU32::MIN, version, &state)
            .await
            .unwrap();
        let next = store
            .commit_checkpoint(
                &state_id,
                CheckpointWrite {
                    expected: revision,
                    schema_version: NonZeroU32::MIN,
                    position,
                    state: &state,
                    mode: CheckpointMode::Advance,
                },
            )
            .await
            .unwrap();
        assert_eq!(next.get(), number);
        revision = Some(next);
        final_position = Some(position);
    }
    drop(store);
    pool.close().await;
    let (reopened, reopened_pool) = server.open().await;
    assert_eq!(
        reopened.hydrate(&state_id, NonZeroU32::MIN).await.unwrap(),
        Hydrated::Found {
            position: Version::new(2).unwrap(),
            state: b"tls-state-2".to_vec(),
        }
    );
    assert_eq!(
        reopened
            .hydrate_checkpoint(&state_id, NonZeroU32::MIN)
            .await
            .unwrap(),
        CheckpointHydrated::Found {
            revision: NonZeroU64::new(2).unwrap(),
            position: final_position.unwrap(),
            state: b"tls-state-2".to_vec(),
        }
    );
    drop(reopened);
    reopened_pool.close().await;
    admin.close().await;
}
