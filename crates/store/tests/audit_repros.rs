//! Known bugs from todo.md; ignored until fixed, asserting the required behavior.
#![allow(clippy::unwrap_used, reason = "audit regression assertions")]
use futures::TryStreamExt;
use mnesis::{AggregateRoot, Version, events};
use mnesis_inmemory::{InMemoryCheckpointStore, InMemorySnapshotStore, InMemoryStore};
use mnesis_store::cbor::{ChunkWriter, decode_chunk};
use mnesis_store::checkpoint::{CheckpointHydrated, CheckpointStore};
use mnesis_store::envelope::pending_envelope;
use mnesis_store::export::{ConsistentExporter, ExportSession};
use mnesis_store::import::{
    AtomicAppend, AtomicAppendError, Atomicity, EventImporter, InvalidRoute, InvalidRun,
    PlannedAppend, validate_atomic_runs,
};
use mnesis_store::state::{AfterEventTypes, SnapshotStore};
use mnesis_store::{
    Decode, Encode, PendingBatch, PersistedEnvelope, RawEventStore, Repository, StreamKey,
};
use mnesis_test_domains::{Counter, CounterEvent, CounterState, TestId};
use parking_lot::Mutex;
use std::{convert::Infallible, num::NonZeroU32, sync::Arc};

#[test]
fn commit_must_not_accept_unrelated_version() {
    let mut root = AggregateRoot::<Counter>::new(TestId::new("s"));
    root.commit_persisted::<0>(&events![CounterEvent::Incremented])
        .expect("root is usable");
    assert_eq!(root.version().unwrap().as_u64(), 1);
    assert_eq!(root.state().unwrap().value, 1);
    root.commit_persisted::<0>(&events![CounterEvent::Incremented])
        .expect("root is usable");
    assert_eq!(root.version(), Version::new(2));
    assert_eq!(root.state().unwrap().value, 2);
}

#[test]
fn snapshot_tail_has_its_own_replay_budget() {
    let mut root = AggregateRoot::<Counter>::restore(
        TestId::new("s"),
        CounterState { value: 10 },
        Version::new(1_000_000).unwrap(),
    );
    assert!(
        root.replay(Version::new(1_000_001).unwrap(), &CounterEvent::Incremented)
            .is_ok()
    );
}

#[tokio::test]
async fn ceiling_snapshot_is_restored_without_replay() {
    let raw = InMemoryStore::new().into_store();
    let snapshots = InMemorySnapshotStore::<CounterState, Version>::new();
    let id = TestId::new("s");
    snapshots
        .commit(
            &id,
            NonZeroU32::MIN,
            Version::new(u64::MAX).unwrap(),
            &CounterState { value: 123 },
        )
        .await
        .unwrap();
    let repo = raw
        .repository::<Counter>()
        .json()
        .snapshot_store(snapshots)
        .snapshot_trigger(AfterEventTypes::new(&[]))
        .build();
    let root = repo.load(id).await.unwrap();
    assert_eq!(root.version(), Version::new(u64::MAX));
    assert_eq!(root.state().unwrap().value, 123);
}

#[derive(Clone)]
struct ObservingCodec(Arc<Mutex<Vec<EnvelopeContext>>>);
type EnvelopeContext = (u64, u32, bool);
impl Encode<CounterEvent> for ObservingCodec {
    type Error = Infallible;
    fn encode(&self, _: &CounterEvent) -> Result<bytes::Bytes, Infallible> {
        Ok(bytes::Bytes::from_static(b"x"))
    }
}
impl Decode<CounterEvent> for ObservingCodec {
    type Output<'a> = CounterEvent;
    type Error = Infallible;
    fn decode<'a>(&'a self, env: &'a PersistedEnvelope) -> Result<CounterEvent, Infallible> {
        self.0.lock().push((
            env.version().as_u64(),
            env.schema_version(),
            env.metadata().is_some(),
        ));
        Ok(CounterEvent::Incremented)
    }
}
#[tokio::test]
async fn identity_upcast_preserves_envelope_context() {
    let store = InMemoryStore::new().into_store();
    let rows: Vec<_> = (1..=2)
        .map(|v| {
            pending_envelope(Version::new(v).unwrap())
                .event_type("Incremented")
                .payload(b"x".as_slice())
                .metadata(b"m".as_slice())
                .schema_version(mnesis_store::SchemaVersion::from_u32(7).unwrap())
                .build()
                .unwrap()
        })
        .collect();
    store
        .append(
            &StreamKey::from_slice(b"s"),
            None,
            PendingBatch::new(&rows).unwrap(),
        )
        .await
        .unwrap();
    let observed = Arc::new(Mutex::new(Vec::new()));
    let repo = store
        .repository::<Counter>()
        .codec(ObservingCodec(Arc::clone(&observed)))
        .build();
    repo.load_with(
        TestId::new("s"),
        |_| Ok::<_, Infallible>(()),
        |m| Ok::<_, Infallible>(m),
    )
    .await
    .unwrap();
    assert_eq!(*observed.lock(), vec![(1, 7, true), (2, 7, true)]);
}

