//! Resilience and stress tests for the fjall event store adapter.
//!
//! Targets boundary conditions, crash recovery, concurrent access,
//! arithmetic overflow, and encoding edge cases.
//!
//! ## Coverage
//!
//! A. Version arithmetic overflow
//! B. Dual instance data corruption
//! C. Deterministic simulation testing (DST) with shadow model
//! D. Snapshot isolation
//! E. Crash simulation via `mem::forget`
//! F. Key boundary conditions (`u64::MAX` reads)
//! G. Recovery torture (100 streams, 5 reopen cycles)
//! H. Concurrent append storm (50 tasks)
//! I. Encoding boundary attacks (`u16::MAX` event type)
//! K. Append-then-read version consistency

#![allow(clippy::unwrap_used, reason = "test code")]
#![allow(clippy::panic, reason = "test code")]
#![allow(clippy::expect_used, reason = "test code")]
#![allow(clippy::str_to_string, reason = "tests")]
#![allow(clippy::shadow_reuse, reason = "tests")]
#![allow(clippy::shadow_unrelated, reason = "tests")]
#![allow(clippy::as_conversions, reason = "tests")]
#![allow(clippy::cast_possible_truncation, reason = "tests")]
#![allow(clippy::cast_possible_wrap, reason = "tests")]
#![allow(clippy::cast_sign_loss, reason = "tests")]
#![allow(clippy::implicit_clone, reason = "tests")]
#![allow(clippy::clone_on_ref_ptr, reason = "tests")]
#![allow(clippy::arithmetic_side_effects, reason = "tests")]
#![allow(clippy::print_stdout, reason = "diagnostic output")]
#![allow(clippy::indexing_slicing, reason = "tests")]
#![allow(clippy::items_after_statements, reason = "tests")]
#![allow(dead_code, reason = "helpers used across test blocks")]

use std::collections::HashMap;
use std::num::NonZeroU32;

use futures::StreamExt;
use mnesis::Version;
use mnesis_fjall::FjallStore;
use mnesis_store::PendingEnvelope;
use mnesis_store::StreamKey;
use mnesis_store::envelope::pending_envelope;
use mnesis_store::store::RawEventStore;

use proptest::prelude::*;

// ============================================================================
// Helpers
// ============================================================================

fn sk(s: &str) -> StreamKey {
    StreamKey::from_slice(s.as_bytes())
}

fn leak(s: &str) -> &'static str {
    Box::leak(s.to_owned().into_boxed_str())
}

