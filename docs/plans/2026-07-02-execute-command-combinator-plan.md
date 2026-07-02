# `execute` Command Combinator — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `repo.execute(&mut root, cmd)` — one call that decides then saves — so the decide→save hand-off can't be forgotten or misthreaded (#251).

**Architecture:** A blanket-implemented `CommandRepository<A>: Repository<A>` extension trait with one provided method `execute`, mirroring `SagaRepository::react_and_save`. `execute` calls the existing pure `AggregateRoot::handle` then the existing atomic `Repository::save`. No new persistence machinery. Two-domain `ExecuteError` (`Decide` | `Store`), conflict surfaced not retried.

**Tech Stack:** Rust 2024, `nexus-store` crate, `thiserror`, `futures`, `tokio` (tests), `InMemoryStore`/`FjallStore` (test adapters).

**Design doc:** `docs/plans/2026-07-02-execute-command-combinator-design.md`

**Conventions:**
- Run one test: `nix develop -c cargo test -p nexus-store -- <name>`.
- Do **not** run `nix flake check` by hand — the pre-commit hook runs it on `git commit`. Just commit; let the hook gate.
- `git add` any new source file before committing (the gate fails on untracked modules).
- All `use` at top of file. No inline `use`. Strict clippy (`pedantic`/`nursery` denied).

---

## File Structure

- **Create** `crates/nexus-store/src/conflict.rs` — `ConflictPredicate` + `sealed` module, moved verbatim from `saga.rs` so `SagaError` and `ExecuteError` share one impl.
- **Create** `crates/nexus-store/src/execute.rs` — `CommandRepository` trait, blanket impl, `ExecuteError`, unit tests.
- **Create** `crates/nexus-store/tests/execute_tests.rs` — the 4 cross-cutting categories over `InMemoryStore` + the equivalence test.
- **Create** `crates/nexus-fjall/tests/execute_lifecycle_tests.rs` — the persistent-adapter lifecycle test (write → close → reopen → verify; rejection persists nothing).
- **Modify** `crates/nexus-store/src/saga.rs` — delete the `ConflictPredicate`/`sealed` block, import from `crate::conflict`.
- **Modify** `crates/nexus-store/src/lib.rs` — add `pub mod conflict;` and `pub mod execute;`; move `ConflictPredicate` re-export to `conflict`; add `CommandRepository`/`ExecuteError` re-exports.
- **Modify** `examples/fjall-end-to-end/src/lib.rs` + `README.md` — replace every `handle`+`save` pair with `execute`; update prose.

---

## Task 1: Extract `ConflictPredicate` to a shared module (no behavior change)

**Files:**
- Create: `crates/nexus-store/src/conflict.rs`
- Modify: `crates/nexus-store/src/saga.rs:24-46` (remove the block)
- Modify: `crates/nexus-store/src/lib.rs:105` (add `pub mod conflict;`), `:158-161` (re-export move)

- [ ] **Step 1: Create `conflict.rs` with the moved trait**

```rust
//! Shared optimistic-concurrency conflict predicate.
//!
//! Moved out of `saga.rs` so both [`SagaError`](crate::SagaError) and
//! [`ExecuteError`](crate::ExecuteError) delegate to the same
//! [`StoreError::is_conflict`](crate::StoreError::is_conflict) — one truth,
//! two callers.

use crate::error::StoreError;

mod sealed {
    pub trait Sealed {}
}

/// Predicate over a repository error: is this an optimistic-concurrency
/// conflict (and therefore retryable by reloading + re-deciding)?
///
/// Sealed: implemented inside this crate for [`StoreError`] only. Lets error
/// wrappers delegate without naming a concrete store error — `Snapshotting`'s
/// `Repository::Error` is the inner `StoreError`, so one impl serves bare and
/// snapshotted repositories alike.
pub trait ConflictPredicate: sealed::Sealed {
    /// `true` iff this error is an optimistic-concurrency conflict.
    fn is_conflict(&self) -> bool;
}

impl<A, EncErr, DecErr> sealed::Sealed for StoreError<A, EncErr, DecErr> {}

impl<A, EncErr, DecErr> ConflictPredicate for StoreError<A, EncErr, DecErr> {
    fn is_conflict(&self) -> bool {
        Self::is_conflict(self)
    }
}
```

