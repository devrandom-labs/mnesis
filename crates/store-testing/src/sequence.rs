//! Sequence/Protocol conformance: multi-step interactions on one store —
//! append→read round-trips, optimistic-conflict protocol, `$all` ordering and
//! resume, and the subscription catch-up→live protocol.

use core::future::Future;
use core::time::Duration;

use futures::StreamExt;
use futures::pin_mut;
use mnesis::Version;
use mnesis_store::store::RawEventStore;
use mnesis_store::wake::WakeSource;
use mnesis_store::{AppendError, PendingBatch, Step, StreamKey, Subscription};
use tokio::time::timeout;

use crate::row::{
    ConformanceRow, SubId, append_event, append_event_at, append_rows, assert_strictly_increasing,
    drain_all, drain_all_attributed, drain_stream, envelope_for,
};

/// Upper bound on any single subscription wait — a hang here means a lost
/// wake, which is exactly what the check exists to catch.
const WAIT: Duration = Duration::from_secs(10);

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
        .append(&id, None, PendingBatch::of(&env))
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
        .append(&id, None, PendingBatch::of(&stale))
        .await
        .expect_err("stale append must conflict");

    // …then the corrected retry (head is 1, next event is v2).
    let retry = envelope_for(&ConformanceRow::new(2, "E", vec![2]));
    store
        .append(&id, Version::new(1), PendingBatch::of(&retry))
        .await
        .expect("retry with corrected expected_version must succeed");

    let got = drain_stream(&store, &id, Version::INITIAL).await;
    let versions: Vec<u64> = got.iter().map(|r| r.version).collect();
    assert_eq!(versions, vec![1, 2], "retry lands exactly one new event");
}

/// Empty store: `read_all(None)` yields nothing.
pub async fn check_all_empty_store_yields_none<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let got = drain_all(&store, None).await;
    assert!(
        got.is_empty(),
        "empty store: read_all(None) must yield nothing"
    );
}

/// `read_all(None)` yields every event across streams in append (position)
/// order, positions strictly increasing.
pub async fn check_all_global_order_across_streams<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let a = StreamKey::from_slice(b"a");
    let b = StreamKey::from_slice(b"b");
    append_event(&store, &a, 1, b"a1").await;
    append_event(&store, &a, 2, b"a2").await;
    append_event(&store, &b, 1, b"b1").await;
    append_event(&store, &a, 3, b"a3").await;
    append_event(&store, &a, 4, b"a4").await;

    let got = drain_all(&store, None).await;
    let payloads: Vec<Vec<u8>> = got.iter().map(|(_, p)| p.clone()).collect();
    assert_eq!(
        payloads,
        vec![
            b"a1".to_vec(),
            b"a2".to_vec(),
            b"b1".to_vec(),
            b"a3".to_vec(),
            b"a4".to_vec()
        ],
        "read_all(None) must yield every event across streams in append order",
    );
    assert_strictly_increasing(&got);
}

/// #330: `append` returns the `$all` position it assigned to the run's last
/// event — the read-your-writes token.
///
/// The returned position must be the exact position `$all` reports for that
/// event, and successive appends (within a stream and across streams) must
/// return strictly increasing positions. Without this an application cannot
/// await its own write over an `$all` projection without dropping to a raw scan.
pub async fn check_append_returns_assigned_all_position<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let a = StreamKey::from_slice(b"a");
    let b = StreamKey::from_slice(b"b");
    let (store, _guard) = factory().await;

    let a1 = append_event_at(&store, &a, 1, b"a1").await;
    let a2 = append_event_at(&store, &a, 2, b"a2").await;
    let b1 = append_event_at(&store, &b, 1, b"b1").await;

    assert!(
        a1 < a2,
        "successive appends to one stream must return strictly increasing positions",
    );
    assert!(
        a2 < b1,
        "positions are store-wide: an append to another stream advances past the last",
    );

    // Each returned position must be the exact position `$all` reports for that
    // event — proven by pairing position to payload over the full `$all` read.
    let all = drain_all(&store, None).await;
    let at = |p: S::AllPosition| {
        all.iter().find(|(pos, _)| *pos == p).map_or_else(
            || panic!("returned position {p:?} is absent from $all"),
            |(_, payload)| payload.clone(),
        )
    };
    assert_eq!(at(a1), b"a1".to_vec(), "a1's position must name a1 in $all");
    assert_eq!(at(a2), b"a2".to_vec(), "a2's position must name a2 in $all");
    assert_eq!(at(b1), b"b1".to_vec(), "b1's position must name b1 in $all");
}

