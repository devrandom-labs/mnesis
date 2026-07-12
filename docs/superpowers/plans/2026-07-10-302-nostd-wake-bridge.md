# no_std WakeSource Bridge (`mnesis-wake-nostd`, #302) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A no_std+alloc `WakeSource` impl (`GlobalWake`, one global eventcount) in a new `mnesis-wake-nostd` crate, proven by the 4-category test suite, an embassy-executor end-to-end subscription test, and bare-metal compile gates.

**Architecture:** One `Arc<Inner { AtomicU64 generation, event_listener::Event }>` shared by all registrations; `wake` bumps the generation and notifies all listeners; `arm` captures the generation synchronously, then (listener-register → generation-recheck → await) closes the lost-wakeup window. Spurious wakes are contract-permitted, so no per-stream routing, no map, no mutex.

**Tech Stack:** Rust (pinned stable via `rust-toolchain.toml`, edition 2024), `event-listener` 5 (no_std mode), dev-only: tokio (race tests), embassy-executor 0.10 `platform-std` (acceptance test), `mnesis-inmemory` (store double).

**Spec:** `docs/superpowers/specs/2026-07-10-nostd-wakesource-design.md`
**Branch:** `feat/302-nostd-wake-bridge` (already created off `origin/main`; spec committed as `265c03f`)

**Facts already verified (do not re-derive):**
- `core::convert::Infallible` satisfies `core::error::Error + Send + Sync + 'static` on the pinned toolchain (compile-probed 2026-07-10) → `type Error = Infallible` is legal for `WakeSource`.
- `event_listener::Event::listen()` registers the listener into the notify list **at creation** (verified in the v5 source), and `notify()` emits a `SeqCst` fence when needed. no_std mode = `default-features = false` (requires alloc; `listen()` boxes).
- The flake's `mnesis-store-nostd` gate **already** builds `mnesis-store --no-default-features --features subscription,export,import,snapshot,projection` for `thumbv7em-none-eabihf` (#301) — the card's "verify the generic loop compiles bare-metal" item is already covered. New gate work = building `mnesis-wake-nostd` itself for the two targets.
- `Id` has a blanket impl (`mnesis/src/id.rs:42`); `StreamKey` (derives `Clone, Debug, Hash, PartialEq, Eq`, has `Display` + `AsRef<[u8]>`) is an `Id` — usable directly as the subscribe id in tests.
- embassy-executor newest published version is **0.10.0**; host features are `platform-std` + `executor-thread` (the pre-0.10 name `arch-std` is gone). `Spawner::spawn(token)` returns `()`; the `#[embassy_executor::task]` fn itself returns a `Result` (main-branch std example: `spawner.spawn(run().unwrap())`).
- `InMemoryStore` assoc types: `Error = InMemoryStoreError`, `Stream = InMemoryStream`, `AllPosition = InMemoryAllPos`, `AllStream = InMemoryAllStream`; constructor `InMemoryStore::new()`. Its `RawEventStore` impl uses `async fn` for the `-> impl Future` trait methods (allowed; precedent at `mnesis-inmemory/src/lib.rs:379`).
- `tokio::sync` primitives (used inside `InMemoryStore`) do not need a tokio runtime — awaiting them under embassy works.

**Conventions that bite (from memories/CLAUDE.md):**
- NEVER run the full `nix flake check` by hand before committing — the pre-commit hook runs it. Targeted verification only (`nix develop -c cargo nextest run -p mnesis-wake-nostd`).
- `git add` new files BEFORE committing triggers the hook correctly (`nix flake check` ignores untracked files — an untracked module makes the gate fail on a missing file).
- Run `nix develop -c cargo fmt --all` after substantial edits, before staging.
- Use `cargo add` for new deps (never hand-write versions), then hoist the resolved version to root `[workspace.dependencies]` and set `workspace = true` in the crate (repo convention: versions declared once at the root).
- `cargo hakari generate` after dependency-graph changes.
- gh CLI as the `joeldsouzax` account.
- Don't thrash feature sets: all tests run under the crate's default (only) feature set.

---

## File Structure

```
crates/mnesis-wake-nostd/
  Cargo.toml                      # no_std crate: mnesis-store(subscription) + event-listener
  src/lib.rs                      # GlobalWake + GlobalWakeReg (~150 lines incl. docs)
  tests/wake_contract.rs          # 4-category suite (tokio dev-dep)
  tests/embassy_subscription.rs   # acceptance: embassy drives a real Subscription
Cargo.toml                        # members += mnesis-wake-nostd; workspace dep event-listener
.config/hakari.toml               # final-excludes += mnesis-wake-nostd
flake.nix                         # mnesis-nostd + mnesis-wasm gates build the new crate
CLAUDE.md                         # crate graph + subscription-machinery entry
```

---

### Task 1: Scaffold the `mnesis-wake-nostd` crate

**Files:**
- Create: `crates/mnesis-wake-nostd/Cargo.toml`
- Create: `crates/mnesis-wake-nostd/src/lib.rs` (skeleton)
- Modify: `Cargo.toml` (root: `members`, `[workspace.dependencies]`)
- Modify: `.config/hakari.toml` (`final-excludes`)

