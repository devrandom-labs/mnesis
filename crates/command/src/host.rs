use std::convert::Infallible;
use std::fmt::Debug;
use std::future::Future;
use std::num::{NonZeroU64, NonZeroUsize};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use mnesis::{Aggregate, EventOf, Handle, Version};
use mnesis_store::{RawEventStore, Store, StreamKey};
use parking_lot::{Mutex, MutexGuard};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::{JoinError, JoinHandle};
use tokio_util::sync::PollSender;
use tower_service::Service;

use crate::engine::{Engine, Handoff};
use crate::trace::{self, Trace};
use crate::{Error, Receipt, Request};

type Reply<T, A, I, E> = oneshot::Sender<Result<T, Error<<A as Aggregate>::Error, I, E>>>;
type Started<S, A, C, I, const N: usize> = Result<
    (
        CommandProcessor<A, C, I, N, <S as RawEventStore>::Error>,
        Worker<C, I>,
    ),
    Error<<A as Aggregate>::Error, I, <S as RawEventStore>::Error>,
>;

/// Explicit resource limits; no product release capacity is implied.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Number of waiting calls; one additional call may be in flight.
    pub queue: NonZeroUsize,
    /// Nonzero deadline for each store operation, excluding queue waiting.
    /// Requires Tokio time; it cannot preempt application work that does not yield.
    pub operation: Duration,
}

/// Observed shutdown preserves the exact unresolved write for the next host.
#[derive(Debug)]
pub struct Shutdown<C, I = NonZeroU64> {
    stream: StreamKey,
    uncertain: Option<Request<C, I>>,
    cause: Option<JoinError>,
}
impl<C, I> Shutdown<C, I> {
    /// Original unresolved request, including its CAS base. Absence does not
    /// mean application work has completed. The report cannot be rewritten
    /// before passing it to `CommandProcessor::resume`.
    pub const fn uncertain(&self) -> Option<&Request<C, I>> {
        self.uncertain.as_ref()
    }

    /// Remaining original task cancellation or panic. None means normal
    /// completion or that the cause has already been taken.
    pub const fn cause(&self) -> Option<&JoinError> {
        self.cause.as_ref()
    }

    /// Take the original task error, including its panic payload, while retaining
    /// the unresolved request and stream for resumption.
    pub const fn take_cause(&mut self) -> Option<JoinError> {
        self.cause.take()
    }
}

/// Owned task completion, retaining both its cause and unresolved original.
///
/// Await this after stopping or aborting the processor. Dropping it detaches the
/// task, as with Tokio's join handle, and loses the final report. Dropping a
/// temporary mutable borrow while waiting preserves the owned worker.
#[must_use = "await worker completion to retain unresolved requests and failures"]
pub struct Worker<C, I = NonZeroU64> {
    task: JoinHandle<()>,
    stream: StreamKey,
    handoff: Handoff<C, I>,
}
impl<C, I> Worker<C, I> {
    /// Request task cancellation. Await this worker to observe completion.
    pub fn abort(&self) {
        self.task.abort();
    }
}
impl<C, I> Future for Worker<C, I> {
    type Output = Shutdown<C, I>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let worker = self.get_mut();
        Pin::new(&mut worker.task).poll(cx).map(|result| Shutdown {
            stream: worker.stream.clone(),
            uncertain: worker.handoff.lock().take(),
            cause: result.err(),
        })
    }
}

