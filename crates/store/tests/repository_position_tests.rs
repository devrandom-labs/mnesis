//! `Repository::save` returns the `$all` position its events landed at (#330,
//! PR2) — the repository-seam analogue of `append_position_tests.rs`. This is
//! the read-your-writes token an application author gets without dropping to
//! `RawEventStore`: the value returned by `save` is the exact position `$all`
//! reports for the last saved event, so a projection whose checkpoint has
//! reached it has necessarily seen the write.

#![allow(clippy::unwrap_used, reason = "tests")]
#![allow(clippy::expect_used, reason = "tests")]
#![allow(clippy::panic, reason = "tests")]

use std::convert::Infallible;

use bytes::Bytes;
use futures::TryStreamExt;
use mnesis::{Aggregate, AggregateState, DomainEvent, Events, Handle, Message, Version, events};
use mnesis_inmemory::InMemoryStore;
use mnesis_store::codec::{Decode, Encode};
use mnesis_store::store::RawEventStore;
use mnesis_store::{CommandRepository, Execution, PersistedEnvelope, Repository, Store};

// ── Minimal owning-codec aggregate (no `json` feature → runs on the gate) ──

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CtrId([u8; 8]);
impl CtrId {
    const fn new(n: u64) -> Self {
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
        // A stable, non-empty byte key (the inmemory store forbids empty ids).
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Added(u64);
impl Message for Added {}
impl DomainEvent for Added {
    fn name(&self) -> &'static str {
        "Added"
    }
}

#[derive(Debug, Clone, Default)]
struct CtrState {
    total: u64,
}
impl AggregateState for CtrState {
    type Event = Added;
    fn initial() -> Self {
        Self::default()
    }
    fn apply(mut self, event: &Added) -> Self {
        self.total += event.0;
        self
    }
}

#[derive(Debug)]
struct Ctr;
impl Aggregate for Ctr {
    type State = CtrState;
    type Error = Infallible;
    type Id = CtrId;
}

#[derive(Debug)]
struct Add(u64);
impl Message for Add {}
impl Handle<Add, 0> for Ctr {
    fn handle(_state: &CtrState, cmd: Add) -> Result<Option<Events<Added, 0>>, Infallible> {
        Ok(Some(events![Added(cmd.0)]))
    }
}

struct CtrCodec;
impl Encode<Added> for CtrCodec {
    type Error = Infallible;
    fn encode(&self, event: &Added) -> Result<Bytes, Infallible> {
        Ok(Bytes::copy_from_slice(&event.0.to_le_bytes()))
    }
}
impl Decode<Added> for CtrCodec {
    type Output<'a> = Added;
    type Error = Infallible;
    fn decode<'a>(&'a self, env: &'a PersistedEnvelope) -> Result<Added, Infallible> {
        let mut buf = [0u8; 8];
        buf.copy_from_slice(&env.payload()[..8]);
        Ok(Added(u64::from_le_bytes(buf)))
    }
}

async fn last_all_position<S: RawEventStore>(store: &Store<S>) -> S::AllPosition {
    let rows: Vec<(S::AllPosition, _, PersistedEnvelope)> = store
        .read_all(None)
        .await
        .expect("read_all")
        .try_collect()
        .await
        .expect("drain $all");
    rows.last().expect("at least one $all row").0
}

#[tokio::test]
async fn save_returns_the_position_all_reports_for_the_last_saved_event() {
    let store = Store::new(InMemoryStore::new());
    let repo = store.repository::<Ctr>().codec(CtrCodec).build();

    let mut ctr = repo.load(CtrId::new(1)).await.unwrap();
    let decided: Events<Added, 1> = events![Added(3), Added(4)];
    let returned = repo.save(&mut ctr, &decided).await.unwrap();

    assert_eq!(
        returned,
        last_all_position(&store).await,
        "save must return the $all position of the last saved event"
    );
    assert_eq!(ctr.version(), Version::new(2));
}

#[tokio::test]
async fn successive_saves_return_strictly_increasing_positions() {
    let store = Store::new(InMemoryStore::new());
    let repo = store.repository::<Ctr>().codec(CtrCodec).build();

    let one: Events<Added, 0> = events![Added(1)];
    let two: Events<Added, 0> = events![Added(2)];

    let mut a = repo.load(CtrId::new(1)).await.unwrap();
    let first = repo.save(&mut a, &one).await.unwrap();

    let mut b = repo.load(CtrId::new(2)).await.unwrap();
    let second = repo.save(&mut b, &two).await.unwrap();

    assert!(
        first < second,
        "a later save lands at a strictly greater $all position"
    );
}

#[tokio::test]
async fn execute_carries_the_position_all_reports() {
    let store = Store::new(InMemoryStore::new());
    let repo = store.repository::<Ctr>().codec(CtrCodec).build();

    let mut ctr = repo.load(CtrId::new(1)).await.unwrap();
    let Execution::Executed { position, events } = repo.execute(&mut ctr, Add(7)).await.unwrap()
    else {
        panic!("expected Executed, got Ignored");
    };

    assert_eq!(events.iter().collect::<Vec<_>>(), vec![&Added(7)]);
    assert_eq!(
        position,
        last_all_position(&store).await,
        "execute must carry the $all position of the decided event"
    );
}
