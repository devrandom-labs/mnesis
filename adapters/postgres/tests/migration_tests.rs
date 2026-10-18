//! Native schema rejection witnesses; fixtures always restore their mutation.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test assertions")]

mod support;

use std::sync::Arc;
use std::time::Duration;

use futures::{StreamExt, TryStreamExt};
use mnesis::Version;
use mnesis_postgres::{PgAllPos, PostgresError, PostgresStore};
use mnesis_store::envelope::{PendingBatch, PendingEnvelope, pending_envelope};
use mnesis_store::{RawEventStore, StreamKey};
use sqlx::PgPool;
use sqlx::migrate::{Migrate, MigrateError};
use sqlx::postgres::PgPoolOptions;
use tokio::sync::Barrier;
use tokio::task::yield_now;
use tokio::time::timeout;

async fn fixture() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    let pool = PgPool::connect(&url).await.unwrap();
    PostgresStore::migrate(&pool).await.unwrap();
    let _store = PostgresStore::from_pool(pool.clone()).await.unwrap();
    Some(pool)
}

#[tokio::test]
async fn opening_rejects_inherited_checkpoint_rows_outside_the_parent_primary_key() {
    let Some(pool) = fixture().await else { return };
    let number = support::key(&pool, b"inherited-checkpoint").await;
    sqlx::raw_sql("CREATE TABLE mnesis.checkpoint_child () INHERITS (mnesis.checkpoints)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO mnesis.checkpoints (key, revision, schema_version, txid, global_seq, state) VALUES ($1, 1, 1, 1, 1, 'parent'::bytea)")
        .bind(number).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO mnesis.checkpoint_child (key, revision, schema_version, txid, global_seq, state) VALUES ($1, 2, 1, 2, 2, 'child'::bytea)")
        .bind(number).execute(&pool).await.unwrap();
    let states: Vec<Vec<u8>> =
        sqlx::query_scalar("SELECT state FROM mnesis.checkpoints WHERE key = $1 ORDER BY revision")
            .bind(number)
            .fetch_all(&pool)
            .await
            .unwrap();
    let opened = PostgresStore::from_pool(pool.clone()).await;
    sqlx::query("DROP TABLE mnesis.checkpoint_child")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM mnesis.checkpoints WHERE key = $1")
        .bind(number)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql("CREATE TABLE mnesis.checkpoint_parent (key bigint NOT NULL); ALTER TABLE mnesis.checkpoints INHERIT mnesis.checkpoint_parent")
        .execute(&pool).await.unwrap();
    let inherited = PostgresStore::from_pool(pool.clone()).await;
    sqlx::raw_sql("ALTER TABLE mnesis.checkpoints NO INHERIT mnesis.checkpoint_parent; DROP TABLE mnesis.checkpoint_parent")
        .execute(&pool).await.unwrap();
    assert_eq!(states, [b"parent".to_vec(), b"child".to_vec()]);
    assert!(
        matches!(opened, Err(PostgresError::Schema { .. })),
        "startup must reject inherited checkpoint rows beyond parent uniqueness"
    );
    assert!(
        matches!(inherited, Err(PostgresError::Schema { .. })),
        "the checkpoint table must also reject an unsupported inheritance parent"
    );
}

#[tokio::test]
async fn opening_rejects_a_missing_optimistic_constraint() {
    let Some(pool) = fixture().await else { return };
    sqlx::query("ALTER TABLE mnesis.events DROP CONSTRAINT events_stream_id_version_key")
        .execute(&pool)
        .await
        .unwrap();
    let opened = PostgresStore::from_pool(pool.clone()).await;
    sqlx::query("ALTER TABLE mnesis.events ADD CONSTRAINT events_stream_id_version_key UNIQUE (stream_id, version)")
        .execute(&pool).await.unwrap();
    assert!(
        matches!(opened, Err(PostgresError::Schema { .. })),
        "open ignored the missing conflict arbiter"
    );
}

#[tokio::test]
async fn opening_rejects_a_wrong_optimistic_constraint_with_the_right_name() {
    let Some(pool) = fixture().await else { return };
    sqlx::raw_sql("TRUNCATE mnesis.events; ALTER TABLE mnesis.events DROP CONSTRAINT events_stream_id_version_key; \
        ALTER TABLE mnesis.events ADD CONSTRAINT events_stream_id_version_key UNIQUE (payload)")
        .execute(&pool).await.unwrap();
    let opened = PostgresStore::from_pool(pool.clone()).await;
    sqlx::raw_sql("ALTER TABLE mnesis.events DROP CONSTRAINT events_stream_id_version_key; \
        ALTER TABLE mnesis.events ADD CONSTRAINT events_stream_id_version_key UNIQUE (stream_id, version)")
        .execute(&pool).await.unwrap();
    assert!(
        matches!(opened, Err(PostgresError::Schema { .. })),
        "a constraint name alone does not prove its contract"
    );
}

