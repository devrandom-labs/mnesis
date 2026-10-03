use crate::blocking::{BlockingConfig, BlockingStream, Executor};
use crate::scan::{GlobalScan, ScanCursor, StreamScan};
use crate::store::Storage;
use crate::{Durability, FjallError, FjallStoreBuilder, GlobalSeq};
use futures::executor::block_on;
use mnesis::Version;
use mnesis_store::error::AppendError;
use mnesis_store::wake::WakeSource;
use mnesis_store::{PendingBatch, RawEventStore, StreamKey};
use mnesis_wake::{NotifyError, WakeReg};
use std::path::Path;
use std::sync::Arc;

/// Runtime-neutral Fjall adapter with one bounded blocking I/O worker.
///
/// Once an operation starts it completes, including commit/wake bookkeeping,
/// even if its future is dropped. Queued canceled operations are skipped.
/// Drop closes admission and releases storage on the worker; use [`Self::close`]
/// to await complete shutdown before reopening or deleting the directory.
#[derive(Clone)]
pub struct FjallStore {
    pub(crate) storage: Arc<Storage>,
    pub(crate) executor: Executor,
    pub(crate) config: BlockingConfig,
}

impl FjallStore {
    pub(crate) fn new(engine: Storage, config: BlockingConfig) -> Result<Self, FjallError> {
        let storage = Arc::new(engine);
        let executor = Executor::new(
            config.queue_capacity,
            config.max_open_scans,
            Arc::clone(&storage),
        )?;
        Ok(Self {
            storage,
            executor,
            config,
        })
    }

    /// Configure a store. Opening/recovery is explicitly synchronous.
    #[must_use]
    pub fn builder(path: impl AsRef<Path>) -> FjallStoreBuilder {
        FjallStoreBuilder::new(path)
    }

    /// The acknowledgment policy used for event and state writes.
    #[must_use]
    pub fn durability(&self) -> Durability {
        self.storage.durability()
    }

    /// Sync all completed writes on the blocking worker.
    ///
    /// # Errors
    /// Errors can leave commit outcomes uncertain; reopen and inspect history
    /// before retrying. See [`Durability`] and the adapter README.
    pub async fn flush(&self) -> Result<(), FjallError> {
        let slot = self.executor.reserve().await?;
        let storage = Arc::clone(&self.storage);
        slot.submit(move || storage.flush()).await?
    }

    /// Drain admitted work and wait for engine shutdown, without blocking the
    /// calling executor. Release other clones and unfinished scans first.
    /// Cancellation closes this handle; admitted running work still completes.
    ///
    /// # Errors
    /// Returns [`FjallError::OutstandingHandles`] while another store/scan
    /// retains the worker. Worker termination errors remain typed.
    pub async fn close(self) -> Result<(), FjallError> {
        let Self {
            storage,
            executor,
            config: _,
        } = self;
        drop(storage);
        executor.shutdown().await
    }
}

impl RawEventStore for FjallStore {
    type Error = FjallError;
    type Stream = BlockingStream<ScanCursor<StreamScan>>;
    type AllPosition = GlobalSeq;
    type AllStream = BlockingStream<ScanCursor<GlobalScan>>;

    async fn append(
        &self,
        id: &StreamKey,
        expected_version: Option<Version>,
        envelopes: PendingBatch<'_>,
    ) -> Result<GlobalSeq, AppendError<FjallError>> {
        crate::limits::validate_key(id.as_ref(), crate::MAX_STREAM_ID_LEN)
            .map_err(AppendError::Store)?;
        let slot = self.executor.reserve().await.map_err(AppendError::Store)?;
        let storage = Arc::clone(&self.storage);
        let owned_id = id.clone();
        // Cross-thread work must own inputs; admission bounds these copies.
        let first = envelopes.first().clone();
        let rest: Vec<_> = envelopes.iter().skip(1).cloned().collect();
        slot.submit(move || {
            block_on(storage.append(
                &owned_id,
                expected_version,
                PendingBatch::from_parts(&first, &rest),
            ))
        })
        .await
        .map_err(AppendError::Store)?
    }

