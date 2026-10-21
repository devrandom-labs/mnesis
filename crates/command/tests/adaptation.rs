//! Unrelated consumers exercise the production controller, not a copied adapter.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test failures retain diagnostics"
)]

use std::fmt;
use std::num::{NonZeroU64, NonZeroUsize};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use futures::TryStreamExt;
use mnesis::{Aggregate, AggregateState, DomainEvent, Events, Handle, Message, Version, events};
use mnesis_command::{Cause, CodecError, CommandProcessor, Error, Limits, Request};
use mnesis_inmemory::InMemoryStore;
use mnesis_store::{PendingBatch, RawEventStore, Store, StoreError, StreamKey, pending_envelope};
use serde::{Deserialize, Serialize};

#[path = "support/finish.rs"]
mod shutdown;
use shutdown::finish;

static DECISIONS: AtomicUsize = AtomicUsize::new(0);
#[derive(Debug, thiserror::Error)]
#[error("fixture rejection")]
struct Rejected;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Count {
    One(u8),
    Pair(u8, u8),
    Ignore,
    Reject,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Added(u8);
impl Message for Added {}
impl DomainEvent for Added {
    fn name(&self) -> &'static str {
        "added"
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Total(u64);
impl AggregateState for Total {
    type Event = Added;
    fn initial() -> Self {
        Self(0)
    }
    fn apply(self, event: &Added) -> Self {
        Self(self.0 + u64::from(event.0))
    }
}
struct Counter;
impl Aggregate for Counter {
    type State = Total;
    type Error = Rejected;
    type Id = String;
}
impl Handle<Count, 1> for Counter {
    fn handle(_: &Total, command: Count) -> Result<Option<Events<Added, 1>>, Rejected> {
        DECISIONS.fetch_add(1, Ordering::SeqCst);
        match command {
            Count::One(value) => Ok(Some(events![Added(value)])),
            Count::Pair(first, second) => Ok(Some(events![Added(first), Added(second)])),
            Count::Ignore => Ok(None),
            Count::Reject => Err(Rejected),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Rename(String);
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Renamed(String);
impl Message for Renamed {}
impl DomainEvent for Renamed {
    fn name(&self) -> &'static str {
        "renamed"
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Label(String);
impl AggregateState for Label {
    type Event = Renamed;
    fn initial() -> Self {
        Self(String::new())
    }
    fn apply(self, event: &Renamed) -> Self {
        Self(event.0.clone())
    }
}
struct Register;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Key(&'static str);
impl AsRef<[u8]> for Key {
    fn as_ref(&self) -> &[u8] {
        self.0.as_bytes()
    }
}
impl fmt::Display for Key {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "label:{}", self.0)
    }
}
impl Aggregate for Register {
    type State = Label;
    type Error = Rejected;
    type Id = Key;
}
impl Handle<Rename> for Register {
    fn handle(state: &Label, command: Rename) -> Result<Option<Events<Renamed>>, Rejected> {
        DECISIONS.fetch_add(1, Ordering::SeqCst);
        Ok((state.0 != command.0).then(|| events![Renamed(command.0)]))
    }
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
enum Identity {
    Client(NonZeroU64),
    Internal(NonZeroU64),
}

const fn limits() -> Limits {
    Limits {
        queue: NonZeroUsize::new(8).unwrap(),
        operation: Duration::from_secs(2),
    }
}
const fn request<C>(id: u64, previous: u64, command: C) -> Request<C> {
    Request {
        id: NonZeroU64::new(id).unwrap(),
        expected: Version::new(previous),
        command,
    }
}
async fn raw(store: &InMemoryStore, id: &[u8]) -> Vec<serde_json::Value> {
    let records = store
        .read_stream(&StreamKey::from_slice(id), Version::INITIAL)
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    records
        .iter()
        .map(|record| serde_json::from_slice(record.payload()).unwrap())
        .collect()
}

#[tokio::test]
async fn unrelated_aggregates_use_the_same_controller_without_domain_policy() {
    // One serial scenario owns DECISIONS: unrelated tests cannot perturb its
    // independent replay oracle.
    let store = Store::new(InMemoryStore::new());
    counter(&store).await;
    register(&store).await;
    domain_limit(&store).await;
    encoding_failures(&store).await;
    lossy_admission(&store).await;
    evolution(&store).await;
    corrupt_history(&store).await;
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum EvolvedCommand {
    Original(Count),
    Increase { amount: u8 },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum EvolvedFact {
    Original(Added),
    Increased { amount: u8 },
}
impl Message for EvolvedFact {}
impl DomainEvent for EvolvedFact {
    fn name(&self) -> &'static str {
        "added"
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct EvolvedState(u64);
impl AggregateState for EvolvedState {
    type Event = EvolvedFact;
    fn initial() -> Self {
        Self(0)
    }
    fn apply(self, fact: &EvolvedFact) -> Self {
        let amount = match fact {
            EvolvedFact::Original(Added(value)) | EvolvedFact::Increased { amount: value } => {
                *value
            }
        };
        Self(self.0 + u64::from(amount))
    }
}
struct Evolved;
impl Aggregate for Evolved {
    type State = EvolvedState;
    type Error = Rejected;
    type Id = String;
}
impl Handle<EvolvedCommand, 1> for Evolved {
    fn handle(
        state: &EvolvedState,
        command: EvolvedCommand,
    ) -> Result<Option<Events<EvolvedFact, 1>>, Rejected> {
        match command {
            EvolvedCommand::Original(original) => {
                Counter::handle(&Total(state.0), original).map(|batch| {
                    batch.map(|facts| {
                        let mut mapped = Events::new(EvolvedFact::Original(facts.first().clone()));
                        for fact in facts.rest() {
                            mapped.add(EvolvedFact::Original(fact.clone()));
                        }
                        mapped
                    })
                })
            }
            EvolvedCommand::Increase { amount } => {
                DECISIONS.fetch_add(1, Ordering::SeqCst);
                Ok(Some(events![EvolvedFact::Increased { amount }]))
            }
        }
    }
}

async fn evolution(store: &Store<InMemoryStore>) {
    let (old, task) = CommandProcessor::<Counter, Count, NonZeroU64, 1>::start(
        store,
        "evolution".into(),
        limits(),
    )
    .unwrap();
    let original = request(1, 0, Count::Pair(2, 5));
    let receipt = old.submit(original.clone()).await.unwrap();
    old.stop();
    assert_eq!(finish(task).await.uncertain(), None);
    let prefix = raw(store.raw(), b"evolution").await;
    let (new, worker) = CommandProcessor::<Evolved, EvolvedCommand, NonZeroU64, 1>::start(
        store,
        "evolution".into(),
        limits(),
    )
    .unwrap();
    let compatible = request(1, 0, EvolvedCommand::Original(Count::Pair(2, 5)));
    assert_eq!(compatible.encode().unwrap(), original.encode().unwrap());
    let before = DECISIONS.load(Ordering::SeqCst);
    assert_eq!(
        new.read().await.unwrap(),
        (Version::new(1), EvolvedState(7))
    );
    assert_eq!(
        new.original(NonZeroU64::MIN).await.unwrap(),
        Some(compatible.clone())
    );
    assert_eq!(new.submit(compatible).await.unwrap(), receipt);
    assert_eq!(DECISIONS.load(Ordering::SeqCst), before);
    assert_eq!(raw(store.raw(), b"evolution").await, prefix);
    let changed = new
        .submit(request(1, 0, EvolvedCommand::Increase { amount: 7 }))
        .await;
    assert!(matches!(changed, Err(Error::Mismatch)));
    let next = new
        .submit(request(2, 1, EvolvedCommand::Increase { amount: 9 }))
        .await
        .unwrap();
    assert_eq!((next.version, next.events), (Version::new(2).unwrap(), 1));
    assert_eq!(
        new.read().await.unwrap(),
        (Version::new(2), EvolvedState(16))
    );
    assert_eq!(DECISIONS.load(Ordering::SeqCst), before + 1);
    let records = raw(store.raw(), b"evolution").await;
    assert_eq!(&records[..1], prefix);
    assert_eq!(records[1]["facts"], serde_json::json!([{"amount":9}]));
    new.stop();
    assert_eq!(finish(worker).await.uncertain(), None);
}

async fn corrupt_history(store: &Store<InMemoryStore>) {
    let first = serde_json::json!({"schema":2,"request":{"id":1,"expected":null,"command":{"One":1}},
        "receipt":{"id":1,"version":1,"events":1},"facts":[1]});
    let mut second = serde_json::json!({"schema":2,"request":{"id":2,"expected":1,"command":{"One":1}},
        "receipt":{"id":2,"version":2,"events":1},"facts":[1]});
    let cumulative = second.clone();
    second["request"]["id"] = serde_json::json!(1);
    second["receipt"]["id"] = serde_json::json!(1);
    second["request"]["command"] = serde_json::json!("Ignore");
    second["receipt"]["events"] = serde_json::json!(0);
    second["facts"] = serde_json::json!([]);
    for (name, later) in [("duplicate", second), ("cumulative", cumulative)] {
        let key = StreamKey::from_slice(name.as_bytes());
        let batch = [first.clone(), later]
            .into_iter()
            .enumerate()
            .map(|(index, value)| {
                pending_envelope(Version::new(u64::try_from(index).unwrap() + 1).unwrap())
                    .event_type("controller.accepted.v2")
                    .payload(serde_json::to_vec(&value).unwrap())
                    .build()
                    .unwrap()
            })
            .collect::<Vec<_>>();
        store
            .raw()
            .append(&key, None, PendingBatch::new(&batch).unwrap())
            .await
            .unwrap();
        let before = raw(store.raw(), name.as_bytes()).await;
        let decisions = DECISIONS.load(Ordering::SeqCst);
        let (processor, task) =
            CommandProcessor::<Limited, Count, NonZeroU64, 1>::start(store, name.into(), limits())
                .unwrap();
        assert!(matches!(processor.read().await, Err(Error::Corrupt)));
        assert!(matches!(
            processor.original(NonZeroU64::MIN).await,
            Err(Error::Corrupt)
        ));
        assert!(matches!(
            processor.submit(request(1, 0, Count::One(1))).await,
            Err(Error::Corrupt)
        ));
        assert!(matches!(
            processor.submit(request(3, 2, Count::Ignore)).await,
            Err(Error::Corrupt)
        ));
        assert_eq!(DECISIONS.load(Ordering::SeqCst), decisions);
        assert_eq!(raw(store.raw(), name.as_bytes()).await, before);
        processor.stop();
        assert_eq!(finish(task).await.uncertain(), None);
    }
    corrupt_metadata(store, first).await;
}

async fn corrupt_metadata(store: &Store<InMemoryStore>, mut value: serde_json::Value) {
    value["receipt"]["id"] = serde_json::json!(2);
    let key = StreamKey::from_slice(b"metadata");
    let record = pending_envelope(Version::INITIAL)
        .event_type("controller.accepted.v2")
        .payload(serde_json::to_vec(&value).unwrap())
        .build()
        .unwrap();
    store
        .raw()
        .append(&key, None, PendingBatch::new(&[record]).unwrap())
        .await
        .unwrap();
    let before = raw(store.raw(), b"metadata").await;
    let decisions = DECISIONS.load(Ordering::SeqCst);
    let (processor, task) = CommandProcessor::<Limited, Count, NonZeroU64, 1>::start(
        store,
        "metadata".into(),
        limits(),
    )
    .unwrap();
    for failure in [
        processor.read().await.map(|_| ()),
        processor.original(NonZeroU64::MIN).await.map(|_| ()),
        processor
            .submit(request(1, 0, Count::One(1)))
            .await
            .map(|_| ()),
    ] {
        assert!(
            matches!(failure, Err(Error::Failed(cause)) if matches!(*cause, Cause::Store(StoreError::Decode(CodecError::Acceptance))))
        );
    }
    assert_eq!(DECISIONS.load(Ordering::SeqCst), decisions);
    assert_eq!(raw(store.raw(), b"metadata").await, before);
    processor.stop();
    assert_eq!(finish(task).await.uncertain(), None);
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct LossyRename {
    label: String,
    #[serde(skip)]
    detail: u8,
}
impl Handle<LossyRename> for Register {
    fn handle(_: &Label, command: LossyRename) -> Result<Option<Events<Renamed>>, Rejected> {
        DECISIONS.fetch_add(1, Ordering::SeqCst);
        Ok(Some(events![Renamed(command.label)]))
    }
}
async fn lossy_admission(store: &Store<InMemoryStore>) {
    let (controller, task) =
        CommandProcessor::<Register, LossyRename>::start(store, Key("lossy"), limits()).unwrap();
    let before = DECISIONS.load(Ordering::SeqCst);
    let submitted = controller
        .submit(request(
            1,
            0,
            LossyRename {
                label: String::from("ready"),
                detail: 7,
            },
        ))
        .await;
    assert!(matches!(submitted, Err(Error::Request(CodecError::Lossy))));
    assert_eq!(DECISIONS.load(Ordering::SeqCst), before);
    assert_eq!(
        raw(store.raw(), b"lossy").await,
        Vec::<serde_json::Value>::new()
    );
    controller.stop();
    assert_eq!(finish(task).await.uncertain(), None);
}

#[expect(
    clippy::cognitive_complexity,
    reason = "one codec failure scenario checks request and record bounds, empty durable history, definite failure and reuse of an unaccepted identity"
)]
async fn encoding_failures(store: &Store<InMemoryStore>) {
    let (controller, task) =
        CommandProcessor::<Register, Rename>::start(store, Key("oversized"), limits()).unwrap();
    let oversized_record = request(1, 0, Rename("x".repeat(2500)));
    assert!(oversized_record.encode().unwrap().len() < 4096);
    let failed_record = controller.submit(oversized_record).await;
    assert!(
        matches!(failed_record, Err(Error::Failed(cause)) if matches!(*cause, Cause::Store(StoreError::Encode(CodecError::Bytes { source:Some(_) }))))
    );
    assert_eq!(
        raw(store.raw(), b"oversized").await,
        Vec::<serde_json::Value>::new()
    );
    let original = controller.original(NonZeroU64::MIN).await.unwrap();
    assert_eq!(original, None);
    let oversized_request = request(1, 0, Rename("x".repeat(4096)));
    let before = DECISIONS.load(Ordering::SeqCst);
    let failed_request = controller.submit(oversized_request).await;
    assert!(matches!(
        failed_request,
        Err(Error::Request(CodecError::Bytes { source: Some(_) }))
    ));
    assert_eq!(DECISIONS.load(Ordering::SeqCst), before);
    let accepted = controller
        .submit(request(1, 0, Rename(String::from("small"))))
        .await
        .unwrap();
    assert_eq!((accepted.version, accepted.events), (Version::INITIAL, 1));
    let state = controller.read().await.unwrap();
    assert_eq!(
        state,
        (Some(Version::INITIAL), Label(String::from("small")))
    );
    assert_eq!(raw(store.raw(), b"oversized").await.len(), 1);
    controller.stop();
    assert_eq!(finish(task).await.uncertain(), None);
}

#[expect(
    clippy::cognitive_complexity,
    reason = "one acceptance/restart scenario compares public results against raw history and a handler-call oracle"
)]
async fn counter(store: &Store<InMemoryStore>) {
    let (controller, task) = CommandProcessor::<Counter, Count, NonZeroU64, 1>::start(
        store,
        String::from("count"),
        limits(),
    )
    .unwrap();
    let pair = request(1, 0, Count::Pair(2, 5));
    let first = controller.submit(pair.clone()).await.unwrap();
    let nothing = request(2, 1, Count::Ignore);
    let noop = controller.submit(nothing.clone()).await.unwrap();
    assert_eq!((first.version.as_u64(), first.events), (1, 2));
    assert_eq!((noop.version.as_u64(), noop.events), (2, 0));
    let records = raw(store.raw(), b"count").await;
    assert_eq!(records.len(), 2);
    assert_eq!(
        records[0]["request"]["command"]["Pair"],
        serde_json::json!([2, 5])
    );
    assert_eq!(records[0]["facts"], serde_json::json!([2, 5]));
    assert_eq!(
        records[0]["receipt"],
        serde_json::json!({"id": 1, "version": 1, "events": 2})
    );
    assert_eq!(records[1]["facts"], serde_json::json!([]));
    assert_eq!(
        records[1]["receipt"],
        serde_json::json!({"id": 2, "version": 2, "events": 0})
    );
    let rejected = controller.submit(request(3, 2, Count::Reject)).await;
    assert!(
        matches!(rejected, Err(Error::Failed(cause)) if matches!(*cause, Cause::Rejected(Rejected)))
    );
    let changed = controller.submit(request(1, 0, Count::Ignore)).await;
    assert!(matches!(changed, Err(Error::Mismatch)));
    let metadata = controller.submit(request(1, 2, Count::Pair(2, 5))).await;
    assert!(matches!(metadata, Err(Error::Mismatch)));
    let conflict = controller.submit(request(4, 0, Count::Ignore)).await;
    assert!(matches!(conflict, Err(Error::Conflict)));
    let before = DECISIONS.load(Ordering::SeqCst);
    let retried = controller.submit(pair.clone()).await.unwrap();
    let retried_noop = controller.submit(nothing.clone()).await.unwrap();
    let original = controller.original(NonZeroU64::MIN).await.unwrap();
    let state = controller.read().await.unwrap();
    let after = DECISIONS.load(Ordering::SeqCst);
    assert_eq!((retried, retried_noop), (first.clone(), noop.clone()));
    assert_eq!(original, Some(pair.clone()));
    assert_eq!(state, (Version::new(2), Total(7)));
    assert_eq!(before, after);
    assert_eq!(raw(store.raw(), b"count").await, records);
    controller.stop();
    let shutdown = finish(task).await;
    assert_eq!(shutdown.uncertain(), None);
    let closed = controller.submit(request(3, 2, Count::Ignore)).await;
    assert!(matches!(closed, Err(Error::Closed)));
    let wrong = CommandProcessor::<Counter, Count, NonZeroU64, 1>::resume(
        store,
        String::from("another"),
        limits(),
        &shutdown,
    );
    assert!(matches!(wrong, Err(Error::Scope)));
    let (restarted, observed) = CommandProcessor::<Counter, Count, NonZeroU64, 1>::resume(
        store,
        String::from("count"),
        limits(),
        &shutdown,
    )
    .unwrap();
    let restored = restarted.read().await.unwrap();
    let restored_pair = restarted.submit(pair).await.unwrap();
    let restored_noop = restarted.submit(nothing).await.unwrap();
    assert_eq!(restored, state);
    assert_eq!((restored_pair, restored_noop), (first, noop));
    assert_eq!(DECISIONS.load(Ordering::SeqCst), before);
    restarted.stop();
    assert_eq!(finish(observed).await.uncertain(), None);
}

#[expect(
    clippy::cognitive_complexity,
    reason = "single scenario compares two ID namespaces and unrelated state with raw records and the replay oracle"
)]
async fn register(store: &Store<InMemoryStore>) {
    let (controller, task) =
        CommandProcessor::<Register, Rename, Identity>::start(store, Key("label"), limits())
            .unwrap();
    let client = Request {
        id: Identity::Client(NonZeroU64::MIN),
        expected: None,
        command: Rename(String::from("ready")),
    };
    let internal = Request {
        id: Identity::Internal(NonZeroU64::MIN),
        expected: Version::new(1),
        command: Rename(String::from("ready")),
    };
    let accepted = controller.submit(client.clone()).await.unwrap();
    let noop = controller.submit(internal).await.unwrap();
    let again = controller.submit(client).await.unwrap();
    assert_eq!(accepted, again);
    assert_eq!((noop.version.as_u64(), noop.events), (2, 0));
    let labels = raw(store.raw(), b"label").await;
    assert_eq!(labels.len(), 2);
    assert_eq!(labels[0]["facts"], serde_json::json!(["ready"]));
    assert_eq!(labels[1]["facts"], serde_json::json!([]));
    let before = DECISIONS.load(Ordering::SeqCst);
    let state = controller.read().await.unwrap();
    assert_eq!(state, (Version::new(2), Label(String::from("ready"))));
    assert_eq!(DECISIONS.load(Ordering::SeqCst), before);
    controller.stop();
    assert_eq!(finish(task).await.uncertain(), None);
}

struct Limited;
impl Aggregate for Limited {
    type State = Total;
    type Error = Rejected;
    type Id = String;
    const MAX_REHYDRATION_EVENTS: NonZeroUsize = NonZeroUsize::MIN;
}
impl Handle<Count, 1> for Limited {
    fn handle(state: &Total, command: Count) -> Result<Option<Events<Added, 1>>, Rejected> {
        Counter::handle(state, command)
    }
}
#[expect(
    clippy::cognitive_complexity,
    reason = "one boundary scenario checks both rejection without persistence and no-op acceptance"
)]
async fn domain_limit(store: &Store<InMemoryStore>) {
    let (controller, task) = CommandProcessor::<Limited, Count, NonZeroU64, 1>::start(
        store,
        String::from("limited"),
        limits(),
    )
    .unwrap();
    let denied = controller.submit(request(1, 0, Count::Pair(2, 5))).await;
    assert!(
        matches!(denied, Err(Error::Failed(cause)) if matches!(*cause, Cause::ReplayLimit { maximum:1 }))
    );
    assert_eq!(
        raw(store.raw(), b"limited").await,
        Vec::<serde_json::Value>::new()
    );
    let accepted = controller
        .submit(request(1, 0, Count::Ignore))
        .await
        .unwrap();
    assert_eq!((accepted.version.as_u64(), accepted.events), (1, 0));
    let state = controller.read().await.unwrap();
    assert_eq!(state, (Version::new(1), Total(0)));
    let single = controller
        .submit(request(2, 1, Count::One(5)))
        .await
        .unwrap();
    assert_eq!((single.version.as_u64(), single.events), (2, 1));
    let cumulative = controller.submit(request(3, 2, Count::One(9))).await;
    assert!(
        matches!(cumulative, Err(Error::Failed(cause)) if matches!(*cause, Cause::ReplayLimit { maximum:1 }))
    );
    assert_eq!(raw(store.raw(), b"limited").await.len(), 2);
    let without_facts = controller
        .submit(request(3, 2, Count::Ignore))
        .await
        .unwrap();
    assert_eq!(
        (without_facts.version.as_u64(), without_facts.events),
        (3, 0)
    );
    let full = controller.read().await.unwrap();
    assert_eq!(full, (Version::new(3), Total(5)));
    controller.stop();
    assert_eq!(finish(task).await.uncertain(), None);
}
