//! Public fact consumption checkpoints each complete physical acceptance.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "consumer failures retain exact outcomes"
)]

use std::convert::Infallible;
use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use futures::TryStreamExt;
use mnesis::{Aggregate, AggregateState, DomainEvent, Events, Handle, Message, Version, events};
use mnesis_command::{Acceptance, AcceptanceCodec, CommandProcessor, Limits, Receipt, Request};
use mnesis_inmemory::{InMemoryCheckpointStore, InMemoryStore};
use mnesis_store::checkpoint::{CheckpointHydrated, CheckpointStore};
use mnesis_store::state::EveryNEvents;
use mnesis_store::{DecodedStreamExt, Projection, Projector, RawEventStore, Store, StreamKey};
use serde::{Deserialize, Serialize};

#[path = "support/finish.rs"]
mod shutdown;
use shutdown::finish;

static DECISIONS: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Command {
    Pair(u8, u8),
    Ignore,
    One(u8),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Written(u8);
impl Message for Written {}
impl DomainEvent for Written {
    fn name(&self) -> &'static str {
        "number"
    }
}

#[derive(Clone, Debug)]
struct Total(u64);
impl AggregateState for Total {
    type Event = Written;
    fn initial() -> Self {
        Self(0)
    }
    fn apply(self, event: &Written) -> Self {
        Self(self.0 + u64::from(event.0))
    }
}

struct Counter;
impl Aggregate for Counter {
    type State = Total;
    type Error = Infallible;
    type Id = String;
}
impl Handle<Command, 1> for Counter {
    fn handle(_: &Total, command: Command) -> Result<Option<Events<Written, 1>>, Infallible> {
        DECISIONS.fetch_add(1, Ordering::SeqCst);
        Ok(match command {
            Command::Pair(one, two) => Some(events![Written(one), Written(two)]),
            Command::Ignore => None,
            Command::One(value) => Some(events![Written(value)]),
        })
    }
}