struct Call<A: Aggregate, C, I, E> {
    message: Message<A, C, I, E>,
    trace: Trace,
    #[cfg(feature = "tracing")]
    queue: Option<Trace>,
}
enum Message<A: Aggregate, C, I, E> {
    Retry {
        id: I,
        reply: Reply<Option<Receipt<I>>, A, I, E>,
    },
    Submit {
        request: Request<C, I>,
        reply: Reply<Receipt<I>, A, I, E>,
    },
    Read {
        reply: Reply<(Option<Version>, A::State), A, I, E>,
    },
    Original {
        id: I,
        reply: Reply<Option<Request<C, I>>, A, I, E>,
    },
}
impl<A: Aggregate, C, I, E> Message<A, C, I, E> {
    const fn operation(&self) -> &'static str {
        match self {
            Self::Retry { .. } => "retry",
            Self::Submit { .. } => "submit",
            Self::Read { .. } => "read",
            Self::Original { .. } => "original",
        }
    }
    fn reject(self) {
        match self {
            Self::Retry { reply, .. } => {
                let _ = reply.send(Err(Error::Closed));
            }
            Self::Submit { reply, .. } => {
                let _ = reply.send(Err(Error::Closed));
            }
            Self::Read { reply } => {
                let _ = reply.send(Err(Error::Closed));
            }
            Self::Original { reply, .. } => {
                let _ = reply.send(Err(Error::Closed));
            }
        }
    }
}
impl<A: Aggregate, C, I, E> Call<A, C, I, E> {
    fn new(message: Message<A, C, I, E>) -> Self {
        Self {
            trace: Trace::request(message.operation()),
            message,
            #[cfg(feature = "tracing")]
            queue: None,
        }
    }
    const fn finish(&mut self, outcome: &'static str) {
        self.trace.finish(outcome);
        #[cfg(feature = "tracing")]
        if let Some(queue) = &mut self.queue {
            queue.finish(outcome);
        }
    }
    fn reject(mut self) {
        self.finish("closed");
        self.message.reject();
    }
}
fn respond<T, D, I, E>(
    trace: &mut Trace,
    reply: oneshot::Sender<Result<T, Error<D, I, E>>>,
    result: Result<T, Error<D, I, E>>,
) {
    trace.finish(trace::outcome(&result));
    if reply.send(result).is_err() {
        trace::reply_dropped();
    }
}
impl<A, C, I, E> Call<A, C, I, E>
where
    A: Aggregate + 'static,
    A::State: Clone,
    C: Clone + Debug + Eq + Serialize + DeserializeOwned + Send + Sync + 'static,
    I: Clone + Debug + Ord + Serialize + DeserializeOwned + Send + Sync + 'static,
    EventOf<A>: Clone + Eq + Serialize + DeserializeOwned,
    E: std::error::Error + Send + Sync + 'static,
{
    async fn execute<S, const N: usize>(mut self, engine: &mut Engine<S, A, C, I, N>)
    where
        S: RawEventStore<Error = E> + 'static,
        A: Handle<C, N>,
    {
        self.trace
            .scope()
            .run(async move {
                #[cfg(feature = "tracing")]
                if let Some(mut queue) = self.queue.take() {
                    queue.finish("ok");
                }
                match self.message {
                    Message::Retry { id, reply } => {
                        respond(&mut self.trace, reply, engine.retry(&id).await);
                    }
                    Message::Submit { request, reply } => {
                        respond(&mut self.trace, reply, engine.submit(request).await);
                    }
                    Message::Read { reply } => respond(&mut self.trace, reply, engine.read().await),
                    Message::Original { id, reply } => {
                        respond(&mut self.trace, reply, engine.original(&id).await);
                    }
                }
            })
            .await;
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Lifecycle {
    Running,
    Stopping,
}

struct Admission(Mutex<watch::Sender<Lifecycle>>);
impl Admission {
    fn lock(&self) -> MutexGuard<'_, watch::Sender<Lifecycle>> {
        self.0.lock()
    }
}
struct Inbox<A: Aggregate, C, I, E> {
    receiver: mpsc::Receiver<Call<A, C, I, E>>,
    admission: Arc<Admission>,
    trace: Trace,
}
impl<A: Aggregate, C, I, E> Drop for Inbox<A, C, I, E> {
    fn drop(&mut self) {
        self.trace.scope().sync(|| {
            let mut cleanup = Trace::stage("shutdown");
            self.admission.lock().send_replace(Lifecycle::Stopping);
            self.receiver.close();
            // Ready permits may outlive the worker. Never wait for them to be used.
            while let Ok(call) = self.receiver.try_recv() {
                call.reject();
            }
            cleanup.finish("ok");
        });
    }
}

/// Bounded generic command admission over the aggregate's existing Mnesis rules.
///
/// Application code must check current authorization before every call that
/// can disclose state or receipts. This handle executes no external effects.
/// Keep and await the task returned by `start` or `resume`.
pub struct CommandProcessor<A: Aggregate, C, I = NonZeroU64, const N: usize = 0, E = Infallible> {
    id: A::Id,
    sender: mpsc::Sender<Call<A, C, I, E>>,
    service: PollSender<Call<A, C, I, E>>,
    admission: Arc<Admission>,
}
impl<A: Aggregate, C, I, const N: usize, E> Clone for CommandProcessor<A, C, I, N, E> {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            sender: self.sender.clone(),
            service: self.service.clone(),
            admission: Arc::clone(&self.admission),
        }
    }
}
impl<A, C, I, const N: usize> CommandProcessor<A, C, I, N>
where
    A: Handle<C, N> + 'static,
    A::State: Clone,
    C: Clone + Debug + Eq + Serialize + DeserializeOwned + Send + Sync + 'static,
    I: Clone + Debug + Ord + Serialize + DeserializeOwned + Send + Sync + 'static,
    EventOf<A>: Clone + Eq + Serialize + DeserializeOwned,
{
    /// Start one aggregate host without executing application effects.
    /// The application's Tokio runtime must enable time and outlive the worker.
    /// Startup does not probe its timer driver, stored history or database health.
    ///
    /// # Errors
    /// Rejects invalid limits or startup outside a Tokio runtime.
    pub fn start<S: RawEventStore + 'static>(
        store: &Store<S>,
        id: A::Id,
        limits: Limits,
    ) -> Started<S, A, C, I, N> {
        CommandProcessor::<A, C, I, N, S::Error>::launch(store, id, limits, None)
    }

    /// Resume after observed shutdown, preserving an unresolved request's base.
    ///
    /// # Errors
    /// Rejects another stream, invalid limits, unavailable runtime and unresolved
    /// requests that cannot be encoded. Failure leaves the report available.
    pub fn resume<S: RawEventStore + 'static>(
        store: &Store<S>,
        id: A::Id,
        limits: Limits,
        shutdown: &Shutdown<C, I>,
    ) -> Started<S, A, C, I, N> {
        if shutdown.stream.as_bytes() != id.as_ref() {
            return Err(Error::Scope);
        }
        if let Some(request) = &shutdown.uncertain {
            request.validate().map_err(Error::Request)?;
        }
        CommandProcessor::<A, C, I, N, S::Error>::launch(
            store,
            id,
            limits,
            shutdown.uncertain.clone(),
        )
    }
}

