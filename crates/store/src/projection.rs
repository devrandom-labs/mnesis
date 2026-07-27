use core::iter;
use core::num::NonZeroU32;

use mnesis::{DomainEvent, Id, Version};

use crate::decoded::Decoded;
use crate::state::{Hydrated, PersistTrigger, SnapshotStore};
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
/// Sealed on purpose: the pairing of position and event is **structural**.
/// A caller can never hand [`Projection::advance`] a position that did not
/// arrive with the event, so a committed checkpoint always describes the
/// state it is saved with — the same illegal-states-unrepresentable bet as
/// the atomic [`SnapshotStore::commit`].
pub trait Positioned: sealed::Sealed {
    /// The decoded event type carried by the item.
    type Event;
    /// The position type the stepper checkpoints at.
    type Pos: Copy + Send;
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
/// Per-stream (`Pos` defaults to [`Version`]):
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
/// `$all` (`Pos` = the adapter's [`AllPosition`]) is the
/// **same loop** — the `(position, StreamKey, Decoded)` tuple `.decoded()`
/// yields feeds [`advance`] whole; only the subscribe call and the snapshot
/// store's position type differ:
/// ```ignore
/// let (mut proj, mut state) =
///     Projection::load(id, projector, trigger, &snapshots, schema).await?;
/// let stream = subscription
///     .subscribe_all(proj.checkpoint())?
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
pub struct Projection<I, P: Projector, Trig, SS, Pos = Version> {
    id: I,
    projector: P,
    trigger: Trig,
    snapshot_store: SS,
    schema_version: NonZeroU32,
    /// Last position durably committed together with the state.
    checkpoint: Option<Pos>,
    /// Folded-but-not-yet-persisted tail position, flushed on shutdown.
    pending: Option<Pos>,
    /// `Some(old_schema)` iff `load` discarded a snapshot under a different
    /// schema version — the projection is re-folding from scratch. Surfaced via
    /// [`rebuilding_from`](Projection::rebuilding_from) so a host can distinguish
    /// a costly schema-bump rebuild from an ordinary fresh start.
    rebuilt_from: Option<NonZeroU32>,
}

impl<I, P, Trig, SS, Pos> Projection<I, P, Trig, SS, Pos>
where
    I: Id,
    P: Projector,
    Trig: PersistTrigger<Pos>,
    SS: SnapshotStore<P::State, Pos>,
    Pos: Copy + Send,
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

    /// The last durably-committed position — pass to `subscribe` (per-stream,
    /// `Pos = Version`) or `subscribe_all` (`Pos` = the adapter's
    /// [`AllPosition`]) as the resume point. `None` means
    /// "from the beginning".
    pub const fn checkpoint(&self) -> Option<Pos> {
        self.checkpoint
    }

    /// Fold one decoded event, then commit `(state, position)` together if the
    /// [`PersistTrigger`] fires. Returns the new state.
    ///
    /// Accepts either item shape a decoded stream yields (see [`Positioned`]):
    /// a bare [`Decoded<E>`](Decoded) from a per-stream subscription (the
    /// position is its `version`), or the `(position, StreamKey, Decoded<E>)`
    /// tuple from an `$all` subscription — fed whole, no unpacking (the stream
    /// key is forwarded to [`Projector::apply_attributed`]). The item's position
    /// becomes the candidate checkpoint. On a commit the checkpoint advances
    /// and the pending tail clears; otherwise the position is remembered as
    /// `pending` for the next [`flush`](Projection::flush).
    ///
    /// # Errors
    ///
    /// - [`ProjectionError::Apply`] if the projector rejects the event. The
    ///   consumed state is not recoverable (the fold owns it by value), so a
    ///   failed `advance` ends the projection — reload to resume.
    /// - [`ProjectionError::Commit`] if the snapshot commit fails.
    pub async fn advance<It>(
        &mut self,
        state: P::State,
        item: It,
    ) -> Result<P::State, ProjectionError<P::Error, SS::Error>>
    where
        It: Positioned<Event = P::Event, Pos = Pos>,
    {
        let (position, key, decoded) = item.into_parts();
        let folded = self
            .projector
            .apply_attributed(state, key.as_ref(), &decoded.event)
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
        position: Pos,
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
