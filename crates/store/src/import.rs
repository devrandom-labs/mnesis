//! Import contract — picky, per-stream, halt-not-skip.
//!
//! Import takes already-decoded events (the *box* — CBOR default or CESR —
//! turns chunk bytes into [`PersistedEnvelope`]s; import is box-agnostic) and
//! places them onto caller-supplied target streams. The store's
//! sequential-`append` version check does the hard work; import adds routing,
//! a halt-on-trouble rule, and a per-stream report.
//!
//! Events carry no per-event stream id (export does no rewrite), so routing
//! is driven by the **per-stream section** the box records: each section names
//! its origin stream once, and import maps that to a target stream via the
//! caller-supplied `route` closure.
//!
//! Resolved semantics (issue #145 §5):
//!
//! - **Picky per stream** — a stream's first incoming version must equal that
//!   stream's next expected version, else that stream is rejected; import
//!   never silently trims a partial overlap.
//! - **Halt, never apply-skip** — a bad block (failed checksum, or version
//!   trouble) halts *its* stream at the last good version and holds back its
//!   later blocks; it never punches a gap.
//! - **Atomicity is a caller policy** ([`Atomicity`]) — whole-chunk
//!   (all-or-nothing, server bulk-restore) vs per-stream (a bad block stops
//!   only its stream, mobile resilience).
//! - **Idempotency is a side-effect** of the version check — re-importing
//!   already-present events is refused, with no dedup machinery.
//!
//! This module is the contract: the data types (the per-stream outcomes,
//! the report, the error) and the [`EventImporter`] trait. The concrete
//! ingest impl is a later card.

use alloc::vec::Vec;

use bytes::Bytes;
use mnesis::Version;
use thiserror::Error;

use crate::envelope::{PendingBatch, PendingEnvelope, PersistedEnvelope};
use crate::error::AppendError;
use crate::store::{RawEventStore, Store};
use crate::stream_id::StreamKey;

/// Atomicity granularity for an import — a caller policy, not a format
/// property. The same chunk imports either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Atomicity {
    /// All-or-nothing: any bad block rolls back the whole chunk
    /// ([`ImportError::Aborted`]). Best for server bulk-restore on reliable
    /// storage — big transactions, retry the lot.
    WholeChunk,
    /// Each stream's slice commits in its own transaction: a bad block stops
    /// only its stream, the rest commit. Best for mobile resilience on flaky
    /// storage. Reported per stream in an [`ImportReport`].
    PerStream,
}

/// How one stream's import ended, and where the stream sits afterward.
///
/// "Where it sits" lives inside each variant so the success case always
/// carries a real [`Version`] while the trouble cases carry `Option`
/// (`None` = the stream was never touched). Illegal states (a "complete"
/// stream with no version) are unrepresentable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamOutcome {
    /// Every block offered for this stream was applied; it is now at `version`.
    Complete { version: Version },
    /// A block failed its per-block checksum. The good prefix (if any) was
    /// applied, leaving the stream at `reached` (`None` = untouched). The
    /// corrupt block's own version is deliberately absent — a failed checksum
    /// means its decoded header cannot be trusted; re-fetch from `reached`'s
    /// successor.
    Corrupt { reached: Option<Version> },
    /// A block's version did not match the stream's next expected version
    /// (stale overlap or forward gap). Good prefix applied, stream left at
    /// `reached`; `got` is trustworthy (the checksum passed, only the
    /// position was wrong).
    Mismatch {
        reached: Option<Version>,
        got: Version,
    },
}

impl StreamOutcome {
    /// Whether every offered block for this stream was applied.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        matches!(self, Self::Complete { .. })
    }

    /// Where the stream sits after import (`None` = still empty / untouched).
    #[must_use]
    pub const fn reached(&self) -> Option<Version> {
        match self {
            Self::Complete { version } => Some(*version),
            Self::Corrupt { reached } | Self::Mismatch { reached, .. } => *reached,
        }
    }
}

/// One stream's outcome within an import, tagged with the caller's target
/// stream id (echoed verbatim — import owns no naming policy).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamReport {
    /// The target [`StreamKey`] this outcome is for.
    pub stream: StreamKey,
    /// What happened to it.
    pub outcome: StreamOutcome,
}