- [ ] **Step 2: Remove the block from `saga.rs` and import from `crate::conflict`**

In `crates/nexus-store/src/saga.rs`, delete the `mod sealed { ... }` block and both `ConflictPredicate` definitions/impls (the block at lines 24-46). Add to the top `use` cluster:

```rust
use crate::conflict::ConflictPredicate;
```

Leave `SagaError`'s `impl<SagaErr, StoreErr: ConflictPredicate>` and `is_conflict` exactly as they are — they now resolve `ConflictPredicate` via the import.

- [ ] **Step 3: Wire the module and move the re-export in `lib.rs`**

Add `pub mod conflict;` in the `pub mod` block (alphabetical, before `pub mod codec;` → put after `pub mod cbor;`). Change the saga re-export from:

```rust
pub use saga::{
    ConflictPredicate, ProjectedIntent, ProjectedIntents, ProjectedIntentsIntoIter, Reaction,
    SagaError, SagaRepository,
};
```

to:

```rust
pub use conflict::ConflictPredicate;
pub use saga::{
    ProjectedIntent, ProjectedIntents, ProjectedIntentsIntoIter, Reaction, SagaError,
    SagaRepository,
};
```

- [ ] **Step 4: Verify nothing broke — saga tests still pass**

Run: `nix develop -c cargo test -p nexus-store -- saga`
Expected: PASS (the extraction is behavior-preserving; `ConflictPredicate` resolves at the new path).

- [ ] **Step 5: Commit**

```bash
git add crates/nexus-store/src/conflict.rs crates/nexus-store/src/saga.rs crates/nexus-store/src/lib.rs
git commit -m "refactor(store): extract ConflictPredicate to shared conflict module"
```

---

## Task 2: `ExecuteError` + its `is_conflict`, with a unit test

**Files:**
- Create: `crates/nexus-store/src/execute.rs`
- Modify: `crates/nexus-store/src/lib.rs` (add `pub mod execute;`)

- [ ] **Step 1: Create `execute.rs` with the error type only**

```rust
//! Store-side command combinator — the aggregate analogue of
//! [`SagaRepository`](crate::SagaRepository).
//!
//! [`CommandRepository::execute`] fuses `decide → save` into one call so the
//! decided events can't be forgotten or misthreaded (#251). It adds no
//! persistence machinery — it is the "imperative shell" over the pure
//! [`AggregateRoot::handle`](nexus::AggregateRoot::handle) and the atomic
//! [`Repository::save`](crate::Repository::save).
//!
//! See `docs/plans/2026-07-02-execute-command-combinator-design.md`.

use core::future::Future;

use nexus::{Aggregate, AggregateRoot, EventOf, Events, Handle};

use crate::conflict::ConflictPredicate;
use crate::repository::Repository;

/// Error from a command `execute`. Two failure domains kept distinct
/// (CLAUDE.md rule 3 — one variant = one domain).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ExecuteError<DecideErr, StoreErr> {
    /// The aggregate rejected the command (a domain invariant). Nothing persisted.
    #[error("command rejected: {0}")]
    Decide(#[source] DecideErr),

    /// `save` failed (adapter / codec / conflict / version overflow).
    #[error(transparent)]
    Store(StoreErr),
}

impl<DecideErr, StoreErr: ConflictPredicate> ExecuteError<DecideErr, StoreErr> {
    /// `true` iff the save failed on an optimistic-concurrency conflict.
    /// `Decide` is never a conflict (rule 3 — rejection is not retryable).
    #[must_use]
    pub fn is_conflict(&self) -> bool {
        matches!(self, Self::Store(e) if e.is_conflict())
    }
}

#[cfg(test)]
mod error_tests {
    use super::ExecuteError;
    use crate::error::StoreError;
    use nexus::{ErrorId, Version};

    type TestStoreError =
        StoreError<std::io::Error, std::convert::Infallible, std::convert::Infallible>;
    type TestExecuteError = ExecuteError<&'static str, TestStoreError>;

    #[test]
    fn conflict_store_error_is_conflict() {
        let e: TestExecuteError = ExecuteError::Store(StoreError::Conflict {
            stream_id: ErrorId::from_display(&"s"),
            expected: Some(Version::INITIAL),
            actual: None,
        });
        assert!(e.is_conflict());
    }

    #[test]
    fn decide_error_is_not_conflict() {
        let e: TestExecuteError = ExecuteError::Decide("rejected");
        assert!(!e.is_conflict());
    }
}
```

