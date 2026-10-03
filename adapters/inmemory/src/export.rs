//! A frozen set of stream heads over the adapter's immutable, append-only rows.

use crate::{InMemoryStore, InMemoryStoreError, frame_to_envelope};
use futures::Stream;
use mnesis::Version;
use mnesis_store::export::{ConsistentExporter, ExportError, ExportSession};
use mnesis_store::{PersistedEnvelope, StreamKey};
use std::collections::{HashMap, VecDeque};
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

/// A fixed set of ids and heads captured under the store's one commit lock.
///
/// Retains O(streams) key/head metadata, with no additional historical event
/// copies or storage pins. The source is borrowed for the session's lifetime.
/// The adapter never removes or rewrites event rows; reads are capped at the
/// captured heads, so later appends cannot change this view.
pub struct InMemoryExportSession<'a> {
    store: &'a InMemoryStore,
    heads: HashMap<StreamKey, u64>,
    deadline: Instant,
}

type Item<T> = Result<T, ExportError<InMemoryStoreError>>;
type InnerCursor<'a, T> = Pin<Box<dyn Stream<Item = Item<T>> + Send + 'a>>;

/// A lazy session cursor; expiration or the first error releases its buffer.
pub struct InMemoryExportCursor<'a, T> {
    inner: Option<InnerCursor<'a, T>>,
    deadline: Instant,
}

impl<T> Stream for InMemoryExportCursor<'_, T> {
    type Item = Item<T>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        let Some(inner) = this.inner.as_mut() else {
            return Poll::Ready(None);
        };
        if Instant::now() >= this.deadline {
            this.inner = None;
            return Poll::Ready(Some(Err(ExportError::Expired)));
        }
        let result = inner.as_mut().poll_next(cx);
        if matches!(result, Poll::Ready(None | Some(Err(_)))) {
            this.inner = None;
        }
        result
    }
}

impl ConsistentExporter for InMemoryStore {
    type Session<'a> = InMemoryExportSession<'a>;

    async fn open_export_session(
        &self,
        lifetime: Duration,
    ) -> Result<Self::Session<'_>, ExportError<Self::Error>> {
        if lifetime.is_zero() {
            return Err(ExportError::InvalidLifetime { lifetime });
        }
        let guard = self.inner.lock().await;
        let deadline = Instant::now()
            .checked_add(lifetime)
            .ok_or(ExportError::InvalidLifetime { lifetime })?;
        let heads = guard
            .streams
            .iter()
            .filter_map(|(id, rows)| rows.last().map(|last| (id.clone(), last.version)))
            .collect();
        drop(guard);
        Ok(InMemoryExportSession {
            store: self,
            heads,
            deadline,
        })
    }
}

impl InMemoryExportSession<'_> {
    fn check_live(&self) -> Result<(), ExportError<InMemoryStoreError>> {
        if Instant::now() >= self.deadline {
            Err(ExportError::Expired)
        } else {
            Ok(())
        }
    }
}

impl ExportSession for InMemoryExportSession<'_> {
    type Error = InMemoryStoreError;
    type StreamList<'a>
        = InMemoryExportCursor<'a, StreamKey>
    where
        Self: 'a;
    type ExportStream<'a>
        = InMemoryExportCursor<'a, PersistedEnvelope>
    where
        Self: 'a;

    async fn list_streams(&self) -> Result<Self::StreamList<'_>, ExportError<Self::Error>> {
        self.check_live()?;
        Ok(InMemoryExportCursor {
            inner: Some(Box::pin(futures::stream::iter(
                self.heads.keys().cloned().map(Ok),
            ))),
            deadline: self.deadline,
        })
    }

    async fn export_stream(
        &self,
        id: &StreamKey,
        from: Version,
    ) -> Result<Self::ExportStream<'_>, ExportError<Self::Error>> {
        self.check_live()?;
        let head = self.heads.get(id).copied();
        let key = id.clone();
        let rows =
            futures::stream::unfold((Some(from), VecDeque::new()), move |(next, mut buffer)| {
                let read_key = key.clone();
                async move {
                    if buffer.is_empty() {
                        let start = next?;
                        let ceiling = head?;
                        if start.as_u64() > ceiling {
                            return None;
                        }
                        let guard = self.store.inner.lock().await;
                        let history = guard.streams.get(&read_key)?;
                        let first = history.partition_point(|row| row.version < start.as_u64());
                        buffer = history[first..]
                            .iter()
                            .take_while(|row| row.version <= ceiling)
                            .take(self.store.batch_size.get())
                            .cloned()
                            .collect();
                        drop(guard);
                    }
                    let row = buffer.pop_front()?;
                    let envelope = frame_to_envelope(&row).map_err(ExportError::Store);
                    let following = envelope
                        .as_ref()
                        .ok()
                        .and_then(|event| event.version().next());
                    Some((envelope, (following, buffer)))
                }
            });
        Ok(InMemoryExportCursor {
            inner: Some(Box::pin(rows)),
            deadline: self.deadline,
        })
    }

    async fn close(self) -> Result<(), ExportError<Self::Error>> {
        drop(self);
        Ok(())
    }
}

