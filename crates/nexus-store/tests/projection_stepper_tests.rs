//! Relocated inline test mod of `src/projection.rs` (nexus-inmemory is a
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

    use nexus::{DomainEvent, Message, Version, version};

    use nexus_inmemory::InMemorySnapshotStore;
    use nexus_store::decoded::Decoded;
    use nexus_store::projection::{Projection, ProjectionError, Projector};
    use nexus_store::state::{EveryNEvents, Hydrated, SnapshotStore};

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
}
