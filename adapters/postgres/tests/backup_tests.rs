//! Completed, bounded chunks compose the real Postgres and Fjall adapters.
#![cfg(all(feature = "export", feature = "import"))]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "native backup assertions"
)]

#[cfg(feature = "projection")]
#[path = "support/projection.rs"]
mod projection;

use std::collections::BTreeMap;
use std::io::Read;
use std::num::NonZeroU32;
use std::path::Path;
use std::time::Duration;

use futures::{FutureExt, StreamExt, TryStreamExt, stream};
use mnesis::Version;
use mnesis_fjall::{AllIndex, FjallStore};
#[cfg(feature = "projection")]
use mnesis_postgres::PgAllPos;
use mnesis_postgres::{Limits, PostgresError, PostgresStore};
use mnesis_store::cbor::{ChunkWriter, SectionError, WriteError, decode_chunk};
#[cfg(feature = "projection")]
use mnesis_store::checkpoint::{CheckpointHydrated, CheckpointStore};
use mnesis_store::export::{ConsistentExporter, ExportError, ExportSession, StreamLister};
use mnesis_store::import::{Atomicity, EventImporter};
#[cfg(feature = "projection")]
use mnesis_store::state::{AfterEventTypes, CodecSnapshotStore};
use mnesis_store::value::SchemaVersion;
use mnesis_store::{
    BatchSize, PendingBatch, PendingEnvelope, PersistedEnvelope, RawEventStore, StreamKey,
    pending_envelope,
};
#[cfg(feature = "projection")]
use mnesis_store::{DecodedStreamExt, JsonCodec, Projection, Repository, Store};
#[cfg(feature = "projection")]
use mnesis_test_domains::{Counter as AggregateCounter, CounterEvent, TestId};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tokio::task::yield_now;
use tokio::time::timeout;

const CAPACITY: usize = 16_384;

async fn fixture() -> Option<(PostgresStore, PgPool)> {
    let url = std::env::var("DATABASE_URL").ok()?;
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await
        .unwrap();
    sqlx::raw_sql("DROP SCHEMA IF EXISTS mnesis CASCADE; DROP VIEW IF EXISTS public.events")
        .execute(&pool)
        .await
        .unwrap();
    PostgresStore::migrate(&pool).await.unwrap();
    let limits = Limits {
        rows: BatchSize::new(1).unwrap(),
        bytes: NonZeroU32::new(4096).unwrap(),
    };
    let store = PostgresStore::from_pool_with_limits(pool.clone(), limits)
        .await
        .unwrap();
    Some((store, pool))
}

fn event(index: u8, number: u64) -> PendingEnvelope {
    let schema = if number.is_multiple_of(2) {
        NonZeroU32::MAX
    } else {
        NonZeroU32::MIN
    };
    let payload = if index == 0 {
        vec![index; 3000]
    } else {
        vec![index, 0, 255, index]
    };
    let builder = pending_envelope(Version::new(number).unwrap())
        .event_type("E\0漢")
        .payload(payload)
        .schema_version(SchemaVersion::new(schema));
    if number.is_multiple_of(2) {
        builder.metadata(vec![0, 255, index]).build().unwrap()
    } else {
        builder.build().unwrap()
    }
}

fn key(index: u8) -> StreamKey {
    StreamKey::from_slice(&[index, 0, 255])
}

async fn populate(store: &PostgresStore) {
    for index in 0..16 {
        let last = if index == 0 { 17 } else { 3 };
        for number in 1..=last {
            let pending = event(index, number);
            store
                .append(
                    &key(index),
                    Version::new(number.checked_sub(1).unwrap()),
                    PendingBatch::of(&pending),
                )
                .await
                .unwrap();
        }
    }
}

async fn view_pid(pool: &PgPool) -> i32 {
    sqlx::query_scalar("SELECT pid FROM pg_catalog.pg_stat_activity WHERE datname = pg_catalog.current_database() AND state = 'idle in transaction' AND backend_xmin IS NOT NULL")
        .fetch_one(pool).await.unwrap()
}