#[derive(Debug)]
struct PanickingState(u64);

impl mnesis::AggregateState for PanickingState {
    type Event = CounterEvent;
    fn initial() -> Self {
        Self(0)
    }
    #[allow(
        clippy::panic,
        reason = "deterministic injection of an application panic"
    )]
    fn apply(self, _: &CounterEvent) -> Self {
        panic!("deliberate state-machine panic")
    }
}

struct PanickingAggregate;
impl mnesis::Aggregate for PanickingAggregate {
    type State = PanickingState;
    type Error = Infallible;
    type Id = TestId;
}

#[test]
fn panicking_replay_cannot_expose_usable_inconsistent_root() {
    let mut root = AggregateRoot::<PanickingAggregate>::restore(
        TestId::new("s"),
        PanickingState(123),
        Version::INITIAL,
    );
    assert_eq!(root.state().unwrap().0, 123);
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        root.replay(Version::new(2).unwrap(), &CounterEvent::Incremented)
    }));
    assert!(outcome.is_err());
    assert_eq!(root.version(), Some(Version::INITIAL));
    assert!(matches!(
        root.state(),
        Err(mnesis::KernelError::PoisonedAggregate)
    ));
    assert!(matches!(
        root.replay(Version::new(2).unwrap(), &CounterEvent::Incremented),
        Err(mnesis::KernelError::PoisonedAggregate)
    ));
    assert!(matches!(
        root.commit_persisted::<0>(&events![CounterEvent::Incremented]),
        Err(mnesis::KernelError::PoisonedAggregate)
    ));
    assert_eq!(root.version(), Some(Version::INITIAL));
}

struct CountProjector;
impl mnesis_store::Projector for CountProjector {
    type Event = CounterEvent;
    type State = u64;
    type Error = Infallible;
    fn initial(&self) -> u64 {
        0
    }
    fn apply(&self, state: u64, _: &CounterEvent) -> Result<u64, Infallible> {
        Ok(state + 1)
    }
}

const fn decoded(version: u64) -> mnesis_store::Decoded<CounterEvent> {
    mnesis_store::Decoded {
        event: CounterEvent::Incremented,
        version: Version::new(version).unwrap(),
        metadata: None,
    }
}

#[tokio::test]
#[allow(clippy::panic, reason = "assert the checkpoint was persisted")]
async fn projection_cannot_checkpoint_stale_state() {
    let checkpoints = InMemoryCheckpointStore::<u64, Version>::new();
    let id = TestId::new("p");
    let mut projection = mnesis_store::Projection::load(
        id.clone(),
        CountProjector,
        AfterEventTypes::new(&[]),
        &checkpoints,
        NonZeroU32::MIN,
    )
    .await
    .unwrap();
    let stale = *projection.state().unwrap();
    projection.advance(decoded(1)).await.unwrap();
    let folded = *projection.state().unwrap();
    assert_ne!(stale, folded);
    projection.flush().await.unwrap();
    let CheckpointHydrated::Found {
        position, state, ..
    } = checkpoints
        .hydrate_checkpoint(&id, NonZeroU32::MIN)
        .await
        .unwrap()
    else {
        panic!("checkpoint must exist");
    };
    assert_eq!(position, Version::new(1).unwrap());
    assert_eq!(state, folded, "flush must persist its own matching state");
}

#[tokio::test]
async fn projection_rejects_duplicate_position() {
    let checkpoints = InMemoryCheckpointStore::<u64, Version>::new();
    let mut projection = mnesis_store::Projection::load(
        TestId::new("p"),
        CountProjector,
        AfterEventTypes::new(&[]),
        &checkpoints,
        NonZeroU32::MIN,
    )
    .await
    .unwrap();
    projection.advance(decoded(1)).await.unwrap();
    assert!(matches!(
        projection.advance(decoded(1)).await,
        Err(mnesis_store::ProjectionError::NonIncreasingPosition)
    ));
    assert_eq!(*projection.state().unwrap(), 1);
    assert_eq!(projection.observed(), Version::new(1));
}

#[tokio::test]
async fn atomic_append_rejects_two_runs_routed_to_one_target() {
    let store = InMemoryStore::new();
    let target = StreamKey::from_slice(b"same");
    let rows: Vec<_> = (1..=2)
        .map(|v| PlannedAppend {
            target: target.clone(),
            expected_version: Version::new(v - 1),
            head: pending_envelope(Version::new(v).unwrap())
                .event_type("E")
                .payload(b"x".as_slice())
                .build()
                .unwrap(),
            tail: vec![],
        })
        .collect();
    assert!(matches!(store.atomic_append_many(&rows).await,
        Err(AtomicAppendError::InvalidRoute(error)) if error == InvalidRoute { target, first_index: 0, index: 1 }));
}

