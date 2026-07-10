//! Lifecycle test (rule 7 category 2) for `CommandRepository::execute` on the
//! real persistent `nexus-fjall` adapter: write → close → reopen → verify
//! durable, plus a rejected command that must persist nothing across the
//! reopen. `execute_tests.rs` in `nexus-store` already covers the
//! sequence/protocol, defensive-boundary, and linearizability categories over
//! `InMemoryStore`; this file is the fjall-specific durability proof that a
//! persisted `Store<S>` handle actually loses its in-memory `AggregateRoot`
//! state (and rebuilds it purely from disk) across a real close/reopen.

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

use bytes::Bytes;
use nexus::{Aggregate, AggregateState, DomainEvent, Events, Handle, Message, Version, events};
use nexus_fjall::FjallStore;
use nexus_store::{
    CommandRepository, Decode, Encode, ExecuteError, PersistedEnvelope, RawEventStore, Repository,
};

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

type Repo = nexus_store::EventStore<FjallStore, CtrCodec, Counter>;

fn open_repo(path: &std::path::Path) -> Repo {
    FjallStore::builder(path)
        .open()
        .expect("open fjall store")
        .into_store()
        .repository()
        .codec(CtrCodec)
        .build()
}

// ── Lifecycle: execute survives close + reopen; rejection persists nothing ─
#[tokio::test]
async fn lifecycle_execute_survives_reopen() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("db");

    // Phase 1: open, execute two commands, close (scope end drops the store).
    {
        let repo = open_repo(&db_path);
        let mut ctr = repo.load(CtrId::new(1)).await.unwrap();

        let e1 = repo.execute(&mut ctr, Add(3)).await.unwrap();
        assert_eq!(e1.iter().collect::<Vec<_>>(), vec![&CtrEvent::Added(3)]);

        let e2 = repo.execute(&mut ctr, Add(4)).await.unwrap();
        assert_eq!(e2.iter().collect::<Vec<_>>(), vec![&CtrEvent::Added(4)]);

        assert_eq!(ctr.state().total, 7);
        assert_eq!(ctr.version(), Version::new(2));
    }

    // Phase 2: reopen at the SAME path — durability assertion. If `execute`
    // hadn't actually persisted, this would rehydrate to version None / total 0.
    {
        let repo = open_repo(&db_path);
        let reloaded = repo.load(CtrId::new(1)).await.unwrap();
        assert_eq!(reloaded.state().total, 7);
        assert_eq!(reloaded.version(), Version::new(2));

        // Phase 3: a rejected command against the reopened store must persist
        // nothing — version stays at 2 across a second reload.
        let mut ctr = repo.load(CtrId::new(1)).await.unwrap();
        let err = repo.execute(&mut ctr, Add(0)).await.unwrap_err();
        assert!(matches!(err, ExecuteError::Decide(CtrError::Zero)));
        assert!(!err.is_conflict());

        let after = repo.load(CtrId::new(1)).await.unwrap();
        assert_eq!(after.state().total, 7);
        assert_eq!(after.version(), Version::new(2));
    }
}