> Note: `use nexus::{... Handle, ...}` and the `Repository`/future imports are unused until Task 3 adds the trait. To keep this task compiling clean under strict clippy, add the trait in Task 3 *before* running the full gate; for this task's isolated test run the `#[cfg(test)]` module compiles. If clippy's `unused_imports` bites during this task, trim the `use` line to just what `error_tests` needs (`thiserror` via derive, `ConflictPredicate`) and re-add the rest in Task 3. Simplest path: do Task 2 and Task 3 back-to-back, committing once at the end of Task 3.

- [ ] **Step 2: Wire the module in `lib.rs`**

Add `pub mod execute;` (after `pub mod error;`). Re-export near the saga line:

```rust
pub use execute::{CommandRepository, ExecuteError};
```

(`CommandRepository` lands in Task 3; if compiling Task 2 alone, temporarily export only `ExecuteError` and add `CommandRepository` in Task 3.)

- [ ] **Step 3: Run the error unit tests**

Run: `nix develop -c cargo test -p nexus-store -- execute::error_tests`
Expected: PASS (2 tests).

- [ ] **Step 4: (commit deferred to Task 3 — see note above)**

---

## Task 3: `CommandRepository` trait + blanket impl + `execute`

**Files:**
- Modify: `crates/nexus-store/src/execute.rs`

- [ ] **Step 1: Add the trait and blanket impl above the test module**

```rust
/// The command-facing port: `decide → save` as one callable transaction.
///
/// Extends [`Repository<A>`] and inherits its `load`/`save` unchanged. The one
/// provided method rides on every repository via the blanket impl below — bare
/// [`EventStore`](crate::EventStore) and the
/// [`Snapshotting`](crate::snapshot::Snapshotting) decorator alike.
pub trait CommandRepository<A: Aggregate>: Repository<A> {
    /// Decide `command` against `root`, persist the decided events atomically,
    /// advance `root`, and return the decided events for inspection.
    ///
    /// - `Ok(events)` — the command was accepted and its events are durable.
    /// - `Err(ExecuteError::Decide)` — the aggregate rejected it; nothing persisted.
    /// - `Err(ExecuteError::Store)` — the save failed (see [`ExecuteError::is_conflict`]).
    ///
    /// On a version conflict this returns `Err(ExecuteError::Store(..))` with
    /// `is_conflict() == true` and does **not** retry — retry is the runtime's
    /// job (CLAUDE.md rule 5), matching `SagaRepository::react_and_save`.
    ///
    /// # Errors
    /// See the variants above.
    fn execute<C, const N: usize>(
        &self,
        root: &mut AggregateRoot<A>,
        command: C,
    ) -> impl Future<Output = Result<Events<EventOf<A>, N>, ExecuteError<A::Error, Self::Error>>> + Send
    where
        A: Handle<C, N>,
    {
        async move {
            let decided = root.handle::<C, N>(command).map_err(ExecuteError::Decide)?;
            self.save(root, &decided).await.map_err(ExecuteError::Store)?;
            Ok(decided)
        }
    }
}

// Rides on every repository — bare `EventStore` AND the `Snapshotting`
// decorator — with zero per-type code. Fully static dispatch.
impl<A: Aggregate, R: Repository<A>> CommandRepository<A> for R {}
```

- [ ] **Step 2: Confirm the crate compiles and unit tests pass**

Run: `nix develop -c cargo test -p nexus-store -- execute::`
Expected: PASS. If `unused_imports` fired in Task 2, it clears now (`Handle`, `Repository`, `Future`, `Events`, `EventOf`, `AggregateRoot` are all used).

- [ ] **Step 3: Commit Tasks 2+3 together**

