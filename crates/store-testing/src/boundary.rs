//! Defensive Boundary conformance: inputs that violate the append protocol
//! must be rejected cleanly and completely — nothing lands, nothing corrupts —
//! and legal-but-extreme values must round-trip.

use core::future::Future;

use bytes::Bytes;
use mnesis::Version;
use mnesis_store::envelope::pending_envelope;
use mnesis_store::store::RawEventStore;
use mnesis_store::wake::WakeSource;
use mnesis_store::{AppendError, StreamKey};

use crate::row::{ConformanceRow, append_rows, drain_all, drain_stream, envelope_for};

/// A rejected append leaves the store byte-identical — per-stream AND `$all`.
pub async fn check_conflict_leaves_store_unchanged<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let id = StreamKey::from_slice(b"unchanged");
    let rows = vec![
        ConformanceRow::new(1, "E", vec![1]).with_metadata(vec![7]),
        ConformanceRow::new(2, "E", vec![2]),
    ];
    append_rows(&store, &id, &rows).await;
    let before_stream = drain_stream(&store, &id, Version::INITIAL).await;
    let before_all = drain_all(&store, None).await;

    let stale = envelope_for(&ConformanceRow::new(1, "E", vec![9]));
    store
        .append(&id, None, &[stale])
        .await
        .expect_err("stale append must be rejected");

    let after_stream = drain_stream(&store, &id, Version::INITIAL).await;
    let after_all = drain_all(&store, None).await;
    assert_eq!(
        after_stream, before_stream,
        "per-stream contents changed by a REJECTED append"
    );
    assert_eq!(
        after_all.len(),
        before_all.len(),
        "$all grew by a REJECTED append"
    );
}

/// Envelope versions must be sequential from `expected + 1`: a gap inside the
/// batch is rejected as a Conflict and nothing lands.
pub async fn check_version_gap_batch_rejected<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let id = StreamKey::from_slice(b"gap");
    // Fresh stream, expected None => envelopes must be v1, v2, ... — send v1, v3.
    let envs = vec![
        envelope_for(&ConformanceRow::new(1, "E", vec![1])),
        envelope_for(&ConformanceRow::new(3, "E", vec![3])),
    ];
    let err = store
        .append(&id, None, &envs)
        .await
        .expect_err("a version gap inside the batch must be rejected");
    assert!(
        matches!(err, AppendError::Conflict { .. }),
        "gap rejection must be the Conflict domain, got: {err:?}",
    );
    let got = drain_stream(&store, &id, Version::INITIAL).await;
    assert!(
        got.is_empty(),
        "a rejected batch must land NOTHING (got {} rows)",
        got.len()
    );
}

/// First envelope version must equal `expected + 1` — starting a fresh stream
/// at v3 is rejected.
pub async fn check_wrong_first_version_rejected<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let id = StreamKey::from_slice(b"first");
    let envs = vec![envelope_for(&ConformanceRow::new(3, "E", vec![3]))];
    let err = store
        .append(&id, None, &envs)
        .await
        .expect_err("fresh stream starting at v3 must be rejected");
    assert!(matches!(err, AppendError::Conflict { .. }), "got: {err:?}");
    let got = drain_stream(&store, &id, Version::INITIAL).await;
    assert!(got.is_empty(), "nothing may land");
}

/// Metadata `None` and `Some(non-empty)` are DISTINCT values and round-trip
/// as such — present must never collapse to absent.
///
/// `Some(empty)` metadata is unrepresentable by construction upstream
/// (`ValueError::MetadataEmpty`; the wire reserves `u32::MAX` as the absent
/// sentinel), so the absent-vs-empty confusion can never reach an adapter.
/// The adapter contract is therefore: `None` ↔ `None`, and present ↔
/// byte-faithful. The 1-byte minimum (`vec![0]`) is the smallest
/// representable value — the closest edge to the absent sentinel; the zero
/// byte is deliberate: content must not be confused with length.
pub async fn check_metadata_absent_vs_present_distinct<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let id = StreamKey::from_slice(b"meta");
    let rows = vec![
        ConformanceRow::new(1, "E", vec![1]), // absent
        ConformanceRow::new(2, "E", vec![2]).with_metadata(vec![0]), // 1-byte minimum
        ConformanceRow::new(3, "E", vec![3]).with_metadata(vec![1, 2, 3]),
    ];
    append_rows(&store, &id, &rows).await;
    let got = drain_stream(&store, &id, Version::INITIAL).await;
    assert_eq!(
        got[0].metadata, None,
        "absent metadata must read back as None"
    );
    assert_eq!(
        got[1].metadata,
        Some(vec![0]),
        "1-byte minimum metadata must round-trip byte-for-byte — present must never collapse \
         to absent",
    );
    assert_eq!(got[2].metadata, Some(vec![1, 2, 3]));
}