async fn clean() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    let pool = PgPool::connect(&url).await.unwrap();
    sqlx::raw_sql(
        "DROP SCHEMA IF EXISTS mnesis CASCADE; DROP SCHEMA IF EXISTS shadow CASCADE; \
         DO $$ BEGIN \
         IF (SELECT relkind FROM pg_class WHERE oid = to_regclass('public.events')) = 'v' \
         THEN DROP VIEW public.events; ELSE DROP TABLE IF EXISTS public.events; END IF; END $$;",
    )
    .execute(&pool)
    .await
    .unwrap();
    Some(pool)
}

async fn legacy(pool: &PgPool) -> (i64, String) {
    sqlx::raw_sql(include_str!("fixtures/legacy.sql"))
        .execute(pool)
        .await
        .unwrap();
    sqlx::query_as(
        "INSERT INTO public.events (stream_id, version, event_type, schema_version, payload, metadata) \
         VALUES ('legacy'::bytea, 1, 'Legacy', 4294967295, 'data'::bytea, 'meta'::bytea) \
         RETURNING global_seq, txid::text",
    ).fetch_one(pool).await.unwrap()
}

#[tokio::test]
async fn fresh_install_is_explicit_and_repeated_migration_preserves_data() {
    let Some(pool) = clean().await else { return };
    assert!(matches!(
        PostgresStore::from_pool(pool.clone()).await,
        Err(PostgresError::Schema { .. })
    ));
    assert_eq!(
        sqlx::query_scalar::<_, Option<String>>("SELECT to_regclass('mnesis.events')::text")
            .fetch_one(&pool)
            .await
            .unwrap(),
        None
    );
    PostgresStore::migrate(&pool).await.unwrap();
    let installed: Vec<(i64, bool, Vec<u8>)> = sqlx::query_as(
        "SELECT version, success, checksum FROM mnesis._sqlx_migrations ORDER BY version",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let expected: Vec<_> = sqlx::migrate!("./migrations")
        .iter()
        .map(|migration| (migration.version, true, migration.checksum.to_vec()))
        .collect();
    assert_eq!(installed, expected);
    sqlx::query(
        "INSERT INTO mnesis.events (stream_id, version, event_type, schema_version, payload) \
        VALUES ($1, 1, 'Repeat', 1, 'data'::bytea)",
    )
    .bind(support::key(&pool, b"repeat").await)
    .execute(&pool)
    .await
    .unwrap();
    PostgresStore::migrate(&pool).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM mnesis.events")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn legacy_adoption_preserves_payload_metadata_and_checkpoint() {
    let Some(pool) = clean().await else { return };
    let assigned = legacy(&pool).await;
    PostgresStore::migrate(&pool).await.unwrap();
    let stored: (i64, String) = sqlx::query_as("SELECT global_seq, txid::text FROM mnesis.events")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, assigned);
    let body: (Vec<u8>, Vec<u8>, i64, Option<Vec<u8>>) =
        sqlx::query_as("SELECT keys.value, payload, schema_version, metadata FROM mnesis.events JOIN mnesis.keys ON keys.number = events.stream_id")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        body,
        (
            b"legacy".to_vec(),
            b"data".to_vec(),
            4_294_967_295,
            Some(b"meta".to_vec())
        )
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT relkind::text FROM pg_class WHERE oid = 'public.events'::regclass",
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        "v"
    );
    let store = PostgresStore::from_pool(pool.clone()).await.unwrap();
    let rows: Vec<_> = store
        .read_all(None)
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].0,
        PgAllPos::new(stored.1.parse().unwrap(), u64::try_from(stored.0).unwrap())
    );
    assert_eq!(rows[0].2.payload(), b"data");
    assert!(
        store
            .read_all(Some(rows[0].0))
            .await
            .unwrap()
            .next()
            .await
            .is_none()
    );
    let next = store
        .append(
            &StreamKey::from_bytes("new"),
            None,
            PendingBatch::new(&[event()]).unwrap(),
        )
        .await
        .unwrap();
    assert!(next.seq() > rows[0].0.seq());
}

