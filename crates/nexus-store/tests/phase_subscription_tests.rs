#![cfg(all(feature = "testing", feature = "json"))]
#![allow(clippy::unwrap_used, reason = "tests")]
#![allow(clippy::expect_used, reason = "tests")]
#![allow(clippy::panic, reason = "tests")]

use std::time::Duration;

use futures::StreamExt;
use nexus::{DomainEvent, Message, Version};
use nexus_store::store::RawEventStore;
use nexus_store::testing::InMemoryStore;
use nexus_store::{
    DecodeStreamError, Encode, JsonCodec, Step, StepStreamExt, Store, StreamKey, Subscription,
    pending_envelope,
};
use serde::{Deserialize, Serialize};

const TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
enum Money {
    Deposited { amount: u64 },
}
impl Message for Money {}
impl DomainEvent for Money {
    fn name(&self) -> &'static str {
        match self {
            Self::Deposited { .. } => "Deposited",
        }
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct AcctId(String);
impl std::fmt::Display for AcctId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl AsRef<[u8]> for AcctId {
    fn as_ref(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

fn money_envelope(version: u64, event: &Money) -> nexus_store::PendingEnvelope {
    let bytes = JsonCodec::default().encode(event).unwrap();
    pending_envelope(Version::new(version).unwrap())
        .event_type(event.name())
        .payload(bytes)
        .build()
        .expect("valid envelope")
}

async fn append(store: &Store<InMemoryStore>, id: &AcctId, version: u64, ev: &Money) {
    let expected = Version::new(version - 1);
    store
        .append(
            &StreamKey::from_slice(id.as_ref()),
            expected,
            &[money_envelope(version, ev)],
        )
        .await
        .unwrap();
}

// ═══ 1. Sequence/Protocol ═══
#[tokio::test]
async fn subscribe_decoded_replays_then_caught_up_then_live() {
    let store = Store::new(InMemoryStore::new());
    let id = AcctId("acct-1".to_owned());
    append(&store, &id, 1, &Money::Deposited { amount: 100 }).await;
    append(&store, &id, 2, &Money::Deposited { amount: 200 }).await;

    let stream = Subscription::new(&store)
        .subscribe(&id, None)
        .unwrap()
        .decoded::<Money, _>(JsonCodec::default());
    tokio::pin!(stream);

    // Two decoded replay events in version order.
    for (v, amt) in [(1u64, 100u64), (2, 200)] {
        let step = tokio::time::timeout(TIMEOUT, stream.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        match step {
            Step::Event(d) => {
                assert_eq!(d.version, Version::new(v).unwrap());
                assert_eq!(d.event, Money::Deposited { amount: amt });
            }
            Step::CaughtUp => panic!("caught up before backlog drained"),
        }
    }
    // Then exactly one CaughtUp.
    let marker = tokio::time::timeout(TIMEOUT, stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(marker.is_caught_up(), "got {marker:?}");

    // A live append is a further Event.
    append(&store, &id, 3, &Money::Deposited { amount: 300 }).await;
    let live = tokio::time::timeout(TIMEOUT, stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    match live {
        Step::Event(d) => assert_eq!(d.version, Version::new(3).unwrap()),
        Step::CaughtUp => panic!("CaughtUp emitted twice"),
    }
}

// ═══ 2. Lifecycle: empty backlog → immediate CaughtUp ═══
#[tokio::test]
async fn subscribe_decoded_empty_backlog_is_immediately_caught_up() {
    let store = Store::new(InMemoryStore::new());
    let id = AcctId("empty".to_owned());
    let stream = Subscription::new(&store)
        .subscribe(&id, None)
        .unwrap()
        .decoded::<Money, _>(JsonCodec::default());
    tokio::pin!(stream);
    let first = tokio::time::timeout(TIMEOUT, stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        first.is_caught_up(),
        "empty backlog must yield CaughtUp first, got {first:?}"
    );
}

// ═══ 3. Defensive boundary: corrupt payload → Decode, not panic/Read ═══
#[tokio::test]
async fn subscribe_decoded_corrupt_payload_surfaces_decode() {
    let store = Store::new(InMemoryStore::new());
    let id = AcctId("bad".to_owned());
    let bad = pending_envelope(Version::INITIAL)
        .event_type("Deposited")
        .payload(b"not json".to_vec())
        .build()
        .unwrap();
    store
        .append(&StreamKey::from_slice(id.as_ref()), None, &[bad])
        .await
        .unwrap();

    let stream = Subscription::new(&store)
        .subscribe(&id, None)
        .unwrap()
        .decoded::<Money, _>(JsonCodec::default());
    tokio::pin!(stream);
    let item = tokio::time::timeout(TIMEOUT, stream.next())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(item, Err(DecodeStreamError::Decode(_))),
        "got {item:?}"
    );
}

// ═══ 4. Linearizability/Isolation ═══
#[tokio::test]
async fn subscribe_decoded_caught_up_once_under_concurrent_writes() {
    use std::sync::Arc;
    use tokio::sync::Barrier;

    let store = Store::new(InMemoryStore::new());
    let id = AcctId("lin".to_owned());
    append(&store, &id, 1, &Money::Deposited { amount: 1 }).await;

    let stream = Subscription::new(&store)
        .subscribe(&id, None)
        .unwrap()
        .decoded::<Money, _>(JsonCodec::default());
    tokio::pin!(stream);

    // Drain the single replay event, then the CaughtUp marker.
    let first = tokio::time::timeout(TIMEOUT, stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(first, Step::Event(d) if d.version == Version::new(1).unwrap()));
    let marker = tokio::time::timeout(TIMEOUT, stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(marker.is_caught_up());

    // Concurrent writer appends v2..=v5 after a rendezvous.
    let barrier = Arc::new(Barrier::new(2));
    let wb = Arc::clone(&barrier);
    let ws = store.clone();
    let wid = id.clone();
    let writer = tokio::spawn(async move {
        wb.wait().await;
        for v in 2..=5u64 {
            append(&ws, &wid, v, &Money::Deposited { amount: v }).await;
        }
    });
    barrier.wait().await;

    let mut versions = Vec::new();
    for _ in 0..4 {
        let step = tokio::time::timeout(TIMEOUT, stream.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        match step {
            Step::Event(d) => versions.push(d.version.as_u64()),
            Step::CaughtUp => panic!("CaughtUp emitted more than once"),
        }
    }
    writer.await.unwrap();
    assert_eq!(
        versions,
        vec![2, 3, 4, 5],
        "live events strictly monotonic, no dup/gap, no second CaughtUp"
    );
}
