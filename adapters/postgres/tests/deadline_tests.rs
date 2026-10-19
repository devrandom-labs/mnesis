//! Caller-owned acquisition, statement and lock deadlines on actual adapter work.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "native deadline assertions"
)]

#[path = "support/cluster.rs"]
mod cluster;

use std::borrow::Cow;
use std::io::ErrorKind;
use std::time::Duration;

use mnesis::Version;
use mnesis_postgres::{PostgresError, PostgresStore};
use mnesis_store::{
    AppendError, PendingBatch, PendingEnvelope, RawEventStore, StreamKey, pending_envelope,
};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};
use sqlx::{Error as SqlxError, PgPool};
use tokio::task::yield_now;
use tokio::time::timeout;

use cluster::Cluster;

async fn open(settings: &[(&str, &str)]) -> Option<(PostgresStore, PgPool, PgPool)> {
    let url = std::env::var("DATABASE_URL").ok()?;
    let options = url.parse::<PgConnectOptions>().unwrap();
    let control = PgPool::connect_with(options.clone()).await.unwrap();
    PostgresStore::migrate(&control).await.unwrap();
    sqlx::query("TRUNCATE mnesis.events RESTART IDENTITY")
        .execute(&control)
        .await
        .unwrap();
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_millis(200))
        .connect_with(
            options
                .application_name("mnesis-deadline")
                .options(settings.iter().copied()),
        )
        .await
        .unwrap();
    let store = PostgresStore::from_pool(pool.clone()).await.unwrap();
    timeout(Duration::from_secs(3), async {
        loop {
            let ready: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT FROM pg_stat_activity WHERE application_name = 'mnesis-deadline' \
                 AND state = 'idle' AND query LIKE 'LISTEN %mnesis_events%')",
            ).fetch_one(&control).await.unwrap();
            if ready { break; }
            yield_now().await;
        }
    }).await.expect("the actual listener must reserve its connection");
    Some((store, pool, control))
}

fn event() -> PendingEnvelope {
    pending_envelope(Version::INITIAL)
        .event_type("Deadline")
        .payload(b"preserved".as_slice())
        .build()
        .unwrap()
}

fn code(error: &PostgresError) -> Option<String> {
    match error {
        PostgresError::Sqlx(SqlxError::Database(source)) => source.code().map(Cow::into_owned),
        _ => None,
    }
}

async fn inventory(control: &PgPool, key: &StreamKey) -> (i64, i64) {
    sqlx::query_as(
        "SELECT (SELECT count(*) FROM mnesis.events JOIN mnesis.keys ON keys.number = events.stream_id \
         WHERE keys.value = $1), (SELECT count(*) FROM mnesis.keys WHERE value = $1)",
    )
    .bind(key.as_bytes())
    .fetch_one(control)
    .await
    .unwrap()
}

#[tokio::test]
async fn exhausted_pool_bounds_reads_and_append_before_any_write() {
    let Some((store, pool, control)) = open(&[]).await else {
        return;
    };
    let held = pool.acquire().await.unwrap();
    let key = StreamKey::from_bytes("acquisition-deadline");
    let before = inventory(&control, &key).await;
    let event = event();
    let read = timeout(
        Duration::from_secs(3),
        store.read_stream(&key, Version::INITIAL),
    )
    .await;
    let append = timeout(
        Duration::from_secs(3),
        store.append(&key, None, PendingBatch::of(&event)),
    )
    .await;
    drop(held);
    let after = inventory(&control, &key).await;
    store
        .append(&key, None, PendingBatch::of(&event))
        .await
        .unwrap();
    assert_eq!(
        after, before,
        "failed admission must not persist events or keys"
    );
    assert!(matches!(
        read,
        Ok(Err(PostgresError::Sqlx(SqlxError::PoolTimedOut)))
    ));
    assert!(matches!(
        append,
        Ok(Err(AppendError::Store(PostgresError::Sqlx(
            SqlxError::PoolTimedOut
        ))))
    ));
    drop(store);
    timeout(Duration::from_secs(3), pool.close()).await.unwrap();
    control.close().await;
}