- [ ] **Step 1: Add the crate to the workspace members**

In root `Cargo.toml`, insert into `members` (alphabetical, after `"crates/mnesis-store-testing"`):

```toml
    "crates/mnesis-wake",
    "crates/mnesis-wake-nostd",
```

(The `"crates/mnesis-wake",` line already exists — add only the `mnesis-wake-nostd` line after it.)

- [ ] **Step 2: Create the crate manifest and skeleton**

Create `crates/mnesis-wake-nostd/Cargo.toml`:

```toml
[package]
name = "mnesis-wake-nostd"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true
description = "no_std global-eventcount WakeSource for on-device live-tail subscriptions"
readme = "../../README.md"
keywords = ["event-sourcing", "subscription", "wake", "no-std", "embedded"]
categories = ["embedded", "no-std", "asynchronous", "data-structures"]

[dependencies]
mnesis-store = { version = "0.1.0", path = "../mnesis-store", default-features = false, features = ["subscription"] }

[lints]
workspace = true
```

(No `workspace-hack` dep — this crate is hakari-excluded in Step 4, the `mnesis-nostd-smoketest`/`mnesis-store` precedent: a std workspace-hack edge would poison the bare-metal build.)

Create `crates/mnesis-wake-nostd/src/lib.rs`:

```rust
//! Global-eventcount [`WakeSource`] for no_std on-device live-tail
//! subscriptions (#302).
//!
//! # What this is
//!
//! The no_std counterpart to `mnesis-wake`'s tokio-backed `StreamNotifiers`:
//! a [`WakeSource`] built on `core` atomics plus [`event_listener::Event`]
//! (an eventcount — no_std + alloc), so the generic catch-up-then-live-tail
//! loop in `mnesis-store` can park on a device with no tokio and no OS.
//!
//! **Optional and executor-dependent.** The primary device model is
//! append-and-sync: events are produced locally and synced to a server,
//! where subscriptions run under tokio via `mnesis-wake`. Reach for this
//! crate only when a device genuinely runs an *on-device* live-tail —
//! driving the subscription then also needs a no_std executor (e.g.
//! embassy). Any executor works: the only requirement is polling the
//! futures this crate returns.
//!
//! # Global, not per-stream — spurious wakes by design
//!
//! There is ONE eventcount for the whole store: every [`wake`] rouses every
//! parked registration, per-stream and `$all` alike. The
//! [`WakeRegistration::arm`] contract explicitly permits spurious wakes —
//! the subscription loop's response is a single empty re-scan followed by
//! re-parking. Per-stream routing is a throughput optimization for the
//! server-side many-actor-streams workload (that is `StreamNotifiers`' job,
//! with its keyed registry and drop-guard reaping); an on-device live-tail
//! has a handful of subscriptions, so the routing machinery (a map, a
//! no_std mutex, guards) would cost more than the false wakes it saves. If
//! a real device workload ever shows herd cost, a routed impl can be added
//! as a second type in this crate — an additive, non-breaking change.
//!
//! # Lost-wakeup discipline
//!
//! [`arm`](WakeRegistration::arm) captures a seen-generation synchronously,
//! then the returned future (1) creates a listener — registered into the
//! notify list AT CREATION, before any check — (2) re-checks the
//! generation, returning if it moved, and (3) otherwise awaits the
//! listener. A wake between `arm` and the first poll bumps the generation
//! (caught by 2); a wake after is delivered to the already-registered
//! listener (caught by 3). No wake after `arm` is ever lost.
//!
//! [`wake`]: WakeSource::wake

#![no_std]

extern crate alloc;
```

- [ ] **Step 3: Add the `event-listener` dependency via cargo add, then hoist to the workspace**

```bash
cd /Users/joel/Code/devrandom/mnesis
nix develop -c cargo add -p mnesis-wake-nostd event-listener --no-default-features
```

Expected: cargo resolves and writes `event-listener = { version = "5.4.1", default-features = false }` (or newer 5.x) into `crates/mnesis-wake-nostd/Cargo.toml`.

Then hoist: move the resolved version spec into root `Cargo.toml` `[workspace.dependencies]` (alphabetical, after `criterion`):

```toml
event-listener = { version = "5.4.1", default-features = false }
```

and replace the crate-level entry with:

```toml
event-listener = { workspace = true }
```

(Use whatever exact version cargo add resolved, not necessarily 5.4.1.)

- [ ] **Step 4: Exclude the crate from workspace-hack and regenerate**

In `.config/hakari.toml`, extend `final-excludes` `workspace-members` and its comment block:

```toml
# `mnesis-wake-nostd` is the no_std WakeSource bridge (#302): built for
# thumbv7em/wasm32 by the no_std gates, so a std workspace-hack edge would
# break it — same reasoning as `mnesis-nostd-smoketest`.
workspace-members = ["mnesis", "mnesis-nostd-smoketest", "mnesis-store", "mnesis-wake-nostd"]
```