#[cfg(all(test, feature = "import"))]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod tests {
    use super::{Duration, ExportError, ExportSession, InMemoryStore, Instant};
    use futures::{FutureExt, StreamExt, TryStreamExt};
    use mnesis::Version;
    use mnesis_store::batch::BatchSize;
    use mnesis_store::export::ConsistentExporter;
    use mnesis_store::import::{AtomicAppend, PlannedAppend};
    use mnesis_store::{Store, StreamKey, pending_envelope};

    async fn append_both(store: &InMemoryStore, ids: &[StreamKey], version: u64) {
        let writes = ids
            .iter()
            .map(|id| PlannedAppend {
                target: id.clone(),
                expected_version: Version::new(version - 1),
                head: pending_envelope(Version::new(version).unwrap())
                    .event_type("E")
                    .payload(format!("v{version}").into_bytes())
                    .build()
                    .unwrap(),
                tail: vec![],
            })
            .collect::<Vec<_>>();
        store.atomic_append_many(&writes).await.unwrap();
    }

    #[tokio::test]
    async fn session_never_splits_an_atomic_commit_or_includes_new_streams() {
        for batch in [1, 2, 64] {
            let raw = InMemoryStore::with_batch_size(BatchSize::new(batch).unwrap());
            let store = Store::new(raw);
            let ids = [
                StreamKey::from_slice(b"a"),
                StreamKey::from_slice(&[0xff, 0]),
            ];
            append_both(store.raw(), &ids, 1).await;
            let session = store
                .open_export_session(Duration::from_secs(60))
                .await
                .unwrap();
            let mut a = session
                .export_stream(&ids[0], Version::INITIAL)
                .await
                .unwrap();
            append_both(store.raw(), &ids, 2).await;
            let new_id = StreamKey::from_slice(b"later");
            append_both(store.raw(), std::slice::from_ref(&new_id), 1).await;
            let first = a.next().await.unwrap().unwrap();
            assert_eq!(first.version(), Version::INITIAL);
            assert_eq!(first.payload(), b"v1");
            assert!(a.next().await.is_none());
            let b = session
                .export_stream(&ids[1], Version::INITIAL)
                .await
                .unwrap()
                .try_collect::<Vec<_>>()
                .await
                .unwrap();
            assert_eq!(b.len(), 1);
            assert_eq!(b[0].version(), Version::INITIAL);
            assert_eq!(b[0].payload(), b"v1");
            let mut listed = session
                .list_streams()
                .await
                .unwrap()
                .try_collect::<Vec<_>>()
                .await
                .unwrap();
            listed.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
            assert_eq!(listed, ids);
            for (id, from) in [
                (&new_id, Version::INITIAL),
                (&ids[1], Version::new(2).unwrap()),
                (&ids[1], Version::new(u64::MAX).unwrap()),
            ] {
                let mut empty = session.export_stream(id, from).await.unwrap();
                assert!(empty.next().await.is_none());
                assert!(empty.next().await.is_none());
            }
            drop(a);
            session.close().await.unwrap();
            let newer = store
                .open_export_session(Duration::from_secs(60))
                .await
                .unwrap();
            let events = newer
                .export_stream(&ids[1], Version::INITIAL)
                .await
                .unwrap()
                .try_collect::<Vec<_>>()
                .await
                .unwrap();
            assert_eq!(events.len(), 2);
            assert_eq!(events[1].payload(), b"v2");
        }
    }

    #[tokio::test]
    async fn expiration_rejects_opening_and_fuses_live_cursors() {
        let store = InMemoryStore::new();
        let id = StreamKey::from_slice(b"a");
        append_both(&store, std::slice::from_ref(&id), 1).await;
        let mut session = store
            .open_export_session(Duration::from_secs(60))
            .await
            .unwrap();
        // Exercise the same deadline comparison deterministically, without a
        // timing threshold. Already-exhausted cursors remain fused.
        let mut exhausted = session
            .export_stream(&id, Version::new(2).unwrap())
            .await
            .unwrap();
        assert!(exhausted.next().await.is_none());
        exhausted.deadline = Instant::now();
        assert!(exhausted.next().await.is_none());
        drop(exhausted);
        let mut active = session.export_stream(&id, Version::INITIAL).await.unwrap();
        active.deadline = Instant::now();
        assert!(matches!(
            active.next().await,
            Some(Err(ExportError::Expired))
        ));
        assert!(active.next().await.is_none());
        drop(active);
        session.deadline = Instant::now();
        assert!(matches!(
            session.list_streams().await,
            Err(ExportError::Expired)
        ));
        assert!(matches!(
            session.export_stream(&id, Version::INITIAL).await,
            Err(ExportError::Expired)
        ));
        session.close().await.unwrap();
    }

    #[tokio::test]
    async fn zero_or_unrepresentable_lifetime_is_typed_input_rejection() {
        let store = InMemoryStore::new();
        for lifetime in [Duration::ZERO, Duration::MAX] {
            let result = store.open_export_session(lifetime).await;
            assert!(
                matches!(result, Err(ExportError::InvalidLifetime { lifetime: actual }) if actual == lifetime)
            );
        }
    }

    #[tokio::test]
    async fn canceled_open_and_read_release_pending_lock_acquisition() {
        let store = InMemoryStore::new();
        let id = StreamKey::from_slice(b"a");
        append_both(&store, std::slice::from_ref(&id), 1).await;
        let guard = store.inner.lock().await;
        let mut opening = Box::pin(store.open_export_session(Duration::from_secs(60)));
        assert!(opening.as_mut().now_or_never().is_none());
        drop(opening);
        drop(guard);
        let session = store
            .open_export_session(Duration::from_secs(60))
            .await
            .unwrap();
        let held = store.inner.lock().await;
        let mut cursor = session.export_stream(&id, Version::INITIAL).await.unwrap();
        let mut reading = Box::pin(cursor.next());
        assert!(reading.as_mut().now_or_never().is_none());
        drop(reading);
        drop(cursor);
        drop(held);
        tokio::time::timeout(
            Duration::from_secs(1),
            append_both(&store, std::slice::from_ref(&id), 2),
        )
        .await
        .unwrap();
        let events = session
            .export_stream(&id, Version::INITIAL)
            .await
            .unwrap()
            .try_collect::<Vec<_>>()
            .await
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].payload(), b"v1");
        session.close().await.unwrap();
    }

    #[tokio::test]
    async fn refilling_cannot_extend_the_frozen_head() {
        let store = InMemoryStore::with_batch_size(BatchSize::new(2).unwrap());
        let id = StreamKey::from_slice(b"paged");
        for version in 1..=5 {
            append_both(&store, std::slice::from_ref(&id), version).await;
        }
        let session = store
            .open_export_session(Duration::from_secs(60))
            .await
            .unwrap();
        let mut cursor = session
            .export_stream(&id, Version::new(2).unwrap())
            .await
            .unwrap();
        let first = cursor.next().await.unwrap().unwrap();
        assert_eq!(first.version(), Version::new(2).unwrap());
        assert_eq!(first.payload(), b"v2");
        append_both(&store, std::slice::from_ref(&id), 6).await;
        for expected in 3..=5 {
            let event = cursor.next().await.unwrap().unwrap();
            assert_eq!(event.version(), Version::new(expected).unwrap());
            assert_eq!(event.payload(), format!("v{expected}").as_bytes());
        }
        assert!(cursor.next().await.is_none());
        assert!(cursor.next().await.is_none());
        drop(cursor);
        session.close().await.unwrap();
    }
}