#[tokio::test]
async fn lock_deadline_rejects_append_then_releases_its_transaction() {
    let Some((store, pool, control)) = open(&[("lock_timeout", "100ms")]).await else {
        return;
    };
    let key = StreamKey::from_bytes("lock-deadline");
    let before = inventory(&control, &key).await;
    let mut held = control.begin().await.unwrap();
    sqlx::query("LOCK TABLE mnesis.events IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *held)
        .await
        .unwrap();
    let event = event();
    let result = timeout(
        Duration::from_secs(3),
        store.append(&key, None, PendingBatch::of(&event)),
    )
    .await;
    held.rollback().await.unwrap();
    let after = inventory(&control, &key).await;
    store
        .append(&key, None, PendingBatch::of(&event))
        .await
        .unwrap();
    assert_eq!(
        after, before,
        "lock rejection must not persist events or keys"
    );
    assert!(
        matches!(&result, Ok(Err(AppendError::Store(error)))
        if code(error).as_deref() == Some("55P03")),
        "{result:?}"
    );
    drop(store);
    timeout(Duration::from_secs(3), pool.close()).await.unwrap();
    control.close().await;
}

#[tokio::test]
async fn statement_deadline_rolls_back_an_already_staged_insert() {
    let Some((store, pool, control)) = open(&[("statement_timeout", "100ms")]).await else {
        return;
    };
    sqlx::raw_sql(
        "CREATE FUNCTION mnesis.deadline_pause() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN PERFORM pg_sleep(5); RETURN NEW; END $$; \
         CREATE TRIGGER deadline_pause AFTER INSERT ON mnesis.events \
         FOR EACH ROW EXECUTE FUNCTION mnesis.deadline_pause()",
    )
    .execute(&control)
    .await
    .unwrap();
    let key = StreamKey::from_bytes("statement-deadline");
    let before = inventory(&control, &key).await;
    let event = event();
    let result = timeout(
        Duration::from_secs(3),
        store.append(&key, None, PendingBatch::of(&event)),
    )
    .await;
    sqlx::raw_sql(
        "DROP TRIGGER deadline_pause ON mnesis.events; DROP FUNCTION mnesis.deadline_pause()",
    )
    .execute(&control)
    .await
    .unwrap();
    let after = inventory(&control, &key).await;
    store
        .append(&key, None, PendingBatch::of(&event))
        .await
        .unwrap();
    assert_eq!(
        after, before,
        "statement rejection must roll back events and keys"
    );
    assert!(
        matches!(&result, Ok(Err(AppendError::Store(error)))
        if code(error).as_deref() == Some("57014")),
        "{result:?}"
    );
    drop(store);
    timeout(Duration::from_secs(3), pool.close()).await.unwrap();
    control.close().await;
}

#[tokio::test]
async fn startup_obeys_the_native_lock_deadline_without_changing_metadata() {
    let Some((store, pool, control)) = open(&[("lock_timeout", "100ms")]).await else {
        return;
    };
    drop(store);
    let before: Vec<(i64, Vec<u8>)> =
        sqlx::query_as("SELECT version, checksum FROM mnesis._sqlx_migrations ORDER BY version")
            .fetch_all(&control)
            .await
            .unwrap();
    let mut held = control.begin().await.unwrap();
    sqlx::query("LOCK TABLE mnesis._sqlx_migrations IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *held)
        .await
        .unwrap();
    let result = timeout(
        Duration::from_secs(3),
        PostgresStore::from_pool(pool.clone()),
    )
    .await;
    held.rollback().await.unwrap();
    let after: Vec<(i64, Vec<u8>)> =
        sqlx::query_as("SELECT version, checksum FROM mnesis._sqlx_migrations ORDER BY version")
            .fetch_all(&control)
            .await
            .unwrap();
    let repaired = PostgresStore::from_pool(pool.clone()).await.unwrap();
    assert_eq!(after, before);
    assert!(matches!(&result, Ok(Err(error))
        if code(error).as_deref() == Some("55P03")));
    drop(repaired);
    timeout(Duration::from_secs(3), pool.close()).await.unwrap();
    control.close().await;
}

#[tokio::test]
async fn permanent_server_loss_bounds_operations_and_pool_shutdown() {
    if std::env::var_os("DATABASE_URL").is_none() {
        return;
    }
    let cluster = Cluster::initialize();
    cluster.launch(&format!(
        "-k {} -p 55437 -c listen_addresses=''",
        cluster.socket().display()
    ));
    let options = PgConnectOptions::new()
        .host(cluster.socket().to_str().unwrap())
        .port(55437)
        .username("mnesis")
        .database("postgres")
        .ssl_mode(PgSslMode::Disable);
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_millis(200))
        .connect_with(options)
        .await
        .unwrap();
    PostgresStore::migrate(&pool).await.unwrap();
    let store = PostgresStore::from_pool(pool.clone()).await.unwrap();
    assert!(cluster.socket().join(".s.PGSQL.55437").exists());
    let stopped = cluster.stop().unwrap();
    assert!(stopped.status.success(), "{stopped:?}");
    assert!(!cluster.socket().join(".s.PGSQL.55437").exists());
    for _ in 0..3 {
        let error = timeout(Duration::from_secs(3), store.read_all(None))
            .await
            .expect("unavailable reads must be bounded")
            .err()
            .expect("the stopped server cannot provide a scan");
        assert!(
            matches!(error, PostgresError::Sqlx(SqlxError::Io(ref source))
                if source.kind() == ErrorKind::NotFound),
            "{error:?}"
        );
        assert!(pool.size() <= 2);
    }
    drop(store);
    timeout(Duration::from_secs(3), pool.close())
        .await
        .expect("shutdown must complete while the server stays down");
    assert_eq!(pool.size(), 0);
}