#[tokio::test]
async fn concurrent_migrations_apply_one_record_and_release_locks() {
    let Some(pool) = clean().await else { return };
    let mut held = pool.acquire().await.unwrap();
    held.close_on_drop();
    held.lock().await.unwrap();
    let barrier = Arc::new(Barrier::new(3));
    let first = {
        let backing = pool.clone();
        let start = Arc::clone(&barrier);
        tokio::spawn(async move {
            start.wait().await;
            PostgresStore::migrate(&backing).await
        })
    };
    let second = {
        let backing = pool.clone();
        let start = Arc::clone(&barrier);
        tokio::spawn(async move {
            start.wait().await;
            PostgresStore::migrate(&backing).await
        })
    };
    barrier.wait().await;
    timeout(Duration::from_secs(3), async {
        loop {
            let waiting: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_locks \
                WHERE locktype = 'advisory' AND NOT granted AND database = \
                (SELECT oid FROM pg_database WHERE datname = current_database())",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            if waiting == 2 {
                break;
            }
            yield_now().await;
        }
    })
    .await
    .expect("both migrations must contend on SQLx's native lock");
    held.unlock().await.unwrap();
    first.await.unwrap().unwrap();
    second.await.unwrap().unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM mnesis._sqlx_migrations")
            .fetch_one(&pool)
            .await
            .unwrap(),
        i64::try_from(sqlx::migrate!("./migrations").iter().count()).unwrap()
    );
    unlocked(&pool).await;
}

async fn unlocked(pool: &PgPool) {
    timeout(Duration::from_secs(3), async {
        loop {
            let locks: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_locks \
                WHERE locktype = 'advisory' AND database = (SELECT oid FROM pg_database WHERE datname = current_database())")
                .fetch_one(pool).await.unwrap();
            if locks == 0 { break; }
            yield_now().await;
        }
    }).await.expect("scoped migration sessions must release all reentrant locks");
}

#[tokio::test]
async fn failed_legacy_move_rolls_back_and_can_be_repaired() {
    let Some(pool) = clean().await else { return };
    let assigned = legacy(&pool).await;
    sqlx::raw_sql("CREATE SCHEMA mnesis; CREATE TABLE mnesis.events_watermark_idx (collision int)")
        .execute(&pool)
        .await
        .unwrap();
    let error = PostgresStore::migrate(&pool).await.unwrap_err();
    assert!(
        matches!(error, PostgresError::Migration(MigrateError::ExecuteMigration(
        sqlx::Error::Database(ref source), 1)) if source.code().as_deref() == Some("42P07"))
    );
    assert_eq!(
        sqlx::query_as::<_, (i64, String)>("SELECT global_seq, txid::text FROM public.events")
            .fetch_one(&pool)
            .await
            .unwrap(),
        assigned
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM mnesis._sqlx_migrations")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    sqlx::query("DROP TABLE mnesis.events_watermark_idx")
        .execute(&pool)
        .await
        .unwrap();
    timeout(Duration::from_secs(3), PostgresStore::migrate(&pool))
        .await
        .expect("failure must not leak the migration lock")
        .unwrap();
    assert_eq!(
        sqlx::query_as::<_, (i64, String)>("SELECT global_seq, txid::text FROM mnesis.events")
            .fetch_one(&pool)
            .await
            .unwrap(),
        assigned
    );
}

#[tokio::test]
async fn checksum_is_rejected_without_repair() {
    let Some(pool) = clean().await else { return };
    PostgresStore::migrate(&pool).await.unwrap();
    let checksum: Vec<u8> =
        sqlx::query_scalar("SELECT checksum FROM mnesis._sqlx_migrations WHERE version = 1")
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("UPDATE mnesis._sqlx_migrations SET checksum = 'wrong'::bytea WHERE version = 1")
        .execute(&pool)
        .await
        .unwrap();
    let opened = PostgresStore::from_pool(pool.clone()).await;
    let migrated = PostgresStore::migrate(&pool).await;
    let unchanged = sqlx::query_scalar::<_, Vec<u8>>(
        "SELECT checksum FROM mnesis._sqlx_migrations WHERE version = 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE mnesis._sqlx_migrations SET checksum = $1 WHERE version = 1")
        .bind(checksum)
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(opened, Err(PostgresError::Schema { .. })));
    assert!(matches!(
        migrated,
        Err(PostgresError::Migration(MigrateError::VersionMismatch(1)))
    ));
    assert_eq!(unchanged, b"wrong");
}

