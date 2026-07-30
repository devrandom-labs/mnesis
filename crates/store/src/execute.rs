//! Store-side command combinator — the aggregate analogue of
//! [`SagaRepository`](crate::SagaRepository).
//!
//! [`CommandRepository::execute`] fuses `decide → save` into one call so the
//! decided events can't be forgotten or misthreaded (#251). It adds no
//! persistence machinery — it is the "imperative shell" over the pure
//! [`AggregateRoot::handle`](mnesis::AggregateRoot::handle) and the atomic
//! [`Repository::save`](crate::Repository::save).
//!
//! See `docs/plans/2026-07-02-execute-command-combinator-design.md`.

use core::fmt;
use core::future::Future;

use mnesis::{Aggregate, AggregateRoot, DomainEvent, EventOf, Events, Handle};

use crate::conflict::ConflictPredicate;
use crate::repository::Repository;

/// Error from a command `execute`. Two failure domains kept distinct
/// (CLAUDE.md rule 3 — one variant = one domain).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ExecuteError<DecideErr, StoreErr> {
    /// The aggregate rejected the command (a domain invariant). Nothing persisted.
    #[error("command rejected: {0}")]
    Decide(#[source] DecideErr),

    /// `save` failed (adapter / codec / conflict / version overflow).
    #[error(transparent)]
    Store(StoreErr),
}

impl<DecideErr, StoreErr: ConflictPredicate> ExecuteError<DecideErr, StoreErr> {
    /// `true` iff the save failed on an optimistic-concurrency conflict.
    /// `Decide` is never a conflict (rule 3 — rejection is not retryable).
    #[must_use]
    pub fn is_conflict(&self) -> bool {
        matches!(self, Self::Store(e) if e.is_conflict())
    }
}

/// Outcome of one [`CommandRepository::execute`] — the aggregate-side dual of
/// [`Reaction`](crate::Reaction).
///
/// Two variants, exactly mirroring [`Handle::handle`]'s own `Option`: a no-op
/// decision persists nothing and has no position; an accepted command's events
/// are durable and carry the `$all` position they landed at.
///
/// `#[must_use]`: the `Executed` position is the read-your-writes token — a
/// caller that reads its own write back needs it (#330).
#[must_use = "the read-your-writes position and the decided events should be inspected"]
pub enum Execution<A: Aggregate, P, const N: usize> {
    /// [`Handle::handle`] returned `Ok(None)` — accepted, decided nothing.
    /// **No append was issued**, so `root` keeps its version, no `GlobalSeq`
    /// is burned, and there is no position. Read the unchanged state off `root`.
    Ignored,
    /// The command was accepted and its events are durable.
    Executed {
        /// The `$all` position the **last** decided event landed at — the
        /// read-your-writes token. A projection whose checkpoint has reached it
        /// has necessarily observed this whole append.
        position: P,
        /// The decided events, for inspection.
        events: Events<EventOf<A>, N>,
    },
}

// Manual Debug: `A` is a bare marker (never `Debug`); its event type and the
// position are, so no extra bound leaks onto the marker.
impl<A: Aggregate, P: fmt::Debug, const N: usize> fmt::Debug for Execution<A, P, N>
where
    EventOf<A>: DomainEvent,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ignored => f.write_str("Ignored"),
            Self::Executed { position, events } => f
                .debug_struct("Executed")
                .field("position", position)
                .field("events", events)
                .finish(),
        }
    }
}