async fn waiting(pool: &PgPool, pid: i32) -> Result<(), tokio::time::error::Elapsed> {
    timeout(Duration::from_secs(3), async {
        loop {
            let blocked: bool = sqlx::query_scalar("SELECT EXISTS (SELECT FROM pg_catalog.pg_stat_activity WHERE pid = $1 AND wait_event_type = 'Lock')")
                .bind(pid).fetch_one(pool).await.unwrap();
            if blocked { return; }
            yield_now().await;
        }
    }).await
}

#[derive(Debug, PartialEq, Eq)]
struct Saved {
    version: Version,
    schema: u32,
    kind: String,
    payload: Vec<u8>,
    metadata: Option<Vec<u8>>,
}

impl From<PersistedEnvelope> for Saved {
    fn from(event: PersistedEnvelope) -> Self {
        Self {
            version: event.version(),
            schema: event.schema_version(),
            kind: event.event_type().to_owned(),
            payload: event.payload().to_vec(),
            metadata: event.metadata().map(<[u8]>::to_vec),
        }
    }
}

async fn contents(store: &impl StreamLister) -> BTreeMap<Vec<u8>, Vec<Saved>> {
    let listing = store.list_streams().await.unwrap();
    futures::pin_mut!(listing);
    let mut saved = BTreeMap::new();
    while let Some(id) = listing.try_next().await.unwrap() {
        let rows = store
            .read_stream(&id, Version::INITIAL)
            .await
            .unwrap()
            .map_ok(Saved::from)
            .try_collect()
            .await
            .unwrap();
        saved.insert(id.as_bytes().to_vec(), rows);
    }
    saved
}

async fn backup(source: &impl ConsistentExporter, directory: &Path) -> usize {
    let session = source
        .open_export_session(Duration::from_secs(60))
        .await
        .unwrap();
    let mut buffer = vec![0; CAPACITY];
    let mut count = 0_usize;
    {
        let listing = session.list_streams().await.unwrap();
        futures::pin_mut!(listing);
        while let Some(id) = listing.try_next().await.unwrap() {
            let events = session.export_stream(&id, Version::INITIAL).await.unwrap();
            let batches = events.chunks(2);
            futures::pin_mut!(batches);
            while let Some(batch) = batches.next().await {
                let mut writer =
                    ChunkWriter::new(buffer.as_mut_slice(), Some(b"backup-law")).unwrap();
                writer
                    .section(id.as_bytes())
                    .unwrap()
                    .try_extend(stream::iter(batch))
                    .await
                    .unwrap();
                let remaining = writer.finish().unwrap().len();
                let used = CAPACITY.checked_sub(remaining).unwrap();
                std::fs::write(directory.join(format!("{count}.nxch")), &buffer[..used]).unwrap();
                count = count.checked_add(1).unwrap();
            }
        }
    }
    session.close().await.unwrap();
    if count == 0 {
        let remaining = ChunkWriter::new(buffer.as_mut_slice(), Some(b"backup-law"))
            .unwrap()
            .finish()
            .unwrap()
            .len();
        let used = CAPACITY.checked_sub(remaining).unwrap();
        std::fs::write(directory.join("0.nxch"), &buffer[..used]).unwrap();
        return 1;
    }
    count
}

fn read_chunk(path: &Path) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(CAPACITY);
    std::fs::File::open(path)
        .unwrap()
        .take(u64::try_from(CAPACITY.checked_add(1).unwrap()).unwrap())
        .read_to_end(&mut bytes)
        .unwrap();
    assert!(
        bytes.len() <= CAPACITY,
        "reject oversized artifact before decoding"
    );
    bytes
}

