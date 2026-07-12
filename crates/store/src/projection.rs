use core::iter;
use core::num::NonZeroU32;

use mnesis::{DomainEvent, Id, Version};

use crate::decoded::Decoded;
use crate::state::{Hydrated, PersistTrigger, SnapshotStore};

/// A pure fold function over domain events.
///
/// Processes events one at a time to produce derived state. The
/// framework handles all IO (reading events, persisting state,
/// checkpointing). The projector is only responsible for computation.
///
/// Fallible: `apply` returns `Result` because projections may perform
/// checked arithmetic or encounter domain-specific edge cases.
/// Recovery policy (skip, fail, dead-letter) is handled by middleware
/// layers, not the projector itself.
///
/// # Comparison with `AggregateState`
///
/// `AggregateState::apply` is infallible because an aggregate always
/// applies its own events. A projector may process events from any
/// source, and may do derived computations (sums, counts) that can
/// overflow.
pub trait Projector: Send + Sync + 'static {
    /// The domain event type this projector handles.
    type Event: DomainEvent;

    /// The derived state produced by folding events.
    type State: Send + Sync + 'static;

    /// Error type for fallible projection logic.
    type Error: core::error::Error + Send + Sync + 'static;

    /// The initial state before any events have been applied.
    fn initial(&self) -> Self::State;

    /// Apply a single event to the current state, producing new state.
    ///
    /// Must use checked arithmetic for all computations. Return `Err`
    /// on overflow, underflow, or any domain-specific invariant
    /// violation. The framework decides recovery policy.
    ///
    /// # Errors
    ///
    /// Returns `Self::Error` when the event cannot be applied — e.g.,
    /// arithmetic overflow, underflow, or domain invariant violation.
    fn apply(&self, state: Self::State, event: &Self::Event) -> Result<Self::State, Self::Error>;
}

// ═══════════════════════════════════════════════════════════════════════════
// Projection<I, P, Trig, SS> — inert per-event assembly of the four primitives
// ═══════════════════════════════════════════════════════════════════════════

/// A per-event projection **stepper** — the ergonomic assembly of the four
/// projection primitives ([`Projector`], [`PersistTrigger`],
/// [`SnapshotStore`], and — outside this type — a [`Subscription`]).
///
/// It owns **no loop**. mnesis still ships no runner: the host drives the
/// stepper one event at a time. A tokio `while let` calls [`advance`] in its
/// body; a Bombay/Agency actor calls it from its message handler. Both shrink
/// to a single `advance` call and share no loop code, so nothing can drift.
///
/// The stepper holds only the *bookkeeping* — the last-persisted `checkpoint`
/// and the folded-but-unpersisted `pending` position — plus the persist
/// decision. **State flows through the caller**, not the stepper, because
/// [`Projector::apply`] consumes state by value: `load` hands back the starting
/// state, [`advance`] returns the next state, and [`flush`] takes the final
/// state by reference. This keeps the primitive Clone-free and unopinionated —
/// it decides *when* to persist and tracks *where* you are, and never owns your
/// read model.
///
/// The codec is **absent by construction**: decode the raw subscription with
/// [`StepStreamExt::decoded`](crate::StepStreamExt) /
/// [`DecodedStreamExt`](crate::DecodedStreamExt) *before* the event reaches
/// [`advance`], so this type never names a codec and never restates the
/// owning-codec `for<'a>` bound.
///
/// # Assembly (consumer-owned loop)
///
/// ```ignore
/// let (mut proj, mut state) =
///     Projection::load(id, projector, trigger, &snapshots, schema).await?;
/// let stream = subscription
///     .subscribe(proj.id(), proj.checkpoint())?
///     .events()
///     .decoded(codec);
/// tokio::pin!(stream);
/// while let Some(item) = stream.next().await {
///     state = proj.advance(state, item?).await?;
/// }
/// proj.flush(&state).await?;
/// ```
///
/// [`advance`]: Projection::advance
/// [`flush`]: Projection::flush
pub struct Projection<I, P: Projector, Trig, SS> {
    id: I,
    projector: P,
    trigger: Trig,
    snapshot_store: SS,
    schema_version: NonZeroU32,
    /// Last position durably committed together with the state.
    checkpoint: Option<Version>,
    /// Folded-but-not-yet-persisted tail position, flushed on shutdown.
    pending: Option<Version>,
    /// `Some(old_schema)` iff `load` discarded a snapshot under a different
    /// schema version — the projection is re-folding from scratch. Surfaced via
    /// [`rebuilding_from`](Projection::rebuilding_from) so a host can distinguish
    /// a costly schema-bump rebuild from an ordinary fresh start.
    rebuilt_from: Option<NonZeroU32>,
}

