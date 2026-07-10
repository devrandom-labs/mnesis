//! The kit's own red/green loop: every conformance check runs against
//! `InMemoryStore`, the reference adapter. A check that fails here is a kit
//! bug (or a real `InMemoryStore` contract violation — either way, a find).

#![allow(clippy::unwrap_used, reason = "tests")]
#![allow(clippy::expect_used, reason = "tests")]
#![allow(clippy::missing_panics_doc, reason = "tests")]

use nexus::Version;
use nexus_inmemory::{InMemorySnapshotStore, InMemoryStore};
use nexus_store_testing::boundary;
use nexus_store_testing::linearizability;
use nexus_store_testing::sequence;
use nexus_store_testing::{atomic, lifecycle, snapshot};

#[allow(
    clippy::unused_async,
    reason = "factory shape must stay async fn — other adapters' factories (fjall, postgres) \
              await inside; InMemoryStore::new() alone has nothing to await"
)]
async fn factory() -> (InMemoryStore, ()) {
    (InMemoryStore::new(), ())
}

// Check modules land in Tasks 2-6; the macro invocation replaces these direct
// calls in Task 7.
#[tokio::test]
async fn sequence_part1() {
    sequence::check_empty_read_yields_none(&factory).await;
    sequence::check_append_then_read_round_trips(&factory).await;
    sequence::check_versions_strictly_monotonic_and_fused(&factory).await;
    sequence::check_large_stream_completes(&factory).await;
    sequence::check_read_stream_from_is_inclusive(&factory).await;
    sequence::check_append_conflict_is_surfaced(&factory).await;
    sequence::check_append_retry_after_conflict_succeeds(&factory).await;
}

#[tokio::test]
async fn sequence_all_reads() {
    sequence::check_all_empty_store_yields_none(&factory).await;
    sequence::check_all_global_order_across_streams(&factory).await;
    sequence::check_all_from_is_exclusive(&factory).await;
    sequence::check_all_multi_resume_cycles(&factory).await;
    sequence::check_all_boundary_then_new_append(&factory).await;
    sequence::check_read_stream_inclusive_read_all_exclusive_coexist(&factory).await;
}

#[tokio::test]
async fn sequence_subscription() {
    sequence::check_subscription_backlog_then_caught_up_then_live(&factory).await;
    sequence::check_subscription_resume_strict_after(&factory).await;
    sequence::check_subscription_all_backlog_then_caught_up_then_live(&factory).await;
    sequence::check_subscription_large_backlog_crosses_chunk_seam(&factory).await;
}

#[tokio::test]
async fn boundary_checks() {
    boundary::check_conflict_leaves_store_unchanged(&factory).await;
    boundary::check_version_gap_batch_rejected(&factory).await;
    boundary::check_wrong_first_version_rejected(&factory).await;
    boundary::check_metadata_absent_vs_present_distinct(&factory).await;
    boundary::check_max_length_event_type_round_trips(&factory).await;
    boundary::check_prefix_stream_ids_isolated(&factory).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn linearizability_checks() {
    linearizability::check_concurrent_same_stream_single_winner(&factory).await;
    linearizability::check_concurrent_distinct_streams_all_land(&factory).await;
    linearizability::check_wake_after_idle(&factory).await;
    linearizability::check_caught_up_boundary_race(&factory).await;
}

// Lifecycle against InMemoryStore is a self-check ONLY: "reopen" hands back
// the same store (in-memory has nothing to close). It validates the kit's
// closure plumbing, not real persistence — fjall/postgres prove that part.
#[allow(
    clippy::unused_async,
    reason = "factory/reopen shape must stay async fn to match the lifecycle module's Future bound"
)]
#[tokio::test]
async fn lifecycle_plumbing() {
    let open = || async { (InMemoryStore::new(), ()) };
    let reopen = |store: InMemoryStore, (): ()| async move { (store, ()) };
    lifecycle::check_reopen_preserves_events(&open, &reopen).await;
    lifecycle::check_reopen_preserves_position_watermark(&open, &reopen).await;
    lifecycle::check_reopen_conflict_state_intact(&open, &reopen).await;
    lifecycle::check_reopen_subscription_catches_up(&open, &reopen).await;
}

#[tokio::test]
async fn atomic_checks() {
    atomic::check_atomic_multi_stream_commits_all(&factory).await;
    atomic::check_atomic_conflict_aborts_all(&factory).await;
    atomic::check_atomic_empty_batch_is_noop(&factory).await;
}

#[tokio::test]
async fn snapshot_checks() {
    let sfactory = || async { (InMemorySnapshotStore::<Vec<u8>, Version>::new(), ()) };
    let p1 = Version::new(5).expect("5 is non-zero");
    let p2 = Version::new(9).expect("9 is non-zero");
    snapshot::check_snapshot_absent_then_commit_then_found(&sfactory, p1, p2).await;
    snapshot::check_snapshot_stale_on_schema_change(&sfactory, p1, p2).await;
    snapshot::check_snapshot_overwrite_latest_wins(&sfactory, p1, p2).await;
}
