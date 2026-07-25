//! `RawEventStore::append` returns the `$all` position its events landed at
//! (#330) — the value a caller awaits to get read-your-writes over an `$all`
//! projection.
//!
//! The contract under test: the returned position is the position of the run's
//! **last** event, and it is the same position `$all` reports for that event.
//! Everything else (monotonicity, agreement with `$all` delivery order under
//! concurrency) follows from that and is asserted here rather than assumed.

#![allow(clippy::unwrap_used, reason = "tests")]
#![allow(clippy::expect_used, reason = "tests")]
#![allow(clippy::panic, reason = "tests")]

use std::sync::Arc;

use bytes::Bytes;
use futures::TryStreamExt;
use mnesis::Version;
use mnesis_inmemory::InMemoryStore;
use mnesis_store::store::RawEventStore;
use mnesis_store::{PendingBatch, PendingEnvelope, PersistedEnvelope, Store, StreamKey};
use tokio::sync::Barrier;

fn sk(id: &str) -> StreamKey {
    StreamKey::from_slice(id.as_bytes())
}

fn env(version: u64) -> PendingEnvelope {
    mnesis_store::pending_envelope(Version::new(version).unwrap())
        .event_type("Recorded")
        .payload(Bytes::from_static(b"payload"))
        .build()
        .expect("valid envelope")
}

/// Every `$all` row, in delivery order.
async fn drain_all<S: RawEventStore>(
    store: &Store<S>,
) -> Vec<(S::AllPosition, StreamKey, PersistedEnvelope)> {
    store
        .read_all(None)
        .await
        .expect("read_all")
        .try_collect()
        .await
        .expect("drain $all")
}

// ═══ 1. Sequence/protocol ═══

#[tokio::test]
async fn append_returns_the_position_all_reports_for_that_event() {
    let store = Store::new(InMemoryStore::new());
    let only = env(1);

    let returned = store
        .append(&sk("s1"), None, PendingBatch::of(&only))
        .await
        .expect("append");

    let rows = drain_all(&store).await;
    assert_eq!(rows.len(), 1, "one append of one event is one $all row");
    assert_eq!(
        rows[0].0, returned,
        "the returned position must be the position $all reports for that event"
    );
}

#[tokio::test]
async fn multi_event_append_returns_the_last_events_position() {
    let store = Store::new(InMemoryStore::new());
    let envs = [env(1), env(2), env(3)];

    let returned = store
        .append(&sk("s1"), None, PendingBatch::new(&envs).unwrap())
        .await
        .expect("append");

    let rows = drain_all(&store).await;
    assert_eq!(rows.len(), 3, "three events are three $all rows");
    assert_eq!(
        rows[2].0, returned,
        "the returned position is the LAST event's, so a consumer that has \
         reached it has necessarily been delivered the whole run"
    );
    assert_ne!(
        rows[0].0, returned,
        "the first event's position must not be what is returned"
    );
}

#[tokio::test]
async fn successive_appends_return_strictly_increasing_positions() {
    let store = Store::new(InMemoryStore::new());
    let (e1, e2, e3) = (env(1), env(2), env(1));

    let first = store
        .append(&sk("s1"), None, PendingBatch::of(&e1))
        .await
        .expect("append 1");
    let second = store
        .append(&sk("s1"), Version::new(1), PendingBatch::of(&e2))
        .await
        .expect("append 2");
    let other_stream = store
        .append(&sk("s2"), None, PendingBatch::of(&e3))
        .await
        .expect("append 3");

    assert!(first < second, "positions advance within one stream");
    assert!(
        second < other_stream,
        "positions advance across streams — `$all` order is store-wide"
    );
}

// ═══ 3. Defensive boundary ═══

#[tokio::test]
async fn a_rejected_append_returns_no_position_and_writes_nothing() {
    let store = Store::new(InMemoryStore::new());
    let first = env(1);
    store
        .append(&sk("s1"), None, PendingBatch::of(&first))
        .await
        .expect("append");

    // Stale expectation: the stream is at 1, the caller thinks it is fresh.
    let duplicate = env(1);
    let err = store
        .append(&sk("s1"), None, PendingBatch::of(&duplicate))
        .await
        .expect_err("stale expected_version must be rejected");
    assert!(
        matches!(err, mnesis_store::AppendError::Conflict { .. }),
        "a stale expectation is a Conflict, got {err:?}"
    );

    let rows = drain_all(&store).await;
    assert_eq!(rows.len(), 1, "the rejected append must not have landed");
}

// ═══ 4. Linearizability/isolation ═══

#[tokio::test]
async fn concurrent_appends_return_positions_ordered_as_all_delivers_them() {
    let store = Arc::new(Store::new(InMemoryStore::new()));
    let barrier = Arc::new(Barrier::new(2));

    let mut handles = Vec::new();
    for name in ["s1", "s2"] {
        let task_store = Arc::clone(&store);
        let task_barrier = Arc::clone(&barrier);
        handles.push(tokio::spawn(async move {
            let only = env(1);
            task_barrier.wait().await;
            let pos = task_store
                .append(&sk(name), None, PendingBatch::of(&only))
                .await
                .expect("concurrent append");
            (sk(name), pos)
        }));
    }

    let mut claimed = Vec::new();
    for h in handles {
        claimed.push(h.await.expect("task panicked"));
    }
    assert_ne!(
        claimed[0].1, claimed[1].1,
        "two concurrent appends must not claim the same position"
    );

    // The returned positions, sorted, must name the same streams in the same
    // order `$all` delivers them — the returned position is a real coordinate in
    // the `$all` sequence, not a private counter.
    claimed.sort_by_key(|&(_, pos)| pos);
    let delivered = drain_all(&store).await;
    assert_eq!(delivered.len(), 2, "both appends landed");
    for (i, (expected_key, expected_pos)) in claimed.iter().enumerate() {
        assert_eq!(
            delivered[i].0, *expected_pos,
            "delivery {i} must carry the position that append returned"
        );
        assert_eq!(
            delivered[i].1, *expected_key,
            "delivery {i} must be the stream whose append returned that position"
        );
    }
}
