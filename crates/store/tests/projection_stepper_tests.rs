//! Relocated inline test mod of `src/projection.rs` (mnesis-inmemory is a
//! dev-dependency; type unification with it requires an integration test).

#![cfg(feature = "projection")]

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        reason = "test module"
    )]

    use core::num::{NonZeroU32, NonZeroU64};

    use mnesis::{DomainEvent, Message, Version, version};

    use mnesis_inmemory::{InMemoryAllPos, InMemorySnapshotStore};
    use mnesis_store::decoded::Decoded;
    use mnesis_store::projection::{Projection, ProjectionError, Projector};
    use mnesis_store::state::{
        AfterEventTypes, EveryNEvents, Hydrated, PersistTrigger, SnapshotStore,
    };

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
        let (pos, st) = ss
            .hydrate(&id, NonZeroU32::MIN)
            .await
            .unwrap()
            .into_found()
            .unwrap();
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
        assert_eq!(
            ss.hydrate(&id, NonZeroU32::MIN).await.unwrap(),
            Hydrated::Absent
        );

        p.flush(&state).await.unwrap();
        assert_eq!(p.checkpoint(), Some(version!(2)));
        let (pos, st) = ss
            .hydrate(&id, NonZeroU32::MIN)
            .await
            .unwrap()
            .into_found()
            .unwrap();
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
        assert_eq!(
            ss.hydrate(&id, NonZeroU32::MIN).await.unwrap(),
            Hydrated::Absent
        );
    }

    // ── defensive: a schema-mismatched snapshot starts fresh, but flags a rebuild ─

    #[tokio::test]
    async fn load_stale_schema_starts_from_initial_and_signals_rebuild() {
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

        // Load with schema v2 — the v1 snapshot must not be restored...
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
        // ...but the rebuild is *visible*: a host can tell this apart from a
        // fresh start (which reports `None`) — the whole point of the Stale
        // signal. The invalidated schema version (v1) is reported.
        assert_eq!(p.rebuilding_from(), Some(NonZeroU32::MIN));
    }

    #[tokio::test]
    async fn load_fresh_does_not_signal_rebuild() {
        let ss = store();
        let (p, _state) = Projection::load(
            TestId("s"),
            CountingProjector,
            EveryNEvents(NonZeroU64::MIN),
            &ss,
            NonZeroU32::MIN,
        )
        .await
        .unwrap();
        // A genuinely-new projection is not a rebuild.
        assert_eq!(p.rebuilding_from(), None);
    }

    // ── $all duality (#327): the SAME stepper drives an $all projection ─────
    //
    // The $all position rides beside the item as `(P, Decoded<E>)` — exactly
    // what `.decoded()` yields on a subscribe_all stream — and feeds `advance`
    // whole. `InMemoryAllPos` stands in for fjall's `GlobalSeq`.

    /// `$all` trigger that always fires — the per-event-commit dual of
    /// `EveryNEvents(1)`, which is deliberately `Version`-only (#328).
    struct Always;
    impl PersistTrigger<InMemoryAllPos> for Always {
        fn should_persist(
            &self,
            _old_position: Option<InMemoryAllPos>,
            _new_position: InMemoryAllPos,
            _event_names: impl Iterator<Item: AsRef<str>>,
        ) -> bool {
            true
        }
    }

    fn all_pos(v: u64) -> InMemoryAllPos {
        InMemoryAllPos::new(v).expect("nonzero position")
    }

    fn all_store() -> InMemorySnapshotStore<CountState, InMemoryAllPos> {
        InMemorySnapshotStore::new()
    }

    // ── 1. Sequence/Protocol ($all): advance folds and checkpoints the tag ──

    #[tokio::test]
    async fn all_advance_accepts_tuple_items_and_checkpoints_the_all_tag() {
        let ss = all_store();
        let id = TestId("all");
        let (mut p, mut state) =
            Projection::load(id.clone(), CountingProjector, Always, &ss, NonZeroU32::MIN)
                .await
                .unwrap();
        assert_eq!(p.checkpoint(), None);

        // Gappy positions on purpose: $all is monotonic but NOT gapless
        // (aborted appends burn values) — the stepper checkpoints whatever
        // tag arrives. Inner versions are per-stream (two streams, both v1).
        state = p
            .advance(state, (all_pos(3), decoded(TestEvent::Added(10), 1)))
            .await
            .unwrap();
        state = p
            .advance(state, (all_pos(7), decoded(TestEvent::Added(20), 1)))
            .await
            .unwrap();

        assert_eq!(p.checkpoint(), Some(all_pos(7)));
        assert_eq!(
            state,
            CountState {
                count: 2,
                total: 30
            }
        );

        // Persisted together, atomically, under the $all position type.
        let (pos, st) = ss
            .hydrate(&id, NonZeroU32::MIN)
            .await
            .unwrap()
            .into_found()
            .unwrap();
        assert_eq!(pos, all_pos(7));
        assert_eq!(
            st,
            CountState {
                count: 2,
                total: 30
            }
        );
    }

    // ── 2. Lifecycle ($all): commit → reload → resume from the $all tag ─────

    #[tokio::test]
    async fn all_load_resumes_state_and_checkpoint_from_snapshot() {
        let ss = all_store();
        let id = TestId("all");
        {
            let (mut p, state) =
                Projection::load(id.clone(), CountingProjector, Always, &ss, NonZeroU32::MIN)
                    .await
                    .unwrap();
            let _ = p
                .advance(state, (all_pos(9), decoded(TestEvent::Added(10), 1)))
                .await
                .unwrap();
        }

        let (mut p2, state2) =
            Projection::load(id, CountingProjector, Always, &ss, NonZeroU32::MIN)
                .await
                .unwrap();
        assert_eq!(
            p2.checkpoint(),
            Some(all_pos(9)),
            "resume point is the $all tag, not a Version"
        );
        assert_eq!(
            state2,
            CountState {
                count: 1,
                total: 10
            }
        );

        // Resume folds onto the restored state at a later (gappy) position.
        let resumed = p2
            .advance(state2, (all_pos(12), decoded(TestEvent::Added(5), 2)))
            .await
            .unwrap();
        assert_eq!(p2.checkpoint(), Some(all_pos(12)));
        assert_eq!(
            resumed,
            CountState {
                count: 2,
                total: 15
            }
        );
    }

    // ── flush semantics ($all) + AfterEventTypes genericity in the stepper ──

    #[tokio::test]
    async fn all_flush_commits_tail_under_a_generic_after_event_types_trigger() {
        let ss = all_store();
        let id = TestId("all");
        // AfterEventTypes is position-generic (#328); "Removed" never arrives,
        // so only flush persists.
        let (mut p, mut state) = Projection::load(
            id.clone(),
            CountingProjector,
            AfterEventTypes::new(&["Removed"]),
            &ss,
            NonZeroU32::MIN,
        )
        .await
        .unwrap();

        state = p
            .advance(state, (all_pos(2), decoded(TestEvent::Added(10), 1)))
            .await
            .unwrap();
        assert_eq!(p.checkpoint(), None, "trigger must not have fired");
        assert_eq!(
            ss.hydrate(&id, NonZeroU32::MIN).await.unwrap(),
            Hydrated::Absent
        );

        p.flush(&state).await.unwrap();
        assert_eq!(p.checkpoint(), Some(all_pos(2)));
        let (pos, st) = ss
            .hydrate(&id, NonZeroU32::MIN)
            .await
            .unwrap()
            .into_found()
            .unwrap();
        assert_eq!(pos, all_pos(2));
        assert_eq!(
            st,
            CountState {
                count: 1,
                total: 10
            }
        );
    }
}
