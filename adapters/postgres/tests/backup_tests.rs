//! Completed, bounded chunks compose the real Postgres and Fjall adapters.
#![cfg(all(feature = "export", feature = "import"))]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "native backup assertions"
)]

#[cfg(feature = "projection")]
#[path = "support/consumer.rs"]
mod consumer;
#[cfg(feature = "projection")]
#[path = "support/projection.rs"]
mod projection;

use std::collections::BTreeMap;
use std::io::{self, ErrorKind, Read};
use std::num::NonZeroU32;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use futures::{FutureExt, StreamExt, TryStreamExt, stream};
use mnesis::Version;
use mnesis_fjall::{AllIndex, FjallStore};
use mnesis_postgres::{Limits, PostgresError, PostgresStore};
use mnesis_store::cbor::{
    ChunkError, ChunkWriter, CompletionError, SalvagedChunk, SectionError, WriteError,
    decode_chunk, salvage_chunk,
};
#[cfg(feature = "projection")]
use mnesis_store::checkpoint::{CheckpointHydrated, CheckpointStore};
use mnesis_store::export::{ConsistentExporter, ExportError, ExportSession, StreamLister};
use mnesis_store::import::{
    AbortReason, AtomicAppend, Atomicity, EventImporter, ImportBlock, ImportError, InvalidRoute,
    PlannedAppend, StreamOutcome, StreamReport,
};
use mnesis_store::value::SchemaVersion;
use mnesis_store::{
    BatchSize, PendingBatch, PendingEnvelope, PersistedEnvelope, RawEventStore, StreamKey,
    pending_envelope,
};
#[cfg(feature = "projection")]
use mnesis_store::{Repository, Store};
#[cfg(feature = "projection")]
use mnesis_test_domains::{Counter as AggregateCounter, CounterEvent, TestId};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tokio::sync::Barrier;
use tokio::task::yield_now;
use tokio::time::timeout;

const CAPACITY: usize = 16_384;

