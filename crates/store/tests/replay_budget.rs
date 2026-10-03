//! A08: replay budgets count successful work in this root, not history position.
#![allow(clippy::unwrap_used, reason = "test assertions")]
use mnesis::{Aggregate, AggregateRoot, Events, KernelError, Version, events};
use mnesis_inmemory::{InMemorySnapshotStore, InMemoryStore};
use mnesis_store::state::{AfterEventTypes, SnapshotStore};
use mnesis_store::{RawEventStore, Repository};
use mnesis_test_domains::{CounterError, CounterEvent, CounterState, TestId};
use std::num::{NonZeroU32, NonZeroUsize};
struct Limited;
impl Aggregate for Limited {
    type State = CounterState;
    type Error = CounterError;
    type Id = TestId;
    const MAX_REHYDRATION_EVENTS: NonZeroUsize = match NonZeroUsize::new(2) {
        Some(limit) => limit,
        None => unreachable!(),
    };
}
const fn v(value: u64) -> Version {
    Version::new(value).unwrap()
}
#[test]
fn fresh_and_restored_roots_have_the_same_work_budget() {
    for baseline in [0u64, 1_000_000] {
        let mut root = if baseline == 0 {
            AggregateRoot::<Limited>::new(TestId::new("s"))
        } else {
            AggregateRoot::restore(TestId::new("s"), CounterState { value: 0 }, v(baseline))
        };
        for offset in 1..=2 {
            root.replay(
                v(baseline.checked_add(offset).unwrap()),
                &CounterEvent::Incremented,
            )
            .unwrap();
        }
        let previous = root.version();
        assert!(matches!(
            root.replay(
                v(baseline.checked_add(3).unwrap()),
                &CounterEvent::Incremented
            ),
            Err(KernelError::RehydrationLimitExceeded { max: 2 })
        ));
        assert_eq!(root.version(), previous);
        assert_eq!(root.state().unwrap().value, 2);
    }
}
#[test]
fn rejected_versions_and_commits_do_not_consume_replay_budget() {
    let mut root = AggregateRoot::<Limited>::new(TestId::new("s"));
    assert!(matches!(
        root.replay(v(2), &CounterEvent::Incremented),
        Err(KernelError::VersionMismatch { .. })
    ));
    let committed: Events<CounterEvent, 2> = events![
        CounterEvent::Incremented,
        CounterEvent::Incremented,
        CounterEvent::Incremented
    ];
    root.commit_persisted(&committed).unwrap();
    assert!(matches!(
        root.replay(v(5), &CounterEvent::Incremented),
        Err(KernelError::VersionMismatch { .. })
    ));
    root.replay(v(4), &CounterEvent::Incremented).unwrap();
    assert!(matches!(
        root.replay(v(4), &CounterEvent::Incremented),
        Err(KernelError::VersionMismatch { .. })
    ));
    root.replay(v(5), &CounterEvent::Incremented).unwrap();
    assert!(matches!(
        root.replay(v(6), &CounterEvent::Incremented),
        Err(KernelError::RehydrationLimitExceeded { max: 2 })
    ));
    assert_eq!(root.version(), Some(v(5)));
    assert_eq!(root.state().unwrap().value, 5);
}
#[test]
fn a_short_tail_can_reach_the_version_ceiling() {
    let mut root = AggregateRoot::<Limited>::restore(
        TestId::new("s"),
        CounterState { value: 0 },
        v(u64::MAX.checked_sub(1).unwrap()),
    );
    root.replay(v(u64::MAX), &CounterEvent::Incremented)
        .unwrap();
    assert_eq!(root.version(), Some(v(u64::MAX)));
    assert_eq!(root.state().unwrap().value, 1);
    assert!(matches!(
        root.replay(v(u64::MAX), &CounterEvent::Incremented),
        Err(KernelError::VersionOverflow)
    ));
    assert_eq!(root.state().unwrap().value, 1);
}

#[tokio::test]
async fn snapshot_tail_limit_is_independent_of_the_snapshot_boundary() {
    let store = InMemoryStore::new().into_store();
    let writer = store.repository::<Limited>().json().build();
    let id = TestId::new("s");
    let mut root = writer.load(id.clone()).await.unwrap();
    let committed: Events<CounterEvent, 2> = events![
        CounterEvent::Incremented,
        CounterEvent::Incremented,
        CounterEvent::Incremented
    ];
    writer.save(&mut root, &committed).await.unwrap();
    let snapshots = InMemorySnapshotStore::<CounterState, Version>::new();
    snapshots
        .commit(&id, NonZeroU32::MIN, v(1), &CounterState { value: 1 })
        .await
        .unwrap();
    let reader = store
        .repository::<Limited>()
        .json()
        .snapshot_store(snapshots)
        .snapshot_trigger(AfterEventTypes::new(&[]))
        .build();
    let restored = reader.load(id.clone()).await.unwrap();
    assert_eq!(restored.version(), Some(v(3)));
    assert_eq!(restored.state().unwrap().value, 3);
    let one: Events<CounterEvent> = events![CounterEvent::Incremented];
    writer.save(&mut root, &one).await.unwrap();
    assert!(matches!(
        reader.load(id).await,
        Err(mnesis_store::StoreError::Kernel(
            KernelError::RehydrationLimitExceeded { max: 2 }
        ))
    ));
}
