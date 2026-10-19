//! Independent-cluster restore characterization using native PostgreSQL tools.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "native restore assertions"
)]

#[cfg(all(feature = "projection", feature = "snapshot"))]
use std::io::{Write, stderr};
use std::process::Command;
#[cfg(all(feature = "projection", feature = "snapshot"))]
use std::process::Output;
use std::sync::Arc;
#[cfg(all(feature = "projection", feature = "snapshot"))]
use std::time::Instant;

#[path = "support/cluster.rs"]
mod cluster;

#[cfg(all(feature = "projection", feature = "snapshot"))]
#[path = "support/consumer.rs"]
mod consumer;
#[cfg(all(feature = "projection", feature = "snapshot"))]
#[path = "support/projection.rs"]
mod projection;

#[cfg(feature = "projection")]
use std::num::NonZeroU32;
#[cfg(feature = "projection")]
use std::num::NonZeroU64;

use futures::TryStreamExt;
use mnesis::Version;
use mnesis_postgres::{History, PgAllPos, PostgresError, PostgresStore};
#[cfg(all(feature = "projection", feature = "snapshot"))]
use mnesis_store::checkpoint::CheckpointHydrated;
#[cfg(feature = "projection")]
use mnesis_store::checkpoint::{CheckpointError, CheckpointMode, CheckpointStore, CheckpointWrite};
#[cfg(all(feature = "projection", feature = "snapshot"))]
use mnesis_store::state::{Hydrated, SnapshotStore};
use mnesis_store::{PendingBatch, RawEventStore, StreamKey, pending_envelope};
#[cfg(all(feature = "projection", feature = "snapshot"))]
use mnesis_store::{Repository, Store};
#[cfg(all(feature = "projection", feature = "snapshot"))]
use mnesis_test_domains::{Counter as AggregateCounter, CounterEvent, TestId};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tokio::sync::Barrier;

use cluster::Cluster;

impl Cluster {
    fn start() -> Self {
        let cluster = Self::initialize();
        cluster.boot();
        cluster
    }

    fn boot(&self) {
        let options = format!("-h '' -k {} -p 55436", self.socket().display());
        self.launch(&options);
    }

    #[cfg(all(feature = "projection", feature = "snapshot"))]
    fn copy_physical(&self) -> Self {
        let target = Self::new();
        let copied = Command::new("pg_basebackup")
            .args(["--checkpoint=fast", "--wal-method=stream", "--dbname"])
            .arg(self.url())
            .arg("-D")
            .arg(target.data())
            .output()
            .unwrap();
        assert!(copied.status.success(), "{copied:?}");
        let verified = target.verify();
        assert!(verified.status.success(), "{verified:?}");
        target
    }

    #[cfg(all(feature = "projection", feature = "snapshot"))]
    fn halt(&self) {
        let stopped = self.stop().unwrap();
        assert!(stopped.status.success(), "{stopped:?}");
        let status = Command::new("pg_ctl")
            .arg("-D")
            .arg(self.data())
            .arg("status")
            .output()
            .unwrap();
        assert_eq!(status.status.code(), Some(3), "{status:?}");
    }

    #[cfg(all(feature = "projection", feature = "snapshot"))]
    fn verify(&self) -> Output {
        Command::new("pg_verifybackup")
            .arg("--format=plain")
            .arg(self.data())
            .output()
            .unwrap()
    }

    fn url(&self) -> String {
        format!(
            "postgres:///postgres?host={}&port=55436&user=mnesis",
            self.socket().display()
        )
    }

    fn copy_into(&self, target: &Self) {
        let dump = self.socket().join("backup.sql");
        let dumped = Command::new("pg_dump")
            .args([
                "--no-owner",
                "--no-privileges",
                "--data-only",
                "--exclude-table-data=mnesis._sqlx_migrations",
                "--exclude-table-data=mnesis.history",
                "--schema=mnesis",
                "--file",
            ])
            .arg(&dump)
            .arg("--dbname")
            .arg(self.url())
            .output()
            .unwrap();
        assert!(dumped.status.success(), "{dumped:?}");
        let restored = Command::new("psql")
            .args(["--no-psqlrc", "--set", "ON_ERROR_STOP=1", "--dbname"])
            .arg(target.url())
            .arg("--file")
            .arg(&dump)
            .output()
            .unwrap();
        assert!(restored.status.success(), "{restored:?}");
    }
}

