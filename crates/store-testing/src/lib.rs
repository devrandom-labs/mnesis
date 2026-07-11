//! The executable conformance kit for `nexus-store` adapters (issue #281).
//!
//! ## What this is
//!
//! Every store adapter (`nexus-inmemory`, `nexus-fjall`, `nexus-postgres`,
//! and any future one) implements the same seam: `RawEventStore` +
//! `WakeSource`, optionally `AtomicAppend` and `SnapshotStore`. That seam has
//! a contract that goes well beyond "the trait compiles" — inclusive vs.
//! exclusive read bounds, optimistic-conflict rejection, subscription
//! catch-up→live ordering, concurrent-writer linearizability. This crate
//! pins that contract as **runnable checks**, organized into four
//! cross-cutting categories (CLAUDE.md rule 7) plus two opt-in capability
//! modules:
//!
//! - [`sequence`] — multi-step protocol: append/read round-trips, optimistic
//!   conflict + retry, `$all` ordering and resume, subscription
//!   catch-up-then-live.
//! - [`boundary`] — defensive inputs: version gaps, wrong first version,
//!   metadata absent vs. present, max-length event types, prefix-colliding
//!   stream ids.
//! - [`linearizability`] — concurrent writers: single-winner on a contended
//!   stream, all-land on distinct streams, wake-after-idle, the
//!   `CaughtUp`-boundary race.
//! - [`lifecycle`] — close → reopen (persistent adapters only): events,
//!   the `$all` watermark, and conflict state all survive a reopen.
//! - [`atomic`] (feature `atomic-append`) — `AtomicAppend`: multi-stream
//!   commits are all-or-nothing.
//! - [`snapshot`] (feature `snapshot`) — `SnapshotStore`: state and position
//!   commit/hydrate atomically together, and a schema bump reads back
//!   `Stale`, never decode garbage.
//!
//! An adapter proves conformance by invoking the [`conformance!`] macro (and
//! the capability macros it needs) once, from one test file. Each generates
//! one named `#[tokio::test]` per check, so nextest reports every contract
//! rule as its own test — a failure names the exact rule that broke, not
//! "some test in the suite."
//!
//! ## The factory contract
//!
//! Every macro takes a factory: `Fn() -> Fut` where
//! `Fut: Future<Output = (S, C)> + Send`, `S: RawEventStore + WakeSource`.
//! `C` is an adapter-chosen **guard** kept alive for the check's duration —
//! a `TempDir` for fjall, `()` for an adapter that owns its storage outright
//! (in-memory, a connection pool). The factory is called once per generated
//! test and must produce a **fresh** store each call; checks never share
//! state across tests.
//!
//! ```ignore
//! nexus_store_testing::conformance! {
//!     factory: || async { (InMemoryStore::new(), ()) },
//! }
//! ```
//!
//! ## Capability and lifecycle macros
//!
//! - [`conformance_atomic_append!`] — `AtomicAppend` checks; requires the
//!   `atomic-append` feature and an `S: AtomicAppend` factory.
//! - [`conformance_snapshot!`] — `SnapshotStore` checks; requires the
//!   `snapshot` feature, an `S: SnapshotStore<_, P>` factory, and two
//!   ascending sample `P` positions.
//! - [`conformance_lifecycle!`] — close/reopen checks against the SAME
//!   backing storage; skipped entirely by in-memory adapters (nothing to
//!   reopen), run by every persistent adapter (fjall, postgres).
//!
//! ## `skip_unless:` for environment-gated adapters
//!
//! Every macro accepts an optional `skip_unless: <fn() -> bool>` that guards
//! each generated test: when it returns `false` the test returns
//! immediately (a vacuous pass, not a failure). `nexus-postgres` uses this
//! to skip the whole matrix when `DATABASE_URL` is unset locally, while
//! still running for real under the nixosTest CI attribute that supplies a
//! live database.
//!
//! ## Contract notes
//!
//! Ambiguities pinned during this work, load-bearing for anyone writing a
//! new adapter:
//!
//! - **Read visibility under concurrent append is adapter-unspecified.**
//!   Whether a reader started before a concurrent commit observes it is not
//!   part of the contract — `FjallStore` pins one snapshot at scan-open
//!   (repeatable-read), `InMemoryStore` re-reads live state on each refill.
//!   Both are conformant; the kit asserts eventual convergence (every
//!   committed event is observed once the reader catches up), never
//!   mid-flight visibility.
//! - **`GlobalSeq` / `$all` positions are strictly monotonic but never
//!   gapless.** An aborted append may burn a position with no event landing
//!   there. Checks assert strict ordering, never contiguity.
//! - **Empty stream ids are a permitted adapter limitation.** fjall/LSM
//!   rejects an empty key, so the kit only ever constructs non-empty stream
//!   ids — an adapter is not required to support the empty id.
//! - **`Some(empty)` metadata is unrepresentable by construction.** The
//!   envelope's metadata value type rejects a zero-length `Some` at
//!   construction (`ValueError::MetadataEmpty`) — the wire format reserves
//!   `u32::MAX` as the absent-metadata sentinel, so "empty but present"
//!   would collide with "absent". The boundary check is
//!   `check_metadata_absent_vs_present_distinct`, not …`_vs_empty_`.
//!
//! The full "writing a store adapter" guide (worked example, toy adapter,
//! troubleshooting) is PR3 of #281 — this crate stays the executable
//! contract, not the tutorial.