(Keep the existing comment lines for the other three members; add the new comment above the list.)

Then:

```bash
nix develop -c cargo hakari generate
```

Expected: exits 0; `crates/workspace-hack/Cargo.toml` may or may not change (commit whatever it produces).

- [ ] **Step 5: Verify the empty crate builds and commit**

```bash
nix develop -c cargo check -p mnesis-wake-nostd
nix develop -c cargo fmt --all
git add -A
git commit -m "feat(wake-nostd): scaffold mnesis-wake-nostd crate (#302)

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

Expected: check passes; the pre-commit hook runs `nix flake check` and passes.

---

### Task 2: `GlobalWake` — TDD the core contract

**Files:**
- Create: `crates/mnesis-wake-nostd/tests/wake_contract.rs`
- Modify: `crates/mnesis-wake-nostd/src/lib.rs`
- Modify: `crates/mnesis-wake-nostd/Cargo.toml` (dev-deps)

- [ ] **Step 1: Add tokio dev-dependency**

```bash
nix develop -c cargo add -p mnesis-wake-nostd --dev tokio --features macros,rt-multi-thread,time,sync
```

Expected: tokio is already a workspace dependency, so cargo writes `tokio = { workspace = true, features = [...] }`. If it writes a concrete version instead, rewrite the entry by hand to `tokio = { workspace = true, features = ["macros", "rt-multi-thread", "time", "sync"] }` (the workspace already declares tokio's version).

- [ ] **Step 2: Write the first failing tests**

Create `crates/mnesis-wake-nostd/tests/wake_contract.rs`:

```rust
//! Contract tests for `GlobalWake` — the 4 cross-cutting categories.
//!
//! Category 1 (sequence/protocol) lives here; categories 2–4 are added in
//! the sibling test modules below as the suite grows (same file).

#![allow(clippy::unwrap_used, reason = "test code")]
#![allow(clippy::expect_used, reason = "test code")]

use std::time::Duration;

use mnesis_store::wake::{WakeRegistration, WakeSource};
use mnesis_wake_nostd::GlobalWake;
use tokio::time::timeout;

/// Generous upper bound on "a wake must arrive". A timeout here means a
/// genuinely lost wakeup.
const MUST_WAKE: Duration = Duration::from_secs(5);
/// Short bound for asserting the negative: "this waiter must NOT be woken".
const MUST_NOT_WAKE: Duration = Duration::from_millis(150);

// ───────────────── Category 1: sequence / protocol ─────────────────

/// register → arm → wake resolves the armed future.
#[tokio::test]
async fn register_arm_wake_resolves() {
    let wake = GlobalWake::new();
    let reg = wake.register(Some(b"stream-a")).unwrap();
    let wait = reg.arm();
    wake.wake(b"stream-a");
    timeout(MUST_WAKE, wait)
        .await
        .expect("armed wait must resolve after wake");
}

/// A resolved arm is one-shot: re-arming after the wake needs a FRESH wake.
#[tokio::test]
async fn rearm_requires_fresh_wake() {
    let wake = GlobalWake::new();
    let reg = wake.register(Some(b"s")).unwrap();

    let first = reg.arm();
    wake.wake(b"s");
    timeout(MUST_WAKE, first)
        .await
        .expect("first armed wait must resolve");

    // Re-arm AFTER the wake: the old generation bump must not satisfy it.
    let second = reg.arm();
    assert!(
        timeout(MUST_NOT_WAKE, second).await.is_err(),
        "re-armed wait must not resolve from a pre-arm wake"
    );

    let third = reg.arm();
    wake.wake(b"s");
    timeout(MUST_WAKE, third)
        .await
        .expect("re-armed wait must resolve after a fresh wake");
}

/// An armed wait with no wake at all stays pending.
#[tokio::test]
async fn armed_without_wake_stays_pending() {
    let wake = GlobalWake::new();
    let reg = wake.register(None).unwrap();
    let wait = reg.arm();
    assert!(
        timeout(MUST_NOT_WAKE, wait).await.is_err(),
        "armed wait must stay pending with no wake"
    );
}
```

- [ ] **Step 3: Run the tests to verify they fail**

```bash
nix develop -c cargo nextest run -p mnesis-wake-nostd
```

Expected: compile error — `GlobalWake` does not exist. (In Rust TDD, failure-to-compile is the red step.)

- [ ] **Step 4: Implement `GlobalWake`**

Append to `crates/mnesis-wake-nostd/src/lib.rs` (after the `extern crate alloc;` line):

```rust
use alloc::sync::Arc;
use core::convert::Infallible;
use core::fmt;
use core::future::Future;
use core::sync::atomic::{AtomicU64, Ordering};

use event_listener::Event;
use mnesis_store::wake::{WakeRegistration, WakeSource};