async fn context(pool: &PgPool) -> (i64, i64) {
    let mut transaction = pool.begin().await.unwrap();
    sqlx::raw_sql("CREATE ROLE mnesis_history_probe NOLOGIN; SET LOCAL ROLE mnesis_history_probe")
        .execute(&mut *transaction)
        .await
        .unwrap();
    let identity = sqlx::query_as("SELECT system_identifier, d.oid::bigint FROM pg_catalog.pg_control_system() CROSS JOIN pg_catalog.pg_database d WHERE d.datname = pg_catalog.current_database()")
        .fetch_one(&mut *transaction)
        .await
        .unwrap();
    transaction.rollback().await.unwrap();
    identity
}

async fn migrated(cluster: &Cluster) -> (PostgresStore, PgPool) {
    let pool = PgPoolOptions::new()
        .max_connections(6)
        .connect(&cluster.url())
        .await
        .unwrap();
    PostgresStore::migrate(&pool).await.unwrap();
    let store = PostgresStore::from_pool(pool.clone()).await.unwrap();
    (store, pool)
}

async fn append(store: &PostgresStore) -> PgAllPos {
    let event = pending_envelope(Version::INITIAL)
        .event_type("Saved")
        .payload(b"saved payload".as_slice())
        .build()
        .unwrap();
    store
        .append(
            &StreamKey::from_bytes("saved"),
            None,
            PendingBatch::of(&event),
        )
        .await
        .unwrap()
}

#[cfg(feature = "projection")]
const fn checkpoint(
    position: PgAllPos,
    state: &Vec<u8>,
    expected: Option<NonZeroU64>,
) -> CheckpointWrite<'_, Vec<u8>, PgAllPos> {
    CheckpointWrite {
        expected,
        schema_version: NonZeroU32::MIN,
        position,
        state,
        mode: CheckpointMode::Advance,
    }
}

#[tokio::test]
async fn copied_future_transactions_reject_before_global_history_can_be_used() {
    if std::env::var_os("DATABASE_URL").is_none() {
        return;
    }
    let source = Cluster::start();
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&source.url())
        .await
        .unwrap();
    PostgresStore::migrate(&pool).await.unwrap();
    for _ in 0..128 {
        let _: String = sqlx::query_scalar("SELECT pg_current_xact_id()::text")
            .fetch_one(&pool)
            .await
            .unwrap();
    }
    let store = PostgresStore::from_pool(pool.clone()).await.unwrap();
    let id = StreamKey::from_bytes("restored-history");
    let event = pending_envelope(Version::INITIAL)
        .event_type("Restored")
        .payload(b"original".as_slice())
        .build()
        .unwrap();
    let position = store
        .append(&id, None, PendingBatch::of(&event))
        .await
        .unwrap();
    let target = Cluster::start();
    let target_pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&target.url())
        .await
        .unwrap();
    PostgresStore::migrate(&target_pool).await.unwrap();
    source.copy_into(&target);
    let opened = PostgresStore::from_pool(target_pool.clone()).await;
    let source_context = context(&pool).await;
    let target_context = context(&target_pool).await;
    assert_ne!(source_context.0, target_context.0);
    assert_eq!(source_context.1, target_context.1);
    let payloads: Vec<Vec<u8>> = sqlx::query_scalar("SELECT payload FROM mnesis.events")
        .fetch_all(&target_pool)
        .await
        .unwrap();
    let target_xid: String = sqlx::query_scalar("SELECT pg_current_xact_id()::text")
        .fetch_one(&target_pool)
        .await
        .unwrap();
    assert_eq!(payloads, [b"original".to_vec()]);
    assert!(position.txid() > target_xid.parse::<u64>().unwrap());
    assert!(matches!(opened, Err(PostgresError::FutureTransaction)));
    assert_future_boundary(&target_pool).await;
    drop(store);
    drop(target_pool);
    drop(pool);
    drop(target);
    drop(source);
}