```bash
git add crates/nexus-store/src/execute.rs crates/nexus-store/src/lib.rs
git commit -m "feat(store): CommandRepository::execute — fuse decide and save (#251)"
```

---

## Task 4: The 4 cross-cutting categories + equivalence, over `InMemoryStore`

**Files:**
- Create: `crates/nexus-store/tests/execute_tests.rs`

Model the harness on `crates/nexus-store/tests/saga_repository_tests.rs` (same allows, same `InMemoryStore`/`Store` setup). Define a minimal **counter** aggregate with a `Handle` impl and a JSON-ish codec.

- [ ] **Step 1: Write the test file with the aggregate, codec, and all assertions**

```rust
//! `CommandRepository::execute` integration tests — the 4 mandatory
//! cross-cutting categories (CLAUDE.md rule 7) over `InMemoryStore`, plus the
//! equivalence-with-manual-two-step proof.

#![allow(clippy::unwrap_used, reason = "tests")]
#![allow(clippy::expect_used, reason = "tests")]
#![allow(clippy::panic, reason = "panic in test match arms is an assertion")]
#![allow(
    clippy::shadow_reuse,
    reason = "the spawn-closure clone-and-shadow pattern is idiomatic for tokio tests"
)]

use std::convert::Infallible;
use std::sync::Arc;

use bytes::Bytes;
use nexus::{
    Aggregate, AggregateРoot, AggregateState, DomainEvent, Events, Handle, Message, Version, events,
};
use nexus_store::testing::InMemoryStore;
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

// ── Minimal JSON-free codec (length-agnostic: encode the u64 LE) ──────────
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
    fn decode<'a>(
        &'a self,
        env: &'a PersistedEnvelope,
    ) -> Result<Self::Output<'a>, Infallible> {
        let mut buf = [0u8; 8];
        buf.copy_from_slice(&env.payload()[..8]);
        Ok(CtrEvent::Added(u64::from_le_bytes(buf)))
    }
}

fn repo() -> impl CommandRepository<Counter, Error = impl std::error::Error + Send + Sync + 'static>
{
    Store::new(InMemoryStore::new())
        .repository::<Counter>()
        .codec(CtrCodec)
        .build()
}

// ── 1. Sequence/protocol ──────────────────────────────────────────────────
#[tokio::test]
async fn sequence_execute_chain_advances_version_and_state() {
    let repo = repo();
    let mut ctr = repo.load(CtrId::new(1)).await.unwrap();

    let e1 = repo.execute(&mut ctr, Add(3)).await.unwrap();
    assert_eq!(e1.as_slice(), &[CtrEvent::Added(3)]);
    assert_eq!(ctr.version(), Version::new(1));
    assert_eq!(ctr.state().total, 3);

    let e2 = repo.execute(&mut ctr, Add(4)).await.unwrap();
    assert_eq!(e2.as_slice(), &[CtrEvent::Added(4)]);
    assert_eq!(ctr.version(), Version::new(2));
    assert_eq!(ctr.state().total, 7);

    // Reload from scratch — replays to identical state.
    let reloaded = repo.load(CtrId::new(1)).await.unwrap();
    assert_eq!(reloaded.state().total, 7);
    assert_eq!(reloaded.version(), Version::new(2));
}

// ── 3. Defensive boundary: rejection persists nothing ─────────────────────
#[tokio::test]
async fn defensive_rejected_command_persists_nothing() {
    let repo = repo();
    let mut ctr = repo.load(CtrId::new(2)).await.unwrap();

    let err = repo.execute(&mut ctr, Add(0)).await.unwrap_err();
    assert!(matches!(err, ExecuteError::Decide(CtrError::Zero)));
    assert!(!err.is_conflict());
    assert_eq!(ctr.version(), None); // unchanged — never advanced
    assert_eq!(ctr.state().total, 0);

    let reloaded = repo.load(CtrId::new(2)).await.unwrap();
    assert_eq!(reloaded.version(), None); // stream empty
}

// ── 3b. Defensive boundary: stale root → conflict ─────────────────────────
#[tokio::test]
async fn defensive_stale_root_conflicts() {
    let repo = repo();
    // Two roots at the same id; write with the first, then the stale second.
    let mut a = repo.load(CtrId::new(3)).await.unwrap();
    let mut b = repo.load(CtrId::new(3)).await.unwrap(); // both at version None

    repo.execute(&mut a, Add(1)).await.unwrap(); // a → v1
    let err = repo.execute(&mut b, Add(2)).await.unwrap_err(); // b still expects None
    assert!(err.is_conflict());
    assert!(matches!(err, ExecuteError::Store(_)));
}

// ── 4. Linearizability: concurrent execute, one wins ──────────────────────
#[tokio::test]
async fn linearizable_concurrent_execute_one_winner() {
    let repo = Arc::new(repo());
    let barrier = Arc::new(Barrier::new(2));

    let mut handles = Vec::new();
    for n in [10u64, 20u64] {
        let repo = Arc::clone(&repo);
        let barrier = Arc::clone(&barrier);
        handles.push(tokio::spawn(async move {
            let mut ctr = repo.load(CtrId::new(4)).await.unwrap();
            barrier.wait().await; // ensure both loaded at version None before either writes
            repo.execute(&mut ctr, Add(n)).await
        }));
    }

    let mut wins = 0;
    let mut conflicts = 0;
    for h in handles {
        match h.await.unwrap() {
            Ok(_) => wins += 1,
            Err(e) => {
                assert!(e.is_conflict());
                conflicts += 1;
            }
        }
    }
    assert_eq!(wins, 1);
    assert_eq!(conflicts, 1);

    // Exactly one event in the stream.
    let final_ctr = repo.load(CtrId::new(4)).await.unwrap();
    assert_eq!(final_ctr.version(), Version::new(1));
}

// ── Equivalence: execute == manual handle + save ──────────────────────────
#[tokio::test]
async fn equivalence_execute_matches_manual_two_step() {
    // Manual two-step.
    let manual = repo();
    let mut m = manual.load(CtrId::new(5)).await.unwrap();
    let decided = m.handle::<Add, 0>(Add(9)).unwrap();
    manual.save(&mut m, &decided).await.unwrap();

    // Combinator.
    let fused = repo();
    let mut f = fused.load(CtrId::new(5)).await.unwrap();
    let returned = fused.execute(&mut f, Add(9)).await.unwrap();

    // Same decided events, same resulting version + state.
    assert_eq!(returned.as_slice(), decided.as_slice());
    assert_eq!(f.version(), m.version());
    assert_eq!(f.state(), m.state());
}
```

