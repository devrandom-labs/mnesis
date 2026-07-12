//! Shared toy test domains for mnesis's `tests/` integration targets (#239).
//!
//! Before this crate, roughly half a dozen integration-test files each redefined
//! the *same* toy `Counter` aggregate from scratch — an `Incremented` /
//! `Decremented` / `Set` event, a `value: i64` state, a `String`-backed id, and a
//! unit error. CLAUDE.md rule 8 requires each invariant be tested once in a
//! canonical location; this crate is that location for the near-identical
//! enum-`Counter` domain and the id/error boilerplate that travelled with it.
//!
//! # Scope
//!
//! Only the *genuinely duplicated* pieces live here. Purpose-built domains whose
//! tests assert on bespoke semantics stay local to their test files by design:
//! the fixture's `u64` overflow counter, the aggregate-root negative-guard
//! counter, and the zero-copy `#[repr(C)]` delta event each pin behaviour this
//! canonical domain deliberately does not model.
//!
//! # Reuse mechanism
//!
//! Consumed as a path-only dev-dependency (a legal dev-dep cycle — this crate
//! depends on `mnesis`, and the `mnesis` / `mnesis-store` crates dev-depend back
//! on it). That unifies the domain type across each crate's `tests/` integration
//! targets, which is where every duplicated definition lived.

use mnesis::Aggregate;
use mnesis::AggregateState;
use mnesis::DomainEvent;
use mnesis::Message;

/// Canonical `String`-backed test id.
///
/// Replaces the per-file `CounterId(String)` / `PId(String)` newtypes. A
/// `String` backing satisfies `Id`'s `AsRef<[u8]>` (via the bytes of the string)
/// and is the shape the store tests need for stream keys; the kernel tests only
/// require *some* `Id`, so it serves them too. `Id` itself is satisfied for free
/// via mnesis's blanket impl — no hand-written `impl Id`.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct TestId(String);

impl TestId {
    /// Construct from any string-like value (`TestId::new("acc-1")`).
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Construct a `p-{n}` numbered id — the pattern the kernel property tests
    /// used to mint stable ids from a loop counter.
    #[must_use]
    pub fn numbered(n: u64) -> Self {
        Self(format!("p-{n}"))
    }
}

impl core::fmt::Display for TestId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<[u8]> for TestId {
    fn as_ref(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

/// The canonical toy counter event.
///
/// A superset of the variants the consolidated files used: `snapshot_integration`
/// exercises only `Incremented` / `Decremented`, while `repository_qa` and the
/// kernel property tests also drive `Set`. `Set` carries an `i64` so it can name
/// any reachable counter value.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CounterEvent {
    /// Add one.
    Incremented,
    /// Subtract one.
    Decremented,
    /// Overwrite the running value.
    Set(i64),
}

impl Message for CounterEvent {}

impl DomainEvent for CounterEvent {
    fn name(&self) -> &'static str {
        match self {
            Self::Incremented => "Incremented",
            Self::Decremented => "Decremented",
            Self::Set(_) => "Set",
        }
    }
}

/// The running counter value folded from [`CounterEvent`]s.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CounterState {
    /// Current value.
    pub value: i64,
}

impl AggregateState for CounterState {
    type Event = CounterEvent;

    fn initial() -> Self {
        Self::default()
    }

    fn apply(mut self, event: &CounterEvent) -> Self {
        match event {
            CounterEvent::Incremented => self.value += 1,
            CounterEvent::Decremented => self.value -= 1,
            CounterEvent::Set(v) => self.value = *v,
        }
        self
    }
}

/// Error type for the counter aggregate.
///
/// A unit error: the consolidated store tests drive the counter through
/// replay/persist paths that never reject, so a single opaque failure domain is
/// all they shared.
#[derive(Debug, thiserror::Error)]
#[error("counter error")]
pub struct CounterError;

/// The counter aggregate marker.
///
/// A bare marker unit struct — state lives in `AggregateRoot<Counter>`. The
/// consolidated tests drive it through `replay` / persistence, not through
/// `Handle`, so no command handlers are defined here; a file that needs to decide
/// a command implements `Handle` on this marker locally.
#[derive(Debug)]
pub struct Counter;

impl Aggregate for Counter {
    type State = CounterState;
    type Error = CounterError;
    type Id = TestId;
}
