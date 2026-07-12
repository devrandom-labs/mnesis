//! Per-stream wake registry with a deterministic (drop-guard) lifecycle.
//!
//! # What this is
//!
//! The *ephemeral, in-process* half of the subscription wake-up path. It holds
//! one `watch`-backed wake-generation counter per stream that currently has at
//! least one live registration, plus one store-wide `$all` generation. After
//! an adapter durably commits event(s) to stream `X`, it calls
//! [`StreamNotifiers::wake`] with `X`; that bumps `X`'s generation — resolving
//! every armed wait on `X` — and the `$all` generation (a per-stream commit is
//! also an `$all` event, so every armed `$all` wait resolves too). A stream
//! with no current registrations costs one map miss and nothing else.
//!
//! This replaces a single store-wide notifier (which woke *every* subscriber
//! on *every* commit — an O(subscribers) thundering herd) with O(1) wake-by-
//! stream routing.
//!
//! # What this is NOT
//!
//! This is wake-*routing* only. It does not track subscriber identity, cursor
//! position, or anything durable. A registration is a handle to a parked task
//! in *this* process — it cannot be persisted and has no meaning across a
//! restart. Durable, resumable subscriptions (e.g. an actor that passivates
//! and later resumes from its last position) are a separate, higher-layer
//! concern that persists a cursor; this registry is the in-memory wake handle
//! that such a subscription creates while it is active.
//!
//! # Lifecycle — drop-guard
//!
//! An entry exists *iff* at least one live per-stream registration (a
//! [`WakeReg`] holding an internal drop-guard) for that stream is alive.
//! [`register`](mnesis_store::wake::WakeSource::register) creates-or-reuses the
//! entry and increments a subscriber count; dropping the registration
//! decrements it and removes the entry when it reaches zero. The map therefore
//! holds an entry per *currently-active* stream, not per stream ever seen —
//! bounded memory, truthful [`active_streams`](StreamNotifiers::active_streams),
//! and no sweep task (cleanup is synchronous in `Drop`, so no async runtime is
//! required).
//!
//! Drop-guard is chosen over a `Weak` + lazy-cleanup scheme because the
//! intended workload (per-entity streams under an actor model that passivates
//! and reactivates constantly) wants the entry's lifetime to equal "a task is
//! parked here", reaped the instant the last subscriber leaves. Lazy cleanup
//! would accumulate one dead entry per passivated stream until a sweep ran, and
//! a sweep needs a timer/runtime this layer deliberately avoids.
//!
//! # Ordering contract
//!
//! Callers MUST [`arm`](mnesis_store::wake::WakeRegistration::arm) the wait
//! *before* performing the read that could miss the event, and producers MUST
//! call [`wake`](StreamNotifiers::wake) *after* the commit is durable. Together
//! these close the lost-wakeup race: `arm` pins the seen generation to the
//! exact instant it is called, so a wake landing at any later point — even
//! between `arm` and the `await` — bumps past the pinned generation and
//! resolves the wait. A subscriber therefore either observes the wake, or
//! performs its read after the data is already visible.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Weak};

use foldhash::fast::RandomState;
use parking_lot::Mutex;
use thiserror::Error;
use tokio::sync::watch;

use mnesis_store::wake::{WakeRegistration, WakeSource};

/// Errors produced by [`StreamNotifiers`].
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum NotifyError {
    /// The live-subscriber count for a single stream would exceed `usize::MAX`.
    ///
    /// Unreachable in practice — the count is bounded by the number of live
    /// registrations, which is bounded by available memory. Modelled
    /// as a returned error rather than a panic to honour the project's
    /// arithmetic-safety rule (no bare arithmetic, no silent saturation).
    #[error("live-subscriber count overflow for a single stream")]
    SubscriberOverflow,
}

