//! Revision-checked projection checkpoints, separate from snapshot caches.

use std::collections::HashMap;
use std::convert::Infallible;
use std::num::{NonZeroU32, NonZeroU64};

use mnesis::Id;
use mnesis_store::checkpoint::{
    CheckpointError, CheckpointHydrated, CheckpointStore, CheckpointWrite,
};
use tokio::sync::RwLock;

#[derive(Debug)]
struct Record<S, P> {
    revision: NonZeroU64,
    schema_version: NonZeroU32,
    position: P,
    state: S,
}

/// Atomic checkpoint storage keyed by the id's stable byte identity.
///
/// A single write lock spans comparison, validation and complete replacement.
/// Checkpoints have their own namespace; aggregate snapshot writes cannot
/// bypass their revisions or overwrite projection state.
#[derive(Debug, Default)]
pub struct InMemoryCheckpointStore<S, P> {
    records: RwLock<HashMap<Vec<u8>, Record<S, P>>>,
}

impl<S, P> InMemoryCheckpointStore<S, P> {
    /// Create an empty checkpoint store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            records: RwLock::new(HashMap::new()),
        }
    }
}

impl<S, P> CheckpointStore<S, P> for InMemoryCheckpointStore<S, P>
where
    S: Clone + Send + Sync + 'static,
    P: Copy + Ord + Send + Sync + 'static,
{
    type Error = Infallible;

    async fn hydrate_checkpoint(
        &self,
        id: &impl Id,
        schema_version: NonZeroU32,
    ) -> Result<CheckpointHydrated<S, P>, Self::Error> {
        let records = self.records.read().await;
        Ok(match records.get(id.as_ref()) {
            None => CheckpointHydrated::Absent,
            Some(record) if record.schema_version != schema_version => CheckpointHydrated::Stale {
                revision: record.revision,
                stored_schema: record.schema_version,
            },
            Some(record) => CheckpointHydrated::Found {
                revision: record.revision,
                position: record.position,
                state: record.state.clone(),
            },
        })
    }

    async fn commit_checkpoint(
        &self,
        id: &impl Id,
        write: CheckpointWrite<'_, S, P>,
    ) -> Result<NonZeroU64, CheckpointError<Self::Error>> {
        // Clone before locking. An application Clone panic cannot replace an
        // existing checkpoint or occur after successful revision validation.
        let state = write.state.clone();
        let mut records = self.records.write().await;
        let current = records
            .get(id.as_ref())
            .map(|record| (record.revision, record.schema_version, record.position));
        let revision = write.next_revision(current)?;
        records.insert(
            id.as_ref().to_vec(),
            Record {
                revision,
                schema_version: write.schema_version,
                position: write.position,
                state,
            },
        );
        drop(records);
        Ok(revision)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "checkpoint protocol assertions")]
mod tests {
    use super::{InMemoryCheckpointStore, Record};
    use core::num::{NonZeroU32, NonZeroU64};
    use mnesis_store::StreamKey;
    use mnesis_store::checkpoint::{
        CheckpointError, CheckpointHydrated, CheckpointMode, CheckpointRejection, CheckpointStore,
        CheckpointWrite,
    };
    use std::convert::Infallible;
    use std::sync::Arc;
    use tokio::sync::Barrier;

