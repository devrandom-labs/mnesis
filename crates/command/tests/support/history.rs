//! Command histories checked against a small independent list-based model.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "property failures retain the generated counterexample"
)]

use std::num::{NonZeroU64, NonZeroUsize};
use std::time::Duration;

use mnesis::Version;
use mnesis_command::{Cause, CommandProcessor, Error, Limits, Request};
use mnesis_inmemory::InMemoryStore;
use mnesis_store::{RawEventStore, Store};

#[path = "counter.rs"]
mod counter;
use counter::{Counter, Rejection, Sum};
#[path = "finish.rs"]
pub mod shutdown;
use shutdown::finish;
#[path = "../faults/mod.rs"]
pub mod faults;
pub use counter::Operation;
use faults::{Fault, FaultStore};

type Backend = FaultStore<InMemoryStore>;
struct Seen {
    id: u64,
    expected: u64,
    operation: Operation,
    version: u64,
    events: usize,
}
#[derive(Default)]
struct Model {
    accepted: Vec<Seen>,
    sum: u64,
}
#[derive(Debug, PartialEq, Eq)]
enum Answer {
    Accepted(u64, usize),
    Mismatch,
    Conflict,
    Rejected,
}
impl Model {
    fn decide(&mut self, id: u64, expected: u64, operation: Operation) -> Answer {
        if let Some(prior) = self.accepted.iter().find(|entry| entry.id == id) {
            return if prior.expected == expected && prior.operation == operation {
                Answer::Accepted(prior.version, prior.events)
            } else {
                Answer::Mismatch
            };
        }
        let version = u64::try_from(self.accepted.len()).unwrap();
        if expected != version {
            return Answer::Conflict;
        }
        let events = match operation {
            Operation::Add(amount) => {
                self.sum += u64::from(amount);
                1
            }
            Operation::Ignore => 0,
            Operation::Reject => return Answer::Rejected,
        };
        self.accepted.push(Seen {
            id,
            expected,
            operation,
            version: version + 1,
            events,
        });
        Answer::Accepted(version + 1, events)
    }
}
type Factory = CommandProcessor<Counter, Operation>;
type Host = CommandProcessor<Counter, Operation, NonZeroU64, 0, <Backend as RawEventStore>::Error>;
type Task = mnesis_command::Worker<Operation>;
fn start(store: &Store<Backend>) -> (Host, Task) {
    Factory::start(store, String::from("history"), limits()).unwrap()
}
const fn limits() -> Limits {
    Limits {
        queue: NonZeroUsize::MIN,
        operation: Duration::from_secs(2),
    }
}
async fn restart(store: &Store<Backend>, host: Host, task: Task, model: &Model) -> (Host, Task) {
    host.stop();
    let shutdown = finish(task).await;
    assert_eq!(shutdown.uncertain(), None);
    let (restored, observed) =
        Factory::resume(store, String::from("history"), limits(), &shutdown).unwrap();
    let state = restored.read().await.unwrap();
    assert_eq!(
        state.0.map_or(0, Version::as_u64),
        u64::try_from(model.accepted.len()).unwrap()
    );
    assert_eq!(state.1, Sum(model.sum));
    (restored, observed)
}
async fn apply(host: &Host, model: &mut Model, id: u64, expected: u64, operation: Operation) {
    let predicted = model.decide(id, expected, operation.clone());
    let submitted = host
        .submit(Request {
            id: NonZeroU64::new(id).unwrap(),
            expected: Version::new(expected),
            command: operation,
        })
        .await;
    let observed = match submitted {
        Ok(receipt) => {
            assert_eq!(receipt.id.get(), id);
            Answer::Accepted(receipt.version.as_u64(), receipt.events)
        }
        Err(Error::Mismatch) => Answer::Mismatch,
        Err(Error::Conflict) => Answer::Conflict,
        Err(Error::Failed(cause)) if matches!(*cause, Cause::Rejected(Rejection)) => {
            Answer::Rejected
        }
        Err(other) => panic!("unexpected controller error: {other:?}"),
    };
    assert_eq!(observed, predicted);
    let restored = host.read().await.unwrap();
    assert_eq!(
        restored.0.map_or(0, Version::as_u64),
        u64::try_from(model.accepted.len()).unwrap()
    );
    assert_eq!(restored.1, Sum(model.sum));
}
#[expect(
    clippy::cognitive_complexity,
    reason = "one generated scenario keeps command selection, restart and independent state observations together"
)]
pub async fn history(inputs: &[(u8, u8, u8, u8)]) {
    let store = Store::new(FaultStore::new(InMemoryStore::new()));
    let (mut host, mut task) = start(&store);
    let mut model = Model::default();
    // Every case exercises change, no-op, exact retry and mismatch before the
    // generated suffix, so an all-rejected history cannot make the law vacuous.
    apply(&host, &mut model, 1, 0, Operation::Add(7)).await;
    apply(&host, &mut model, 2, 1, Operation::Ignore).await;
    apply(&host, &mut model, 1, 0, Operation::Add(7)).await;
    apply(&host, &mut model, 2, 1, Operation::Ignore).await;
    apply(&host, &mut model, 1, 0, Operation::Add(8)).await;
    store.raw().arm(Fault::Rollback);
    let unknown = host
        .submit(Request {
            id: NonZeroU64::new(3).unwrap(),
            expected: Version::new(2),
            command: Operation::Add(5),
        })
        .await;
    assert!(matches!(unknown, Err(Error::Uncertain { .. })));
    let barrier = host.read().await;
    assert!(matches!(barrier, Err(Error::Uncertain { .. })));
    let changed = host
        .submit(Request {
            id: NonZeroU64::new(3).unwrap(),
            expected: Version::new(2),
            command: Operation::Add(6),
        })
        .await;
    assert!(matches!(changed, Err(Error::Mismatch)));
    let preserved = host.read().await;
    assert!(matches!(preserved, Err(Error::Uncertain { .. })));
    host.stop();
    let unsettled = finish(task).await;
    assert_eq!(
        unsettled.uncertain(),
        Some(&Request {
            id: NonZeroU64::new(3).unwrap(),
            expected: Version::new(2),
            command: Operation::Add(5)
        })
    );
    (host, task) = Factory::resume(&store, String::from("history"), limits(), &unsettled).unwrap();
    let retried = host
        .retry(NonZeroU64::new(3).unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retried.version, Version::new(3).unwrap());
    apply(&host, &mut model, 3, 2, Operation::Add(5)).await;
    (host, task) = restart(&store, host, task, &model).await;
    for &(tag, identity, metadata, amount) in inputs {
        if tag % 7 == 0 {
            (host, task) = restart(&store, host, task, &model).await;
            continue;
        }
        let expected = if metadata % 2 == 0 {
            u64::try_from(model.accepted.len()).unwrap()
        } else {
            u64::from(metadata % 32)
        };
        let operation = match tag % 3 {
            0 => Operation::Add(amount),
            1 => Operation::Ignore,
            _ => Operation::Reject,
        };
        apply(
            &host,
            &mut model,
            u64::from(identity % 16) + 1,
            expected,
            operation,
        )
        .await;
    }
    host.stop();
    assert_eq!(finish(task).await.uncertain(), None);
}

pub async fn run(bytes: &[u8]) {
    let inputs = bytes[..bytes.len().min(384)]
        .as_chunks::<4>()
        .0
        .iter()
        .copied()
        .map(Into::into)
        .collect::<Vec<_>>();
    history(&inputs).await;
}