/// In-memory, per-stream wake registry. Cheap to share via `Arc`.
#[derive(Debug)]
pub struct StreamNotifiers {
    // A single `Mutex` over a `foldhash`-hashed map. Two facts drove this over a
    // sharded/lock-free map (`dashmap`, `papaya`) on the IoT/mobile target:
    //
    //  - Contention is not the bottleneck at this scale. Each critical section
    //    is a lookup plus a `watch` generation bump (wake) or receiver clone
    //    (register); lock occupancy ≈ ops × section-time, so
    //    even ~10k wakes/sec on a low-core device sits near ~1% occupancy. The
    //    hasher, by contrast, runs on *every* op, so swapping SipHash → foldhash
    //    is the unconditional win. (hashbrown made foldhash its default in 0.15:
    //    rust-lang/hashbrown#563.)
    //  - `papaya`/`scc`/`flurry` use epoch/RCU reclamation, which defers freeing
    //    a removed node past its logical removal — memory-scarce-hostile and in
    //    tension with this module's deterministic reap-at-zero. A `Mutex` frees
    //    the entry the instant `release` removes it.
    //
    // If profiling on real high-core hardware ever shows this global lock
    // contended, `dashmap` is a drop-in with the same API and the same foldhash
    // hasher; revisit then, not before.
    map: Mutex<HashMap<Box<[u8]>, Entry, RandomState>>,
    /// Store-wide `$all` generation counter, bumped on every
    /// [`wake`](Self::wake) (a per-stream commit is also an `$all` event).
    /// Always present (no drop-guard / refcount): every commit bumps it, and
    /// every `$all` subscriber genuinely wants every event, so there is no
    /// thundering herd to avoid here — unlike the per-stream `map`.
    all_gen_tx: watch::Sender<u64>,
    /// A `Weak` back-reference to the `Arc<Self>` this lives in, set at
    /// construction via [`Arc::new_cyclic`]. The [`WakeSource::register`] trait
    /// method takes `&self` (its receiver is fixed by the trait), yet must
    /// build a [`SubscriptionGuard`] carrying a back-reference to the registry
    /// for the drop-guard reap channel. Cloning this `Weak` provides that
    /// back-reference without changing the trait signature; the guard's `Drop`
    /// upgrades it, which succeeds while any strong reference exists (and is a
    /// no-op reap when the registry is already gone).
    weak_self: Weak<Self>,
}

#[derive(Debug)]
struct Entry {
    /// Per-stream generation counter, bumped on every [`wake`] for this
    /// stream: a monotone (wrapping) generation an armed registration watches,
    /// and a cursor can read and compare to detect a missed wake without
    /// parking.
    gen_tx: watch::Sender<u64>,
    /// Number of live [`SubscriptionGuard`]s for this stream.
    subscribers: usize,
}

impl StreamNotifiers {
    /// Create an empty registry behind an `Arc`.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Self {
            map: Mutex::default(),
            all_gen_tx: watch::Sender::new(0),
            weak_self: Weak::clone(weak_self),
        })
    }

    /// Register interest in `stream`, returning the drop-guard plus a `watch`
    /// receiver on the stream's per-stream generation. The receiver resolves
    /// `changed()` on every subsequent [`wake`](Self::wake) of this stream.
    ///
    /// A single map-lock critical section does insert-or-get, the *one*
    /// subscriber increment, the receiver clone, and the guard build — so the
    /// subscriber count is incremented exactly once, never double-counted; the
    /// guard's `Drop` will decrement exactly once via [`release`](Self::release).
    ///
    /// Takes `&self` (not `&Arc<Self>`) so it is reachable from the
    /// [`WakeSource::register`] trait method, whose receiver is fixed at
    /// `&self`; the guard's back-reference is cloned from
    /// [`weak_self`](Self::weak_self).
    ///
    /// # Errors
    ///
    /// [`NotifyError::SubscriberOverflow`] if the live-subscriber count for the
    /// stream would overflow `usize` (unreachable in practice).
    fn register_entry(
        &self,
        stream: &[u8],
    ) -> Result<(SubscriptionGuard, watch::Receiver<u64>), NotifyError> {
        let key: Box<[u8]> = Box::from(stream);
        let mut map = self.map.lock();
        let entry = map.entry(key.clone()).or_insert_with(|| Entry {
            gen_tx: watch::Sender::new(0),
            subscribers: 0,
        });
        entry.subscribers = entry
            .subscribers
            .checked_add(1)
            .ok_or(NotifyError::SubscriberOverflow)?;
        let rx = entry.gen_tx.subscribe();
        drop(map);
        Ok((
            SubscriptionGuard {
                registry: Weak::clone(&self.weak_self),
                key,
            },
            rx,
        ))
    }

    /// Wake every registration armed on `stream`, and every `$all`
    /// registration (a per-stream commit is also an `$all` event). The
    /// per-stream bump is a no-op when the stream has no live registrations.
    ///
    /// MUST be called *after* the corresponding event(s) are durably committed,
    /// so that a woken subscriber re-reads already-visible data.
    pub fn wake(&self, stream: &[u8]) {
        {
            let map = self.map.lock();
            if let Some(entry) = map.get(stream) {
                entry.gen_tx.send_modify(|g| *g = g.wrapping_add(1));
            }
        }
        self.all_gen_tx.send_modify(|g| *g = g.wrapping_add(1));
    }

    /// Current per-stream wake generation for `stream`, or `0` when the stream
    /// has no live entry. Bumped by every [`wake`](Self::wake); a cursor reads
    /// it to detect a wake that arrived between two reads. Wrapping, so compare
    /// for inequality, not ordering.
    ///
    /// The `0`-for-absent-stream return is unambiguous in practice: a
    /// subscription cursor reads this only while holding its own live
    /// [`WakeReg`], so its entry is guaranteed present and a genuine
    /// generation of `0` (the entry's initial value) cannot be confused with
    /// the absent-stream `0`.
    #[must_use]
    pub fn generation(&self, stream: &[u8]) -> u64 {
        self.map
            .lock()
            .get(stream)
            .map_or(0, |e| *e.gen_tx.borrow())
    }

    /// Current store-wide `$all` wake generation. Bumped by every
    /// [`wake`](Self::wake). Wrapping, so compare for inequality, not
    /// ordering.
    #[must_use]
    pub fn all_generation(&self) -> u64 {
        *self.all_gen_tx.borrow()
    }

    /// Number of streams with at least one live subscriber. Diagnostics only.
    #[must_use]
    pub fn active_streams(&self) -> usize {
        self.map.lock().len()
    }

    /// Drop-guard back-channel: decrement a stream's subscriber count and remove
    /// the entry when it reaches zero. Atomic under the map lock, so it cannot
    /// race a concurrent `subscribe` for the same key into a lost wakeup.
    fn release(&self, key: &[u8]) {
        let mut map = self.map.lock();
        let Some(entry) = map.get_mut(key) else {
            // No entry → a guard outlived its entry. Impossible by construction;
            // nothing to do.
            return;
        };
        match entry.subscribers.checked_sub(1) {
            // Last subscriber (or an impossible underflow) → reap the entry.
            Some(0) | None => {
                map.remove(key);
            }
            Some(remaining) => entry.subscribers = remaining,
        }
    }
}