    async fn read_stream(&self, id: &StreamKey, from: Version) -> Result<Self::Stream, FjallError> {
        crate::limits::validate_key(id.as_ref(), crate::MAX_STREAM_ID_LEN)?;
        let cleanup = self.executor.reserve_cursor()?;
        let slot = self.executor.reserve().await?;
        let storage = Arc::clone(&self.storage);
        let owned_id = id.clone();
        let cursor = slot
            .submit(move || {
                block_on(storage.read_stream(&owned_id, from)).map(|cursor| cleanup.attach(cursor))
            })
            .await??;
        Ok(BlockingStream::new(
            cursor,
            self.executor.clone(),
            self.config,
        ))
    }

    async fn read_all(&self, from: Option<GlobalSeq>) -> Result<Self::AllStream, FjallError> {
        let cleanup = self.executor.reserve_cursor()?;
        let slot = self.executor.reserve().await?;
        let storage = Arc::clone(&self.storage);
        let cursor = slot
            .submit(move || block_on(storage.read_all(from)).map(|cursor| cleanup.attach(cursor)))
            .await??;
        Ok(BlockingStream::new(
            cursor,
            self.executor.clone(),
            self.config,
        ))
    }
}

impl WakeSource for FjallStore {
    type Registration = WakeReg;
    type Error = NotifyError;
    fn register(&self, stream: Option<&[u8]>) -> Result<WakeReg, NotifyError> {
        self.storage.register(stream)
    }
    fn wake(&self, stream: &[u8]) {
        self.storage.wake(stream);
    }
}

#[cfg(feature = "import")]
mod atomic {
    use super::{Arc, FjallError, FjallStore, GlobalSeq, block_on};
    use mnesis_store::import::{
        AtomicAppend, AtomicAppendError, PlannedAppend, validate_atomic_runs,
        validate_distinct_targets,
    };

    impl AtomicAppend for FjallStore {
        async fn atomic_append_many(
            &self,
            writes: &[PlannedAppend],
        ) -> Result<Option<GlobalSeq>, AtomicAppendError<FjallError>> {
            if writes.is_empty() {
                return Ok(None);
            }
            validate_distinct_targets(writes.iter().map(|write| &write.target))?;
            validate_atomic_runs(writes)?;
            for write in writes {
                crate::limits::validate_key(write.target.as_ref(), crate::MAX_STREAM_ID_LEN)
                    .map_err(AtomicAppendError::Store)?;
            }
            let slot = self
                .executor
                .reserve()
                .await
                .map_err(AtomicAppendError::Store)?;
            let storage = Arc::clone(&self.storage);
            let owned_writes = writes.to_vec();
            slot.submit(move || block_on(storage.atomic_append_many(&owned_writes)))
                .await
                .map_err(AtomicAppendError::Store)?
        }
    }
}

#[cfg(feature = "snapshot")]
mod state {
    use super::{Arc, FjallError, FjallStore, Storage, Version, block_on};
    use mnesis::Id;
    use mnesis_store::state::{Hydrated, SnapshotStore};
    use std::num::NonZeroU32;

    impl SnapshotStore<Vec<u8>, Version> for FjallStore {
        type Error = FjallError;
        async fn hydrate(
            &self,
            id: &impl Id,
            schema: NonZeroU32,
        ) -> Result<Hydrated<Vec<u8>, Version>, FjallError> {
            crate::limits::validate_key(id.as_ref(), crate::MAX_KEY_LEN)?;
            let slot = self.executor.reserve().await?;
            let storage = Arc::clone(&self.storage);
            let owned_id = id.clone();
            slot.submit(move || {
                block_on(<Storage as SnapshotStore<Vec<u8>, Version>>::hydrate(
                    &storage, &owned_id, schema,
                ))
            })
            .await?
        }
        async fn commit(
            &self,
            id: &impl Id,
            schema: NonZeroU32,
            position: Version,
            state: &Vec<u8>,
        ) -> Result<(), FjallError> {
            crate::limits::validate_key(id.as_ref(), crate::MAX_KEY_LEN)?;
            crate::limits::state_value_len(state.len())?;
            let slot = self.executor.reserve().await?;
            let storage = Arc::clone(&self.storage);
            let owned_id = id.clone();
            let owned_state = state.clone();
            slot.submit(move || {
                block_on(<Storage as SnapshotStore<Vec<u8>, Version>>::commit(
                    &storage,
                    &owned_id,
                    schema,
                    position,
                    &owned_state,
                ))
            })
            .await?
        }
    }
}

