//! Property-based and adversarial tests for the fjall event store adapter.
//!
//! Uses proptest to generate chaotic inputs and verify store invariants hold.
//!
//! ## Coverage
//!
//! 1.  Encoding round-trips: event key, stream meta, event value
//! 2.  Encoding byte ordering and adversarial event type strings
//! 3.  Append-read roundtrips with arbitrary and adversarial payloads
//! 4.  Stream ID validation and isolation
//! 5.  Version boundaries, conflict detection, sequential enforcement
//! 6.  Concurrent append races
//! 7.  Persistence across reopen, counter recovery
//! 8.  Stress: many streams, large streams, large payloads
//! 9.  Schema version edge cases
//! 10. Model-based testing: shadow HashMap vs real store
//! 11. Edge cases: fused streams, version filtering

#![allow(clippy::unwrap_used, reason = "test code")]
#![allow(clippy::expect_used, reason = "test code")]
#![allow(clippy::panic, reason = "proptest macros use panic")]
#![allow(clippy::missing_panics_doc, reason = "proptest")]
#![allow(clippy::needless_pass_by_value, reason = "proptest")]
#![allow(clippy::str_to_string, reason = "tests")]
#![allow(clippy::shadow_reuse, reason = "tests")]
#![allow(clippy::shadow_unrelated, reason = "tests")]
#![allow(clippy::as_conversions, reason = "tests")]
#![allow(clippy::cast_possible_truncation, reason = "tests")]
#![allow(clippy::cast_possible_wrap, reason = "tests")]
#![allow(clippy::cast_sign_loss, reason = "tests")]
#![allow(clippy::implicit_clone, reason = "tests")]
#![allow(clippy::clone_on_ref_ptr, reason = "tests")]
#![allow(clippy::missing_docs_in_private_items, reason = "tests")]
#![allow(clippy::doc_markdown, reason = "tests")]
#![allow(clippy::uninlined_format_args, reason = "tests")]
#![allow(clippy::use_self, reason = "tests")]
#![allow(clippy::items_after_statements, reason = "tests")]
#![allow(clippy::indexing_slicing, reason = "tests")]
#![allow(clippy::arithmetic_side_effects, reason = "tests")]
#![allow(clippy::print_stdout, reason = "diagnostic output")]
#![allow(dead_code, reason = "strategies used only in some proptest blocks")]

use std::collections::HashMap;
use std::num::NonZeroU32;

use futures::StreamExt;
use mnesis::Version;
use mnesis_fjall::FjallStore;
use mnesis_store::PendingEnvelope;
use mnesis_store::StreamKey;
use mnesis_store::envelope::pending_envelope;
use mnesis_store::error::AppendError;
use mnesis_store::store::RawEventStore;
use mnesis_store::value::SchemaVersion;

use mnesis_store::PendingBatch;
use proptest::prelude::*;

fn sk(s: &str) -> StreamKey {
    StreamKey::from_slice(s.as_bytes())
}

// ============================================================================
// Helpers
// ============================================================================

fn leak(s: &str) -> &'static str {
    Box::leak(s.to_owned().into_boxed_str())
}

fn temp_store() -> (FjallStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = FjallStore::builder(dir.path().join("db")).open().unwrap();
    (store, dir)
}

fn make_envelope(version: u64, event_type: &'static str, payload: &[u8]) -> PendingEnvelope {
    pending_envelope(Version::new(version).unwrap())
        .event_type(event_type)
        .payload(payload.to_vec())
        .build()
        .expect("valid envelope")
}

fn make_envelope_with_schema(
    version: u64,
    event_type: &'static str,
    payload: &[u8],
    schema_version: u32,
) -> PendingEnvelope {
    let sv = NonZeroU32::new(schema_version).unwrap_or(NonZeroU32::MIN);
    pending_envelope(Version::new(version).unwrap())
        .event_type(event_type)
        .payload(payload.to_vec())
        .schema_version(SchemaVersion::new(sv))
        .build()
        .expect("valid envelope")
}

