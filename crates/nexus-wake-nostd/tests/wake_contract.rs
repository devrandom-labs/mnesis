//! Contract tests for `GlobalWake` — the 4 cross-cutting categories.
//!
//! All 4 categories (sequence/protocol, lifecycle, defensive boundary,
//! linearizability/isolation) live here.

#![allow(clippy::unwrap_used, reason = "test code")]
#![allow(clippy::expect_used, reason = "test code")]

use std::sync::Arc;
use std::time::Duration;

use nexus_store::wake::{WakeRegistration, WakeSource};
use nexus_wake_nostd::GlobalWake;
use tokio::sync::Barrier;
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

/// Ported from nexus-wake: a registration armed before a concurrent wake
/// must never miss it. Repeated to shake out scheduling races.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn armed_wait_never_loses_a_concurrent_wake() {
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
    let wake = GlobalWake::new();
    let workers = 16usize;
    let iterations = 100usize;
    let barrier = Arc::new(Barrier::new(workers));

    let mut handles = Vec::with_capacity(workers);
    for _ in 0..workers {
        let src = wake.clone();
        let worker_barrier = Arc::clone(&barrier);
        handles.push(tokio::spawn(async move {
            worker_barrier.wait().await; // all workers start together
            for _ in 0..iterations {
                let reg = src.register(Some(b"churn")).unwrap();
                let wait = reg.arm();
                src.wake(b"churn"); // own wake satisfies own arm (global)
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