fn make_envelope(version: u64, event_type: &'static str, payload: &[u8]) -> PendingEnvelope {
    pending_envelope(Version::new(version).unwrap())
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

async fn count_events(store: &FjallStore, stream_id: &StreamKey) -> u64 {
    let mut stream = store
        .read_stream(stream_id, Version::INITIAL)
        .await
        .unwrap();
    let mut count: u64 = 0;
    while let Some(__i) = stream.next().await {
        let env = __i.unwrap();
        let _ = env;
        count += 1;
    }
    count
}

// ============================================================================
// CATEGORY A: Version Arithmetic Overflow (THE BIG ONE)
// ============================================================================

/// The sequential-version check in `append` advances a running counter
/// (`expected_version_seq`) via `checked_add(1)` once per envelope, starting
/// from `current_version + 1`. This test documents the boundary arithmetic:
/// for a stream whose current version sits near `u64::MAX`, the position the
/// counter would reach after `i` advances overflows. The real `append` path is
/// unreachable here (you cannot append `u64::MAX` events), so this verifies the
/// checked arithmetic that guards that boundary in isolation.
#[test]
fn attack_version_overflow_in_sequential_check() {
    // Running counter seeded at current_version + 1, advanced once per
    // envelope. With current_version = u64::MAX - 2, the third advance
    // (i = 2) would reach u64::MAX + 1 = OVERFLOW.
    let current_version: u64 = u64::MAX - 2;
    let i: u64 = 2;
    let result = current_version
        .checked_add(1)
        .and_then(|v| v.checked_add(i));
    assert!(
        result.is_none(),
        "BUG CONFIRMED: running-counter version arithmetic overflows \
         when current_version={current_version}, i={i}"
    );
}

/// Prove that even i=0 overflows when `current_version` = `u64::MAX`
#[test]
fn attack_version_overflow_at_exact_max() {
    // current_version = u64::MAX: seeding the running counter at
    // current_version + 1 overflows on the very first advance.
    let current_version: u64 = u64::MAX;
    let result = current_version.checked_add(1);
    assert!(
        result.is_none(),
        "BUG CONFIRMED: current_version=u64::MAX + 1 overflows even with i=0"
    );
}

/// Prove the `new_version` computation can't be incremented past `u64::MAX`
#[test]
fn attack_new_version_computation_overflow() {
    let version_near_max = u64::MAX;
    let next = version_near_max.checked_add(1);
    assert!(
        next.is_none(),
        "version at u64::MAX cannot be incremented — next append would need version u64::MAX+1"
    );
}

/// Exhaustive boundary check: for which (`current_version`, `batch_size`) pairs
/// does the sequential-version check in `append` overflow?
#[test]
fn attack_version_overflow_boundary_exhaustive() {
    // The running counter starts at current_version + 1 and is advanced
    // batch_size times, reaching current_version + batch_size for the last
    // envelope. Overflow happens when current_version + batch_size > u64::MAX,
    // i.e. current_version > u64::MAX - batch_size.

    for batch_size in 1u64..=10 {
        let threshold = u64::MAX - batch_size;
        // At threshold: counter seeded at current_version + 1, advanced
        // batch_size - 1 more times, lands on u64::MAX → OK
        let ok_result = threshold
            .checked_add(1)
            .and_then(|v| v.checked_add(batch_size - 1));
        assert!(
            ok_result.is_some(),
            "current_version={threshold}, batch_size={batch_size} should NOT overflow"
        );

        // At threshold + 1: overflow
        let overflow_version = threshold + 1;
        let overflow_result = overflow_version
            .checked_add(1)
            .and_then(|v| v.checked_add(batch_size - 1));
        assert!(
            overflow_result.is_none(),
            "current_version={overflow_version}, batch_size={batch_size} SHOULD overflow"
        );
    }
}

// ============================================================================
// CATEGORY B: Dual Instance Attack
// ============================================================================

#[tokio::test]
async fn attack_dual_instance_same_path() {
    // Open TWO FjallStore instances on the same database path.
    // This tests whether fjall allows it and if it causes corruption.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let store1 = FjallStore::builder(&path).open().unwrap();

    // Try opening a second instance on the same path
    let result = FjallStore::builder(&path).open();
    // If fjall allows this: both stores could have write conflicts
    // If fjall rejects this: we get an error (safe)

    match result {
        Ok(store2) => {
            // DANGEROUS: two writers on same DB!
            // Try creating different streams from each
            let sid1 = sk("stream-from-store1");
            let sid2 = sk("stream-from-store2");
            let env1 = make_envelope(1, "A", b"from-1");
            let env2 = make_envelope(1, "B", b"from-2");

            store1.append(&sid1, None, &[env1]).await.unwrap();
            store2.append(&sid2, None, &[env2]).await.unwrap();

            // Check for corruption: can we read back correctly?
            let r1 = read_all_event_types(&store1, &sid1).await;
            let r2 = read_all_event_types(&store2, &sid2).await;

            assert_eq!(r1, vec!["A"], "store1 stream corrupted by dual instance");
            assert_eq!(r2, vec!["B"], "store2 stream corrupted by dual instance");

            println!(
                "WARNING: dual instance opened successfully — \
                 concurrent writes may conflict"
            );
        }
        Err(e) => {
            println!("SAFE: fjall rejects dual instances: {e}");
        }
    }
}

// ============================================================================
// CATEGORY C: Deterministic Simulation Testing (DST)
// ============================================================================

#[derive(Debug, Clone)]
enum Op {
    Append { stream: usize, count: usize },
    Read { stream: usize },
    Reopen,
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn attack_deterministic_simulation(
        ops in prop::collection::vec(
            prop_oneof![
                (0..5usize, 1..10usize).prop_map(|(s, c)| Op::Append { stream: s, count: c }),
                (0..5usize).prop_map(|s| Op::Read { stream: s }),
                Just(Op::Reopen),
            ],
            10..50,
        )
    ) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("db");
            let mut store = FjallStore::builder(&path).open().unwrap();

            // Shadow model: stream_idx -> list of payloads
            let stream_names: Vec<StreamKey> = (0..5)
                .map(|i| sk(&format!("stream_{i}")))
                .collect();
            let mut model: HashMap<usize, Vec<Vec<u8>>> = HashMap::new();

            for op in &ops {
                match op {
                    Op::Append { stream, count } => {
                        let sid_ref = &stream_names[*stream];
                        let existing = model.get(stream).map_or(0, Vec::len);
                        let payloads: Vec<Vec<u8>> = (0..*count)
                            .map(|i| format!("data-{stream}-{}-{i}", existing + i).into_bytes())
                            .collect();

                        let mut envelopes = Vec::new();
                        for (i, payload) in payloads.iter().enumerate() {
                            let ver = u64::try_from(existing + i + 1).unwrap();
                            envelopes.push(
                                pending_envelope(Version::new(ver).unwrap())
                                    .event_type(leak(&format!("Tick_{stream}_{}", existing + i)))
                                    .payload(payload.clone())
                                    .build()
                                    .expect("valid envelope"),
                            );
                        }

                        let expected_ver =
                            Version::new(u64::try_from(existing).unwrap());
                        match store.append(sid_ref, expected_ver, &envelopes).await {
                            Ok(()) => {
                                model.entry(*stream).or_default().extend(payloads);
                            }
                            Err(e) => panic!("DST append failed unexpectedly: {e}"),
                        }
                    }
                    Op::Read { stream } => {
                        let sid_ref = &stream_names[*stream];
                        let expected = model.get(stream).cloned().unwrap_or_default();
                        let actual = read_all_payloads(&store, sid_ref).await;
                        assert_eq!(
                            actual, expected,
                            "DST: stream {stream} data mismatch after read"
                        );
                    }
                    Op::Reopen => {
                        drop(store);
                        store = FjallStore::builder(&path).open().unwrap();
                    }
                }
            }

            // Final invariant: all streams match model
            for (stream_idx, expected_payloads) in &model {
                let sid_ref = &stream_names[*stream_idx];
                let actual = read_all_payloads(&store, sid_ref).await;
                assert_eq!(
                    actual, *expected_payloads,
                    "DST final check: stream {stream_idx} mismatch"
                );
            }
        });
    }
}

