//! `AtomicAppend` capability conformance: several per-stream runs commit in
//! ONE transaction — all land or none do.

use core::future::Future;

use mnesis::Version;
use mnesis_store::StreamKey;
use mnesis_store::import::{
    AtomicAppend, AtomicAppendError, InvalidRoute, InvalidRun, PlannedAppend,
};
use mnesis_store::wake::WakeSource;
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
            head: envelope_for(&ConformanceRow::new(1, "E", vec![1])),
            tail: Vec::new(),
        },
        PlannedAppend {
            target: StreamKey::from_slice(b"fresh-b"),
            expected_version: None,
            head: envelope_for(&ConformanceRow::new(1, "E", vec![2])),
            tail: vec![envelope_for(&ConformanceRow::new(2, "E", vec![3]))],
        },
        PlannedAppend {
            target: existing.clone(),
            expected_version: Version::new(1),
            head: envelope_for(&ConformanceRow::new(2, "E", vec![4])),
            tail: Vec::new(),
        },
    ];
    let committed = store
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

    // The returned position is the read-your-writes token for the WHOLE
    // transaction (#330): a consumer that has reached it has been delivered
    // every event the batch committed, across every stream it touched.
    let highest = all
        .iter()
        .map(|(pos, _)| *pos)
        .max()
        .expect("the batch committed events");
    assert_eq!(
        committed,
        Some(highest),
        "atomic_append_many must return the highest position it committed"
    );
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
            head: envelope_for(&ConformanceRow::new(1, "E", vec![1])),
            tail: Vec::new(),
        },
        PlannedAppend {
            // WRONG: head is 1, we claim fresh.
            target: existing.clone(),
            expected_version: None,
            head: envelope_for(&ConformanceRow::new(1, "E", vec![9])),
            tail: Vec::new(),
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

/// An empty batch is a no-op `Ok` — and commits no position.
pub async fn check_atomic_empty_batch_is_noop<S, C, F, Fut>(factory: &F)
where
    S: AtomicAppend + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let committed = store
        .atomic_append_many(&[])
        .await
        .unwrap_or_else(|e| panic!("empty atomic batch must be Ok: {e:?}"));
    assert_eq!(
        committed, None,
        "an empty batch commits nothing, so there is no position to return"
    );
    assert!(
        drain_all(&store, None).await.is_empty(),
        "empty batch must write nothing"
    );
}

fn planned_run(
    target: &StreamKey,
    expected_version: Option<Version>,
    version: u64,
    payload: u8,
) -> PlannedAppend {
    PlannedAppend {
        target: target.clone(),
        expected_version,
        head: envelope_for(&ConformanceRow::new(version, "E", vec![payload])),
        tail: Vec::new(),
    }
}

/// Malformed runs are input errors even when storage has a conflicting head.
pub async fn check_atomic_malformed_runs_reject_all<S, C, F, Fut>(factory: &F)
where
    S: AtomicAppend + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    for (first, second, expected, actual) in [
        (2, None, 1, 2),
        (1, Some(1), 2, 1),
        (1, Some(3), 2, 3),
        (2, Some(1), 3, 1),
    ] {
        for existing in [false, true] {
            let (store, _guard) = factory().await;
            let target = StreamKey::from_slice(b"malformed");
            if existing {
                append_rows(&store, &target, &[ConformanceRow::new(1, "E", vec![9])]).await;
            }
            let before = drain_all(&store, None).await;
            let fresh = StreamKey::from_slice(b"fresh");
            let mut malformed = planned_run(&target, None, first, 2);
            if let Some(version) = second {
                malformed
                    .tail
                    .push(envelope_for(&ConformanceRow::new(version, "E", vec![3])));
            }
            let declared = if first == 2 && second.is_some() {
                Version::new(1)
            } else {
                None
            };
            malformed.expected_version = declared;
            let writes = [planned_run(&fresh, None, 1, 1), malformed];
            let error = store
                .atomic_append_many(&writes)
                .await
                .expect_err("malformed input must reject before storage checks");
            assert!(
                matches!(error, AtomicAppendError::InvalidRun(reason) if reason == InvalidRun::NonSequential {
                    index: 1, expected: Version::new(expected).unwrap(), actual: Version::new(actual).unwrap()
                })
            );
            assert_eq!(drain_all(&store, None).await, before);
            assert_eq!(
                drain_stream(&store, &fresh, Version::INITIAL).await,
                Vec::<ConformanceRow>::new()
            );
        }
    }
}

/// Contiguous, overlapping and gapped duplicate targets reject the whole batch.
pub async fn check_atomic_duplicate_targets_reject_all<S, C, F, Fut>(factory: &F)
where
    S: AtomicAppend + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    for second_version in [1, 3, 5] {
        let (store, _guard) = factory().await;
        let target = StreamKey::from_slice(&[0xff, 0]);
        append_rows(&store, &target, &[ConformanceRow::new(1, "E", vec![9])]).await;
        let before = drain_all(&store, None).await;
        let fresh = StreamKey::from_slice(b"fresh");
        let writes = vec![
            planned_run(&fresh, None, 1, 1),
            planned_run(&target, Version::new(1), 2, 2),
            planned_run(&StreamKey::from_slice(b"other"), None, 1, 3),
            planned_run(&target, Version::new(second_version - 1), second_version, 4),
        ];
        let failure = store
            .atomic_append_many(&writes)
            .await
            .expect_err("duplicate targets must reject before any write");
        assert!(matches!(failure, AtomicAppendError::InvalidRoute(error)
            if error == InvalidRoute { target: target.clone(), first_index: 1, index: 3 }));
        assert_eq!(drain_all(&store, None).await, before);
        assert_eq!(
            drain_stream(&store, &target, Version::INITIAL).await,
            vec![ConformanceRow::new(1, "E", vec![9])]
        );
        assert_eq!(
            drain_stream(&store, &fresh, Version::INITIAL).await,
            Vec::<ConformanceRow>::new()
        );
        assert_eq!(
            drain_stream(&store, &StreamKey::from_slice(b"other"), Version::INITIAL).await,
            Vec::<ConformanceRow>::new()
        );
        let valid = vec![writes[0].clone(), writes[1].clone(), writes[2].clone()];
        assert!(
            store
                .atomic_append_many(&valid)
                .await
                .expect("distinct targets must commit")
                .is_some()
        );
        assert_eq!(drain_all(&store, None).await.len(), 4);
    }
}