/// Per-stream outcomes of an import — one [`StreamReport`] per stream.
///
/// In first-seen order. Describes only work that actually ran (a whole-chunk
/// abort is an [`ImportError`], not a report of "nothing happened").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportReport {
    streams: Vec<StreamReport>,
}

impl ImportReport {
    /// Build a report from per-stream outcomes.
    #[must_use]
    pub const fn new(streams: Vec<StreamReport>) -> Self {
        Self { streams }
    }

    /// All per-stream outcomes, in first-seen order.
    #[must_use]
    pub fn streams(&self) -> &[StreamReport] {
        &self.streams
    }

    /// The streams the sync loop must act on — everything that isn't
    /// [`StreamOutcome::Complete`].
    pub fn unfinished(&self) -> impl Iterator<Item = &StreamReport> {
        self.streams.iter().filter(|s| !s.outcome.is_complete())
    }

    /// Whether every stream completed.
    #[must_use]
    pub fn all_complete(&self) -> bool {
        self.streams.iter().all(|s| s.outcome.is_complete())
    }
}

/// Why a whole-chunk import aborted — the first bad block's failure mode.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AbortReason {
    /// A block failed its per-block checksum.
    #[error("block failed checksum")]
    Corrupt,
    /// A block's version did not match the target stream's next expected
    /// version.
    #[error("version mismatch (expected {expected}, got {got})")]
    Mismatch { expected: Version, got: Version },
}

/// A whole-operation import failure — distinct from the *expected* per-stream
/// outcomes carried in an [`ImportReport`]. `E` is the underlying store error.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ImportError<E> {
    // NOTE: there is no `Malformed` variant. Decoding the backup box is the
    // box's job (`cbor::decode_chunk` → `ChunkError::Malformed`); `import` takes
    // already-decoded `&[StreamSection]`, and the only `EventImporter` impl is
    // the blanket one, so no code path here can produce a malformed-chunk error.
    // A variant nothing can construct would mislead callers; re-add additively
    // (behind an inline-decoding importer) if one is ever introduced.
    /// Whole-chunk atomicity only: a bad block rolled the entire chunk back —
    /// nothing was written. `stream` is the first offender; retry the whole
    /// chunk. (Per-stream atomicity never aborts — it reports per stream.)
    #[error("chunk aborted at stream {stream}: {reason}")]
    Aborted {
        stream: StreamKey,
        reason: AbortReason,
    },
    /// The underlying store transaction failed.
    #[error(transparent)]
    Store(E),
    /// A stream's `version + 1` overflowed `u64`. NOT a conflict — not
    /// retryable.
    #[error("version overflow")]
    VersionOverflow,
}

/// One origin stream's events, as decoded by the backup box (Card 3).
///
/// The origin stream id is recorded **once** per section (export stamps no
/// per-event id); the importer maps it to a target via the `route` closure.
#[derive(Debug, Clone)]
pub struct StreamSection {
    /// The origin stream id, exactly as the box recorded it.
    pub origin: Bytes,
    /// The section's blocks, in version order as the box laid them down.
    pub blocks: Vec<ImportBlock>,
}

/// One block within a [`StreamSection`].
#[derive(Debug, Clone)]
pub enum ImportBlock {
    /// A block whose per-block checksum passed and decoded to an event.
    Event(PersistedEnvelope),
    /// A block whose per-block checksum **failed**. Carries nothing: a failed
    /// checksum means the decoded header — including its version — cannot be
    /// trusted, which is exactly why [`StreamOutcome::Corrupt`] omits the
    /// version.
    Corrupt,
}

/// One planned per-stream write for [`AtomicAppend::atomic_append_many`].
///
/// The run is a contiguous, version-preserving sequence; `expected_version` is
/// the head the target stream must currently be at (`None` = the stream must be
/// fresh). Built by the importer's per-section planner.
///
/// The run is split into [`head`](Self::head) + [`tail`](Self::tail) so its
/// non-emptiness is structural: [`batch`](Self::batch) hands an adapter a
/// [`PendingBatch`] with no runtime check (#330).
#[derive(Debug, Clone)]
pub struct PlannedAppend {
    /// The resolved target [`StreamKey`].
    pub target: StreamKey,
    /// The version the target must currently be at (`None` = fresh stream).
    pub expected_version: Option<Version>,
    /// The run's first envelope — the lowest version in the run.
    pub head: PendingEnvelope,
    /// The rest of the run, in version order (empty for a one-event run).
    pub tail: Vec<PendingEnvelope>,
}