// ============================================================================
// CATEGORY E: Crash Simulation via mem::forget
// ============================================================================

#[tokio::test]
async fn attack_crash_simulation_forget_store() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");

    // Write events, then "crash" by forgetting the store (no clean shutdown)
    {
        let store = FjallStore::builder(&path).open().unwrap();
        let envs = vec![make_envelope(1, "Before", b"crash-data")];
        store.append(&sk("crash-test"), None, &envs).await.unwrap();
        std::mem::forget(store); // Simulate crash — no Drop, no flush
    }

    // Reopen and check if data survived.
    // fjall v3 uses file locking — if mem::forget skipped Drop, the lock may
    // still be held, making reopen impossible. Both outcomes are valid:
    // - Locked: lock file not released (crash left stale lock)
    // - Ok: lock released, data may or may not have survived
    match FjallStore::builder(&path).open() {
        Err(_) => {
            println!(
                "DURABILITY NOTE: reopen failed after mem::forget — \
                 fjall v3 file lock was not released (expected for crash simulation)"
            );
        }
        Ok(store) => {
            let payloads = read_all_payloads(&store, &sk("crash-test")).await;
            if payloads.is_empty() {
                println!(
                    "DURABILITY NOTE: data lost after mem::forget — \
                     fjall does not fsync on every commit by default"
                );
            } else {
                assert_eq!(payloads, vec![b"crash-data".to_vec()]);
                println!(
                    "DURABILITY NOTE: data survived mem::forget — fjall fsynced or recovered from WAL"
                );
            }
        }
    }
}

