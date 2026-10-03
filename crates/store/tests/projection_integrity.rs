#![cfg(all(feature = "projection", feature = "json"))]
#![allow(clippy::unwrap_used, reason = "projection integrity assertions")]

use core::num::{NonZeroU32, NonZeroU64};
use futures::FutureExt;
use mnesis::{DomainEvent, Id, Message, Version};
use mnesis_inmemory::InMemoryCheckpointStore;
use mnesis_store::checkpoint::{
    CheckpointError, CheckpointHydrated, CheckpointRejection, CheckpointStore, CheckpointWrite,
};
use mnesis_store::state::{AfterEventTypes, CodecSnapshotStore};
use mnesis_store::{
    Decoded, JsonCodec, Projection, ProjectionError, ProjectionStateError, Projector, StreamKey,
};
use std::sync::atomic::{AtomicBool, Ordering};

// Deliberately not Clone: the projection must own and move state through folds.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct OwnedState(u64);

#[derive(Debug)]
enum Event {
    Add(u64),
    Fail,
    Panic,
}
impl Message for Event {}
impl DomainEvent for Event {
    fn name(&self) -> &'static str {
        "event"
    }
}

#[derive(Debug, thiserror::Error)]
#[error("fold failed")]
struct FoldError;

struct Sum;
impl Projector for Sum {
    type Event = Event;
    type State = OwnedState;
    type Error = FoldError;

    fn initial(&self) -> OwnedState {
        OwnedState(0)
    }

    #[allow(
        clippy::panic,
        reason = "inject an application panic during a consuming fold"
    )]
    fn apply(&self, state: OwnedState, event: &Event) -> Result<OwnedState, FoldError> {
        match event {
            Event::Add(value) => Ok(OwnedState(state.0.checked_add(*value).ok_or(FoldError)?)),
            Event::Fail => Err(FoldError),
            Event::Panic => panic!("injected application panic"),
        }
    }
}

const fn item(event: Event, position: u64) -> Decoded<Event> {
    Decoded {
        event,
        version: Version::new(position).unwrap(),
        metadata: None,
    }
}

#[tokio::test]
async fn decreasing_and_duplicate_positions_do_not_consume_state_or_pending_tail() {
    let bytes = InMemoryCheckpointStore::<Vec<u8>, Version>::new();
    let codec = CodecSnapshotStore::new(&bytes, JsonCodec::default());
    let id = StreamKey::from_slice(b"ordered");
    let mut projection = Projection::load(
        id.clone(),
        Sum,
        AfterEventTypes::new(&[]),
        &codec,
        NonZeroU32::MIN,
    )
    .await
    .unwrap();
    projection.advance(item(Event::Add(5), 3)).await.unwrap();
    for position in [3, 2, 1] {
        // An invalid position must be rejected before this failing fold runs.
        assert!(matches!(
            projection.advance(item(Event::Fail, position)).await,
            Err(ProjectionError::NonIncreasingPosition)
        ));
        assert_eq!(projection.state().unwrap().0, 5);
        assert_eq!(projection.observed(), Version::new(3));
        assert_eq!(projection.checkpoint(), None);
    }
    projection.flush().await.unwrap();
    projection.flush().await.unwrap();
    let restored = Projection::load(id, Sum, AfterEventTypes::new(&[]), &codec, NonZeroU32::MIN)
        .await
        .unwrap();
    assert_eq!(restored.state().unwrap().0, 5);
    assert_eq!(restored.checkpoint(), Version::new(3));
}

#[tokio::test]
async fn consuming_fold_failure_and_panic_require_reload_from_the_last_checkpoint() {
    for failing_event in [Event::Fail, Event::Panic] {
        let bytes = InMemoryCheckpointStore::<Vec<u8>, Version>::new();
        let codec = CodecSnapshotStore::new(&bytes, JsonCodec::default());
        let id = StreamKey::from_slice(b"fold-failure");
        let mut projection = Projection::load(
            id.clone(),
            Sum,
            AfterEventTypes::new(&["event"]),
            &codec,
            NonZeroU32::MIN,
        )
        .await
        .unwrap();
        projection.advance(item(Event::Add(10), 1)).await.unwrap();
        let should_panic = matches!(failing_event, Event::Panic);
        let failed = std::panic::AssertUnwindSafe(projection.advance(item(failing_event, 2)))
            .catch_unwind()
            .await;
        if should_panic {
            assert!(failed.is_err());
        } else {
            assert!(matches!(failed, Ok(Err(ProjectionError::Apply(FoldError)))));
        }
        assert_eq!(
            projection.state().unwrap_err(),
            ProjectionStateError::Poisoned
        );
        assert_eq!(projection.observed(), Version::new(1));
        assert_eq!(projection.checkpoint(), Version::new(1));
        assert!(matches!(
            projection.advance(item(Event::Add(20), 2)).await,
            Err(ProjectionError::State(ProjectionStateError::Poisoned))
        ));
        assert!(matches!(
            projection.flush().await,
            Err(ProjectionError::State(ProjectionStateError::Poisoned))
        ));
        let mut restored = Projection::load(
            id,
            Sum,
            AfterEventTypes::new(&["event"]),
            &codec,
            NonZeroU32::MIN,
        )
        .await
        .unwrap();
        assert_eq!(restored.state().unwrap().0, 10);
        restored.advance(item(Event::Add(20), 2)).await.unwrap();
        assert_eq!(restored.state().unwrap().0, 30);
        assert_eq!(restored.checkpoint(), Version::new(2));
    }
}

