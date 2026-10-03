//! Closing the Books — bounded streams vs. a long-lived aggregate.
//!
//! Builds the same cash-register domain two ways and prints how many events
//! each must replay to read the current float. See the `mnesis::closing_the_books`
//! module for the narrative.

// Relaxed lints for example code — production crates should NOT do this.
#![allow(clippy::unwrap_used, reason = "example code uses unwrap for brevity")]
#![allow(clippy::expect_used, reason = "example code uses expect for clarity")]
#![allow(
    clippy::print_stdout,
    reason = "example code prints to demonstrate output"
)]
#![allow(
    clippy::implicit_saturating_sub,
    reason = "explicit compare-then-subtract is the project's underflow guard; saturating_sub is banned (CLAUDE.md rule 2)"
)]

use mnesis::*;
use std::fmt;

// =============================================================================
// Shared helpers (a tiny stand-in for what a repository does)
// =============================================================================

/// Append decided events to a stream history and advance the in-memory root.
/// This stands in for a real `Repository`, mirroring its persist-then-apply
/// step by driving `commit_persisted` (the single post-persist sync that
/// advances the version and folds the events into state atomically).
fn record<A: Aggregate, const N: usize>(
    root: &mut AggregateRoot<A>,
    history: &mut Vec<VersionedEvent<EventOf<A>>>,
    decided: &Events<EventOf<A>, N>,
) where
    EventOf<A>: Clone,
{
    root.commit_version(decided)
        .expect("commit must fit before persistence");
    let first = root
        .version()
        .map_or(Version::INITIAL, |v| v.next().expect("version overflow"));
    let run = Version::run(first, decided.len()).expect("version overflow");
    for (version, event) in run.zip(decided.iter()) {
        history.push(VersionedEvent::new(version, event.clone()));
    }
    root.commit_persisted(decided).expect("root is usable");
}

/// Rebuild current state from one stream by replaying every event, returning
/// the rehydrated root. The pattern's payoff shows up at the call site: a
/// bounded stream means only a handful of events to replay here.
fn replay_stream<A: Aggregate>(
    id: A::Id,
    history: &[VersionedEvent<EventOf<A>>],
) -> AggregateRoot<A> {
    let mut root = AggregateRoot::<A>::new(id);
    for versioned in history {
        root.replay(versioned.version(), versioned.event())
            .expect("valid history");
    }
    root
}

// =============================================================================
// Pattern: CashierShift — a bounded, lifecycle-scoped stream
// =============================================================================

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct CashierShiftId {
    register: String,
    shift_number: u32,
    urn: String,
}

impl CashierShiftId {
    fn new(register: &str, shift_number: u32) -> Self {
        Self {
            register: register.to_owned(),
            shift_number,
            urn: format!("urn:cashier_shift:{register}:{shift_number}"),
        }
    }
}

impl fmt::Display for CashierShiftId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.urn)
    }
}

impl AsRef<[u8]> for CashierShiftId {
    fn as_ref(&self) -> &[u8] {
        self.urn.as_bytes()
    }
}

#[derive(Debug, Clone, PartialEq, DomainEvent)]
enum ShiftEvent {
    Opened(ShiftOpened),
    TransactionRegistered(TransactionRegistered),
    Closed(ShiftClosed),
}

#[derive(Debug, Clone, PartialEq)]
struct ShiftOpened {
    opening_float: u64,
}

#[derive(Debug, Clone, PartialEq)]
struct TransactionRegistered {
    amount: u64,
}

/// The summary ("closing the books") event: a first-class domain event that
/// carries the minimum state the next shift needs to open.
#[derive(Debug, Clone, PartialEq)]
struct ShiftClosed {
    declared_tender: u64,
    overage: u64,
    shortage: u64,
    final_float: u64,
}

#[derive(Debug, Clone)]
struct ShiftState {
    opening_float: u64,
    registered_total: Result<u64, ShiftError>,
    is_open: bool,
    closed: bool,
}

impl ShiftState {
    fn total_after(&self, amount: u64) -> Result<u64, ShiftError> {
        let total = self.registered_total.clone()?;
        let updated = total
            .checked_add(amount)
            .ok_or(ShiftError::AmountOverflow { total, amount })?;
        self.opening_float
            .checked_add(updated)
            .ok_or(ShiftError::AmountOverflow {
                total: self.opening_float,
                amount: updated,
            })?;
        Ok(updated)
    }
}

