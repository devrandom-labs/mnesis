//! Versioned representation upgrade: existing positions and failed-step recovery.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "native test assertions"
)]

use futures::TryStreamExt;
use mnesis::Version;
use mnesis_postgres::{PgAllPos, PostgresError, PostgresStore};
use mnesis_store::envelope::pending_envelope;
use mnesis_store::{PendingBatch, RawEventStore, StreamKey};
use sqlx::PgPool;
use sqlx::migrate::{MigrateError, Migrator};

async fn previous() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    let pool = PgPool::connect(&url).await.unwrap();
    sqlx::raw_sql("DROP SCHEMA IF EXISTS mnesis CASCADE; DROP VIEW IF EXISTS public.events")
        .execute(&pool)
        .await
        .unwrap();
    let embedded = sqlx::migrate!("./migrations");
    let mut first = Migrator::with_migrations(
        embedded
            .iter()
            .filter(|m| m.version == 1)
            .cloned()
            .collect(),
    );
    first.create_schema("mnesis");
    first.dangerous_set_table_name("mnesis._sqlx_migrations");
    let mut connection = pool.acquire().await.unwrap();
    connection.close_on_drop();
    first
        .run_direct(None, &mut *connection, false)
        .await
        .unwrap();
    drop(connection);
    Some(pool)
}

async fn old_row(pool: &PgPool, version: i64) -> (i64, String) {
    sqlx::query_as("INSERT INTO mnesis.events (stream_id, version, event_type, schema_version, payload, metadata) VALUES ('upgrade'::bytea, $1, 'Old', 4294967295, 'old'::bytea, 'meta'::bytea) RETURNING global_seq, txid::text")
        .bind(version).fetch_one(pool).await.unwrap()
}

#[tokio::test]
async fn populated_version_one_preserves_position_and_next_identity() {
    let Some(pool) = previous().await else { return };
    let before = old_row(&pool, 1).await;
    PostgresStore::migrate(&pool).await.unwrap();
    PostgresStore::migrate(&pool).await.unwrap();
    let store = PostgresStore::from_pool(pool.clone()).await.unwrap();
    let existing: Vec<_> = store
        .read_all(None)
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(existing.len(), 1);
    let position = PgAllPos::new(before.1.parse().unwrap(), u64::try_from(before.0).unwrap());
    assert_eq!(existing[0].0, position);
    assert_eq!(existing[0].1, StreamKey::from_bytes("upgrade"));
    assert_eq!(existing[0].2.event_type(), "Old");
    assert_eq!(existing[0].2.payload(), b"old");
    assert_eq!(existing[0].2.metadata(), Some(b"meta".as_slice()));
    assert_eq!(existing[0].2.schema_version(), u32::MAX);
    let event = pending_envelope(Version::new(2).unwrap())
        .event_type("New\0Type")
        .payload(b"new".as_slice())
        .build()
        .unwrap();
    let next = store
        .append(
            &existing[0].1,
            Some(Version::INITIAL),
            PendingBatch::new(&[event]).unwrap(),
        )
        .await
        .unwrap();
    assert!(next > position);
    assert!(next.seq() > position.seq());
    let resumed: Vec<_> = store
        .read_all(Some(position))
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(resumed.len(), 1);
    assert_eq!(resumed[0].0, next);
    assert_eq!(resumed[0].2.version().as_u64(), 2);
    assert_eq!(resumed[0].2.event_type(), "New\0Type");
    let redundant: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_indexes WHERE schemaname = 'mnesis' AND indexname = 'events_stream_idx'").fetch_one(&pool).await.unwrap();
    assert_eq!(redundant, 0);
}

#[tokio::test]
async fn rejected_version_two_rolls_back_and_preserves_repairable_version_one() {
    let Some(pool) = previous().await else { return };
    let before = old_row(&pool, -1).await;
    let failure = PostgresStore::migrate(&pool).await.unwrap_err();
    assert!(
        matches!(failure, PostgresError::Migration(MigrateError::ExecuteMigration(ref source, 2)) if source.as_database_error().and_then(sqlx::error::DatabaseError::code).as_deref() == Some("23514"))
    );
    let unchanged: (Vec<u8>, i64, i64, String) =
        sqlx::query_as("SELECT stream_id, version, global_seq, txid::text FROM mnesis.events")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        unchanged,
        (b"upgrade".to_vec(), -1, before.0, before.1.clone())
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM mnesis._sqlx_migrations")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, Option<String>>("SELECT to_regclass('mnesis.keys')::text")
            .fetch_one(&pool)
            .await
            .unwrap(),
        None
    );
    sqlx::query("UPDATE mnesis.events SET version = 1")
        .execute(&pool)
        .await
        .unwrap();
    PostgresStore::migrate(&pool).await.unwrap();
    let store = PostgresStore::from_pool(pool).await.unwrap();
    let repaired: Vec<_> = store
        .read_all(None)
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(repaired.len(), 1);
    assert_eq!(
        repaired[0].0,
        PgAllPos::new(before.1.parse().unwrap(), u64::try_from(before.0).unwrap())
    );
    assert_eq!(repaired[0].2.version(), Version::INITIAL);
    assert_eq!(repaired[0].2.payload(), b"old");
}

#[tokio::test]
async fn foreign_public_view_is_rejected_without_replacement() {
    let Some(pool) = previous().await else { return };
    old_row(&pool, 1).await;
    sqlx::query("CREATE VIEW public.events AS SELECT 'foreign'::text AS owner")
        .execute(&pool)
        .await
        .unwrap();
    let result = PostgresStore::migrate(&pool).await;
    assert!(
        matches!(result, Err(PostgresError::Migration(_))),
        "foreign view must not be dropped: {result:?}"
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT owner FROM public.events")
            .fetch_one(&pool)
            .await
            .unwrap(),
        "foreign"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM mnesis._sqlx_migrations")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    sqlx::query("DROP VIEW public.events")
        .execute(&pool)
        .await
        .unwrap();
    PostgresStore::migrate(&pool).await.unwrap();
}