#![allow(
    clippy::unwrap_used,
    reason = "test harness — assertions naturally use unwrap"
)]
#![allow(
    clippy::expect_used,
    reason = "test harness — assertions naturally use expect"
)]
#![allow(clippy::panic, reason = "test harness — failures signal via panic")]
#![allow(
    clippy::missing_panics_doc,
    reason = "test harness — every check panics on failure"
)]
pub mod boundary;
pub mod lifecycle;
pub mod linearizability;
pub mod row;
pub mod sequence;

#[cfg(feature = "atomic-append")]
pub mod atomic;
#[cfg(feature = "snapshot")]
pub mod snapshot;

pub use row::{ConformanceRow, SubId};

// ═══════════════════════════════════════════════════════════════════════════
// `conformance!` macro entry points (#281)
// ═══════════════════════════════════════════════════════════════════════════

/// One generated conformance test: skip-guard, factory, check call.
#[doc(hidden)]
#[macro_export]
macro_rules! __conformance_case {
    ($module:ident, $check:ident, $factory:expr, $skip:expr) => {
        #[tokio::test]
        async fn $check() {
            if !($skip)() {
                return;
            }
            $crate::$module::$check(&$factory).await;
        }
    };
    (multi_thread: $module:ident, $check:ident, $factory:expr, $skip:expr) => {
        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn $check() {
            if !($skip)() {
                return;
            }
            $crate::$module::$check(&$factory).await;
        }
    };
}

