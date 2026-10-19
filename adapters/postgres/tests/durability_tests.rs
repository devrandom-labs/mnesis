//! Native durability settings must match acknowledged-write guarantees.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "native durability assertions"
)]

#[path = "support/cluster.rs"]
mod cluster;

#[cfg(any(feature = "snapshot", feature = "projection"))]
use std::num::NonZeroU32;
#[cfg(feature = "projection")]
use std::num::NonZeroU64;
use std::process::Command;
use std::time::Duration;

use futures::TryStreamExt;
use mnesis::Version;
use mnesis_postgres::{PgAllPos, PostgresError, PostgresStore};
#[cfg(feature = "projection")]
use mnesis_store::checkpoint::{
    CheckpointError, CheckpointHydrated, CheckpointMode, CheckpointStore, CheckpointWrite,
};
#[cfg(feature = "import")]
use mnesis_store::import::{AtomicAppend, AtomicAppendError, PlannedAppend};
#[cfg(feature = "snapshot")]
use mnesis_store::state::{Hydrated, SnapshotStore};
use mnesis_store::{
    AppendError, PendingBatch, PendingEnvelope, RawEventStore, StreamKey, pending_envelope,
};
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};
use tokio::task::yield_now;
use tokio::time::timeout;

use cluster::Cluster;

const PORT: u16 = 55436;

#[tokio::test]
async fn asynchronous_connection_settings_reject_before_runtime_open() {
    let Ok(address) = std::env::var("DATABASE_URL") else {
        return;
    };
    let options = address.parse::<PgConnectOptions>().unwrap();
    let healthy = PgPoolOptions::new()
        .max_connections(4)
        .connect_with(options.clone().options([("synchronous_commit", "on")]))
        .await
        .unwrap();
    PostgresStore::migrate(&healthy).await.unwrap();
    let enabled: String = sqlx::query_scalar("SHOW synchronous_commit")
        .fetch_one(&healthy)
        .await
        .unwrap();
    assert_eq!(enabled, "on");
    let control = PostgresStore::from_pool(healthy.clone()).await.unwrap();
    drop(control);
    let asynchronous = PgPoolOptions::new()
        .max_connections(4)
        .connect_with(options.options([("synchronous_commit", "off")]))
        .await
        .unwrap();
    let disabled: String = sqlx::query_scalar("SHOW synchronous_commit")
        .fetch_one(&asynchronous)
        .await
        .unwrap();
    assert_eq!(disabled, "off");
    let rejection = PostgresStore::from_pool(asynchronous.clone())
        .await
        .err()
        .expect("asynchronous commit must reject the durable runtime profile");
    rejected(&rejection);
    asynchronous.close().await;
    healthy.close().await;
}

struct Fixture {
    store: PostgresStore,
    pool: PgPool,
    admin: PgPool,
    position: PgAllPos,
}

fn event() -> PendingEnvelope {
    pending_envelope(Version::INITIAL)
        .event_type("Durable")
        .payload(b"acknowledged state".as_slice())
        .build()
        .unwrap()
}

fn rejected(error: &PostgresError) {
    assert!(
        matches!(error, PostgresError::Durability { setting } if setting.to_string() == "synchronous_commit"),
        "{error:?}"
    );
}

