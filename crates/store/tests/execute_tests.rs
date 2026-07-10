//! `CommandRepository::execute` integration tests — the mandatory
//! cross-cutting categories (rule 7) over `InMemoryStore`, plus the
//! equivalence-with-manual-two-step proof.

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
use futures::future::join_all;
use nexus::{Aggregate, AggregateState, DomainEvent, Events, Handle, Message, Version, events};
use nexus_inmemory::InMemoryStore;
use nexus_store::{
    CommandRepository, Decode, Encode, ExecuteError, PersistedEnvelope, Repository, Store,
};
use tokio::sync::Barrier;

// ── Aggregate identity ────────────────────────────────────────────────────
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

// ── Events ────────────────────────────────────────────────────────────────
#[derive(Debug, Clone, PartialEq, Eq)]
enum CtrEvent {
    Added(u64),
}
impl Message for CtrEvent {}
impl DomainEvent for CtrEvent {
    fn name(&self) -> &'static str {
        match self {
            Self::Added(_) => "Added",
        }
    }
}

// ── State ─────────────────────────────────────────────────────────────────
#[derive(Debug, Clone, PartialEq, Eq)]
struct CtrState {
    total: u64,
}
impl AggregateState for CtrState {
    type Event = CtrEvent;
    fn initial() -> Self {
        Self { total: 0 }
    }
    fn apply(mut self, event: &CtrEvent) -> Self {
        match event {
            CtrEvent::Added(n) => self.total += n,
        }
        self
    }
}

#[derive(Debug, thiserror::Error, PartialEq)]
enum CtrError {
    #[error("cannot add zero")]
    Zero,
}

// ── Marker + Handle ───────────────────────────────────────────────────────
struct Counter;
impl Aggregate for Counter {
    type State = CtrState;
    type Error = CtrError;
    type Id = CtrId;
}
struct Add(u64);
impl Handle<Add> for Counter {
    fn handle(_state: &CtrState, cmd: Add) -> Result<Events<CtrEvent>, CtrError> {
        if cmd.0 == 0 {
            return Err(CtrError::Zero);
        }
        Ok(events![CtrEvent::Added(cmd.0)])
    }
}

// ── Codec: encode the u64 LE ──────────────────────────────────────────────
struct CtrCodec;
impl Encode<CtrEvent> for CtrCodec {
    type Error = Infallible;
    fn encode(&self, event: &CtrEvent) -> Result<Bytes, Infallible> {
        let CtrEvent::Added(n) = event;
        Ok(Bytes::copy_from_slice(&n.to_le_bytes()))
    }
}
impl Decode<CtrEvent> for CtrCodec {
    type Output<'a> = CtrEvent;
    type Error = Infallible;
    fn decode<'a>(&'a self, env: &'a PersistedEnvelope) -> Result<Self::Output<'a>, Infallible> {
        let mut buf = [0u8; 8];
        buf.copy_from_slice(&env.payload()[..8]);
        Ok(CtrEvent::Added(u64::from_le_bytes(buf)))
    }
}

type Repo = nexus_store::EventStore<InMemoryStore, CtrCodec, Counter>;

fn new_repo() -> Repo {
    Store::new(InMemoryStore::new())
        .repository()
        .codec(CtrCodec)
        .build()
}

// ── 1. Sequence/protocol ──────────────────────────────────────────────────
#[tokio::test]
async fn sequence_execute_chain_advances_version_and_state() {
    let repo = new_repo();
    let mut ctr = repo.load(CtrId::new(1)).await.unwrap();

    let e1 = repo.execute(&mut ctr, Add(3)).await.unwrap();
    assert_eq!(e1.iter().collect::<Vec<_>>(), vec![&CtrEvent::Added(3)]);
    assert_eq!(ctr.version(), Version::new(1));
    assert_eq!(ctr.state().total, 3);

    let e2 = repo.execute(&mut ctr, Add(4)).await.unwrap();
    assert_eq!(e2.iter().collect::<Vec<_>>(), vec![&CtrEvent::Added(4)]);
    assert_eq!(ctr.version(), Version::new(2));
    assert_eq!(ctr.state().total, 7);

    let reloaded = repo.load(CtrId::new(1)).await.unwrap();
    assert_eq!(reloaded.state().total, 7);
    assert_eq!(reloaded.version(), Version::new(2));
}

