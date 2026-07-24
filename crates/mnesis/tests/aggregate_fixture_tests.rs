//! Tests for `mnesis::testing::AggregateFixture`. Exercises the real
//! given (replay) -> when (handle) -> then chain on a sample aggregate.
#![cfg(all(feature = "testing", feature = "derive"))]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    unused_must_use,
    reason = "test code; terminal then_expect_* assertions intentionally drop the returned fixture"
)]

use mnesis::testing::AggregateFixture;
use mnesis::{AggregateState, DomainEvent, Events, Handle, Message, events};

// ── Sample domain ────────────────────────────────────────────────
// Store the id as fixed bytes so `as_ref` can borrow them (the kernel's
// own `Id` tests use the same `[u8; N]` pattern). `Id` is satisfied for
// free via the blanket impl — no hand-written `impl Id`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
struct CounterId([u8; 8]);
impl CounterId {
    const fn new(n: u64) -> Self {
        Self(n.to_le_bytes())
    }
}
impl core::fmt::Display for CounterId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", u64::from_le_bytes(self.0))
    }
}
impl AsRef<[u8]> for CounterId {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq)]
enum CounterEvent {
    Incremented(u64),
}
impl Message for CounterEvent {}
impl DomainEvent for CounterEvent {
    fn name(&self) -> &'static str {
        "Incremented"
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct CounterState {
    total: u64,
}
impl AggregateState for CounterState {
    type Event = CounterEvent;
    fn initial() -> Self {
        Self::default()
    }
    fn apply(mut self, event: &CounterEvent) -> Self {
        match event {
            CounterEvent::Incremented(by) => {
                self.total = self
                    .total
                    .checked_add(*by)
                    .expect("test totals never overflow");
                self
            }
        }
    }
}

#[derive(Debug, PartialEq, thiserror::Error)]
#[error("counter error")]
enum CounterError {
    Overflow,
}

#[mnesis::aggregate(state = CounterState, error = CounterError, id = CounterId)]
struct Counter;

#[derive(Debug)]
struct Increment(u64);

impl Handle<Increment> for Counter {
    fn handle(
        state: &CounterState,
        cmd: Increment,
    ) -> Result<Option<Events<CounterEvent>>, CounterError> {
        state
            .total
            .checked_add(cmd.0)
            .ok_or(CounterError::Overflow)?;
        Ok(Some(events![CounterEvent::Incremented(cmd.0)]))
    }
}

/// Raise the counter to at least `floor`. Already at or above it means there
/// is nothing to record — accepted, but no event (#329).
#[derive(Debug)]
struct RaiseTo(u64);

impl Handle<RaiseTo> for Counter {
    fn handle(
        state: &CounterState,
        cmd: RaiseTo,
    ) -> Result<Option<Events<CounterEvent>>, CounterError> {
        match cmd.0.checked_sub(state.total) {
            None | Some(0) => Ok(None),
            Some(delta) => Ok(Some(events![CounterEvent::Incremented(delta)])),
        }
    }
}

#[derive(Debug)]
struct IncrementBounded {
    by: u64,
    max: u64,
}

impl Handle<IncrementBounded> for Counter {
    fn handle(
        state: &CounterState,
        cmd: IncrementBounded,
    ) -> Result<Option<Events<CounterEvent>>, CounterError> {
        let next = state
            .total
            .checked_add(cmd.by)
            .ok_or(CounterError::Overflow)?;
        if next > cmd.max {
            return Err(CounterError::Overflow);
        }
        Ok(Some(events![CounterEvent::Incremented(cmd.by)]))
    }
}

// ── Tests ────────────────────────────────────────────────────────
#[test]
fn given_replays_history_then_asserts_state() {
    AggregateFixture::<Counter>::new()
        .given([CounterEvent::Incremented(10), CounterEvent::Incremented(5)])
        .then_expect_state(|s| assert_eq!(s.total, 15));
}

#[test]
fn with_id_constructs_equivalently_to_new() {
    AggregateFixture::<Counter>::with_id(CounterId::new(42))
        .given([CounterEvent::Incremented(7)])
        .then_expect_state(|s| assert_eq!(s.total, 7));
}

#[test]
fn when_decides_then_expect_events() {
    AggregateFixture::<Counter>::new()
        .given([CounterEvent::Incremented(10)])
        .when(Increment(5))
        .then_expect_events([CounterEvent::Incremented(5)]);
}

#[test]
fn when_on_empty_history_decides_from_initial() {
    AggregateFixture::<Counter>::new()
        .given([])
        .when(Increment(3))
        .then_expect_events([CounterEvent::Incremented(3)]);
}

#[test]
fn rejected_command_then_expect_error_exact() {
    AggregateFixture::<Counter>::new()
        .given([CounterEvent::Incremented(90)])
        .when(IncrementBounded { by: 20, max: 100 })
        .then_expect_error(CounterError::Overflow);
}

#[test]
fn rejected_command_then_expect_error_matching() {
    AggregateFixture::<Counter>::new()
        .given([CounterEvent::Incremented(90)])
        .when(IncrementBounded { by: 20, max: 100 })
        .then_expect_error_matching(|e| matches!(e, CounterError::Overflow));
}

#[test]
fn resulting_state_reflects_decided_events() {
    AggregateFixture::<Counter>::new()
        .given([CounterEvent::Incremented(10)])
        .when(Increment(5))
        .then_expect_state(|s| assert_eq!(s.total, 15));
}

#[test]
fn events_and_state_chained_after_one_command() {
    AggregateFixture::<Counter>::new()
        .given([CounterEvent::Incremented(1)])
        .when(Increment(2))
        .then_expect_events([CounterEvent::Incremented(2)])
        .then_expect_state(|s| assert_eq!(s.total, 3));
}

#[test]
fn rejected_command_leaves_state_unchanged() {
    AggregateFixture::<Counter>::new()
        .given([CounterEvent::Incremented(90)])
        .when(IncrementBounded { by: 20, max: 100 })
        .then_expect_error(CounterError::Overflow)
        .then_expect_state(|s| assert_eq!(s.total, 90));
}

// ── Lifecycle: rehydration parity ────────────────────────────────
#[test]
fn multi_event_history_replays_in_order() {
    AggregateFixture::<Counter>::with_id(CounterId::new(1))
        .given([
            CounterEvent::Incremented(1),
            CounterEvent::Incremented(2),
            CounterEvent::Incremented(3),
        ])
        .then_expect_state(|s| assert_eq!(s.total, 6));
}

// ── Defensive: wrong expectations panic (can-fail proofs) ─────────
// A fixture whose assertions can't fail is worthless; each proof feeds a
// deliberately wrong expectation and asserts (via #[should_panic]) that the
// fixture panics. The `expected =` substring matches the assertion message.
#[test]
#[should_panic(expected = "then_expect_events")]
fn then_expect_events_panics_on_wrong_events() {
    AggregateFixture::<Counter>::new()
        .given([])
        .when(Increment(5))
        .then_expect_events([CounterEvent::Incremented(999)]);
}

#[test]
#[should_panic(expected = "then_expect_events")]
fn then_expect_events_panics_when_command_rejected() {
    AggregateFixture::<Counter>::new()
        .given([CounterEvent::Incremented(90)])
        .when(IncrementBounded { by: 20, max: 100 })
        .then_expect_events([CounterEvent::Incremented(20)]);
}

// ── No-op decisions (#329) ───────────────────────────────────────
#[test]
fn no_op_command_is_ignored_not_an_error() {
    AggregateFixture::<Counter>::new()
        .given([CounterEvent::Incremented(10)])
        .when(RaiseTo(10))
        .then_expect_ignored();
}

#[test]
fn ignored_command_leaves_state_untouched() {
    AggregateFixture::<Counter>::new()
        .given([CounterEvent::Incremented(10)])
        .when(RaiseTo(4))
        .then_expect_ignored()
        .then_expect_state(|s| assert_eq!(s.total, 10));
}

#[test]
fn same_command_decides_events_when_it_changes_something() {
    AggregateFixture::<Counter>::new()
        .given([CounterEvent::Incremented(10)])
        .when(RaiseTo(25))
        .then_expect_events([CounterEvent::Incremented(15)])
        .then_expect_state(|s| assert_eq!(s.total, 25));
}

#[test]
#[should_panic(expected = "then_expect_ignored")]
fn then_expect_ignored_panics_when_events_were_decided() {
    AggregateFixture::<Counter>::new()
        .given([])
        .when(Increment(1))
        .then_expect_ignored();
}

#[test]
#[should_panic(expected = "then_expect_ignored")]
fn then_expect_ignored_panics_when_command_was_rejected() {
    AggregateFixture::<Counter>::new()
        .given([CounterEvent::Incremented(u64::MAX)])
        .when(Increment(1))
        .then_expect_ignored();
}

#[test]
#[should_panic(expected = "use then_expect_ignored")]
fn then_expect_events_panics_on_a_no_op() {
    AggregateFixture::<Counter>::new()
        .given([CounterEvent::Incremented(10)])
        .when(RaiseTo(10))
        .then_expect_events([CounterEvent::Incremented(0)]);
}

#[test]
#[should_panic(expected = "then_expect_error")]
fn then_expect_error_panics_when_command_succeeded() {
    AggregateFixture::<Counter>::new()
        .given([])
        .when(Increment(1))
        .then_expect_error(CounterError::Overflow);
}

#[test]
#[should_panic(expected = "then_expect_error_matching")]
fn then_expect_error_matching_panics_when_command_succeeded() {
    AggregateFixture::<Counter>::new()
        .given([])
        .when(Increment(1))
        .then_expect_error_matching(|e| matches!(e, CounterError::Overflow));
}

// Panic originates in the caller's own `assert_eq!` closure (not the
// fixture), so the matched substring is assert_eq!'s own message rather
// than a `then_expect_*` prefix.
#[test]
#[should_panic(expected = "left == right")]
fn then_expect_state_panics_on_wrong_state() {
    AggregateFixture::<Counter>::new()
        .given([CounterEvent::Incremented(10)])
        .then_expect_state(|s| assert_eq!(s.total, 999));
}