/// #330: a multi-event append returns the LAST event's position, so a consumer
/// that has reached it has necessarily been delivered the whole run.
pub async fn check_multi_event_append_returns_last_position<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let id = StreamKey::from_slice(b"multi");
    let (store, _guard) = factory().await;

    let rows = [
        ConformanceRow::new(1, "E", b"v1".to_vec()),
        ConformanceRow::new(2, "E", b"v2".to_vec()),
        ConformanceRow::new(3, "E", b"v3".to_vec()),
    ];
    let envs: Vec<_> = rows.iter().map(envelope_for).collect();
    let returned = store
        .append(
            &id,
            None,
            PendingBatch::new(&envs).expect("three rows are non-empty"),
        )
        .await
        .unwrap_or_else(|e| panic!("multi-event append failed: {e:?}"));

    let all = drain_all(&store, None).await;
    let last = all.last().expect("three events landed");
    assert_eq!(
        returned, last.0,
        "the returned position must be the LAST event's, not the first",
    );
    assert_eq!(last.1, b"v3".to_vec(), "and the last event is v3");
    assert_ne!(
        returned, all[0].0,
        "the first event's position must not be what append returned",
    );
}

/// #333: every `$all` item carries the origin [`StreamKey`](mnesis_store::StreamKey).
///
/// Attribution is a store guarantee, not a payload convention. Interleaves two
/// streams and asserts each item's key matches its append target, in position
/// order.
pub async fn check_all_items_carry_their_stream_key<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let alpha = StreamKey::from_slice(b"alpha");
    let beta = StreamKey::from_slice(b"beta");
    append_event(&store, &alpha, 1, b"a1").await;
    append_event(&store, &beta, 1, b"b1").await;
    append_event(&store, &alpha, 2, b"a2").await;

    let got = drain_all_attributed(&store, None).await;
    let attributed: Vec<(Vec<u8>, Vec<u8>)> =
        got.iter().map(|(_, k, p)| (k.clone(), p.clone())).collect();
    assert_eq!(
        attributed,
        vec![
            (b"alpha".to_vec(), b"a1".to_vec()),
            (b"beta".to_vec(), b"b1".to_vec()),
            (b"alpha".to_vec(), b"a2".to_vec()),
        ],
        "each $all item must carry the StreamKey of the stream it was appended to, in position order",
    );
}

/// `read_all(Some(p))` is EXCLUSIVE: strictly after `p`.
pub async fn check_all_from_is_exclusive<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let a = StreamKey::from_slice(b"a");
    append_event(&store, &a, 1, b"a1").await;
    append_event(&store, &a, 2, b"a2").await;
    append_event(&store, &a, 3, b"a3").await;

    let full = drain_all(&store, None).await;
    assert_eq!(full.len(), 3);
    let checkpoint = full[0].0;

    let rest = drain_all(&store, Some(checkpoint)).await;
    let payloads: Vec<Vec<u8>> = rest.iter().map(|(_, p)| p.clone()).collect();
    assert_eq!(
        payloads,
        vec![b"a2".to_vec(), b"a3".to_vec()],
        "read_all(Some(p)) is EXCLUSIVE",
    );
    assert!(
        rest[0].0 > checkpoint,
        "resumed position must be strictly after checkpoint"
    );
}