/// Shared eventcount state — one per store, shared by every registration.
struct Inner {
    /// Wake generation. Bumped on every [`WakeSource::wake`]; `fetch_add`
    /// wraps on overflow BY DESIGN — the value is compared for inequality
    /// only, and a false equality needs exactly 2^64 intervening wakes
    /// between an `arm` and its first poll, unreachable in practice (the
    /// same wrapping discipline as `StreamNotifiers`' generations).
    generation: AtomicU64,
    /// Multi-waiter eventcount. `listen()` registers the listener into the
    /// notify list at creation, which is what makes the arm-then-recheck
    /// sequence in [`WakeRegistration::arm`] lost-wakeup-safe.
    event: Event,
}

/// Global (unrouted) wake source: one eventcount for the whole store.
///
/// Cheap to clone (one `Arc` increment). The store adapter holds one and
/// calls [`WakeSource::wake`] after each durable commit; every parked
/// registration — any stream, `$all` — wakes on every commit. See the
/// crate docs for why this is correct (spurious wakes are permitted) and
/// when to prefer `mnesis-wake`'s routed `StreamNotifiers` instead.
#[derive(Clone)]
pub struct GlobalWake {
    inner: Arc<Inner>,
}

impl GlobalWake {
    /// Create a fresh wake source with no parked registrations.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                generation: AtomicU64::new(0),
                event: Event::new(),
            }),
        }
    }
}

impl Default for GlobalWake {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for GlobalWake {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GlobalWake").finish_non_exhaustive()
    }
}

/// Registration handle: a clone of the shared eventcount. Carries no
/// per-stream identity (the source is global) and needs no drop-guard
/// (there is no per-stream entry to reap).
pub struct GlobalWakeReg {
    inner: Arc<Inner>,
}

impl fmt::Debug for GlobalWakeReg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GlobalWakeReg").finish_non_exhaustive()
    }
}

impl WakeSource for GlobalWake {
    type Registration = GlobalWakeReg;
    type Error = Infallible;

    /// Register interest. The stream key is deliberately ignored — every
    /// registration watches the one global eventcount (see crate docs).
    ///
    /// # Errors
    ///
    /// None — registration is a single `Arc` clone. `Infallible` makes
    /// that unrepresentable rather than documenting an unused error path.
    fn register(&self, _stream: Option<&[u8]>) -> Result<GlobalWakeReg, Infallible> {
        Ok(GlobalWakeReg {
            inner: Arc::clone(&self.inner),
        })
    }

    fn wake(&self, _stream: &[u8]) {
        // Release pairs with the Acquire loads in `arm`: a registration
        // that observes the bumped generation also observes everything
        // the waker wrote before calling `wake` (the durable commit).
        self.inner.generation.fetch_add(1, Ordering::Release);
        // Rouse every parked listener; `notify` provides its own fence.
        self.inner.event.notify(usize::MAX);
    }
}

impl WakeRegistration for GlobalWakeReg {
    fn arm(&self) -> impl Future<Output = ()> + Send + 'static {
        let inner = Arc::clone(&self.inner);
        // Captured NOW, synchronously — this is the contract's "seen
        // version at the moment `arm` is called". A wake landing between
        // this load and the future's first poll bumps the generation and
        // is caught by the re-check below.
        let seen = inner.generation.load(Ordering::Acquire);
        async move {
            // Registered into the notify list at creation — BEFORE the
            // generation re-check — so a wake after the re-check is
            // delivered to this listener, never lost.
            let listener = inner.event.listen();
            if inner.generation.load(Ordering::Acquire) != seen {
                return;
            }
            listener.await;
        }
    }
}
```

- [ ] **Step 5: Run the tests to verify they pass**

```bash
nix develop -c cargo nextest run -p mnesis-wake-nostd
```

Expected: 3 tests PASS.

If clippy (at commit time) rejects `fetch_add` under a restriction lint, replace it with the equivalent explicit-wrapping form and keep the doc comment:
`self.inner.generation.fetch_update(Ordering::Release, Ordering::Relaxed, |g| Some(g.wrapping_add(1)))` is NOT needed — `fetch_add` on atomics is defined as wrapping and is not an arithmetic operator; only change this if the gate actually complains.

- [ ] **Step 6: Format and commit**

```bash
nix develop -c cargo fmt --all
git add -A
git commit -m "feat(wake-nostd): GlobalWake global-eventcount WakeSource (#302)

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

Expected: hook passes.

---

### Task 3: Complete the 4-category test suite

**Files:**
- Modify: `crates/mnesis-wake-nostd/tests/wake_contract.rs`

- [ ] **Step 1: Add the remaining tests**

Append to `crates/mnesis-wake-nostd/tests/wake_contract.rs`:

