#![allow(
    dead_code,
    reason = "different integration targets use different controlled faults"
)]

use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use futures::{Stream, StreamExt, TryStreamExt};
use mnesis::Version;
use mnesis_store::{AppendError, PendingBatch, PersistedEnvelope, RawEventStore, StreamKey};
use parking_lot::Mutex;
use tokio::sync::Semaphore;
pub struct Gate {
    entered: Semaphore,
    release: Semaphore,
    done: Semaphore,
}
impl Gate {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            entered: Semaphore::new(0),
            release: Semaphore::new(0),
            done: Semaphore::new(0),
        })
    }
    pub async fn entered(&self) {
        tokio::time::timeout(Duration::from_secs(2), self.entered.acquire())
            .await
            .expect("controlled fault boundary was not reached")
            .unwrap()
            .forget();
    }
    pub fn release(&self) {
        self.release.add_permits(1);
    }
    pub async fn done(&self) {
        tokio::time::timeout(Duration::from_secs(2), self.done.acquire())
            .await
            .expect("controlled server operation did not finish")
            .unwrap()
            .forget();
    }
    pub async fn wait(&self) {
        self.entered.add_permits(1);
        self.release.acquire().await.unwrap().forget();
    }
}
#[derive(Default)]
pub enum Fault {
    #[default]
    Normal,
    CommitLost,
    Rollback,
    CompetingCommit,
    Delayed(Arc<Gate>),
}
#[derive(Debug, thiserror::Error)]
pub enum FaultError<E> {
    #[error(transparent)]
    Store(E),
    #[error("write response unavailable")]
    Lost,
}
pub struct FaultStore<S> {
    pub inner: Arc<S>,
    fault: Mutex<Fault>,
    pub read_gate: Mutex<Option<Arc<Gate>>>,
    pub after_read: Mutex<Option<(usize, Arc<Gate>)>>,
}
impl<S> FaultStore<S> {
    pub fn new(inner: S) -> Self {
        Self {
            inner: Arc::new(inner),
            fault: Mutex::new(Fault::Normal),
            read_gate: Mutex::new(None),
            after_read: Mutex::new(None),
        }
    }
    pub fn arm(&self, fault: Fault) {
        *self.fault.lock() = fault;
    }
}
fn append_error<E: std::error::Error + 'static>(
    error: AppendError<E>,
) -> AppendError<FaultError<E>> {
    match error {
        AppendError::Conflict {
            stream_id,
            expected,
            actual,
        } => AppendError::Conflict {
            stream_id,
            expected,
            actual,
        },
        AppendError::Store(cause) => AppendError::Store(FaultError::Store(cause)),
        #[allow(
            clippy::panic,
            reason = "new upstream error variants must fail this test harness loudly"
        )]
        _ => panic!("unsupported upstream append error"),
    }
}
impl<S: RawEventStore + 'static> RawEventStore for FaultStore<S> {
    type Error = FaultError<S::Error>;
    type Stream = Pin<Box<dyn Stream<Item = Result<PersistedEnvelope, Self::Error>> + Send>>;
    type AllPosition = S::AllPosition;
    type AllStream = Pin<
        Box<
            dyn Stream<Item = Result<(S::AllPosition, StreamKey, PersistedEnvelope), Self::Error>>
                + Send,
        >,
    >;
    async fn append(
        &self,
        id: &StreamKey,
        expected: Option<Version>,
        envelopes: PendingBatch<'_>,
    ) -> Result<Self::AllPosition, AppendError<Self::Error>> {
        let fault = std::mem::take(&mut *self.fault.lock());
        match fault {
            Fault::Rollback => Err(AppendError::Store(FaultError::Lost)),
            Fault::Normal => self
                .inner
                .append(id, expected, envelopes)
                .await
                .map_err(append_error),
            Fault::CompetingCommit => {
                self.inner
                    .append(id, expected, envelopes)
                    .await
                    .map_err(append_error)?;
                self.inner
                    .append(id, expected, envelopes)
                    .await
                    .map_err(append_error)
            }
            Fault::CommitLost => {
                self.inner
                    .append(id, expected, envelopes)
                    .await
                    .map_err(append_error)?;
                Err(AppendError::Store(FaultError::Lost))
            }
            Fault::Delayed(gate) => {
                let backend = Arc::clone(&self.inner);
                let stream = id.clone();
                let batch = envelopes.iter().cloned().collect::<Vec<_>>();
                // Server work outlives the dropped client response future.
                let (send, receive) = tokio::sync::oneshot::channel();
                tokio::spawn(async move {
                    gate.wait().await;
                    let result = backend
                        .append(&stream, expected, PendingBatch::new(&batch).unwrap())
                        .await
                        .map_err(append_error);
                    let _ = send.send(result);
                    gate.done.add_permits(1);
                });
                receive.await.unwrap()
            }
        }
    }
    async fn read_stream(
        &self,
        id: &StreamKey,
        from: Version,
    ) -> Result<Self::Stream, Self::Error> {
        let gate = self.read_gate.lock().take();
        if let Some(control) = gate {
            control.wait().await;
        }
        let after = self.after_read.lock().take();
        let stream = self
            .inner
            .read_stream(id, from)
            .await
            .map_err(FaultError::Store)?
            .map_err(FaultError::Store);
        if let Some((limit, write)) = after {
            let snapshot: Vec<_> = stream.take(limit.saturating_add(1)).try_collect().await?;
            assert!(
                snapshot.len() <= limit,
                "snapshot exceeded its controlled bound"
            );
            write.release();
            write.done().await;
            return Ok(Box::pin(futures::stream::iter(
                snapshot.into_iter().map(Ok),
            )));
        }
        Ok(Box::pin(stream))
    }
    async fn read_all(
        &self,
        from: Option<Self::AllPosition>,
    ) -> Result<Self::AllStream, Self::Error> {
        Ok(Box::pin(
            self.inner
                .read_all(from)
                .await
                .map_err(FaultError::Store)?
                .map_err(FaultError::Store),
        ))
    }
}
