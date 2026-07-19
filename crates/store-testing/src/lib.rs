//! The executable conformance kit for `mnesis-store` adapters — and the guide
//! to writing one (issue #281).
//!
//! Every store adapter (`mnesis-inmemory`, `mnesis-fjall`, `mnesis-postgres`,
//! and any future one) implements the same seam:
//! [`RawEventStore`](mnesis_store::store::RawEventStore) +
//! [`WakeSource`](mnesis_store::wake::WakeSource), optionally `AtomicAppend`
//! and [`SnapshotStore`](mnesis_store::state::SnapshotStore). That seam has
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
//! The rest of this page is the **writing-a-store-adapter guide**. Sections
//! 1–4 restate the seam's contract in one place (the trait docs in
//! `mnesis-store` remain the normative source; follow the links); section 5
//! shows how to prove an implementation with the kit; section 6 lists the
//! pinned ambiguities that trip up new adapters. It assumes no knowledge of
//! the shipped adapters — you never need to read `mnesis-fjall` or
//! `mnesis-postgres` source.
//!
//! # What you implement
//!
//! One store type implements two mandatory traits.
//!
//! [`RawEventStore`](mnesis_store::store::RawEventStore) — bytes in, bytes
//! out. The adapter never sees typed events or codecs; the repository facade
//! encodes into [`PendingEnvelope`](mnesis_store::envelope::PendingEnvelope)s
//! before calling you. You supply:
//!
//! - `type Error` — your own error type, bound
//!   `core::error::Error + Send + Sync + 'static`. Keep it distinct from
//!   `mnesis-store`'s facade error types; the facade wraps yours, and a shared
//!   type would double-wrap.
//! - `type Stream` — the per-stream read cursor: an owned, `'static`,
//!   `Send` `futures::Stream` with
//!   `Item = Result<PersistedEnvelope, Self::Error>` (the
//!   [`EventStream`](mnesis_store::stream::EventStream) marker bound).
//! - `type AllPosition` — your store-local `$all` resume position: any
//!   `Copy + Ord + Send + Sync + Debug + 'static` type implementing
//!   [`AllPosition`](mnesis_store::store::AllPosition). A scalar sequence
//!   for an embedded store, a commit-ordered composite for a concurrent SQL
//!   store. It is never carried on the envelope; it rides only on `$all`
//!   items. `mnesis-store` ships no scalar impl, and the orphan rule blocks
//!   `impl AllPosition for u64` in your crate — define a local newtype:
//!   `struct MyPos(u64); impl AllPosition for MyPos {}` (plus the derives
//!   the supertraits need).
//! - `type AllStream` — the all-streams read cursor: an owned, `'static`,
//!   `Send` `futures::Stream` with
//!   `Item = Result<(Self::AllPosition, StreamKey, PersistedEnvelope), Self::Error>`
//!   — every item is tagged with its position AND the origin
//!   [`StreamKey`](mnesis_store::StreamKey) (stream attribution is a store
//!   guarantee — a `$all` consumer routes without decoding the payload). The
//!   per-stream `read_stream` does NOT stamp the id: there it is the query
//!   argument, so re-stamping it on every item would be redundant.
//! - Make both stream types `Unpin`. The trait imposes no such bound, but
//!   the subscription path
//!   ([`Subscription`](mnesis_store::subscription::Subscription)) requires it.
//! - Three methods — [`append`](mnesis_store::store::RawEventStore::append),
//!   [`read_stream`](mnesis_store::store::RawEventStore::read_stream),
//!   [`read_all`](mnesis_store::store::RawEventStore::read_all) — whose
//!   contracts are the next two sections.
//!
//! [`WakeSource`](mnesis_store::wake::WakeSource) — how live subscriptions
//! learn that a commit landed. See "The wake contract" below.
//!
//! Optional capability traits, each with its own kit module:
//!
//! - `AtomicAppend` (at `mnesis_store::import::AtomicAppend`, behind
//!   `mnesis-store`'s `import` feature) — commit several per-stream runs in
//!   **one** transaction, all-or-nothing; the primitive bulk import needs.
//!   Each write's `expected_version` is validated against the target's
//!   **running** head (counting earlier writes to the same target inside the
//!   batch); any mismatch aborts the whole transaction with
//!   `AtomicAppendError::Conflict { index, actual }`, and on any failure
//!   **no** write is applied.
//! - [`SnapshotStore<Vec<u8>, P>`](mnesis_store::state::SnapshotStore) —
//!   atomic persistence of derived state plus the position it was folded to
//!   (`hydrate` / `commit`). Byte-level: `S = Vec<u8>`; typed state is a
//!   codec bridge upstream, not your concern. Two useful instantiations:
//!   `P = Version` (aggregate snapshots) and `P =` your `AllPosition`
//!   (projection checkpoints). `hydrate` returns the three-state
//!   [`Hydrated`](mnesis_store::state::Hydrated) — `Absent` (never saved),
//!   `Stale` (saved under a different schema version; the caller rebuilds),
//!   `Found` (position + state). State and position commit **together**: the
//!   trait has no "save state alone", and your implementation must persist
//!   the pair atomically so a half-write is unrepresentable.
//!
//! Consumers never call you directly — they go through the `Store<S>`
//! handle, repositories, and subscriptions, all generic over the seam.
//! Implement the traits and everything above them works.
//!
//! ## Storing an event
//!
//! `append` hands you `&[PendingEnvelope]`; reads must hand back
//! [`PersistedEnvelope`](mnesis_store::envelope::PersistedEnvelope)s. The
//! supported recipe is the canonical wire frame: persist, per event, the
//! `Version` (from `PendingEnvelope::version()`) plus the output of
//! [`encode_frame`](mnesis_store::wire::encode_frame):
//!
//! ```ignore
//! let frame = mnesis_store::wire::encode_frame(
//!     env.schema_version_value(),
//!     &env.event_type_value(),
//!     &env.payload_value(),
//!     env.metadata_value().as_ref(),
//! )?; // EncodedFrame { value: Bytes, offsets: FrameOffsets }
//! ```
//!
//! Store `frame.value` (one contiguous `Bytes` buffer), `frame.offsets`,
//! and the `SchemaVersion`; on read, rebuild with
//! [`PersistedEnvelope::try_new`](mnesis_store::envelope::PersistedEnvelope::try_new)
//! `(version, value, schema_version, offsets.event_type, offsets.payload,
//! offsets.metadata)`. The frame lands the payload on a 16-byte boundary
//! inside the buffer — an invariant zero-copy codecs (rkyv, POD) rely on. A
//! custom storage layout is allowed, but then payload alignment and field
//! re-validation are on you.
//!
//! # The append contract
//!
//! [`append(id, expected_version, envelopes)`](mnesis_store::store::RawEventStore::append)
//! is optimistic concurrency:
//!
//! - `expected_version` is the stream head the caller last saw: `None` = a
//!   fresh stream with no events, `Some(v)` = the head is exactly `v`.
//!   Compare it against the stream's **actual** current head; on mismatch
//!   return [`AppendError::Conflict`](mnesis_store::error::AppendError)
//!   carrying the stream id, the caller's expectation, and the actual head —
//!   the caller reloads from `actual` and retries. The diagnostic id field
//!   is `mnesis::ErrorId`, built truncation-aware from the key's `Display`:
//!   `stream_id: ErrorId::from_display(id)`.
//! - The head check and the event insertion **must** be one atomic step (a
//!   transaction, CAS, or a lock). A check-then-insert with a window between
//!   lets a concurrent writer slip in and corrupt the stream; the kit's
//!   linearizability checks race real writers at exactly this seam.
//! - Envelope versions must run strictly sequentially from
//!   `expected_version + 1` (from `1` when `None`). A gap, duplicate, or
//!   out-of-order batch is rejected in the `Conflict` domain — and
//!   **nothing** lands: a rejected append leaves the store byte-identical,
//!   per-stream and `$all` alike. In that `Conflict`, `expected` is the
//!   caller's stated expectation and `actual` is the store's current head —
//!   the fields describe the head disagreement, never the malformed batch.
//! - Stamp every accepted event with the next `AllPosition`: strictly
//!   monotonic across **all** streams in commit order, **not** required to
//!   be gapless — an aborted append may burn positions, and readers
//!   tolerate the gaps.
//! - An empty `envelopes` slice: run the head check first (a stale
//!   `expected_version` is still a `Conflict`), then return `Ok` — nothing
//!   written, nobody woken.
//! - After the commit is durable — never before — fire your wake path (see
//!   "The wake contract").
//!
//! # The read contract
//!
//! Two read methods, deliberately asymmetric.
//!
//! [`read_stream(id, from)`](mnesis_store::store::RawEventStore::read_stream)
//! — a bounded scan of one stream:
//!
//! - `from` is **inclusive**: yield every event with `version >= from`, in
//!   ascending `Version` order, then terminate with `None`.
//! - An absent stream is an **empty** stream, never an error.
//! - After `None` the stream stays `None` (fused) — the kit polls again to
//!   prove it.
//! - Internal batching/pagination is allowed and must be invisible; bounding
//!   resident memory is your concern (fjall, for instance, holds one lazy
//!   LSM cursor rather than fixed-size batches).
//!
//! [`read_all(from: Option<AllPosition>)`](mnesis_store::store::RawEventStore::read_all)
//! — a bounded scan across all streams:
//!
//! - `from` is **exclusive**: `None` = from the very beginning, `Some(p)` =
//!   strictly after `p`. Yield in ascending position order, each item tagged
//!   `(position, stream key, envelope)`, then terminate with `None` when
//!   caught up.
//! - Resume is `Ord`-based: the subscription loop reopens with the last
//!   position it delivered, and there is deliberately no successor function.
//!   Your scan must read "strictly greater than `from`" — tolerating gaps by
//!   scanning a range, never by stepping `+1`.
//!
//! The asymmetry (inclusive `Version` vs. exclusive `AllPosition`) is
//! intentional: a single stream's versions are a gapless successor sequence,
//! so the resume seam computes `v + 1` itself and asks inclusively — while a
//! composite `$all` position (e.g. postgres's transaction-ordered pair) has
//! no natural `+1`, so resume must be "strictly after what I saw". Both
//! reads serve the same strict-after resume; the difference is who computes
//! the successor.
//!
//! What a scan opened *before* a concurrent commit observes is
//! adapter-unspecified — see "Contract notes".
//!
//! # The wake contract
//!
//! A live subscription is a catch-up-then-park loop; the loop itself ships
//! generically in `mnesis-store` and works for any adapter. Your half is
//! [`WakeSource`](mnesis_store::wake::WakeSource): two methods and one
//! call-site discipline.
//!
//! - [`register(stream: Option<&[u8]>)`](mnesis_store::wake::WakeSource::register)
//!   — called once, synchronously, when a subscription opens
//!   (`None` registers for `$all`). Return a
//!   [`WakeRegistration`](mnesis_store::wake::WakeRegistration) that keeps
//!   wake-routing alive until dropped.
//! - [`arm`](mnesis_store::wake::WakeRegistration::arm) — returns an owned
//!   `'static` future. Contract: the future captures a "seen point" at the
//!   moment `arm` is called and resolves once a wake is delivered **after**
//!   that point — a wake landing between `arm` and the `.await` must NOT be
//!   lost. The generic loop arms *before* its confirming re-scan whenever it
//!   thinks it is caught up; that ordering plus your arm-time capture is the
//!   entire lost-wakeup defense.
//! - [`wake(stream)`](mnesis_store::wake::WakeSource::wake) — call after
//!   **every** durable commit to `stream`, never before (a woken subscriber
//!   immediately re-reads and must see the data). A per-stream commit is
//!   also an `$all` event: `$all` observers must be woken too.
//! - Spurious wakes are permitted — each costs one empty re-scan. Lost wakes
//!   are not — a lost wake is a subscription hung forever, and the kit's
//!   `check_wake_after_idle` and `check_caught_up_boundary_race` exist to
//!   catch exactly that.
//!
//! In-process adapters should not build this machinery: embed
//! `mnesis_wake::StreamNotifiers` and delegate — the exact shape
//! `mnesis-inmemory` and `mnesis-fjall` ship:
//!
//! ```ignore
//! use std::sync::Arc;
//!
//! use mnesis_store::wake::WakeSource;
//! use mnesis_wake::{NotifyError, StreamNotifiers, WakeReg};
//!
//! struct MyStore {
//!     // ... your storage ...
//!     notifiers: Arc<StreamNotifiers>, // StreamNotifiers::new() -> Arc<StreamNotifiers>
//! }
//!
//! impl WakeSource for MyStore {
//!     type Registration = WakeReg;
//!     type Error = NotifyError;
//!
//!     fn register(&self, stream: Option<&[u8]>) -> Result<WakeReg, NotifyError> {
//!         self.notifiers.register(stream)
//!     }
//!
//!     fn wake(&self, stream: &[u8]) {
//!         self.notifiers.wake(stream); // per-stream subscribers + the `$all` generation
//!     }
//! }
//! ```
//!
//! …and at the end of a successful non-empty `append`, after the commit is
//! durable: `self.notifiers.wake(id.as_ref());` (one call — it wakes the
//! stream's subscribers and bumps the store-wide `$all` generation).
//! A distributed adapter implements the same two traits over its own signal
//! (postgres: `LISTEN`/`NOTIFY`).
//!
//! # Running the kit
//!
//! An adapter proves conformance by invoking the [`conformance!`] macro (and
//! the capability macros it needs) once, from one test file. Each generates
//! one named `#[tokio::test]` per check, so nextest reports every contract
//! rule as its own test — a failure names the exact rule that broke, not
//! "some test in the suite." Dependencies you'll need: `tokio` with
//! `macros` + `rt-multi-thread` (plus `sync`/`time` if your adapter uses
//! tokio primitives), `thiserror` for your error enum (workspace rule), and
//! `mnesis-wake` for the in-process `WakeSource`.
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
//! mnesis_store_testing::conformance! {
//!     factory: || async { (InMemoryStore::new(), ()) },
//! }
//! ```
//!
//! ## Capability and lifecycle macros
//!
//! - [`conformance_atomic_append!`] — `AtomicAppend` checks; requires the
//!   `atomic-append` feature and an `S: AtomicAppend` factory.
//! - [`conformance_snapshot!`] — `SnapshotStore` checks; requires the
//!   `snapshot` feature, an `S: SnapshotStore<_, P>` factory, and two pairs
//!   of ascending sample `P` positions: `positions` (ordinary values) and
//!   `extremes` (the representable edges, proving the position codec has no
//!   off-by-one at either end).
//! - [`conformance_lifecycle!`] — close/reopen checks against the SAME
//!   backing storage; skipped entirely by in-memory adapters (nothing to
//!   reopen), run by every persistent adapter (fjall, postgres). Takes two
//!   closures: `open` (the usual factory shape) and `reopen`
//!   (`Fn(S, C) -> Fut<Output = (S, C)>`), which consumes the prior pair so
//!   it can drop the store before reopening the same storage.
//!
//! ## `skip_unless:` for environment-gated adapters
//!
//! Every macro accepts an optional `skip_unless: <fn() -> bool>` that guards
//! each generated test: when it returns `false` the test returns
//! immediately (a vacuous pass, not a failure). `mnesis-postgres` uses this
//! to skip the whole matrix when `DATABASE_URL` is unset locally, while
//! still running for real under the nixosTest CI attribute that supplies a
//! live database.
//!
//! # Contract notes
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
//! - **Public error enums are `#[non_exhaustive]`.**
//!   [`AppendError`](mnesis_store::error::AppendError),
//!   `AtomicAppendError`, and their siblings may grow variants without a
//!   major bump — match the variant you handle (`Conflict`) plus a wildcard
//!   arm, never exhaustively.

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
/// mnesis_store_testing::conformance! {
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
            $crate::__conformance_case!(sequence, check_all_items_carry_their_stream_key, $factory, $skip);
            $crate::__conformance_case!(sequence, check_all_from_is_exclusive, $factory, $skip);
            $crate::__conformance_case!(sequence, check_all_multi_resume_cycles, $factory, $skip);
            $crate::__conformance_case!(sequence, check_all_boundary_then_new_append, $factory, $skip);
            $crate::__conformance_case!(sequence, check_read_stream_inclusive_read_all_exclusive_coexist, $factory, $skip);
            $crate::__conformance_case!(sequence, check_subscription_backlog_then_caught_up_then_live, $factory, $skip);
            $crate::__conformance_case!(sequence, check_subscription_resume_strict_after, $factory, $skip);
            $crate::__conformance_case!(sequence, check_subscription_all_backlog_then_caught_up_then_live, $factory, $skip);
            $crate::__conformance_case!(sequence, check_subscription_large_backlog_crosses_chunk_seam, $factory, $skip);
            $crate::__conformance_case!(sequence, check_subscription_absent_stream_waits_then_delivers, $factory, $skip);
            $crate::__conformance_case!(sequence, check_subscription_beyond_head_filters_below_bound, $factory, $skip);
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
/// mnesis_store_testing::conformance_atomic_append! {
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
/// mnesis_store_testing::conformance_snapshot! {
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
/// mnesis_store_testing::conformance_lifecycle! {
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