impl Fixture {
    async fn replaced() -> Option<Self> {
        let address = std::env::var("DATABASE_URL").ok()?;
        let name = format!("pg8-durability-{}", std::process::id());
        let options = address
            .parse::<PgConnectOptions>()
            .unwrap()
            .options([("synchronous_commit", "on")]);
        let admin = PgPoolOptions::new()
            .max_connections(2)
            .connect_with(options.clone())
            .await
            .unwrap();
        PostgresStore::migrate(&admin).await.unwrap();
        sqlx::raw_sql("TRUNCATE mnesis.events, mnesis.snapshots, mnesis.checkpoints, mnesis.keys RESTART IDENTITY")
            .execute(&admin).await.unwrap();
        let runtime_options = options.application_name(&name);
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .connect_with(runtime_options.clone())
            .await
            .unwrap();
        let store = PostgresStore::from_pool(pool.clone()).await.unwrap();
        let event = event();
        let position = store
            .append(
                &StreamKey::from_bytes("control"),
                None,
                PendingBatch::of(&event),
            )
            .await
            .unwrap();
        timeout(Duration::from_secs(3), async {
            loop {
                let ready: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_stat_activity WHERE application_name = $1 AND state = 'idle' AND query LIKE 'LISTEN %mnesis_events%')")
                    .bind(&name).fetch_one(&admin).await.unwrap();
                if ready { break; }
                yield_now().await;
            }
        }).await.unwrap();
        let mut original = pool.acquire().await.unwrap();
        let old: i32 = sqlx::query_scalar("SELECT pg_catalog.pg_backend_pid()")
            .fetch_one(&mut *original)
            .await
            .unwrap();
        pool.set_connect_options(runtime_options.options([("synchronous_commit", "off")]));
        original.close().await.unwrap();
        let mut replacement = pool.acquire().await.unwrap();
        let (new, setting): (i32, String) = sqlx::query_as(
            "SELECT pg_catalog.pg_backend_pid(), pg_catalog.current_setting('synchronous_commit')",
        )
        .fetch_one(&mut *replacement)
        .await
        .unwrap();
        assert_ne!(old, new);
        assert_eq!(setting, "off");
        drop(replacement);
        Some(Self {
            store,
            pool,
            admin,
            position,
        })
    }

    async fn unchanged(self) {
        drop(self.store);
        self.pool.close().await;
        let counts: (i64, i64, i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM mnesis.events), (SELECT count(*) FROM mnesis.keys), (SELECT count(*) FROM mnesis.snapshots), (SELECT count(*) FROM mnesis.checkpoints)")
            .fetch_one(&self.admin).await.unwrap();
        assert_eq!(counts, (1, 1, 0, 0));
        let (transaction, sequence, payload): (String, i64, Vec<u8>) =
            sqlx::query_as("SELECT txid::text, global_seq, payload FROM mnesis.events")
                .fetch_one(&self.admin)
                .await
                .unwrap();
        assert_eq!(transaction.parse::<u64>().unwrap(), self.position.txid());
        assert_eq!(u64::try_from(sequence).unwrap(), self.position.seq());
        assert_eq!(payload, b"acknowledged state");
        self.admin.close().await;
    }
}

#[tokio::test]
async fn replacement_connection_cannot_acknowledge_an_asynchronous_append() {
    let Some(fixture) = Fixture::replaced().await else {
        return;
    };
    let event = event();
    let result = fixture
        .store
        .append(
            &StreamKey::from_bytes("rejected"),
            None,
            PendingBatch::of(&event),
        )
        .await;
    assert!(
        matches!(&result, Err(AppendError::Store(PostgresError::Durability { setting })) if setting.to_string() == "synchronous_commit"),
        "{result:?}"
    );
    fixture.unchanged().await;
}

#[cfg(feature = "import")]
#[tokio::test]
async fn replacement_connection_cannot_acknowledge_an_asynchronous_atomic_append() {
    let Some(fixture) = Fixture::replaced().await else {
        return;
    };
    let writes = ["first", "last"].map(|id| PlannedAppend {
        target: StreamKey::from_bytes(id),
        expected_version: None,
        head: event(),
        tail: Vec::new(),
    });
    let result = fixture.store.atomic_append_many(&writes).await;
    assert!(
        matches!(&result, Err(AtomicAppendError::Store(PostgresError::Durability { setting })) if setting.to_string() == "synchronous_commit"),
        "{result:?}"
    );
    fixture.unchanged().await;
}

#[cfg(feature = "snapshot")]
#[tokio::test]
async fn replacement_connection_cannot_acknowledge_an_asynchronous_snapshot() {
    let Some(fixture) = Fixture::replaced().await else {
        return;
    };
    let rejection = fixture
        .store
        .commit(
            &StreamKey::from_bytes("rejected"),
            NonZeroU32::MIN,
            Version::INITIAL,
            &b"unflushed snapshot".to_vec(),
        )
        .await
        .expect_err("replacement asynchronous snapshot must reject");
    rejected(&rejection);
    fixture.unchanged().await;
}

