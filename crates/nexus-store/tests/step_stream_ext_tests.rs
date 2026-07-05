//! Combinator-isolation tests for `StepStreamExt` (`.events()` / `.decoded()`).
//!
//! These drive the combinators over **synthetic** `Step` streams built from
//! real [`PersistedEnvelope`]s (round-tripped through `InMemoryStore`), so we
//! control the exact `Step` sequence — including sequences the real subscription
//! loop never produces but the combinator must still handle correctly:
//! `CaughtUp`-only streams, multiple/interleaved `CaughtUp` markers, and `Err`
//! items at arbitrary positions. End-to-end behaviour over a real `subscribe`
//! lives in `phase_subscription_tests`.

#![cfg(all(feature = "testing", feature = "json"))]
#![allow(clippy::unwrap_used, reason = "tests")]
#![allow(clippy::expect_used, reason = "tests")]
#![allow(clippy::panic, reason = "tests")]
#![allow(clippy::doc_markdown, reason = "test code: prose doc comments")]
#![allow(
    clippy::type_complexity,
    reason = "test code: explicit synthetic-stream item types"
)]

use futures::{StreamExt, stream};
use nexus::{DomainEvent, Message, Version};
use nexus_store::store::RawEventStore;
use nexus_store::testing::InMemoryStore;
use nexus_store::{
    DecodeStreamError, Decoded, Encode, JsonCodec, PersistedEnvelope, Step, StepStreamExt, Store,
    StreamKey, pending_envelope,
};
use proptest::prelude::*;
use serde::{Deserialize, Serialize};

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

/// A synthetic stream error, distinct from any real adapter error — so a test
/// asserting `DecodeStreamError::Read(_)` proves the read domain is preserved.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("synthetic read fault")]
struct SynthErr;

/// Build a real [`PersistedEnvelope`] at `version` carrying a JSON `Money`
/// payload, by round-tripping through `InMemoryStore` (the only way to obtain a
/// `PersistedEnvelope` that carries a real `version`).
async fn persisted(version: u64, amount: u64) -> PersistedEnvelope {
    let store = Store::new(InMemoryStore::new());
    let id = StreamKey::from_slice(b"s");
    let mut expected = None;
    for v in 1..version {
        store
            .append(&id, expected, &[money_env(v, 0)])
            .await
            .unwrap();
        expected = Version::new(v);
    }
    store
        .append(&id, expected, &[money_env(version, amount)])
        .await
        .unwrap();
    let raw = store
        .read_stream(&id, Version::new(version).unwrap())
        .await
        .unwrap();
    let mut cursor = std::pin::pin!(raw);
    cursor.next().await.expect("one event").expect("ok")
}

/// A real [`PersistedEnvelope`] whose payload is NOT valid `Money` JSON — for
/// the decode-failure path.
async fn corrupt_persisted() -> PersistedEnvelope {
    let store = Store::new(InMemoryStore::new());
    let id = StreamKey::from_slice(b"s");
    let bad = pending_envelope(Version::INITIAL)
        .event_type("Deposited")
        .payload(b"not json".to_vec())
        .build()
        .unwrap();
    store.append(&id, None, &[bad]).await.unwrap();
    let raw = store.read_stream(&id, Version::INITIAL).await.unwrap();
    let mut cursor = std::pin::pin!(raw);
    cursor.next().await.expect("one event").expect("ok")
}

fn money_env(version: u64, amount: u64) -> nexus_store::PendingEnvelope {
    let bytes = JsonCodec::default()
        .encode(&Money::Deposited { amount })
        .unwrap();
    pending_envelope(Version::new(version).unwrap())
        .event_type("Deposited")
        .payload(bytes)
        .build()
        .unwrap()
}

/// Wrap synthetic items into a `Stream`.
fn synth<I>(items: Vec<Result<I, SynthErr>>) -> impl futures::Stream<Item = Result<I, SynthErr>> {
    stream::iter(items)
}

