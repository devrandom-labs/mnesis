#![cfg(feature = "projection")]
#![allow(clippy::unwrap_used, reason = "checkpoint lifecycle assertions")]

use core::num::{NonZeroU32, NonZeroU64};
use mnesis::Version;
use mnesis_fjall::{FjallStore, GlobalSeq};
use mnesis_store::StreamKey;
use mnesis_store::checkpoint::{
    CheckpointError, CheckpointHydrated, CheckpointMode, CheckpointRejection, CheckpointStore,
    CheckpointWrite,
};

const fn advance<P>(
    expected: Option<NonZeroU64>,
    position: P,
    state: &Vec<u8>,
) -> CheckpointWrite<'_, Vec<u8>, P> {
    CheckpointWrite {
        expected,
        schema_version: NonZeroU32::MIN,
        position,
        state,
        mode: CheckpointMode::Advance,
    }
}

#[tokio::test]
async fn stale_and_non_increasing_writes_preserve_the_complete_checkpoint() {
    let directory = tempfile::tempdir().unwrap();
    let store = FjallStore::builder(directory.path()).open().unwrap();
    let id = StreamKey::from_slice(b"ordered");
    let initial = vec![1];
    let latest = vec![2];
    let first = store
        .commit_checkpoint(&id, advance(None, GlobalSeq::INITIAL, &initial))
        .await
        .unwrap();
    let next = store
        .commit_checkpoint(
            &id,
            advance(Some(first), GlobalSeq::new(7).unwrap(), &latest),
        )
        .await
        .unwrap();
    for expected in [None, Some(first)] {
        assert!(matches!(
            store
                .commit_checkpoint(&id, advance(expected, GlobalSeq::new(8).unwrap(), &initial))
                .await,
            Err(CheckpointError::Rejected(
                CheckpointRejection::Conflict { .. }
            ))
        ));
    }
    for position in [7, 6] {
        assert!(matches!(
            store
                .commit_checkpoint(
                    &id,
                    advance(Some(next), GlobalSeq::new(position).unwrap(), &initial)
                )
                .await,
            Err(CheckpointError::Rejected(
                CheckpointRejection::NonIncreasingPosition
            ))
        ));
    }
    let found = <FjallStore as CheckpointStore<Vec<u8>, GlobalSeq>>::hydrate_checkpoint(
        &store,
        &id,
        NonZeroU32::MIN,
    )
    .await
    .unwrap();
    assert_eq!(
        found,
        CheckpointHydrated::Found {
            revision: next,
            position: GlobalSeq::new(7).unwrap(),
            state: latest
        }
    );
    store.close().await.unwrap();
}

#[tokio::test]
async fn schema_rebuild_replaces_only_the_loaded_revision_without_resetting_it() {
    let directory = tempfile::tempdir().unwrap();
    let store = FjallStore::builder(directory.path()).open().unwrap();
    let id = StreamKey::from_slice(b"rebuild");
    let new = NonZeroU32::new(2).unwrap();
    let original = vec![1];
    let rebuilt = vec![2];
    let first = store
        .commit_checkpoint(&id, advance(None, GlobalSeq::new(7).unwrap(), &original))
        .await
        .unwrap();
    let stale =
        <FjallStore as CheckpointStore<Vec<u8>, GlobalSeq>>::hydrate_checkpoint(&store, &id, new)
            .await
            .unwrap();
    assert_eq!(
        stale,
        CheckpointHydrated::Stale {
            revision: first,
            stored_schema: NonZeroU32::MIN
        }
    );
    let proposal = |mode| CheckpointWrite {
        expected: Some(first),
        schema_version: new,
        position: GlobalSeq::INITIAL,
        state: &rebuilt,
        mode,
    };
    assert!(matches!(
        store
            .commit_checkpoint(&id, proposal(CheckpointMode::Advance))
            .await,
        Err(CheckpointError::Rejected(
            CheckpointRejection::SchemaMismatch { .. }
        ))
    ));
    let next = store
        .commit_checkpoint(&id, proposal(CheckpointMode::Rebuild))
        .await
        .unwrap();
    assert_eq!(next.get(), 2);
    let found =
        <FjallStore as CheckpointStore<Vec<u8>, GlobalSeq>>::hydrate_checkpoint(&store, &id, new)
            .await
            .unwrap();
    assert_eq!(
        found,
        CheckpointHydrated::Found {
            revision: next,
            position: GlobalSeq::INITIAL,
            state: rebuilt
        }
    );
    store.close().await.unwrap();
}