/// Multi-resume cycles reconstruct the single-shot read exactly — no gap,
/// duplicate, or skip across the seams.
pub async fn check_all_multi_resume_cycles<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let a = StreamKey::from_slice(b"a");
    let b = StreamKey::from_slice(b"b");
    let mut va = 0u64;
    let mut vb = 0u64;
    let mut expected: Vec<Vec<u8>> = Vec::new();
    for i in 0..10u64 {
        if i % 2 == 0 {
            va += 1;
            let p = format!("a{va}").into_bytes();
            append_event(&store, &a, va, &p).await;
            expected.push(p);
        } else {
            vb += 1;
            let p = format!("b{vb}").into_bytes();
            append_event(&store, &b, vb, &p).await;
            expected.push(p);
        }
    }

    let full = drain_all(&store, None).await;
    let full_payloads: Vec<Vec<u8>> = full.iter().map(|(_, p)| p.clone()).collect();
    assert_eq!(
        full_payloads, expected,
        "single-shot read_all(None) must match append order"
    );

    let mut acc: Vec<(S::AllPosition, Vec<u8>)> = Vec::new();
    let mut checkpoint: Option<S::AllPosition> = None;
    loop {
        let stream = store
            .read_all(checkpoint)
            .await
            .unwrap_or_else(|e| panic!("open read_all cycle failed: {e:?}"));
        pin_mut!(stream);
        let mut taken = 0;
        let mut advanced = false;
        while let Some(item) = stream.next().await {
            let (pos, _key, env) = item.unwrap_or_else(|e| panic!("cycle item errored: {e:?}"));
            acc.push((pos, env.payload().to_vec()));
            checkpoint = Some(pos);
            advanced = true;
            taken += 1;
            if taken == 3 {
                break;
            }
        }
        if !advanced {
            break;
        }
    }

    let acc_payloads: Vec<Vec<u8>> = acc.iter().map(|(_, p)| p.clone()).collect();
    assert_eq!(
        acc_payloads, full_payloads,
        "multi-resume cycles must reconstruct the full stream exactly",
    );
    assert_strictly_increasing(&acc);
}

/// `read_all(Some(last))` is empty at the boundary; a later append surfaces
/// exactly the new event from the same checkpoint.
pub async fn check_all_boundary_then_new_append<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let a = StreamKey::from_slice(b"a");
    let b = StreamKey::from_slice(b"b");
    append_event(&store, &a, 1, b"a1").await;
    append_event(&store, &b, 1, b"b1").await;

    let full = drain_all(&store, None).await;
    assert_eq!(full.len(), 2);
    let last = full.last().expect("non-empty").0;

    let empty = drain_all(&store, Some(last)).await;
    assert!(
        empty.is_empty(),
        "nothing is strictly after the last position"
    );

    append_event(&store, &a, 2, b"a2").await;
    let after = drain_all(&store, Some(last)).await;
    let payloads: Vec<Vec<u8>> = after.iter().map(|(_, p)| p.clone()).collect();
    assert_eq!(
        payloads,
        vec![b"a2".to_vec()],
        "same checkpoint surfaces exactly the new event"
    );
    assert!(
        after[0].0 > last,
        "new position must be strictly after the prior last"
    );
}

/// Inclusive `read_stream` and exclusive `read_all` coexist on one store —
/// the intentional asymmetry (CLAUDE rule 4).
pub async fn check_read_stream_inclusive_read_all_exclusive_coexist<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let a = StreamKey::from_slice(b"a");
    append_event(&store, &a, 1, b"a1").await;
    append_event(&store, &a, 2, b"a2").await;
    append_event(&store, &a, 3, b"a3").await;

    let got = drain_stream(&store, &a, Version::new(2).expect("v2")).await;
    let versions: Vec<u64> = got.iter().map(|r| r.version).collect();
    assert_eq!(versions, vec![2, 3], "read_stream(from=2) is INCLUSIVE");

    let full = drain_all(&store, None).await;
    assert_eq!(full.len(), 3);
    let pos_of_a2 = full[1].0;
    let after = drain_all(&store, Some(pos_of_a2)).await;
    let payloads: Vec<Vec<u8>> = after.iter().map(|(_, p)| p.clone()).collect();
    assert_eq!(
        payloads,
        vec![b"a3".to_vec()],
        "read_all(from=pos(a2)) is EXCLUSIVE"
    );
}