// ── Defensive boundary: rejection persists nothing ────────────────────────
#[tokio::test]
async fn defensive_rejected_command_persists_nothing() {
    let repo = new_repo();
    let mut ctr = repo.load(CtrId::new(2)).await.unwrap();

    let err = repo.execute(&mut ctr, Add(0)).await.unwrap_err();
    assert!(matches!(err, ExecuteError::Decide(CtrError::Zero)));
    assert!(!err.is_conflict());
    assert_eq!(ctr.version(), None);
    assert_eq!(ctr.state().total, 0);

    let reloaded = repo.load(CtrId::new(2)).await.unwrap();
    assert_eq!(reloaded.version(), None);
}

// ── Defensive boundary: stale root → conflict ─────────────────────────────
#[tokio::test]
async fn defensive_stale_root_conflicts() {
    let repo = new_repo();
    let mut a = repo.load(CtrId::new(3)).await.unwrap();
    let mut b = repo.load(CtrId::new(3)).await.unwrap();

    repo.execute(&mut a, Add(1)).await.unwrap();
    let err = repo.execute(&mut b, Add(2)).await.unwrap_err();
    assert!(err.is_conflict());
    assert!(matches!(err, ExecuteError::Store(_)));
}

// ── Linearizability: concurrent execute, one wins ─────────────────────────
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn linearizable_concurrent_execute_one_winner() {
    let repo = Arc::new(new_repo());
    let id = CtrId::new(4);
    let barrier = Arc::new(Barrier::new(2));

    let tasks = [10u64, 20u64].into_iter().map(|n| {
        let repo = Arc::clone(&repo);
        let id = id.clone();
        let barrier = Arc::clone(&barrier);
        tokio::spawn(async move {
            let mut ctr = repo.load(id).await.unwrap();
            barrier.wait().await; // maximize overlap on the append
            repo.execute(&mut ctr, Add(n)).await
        })
    });
    let results: Vec<_> = join_all(tasks)
        .await
        .into_iter()
        .map(|j| j.unwrap())
        .collect();

    let wins = results.iter().filter(|r| r.is_ok()).count();
    let conflicts = results
        .iter()
        .filter(|r| r.as_ref().err().is_some_and(ExecuteError::is_conflict))
        .count();
    assert_eq!(wins, 1, "exactly one writer commits");
    assert_eq!(
        conflicts, 1,
        "the loser surfaces a conflict, not a silent drop"
    );

    let final_ctr = repo.load(id).await.unwrap();
    assert_eq!(final_ctr.version(), Version::new(1));
    // Exactly one of the two commands landed — total is either 10 or 20,
    // never both (30) and never neither (0).
    assert!(
        final_ctr.state().total == 10 || final_ctr.state().total == 20,
        "expected exactly one command's effect, got {}",
        final_ctr.state().total
    );
}

// ── Equivalence: execute == manual handle + save ──────────────────────────
#[tokio::test]
async fn equivalence_execute_matches_manual_two_step() {
    let manual = new_repo();
    let mut m = manual.load(CtrId::new(5)).await.unwrap();
    let decided = m.handle::<Add, 0>(Add(9)).unwrap();
    manual.save(&mut m, &decided).await.unwrap();

    let fused = new_repo();
    let mut f = fused.load(CtrId::new(6)).await.unwrap();
    let returned = fused.execute(&mut f, Add(9)).await.unwrap();

    assert_eq!(
        returned.iter().collect::<Vec<_>>(),
        decided.iter().collect::<Vec<_>>()
    );
    assert_eq!(f.version(), m.version());
    assert_eq!(f.state(), m.state());
}