/// A maximum-length event type (`u16::MAX` bytes) round-trips.
pub async fn check_max_length_event_type_round_trips<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let id = StreamKey::from_slice(b"maxtype");
    let long = "a".repeat(usize::from(u16::MAX));
    let env = pending_envelope(Version::INITIAL)
        .event_type_bytes(Bytes::from(long.clone().into_bytes()))
        .expect("u16::MAX event type is within the cap")
        .payload(vec![1])
        .build()
        .expect("valid envelope");
    store
        .append(&id, None, &[env])
        .await
        .unwrap_or_else(|e| panic!("append max-len event type failed: {e:?}"));
    let got = drain_stream(&store, &id, Version::INITIAL).await;
    assert_eq!(
        got[0].event_type, long,
        "u16::MAX event type must round-trip"
    );
}

/// A 1 MiB payload round-trips byte-for-byte — well beyond any internal
/// buffer/batch size (the round-trip check tops out at 4 KiB).
pub async fn check_large_payload_round_trips<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let id = StreamKey::from_slice(b"large-payload");
    let payload: Vec<u8> = (0..1_048_576u32)
        .map(|i| u8::try_from(i % 251).unwrap_or(0)) // prime modulus: no 256-aligned repeats
        .collect();
    append_rows(&store, &id, &[ConformanceRow::new(1, "E", payload.clone())]).await;
    let got = drain_stream(&store, &id, Version::INITIAL).await;
    assert_eq!(got.len(), 1);
    assert_eq!(
        got[0].payload, payload,
        "1 MiB payload must round-trip byte-for-byte"
    );
}

/// Stream ids sharing a byte prefix ("a", "ab") are fully isolated.
///
/// A prefix-collision in the adapter's key encoding would leak events across
/// streams; binary (non-UTF-8) and Unicode ids round-trip like any other id.
pub async fn check_prefix_stream_ids_isolated<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let a = StreamKey::from_slice(b"a");
    let ab = StreamKey::from_slice(b"ab");
    let unicode = StreamKey::from_slice("поток-流".as_bytes());
    let binary = StreamKey::from_slice(&[0x00, 0xff, 0x42]);
    append_rows(&store, &a, &[ConformanceRow::new(1, "E", vec![1])]).await;
    append_rows(
        &store,
        &ab,
        &[
            ConformanceRow::new(1, "E", vec![2]),
            ConformanceRow::new(2, "E", vec![3]),
        ],
    )
    .await;
    append_rows(&store, &unicode, &[ConformanceRow::new(1, "E", vec![4])]).await;
    append_rows(&store, &binary, &[ConformanceRow::new(1, "E", vec![5])]).await;

    let got_a = drain_stream(&store, &a, Version::INITIAL).await;
    assert_eq!(got_a.len(), 1, "stream 'a' must not see 'ab' events");
    assert_eq!(got_a[0].payload, vec![1]);

    let got_ab = drain_stream(&store, &ab, Version::INITIAL).await;
    assert_eq!(got_ab.len(), 2, "stream 'ab' must not see 'a' events");

    let got_u = drain_stream(&store, &unicode, Version::INITIAL).await;
    assert_eq!(got_u.len(), 1, "unicode stream id must round-trip");
    assert_eq!(got_u[0].payload, vec![4]);

    let got_binary = drain_stream(&store, &binary, Version::INITIAL).await;
    assert_eq!(
        got_binary.len(),
        1,
        "binary (non-UTF-8) stream id must round-trip"
    );
    assert_eq!(got_binary[0].payload, vec![5]);
}