impl PlannedAppend {
    /// The run as the non-empty batch [`AtomicAppend`] hands to the store.
    #[must_use]
    pub fn batch(&self) -> PendingBatch<'_> {
        PendingBatch::from_parts(&self.head, &self.tail)
    }
}

/// Failure of an [`AtomicAppend::atomic_append_many`] transaction.
///
/// `Conflict` is the cross-stream picky check: write `index`'s
/// `expected_version` did not match the target's actual head (`actual`). The
/// whole transaction is rolled back — nothing landed. `Store` is an
/// adapter-level failure. Distinct domains, distinct variants (CLAUDE rule 3).
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum AtomicAppendError<E> {
    /// Write at `index` had a head mismatch; `actual` is the target's real head.
    #[error("atomic append conflict at write {index}: actual head {actual:?}")]
    Conflict {
        index: usize,
        actual: Option<Version>,
    },
    /// Adapter-level failure (I/O, encoding, global-seq overflow, …).
    #[error("atomic append store error: {0}")]
    Store(#[source] E),
}

/// Adapter capability: commit several per-stream runs in **one** atomic
/// transaction.
///
/// Either every run lands or none do. This is the primitive
/// [`Atomicity::WholeChunk`] needs and that [`RawEventStore::append`]
/// (per-stream only) cannot provide. Adapters implement it with a real
/// transaction (fjall cross-partition `write_tx`, postgres `BEGIN..COMMIT`,
/// `InMemoryStore` its single mutex).
///
/// # Contract
///
/// - For each write, the target's actual head must equal `expected_version`,
///   else the whole transaction aborts with [`AtomicAppendError::Conflict`]
///   carrying that write's `index` and the target's real head.
/// - Each write's `events` must be a contiguous run starting at
///   `expected_version + 1`. The caller (the importer's planner) guarantees
///   this; implementations validate defensively at their own boundary.
/// - Each write is validated against the target's **running** head, including
///   prior writes to the same target in this batch. A non-injective route (two
///   writes to one stream) therefore surfaces as [`AtomicAppendError::Conflict`]
///   on the second write — never a silently concatenated, gap-creating stream.
/// - On any failure, **no** write is applied.
///
/// # Return value
///
/// On success this returns the **highest** [`AllPosition`](RawEventStore::AllPosition)
/// the transaction committed, across every stream it touched — the
/// read-your-writes token for the whole batch (#330). A consumer that has
/// reached it has been delivered every event the batch wrote. `None` iff
/// `writes` is empty (a no-op commits no position); unlike
/// [`RawEventStore::append`], the empty case is *reachable* here — a caller may
/// hand an empty `writes` — so it is answered with `Option` rather than removed.
///
/// [`AllPosition`]: RawEventStore::AllPosition
pub trait AtomicAppend: RawEventStore {
    /// Append every write atomically. See the trait contract.
    fn atomic_append_many(
        &self,
        writes: &[PlannedAppend],
    ) -> impl core::future::Future<
        Output = Result<Option<Self::AllPosition>, AtomicAppendError<Self::Error>>,
    > + Send;
}

/// `Store<S>` forwards [`AtomicAppend`] to its inner backend (issue #247). With
/// `Store<S>` already a [`RawEventStore`], this gives it [`EventImporter`] for
/// free via the blanket impl below — so a handle holder can `store.import(..)`
/// without `.raw()`.
impl<S: AtomicAppend> AtomicAppend for Store<S> {
    async fn atomic_append_many(
        &self,
        writes: &[PlannedAppend],
    ) -> Result<Option<Self::AllPosition>, AtomicAppendError<Self::Error>> {
        self.raw().atomic_append_many(writes).await
    }
}

/// Place decoded events onto caller-routed target streams, picky per stream,
/// halt-not-skip, under an [`Atomicity`] policy.
///
/// Input is per-stream [`StreamSection`]s carrying [`ImportBlock`]s. Each
/// section's origin stream id (recorded once by the box — export stamps no
/// per-event id) is mapped to the receiver's target stream id `I` by `route`
/// (e.g. `task-123` → `phone:task-123`); import holds no naming policy of its
/// own.
///
/// On success returns an [`ImportReport`] of per-stream outcomes (per-stream
/// atomicity) or completes (whole-chunk). `Err` is reserved for
/// whole-operation failures.
///
/// [`Atomicity::PerStream`] is best-effort per stream: a store failure stops
/// the import and surfaces an error, but sections already committed are not
/// rolled back. Empty sections produce no report entry.
pub trait EventImporter: RawEventStore + AtomicAppend {
    /// Import per-stream sections onto caller-routed target streams.
    ///
    /// `route` maps each section's origin id bytes to a target [`StreamKey`]
    /// (e.g. `task-123` → `phone:task-123`); for an identity restore it is
    /// simply [`StreamKey::from_slice`].
    fn import<R>(
        &self,
        sections: &[StreamSection],
        route: R,
        atomicity: Atomicity,
    ) -> impl core::future::Future<Output = Result<ImportReport, ImportError<Self::Error>>> + Send
    where
        R: Fn(&[u8]) -> StreamKey + Send;
}

// =============================================================================
// Per-section planner — pure, no store access
// =============================================================================

/// Where a section's contiguous run stopped.
#[derive(Debug)]
enum Halt {
    /// Every block in the section was consumed.
    Complete,
    /// Stopped at a corrupt block.
    Corrupt,
    /// Stopped at a version discontinuity; `got` is the offending version.
    Gap { got: Version },
}

/// The store-independent plan for one [`StreamSection`].
#[derive(Debug)]
enum SectionPlan {
    /// No blocks — nothing to do, no report entry.
    Empty,
    /// The first block was corrupt — nothing can be appended.
    FirstCorrupt,
    /// A contiguous run to append, and how it ended.
    ///
    /// `(expected_version, events)` become a [`PlannedAppend`] once `route`
    /// resolves the target (whole-chunk path).
    Run {
        /// The run's first version. Cached (not `events[0].version()`) so the
        /// consumer maps a store conflict to `got` without indexing/unwrap on
        /// the non-empty run.
        first: Version,
        /// The head the target must be at (`None` = fresh).
        expected_version: Option<Version>,
        /// The run's first envelope. Split from the tail so the run's
        /// non-emptiness is carried by the shape rather than asserted in prose
        /// — `PendingBatch::from_parts` then needs no runtime check (#330).
        head: PendingEnvelope,
        /// The rest of the run, in version order (empty for a one-event run).
        tail: Vec<PendingEnvelope>,
        /// The run's last version (where the stream lands on success). Cached
        /// so the consumer needs no `events.last()` unwrap.
        last: Version,
        /// Why the run stopped.
        halt: Halt,
    },
}

/// Planner failure — a stream version overflowed `u64` (NOT a conflict).
#[derive(Debug)]
enum PlanError {
    VersionOverflow,
}

/// Build a section's plan: decode the first block, accumulate the longest
/// contiguous run, and record why it stopped. Pure — no store access.
fn plan_section(section: &StreamSection) -> Result<SectionPlan, PlanError> {
    let mut blocks = section.blocks.iter();
    let Some(first_block) = blocks.next() else {
        return Ok(SectionPlan::Empty);
    };
    let first_event = match first_block {
        ImportBlock::Corrupt => return Ok(SectionPlan::FirstCorrupt),
        ImportBlock::Event(event) => event,
    };

    let first = first_event.version();
    // expected head = first - 1; first == 1 → None (fresh stream). Checked.
    let expected_version = first.as_u64().checked_sub(1).and_then(Version::new);

    let head = PendingEnvelope::from_persisted(first_event);
    let mut tail = Vec::new();
    let mut last = first;
    let halt = loop {
        let Some(block) = blocks.next() else {
            break Halt::Complete;
        };
        let event = match block {
            ImportBlock::Corrupt => break Halt::Corrupt,
            ImportBlock::Event(event) => event,
        };
        // Only reached when a successor block exists; a run ending at u64::MAX
        // with no successor completes above without ever calling next().
        let expected_next = last.next().ok_or(PlanError::VersionOverflow)?;
        if event.version() != expected_next {
            break Halt::Gap {
                got: event.version(),
            };
        }
        tail.push(PendingEnvelope::from_persisted(event));
        last = event.version();
    };

    Ok(SectionPlan::Run {
        first,
        expected_version,
        head,
        tail,
        last,
        halt,
    })
}

// =============================================================================
// Blanket EventImporter impl
// =============================================================================

impl<S: RawEventStore + AtomicAppend> EventImporter for S {
    async fn import<R>(
        &self,
        sections: &[StreamSection],
        route: R,
        atomicity: Atomicity,
    ) -> Result<ImportReport, ImportError<Self::Error>>
    where
        R: Fn(&[u8]) -> StreamKey + Send,
    {
        match atomicity {
            Atomicity::PerStream => import_per_stream(self, sections, route).await,
            Atomicity::WholeChunk => import_whole_chunk(self, sections, route).await,
        }
    }
}

/// `PerStream` import: each section its own `append` transaction. A bad block
/// stops only its stream; the rest commit. Always returns `Ok(report)` unless
/// a genuine store error or version overflow occurs.
///
/// This is the **only** import path that needs just [`RawEventStore`] — no
/// cross-stream [`AtomicAppend`]. A produce-only device adapter that cannot do
/// a cross-partition transaction (so cannot implement [`EventImporter`], whose
/// unified `import` offers [`Atomicity::WholeChunk`] too) still gets per-stream
/// restore by calling this function directly. It is the exact mobile-resilience
/// path `WholeChunk` cannot serve.
///
/// An empty section (no blocks) produces no `StreamReport` entry; a caller
/// correlating sections to report entries must not assume positional
/// correspondence.
///
/// # Errors
///
/// Returns [`ImportError::Store`] if the underlying `append` fails, or
/// [`ImportError::VersionOverflow`] if a stream's `version + 1` overflows
/// `u64`. In either case sections already appended remain committed —
/// `PerStream` performs no cross-stream rollback, and the partial report is
/// discarded with the error.
pub async fn import_per_stream<S, R>(
    store: &S,
    sections: &[StreamSection],
    route: R,
) -> Result<ImportReport, ImportError<S::Error>>
where
    S: RawEventStore,
    R: Fn(&[u8]) -> StreamKey + Send,
{
    let mut reports = Vec::with_capacity(sections.len());
    for section in sections {
        let target = route(section.origin.as_ref());
        let plan = match plan_section(section) {
            Ok(plan) => plan,
            Err(PlanError::VersionOverflow) => return Err(ImportError::VersionOverflow),
        };
        let outcome = match plan {
            SectionPlan::Empty => continue,
            SectionPlan::FirstCorrupt => StreamOutcome::Corrupt { reached: None },
            SectionPlan::Run {
                first,
                expected_version,
                head,
                tail,
                last,
                halt,
            } => match store
                .append(
                    &target,
                    expected_version,
                    PendingBatch::from_parts(&head, &tail),
                )
                .await
            {
                Ok(_position) => match halt {
                    Halt::Complete => StreamOutcome::Complete { version: last },
                    Halt::Corrupt => StreamOutcome::Corrupt {
                        reached: Some(last),
                    },
                    Halt::Gap { got } => StreamOutcome::Mismatch {
                        reached: Some(last),
                        got,
                    },
                },
                Err(AppendError::Conflict { .. }) => StreamOutcome::Mismatch {
                    reached: None,
                    got: first,
                },
                Err(AppendError::Store(error)) => return Err(ImportError::Store(error)),
            },
        };
        reports.push(StreamReport {
            stream: target,
            outcome,
        });
    }
    Ok(ImportReport::new(reports))
}

/// [`WholeChunk`] import: all-or-nothing across every section. Any halt (corrupt
/// block or internal gap) or head conflict aborts the whole chunk — nothing
/// lands. First offender (section order, then block order) wins.
///
/// [`WholeChunk`]: Atomicity::WholeChunk
async fn import_whole_chunk<S, R>(
    store: &S,
    sections: &[StreamSection],
    route: R,
) -> Result<ImportReport, ImportError<S::Error>>
where
    S: RawEventStore + AtomicAppend,
    R: Fn(&[u8]) -> StreamKey + Send,
{
    // Phase 1 — plan every section purely. Any halt is a hard abort here.
    let mut writes: Vec<PlannedAppend> = Vec::with_capacity(sections.len());
    let mut firsts: Vec<Version> = Vec::with_capacity(sections.len());
    let mut lasts: Vec<Version> = Vec::with_capacity(sections.len());
    for section in sections {
        let target = route(section.origin.as_ref());
        let plan = match plan_section(section) {
            Ok(plan) => plan,
            Err(PlanError::VersionOverflow) => return Err(ImportError::VersionOverflow),
        };
        // Exhaustive: a future SectionPlan variant must be handled here (no `..`
        // catch-all), matching import_per_stream's exhaustiveness.
        let (first, expected_version, head, tail, last, halt) = match plan {
            SectionPlan::Empty => continue, // skip; no report entry
            SectionPlan::FirstCorrupt => {
                return Err(ImportError::Aborted {
                    stream: target,
                    reason: AbortReason::Corrupt,
                });
            }
            SectionPlan::Run {
                first,
                expected_version,
                head,
                tail,
                last,
                halt,
            } => (first, expected_version, head, tail, last, halt),
        };
        match halt {
            Halt::Complete => {}
            Halt::Corrupt => {
                return Err(ImportError::Aborted {
                    stream: target,
                    reason: AbortReason::Corrupt,
                });
            }
            Halt::Gap { got } => {
                let expected = last.next().ok_or(ImportError::VersionOverflow)?;
                return Err(ImportError::Aborted {
                    stream: target,
                    reason: AbortReason::Mismatch { expected, got },
                });
            }
        }
        firsts.push(first);
        lasts.push(last);
        writes.push(PlannedAppend {
            target,
            expected_version,
            head,
            tail,
        });
    }

    // Phase 2 — commit every clean run in one transaction. The committed `$all`
    // position is not surfaced in the report (it is keyed by per-stream
    // `Version`); a positioned import report is a PR2 follow-up (#330).
    match store.atomic_append_many(&writes).await {
        Ok(_position) => {
            let reports = writes
                .into_iter()
                .zip(lasts)
                .map(|(write, last)| StreamReport {
                    stream: write.target,
                    outcome: StreamOutcome::Complete { version: last },
                })
                .collect();
            Ok(ImportReport::new(reports))
        }
        Err(AtomicAppendError::Conflict { index, actual }) => {
            Err(map_atomic_conflict(&firsts, &writes, index, actual))
        }
        Err(AtomicAppendError::Store(error)) => Err(ImportError::Store(error)),
    }
}

/// Map an [`AtomicAppend`] conflict into an [`ImportError::Aborted`],
/// defensively (rule 4 — validate at our own boundary). The primitive's
/// contract is `index < writes.len()` (== `firsts.len()`), but a misbehaving
/// adapter returning an out-of-range index must NOT cause an OOB panic:
/// `writes`/`firsts` are non-empty and equal-length whenever a Conflict is
/// returned, so a `None` from `.get(index)` means a broken adapter, and we
/// degrade to the first planned run so the abort is still reported coherently.
/// `firsts[index]` is the conflicting run's cached first version (no
/// `events[0]` indexing).
fn map_atomic_conflict<E>(
    firsts: &[Version],
    writes: &[PlannedAppend],
    index: usize,
    actual: Option<Version>,
) -> ImportError<E> {
    let (Some(&got), Some(write)) = (
        firsts.get(index).or_else(|| firsts.first()),
        writes.get(index).or_else(|| writes.first()),
    ) else {
        // Unreachable: a Conflict implies a non-empty batch.
        return ImportError::VersionOverflow;
    };
    let expected = match actual {
        Some(head) => match head.next() {
            Some(next) => next,
            None => return ImportError::VersionOverflow,
        },
        None => Version::INITIAL,
    };
    ImportError::Aborted {
        stream: write.target.clone(),
        reason: AbortReason::Mismatch { expected, got },
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code asserts exact values"
)]
mod plan_tests {
    use super::*;
    use crate::envelope::PersistedEnvelope;
    use crate::value::SchemaVersion;
    use bytes::Bytes;

    fn v(n: u64) -> Version {
        Version::new(n).expect("test version must be nonzero")
    }

    // ── per-section planner ──────────────────────────────────────────────────

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

    #[test]
    fn plan_empty_section_is_empty() {
        assert!(matches!(
            plan_section(&section("s", vec![])),
            Ok(SectionPlan::Empty)
        ));
    }

    #[test]
    fn plan_first_block_corrupt_is_first_corrupt() {
        let s = section("s", vec![ImportBlock::Corrupt, evt(1)]);
        assert!(matches!(plan_section(&s), Ok(SectionPlan::FirstCorrupt)));
    }

    #[test]
    fn plan_contiguous_run_from_one_is_complete() {
        let s = section("s", vec![evt(1), evt(2), evt(3)]);
        let plan = plan_section(&s).expect("plans");
        match plan {
            SectionPlan::Run {
                first,
                expected_version,
                last,
                halt,
                tail,
                ..
            } => {
                assert_eq!(first, v(1));
                assert_eq!(expected_version, None); // first == 1 → fresh stream
                assert_eq!(last, v(3));
                assert_eq!(tail.len(), 2, "head + 2 tail == the 3-event run");
                assert!(matches!(halt, Halt::Complete));
            }
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn plan_run_from_midstream_sets_expected_to_first_minus_one() {
        let s = section("s", vec![evt(3), evt(4)]);
        match plan_section(&s).expect("plans") {
            SectionPlan::Run {
                first,
                expected_version,
                last,
                halt,
                ..
            } => {
                assert_eq!(first, v(3));
                assert_eq!(expected_version, Some(v(2)));
                assert_eq!(last, v(4));
                assert!(matches!(halt, Halt::Complete));
            }
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn plan_internal_gap_halts_with_got() {
        // v3,v4,v6 → run [3,4], halt at the gap (got = 6).
        let s = section("s", vec![evt(3), evt(4), evt(6)]);
        match plan_section(&s).expect("plans") {
            SectionPlan::Run {
                last, halt, tail, ..
            } => {
                assert_eq!(last, v(4));
                assert_eq!(tail.len(), 1, "head + 1 tail == the 2-event run");
                assert!(matches!(halt, Halt::Gap { got } if got == v(6)));
            }
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn plan_internal_corrupt_halts_corrupt() {
        let s = section("s", vec![evt(1), evt(2), ImportBlock::Corrupt, evt(3)]);
        match plan_section(&s).expect("plans") {
            SectionPlan::Run {
                last, halt, tail, ..
            } => {
                assert_eq!(last, v(2));
                assert_eq!(tail.len(), 1, "head + 1 tail == the 2-event run");
                assert!(matches!(halt, Halt::Corrupt));
            }
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn plan_overflow_building_run_errors() {
        // last == u64::MAX with a following block forces prev.next() overflow.
        let s = section("s", vec![evt(u64::MAX), evt(1)]);
        assert!(matches!(plan_section(&s), Err(PlanError::VersionOverflow)));
    }

    #[test]
    fn plan_run_ending_at_u64_max_with_no_successor_completes() {
        // Complement of plan_overflow_building_run_errors: a run that ENDS at
        // u64::MAX with NO following block must complete — `last.next()` is never
        // called, so no spurious VersionOverflow. Guards against a refactor that
        // moves the overflow check to fire unconditionally.
        let s = section("s", vec![evt(u64::MAX)]);
        match plan_section(&s).expect("plans") {
            SectionPlan::Run {
                first,
                expected_version,
                tail,
                last,
                halt,
                ..
            } => {
                assert_eq!(first, v(u64::MAX));
                assert_eq!(expected_version, Some(v(u64::MAX - 1)));
                assert!(tail.is_empty(), "a one-event run is head-only");
                assert_eq!(last, v(u64::MAX));
                assert!(matches!(halt, Halt::Complete));
            }
            other => panic!("expected Run, got {other:?}"),
        }
    }

    // ── EventImporter PerStream behavioral tests ─────────────────────────────
}