impl<A, C, I, const N: usize, E> CommandProcessor<A, C, I, N, E>
where
    A: Handle<C, N> + 'static,
    A::State: Clone,
    C: Clone + Debug + Eq + Serialize + DeserializeOwned + Send + Sync + 'static,
    I: Clone + Debug + Ord + Serialize + DeserializeOwned + Send + Sync + 'static,
    EventOf<A>: Clone + Eq + Serialize + DeserializeOwned,
    E: std::error::Error + Send + Sync + 'static,
{
    fn launch<S: RawEventStore<Error = E> + 'static>(
        store: &Store<S>,
        id: A::Id,
        limits: Limits,
        pending: Option<Request<C, I>>,
    ) -> Started<S, A, C, I, N> {
        if limits.operation.is_zero() {
            return Err(Error::DeadlineZero);
        }
        if limits.queue.get() > tokio::sync::Semaphore::MAX_PERMITS {
            return Err(Error::QueueLimit {
                requested: limits.queue.get(),
                maximum: tokio::sync::Semaphore::MAX_PERMITS,
            });
        }
        let runtime = tokio::runtime::Handle::try_current().map_err(Error::Runtime)?;
        let (sender, receiver) = mpsc::channel(limits.queue.get());
        let (stop, stopping) = watch::channel(Lifecycle::Running);
        let admission = Arc::new(Admission(Mutex::new(stop)));
        let worker = Inbox {
            receiver,
            admission: Arc::clone(&admission),
            trace: Trace::worker(),
        };
        let handoff = Arc::new(Mutex::new(None));
        let engine = Engine::new(
            store,
            id.clone(),
            limits.operation,
            pending,
            Arc::clone(&handoff),
        );
        let stream = StreamKey::from_slice(id.as_ref());
        let scope = worker.trace.scope();
        let task = Worker {
            task: runtime.spawn(scope.run(run(engine, worker, stopping))),
            stream,
            handoff,
        };
        let service = PollSender::new(sender.clone());
        Ok((
            Self {
                id,
                sender,
                service,
                admission,
            },
            task,
        ))
    }

    /// Aggregate identity bound at startup; application effects use this scope.
    pub const fn id(&self) -> &A::Id {
        &self.id
    }

    /// Physical accepted-record limit. Aggregate fact limits may be lower.
    #[must_use]
    pub const fn capacity() -> usize {
        crate::model::MAX_ACCEPTED
    }

    /// Retry the locally retained original or return an accepted original's receipt.
    /// None is no known request, not evidence that another handler rolled back.
    /// Application authorization must precede this call.
    ///
    /// # Errors
    /// Preserves unknown outcomes and rejects unavailable/closed admission.
    pub async fn retry(&self, id: I) -> Result<Option<Receipt<I>>, Error<A::Error, I, E>> {
        let (reply, response) = oneshot::channel();
        self.send(Message::Retry {
            id: id.clone(),
            reply,
        })?;
        response.await.map_err(|_| Error::Uncertain {
            command: id,
            source: None,
        })?
    }

    /// Submit the exact original request. Once admitted it survives cancellation
    /// of its waiting caller. Overload rejects before admission.
    /// An unknown request may retry only its original conditional append;
    /// unchanged reads do not establish rollback.
    ///
    /// # Errors
    /// Preserves mismatch, conflict, domain rejection and uncertain writes.
    pub async fn submit(
        &self,
        request: Request<C, I>,
    ) -> Result<Receipt<I>, Error<A::Error, I, E>> {
        request.validate().map_err(|error| {
            trace::invalid_request();
            Error::Request(error)
        })?;
        let id = request.id.clone();
        let (reply, response) = oneshot::channel();
        self.send(Message::Submit { request, reply })?;
        response.await.map_err(|_| Error::Uncertain {
            command: id,
            source: None,
        })?
    }

    /// Read reconstructed domain state, without invoking its command handler.
    ///
    /// # Errors
    /// Rejects unavailable/corrupt reads and preserves unresolved-write barriers.
    pub async fn read(&self) -> Result<(Option<Version>, A::State), Error<A::Error, I, E>> {
        let (reply, response) = oneshot::channel();
        self.send(Message::Read { reply })?;
        response.await.map_err(|_| Error::Closed)?
    }

    /// Recover the exact persisted original for a retry identity.
    ///
    /// # Errors
    /// An absent original cannot resolve an uncertain write. Known originals
    /// remain readable while an unrelated write is unsettled.
    pub async fn original(&self, id: I) -> Result<Option<Request<C, I>>, Error<A::Error, I, E>> {
        let (reply, response) = oneshot::channel();
        self.send(Message::Original { id, reply })?;
        response.await.map_err(|_| Error::Closed)?
    }

    /// Stop admission; await the observed task to retain unresolved work.
    pub fn stop(&self) {
        self.admission.lock().send_replace(Lifecycle::Stopping);
    }

    fn send(&self, message: Message<A, C, I, E>) -> Result<(), Error<A::Error, I, E>> {
        let mut call = Call::new(message);
        #[cfg(feature = "tracing")]
        {
            call.queue = Some(call.trace.child("queue"));
        }
        let mut stage = call.trace.child("admission");
        let admission = self.admission.lock();
        if *admission.borrow() == Lifecycle::Stopping {
            drop(admission);
            call.finish("closed");
            stage.finish("closed");
            return Err(Error::Closed);
        }
        let sent = self.sender.try_send(call);
        drop(admission);
        let result = sent.map_err(|error| match error {
            mpsc::error::TrySendError::Full(mut queued) => {
                queued.finish("queue_full");
                Error::QueueFull
            }
            mpsc::error::TrySendError::Closed(mut stopped) => {
                stopped.finish("closed");
                Error::Closed
            }
        });
        stage.finish(trace::outcome(&result));
        result
    }
}