impl AggregateState for ShiftState {
    type Event = ShiftEvent;
    fn initial() -> Self {
        Self {
            opening_float: 0,
            registered_total: Ok(0),
            is_open: false,
            closed: false,
        }
    }
    fn apply(mut self, event: &ShiftEvent) -> Self {
        if self.registered_total.is_err() {
            return self;
        }
        match event {
            ShiftEvent::Opened(e) => {
                self.opening_float = e.opening_float;
                self.registered_total = Ok(0);
                self.is_open = true;
            }
            ShiftEvent::TransactionRegistered(e) => {
                self.registered_total = self.total_after(e.amount)
            }
            ShiftEvent::Closed(_) => {
                self.is_open = false;
                self.closed = true;
            }
        }
        self
    }
}

#[mnesis::aggregate(state = ShiftState, error = ShiftError, id = CashierShiftId)]
struct CashierShift;

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
enum ShiftError {
    #[error("shift already opened")]
    AlreadyOpen,
    #[error("shift is not open")]
    NotOpen,
    #[error("amount overflow: total {total}, added {amount}")]
    AmountOverflow { total: u64, amount: u64 },
}

struct OpenShift {
    opening_float: u64,
}

struct RegisterTransaction {
    amount: u64,
}

struct CloseShift {
    declared_tender: u64,
}

impl Handle<OpenShift> for CashierShift {
    fn handle(
        state: &ShiftState,
        cmd: OpenShift,
    ) -> Result<Option<Events<ShiftEvent>>, ShiftError> {
        state.registered_total.clone()?;
        if state.is_open || state.closed {
            return Err(ShiftError::AlreadyOpen);
        }
        Ok(Some(events![ShiftEvent::Opened(ShiftOpened {
            opening_float: cmd.opening_float,
        })]))
    }
}

impl Handle<RegisterTransaction> for CashierShift {
    fn handle(
        state: &ShiftState,
        cmd: RegisterTransaction,
    ) -> Result<Option<Events<ShiftEvent>>, ShiftError> {
        state.registered_total.clone()?;
        if !state.is_open {
            return Err(ShiftError::NotOpen);
        }
        state.total_after(cmd.amount)?;
        Ok(Some(events![ShiftEvent::TransactionRegistered(
            TransactionRegistered { amount: cmd.amount }
        )]))
    }
}

impl Handle<CloseShift> for CashierShift {
    fn handle(
        state: &ShiftState,
        cmd: CloseShift,
    ) -> Result<Option<Events<ShiftEvent>>, ShiftError> {
        let total = state.registered_total.clone()?;
        if !state.is_open {
            return Err(ShiftError::NotOpen);
        }
        let expected =
            state
                .opening_float
                .checked_add(total)
                .ok_or(ShiftError::AmountOverflow {
                    total: state.opening_float,
                    amount: total,
                })?;
        let difference = cmd.declared_tender.abs_diff(expected);
        let overage = if cmd.declared_tender > expected {
            difference
        } else {
            0
        };
        let shortage = if expected > cmd.declared_tender {
            difference
        } else {
            0
        };
        Ok(Some(events![ShiftEvent::Closed(ShiftClosed {
            declared_tender: cmd.declared_tender,
            overage,
            shortage,
            final_float: cmd.declared_tender,
        })]))
    }
}

// =============================================================================
// Anti-pattern: CashRegister — one never-ending stream (would need a snapshot)
// =============================================================================

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct RegisterId(String);