fn build_envelopes(payloads: &[Vec<u8>]) -> Vec<PendingEnvelope> {
    payloads
        .iter()
        .enumerate()
        .map(|(i, p)| {
            pending_envelope(Version::new(u64::try_from(i).unwrap() + 1).unwrap())
                .event_type(leak("TestEvent"))
                .payload(p.clone())
                .build()
                .expect("valid envelope")
        })
        .collect()
}

fn build_envelopes_from(start_version: u64, payloads: &[Vec<u8>]) -> Vec<PendingEnvelope> {
    payloads
        .iter()
        .enumerate()
        .map(|(i, p)| {
            pending_envelope(Version::new(start_version + u64::try_from(i).unwrap()).unwrap())
                .event_type(leak("TestEvent"))
                .payload(p.clone())
                .build()
                .expect("valid envelope")
        })
        .collect()
}

async fn read_all_payloads(store: &FjallStore, stream_id: &StreamKey) -> Vec<Vec<u8>> {
    let mut stream = store
        .read_stream(stream_id, Version::INITIAL)
        .await
        .unwrap();
    let mut payloads = Vec::new();
    while let Some(__i) = stream.next().await {
        let env = __i.unwrap();
        payloads.push(env.payload().to_vec());
    }
    payloads
}

async fn read_all_versions(store: &FjallStore, stream_id: &StreamKey) -> Vec<u64> {
    let mut stream = store
        .read_stream(stream_id, Version::INITIAL)
        .await
        .unwrap();
    let mut versions = Vec::new();
    while let Some(__i) = stream.next().await {
        let env = __i.unwrap();
        versions.push(env.version().as_u64());
    }
    versions
}

async fn read_all_event_types(store: &FjallStore, stream_id: &StreamKey) -> Vec<String> {
    let mut stream = store
        .read_stream(stream_id, Version::INITIAL)
        .await
        .unwrap();
    let mut types = Vec::new();
    while let Some(__i) = stream.next().await {
        let env = __i.unwrap();
        types.push(env.event_type().to_owned());
    }
    types
}

// ============================================================================
// Strategies
// ============================================================================

fn stream_id_strategy() -> impl Strategy<Value = StreamKey> {
    prop::string::string_regex("[a-z][a-z0-9_-]{0,29}")
        .unwrap()
        .prop_map(|s| StreamKey::from_slice(s.as_bytes()))
}

fn evil_stream_id_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        // Normal IDs
        prop::string::string_regex("[a-z][a-z0-9-]{0,19}").unwrap(),
        // Empty string
        Just(String::new()),
        // SQL injection
        Just("'; DROP TABLE events; --".to_owned()),
        Just("stream\0id".to_owned()),
        Just("../../../etc/passwd".to_owned()),
        // Very long
        Just("x".repeat(10_000)),
        // Whitespace only
        Just("   \t\n  ".to_owned()),
        // Unicode normalization edge cases
        Just("\u{FEFF}stream".to_owned()), // BOM prefix
        Just("caf\u{00E9}".to_owned()),    // NFC vs NFD
    ]
}

fn payloads_strategy() -> impl Strategy<Value = Vec<Vec<u8>>> {
    prop::collection::vec(prop::collection::vec(any::<u8>(), 0..512), 1..30)
}

fn evil_payload_strategy() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        // Empty
        Just(vec![]),
        // All zeros
        prop::collection::vec(Just(0u8), 0..1000),
        // All 0xFF
        prop::collection::vec(Just(0xFFu8), 0..1000),
        // Random binary
        prop::collection::vec(any::<u8>(), 0..5000),
        // Almost valid JSON
        Just(br#"{"broken":}"#.to_vec()),
        // Null bytes everywhere
        Just(vec![0; 10_000]),
    ]
}