#[tokio::test]
async fn stale_projection_writer_cannot_overwrite_a_newer_complete_checkpoint() {
    let bytes = InMemoryCheckpointStore::<Vec<u8>, Version>::new();
    let codec = CodecSnapshotStore::new(&bytes, JsonCodec::default());
    let id = StreamKey::from_slice(b"writers");
    let mut first = Projection::load(
        id.clone(),
        Sum,
        AfterEventTypes::new(&["event"]),
        &codec,
        NonZeroU32::MIN,
    )
    .await
    .unwrap();
    let mut stale = Projection::load(
        id.clone(),
        Sum,
        AfterEventTypes::new(&["event"]),
        &codec,
        NonZeroU32::MIN,
    )
    .await
    .unwrap();
    first.advance(item(Event::Add(10), 1)).await.unwrap();
    first.advance(item(Event::Add(20), 2)).await.unwrap();
    assert!(matches!(
        stale.advance(item(Event::Add(999), 3)).await,
        Err(ProjectionError::Commit(CheckpointError::Rejected(
            CheckpointRejection::Conflict { .. }
        )))
    ));
    assert_eq!(
        stale.state().unwrap_err(),
        ProjectionStateError::ReloadRequired
    );
    let restored = Projection::load(
        id,
        Sum,
        AfterEventTypes::new(&["event"]),
        &codec,
        NonZeroU32::MIN,
    )
    .await
    .unwrap();
    assert_eq!(restored.state().unwrap().0, 30);
    assert_eq!(restored.checkpoint(), Version::new(2));
}

#[tokio::test]
async fn schema_rebuild_requires_explicit_intent_and_invalidates_old_writers() {
    let bytes = InMemoryCheckpointStore::<Vec<u8>, Version>::new();
    let codec = CodecSnapshotStore::new(&bytes, JsonCodec::default());
    let id = StreamKey::from_slice(b"schemas");
    let old = NonZeroU32::MIN;
    let new = NonZeroU32::new(2).unwrap();
    let mut original = Projection::load(
        id.clone(),
        Sum,
        AfterEventTypes::new(&["event"]),
        &codec,
        old,
    )
    .await
    .unwrap();
    original.advance(item(Event::Add(50), 5)).await.unwrap();
    assert!(matches!(
        Projection::load(id.clone(), Sum, AfterEventTypes::new(&[]), &codec, new).await,
        Err(CheckpointError::Rejected(
            CheckpointRejection::SchemaMismatch { .. }
        ))
    ));
    let mut rebuild = Projection::rebuild(
        id.clone(),
        Sum,
        AfterEventTypes::new(&["event"]),
        &codec,
        new,
    )
    .await
    .unwrap();
    assert_eq!(rebuild.rebuilding_from(), Some(old));
    assert_eq!(rebuild.checkpoint(), None);
    assert_eq!(rebuild.state().unwrap().0, 0);
    rebuild.advance(item(Event::Add(1), 1)).await.unwrap();
    rebuild.advance(item(Event::Add(2), 2)).await.unwrap();
    assert!(matches!(
        original.advance(item(Event::Add(100), 6)).await,
        Err(ProjectionError::Commit(CheckpointError::Rejected(
            CheckpointRejection::Conflict { .. }
        )))
    ));
    let restored = Projection::load(id.clone(), Sum, AfterEventTypes::new(&[]), &codec, new)
        .await
        .unwrap();
    assert_eq!(restored.state().unwrap().0, 3);
    assert_eq!(restored.checkpoint(), Version::new(2));
    assert!(matches!(
        Projection::rebuild(id, Sum, AfterEventTypes::new(&[]), &codec, new).await,
        Err(CheckpointError::Rejected(
            CheckpointRejection::InvalidRebuild
        ))
    ));
    assert!(matches!(
        Projection::rebuild(
            StreamKey::from_slice(b"absent"),
            Sum,
            AfterEventTypes::new(&[]),
            &codec,
            new
        )
        .await,
        Err(CheckpointError::Rejected(
            CheckpointRejection::InvalidRebuild
        ))
    ));
}

#[derive(Clone, Copy)]
enum Fault {
    ErrorBefore,
    ErrorAfter,
    PendingBefore,
    PendingAfter,
}