async fn assert_future_boundary(pool: &PgPool) {
    sqlx::query("UPDATE mnesis.events SET txid = (pg_catalog.pg_current_xact_id()::text::numeric + 1)::text::xid8")
        .execute(pool).await.unwrap();
    let boundary: (String, String) = sqlx::query_as("SELECT txid::text, pg_catalog.pg_snapshot_xmax(pg_catalog.pg_current_snapshot())::text FROM mnesis.events")
        .fetch_one(pool).await.unwrap();
    assert_eq!(boundary.0, boundary.1);
    assert!(matches!(
        PostgresStore::from_pool(pool.clone()).await,
        Err(PostgresError::FutureTransaction)
    ));
}

#[tokio::test]
async fn foreign_resume_and_checkpoint_positions_reject_before_pool_acquisition() {
    if std::env::var_os("DATABASE_URL").is_none() {
        return;
    }
    let cluster = Cluster::start();
    let (store, pool) = migrated(&cluster).await;
    let position = append(&store).await;
    assert_eq!(position.history(), store.history());
    assert_eq!(
        History::new(*position.history().as_bytes()),
        store.history()
    );
    drop(store);
    sqlx::raw_sql("DROP SCHEMA mnesis CASCADE")
        .execute(&pool)
        .await
        .unwrap();
    PostgresStore::migrate(&pool).await.unwrap();
    let restored = PostgresStore::from_pool(pool.clone()).await.unwrap();
    let expected = restored.history();
    assert_ne!(expected, position.history());
    pool.close().await;
    assert!(matches!(restored.read_all(Some(position)).await,
        Err(PostgresError::HistoryMismatch { expected: actual_expected, actual })
        if actual_expected == expected && actual == position.history()));
    #[cfg(feature = "projection")]
    {
        let state = vec![1];
        let write = checkpoint(position, &state, None);
        assert!(
            matches!(restored.commit_checkpoint(&StreamKey::from_bytes("projection"), write).await,
            Err(CheckpointError::Store(PostgresError::HistoryMismatch { expected: actual_expected, actual }))
            if actual_expected == expected && actual == position.history())
        );
    }
}

#[tokio::test]
async fn copied_native_database_bindings_reject_without_repair() {
    if std::env::var_os("DATABASE_URL").is_none() {
        return;
    }
    let source = Cluster::start();
    let target = Cluster::start();
    let (_store, source_pool) = migrated(&source).await;
    let (store, pool) = migrated(&target).await;
    let source_context = context(&source_pool).await;
    let original = store.history();
    drop(store);
    sqlx::query("UPDATE mnesis.history SET system_id = $1")
        .bind(source_context.0)
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        PostgresStore::from_pool(pool.clone()).await,
        Err(PostgresError::DatabaseHistory)
    ));
    let unchanged: [u8; 16] = sqlx::query_scalar("SELECT value FROM mnesis.history")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(&unchanged, original.as_bytes());
    sqlx::query("UPDATE mnesis.history SET system_id = (SELECT system_identifier FROM pg_catalog.pg_control_system()), database_id = (SELECT oid FROM pg_catalog.pg_database WHERE datname = 'template1')")
        .execute(&pool).await.unwrap();
    assert!(matches!(
        PostgresStore::from_pool(pool.clone()).await,
        Err(PostgresError::DatabaseHistory)
    ));
}