> **Fix the typo before running:** `AggregateРoot` in the `use` line above contains a Cyrillic `Р` — retype it as ASCII `AggregateRoot`. (Guard against copy-paste.)

- [ ] **Step 2: Verify `Events::as_slice`, `AggregateRoot::state`, and `Version::new` names**

Run: `nix develop -c grep -rn "pub fn as_slice\|pub const fn state\|pub fn state\|pub const fn new" crates/nexus/src/events.rs crates/nexus/src/aggregate.rs crates/nexus/src/version.rs`
Expected: confirms `Events::as_slice`, `AggregateRoot::state`, `Version::new` exist. If any name differs (e.g. state accessor), adjust the assertions to the real accessor before proceeding.

- [ ] **Step 3: Run the integration tests**

Run: `nix develop -c cargo test -p nexus-store --test execute_tests`
Expected: PASS (5 tests). The lifecycle category is Task 5 (needs a persistent adapter).

- [ ] **Step 4: Commit**

```bash
git add crates/nexus-store/tests/execute_tests.rs
git commit -m "test(store): execute — sequence, defensive, linearizability, equivalence (#251)"
```

---

## Task 5: Lifecycle test over `FjallStore` (write → close → reopen → verify)

**Files:**
- Create: `crates/nexus-fjall/tests/execute_lifecycle_tests.rs`

Model on the existing fjall integration tests and `examples/fjall-end-to-end` for the `FjallStore::builder(path).open()?.into_store()` open/reopen pattern. Reuse a counter-style aggregate + codec (copy the definitions from `execute_tests.rs`; a shared test-support module is overkill for one reuse — inline them).

- [ ] **Step 1: Confirm the fjall open/reopen API**