```rust
// ───────────────── Category 2: lifecycle ─────────────────

/// A parked future keeps `Inner` alive via its own `Arc`: dropping the
/// source handle and the registration must not invalidate it, and a wake
/// through a surviving clone must still resolve it.
#[tokio::test]
async fn parked_future_survives_dropping_source_and_registration() {
    let wake = GlobalWake::new();
    let clone = wake.clone();
    let reg = wake.register(Some(b"s")).unwrap();
    let wait = reg.arm();
    drop(reg);
    drop(wake);

    clone.wake(b"s");
    timeout(MUST_WAKE, wait)
        .await
        .expect("wake through a surviving clone must resolve the parked wait");
}

/// Dropping an armed-but-unpolled future is a no-op (no hang, no panic),
/// and the source still works afterwards.
#[tokio::test]
async fn dropping_armed_future_is_sound() {
    let wake = GlobalWake::new();
    let reg = wake.register(None).unwrap();
    let wait = reg.arm();
    drop(wait);

    let wait2 = reg.arm();
    wake.wake(b"any");
    timeout(MUST_WAKE, wait2)
        .await
        .expect("source must still deliver wakes after a dropped arm");
}

// ───────────────── Category 3: defensive boundary ─────────────────

/// The global source treats every registration target identically:
/// `$all` (None), the empty key, and any named key all wake on any wake.
#[tokio::test]
async fn all_registration_targets_are_equivalent() {
    let wake = GlobalWake::new();
    let reg_all = wake.register(None).unwrap();
    let reg_empty = wake.register(Some(b"")).unwrap();
    let reg_named = wake.register(Some(b"some-stream")).unwrap();

    let waits = [reg_all.arm(), reg_empty.arm(), reg_named.arm()];
    wake.wake(b"a-completely-different-stream");
    for wait in waits {
        timeout(MUST_WAKE, wait)
            .await
            .expect("every registration target must wake on any stream's wake");
    }
}

/// A wake BEFORE `arm` must not satisfy the arm on its own (the seen
/// generation is captured at `arm` time, after the wake).
#[tokio::test]
async fn wake_before_arm_does_not_satisfy_arm() {
    let wake = GlobalWake::new();
    let reg = wake.register(Some(b"s")).unwrap();
    wake.wake(b"s"); // lands before arm
    let wait = reg.arm();
    assert!(
        timeout(MUST_NOT_WAKE, wait).await.is_err(),
        "a pre-arm wake must not resolve a later arm"
    );
}

/// Drives the wake purely through the trait surface (generic bound, the
/// way the subscription loop uses it) — proves trait dispatch works and
/// both a per-stream and an `$all` registration are roused.
#[tokio::test]
async fn trait_surface_wake_rouses_stream_and_all() {
    async fn arm_and_wake<W: WakeSource>(src: &W) {
        let per_stream = src.register(Some(b"k")).unwrap_or_else(|_| unreachable!());
        let all = src.register(None).unwrap_or_else(|_| unreachable!());
        let wait_stream = per_stream.arm();
        let wait_all = all.arm();
        WakeSource::wake(src, b"k");
        timeout(MUST_WAKE, wait_stream)
            .await
            .expect("trait wake must rouse the per-stream registration");
        timeout(MUST_WAKE, wait_all)
            .await
            .expect("trait wake must rouse the $all registration");
    }

    let wake = GlobalWake::new();
    arm_and_wake(&wake).await;
}

// ───────────────── Category 4: linearizability / isolation ─────────────────

/// Ported from mnesis-wake: a registration armed before a concurrent wake
/// must never miss it. Repeated to shake out scheduling races.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn armed_wait_never_loses_a_concurrent_wake() {
    use std::sync::Arc;
    use tokio::sync::Barrier;

    for _ in 0..50 {
        let wake = GlobalWake::new();
        let reg = wake.register(Some(b"k")).unwrap();
        let wait = reg.arm(); // armed BEFORE the race
        let start = Arc::new(Barrier::new(2));

        let start_prod = Arc::clone(&start);
        let waker = wake.clone();
        let prod = tokio::spawn(async move {
            start_prod.wait().await;
            waker.wake(b"k");
        });

        start.wait().await;
        timeout(MUST_WAKE, wait)
            .await
            .expect("an armed wait must not lose a concurrent wake");
        prod.await.unwrap();
    }
}

/// Concurrent arm/wake churn across many tasks completes without deadlock
/// or a lost wake: every armed-then-woken wait resolves.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_arm_wake_churn_completes() {
    use std::sync::Arc;
    use tokio::sync::Barrier;

    let wake = GlobalWake::new();
    let workers = 16usize;
    let iterations = 100usize;
    let barrier = Arc::new(Barrier::new(workers));

    let mut handles = Vec::with_capacity(workers);
    for _ in 0..workers {
        let wake = wake.clone();
        let barrier = Arc::clone(&barrier);
        handles.push(tokio::spawn(async move {
            barrier.wait().await; // all workers start together
            for _ in 0..iterations {
                let reg = wake.register(Some(b"churn")).unwrap();
                let wait = reg.arm();
                wake.wake(b"churn"); // own wake satisfies own arm (global)
                timeout(MUST_WAKE, wait)
                    .await
                    .expect("armed wait must resolve under churn");
            }
        }));
    }
    for h in handles {
        h.await.unwrap();
    }
}
```

- [ ] **Step 2: Run the full suite**

```bash
nix develop -c cargo nextest run -p mnesis-wake-nostd
```

Expected: 10 tests PASS (3 from Task 2 + 7 new).