#[tokio::test]
async fn committed_transactions_above_an_unrelated_xmin_still_open() {
    if std::env::var_os("DATABASE_URL").is_none() {
        return;
    }
    let cluster = Cluster::start();
    let (store, pool) = migrated(&cluster).await;
    let mut held = pool.begin().await.unwrap();
    let held_xid: String = sqlx::query_scalar("SELECT pg_current_xact_id()::text")
        .fetch_one(&mut *held)
        .await
        .unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let writer = store.clone();
    let ready = Arc::clone(&barrier);
    let task = tokio::spawn(async move {
        ready.wait().await;
        append(&writer).await
    });
    barrier.wait().await;
    let position = task.await.unwrap();
    assert!(position.txid() > held_xid.parse::<u64>().unwrap());
    let opened = PostgresStore::from_pool(pool.clone()).await;
    held.rollback().await.unwrap();
    let reopened = opened.unwrap();
    assert_eq!(reopened.history(), store.history());
    let rows: Vec<_> = reopened
        .read_all(None)
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, position);
}

#[cfg(all(feature = "projection", feature = "snapshot"))]
#[tokio::test]
async fn physical_backup_preserves_history_state_and_metadata_and_can_advance() {
    if std::env::var_os("DATABASE_URL").is_none() {
        return;
    }
    let source = Cluster::start();
    let (store, pool) = migrated(&source).await;
    let flush: String = sqlx::query_scalar("SHOW fsync")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(flush, "on");
    let position = append(&store).await;
    let id = StreamKey::from_bytes("projection");
    let state = b"saved projection state".to_vec();
    let revision = store
        .commit_checkpoint(&id, checkpoint(position, &state, None))
        .await
        .unwrap();
    store
        .commit(
            &id,
            NonZeroU32::MIN,
            Version::INITIAL,
            &b"saved cache".to_vec(),
        )
        .await
        .unwrap();
    drop(store);
    pool.close().await;
    let target = source.copy_physical();
    source.halt();
    drop(source);
    target.boot();
    let restored_pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&target.url())
        .await
        .unwrap();
    let restored = PostgresStore::from_pool(restored_pool.clone())
        .await
        .unwrap();
    assert_physical_pair(&restored, position, revision).await;
    let next_event = pending_envelope(Version::new(2).unwrap())
        .event_type("Next")
        .payload(b"next".as_slice())
        .build()
        .unwrap();
    let next = restored
        .append(
            &StreamKey::from_bytes("saved"),
            Some(Version::INITIAL),
            PendingBatch::of(&next_event),
        )
        .await
        .unwrap();
    assert!(next > position);
    assert_eq!(next.history(), position.history());
    let resumed: Vec<_> = restored
        .read_all(Some(position))
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(resumed.len(), 1);
    assert_eq!(resumed[0].0, next);
    assert_eq!(resumed[0].2.version(), Version::new(2).unwrap());
    let next_revision = restored
        .commit_checkpoint(
            &id,
            checkpoint(next, &b"advanced projection".to_vec(), Some(revision)),
        )
        .await
        .unwrap();
    assert_eq!(next_revision.get(), revision.get().checked_add(1).unwrap());
}

#[cfg(all(feature = "projection", feature = "snapshot"))]
#[tokio::test]
async fn physical_operator_drill_restarts_the_saved_projection_and_advances() {
    if std::env::var_os("DATABASE_URL").is_none() {
        return;
    }
    let source = Cluster::start();
    let (raw, pool) = migrated(&source).await;
    let profile: (String, String) =
        sqlx::query_as("SELECT current_setting('fsync'), current_setting('synchronous_commit')")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(profile, ("on".to_owned(), "on".to_owned()));
    projection::events(&raw).await;
    let (saved, state) = consumer::resume(&raw).await;
    assert_eq!(state, 4);
    let metadata: Vec<(i64, Vec<u8>)> =
        sqlx::query_as("SELECT version, checksum FROM mnesis._sqlx_migrations ORDER BY version")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        metadata.iter().map(|row| row.0).collect::<Vec<_>>(),
        [1, 2, 3, 4, 5]
    );
    drop(raw);
    pool.close().await;
    let started = Instant::now();
    let target = source.copy_physical();
    let backup_time = started.elapsed();
    source.halt();
    let recovery = Instant::now();
    target.boot();
    let target_pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&target.url())
        .await
        .unwrap();
    let restored = PostgresStore::from_pool(target_pool.clone()).await.unwrap();
    let restored_metadata: Vec<(i64, Vec<u8>)> =
        sqlx::query_as("SELECT version, checksum FROM mnesis._sqlx_migrations ORDER BY version")
            .fetch_all(&target_pool)
            .await
            .unwrap();
    assert_eq!(restored_metadata, metadata);
    assert_eq!(restored.history(), saved.history());
    assert_eq!(consumer::resume(&restored).await, (saved, 4));
    let recovery_time = recovery.elapsed();
    let store = Store::new(restored.clone());
    let repository = store.repository::<AggregateCounter>().json().build();
    let mut root = repository.load(TestId::new("first")).await.unwrap();
    repository
        .save::<0>(&mut root, &mnesis::events![CounterEvent::Incremented])
        .await
        .unwrap();
    let (advanced, value) = consumer::resume(&restored).await;
    assert_eq!(value, 5);
    assert!(advanced > saved);
    assert_eq!(advanced.history(), saved.history());
    writeln!(stderr().lock(),
        "physical recovery fixture: backup={backup_time:?}, stopped-primary-to-reloaded-projection={recovery_time:?}, checkpoint={saved:?}, advanced={advanced:?}"
    ).unwrap();
    drop(repository);
    drop(store);
    drop(restored);
    target_pool.close().await;
}