/// Take the next subscription item within `WAIT`, panicking on hang, stream
/// end, or read error. Returns the `Step`.
async fn next_step<St, T, E>(stream: &mut core::pin::Pin<&mut St>, what: &str) -> Step<T>
where
    St: futures::Stream<Item = Result<Step<T>, E>>,
    E: core::fmt::Debug,
{
    timeout(WAIT, stream.next())
        .await
        .unwrap_or_else(|_| panic!("{what}: subscription hung (lost wake?)"))
        .unwrap_or_else(|| panic!("{what}: subscription ended (must never return None)"))
        .unwrap_or_else(|e| panic!("{what}: subscription item errored: {e:?}"))
}

/// Per-stream subscription protocol: backlog in order, then `CaughtUp`
/// exactly once, then live events.
pub async fn check_subscription_backlog_then_caught_up_then_live<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    S::Stream: Unpin,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (raw, _guard) = factory().await;
    let store = raw.into_store();
    let id = SubId::new("sub-proto");
    for v in 1..=3u64 {
        append_event(&store, &id.key(), v, format!("p{v}").as_bytes()).await;
    }

    let sub = Subscription::new(&store);
    let stream = sub
        .subscribe(&id, None)
        .unwrap_or_else(|e| panic!("register failed: {e:?}"));
    pin_mut!(stream);

    for want in 1..=3u64 {
        match next_step(&mut stream, "backlog").await {
            Step::Event(env) => assert_eq!(
                env.version().as_u64(),
                want,
                "backlog must replay in version order",
            ),
            Step::CaughtUp => panic!("CaughtUp before the backlog drained (at v{want})"),
        }
    }
    match next_step(&mut stream, "boundary").await {
        Step::CaughtUp => {}
        Step::Event(env) => panic!(
            "expected CaughtUp after backlog, got Event v{}",
            env.version()
        ),
    }

    // Live phase: an append after CaughtUp is delivered.
    append_event(&store, &id.key(), 4, b"p4").await;
    match next_step(&mut stream, "live").await {
        Step::Event(env) => assert_eq!(env.version().as_u64(), 4, "live event must be v4"),
        Step::CaughtUp => panic!("CaughtUp must be emitted exactly once"),
    }
}

/// `subscribe(Some(v))` resumes STRICTLY AFTER `v` — no duplicate of the
/// checkpointed event.
pub async fn check_subscription_resume_strict_after<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    S::Stream: Unpin,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (raw, _guard) = factory().await;
    let store = raw.into_store();
    let id = SubId::new("sub-resume");
    for v in 1..=5u64 {
        append_event(&store, &id.key(), v, b"p").await;
    }

    let sub = Subscription::new(&store);
    let stream = sub
        .subscribe(&id, Some(Version::new(3).expect("v3")))
        .unwrap_or_else(|e| panic!("register failed: {e:?}"));
    pin_mut!(stream);

    match next_step(&mut stream, "resume").await {
        Step::Event(env) => assert_eq!(
            env.version().as_u64(),
            4,
            "resume from Some(3) must deliver v4 first (strict-after, no dup)",
        ),
        Step::CaughtUp => panic!("expected v4 before CaughtUp"),
    }
}