/// RAII handle keeping a stream's wake entry registered in a
/// [`StreamNotifiers`].
///
/// While alive, the stream's entry stays in the map and is wakeable by the
/// producer. On drop, the stream's subscriber count is decremented and the entry
/// removed once it reaches zero. Carried inside [`WakeReg`] so it drops exactly
/// when the registration is dropped (e.g. on passivation).
#[derive(Debug)]
struct SubscriptionGuard {
    /// A `Weak` (not `Arc`) back-reference to the owning registry. `Weak`
    /// because the guard is built from `&self` (so the [`WakeSource::register`]
    /// trait method, whose receiver is `&self`, can construct it), and because
    /// a guard must not keep the whole registry alive on its own. On drop it
    /// upgrades to run [`release`](StreamNotifiers::release); if the registry
    /// is already gone the reap is a no-op (there is nothing left to
    /// decrement).
    registry: Weak<StreamNotifiers>,
    key: Box<[u8]>,
}

impl Drop for SubscriptionGuard {
    fn drop(&mut self) {
        // Upgrade to run the reap. `None` means the registry was already
        // dropped, in which case its map (and this entry) is already gone —
        // nothing to decrement.
        if let Some(registry) = self.registry.upgrade() {
            registry.release(&self.key);
        }
    }
}

/// In-process registration: a `watch` receiver on the target's generation,
/// plus (for a per-stream target) the drop-guard that reaps the entry.
///
/// For an `$all` target there is no entry to reap (the `$all` generation is
/// permanently alive), so `_guard` is `None`.
#[derive(Debug)]
pub struct WakeReg {
    rx: watch::Receiver<u64>,
    _guard: Option<SubscriptionGuard>,
}

impl WakeRegistration for WakeReg {
    fn arm(&self) -> impl Future<Output = ()> + Send + 'static {
        let mut rx = self.rx.clone();
        // The clone alone is insufficient: a freshly cloned `watch::Receiver`
        // inherits the sender's version as of the clone, which may predate this
        // `arm` call. `mark_unchanged()` narrows the captured-version point to
        // the exact instant `arm` is called, making the contract's "seen-version
        // at arm() time" precise and closing the clone→mark window — so only a
        // *future* bump (a `wake` after this point) resolves `changed()`, and a
        // wake landing between `arm` and the await is observed, never lost.
        rx.mark_unchanged();
        async move {
            // `Err` only if every sender has been dropped; treat that as a
            // (final) wake so the parked task makes progress rather than
            // hanging.
            let _ = rx.changed().await;
        }
    }
}

