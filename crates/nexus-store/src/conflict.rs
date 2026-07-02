//! Shared optimistic-concurrency conflict predicate.
//!
//! Moved out of `saga.rs` so both [`SagaError`](crate::SagaError) and
//! `ExecuteError` delegate to the same
//! [`StoreError::is_conflict`](crate::StoreError::is_conflict) — one truth,
//! two callers.

use crate::error::StoreError;

mod sealed {
    pub trait Sealed {}
}

/// Predicate over a repository error: is this an optimistic-concurrency
/// conflict (and therefore retryable by reloading + re-deciding)?
///
/// Sealed: implemented inside this crate for [`StoreError`] only. Lets error
/// wrappers delegate without naming a concrete store error — `Snapshotting`'s
/// `Repository::Error` is the inner `StoreError`, so one impl serves bare and
/// snapshotted repositories alike.
pub trait ConflictPredicate: sealed::Sealed {
    /// `true` iff this error is an optimistic-concurrency conflict.
    fn is_conflict(&self) -> bool;
}

impl<A, EncErr, DecErr> sealed::Sealed for StoreError<A, EncErr, DecErr> {}

impl<A, EncErr, DecErr> ConflictPredicate for StoreError<A, EncErr, DecErr> {
    fn is_conflict(&self) -> bool {
        Self::is_conflict(self)
    }
}