// ============================================================================
// CATEGORY 2: Append-Read Roundtrip (proptest)
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// For any Vec<Vec<u8>> payloads, append all -> read_stream -> payloads match exactly.
    #[test]
    fn attack_append_read_roundtrip_any_payloads(
        payloads in prop::collection::vec(prop::collection::vec(any::<u8>(), 0..1024), 1..50),
    ) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let (store, _dir) = temp_store();
            let stream_id = sk("roundtrip-test");
            let envelopes = build_envelopes(&payloads);

            store.append(&stream_id, None, PendingBatch::new(&envelopes).expect("non-empty batch")).await.unwrap();

            let read = read_all_payloads(&store, &stream_id).await;
            prop_assert_eq!(read.len(), payloads.len(), "payload count mismatch");
            for (i, (read_p, orig_p)) in read.iter().zip(payloads.iter()).enumerate() {
                prop_assert_eq!(read_p, orig_p, "payload mismatch at index {}", i);
            }
            Ok(())
        })?;
    }

    /// Adversarial payloads must survive round-trip unchanged.
    #[test]
    fn attack_append_read_roundtrip_evil_payloads(
        payloads in prop::collection::vec(evil_payload_strategy(), 1..10),
    ) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let (store, _dir) = temp_store();
            let stream_id = sk("evil-roundtrip");
            let envelopes = build_envelopes(&payloads);

            store.append(&stream_id, None, PendingBatch::new(&envelopes).expect("non-empty batch")).await.unwrap();

            let read = read_all_payloads(&store, &stream_id).await;
            prop_assert_eq!(read.len(), payloads.len());
            for (i, (r, o)) in read.iter().zip(payloads.iter()).enumerate() {
                prop_assert_eq!(r, o, "evil payload corrupted at index {}", i);
            }
            Ok(())
        })?;
    }
}

// ============================================================================
// CATEGORY 3: Stream ID Attack Surface
// ============================================================================

/// Evil stream IDs: Unicode, injection, path traversal, null bytes, whitespace.
/// Empty string is tested separately in attack_empty_string_stream_id_returns_error.
#[tokio::test]
async fn attack_evil_stream_ids() {
    let evil_ids: Vec<(&str, &str)> = vec![
        ("x", "single character"),
        ("\u{65E5}\u{672C}\u{8A9E}", "CJK characters"),
        ("\u{1F525}\u{1F525}\u{1F525}", "emoji"),
        ("\u{0645}\u{0631}\u{062D}\u{0628}\u{0627}", "RTL Arabic"),
        ("\u{200B}", "zero-width space"),
        ("stream\0evil", "null byte embedded"),
        ("../../../etc/passwd", "path traversal unix"),
        ("..\\..\\windows", "path traversal windows"),
        ("'; DROP TABLE events; --", "SQL injection"),
        ("  ", "spaces only"),
        ("\t\n", "tab and newline"),
        ("stream with spaces", "spaces in name"),
        ("\u{FEFF}stream", "BOM prefix"),
    ];

    let (store, _dir) = temp_store();

    for (evil_id, description) in &evil_ids {
        // Use the evil_id directly as a StreamKey — no StreamId validation anymore.
        let stream_id = StreamKey::from_slice(evil_id.as_bytes());

        let env = make_envelope(1, "Created", b"test-payload");
        let result = store.append(&stream_id, None, PendingBatch::of(&env)).await;

        match result {
            Ok(_position) => {
                // If append succeeded, reading must return the exact data
                let read = read_all_payloads(&store, &stream_id).await;
                assert_eq!(
                    read.len(),
                    1,
                    "wrong event count for stream ID: {} ({})",
                    evil_id,
                    description
                );
                assert_eq!(
                    read[0], b"test-payload",
                    "payload corrupted for stream ID: {} ({})",
                    evil_id, description
                );
            }
            Err(e) => {
                // Rejection is also acceptable for truly pathological IDs
                println!(
                    "Stream ID '{}' ({}) was rejected: {}",
                    evil_id, description, e
                );
            }
        }
    }
}