impl<A, C, I, const N: usize, E> Service<Request<C, I>> for CommandProcessor<A, C, I, N, E>
where
    A: Handle<C, N> + 'static,
    A::State: Clone,
    C: Clone + Debug + Eq + Serialize + DeserializeOwned + Send + Sync + 'static,
    I: Clone + Debug + Ord + Serialize + DeserializeOwned + Send + Sync + 'static,
    EventOf<A>: Clone + Eq + Serialize + DeserializeOwned,
    E: std::error::Error + Send + Sync + 'static,
{
    type Response = Receipt<I>;
    type Error = Error<A::Error, I, E>;
    type Future = Pin<Box<dyn Future<Output = Result<Receipt<I>, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let lifecycle = *self.admission.lock().borrow();
        if lifecycle == Lifecycle::Stopping {
            self.service.close();
            self.service.abort_send();
            return Poll::Ready(Err(Error::Closed));
        }
        self.service.poll_reserve(cx).map_err(|_| Error::Closed)
    }

    fn call(&mut self, request: Request<C, I>) -> Self::Future {
        if let Err(error) = request.validate() {
            trace::invalid_request();
            self.service.abort_send();
            return Box::pin(async { Err(Error::Request(error)) });
        }
        let id = request.id.clone();
        let (reply, response) = oneshot::channel();
        let mut call = Call::new(Message::Submit { request, reply });
        #[cfg(feature = "tracing")]
        {
            call.queue = Some(call.trace.child("queue"));
        }
        let mut stage = call.trace.child("admission");
        let admission = self.admission.lock();
        if *admission.borrow() == Lifecycle::Stopping {
            drop(admission);
            call.finish("closed");
            stage.finish("closed");
            self.service.close();
            self.service.abort_send();
            return Box::pin(async { Err(Error::Closed) });
        }
        let sent = self.service.send_item(call);
        drop(admission);
        if let Err(error) = sent {
            if let Some(mut closed) = error.into_inner() {
                closed.finish("closed");
            }
            stage.finish("closed");
            return Box::pin(async { Err(Error::Closed) });
        }
        stage.finish("ok");
        Box::pin(async move {
            response.await.map_err(|_| Error::Uncertain {
                command: id,
                source: None,
            })?
        })
    }
}

async fn run<S, A, C, I, const N: usize>(
    mut engine: Engine<S, A, C, I, N>,
    mut worker: Inbox<A, C, I, S::Error>,
    mut stopping: watch::Receiver<Lifecycle>,
) where
    S: RawEventStore + 'static,
    A: Handle<C, N> + 'static,
    A::State: Clone,
    C: Clone + Debug + Eq + Serialize + DeserializeOwned + Send + Sync + 'static,
    I: Clone + Debug + Ord + Serialize + DeserializeOwned + Send + Sync + 'static,
    EventOf<A>: Clone + Eq + Serialize + DeserializeOwned,
{
    loop {
        if *stopping.borrow() == Lifecycle::Stopping {
            break;
        }
        let call = tokio::select! {
            biased;
            _ = stopping.changed() => break,
            message = worker.receiver.recv() => {
                let Some(call) = message else { break };
                call
            }
        };
        let lifecycle = *stopping.borrow();
        if lifecycle == Lifecycle::Stopping {
            call.reject();
            break;
        }
        call.execute(&mut engine).await;
    }
    worker.trace.finish("ok");
}