#[cfg(all(feature = "projection", feature = "snapshot"))]
#[tokio::test]
async fn physical_manifest_rejects_corrupt_and_missing_files_before_boot() {
    if std::env::var_os("DATABASE_URL").is_none() {
        return;
    }
    let source = Cluster::start();
    let (store, pool) = migrated(&source).await;
    append(&store).await;
    drop(store);
    pool.close().await;
    let backup = source.copy_physical();
    let path = backup.data().join("PG_VERSION");
    let original = std::fs::read(&path).unwrap();
    let mut damaged = original.clone();
    *damaged.first_mut().unwrap() ^= 0xff;
    assert_eq!(damaged.len(), original.len());
    std::fs::write(&path, &damaged).unwrap();
    let corrupt = backup.verify();
    std::fs::remove_file(&path).unwrap();
    let missing = backup.verify();
    std::fs::write(&path, &original).unwrap();
    let repaired = backup.verify();
    assert_eq!(corrupt.status.code(), Some(1), "{corrupt:?}");
    assert_eq!(missing.status.code(), Some(1), "{missing:?}");
    assert_eq!(
        String::from_utf8_lossy(&corrupt.stderr),
        "pg_verifybackup: error: checksum mismatch for file \"PG_VERSION\"\n"
    );
    assert_eq!(
        String::from_utf8_lossy(&missing.stderr),
        "pg_verifybackup: error: \"PG_VERSION\" is present in the manifest but not on disk\n"
    );
    assert!(repaired.status.success(), "{repaired:?}");
    assert_ne!(corrupt.stderr, missing.stderr);
    writeln!(
        stderr().lock(),
        "corrupt backup verifier: {corrupt:?}; missing file verifier: {missing:?}"
    )
    .unwrap();
}

#[cfg(all(feature = "projection", feature = "snapshot"))]
async fn assert_physical_pair(restored: &PostgresStore, position: PgAllPos, revision: NonZeroU64) {
    let id = StreamKey::from_bytes("projection");
    assert_eq!(restored.history(), position.history());
    assert_eq!(
        restored
            .hydrate_checkpoint(&id, NonZeroU32::MIN)
            .await
            .unwrap(),
        CheckpointHydrated::Found {
            revision,
            position,
            state: b"saved projection state".to_vec()
        }
    );
    assert_eq!(
        restored.hydrate(&id, NonZeroU32::MIN).await.unwrap(),
        Hydrated::Found {
            position: Version::INITIAL,
            state: b"saved cache".to_vec()
        }
    );
    let rows: Vec<_> = restored
        .read_all(None)
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, position);
    assert_eq!(rows[0].1, StreamKey::from_bytes("saved"));
    assert_eq!(rows[0].2.payload(), b"saved payload");
}