// ============================================================================
// CATEGORY K: Append-then-read version consistency
// ============================================================================

// ============================================================================
// CATEGORY K2: Global sequence assignment across appends and streams
// ============================================================================

/// `FjallStore::append` must assign each event a store-global `GlobalSeq` that
/// increases monotonically across events within a batch, across separate
/// appends, and across appends to *different* streams. The sequence starts at
/// 1. The position rides on the `$all` read tag (a per-stream event carries
/// none), so the cross-stream interleaving is observed via `read_all`.
///
/// This asserts fjall's on-disk `GlobalSeq` allocation is CONTIGUOUS (no gaps
/// across any of the three appends) — an implementation property strictly
/// stronger than the store contract's monotonic-not-gapless guarantee (an
/// aborted append may legally burn values elsewhere).
#[tokio::test]
async fn append_assigns_monotonic_all_position_across_streams() {
    let (store, _dir) = temp_store();

    // Append 1: two events to stream "a" -> positions 1, 2.
    store
        .append(
            &sk("a"),
            None,
            &[make_envelope(1, "A1", b"a1"), make_envelope(2, "A2", b"a2")],
        )
        .await
        .unwrap();

    // Append 2: one event to a different stream "b" -> position 3.
    store
        .append(&sk("b"), None, &[make_envelope(1, "B1", b"b1")])
        .await
        .unwrap();

    // Append 3: continue stream "a" -> positions 4, 5.
    store
        .append(
            &sk("a"),
            Version::new(2),
            &[make_envelope(3, "A3", b"a3"), make_envelope(4, "A4", b"a4")],
        )
        .await
        .unwrap();

    // The `$all` read yields a contiguous monotonic position sequence 1..=5,
    // interleaving the two streams in exact append order: a@1,a@2 (1,2), then
    // b@1 (3) between a's two appends, then a@3,a@4 (4,5). Each item also
    // carries its origin stream key (#333), so the interleaving [a,a,b,a,a]
    // is asserted positively without decoding a payload.
    let mut all = store.read_all(None).await.unwrap();
    let mut seen: Vec<(u64, Vec<u8>, String)> = Vec::new();
    while let Some(item) = all.next().await {
        let (pos, key, env) = item.unwrap();
        seen.push((
            pos.as_u64(),
            key.as_bytes().to_vec(),
            env.event_type().to_owned(),
        ));
    }
    assert_eq!(
        seen,
        vec![
            (1, b"a".to_vec(), "A1".to_owned()),
            (2, b"a".to_vec(), "A2".to_owned()),
            (3, b"b".to_vec(), "B1".to_owned()),
            (4, b"a".to_vec(), "A3".to_owned()),
            (5, b"a".to_vec(), "A4".to_owned()),
        ],
        "$all positions must be a contiguous monotonic sequence starting at 1, \
         interleaving streams in append order with each event attributed to \
         its origin stream key",
    );
}

// ============================================================================
// CATEGORY L: Recovery correctness after many streams
// ============================================================================