Run: `nix develop -c grep -rn "FjallStore::builder\|\.open()\|into_store\|tempdir\|TempDir" crates/nexus-fjall/tests/*.rs examples/fjall-end-to-end/src/*.rs | head`
Expected: shows the builder→open→into_store sequence and the tempdir pattern to copy.

- [ ] **Step 2: Write the lifecycle test**

Using the confirmed API, write a test that:
1. Opens a `FjallStore` at a `tempfile::TempDir` path, builds a `repository::<Counter>()`.
2. `execute`s two `Add` commands; asserts durability of the returned events and version.
3. **Drops** the store/keyspace (close), reopens at the same path, `load`s, and asserts `state().total` and `version()` survived.
4. `execute`s a rejected `Add(0)`; reopens and asserts the stream is unchanged (rejection persisted nothing across a reopen).

```rust
// Skeleton — fill open/reopen with the API confirmed in Step 1.
#[tokio::test]
async fn lifecycle_execute_survives_reopen() {
    let dir = tempfile::TempDir::new().unwrap();
    {
        let store = /* FjallStore::builder(dir.path()).open().unwrap().into_store() */;
        let repo = store.repository::<Counter>().codec(CtrCodec).build();
        let mut ctr = repo.load(CtrId::new(1)).await.unwrap();
        repo.execute(&mut ctr, Add(3)).await.unwrap();
        repo.execute(&mut ctr, Add(4)).await.unwrap();
        assert_eq!(ctr.state().total, 7);
    } // store dropped — closed

    let store = /* reopen at dir.path() */;
    let repo = store.repository::<Counter>().codec(CtrCodec).build();
    let reloaded = repo.load(CtrId::new(1)).await.unwrap();
    assert_eq!(reloaded.state().total, 7);
    assert_eq!(reloaded.version(), Version::new(2));

    // Rejection persists nothing across the open store.
    let mut ctr = repo.load(CtrId::new(1)).await.unwrap();
    let err = repo.execute(&mut ctr, Add(0)).await.unwrap_err();
    assert!(matches!(err, ExecuteError::Decide(_)));
    let after = repo.load(CtrId::new(1)).await.unwrap();
    assert_eq!(after.version(), Version::new(2)); // unchanged
}
```

- [ ] **Step 3: Confirm `nexus-fjall` has the test deps**

Run: `nix develop -c grep -n "tempfile\|tokio\|nexus-store" crates/nexus-fjall/Cargo.toml`
Expected: `tokio` (with `macros`/`rt`) and `tempfile` under `[dev-dependencies]`, plus `nexus-store` with the `testing` feature if the codec helpers need it. If `tempfile` is missing, add it: `nix develop -c cargo add --dev tempfile -p nexus-fjall` (never hand-edit versions — CLAUDE convention).

- [ ] **Step 4: Run the lifecycle test**

