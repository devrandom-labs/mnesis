//! A06: caught application panics must not expose reusable inconsistent roots.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions and deterministic application panic injection"
)]
use futures::{FutureExt, StreamExt};
use mnesis::{
    Aggregate, AggregateRoot, AggregateState, DecisionError, Events, Handle, KernelError, Version,
    events,
};
use mnesis_inmemory::InMemoryStore;
use mnesis_store::{CommandRepository, RawEventStore, Repository, SagaRepository, StreamKey};
use mnesis_test_domains::{CounterEvent, TestId};
use std::convert::Infallible;
use std::panic::{AssertUnwindSafe, catch_unwind};

// Deliberately not Clone: integrity cannot depend on copying old state.
#[derive(Debug)]
struct State {
    value: u64,
    panic_at: Option<u64>,
}
impl AggregateState for State {
    type Event = CounterEvent;
    fn initial() -> Self {
        Self {
            value: 0,
            panic_at: None,
        }
    }
    fn apply(mut self, _: &CounterEvent) -> Self {
        self.value = self.value.checked_add(1).expect("small test history");
        assert_ne!(
            self.panic_at,
            Some(self.value),
            "injected application panic"
        );
        self
    }
}
struct Fragile;
impl Aggregate for Fragile {
    type State = State;
    type Error = Infallible;
    type Id = TestId;
}
impl Handle<()> for Fragile {
    fn handle(_: &State, (): ()) -> Result<Option<Events<CounterEvent>>, Infallible> {
        Ok(None)
    }
}
impl mnesis::Saga for Fragile {
    type CorrelationKey = ();
    type Command = CounterEvent;
    fn intent_for(_: &CounterEvent) -> Option<CounterEvent> {
        Some(CounterEvent::Incremented)
    }
}
impl mnesis::React<CounterEvent> for Fragile {
    fn correlate(_: &CounterEvent) -> Option<()> {
        Some(())
    }
    fn react(_: &State, _: &CounterEvent) -> Result<Option<Events<CounterEvent>>, Infallible> {
        Ok(Some(one_event()))
    }
}

fn one_event() -> Events<CounterEvent> {
    events![CounterEvent::Incremented]
}

fn restored(panic_at: u64) -> AggregateRoot<Fragile> {
    AggregateRoot::restore(
        TestId::new("s"),
        State {
            value: 1,
            panic_at: Some(panic_at),
        },
        Version::INITIAL,
    )
}
fn assert_unusable(root: &mut AggregateRoot<Fragile>) {
    assert!(matches!(root.state(), Err(KernelError::PoisonedAggregate)));
    assert!(matches!(
        root.handle(()),
        Err(DecisionError::Kernel(KernelError::PoisonedAggregate))
    ));
    assert!(matches!(
        root.react::<CounterEvent, 0>(&CounterEvent::Incremented),
        Err(DecisionError::Kernel(KernelError::PoisonedAggregate))
    ));
    assert!(matches!(
        root.replay(Version::new(5).unwrap(), &CounterEvent::Incremented),
        Err(KernelError::PoisonedAggregate)
    ));
    assert!(matches!(
        root.commit_persisted(&one_event()),
        Err(KernelError::PoisonedAggregate)
    ));
}

#[test]
fn first_middle_last_replay_panics_reject_every_further_transition() {
    for panic_at in 2..=4 {
        let mut root = restored(panic_at);
        for version in 2..panic_at {
            root.replay(Version::new(version).unwrap(), &CounterEvent::Incremented)
                .unwrap();
        }
        let previous = root.version();
        assert!(
            catch_unwind(AssertUnwindSafe(
                || root.replay(Version::new(panic_at).unwrap(), &CounterEvent::Incremented)
            ))
            .is_err()
        );
        assert_unusable(&mut root);
        assert_eq!(root.version(), previous);
    }
}

#[test]
fn first_middle_last_committed_fold_panics_keep_boundary_but_no_state() {
    for panic_at in 2..=4 {
        let mut root = restored(panic_at);
        let committed: Events<CounterEvent, 2> = events![
            CounterEvent::Incremented,
            CounterEvent::Incremented,
            CounterEvent::Incremented
        ];
        assert!(catch_unwind(AssertUnwindSafe(|| root.commit_persisted(&committed))).is_err());
        assert_unusable(&mut root);
        assert_eq!(root.version(), Version::new(4));
    }
}

#[tokio::test]
async fn post_persist_panic_keeps_exactly_once_history_and_requires_reload() {
    for panic_at in 2..=4 {
        let store = InMemoryStore::new().into_store();
        let repo = store.repository::<Fragile>().json().build();
        let id = TestId::new("s");
        let mut seed = repo.load(id.clone()).await.unwrap();
        repo.save(&mut seed, &one_event()).await.unwrap();
        let mut root = restored(panic_at);
        let committed: Events<CounterEvent, 2> = events![
            CounterEvent::Incremented,
            CounterEvent::Incremented,
            CounterEvent::Incremented
        ];
        assert!(
            AssertUnwindSafe(repo.save(&mut root, &committed))
                .catch_unwind()
                .await
                .is_err()
        );
        assert_unusable(&mut root);
        assert!(matches!(
            repo.save(&mut root, &one_event()).await,
            Err(mnesis_store::StoreError::Kernel(
                KernelError::PoisonedAggregate
            ))
        ));
        assert!(matches!(
            repo.execute(&mut root, ()).await,
            Err(mnesis_store::ExecuteError::Kernel(
                KernelError::PoisonedAggregate
            ))
        ));
        assert!(matches!(
            repo.react_and_save::<CounterEvent, 0>(&mut root, &CounterEvent::Incremented)
                .await,
            Err(mnesis_store::SagaError::Kernel(
                KernelError::PoisonedAggregate
            ))
        ));
        let mut rows = store
            .read_stream(&StreamKey::from_slice(b"s"), Version::INITIAL)
            .await
            .unwrap();
        let mut versions = Vec::new();
        while let Some(row) = rows.next().await {
            versions.push(row.unwrap().version().as_u64());
        }
        assert_eq!(versions, vec![1, 2, 3, 4]);
        let mut recovered = repo.load(id).await.unwrap();
        assert_eq!(recovered.version(), Version::new(4));
        assert_eq!(recovered.state().unwrap().value, 4);
        repo.save(&mut recovered, &one_event()).await.unwrap();
        assert_eq!(recovered.version(), Version::new(5));
        assert_eq!(recovered.state().unwrap().value, 5);
    }
}