#[tokio::test]
async fn malformed_atomic_run_is_not_a_retryable_head_conflict() {
    let store = InMemoryStore::new();
    let write = PlannedAppend {
        target: StreamKey::from_slice(b"gap"),
        expected_version: None,
        head: pending_envelope(Version::INITIAL)
            .event_type("E")
            .payload(b"one".as_slice())
            .build()
            .unwrap(),
        tail: vec![
            pending_envelope(Version::new(3).unwrap())
                .event_type("E")
                .payload(b"three".as_slice())
                .build()
                .unwrap(),
        ],
    };
    let failure = store.atomic_append_many(&[write]).await.unwrap_err();
    assert!(
        matches!(failure, AtomicAppendError::InvalidRun(error) if error == InvalidRun::NonSequential {
            index: 0, expected: Version::new(2).unwrap(), actual: Version::new(3).unwrap()
        })
    );
}

fn raw_run(expected_version: Option<Version>, versions: &[u64]) -> PlannedAppend {
    let mut rows = versions.iter().map(|&version| {
        pending_envelope(Version::new(version).unwrap())
            .event_type("E")
            .payload(b"data".as_slice())
            .build()
            .unwrap()
    });
    PlannedAppend {
        target: StreamKey::from_slice(b"target"),
        expected_version,
        head: rows.next().unwrap(),
        tail: rows.collect(),
    }
}

#[test]
fn atomic_run_validation_preserves_exact_ceiling_and_reports_required_overflow() {
    let valid = raw_run(Version::new(u64::MAX - 1), &[u64::MAX]);
    assert_eq!(validate_atomic_runs(&[valid]), Ok(()));
    let beyond = raw_run(Version::new(u64::MAX - 1), &[u64::MAX, 1]);
    assert_eq!(
        validate_atomic_runs(&[beyond]),
        Err(InvalidRun::VersionOverflow { index: 0 })
    );
    let no_successor = raw_run(Version::new(u64::MAX), &[1]);
    assert_eq!(
        validate_atomic_runs(&[no_successor]),
        Err(InvalidRun::VersionOverflow { index: 0 })
    );
    assert_eq!(validate_atomic_runs(&[]), Ok(()));
}

#[tokio::test]
async fn truncated_session_backup_must_not_restore_half_an_atomic_source_commit() {
    let source = InMemoryStore::new();
    let ids = [StreamKey::from_slice(b"a"), StreamKey::from_slice(b"b")];
    let writes = ids
        .iter()
        .map(|id| PlannedAppend {
            target: id.clone(),
            expected_version: None,
            head: pending_envelope(Version::INITIAL)
                .event_type("E")
                .payload(b"atomic".as_slice())
                .build()
                .unwrap(),
            tail: vec![],
        })
        .collect::<Vec<_>>();
    source.atomic_append_many(&writes).await.unwrap();
    let session = source
        .open_export_session(std::time::Duration::from_secs(60))
        .await
        .unwrap();
    let mut full = ChunkWriter::new(Vec::new(), None).unwrap();
    let mut prefix = ChunkWriter::new(Vec::new(), None).unwrap();
    for (index, id) in ids.iter().enumerate() {
        let events = session.export_stream(id, Version::INITIAL).await.unwrap();
        full.section(id.as_bytes())
            .unwrap()
            .try_extend(events)
            .await
            .unwrap();
        if index == 0 {
            let first = session.export_stream(id, Version::INITIAL).await.unwrap();
            prefix
                .section(id.as_bytes())
                .unwrap()
                .try_extend(first)
                .await
                .unwrap();
        }
    }
    session.close().await.unwrap();
    let full_bytes = full.finish().unwrap();
    let prefix_bytes = prefix.into_unfinished_sink();
    assert_eq!(full_bytes[..prefix_bytes.len()], prefix_bytes);
    assert_eq!(decode_chunk(&full_bytes).unwrap().len(), 2);
    let truncated = decode_chunk(&full_bytes[..prefix_bytes.len()]);
    let incomplete = matches!(
        &truncated,
        Err(mnesis_store::ChunkError::Completion(
            mnesis_store::CompletionError::Missing
        ))
    );
    let destination = InMemoryStore::new();
    if let Ok(sections) = truncated {
        destination
            .import(&sections, StreamKey::from_slice, Atomicity::WholeChunk)
            .await
            .unwrap();
    }
    let mut counts = Vec::new();
    for id in &ids {
        counts.push(raw_history(&destination, id).await.len());
    }
    assert_eq!(
        counts,
        [0, 0],
        "incomplete backup restored half of the source's atomic transaction"
    );
    assert!(incomplete, "normal restore must report missing completion");
    destination.atomic_append_many(&writes).await.unwrap();
    for id in &ids {
        let rows = raw_history(&destination, id).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].version(), Version::INITIAL);
        assert_eq!(rows[0].payload(), b"atomic");
    }
}

async fn raw_history(store: &InMemoryStore, id: &StreamKey) -> Vec<PersistedEnvelope> {
    store
        .read_stream(id, Version::INITIAL)
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap()
}