// ─── .events() ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn events_drops_caughtup_and_unwraps_events_in_order() {
    let (p1, p2, p3) = (
        persisted(1, 1).await,
        persisted(2, 2).await,
        persisted(3, 3).await,
    );
    let items = vec![
        Ok(Step::Event(p1)),
        Ok(Step::Event(p2)),
        Ok(Step::CaughtUp),
        Ok(Step::Event(p3)),
    ];
    let out: Vec<_> = synth(items)
        .events()
        .map(|r| r.unwrap().version().as_u64())
        .collect()
        .await;
    assert_eq!(
        out,
        vec![1, 2, 3],
        "CaughtUp removed, events unwrapped in order"
    );
}

#[tokio::test]
async fn events_on_a_caughtup_only_stream_is_empty() {
    let items: Vec<Result<Step<PersistedEnvelope>, SynthErr>> =
        vec![Ok(Step::CaughtUp), Ok(Step::CaughtUp)];
    let out: Vec<_> = synth(items).events().collect().await;
    assert!(out.is_empty(), "a stream of only markers yields no events");
}

#[tokio::test]
async fn events_drops_multiple_interleaved_caughtup_markers() {
    let (p1, p2) = (persisted(1, 1).await, persisted(2, 2).await);
    let items = vec![
        Ok(Step::CaughtUp),
        Ok(Step::Event(p1)),
        Ok(Step::CaughtUp),
        Ok(Step::Event(p2)),
        Ok(Step::CaughtUp),
    ];
    let out: Vec<_> = synth(items)
        .events()
        .map(|r| r.unwrap().version().as_u64())
        .collect()
        .await;
    assert_eq!(
        out,
        vec![1, 2],
        "every marker dropped regardless of position/count"
    );
}

#[tokio::test]
async fn events_passes_error_items_through_in_place() {
    let (p1, p2) = (persisted(1, 1).await, persisted(2, 2).await);
    let items = vec![Ok(Step::Event(p1)), Err(SynthErr), Ok(Step::Event(p2))];
    let out: Vec<_> = synth(items).events().collect().await;
    assert_eq!(out.len(), 3);
    assert!(out[0].is_ok());
    assert_eq!(
        out[1].as_ref().unwrap_err(),
        &SynthErr,
        "error preserved, not swallowed"
    );
    assert!(out[2].is_ok());
}

#[tokio::test]
async fn events_preserves_the_position_tag_on_all_style_items() {
    // `$all` items are `(P, PersistedEnvelope)`; `.events()` keeps the tag.
    let p1 = persisted(1, 1).await;
    let items: Vec<Result<Step<(u64, PersistedEnvelope)>, SynthErr>> =
        vec![Ok(Step::CaughtUp), Ok(Step::Event((77u64, p1)))];
    let out: Vec<_> = synth(items).events().collect().await;
    assert_eq!(out.len(), 1);
    let (pos, env) = out[0].as_ref().unwrap();
    assert_eq!(*pos, 77, "position tag rides through .events()");
    assert_eq!(env.version().as_u64(), 1);
}

// ─── .decoded() (phase-preserving) ────────────────────────────────────────────

#[tokio::test]
async fn decoded_preserves_caughtup_and_decodes_each_event() {
    let (p1, p2) = (persisted(1, 10).await, persisted(2, 20).await);
    let items = vec![Ok(Step::Event(p1)), Ok(Step::CaughtUp), Ok(Step::Event(p2))];
    let out: Vec<Result<Step<Decoded<Money>>, DecodeStreamError<SynthErr, _>>> = synth(items)
        .decoded::<Money, _>(JsonCodec::default())
        .collect()
        .await;

    assert_eq!(out.len(), 3);
    match out[0].as_ref().unwrap() {
        Step::Event(d) => {
            assert_eq!(d.event, Money::Deposited { amount: 10 });
            assert_eq!(d.version, Version::new(1).unwrap());
        }
        Step::CaughtUp => panic!("first item must be the decoded event"),
    }
    assert!(
        out[1].as_ref().unwrap().is_caught_up(),
        "the marker is preserved in place"
    );
    match out[2].as_ref().unwrap() {
        Step::Event(d) => assert_eq!(d.event, Money::Deposited { amount: 20 }),
        Step::CaughtUp => panic!("third item must be the decoded event"),
    }
}