/// `$all` subscription protocol: cross-stream backlog in position order, then
/// `CaughtUp` exactly once, then live events with strictly increasing tags.
pub async fn check_subscription_all_backlog_then_caught_up_then_live<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    S::AllStream: Unpin,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (raw, _guard) = factory().await;
    let store = raw.into_store();
    let a = StreamKey::from_slice(b"a");
    let b = StreamKey::from_slice(b"b");
    append_event(&store, &a, 1, b"a1").await;
    append_event(&store, &b, 1, b"b1").await;
    append_event(&store, &a, 2, b"a2").await;

    let sub = Subscription::new(&store);
    let stream = sub
        .subscribe_all(None)
        .unwrap_or_else(|e| panic!("register failed: {e:?}"));
    pin_mut!(stream);

    let mut backlog: Vec<(S::AllPosition, Vec<u8>, Vec<u8>)> = Vec::new();
    while let Step::Event((pos, key, env)) = next_step(&mut stream, "all backlog").await {
        backlog.push((pos, key.as_bytes().to_vec(), env.payload().to_vec()));
    }
    let payloads: Vec<Vec<u8>> = backlog.iter().map(|(_, _, p)| p.clone()).collect();
    assert_eq!(
        payloads,
        vec![b"a1".to_vec(), b"b1".to_vec(), b"a2".to_vec()],
        "$all backlog must replay in position order",
    );
    let keys: Vec<Vec<u8>> = backlog.iter().map(|(_, k, _)| k.clone()).collect();
    assert_eq!(
        keys,
        vec![b"a".to_vec(), b"b".to_vec(), b"a".to_vec()],
        "$all backlog items must carry the StreamKey of their append target",
    );
    let positions: Vec<(S::AllPosition, Vec<u8>)> =
        backlog.iter().map(|(p, _, _)| (*p, Vec::new())).collect();
    assert_strictly_increasing(&positions);
    let last = backlog.last().expect("non-empty").0;

    append_event(&store, &b, 2, b"b2").await;
    match next_step(&mut stream, "all live").await {
        Step::Event((pos, key, env)) => {
            assert_eq!(
                env.payload(),
                b"b2",
                "live $all event must be the new append"
            );
            assert_eq!(
                key.as_bytes(),
                b"b",
                "live $all item must carry the StreamKey of its append target",
            );
            assert!(
                pos > last,
                "live position must be strictly after the backlog"
            );
        }
        Step::CaughtUp => panic!("CaughtUp must be emitted exactly once"),
    }
}

/// Subscribing to a stream that does not exist yet parks (after `CaughtUp`)
/// and is woken by the stream's FIRST event — the producer-after-consumer
/// startup order must work.
pub async fn check_subscription_absent_stream_waits_then_delivers<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    S::Stream: Unpin,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (raw, _guard) = factory().await;
    let store = raw.into_store();
    let id = SubId::new("ghost");

    let sub = Subscription::new(&store);
    let stream = sub
        .subscribe(&id, None)
        .unwrap_or_else(|e| panic!("register failed: {e:?}"));
    pin_mut!(stream);

    // An absent stream has an empty backlog: CaughtUp arrives first.
    match next_step(&mut stream, "absent-stream boundary").await {
        Step::CaughtUp => {}
        Step::Event(env) => panic!("absent stream must have no backlog, got v{}", env.version()),
    }

    // The FIRST event ever written to the stream wakes the parked subscriber.
    append_event(&store, &id.key(), 1, b"first").await;
    match next_step(&mut stream, "absent-stream first event").await {
        Step::Event(env) => {
            assert_eq!(env.version().as_u64(), 1, "the first event must be v1");
            assert_eq!(env.payload(), b"first");
        }
        Step::CaughtUp => panic!("CaughtUp must be emitted exactly once"),
    }
}

/// Two simultaneous subscribers on ONE stream each receive the full event
/// sequence — subscriptions are fan-out, never competing-consumer queues.
pub async fn check_two_subscribers_same_stream_both_receive<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    S::Stream: Unpin,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (raw, _guard) = factory().await;
    let store = raw.into_store();
    let id = SubId::new("fanout");
    append_event(&store, &id.key(), 1, b"p1").await;

    let sub = Subscription::new(&store);
    let stream_a = sub
        .subscribe(&id, None)
        .unwrap_or_else(|e| panic!("register a failed: {e:?}"));
    let stream_b = sub
        .subscribe(&id, None)
        .unwrap_or_else(|e| panic!("register b failed: {e:?}"));
    pin_mut!(stream_a);
    pin_mut!(stream_b);

    // Both drain the backlog and reach CaughtUp independently.
    match next_step(&mut stream_a, "fanout backlog a").await {
        Step::Event(env) => assert_eq!(env.version().as_u64(), 1, "subscriber a backlog"),
        Step::CaughtUp => panic!("subscriber a: CaughtUp before backlog"),
    }
    match next_step(&mut stream_a, "fanout boundary a").await {
        Step::CaughtUp => {}
        Step::Event(env) => panic!("subscriber a: expected CaughtUp, got v{}", env.version()),
    }
    match next_step(&mut stream_b, "fanout backlog b").await {
        Step::Event(env) => assert_eq!(env.version().as_u64(), 1, "subscriber b backlog"),
        Step::CaughtUp => panic!("subscriber b: CaughtUp before backlog"),
    }
    match next_step(&mut stream_b, "fanout boundary b").await {
        Step::CaughtUp => {}
        Step::Event(env) => panic!("subscriber b: expected CaughtUp, got v{}", env.version()),
    }

    // One live append reaches BOTH subscribers.
    append_event(&store, &id.key(), 2, b"p2").await;
    match next_step(&mut stream_a, "fanout live a").await {
        Step::Event(env) => assert_eq!(
            env.version().as_u64(),
            2,
            "subscriber a must receive the live event — fan-out, not a queue",
        ),
        Step::CaughtUp => panic!("subscriber a: CaughtUp must be emitted exactly once"),
    }
    match next_step(&mut stream_b, "fanout live b").await {
        Step::Event(env) => assert_eq!(
            env.version().as_u64(),
            2,
            "subscriber b must receive the live event — fan-out, not a queue",
        ),
        Step::CaughtUp => panic!("subscriber b: CaughtUp must be emitted exactly once"),
    }
}

