//! Application schemas cannot replace PostgreSQL's query operations.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "native query-name assertions"
)]

use std::time::Duration;

use futures::TryStreamExt;
use mnesis::Version;
use mnesis_postgres::PostgresStore;
#[cfg(feature = "projection")]
use mnesis_store::checkpoint::{
    CheckpointHydrated, CheckpointMode, CheckpointStore, CheckpointWrite,
};
use mnesis_store::error::AppendError;
#[cfg(feature = "snapshot")]
use mnesis_store::state::{Hydrated, SnapshotStore};
use mnesis_store::{PendingBatch, RawEventStore, StreamKey, pending_envelope};
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
#[cfg(any(feature = "snapshot", feature = "projection"))]
use std::num::NonZeroU32;
use tokio::task::yield_now;
use tokio::time::timeout;

async fn fixture() -> Option<(PostgresStore, PgPool, PgPool)> {
    let address = std::env::var("DATABASE_URL").ok()?;
    let name = format!("pg9-query-names-{}", std::process::id());
    let options = address.parse::<PgConnectOptions>().unwrap();
    let admin = PgPoolOptions::new()
        .max_connections(2)
        .connect_with(options.clone())
        .await
        .unwrap();
    PostgresStore::migrate(&admin).await.unwrap();
    sqlx::raw_sql("TRUNCATE mnesis.events, mnesis.snapshots, mnesis.checkpoints, mnesis.keys RESTART IDENTITY")
        .execute(&admin).await.unwrap();
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect_with(options.application_name(&name))
        .await
        .unwrap();
    let store = PostgresStore::from_pool(pool.clone()).await.unwrap();
    for (key, body) in [("first", "first-body"), ("second", "second-body")] {
        let event = pending_envelope(Version::INITIAL)
            .event_type("Private")
            .payload(body.as_bytes())
            .build()
            .unwrap();
        store
            .append(&StreamKey::from_bytes(key), None, PendingBatch::of(&event))
            .await
            .unwrap();
    }
    timeout(Duration::from_secs(3), async {
        loop {
            let ready: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_stat_activity WHERE application_name = $1 AND state = 'idle' AND query LIKE 'LISTEN %mnesis_events%')")
                .bind(&name).fetch_one(&admin).await.unwrap();
            if ready { break; }
            yield_now().await;
        }
    }).await.unwrap();
    Some((store, pool, admin))
}

async fn remove_probe(pool: &PgPool, admin: &PgPool) {
    let path: String = sqlx::query_scalar("SHOW search_path")
        .fetch_one(pool)
        .await
        .unwrap();
    sqlx::raw_sql("DROP SCHEMA mnesis_name_probe CASCADE")
        .execute(admin)
        .await
        .unwrap();
    assert_eq!(path, "mnesis_name_probe, pg_catalog");
}

#[tokio::test]
async fn application_byte_equality_cannot_return_another_streams_events() {
    let Some((store, pool, admin)) = fixture().await else {
        return;
    };
    let id = StreamKey::from_bytes("second");
    let healthy: Vec<_> = store
        .read_stream(&id, Version::INITIAL)
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(healthy.len(), 1);
    assert_eq!(healthy[0].payload(), b"second-body");
    sqlx::raw_sql("CREATE SCHEMA mnesis_name_probe; CREATE FUNCTION mnesis_name_probe.equal_bytes(bytea, bytea) RETURNS boolean LANGUAGE sql IMMUTABLE AS $$ SELECT true $$; CREATE OPERATOR mnesis_name_probe.= (LEFTARG = bytea, RIGHTARG = bytea, FUNCTION = mnesis_name_probe.equal_bytes)")
        .execute(&admin).await.unwrap();
    sqlx::raw_sql("SET search_path TO mnesis_name_probe, pg_catalog")
        .execute(&pool)
        .await
        .unwrap();
    let (redirected, native): (bool, bool) = sqlx::query_as("SELECT pg_catalog.decode('01', 'hex') = pg_catalog.decode('02', 'hex'), pg_catalog.decode('01', 'hex') OPERATOR(pg_catalog.=) pg_catalog.decode('02', 'hex')")
        .fetch_one(&pool).await.unwrap();
    assert!(redirected);
    assert!(!native);
    let result = store
        .read_stream(&id, Version::INITIAL)
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await;
    remove_probe(&pool, &admin).await;
    let rows = result.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].version(), Version::INITIAL);
    assert_eq!(rows[0].payload(), b"second-body");
    drop(store);
    pool.close().await;
    admin.close().await;
}

