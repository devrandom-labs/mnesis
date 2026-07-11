//! Subscription tests for the fjall event store adapter.
//!
//! Covers 4 cross-cutting test categories:
//! 1. Sequence/Protocol — catch-up then live, subscribe from position
//! 2. Lifecycle — drop/resubscribe, close/reopen/subscribe
//! 3. Defensive Boundary — nonexistent stream, beyond-head subscribe
//! 4. Linearizability — concurrent append+subscribe, multiple subscribers

#![allow(clippy::unwrap_used, reason = "test code")]
#![allow(clippy::panic, reason = "test code")]
#![allow(clippy::expect_used, reason = "test code")]
#![allow(clippy::shadow_reuse, reason = "tests")]

use std::time::Duration;

use futures::StreamExt;
use nexus::Version;
use nexus_fjall::FjallStore;
use nexus_store::store::RawEventStore;
use nexus_store::{
    PendingEnvelope, StepStreamExt, Store, StreamKey, Subscription, pending_envelope,
};
use tokio::time::timeout;

fn sk(s: &str) -> StreamKey {
    StreamKey::from_slice(s.as_bytes())
}

fn make_envelope(version: u64, event_type: &'static str, payload: &[u8]) -> PendingEnvelope {
    pending_envelope(Version::new(version).expect("test version must be > 0"))
        .event_type(event_type)
        .payload(payload.to_vec())
        .build()
        .expect("valid envelope")
}

fn temp_store() -> (FjallStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = FjallStore::builder(dir.path().join("db")).open().unwrap();
    (store, dir)
}

/// Helper: append a single event to a stream, with expected version.
async fn append_one(
    store: &Store<FjallStore>,
    id: &StreamKey,
    version: u64,
    expected: Option<Version>,
    event_type: &'static str,
) {
    let envelope = make_envelope(version, event_type, format!("payload-{version}").as_bytes());
    store.append(id, expected, &[envelope]).await.unwrap();
}

/// Timeout duration for operations that should complete quickly.
const TIMEOUT: Duration = Duration::from_secs(2);

// ═══════════════════════════════════════════════════════════════════════════
// 4. Linearizability/Isolation Tests
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn concurrent_append_and_subscribe() {
    let (store, _dir) = temp_store();
    let store = Store::new(store);
    let id = sk("concurrent-stream");
    let event_count: u64 = 50;

    let stream = Subscription::new(&store)
        .subscribe(&id, None)
        .unwrap()
        .events();
    futures::pin_mut!(stream);

    // Spawn a task that appends events sequentially.
    let writer_store = store.clone();
    let writer_id = id.clone();
    let writer = tokio::spawn(async move {
        for i in 1..=event_count {
            let expected = if i == 1 {
                None
            } else {
                Version::new(i.checked_sub(1).unwrap())
            };
            append_one(&writer_store, &writer_id, i, expected, "ConcurrentEvent").await;
            // Yield to allow reader to interleave.
            tokio::task::yield_now().await;
        }
    });

    // Read all events from the subscriber.
    for expected_version in 1..=event_count {
        let env = timeout(TIMEOUT, stream.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(
            env.version(),
            Version::new(expected_version).unwrap(),
            "expected version {expected_version}, got {}",
            env.version()
        );
        assert_eq!(env.event_type(), "ConcurrentEvent");
    }

    writer.await.unwrap();
}

// ═══════════════════════════════════════════════════════════════════════════
// 5. Static-ness compile-time guarantee
// ═══════════════════════════════════════════════════════════════════════════

/// The cursor returned by `subscribe` must be `'static` — the whole point
/// of the Arc-based subscription shape. If this assertion compiles, the
/// cursor outlives any caller scope and can be spawned across tasks.
#[tokio::test]
async fn subscription_cursor_is_static() {
    fn assert_static<T: 'static>(_: &T) {}
    let (store, _dir) = temp_store();
    let store = Store::new(store);
    let id = sk("s-1");
    let sub = Subscription::new(&store)
        .subscribe(&id, None)
        .unwrap()
        .events();
    assert_static(&sub);
}
