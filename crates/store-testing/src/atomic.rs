//! `AtomicAppend` capability conformance: several per-stream runs commit in
//! ONE transaction — all land or none do.

use core::future::Future;

use nexus::Version;
use nexus_store::StreamKey;
use nexus_store::import::{AtomicAppend, AtomicAppendError, PlannedAppend};
use nexus_store::wake::WakeSource;
// NOTE: RawEventStore is NOT imported — `AtomicAppend: RawEventStore` is a
// supertrait, and nothing here names the trait directly (unused imports deny).

use crate::row::{
    ConformanceRow, append_rows, assert_strictly_increasing, drain_all, drain_stream, envelope_for,
};

/// Three runs across three streams (two fresh, one existing) commit together.
pub async fn check_atomic_multi_stream_commits_all<S, C, F, Fut>(factory: &F)
where
    S: AtomicAppend + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let existing = StreamKey::from_slice(b"existing");
    append_rows(&store, &existing, &[ConformanceRow::new(1, "E", vec![0])]).await;

    let writes = vec![
        PlannedAppend {
            target: StreamKey::from_slice(b"fresh-a"),
            expected_version: None,
            events: vec![envelope_for(&ConformanceRow::new(1, "E", vec![1]))],
        },
        PlannedAppend {
            target: StreamKey::from_slice(b"fresh-b"),
            expected_version: None,
            events: vec![
                envelope_for(&ConformanceRow::new(1, "E", vec![2])),
                envelope_for(&ConformanceRow::new(2, "E", vec![3])),
            ],
        },
        PlannedAppend {
            target: existing.clone(),
            expected_version: Version::new(1),
            events: vec![envelope_for(&ConformanceRow::new(2, "E", vec![4]))],
        },
    ];
    store
        .atomic_append_many(&writes)
        .await
        .unwrap_or_else(|e| panic!("atomic append must succeed: {e:?}"));

    assert_eq!(
        drain_stream(&store, &StreamKey::from_slice(b"fresh-a"), Version::INITIAL)
            .await
            .len(),
        1
    );
    assert_eq!(
        drain_stream(&store, &StreamKey::from_slice(b"fresh-b"), Version::INITIAL)
            .await
            .len(),
        2
    );
    assert_eq!(
        drain_stream(&store, &existing, Version::INITIAL)
            .await
            .len(),
        2
    );
    let all = drain_all(&store, None).await;
    assert_eq!(all.len(), 5, "$all must hold every committed event");
    assert_strictly_increasing(&all);
}

/// A conflict in ONE run aborts the WHOLE batch: no stream changes, the error
/// names the offending write index and the actual head.
pub async fn check_atomic_conflict_aborts_all<S, C, F, Fut>(factory: &F)
where
    S: AtomicAppend + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let existing = StreamKey::from_slice(b"existing");
    append_rows(&store, &existing, &[ConformanceRow::new(1, "E", vec![0])]).await;
    let all_before = drain_all(&store, None).await;

    let writes = vec![
        PlannedAppend {
            target: StreamKey::from_slice(b"fresh-a"),
            expected_version: None,
            events: vec![envelope_for(&ConformanceRow::new(1, "E", vec![1]))],
        },
        PlannedAppend {
            // WRONG: head is 1, we claim fresh.
            target: existing.clone(),
            expected_version: None,
            events: vec![envelope_for(&ConformanceRow::new(1, "E", vec![9]))],
        },
    ];
    let err = store
        .atomic_append_many(&writes)
        .await
        .expect_err("a conflicting run must abort the batch");
    match err {
        AtomicAppendError::Conflict { index, actual } => {
            assert_eq!(index, 1, "the error must name the offending write");
            assert_eq!(
                actual,
                Version::new(1),
                "the error must carry the actual head"
            );
        }
        other => panic!("expected Conflict, got {other:?}"),
    }

    let fresh = drain_stream(&store, &StreamKey::from_slice(b"fresh-a"), Version::INITIAL).await;
    assert!(
        fresh.is_empty(),
        "NOTHING may land on any stream of an aborted batch"
    );
    let all_after = drain_all(&store, None).await;
    assert_eq!(
        all_after.len(),
        all_before.len(),
        "$all must be untouched by an aborted batch"
    );
}

/// An empty batch is a no-op `Ok`.
pub async fn check_atomic_empty_batch_is_noop<S, C, F, Fut>(factory: &F)
where
    S: AtomicAppend + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    store
        .atomic_append_many(&[])
        .await
        .unwrap_or_else(|e| panic!("empty atomic batch must be Ok: {e:?}"));
    assert!(
        drain_all(&store, None).await.is_empty(),
        "empty batch must write nothing"
    );
}