#[tokio::test]
async fn independent_position_domains_survive_close_and_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let store = FjallStore::builder(directory.path()).open().unwrap();
    let id = StreamKey::from_slice(b"same-id");
    let global_state = vec![3, 4];
    let stream_state = vec![8, 9];
    let global_revision = store
        .commit_checkpoint(&id, advance(None, GlobalSeq::INITIAL, &global_state))
        .await
        .unwrap();
    let stream_revision = store
        .commit_checkpoint(&id, advance(None, Version::new(11).unwrap(), &stream_state))
        .await
        .unwrap();
    #[cfg(feature = "snapshot")]
    mnesis_store::SnapshotStore::<Vec<u8>, Version>::commit(
        &store,
        &id,
        NonZeroU32::MIN,
        Version::new(1).unwrap(),
        &vec![99],
    )
    .await
    .unwrap();
    store.close().await.unwrap();
    let reopened = FjallStore::builder(directory.path()).open().unwrap();
    let global = <FjallStore as CheckpointStore<Vec<u8>, GlobalSeq>>::hydrate_checkpoint(
        &reopened,
        &id,
        NonZeroU32::MIN,
    )
    .await
    .unwrap();
    assert_eq!(
        global,
        CheckpointHydrated::Found {
            revision: global_revision,
            position: GlobalSeq::INITIAL,
            state: global_state
        }
    );
    let stream = <FjallStore as CheckpointStore<Vec<u8>, Version>>::hydrate_checkpoint(
        &reopened,
        &id,
        NonZeroU32::MIN,
    )
    .await
    .unwrap();
    assert_eq!(
        stream,
        CheckpointHydrated::Found {
            revision: stream_revision,
            position: Version::new(11).unwrap(),
            state: stream_state
        }
    );
    reopened.close().await.unwrap();
}

type WriteResult = Result<NonZeroU64, CheckpointError<mnesis_fjall::FjallError>>;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn competing_checkpoint_writers_have_one_complete_winner_per_revision() {
    let directory = tempfile::tempdir().unwrap();
    let store = FjallStore::builder(directory.path()).open().unwrap();
    let id = StreamKey::from_slice(b"racing");
    let (first_revision, first_position) = race(&store, &id, None, [1, 2]).await;
    assert_eq!(first_revision, NonZeroU64::MIN);
    assert_record(&store, &id, first_revision, first_position).await;
    let (next_revision, next_position) = race(&store, &id, Some(first_revision), [3, 4]).await;
    assert_eq!(next_revision.get(), 2);
    assert_record(&store, &id, next_revision, next_position).await;
    store.close().await.unwrap();
    let reopened = FjallStore::builder(directory.path()).open().unwrap();
    assert_record(&reopened, &id, next_revision, next_position).await;
    reopened.close().await.unwrap();
}

async fn race(
    store: &FjallStore,
    id: &StreamKey,
    expected: Option<NonZeroU64>,
    positions: [u64; 2],
) -> (NonZeroU64, u64) {
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(3));
    let [left, right] = positions.map(|position| {
        let shared_store = store.clone();
        let shared_barrier = std::sync::Arc::clone(&barrier);
        let owned_id = id.clone();
        tokio::spawn(async move {
            shared_barrier.wait().await;
            let state = position.to_le_bytes().to_vec();
            let result = shared_store
                .commit_checkpoint(
                    &owned_id,
                    advance(expected, GlobalSeq::new(position).unwrap(), &state),
                )
                .await;
            (result, position)
        })
    });
    barrier.wait().await;
    let (left_outcome, right_outcome) = tokio::join!(left, right);
    winner(left_outcome.unwrap(), right_outcome.unwrap())
}

#[allow(
    clippy::panic,
    reason = "report a race that did not have exactly one CAS winner"
)]
fn winner(left: (WriteResult, u64), right: (WriteResult, u64)) -> (NonZeroU64, u64) {
    match (left, right) {
        (
            (Ok(revision), position),
            (Err(CheckpointError::Rejected(CheckpointRejection::Conflict { .. })), _),
        )
        | (
            (Err(CheckpointError::Rejected(CheckpointRejection::Conflict { .. })), _),
            (Ok(revision), position),
        ) => (revision, position),
        other => panic!("expected one winner and one revision conflict: {other:?}"),
    }
}

async fn assert_record(store: &FjallStore, id: &StreamKey, revision: NonZeroU64, position: u64) {
    let actual = <FjallStore as CheckpointStore<Vec<u8>, GlobalSeq>>::hydrate_checkpoint(
        store,
        id,
        NonZeroU32::MIN,
    )
    .await
    .unwrap();
    assert_eq!(
        actual,
        CheckpointHydrated::Found {
            revision,
            position: GlobalSeq::new(position).unwrap(),
            state: position.to_le_bytes().to_vec()
        }
    );
}
