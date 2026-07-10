//! Global-eventcount [`WakeSource`] for `no_std` on-device live-tail
//! subscriptions (#302).
//!
//! # What this is
//!
//! The `no_std` counterpart to `nexus-wake`'s tokio-backed `StreamNotifiers`:
//! a [`WakeSource`] built on `core` atomics plus [`event_listener::Event`]
//! (an eventcount — `no_std` + alloc), so the generic catch-up-then-live-tail
//! loop in `nexus-store` can park on a device with no tokio and no OS.
//!
//! **Optional and executor-dependent.** The primary device model is
//! append-and-sync: events are produced locally and synced to a server,
//! where subscriptions run under tokio via `nexus-wake`. Reach for this
//! crate only when a device genuinely runs an *on-device* live-tail —
//! driving the subscription then also needs a `no_std` executor (e.g.
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
//! `no_std` mutex, guards) would cost more than the false wakes it saves. If
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

use alloc::sync::Arc;
use core::convert::Infallible;
use core::fmt;
use core::future::Future;
use core::sync::atomic::{AtomicU32, Ordering};

use event_listener::Event;
use nexus_store::wake::{WakeRegistration, WakeSource};

/// Shared eventcount state — one per store, shared by every registration.
struct Inner {
    /// Wake generation. Bumped on every [`WakeSource::wake`]; `fetch_add`
    /// wraps on overflow BY DESIGN — the value is compared for inequality
    /// only, and a false equality needs exactly 2^32 intervening wakes
    /// between an `arm` and its first poll, a window microseconds long in
    /// practice, so still unreachable (the same wrapping discipline as
    /// `StreamNotifiers`' generations). `AtomicU64` is deliberately NOT
    /// used: thumbv7em (Cortex-M4, this crate's primary target class) has
    /// no 64-bit atomics, and it would fail to compile there.
    generation: AtomicU32,
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
/// when to prefer `nexus-wake`'s routed `StreamNotifiers` instead.
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
                generation: AtomicU32::new(0),
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
