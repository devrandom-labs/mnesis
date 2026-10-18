//! Conditional persistence for projection state. Unlike aggregate snapshot
//! caches, a projection must reject stale writers and position regressions.

use core::future::Future;
use core::num::{NonZeroU32, NonZeroU64};

use mnesis::Id;

/// One atomic checkpoint read, including its revision for a subsequent write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckpointHydrated<S, P> {
    /// No checkpoint exists.
    Absent,
    /// A different schema exists. Its state must not be decoded as this schema.
    Stale {
        /// Revision that an explicit rebuild must replace conditionally.
        revision: NonZeroU64,
        /// Schema of the existing checkpoint.
        stored_schema: NonZeroU32,
    },
    /// State and position at the requested schema.
    Found {
        /// Revision that the next write must replace conditionally.
        revision: NonZeroU64,
        /// Last event represented by the state.
        position: P,
        /// Restored projection state.
        state: S,
    },
}

/// Whether a write advances the same schema or explicitly replaces another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckpointMode {
    /// Require the same schema and a strictly greater position.
    Advance,
    /// Require an existing, different schema; allow rebuilding from an earlier
    /// position, while still requiring the exact revision that was read.
    Rebuild,
}

/// A matching state/position pair and the revision it may replace.
pub struct CheckpointWrite<'a, S, P> {
    /// `None` permits creation only when no checkpoint exists.
    pub expected: Option<NonZeroU64>,
    /// Schema of the proposed state.
    pub schema_version: NonZeroU32,
    /// Last event represented by the proposed state.
    pub position: P,
    /// Proposed state, encoded before acquiring the writer lock when possible.
    pub state: &'a S,
    /// An explicit schema rebuild differs from ordinary advancement.
    pub mode: CheckpointMode,
}

impl<S, P: Copy + Ord> CheckpointWrite<'_, S, P> {
    /// Validate against metadata read under the adapter's writer lock and
    /// compute the next revision. The lock must remain held through the write.
    /// Revisions never reset on a schema change, preventing stale-writer reuse.
    ///
    /// # Errors
    /// Rejects stale revisions, schema/position violations and revision overflow.
    pub fn next_revision(
        &self,
        current: Option<(NonZeroU64, NonZeroU32, P)>,
    ) -> Result<NonZeroU64, CheckpointRejection> {
        let actual = current.map(|(revision, _, _)| revision);
        if self.expected != actual {
            return Err(CheckpointRejection::Conflict {
                expected: self.expected,
                actual,
            });
        }
        match (self.mode, current) {
            (CheckpointMode::Advance, Some((_, schema, position))) => {
                if schema != self.schema_version {
                    return Err(CheckpointRejection::SchemaMismatch {
                        stored: schema,
                        requested: self.schema_version,
                    });
                }
                if self.position <= position {
                    return Err(CheckpointRejection::NonIncreasingPosition);
                }
            }
            (CheckpointMode::Rebuild, Some((_, schema, _))) if schema != self.schema_version => {}
            (CheckpointMode::Rebuild, _) => return Err(CheckpointRejection::InvalidRebuild),
            (CheckpointMode::Advance, None) => {}
        }
        let next = actual.map_or(Some(1), |revision| revision.get().checked_add(1));
        next.and_then(NonZeroU64::new)
            .ok_or(CheckpointRejection::RevisionOverflow)
    }
}

/// Atomic conditional storage for projection checkpoints.
///
/// Each id has a monotonically increasing revision, independent of event
/// position and schema. Reads return one consistent record. Writes compare
/// the expected revision, validate schema/ordering and replace the entire
/// record in one transaction or critical section. No unconditional write or
/// deletion may bypass this contract in the checkpoint namespace.
///
/// An error or a canceled write may have committed. The caller must reload
/// before continuing; it must not infer an uncommitted outcome from an error.
pub trait CheckpointStore<S, P>: Send + Sync {
    /// Adapter-specific read/write failure.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Read one checkpoint, skipping state decoding on a schema mismatch.
    fn hydrate_checkpoint(
        &self,
        id: &impl Id,
        schema_version: NonZeroU32,
    ) -> impl Future<Output = Result<CheckpointHydrated<S, P>, Self::Error>> + Send;

    /// Compare and replace one complete checkpoint atomically.
    fn commit_checkpoint(
        &self,
        id: &impl Id,
        write: CheckpointWrite<'_, S, P>,
    ) -> impl Future<Output = Result<NonZeroU64, CheckpointError<Self::Error>>> + Send;
}

impl<S, P, CS: CheckpointStore<S, P>> CheckpointStore<S, P> for &CS {
    type Error = CS::Error;

    fn hydrate_checkpoint(
        &self,
        id: &impl Id,
        schema_version: NonZeroU32,
    ) -> impl Future<Output = Result<CheckpointHydrated<S, P>, Self::Error>> + Send {
        CS::hydrate_checkpoint(self, id, schema_version)
    }

    fn commit_checkpoint(
        &self,
        id: &impl Id,
        write: CheckpointWrite<'_, S, P>,
    ) -> impl Future<Output = Result<NonZeroU64, CheckpointError<Self::Error>>> + Send {
        CS::commit_checkpoint(self, id, write)
    }
}

/// A checkpoint write rejected without changing storage.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CheckpointRejection {
    /// Another writer changed the record, or creation raced with another writer.
    #[error("checkpoint revision changed: expected {expected:?}, found {actual:?}")]
    Conflict {
        /// Revision the caller loaded.
        expected: Option<NonZeroU64>,
        /// Revision stored under the writer lock.
        actual: Option<NonZeroU64>,
    },
    /// Schema replacement requires an explicit rebuild.
    #[error("checkpoint schema changed: stored {stored}, requested {requested}")]
    SchemaMismatch {
        /// Existing schema.
        stored: NonZeroU32,
        /// Proposed schema.
        requested: NonZeroU32,
    },
    /// Rebuild is only valid when replacing an existing, different schema.
    #[error("checkpoint rebuild requires an existing different schema")]
    InvalidRebuild,
    /// Same-schema writes must advance strictly, with gaps permitted.
    #[error("checkpoint position did not increase")]
    NonIncreasingPosition,
    /// The next revision cannot be represented; never wrap or reset it.
    #[error("checkpoint revision overflow")]
    RevisionOverflow,
}

/// Separate storage failures from conditional-write rejection.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CheckpointError<E> {
    /// Adapter failure; the commit outcome may be uncertain.
    #[error("checkpoint storage failed")]
    Store(#[source] E),
    /// The transaction rejected the proposal without changing storage.
    #[error(transparent)]
    Rejected(#[from] CheckpointRejection),
}
