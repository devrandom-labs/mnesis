//! The kit's own red/green loop: every conformance check runs against
//! `InMemoryStore`, the reference adapter. A check that fails here is a kit
//! bug (or a real `InMemoryStore` contract violation — either way, a find).

#![allow(clippy::unwrap_used, reason = "tests")]
#![allow(clippy::expect_used, reason = "tests")]
#![allow(clippy::missing_panics_doc, reason = "tests")]

use nexus_inmemory::InMemoryStore;
use nexus_store_testing::sequence;

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
