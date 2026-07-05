use core::iter;
use core::num::NonZeroU32;

use nexus::{DomainEvent, Id, Version};

use crate::decoded::Decoded;
use crate::state::{PersistTrigger, SnapshotStore};

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
/// It owns **no loop**. nexus still ships no runner: the host drives the
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
    /// Resolves `(state, checkpoint)` from the snapshot store atomically. If a
    /// snapshot exists for `id` at `schema_version`, its state and position are
    /// restored; otherwise the state is [`Projector::initial`] and the
    /// checkpoint is `None` (subscribe from the beginning). A snapshot saved
    /// under a *different* schema version is invisible — a schema bump forces a
    /// full replay from `initial()`.
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
        let (state, checkpoint) = match snapshot_store.hydrate(&id, schema_version).await? {
            Some((position, state)) => (state, Some(position)),
            None => (projector.initial(), None),
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
            },
            state,
        ))
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

#[cfg(all(test, feature = "testing"))]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        reason = "test module"
    )]

    use core::num::{NonZeroU32, NonZeroU64};

    use nexus::{DomainEvent, Message, Version, version};

    use super::{Projection, ProjectionError, Projector};
    use crate::decoded::Decoded;
    use crate::state::{EveryNEvents, InMemorySnapshotStore, SnapshotStore};

    // ── fixtures ────────────────────────────────────────────────────────────

    #[derive(Debug, Clone, Hash, PartialEq, Eq)]
    struct TestId(&'static str);
    impl core::fmt::Display for TestId {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.write_str(self.0)
        }
    }
    impl AsRef<[u8]> for TestId {
        fn as_ref(&self) -> &[u8] {
            self.0.as_bytes()
        }
    }

    #[derive(Debug, Clone, PartialEq)]
    struct CountState {
        count: u64,
        total: u64,
    }

    #[derive(Debug)]
    enum TestEvent {
        Added(u64),
        Removed(u64),
    }
    impl Message for TestEvent {}
    impl DomainEvent for TestEvent {
        fn name(&self) -> &'static str {
            match self {
                Self::Added(_) => "Added",
                Self::Removed(_) => "Removed",
            }
        }
    }

    #[derive(Debug, thiserror::Error)]
    #[error("projection overflow")]
    struct TestProjectionError;

    struct CountingProjector;
    impl Projector for CountingProjector {
        type Event = TestEvent;
        type State = CountState;
        type Error = TestProjectionError;

        fn initial(&self) -> CountState {
            CountState { count: 0, total: 0 }
        }
        fn apply(
            &self,
            state: CountState,
            event: &TestEvent,
        ) -> Result<CountState, TestProjectionError> {
            let count = state.count.checked_add(1).ok_or(TestProjectionError)?;
            let total = match event {
                TestEvent::Added(n) => state.total.checked_add(*n).ok_or(TestProjectionError)?,
                TestEvent::Removed(n) => state.total.checked_sub(*n).ok_or(TestProjectionError)?,
            };
            Ok(CountState { count, total })
        }
    }

    /// Build a `Decoded` event at a given version, the way `.decoded()` would.
    const fn decoded(event: TestEvent, ver: u64) -> Decoded<TestEvent> {
        Decoded {
            event,
            version: Version::new(ver).expect("nonzero version"),
            metadata: None,
        }
    }

    fn store() -> InMemorySnapshotStore<CountState, Version> {
        InMemorySnapshotStore::new()
    }

    // ── 1. Sequence/Protocol: load → advance×n commits and folds ────────────

    #[tokio::test]
    async fn advance_folds_and_commits_each_event_when_trigger_always_fires() {
        let ss = store();
        let id = TestId("s");
        let (mut p, mut state) = Projection::load(
            id.clone(),
            CountingProjector,
            EveryNEvents(NonZeroU64::MIN),
            &ss,
            NonZeroU32::MIN,
        )
        .await
        .unwrap();

        assert_eq!(p.checkpoint(), None);
        state = p
            .advance(state, decoded(TestEvent::Added(10), 1))
            .await
            .unwrap();
        state = p
            .advance(state, decoded(TestEvent::Added(20), 2))
            .await
            .unwrap();
        state = p
            .advance(state, decoded(TestEvent::Added(30), 3))
            .await
            .unwrap();

        assert_eq!(p.checkpoint(), Some(version!(3)));
        assert_eq!(
            state,
            CountState {
                count: 3,
                total: 60
            }
        );

        // Persisted together, atomically.
        let (pos, st) = ss.hydrate(&id, NonZeroU32::MIN).await.unwrap().unwrap();
        assert_eq!(pos, version!(3));
        assert_eq!(
            st,
            CountState {
                count: 3,
                total: 60
            }
        );
    }

    // ── 2. Lifecycle: commit → reload → resume from checkpoint ───────────────

    #[tokio::test]
    async fn load_resumes_state_and_checkpoint_from_snapshot() {
        let ss = store();
        let id = TestId("s");

        {
            let (mut p, mut state) = Projection::load(
                id.clone(),
                CountingProjector,
                EveryNEvents(NonZeroU64::MIN),
                &ss,
                NonZeroU32::MIN,
            )
            .await
            .unwrap();
            state = p
                .advance(state, decoded(TestEvent::Added(10), 1))
                .await
                .unwrap();
            state = p
                .advance(state, decoded(TestEvent::Added(20), 2))
                .await
                .unwrap();
            let _ = p
                .advance(state, decoded(TestEvent::Added(30), 3))
                .await
                .unwrap();
        }

        // Fresh stepper over the same snapshot store: state + checkpoint restored.
        let (mut p2, state2) = Projection::load(
            id.clone(),
            CountingProjector,
            EveryNEvents(NonZeroU64::MIN),
            &ss,
            NonZeroU32::MIN,
        )
        .await
        .unwrap();
        assert_eq!(p2.checkpoint(), Some(version!(3)));
        assert_eq!(
            state2,
            CountState {
                count: 3,
                total: 60
            }
        );

        // Resume: the next event folds onto the restored state.
        let resumed = p2
            .advance(state2, decoded(TestEvent::Added(40), 4))
            .await
            .unwrap();
        assert_eq!(p2.checkpoint(), Some(version!(4)));
        assert_eq!(
            resumed,
            CountState {
                count: 4,
                total: 100
            }
        );
    }

    // ── 3. Defensive boundary: a failing apply surfaces as ::Apply ───────────

    #[tokio::test]
    async fn advance_surfaces_projector_apply_error() {
        let ss = store();
        let (mut p, state) = Projection::load(
            TestId("s"),
            CountingProjector,
            EveryNEvents(NonZeroU64::MIN),
            &ss,
            NonZeroU32::MIN,
        )
        .await
        .unwrap();

        // Removed(1) from total 0 underflows in the projector.
        let err = p
            .advance(state, decoded(TestEvent::Removed(1), 1))
            .await
            .unwrap_err();
        assert!(
            matches!(err, ProjectionError::Apply(_)),
            "expected Apply, got {err:?}"
        );
        // Nothing was committed on the failed fold.
        assert_eq!(p.checkpoint(), None);
    }

    // ── flush semantics: tail is committed once on shutdown ──────────────────

    #[tokio::test]
    async fn flush_commits_folded_but_unpersisted_tail() {
        let ss = store();
        let id = TestId("s");
        // Trigger never fires (bucket of 100) — only flush persists.
        let (mut p, mut state) = Projection::load(
            id.clone(),
            CountingProjector,
            EveryNEvents(NonZeroU64::new(100).unwrap()),
            &ss,
            NonZeroU32::MIN,
        )
        .await
        .unwrap();

        state = p
            .advance(state, decoded(TestEvent::Added(10), 1))
            .await
            .unwrap();
        state = p
            .advance(state, decoded(TestEvent::Added(20), 2))
            .await
            .unwrap();
        assert_eq!(p.checkpoint(), None, "trigger must not have fired");
        assert!(ss.hydrate(&id, NonZeroU32::MIN).await.unwrap().is_none());

        p.flush(&state).await.unwrap();
        assert_eq!(p.checkpoint(), Some(version!(2)));
        let (pos, st) = ss.hydrate(&id, NonZeroU32::MIN).await.unwrap().unwrap();
        assert_eq!(pos, version!(2));
        assert_eq!(
            st,
            CountState {
                count: 2,
                total: 30
            }
        );
    }

    #[tokio::test]
    async fn flush_is_a_noop_when_nothing_is_pending() {
        let ss = store();
        let id = TestId("s");
        let (mut p, state) = Projection::load(
            id.clone(),
            CountingProjector,
            EveryNEvents(NonZeroU64::MIN),
            &ss,
            NonZeroU32::MIN,
        )
        .await
        .unwrap();

        // Never advanced: flush must not write a spurious snapshot.
        p.flush(&state).await.unwrap();
        assert_eq!(p.checkpoint(), None);
        assert!(ss.hydrate(&id, NonZeroU32::MIN).await.unwrap().is_none());
    }

    // ── defensive: a schema-mismatched snapshot is invisible → start fresh ───

    #[tokio::test]
    async fn load_ignores_stale_schema_and_starts_from_initial() {
        let ss = store();
        let id = TestId("s");
        // Pre-commit a v1 snapshot.
        ss.commit(
            &id,
            NonZeroU32::MIN,
            version!(5),
            &CountState {
                count: 99,
                total: 999,
            },
        )
        .await
        .unwrap();

        // Load with schema v2 — the v1 snapshot must be invisible.
        let (p, state) = Projection::load(
            id,
            CountingProjector,
            EveryNEvents(NonZeroU64::MIN),
            &ss,
            NonZeroU32::new(2).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(p.checkpoint(), None);
        assert_eq!(state, CountState { count: 0, total: 0 });
    }
}