// Reading must work without requiring fact Clone, Eq or Serialize.
#[derive(Debug, Deserialize)]
struct Read(u8);
impl Message for Read {}
impl DomainEvent for Read {
    fn name(&self) -> &'static str {
        "number"
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Model {
    facts: Vec<u8>,
    records: usize,
}

#[derive(Debug, thiserror::Error)]
#[error("projection record count overflow")]
struct Overflow;

struct Reader;
impl Projector for Reader {
    type Event = Acceptance<Command, Read>;
    type State = Model;
    type Error = Overflow;
    fn initial(&self) -> Model {
        Model {
            facts: Vec::new(),
            records: 0,
        }
    }
    fn apply(&self, mut state: Model, event: &Self::Event) -> Result<Model, Overflow> {
        state.records = state.records.checked_add(1).ok_or(Overflow)?;
        state.facts.extend(event.facts().iter().map(|fact| fact.0));
        Ok(state)
    }
}

const fn request(id: u64, expected: Option<Version>, command: Command) -> Request<Command> {
    Request {
        id: NonZeroU64::new(id).unwrap(),
        expected,
        command,
    }
}

async fn persisted(
    checkpoints: &InMemoryCheckpointStore<Model, Version>,
    position: Version,
    model: &Model,
) {
    let saved = checkpoints
        .hydrate_checkpoint(&String::from("view"), NonZeroU32::MIN)
        .await
        .unwrap();
    let expected_revision = u64::try_from(model.records).unwrap();
    assert!(
        matches!(saved, CheckpointHydrated::Found { revision, position: observed, state }
        if revision.get() == expected_revision && observed == position && &state == model)
    );
}

async fn project_prefix(
    store: &InMemoryStore,
    checkpoints: &InMemoryCheckpointStore<Model, Version>,
    originals: [(&Request<Command>, &Receipt); 2],
) {
    let [(pair, first), (noop, second)] = originals;
    let mut projection = Projection::load(
        String::from("view"),
        Reader,
        EveryNEvents(NonZeroU64::MIN),
        checkpoints,
        NonZeroU32::MIN,
    )
    .await
    .unwrap();
    let stream = store
        .read_stream(&StreamKey::from_slice(b"counter"), Version::INITIAL)
        .await
        .unwrap()
        .decoded(AcceptanceCodec);
    tokio::pin!(stream);
    let item = stream.try_next().await.unwrap().unwrap();
    assert_eq!(item.version, first.version);
    assert_eq!(item.event.request(), pair);
    assert_eq!(item.event.receipt(), first);
    assert_eq!(item.event.name(), "controller.accepted.v2");
    projection.advance(item).await.unwrap();
    persisted(
        checkpoints,
        first.version,
        &Model {
            facts: vec![5, 9],
            records: 1,
        },
    )
    .await;
    let empty = stream.try_next().await.unwrap().unwrap();
    assert_eq!(empty.event.request(), noop);
    assert_eq!(empty.event.receipt(), second);
    assert!(empty.event.facts().is_empty());
    projection.advance(empty).await.unwrap();
    assert!(stream.try_next().await.unwrap().is_none());
    persisted(
        checkpoints,
        second.version,
        &Model {
            facts: vec![5, 9],
            records: 2,
        },
    )
    .await;
}

#[tokio::test]
async fn public_acceptances_checkpoint_complete_records_and_resume_without_repeating_facts() {
    DECISIONS.store(0, Ordering::SeqCst);
    let store = Store::new(InMemoryStore::new());
    let (processor, worker) = CommandProcessor::<Counter, Command, NonZeroU64, 1>::start(
        &store,
        String::from("counter"),
        Limits {
            queue: NonZeroUsize::MIN,
            operation: Duration::from_secs(2),
        },
    )
    .unwrap();
    let pair = request(1, None, Command::Pair(5, 9));
    let first = processor.submit(pair.clone()).await.unwrap();
    let noop = request(2, Some(first.version), Command::Ignore);
    let second = processor.submit(noop.clone()).await.unwrap();
    assert_eq!((first.version.as_u64(), first.events), (1, 2));
    assert_eq!((second.version.as_u64(), second.events), (2, 0));
    let checkpoints = InMemoryCheckpointStore::new();
    project_prefix(
        store.raw(),
        &checkpoints,
        [(&pair, &first), (&noop, &second)],
    )
    .await;
    let mut resumed = Projection::load(
        String::from("view"),
        Reader,
        EveryNEvents(NonZeroU64::MIN),
        &checkpoints,
        NonZeroU32::MIN,
    )
    .await
    .unwrap();
    assert_eq!(resumed.checkpoint(), Some(second.version));
    assert_eq!(
        resumed.state().unwrap(),
        &Model {
            facts: vec![5, 9],
            records: 2
        }
    );
    let third = processor
        .submit(request(3, Some(second.version), Command::One(17)))
        .await
        .unwrap();
    let tail = store
        .raw()
        .read_stream(
            &StreamKey::from_slice(b"counter"),
            resumed.checkpoint().unwrap().next().unwrap(),
        )
        .await
        .unwrap()
        .decoded(AcceptanceCodec);
    tokio::pin!(tail);
    let next = tail.try_next().await.unwrap().unwrap();
    assert_eq!(next.version, third.version);
    resumed.advance(next).await.unwrap();
    assert!(tail.try_next().await.unwrap().is_none());
    let final_state = Model {
        facts: vec![5, 9, 17],
        records: 3,
    };
    assert_eq!(resumed.state().unwrap(), &final_state);
    persisted(&checkpoints, third.version, &final_state).await;
    assert_eq!(processor.submit(pair).await.unwrap(), first);
    assert_eq!(processor.submit(noop).await.unwrap(), second);
    assert_eq!(DECISIONS.load(Ordering::SeqCst), 3);
    processor.stop();
    assert_eq!(finish(worker).await.uncertain(), None);
}
