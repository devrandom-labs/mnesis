# no_std WakeSource bridge — `nexus-wake-nostd` (#302)

**Date:** 2026-07-10
**Issue:** [#302 — [freeze] no_std WakeSource bridge — on-device live-tail subscriptions](https://github.com/devrandom-labs/nexus/issues/302)
**Status:** Approved design, pre-implementation
**Depends on:** #300 (WakeSource trait standalone, merged), #301 (store no_std, merged `7d75a78`)

## Problem

`WakeSource` (`nexus-store/src/wake.rs`) is the adapter-pluggable wake seam the
generic catch-up-then-live-tail loop parks on. The only in-process impl is
`StreamNotifiers` in `nexus-wake` — tokio-backed (`Notify` + `watch`
generations), so a no_std device cannot run an on-device live-tail
subscription. The primary device model is append-and-sync (subscriptions live
server-side), so this bridge is **optional and additive** — scheduled pre-freeze
only to complete the no_std story end-to-end. It is not a hard freeze gate.

Additionally, the generic loop (`subscription_cursor.rs`, `wake.rs`,
`catchup.rs` behind the dep-free `subscription` feature) *reads* as no_std but
has never been **compiled** for a bare-metal target. Rule 9: measure, don't
assert.

## Decisions (settled during brainstorming)

1. **Home: new sibling crate `crates/nexus-wake-nostd`** (publishable).
   Mirrors the #300 split — one crate per wake backend, each owning its own
   deps. `nexus-wake` keeps owning tokio; `nexus-store`'s `subscription`
   feature stays dep-free. Name follows the `nexus-nostd-smoketest` convention.
2. **Scope: global eventcount, no per-stream routing.** The
   `WakeRegistration::arm` contract explicitly permits spurious wakes, and the
   loop's response to one is a single empty re-scan. Per-stream routing is a
   throughput optimization for the server-side many-actor-streams workload
   (that is `StreamNotifiers`' job); an on-device live-tail has a handful of
   subscriptions. One shared `(AtomicU64 generation, event_listener::Event)`
   wakes every registration on every commit: no map, no mutex dep, no
   drop-guard. A routed no_std impl remains possible later as an additive
   sibling type in the same crate.
3. **Acceptance-test executor: `embassy-executor`** (arch-std, host,
   dev-dependency only) — the exact executor the card names. Concurrency and
   lost-wakeup race tests run under tokio multi-thread (dev-only) for real
   cross-thread overlap.

## Design

### Crate: `nexus-wake-nostd`

`#![no_std]` + `extern crate alloc`. Dependencies:

- `nexus-store` — `default-features = false, features = ["subscription"]`
  (the `wake` traits; dep-free gate since #300, no_std since #301)
- `event-listener` — `default-features = false` (no_std + alloc mode;
  verified against the v5 source: `default = ["std"]`, `listen()` boxes the
  listener, so alloc is required — matching the card's no_std+alloc scope)

No tokio, no hash map, no mutex crate. Dual-licensed MIT OR Apache-2.0,
workspace lints, `version.workspace = true`.

### The type: `GlobalWake`

> **Superseded during implementation (`8e48f55`):** the generation counter is
> `AtomicU32`, not `AtomicU64` — `thumbv7em` (Cortex-M4, the primary bare-metal
> target) has no 64-bit atomics, so `AtomicU64` fails to compile there. The
> wrap analysis holds at 32 bits (2^32 wakes inside the arm→first-poll window
> is unreachable). Read `AtomicU64` below as `AtomicU32`.

```rust
/// Shared wake state: one eventcount for the whole store.
struct Inner {
    /// Bumped (wrapping) on every wake. Compared for inequality only —
    /// equality after exactly 2^64 intervening wakes is unreachable in
    /// practice (same wrapping discipline as StreamNotifiers' generations).
    generation: AtomicU64,
    /// Multi-waiter eventcount. `listen()` registers the listener into the
    /// notify list AT CREATION (verified in the event-listener v5 source),
    /// which is what makes arm-before-confirm-rescan expressible.
    event: event_listener::Event,
}

/// Cheap-clone handle (one `Arc` inc). The store adapter holds one and calls
/// `wake` after each durable commit; `Subscription` registers through it.
pub struct GlobalWake { inner: Arc<Inner> }

pub struct GlobalWakeReg { inner: Arc<Inner> }
```

- **`WakeSource::register(stream)`** ignores the stream key entirely —
  per-stream and `$all` registrations are identical. Infallible:
  `type Error = Infallible` (verify at implementation time that
  `core::error::Error` is implemented for `Infallible`; if not, a one-variant
  `thiserror` never-style enum instead).
- **`WakeSource::wake(_)`**: `generation.fetch_add(1, Release)` then
  `event.notify(usize::MAX)`. Every parked registration wakes; a registration
  whose stream didn't change performs one empty re-scan and re-parks —
  permitted spurious wake, by design.
- **`WakeRegistration::arm()`**: capture
  `seen = generation.load(Acquire)` **synchronously, before returning the
  future** (this is the contract's "seen version at the moment `arm` is
  called"). The returned `'static` future clones the `Arc` and, when polled:
  1. `let listener = inner.event.listen();` — registered into the notify
     list immediately, before any check;
  2. if `inner.generation.load(Acquire) != seen` → return (a wake landed
     between `arm` and this poll — caught by the generation);
  3. else `listener.await` → return (any later wake is caught by the
     already-registered listener; `notify` emits its own `SeqCst` fence).

  No wake after `arm` can be lost: it either bumps the generation before
  step 2's load (caught by 2) or notifies after step 1's registration
  (caught by 3). The `Release`-store/`Acquire`-load pair on `generation` is
  the self-contained structural ordering proof (rule 5) — no reliance on
  event-listener internals for the generation path.

### Compile gates (rule 9 — measure, don't assert)

Extend the flake's existing bare-metal gates (`thumbv7em-none-eabihf`,
`wasm32-unknown-unknown`, the #304 pattern) to build `nexus-wake-nostd`.
Building it transitively compiles
`nexus-store --no-default-features --features subscription` — i.e.
`subscription_cursor.rs`, `wake.rs`, `catchup.rs` — for a bare-metal target
for the first time, which is the card's explicit verification item. During
planning, check whether the #301 gate already enables `subscription`; if a
std-only combinator snuck into the loop, that gate failure is the finding.

Hakari: exclude `nexus-wake-nostd` from `workspace-hack` (the
`nexus-nostd-smoketest` precedent in `.config/hakari.toml`) so no std
workspace-hack edge poisons the bare-metal build. Confirm how #301 handled
this for `nexus-store` itself and follow the same mechanism.

### Testing (4 cross-cutting categories first)

All in `nexus-wake-nostd` (dev-deps: tokio, embassy-executor +
`critical-section` with its `std` feature, `nexus-inmemory`, `futures`).

1. **Sequence/protocol:** register → arm → wake resolves; resolve → re-arm →
   needs a *fresh* wake to resolve again; an armed future with no wake stays
   pending (timeout-bounded negative assertion).
2. **Lifecycle:** dropping the last `GlobalWake` handle while a future is
   parked is sound (the future's own `Arc` keeps `Inner` alive; assert it
   still resolves when woken through a surviving clone, and that dropping
   everything parked leaks nothing / doesn't hang the test). Dropping a
   registration mid-arm is a no-op.
3. **Defensive boundary:** `register(None)`, `register(Some(b""))`, and
   arbitrary keys behave identically (the global impl's contract);
   wake-before-arm does not satisfy a later arm on its own.
4. **Linearizability:** port `nexus-wake`'s
   `armed_wait_never_loses_a_concurrent_wake` (50-iteration barrier race,
   tokio multi-thread) and a concurrent arm/wake churn test. These exercise
   `Send` on the arm future across real threads.

**Acceptance test (the card's checkbox):** `embassy-executor` (single-threaded,
host `arch-std`) drives a real `Subscription` end-to-end. Test double: a
wrapper store `struct DeviceStore { inner: InMemoryStore, wake: GlobalWake }`
that delegates `RawEventStore` to `inner` and `WakeSource` to `wake`, with
`append` delegating then calling `self.wake.wake(stream)` after the commit
returns (the MUST-wake-after-durable-commit ordering). The subscriber task
sees the seeded backlog, `Step::CaughtUp`, then a post-subscribe live append;
results reported back to the test thread over a channel with a timeout. This
is delegation, not a reimplementation of store logic (rule 8 / nexus addendum:
reuse `InMemoryStore`).

### Documentation

- Crate docs state (the card's second checkbox): **optional,
  executor-dependent** — the primary device model is append-and-sync with
  server-side subscriptions; use this only for genuine on-device live-tail,
  driven by a no_std executor such as embassy. Document the global-eventcount
  trade (every commit wakes every parked subscription; cost = one empty
  re-scan per false wake) and the additive upgrade path (a routed sibling
  type) if a real device workload ever shows herd cost.
- CLAUDE.md architecture section: add the `nexus-wake-nostd` entry to the
  crate graph and the subscription-machinery notes.
- #280 (crates.io reservation): add `nexus-wake-nostd` to the names list.
- Close-out comment on #302 mapping each acceptance checkbox to the artifact.

## Out of scope

- Per-stream routed no_std wake (additive follow-up if ever measured
  necessary).
- Any no_std *store adapter* (the subscription loop still needs a
  `RawEventStore`; on-device that's a future embedded adapter's concern).
- Changes to `nexus-wake` (tokio impl) or the `WakeSource` trait itself.