/// Run the full core conformance matrix (sequence + boundary +
/// linearizability) against a store factory.
///
/// The factory returns `(store, guard)`: the guard keeps any backing resource
/// (a temp dir, a pool) alive for the check's duration — use `()` when the
/// store owns everything.
///
/// ```ignore
/// nexus_store_testing::conformance! {
///     factory: || async { (InMemoryStore::new(), ()) },
/// }
/// ```
///
/// Requires `tokio` (with `macros`, `rt-multi-thread`) as a dev-dependency of
/// the invoking crate.
#[macro_export]
macro_rules! conformance {
    (factory: $factory:expr $(,)?) => {
        $crate::conformance! { factory: $factory, skip_unless: || true }
    };
    (factory: $factory:expr, skip_unless: $skip:expr $(,)?) => {
        mod conformance_sequence {
            #[allow(clippy::wildcard_imports, reason = "re-import the invocation scope")]
            use super::*;

            $crate::__conformance_case!(sequence, check_empty_read_yields_none, $factory, $skip);
            $crate::__conformance_case!(sequence, check_append_then_read_round_trips, $factory, $skip);
            $crate::__conformance_case!(sequence, check_versions_strictly_monotonic_and_fused, $factory, $skip);
            $crate::__conformance_case!(sequence, check_large_stream_completes, $factory, $skip);
            $crate::__conformance_case!(sequence, check_read_stream_from_is_inclusive, $factory, $skip);
            $crate::__conformance_case!(sequence, check_append_conflict_is_surfaced, $factory, $skip);
            $crate::__conformance_case!(sequence, check_append_retry_after_conflict_succeeds, $factory, $skip);
            $crate::__conformance_case!(sequence, check_all_empty_store_yields_none, $factory, $skip);
            $crate::__conformance_case!(sequence, check_all_global_order_across_streams, $factory, $skip);
            $crate::__conformance_case!(sequence, check_all_from_is_exclusive, $factory, $skip);
            $crate::__conformance_case!(sequence, check_all_multi_resume_cycles, $factory, $skip);
            $crate::__conformance_case!(sequence, check_all_boundary_then_new_append, $factory, $skip);
            $crate::__conformance_case!(sequence, check_read_stream_inclusive_read_all_exclusive_coexist, $factory, $skip);
            $crate::__conformance_case!(sequence, check_subscription_backlog_then_caught_up_then_live, $factory, $skip);
            $crate::__conformance_case!(sequence, check_subscription_resume_strict_after, $factory, $skip);
            $crate::__conformance_case!(sequence, check_subscription_all_backlog_then_caught_up_then_live, $factory, $skip);
            $crate::__conformance_case!(sequence, check_subscription_large_backlog_crosses_chunk_seam, $factory, $skip);
            $crate::__conformance_case!(sequence, check_subscription_absent_stream_waits_then_delivers, $factory, $skip);
            $crate::__conformance_case!(sequence, check_two_subscribers_same_stream_both_receive, $factory, $skip);
        }

        mod conformance_boundary {
            #[allow(clippy::wildcard_imports, reason = "re-import the invocation scope")]
            use super::*;

            $crate::__conformance_case!(boundary, check_conflict_leaves_store_unchanged, $factory, $skip);
            $crate::__conformance_case!(boundary, check_version_gap_batch_rejected, $factory, $skip);
            $crate::__conformance_case!(boundary, check_wrong_first_version_rejected, $factory, $skip);
            $crate::__conformance_case!(boundary, check_metadata_absent_vs_present_distinct, $factory, $skip);
            $crate::__conformance_case!(boundary, check_max_length_event_type_round_trips, $factory, $skip);
            $crate::__conformance_case!(boundary, check_prefix_stream_ids_isolated, $factory, $skip);
            $crate::__conformance_case!(boundary, check_large_payload_round_trips, $factory, $skip);
        }

        mod conformance_linearizability {
            #[allow(clippy::wildcard_imports, reason = "re-import the invocation scope")]
            use super::*;

            $crate::__conformance_case!(multi_thread: linearizability, check_concurrent_same_stream_single_winner, $factory, $skip);
            $crate::__conformance_case!(multi_thread: linearizability, check_concurrent_distinct_streams_all_land, $factory, $skip);
            $crate::__conformance_case!(multi_thread: linearizability, check_wake_after_idle, $factory, $skip);
            $crate::__conformance_case!(multi_thread: linearizability, check_caught_up_boundary_race, $factory, $skip);
        }
    };
}

/// Run the `AtomicAppend` capability conformance (feature `atomic-append`).
///
/// Same factory shape as [`conformance!`]: `Fn() -> Fut<Output = (S, C)>`
/// with `S: AtomicAppend`.
///
/// ```ignore
/// nexus_store_testing::conformance_atomic_append! {
///     factory: || async { (InMemoryStore::new(), ()) },
/// }
/// ```
#[cfg(feature = "atomic-append")]
#[macro_export]
macro_rules! conformance_atomic_append {
    (factory: $factory:expr $(,)?) => {
        $crate::conformance_atomic_append! { factory: $factory, skip_unless: || true }
    };
    (factory: $factory:expr, skip_unless: $skip:expr $(,)?) => {
        mod conformance_atomic {
            #[allow(clippy::wildcard_imports, reason = "re-import the invocation scope")]
            use super::*;

            $crate::__conformance_case!(
                atomic,
                check_atomic_multi_stream_commits_all,
                $factory,
                $skip
            );
            $crate::__conformance_case!(atomic, check_atomic_conflict_aborts_all, $factory, $skip);
            $crate::__conformance_case!(atomic, check_atomic_empty_batch_is_noop, $factory, $skip);
        }
    };
}