async fn small(store: &PostgresStore) {
    for index in 1..=3 {
        for number in 1..=2 {
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

async fn packed(source: &impl ConsistentExporter) -> Vec<u8> {
    let session = source
        .open_export_session(Duration::from_secs(60))
        .await
        .unwrap();
    let mut bytes = vec![0; CAPACITY];
    let mut writer = ChunkWriter::new(bytes.as_mut_slice(), Some(b"backup-law")).unwrap();
    {
        let listing = session.list_streams().await.unwrap();
        futures::pin_mut!(listing);
        while let Some(id) = listing.try_next().await.unwrap() {
            let events = session.export_stream(&id, Version::INITIAL).await.unwrap();
            writer
                .section(id.as_bytes())
                .unwrap()
                .try_extend(events)
                .await
                .unwrap();
        }
    }
    let remaining = writer.finish().unwrap().len();
    session.close().await.unwrap();
    bytes.truncate(CAPACITY.checked_sub(remaining).unwrap());
    bytes
}

fn routed(origin: &[u8]) -> StreamKey {
    let mut bytes = vec![b'r', 0, 255];
    bytes.extend_from_slice(origin);
    StreamKey::from_bytes(bytes)
}

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

fn read_chunk(path: &Path) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(CAPACITY.checked_add(1).unwrap());
    std::fs::File::open(path)?
        .take(u64::try_from(CAPACITY.checked_add(1).unwrap()).unwrap())
        .read_to_end(&mut bytes)?;
    if bytes.len() > CAPACITY {
        return Err(io::Error::new(
            ErrorKind::FileTooLarge,
            "backup chunk exceeds fixed capacity",
        ));
    }
    Ok(bytes)
}

async fn restore(
    store: &impl EventImporter,
    directory: &Path,
    count: usize,
    policy: Atomicity,
) -> io::Result<()> {
    for index in 0..count {
        let bytes = read_chunk(&directory.join(format!("{index}.nxch")))?;
        let _sections = decode_chunk(&bytes).unwrap();
    }
    for index in 0..count {
        let bytes = read_chunk(&directory.join(format!("{index}.nxch")))?;
        let sections = decode_chunk(&bytes).unwrap();
        let report = store
            .import(&sections, StreamKey::from_slice, policy)
            .await
            .unwrap();
        assert_eq!(report.unfinished().count(), 0);
    }
    Ok(())
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
        restore(&destination, directory.path(), count, policy)
            .await
            .unwrap();
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
    restore(&fjall, first.path(), count, Atomicity::WholeChunk)
        .await
        .unwrap();
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
    restore(&destination, second.path(), returned, Atomicity::PerStream)
        .await
        .unwrap();
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
    let bytes = read_chunk(&directory.path().join("0.nxch")).unwrap();
    assert_eq!(decode_chunk(&bytes).unwrap().len(), 0);
    for policy in [Atomicity::WholeChunk, Atomicity::PerStream] {
        restore(&store, directory.path(), 1, policy).await.unwrap();
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
    let bytes = read_chunk(&directory.path().join("0.nxch")).unwrap();
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
#[tokio::test]
async fn logical_restore_rebuilds_a_projection_then_restarts_and_advances_it() {
    let Some((source, pool)) = fixture().await else {
        return;
    };
    projection::events(&source).await;
    let (old, state) = consumer::resume(&source).await;
    assert_eq!(state, 4);
    let directory = tempfile::tempdir().unwrap();
    let count = backup(&source, directory.path()).await;
    assert_eq!(count, 2);
    drop(source);
    pool.close().await;
    let (destination, target_pool) = fixture().await.unwrap();
    restore(&destination, directory.path(), count, Atomicity::WholeChunk)
        .await
        .unwrap();
    assert_eq!(
        destination
            .hydrate_checkpoint(&StreamKey::from_slice(b"restored-counter"), NonZeroU32::MIN)
            .await
            .unwrap(),
        CheckpointHydrated::Absent
    );
    let (rebuilt, rebuilt_state) = consumer::resume(&destination).await;
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
    assert_eq!(consumer::resume(&reopened).await, (rebuilt, 4));
    let store = Store::new(reopened.clone());
    let repository = store.repository::<AggregateCounter>().json().build();
    let mut root = repository.load(TestId::new("first")).await.unwrap();
    repository
        .save::<0>(&mut root, &mnesis::events![CounterEvent::Incremented])
        .await
        .unwrap();
    let (advanced, advanced_state) = consumer::resume(&reopened).await;
    assert_eq!(advanced_state, 5);
    assert!(advanced > rebuilt);
    assert_eq!(advanced.history(), reopened.history());
    drop(repository);
    drop(store);
    drop(reopened);
    reopened_pool.close().await;
}

#[tokio::test]
async fn completed_chunk_uses_each_binary_route_once_and_preserves_all_fields() {
    let Some((source, pool)) = fixture().await else {
        return;
    };
    small(&source).await;
    let expected: BTreeMap<_, _> = contents(&source)
        .await
        .into_iter()
        .map(|(origin, events)| (routed(&origin).as_bytes().to_vec(), events))
        .collect();
    let bytes = packed(&source).await;
    let sections = decode_chunk(&bytes).unwrap();
    assert_eq!(sections.len(), 3);
    drop(source);
    pool.close().await;
    for policy in [Atomicity::WholeChunk, Atomicity::PerStream] {
        let (target, target_pool) = fixture().await.unwrap();
        let calls = AtomicUsize::new(0);
        let report = target
            .import(
                &sections,
                |origin| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    routed(origin)
                },
                policy,
            )
            .await
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        assert_eq!(
            report.streams(),
            &[1, 2, 3].map(|index| StreamReport {
                stream: routed(key(index).as_bytes()),
                outcome: StreamOutcome::Complete {
                    version: Version::new(2).unwrap()
                },
            })
        );
        assert_eq!(contents(&target).await, expected);
        drop(target);
        target_pool.close().await;
    }
}

#[tokio::test]
async fn completed_chunk_late_duplicate_rejects_before_any_write_under_both_policies() {
    let Some((source, pool)) = fixture().await else {
        return;
    };
    small(&source).await;
    let bytes = packed(&source).await;
    let sections = decode_chunk(&bytes).unwrap();
    assert_eq!(sections.len(), 3);
    drop(source);
    pool.close().await;
    for policy in [Atomicity::WholeChunk, Atomicity::PerStream] {
        for existing in [None, Some(event(1, 1))] {
            let (target, target_pool) = fixture().await.unwrap();
            if let Some(pending) = existing {
                target
                    .append(&key(1), None, PendingBatch::of(&pending))
                    .await
                    .unwrap();
            }
            let before = contents(&target).await;
            let calls = AtomicUsize::new(0);
            let error = target
                .import(
                    &sections,
                    |origin| {
                        calls.fetch_add(1, Ordering::SeqCst);
                        if origin == key(3).as_bytes() {
                            key(1)
                        } else {
                            StreamKey::from_slice(origin)
                        }
                    },
                    policy,
                )
                .await
                .unwrap_err();
            assert_eq!(calls.load(Ordering::SeqCst), 3);
            assert!(matches!(error, ImportError::InvalidRoute(InvalidRoute {
                target: duplicate, first_index: 0, index: 2,
            }) if duplicate == key(1)));
            assert_eq!(contents(&target).await, before);
            drop(target);
            target_pool.close().await;
        }
    }
}

#[tokio::test]
async fn completed_chunk_final_conflict_keeps_exact_policy_progress_and_report() {
    let Some((source, pool)) = fixture().await else {
        return;
    };
    small(&source).await;
    let original = contents(&source).await;
    let bytes = packed(&source).await;
    let sections = decode_chunk(&bytes).unwrap();
    drop(source);
    pool.close().await;
    for policy in [Atomicity::WholeChunk, Atomicity::PerStream] {
        let (target, target_pool) = fixture().await.unwrap();
        let seed = pending_envelope(Version::INITIAL)
            .event_type("Existing")
            .payload(b"independent target".as_slice())
            .build()
            .unwrap();
        target
            .append(&key(3), None, PendingBatch::of(&seed))
            .await
            .unwrap();
        let expected = contents(&target).await;
        let result = target
            .import(&sections, StreamKey::from_slice, policy)
            .await;
        match policy {
            Atomicity::WholeChunk => {
                assert!(matches!(result.unwrap_err(), ImportError::Aborted {
                    stream: conflict, reason: AbortReason::Mismatch { expected: next, got },
                } if conflict == key(3) && next == Version::new(2).unwrap() && got == Version::INITIAL));
                assert_eq!(contents(&target).await, expected);
            }
            Atomicity::PerStream => {
                let report = result.unwrap();
                assert_eq!(
                    report.streams(),
                    &[
                        StreamReport {
                            stream: key(1),
                            outcome: StreamOutcome::Complete {
                                version: Version::new(2).unwrap()
                            }
                        },
                        StreamReport {
                            stream: key(2),
                            outcome: StreamOutcome::Complete {
                                version: Version::new(2).unwrap()
                            }
                        },
                        StreamReport {
                            stream: key(3),
                            outcome: StreamOutcome::Mismatch {
                                reached: None,
                                got: Version::INITIAL
                            }
                        },
                    ]
                );
                let mut restored = contents(&target).await;
                for index in [1, 2] {
                    let id = key(index).as_bytes().to_vec();
                    assert_eq!(restored.remove(&id).as_ref(), original.get(&id));
                }
                assert_eq!(restored, expected);
            }
        }
        drop(target);
        target_pool.close().await;
    }
}

#[tokio::test]
async fn completed_chunk_keeps_one_view_across_an_overlapping_atomic_commit() {
    let Some((source, pool)) = fixture().await else {
        return;
    };
    small(&source).await;
    let expected = contents(&source).await;
    let session = source
        .open_export_session(Duration::from_secs(60))
        .await
        .unwrap();
    let start = Arc::new(Barrier::new(2));
    let signal = Arc::clone(&start);
    let concurrent = source.clone();
    let writer = tokio::spawn(async move {
        signal.wait().await;
        let writes = [1, 2, 3].map(|index| PlannedAppend {
            target: key(index),
            expected_version: Version::new(2),
            head: event(index, 3),
            tail: vec![],
        });
        concurrent.atomic_append_many(&writes).await.unwrap()
    });
    let mut bytes = vec![0; CAPACITY];
    let mut chunk = ChunkWriter::new(bytes.as_mut_slice(), Some(b"backup-law")).unwrap();
    let first = session
        .export_stream(&key(1), Version::INITIAL)
        .await
        .unwrap();
    chunk
        .section(key(1).as_bytes())
        .unwrap()
        .try_extend(first)
        .await
        .unwrap();
    start.wait().await;
    assert!(writer.await.unwrap().is_some());
    for index in [2, 3] {
        let events = session
            .export_stream(&key(index), Version::INITIAL)
            .await
            .unwrap();
        chunk
            .section(key(index).as_bytes())
            .unwrap()
            .try_extend(events)
            .await
            .unwrap();
    }
    let remaining = chunk.finish().unwrap().len();
    session.close().await.unwrap();
    bytes.truncate(CAPACITY.checked_sub(remaining).unwrap());
    let sections = decode_chunk(&bytes).unwrap();
    let current = contents(&source).await;
    assert_eq!(
        current.values().map(Vec::len).collect::<Vec<_>>(),
        [3, 3, 3]
    );
    drop(source);
    pool.close().await;
    let (target, target_pool) = fixture().await.unwrap();
    let report = target
        .import(&sections, StreamKey::from_slice, Atomicity::WholeChunk)
        .await
        .unwrap();
    assert_eq!(report.streams().len(), 3);
    assert_eq!(contents(&target).await, expected);
    drop(target);
    target_pool.close().await;
}

#[tokio::test]
async fn independent_chunk_inventory_rejects_missing_or_oversized_final_file_before_writes() {
    let Some((source, pool)) = fixture().await else {
        return;
    };
    small(&source).await;
    let expected = contents(&source).await;
    let directory = tempfile::tempdir().unwrap();
    let count = backup(&source, directory.path()).await;
    assert_eq!(count, 3);
    let final_file = directory
        .path()
        .join(format!("{}.nxch", count.checked_sub(1).unwrap()));
    let original = read_chunk(&final_file).unwrap();
    drop(source);
    pool.close().await;
    for policy in [Atomicity::WholeChunk, Atomicity::PerStream] {
        let (target, target_pool) = fixture().await.unwrap();
        let held = directory.path().join("held.nxch");
        std::fs::rename(&final_file, &held).unwrap();
        let missing = restore(&target, directory.path(), count, policy)
            .await
            .unwrap_err();
        std::fs::rename(&held, &final_file).unwrap();
        assert_eq!(missing.kind(), ErrorKind::NotFound);
        assert_eq!(contents(&target).await, BTreeMap::new());
        std::fs::write(&final_file, vec![0; CAPACITY]).unwrap();
        assert_eq!(read_chunk(&final_file).unwrap().len(), CAPACITY);
        std::fs::write(&final_file, vec![0; CAPACITY.checked_add(1).unwrap()]).unwrap();
        let oversized = restore(&target, directory.path(), count, policy)
            .await
            .unwrap_err();
        std::fs::write(&final_file, &original).unwrap();
        assert_eq!(oversized.kind(), ErrorKind::FileTooLarge);
        assert_eq!(contents(&target).await, BTreeMap::new());
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM mnesis.keys")
                .fetch_one(&target_pool)
                .await
                .unwrap(),
            0
        );
        restore(&target, directory.path(), count, policy)
            .await
            .unwrap();
        assert_eq!(contents(&target).await, expected);
        drop(target);
        target_pool.close().await;
    }
}

fn partial(bytes: &[u8]) -> SalvagedChunk {
    assert_eq!(
        decode_chunk(bytes).unwrap().first().unwrap().blocks.len(),
        2
    );
    let marker = b"unique-corrupt-tail-marker";
    assert_eq!(
        bytes
            .windows(marker.len())
            .filter(|item| *item == marker)
            .count(),
        1
    );
    let offset = bytes
        .windows(marker.len())
        .position(|item| item == marker)
        .unwrap();
    let mut damaged = bytes.to_vec();
    *damaged.get_mut(offset).unwrap() ^= 1;
    assert!(matches!(
        decode_chunk(&damaged),
        Err(ChunkError::Completion(CompletionError::Checksum))
    ));
    let recovered = salvage_chunk(&damaged).unwrap();
    assert_eq!(recovered.header().format_version, 2);
    assert_eq!(
        recovered.header().origin.as_deref(),
        Some(b"backup-law".as_slice())
    );
    assert!(matches!(
        recovered.completion(),
        Err(CompletionError::Checksum)
    ));
    assert!(
        matches!(recovered.sections(), [section] if section.origin.as_ref() == key(1).as_bytes()
        && matches!(section.blocks.as_slice(), [ImportBlock::Event(event), ImportBlock::Corrupt]
            if event.version() == Version::INITIAL))
    );
    let torn = salvage_chunk(&bytes[..bytes.len().checked_sub(1).unwrap()]).unwrap();
    assert!(matches!(torn.completion(), Err(CompletionError::Missing)));
    assert_eq!(torn.header().origin, recovered.header().origin);
    assert_eq!(torn.sections().first().unwrap().blocks.len(), 2);
    let mut malformed = ChunkWriter::new(Vec::new(), Some(b"backup-law"))
        .unwrap()
        .into_unfinished_sink();
    malformed.extend_from_slice(&[0xa1, 0x00, 0x05]);
    assert!(matches!(
        decode_chunk(&malformed),
        Err(ChunkError::Decode {
            context: "malformed section heading",
            ..
        })
    ));
    assert!(matches!(
        salvage_chunk(&malformed),
        Err(ChunkError::Decode {
            context: "malformed section heading",
            ..
        })
    ));
    recovered
}

#[tokio::test]
async fn damaged_chunk_requires_explicit_salvage_and_retains_prefix_report_and_provenance() {
    let Some((source, pool)) = fixture().await else {
        return;
    };
    let first = pending_envelope(Version::INITIAL)
        .event_type("Recovered")
        .payload(b"good-prefix".as_slice())
        .build()
        .unwrap();
    let second = pending_envelope(Version::new(2).unwrap())
        .event_type("Damaged")
        .payload(b"unique-corrupt-tail-marker".as_slice())
        .build()
        .unwrap();
    source
        .append(&key(1), None, PendingBatch::from_parts(&first, &[second]))
        .await
        .unwrap();
    let mut prefix = contents(&source).await;
    prefix.get_mut(key(1).as_bytes()).unwrap().truncate(1);
    let recovered = partial(&packed(&source).await);
    drop(source);
    pool.close().await;
    let (target, target_pool) = fixture().await.unwrap();
    assert_eq!(contents(&target).await, BTreeMap::new());
    let sections = recovered.into_sections_for_partial_recovery();
    let error = target
        .import(&sections, StreamKey::from_slice, Atomicity::WholeChunk)
        .await
        .unwrap_err();
    assert!(
        matches!(error, ImportError::Aborted { stream: rejected, reason: AbortReason::Corrupt } if rejected == key(1))
    );
    assert_eq!(contents(&target).await, BTreeMap::new());
    let report = target
        .import(&sections, StreamKey::from_slice, Atomicity::PerStream)
        .await
        .unwrap();
    assert_eq!(
        report.streams(),
        &[StreamReport {
            stream: key(1),
            outcome: StreamOutcome::Corrupt {
                reached: Some(Version::INITIAL)
            },
        }]
    );
    assert_eq!(contents(&target).await, prefix);
    drop(target);
    target_pool.close().await;
}