async fn restore(store: &impl EventImporter, directory: &Path, count: usize, policy: Atomicity) {
    for index in 0..count {
        let bytes = read_chunk(&directory.join(format!("{index}.nxch")));
        let sections = decode_chunk(&bytes).unwrap();
        let report = store
            .import(&sections, StreamKey::from_slice, policy)
            .await
            .unwrap();
        assert_eq!(report.unfinished().count(), 0);
    }
}

#[tokio::test]
async fn completed_chunks_restore_postgres_with_new_history_and_continuations() {
    let Some((source, pool)) = fixture().await else {
        return;
    };
    populate(&source).await;
    let expected = contents(&source).await;
    let old = source
        .read_all(None)
        .await
        .unwrap()
        .next()
        .await
        .unwrap()
        .unwrap()
        .0;
    let directory = tempfile::tempdir().unwrap();
    let count = backup(&source, directory.path()).await;
    assert_eq!(count, 39);
    drop(source);
    pool.close().await;
    for policy in [Atomicity::WholeChunk, Atomicity::PerStream] {
        let (destination, target_pool) = fixture().await.unwrap();
        restore(&destination, directory.path(), count, policy).await;
        assert_eq!(contents(&destination).await, expected);
        assert!(matches!(destination.read_all(Some(old)).await,
            Err(PostgresError::HistoryMismatch { expected: current, actual })
            if current == destination.history() && actual == old.history()));
        let pending = event(0, 18);
        let position = destination
            .append(&key(0), Version::new(17), PendingBatch::of(&pending))
            .await
            .unwrap();
        assert_eq!(position.history(), destination.history());
        assert_eq!(
            destination
                .read_all(Some(position))
                .await
                .unwrap()
                .try_collect::<Vec<_>>()
                .await
                .unwrap()
                .len(),
            0
        );
        let rows = destination
            .read_stream(&key(0), Version::new(18).unwrap())
            .await
            .unwrap()
            .map_ok(Saved::from)
            .try_collect::<Vec<_>>()
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].version, Version::new(18).unwrap());
        drop(destination);
        target_pool.close().await;
    }
}

#[tokio::test]
async fn completed_chunks_interchange_through_fjall_reopen() {
    let Some((source, pool)) = fixture().await else {
        return;
    };
    populate(&source).await;
    let expected = contents(&source).await;
    let first = tempfile::tempdir().unwrap();
    let count = backup(&source, first.path()).await;
    drop(source);
    pool.close().await;
    let files = tempfile::tempdir().unwrap();
    let fjall = FjallStore::builder(files.path())
        .all_index(AllIndex::Disabled)
        .open()
        .unwrap();
    restore(&fjall, first.path(), count, Atomicity::WholeChunk).await;
    fjall.close().await.unwrap();
    let reopened = FjallStore::builder(files.path())
        .all_index(AllIndex::Disabled)
        .open()
        .unwrap();
    assert_eq!(contents(&reopened).await, expected);
    let second = tempfile::tempdir().unwrap();
    let returned = backup(&reopened, second.path()).await;
    assert_eq!(returned, count);
    reopened.close().await.unwrap();
    let (destination, target_pool) = fixture().await.unwrap();
    restore(&destination, second.path(), returned, Atomicity::PerStream).await;
    assert_eq!(contents(&destination).await, expected);
    drop(destination);
    target_pool.close().await;
}

#[tokio::test]
async fn empty_backup_has_one_completed_chunk_and_no_sections() {
    let Some((store, pool)) = fixture().await else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    assert_eq!(backup(&store, directory.path()).await, 1);
    let bytes = read_chunk(&directory.path().join("0.nxch"));
    assert_eq!(decode_chunk(&bytes).unwrap().len(), 0);
    for policy in [Atomicity::WholeChunk, Atomicity::PerStream] {
        restore(&store, directory.path(), 1, policy).await;
        assert_eq!(contents(&store).await, BTreeMap::new());
    }
    drop(store);
    pool.close().await;
}