#[tokio::test]
async fn newer_schema_is_rejected_without_repair() {
    let Some(pool) = clean().await else { return };
    PostgresStore::migrate(&pool).await.unwrap();
    let (count, latest): (i64, i64) =
        sqlx::query_as("SELECT count(*), max(version) FROM mnesis._sqlx_migrations")
            .fetch_one(&pool)
            .await
            .unwrap();
    let future = latest.checked_add(1).unwrap();
    sqlx::query("INSERT INTO mnesis._sqlx_migrations (version, description, success, checksum, execution_time) \
        VALUES ($1, 'future', true, 'future'::bytea, 0)").bind(future).execute(&pool).await.unwrap();
    let opened = PostgresStore::from_pool(pool.clone()).await;
    let migrated = PostgresStore::migrate(&pool).await;
    let unchanged = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM mnesis._sqlx_migrations")
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM mnesis._sqlx_migrations WHERE version = $1")
        .bind(future)
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(opened, Err(PostgresError::Schema { .. })));
    assert!(matches!(
        migrated,
        Err(PostgresError::Migration(MigrateError::VersionMissing(found))) if found == future
    ));
    assert_eq!(unchanged, count.checked_add(1).unwrap());
}

fn event() -> PendingEnvelope {
    pending_envelope(Version::INITIAL)
        .event_type("Scoped")
        .payload(b"right".as_slice())
        .build()
        .unwrap()
}

#[tokio::test]
async fn caller_search_path_cannot_redirect_event_storage() {
    let Some(pool) = clean().await else { return };
    PostgresStore::migrate(&pool).await.unwrap();
    sqlx::raw_sql(
        "CREATE SCHEMA shadow; CREATE TABLE shadow.events (LIKE mnesis.events INCLUDING ALL)",
    )
    .execute(&pool)
    .await
    .unwrap();
    let options = pool
        .connect_options()
        .as_ref()
        .clone()
        .options([("search_path", "shadow,public")]);
    let backing = PgPoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
        .unwrap();
    let store = PostgresStore::from_pool(backing).await.unwrap();
    store
        .append(
            &StreamKey::from_bytes("scope"),
            None,
            PendingBatch::new(&[event()]).unwrap(),
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
    assert_eq!(rows[0].2.payload(), b"right");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM shadow.events")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM mnesis.events")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn runtime_role_reads_and_appends_without_schema_creation_rights() {
    let Some(pool) = clean().await else { return };
    PostgresStore::migrate(&pool).await.unwrap();
    sqlx::raw_sql(
        "DROP ROLE IF EXISTS mnesis_pg4_runtime; CREATE ROLE mnesis_pg4_runtime LOGIN; \
        GRANT USAGE ON SCHEMA mnesis TO mnesis_pg4_runtime; \
        GRANT SELECT ON mnesis._sqlx_migrations TO mnesis_pg4_runtime; \
        GRANT SELECT, INSERT ON mnesis.keys TO mnesis_pg4_runtime; \
        GRANT USAGE ON SEQUENCE mnesis.keys_number_seq TO mnesis_pg4_runtime; \
        GRANT SELECT, INSERT ON mnesis.events TO mnesis_pg4_runtime; \
        GRANT USAGE ON SEQUENCE mnesis.events_global_seq_seq TO mnesis_pg4_runtime",
    )
    .execute(&pool)
    .await
    .unwrap();
    let options = pool
        .connect_options()
        .as_ref()
        .clone()
        .username("mnesis_pg4_runtime");
    let runtime = PgPoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
        .unwrap();
    let rights: (bool, bool) = sqlx::query_as(
        "SELECT has_schema_privilege(current_user, 'mnesis', 'CREATE'), \
        has_database_privilege(current_user, current_database(), 'CREATE')",
    )
    .fetch_one(&runtime)
    .await
    .unwrap();
    assert_eq!(rights, (false, false));
    let store = PostgresStore::from_pool(runtime.clone()).await.unwrap();
    store
        .append(
            &StreamKey::from_bytes("role"),
            None,
            PendingBatch::new(&[event()]).unwrap(),
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
    assert_eq!(rows[0].2.payload(), b"right");
    drop(store);
    runtime.close().await;
    sqlx::raw_sql("DROP OWNED BY mnesis_pg4_runtime; DROP ROLE mnesis_pg4_runtime")
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn adoption_preserves_empty_metadata_as_corruption_and_null_as_absence() {
    let Some(pool) = clean().await else { return };
    legacy(&pool).await;
    sqlx::raw_sql("UPDATE public.events SET metadata = ''::bytea; \
        INSERT INTO public.events (stream_id, version, event_type, schema_version, payload, metadata) \
        VALUES ('legacy'::bytea, 2, 'Legacy', 1, 'next'::bytea, NULL)")
        .execute(&pool).await.unwrap();
    PostgresStore::migrate(&pool).await.unwrap();
    let metadata: Vec<Option<Vec<u8>>> =
        sqlx::query_scalar("SELECT metadata FROM mnesis.events ORDER BY version")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(metadata, [Some(Vec::new()), None]);
    let store = PostgresStore::from_pool(pool).await.unwrap();
    let key = StreamKey::from_bytes("legacy");
    let mut all = store.read_stream(&key, Version::INITIAL).await.unwrap();
    assert!(matches!(
        all.next().await,
        Some(Err(PostgresError::CorruptRow { .. }))
    ));
    assert!(
        all.next().await.is_none(),
        "corruption must terminate without skipping"
    );
    let rows: Vec<_> = store
        .read_stream(&key, Version::new(2).unwrap())
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].metadata(), None);
    assert_eq!(rows[0].payload(), b"next");
}

#[tokio::test]
async fn cancelling_a_blocked_migration_releases_its_session_and_transaction() {
    let Some(pool) = clean().await else { return };
    let assigned = legacy(&pool).await;
    let mut held = pool.begin().await.unwrap();
    sqlx::query("LOCK TABLE public.events IN ACCESS SHARE MODE")
        .execute(&mut *held)
        .await
        .unwrap();
    let backing = pool.clone();
    let task = tokio::spawn(async move { PostgresStore::migrate(&backing).await });
    timeout(Duration::from_secs(3), async {
        loop {
            let waiting: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_locks l \
                WHERE NOT l.granted AND l.mode = 'AccessExclusiveLock' \
                AND l.relation = 'public.events'::regclass AND l.database = \
                (SELECT oid FROM pg_database WHERE datname = current_database())",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            if waiting == 1 {
                break;
            }
            yield_now().await;
        }
    })
    .await
    .expect("migration must actually block on the legacy table");
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    held.rollback().await.unwrap();
    unlocked(&pool).await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM mnesis._sqlx_migrations")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_as::<_, (i64, String)>("SELECT global_seq, txid::text FROM public.events")
            .fetch_one(&pool)
            .await
            .unwrap(),
        assigned
    );
    timeout(Duration::from_secs(3), PostgresStore::migrate(&pool))
        .await
        .expect("cancelled migration must release its session lock")
        .unwrap();
    assert_eq!(
        sqlx::query_as::<_, (i64, String)>("SELECT global_seq, txid::text FROM mnesis.events")
            .fetch_one(&pool)
            .await
            .unwrap(),
        assigned
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM mnesis._sqlx_migrations")
            .fetch_one(&pool)
            .await
            .unwrap(),
        i64::try_from(sqlx::migrate!("./migrations").iter().count()).unwrap()
    );
}

