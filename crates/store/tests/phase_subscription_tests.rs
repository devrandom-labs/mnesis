#![cfg(feature = "json")]
#![allow(clippy::unwrap_used, reason = "tests")]
#![allow(clippy::expect_used, reason = "tests")]
#![allow(clippy::panic, reason = "tests")]
#![allow(clippy::doc_markdown, reason = "test code: prose doc comments")]

use std::time::Duration;

use futures::{Stream, StreamExt};
use mnesis::{DomainEvent, Message, Version};
use mnesis_inmemory::InMemoryStore;
use mnesis_store::store::RawEventStore;
use mnesis_store::{
    DecodeStreamError, Decoded, DecodedStreamExt, Encode, JsonCodec, Step, StepStreamExt, Store,
    StreamKey, Subscription, pending_envelope,
};
use serde::{Deserialize, Serialize};

const TIMEOUT: Duration = Duration::from_secs(2);

/// Receive the next `Ok` item from a subscription stream, bounded by a timeout
/// so a stuck/parked cursor fails the test instead of hanging it. Works for any
/// item shape (`Step<_>`, bare envelope, `Decoded<_>`).
async fn recv<St, T, E>(s: &mut St) -> T
where
    St: Stream<Item = Result<T, E>> + Unpin,
    E: std::fmt::Debug,
{
    tokio::time::timeout(TIMEOUT, s.next())
        .await
        .expect("timed out waiting for the next stream item")
        .expect("stream ended unexpectedly (a subscription never returns None)")
        .expect("stream yielded an Err item")
}

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

// ═══ 1b. Sequence/Protocol — the CaughtUp latch survives internal scan reopens ═══

/// The live loop reopens its bounded scan every `CATCHUP_CHUNK` (1024) delivered
/// rows. A backlog well above 2× forces several reopens; the `caught_up` latch
/// must survive them so that **exactly one** `CaughtUp` is emitted, and only
/// after the *final* backlog event — never mid-backlog, never repeated. This is
/// the single most important invariant of the phase marker.
#[tokio::test]
async fn subscribe_emits_exactly_one_caughtup_across_a_large_multi_chunk_backlog() {
    const N: u64 = 2600; // > 2 × CATCHUP_CHUNK (1024)
    let store = Store::new(InMemoryStore::new());
    let id = AcctId("big".to_owned());

    // One append of the whole backlog (fast; still one event per version).
    let envs: Vec<_> = (1..=N)
        .map(|v| money_envelope(v, &Money::Deposited { amount: v }))
        .collect();
    store
        .append(&StreamKey::from_slice(id.as_ref()), None, &envs)
        .await
        .unwrap();

    let stream = Subscription::new(&store).subscribe(&id, None).unwrap();
    tokio::pin!(stream);

    // Drain exactly N events (strict version order) then exactly one CaughtUp.
    let mut caughtup_count = 0u32;
    let mut caughtup_after: Option<u64> = None;
    let mut last_version = 0u64;
    for _ in 0..=N {
        match recv(&mut stream).await {
            Step::Event(env) => {
                let v = env.version().as_u64();
                assert_eq!(v, last_version + 1, "backlog must arrive in strict order");
                assert_eq!(
                    caughtup_count, 0,
                    "no event may follow CaughtUp during catch-up"
                );
                last_version = v;
            }
            Step::CaughtUp => {
                caughtup_count += 1;
                caughtup_after = Some(last_version);
            }
        }
    }
    assert_eq!(
        caughtup_count, 1,
        "exactly one CaughtUp across the whole backlog"
    );
    assert_eq!(
        caughtup_after,
        Some(N),
        "CaughtUp lands after the final backlog event, not mid-scan"
    );
    assert_eq!(last_version, N, "every backlog event was delivered");

    // Post-boundary: two live appends each arrive as Event, no further CaughtUp.
    append(&store, &id, N + 1, &Money::Deposited { amount: 1 }).await;
    append(&store, &id, N + 2, &Money::Deposited { amount: 2 }).await;
    for expected in [N + 1, N + 2] {
        match recv(&mut stream).await {
            Step::Event(env) => assert_eq!(env.version().as_u64(), expected),
            Step::CaughtUp => panic!("CaughtUp must be emitted only once, ever"),
        }
    }
}