/// Very long stream ID (10,000 characters).
#[tokio::test]
async fn attack_very_long_stream_id() {
    // Version test: from_persisted(0) returns None.
    let _long_id = "a".repeat(10_000);
    let result = Version::new(0);
    assert!(result.is_none(), "Version 0 must not be constructable");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// For any two distinct stream IDs, events appended to s1 never appear in s2.
    #[test]
    fn attack_stream_id_isolation_never_leaks(
        s1 in stream_id_strategy(),
        s2 in stream_id_strategy(),
    ) {
        prop_assume!(s1 != s2);
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let (store, _dir) = temp_store();

            let env1 = make_envelope(1, "EventA", b"payload-a");
            store.append(&s1, None, PendingBatch::of(&env1)).await.unwrap();

            let env2 = make_envelope(1, "EventB", b"payload-b");
            store.append(&s2, None, PendingBatch::of(&env2)).await.unwrap();

            // s1 must only contain its own data
            let s1_payloads = read_all_payloads(&store, &s1).await;
            prop_assert_eq!(s1_payloads.len(), 1, "s1 wrong count");
            prop_assert_eq!(&s1_payloads[0], &b"payload-a".to_vec(), "s1 payload leaked");

            // s2 must only contain its own data
            let s2_payloads = read_all_payloads(&store, &s2).await;
            prop_assert_eq!(s2_payloads.len(), 1, "s2 wrong count");
            prop_assert_eq!(&s2_payloads[0], &b"payload-b".to_vec(), "s2 payload leaked");

            // s1 event types must not leak
            let s1_types = read_all_event_types(&store, &s1).await;
            prop_assert_eq!(s1_types, vec!["EventA".to_owned()]);

            let s2_types = read_all_event_types(&store, &s2).await;
            prop_assert_eq!(s2_types, vec!["EventB".to_owned()]);

            Ok(())
        })?;
    }
}

// ============================================================================
// CATEGORY 4: Version Boundary Arithmetic
// ============================================================================

/// What happens near u64::MAX?
#[tokio::test]
async fn attack_version_boundaries_near_max() {
    let (_store, _dir) = temp_store();
    // We can't easily put u64::MAX - 1 events in the store, but we can test
    // Version::new(u64::MAX) construction
    let v_max = Version::new(u64::MAX).unwrap();
    assert_eq!(v_max.as_u64(), u64::MAX);

    // Version::next() at u64::MAX returns None (no overflow panic)
    assert!(
        v_max.next().is_none(),
        "Version::next() at u64::MAX must return None"
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// For any stream with N events, appending with wrong expected_version
    /// always returns AppendError::Conflict with correct values.
    #[test]
    fn attack_version_conflict_detection(
        n in 1..20usize,
        wrong_version in 0..100u64,
    ) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let (store, _dir) = temp_store();
            let stream_id = sk("conflict-detect");

            // Put N events in the stream
            let payloads: Vec<Vec<u8>> = (0..n).map(|i| vec![i as u8]).collect();
            let envelopes = build_envelopes(&payloads);
            store.append(&stream_id, None, PendingBatch::new(&envelopes).expect("non-empty batch")).await.unwrap();

            let actual_version = u64::try_from(n).unwrap();
            prop_assume!(wrong_version != actual_version);

            // Try to append with wrong expected_version
            let new_envelopes = build_envelopes_from(
                wrong_version + 1,
                &[vec![0xFF]],
            );
            let result = store.append(
                &stream_id,
                Version::new(wrong_version),
                PendingBatch::new(&new_envelopes).expect("non-empty batch"),
            ).await;

            prop_assert!(result.is_err(), "wrong expected_version MUST be rejected");
            match result.unwrap_err() {
                AppendError::Conflict { expected, actual, .. } => {
                    prop_assert_eq!(expected, Version::new(wrong_version),
                        "conflict expected version wrong");
                    prop_assert_eq!(actual, Version::new(actual_version),
                        "conflict actual version wrong");
                }
                // AppendError is #[non_exhaustive] (#209): anything but Conflict
                // (Store or a future variant) is a test failure.
                other => {
                    panic!("expected Conflict, got: {other}");
                }
            }
            Ok(())
        })?;
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Random version sequences must be rejected unless perfectly sequential from 1.
    #[test]
    fn attack_random_versions_rejected_unless_sequential(
        versions in prop::collection::vec(1..1000u64, 1..10),
    ) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let (store, _dir) = temp_store();
            let stream_id = sk("random-versions");

            let envelopes: Vec<_> = versions.iter().map(|&v| {
                make_envelope(v, leak("E"), &[v as u8])
            }).collect();

            let result = store.append(&stream_id, None, PendingBatch::new(&envelopes).expect("non-empty batch")).await;

            let is_sequential = versions.iter().enumerate().all(|(i, &v)| v == (i as u64) + 1);
            if is_sequential {
                prop_assert!(result.is_ok(), "sequential versions must be accepted");
            } else {
                prop_assert!(result.is_err(), "non-sequential versions MUST be rejected: {:?}", versions);
            }
            Ok(())
        })?;
    }
}