#[cfg(feature = "projection")]
mod checkpoints {
    use super::{Arc, FjallError, FjallStore, GlobalSeq, Storage, Version, block_on};
    use mnesis::Id;
    use mnesis_store::checkpoint::{
        CheckpointError, CheckpointHydrated, CheckpointStore, CheckpointWrite,
    };
    use std::num::{NonZeroU32, NonZeroU64};

    macro_rules! checkpoint_impl {
        ($position:ty) => {
            impl CheckpointStore<Vec<u8>, $position> for FjallStore {
                type Error = FjallError;

                async fn hydrate_checkpoint(
                    &self,
                    id: &impl Id,
                    schema: NonZeroU32,
                ) -> Result<CheckpointHydrated<Vec<u8>, $position>, FjallError> {
                    crate::limits::validate_key(id.as_ref(), crate::MAX_KEY_LEN)?;
                    let slot = self.executor.reserve().await?;
                    let storage = Arc::clone(&self.storage);
                    let owned_id = id.clone();
                    slot.submit(move || {
                        block_on(
                            <Storage as CheckpointStore<Vec<u8>, $position>>::hydrate_checkpoint(
                                &storage, &owned_id, schema,
                            ),
                        )
                    })
                    .await?
                }

                async fn commit_checkpoint(
                    &self,
                    id: &impl Id,
                    write: CheckpointWrite<'_, Vec<u8>, $position>,
                ) -> Result<NonZeroU64, CheckpointError<FjallError>> {
                    crate::limits::validate_key(id.as_ref(), crate::MAX_KEY_LEN)
                        .map_err(CheckpointError::Store)?;
                    crate::limits::checkpoint_value_len(write.state.len())
                        .map_err(CheckpointError::Store)?;
                    let slot = self
                        .executor
                        .reserve()
                        .await
                        .map_err(CheckpointError::Store)?;
                    let storage = Arc::clone(&self.storage);
                    let owned_id = id.clone();
                    let state = write.state.clone();
                    let expected = write.expected;
                    let schema_version = write.schema_version;
                    let position = write.position;
                    let mode = write.mode;
                    slot.submit(move || {
                        block_on(
                            <Storage as CheckpointStore<Vec<u8>, $position>>::commit_checkpoint(
                                &storage,
                                &owned_id,
                                CheckpointWrite {
                                    expected,
                                    schema_version,
                                    position,
                                    state: &state,
                                    mode,
                                },
                            ),
                        )
                    })
                    .await
                    .map_err(CheckpointError::Store)?
                }
            }
        };
    }
    checkpoint_impl!(Version);
    checkpoint_impl!(GlobalSeq);
}

#[cfg(feature = "export")]
mod listing {
    use super::{Arc, BlockingStream, FjallError, FjallStore, block_on};
    use crate::store::stream_lister_impl::StreamIdCursor;
    use mnesis_store::export::StreamLister;

    impl StreamLister for FjallStore {
        type StreamList = BlockingStream<StreamIdCursor>;
        async fn list_streams(&self) -> Result<Self::StreamList, FjallError> {
            let cleanup = self.executor.reserve_cursor()?;
            let slot = self.executor.reserve().await?;
            let storage = Arc::clone(&self.storage);
            let cursor = slot
                .submit(move || {
                    block_on(storage.list_streams()).map(|cursor| cleanup.attach(cursor))
                })
                .await??;
            Ok(BlockingStream::new(
                cursor,
                self.executor.clone(),
                self.config,
            ))
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, reason = "test assertions")]
mod tests {
    use super::{FjallStore, PendingBatch, RawEventStore, StreamKey, Version, block_on};
    use crate::{BlockingConfig, FjallError};
    use futures::{FutureExt, StreamExt};
    use mnesis_store::envelope::pending_envelope;
    use mnesis_store::wake::{WakeRegistration, WakeSource};
    use std::sync::Arc;