#[tokio::test]
async fn every_incomplete_chunk_rejects_before_destination_writes() {
    let Some((source, pool)) = fixture().await else {
        return;
    };
    let pending = event(1, 1);
    source
        .append(&key(1), None, PendingBatch::of(&pending))
        .await
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    assert_eq!(backup(&source, directory.path()).await, 1);
    let bytes = read_chunk(&directory.path().join("0.nxch"));
    drop(source);
    pool.close().await;
    let (destination, target_pool) = fixture().await.unwrap();
    for policy in [Atomicity::WholeChunk, Atomicity::PerStream] {
        for cut in 0..bytes.len() {
            let decoded = decode_chunk(&bytes[..cut]);
            let rejected = decoded.is_err();
            if let Ok(sections) = decoded {
                destination
                    .import(&sections, StreamKey::from_slice, policy)
                    .await
                    .unwrap();
            }
            assert!(rejected, "accepted incomplete chunk at {cut}");
        }
        assert_eq!(contents(&destination).await, BTreeMap::new());
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM mnesis.keys")
                .fetch_one(&target_pool)
                .await
                .unwrap(),
            0
        );
    }
    drop(destination);
    target_pool.close().await;
}

#[tokio::test]
async fn native_source_failure_cannot_complete_a_chunk() {
    let Some((store, pool)) = fixture().await else {
        return;
    };
    let pending = event(1, 1);
    store
        .append(&key(1), None, PendingBatch::of(&pending))
        .await
        .unwrap();
    let session = store
        .open_export_session(Duration::from_secs(60))
        .await
        .unwrap();
    let pid = view_pid(&pool).await;
    let cursor = session
        .export_stream(&key(1), Version::INITIAL)
        .await
        .unwrap();
    let mut lock = pool.begin().await.unwrap();
    sqlx::query("LOCK TABLE mnesis.events IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *lock)
        .await
        .unwrap();
    let mut buffer = vec![0; CAPACITY];
    let mut writer = ChunkWriter::new(buffer.as_mut_slice(), None).unwrap();
    let failure = {
        let mut section = writer.section(key(1).as_bytes()).unwrap();
        let mut reading = Box::pin(section.try_extend(cursor));
        assert!(reading.as_mut().now_or_never().is_none());
        let blocked = waiting(&pool, pid).await;
        let terminated: bool = sqlx::query_scalar("SELECT pg_catalog.pg_terminate_backend($1)")
            .bind(pid)
            .fetch_one(&pool)
            .await
            .unwrap();
        let result = timeout(Duration::from_secs(3), reading).await;
        lock.rollback().await.unwrap();
        blocked.expect("the source query must actually be in flight");
        assert!(terminated);
        result.unwrap().unwrap_err()
    };
    assert!(
        matches!(failure, SectionError::Read(ExportError::Store(PostgresError::Sqlx(sqlx::Error::Database(source))))
        if source.code().as_deref() == Some("57P01"))
    );
    assert!(matches!(writer.finish(), Err(WriteError::Poisoned)));
    assert!(session.close().await.is_err());
    drop(store);
    pool.close().await;
}

#[tokio::test]
async fn canceling_a_native_drain_cannot_complete_a_chunk() {
    let Some((store, pool)) = fixture().await else {
        return;
    };
    let pending = event(1, 1);
    store
        .append(&key(1), None, PendingBatch::of(&pending))
        .await
        .unwrap();
    let session = store
        .open_export_session(Duration::from_secs(60))
        .await
        .unwrap();
    let pid = view_pid(&pool).await;
    let cursor = session
        .export_stream(&key(1), Version::INITIAL)
        .await
        .unwrap();
    let mut lock = pool.begin().await.unwrap();
    sqlx::query("LOCK TABLE mnesis.events IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *lock)
        .await
        .unwrap();
    let mut buffer = vec![0; CAPACITY];
    let mut writer = ChunkWriter::new(buffer.as_mut_slice(), None).unwrap();
    let blocked = {
        let mut section = writer.section(key(1).as_bytes()).unwrap();
        let mut reading = Box::pin(section.try_extend(cursor));
        assert!(reading.as_mut().now_or_never().is_none());
        let observed = waiting(&pool, pid).await;
        drop(reading);
        observed
    };
    drop(session);
    let released = timeout(Duration::from_secs(3), async {
        loop {
            let pinned: bool = sqlx::query_scalar("SELECT EXISTS (SELECT FROM pg_catalog.pg_stat_activity WHERE pid = $1 AND backend_xmin IS NOT NULL)")
                .bind(pid).fetch_one(&pool).await.unwrap();
            if !pinned { return; }
            yield_now().await;
        }
    }).await;
    lock.rollback().await.unwrap();
    blocked.expect("cancellation must interrupt a real blocked source query");
    released.expect("abandoning backup must release its native view");
    assert!(matches!(writer.finish(), Err(WriteError::Poisoned)));
    drop(store);
    pool.close().await;
}

#[cfg(feature = "projection")]
async fn project(store: &PostgresStore) -> (PgAllPos, i64) {
    let codec = CodecSnapshotStore::new(store, JsonCodec::default());
    let mut consumer = Projection::load(
        StreamKey::from_slice(b"restored-counter"),
        projection::Counter,
        AfterEventTypes::new(&["Incremented", "Set"]),
        &codec,
        NonZeroU32::MIN,
    )
    .await
    .unwrap();
    let events = store
        .read_all(consumer.checkpoint())
        .await
        .unwrap()
        .decoded::<CounterEvent, _>(JsonCodec::default());
    futures::pin_mut!(events);
    while let Some(item) = events.try_next().await.unwrap() {
        consumer.advance(item).await.unwrap();
    }
    (
        consumer.checkpoint().unwrap(),
        consumer.state().unwrap().value,
    )
}

#[cfg(feature = "projection")]
#[tokio::test]
async fn logical_restore_rebuilds_a_projection_then_restarts_and_advances_it() {
    let Some((source, pool)) = fixture().await else {
        return;
    };
    projection::events(&source).await;
    let (old, state) = project(&source).await;
    assert_eq!(state, 4);
    let directory = tempfile::tempdir().unwrap();
    let count = backup(&source, directory.path()).await;
    assert_eq!(count, 2);
    drop(source);
    pool.close().await;
    let (destination, target_pool) = fixture().await.unwrap();
    restore(&destination, directory.path(), count, Atomicity::WholeChunk).await;
    assert_eq!(
        destination
            .hydrate_checkpoint(&StreamKey::from_slice(b"restored-counter"), NonZeroU32::MIN)
            .await
            .unwrap(),
        CheckpointHydrated::Absent
    );
    let (rebuilt, rebuilt_state) = project(&destination).await;
    assert_eq!(rebuilt_state, 4);
    assert_eq!(rebuilt.history(), destination.history());
    assert_ne!(rebuilt.history(), old.history());
    let options = target_pool.connect_options().as_ref().clone();
    drop(destination);
    target_pool.close().await;
    let reopened_pool = PgPoolOptions::new()
        .max_connections(8)
        .connect_with(options)
        .await
        .unwrap();
    let reopened = PostgresStore::from_pool(reopened_pool.clone())
        .await
        .unwrap();
    assert_eq!(project(&reopened).await, (rebuilt, 4));
    let store = Store::new(reopened.clone());
    let repository = store.repository::<AggregateCounter>().json().build();
    let mut root = repository.load(TestId::new("first")).await.unwrap();
    repository
        .save::<0>(&mut root, &mnesis::events![CounterEvent::Incremented])
        .await
        .unwrap();
    let (advanced, advanced_state) = project(&reopened).await;
    assert_eq!(advanced_state, 5);
    assert!(advanced > rebuilt);
    assert_eq!(advanced.history(), reopened.history());
    drop(repository);
    drop(store);
    drop(reopened);
    reopened_pool.close().await;
}
