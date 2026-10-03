//! Deadline-bound database views owned exclusively by the I/O worker.

use crate::blocking::{BlockingConfig, Cleanup, Cursor, CursorSlot};
use crate::scan::{ScanCursor, StreamScan};
use crate::store::Storage;
use crate::store::stream_lister_impl::StreamIdCursor;
use crate::{FjallError, FjallStore};
use futures::Stream;
use mnesis::{ErrorId, Version};
use mnesis_store::export::{ConsistentExporter, ExportError, ExportSession};
use mnesis_store::{PersistedEnvelope, StreamKey};
use std::collections::{BTreeMap, VecDeque};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

type Error = ExportError<FjallError>;
type Item<T> = Result<T, Error>;
type InnerCursor<'a, T> = Pin<Box<dyn Stream<Item = Item<T>> + Send + 'a>>;

struct View {
    snapshot: fjall::Snapshot,
    deadline: Instant,
    #[cfg(test)]
    drop_notice: Option<futures::channel::oneshot::Sender<std::thread::ThreadId>>,
}

/// Worker-local registry. Each live view has one reserved cleanup slot.
pub struct Views {
    active: BTreeMap<u64, View>,
    next_id: Option<u64>,
}

impl Default for Views {
    fn default() -> Self {
        Self {
            active: BTreeMap::new(),
            next_id: Some(1),
        }
    }
}

impl Views {
    pub fn expire(&mut self) {
        let now = Instant::now();
        self.active.retain(|_, view| view.deadline > now);
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.active.values().map(|view| view.deadline).min()
    }

    fn get(&self, id: u64) -> Result<&fjall::Snapshot, Error> {
        let view = self.active.get(&id).ok_or(ExportError::Expired)?;
        if Instant::now() >= view.deadline {
            return Err(ExportError::Expired);
        }
        Ok(&view.snapshot)
    }

    fn open(
        &mut self,
        storage: &Storage,
        lifetime: Duration,
        slot: CursorSlot,
    ) -> Result<Lease, Error> {
        let id = self
            .next_id
            .ok_or(ExportError::Store(FjallError::ExportSessionIdOverflow))?;
        let deadline = Instant::now()
            .checked_add(lifetime)
            .filter(|_| !lifetime.is_zero())
            .ok_or(ExportError::InvalidLifetime { lifetime })?;
        let snapshot = storage.db.read_tx();
        self.active.insert(
            id,
            View {
                snapshot,
                deadline,
                #[cfg(test)]
                drop_notice: None,
            },
        );
        self.next_id = id.checked_add(1);
        let cleanup = slot.attach_cleanup(move |worker| {
            worker.views.active.remove(&id);
        });
        Ok(Lease {
            id,
            deadline,
            cleanup,
        })
    }
}

struct Lease {
    id: u64,
    deadline: Instant,
    cleanup: Cleanup,
}

/// One consistent cross-keyspace view, borrowing its source store.
///
/// The worker releases this view at expiry, drop or explicit close. Live views
/// can retain old database versions and delay reclamation. Session and cursor
/// handles each consume one `BlockingConfig::max_open_scans` cleanup slot;
/// keeping an expired handle still consumes its admission slot until released.
/// Iterators exist only within bounded worker fetches, so idle cursors do not
/// keep expired views pinned. Already-yielded envelopes retain their own bytes.
pub struct FjallExportSession<'a> {
    store: &'a FjallStore,
    lease: Lease,
}

/// Bounded, lazy session cursor. Expiry and errors are terminal.
pub struct FjallExportCursor<'a, T> {
    inner: Option<InnerCursor<'a, T>>,
    deadline: Instant,
    cleanup: Option<Cleanup>,
}

