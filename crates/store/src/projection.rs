use core::iter;
use core::num::{NonZeroU32, NonZeroU64};

use mnesis::{DomainEvent, Id, Version};

use crate::checkpoint::{
    CheckpointError, CheckpointHydrated, CheckpointMode, CheckpointRejection, CheckpointStore,
    CheckpointWrite,
};
use crate::decoded::Decoded;
use crate::state::PersistTrigger;
use crate::store::AllPosition;
use crate::stream_id::StreamKey;

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

    /// Apply one event together with its origin-stream attribution, when the
    /// item carries one.
    ///
    /// `key` is `Some` iff the event arrived off an `$all` read — the origin
    /// [`StreamKey`] the store stamps beside every `$all` item (#333). On a
    /// per-stream fold it is `None`: there the stream id is the query argument
    /// the caller already holds, and the item carries no tag.
    ///
    /// The default ignores the key and delegates to [`apply`](Self::apply), so
    /// a single-stream projector implements only `apply`. A multi-stream
    /// projector that routes by origin stream overrides this method instead.
    /// [`Projection::advance`] calls only this method.
    ///
    /// # Errors
    ///
    /// Returns `Self::Error` when the event cannot be applied.
    fn apply_attributed(
        &self,
        state: Self::State,
        key: Option<&StreamKey>,
        event: &Self::Event,
    ) -> Result<Self::State, Self::Error> {
        let _ = key;
        self.apply(state, event)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Positioned — the stepper's input contract over both stream item shapes
// ═══════════════════════════════════════════════════════════════════════════

mod sealed {
    pub trait Sealed {}
}

/// A decoded stream item carrying the position the stepper checkpoints at.
///
/// The two shapes a decoded subscription yields (the typed duals of
/// [`RawItem`](crate::decoded::RawItem)):
///
/// - [`Decoded<E>`] (per-stream) — the bookmark is the `version` *inside*
///   the box; <code>Pos = [Version]</code>.
/// - `(P, StreamKey, Decoded<E>)` (`$all`) — the bookmark is the
///   [`AllPosition`] tag riding *beside* the box,
///   exactly as `.decoded()` yields it; `Pos = P`. The [`StreamKey`] flows to
///   [`Projector::apply_attributed`]: a multi-stream projector that routes by
///   origin stream overrides it; the defaulted method ignores the key and
///   delegates to [`Projector::apply`], so a single-stream projector is
///   untouched.
///
/// Sealed to accept the two decoded item shapes. Sealing does not prove that
/// an event came from storage: callers can construct decoded items. The
/// projection validates position ordering before folding each item.
pub trait Positioned: sealed::Sealed {
    /// The decoded event type carried by the item.
    type Event;
    /// The position type the stepper checkpoints at.
    type Pos: Copy + Ord + Send;
    /// Split the item into its bookmark, its origin-stream attribution
    /// (`$all` items only), and the decoded box.
    fn into_parts(self) -> (Self::Pos, Option<StreamKey>, Decoded<Self::Event>);
}

impl<E> sealed::Sealed for Decoded<E> {}
impl<E> Positioned for Decoded<E> {
    type Event = E;
    type Pos = Version;
    fn into_parts(self) -> (Version, Option<StreamKey>, Self) {
        (self.version, None, self)
    }
}

impl<E, P: AllPosition> sealed::Sealed for (P, StreamKey, Decoded<E>) {}
impl<E, P: AllPosition> Positioned for (P, StreamKey, Decoded<E>) {
    type Event = E;
    type Pos = P;
    fn into_parts(self) -> (P, Option<StreamKey>, Decoded<E>) {
        (self.0, Some(self.1), self.2)
    }
}

/// An event-driven projection that owns its state and checkpoint metadata.
///
/// The host owns the event loop. Decode subscription items before passing them
/// to [`advance`](Self::advance); both per-stream and `$all` items are accepted.
/// State is moved through the pure projector without cloning. Only a complete
/// successful fold is installed, and [`flush`](Self::flush) persists that state
/// together with the position it represents.
///
/// Positions must increase strictly; gaps are allowed for filtered streams.
/// Conditional writes prevent a stale worker from overwriting a newer revision.
/// A failed fold poisons this instance. A failed or canceled checkpoint write
/// requires reloading because it may have committed. Neither instance can be
/// reused to fold or flush. Ordinary loads reject schema mismatches; use
/// [`rebuild`](Self::rebuild) explicitly to replace a different schema.
///
/// ```ignore
/// let mut projection = Projection::load(id, projector, trigger, &checkpoints, schema).await?;
/// let stream = subscription
///     .subscribe(projection.id(), projection.checkpoint())?
///     .events()
///     .decoded(codec);
/// tokio::pin!(stream);
/// while let Some(item) = stream.next().await {
///     projection.advance(item?).await?;
/// }
/// projection.flush().await?;
/// ```
pub struct Projection<I, P: Projector, Trig, CS, Pos = Version> {
    id: I,
    projector: P,
    trigger: Trig,
    checkpoint_store: CS,
    schema_version: NonZeroU32,
    state: Option<P::State>,
    /// Last successfully acknowledged checkpoint revision.
    revision: Option<NonZeroU64>,
    /// Last successfully acknowledged durable position.
    checkpoint: Option<Pos>,
    /// Last successfully folded position, including the unpersisted tail.
    observed: Option<Pos>,
    /// Set before starting any write, cleared only after successful acknowledgment.
    reload_required: bool,
    /// Schema deliberately discarded when starting this rebuild.
    rebuilt_from: Option<NonZeroU32>,
}

impl<I, P, Trig, CS, Pos> Projection<I, P, Trig, CS, Pos>
where
    I: Id,
    P: Projector,
    Trig: PersistTrigger<Pos>,
    CS: CheckpointStore<P::State, Pos>,
    Pos: Copy + Ord + Send,
{
    /// Restore a matching checkpoint, or start fresh when none exists.
    ///
    /// # Errors
    /// Returns the adapter error on read failure. A different schema returns
    /// [`CheckpointRejection::SchemaMismatch`]; rebuilding must be explicit.
    pub async fn load(
        id: I,
        projector: P,
        trigger: Trig,
        checkpoint_store: CS,
        schema_version: NonZeroU32,
    ) -> Result<Self, CheckpointError<CS::Error>> {
        Self::assemble(
            id,
            projector,
            trigger,
            checkpoint_store,
            (schema_version, CheckpointMode::Advance),
        )
        .await
    }

    /// Start a deliberate rebuild of an existing, different schema.
    ///
    /// The old checkpoint remains intact until the first conditional write.
    /// A concurrent change rejects that write; revisions never reset. Replay
    /// from the beginning and retain the same instance across unpersisted folds.
    ///
    /// # Errors
    /// Returns an adapter read error, or [`CheckpointRejection::InvalidRebuild`]
    /// when the checkpoint is absent or already has the requested schema.
    pub async fn rebuild(
        id: I,
        projector: P,
        trigger: Trig,
        checkpoint_store: CS,
        schema_version: NonZeroU32,
    ) -> Result<Self, CheckpointError<CS::Error>> {
        Self::assemble(
            id,
            projector,
            trigger,
            checkpoint_store,
            (schema_version, CheckpointMode::Rebuild),
        )
        .await
    }

    async fn assemble(
        id: I,
        projector: P,
        trigger: Trig,
        checkpoint_store: CS,
        schema: (NonZeroU32, CheckpointMode),
    ) -> Result<Self, CheckpointError<CS::Error>> {
        let (schema_version, mode) = schema;
        let hydrated = checkpoint_store
            .hydrate_checkpoint(&id, schema_version)
            .await
            .map_err(CheckpointError::Store)?;
        let (state, checkpoint, revision, rebuilt_from) = match (mode, hydrated) {
            (
                CheckpointMode::Advance,
                CheckpointHydrated::Found {
                    revision,
                    position,
                    state,
                },
            ) => (state, Some(position), Some(revision), None),
            (CheckpointMode::Advance, CheckpointHydrated::Absent) => {
                (projector.initial(), None, None, None)
            }
            (CheckpointMode::Advance, CheckpointHydrated::Stale { stored_schema, .. }) => {
                return Err(CheckpointRejection::SchemaMismatch {
                    stored: stored_schema,
                    requested: schema_version,
                }
                .into());
            }
            (
                CheckpointMode::Rebuild,
                CheckpointHydrated::Stale {
                    revision,
                    stored_schema,
                },
            ) => (
                projector.initial(),
                None,
                Some(revision),
                Some(stored_schema),
            ),
            (
                CheckpointMode::Rebuild,
                CheckpointHydrated::Absent | CheckpointHydrated::Found { .. },
            ) => return Err(CheckpointRejection::InvalidRebuild.into()),
        };
        Ok(Self {
            id,
            projector,
            trigger,
            checkpoint_store,
            schema_version,
            state: Some(state),
            revision,
            checkpoint,
            observed: checkpoint,
            reload_required: false,
            rebuilt_from,
        })
    }

    /// The schema deliberately discarded by this instance's rebuild.
    #[must_use]
    pub const fn rebuilding_from(&self) -> Option<NonZeroU32> {
        self.rebuilt_from
    }

    /// Id used for checkpoint storage and subscription assembly.
    pub const fn id(&self) -> &I {
        &self.id
    }

    /// Last successfully acknowledged durable position.
    ///
    /// On a newly loaded instance this is the subscription resume point. After
    /// a write error or cancellation it remains diagnostic metadata; reload
    /// before opening a new subscription or continuing the fold.
    pub const fn checkpoint(&self) -> Option<Pos> {
        self.checkpoint
    }

    /// Last successfully folded position, including any unpersisted tail.
    ///
    /// Use this to reopen a cursor while continuing the same healthy instance.
    /// Reloaded instances instead start from their durable checkpoint.
    pub const fn observed(&self) -> Option<Pos> {
        self.observed
    }

    /// Borrow the owned read model while this instance is usable.
    ///
    /// # Errors
    /// A failed/panicking fold returns [`ProjectionStateError::Poisoned`]. A
    /// failed/canceled write returns [`ProjectionStateError::ReloadRequired`].
    pub fn state(&self) -> Result<&P::State, ProjectionStateError> {
        if self.reload_required {
            return Err(ProjectionStateError::ReloadRequired);
        }
        self.state.as_ref().ok_or(ProjectionStateError::Poisoned)
    }

    /// Validate ordering, fold one event, then persist if the trigger fires.
    ///
    /// # Errors
    /// Rejects duplicates/regressions before applying or changing state.
    /// Projector failure poisons this instance; checkpoint failure requires a
    /// reload. Dropping a pending write future has the same reload requirement.
    pub async fn advance<It>(
        &mut self,
        item: It,
    ) -> Result<(), ProjectionError<P::Error, CS::Error>>
    where
        It: Positioned<Event = P::Event, Pos = Pos>,
    {
        self.state()?;
        let (position, key, decoded) = item.into_parts();
        if self.observed.is_some_and(|previous| position <= previous) {
            return Err(ProjectionError::NonIncreasingPosition);
        }
        let state = self.state.take().ok_or(ProjectionStateError::Poisoned)?;
        let folded = self
            .projector
            .apply_attributed(state, key.as_ref(), &decoded.event)
            .map_err(ProjectionError::Apply)?;
        self.state = Some(folded);
        self.observed = Some(position);
        if self
            .trigger
            .should_persist(self.checkpoint, position, iter::once(decoded.event.name()))
        {
            self.flush().await?;
        }
        Ok(())
    }

    /// Persist the owned state and its unpersisted observed position together.
    /// A no-op when a healthy instance has no unpersisted events.
    ///
    /// # Errors
    /// Rejects poisoned or uncertain instances. A checkpoint failure requires
    /// reloading, even if the adapter rejected the write without committing.
    #[cfg_attr(feature = "tracing", tracing::instrument(name = "mnesis.projection.commit", level = "debug", skip_all, fields(id = %self.id)))]
    pub async fn flush(&mut self) -> Result<(), ProjectionError<P::Error, CS::Error>> {
        self.state()?;
        let Some(position) = self
            .observed
            .filter(|observed| Some(*observed) != self.checkpoint)
        else {
            return Ok(());
        };
        let state = self.state.as_ref().ok_or(ProjectionStateError::Poisoned)?;
        let mode = if self.checkpoint.is_none() && self.revision.is_some() {
            CheckpointMode::Rebuild
        } else {
            CheckpointMode::Advance
        };
        self.reload_required = true;
        let revision = self
            .checkpoint_store
            .commit_checkpoint(
                &self.id,
                CheckpointWrite {
                    expected: self.revision,
                    schema_version: self.schema_version,
                    position,
                    state,
                    mode,
                },
            )
            .await
            .map_err(ProjectionError::Commit)?;
        self.revision = Some(revision);
        self.checkpoint = Some(position);
        self.reload_required = false;
        Ok(())
    }
}

/// An instance whose state may no longer be used; recovery requires reloading.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ProjectionStateError {
    /// A consuming fold failed or panicked; no complete state remains.
    #[error("projection fold failed; reload required")]
    Poisoned,
    /// A checkpoint write failed or was canceled; its outcome may be uncertain.
    #[error("projection checkpoint outcome uncertain; reload required")]
    ReloadRequired,
}

/// Projection failures keep application, persistence and integrity domains separate.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ProjectionError<PErr, SErr> {
    /// The projector rejected the event; this instance is now poisoned.
    #[error("projector failed to apply event")]
    Apply(#[source] PErr),
    /// A checkpoint write failed or was rejected; reload this instance.
    #[error("checkpoint commit failed")]
    Commit(#[source] CheckpointError<SErr>),
    /// The instance cannot be reused after fold/write failure or cancellation.
    #[error(transparent)]
    State(#[from] ProjectionStateError),
    /// The input repeated or regressed the last successfully folded position.
    #[error("projection position did not increase")]
    NonIncreasingPosition,
}