impl fmt::Display for RegisterId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<[u8]> for RegisterId {
    fn as_ref(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

#[derive(Debug, Clone, PartialEq, DomainEvent)]
enum RegisterEvent {
    Opened(RegisterOpened),
    Sale(SaleRegistered),
}

#[derive(Debug, Clone, PartialEq)]
struct RegisterOpened {
    opening_float: u64,
}

#[derive(Debug, Clone, PartialEq)]
struct SaleRegistered {
    amount: u64,
}

#[derive(Debug, Clone)]
struct RegisterState {
    float: Result<u64, RegisterError>,
    is_open: bool,
}

impl RegisterState {
    fn float_after(&self, amount: u64) -> Result<u64, RegisterError> {
        let total = self.float.clone()?;
        total
            .checked_add(amount)
            .ok_or(RegisterError::AmountOverflow { total, amount })
    }
}

impl AggregateState for RegisterState {
    type Event = RegisterEvent;
    fn initial() -> Self {
        Self {
            float: Ok(0),
            is_open: false,
        }
    }
    fn apply(mut self, event: &RegisterEvent) -> Self {
        if self.float.is_err() {
            return self;
        }
        match event {
            RegisterEvent::Opened(e) => {
                self.float = Ok(e.opening_float);
                self.is_open = true;
            }
            RegisterEvent::Sale(e) => self.float = self.float_after(e.amount),
        }
        self
    }
}

#[mnesis::aggregate(state = RegisterState, error = RegisterError, id = RegisterId)]
struct CashRegister;

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
enum RegisterError {
    #[error("register already open")]
    AlreadyOpen,
    #[error("register is not open")]
    NotOpen,
    #[error("amount overflow: total {total}, added {amount}")]
    AmountOverflow { total: u64, amount: u64 },
}

struct OpenRegister {
    opening_float: u64,
}

struct RegisterSale {
    amount: u64,
}

impl Handle<OpenRegister> for CashRegister {
    fn handle(
        state: &RegisterState,
        cmd: OpenRegister,
    ) -> Result<Option<Events<RegisterEvent>>, RegisterError> {
        state.float.clone()?;
        // No CloseRegister command: a register has no lifecycle end, so (unlike
        // a shift) there is no `closed` state to guard against here.
        if state.is_open {
            return Err(RegisterError::AlreadyOpen);
        }
        Ok(Some(events![RegisterEvent::Opened(RegisterOpened {
            opening_float: cmd.opening_float,
        })]))
    }
}

impl Handle<RegisterSale> for CashRegister {
    fn handle(
        state: &RegisterState,
        cmd: RegisterSale,
    ) -> Result<Option<Events<RegisterEvent>>, RegisterError> {
        state.float.clone()?;
        if !state.is_open {
            return Err(RegisterError::NotOpen);
        }
        state.float_after(cmd.amount)?;
        Ok(Some(events![RegisterEvent::Sale(SaleRegistered {
            amount: cmd.amount,
        })]))
    }
}

/// Drive 5,000 sales into one never-ending stream, then show that reading the
/// current float means replaying the whole thing. Self-contained like
/// `run_cashier_shift_demo`: it prints its own section.
fn run_long_lived_demo() {
    println!("=== Anti-pattern: one long-lived CashRegister stream ===");

    let id = RegisterId("till-1".to_owned());
    let mut register = AggregateRoot::<CashRegister>::new(id.clone());
    let mut stream: Vec<VersionedEvent<RegisterEvent>> = Vec::new();

    let opened = register
        .handle(OpenRegister { opening_float: 100 })
        .expect("open register")
        .expect("command decided events");
    record(&mut register, &mut stream, &opened);
    for _ in 0..5000 {
        let sale = register
            .handle(RegisterSale { amount: 1 })
            .expect("register sale")
            .expect("command decided events");
        record(&mut register, &mut stream, &sale);
    }

    // To read the current float we must replay the ENTIRE stream.
    let root = replay_stream::<CashRegister>(id, &stream);
    println!(
        "long-lived CashRegister -> replaying all {} events recovers current float = {}",
        stream.len(),
        root.state()
            .expect("aggregate state is available")
            .float
            .as_ref()
            .expect("valid register arithmetic"),
    );
}

fn run_cashier_shift_demo() {
    println!("=== Closing the Books: CashierShift (bounded streams) ===");

    // Shift #1: open with float 100, register 10 sales of 1 each, then close.
    let shift1_id = CashierShiftId::new("till-1", 1);
    let mut shift1 = AggregateRoot::<CashierShift>::new(shift1_id.clone());
    let mut shift1_stream: Vec<VersionedEvent<ShiftEvent>> = Vec::new();

    let opened = shift1
        .handle(OpenShift { opening_float: 100 })
        .expect("open shift 1")
        .expect("command decided events");
    record(&mut shift1, &mut shift1_stream, &opened);
    for _ in 0..10 {
        let txn = shift1
            .handle(RegisterTransaction { amount: 1 })
            .expect("register txn")
            .expect("command decided events");
        record(&mut shift1, &mut shift1_stream, &txn);
    }
    let closed = shift1
        .handle(CloseShift {
            declared_tender: 110,
        })
        .expect("close shift 1")
        .expect("command decided events");
    // The summary event carries the closing float forward — read it from the
    // decided ShiftClosed event rather than recomputing it.
    let final_float = closed
        .iter()
        .find_map(|e| match e {
            ShiftEvent::Closed(c) => Some(c.final_float),
            ShiftEvent::Opened(_) | ShiftEvent::TransactionRegistered(_) => None,
        })
        .expect("close produced a ShiftClosed event");
    record(&mut shift1, &mut shift1_stream, &closed);
    println!(
        "shift #1 closed: stream length = {}, final_float = {final_float}",
        shift1_stream.len()
    );

    // Shift #2: a brand-new stream, opened from the carried-forward float.
    let shift2_id = CashierShiftId::new(&shift1_id.register, shift1_id.shift_number + 1);
    let mut shift2 = AggregateRoot::<CashierShift>::new(shift2_id.clone());
    let mut shift2_stream: Vec<VersionedEvent<ShiftEvent>> = Vec::new();

    let opened = shift2
        .handle(OpenShift {
            opening_float: final_float,
        })
        .expect("open shift 2")
        .expect("command decided events");
    record(&mut shift2, &mut shift2_stream, &opened);
    for _ in 0..10 {
        let txn = shift2
            .handle(RegisterTransaction { amount: 1 })
            .expect("register txn")
            .expect("command decided events");
        record(&mut shift2, &mut shift2_stream, &txn);
    }

    // Read current state by replaying ONLY shift #2's short stream — no earlier
    // shift is touched. The recovered total confirms the carry-forward worked.
    let shift2_root = replay_stream::<CashierShift>(shift2_id.clone(), &shift2_stream);
    println!("shift #2 {shift2_id} opened from carried float {final_float}");
    println!(
        "replaying ONLY the current shift ({} events) recovers registered total = {}",
        shift2_stream.len(),
        shift2_root
            .state()
            .expect("aggregate state is available")
            .registered_total
            .as_ref()
            .expect("valid shift arithmetic"),
    );
}

fn main() {
    run_long_lived_demo();
    println!();
    run_cashier_shift_demo();
    println!();
    println!("Takeaway: the summary event (ShiftClosed) carries the float forward,");
    println!("so each shift is a short, independent stream — no snapshot required.");
}

#[cfg(test)]
mod tests {
    use super::*;
    use mnesis::testing::AggregateFixture;

    fn fixture() -> AggregateFixture<CashierShift> {
        AggregateFixture::with_id(CashierShiftId::new("till-1", 1))
    }

    #[test]
    fn close_with_exact_tender_has_no_discrepancy() {
        let _ = fixture()
            .given([
                ShiftEvent::Opened(ShiftOpened { opening_float: 100 }),
                ShiftEvent::TransactionRegistered(TransactionRegistered { amount: 50 }),
            ])
            .when(CloseShift {
                declared_tender: 150,
            })
            .then_expect_events([ShiftEvent::Closed(ShiftClosed {
                declared_tender: 150,
                overage: 0,
                shortage: 0,
                final_float: 150,
            })]);
    }

    #[test]
    fn close_with_surplus_reports_overage() {
        let _ = fixture()
            .given([
                ShiftEvent::Opened(ShiftOpened { opening_float: 100 }),
                ShiftEvent::TransactionRegistered(TransactionRegistered { amount: 50 }),
            ])
            .when(CloseShift {
                declared_tender: 160,
            })
            .then_expect_events([ShiftEvent::Closed(ShiftClosed {
                declared_tender: 160,
                overage: 10,
                shortage: 0,
                final_float: 160,
            })]);
    }

    #[test]
    fn close_with_deficit_reports_shortage() {
        let _ = fixture()
            .given([
                ShiftEvent::Opened(ShiftOpened { opening_float: 100 }),
                ShiftEvent::TransactionRegistered(TransactionRegistered { amount: 50 }),
            ])
            .when(CloseShift {
                declared_tender: 140,
            })
            .then_expect_events([ShiftEvent::Closed(ShiftClosed {
                declared_tender: 140,
                overage: 0,
                shortage: 10,
                final_float: 140,
            })]);
    }

    #[test]
    fn close_before_open_is_rejected() {
        let _ = fixture()
            .given(Vec::<ShiftEvent>::new())
            .when(CloseShift {
                declared_tender: 100,
            })
            .then_expect_error(ShiftError::NotOpen);
    }
}

#[cfg(test)]
mod audit_arithmetic {
    use super::{
        CashRegister, CashierShift, CloseShift, OpenRegister, OpenShift, RegisterError,
        RegisterEvent, RegisterOpened, RegisterSale, RegisterState, RegisterTransaction,
        SaleRegistered, ShiftError, ShiftEvent, ShiftOpened, ShiftState, TransactionRegistered,
    };
    use mnesis::{AggregateState, Handle};

    #[test]
    fn registered_total_overflow_rejects_before_emitting_an_event() {
        let state = ShiftState {
            registered_total: Ok(u64::MAX),
            is_open: true,
            ..ShiftState::initial()
        };
        assert!(matches!(
            <CashierShift as Handle<RegisterTransaction>>::handle(
                &state,
                RegisterTransaction { amount: 1 }
            ),
            Err(ShiftError::AmountOverflow {
                total: u64::MAX,
                amount: 1
            })
        ));
    }

    #[test]
    fn register_float_overflow_rejects_before_emitting_an_event() {
        let state = RegisterState {
            float: Ok(u64::MAX),
            is_open: true,
        };
        assert!(matches!(
            <CashRegister as Handle<RegisterSale>>::handle(&state, RegisterSale { amount: 1 }),
            Err(RegisterError::AmountOverflow {
                total: u64::MAX,
                amount: 1
            })
        ));
    }

    #[test]
    fn overflowing_expected_tender_rejects_during_close() {
        let state = ShiftState {
            opening_float: u64::MAX,
            registered_total: Ok(1),
            is_open: true,
            closed: false,
        };
        assert!(matches!(
            <CashierShift as Handle<CloseShift>>::handle(&state, CloseShift { declared_tender: 0 }),
            Err(ShiftError::AmountOverflow {
                total: u64::MAX,
                amount: 1
            })
        ));
    }

    #[test]
    fn register_boundary_and_invalid_history_are_exact() {
        for (total, amount, expected) in [
            (0, 0, 0),
            (0, u64::MAX, u64::MAX),
            (u64::MAX - 1, 1, u64::MAX),
            (u64::MAX, 0, u64::MAX),
        ] {
            let state = RegisterState {
                float: Ok(total),
                is_open: true,
            };
            let events =
                <CashRegister as Handle<RegisterSale>>::handle(&state, RegisterSale { amount })
                    .unwrap()
                    .unwrap();
            assert_eq!(state.apply(events.first()).float, Ok(expected));
        }
        let invalid = RegisterState {
            float: Ok(u64::MAX),
            is_open: true,
        }
        .apply(&RegisterEvent::Sale(SaleRegistered { amount: 1 }));
        let error = RegisterError::AmountOverflow {
            total: u64::MAX,
            amount: 1,
        };
        assert_eq!(invalid.float, Err(error.clone()));
        assert!(
            matches!(<CashRegister as Handle<OpenRegister>>::handle(&invalid, OpenRegister { opening_float: 0 }), Err(actual) if actual == error)
        );
        assert!(
            matches!(<CashRegister as Handle<RegisterSale>>::handle(&invalid, RegisterSale { amount: 0 }), Err(actual) if actual == error)
        );
        assert_eq!(
            invalid
                .apply(&RegisterEvent::Opened(RegisterOpened { opening_float: 0 }))
                .float,
            Err(error)
        );
    }

    #[test]
    fn opening_float_is_part_of_the_transaction_limit_and_invalid_history_is_sticky() {
        let state = ShiftState {
            opening_float: u64::MAX,
            registered_total: Ok(0),
            is_open: true,
            closed: false,
        };
        let error = ShiftError::AmountOverflow {
            total: u64::MAX,
            amount: 1,
        };
        assert!(
            matches!(<CashierShift as Handle<RegisterTransaction>>::handle(&state, RegisterTransaction { amount: 1 }), Err(actual) if actual == error)
        );
        let zero = <CashierShift as Handle<RegisterTransaction>>::handle(
            &state,
            RegisterTransaction { amount: 0 },
        )
        .unwrap()
        .unwrap();
        assert_eq!(state.clone().apply(zero.first()).registered_total, Ok(0));
        let invalid = state.apply(&ShiftEvent::TransactionRegistered(TransactionRegistered {
            amount: 1,
        }));
        assert_eq!(invalid.registered_total, Err(error.clone()));
        assert!(
            matches!(<CashierShift as Handle<OpenShift>>::handle(&invalid, OpenShift { opening_float: 0 }), Err(actual) if actual == error)
        );
        assert!(
            matches!(<CashierShift as Handle<CloseShift>>::handle(&invalid, CloseShift { declared_tender: 0 }), Err(actual) if actual == error)
        );
        assert_eq!(
            invalid
                .apply(&ShiftEvent::Opened(ShiftOpened { opening_float: 0 }))
                .registered_total,
            Err(error)
        );
    }
}