impl<T> Stream for FjallExportCursor<'_, T> {
    type Item = Item<T>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        let Some(inner) = this.inner.as_mut() else {
            return Poll::Ready(None);
        };
        let result = if Instant::now() >= this.deadline {
            Poll::Ready(Some(Err(ExportError::Expired)))
        } else {
            inner.as_mut().poll_next(cx)
        };
        if matches!(result, Poll::Ready(None | Some(Err(_)))) {
            this.inner = None;
            this.cleanup = None;
        }
        result
    }
}

impl ConsistentExporter for FjallStore {
    type Session<'a> = FjallExportSession<'a>;

    async fn open_export_session(&self, lifetime: Duration) -> Result<Self::Session<'_>, Error> {
        if lifetime.is_zero() || Instant::now().checked_add(lifetime).is_none() {
            return Err(ExportError::InvalidLifetime { lifetime });
        }
        let cleanup = self.executor.reserve_cursor().map_err(ExportError::Store)?;
        let slot = self.executor.reserve().await.map_err(ExportError::Store)?;
        let storage = Arc::clone(&self.storage);
        let lease = slot
            .submit_with(move |worker| worker.views.open(&storage, lifetime, cleanup))
            .await
            .map_err(ExportError::Store)??;
        Ok(FjallExportSession { store: self, lease })
    }
}

trait SessionScan: Send + 'static {
    type Item: Send + 'static;
    type Cursor: Cursor<Item = Self::Item>;
    fn open(
        &self,
        snapshot: &fjall::Snapshot,
        storage: &Storage,
    ) -> Result<Self::Cursor, FjallError>;
    fn advance(&mut self, item: &Self::Item) -> bool;
}

struct Listing {
    after: Option<StreamKey>,
}
struct Events {
    id: StreamKey,
    from: Version,
}

impl SessionScan for Listing {
    type Item = StreamKey;
    type Cursor = StreamIdCursor;
    fn open(
        &self,
        snapshot: &fjall::Snapshot,
        storage: &Storage,
    ) -> Result<Self::Cursor, FjallError> {
        Ok(StreamIdCursor::from_iter(
            storage
                .partitions
                .stream_ids_after(snapshot, self.after.as_ref()),
        ))
    }
    fn advance(&mut self, item: &StreamKey) -> bool {
        self.after = Some(item.clone());
        true
    }
}

impl SessionScan for Events {
    type Item = PersistedEnvelope;
    type Cursor = ScanCursor<StreamScan>;
    fn open(
        &self,
        snapshot: &fjall::Snapshot,
        storage: &Storage,
    ) -> Result<Self::Cursor, FjallError> {
        ScanCursor::open_snapshot(
            snapshot,
            storage.partitions.events(),
            StreamScan {
                id: self.id.clone(),
                label: ErrorId::from_display(&self.id),
            },
            self.from,
        )
    }
    fn advance(&mut self, item: &PersistedEnvelope) -> bool {
        let Some(next) = item.version().next() else {
            return false;
        };
        self.from = next;
        true
    }
}

struct Batch<S: SessionScan> {
    scan: Option<S>,
    rows: VecDeque<Item<S::Item>>,
}

fn fetch<S: SessionScan>(
    snapshot: &fjall::Snapshot,
    storage: &Storage,
    mut scan: S,
    config: BlockingConfig,
) -> Result<Batch<S>, Error> {
    let mut cursor = scan.open(snapshot, storage).map_err(ExportError::Store)?;
    let mut rows = VecDeque::new();
    let mut bytes = 0usize;
    for _ in 0..config.scan_batch_rows.get() {
        let Some((row, size)) = cursor.next_row() else {
            return Ok(Batch { scan: None, rows });
        };
        let item = match row {
            Ok(item) => item,
            Err(error) => {
                rows.push_back(Err(ExportError::Store(error)));
                return Ok(Batch { scan: None, rows });
            }
        };
        let Some(total) = bytes.checked_add(size) else {
            rows.push_back(Err(ExportError::Store(FjallError::ScanLengthOverflow)));
            return Ok(Batch { scan: None, rows });
        };
        let more = scan.advance(&item);
        rows.push_back(Ok(item));
        if !more {
            return Ok(Batch { scan: None, rows });
        }
        bytes = total;
        if bytes >= config.scan_batch_bytes.get() {
            break;
        }
    }
    Ok(Batch {
        scan: Some(scan),
        rows,
    })
}