Run: `nix develop -c cargo test -p nexus-fjall --test execute_lifecycle_tests`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/nexus-fjall/tests/execute_lifecycle_tests.rs crates/nexus-fjall/Cargo.toml
git commit -m "test(fjall): execute lifecycle — durable across reopen (#251)"
```

---

## Task 6: Dogfood — migrate the fjall example, close #227's repetition

**Files:**
- Modify: `examples/fjall-end-to-end/src/lib.rs` (sites ~98-104, 140-141, 214-216)
- Modify: `examples/fjall-end-to-end/README.md`, `examples/fjall-end-to-end/src/lib.rs:30` header prose

- [ ] **Step 1: Add the import**

In `examples/fjall-end-to-end/src/lib.rs`, add `CommandRepository` to the `nexus_store` import group.

- [ ] **Step 2: Collapse each two-step into `execute`**

Replace every pair of the form:

```rust
let decided = account.handle(SomeCmd { .. })?;
repo.save(&mut account, &decided).await?;
```

with:

```rust
repo.execute(&mut account, SomeCmd { .. }).await?;
```

Concretely, in `seed_account` the `OpenAccount` pair (lines ~98-101) and the `Deposit` loop pair (lines ~103-104) each collapse to one `execute` line; likewise the `Withdraw` site (~140-141) and the `Deposit` site (~214-216). Where the returned events were bound and inspected, keep the binding: `let decided = repo.execute(&mut account, cmd).await?;`.

- [ ] **Step 3: Update the prose**

In the `lib.rs` header comment (line ~30) and `README.md` (line ~36), change the "`repo.load(id)` / `repo.save(..)`" description to present `repo.execute(&mut root, cmd)` as the sanctioned one-call command path, noting `handle` + `save` remains the escape hatch for inspecting events before they land.

- [ ] **Step 4: Run the example's tests**

Run: `nix develop -c cargo test -p fjall-end-to-end`
Expected: PASS (behavior is identical; `execute` == `handle` + `save`).

- [ ] **Step 5: Commit**

```bash
git add examples/fjall-end-to-end/
git commit -m "docs(example): use execute in fjall-end-to-end, closes #227 repetition (#251)"
```

---

## Task 7: Final gate + issue close-out

- [ ] **Step 1: Let the full gate run on a no-op commit or rely on the last commit's hook**

The pre-commit hook already ran `nix flake check` on each commit above. If you want a clean final confirmation without a code change, run the manual all-targets clippy the flake's `--lib` gate skips (CLAUDE convention — the gate is `--lib` only):

Run: `nix develop -c cargo clippy --all-targets --all-features -- -D warnings`
Expected: clean (no warnings). Fix any lint in **non-test** code immediately; test-only lints get scoped `#[allow(.., reason = "..")]`.

- [ ] **Step 2: Open the PR**

Branch was created off `origin/main` at the start (see execution note). Push and open with the `joeldsouzax` gh account:

```bash
git push -u origin feat/execute-command-combinator
gh pr create --title "feat(store): execute — one-call decide+save combinator (#251)" \
  --body "$(cat <<'EOF'
Closes #251. Resolves the #227 per-command two-step repetition.

Adds `CommandRepository::execute(&mut root, cmd)` — a blanket-impl'd combinator
that fuses the pure `AggregateRoot::handle` with the atomic `Repository::save`,
mirroring `SagaRepository::react_and_save`. The decided events are never named
by the caller, so the decide→save hand-off can't be forgotten or misthreaded.

- `handle` and `save` unchanged; `execute` is a pure shell (equivalence-tested).
- Two-domain `ExecuteError` (`Decide` | `Store`); conflict surfaced, never retried
  (rule 5), consistent with the saga side.
- `ConflictPredicate` extracted to a shared `conflict` module (saga behavior unchanged).
- 4 cross-cutting test categories + equivalence over `InMemoryStore`; lifecycle over `FjallStore`.
- `fjall-end-to-end` example migrated to `execute`.

Design: docs/plans/2026-07-02-execute-command-combinator-design.md

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Self-Review Notes (author checklist — done)

- **Spec coverage:** trait/blanket impl (T3) ✓, `ExecuteError` two domains + `is_conflict` (T2) ✓, return decided events (T3) ✓, conflict-not-retry (T3 docs + T4 test) ✓, `ConflictPredicate` shared move (T1) ✓, placement in `execute.rs` mirroring saga (T2/T3) ✓, all 4 test categories (T4 seq/defensive/linearizability + T5 lifecycle) ✓, equivalence test (T4) ✓, example migration/#227 (T6) ✓, `#[non_exhaustive]` freeze carve-out on the error (T2) ✓, scope-limited to `execute` only (no load+execute) ✓.
- **Type consistency:** `CommandRepository`, `ExecuteError<DecideErr, StoreErr>`, `Decide`/`Store` variants, `execute<C, const N: usize>`, `Counter`/`CtrId`/`CtrEvent`/`CtrState`/`CtrError`/`Add`/`CtrCodec` used consistently across T2-T6.
- **Watch-items flagged for the implementer:** (1) Task 2/3 commit-together to avoid transient `unused_imports`; (2) the Cyrillic-`Р` typo guard in T4; (3) verify `Events::as_slice`/`AggregateRoot::state`/`Version::new` accessor names in T4 Step 2 before trusting the assertions; (4) confirm fjall open/reopen + `tempfile` dep in T5 rather than assuming.