#[tokio::test]
async fn opening_a_prepared_database_does_not_require_writes() {
    let Some(control) = clean().await else { return };
    PostgresStore::migrate(&control).await.unwrap();
    let options = control
        .connect_options()
        .as_ref()
        .clone()
        .options([("default_transaction_read_only", "on")]);
    let readonly = PgPoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
        .unwrap();
    let reader = PostgresStore::from_pool(readonly)
        .await
        .expect("opening a prepared schema must not execute DDL");
    assert!(reader.read_all(None).await.unwrap().next().await.is_none());
}

#[tokio::test]
async fn shipped_sql_rejects_legacy_mismatch_without_rust_preflight() {
    let Some(pool) = clean().await else { return };
    legacy(&pool).await;
    sqlx::raw_sql("ALTER TABLE public.events DROP CONSTRAINT events_stream_id_version_key; CREATE SCHEMA mnesis")
        .execute(&pool).await.unwrap();
    let mut transaction = pool.begin().await.unwrap();
    let result = sqlx::raw_sql(include_str!("../migrations/1_events.sql"))
        .execute(&mut *transaction)
        .await;
    transaction.rollback().await.unwrap();
    let unchanged: (i64, Vec<u8>) = sqlx::query_as("SELECT version, payload FROM public.events")
        .fetch_one(&pool)
        .await
        .unwrap();
    // Restore the fixture before assertions so failure cannot poison later laws.
    sqlx::query("ALTER TABLE public.events ADD CONSTRAINT events_stream_id_version_key UNIQUE (stream_id, version)")
        .execute(&pool).await.unwrap();
    PostgresStore::migrate(&pool).await.unwrap();
    assert_eq!(unchanged, (1, b"data".to_vec()));
    assert!(
        matches!(result, Err(sqlx::Error::Database(ref source))
        if source.code().as_deref() == Some("55000")),
        "the shipped migration must reject an incompatible legacy table itself: {result:?}"
    );
}