    #[tokio::test(flavor = "current_thread")]
    async fn canceled_append_before_start_leaves_no_event_or_wake() {
        let directory = tempfile::tempdir().unwrap();
        let store = FjallStore::builder(directory.path()).open().unwrap();
        let id = StreamKey::from_slice(b"canceled");
        let registration = store.register(Some(id.as_ref())).unwrap();
        let mut wake = Box::pin(registration.arm());
        let (release, wait) = std::sync::mpsc::channel();
        let (started, entered) = futures::channel::oneshot::channel();
        let blocker = store.executor.reserve().await.unwrap().submit(move || {
            let _ = started.send(());
            let _ = wait.recv();
        });
        entered.await.unwrap();
        let event = pending_envelope(Version::new(1).unwrap())
            .event_type("E")
            .payload(b"p".as_slice())
            .build()
            .unwrap();
        let mut append = Box::pin(store.append(&id, None, PendingBatch::from_parts(&event, &[])));
        assert!(append.as_mut().now_or_never().is_none());
        drop(append);
        release.send(()).unwrap();
        blocker.await.unwrap();
        let mut rows = store
            .read_stream(&id, Version::new(1).unwrap())
            .await
            .unwrap();
        assert!(rows.next().await.is_none());
        assert!(wake.as_mut().now_or_never().is_none());
        store.close().await.unwrap();
    }

    #[cfg(feature = "projection")]
    #[derive(Debug)]
    struct Added(u8);
    #[cfg(feature = "projection")]
    impl mnesis::Message for Added {}
    #[cfg(feature = "projection")]
    impl mnesis::DomainEvent for Added {
        fn name(&self) -> &'static str {
            "added"
        }
    }
    #[cfg(feature = "projection")]
    struct ByteProjector;
    #[cfg(feature = "projection")]
    impl mnesis_store::Projector for ByteProjector {
        type Event = Added;
        type State = Vec<u8>;
        type Error = core::convert::Infallible;
        fn initial(&self) -> Vec<u8> {
            Vec::new()
        }
        fn apply(&self, mut state: Vec<u8>, event: &Added) -> Result<Vec<u8>, Self::Error> {
            state.push(event.0);
            Ok(state)
        }
    }

    #[cfg(feature = "projection")]
    #[tokio::test(flavor = "current_thread")]
    async fn canceling_a_queued_projection_write_requires_reload_without_persisting_it() {
        use mnesis_store::{AfterEventTypes, Decoded, Projection, ProjectionStateError};
        let directory = tempfile::tempdir().unwrap();
        let store = FjallStore::builder(directory.path()).open().unwrap();
        let id = StreamKey::from_slice(b"canceled-checkpoint");
        let mut projection = Projection::load(
            id.clone(),
            ByteProjector,
            AfterEventTypes::new(&["added"]),
            &store,
            std::num::NonZeroU32::MIN,
        )
        .await
        .unwrap();
        let (release, wait) = std::sync::mpsc::channel();
        let (started, entered) = futures::channel::oneshot::channel();
        let blocker = store.executor.reserve().await.unwrap().submit(move || {
            let _ = started.send(());
            let _ = wait.recv();
        });
        entered.await.unwrap();
        let event = || Decoded {
            event: Added(7),
            version: Version::INITIAL,
            metadata: None,
        };
        assert!(projection.advance(event()).now_or_never().is_none());
        assert_eq!(
            projection.state().unwrap_err(),
            ProjectionStateError::ReloadRequired
        );
        assert_eq!(projection.observed(), Some(Version::INITIAL));
        assert_eq!(projection.checkpoint(), None);
        release.send(()).unwrap();
        blocker.await.unwrap();
        drop(projection);
        let mut reloaded = Projection::load(
            id.clone(),
            ByteProjector,
            AfterEventTypes::new(&["added"]),
            &store,
            std::num::NonZeroU32::MIN,
        )
        .await
        .unwrap();
        assert_eq!(reloaded.state().unwrap(), &Vec::<u8>::new());
        assert_eq!(reloaded.checkpoint(), None);
        reloaded.advance(event()).await.unwrap();
        assert_eq!(reloaded.state().unwrap(), &[7]);
        drop(reloaded);
        store.close().await.unwrap();
        let reopened = FjallStore::builder(directory.path()).open().unwrap();
        let recovered = Projection::load(
            id,
            ByteProjector,
            AfterEventTypes::new(&["added"]),
            &reopened,
            std::num::NonZeroU32::MIN,
        )
        .await
        .unwrap();
        assert_eq!(recovered.state().unwrap(), &[7]);
        assert_eq!(recovered.checkpoint(), Some(Version::INITIAL));
        drop(recovered);
        reopened.close().await.unwrap();
    }