#[cfg(feature = "projection")]
#[tokio::test]
async fn replacement_connection_cannot_acknowledge_an_asynchronous_checkpoint() {
    let Some(fixture) = Fixture::replaced().await else {
        return;
    };
    let state = b"unflushed projection".to_vec();
    let result = fixture
        .store
        .commit_checkpoint(
            &StreamKey::from_bytes("rejected"),
            CheckpointWrite {
                expected: None,
                schema_version: NonZeroU32::MIN,
                position: fixture.position,
                state: &state,
                mode: CheckpointMode::Advance,
            },
        )
        .await;
    assert!(
        matches!(&result, Err(CheckpointError::Store(PostgresError::Durability { setting })) if setting.to_string() == "synchronous_commit"),
        "{result:?}"
    );
    fixture.unchanged().await;
}

async fn commit_mode(pool: &PgPool, mode: &str) {
    let actual: String =
        sqlx::query_scalar("SELECT pg_catalog.set_config('synchronous_commit', $1, false)")
            .bind(mode)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(actual, mode);
    pool.set_connect_options(
        pool.connect_options()
            .as_ref()
            .clone()
            .options([("synchronous_commit", mode)]),
    );
}

#[tokio::test]
async fn supported_commit_modes_are_preserved_after_acknowledged_writes() {
    for mode in ["on", "local", "remote_write", "remote_apply"] {
        let Some(fixture) = Fixture::replaced().await else {
            return;
        };
        commit_mode(&fixture.pool, mode).await;
        let event = event();
        let position = fixture
            .store
            .append(
                &StreamKey::from_bytes("accepted"),
                None,
                PendingBatch::of(&event),
            )
            .await
            .unwrap();
        assert!(position > fixture.position);
        let actual: String = sqlx::query_scalar("SHOW synchronous_commit")
            .fetch_one(&fixture.pool)
            .await
            .unwrap();
        assert_eq!(actual, mode);
        drop(fixture.store);
        fixture.pool.close().await;
        let rows: Vec<(Vec<u8>, String, Vec<u8>)> = sqlx::query_as("SELECT k.value, e.version::text, e.payload FROM mnesis.events e JOIN mnesis.keys k ON k.number = e.stream_id ORDER BY k.value")
            .fetch_all(&fixture.admin).await.unwrap();
        assert_eq!(
            rows,
            [
                (
                    b"accepted".to_vec(),
                    "1".to_owned(),
                    b"acknowledged state".to_vec()
                ),
                (
                    b"control".to_vec(),
                    "1".to_owned(),
                    b"acknowledged state".to_vec()
                ),
            ]
        );
        fixture.admin.close().await;
    }
}

#[tokio::test]
async fn a_setting_changed_by_a_native_write_trigger_rejects_before_commit() {
    for trigger in [
        "CREATE TRIGGER durability_probe AFTER INSERT ON mnesis.events FOR EACH ROW EXECUTE FUNCTION mnesis.durability_probe()",
        "CREATE CONSTRAINT TRIGGER durability_probe AFTER INSERT ON mnesis.events DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION mnesis.durability_probe()",
    ] {
        let Some(fixture) = Fixture::replaced().await else {
            return;
        };
        commit_mode(&fixture.pool, "on").await;
        sqlx::raw_sql("CREATE OR REPLACE FUNCTION mnesis.durability_probe() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_catalog.set_config('synchronous_commit', 'off', true); RETURN NEW; END $$")
            .execute(&fixture.admin).await.unwrap();
        sqlx::raw_sql(trigger)
            .execute(&fixture.admin)
            .await
            .unwrap();
        let event = event();
        let result = fixture
            .store
            .append(
                &StreamKey::from_bytes("rejected"),
                None,
                PendingBatch::of(&event),
            )
            .await;
        sqlx::raw_sql("DROP TRIGGER durability_probe ON mnesis.events; DROP FUNCTION mnesis.durability_probe()")
            .execute(&fixture.admin).await.unwrap();
        let restored: String = sqlx::query_scalar("SHOW synchronous_commit")
            .fetch_one(&fixture.pool)
            .await
            .unwrap();
        fixture.unchanged().await;
        assert!(
            matches!(&result, Err(AppendError::Store(PostgresError::Durability { setting })) if setting.to_string() == "synchronous_commit"),
            "{trigger}: {result:?}"
        );
        assert_eq!(restored, "on");
    }
}