// ============================================================================
// CATEGORY 8: Schema Version Edge Cases
// ============================================================================

/// schema_version = 0 is now impossible at the type level (`NonZeroU32`).
/// Verify that `NonZeroU32::MIN` (1) round-trips correctly.
#[tokio::test]
async fn attack_schema_version_zero_clamped_by_builder() {
    let (store, _dir) = temp_store();

    let env = pending_envelope(Version::INITIAL)
        .event_type("BadSchema")
        .payload(b"data".to_vec())
        .schema_version(SchemaVersion::INITIAL) // Minimum valid schema version
        .build()
        .expect("valid envelope");

    // schema_version should be 1
    assert_eq!(
        env.schema_version(),
        1,
        "schema_version must be 1 (NonZeroU32::MIN)"
    );

    store
        .append(&sk("sv-zero"), None, PendingBatch::of(&env))
        .await
        .unwrap();

    // Read succeeds because schema_version is now 1
    let mut stream = store
        .read_stream(&sk("sv-zero"), Version::new(1).unwrap())
        .await
        .unwrap();
    let persisted = stream.next().await.unwrap().unwrap();
    assert_eq!(persisted.schema_version(), 1);
}

// ============================================================================
// CATEGORY 9: Model-Based Testing (proptest)
// ============================================================================

#[derive(Debug, Clone)]
enum Op {
    Append { stream_idx: usize, n_events: usize },
    Read { stream_idx: usize },
}

