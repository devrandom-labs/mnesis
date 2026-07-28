//! Builder-level metadata provider tests (#344).
//!
//! Covers the four rule-7 categories for the typed facade's new
//! `.metadata(provider)` slot, plus inheritance proofs that the provider
//! rides through `SagaRepository::react_and_save` and
//! `CommandRepository::execute` (both call `Repository::save` internally).

#![allow(clippy::unwrap_used, reason = "tests")]
#![allow(clippy::expect_used, reason = "tests")]
#![allow(clippy::panic, reason = "panic in test match arms is an assertion")]
#![allow(
    clippy::shadow_reuse,
    reason = "the spawn-closure clone-and-shadow pattern is idiomatic for tokio tests"
)]
#![allow(
    clippy::missing_const_for_fn,
    reason = "test-helper fns need not be const"
)]

use std::convert::Infallible;
use std::sync::Arc;

use bytes::Bytes;
use futures::TryStreamExt;
use futures::future::join_all;
use mnesis::{
    Aggregate, AggregateState, DomainEvent, Events, Handle, Message, React, Saga, Version, events,
};
use mnesis_inmemory::InMemoryStore;
use mnesis_store::store::RawEventStore;
use mnesis_store::value::{Metadata, Payload};
use mnesis_store::{
    CommandRepository, Decode, Encode, Execution, PersistedEnvelope, Reaction, Repository,
    SagaRepository, Store,
};
use parking_lot::Mutex;
use tokio::sync::Barrier;

// ═══════════════════════════════════════════════════════════════════════════
// Minimal owning-codec aggregate and saga vocabulary
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CtrId([u8; 8]);
impl CtrId {
    fn new(n: u64) -> Self {
        Self(n.to_le_bytes())
    }
}
impl core::fmt::Display for CtrId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", u64::from_le_bytes(self.0))
    }
}
impl AsRef<[u8]> for CtrId {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CtrEvent {
    Added(u64),
    Cleared,
}
impl Message for CtrEvent {}
impl DomainEvent for CtrEvent {
    fn name(&self) -> &'static str {
        match self {
            Self::Added(_) => "Added",
            Self::Cleared => "Cleared",
        }
    }
}

#[derive(Debug, Clone, Default)]
struct CtrState {
    total: u64,
}
impl AggregateState for CtrState {
    type Event = CtrEvent;
    fn initial() -> Self {
        Self::default()
    }
    fn apply(mut self, event: &CtrEvent) -> Self {
        match event {
            CtrEvent::Added(n) => self.total += n,
            CtrEvent::Cleared => self.total = 0,
        }
        self
    }
}

#[derive(Debug)]
struct Counter;
impl Aggregate for Counter {
    type State = CtrState;
    type Error = Infallible;
    type Id = CtrId;
}

#[derive(Debug)]
struct Add(u64);
impl Message for Add {}
impl Handle<Add> for Counter {
    fn handle(_state: &CtrState, cmd: Add) -> Result<Option<Events<CtrEvent>>, Infallible> {
        Ok(Some(events![CtrEvent::Added(cmd.0)]))
    }
}

struct CtrCodec;
impl Encode<CtrEvent> for CtrCodec {
    type Error = Infallible;
    fn encode(&self, event: &CtrEvent) -> Result<Bytes, Infallible> {
        let byte: u8 = match event {
            CtrEvent::Added(n) => u8::try_from(*n % 251).unwrap_or(0),
            CtrEvent::Cleared => 255,
        };
        Ok(Bytes::copy_from_slice(&[byte]))
    }
}
impl Decode<CtrEvent> for CtrCodec {
    type Output<'a> = CtrEvent;
    type Error = Infallible;
    fn decode<'a>(&'a self, env: &'a PersistedEnvelope) -> Result<CtrEvent, Infallible> {
        let byte = env.payload().first().copied().unwrap_or(0);
        Ok(if byte == 255 {
            CtrEvent::Cleared
        } else {
            CtrEvent::Added(u64::from(byte))
        })
    }
}

