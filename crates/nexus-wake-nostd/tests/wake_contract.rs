//! Contract tests for `GlobalWake` — the 4 cross-cutting categories.
//!
//! Category 1 (sequence/protocol) lives here; categories 2–4 are added in
//! Task 3 (same file).

#![allow(clippy::unwrap_used, reason = "test code")]
#![allow(clippy::expect_used, reason = "test code")]

use std::time::Duration;

use nexus_store::wake::{WakeRegistration, WakeSource};
use nexus_wake_nostd::GlobalWake;
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