    fn write(
        expected: Option<NonZeroU64>,
        schema_version: NonZeroU32,
        position: u64,
        state: &u64,
        mode: CheckpointMode,
    ) -> CheckpointWrite<'_, u64, u64> {
        CheckpointWrite {
            expected,
            schema_version,
            position,
            state,
            mode,
        }
    }

    #[tokio::test]
    async fn stale_and_non_increasing_writes_preserve_the_complete_record() {
        let store = InMemoryCheckpointStore::new();
        let id = StreamKey::from_slice(b"ordered");
        let schema = NonZeroU32::MIN;
        assert_eq!(
            store.hydrate_checkpoint(&id, schema).await.unwrap(),
            CheckpointHydrated::Absent
        );
        let first = store
            .commit_checkpoint(&id, write(None, schema, 10, &100, CheckpointMode::Advance))
            .await
            .unwrap();
        for position in [10, 9, 0] {
            assert!(matches!(
                store
                    .commit_checkpoint(
                        &id,
                        write(Some(first), schema, position, &999, CheckpointMode::Advance)
                    )
                    .await,
                Err(CheckpointError::Rejected(
                    CheckpointRejection::NonIncreasingPosition
                ))
            ));
        }
        let next = store
            .commit_checkpoint(
                &id,
                write(Some(first), schema, 30, &300, CheckpointMode::Advance),
            )
            .await
            .unwrap();
        assert_eq!(next.get(), 2);
        assert!(matches!(
            store
                .commit_checkpoint(
                    &id,
                    write(Some(first), schema, 40, &400, CheckpointMode::Advance)
                )
                .await,
            Err(CheckpointError::Rejected(
                CheckpointRejection::Conflict { .. }
            ))
        ));
        assert_eq!(
            store.hydrate_checkpoint(&id, schema).await.unwrap(),
            CheckpointHydrated::Found {
                revision: next,
                position: 30,
                state: 300
            }
        );
    }

    #[tokio::test]
    async fn schema_rebuild_is_explicit_and_does_not_reset_revision() {
        let store = InMemoryCheckpointStore::new();
        let id = StreamKey::from_slice(b"rebuild");
        let old = NonZeroU32::MIN;
        let new = NonZeroU32::new(2).unwrap();
        assert!(matches!(
            store
                .commit_checkpoint(&id, write(None, new, 1, &1, CheckpointMode::Rebuild))
                .await,
            Err(CheckpointError::Rejected(
                CheckpointRejection::InvalidRebuild
            ))
        ));
        let revision = store
            .commit_checkpoint(&id, write(None, old, 50, &500, CheckpointMode::Advance))
            .await
            .unwrap();
        assert_eq!(
            store.hydrate_checkpoint(&id, new).await.unwrap(),
            CheckpointHydrated::Stale {
                revision,
                stored_schema: old
            }
        );
        assert!(matches!(
            store
                .commit_checkpoint(
                    &id,
                    write(Some(revision), new, 1, &10, CheckpointMode::Advance)
                )
                .await,
            Err(CheckpointError::Rejected(
                CheckpointRejection::SchemaMismatch { .. }
            ))
        ));
        assert!(matches!(
            store
                .commit_checkpoint(
                    &id,
                    write(Some(revision), old, 1, &10, CheckpointMode::Rebuild)
                )
                .await,
            Err(CheckpointError::Rejected(
                CheckpointRejection::InvalidRebuild
            ))
        ));
        let rebuilt = store
            .commit_checkpoint(
                &id,
                write(Some(revision), new, 1, &10, CheckpointMode::Rebuild),
            )
            .await
            .unwrap();
        assert_eq!(rebuilt.get(), 2);
        assert!(matches!(
            store
                .commit_checkpoint(
                    &id,
                    write(Some(revision), old, 60, &600, CheckpointMode::Advance)
                )
                .await,
            Err(CheckpointError::Rejected(
                CheckpointRejection::Conflict { .. }
            ))
        ));
        assert_eq!(
            store.hydrate_checkpoint(&id, new).await.unwrap(),
            CheckpointHydrated::Found {
                revision: rebuilt,
                position: 1,
                state: 10
            }
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[allow(clippy::panic, reason = "report unexpected paired race outcomes")]
    async fn racing_creations_and_updates_each_have_exactly_one_winner() {
        let store = Arc::new(InMemoryCheckpointStore::new());
        let id = StreamKey::from_slice(b"race");
        let schema = NonZeroU32::MIN;
        let (left, right) = race(&store, &id, None, [(1, 10), (2, 20)]).await;
        let (first, expected_position, expected_state) = match (left, right) {
            (
                Ok(revision),
                Err(CheckpointError::Rejected(CheckpointRejection::Conflict { .. })),
            ) => (revision, 1, 10),
            (
                Err(CheckpointError::Rejected(CheckpointRejection::Conflict { .. })),
                Ok(revision),
            ) => (revision, 2, 20),
            outcomes => panic!("creation must have one winner: {outcomes:?}"),
        };
        assert_eq!(
            store.hydrate_checkpoint(&id, schema).await.unwrap(),
            CheckpointHydrated::Found {
                revision: first,
                position: expected_position,
                state: expected_state
            }
        );
        let (update_left, update_right) = race(&store, &id, Some(first), [(3, 30), (4, 40)]).await;
        let (next, position, state) = match (update_left, update_right) {
            (
                Ok(revision),
                Err(CheckpointError::Rejected(CheckpointRejection::Conflict { .. })),
            ) => (revision, 3, 30),
            (
                Err(CheckpointError::Rejected(CheckpointRejection::Conflict { .. })),
                Ok(revision),
            ) => (revision, 4, 40),
            outcomes => panic!("update must have one winner: {outcomes:?}"),
        };
        assert_eq!(next.get(), 2);
        assert_eq!(
            store.hydrate_checkpoint(&id, schema).await.unwrap(),
            CheckpointHydrated::Found {
                revision: next,
                position,
                state
            }
        );
    }

    type WriteResult = Result<NonZeroU64, CheckpointError<Infallible>>;

    async fn race(
        store: &Arc<InMemoryCheckpointStore<u64, u64>>,
        id: &StreamKey,
        expected: Option<NonZeroU64>,
        proposals: [(u64, u64); 2],
    ) -> (WriteResult, WriteResult) {
        let barrier = Arc::new(Barrier::new(3));
        let [left, right] = proposals.map(|(position, state)| {
            let shared_store = Arc::clone(store);
            let shared_barrier = Arc::clone(&barrier);
            let owned_id = id.clone();
            tokio::spawn(async move {
                shared_barrier.wait().await;
                shared_store
                    .commit_checkpoint(
                        &owned_id,
                        write(
                            expected,
                            NonZeroU32::MIN,
                            position,
                            &state,
                            CheckpointMode::Advance,
                        ),
                    )
                    .await
            })
        });
        barrier.wait().await;
        let (left_result, right_result) = tokio::join!(left, right);
        (left_result.unwrap(), right_result.unwrap())
    }

    #[tokio::test]
    async fn revision_overflow_never_wraps_or_changes_state() {
        let store = InMemoryCheckpointStore::new();
        let id = StreamKey::from_slice(b"revision-ceiling");
        let schema = NonZeroU32::MIN;
        let revision = NonZeroU64::MAX;
        store.records.write().await.insert(
            id.as_ref().to_vec(),
            Record {
                revision,
                schema_version: schema,
                position: 10,
                state: 100,
            },
        );
        assert!(matches!(
            store
                .commit_checkpoint(
                    &id,
                    write(Some(revision), schema, 11, &110, CheckpointMode::Advance)
                )
                .await,
            Err(CheckpointError::Rejected(
                CheckpointRejection::RevisionOverflow
            ))
        ));
        assert_eq!(
            store.hydrate_checkpoint(&id, schema).await.unwrap(),
            CheckpointHydrated::Found {
                revision,
                position: 10,
                state: 100
            }
        );
    }
}