impl FjallExportSession<'_> {
    fn check_live(&self) -> Result<(), Error> {
        if Instant::now() >= self.lease.deadline {
            Err(ExportError::Expired)
        } else {
            Ok(())
        }
    }

    fn cursor<S: SessionScan>(&self, initial: S) -> Result<FjallExportCursor<'_, S::Item>, Error> {
        self.check_live()?;
        let cleanup = self
            .store
            .executor
            .reserve_cursor()
            .map_err(ExportError::Store)?
            .attach_cleanup(|_| {});
        let stream = futures::stream::unfold(
            Batch {
                scan: Some(initial),
                rows: VecDeque::new(),
            },
            move |mut batch| async move {
                if batch.rows.is_empty() {
                    let scan = batch.scan.take()?;
                    let storage = Arc::clone(&self.store.storage);
                    let id = self.lease.id;
                    let config = self.store.config;
                    let result = async {
                        self.store
                            .executor
                            .reserve()
                            .await
                            .map_err(ExportError::Store)?
                            .submit_with(move |worker| {
                                fetch(worker.views.get(id)?, &storage, scan, config)
                            })
                            .await
                            .map_err(ExportError::Store)?
                    }
                    .await;
                    batch = match result {
                        Ok(fetched) => fetched,
                        Err(error) => Batch {
                            scan: None,
                            rows: VecDeque::from([Err(error)]),
                        },
                    };
                }
                batch.rows.pop_front().map(|row| (row, batch))
            },
        );
        Ok(FjallExportCursor {
            inner: Some(Box::pin(stream)),
            deadline: self.lease.deadline,
            cleanup: Some(cleanup),
        })
    }
}

impl ExportSession for FjallExportSession<'_> {
    type Error = FjallError;
    type StreamList<'a>
        = FjallExportCursor<'a, StreamKey>
    where
        Self: 'a;
    type ExportStream<'a>
        = FjallExportCursor<'a, PersistedEnvelope>
    where
        Self: 'a;

    async fn list_streams(&self) -> Result<Self::StreamList<'_>, Error> {
        self.cursor(Listing { after: None })
    }

    async fn export_stream(
        &self,
        id: &StreamKey,
        from: Version,
    ) -> Result<Self::ExportStream<'_>, Error> {
        self.check_live()?;
        crate::limits::validate_key(id.as_bytes(), crate::MAX_STREAM_ID_LEN)
            .map_err(ExportError::Store)?;
        self.cursor(Events {
            id: id.clone(),
            from,
        })
    }

    async fn close(self) -> Result<(), Error> {
        self.lease
            .cleanup
            .finish()
            .await
            .map_err(ExportError::Store)
    }
}

#[cfg(test)]
impl Drop for View {
    fn drop(&mut self) {
        if let Some(notice) = self.drop_notice.take() {
            let _ = notice.send(std::thread::current().id());
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "native storage test assertions")]
mod tests {
    use super::{Duration, ExportError, FjallError, FjallExportSession, FjallStore};
    use crate::{AllIndex, BlockingConfig};
    use futures::channel::oneshot;
    use futures::{FutureExt, StreamExt};
    use mnesis::Version;
    use mnesis_store::export::{ConsistentExporter, ExportSession};
    use mnesis_store::{PendingBatch, RawEventStore, StreamKey, pending_envelope};
    use std::num::NonZeroUsize;
    use std::sync::Arc;
    use std::thread::ThreadId;

    async fn watch_drop(
        session: &FjallExportSession<'_>,
    ) -> (ThreadId, oneshot::Receiver<ThreadId>) {
        let (notice, observed) = oneshot::channel();
        let id = session.lease.id;
        let worker = session
            .store
            .executor
            .reserve()
            .await
            .unwrap()
            .submit_with(move |state| {
                state.views.active.get_mut(&id).unwrap().drop_notice = Some(notice);
                std::thread::current().id()
            })
            .await
            .unwrap();
        (worker, observed)
    }