// ═══ 1c. Sequence/Protocol — $all replays in position order, one CaughtUp, then live ═══

/// `subscribe_all` observes events from *all* streams in `$all` (global) position
/// order: replay in strictly-ascending position, exactly one CaughtUp, then a
/// live append on a third stream with a strictly-greater position.
#[tokio::test]
async fn subscribe_all_replays_position_order_then_one_caughtup_then_live() {
    let store = Store::new(InMemoryStore::new());
    let a = AcctId("a".to_owned());
    let b = AcctId("b".to_owned());
    // Interleave across two streams so ordering is genuinely $all, not per-stream.
    append(&store, &a, 1, &Money::Deposited { amount: 1 }).await;
    append(&store, &b, 1, &Money::Deposited { amount: 2 }).await;
    append(&store, &a, 2, &Money::Deposited { amount: 3 }).await;

    let stream = Subscription::new(&store).subscribe_all(None).unwrap();
    tokio::pin!(stream);

    let mut positions = Vec::new();
    let mut keys = Vec::new();
    let mut caughtup_count = 0u32;
    for _ in 0..4 {
        match recv(&mut stream).await {
            Step::Event((pos, key, _env)) => {
                assert_eq!(
                    caughtup_count, 0,
                    "no $all event may follow CaughtUp during catch-up"
                );
                positions.push(pos.as_u64());
                keys.push(key);
            }
            Step::CaughtUp => caughtup_count += 1,
        }
    }
    assert_eq!(caughtup_count, 1, "exactly one CaughtUp over $all catch-up");
    assert_eq!(positions.len(), 3, "all three seeded events replayed");
    assert!(
        positions.windows(2).all(|w| w[0] < w[1]),
        "$all positions strictly ascending: {positions:?}"
    );
    let key_bytes: Vec<&[u8]> = keys.iter().map(StreamKey::as_bytes).collect();
    assert_eq!(
        key_bytes,
        vec![b"a".as_slice(), b"b".as_slice(), b"a".as_slice()],
        "$all items must carry the stream key they were appended to"
    );

    // Live append on a *third* stream — arrives with a strictly-greater position.
    let c = AcctId("c".to_owned());
    append(&store, &c, 1, &Money::Deposited { amount: 9 }).await;
    match recv(&mut stream).await {
        Step::Event((pos, key, _)) => {
            assert!(
                pos.as_u64() > *positions.last().unwrap(),
                "live $all position must exceed the last catch-up position"
            );
            assert_eq!(
                key.as_bytes(),
                b"c",
                "live $all item must carry the appended stream's key"
            );
        }
        Step::CaughtUp => panic!("CaughtUp emitted twice"),
    }
}

/// `subscribe_all().decoded()` keeps the `$all` position and stream-key tags
/// **beside** the decoded box and preserves the phase:
/// `Step<(AllPosition, StreamKey, Decoded<E>)>`.
#[tokio::test]
async fn subscribe_all_decoded_preserves_position_tag_and_phase() {
    let store = Store::new(InMemoryStore::new());
    let a = AcctId("a".to_owned());
    append(&store, &a, 1, &Money::Deposited { amount: 100 }).await;

    let stream = Subscription::new(&store)
        .subscribe_all(None)
        .unwrap()
        .decoded::<Money, _>(JsonCodec::default());
    tokio::pin!(stream);

    match recv(&mut stream).await {
        Step::Event((_pos, key, d)) => {
            assert_eq!(d.event, Money::Deposited { amount: 100 });
            assert_eq!(d.version, Version::new(1).unwrap());
            assert_eq!(
                key.as_bytes(),
                b"a",
                "decode must preserve the stream-key tag"
            );
        }
        Step::CaughtUp => panic!("event must precede CaughtUp"),
    }
    assert!(
        recv(&mut stream).await.is_caught_up(),
        "expected CaughtUp after the single backlog event"
    );
}

// ═══ 2. Lifecycle — resume from a checkpoint preserves phase semantics ═══