    #[cfg(feature = "projection")]
    struct PauseCheckpointCompletion<'a> {
        store: &'a FjallStore,
        committed: tokio::sync::Notify,
    }

    #[cfg(feature = "projection")]
    impl mnesis_store::checkpoint::CheckpointStore<Vec<u8>, Version> for PauseCheckpointCompletion<'_> {
        type Error = FjallError;
        async fn hydrate_checkpoint(
            &self,
            id: &impl mnesis::Id,
            schema: std::num::NonZeroU32,
        ) -> Result<mnesis_store::checkpoint::CheckpointHydrated<Vec<u8>, Version>, Self::Error>
        {
            mnesis_store::checkpoint::CheckpointStore::<Vec<u8>, Version>::hydrate_checkpoint(
                self.store, id, schema,
            )
            .await
        }
        async fn commit_checkpoint(
            &self,
            id: &impl mnesis::Id,
            write: mnesis_store::checkpoint::CheckpointWrite<'_, Vec<u8>, Version>,
        ) -> Result<std::num::NonZeroU64, mnesis_store::checkpoint::CheckpointError<Self::Error>>
        {
            // Delegate the complete write to the real public adapter, then
            // suspend this middleware's completion until the caller cancels.
            mnesis_store::checkpoint::CheckpointStore::<Vec<u8>, Version>::commit_checkpoint(
                self.store, id, write,
            )
            .await?;
            self.committed.notify_one();
            core::future::pending().await
        }
    }

    #[cfg(feature = "projection")]
    #[tokio::test(flavor = "current_thread")]
    #[allow(
        clippy::panic,
        reason = "completion must stay pending at the injected boundary"
    )]
    async fn canceling_projection_completion_after_fjall_commit_reloads_the_exact_record() {
        use mnesis_store::{AfterEventTypes, Decoded, Projection, ProjectionStateError};
        let directory = tempfile::tempdir().unwrap();
        let store = FjallStore::builder(directory.path()).open().unwrap();
        let id = StreamKey::from_slice(b"committed-checkpoint");
        let paused = PauseCheckpointCompletion {
            store: &store,
            committed: tokio::sync::Notify::new(),
        };
        let mut projection = Projection::load(
            id.clone(),
            ByteProjector,
            AfterEventTypes::new(&["added"]),
            &paused,
            std::num::NonZeroU32::MIN,
        )
        .await
        .unwrap();
        {
            let future = projection.advance(Decoded {
                event: Added(7),
                version: Version::INITIAL,
                metadata: None,
            });
            tokio::pin!(future);
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                tokio::select! {
                    () = paused.committed.notified() => {},
                    result = &mut future => panic!("completion cannot return before cancellation: {result:?}"),
                }
            }).await.unwrap();
            // Leaving this scope drops the still-pending advance future.
        }
        assert_eq!(
            projection.state().unwrap_err(),
            ProjectionStateError::ReloadRequired
        );
        assert_eq!(projection.checkpoint(), None);
        assert_eq!(projection.observed(), Some(Version::INITIAL));
        drop(projection);
        let recovered = Projection::<_, _, _, _, Version>::load(
            id.clone(),
            ByteProjector,
            AfterEventTypes::new(&["added"]),
            &store,
            std::num::NonZeroU32::MIN,
        )
        .await
        .unwrap();
        assert_eq!(recovered.state().unwrap(), &[7]);
        assert_eq!(recovered.checkpoint(), Some(Version::INITIAL));
        drop(recovered);
        store.close().await.unwrap();
        let reopened = FjallStore::builder(directory.path()).open().unwrap();
        let reloaded = Projection::<_, _, _, _, Version>::load(
            id,
            ByteProjector,
            AfterEventTypes::new(&["added"]),
            &reopened,
            std::num::NonZeroU32::MIN,
        )
        .await
        .unwrap();
        assert_eq!(reloaded.state().unwrap(), &[7]);
        assert_eq!(reloaded.checkpoint(), Some(Version::INITIAL));
        drop(reloaded);
        reopened.close().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn canceled_completion_after_commit_preserves_event_and_wake() {
        let directory = tempfile::tempdir().unwrap();
        let store = FjallStore::builder(directory.path()).open().unwrap();
        let id = StreamKey::from_slice(b"committed");
        let registration = store.register(Some(id.as_ref())).unwrap();
        let wake = registration.arm();
        let storage = Arc::clone(&store.storage);
        let owned_id = id.clone();
        let event = pending_envelope(Version::new(1).unwrap())
            .event_type("E")
            .payload(b"p".as_slice())
            .build()
            .unwrap();
        let (release, wait) = std::sync::mpsc::channel();
        let (committed, observed) = futures::channel::oneshot::channel();
        let completion = store.executor.reserve().await.unwrap().submit(move || {
            let position =
                block_on(storage.append(&owned_id, None, PendingBatch::from_parts(&event, &[])))
                    .unwrap();
            committed.send(position).unwrap();
            let _ = wait.recv();
        });
        assert_eq!(observed.await.unwrap().as_u64(), 1);
        drop(completion);
        assert!(wake.now_or_never().is_some());
        release.send(()).unwrap();
        let mut rows = store
            .read_stream(&id, Version::new(1).unwrap())
            .await
            .unwrap();
        assert!(rows.next().await.unwrap().is_ok());
        assert!(rows.next().await.is_none());
        store.close().await.unwrap();
        let reopened = FjallStore::builder(directory.path()).open().unwrap();
        let mut recovered = reopened
            .read_stream(&id, Version::new(1).unwrap())
            .await
            .unwrap();
        assert!(recovered.next().await.unwrap().is_ok());
        assert!(recovered.next().await.is_none());
        reopened.close().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn close_requires_clones_and_unfinished_scans_to_be_released() {
        let directory = tempfile::tempdir().unwrap();
        let store = FjallStore::builder(directory.path())
            .blocking(BlockingConfig::default())
            .open()
            .unwrap();
        let other = store.clone();
        assert!(matches!(
            store.close().await,
            Err(FjallError::OutstandingHandles { .. })
        ));
        let scan = other.read_all(None).await.unwrap();
        let last = other.clone();
        assert!(matches!(
            other.close().await,
            Err(FjallError::OutstandingHandles { .. })
        ));
        drop(scan);
        last.close().await.unwrap();
        let reopened = FjallStore::builder(directory.path()).open().unwrap();
        reopened.close().await.unwrap();
    }
    #[tokio::test(flavor = "current_thread")]
    async fn open_scan_limit_is_typed_and_does_not_block_other_operations() {
        let directory = tempfile::tempdir().unwrap();
        let config = BlockingConfig {
            max_open_scans: std::num::NonZeroUsize::new(1).unwrap(),
            ..BlockingConfig::default()
        };
        let store = FjallStore::builder(directory.path())
            .blocking(config)
            .open()
            .unwrap();
        let id = StreamKey::from_slice(b"limited");
        let scan = store
            .read_stream(&id, Version::new(1).unwrap())
            .await
            .unwrap();
        assert!(matches!(
            store.read_all(None).await,
            Err(FjallError::ScanCapacity(_))
        ));
        // Writes and flushes have independent admission; retaining the scan
        // does not consume their capacity.
        let event = pending_envelope(Version::new(1).unwrap())
            .event_type("E")
            .payload(b"p".as_slice())
            .build()
            .unwrap();
        assert_eq!(
            store
                .append(&id, None, PendingBatch::from_parts(&event, &[]))
                .await
                .unwrap()
                .as_u64(),
            1
        );
        store.flush().await.unwrap();
        drop(scan);
        store.close().await.unwrap();
    }
}
