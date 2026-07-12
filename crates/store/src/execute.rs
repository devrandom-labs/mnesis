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

use core::future::Future;

use mnesis::{Aggregate, AggregateRoot, EventOf, Events, Handle};

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

/// The command-facing port: `decide → save` as one callable transaction.
///
/// Extends [`Repository<A>`] and inherits its `load`/`save` unchanged. The one
/// provided method rides on every repository via the blanket impl below — bare
/// [`EventStore`](crate::EventStore) and the
/// [`Snapshotting`](crate::snapshot::Snapshotting) decorator alike.
pub trait CommandRepository<A: Aggregate>: Repository<A> {
    /// Decide `command` against `root`, persist the decided events atomically,
    /// advance `root`, and return the decided events for inspection.
    ///
    /// - `Ok(events)` — the command was accepted and its events are durable.
    /// - `Err(ExecuteError::Decide)` — the aggregate rejected it; nothing persisted.
    /// - `Err(ExecuteError::Store)` — the save failed (see [`ExecuteError::is_conflict`]).
    ///
    /// On a version conflict this returns `Err(ExecuteError::Store(..))` with
    /// `is_conflict() == true` and does **not** retry — retry is the runtime's
    /// job (CLAUDE.md rule 5), matching `SagaRepository::react_and_save`.
    ///
    /// # Errors
    /// See the variants above.
    #[allow(
        clippy::type_complexity,
        reason = "the decided-events-or-typed-error return is intrinsic to the contract; an \
                  alias would hide the `impl Future`/`Send` capture the API depends on"
    )]
    fn execute<C, const N: usize>(
        &self,
        root: &mut AggregateRoot<A>,
        command: C,
    ) -> impl Future<Output = Result<Events<EventOf<A>, N>, ExecuteError<A::Error, Self::Error>>> + Send
    where
        A: Handle<C, N>,
        C: Send,
    {
        async move {
            let decided = root.handle::<C, N>(command).map_err(ExecuteError::Decide)?;
            self.save(root, &decided)
                .await
                .map_err(ExecuteError::Store)?;
            Ok(decided)
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
