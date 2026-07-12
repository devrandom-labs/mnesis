//! Relocated inline test mod of `src/import.rs` (mnesis-inmemory is a
//! dev-dependency; type unification with it requires an integration test).
//! The store-free planner tests over the private `plan_section`/`SectionPlan`
//! stayed inline as `import::plan_tests`.

#![cfg(feature = "import")]

use mnesis::Version;
use thiserror::Error;

use mnesis_store::envelope::PersistedEnvelope;
use mnesis_store::error::AppendError;
use mnesis_store::store::{RawEventStore, Store};
use mnesis_store::stream_id::StreamKey;

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code asserts exact values"
)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use futures::StreamExt;
    use mnesis_store::envelope::{PendingEnvelope, pending_envelope};
    use mnesis_store::export::EventExporter;
    use mnesis_store::import::*;
    use mnesis_store::value::SchemaVersion;
    use proptest::prelude::*;
    use static_assertions::assert_impl_all;
    use std::error::Error as _;
    use std::sync::Arc;
    use tokio::sync::Barrier;

    fn v(n: u64) -> Version {
        Version::new(n).expect("test version must be nonzero")
    }

    fn persisted(version: u64, payload: &[u8]) -> PersistedEnvelope {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"E");
        buf.extend_from_slice(payload);
        let et_end = 1u32;
        let pl_end = et_end + u32::try_from(payload.len()).expect("payload fits u32");
        PersistedEnvelope::try_new(
            v(version),
            Bytes::from(buf),
            SchemaVersion::INITIAL,
            0..et_end,
            et_end..pl_end,
            None,
        )
        .expect("valid persisted envelope")
    }

    fn evt(version: u64) -> ImportBlock {
        ImportBlock::Event(persisted(version, b"p"))
    }

    fn section(origin: &str, blocks: Vec<ImportBlock>) -> StreamSection {
        StreamSection {
            origin: Bytes::copy_from_slice(origin.as_bytes()),
            blocks,
        }
    }

    fn report(stream: &str, outcome: StreamOutcome) -> StreamReport {
        StreamReport {
            stream: StreamKey::from_slice(stream.as_bytes()),
            outcome,
        }
    }

    // ── Send + Sync: report/outcome/error types travel between tasks ─────────

    assert_impl_all!(StreamOutcome: Send, Sync, Copy, Clone);
    assert_impl_all!(Atomicity: Send, Sync, Copy, Clone);
    assert_impl_all!(AbortReason: Send, Sync, Clone);
    assert_impl_all!(StreamReport: Send, Sync, Clone);
    assert_impl_all!(ImportReport: Send, Sync, Clone);
    assert_impl_all!(ImportError<std::io::Error>: Send, Sync);

    // ── Atomicity: equality + Copy semantics ────────────────────────────────

    #[test]
    fn atomicity_equality_and_copy_semantics() {
        let policy = Atomicity::WholeChunk;
        let copied = policy; // `Copy`: `policy` is still usable below.
        assert_eq!(policy, copied);
        assert_eq!(Atomicity::WholeChunk, Atomicity::WholeChunk);
        assert_eq!(Atomicity::PerStream, Atomicity::PerStream);
        assert_ne!(Atomicity::WholeChunk, Atomicity::PerStream);
    }

    // ── StreamOutcome::is_complete / reached — all variants ─────────────────

    #[test]
    fn complete_is_complete_and_reaches_its_version() {
        let outcome = StreamOutcome::Complete { version: v(5) };
        assert!(outcome.is_complete());
        assert_eq!(outcome.reached(), Some(v(5)));
    }

    #[test]
    fn corrupt_untouched_is_not_complete_and_reaches_none() {
        let outcome = StreamOutcome::Corrupt { reached: None };
        assert!(!outcome.is_complete());
        assert_eq!(outcome.reached(), None);
    }

    #[test]
    fn corrupt_with_prefix_reaches_that_prefix() {
        let outcome = StreamOutcome::Corrupt {
            reached: Some(v(3)),
        };
        assert!(!outcome.is_complete());
        assert_eq!(outcome.reached(), Some(v(3)));
    }

    #[test]
    fn mismatch_untouched_reaches_none_not_got() {
        // reached and got are distinct values: a bug returning `got` would
        // make this fail.
        let outcome = StreamOutcome::Mismatch {
            reached: None,
            got: v(8),
        };
        assert!(!outcome.is_complete());
        assert_eq!(outcome.reached(), None);
    }

    #[test]
    fn mismatch_with_prefix_reaches_prefix_not_got() {
        let outcome = StreamOutcome::Mismatch {
            reached: Some(v(3)),
            got: v(8),
        };
        assert!(!outcome.is_complete());
        // Must be the prefix (3), never the offending version (8).
        assert_eq!(outcome.reached(), Some(v(3)));
    }

    // ── ImportReport / StreamReport: new / streams / unfinished / all_complete

    #[test]
    fn empty_report_is_vacuously_all_complete_with_no_unfinished() {
        let report: ImportReport = ImportReport::new(Vec::new());
        assert!(report.streams().is_empty());
        assert!(report.all_complete());
        assert_eq!(report.unfinished().count(), 0);
    }

    #[test]
    fn all_complete_report_reports_no_unfinished() {
        let streams = vec![
            report("a", StreamOutcome::Complete { version: v(1) }),
            report("b", StreamOutcome::Complete { version: v(2) }),
        ];
        let report = ImportReport::new(streams.clone());
        assert_eq!(report.streams(), streams.as_slice());
        assert!(report.all_complete());
        assert_eq!(report.unfinished().count(), 0);
    }

    #[test]
    fn mixed_report_filters_exactly_the_non_complete_streams() {
        let streams = vec![
            report("done-1", StreamOutcome::Complete { version: v(1) }),
            report(
                "corrupt",
                StreamOutcome::Corrupt {
                    reached: Some(v(2)),
                },
            ),
            report("done-2", StreamOutcome::Complete { version: v(3) }),
            report(
                "mismatch",
                StreamOutcome::Mismatch {
                    reached: None,
                    got: v(9),
                },
            ),
        ];
        let report = ImportReport::new(streams);

        // streams() preserves first-seen order and full set.
        let all_ids: Vec<&[u8]> = report
            .streams()
            .iter()
            .map(|s| s.stream.as_bytes())
            .collect();
        assert_eq!(
            all_ids,
            [
                b"done-1".as_ref(),
                b"corrupt".as_ref(),
                b"done-2".as_ref(),
                b"mismatch".as_ref()
            ]
        );

        // unfinished() is EXACTLY the two non-Complete streams, in order.
        let unfinished_ids: Vec<&[u8]> = report.unfinished().map(|s| s.stream.as_bytes()).collect();
        assert_eq!(unfinished_ids, [b"corrupt".as_ref(), b"mismatch".as_ref()]);

        assert!(!report.all_complete());
    }

    #[test]
    fn single_incomplete_stream_blocks_all_complete() {
        let report = ImportReport::new(vec![report(
            "only",
            StreamOutcome::Corrupt { reached: None },
        )]);
        assert!(!report.all_complete());
        assert_eq!(report.unfinished().count(), 1);
    }

    // ── AbortReason / ImportError Display + source ──────────────────────────

    #[test]
    fn abort_reason_display_strings_are_exact() {
        assert_eq!(AbortReason::Corrupt.to_string(), "block failed checksum");
        assert_eq!(
            AbortReason::Mismatch {
                expected: v(4),
                got: v(7),
            }
            .to_string(),
            "version mismatch (expected 4, got 7)",
        );
    }

    #[test]
    fn import_error_malformed_display_is_exact() {
        let err: ImportError<std::io::Error> = ImportError::Malformed("bad magic");
        assert_eq!(err.to_string(), "malformed chunk: bad magic");
    }

    #[test]
    fn import_error_aborted_display_formats_stream_and_reason() {
        let err: ImportError<std::io::Error> = ImportError::Aborted {
            stream: StreamKey::from_slice(b"phone:task-123"),
            reason: AbortReason::Mismatch {
                expected: v(2),
                got: v(5),
            },
        };
        // {stream} → StreamKey's Display (the UTF-8 string, unquoted); reason
        // via its own Display.
        assert_eq!(
            err.to_string(),
            "chunk aborted at stream phone:task-123: version mismatch (expected 2, got 5)",
        );
    }

    #[test]
    fn import_error_version_overflow_display_is_exact() {
        let err: ImportError<std::io::Error> = ImportError::VersionOverflow;
        assert_eq!(err.to_string(), "version overflow");
    }

    #[test]
    fn import_error_store_is_transparent_forwarding_display_and_source() {
        // A store error WITH its own source, to prove transparent forwarding
        // reaches through to the inner error's source (not just Display).
        #[derive(Debug, Error)]
        #[error("root cause")]
        struct RootCause;

        #[derive(Debug, Error)]
        #[error("store failed")]
        struct DummyStore(#[source] RootCause);

        let err: ImportError<DummyStore> = ImportError::Store(DummyStore(RootCause));

        // transparent → Display equals the inner store error's Display.
        assert_eq!(err.to_string(), "store failed");
        // transparent → source() forwards to the inner error's own source.
        let source = err
            .source()
            .expect("transparent Store must forward a source");
        assert_eq!(source.to_string(), "root cause");
    }

    #[test]
    fn non_store_variants_expose_no_error_source() {
        // Error-chain shape pin: `Store` is the ONLY source-forwarding variant.
        // `Aborted` deliberately interpolates its `reason` into the message
        // (not `#[source]`), so it terminates the chain too.
        let malformed: ImportError<std::io::Error> = ImportError::Malformed("x");
        assert!(malformed.source().is_none());

        let overflow: ImportError<std::io::Error> = ImportError::VersionOverflow;
        assert!(overflow.source().is_none());

        let aborted: ImportError<std::io::Error> = ImportError::Aborted {
            stream: StreamKey::from_slice(b"s"),
            reason: AbortReason::Corrupt,
        };
        assert!(aborted.source().is_none());
    }

    // ── AtomicAppend helpers and tests ───────────────────────────────────────

    fn sk(s: &str) -> StreamKey {
        StreamKey::from_slice(s.as_bytes())
    }

    fn pending(ver: u64, payload: &[u8]) -> PendingEnvelope {
        pending_envelope(v(ver))
            .event_type("E")
            .payload(Bytes::copy_from_slice(payload))
            .build()
            .expect("valid envelope")
    }

    fn planned(target: &str, expected: Option<u64>, versions: &[u64]) -> PlannedAppend {
        PlannedAppend {
            target: sk(target),
            expected_version: expected.and_then(Version::new),
            events: versions.iter().map(|n| pending(*n, b"p")).collect(),
        }
    }

    async fn head_len(store: &mnesis_inmemory::InMemoryStore, id: &StreamKey) -> usize {
        store
            .read_stream(id, Version::INITIAL)
            .await
            .expect("read opens")
            .filter_map(|r| async move { r.ok() })
            .count()
            .await
    }

    #[tokio::test]
    async fn atomic_append_many_commits_all_writes() {
        let store = mnesis_inmemory::InMemoryStore::new();
        let writes = vec![planned("a", None, &[1, 2]), planned("b", None, &[1])];
        store.atomic_append_many(&writes).await.expect("commits");
        assert_eq!(head_len(&store, &sk("a")).await, 2);
        assert_eq!(head_len(&store, &sk("b")).await, 1);
    }

    #[tokio::test]
    async fn atomic_append_many_rolls_back_all_on_one_conflict() {
        let store = mnesis_inmemory::InMemoryStore::new();
        // Pre-seed "b" to v1 so the second write (expecting fresh) conflicts.
        store
            .append(&sk("b"), None, &[pending(1, b"seed")])
            .await
            .expect("seed");

        let writes = vec![
            planned("a", None, &[1, 2]), // would be fine alone
            planned("b", None, &[1]),    // conflicts: "b" is already at v1
        ];
        let err = store
            .atomic_append_many(&writes)
            .await
            .expect_err("must conflict");
        match err {
            AtomicAppendError::Conflict { index, actual } => {
                assert_eq!(index, 1);
                assert_eq!(actual, Version::new(1));
            }
            other => panic!("expected Conflict, got {other:?}"),
        }
        // Atomicity: NOTHING landed — "a" must still be empty.
        assert_eq!(head_len(&store, &sk("a")).await, 0, "rolled back");
        assert_eq!(head_len(&store, &sk("b")).await, 1, "unchanged");
    }

    fn identity_route(origin: &[u8]) -> StreamKey {
        StreamKey::from_slice(origin)
    }

    async fn versions(store: &mnesis_inmemory::InMemoryStore, id: &StreamKey) -> Vec<u64> {
        store
            .read_stream(id, Version::INITIAL)
            .await
            .expect("read opens")
            .map(|r| r.expect("no read error").version().as_u64())
            .collect()
            .await
    }

    #[tokio::test]
    async fn per_stream_clean_multi_stream_all_complete() {
        let store = mnesis_inmemory::InMemoryStore::new();
        let sections = vec![
            section("a", vec![evt(1), evt(2), evt(3)]),
            section("b", vec![evt(1), evt(2)]),
        ];
        let report = store
            .import(&sections, identity_route, Atomicity::PerStream)
            .await
            .expect("import ok");

        assert!(report.all_complete());
        assert_eq!(report.streams().len(), 2);
        assert_eq!(versions(&store, &sk("a")).await, vec![1, 2, 3]);
        assert_eq!(versions(&store, &sk("b")).await, vec![1, 2]);
        assert_eq!(
            report.streams()[0].outcome,
            StreamOutcome::Complete { version: v(3) }
        );
    }

    #[tokio::test]
    async fn per_stream_partial_overlap_is_picky_rejected() {
        let store = mnesis_inmemory::InMemoryStore::new();
        for n in 1..=3 {
            store
                .append(&sk("a"), Version::new(n - 1), &[pending(n, b"seed")])
                .await
                .expect("seed");
        }
        let sections = vec![section("a", vec![evt(2), evt(3), evt(4), evt(5)])];
        let report = store
            .import(&sections, identity_route, Atomicity::PerStream)
            .await
            .expect("import ok");

        assert_eq!(
            report.streams()[0].outcome,
            StreamOutcome::Mismatch {
                reached: None,
                got: v(2)
            }
        );
        assert_eq!(versions(&store, &sk("a")).await, vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn per_stream_internal_gap_applies_prefix_then_halts() {
        let store = mnesis_inmemory::InMemoryStore::new();
        let sections = vec![section("a", vec![evt(1), evt(2), evt(4)])];
        let report = store
            .import(&sections, identity_route, Atomicity::PerStream)
            .await
            .expect("import ok");

        assert_eq!(
            report.streams()[0].outcome,
            StreamOutcome::Mismatch {
                reached: Some(v(2)),
                got: v(4)
            }
        );
        assert_eq!(versions(&store, &sk("a")).await, vec![1, 2], "v4 held back");
    }

    #[tokio::test]
    async fn per_stream_mid_section_corrupt_applies_prefix_then_corrupt() {
        let store = mnesis_inmemory::InMemoryStore::new();
        let sections = vec![section(
            "a",
            vec![evt(1), evt(2), ImportBlock::Corrupt, evt(3)],
        )];
        let report = store
            .import(&sections, identity_route, Atomicity::PerStream)
            .await
            .expect("import ok");

        assert_eq!(
            report.streams()[0].outcome,
            StreamOutcome::Corrupt {
                reached: Some(v(2))
            }
        );
        assert_eq!(versions(&store, &sk("a")).await, vec![1, 2]);
    }

    #[tokio::test]
    async fn per_stream_first_block_corrupt_reaches_none() {
        let store = mnesis_inmemory::InMemoryStore::new();
        let sections = vec![section("a", vec![ImportBlock::Corrupt, evt(1)])];
        let report = store
            .import(&sections, identity_route, Atomicity::PerStream)
            .await
            .expect("import ok");
        assert_eq!(
            report.streams()[0].outcome,
            StreamOutcome::Corrupt { reached: None }
        );
        assert_eq!(versions(&store, &sk("a")).await, Vec::<u64>::new());
    }

    #[tokio::test]
    async fn per_stream_empty_chunk_is_empty_report() {
        let store = mnesis_inmemory::InMemoryStore::new();
        let report = store
            .import(&[], identity_route, Atomicity::PerStream)
            .await
            .expect("import ok");
        assert!(report.streams().is_empty());
        assert!(report.all_complete());
    }

    #[tokio::test]
    async fn per_stream_version_overflow_surfaces_error() {
        let store = mnesis_inmemory::InMemoryStore::new();
        let sections = vec![section("a", vec![evt(u64::MAX), evt(1)])];
        let err = store
            .import(&sections, identity_route, Atomicity::PerStream)
            .await
            .expect_err("overflow");
        assert!(matches!(err, ImportError::VersionOverflow));
    }

    // ── FailingAppendStore — test-only store for exercising Store-error arm ──

    /// Test-only store that delegates to `InMemoryStore` but fails `append`
    /// (with a Store error) for one target stream — the only way to exercise
    /// `import_per_stream`'s `AppendError::Store` arm, which `InMemoryStore`
    /// alone cannot produce.
    struct FailingAppendStore {
        inner: mnesis_inmemory::InMemoryStore,
        fail_on: String,
    }

    impl mnesis_store::store::RawEventStore for FailingAppendStore {
        type Error = mnesis_inmemory::InMemoryStoreError;
        type Stream = mnesis_inmemory::InMemoryStream;
        type AllPosition = mnesis_inmemory::InMemoryAllPos;
        type AllStream = mnesis_inmemory::InMemoryAllStream;

        async fn append(
            &self,
            id: &StreamKey,
            expected_version: Option<Version>,
            envelopes: &[mnesis_store::envelope::PendingEnvelope],
        ) -> Result<(), AppendError<Self::Error>> {
            if id.to_string() == self.fail_on {
                return Err(AppendError::Store(
                    mnesis_inmemory::InMemoryStoreError::VersionOverflow,
                ));
            }
            self.inner.append(id, expected_version, envelopes).await
        }

        async fn read_stream(
            &self,
            id: &StreamKey,
            from: Version,
        ) -> Result<Self::Stream, Self::Error> {
            self.inner.read_stream(id, from).await
        }

        async fn read_all(
            &self,
            from: Option<Self::AllPosition>,
        ) -> Result<Self::AllStream, Self::Error> {
            self.inner.read_all(from).await
        }
    }

    impl mnesis_store::import::AtomicAppend for FailingAppendStore {
        async fn atomic_append_many(
            &self,
            writes: &[mnesis_store::import::PlannedAppend],
        ) -> Result<(), mnesis_store::import::AtomicAppendError<Self::Error>> {
            self.inner.atomic_append_many(writes).await
        }
    }

    #[tokio::test]
    async fn per_stream_store_error_propagates_and_keeps_prior_commits() {
        let store = FailingAppendStore {
            inner: mnesis_inmemory::InMemoryStore::new(),
            fail_on: "b".to_owned(),
        };
        let sections = vec![
            section("a", vec![evt(1), evt(2)]), // commits
            section("b", vec![evt(1)]),         // append fails with Store
        ];
        let err = store
            .import(&sections, identity_route, Atomicity::PerStream)
            .await
            .expect_err("store error must surface");
        assert!(matches!(err, ImportError::Store(_)));
        // No cross-stream rollback: section "a" stayed committed.
        assert_eq!(versions(&store.inner, &sk("a")).await, vec![1, 2]);
    }

    // ── EventImporter WholeChunk behavioral tests ────────────────────────────

    #[tokio::test]
    async fn whole_chunk_clean_commits_all_complete() {
        let store = mnesis_inmemory::InMemoryStore::new();
        let sections = vec![
            section("a", vec![evt(1), evt(2)]),
            section("b", vec![evt(1)]),
        ];
        let report = store
            .import(&sections, identity_route, Atomicity::WholeChunk)
            .await
            .expect("import ok");
        assert!(report.all_complete());
        assert_eq!(versions(&store, &sk("a")).await, vec![1, 2]);
        assert_eq!(versions(&store, &sk("b")).await, vec![1]);
    }

    #[tokio::test]
    async fn whole_chunk_corrupt_block_aborts_nothing_lands() {
        let store = mnesis_inmemory::InMemoryStore::new();
        let sections = vec![
            section("a", vec![evt(1), evt(2)]),               // would be fine
            section("b", vec![evt(1), ImportBlock::Corrupt]), // bad block
        ];
        let err = store
            .import(&sections, identity_route, Atomicity::WholeChunk)
            .await
            .expect_err("aborts");
        match err {
            ImportError::Aborted { stream, reason } => {
                assert_eq!(stream, sk("b"));
                assert_eq!(reason, AbortReason::Corrupt);
            }
            other => panic!("expected Aborted, got {other:?}"),
        }
        // Atomicity: NOTHING landed — even the clean section "a" must be empty.
        assert_eq!(versions(&store, &sk("a")).await, Vec::<u64>::new());
        assert_eq!(versions(&store, &sk("b")).await, Vec::<u64>::new());
    }

    #[tokio::test]
    async fn whole_chunk_internal_gap_aborts_mismatch() {
        let store = mnesis_inmemory::InMemoryStore::new();
        let sections = vec![section("a", vec![evt(1), evt(2), evt(4)])];
        let err = store
            .import(&sections, identity_route, Atomicity::WholeChunk)
            .await
            .expect_err("aborts");
        match err {
            ImportError::Aborted { stream, reason } => {
                assert_eq!(stream, sk("a"));
                assert_eq!(
                    reason,
                    AbortReason::Mismatch {
                        expected: v(3),
                        got: v(4)
                    }
                );
            }
            other => panic!("expected Aborted, got {other:?}"),
        }
        assert_eq!(versions(&store, &sk("a")).await, Vec::<u64>::new());
    }

    #[tokio::test]
    async fn whole_chunk_head_conflict_aborts_mismatch() {
        let store = mnesis_inmemory::InMemoryStore::new();
        for n in 1..=2 {
            store
                .append(&sk("a"), Version::new(n - 1), &[pending(n, b"seed")])
                .await
                .expect("seed");
        }
        let sections = vec![section("a", vec![evt(1), evt(2)])];
        let err = store
            .import(&sections, identity_route, Atomicity::WholeChunk)
            .await
            .expect_err("aborts");
        match err {
            ImportError::Aborted { stream, reason } => {
                assert_eq!(stream, sk("a"));
                // store head 2 → wanted v3 next; offered v1.
                assert_eq!(
                    reason,
                    AbortReason::Mismatch {
                        expected: v(3),
                        got: v(1)
                    }
                );
            }
            other => panic!("expected Aborted, got {other:?}"),
        }
        assert_eq!(versions(&store, &sk("a")).await, vec![1, 2], "unchanged");
    }

    #[tokio::test]
    async fn whole_chunk_first_block_corrupt_aborts() {
        let store = mnesis_inmemory::InMemoryStore::new();
        let sections = vec![section("a", vec![ImportBlock::Corrupt])];
        let err = store
            .import(&sections, identity_route, Atomicity::WholeChunk)
            .await
            .expect_err("aborts");
        assert!(matches!(
            err,
            ImportError::Aborted { ref stream, reason: AbortReason::Corrupt } if *stream == sk("a")
        ));
    }

    #[tokio::test]
    async fn whole_chunk_conflict_on_later_section_reports_that_section() {
        let store = mnesis_inmemory::InMemoryStore::new();
        // Pre-seed "b" to v1 so the SECOND write conflicts (index 1), while "a"
        // (index 0) would be fine — exercises index > 0 in the Conflict arm.
        store
            .append(&sk("b"), None, &[pending(1, b"seed")])
            .await
            .expect("seed");
        let sections = vec![
            section("a", vec![evt(1), evt(2)]),
            section("b", vec![evt(1)]),
        ];
        let err = store
            .import(&sections, identity_route, Atomicity::WholeChunk)
            .await
            .expect_err("aborts");
        match err {
            ImportError::Aborted { stream, reason } => {
                assert_eq!(
                    stream,
                    sk("b"),
                    "must report the conflicting SECOND section"
                );
                // "b" head is 1 → wanted v2 next; offered v1.
                assert_eq!(
                    reason,
                    AbortReason::Mismatch {
                        expected: v(2),
                        got: v(1)
                    }
                );
            }
            other => panic!("expected Aborted, got {other:?}"),
        }
        // All-or-nothing: the clean section "a" must NOT have landed.
        assert_eq!(versions(&store, &sk("a")).await, Vec::<u64>::new());
        assert_eq!(versions(&store, &sk("b")).await, vec![1], "unchanged");
    }

    #[tokio::test]
    async fn whole_chunk_empty_section_skipped_others_commit() {
        let store = mnesis_inmemory::InMemoryStore::new();
        let sections = vec![section("empty", vec![]), section("b", vec![evt(1), evt(2)])];
        let report = store
            .import(&sections, identity_route, Atomicity::WholeChunk)
            .await
            .expect("import ok");
        assert!(report.all_complete());
        // The empty section produces NO report entry; only "b".
        assert_eq!(report.streams().len(), 1);
        assert_eq!(report.streams()[0].stream, sk("b"));
        assert_eq!(versions(&store, &sk("b")).await, vec![1, 2]);
    }

    // ── Defensive boundary: non-injective route (two origins → one target) ────

    #[tokio::test]
    async fn whole_chunk_non_injective_route_aborts_no_corruption() {
        // A non-injective route maps BOTH origin sections to the same target.
        // WholeChunk must abort with nothing landed — never a [1,2,1,2] stream.
        let store = mnesis_inmemory::InMemoryStore::new();
        let sections = vec![
            section("o1", vec![evt(1), evt(2)]),
            section("o2", vec![evt(1), evt(2)]),
        ];
        let to_same = |_origin: &[u8]| sk("T");
        let err = store
            .import(&sections, to_same, Atomicity::WholeChunk)
            .await
            .expect_err("non-injective route must abort");
        assert!(matches!(err, ImportError::Aborted { .. }));
        // All-or-nothing + no corruption: T is empty, definitely not [1,2,1,2].
        assert_eq!(versions(&store, &sk("T")).await, Vec::<u64>::new());
    }

    #[tokio::test]
    async fn per_stream_non_injective_route_second_rejected_no_corruption() {
        // PerStream: the first section commits the target; the second (same
        // target) is picky-rejected — the stream is [1,2], never [1,2,1,2].
        let store = mnesis_inmemory::InMemoryStore::new();
        let sections = vec![
            section("o1", vec![evt(1), evt(2)]),
            section("o2", vec![evt(1), evt(2)]),
        ];
        let to_same = |_origin: &[u8]| sk("T");
        let report = store
            .import(&sections, to_same, Atomicity::PerStream)
            .await
            .expect("import ok");
        assert_eq!(
            versions(&store, &sk("T")).await,
            vec![1, 2],
            "no [1,2,1,2] corruption — second section rejected, not appended"
        );
        assert_eq!(
            report.streams()[0].outcome,
            StreamOutcome::Complete { version: v(2) }
        );
        assert_eq!(
            report.streams()[1].outcome,
            StreamOutcome::Mismatch {
                reached: None,
                got: v(1)
            }
        );
    }

    // ── #247: the Store handle is the front door — import, no .raw() ─────────

    #[tokio::test]
    async fn store_handle_imports_without_raw() {
        // Substitutable: generic `EventImporter`/`AtomicAppend`-bounded code
        // accepts the handle directly (the structural win of #247).
        fn assert_importer<I: EventImporter>(_: &I) {}
        fn assert_atomic<A: AtomicAppend>(_: &A) {}

        // The handle itself imports and atomic-appends — never `.raw()`.
        let store = Store::new(mnesis_inmemory::InMemoryStore::new());
        let sections = vec![section("a", vec![evt(1), evt(2)])];
        let report = store
            .import(&sections, identity_route, Atomicity::PerStream)
            .await
            .expect("import via the handle");
        assert!(report.all_complete());
        assert_eq!(
            report.streams()[0].outcome,
            StreamOutcome::Complete { version: v(2) }
        );
        store
            .atomic_append_many(&[planned("b", None, &[1])])
            .await
            .expect("atomic append via the handle");

        // The generic bounds above accept the handle directly (the #247 win).
        assert_importer(&store);
        assert_atomic(&store);
    }

    // ── Round-trip + idempotency (Card-1 export ↔ Card-2 import) ────────────

    async fn export_section(store: &mnesis_inmemory::InMemoryStore, origin: &str) -> StreamSection {
        let blocks = store
            .export_stream(&sk(origin), Version::INITIAL)
            .await
            .expect("export opens")
            .map(|r| ImportBlock::Event(r.expect("no read error")))
            .collect::<Vec<_>>()
            .await;
        section(origin, blocks)
    }

    #[tokio::test]
    async fn export_then_import_round_trips_byte_equal_modulo_all_position() {
        // Source store with two streams.
        let source = mnesis_inmemory::InMemoryStore::new();
        for n in 1..=3 {
            source
                .append(&sk("acct-1"), Version::new(n - 1), &[pending(n, b"a")])
                .await
                .expect("seed a");
        }
        source
            .append(&sk("acct-2"), None, &[pending(1, b"b")])
            .await
            .expect("seed b");

        let sections = vec![
            export_section(&source, "acct-1").await,
            export_section(&source, "acct-2").await,
        ];

        // Import into a FRESH store, identity routing.
        let target = mnesis_inmemory::InMemoryStore::new();
        // Pre-seed a warmup event so the target's `$all` counter is already
        // advanced — imported events must get FRESH positions (2..) rather than
        // copying the source's (which started at 1), making the restamp
        // observable on the `$all` read.
        target
            .append(&sk("warmup"), None, &[pending(1, b"warmup")])
            .await
            .expect("warmup seed");
        let report = target
            .import(&sections, identity_route, Atomicity::PerStream)
            .await
            .expect("import ok");
        assert!(report.all_complete());

        // Per-stream events are byte-equal (a per-stream event carries no global
        // position): compare version/type/payload/metadata/schema verbatim.
        for origin in ["acct-1", "acct-2"] {
            let src: Vec<PersistedEnvelope> = source
                .read_stream(&sk(origin), Version::INITIAL)
                .await
                .expect("read")
                .map(|r| r.expect("ok"))
                .collect()
                .await;
            let dst: Vec<PersistedEnvelope> = target
                .read_stream(&sk(origin), Version::INITIAL)
                .await
                .expect("read")
                .map(|r| r.expect("ok"))
                .collect()
                .await;
            assert_eq!(src.len(), dst.len(), "{origin} length");
            for (s, d) in src.iter().zip(dst.iter()) {
                assert_eq!(s.version(), d.version(), "{origin} version");
                assert_eq!(s.event_type(), d.event_type(), "{origin} type");
                assert_eq!(s.payload(), d.payload(), "{origin} payload");
                assert_eq!(s.metadata(), d.metadata(), "{origin} metadata");
                assert_eq!(s.schema_version(), d.schema_version(), "{origin} schema");
            }
        }

        // Restamp observable on `$all`: the target re-stamped fresh positions, so
        // its `$all` order is the warmup (1) then the imported events (2..=5),
        // strictly increasing — NOT the source's positions copied verbatim.
        let target_positions: Vec<u64> = target
            .read_all(None)
            .await
            .expect("read_all")
            .map(|r| r.expect("ok").0.as_u64())
            .collect()
            .await;
        assert_eq!(
            target_positions,
            vec![1, 2, 3, 4, 5],
            "import must re-stamp fresh $all positions, not copy the source's",
        );
    }

    #[tokio::test]
    async fn reimport_same_chunk_is_idempotent_per_stream() {
        let store = mnesis_inmemory::InMemoryStore::new();
        let sections = vec![section("a", vec![evt(1), evt(2), evt(3)])];

        let first = store
            .import(&sections, identity_route, Atomicity::PerStream)
            .await
            .expect("import ok");
        assert!(first.all_complete());

        let second = store
            .import(&sections, identity_route, Atomicity::PerStream)
            .await
            .expect("import ok");
        assert_eq!(
            second.streams()[0].outcome,
            StreamOutcome::Mismatch {
                reached: None,
                got: v(1)
            }
        );
        assert_eq!(versions(&store, &sk("a")).await, vec![1, 2, 3], "no dupes");
    }

    #[tokio::test]
    async fn reimport_same_chunk_whole_chunk_aborts() {
        let store = mnesis_inmemory::InMemoryStore::new();
        let sections = vec![section("a", vec![evt(1), evt(2)])];
        store
            .import(&sections, identity_route, Atomicity::WholeChunk)
            .await
            .expect("first import ok");
        let err = store
            .import(&sections, identity_route, Atomicity::WholeChunk)
            .await
            .expect_err("re-import aborts");
        match err {
            ImportError::Aborted { stream, reason } => {
                assert_eq!(stream, sk("a"));
                assert_eq!(
                    reason,
                    AbortReason::Mismatch {
                        expected: v(3),
                        got: v(1)
                    }
                );
            }
            other => panic!("expected Aborted, got {other:?}"),
        }
        assert_eq!(versions(&store, &sk("a")).await, vec![1, 2], "no dupes");
    }

    // ── Linearizability: concurrent import vs direct writer (CLAUDE rule 7 §4) ─

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn per_stream_import_concurrent_with_writer_surfaces_conflict_no_torn_stream() {
        // Two actors race to populate the SAME fresh target stream from v1:
        // a direct writer (append v1..=N) and an importer (a v1..=N section).
        // Exactly one wins the v1 slot; the other must surface a Mismatch (a
        // conflict, never a silent retry — CLAUDE rule 5). The stream must end
        // as a gapless prefix from v1, never torn.
        let store = Arc::new(mnesis_inmemory::InMemoryStore::new());
        let n = 50u64;
        let barrier = Arc::new(Barrier::new(2));

        let writer_store = Arc::clone(&store);
        let writer_barrier = Arc::clone(&barrier);
        let writer = tokio::spawn(async move {
            writer_barrier.wait().await;
            for vn in 1..=n {
                // Conflicts are expected once the importer wins; ignore them.
                let _ = writer_store
                    .append(&sk("race"), Version::new(vn - 1), &[pending(vn, b"w")])
                    .await;
            }
        });

        let importer_store = Arc::clone(&store);
        let importer_barrier = Arc::clone(&barrier);
        let importer = tokio::spawn(async move {
            importer_barrier.wait().await;
            let blocks: Vec<ImportBlock> = (1..=n).map(evt).collect();
            let sections = vec![section("race", blocks)];
            importer_store
                .import(&sections, identity_route, Atomicity::PerStream)
                .await
                .expect("import returns Ok (conflicts are per-stream outcomes)")
        });

        writer.await.expect("writer task");
        let report = importer.await.expect("importer task");

        // The importer's outcome is either Complete (it won v1) or Mismatch
        // (the writer won) — never a partial/torn application.
        let outcome = report.streams().first().map(|s| s.outcome);
        match outcome {
            Some(
                StreamOutcome::Complete { .. } | StreamOutcome::Mismatch { reached: None, .. },
            ) => {}
            other => panic!("unexpected importer outcome: {other:?}"),
        }

        // Whatever the interleaving, the final stream is a gapless prefix from 1.
        let final_versions = versions(&store, &sk("race")).await;
        for (expected, got) in (1u64..).zip(final_versions.iter()) {
            assert_eq!(*got, expected, "stream must be a gapless prefix from 1");
        }
        assert!(!final_versions.is_empty(), "some events landed");
    }

    // ── State-machine property test: PerStream import never gaps ────────────

    // Model-based: a sequence of single-stream PerStream imports against one
    // target. The model tracks the stream's next-expected version; each import
    // offers a first version + a contiguous block count. Invariants: the
    // reported outcome matches the model's picky rule, the stream never develops
    // a gap, and what landed matches the report.
    #[derive(Debug, Clone)]
    enum Cmd {
        // Offer `count` contiguous events starting at `first`.
        Import { first: u64, count: u64 },
    }

    fn cmd_strategy() -> impl Strategy<Value = Cmd> {
        // Boundary-inclusive: first ∈ {1,2,3}, count ∈ {1,2,3}.
        (prop_oneof![Just(1u64), Just(2u64), Just(3u64)], 1u64..=3)
            .prop_map(|(first, count)| Cmd::Import { first, count })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        #[test]
        fn state_machine_per_stream_never_gaps_and_report_matches_model(
            cmds in proptest::collection::vec(cmd_strategy(), 1..12),
        ) {
            let rt = tokio::runtime::Builder::new_current_thread()
                .build()
                .expect("runtime");
            rt.block_on(async {
                let store = mnesis_inmemory::InMemoryStore::new();
                let id = sk("m");
                // Model: next expected version (1 = fresh).
                let mut next: u64 = 1;

                for cmd in cmds {
                    let Cmd::Import { first, count } = cmd;
                    let blocks: Vec<ImportBlock> =
                        (first..first + count).map(evt).collect();
                    let sections = vec![section("m", blocks)];
                    let report = {
                        store
                            .import(&sections, identity_route, Atomicity::PerStream)
                            .await
                            .expect("import ok")
                    };
                    let outcome = report.streams()[0].outcome;

                    if first == next {
                        // Picky check passes: the whole contiguous run applies.
                        let landed_last = first + count - 1;
                        prop_assert_eq!(
                            outcome,
                            StreamOutcome::Complete { version: v(landed_last) }
                        );
                        next = landed_last + 1;
                    } else {
                        // Picky reject: nothing applied, model unchanged.
                        prop_assert_eq!(
                            outcome,
                            StreamOutcome::Mismatch { reached: None, got: v(first) }
                        );
                    }

                    // Invariant: the stream is ALWAYS a gapless prefix 1..next-1.
                    let got = versions(&store, &id).await;
                    let expected: Vec<u64> = (1..next).collect();
                    prop_assert_eq!(got, expected);
                }
                Ok(())
            })?;
        }
    }
}