impl WakeSource for StreamNotifiers {
    type Registration = WakeReg;
    type Error = NotifyError;

    fn register(&self, stream: Option<&[u8]>) -> Result<WakeReg, NotifyError> {
        match stream {
            None => Ok(WakeReg {
                rx: self.all_gen_tx.subscribe(),
                _guard: None,
            }),
            Some(s) => {
                let (guard, rx) = self.register_entry(s)?;
                Ok(WakeReg {
                    rx,
                    _guard: Some(guard),
                })
            }
        }
    }

    fn wake(&self, stream: &[u8]) {
        // A per-stream commit is also an `$all` event. The inherent
        // [`wake`](StreamNotifiers::wake) already does both bumps: the
        // per-stream generation AND the `$all` generation that a
        // [`WakeReg`] armed on `$all` watches. Delegate to it so the trait and
        // inherent paths are behaviourally identical and there is a single
        // source of truth for the wake effect. The inherent method shadows this
        // trait method at unqualified call sites; the explicit `<Self>` path
        // selects the inherent one without recursing.
        <Self>::wake(self, stream);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test code")]
#[allow(clippy::expect_used, reason = "test code")]
#[allow(
    clippy::shadow_reuse,
    reason = "Arc::clone rebinds are idiomatic for task captures"
)]
mod tests {
    use super::{Arc, StreamNotifiers};
    use mnesis_store::wake::{WakeRegistration, WakeSource};
    use std::time::Duration;
    use tokio::sync::Barrier;
    use tokio::time::timeout;

    /// Generous upper bound on "a wake must arrive". Far longer than any real
    /// scheduling delay, so a timeout here means a genuinely lost wakeup.
    const MUST_WAKE: Duration = Duration::from_secs(5);
    /// Short bound for asserting the negative: "this wait must NOT resolve".
    const MUST_NOT_WAKE: Duration = Duration::from_millis(150);

    // ───────────────────────── Category 1: sequence / protocol ─────────────────────────

    /// register → arm → wake resolves the armed wait, in that order. No spawn
    /// or barrier needed: `arm` pins the seen generation synchronously, so a
    /// wake after `arm` cannot be lost even before the wait is first polled.
    #[tokio::test]
    async fn register_then_wake_resolves_armed_wait() {
        let reg = StreamNotifiers::new();
        let registration = reg.register(Some(b"s1")).unwrap();
        assert_eq!(reg.active_streams(), 1);

        let wait = registration.arm(); // seen generation pinned NOW
        reg.wake(b"s1");
        timeout(MUST_WAKE, wait)
            .await
            .expect("armed wait must resolve after wake()");
    }

    /// A full register → wake → drop sequence leaves the registry empty.
    #[tokio::test]
    async fn register_wake_drop_sequence_leaves_no_entry() {
        let reg = StreamNotifiers::new();
        let registration = reg.register(Some(b"s1")).unwrap();
        reg.wake(b"s1"); // live registration, none armed: a harmless no-op
        assert_eq!(reg.active_streams(), 1);
        drop(registration);
        assert_eq!(reg.active_streams(), 0);
    }

    // ───────────────────────── Category 2: lifecycle ─────────────────────────

    /// Two registrations on one stream share ONE entry (a single wake resolves
    /// both armed waits); the entry is reaped only when the last registration
    /// drops.
    #[tokio::test]
    async fn refcount_reap_at_zero() {
        let reg = StreamNotifiers::new();
        let r1 = reg.register(Some(b"k")).unwrap();
        let r2 = reg.register(Some(b"k")).unwrap();
        // One stream entry, not two.
        assert_eq!(reg.active_streams(), 1);
        // Both registrations watch the SAME entry, so a single wake resolves
        // every armed wait of the stream.
        let w1 = r1.arm();
        let w2 = r2.arm();
        reg.wake(b"k");
        timeout(MUST_WAKE, w1)
            .await
            .expect("first armed wait must resolve on the shared wake");
        timeout(MUST_WAKE, w2)
            .await
            .expect("second armed wait must resolve on the shared wake");

        drop(r1);
        assert_eq!(reg.active_streams(), 1); // one registration remains
        drop(r2);
        assert_eq!(reg.active_streams(), 0); // reaped at zero
    }