/// The command-facing port: `decide → save` as one callable transaction.
///
/// Extends [`Repository<A>`] and inherits its `load`/`save` unchanged. The one
/// provided method rides on every repository via the blanket impl below — bare
/// [`EventStore`](crate::EventStore) and the
/// [`Snapshotting`](crate::snapshot::Snapshotting) decorator alike.
pub trait CommandRepository<A: Aggregate>: Repository<A> {
    /// Decide `command` against `root`, persist the decided events atomically,
    /// advance `root`, and return an [`Execution`] carrying the read-your-writes
    /// position and the decided events.
    ///
    /// - `Ok(Execution::Executed { position, events })` — accepted and durable;
    ///   `position` is the `$all` position the last event landed at (#330).
    /// - `Ok(Execution::Ignored)` — the command was accepted and decided nothing
    ///   ([`Handle::handle`] returned `Ok(None)`); **no append is issued**, so
    ///   `root` keeps its version, no `GlobalSeq` is burned, and a fresh
    ///   aggregate stays streamless. Read the unchanged state off `root`.
    /// - `Err(ExecuteError::Decide)` — the aggregate rejected it; nothing persisted.
    /// - `Err(ExecuteError::Store)` — the save failed (see [`ExecuteError::is_conflict`]).
    ///
    /// [`Execution`] is a two-variant enum rather than [`Handle`]'s bare
    /// `Option` (#330): once `save` returns a position, the accepted branch has
    /// a second field to pair with the events — the exact symmetry with the
    /// saga side's [`Reaction`](crate::Reaction). The no-op branch stays
    /// positionless, since nothing was appended.
    ///
    /// On a version conflict this returns `Err(ExecuteError::Store(..))` with
    /// `is_conflict() == true` and does **not** retry — retry is the runtime's
    /// job (CLAUDE.md rule 5), matching `SagaRepository::react_and_save`.
    ///
    /// # Errors
    /// See the variants above.
    #[allow(
        clippy::type_complexity,
        reason = "the Execution-or-typed-error return is intrinsic to the contract; an \
                  alias would hide the `impl Future`/`Send` capture the API depends on"
    )]
    fn execute<C, const N: usize>(
        &self,
        root: &mut AggregateRoot<A>,
        command: C,
    ) -> impl Future<
        Output = Result<Execution<A, Self::Position, N>, ExecuteError<A::Error, Self::Error>>,
    > + Send
    where
        A: Handle<C, N>,
        C: Send,
    {
        execute_inner(self, root, command)
    }
}

/// Inner body of [`CommandRepository::execute`] — extracted so the
/// `mnesis.aggregate.execute` span can attach to an `async fn` (times the
/// future's polling, not the construction of the `impl Future`). The
/// `tracing::Instrument` combinator shape trips this workspace's deny-level
/// `shadow_reuse`/`let_and_return` lints; a private `async fn` carrying
/// `#[cfg_attr(feature = "tracing", ...)]` is lint-clean.
#[allow(
    clippy::type_complexity,
    reason = "the Execution-or-typed-error return is the same intrinsic contract as the trait method; \
              an alias would hide the `impl Future`/`Send` capture the API depends on"
)]
#[cfg_attr(
    feature = "tracing",
    tracing::instrument(
        name = "mnesis.aggregate.execute",
        level = "debug",
        skip_all,
        fields(
            aggregate = core::any::type_name::<A>(),
            stream = %root.id()
        )
    )
)]
async fn execute_inner<A, R, C, const N: usize>(
    repo: &R,
    root: &mut AggregateRoot<A>,
    command: C,
) -> Result<
    Execution<A, <R as Repository<A>>::Position, N>,
    ExecuteError<A::Error, <R as Repository<A>>::Error>,
>
where
    A: Aggregate + Handle<C, N>,
    R: Repository<A> + ?Sized,
    C: Send,
{
    // A no-op decision never reaches the store: no append, no version,
    // no GlobalSeq burned.
    match root.handle::<C, N>(command).map_err(ExecuteError::Decide)? {
        None => Ok(Execution::Ignored),
        Some(decided) => {
            let position = repo
                .save(root, &decided)
                .await
                .map_err(ExecuteError::Store)?;
            Ok(Execution::Executed {
                position,
                events: decided,
            })
        }
    }
}

// Rides on every repository — bare `EventStore` AND the `Snapshotting`
// decorator — with zero per-type code. Fully static dispatch.
impl<A: Aggregate, R: Repository<A>> CommandRepository<A> for R {}

#[cfg(test)]
mod error_tests {
    use super::ExecuteError;
    use crate::error::StoreError;
    use mnesis::{ErrorId, Version};

    type TestStoreError =
        StoreError<std::io::Error, std::convert::Infallible, std::convert::Infallible>;
    type TestExecuteError = ExecuteError<&'static str, TestStoreError>;

    #[test]
    fn conflict_store_error_is_conflict() {
        let e: TestExecuteError = ExecuteError::Store(StoreError::Conflict {
            stream_id: ErrorId::from_display(&"s"),
            expected: Some(Version::INITIAL),
            actual: None,
        });
        assert!(e.is_conflict());
    }

    #[test]
    fn decide_error_is_not_conflict() {
        let e: TestExecuteError = ExecuteError::Decide("rejected");
        assert!(!e.is_conflict());
    }
}