/// Resume from `Some(v)`: only the tail *strictly after* `v` is replayed, then
/// exactly one CaughtUp — no re-delivery of `v` or earlier (strict-after resume).
#[tokio::test]
async fn subscribe_resume_from_checkpoint_replays_only_the_tail_then_caughtup() {
    let store = Store::new(InMemoryStore::new());
    let id = AcctId("resume".to_owned());
    for v in 1..=3 {
        append(&store, &id, v, &Money::Deposited { amount: v }).await;
    }

    let stream = Subscription::new(&store)
        .subscribe(&id, Some(version(2)))
        .unwrap();
    tokio::pin!(stream);

    // Strictly after v2 → only v3 replays, then CaughtUp.
    match recv(&mut stream).await {
        Step::Event(env) => assert_eq!(env.version().as_u64(), 3, "resume must skip v1, v2"),
        Step::CaughtUp => panic!("v3 must replay before CaughtUp"),
    }
    assert!(
        recv(&mut stream).await.is_caught_up(),
        "CaughtUp after the tail"
    );
}

/// Resume from the head (`Some(last)`): nothing to replay → immediate CaughtUp,
/// zero re-delivery.
#[tokio::test]
async fn subscribe_resume_from_head_is_immediately_caught_up() {
    let store = Store::new(InMemoryStore::new());
    let id = AcctId("head".to_owned());
    for v in 1..=3 {
        append(&store, &id, v, &Money::Deposited { amount: v }).await;
    }

    let stream = Subscription::new(&store)
        .subscribe(&id, Some(version(3)))
        .unwrap();
    tokio::pin!(stream);
    assert!(
        recv(&mut stream).await.is_caught_up(),
        "resume from head must yield CaughtUp first with no re-delivery"
    );
    // The next item must be a genuinely-live append, not a replay of history.
    append(&store, &id, 4, &Money::Deposited { amount: 4 }).await;
    match recv(&mut stream).await {
        Step::Event(env) => assert_eq!(env.version().as_u64(), 4),
        Step::CaughtUp => panic!("CaughtUp emitted twice"),
    }
}

// ═══ 3. Defensive/composition — .events() strips phase; .events().decoded() drops it ═══

/// `.events()` drops every `CaughtUp` and unwraps `Event`, yielding bare
/// envelopes in order — the events-only path a phase-agnostic consumer uses.
#[tokio::test]
async fn events_strips_the_phase_yielding_bare_envelopes_in_order() {
    let store = Store::new(InMemoryStore::new());
    let id = AcctId("ev".to_owned());
    for v in 1..=3 {
        append(&store, &id, v, &Money::Deposited { amount: v }).await;
    }

    let stream = Subscription::new(&store)
        .subscribe(&id, None)
        .unwrap()
        .events();
    tokio::pin!(stream);

    // No CaughtUp is ever observable through .events(); events flow in order,
    // and a live append continues the same bare stream across the (hidden)
    // boundary.
    for v in 1..=3u64 {
        let env: mnesis_store::PersistedEnvelope = recv(&mut stream).await;
        assert_eq!(env.version().as_u64(), v);
    }
    append(&store, &id, 4, &Money::Deposited { amount: 4 }).await;
    let env: mnesis_store::PersistedEnvelope = recv(&mut stream).await;
    assert_eq!(
        env.version().as_u64(),
        4,
        "live event flows through .events() too"
    );
}

/// `.events().decoded()` composes the phase-strip with #249's owning decode:
/// plain `Decoded<E>` items, no phase wrapper.
#[tokio::test]
async fn events_then_decoded_yields_plain_decoded_without_phase() {
    let store = Store::new(InMemoryStore::new());
    let id = AcctId("evd".to_owned());
    append(&store, &id, 1, &Money::Deposited { amount: 111 }).await;

    let stream = Subscription::new(&store)
        .subscribe(&id, None)
        .unwrap()
        .events()
        .decoded::<Money, _>(JsonCodec::default());
    tokio::pin!(stream);

    let d: Decoded<Money> = recv(&mut stream).await;
    assert_eq!(d.event, Money::Deposited { amount: 111 });
    assert_eq!(d.version, Version::new(1).unwrap());
}