#[tokio::test]
async fn application_maximum_cannot_forge_a_committed_head() {
    let Some((store, pool, admin)) = fixture().await else {
        return;
    };
    sqlx::raw_sql("CREATE SCHEMA mnesis_name_probe; CREATE FUNCTION mnesis_name_probe.forge_max(numeric, numeric) RETURNS numeric LANGUAGE sql IMMUTABLE AS $$ SELECT 999::numeric $$; CREATE AGGREGATE mnesis_name_probe.max(numeric) (SFUNC = mnesis_name_probe.forge_max, STYPE = numeric, INITCOND = '999')")
        .execute(&admin).await.unwrap();
    sqlx::raw_sql("SET search_path TO mnesis_name_probe, pg_catalog")
        .execute(&pool)
        .await
        .unwrap();
    let selected: (String, String) = sqlx::query_as(
        "SELECT max(version)::text, pg_catalog.max(version)::text FROM mnesis.events",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(selected, ("999".to_owned(), "1".to_owned()));
    let expected = Version::new(999).unwrap();
    let event = pending_envelope(Version::new(1000).unwrap())
        .event_type("Forged")
        .payload("must-not-land")
        .build()
        .unwrap();
    let result = store
        .append(
            &StreamKey::from_bytes("second"),
            Some(expected),
            PendingBatch::of(&event),
        )
        .await;
    remove_probe(&pool, &admin).await;
    let rows: Vec<(String, Vec<u8>)> =
        sqlx::query_as("SELECT version::text, payload FROM mnesis.events ORDER BY global_seq")
            .fetch_all(&admin)
            .await
            .unwrap();
    assert_eq!(
        rows,
        vec![
            ("1".to_owned(), b"first-body".to_vec()),
            ("1".to_owned(), b"second-body".to_vec())
        ]
    );
    assert!(
        matches!(result, Err(AppendError::Conflict { expected: Some(value), actual: Some(head), .. }) if value == expected && head == Version::INITIAL),
        "{result:?}"
    );
    drop(store);
    pool.close().await;
    admin.close().await;
}

#[cfg(feature = "snapshot")]
#[tokio::test]
async fn application_byte_equality_cannot_redirect_a_snapshot_write() {
    let Some((store, pool, admin)) = fixture().await else {
        return;
    };
    let schema = NonZeroU32::new(1).unwrap();
    store
        .commit(
            &StreamKey::from_bytes("first"),
            schema,
            Version::INITIAL,
            &b"first-state".to_vec(),
        )
        .await
        .unwrap();
    sqlx::raw_sql("CREATE SCHEMA mnesis_name_probe; CREATE FUNCTION mnesis_name_probe.equal_bytes(bytea, bytea) RETURNS boolean LANGUAGE sql IMMUTABLE AS $$ SELECT true $$; CREATE OPERATOR mnesis_name_probe.= (LEFTARG = bytea, RIGHTARG = bytea, FUNCTION = mnesis_name_probe.equal_bytes)")
        .execute(&admin).await.unwrap();
    sqlx::raw_sql("SET search_path TO mnesis_name_probe, pg_catalog")
        .execute(&pool)
        .await
        .unwrap();
    let result = store
        .commit(
            &StreamKey::from_bytes("second"),
            schema,
            Version::INITIAL,
            &b"second-state".to_vec(),
        )
        .await;
    remove_probe(&pool, &admin).await;
    let rows: Vec<(Vec<u8>, Vec<u8>)> = sqlx::query_as("SELECT k.value, s.state FROM mnesis.snapshots s JOIN mnesis.keys k ON k.number = s.key ORDER BY k.number")
        .fetch_all(&admin).await.unwrap();
    result.unwrap();
    assert_eq!(
        rows,
        vec![
            (b"first".to_vec(), b"first-state".to_vec()),
            (b"second".to_vec(), b"second-state".to_vec())
        ]
    );
    drop(store);
    pool.close().await;
    admin.close().await;
}

#[cfg(feature = "snapshot")]
#[tokio::test]
async fn application_byte_equality_cannot_read_another_snapshots_state() {
    let Some((store, pool, admin)) = fixture().await else {
        return;
    };
    let schema = NonZeroU32::new(1).unwrap();
    for (key, body) in [("first", "first-state"), ("second", "second-state")] {
        store
            .commit(
                &StreamKey::from_bytes(key),
                schema,
                Version::INITIAL,
                &body.as_bytes().to_vec(),
            )
            .await
            .unwrap();
    }
    sqlx::raw_sql("CREATE SCHEMA mnesis_name_probe; CREATE FUNCTION mnesis_name_probe.equal_bytes(bytea, bytea) RETURNS boolean LANGUAGE sql IMMUTABLE AS $$ SELECT true $$; CREATE OPERATOR mnesis_name_probe.= (LEFTARG = bytea, RIGHTARG = bytea, FUNCTION = mnesis_name_probe.equal_bytes)")
        .execute(&admin).await.unwrap();
    sqlx::raw_sql("SET search_path TO mnesis_name_probe, pg_catalog")
        .execute(&pool)
        .await
        .unwrap();
    let result = store
        .hydrate(&StreamKey::from_bytes("second"), schema)
        .await;
    remove_probe(&pool, &admin).await;
    assert_eq!(
        result.unwrap(),
        Hydrated::Found {
            position: Version::INITIAL,
            state: b"second-state".to_vec()
        }
    );
    drop(store);
    pool.close().await;
    admin.close().await;
}

#[cfg(feature = "projection")]
#[tokio::test]
async fn application_byte_equality_cannot_read_another_checkpoints_pair() {
    let Some((store, pool, admin)) = fixture().await else {
        return;
    };
    let schema = NonZeroU32::new(1).unwrap();
    let events = store
        .read_all(None)
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    assert_eq!(events.len(), 2);
    for ((key, body), event) in [("first", "first-state"), ("second", "second-state")]
        .into_iter()
        .zip(&events)
    {
        store
            .commit_checkpoint(
                &StreamKey::from_bytes(key),
                CheckpointWrite {
                    expected: None,
                    schema_version: schema,
                    position: event.0,
                    state: &body.as_bytes().to_vec(),
                    mode: CheckpointMode::Advance,
                },
            )
            .await
            .unwrap();
    }
    sqlx::raw_sql("CREATE SCHEMA mnesis_name_probe; CREATE FUNCTION mnesis_name_probe.equal_bytes(bytea, bytea) RETURNS boolean LANGUAGE sql IMMUTABLE AS $$ SELECT true $$; CREATE OPERATOR mnesis_name_probe.= (LEFTARG = bytea, RIGHTARG = bytea, FUNCTION = mnesis_name_probe.equal_bytes)")
        .execute(&admin).await.unwrap();
    sqlx::raw_sql("SET search_path TO mnesis_name_probe, pg_catalog")
        .execute(&pool)
        .await
        .unwrap();
    let result = store
        .hydrate_checkpoint(&StreamKey::from_bytes("second"), schema)
        .await;
    remove_probe(&pool, &admin).await;
    assert_eq!(
        result.unwrap(),
        CheckpointHydrated::Found {
            revision: std::num::NonZeroU64::MIN,
            position: events[1].0,
            state: b"second-state".to_vec(),
        }
    );
    drop(store);
    pool.close().await;
    admin.close().await;
}