    async fn view_count(store: &FjallStore) -> usize {
        store
            .executor
            .reserve()
            .await
            .unwrap()
            .submit_with(|worker| worker.views.active.len())
            .await
            .unwrap()
    }

    #[test]
    fn session_reads_and_close_require_no_calling_tokio_runtime() {
        assert!(tokio::runtime::Handle::try_current().is_err());
        let directory = tempfile::tempdir().unwrap();
        let store = FjallStore::builder(directory.path()).open().unwrap();
        futures::executor::block_on(async {
            let id = StreamKey::from_slice(b"plain executor");
            let event = pending_envelope(Version::INITIAL)
                .event_type("E")
                .payload(b"p".as_slice())
                .build()
                .unwrap();
            store
                .append(&id, None, PendingBatch::of(&event))
                .await
                .unwrap();
            let session = store
                .open_export_session(Duration::from_secs(60))
                .await
                .unwrap();
            let mut events = session.export_stream(&id, Version::INITIAL).await.unwrap();
            let first = events.next().await.unwrap().unwrap();
            assert_eq!(first.version(), Version::INITIAL);
            assert_eq!(first.payload(), b"p");
            assert!(events.next().await.is_none());
            drop(events);
            session.close().await.unwrap();
            store.close().await.unwrap();
        });
        assert!(tokio::runtime::Handle::try_current().is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn canceling_a_next_item_wait_preserves_the_cursor_and_view() {
        let directory = tempfile::tempdir().unwrap();
        let store = FjallStore::builder(directory.path()).open().unwrap();
        let id = StreamKey::from_slice(b"canceled wait");
        let event = pending_envelope(Version::INITIAL)
            .event_type("E")
            .payload(b"before".as_slice())
            .build()
            .unwrap();
        store
            .append(&id, None, PendingBatch::of(&event))
            .await
            .unwrap();
        let session = store
            .open_export_session(Duration::from_secs(60))
            .await
            .unwrap();
        let mut cursor = session.export_stream(&id, Version::INITIAL).await.unwrap();
        let (release, wait) = std::sync::mpsc::channel();
        let (started, entered) = oneshot::channel();
        let running = store.executor.reserve().await.unwrap().submit(move || {
            started.send(()).unwrap();
            wait.recv().unwrap();
        });
        entered.await.unwrap();
        let mut reading = Box::pin(cursor.next());
        assert!(reading.as_mut().now_or_never().is_none());
        drop(reading);
        release.send(()).unwrap();
        running.await.unwrap();
        let later = pending_envelope(Version::new(2).unwrap())
            .event_type("E")
            .payload(b"after".as_slice())
            .build()
            .unwrap();
        store
            .append(&id, Some(Version::INITIAL), PendingBatch::of(&later))
            .await
            .unwrap();
        let first = cursor.next().await.unwrap().unwrap();
        assert_eq!(first.version(), Version::INITIAL);
        assert_eq!(first.payload(), b"before");
        assert!(cursor.next().await.is_none());
        drop(cursor);
        session.close().await.unwrap();
        assert_eq!(view_count(&store).await, 0);
        store.close().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn idle_expiration_destroys_the_view_on_the_worker_without_a_poll() {
        for mode in [AllIndex::Denormalized, AllIndex::Disabled] {
            let directory = tempfile::tempdir().unwrap();
            let store = FjallStore::builder(directory.path())
                .all_index(mode)
                .open()
                .unwrap();
            let id = StreamKey::from_slice(b"buffered");
            let head = pending_envelope(Version::INITIAL)
                .event_type("E")
                .payload(b"p1".as_slice())
                .build()
                .unwrap();
            let tail = pending_envelope(Version::new(2).unwrap())
                .event_type("E")
                .payload(b"p2".as_slice())
                .build()
                .unwrap();
            store
                .append(&id, None, PendingBatch::from_parts(&head, &[tail]))
                .await
                .unwrap();
            let session = store
                .open_export_session(Duration::from_secs(1))
                .await
                .unwrap();
            let mut cursor = session.list_streams().await.unwrap();
            let mut buffered = session.export_stream(&id, Version::INITIAL).await.unwrap();
            let owned = buffered.next().await.unwrap().unwrap();
            assert_eq!(owned.version(), Version::INITIAL);
            let (worker, observed) = watch_drop(&session).await;
            assert_ne!(worker, std::thread::current().id());
            // The only next action is waiting for Drop. No store job or cursor
            // poll can trigger the deadline sweep on behalf of the timer.
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(5), observed)
                    .await
                    .unwrap()
                    .unwrap(),
                worker
            );
            assert!(matches!(
                cursor.next().await,
                Some(Err(ExportError::Expired))
            ));
            assert!(cursor.next().await.is_none());
            assert!(matches!(
                buffered.next().await,
                Some(Err(ExportError::Expired))
            ));
            assert!(buffered.next().await.is_none());
            drop(buffered);
            drop(cursor);
            assert!(matches!(
                session.list_streams().await,
                Err(ExportError::Expired)
            ));
            assert_eq!(view_count(&store).await, 0);
            session.close().await.unwrap();
            store.close().await.unwrap();
            assert_eq!(owned.payload(), b"p1");
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn drop_with_a_full_job_queue_still_releases_the_view_on_the_worker() {
        let directory = tempfile::tempdir().unwrap();
        let store = FjallStore::builder(directory.path())
            .blocking(BlockingConfig {
                queue_capacity: NonZeroUsize::new(1).unwrap(),
                ..BlockingConfig::default()
            })
            .open()
            .unwrap();
        let session = store
            .open_export_session(Duration::from_secs(60))
            .await
            .unwrap();
        let cursor = session.list_streams().await.unwrap();
        let (worker, observed) = watch_drop(&session).await;
        let (release, wait) = std::sync::mpsc::channel();
        let (started, entered) = oneshot::channel();
        let running = store.executor.reserve().await.unwrap().submit(move || {
            started.send(()).unwrap();
            wait.recv().unwrap();
        });
        entered.await.unwrap();
        let queued = store.executor.reserve().await.unwrap().submit(|| 42);
        drop(cursor);
        drop(session);
        release.send(()).unwrap();
        running.await.unwrap();
        assert_eq!(queued.await.unwrap(), 42);
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), observed)
                .await
                .unwrap()
                .unwrap(),
            worker
        );
        assert_eq!(view_count(&store).await, 0);
        store.close().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancel_before_start_creates_no_view_and_cancel_after_create_cleans_it_up() {
        let directory = tempfile::tempdir().unwrap();
        let store = FjallStore::builder(directory.path()).open().unwrap();
        let (release, wait) = std::sync::mpsc::channel();
        let (started, entered) = oneshot::channel();
        let running = store.executor.reserve().await.unwrap().submit(move || {
            started.send(()).unwrap();
            wait.recv().unwrap();
        });
        entered.await.unwrap();
        let mut opening = Box::pin(store.open_export_session(Duration::from_secs(60)));
        assert!(opening.as_mut().now_or_never().is_none());
        drop(opening);
        release.send(()).unwrap();
        running.await.unwrap();
        assert_eq!(view_count(&store).await, 0);
        assert_eq!(
            store
                .executor
                .reserve()
                .await
                .unwrap()
                .submit_with(|state| state.views.next_id)
                .await
                .unwrap(),
            Some(1)
        );

        let storage = Arc::clone(&store.storage);
        let cleanup = store.executor.reserve_cursor().unwrap();
        let (resume, blocked) = std::sync::mpsc::channel();
        let (created, ready) = oneshot::channel();
        let (destroyed, observed) = oneshot::channel();
        let pending = store
            .executor
            .reserve()
            .await
            .unwrap()
            .submit_with(move |state| {
                let lease = state
                    .views
                    .open(&storage, Duration::from_secs(60), cleanup)
                    .unwrap();
                state.views.active.get_mut(&lease.id).unwrap().drop_notice = Some(destroyed);
                created.send(std::thread::current().id()).unwrap();
                blocked.recv().unwrap();
                lease
            });
        let worker = ready.await.unwrap();
        drop(pending);
        resume.send(()).unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), observed)
                .await
                .unwrap()
                .unwrap(),
            worker
        );
        assert_eq!(view_count(&store).await, 0);
        store.close().await.unwrap();
    }

    #[tokio::test]
    async fn session_and_cursor_admission_is_bounded_and_close_releases_capacity() {
        let directory = tempfile::tempdir().unwrap();
        let store = FjallStore::builder(directory.path())
            .blocking(BlockingConfig {
                max_open_scans: NonZeroUsize::new(2).unwrap(),
                ..BlockingConfig::default()
            })
            .open()
            .unwrap();
        let session = store
            .open_export_session(Duration::from_secs(60))
            .await
            .unwrap();
        let mut cursor = session.list_streams().await.unwrap();
        assert!(matches!(
            session.list_streams().await,
            Err(ExportError::Store(FjallError::ScanCapacity(
                tokio::sync::mpsc::error::TrySendError::Full(())
            )))
        ));
        assert!(matches!(
            store.open_export_session(Duration::from_secs(60)).await,
            Err(ExportError::Store(FjallError::ScanCapacity(
                tokio::sync::mpsc::error::TrySendError::Full(())
            )))
        ));
        assert!(cursor.next().await.is_none());
        drop(cursor);
        session.close().await.unwrap();
        assert_eq!(view_count(&store).await, 0);
        let next = store
            .open_export_session(Duration::from_secs(60))
            .await
            .unwrap();
        next.close().await.unwrap();
        store.close().await.unwrap();
    }

    #[tokio::test]
    async fn invalid_lifetime_keys_and_identifier_overflow_do_not_create_extra_views() {
        let directory = tempfile::tempdir().unwrap();
        let store = FjallStore::builder(directory.path()).open().unwrap();
        for lifetime in [Duration::ZERO, Duration::MAX] {
            assert!(matches!(store.open_export_session(lifetime).await,
                Err(ExportError::InvalidLifetime { lifetime: actual }) if actual == lifetime));
        }
        assert_eq!(view_count(&store).await, 0);
        store
            .executor
            .reserve()
            .await
            .unwrap()
            .submit_with(|state| state.views.next_id = Some(u64::MAX))
            .await
            .unwrap();
        let session = store
            .open_export_session(Duration::from_secs(60))
            .await
            .unwrap();
        assert_eq!(session.lease.id, u64::MAX);
        let empty = StreamKey::from_slice(b"");
        assert!(matches!(
            session.export_stream(&empty, Version::INITIAL).await,
            Err(ExportError::Store(FjallError::InvalidKey {
                len: 0,
                max: crate::MAX_STREAM_ID_LEN
            }))
        ));
        assert!(matches!(
            store.open_export_session(Duration::from_secs(60)).await,
            Err(ExportError::Store(FjallError::ExportSessionIdOverflow))
        ));
        assert_eq!(view_count(&store).await, 1);
        session.close().await.unwrap();
        assert_eq!(view_count(&store).await, 0);
        let event = pending_envelope(Version::INITIAL)
            .event_type("E")
            .payload(b"v1".as_slice())
            .build()
            .unwrap();
        let id = StreamKey::from_slice(b"still writable");
        assert_eq!(
            store
                .append(&id, None, PendingBatch::of(&event))
                .await
                .unwrap()
                .as_u64(),
            1
        );
        store.close().await.unwrap();
    }
}