- [ ] **Step 3: Format and commit**

```bash
nix develop -c cargo fmt --all
git add -A
git commit -m "test(wake-nostd): 4-category contract suite for GlobalWake (#302)

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 4: Acceptance test — embassy-executor drives a real `Subscription`

**Files:**
- Create: `crates/mnesis-wake-nostd/tests/embassy_subscription.rs`
- Modify: `crates/mnesis-wake-nostd/Cargo.toml` (dev-deps)

- [ ] **Step 1: Add the dev-dependencies**

```bash
nix develop -c cargo add -p mnesis-wake-nostd --dev embassy-executor --features platform-std,executor-thread
nix develop -c cargo add -p mnesis-wake-nostd --dev mnesis-inmemory --path crates/mnesis-inmemory
nix develop -c cargo add -p mnesis-wake-nostd --dev futures --features std,executor,async-await
nix develop -c cargo add -p mnesis-wake-nostd --dev mnesis --path crates/mnesis
```

Then hoist `embassy-executor`'s resolved version to root `[workspace.dependencies]` (alphabetical, after `criterion`):

```toml
embassy-executor = { version = "0.10.0", features = ["platform-std", "executor-thread"] }
```

and set the crate entry to `embassy-executor = { workspace = true }`. For `futures` and `mnesis`/`mnesis-inmemory`, cargo add should emit `workspace = true` / path entries automatically (they're existing workspace deps); fix by hand to the workspace form if not. Also give the test access to mnesis-store's std surface — the dev-dependency unifies features for test builds only, the lib build stays no_std:

```toml
[dev-dependencies]
mnesis-store = { path = "../mnesis-store", features = ["std", "subscription"] }
```

**Contingency (only if hit):** if the test build fails at link time with an undefined `_critical_section_1_0_acquire`, embassy's std platform needs a critical-section provider — run `nix develop -c cargo add -p mnesis-wake-nostd --dev critical-section --features std` and retry. Do not add it preemptively.

- [ ] **Step 2: Write the acceptance test**

Create `crates/mnesis-wake-nostd/tests/embassy_subscription.rs`:

```rust
//! #302 acceptance: `GlobalWake` drives a real `Subscription` under a
//! no_std executor (embassy). The store double delegates `RawEventStore`
//! to `InMemoryStore` (rule 8: reuse the shipped adapter, don't
//! reimplement) and `WakeSource` to `GlobalWake`; `append` wakes AFTER the
//! inner commit returns (the MUST-wake-after-durable-commit ordering).
//!
//! Flow: seed a 2-event backlog → embassy task subscribes and reports each
//! `Step` over a std mpsc channel → assert backlog, then `CaughtUp`, then
//! append from OUTSIDE the executor and assert the live event arrives —
//! the cross-thread wake path through `GlobalWake`.

#![allow(clippy::unwrap_used, reason = "test code")]
#![allow(clippy::expect_used, reason = "test code")]

use std::sync::mpsc;
use std::time::Duration;

use embassy_executor::Executor;
use futures::StreamExt;
use mnesis::Version;
use mnesis_inmemory::{
    InMemoryAllPos, InMemoryAllStream, InMemoryStore, InMemoryStoreError, InMemoryStream,
};
use mnesis_store::wake::WakeSource;
use mnesis_store::{
    AppendError, PendingEnvelope, RawEventStore, Step, Store, StreamKey, Subscription,
    pending_envelope,
};
use mnesis_wake_nostd::GlobalWake;

const MUST_DELIVER: Duration = Duration::from_secs(5);
const STREAM: &[u8] = b"device";

/// On-device store shape: the in-memory adapter for persistence, the
/// no_std `GlobalWake` for wake routing.
struct DeviceStore {
    inner: InMemoryStore,
    wake: GlobalWake,
}

impl RawEventStore for DeviceStore {
    type Error = InMemoryStoreError;
    type Stream = InMemoryStream;
    type AllPosition = InMemoryAllPos;
    type AllStream = InMemoryAllStream;

    async fn append(
        &self,
        id: &StreamKey,
        expected_version: Option<Version>,
        envelopes: &[PendingEnvelope],
    ) -> Result<(), AppendError<Self::Error>> {
        self.inner.append(id, expected_version, envelopes).await?;
        // Wake only after the commit returned Ok — the WakeSource contract.
        WakeSource::wake(&self.wake, id.as_ref());
        Ok(())
    }

    async fn read_stream(
        &self,
        id: &StreamKey,
        from: Version,
    ) -> Result<Self::Stream, Self::Error> {
        self.inner.read_stream(id, from).await
    }

    async fn read_all(
        &self,
        from: Option<Self::AllPosition>,
    ) -> Result<Self::AllStream, Self::Error> {
        self.inner.read_all(from).await
    }
}

impl WakeSource for DeviceStore {
    type Registration = <GlobalWake as WakeSource>::Registration;
    type Error = <GlobalWake as WakeSource>::Error;