/// Run the `SnapshotStore` capability conformance (feature `snapshot`).
///
/// Same factory shape as [`conformance!`], with `S: SnapshotStore<_, P>`.
/// `positions` are two ascending sample positions of the store's `P` — used
/// to drive the hydrate/commit checks without hardcoding a position type.
///
/// `extremes` are two ascending positions at the `P` type's representable
/// edges (smallest and at/near the ceiling) — used to prove the position
/// codec has no off-by-one at either edge.
///
/// ```ignore
/// nexus_store_testing::conformance_snapshot! {
///     factory: || async { (InMemorySnapshotStore::<Vec<u8>, Version>::new(), ()) },
///     positions: (Version::new(5).unwrap(), Version::new(9).unwrap()),
///     extremes: (Version::new(1).unwrap(), Version::new(u64::MAX).unwrap()),
/// }
/// ```
#[cfg(feature = "snapshot")]
#[macro_export]
macro_rules! conformance_snapshot {
    (factory: $factory:expr, positions: ($p1:expr, $p2:expr), extremes: ($pmin:expr, $pmax:expr) $(,)?) => {
        $crate::conformance_snapshot! { factory: $factory, positions: ($p1, $p2), extremes: ($pmin, $pmax), skip_unless: || true }
    };
    (factory: $factory:expr, positions: ($p1:expr, $p2:expr), extremes: ($pmin:expr, $pmax:expr), skip_unless: $skip:expr $(,)?) => {
        mod conformance_snapshot {
            #[allow(clippy::wildcard_imports, reason = "re-import the invocation scope")]
            use super::*;

            #[tokio::test]
            async fn check_snapshot_absent_then_commit_then_found() {
                if !($skip)() {
                    return;
                }
                $crate::snapshot::check_snapshot_absent_then_commit_then_found(&$factory, $p1, $p2)
                    .await;
            }
            #[tokio::test]
            async fn check_snapshot_stale_on_schema_change() {
                if !($skip)() {
                    return;
                }
                $crate::snapshot::check_snapshot_stale_on_schema_change(&$factory, $p1, $p2).await;
            }
            #[tokio::test]
            async fn check_snapshot_overwrite_latest_wins() {
                if !($skip)() {
                    return;
                }
                $crate::snapshot::check_snapshot_overwrite_latest_wins(&$factory, $p1, $p2).await;
            }
            #[tokio::test]
            async fn check_snapshot_empty_state_round_trips() {
                if !($skip)() {
                    return;
                }
                $crate::snapshot::check_snapshot_empty_state_round_trips(&$factory, $p1, $p2).await;
            }
            #[tokio::test]
            async fn check_snapshot_extreme_positions_round_trip() {
                if !($skip)() {
                    return;
                }
                $crate::snapshot::check_snapshot_extreme_positions_round_trip(
                    &$factory, $pmin, $pmax,
                )
                .await;
            }
        }
    };
}

/// Run the lifecycle conformance (persistent adapters only): `open` yields a
/// fresh `(store, ctx)`; `reopen` consumes both and reopens the SAME storage.
///
/// `open` has the same factory shape as [`conformance!`]; `reopen` is
/// `Fn(S, C) -> Fut<Output = (S, C)>`, taking ownership of the prior
/// `(store, guard)` pair so it can drop the store before reopening the same
/// backing storage.
///
/// ```ignore
/// nexus_store_testing::conformance_lifecycle! {
///     open: open_fresh,
///     reopen: |store: FjallStore, dir: TempDir| async move {
///         drop(store);
///         let reopened = FjallStore::builder(dir.path().join("db"))
///             .open()
///             .expect("reopen fjall store");
///         (reopened, dir)
///     },
/// }
/// ```
#[macro_export]
macro_rules! conformance_lifecycle {
    (open: $open:expr, reopen: $reopen:expr $(,)?) => {
        $crate::conformance_lifecycle! { open: $open, reopen: $reopen, skip_unless: || true }
    };
    (open: $open:expr, reopen: $reopen:expr, skip_unless: $skip:expr $(,)?) => {
        mod conformance_lifecycle {
            #[allow(clippy::wildcard_imports, reason = "re-import the invocation scope")]
            use super::*;

            #[tokio::test]
            async fn check_reopen_preserves_events() {
                if !($skip)() {
                    return;
                }
                $crate::lifecycle::check_reopen_preserves_events(&$open, &$reopen).await;
            }
            #[tokio::test]
            async fn check_reopen_preserves_position_watermark() {
                if !($skip)() {
                    return;
                }
                $crate::lifecycle::check_reopen_preserves_position_watermark(&$open, &$reopen)
                    .await;
            }
            #[tokio::test]
            async fn check_reopen_conflict_state_intact() {
                if !($skip)() {
                    return;
                }
                $crate::lifecycle::check_reopen_conflict_state_intact(&$open, &$reopen).await;
            }
            #[tokio::test]
            async fn check_reopen_subscription_catches_up() {
                if !($skip)() {
                    return;
                }
                $crate::lifecycle::check_reopen_subscription_catches_up(&$open, &$reopen).await;
            }
        }
    };
}