// ── Saga vocabulary for inheritance proof ──────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct OrderId([u8; 8]);
impl OrderId {
    fn new(n: u64) -> Self {
        Self(n.to_le_bytes())
    }
}
impl core::fmt::Display for OrderId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", u64::from_le_bytes(self.0))
    }
}
impl AsRef<[u8]> for OrderId {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SagaEvent {
    PaymentRequested,
}
impl Message for SagaEvent {}
impl DomainEvent for SagaEvent {
    fn name(&self) -> &'static str {
        "PaymentRequested"
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TakePayment;
impl Message for TakePayment {}

#[derive(Debug)]
struct OrderSagaState;
impl AggregateState for OrderSagaState {
    type Event = SagaEvent;
    fn initial() -> Self {
        Self
    }
    fn apply(self, _event: &SagaEvent) -> Self {
        self
    }
}

#[derive(Debug)]
struct OrderSaga;
impl Aggregate for OrderSaga {
    type State = OrderSagaState;
    type Error = Infallible;
    type Id = OrderId;
}
impl Saga for OrderSaga {
    type CorrelationKey = u64;
    type Command = TakePayment;
    fn intent_for(_event: &SagaEvent) -> Option<TakePayment> {
        Some(TakePayment)
    }
}

#[derive(Debug)]
struct OrderPlaced {
    id: u64,
}
impl Message for OrderPlaced {}
impl DomainEvent for OrderPlaced {
    fn name(&self) -> &'static str {
        "OrderPlaced"
    }
}
impl React<OrderPlaced> for OrderSaga {
    fn correlate(event: &OrderPlaced) -> Option<u64> {
        Some(event.id)
    }
    fn react(
        _state: &OrderSagaState,
        _event: &OrderPlaced,
    ) -> Result<Option<Events<SagaEvent>>, Infallible> {
        Ok(Some(events![SagaEvent::PaymentRequested]))
    }
}

struct SagaCodec;
impl Encode<SagaEvent> for SagaCodec {
    type Error = Infallible;
    fn encode(&self, _event: &SagaEvent) -> Result<Bytes, Infallible> {
        Ok(Bytes::copy_from_slice(&[0]))
    }
}
impl Decode<SagaEvent> for SagaCodec {
    type Output<'a> = SagaEvent;
    type Error = Infallible;
    fn decode<'a>(&'a self, env: &'a PersistedEnvelope) -> Result<SagaEvent, Infallible> {
        // Only one saga event variant exists in this test vocabulary.
        let _ = env.payload().first();
        Ok(SagaEvent::PaymentRequested)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Read helpers
// ═══════════════════════════════════════════════════════════════════════════

async fn stream_envelopes(store: &Store<InMemoryStore>, id: &CtrId) -> Vec<PersistedEnvelope> {
    store
        .raw()
        .read_stream(
            &mnesis_store::StreamKey::from_slice(id.as_ref()),
            Version::INITIAL,
        )
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap()
}

async fn all_envelopes(store: &Store<InMemoryStore>) -> Vec<PersistedEnvelope> {
    store
        .raw()
        .read_all(None)
        .await
        .unwrap()
        .map_ok(|(_, _, env)| env)
        .try_collect()
        .await
        .unwrap()
}

// ═══════════════════════════════════════════════════════════════════════════
// 1. Sequence / protocol
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn provider_metadata_round_trips_with_exact_versions() {
    let store = Store::new(InMemoryStore::new());
    let observed: Arc<Mutex<Vec<Version>>> = Arc::new(Mutex::new(Vec::new()));

    let repo = store
        .repository::<Counter>()
        .codec(CtrCodec)
        .metadata({
            let observed = Arc::clone(&observed);
            move |version: Version, event: &CtrEvent, _payload: &Payload| {
                observed.lock().push(version);
                let bytes = match event {
                    CtrEvent::Added(n) => format!("v{version}:Added{n}").into_bytes(),
                    CtrEvent::Cleared => format!("v{version}:Cleared").into_bytes(),
                };
                Some(Metadata::from_bytes(Bytes::from(bytes)).expect("valid metadata"))
            }
        })
        .build();

    let mut ctr = repo.load(CtrId::new(1)).await.unwrap();
    repo.save::<2>(
        &mut ctr,
        &events![CtrEvent::Added(10), CtrEvent::Added(20), CtrEvent::Cleared],
    )
    .await
    .unwrap();

    let envs = stream_envelopes(&store, &CtrId::new(1)).await;
    assert_eq!(envs.len(), 3);

    let want_meta = [
        b"v1:Added10".as_slice(),
        b"v2:Added20".as_slice(),
        b"v3:Cleared".as_slice(),
    ];
    for (env, want) in envs.iter().zip(want_meta.iter()) {
        assert_eq!(env.metadata_bytes().unwrap().as_ref(), *want);
    }

    let versions = observed.lock().clone();
    assert_eq!(
        versions,
        vec![
            Version::INITIAL,
            Version::new(2).unwrap(),
            Version::new(3).unwrap()
        ]
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 2. No-op default
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn default_provider_leaves_metadata_absent() {
    let store = Store::new(InMemoryStore::new());
    let repo = store.repository::<Counter>().codec(CtrCodec).build();

    let mut ctr = repo.load(CtrId::new(2)).await.unwrap();
    repo.save::<1>(&mut ctr, &events![CtrEvent::Added(1), CtrEvent::Added(2)])
        .await
        .unwrap();

    let envs = stream_envelopes(&store, &CtrId::new(2)).await;
    assert_eq!(envs.len(), 2);
    for env in &envs {
        assert!(
            env.metadata().is_none(),
            "default provider must produce absent metadata"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 3. Boundary — mixed Some/None per event
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn provider_mixed_some_and_none_per_event() {
    let store = Store::new(InMemoryStore::new());
    let repo = store
        .repository::<Counter>()
        .codec(CtrCodec)
        .metadata(
            |_version: Version, event: &CtrEvent, _payload: &Payload| match event {
                CtrEvent::Added(_) => None,
                CtrEvent::Cleared => {
                    Some(Metadata::from_bytes(Bytes::from_static(b"reset-marker")).expect("valid"))
                }
            },
        )
        .build();

    let mut ctr = repo.load(CtrId::new(3)).await.unwrap();
    repo.save::<2>(
        &mut ctr,
        &events![CtrEvent::Added(1), CtrEvent::Added(2), CtrEvent::Cleared],
    )
    .await
    .unwrap();

    let envs = stream_envelopes(&store, &CtrId::new(3)).await;
    assert_eq!(envs.len(), 3);
    assert!(envs[0].metadata().is_none());
    assert!(envs[1].metadata().is_none());
    assert_eq!(envs[2].metadata_bytes().unwrap().as_ref(), b"reset-marker");
}

// ═══════════════════════════════════════════════════════════════════════════
// 4. Boundary — cap
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn provider_large_metadata_round_trips() {
    // `MAX_METADATA_LEN` is `u32::MAX - 1`; allocating that in a test is not
    // practical. We exercise a large in-process boundary (4 KiB) and verify the
    // validated constructor still enforces the empty-metadata rule.
    let large_payload: Vec<u8> = (0..4_096u32)
        .map(|i| u8::try_from(i % 251).unwrap_or(0))
        .collect();
    let store = Store::new(InMemoryStore::new());
    let repo = store
        .repository::<Counter>()
        .codec(CtrCodec)
        .metadata({
            let large_payload = large_payload.clone();
            move |_version: Version, _event: &CtrEvent, _payload: &Payload| {
                Some(Metadata::from_bytes(Bytes::from(large_payload.clone())).expect("valid"))
            }
        })
        .build();

    let mut ctr = repo.load(CtrId::new(4)).await.unwrap();
    repo.save::<0>(&mut ctr, &events![CtrEvent::Added(1)])
        .await
        .unwrap();

    let envs = stream_envelopes(&store, &CtrId::new(4)).await;
    assert_eq!(envs.len(), 1);
    assert_eq!(
        envs[0].metadata_bytes().unwrap().as_ref(),
        large_payload.as_slice()
    );
}

#[tokio::test]
async fn metadata_constructor_rejects_empty() {
    let err = Metadata::from_bytes(Bytes::new()).expect_err("empty metadata rejected");
    assert!(matches!(
        err,
        mnesis_store::value::ValueError::MetadataEmpty
    ));
}

// ═══════════════════════════════════════════════════════════════════════════
// 5. Linearizability
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_conflicting_save_loser_metadata_never_lands() {
    // Two repos share the same underlying store via `Store::clone` but stamp
    // distinct per-task metadata. Exactly one append wins; the loser's marker
    // must be absent from every envelope in the stream.
    let shared_store = Store::new(InMemoryStore::new());

    let markers: Vec<Bytes> = vec![Bytes::from_static(b"task-a"), Bytes::from_static(b"task-b")];

    let barrier = Arc::new(Barrier::new(2));
    let tasks: Vec<_> = markers
        .into_iter()
        .enumerate()
        .map(|(idx, marker)| {
            let store = shared_store.clone();
            let barrier = Arc::clone(&barrier);
            tokio::spawn(async move {
                let marker_for_result = marker.clone();
                let repo = store
                    .repository::<Counter>()
                    .codec(CtrCodec)
                    .metadata(
                        move |_version: Version, _event: &CtrEvent, _payload: &Payload| {
                            Some(Metadata::from_bytes(marker.clone()).expect("valid"))
                        },
                    )
                    .build();
                let mut ctr = repo.load(CtrId::new(5)).await.unwrap();
                barrier.wait().await;
                let result = repo
                    .save::<0>(
                        &mut ctr,
                        &events![CtrEvent::Added(u64::try_from(idx + 1).unwrap())],
                    )
                    .await;
                (marker_for_result, result)
            })
        })
        .collect();

    let results: Vec<_> = join_all(tasks)
        .await
        .into_iter()
        .map(|j| j.unwrap())
        .collect();

    let wins: Vec<_> = results
        .iter()
        .filter(|(_, r)| r.is_ok())
        .map(|(m, _)| m.as_ref())
        .collect();
    assert_eq!(wins.len(), 1, "exactly one writer commits");

    let all = all_envelopes(&shared_store).await;
    assert_eq!(all.len(), 1);
    let committed_meta = all[0].metadata_bytes().unwrap();
    assert_eq!(
        committed_meta.as_ref(),
        wins[0],
        "committed event must carry the winner's marker"
    );

    let loser = results
        .iter()
        .find(|(_, r)| r.is_err())
        .map(|(m, _)| m.as_ref())
        .expect("one loser");
    assert_ne!(
        committed_meta.as_ref(),
        loser,
        "loser's metadata must never appear in the committed stream"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 6. Inheritance — saga
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn saga_react_and_save_inherits_metadata_provider() {
    let store = Store::new(InMemoryStore::new());
    let repo = store
        .repository::<OrderSaga>()
        .codec(SagaCodec)
        .metadata(
            |_version: Version, _event: &SagaEvent, _payload: &Payload| {
                Some(Metadata::from_bytes(Bytes::from_static(b"saga-meta")).expect("valid"))
            },
        )
        .build();

    let reaction: Reaction<OrderSaga, _, 0> = repo
        .dispatch(OrderId::new(6), &OrderPlaced { id: 6 })
        .await
        .unwrap();
    assert!(matches!(reaction, Reaction::Reacted { .. }));

    let envs = store
        .raw()
        .read_stream(
            &mnesis_store::StreamKey::from_slice(OrderId::new(6).as_ref()),
            Version::INITIAL,
        )
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    assert_eq!(envs.len(), 1);
    assert_eq!(envs[0].metadata_bytes().unwrap().as_ref(), b"saga-meta");
}

// ═══════════════════════════════════════════════════════════════════════════
// 7. Inheritance — execute
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn command_execute_inherits_metadata_provider() {
    let store = Store::new(InMemoryStore::new());
    let repo = store
        .repository::<Counter>()
        .codec(CtrCodec)
        .metadata(|_version: Version, _event: &CtrEvent, _payload: &Payload| {
            Some(Metadata::from_bytes(Bytes::from_static(b"cmd-meta")).expect("valid"))
        })
        .build();

    let mut ctr = repo.load(CtrId::new(7)).await.unwrap();
    let execution = repo.execute(&mut ctr, Add(5)).await.unwrap();
    assert!(matches!(execution, Execution::Executed { .. }));

    let envs = stream_envelopes(&store, &CtrId::new(7)).await;
    assert_eq!(envs.len(), 1);
    assert_eq!(envs[0].metadata_bytes().unwrap().as_ref(), b"cmd-meta");
}