    fn register(&self, stream: Option<&[u8]>) -> Result<Self::Registration, Self::Error> {
        self.wake.register(stream)
    }

    fn wake(&self, stream: &[u8]) {
        WakeSource::wake(&self.wake, stream);
    }
}

enum Msg {
    Event(u64),
    CaughtUp,
}

#[embassy_executor::task]
async fn drive(store: Store<DeviceStore>, tx: mpsc::Sender<Msg>) {
    let sub = Subscription::new(&store);
    let stream = match sub.subscribe(&StreamKey::from_slice(STREAM), None) {
        Ok(stream) => stream,
        Err(never) => match never {},
    };
    let mut stream = core::pin::pin!(stream);
    while let Some(item) = stream.next().await {
        let msg = match item.expect("in-memory reads never fail") {
            Step::Event(env) => Msg::Event(env.version().as_u64()),
            Step::CaughtUp => Msg::CaughtUp,
        };
        if tx.send(msg).is_err() {
            return; // test thread is done asserting; wind the task down
        }
    }
}

async fn append_one(store: &Store<DeviceStore>, version: u64, expected: Option<u64>) {
    let env = pending_envelope(Version::new(version).expect("nonzero version"))
        .event_type("DeviceEvent")
        .payload(b"p".to_vec())
        .build()
        .expect("valid envelope");
    let expected = expected.and_then(Version::new);
    store
        .append(&StreamKey::from_slice(STREAM), expected, &[env])
        .await
        .expect("append must succeed");
}

#[test]
fn embassy_executor_drives_catch_up_then_live_tail() {
    let store = DeviceStore {
        inner: InMemoryStore::new(),
        wake: GlobalWake::new(),
    }
    .into_store();

    // Seed a 2-event backlog before subscribing.
    futures::executor::block_on(async {
        append_one(&store, 1, None).await;
        append_one(&store, 2, Some(1)).await;
    });

    let (tx, rx) = mpsc::channel();
    let sub_store = store.clone();
    std::thread::spawn(move || {
        // `Executor::run` never returns; the leak is reclaimed at process
        // exit (nextest runs one process per test).
        let executor: &'static mut Executor = Box::leak(Box::new(Executor::new()));
        executor.run(|spawner| spawner.spawn(drive(sub_store, tx).unwrap()));
    });

    // Catch-up: the backlog in order, then the boundary marker.
    assert!(matches!(rx.recv_timeout(MUST_DELIVER), Ok(Msg::Event(1))));
    assert!(matches!(rx.recv_timeout(MUST_DELIVER), Ok(Msg::Event(2))));
    assert!(matches!(rx.recv_timeout(MUST_DELIVER), Ok(Msg::CaughtUp)));

    // Live: append from OUTSIDE the embassy executor — the wake must cross
    // threads through GlobalWake and rouse the parked subscription.
    futures::executor::block_on(append_one(&store, 3, Some(2)));
    assert!(matches!(rx.recv_timeout(MUST_DELIVER), Ok(Msg::Event(3))));
}
```

Note on the spawn line: embassy 0.10's `Spawner::spawn` takes a `SpawnToken` and returns `()`; the `#[embassy_executor::task]`-generated `drive(...)` returns a `Result` (pool acquisition), hence `spawner.spawn(drive(sub_store, tx).unwrap())`. If the resolved embassy version instead generates a token-returning fn, the compiler will say so — then the line is `spawner.spawn(drive(sub_store, tx)).unwrap()`. One of the two forms compiles; both express "spawn or panic in a test".

- [ ] **Step 3: Run the acceptance test**

```bash
nix develop -c cargo nextest run -p mnesis-wake-nostd
```

Expected: 11 tests PASS (10 prior + `embassy_executor_drives_catch_up_then_live_tail`).

- [ ] **Step 4: Regenerate hakari (dep graph changed), format, commit**

```bash
nix develop -c cargo hakari generate
nix develop -c cargo fmt --all
git add -A
git commit -m "test(wake-nostd): embassy-executor end-to-end subscription acceptance (#302)

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

Expected: hook passes (cargo-deny will vet embassy-executor: MIT OR Apache-2.0, compatible).

---

### Task 5: Bare-metal compile gates

**Files:**
- Modify: `flake.nix` (the `mnesis-wasm` and `mnesis-nostd` checks, ~lines 140–156)

- [ ] **Step 1: Add the new crate to both no_std gates**

In `flake.nix`, `mnesis-wasm` check — append one line to `buildPhaseCargoCommand`:

```nix
          mnesis-wasm = craneLib.mkCargoDerivation (commonArgs // {
            inherit cargoArtifacts;
            pname = "mnesis-wasm";
            buildPhaseCargoCommand = ''
              cargo build -p mnesis --target wasm32-unknown-unknown --no-default-features
              cargo build -p mnesis-nostd-smoketest --target wasm32-unknown-unknown --no-default-features --features derive
              cargo build -p mnesis-wake-nostd --target wasm32-unknown-unknown
            '';
          });