    /// After an entry is reaped, re-registering builds a FRESH entry (its
    /// generation restarts at 0) — the old one was not silently kept alive.
    #[tokio::test]
    async fn reregister_after_reap_is_fresh() {
        let reg = StreamNotifiers::new();
        let first = reg.register(Some(b"k")).unwrap();
        reg.wake(b"k");
        assert_eq!(reg.generation(b"k"), 1);
        drop(first);
        assert_eq!(reg.active_streams(), 0);

        let _second = reg.register(Some(b"k")).unwrap();
        assert_eq!(reg.active_streams(), 1);
        assert_eq!(
            reg.generation(b"k"),
            0,
            "reaped entry must not be reused: re-register must build a fresh entry"
        );
    }

    /// Distinct streams get independent entries.
    #[tokio::test]
    async fn distinct_streams_are_independent() {
        let reg = StreamNotifiers::new();
        let a = reg.register(Some(b"a")).unwrap();
        let b = reg.register(Some(b"b")).unwrap();
        assert_eq!(reg.active_streams(), 2);
        drop(a);
        assert_eq!(reg.active_streams(), 1);
        drop(b);
        assert_eq!(reg.active_streams(), 0);
    }

    // ───────────────────────── Category 3: defensive boundary ─────────────────────────

    /// wake on a stream with no subscribers is a no-op and never panics.
    #[tokio::test]
    async fn wake_with_no_subscribers_is_noop() {
        let reg = StreamNotifiers::new();
        reg.wake(b"never-subscribed");
        assert_eq!(reg.active_streams(), 0);
    }

    /// The empty byte slice is a valid stream key end-to-end (register →
    /// wake → reap).
    #[tokio::test]
    async fn empty_key_is_valid() {
        let reg = StreamNotifiers::new();
        let registration = reg.register(Some(b"")).unwrap();
        assert_eq!(reg.active_streams(), 1);
        let wait = registration.arm();
        reg.wake(b"");
        timeout(MUST_WAKE, wait)
            .await
            .expect("empty-key armed wait must resolve");
        drop(registration);
        assert_eq!(reg.active_streams(), 0);
    }

    /// A wake for a DIFFERENT stream must not resolve this stream's armed
    /// wait; a wake for the correct stream then does.
    #[tokio::test]
    async fn wake_is_isolated_per_stream() {
        let reg = StreamNotifiers::new();
        let registration = reg.register(Some(b"A")).unwrap();
        let wait = registration.arm();
        tokio::pin!(wait);

        // Wrong stream: must NOT resolve the wait on "A".
        reg.wake(b"B");
        assert!(
            timeout(MUST_NOT_WAKE, &mut wait).await.is_err(),
            "a wake for stream B must not resolve a wait on stream A"
        );

        // Right stream: now it resolves.
        reg.wake(b"A");
        timeout(MUST_WAKE, wait)
            .await
            .expect("wait must resolve on its own stream");
    }

    // ───────────────────────── Category 4: linearizability / isolation ─────────────────────────

    /// Concurrent register / drop / wake churn on ONE key must never leave an
    /// orphaned entry: once every registration is dropped, the registry is
    /// empty. This fails if reap-at-zero races a concurrent register (lost
    /// decrement or a dangling entry).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_churn_leaves_no_orphan() {
        let reg = StreamNotifiers::new();
        let key: &[u8] = b"race";
        let workers = 16usize;
        let iterations = 200usize;
        let barrier = Arc::new(Barrier::new(workers + 1));