/// An adapter read error surfaces as `DecodeStreamError::Read` (not `Decode`,
/// not a panic) — the read/decode domains stay distinct through `.decoded()`.
/// A stored row with `event_type` set but a JSON payload that fails to decode is
/// the `Decode` case (covered above); here we assert the *read* domain by
/// feeding a corrupt stored frame. `InMemoryStore` surfaces a decode of a bad
/// payload as `Decode`; a genuine adapter read fault is exercised in the
/// combinator-level `step_stream_ext_tests`. This test pins the boundary that
/// the phase marker itself never turns an error into a spurious `CaughtUp`.
#[tokio::test]
async fn decode_error_item_is_not_swallowed_by_the_phase_marker() {
    let store = Store::new(InMemoryStore::new());
    let id = AcctId("errphase".to_owned());
    // One good event, then one un-decodable payload.
    append(&store, &id, 1, &Money::Deposited { amount: 1 }).await;
    let bad = pending_envelope(version(2))
        .event_type("Deposited")
        .payload(b"not json".to_vec())
        .build()
        .unwrap();
    store
        .append(&StreamKey::from_slice(id.as_ref()), Version::new(1), &[bad])
        .await
        .unwrap();

    let stream = Subscription::new(&store)
        .subscribe(&id, None)
        .unwrap()
        .decoded::<Money, _>(JsonCodec::default());
    tokio::pin!(stream);

    // v1 decodes as an Event…
    match recv(&mut stream).await {
        Step::Event(d) => assert_eq!(d.version, version(1)),
        Step::CaughtUp => panic!("v1 must decode before the boundary"),
    }
    // …v2 surfaces as an Err(Decode), NOT as CaughtUp and NOT as a panic.
    let item = tokio::time::timeout(TIMEOUT, stream.next())
        .await
        .expect("timed out")
        .expect("stream ended");
    assert!(
        matches!(item, Err(DecodeStreamError::Decode(_))),
        "a decode failure mid-backlog must be Err(Decode), got {item:?}"
    );
}

// ═══ 4. Linearizability — $all CaughtUp exactly once under concurrent multi-stream writes ═══

/// Two writers append to two different streams concurrently while a `$all`
/// subscriber drains across the catch-up→live boundary: every observed position
/// is strictly increasing (no reorder/dup/phantom) and `CaughtUp` is emitted
/// exactly once. Genuine concurrency via a 3-way `Barrier`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn subscribe_all_caughtup_once_under_concurrent_multistream_writes() {
    use std::sync::Arc;
    use tokio::sync::Barrier;

    let store = Store::new(InMemoryStore::new());
    let stream = Subscription::new(&store).subscribe_all(None).unwrap();
    tokio::pin!(stream);
    // Drain the initial (empty) backlog → immediate CaughtUp.
    assert!(
        recv(&mut stream).await.is_caught_up(),
        "empty $all backlog → CaughtUp first"
    );

    let barrier = Arc::new(Barrier::new(3));
    let mk = |name: &'static str| {
        let s = store.clone();
        let b = Arc::clone(&barrier);
        tokio::spawn(async move {
            b.wait().await;
            for v in 1..=10u64 {
                append(
                    &s,
                    &AcctId(name.to_owned()),
                    v,
                    &Money::Deposited { amount: v },
                )
                .await;
            }
        })
    };
    let w1 = mk("x");
    let w2 = mk("y");
    barrier.wait().await;

    // Consume all 20 live events; positions strictly increasing, no 2nd CaughtUp.
    let mut prev = 0u64;
    for _ in 0..20 {
        match recv(&mut stream).await {
            Step::Event((pos, _, _)) => {
                let p = pos.as_u64();
                assert!(
                    p > prev,
                    "$all positions strictly increasing: {p} after {prev}"
                );
                prev = p;
            }
            Step::CaughtUp => panic!("CaughtUp must be emitted exactly once"),
        }
    }
    w1.await.unwrap();
    w2.await.unwrap();
}

/// Small helper: a `Version` from a literal, unwrapped (all literals here are > 0).
const fn version(v: u64) -> Version {
    Version::new(v).unwrap()
}