/// A backlog larger than the catch-up chunk (1024) crosses the internal
/// rescan seams with no gap or duplicate, and `CaughtUp` still arrives.
pub async fn check_subscription_large_backlog_crosses_chunk_seam<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    S::Stream: Unpin,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    const N: u64 = 2500; // > 2 × CATCHUP_CHUNK (1024)
    let (raw, _guard) = factory().await;
    let store = raw.into_store();
    let id = SubId::new("sub-chunk");
    let rows: Vec<_> = (1..=N)
        .map(|v| ConformanceRow::new(v, "E", vec![]))
        .collect();
    append_rows(&store, &id.key(), &rows).await;

    let sub = Subscription::new(&store);
    let stream = sub
        .subscribe(&id, None)
        .unwrap_or_else(|e| panic!("register failed: {e:?}"));
    pin_mut!(stream);

    let mut versions = Vec::with_capacity(usize::try_from(N).unwrap_or(usize::MAX));
    while let Step::Event(env) = next_step(&mut stream, "chunk backlog").await {
        versions.push(env.version().as_u64());
    }
    let want: Vec<u64> = (1..=N).collect();
    assert_eq!(
        versions, want,
        "backlog across chunk seams must be exactly 1..=N — no gap, no duplicate",
    );
}

/// A subscription opened beyond the head filters below-bound live appends.
///
/// `subscribe(Some(v))` with `v` past the current head parks after an empty
/// backlog; live appends at versions **at or below** `v` wake the loop but
/// must never be delivered — the first delivered event is `v + 1`. A loop or
/// adapter that rescans from the wrong position after a below-bound wake
/// would surface one of the filtered events here.
pub async fn check_subscription_beyond_head_filters_below_bound<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    S::Stream: Unpin,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (raw, _guard) = factory().await;
    let store = raw.into_store();
    let id = SubId::new("beyond-head");
    append_event(&store, &id.key(), 1, b"p1").await;
    append_event(&store, &id.key(), 2, b"p2").await;

    // Head is 2; subscribe strictly after 5 — the backlog is empty.
    let sub = Subscription::new(&store);
    let stream = sub
        .subscribe(&id, Some(Version::new(5).expect("v5")))
        .unwrap_or_else(|e| panic!("register failed: {e:?}"));
    pin_mut!(stream);
    match next_step(&mut stream, "beyond-head boundary").await {
        Step::CaughtUp => {}
        Step::Event(env) => panic!(
            "subscribing beyond the head must have an empty backlog, got v{}",
            env.version(),
        ),
    }

    // Live appends at v3..=v5 are all <= from: each wakes the loop, none may
    // be delivered. v6 is the first version strictly after `from`.
    for v in 3..=6u64 {
        append_event(&store, &id.key(), v, format!("p{v}").as_bytes()).await;
    }
    match next_step(&mut stream, "beyond-head first delivery").await {
        Step::Event(env) => assert_eq!(
            env.version().as_u64(),
            6,
            "the first delivered event must be from+1 (6) — below-bound live appends must be filtered, never delivered",
        ),
        Step::CaughtUp => panic!("CaughtUp must be emitted exactly once"),
    }
}