        let mut handles = Vec::with_capacity(workers + 1);
        for _ in 0..workers {
            let reg = Arc::clone(&reg);
            let barrier = Arc::clone(&barrier);
            handles.push(tokio::spawn(async move {
                barrier.wait().await; // all workers start together
                for _ in 0..iterations {
                    let registration = reg.register(Some(key)).unwrap();
                    reg.wake(key);
                    drop(registration);
                }
            }));
        }
        // A concurrent waker hammering the same key throughout the churn.
        {
            let reg = Arc::clone(&reg);
            let barrier = Arc::clone(&barrier);
            handles.push(tokio::spawn(async move {
                barrier.wait().await;
                for _ in 0..(workers * iterations) {
                    reg.wake(key);
                }
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        assert_eq!(
            reg.active_streams(),
            0,
            "concurrent register/drop/wake churn orphaned a registry entry"
        );
    }

    #[tokio::test]
    async fn wake_increments_stream_generation() {
        let reg = StreamNotifiers::new();
        let _registration = reg.register(Some(b"s1")).unwrap();
        let before = reg.generation(b"s1");
        reg.wake(b"s1");
        let after = reg.generation(b"s1");
        assert_eq!(
            after,
            before + 1,
            "wake must bump the stream generation by 1"
        );
    }

    /// A per-stream wake is also an `$all` event: it bumps the store-wide
    /// `$all` generation even for a stream with no live entry.
    #[tokio::test]
    async fn wake_increments_all_generation() {
        let reg = StreamNotifiers::new();
        let before = reg.all_generation();
        reg.wake(b"any-stream");
        assert_eq!(reg.all_generation(), before + 1);
    }

    /// Waking one stream bumps only its generation, not another's.
    #[tokio::test]
    async fn generations_are_independent_per_stream() {
        let reg = StreamNotifiers::new();
        let _a = reg.register(Some(b"a")).unwrap();
        let _b = reg.register(Some(b"b")).unwrap();
        let a_before = reg.generation(b"a");
        let b_before = reg.generation(b"b");
        reg.wake(b"a");
        assert_eq!(
            reg.generation(b"a"),
            a_before + 1,
            "wake(a) must bump a's generation"
        );
        assert_eq!(
            reg.generation(b"b"),
            b_before,
            "wake(a) must not touch b's generation"
        );
    }

    /// A never-subscribed stream reports generation 0.
    #[tokio::test]
    async fn generation_of_unknown_stream_is_zero() {
        let reg = StreamNotifiers::new();
        assert_eq!(reg.generation(b"never"), 0);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test code")]
mod wake_source_contract_tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::Barrier;
    use tokio::time::timeout;

    const MUST_WAKE: Duration = Duration::from_secs(5);

    /// Ported lost-wakeup test: a registration armed before a concurrent wake
    /// must never miss it. Repeated to shake out scheduling races.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn armed_wait_never_loses_a_concurrent_wake() {
        for _ in 0..50 {
            let reg = StreamNotifiers::new();
            let registration = reg.register(Some(b"k")).unwrap();
            let wait = registration.arm(); // armed BEFORE the race
            let start = Arc::new(Barrier::new(2));

            let start_prod = Arc::clone(&start);
            let reg_prod = Arc::clone(&reg);
            let prod = tokio::spawn(async move {
                start_prod.wait().await;
                reg_prod.wake(b"k");
            });

            start.wait().await;
            timeout(MUST_WAKE, wait)
                .await
                .expect("an armed wait must not lose a concurrent wake");
            prod.await.unwrap();
        }
    }

    /// $all registration wakes on any stream's wake.
    #[tokio::test]
    async fn all_registration_wakes_on_any_stream() {
        let reg = StreamNotifiers::new();
        let registration = reg.register(None).unwrap();
        let wait = registration.arm();
        reg.wake(b"any-stream");
        timeout(MUST_WAKE, wait)
            .await
            .expect("$all must wake on any stream wake");
    }

    /// Drives the wake purely through the `WakeSource` trait surface (a generic
    /// bound, the way the subscription loop uses it): register, arm, then call
    /// the *trait* `wake`. Proves the trait method is not the recursive shadow
    /// of the inherent one (a recursion would hang and time out here) and that
    /// a per-stream wake reaches both a per-stream and an `$all` registration.
    #[tokio::test]
    async fn trait_wake_routes_to_stream_and_all() {
        async fn arm_and_wake<W: WakeSource>(src: &W) {
            let per_stream = src.register(Some(b"k")).unwrap();
            let all = src.register(None).unwrap();
            let wait_stream = per_stream.arm();
            let wait_all = all.arm();
            // Trait-dispatched wake (not the inherent method).
            WakeSource::wake(src, b"k");
            timeout(MUST_WAKE, wait_stream)
                .await
                .expect("trait wake must rouse the per-stream registration");
            timeout(MUST_WAKE, wait_all)
                .await
                .expect("trait wake must rouse the $all registration");
        }

        let reg = StreamNotifiers::new();
        arm_and_wake(reg.as_ref()).await;
    }

    /// Dropping a per-stream `WakeReg` reaps the entry through the guard chain.
    #[tokio::test]
    async fn dropping_registration_reaps_entry() {
        let reg = StreamNotifiers::new();
        let registration = reg.register(Some(b"s")).unwrap();
        assert_eq!(
            reg.active_streams(),
            1,
            "register(Some) must create one entry"
        );
        drop(registration);
        assert_eq!(
            reg.active_streams(),
            0,
            "dropping the registration must reap the entry"
        );
    }
}