fn launch(cluster: &Cluster, settings: &str) {
    cluster.launch(&format!(
        "-h '' -k {} -p {PORT} {settings}",
        cluster.socket().display()
    ));
}

async fn native_pool(cluster: &Cluster) -> PgPool {
    PgPoolOptions::new()
        .max_connections(4)
        .connect_with(
            PgConnectOptions::new_without_pgpass()
                .host(cluster.socket().to_str().unwrap())
                .port(PORT)
                .database("postgres")
                .username("mnesis")
                .ssl_mode(PgSslMode::Disable),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn unsafe_server_settings_reject_runtime_and_migration_without_repair() {
    if std::env::var_os("DATABASE_URL").is_none() {
        return;
    }
    for setting in ["fsync", "full_page_writes"] {
        let server = Cluster::initialize();
        launch(&server, "");
        let original = native_pool(&server).await;
        PostgresStore::migrate(&original).await.unwrap();
        original.close().await;
        let stopped = server.stop().unwrap();
        assert!(stopped.status.success(), "{stopped:?}");
        launch(&server, &format!("-c {setting}=off"));
        let pool = native_pool(&server).await;
        let disabled: String = sqlx::query_scalar("SELECT pg_catalog.current_setting($1)")
            .bind(setting)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(disabled, "off");
        let error = PostgresStore::from_pool(pool.clone()).await.err().unwrap();
        assert!(
            matches!(&error, PostgresError::Durability { setting: actual } if actual.to_string() == setting),
            "{error:?}"
        );
        let migration = PostgresStore::migrate(&pool).await.unwrap_err();
        assert!(
            matches!(&migration, PostgresError::Durability { setting: actual } if actual.to_string() == setting),
            "{migration:?}"
        );
        let versions: Vec<i64> =
            sqlx::query_scalar("SELECT version FROM mnesis._sqlx_migrations ORDER BY version")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(versions, [1, 2, 3, 4, 5]);
        pool.close().await;
    }
}

#[cfg(feature = "snapshot")]
async fn save_snapshot(store: &PostgresStore) {
    let id = StreamKey::from_bytes("saved");
    let state = b"acknowledged state".to_vec();
    store
        .commit(&id, NonZeroU32::MIN, Version::INITIAL, &state)
        .await
        .unwrap();
}

#[cfg(feature = "projection")]
async fn save_checkpoint(store: &PostgresStore, position: PgAllPos) {
    let id = StreamKey::from_bytes("saved");
    let state = b"acknowledged state".to_vec();
    assert_eq!(
        store
            .commit_checkpoint(
                &id,
                CheckpointWrite {
                    expected: None,
                    schema_version: NonZeroU32::MIN,
                    position,
                    state: &state,
                    mode: CheckpointMode::Advance,
                }
            )
            .await
            .unwrap(),
        NonZeroU64::MIN
    );
}

#[cfg(feature = "snapshot")]
async fn check_snapshot(store: &PostgresStore) {
    let id = StreamKey::from_bytes("saved");
    assert_eq!(
        store.hydrate(&id, NonZeroU32::MIN).await.unwrap(),
        Hydrated::Found {
            position: Version::INITIAL,
            state: b"acknowledged state".to_vec(),
        }
    );
}

#[cfg(feature = "projection")]
async fn check_checkpoint(store: &PostgresStore, position: PgAllPos) {
    let id = StreamKey::from_bytes("saved");
    assert_eq!(
        store
            .hydrate_checkpoint(&id, NonZeroU32::MIN)
            .await
            .unwrap(),
        CheckpointHydrated::Found {
            revision: NonZeroU64::MIN,
            position,
            state: b"acknowledged state".to_vec(),
        }
    );
}

fn crash(server: &Cluster) {
    let stopped = Command::new("pg_ctl")
        .arg("-D")
        .arg(server.data())
        .args(["-m", "immediate", "-w", "stop"])
        .output()
        .unwrap();
    assert!(stopped.status.success(), "{stopped:?}");
    let status = Command::new("pg_ctl")
        .arg("-D")
        .arg(server.data())
        .arg("status")
        .output()
        .unwrap();
    assert_eq!(status.status.code(), Some(3), "{status:?}");
}

#[tokio::test]
async fn immediate_primary_restart_recovers_acknowledged_events_and_saved_state() {
    if std::env::var_os("DATABASE_URL").is_none() {
        return;
    }
    let server = Cluster::initialize();
    launch(&server, "");
    let pool = native_pool(&server).await;
    PostgresStore::migrate(&pool).await.unwrap();
    let store = PostgresStore::from_pool(pool.clone()).await.unwrap();
    let id = StreamKey::from_bytes("acknowledged");
    let position = store
        .append(&id, None, PendingBatch::of(&event()))
        .await
        .unwrap();
    #[cfg(feature = "snapshot")]
    save_snapshot(&store).await;
    #[cfg(feature = "projection")]
    save_checkpoint(&store, position).await;
    crash(&server);
    drop(store);
    pool.close().await;
    launch(&server, "");
    let restored_pool = native_pool(&server).await;
    let restored = PostgresStore::from_pool(restored_pool.clone())
        .await
        .unwrap();
    assert_eq!(restored.history(), position.history());
    let rows: Vec<_> = restored
        .read_all(None)
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (
            rows[0].0,
            &rows[0].1,
            rows[0].2.version(),
            rows[0].2.payload()
        ),
        (
            position,
            &id,
            Version::INITIAL,
            b"acknowledged state".as_slice()
        )
    );
    #[cfg(feature = "snapshot")]
    check_snapshot(&restored).await;
    #[cfg(feature = "projection")]
    check_checkpoint(&restored, position).await;
    let next = pending_envelope(Version::new(2).unwrap())
        .event_type("Durable")
        .payload(b"after crash".as_slice())
        .build()
        .unwrap();
    let advanced = restored
        .append(&id, Some(Version::INITIAL), PendingBatch::of(&next))
        .await
        .unwrap();
    assert!(advanced > position);
    let tail: Vec<_> = restored
        .read_all(Some(position))
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(tail.len(), 1);
    assert_eq!(
        (tail[0].0, tail[0].2.version(), tail[0].2.payload()),
        (
            advanced,
            Version::new(2).unwrap(),
            b"after crash".as_slice()
        )
    );
    drop(restored);
    restored_pool.close().await;
}

#[tokio::test]
async fn application_text_comparisons_cannot_hide_an_unsafe_commit_setting() {
    let Some(fixture) = Fixture::replaced().await else {
        return;
    };
    sqlx::raw_sql("CREATE SCHEMA mnesis_durability_probe; CREATE FUNCTION mnesis_durability_probe.never_equal(text, text) RETURNS boolean LANGUAGE sql IMMUTABLE AS $$ SELECT false $$; CREATE OPERATOR mnesis_durability_probe.= (LEFTARG = text, RIGHTARG = text, FUNCTION = mnesis_durability_probe.never_equal)")
        .execute(&fixture.admin).await.unwrap();
    sqlx::raw_sql("SET search_path TO mnesis_durability_probe, pg_catalog")
        .execute(&fixture.pool)
        .await
        .unwrap();
    let (native_commit, redirected): (String, bool) = sqlx::query_as(
        "SELECT pg_catalog.current_setting('synchronous_commit'), 'off'::text = 'off'::text",
    )
    .fetch_one(&fixture.pool)
    .await
    .unwrap();
    assert_eq!(native_commit, "off");
    assert!(!redirected);
    let result = fixture
        .store
        .append(
            &StreamKey::from_bytes("rejected"),
            None,
            PendingBatch::of(&event()),
        )
        .await;
    sqlx::raw_sql("DROP SCHEMA mnesis_durability_probe CASCADE")
        .execute(&fixture.admin)
        .await
        .unwrap();
    assert!(
        matches!(&result, Err(AppendError::Store(PostgresError::Durability { setting })) if setting.to_string() == "synchronous_commit"),
        "{result:?}"
    );
    fixture.unchanged().await;
}