impl<I, P, Trig, SS> Projection<I, P, Trig, SS>
where
    I: Id,
    P: Projector,
    Trig: PersistTrigger,
    SS: SnapshotStore<P::State, Version>,
{
    /// Assemble and hydrate the stepper, returning it alongside the starting
    /// state.
    ///
    /// Resolves `(state, checkpoint)` from the snapshot store atomically:
    /// - [`Hydrated::Found`] → restore its state and position (resume).
    /// - [`Hydrated::Absent`] → [`Projector::initial`], no checkpoint (fresh).
    /// - [`Hydrated::Stale`] → `initial()`, no checkpoint, and
    ///   [`rebuilding_from`](Self::rebuilding_from) reports the discarded schema
    ///   version — a bump invalidated the saved state, so the projection re-folds
    ///   the whole stream. The host sees that instead of a silent full replay.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotStore::Error`] if hydration fails.
    pub async fn load(
        id: I,
        projector: P,
        trigger: Trig,
        snapshot_store: SS,
        schema_version: NonZeroU32,
    ) -> Result<(Self, P::State), SS::Error> {
        let (state, checkpoint, rebuilt_from) = match snapshot_store
            .hydrate(&id, schema_version)
            .await?
        {
            Hydrated::Found { position, state } => (state, Some(position), None),
            Hydrated::Absent => (projector.initial(), None, None),
            Hydrated::Stale { stored_schema } => (projector.initial(), None, Some(stored_schema)),
        };
        Ok((
            Self {
                id,
                projector,
                trigger,
                snapshot_store,
                schema_version,
                checkpoint,
                pending: None,
                rebuilt_from,
            },
            state,
        ))
    }

    /// `Some(old_schema)` when [`load`](Self::load) discarded a snapshot written
    /// under a different schema version — i.e. this projection is re-folding
    /// from the beginning because of a schema bump, not because it is new.
    /// `None` means it resumed from a checkpoint or started genuinely fresh
    /// (disambiguate those two via [`checkpoint`](Self::checkpoint)). A host on a
    /// constrained device can use this to warn/defer/throttle the rebuild.
    #[must_use]
    pub const fn rebuilding_from(&self) -> Option<NonZeroU32> {
        self.rebuilt_from
    }

    /// The id this projection is bound to — pass to `subscribe`.
    pub const fn id(&self) -> &I {
        &self.id
    }

    /// The last durably-committed position — pass to `subscribe` as the resume
    /// point. `None` means "from the beginning".
    pub const fn checkpoint(&self) -> Option<Version> {
        self.checkpoint
    }

    /// Fold one decoded event, then commit `(state, position)` together if the
    /// [`PersistTrigger`] fires. Returns the new state.
    ///
    /// The event's `version` becomes the candidate checkpoint. On a commit the
    /// checkpoint advances and the pending tail clears; otherwise the position
    /// is remembered as `pending` for the next [`flush`](Projection::flush).
    ///
    /// # Errors
    ///
    /// - [`ProjectionError::Apply`] if the projector rejects the event. The
    ///   consumed state is not recoverable (the fold owns it by value), so a
    ///   failed `advance` ends the projection — reload to resume.
    /// - [`ProjectionError::Commit`] if the snapshot commit fails.
    pub async fn advance(
        &mut self,
        state: P::State,
        decoded: Decoded<P::Event>,
    ) -> Result<P::State, ProjectionError<P::Error, SS::Error>> {
        let position = decoded.version;
        let folded = self
            .projector
            .apply(state, &decoded.event)
            .map_err(ProjectionError::Apply)?;

        if self
            .trigger
            .should_persist(self.checkpoint, position, iter::once(decoded.event.name()))
        {
            self.commit(position, &folded).await?;
        } else {
            self.pending = Some(position);
        }
        Ok(folded)
    }

    /// Commit the folded-but-unpersisted tail, if any.
    ///
    /// Call once when the host loop ends (shutdown, passivation) so a state
    /// folded past the last trigger is not lost. A no-op when nothing is
    /// pending.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectionError::Commit`] if the snapshot commit fails.
    pub async fn flush(
        &mut self,
        state: &P::State,
    ) -> Result<(), ProjectionError<P::Error, SS::Error>> {
        match self.pending {
            Some(position) => self.commit(position, state).await,
            None => Ok(()),
        }
    }

    /// Persist `(state, position)` atomically and advance the checkpoint.
    async fn commit(
        &mut self,
        position: Version,
        state: &P::State,
    ) -> Result<(), ProjectionError<P::Error, SS::Error>> {
        self.snapshot_store
            .commit(&self.id, self.schema_version, position, state)
            .await
            .map_err(ProjectionError::Commit)?;
        self.checkpoint = Some(position);
        self.pending = None;
        Ok(())
    }
}

/// Failure from [`Projection::advance`] / [`Projection::flush`] — the fold and
/// the persist are distinct domains and never share a variant (CLAUDE rule 3).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ProjectionError<PErr, SErr> {
    /// The projector rejected the event (overflow, invariant violation, …).
    #[error("projector failed to apply event")]
    Apply(#[source] PErr),
    /// The snapshot store failed to commit `(state, position)`.
    #[error("snapshot commit failed")]
    Commit(#[source] SErr),
}
