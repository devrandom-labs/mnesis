#![cfg(feature = "json")]
#![allow(clippy::unwrap_used, reason = "tests")]
#![allow(clippy::expect_used, reason = "tests")]
#![allow(clippy::panic, reason = "tests")]

use std::time::Duration;

use futures::StreamExt;
use mnesis::{DomainEvent, Message, Version};
use mnesis_inmemory::InMemoryStore;
use mnesis_store::store::RawEventStore;
use mnesis_store::{
    Decode, DecodeStreamError, Decoded, DecodedStreamExt, Encode, FoldDecodedError, JsonCodec,
    PersistedEnvelope, StepStreamExt, Store, StreamKey, Subscription, pending_envelope,
};
use serde::{Deserialize, Serialize};

const TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
enum Money {
    Deposited { amount: u64 },
    Withdrew { amount: u64 },
}
impl Message for Money {}
impl DomainEvent for Money {
    fn name(&self) -> &'static str {
        match self {
            Self::Deposited { .. } => "Deposited",
            Self::Withdrew { .. } => "Withdrew",
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

fn money_envelope(version: u64, event: &Money) -> mnesis_store::PendingEnvelope {
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
async fn decoded_catchup_then_live_reuses_the_codec() {
    let store = Store::new(InMemoryStore::new());
    let id = AcctId("acct-1".to_owned());

    append(&store, &id, 1, &Money::Deposited { amount: 1000 }).await;
    append(&store, &id, 2, &Money::Withdrew { amount: 400 }).await;

    let stream = Subscription::new(&store)
        .subscribe(&id, None)
        .unwrap()
        .events()
        .decoded::<Money, _>(JsonCodec::default());
    tokio::pin!(stream);

    let d1 = tokio::time::timeout(TIMEOUT, stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(d1.event, Money::Deposited { amount: 1000 });
    assert_eq!(d1.version, Version::new(1).unwrap());
    let d2 = tokio::time::timeout(TIMEOUT, stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(d2.event, Money::Withdrew { amount: 400 });
    assert_eq!(d2.version, Version::new(2).unwrap());

    append(&store, &id, 3, &Money::Deposited { amount: 250 }).await;
    let d3 = tokio::time::timeout(TIMEOUT, stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(d3.event, Money::Deposited { amount: 250 });
    assert_eq!(d3.version, Version::new(3).unwrap());
}

// ═══ 3. Defensive Boundary ═══
#[tokio::test]
async fn corrupt_payload_surfaces_decode_not_panic_not_read() {
    let store = Store::new(InMemoryStore::new());
    let id = AcctId("acct-bad".to_owned());
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
        .events()
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

#[tokio::test]
async fn read_error_item_surfaces_read_variant() {
    #[derive(Debug, thiserror::Error)]
    #[error("adapter boom")]
    struct Boom;

    let pending = money_envelope(1, &Money::Deposited { amount: 5 });
    let good = {
        let store = Store::new(InMemoryStore::new());
        let id = AcctId("x".to_owned());
        store
            .append(&StreamKey::from_slice(id.as_ref()), None, &[pending])
            .await
            .unwrap();
        let mut s = std::pin::pin!(
            store
                .read_stream(&StreamKey::from_slice(id.as_ref()), Version::INITIAL)
                .await
                .unwrap()
        );
        s.next().await.unwrap().unwrap()
    };

    let raw = futures::stream::iter(vec![Ok::<PersistedEnvelope, Boom>(good), Err(Boom)]);
    let typed = raw.decoded::<Money, _>(JsonCodec::default());
    tokio::pin!(typed);

    let first = typed.next().await.unwrap();
    assert!(first.is_ok(), "got {first:?}");
    let second = typed.next().await.unwrap();
    assert!(
        matches!(second, Err(DecodeStreamError::Read(Boom))),
        "got {second:?}"
    );
}

#[tokio::test]
async fn decoded_all_preserves_the_position_and_key_beside_the_box() {
    let store = Store::new(InMemoryStore::new());
    let id = AcctId("acct-all".to_owned());
    append(&store, &id, 1, &Money::Deposited { amount: 1 }).await;

    let stream = Subscription::new(&store)
        .subscribe_all(None)
        .unwrap()
        .events()
        .decoded::<Money, _>(JsonCodec::default());
    tokio::pin!(stream);

    let (_pos, key, d) = tokio::time::timeout(TIMEOUT, stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(d.event, Money::Deposited { amount: 1 });
    assert_eq!(d.version, Version::new(1).unwrap());
    assert_eq!(
        key.as_bytes(),
        b"acct-all",
        "$all decoded item must carry the stream key it was appended to"
    );
}

// ═══ for_each_decoded: owning codec folds typed state ═══
#[tokio::test]
async fn for_each_decoded_folds_owning_events() {
    let store = Store::new(InMemoryStore::new());
    let id = AcctId("fe-1".to_owned());
    append(&store, &id, 1, &Money::Deposited { amount: 1000 }).await;
    append(&store, &id, 2, &Money::Withdrew { amount: 400 }).await;

    // Bounded: read the finite history via read_stream (terminates), not the
    // never-ending subscription.
    let raw = std::pin::pin!(
        store
            .read_stream(&StreamKey::from_slice(id.as_ref()), Version::INITIAL)
            .await
            .unwrap()
    );
    let mut balance: i64 = 0;
    let mut last = Version::INITIAL;
    raw.for_each_decoded::<Money, _, _, std::convert::Infallible>(
        JsonCodec::default(),
        |d: Decoded<Money>| {
            match d.event {
                Money::Deposited { amount } => balance += i64::try_from(amount).unwrap(),
                Money::Withdrew { amount } => balance -= i64::try_from(amount).unwrap(),
            }
            last = d.version;
            Ok(())
        },
    )
    .await
    .unwrap();

    assert_eq!(balance, 600);
    assert_eq!(last, Version::new(2).unwrap());
}

// ═══ for_each_decoded: BORROWING codec (zero-copy path, no rkyv feature) ═══
// A codec whose Output borrows the envelope — the KERI rkyv shape, proven with
// a dependency-free stand-in.
struct RawBytesCodec;
impl Decode<[u8]> for RawBytesCodec {
    type Output<'a> = &'a [u8];
    type Error = std::convert::Infallible;
    fn decode<'a>(&'a self, env: &'a PersistedEnvelope) -> Result<&'a [u8], Self::Error> {
        Ok(env.payload())
    }
}

#[tokio::test]
async fn for_each_decoded_folds_borrowed_windows() {
    let store = Store::new(InMemoryStore::new());
    let id = AcctId("fe-zc".to_owned());
    append(&store, &id, 1, &Money::Deposited { amount: 1000 }).await;

    let raw = std::pin::pin!(
        store
            .read_stream(&StreamKey::from_slice(id.as_ref()), Version::INITIAL)
            .await
            .unwrap()
    );
    let mut seen_len = 0usize;
    raw.for_each_decoded::<[u8], _, _, std::convert::Infallible>(
        RawBytesCodec,
        |d: Decoded<&[u8]>| {
            // `d.event` is a window borrowing the envelope — zero copy.
            seen_len = d.event.len();
            assert_eq!(d.version, Version::new(1).unwrap());
            Ok(())
        },
    )
    .await
    .unwrap();

    assert!(seen_len > 0);
}

// ═══ Handler error maps to the Handler variant (not Decode, not Read) ═══
#[tokio::test]
async fn for_each_decoded_surfaces_handler_error() {
    #[derive(Debug, thiserror::Error)]
    #[error("stop")]
    struct Stop;

    let store = Store::new(InMemoryStore::new());
    let id = AcctId("fe-h".to_owned());
    append(&store, &id, 1, &Money::Deposited { amount: 1 }).await;

    let raw = std::pin::pin!(
        store
            .read_stream(&StreamKey::from_slice(id.as_ref()), Version::INITIAL)
            .await
            .unwrap()
    );
    let out = raw
        .for_each_decoded::<Money, _, _, Stop>(JsonCodec::default(), |_d: Decoded<Money>| Err(Stop))
        .await;
    assert!(
        matches!(out, Err(FoldDecodedError::Handler(Stop))),
        "got {out:?}"
    );
}

// ═══ 2. Lifecycle ═══
#[tokio::test]
async fn decoded_resume_from_checkpoint_decodes_the_tail() {
    let store = Store::new(InMemoryStore::new());
    let id = AcctId("life-1".to_owned());
    append(&store, &id, 1, &Money::Deposited { amount: 1 }).await;
    append(&store, &id, 2, &Money::Deposited { amount: 2 }).await;
    append(&store, &id, 3, &Money::Deposited { amount: 3 }).await;

    // Resume strictly after v2 → the typed view must begin at v3.
    let stream = Subscription::new(&store)
        .subscribe(&id, Version::new(2))
        .unwrap()
        .events()
        .decoded::<Money, _>(JsonCodec::default());
    tokio::pin!(stream);

    let d = tokio::time::timeout(TIMEOUT, stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(d.version, Version::new(3).unwrap());
    assert_eq!(d.event, Money::Deposited { amount: 3 });
}

// ═══ 4. Linearizability/Isolation ═══
#[tokio::test]
async fn decoded_observes_concurrent_writes_in_order_no_dup() {
    use std::sync::Arc;
    use tokio::sync::Barrier;

    let store = Store::new(InMemoryStore::new());
    let id = AcctId("lin-1".to_owned());
    append(&store, &id, 1, &Money::Deposited { amount: 1 }).await;

    let stream = Subscription::new(&store)
        .subscribe(&id, None)
        .unwrap()
        .events()
        .decoded::<Money, _>(JsonCodec::default());
    tokio::pin!(stream);

    // Drain v1, then rendezvous with a writer that appends v2..=v6 concurrently.
    let first = tokio::time::timeout(TIMEOUT, stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(first.version, Version::new(1).unwrap());

    let barrier = Arc::new(Barrier::new(2));
    let wb = Arc::clone(&barrier);
    let ws = store.clone();
    let wid = id.clone();
    let writer = tokio::spawn(async move {
        wb.wait().await;
        for v in 2..=6u64 {
            append(&ws, &wid, v, &Money::Deposited { amount: v }).await;
        }
    });

    barrier.wait().await;
    let mut versions = Vec::new();
    for _ in 0..5 {
        let d = tokio::time::timeout(TIMEOUT, stream.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        versions.push(d.version.as_u64());
    }
    writer.await.unwrap();

    // Strictly monotonic, no duplicates, exact set.
    assert_eq!(versions, vec![2, 3, 4, 5, 6]);
}