```

And `mnesis-nostd` (the STRONG bare-metal gate):

```nix
          mnesis-nostd = craneLib.mkCargoDerivation (commonArgs // {
            inherit cargoArtifacts;
            pname = "mnesis-nostd";
            buildPhaseCargoCommand = ''
              cargo build -p mnesis --target thumbv7em-none-eabihf --no-default-features
              cargo build -p mnesis-nostd-smoketest --target thumbv7em-none-eabihf --no-default-features --features derive
              cargo build -p mnesis-wake-nostd --target thumbv7em-none-eabihf
            '';
          });
```

(No `--no-default-features` flag needed — the crate has no features. `-p mnesis-wake-nostd` builds only the lib and its normal deps; dev-deps — tokio, embassy — never enter a `cargo build` of the lib target.)

Also update the gate comment block above `mnesis-wasm` (the `# no_std gates …` comment, flake.nix:130–139) — append one line:

```nix
          # `mnesis-wake-nostd` (#302) also builds on both targets: the no_std
          # WakeSource bridge, proving event-listener + the wake traits are
          # core+alloc clean.
```

- [ ] **Step 2: Build both gates directly (measure, don't assert)**

```bash
nix build .#checks.aarch64-darwin.mnesis-nostd .#checks.aarch64-darwin.mnesis-wasm
```

Expected: both build. **If `event-listener` fails on thumbv7em** (e.g. missing atomics support), that is a design finding, not something to paper over — surface it to the user before choosing between the `portable-atomic` or `critical-section` feature of event-listener (each has target implications).

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "ci(flake): build mnesis-wake-nostd in the thumbv7em/wasm32 no_std gates (#302)

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 6: Documentation

**Files:**
- Modify: `CLAUDE.md` (crate graph + subscription machinery)

- [ ] **Step 1: Update the CLAUDE.md crate dependency graph**

In `CLAUDE.md`, "Crate Dependency Graph" section, add a line to the graph after the `mnesis-wake` line:

```
mnesis-wake       --> mnesis-store            (in-process wake registry; owns tokio/foldhash/parking_lot)
mnesis-wake-nostd --> mnesis-store            (no_std global-eventcount wake; owns event-listener; #302)
```

- [ ] **Step 2: Update the subscription-machinery notes**

In `CLAUDE.md`, inside the "Subscription machinery" bullet (the `mnesis-wake` crate paragraph), append after the existing `**mnesis-wake crate**` sub-bullet:

```markdown
  - **`mnesis-wake-nostd` crate** (#302) — `GlobalWake`, the *no_std+alloc* `WakeSource`: ONE global eventcount (`AtomicU64` generation + `event_listener::Event`) shared by every registration, per-stream and `$all` alike — every commit wakes every parked subscription, and each false wake costs one empty re-scan (spurious wakes are contract-permitted, so routing is an optimization, not correctness; a routed impl can be added later as an additive sibling type). `arm` captures the seen generation synchronously, then listener-register (at `listen()` creation) → generation-recheck → await closes the lost-wakeup window. `register` is `Infallible`. **Optional and executor-dependent**: the primary device model is append-and-sync (subscriptions run server-side under `mnesis-wake`); this exists for genuine on-device live-tail under a no_std executor (embassy), proven by the embassy-executor acceptance test. Built for `thumbv7em`/`wasm32` by the flake's no_std gates; hakari-excluded like `mnesis-store`.
```

- [ ] **Step 3: Format check and commit**

```bash
git add -A
git commit -m "docs: mnesis-wake-nostd in crate graph and subscription notes (#302)

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

- [ ] **Step 4: Note the new crate name on #280 (crates.io reservation)**

```bash
gh issue comment 280 --repo devrandom-labs/mnesis --body "#302 adds a new publishable crate: \`mnesis-wake-nostd\` (no_std WakeSource bridge). Add it to the crates.io reservation list."
```

Expected: comment posted (gh must be on the joeldsouzax account — verify with `gh auth status` if unsure).

---

### Task 7: Finish

- [ ] **Step 1: Full-suite sanity** — the per-commit hooks have run `nix flake check` at every step; run the workspace test suite once more only if any doubt remains from earlier steps (`nix develop -c cargo nextest run`). Do NOT hand-run `nix flake check`.

- [ ] **Step 2: Hand off to the finishing skill** — use superpowers:finishing-a-development-branch: push `feat/302-nostd-wake-bridge`, open the PR (`gh pr create`) titled `feat(wake-nostd): no_std WakeSource bridge — GlobalWake (#302)`, body maps the two #302 acceptance checkboxes to artifacts:
  - "a no_std `WakeSource` impl drives `Subscription` under a no_std executor in a test" → `tests/embassy_subscription.rs`
  - "documented as optional / executor-dependent" → crate-level docs + CLAUDE.md entry
  - plus the finding that the loop's bare-metal compile was already gated by #301 (`mnesis-store-nostd` builds `--features subscription,…` for thumbv7em); the new gates add `mnesis-wake-nostd` itself.
  - Merge policy: squash via `gh pr merge --squash --delete-branch` after CI (user merges or approves merge).