// All storage is delegated to the real adapter. These two seams deliberately
// fail/suspend immediately before or after its atomic checkpoint operation.
struct FaultStore<'a> {
    backing: &'a InMemoryCheckpointStore<Vec<u8>, Version>,
    fault: Fault,
    entered: AtomicBool,
}
impl CheckpointStore<Vec<u8>, Version> for FaultStore<'_> {
    type Error = std::io::Error;

    async fn hydrate_checkpoint(
        &self,
        id: &impl Id,
        schema_version: NonZeroU32,
    ) -> Result<CheckpointHydrated<Vec<u8>, Version>, Self::Error> {
        self.backing
            .hydrate_checkpoint(id, schema_version)
            .await
            .map_err(|never| match never {})
    }

    async fn commit_checkpoint(
        &self,
        id: &impl Id,
        write: CheckpointWrite<'_, Vec<u8>, Version>,
    ) -> Result<NonZeroU64, CheckpointError<Self::Error>> {
        if matches!(self.fault, Fault::ErrorAfter | Fault::PendingAfter) {
            self.backing
                .commit_checkpoint(id, write)
                .await
                .map_err(|error| match error {
                    CheckpointError::Store(never) => match never {},
                    CheckpointError::Rejected(rejection) => CheckpointError::Rejected(rejection),
                    other => CheckpointError::Store(std::io::Error::other(other)),
                })?;
        }
        self.entered.store(true, Ordering::SeqCst);
        match self.fault {
            Fault::ErrorBefore | Fault::ErrorAfter => Err(CheckpointError::Store(
                std::io::Error::other("injected checkpoint error"),
            )),
            Fault::PendingBefore | Fault::PendingAfter => core::future::pending().await,
        }
    }
}

#[tokio::test]
async fn checkpoint_errors_and_cancellation_before_or_after_commit_force_exact_reload() {
    for fault in [
        Fault::ErrorBefore,
        Fault::ErrorAfter,
        Fault::PendingBefore,
        Fault::PendingAfter,
    ] {
        for manual_flush in [false, true] {
            let backing = InMemoryCheckpointStore::<Vec<u8>, Version>::new();
            let faults = FaultStore {
                backing: &backing,
                fault,
                entered: AtomicBool::new(false),
            };
            let codec = CodecSnapshotStore::new(&faults, JsonCodec::default());
            let id = StreamKey::from_slice(b"uncertain");
            let names: &[&str] = if manual_flush { &[] } else { &["event"] };
            let mut projection = Projection::load(
                id.clone(),
                Sum,
                AfterEventTypes::new(names),
                &codec,
                NonZeroU32::MIN,
            )
            .await
            .unwrap();
            let outcome = if manual_flush {
                projection.advance(item(Event::Add(10), 1)).await.unwrap();
                projection.flush().now_or_never()
            } else {
                projection.advance(item(Event::Add(10), 1)).now_or_never()
            };
            assert!(
                faults.entered.load(Ordering::SeqCst),
                "must reach the injected boundary before cancellation"
            );
            match fault {
                Fault::ErrorBefore | Fault::ErrorAfter => {
                    assert!(matches!(outcome, Some(Err(ProjectionError::Commit(_)))));
                }
                Fault::PendingBefore | Fault::PendingAfter => assert!(outcome.is_none()),
            }
            assert_eq!(
                projection.state().unwrap_err(),
                ProjectionStateError::ReloadRequired
            );
            assert_eq!(projection.observed(), Version::new(1));
            assert_eq!(projection.checkpoint(), None);
            assert!(matches!(
                projection.advance(item(Event::Add(999), 2)).await,
                Err(ProjectionError::State(ProjectionStateError::ReloadRequired))
            ));
            assert!(matches!(
                projection.flush().await,
                Err(ProjectionError::State(ProjectionStateError::ReloadRequired))
            ));
            let normal = CodecSnapshotStore::new(&backing, JsonCodec::default());
            let mut restored = Projection::load(
                id,
                Sum,
                AfterEventTypes::new(&["event"]),
                &normal,
                NonZeroU32::MIN,
            )
            .await
            .unwrap();
            let committed = matches!(fault, Fault::ErrorAfter | Fault::PendingAfter);
            assert_eq!(restored.state().unwrap().0, if committed { 10 } else { 0 });
            assert_eq!(
                restored.checkpoint(),
                if committed { Version::new(1) } else { None }
            );
            restored
                .advance(item(Event::Add(20), if committed { 2 } else { 1 }))
                .await
                .unwrap();
            assert_eq!(restored.state().unwrap().0, if committed { 30 } else { 20 });
        }
    }
}

#[tokio::test]
async fn dropping_an_unpolled_advance_or_flush_leaves_the_instance_usable() {
    let bytes = InMemoryCheckpointStore::<Vec<u8>, Version>::new();
    let codec = CodecSnapshotStore::new(&bytes, JsonCodec::default());
    let mut projection = Projection::load(
        StreamKey::from_slice(b"unpolled"),
        Sum,
        AfterEventTypes::new(&[]),
        &codec,
        NonZeroU32::MIN,
    )
    .await
    .unwrap();
    drop(projection.advance(item(Event::Add(10), 1)));
    assert_eq!(projection.state().unwrap().0, 0);
    assert_eq!(projection.observed(), None);
    projection.advance(item(Event::Add(10), 1)).await.unwrap();
    drop(projection.flush());
    assert_eq!(projection.state().unwrap().0, 10);
    assert_eq!(projection.observed(), Version::new(1));
    assert_eq!(projection.checkpoint(), None);
    projection.flush().await.unwrap();
    assert_eq!(projection.checkpoint(), Version::new(1));
}
