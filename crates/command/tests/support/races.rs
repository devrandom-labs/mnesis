//! Both processors must finish deciding before the first real CAS can run.
#![allow(
    clippy::unwrap_used,
    reason = "race failures retain the exact generated inputs and outcomes"
)]

use std::convert::Infallible;
use std::num::{NonZeroU64, NonZeroUsize};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use futures::TryStreamExt;
use mnesis::{Aggregate, AggregateState, DomainEvent, Events, Handle, Message, Version, events};
use mnesis_command::{CommandProcessor, Error, Limits, Receipt, Request};
use mnesis_inmemory::InMemoryStore;
use mnesis_store::{RawEventStore, Store, StreamKey};
use serde::{Deserialize, Serialize};
use tokio::time::timeout;

use super::faults::{FaultStore, Gate};
use super::shutdown::finish;

static DECISIONS: AtomicUsize = AtomicUsize::new(0);
type Backend = FaultStore<InMemoryStore>;
type Host = CommandProcessor<Counter, u8, NonZeroU64, 0, <Backend as RawEventStore>::Error>;

struct History {
    stores: [Store<Backend>; 2],
    before: Vec<(Version, serde_json::Value)>,
    prefix: Option<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Fact(u8);
impl Message for Fact {}
impl DomainEvent for Fact {
    fn name(&self) -> &'static str {
        "race"
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Total(u64);
impl AggregateState for Total {
    type Event = Fact;
    fn initial() -> Self {
        Self(0)
    }
    fn apply(self, event: &Fact) -> Self {
        Self(self.0 + u64::from(event.0))
    }
}
struct Counter;
impl Aggregate for Counter {
    type State = Total;
    type Error = Infallible;
    type Id = String;
}
impl Handle<u8> for Counter {
    fn handle(_: &Total, amount: u8) -> Result<Option<Events<Fact>>, Infallible> {
        DECISIONS.fetch_add(1, Ordering::SeqCst);
        Ok(Some(events![Fact(amount)]))
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Duplicate,
    Conflict,
    Mismatch,
}
#[derive(Clone, Copy, Debug)]
pub enum Winner {
    First,
    Second,
}

async fn rows(store: &InMemoryStore) -> Vec<(Version, serde_json::Value)> {
    store
        .read_stream(&StreamKey::from_slice(b"race"), Version::INITIAL)
        .await
        .unwrap()
        .map_ok(|row| {
            (
                row.version(),
                serde_json::from_slice(row.payload()).unwrap(),
            )
        })
        .try_collect()
        .await
        .unwrap()
}

fn requests(kind: Kind, id: NonZeroU64, amount: u8, expected: Option<Version>) -> [Request<u8>; 2] {
    let first = Request {
        id,
        expected,
        command: amount,
    };
    let second = match kind {
        Kind::Duplicate => first.clone(),
        Kind::Conflict => Request {
            id: if id == NonZeroU64::MIN {
                NonZeroU64::MAX
            } else {
                NonZeroU64::MIN
            },
            ..first
        },
        Kind::Mismatch => Request {
            command: amount.wrapping_add(1),
            ..first
        },
    };
    [first, second]
}

async fn seed(host: &Host, id: NonZeroU64, prefix: Option<u8>) -> Option<Version> {
    let command = prefix?;
    let prefix_id = NonZeroU64::new(if id.get() == 2 { 3 } else { 2 }).unwrap();
    let prior = host
        .submit(Request {
            id: prefix_id,
            expected: None,
            command,
        })
        .await
        .unwrap();
    assert_eq!(
        (prior.id, prior.version, prior.events),
        (prefix_id, Version::INITIAL, 1)
    );
    Some(prior.version)
}

pub async fn run(kind: Kind, winner: Winner, id: NonZeroU64, amount: u8, prefix: Option<u8>) {
    DECISIONS.store(0, Ordering::SeqCst);
    let store = Store::new(FaultStore::new(InMemoryStore::new()));
    let rival = Store::new(FaultStore::shared(Arc::clone(&store.raw().inner)));
    let first_gate = Gate::new();
    let second_gate = Gate::new();
    let limits = Limits {
        queue: NonZeroUsize::MIN,
        operation: Duration::from_secs(2),
    };
    let (one, first_worker) =
        CommandProcessor::<Counter, u8>::start(&store, String::from("race"), limits).unwrap();
    let (two, second_worker) =
        CommandProcessor::<Counter, u8>::start(&rival, String::from("race"), limits).unwrap();
    let expected = seed(&one, id, prefix).await;
    let history = History {
        before: rows(&store.raw().inner).await,
        stores: [store, rival],
        prefix,
    };
    assert_eq!(history.before.len(), usize::from(prefix.is_some()));
    assert_eq!(DECISIONS.load(Ordering::SeqCst), history.before.len());
    for (backend, gate) in history.stores.iter().zip([&first_gate, &second_gate]) {
        *backend.raw().append_gate.lock() = Some(Arc::clone(gate));
    }
    let [first, second] = requests(kind, id, amount, expected);
    let (winning, losing, gates) = match winner {
        Winner::First => (
            (one.clone(), first),
            (two.clone(), second),
            [first_gate, second_gate],
        ),
        Winner::Second => (
            (two.clone(), second),
            (one.clone(), first),
            [second_gate, first_gate],
        ),
    };
    let (accepted_by, accepted_request) = winning;
    let (rejected_by, rejected_request) = losing;
    let receipt = compete(
        &history,
        [&accepted_by, &rejected_by],
        [&accepted_request, &rejected_request],
        gates,
        kind,
    )
    .await;
    verify(
        &history,
        [&accepted_by, &rejected_by],
        [accepted_request, rejected_request],
        receipt,
        kind,
    )
    .await;
    one.stop();
    two.stop();
    assert_eq!(finish(first_worker).await.uncertain(), None);
    assert_eq!(finish(second_worker).await.uncertain(), None);
}

async fn compete(
    history: &History,
    hosts: [&Host; 2],
    originals: [&Request<u8>; 2],
    gates: [Arc<Gate>; 2],
    kind: Kind,
) -> Receipt {
    let [winner_host, loser_host] = hosts.map(Clone::clone);
    let [original, losing_original] = originals.map(Clone::clone);
    let [winning_gate, losing_gate] = gates;
    let mut first_reply = tokio::spawn(async move { winner_host.submit(original).await });
    winning_gate.entered().await;
    let mut second_reply = tokio::spawn(async move { loser_host.submit(losing_original).await });
    losing_gate.entered().await;

    assert_eq!(DECISIONS.load(Ordering::SeqCst), history.before.len() + 2);
    assert!(!first_reply.is_finished());
    assert!(!second_reply.is_finished());
    assert_eq!(rows(&history.stores[0].raw().inner).await, history.before);

    winning_gate.release();
    let completed = timeout(Duration::from_secs(2), async {
        tokio::select! {
            first = &mut first_reply => Ok(first),
            second = &mut second_reply => Err(second),
        }
    })
    .await
    .unwrap();
    let receipt = completed.unwrap().unwrap().unwrap();
    assert!(!second_reply.is_finished());
    for backend in &history.stores {
        *backend.raw().append_gate.lock() = None;
    }
    losing_gate.release();
    let loser = timeout(Duration::from_secs(2), second_reply)
        .await
        .unwrap()
        .unwrap();
    match kind {
        Kind::Duplicate => assert_eq!(loser.unwrap(), receipt),
        Kind::Conflict => assert!(matches!(loser, Err(Error::Conflict))),
        Kind::Mismatch => assert!(matches!(loser, Err(Error::Mismatch))),
    }

    receipt
}

async fn verify(
    history: &History,
    hosts: [&Host; 2],
    originals: [Request<u8>; 2],
    receipt: Receipt,
    kind: Kind,
) {
    let History {
        stores,
        before,
        prefix,
    } = history;
    let [accepted_request, rejected_request] = originals;
    let version = Version::new(if prefix.is_some() { 2 } else { 1 }).unwrap();
    assert_eq!(
        (receipt.id, receipt.version, receipt.events),
        (accepted_request.id, version, 1)
    );
    let committed = rows(&stores[0].raw().inner).await;
    assert_eq!(committed.len(), before.len() + 1);
    assert_eq!(&committed[..before.len()], before.as_slice());
    let acceptance = &committed[before.len()];
    assert_eq!(acceptance.0, version);
    assert_eq!(
        acceptance.1,
        serde_json::json!({
            "schema":2,"request":accepted_request,"receipt":receipt,"facts":[accepted_request.command]
        })
    );
    for host in hosts {
        assert_eq!(
            host.read().await.unwrap(),
            (
                Some(version),
                Total(u64::from(prefix.unwrap_or(0)) + u64::from(accepted_request.command))
            )
        );
        assert_eq!(
            host.submit(accepted_request.clone()).await.unwrap(),
            receipt
        );
        let changed = Request {
            expected: Some(version),
            ..accepted_request.clone()
        };
        assert!(matches!(host.submit(changed).await, Err(Error::Mismatch)));
    }
    let repeated = hosts[1].submit(rejected_request).await;
    match kind {
        Kind::Duplicate => assert_eq!(repeated.unwrap(), receipt),
        Kind::Conflict => assert!(matches!(repeated, Err(Error::Conflict))),
        Kind::Mismatch => assert!(matches!(repeated, Err(Error::Mismatch))),
    }
    assert_eq!(DECISIONS.load(Ordering::SeqCst), before.len() + 2);
    assert_eq!(rows(&stores[0].raw().inner).await, committed);
}