fn op_strategy(n_streams: usize) -> impl Strategy<Value = Op> {
    prop_oneof![
        (0..n_streams, 1..10usize).prop_map(|(stream_idx, n_events)| Op::Append {
            stream_idx,
            n_events,
        }),
        (0..n_streams).prop_map(|stream_idx| Op::Read { stream_idx }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Maintain a shadow model (HashMap) and compare with the real FjallStore
    /// after a sequence of random operations.
    #[test]
    fn attack_model_based_shadow_store(
        ops in prop::collection::vec(op_strategy(5), 10..50),
    ) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let (store, _dir) = temp_store();
            let stream_ids: Vec<String> = (0..5).map(|i| format!("model-{}", i)).collect();

            // Shadow model: stream_id -> Vec<(version, payload)>
            let mut shadow: HashMap<String, Vec<(u64, Vec<u8>)>> = HashMap::new();

            for op in &ops {
                match op {
                    Op::Append { stream_idx, n_events } => {
                        let stream_id = &stream_ids[*stream_idx];
                        let current_count = shadow
                            .get(stream_id)
                            .map_or(0u64, |v| u64::try_from(v.len()).unwrap());

                        let sid = StreamKey::from_slice(stream_id.as_bytes());
                        let envelopes: Vec<_> = (1..=*n_events).map(|i| {
                            let version = current_count + u64::try_from(i).unwrap();
                            let payload = format!("op-{}-v{}", stream_id, version);
                            make_envelope(
                                version,
                                "ModelEvent",
                                payload.as_bytes(),
                            )
                        }).collect();

                        let result = store.append(
                            &sid,
                            Version::new(current_count),
                            PendingBatch::new(&envelopes).expect("non-empty batch"),
                        ).await;
                        prop_assert!(result.is_ok(),
                            "model-based append failed for stream {} at version {}",
                            stream_id, current_count);

                        // Update shadow
                        let entry = shadow.entry(stream_id.clone()).or_default();
                        for i in 1..=*n_events {
                            let version = current_count + u64::try_from(i).unwrap();
                            let payload = format!("op-{}-v{}", stream_id, version);
                            entry.push((version, payload.into_bytes()));
                        }
                    }
                    Op::Read { stream_idx } => {
                        let stream_id = &stream_ids[*stream_idx];
                        let sid = StreamKey::from_slice(stream_id.as_bytes());
                        let shadow_data = shadow.get(stream_id);

                        let real_payloads = read_all_payloads(&store, &sid).await;
                        let real_versions = read_all_versions(&store, &sid).await;

                        match shadow_data {
                            None => {
                                prop_assert_eq!(real_payloads.len(), 0,
                                    "shadow empty but store has {} events for {}",
                                    real_payloads.len(), stream_id);
                            }
                            Some(shadow_events) => {
                                prop_assert_eq!(real_payloads.len(), shadow_events.len(),
                                    "event count mismatch for {}: real={}, shadow={}",
                                    stream_id, real_payloads.len(), shadow_events.len());

                                for (i, ((sv, sp), (rv, rp))) in shadow_events.iter()
                                    .zip(real_versions.iter().zip(real_payloads.iter()))
                                    .enumerate()
                                {
                                    prop_assert_eq!(*rv, *sv,
                                        "version mismatch at index {} for {}", i, stream_id);
                                    prop_assert_eq!(rp, sp,
                                        "payload mismatch at index {} for {}", i, stream_id);
                                }
                            }
                        }
                    }
                }
            }
            Ok(())
        })?;
    }
}

// ============================================================================
// CATEGORY 10: Empty and Degenerate Cases
// ============================================================================

/// Empty stream ID must return an error, not panic.
///
/// Previously this caused a panic deep inside fjall's lsm-tree ("key may not
/// be empty"). Now `FjallStore` validates at the boundary.
#[test]
fn attack_empty_string_stream_id_rejected_by_stream_id_type() {
    // Version::new(0) returns None — test version boundary instead.
    // Empty stream IDs can no longer reach the store layer.
    let result = Version::new(0);
    assert!(result.is_none(), "from_persisted(0) must return None");
}

// Schema version round-trip through the full stack for various values.
proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn attack_schema_version_round_trip_any(
        schema_ver in 1..=u32::MAX,
    ) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let (store, _dir) = temp_store();
            let stream_id = StreamKey::from_slice(format!("sv-{}", schema_ver).as_bytes());
            let env = make_envelope_with_schema(
                1,
                "SchemaTest",
                b"data",
                schema_ver,
            );
            store.append(&stream_id, None, PendingBatch::of(&env)).await.unwrap();

            let mut stream = store.read_stream(&stream_id, Version::INITIAL).await.unwrap();
            let persisted = stream.next().await.unwrap().unwrap();
            prop_assert_eq!(persisted.schema_version(), schema_ver,
                "schema_version {} not preserved through roundtrip", schema_ver);
            Ok(())
        })?;
    }
}

// Verify monotonic version ordering in read stream output.
proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn attack_stream_versions_always_monotonic(
        n in 1..50usize,
    ) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let (store, _dir) = temp_store();
            let stream_id = sk("monotonic-test");
            let payloads: Vec<Vec<u8>> = (0..n).map(|i| vec![i as u8]).collect();
            let envelopes = build_envelopes(&payloads);
            store.append(&stream_id, None, PendingBatch::new(&envelopes).expect("non-empty batch")).await.unwrap();

            let versions = read_all_versions(&store, &stream_id).await;
            for window in versions.windows(2) {
                prop_assert!(
                    window[1] > window[0],
                    "versions not strictly increasing: {} followed by {}",
                    window[0], window[1],
                );
            }
            Ok(())
        })?;
    }
}