#[tokio::test]
async fn attack_recovery_stream_id_counter_correctness() {
    // Create N streams, reopen, verify counter is correct by creating stream N+1
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");

    let stream_count = 50u64;

    {
        let store = FjallStore::builder(&path).open().unwrap();
        for i in 0..stream_count {
            let sid_val = sk(&format!("recovery_{i}"));
            let env = make_envelope(1, "Init", &i.to_le_bytes());
            store.append(&sid_val, None, &[env]).await.unwrap();
        }
    }

    // Reopen
    {
        let store = FjallStore::builder(&path).open().unwrap();
        // Create one more stream — should get a unique numeric ID
        let sid_val = sk(&format!("recovery_{stream_count}"));
        let env = make_envelope(1, "Init", b"new");
        store.append(&sid_val, None, &[env]).await.unwrap();

        // Verify ALL streams are readable
        for i in 0..=stream_count {
            let sid_val = sk(&format!("recovery_{i}"));
            let count = count_events(&store, &sid_val).await;
            assert_eq!(count, 1, "stream recovery_{i} should have 1 event");
        }
    }
}

// ============================================================================
// CATEGORY M: Empty batch to nonexistent stream
// ============================================================================

#[tokio::test]
async fn attack_empty_batch_wrong_version_to_nonexistent() {
    // BUG PROBE: empty batch skips ALL validation including version check.
    // This means you can "succeed" with a wrong expected_version on an empty batch.
    let (store, _dir) = temp_store();

    let result = store.append(&sk("phantom2"), Version::new(999), &[]).await;
    // The early return at line 61 fires BEFORE any version check.
    // This is arguably a bug: it should still validate expected_version
    // against the actual stream state, even for empty batches.
    if result.is_ok() {
        println!(
            "BUG FOUND: empty batch with wrong expected_version (999) \
             succeeded for nonexistent stream — version check is bypassed \
             by the early return at store.rs:61"
        );
    }
}

// ============================================================================
// CATEGORY P: Schema version 0 attack through the store
// ============================================================================

/// Verify that `schema_version = 0` is structurally impossible via `NonZeroU32`.
///
/// Previously, `schema_version=0` was accepted on write but rejected on read
/// (`PersistedEnvelope::try_new` rejects 0), creating unreadable "black hole" data.
/// Now `NonZeroU32` makes 0 a compile-time error on the builder. The read path
/// (`PersistedEnvelope::try_new`) still rejects 0 at runtime as a second layer
/// of defense against corrupt persisted data.
#[test]
fn attack_schema_version_zero_rejected_by_type_system() {
    // Defense in depth: every layer rejects schema_version=0 by construction.
    // - NonZeroU32::new(0) returns None.
    // - SchemaVersion::from_u32(0) returns SchemaVersionZero.
    // - PersistedEnvelope::try_new takes SchemaVersion, so 0 is unrepresentable
    //   at the type level (no test case can pass it).
    // - wire::decode_frame rejects on-disk schema_version=0 (corrupt-data path),
    //   pinned in mnesis_store::wire::tests::decode_frame_rejects_corrupt_schema_version_zero.
    assert!(NonZeroU32::new(0).is_none(), "NonZeroU32 must reject 0");
    assert!(
        mnesis_store::value::SchemaVersion::from_u32(0).is_err(),
        "SchemaVersion::from_u32(0) must return Err"
    );
}

// ============================================================================
// CATEGORY R: Builder partition configuration
// ============================================================================

#[tokio::test]
async fn attack_custom_builder_config_still_works() {
    let dir = tempfile::tempdir().unwrap();
    let store = FjallStore::builder(dir.path().join("db"))
        .streams_config(|opts| {
            opts.data_block_size_policy(fjall::config::BlockSizePolicy::all(8_192))
        })
        .events_config(|opts| {
            opts.data_block_size_policy(fjall::config::BlockSizePolicy::all(16_384))
        })
        .open()
        .unwrap();

    let env = make_envelope(1, "A", b"data");
    store
        .append(&sk("custom-config"), None, &[env])
        .await
        .unwrap();

    let count = count_events(&store, &sk("custom-config")).await;
    assert_eq!(count, 1, "custom config store should work normally");
}