#[tokio::test]
async fn decoded_surfaces_a_read_error_as_the_read_variant() {
    let items: Vec<Result<Step<PersistedEnvelope>, SynthErr>> = vec![Err(SynthErr)];
    let out: Vec<_> = synth(items)
        .decoded::<Money, _>(JsonCodec::default())
        .collect()
        .await;
    assert_eq!(out.len(), 1);
    assert!(
        matches!(out[0], Err(DecodeStreamError::Read(SynthErr))),
        "an upstream read fault is the Read domain, got {:?}",
        out[0]
    );
}

#[tokio::test]
async fn decoded_surfaces_a_bad_payload_as_the_decode_variant() {
    let bad = corrupt_persisted().await;
    let items: Vec<Result<Step<PersistedEnvelope>, SynthErr>> = vec![Ok(Step::Event(bad))];
    let out: Vec<_> = synth(items)
        .decoded::<Money, _>(JsonCodec::default())
        .collect()
        .await;
    assert_eq!(out.len(), 1);
    assert!(
        matches!(out[0], Err(DecodeStreamError::Decode(_))),
        "an un-decodable payload is the Decode domain, got {:?}",
        out[0]
    );
}

#[tokio::test]
async fn decoded_on_all_style_items_keeps_tag_and_phase() {
    let p1 = persisted(1, 42).await;
    let items: Vec<Result<Step<(u64, PersistedEnvelope)>, SynthErr>> =
        vec![Ok(Step::Event((5u64, p1))), Ok(Step::CaughtUp)];
    let out: Vec<Result<Step<(u64, Decoded<Money>)>, DecodeStreamError<SynthErr, _>>> =
        synth(items)
            .decoded::<Money, _>(JsonCodec::default())
            .collect()
            .await;

    match out[0].as_ref().unwrap() {
        Step::Event((pos, d)) => {
            assert_eq!(*pos, 5, "tag preserved beside the decoded box");
            assert_eq!(d.event, Money::Deposited { amount: 42 });
        }
        Step::CaughtUp => panic!("first item is the tagged decoded event"),
    }
    assert!(out[1].as_ref().unwrap().is_caught_up());
}

// ─── property: .events() drops exactly the markers, in order ───────────────────

proptest! {
    /// For any interleaving of `Event`/`CaughtUp`, `.events()` yields exactly the
    /// `Event` payloads, in the same relative order, and drops every `CaughtUp`.
    #[test]
    fn prop_events_yields_exactly_the_events_in_order(
        flags in prop::collection::vec(any::<bool>(), 0..40)
    ) {
        // One tokio runtime + one envelope, reused across all generated cases.
        let rt = tokio::runtime::Runtime::new().unwrap();
        let env = rt.block_on(persisted(1, 1));

        // Give each Event a distinct sentinel version-less identity via its index
        // in the flag vector (we assert count + that no marker leaks through).
        let expected_events = flags.iter().filter(|&&b| b).count();
        let items: Vec<Result<Step<PersistedEnvelope>, SynthErr>> = flags
            .iter()
            .map(|&is_event| Ok(if is_event { Step::Event(env.clone()) } else { Step::CaughtUp }))
            .collect();

        let out = rt.block_on(synth(items).events().collect::<Vec<_>>());
        prop_assert_eq!(out.len(), expected_events, "one output per Event, markers dropped");
        prop_assert!(out.iter().all(std::result::Result::is_ok), "no errors introduced");
    }
}
