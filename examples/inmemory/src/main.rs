//! In-memory event-sourced bank account example.
//!
//! Demonstrates the full Mnesis pattern without any persistence layer:
//! - Domain events as enums with `#[derive(DomainEvent)]`
//! - Aggregate state with exhaustive event handling
//! - Command handling via `Handle<C>` trait
//! - Event replay (rehydration) from in-memory store
//! - Multiple aggregates in the same system

// Relaxed lints for example code — production crates should NOT do this.
#![allow(clippy::unwrap_used, reason = "example code uses unwrap for brevity")]
#![allow(clippy::expect_used, reason = "example code uses expect for clarity")]
#![allow(
    clippy::print_stdout,
    reason = "example code prints to demonstrate output"
)]
#![allow(
    clippy::str_to_string,
    reason = "example code uses to_string for readability"
)]

use mnesis::*;
use std::collections::HashMap;
use std::fmt;

// =============================================================================
// Domain: Bank Account
// =============================================================================

// --- ID ---

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct AccountId(String);

impl fmt::Display for AccountId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AsRef<[u8]> for AccountId {
    fn as_ref(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

// --- Events ---

#[derive(Debug, Clone)]
struct AccountOpened {
    owner: String,
}

#[derive(Debug, Clone)]
struct MoneyDeposited {
    amount: u64,
}

#[derive(Debug, Clone)]
struct MoneyWithdrawn {
    amount: u64,
}

#[derive(Debug, Clone)]
struct AccountClosed;

#[derive(Debug, Clone, DomainEvent)]
enum AccountEvent {
    Opened(AccountOpened),
    Deposited(MoneyDeposited),
    Withdrawn(MoneyWithdrawn),
    Closed(AccountClosed),
}

// --- State ---

#[derive(Default, Debug, Clone)]
struct AccountState {
    owner: String,
    balance: u64,
    is_open: bool,
}

impl AggregateState for AccountState {
    type Event = AccountEvent;
    fn initial() -> Self {
        Self::default()
    }

    fn apply(mut self, event: &AccountEvent) -> Self {
        match event {
            AccountEvent::Opened(e) => {
                self.owner = e.owner.clone();
                self.is_open = true;
            }
            AccountEvent::Deposited(e) => {
                self.balance += e.amount;
            }
            AccountEvent::Withdrawn(e) => {
                self.balance -= e.amount;
            }
            AccountEvent::Closed(_) => {
                self.is_open = false;
            }
        }
        self
    }
}

// --- Errors ---

#[derive(Debug, thiserror::Error)]
enum AccountError {
    #[error("account already open")]
    AlreadyOpen,
    #[error("account is closed")]
    Closed,
    #[error("insufficient funds: have {balance}, need {amount}")]
    InsufficientFunds { balance: u64, amount: u64 },
    #[error("cannot close account with balance {0}")]
    NonZeroBalance(u64),
}

// --- Aggregate ---

#[mnesis::aggregate(state = AccountState, error = AccountError, id = AccountId)]
struct BankAccount;

// --- Commands ---

struct OpenAccount {
    owner: String,
}

struct Deposit {
    amount: u64,
}

struct Withdraw {
    amount: u64,
}

struct CloseAccount;

// Handlers are pure decision functions on the marker type: they read the
// borrowed `state`, validate invariants, and return decided events. They never
// see version or identity — a decision depends only on domain state + command.
impl Handle<OpenAccount> for BankAccount {
    fn handle(
        state: &AccountState,
        cmd: OpenAccount,
    ) -> Result<Option<Events<AccountEvent>>, AccountError> {
        if state.is_open {
            return Err(AccountError::AlreadyOpen);
        }
        Ok(Some(events![AccountEvent::Opened(AccountOpened {
            owner: cmd.owner,
        })]))
    }
}

impl Handle<Deposit> for BankAccount {
    fn handle(
        state: &AccountState,
        cmd: Deposit,
    ) -> Result<Option<Events<AccountEvent>>, AccountError> {
        if !state.is_open {
            return Err(AccountError::Closed);
        }
        Ok(Some(events![AccountEvent::Deposited(MoneyDeposited {
            amount: cmd.amount,
        })]))
    }
}

impl Handle<Withdraw> for BankAccount {
    fn handle(
        state: &AccountState,
        cmd: Withdraw,
    ) -> Result<Option<Events<AccountEvent>>, AccountError> {
        if !state.is_open {
            return Err(AccountError::Closed);
        }
        if state.balance < cmd.amount {
            return Err(AccountError::InsufficientFunds {
                balance: state.balance,
                amount: cmd.amount,
            });
        }
        Ok(Some(events![AccountEvent::Withdrawn(MoneyWithdrawn {
            amount: cmd.amount,
        })]))
    }
}

impl Handle<CloseAccount> for BankAccount {
    fn handle(
        state: &AccountState,
        _cmd: CloseAccount,
    ) -> Result<Option<Events<AccountEvent>>, AccountError> {
        if !state.is_open {
            return Err(AccountError::Closed);
        }
        if state.balance > 0 {
            return Err(AccountError::NonZeroBalance(state.balance));
        }
        Ok(Some(events![AccountEvent::Closed(AccountClosed)]))
    }
}

// =============================================================================
// In-Memory Event Store (not part of mnesis — just for this example)
// =============================================================================

struct InMemoryStore {
    streams: HashMap<AccountId, Vec<VersionedEvent<AccountEvent>>>,
}

impl InMemoryStore {
    fn new() -> Self {
        Self {
            streams: HashMap::new(),
        }
    }

    /// Simulate persist + apply: store the decided events, advance version,
    /// and apply to in-memory state. Operates on the kernel's `AggregateRoot`
    /// directly — the aggregate marker (`BankAccount`) carries no state.
    fn save(&mut self, account: &mut AggregateRoot<BankAccount>, decided: &Events<AccountEvent>) {
        let stream = self.streams.entry(account.id().clone()).or_default();
        let first = account
            .version()
            .map_or(Version::INITIAL, |v| v.next().expect("version overflow"));
        let run = Version::run(first, decided.len()).expect("version overflow");
        let last = run.clone().last().unwrap_or(first);
        for (ver, event) in run.zip(decided.iter()) {
            stream.push(VersionedEvent::new(ver, event.clone()));
        }
        account.commit_persisted(last, decided);
    }

    fn load(&self, id: &AccountId) -> Option<AggregateRoot<BankAccount>> {
        let events = self.streams.get(id)?;
        let mut account = AggregateRoot::<BankAccount>::new(id.clone());
        for e in events {
            account
                .replay(e.version(), e.event())
                .expect("valid event sequence");
        }
        Some(account)
    }
}

// =============================================================================
// Main
// =============================================================================

fn main() {
    let mut store = InMemoryStore::new();
    let alice_id = AccountId("alice-001".into());
    let bob_id = AccountId("bob-001".into());

    // --- Alice opens an account and deposits money ---
    println!("=== Alice's Account ===");

    let mut alice = AggregateRoot::<BankAccount>::new(alice_id.clone());
    let decided = alice
        .handle(OpenAccount {
            owner: "Alice Smith".into(),
        })
        .expect("open")
        .expect("command decided events");
    store.save(&mut alice, &decided);

    let decided = alice
        .handle(Deposit { amount: 1000 })
        .expect("deposit")
        .expect("command decided events");
    store.save(&mut alice, &decided);

    let decided = alice
        .handle(Deposit { amount: 500 })
        .expect("deposit")
        .expect("command decided events");
    store.save(&mut alice, &decided);

    println!(
        "Balance: {} (version: {:?})",
        alice.state().balance,
        alice.version()
    );

    // --- Bob opens an account ---
    println!("\n=== Bob's Account ===");

    let mut bob = AggregateRoot::<BankAccount>::new(bob_id.clone());
    let decided = bob
        .handle(OpenAccount {
            owner: "Bob Jones".into(),
        })
        .expect("open")
        .expect("command decided events");
    store.save(&mut bob, &decided);

    let decided = bob
        .handle(Deposit { amount: 200 })
        .expect("deposit")
        .expect("command decided events");
    store.save(&mut bob, &decided);

    println!(
        "Balance: {} (version: {:?})",
        bob.state().balance,
        bob.version()
    );

    // --- Reload Alice from the store (rehydration) ---
    println!("\n=== Reload Alice from Store ===");

    let mut alice = store.load(&alice_id).expect("alice exists");
    println!(
        "Rehydrated: owner={}, balance={}, version={:?}",
        alice.state().owner,
        alice.state().balance,
        alice.version()
    );

    // --- Alice withdraws and closes ---
    let decided = alice
        .handle(Withdraw { amount: 300 })
        .expect("withdraw")
        .expect("command decided events");
    store.save(&mut alice, &decided);
    println!("After withdrawal: balance={}", alice.state().balance);

    // Try to overdraw
    let err = alice
        .handle(Withdraw { amount: 5000 })
        .expect_err("overdraw");
    println!("Overdraw rejected: {err}");

    // Withdraw remaining and close
    let decided = alice
        .handle(Withdraw { amount: 1200 })
        .expect("withdraw remaining")
        .expect("command decided events");
    store.save(&mut alice, &decided);

    let decided = alice
        .handle(CloseAccount)
        .expect("close")
        .expect("command decided events");
    store.save(&mut alice, &decided);

    println!(
        "Closed: is_open={}, version={:?}",
        alice.state().is_open,
        alice.version()
    );

    // --- Try to deposit to closed account ---
    let alice = store.load(&alice_id).expect("alice exists");
    let err = alice.handle(Deposit { amount: 100 }).expect_err("closed");
    println!("Deposit to closed account rejected: {err}");

    // --- Final state ---
    println!("\n=== Final Store State ===");
    for (id, events) in &store.streams {
        println!("{id}: {} events", events.len());
    }
}
