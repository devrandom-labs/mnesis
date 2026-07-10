//! Sequence/Protocol conformance: multi-step interactions on one store —
//! append→read round-trips, optimistic-conflict protocol, `$all` ordering and
//! resume, and the subscription catch-up→live protocol.

use core::future::Future;

use futures::StreamExt;
use futures::pin_mut;
use nexus::Version;
use nexus_store::store::RawEventStore;
use nexus_store::wake::WakeSource;
use nexus_store::{AppendError, StreamKey};

use crate::row::{ConformanceRow, append_rows, drain_stream, envelope_for};

// Task 3 extends this import block with: core::time::Duration,
// tokio::time::timeout, nexus_store::{Step, Subscription}, and
// crate::row::{SubId, append_event, assert_strictly_increasing, drain_all} —
// unused imports are DENIED, so add them only when their checks land.

/// A fresh, empty stream reads back empty (absent stream = empty, not error).
pub async fn check_empty_read_yields_none<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let got = drain_stream(&store, &StreamKey::from_slice(b"missing"), Version::INITIAL).await;
    assert!(
        got.is_empty(),
        "reading an absent stream must yield an empty stream, got {} rows",
        got.len(),
    );
}

/// Mixed-shape rows round-trip byte-for-byte in insertion order: Unicode and
/// dotted event types, schema versions across the u32 range, payloads from
/// empty through 4 KiB, metadata absent and present.
pub async fn check_append_then_read_round_trips<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let id = StreamKey::from_slice(b"round-trip");
    let rows = vec![
        ConformanceRow::new(1, "Created", vec![]),
        ConformanceRow::new(2, "user.signed_up", vec![0]).with_schema_version(7),
        ConformanceRow::new(3, "ÉvénementUTF8", vec![0; 64]).with_metadata(vec![1, 2, 3]),
        ConformanceRow::new(4, "with spaces 123", vec![0xff; 64]).with_schema_version(u32::MAX),
        ConformanceRow::new(5, "E", (0..=255u8).collect()),
        ConformanceRow::new(
            6,
            "E",
            (0..4096u32)
                .map(|i| u8::try_from(i % 256).unwrap_or(0))
                .collect(),
        ),
    ];
    append_rows(&store, &id, &rows).await;
    let got = drain_stream(&store, &id, Version::INITIAL).await;
    assert_eq!(
        got, rows,
        "rows must round-trip byte-for-byte in insertion order"
    );
}

/// Versions read back strictly monotonic and the stream is fused after `None`.
pub async fn check_versions_strictly_monotonic_and_fused<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let id = StreamKey::from_slice(b"monotonic");
    let rows: Vec<_> = (1..=64u64)
        .map(|v| ConformanceRow::new(v, "E", vec![]))
        .collect();
    append_rows(&store, &id, &rows).await;

    let stream = store
        .read_stream(&id, Version::INITIAL)
        .await
        .unwrap_or_else(|e| panic!("read_stream failed: {e:?}"));
    pin_mut!(stream);
    let mut versions = Vec::new();
    while let Some(item) = stream.next().await {
        versions.push(
            item.unwrap_or_else(|e| panic!("item errored: {e:?}"))
                .version()
                .as_u64(),
        );
    }
    let want: Vec<u64> = (1..=64).collect();
    assert_eq!(
        versions, want,
        "versions must be exactly 1..=64, strictly increasing"
    );
    for i in 0..8 {
        assert!(
            stream.next().await.is_none(),
            "fused-after-None violated on repeat #{i}",
        );
    }
}

/// A stream larger than any internal batch/refill size (1500 events) drains
/// completely with no gap or duplicate across the seams.
pub async fn check_large_stream_completes<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let id = StreamKey::from_slice(b"large");
    let rows: Vec<_> = (1..=1500u64)
        .map(|v| ConformanceRow::new(v, "E", vec![]))
        .collect();
    append_rows(&store, &id, &rows).await;
    let got = drain_stream(&store, &id, Version::INITIAL).await;
    let versions: Vec<u64> = got.iter().map(|r| r.version).collect();
    let want: Vec<u64> = (1..=1500).collect();
    assert_eq!(
        versions, want,
        "1500-event stream must drain exactly 1..=1500"
    );
}

/// `read_stream(from)` is INCLUSIVE: from=3 on a 5-event stream yields 3,4,5.
pub async fn check_read_stream_from_is_inclusive<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let id = StreamKey::from_slice(b"inclusive");
    let rows: Vec<_> = (1..=5u64)
        .map(|v| ConformanceRow::new(v, "E", vec![]))
        .collect();
    append_rows(&store, &id, &rows).await;
    let got = drain_stream(&store, &id, Version::new(3).expect("v3")).await;
    let versions: Vec<u64> = got.iter().map(|r| r.version).collect();
    assert_eq!(
        versions,
        vec![3, 4, 5],
        "read_stream(from=3) is inclusive: yields 3,4,5"
    );
}

/// A mismatched `expected_version` surfaces `AppendError::Conflict` carrying
/// the store's actual head, and the store is untouched.
pub async fn check_append_conflict_is_surfaced<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let id = StreamKey::from_slice(b"conflict");
    append_rows(
        &store,
        &id,
        &[
            ConformanceRow::new(1, "E", vec![1]),
            ConformanceRow::new(2, "E", vec![2]),
        ],
    )
    .await;

    // Stale expectation: stream head is 2, we claim it's still fresh.
    let env = envelope_for(&ConformanceRow::new(1, "E", vec![9]));
    let err = store
        .append(&id, None, &[env])
        .await
        .expect_err("appending with a stale expected_version must fail");
    match err {
        AppendError::Conflict { actual, .. } => {
            assert_eq!(
                actual,
                Version::new(2),
                "Conflict must carry the actual head (2)",
            );
        }
        other => panic!("expected Conflict, got {other:?}"),
    }

    let got = drain_stream(&store, &id, Version::INITIAL).await;
    assert_eq!(got.len(), 2, "a conflicted append must not land any event");
    assert_eq!(got[0].payload, vec![1]);
    assert_eq!(got[1].payload, vec![2]);
}

/// After a conflict, retrying with the corrected expectation succeeds — the
/// standard optimistic-concurrency protocol completes.
pub async fn check_append_retry_after_conflict_succeeds<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let id = StreamKey::from_slice(b"retry");
    append_rows(&store, &id, &[ConformanceRow::new(1, "E", vec![1])]).await;

    // Conflict first…
    let stale = envelope_for(&ConformanceRow::new(1, "E", vec![9]));
    store
        .append(&id, None, &[stale])
        .await
        .expect_err("stale append must conflict");

    // …then the corrected retry (head is 1, next event is v2).
    let retry = envelope_for(&ConformanceRow::new(2, "E", vec![2]));
    store
        .append(&id, Version::new(1), &[retry])
        .await
        .expect("retry with corrected expected_version must succeed");

    let got = drain_stream(&store, &id, Version::INITIAL).await;
    let versions: Vec<u64> = got.iter().map(|r| r.version).collect();
    assert_eq!(versions, vec![1, 2], "retry lands exactly one new event");
}
