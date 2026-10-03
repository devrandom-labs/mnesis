//! Runtime-neutral, bounded admission for synchronous storage work.

use crate::FjallError;
use futures::channel::oneshot;
use futures::{Future, Stream};
use std::collections::VecDeque;
use std::num::NonZeroUsize;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::sync::mpsc;

/// Limits for the store's single blocking worker and lazy scan batches.
#[derive(Clone, Copy, Debug)]
pub struct BlockingConfig {
    /// Maximum admitted jobs waiting behind the running operation.
    pub queue_capacity: NonZeroUsize,
    /// Maximum live scan/session handles plus queued cleanups. Opening at this
    /// limit returns a typed capacity error; dropping a handle queues cleanup.
    pub max_open_scans: NonZeroUsize,
    /// Maximum rows fetched by one scan job.
    pub scan_batch_rows: NonZeroUsize,
    /// Stop fetching when accumulated serialized key/value bytes reach this
    /// budget. The final row may cross it; the row limit always applies.
    /// Engine block caches/backing allocations have separate engine limits.
    pub scan_batch_bytes: NonZeroUsize,
}

impl Default for BlockingConfig {
    #[allow(clippy::expect_used, reason = "fixed nonzero configuration constants")]
    fn default() -> Self {
        Self {
            queue_capacity: NonZeroUsize::new(32).expect("nonzero"),
            max_open_scans: NonZeroUsize::new(32).expect("nonzero"),
            scan_batch_rows: NonZeroUsize::new(64).expect("nonzero"),
            scan_batch_bytes: NonZeroUsize::new(1_048_576).expect("nonzero"),
        }
    }
}

type Job = Box<dyn FnOnce(&mut WorkerState) + Send>;

#[derive(Default)]
pub struct WorkerState {
    #[cfg(feature = "export")]
    pub views: crate::export::Views,
}

struct Control {
    sender: mpsc::Sender<Job>,
    cleanup: mpsc::Sender<Job>,
    finished: oneshot::Receiver<()>,
}

#[derive(Clone, Copy)]
enum Queue {
    Jobs,
    Cleanup,
}

#[derive(Clone, Copy)]
enum OpenQueues {
    Both(Queue),
    Jobs,
    Cleanup,
    Closed,
}

enum WorkerEvent {
    Job {
        work: Job,
        next: OpenQueues,
    },
    ChannelClosed(OpenQueues),
    #[cfg(feature = "export")]
    TimerTick,
}

fn received(work: Option<Job>, next: OpenQueues, closed: OpenQueues) -> WorkerEvent {
    work.map_or_else(
        || WorkerEvent::ChannelClosed(closed),
        |ready| WorkerEvent::Job { work: ready, next },
    )
}

#[cfg(feature = "export")]
struct Timer {
    runtime: Option<tokio::runtime::Runtime>,
}

#[cfg(feature = "export")]
impl Timer {
    fn new() -> Result<Self, FjallError> {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .map(|runtime| Self {
                runtime: Some(runtime),
            })
            .map_err(FjallError::WorkerTimer)
    }

    #[allow(
        clippy::expect_used,
        reason = "the private constructor installs the runtime; only exclusive Drop removes it"
    )]
    fn block_on<F: Future>(&self, future: F) -> F::Output {
        self.runtime
            .as_ref()
            .expect("live timer owns its runtime")
            .block_on(future)
    }
}

#[cfg(feature = "export")]
impl Drop for Timer {
    fn drop(&mut self) {
        // Spawn failure drops this guard on the calling async thread. The
        // private runtime drives only receives/timers and spawns no tasks.
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

async fn next_job(
    receiver: &mut mpsc::Receiver<Job>,
    discarded: &mut mpsc::Receiver<Job>,
    open: OpenQueues,
) -> WorkerEvent {
    match open {
        OpenQueues::Both(priority) => {
            let jobs = receiver.recv();
            let disposal = discarded.recv();
            futures::pin_mut!(jobs, disposal);
            match priority {
                Queue::Cleanup => match futures::future::select(disposal, jobs).await {
                    futures::future::Either::Left((value, _)) => {
                        received(value, OpenQueues::Both(Queue::Jobs), OpenQueues::Jobs)
                    }
                    futures::future::Either::Right((value, _)) => {
                        received(value, OpenQueues::Both(Queue::Cleanup), OpenQueues::Cleanup)
                    }
                },
                Queue::Jobs => match futures::future::select(jobs, disposal).await {
                    futures::future::Either::Left((value, _)) => {
                        received(value, OpenQueues::Both(Queue::Cleanup), OpenQueues::Cleanup)
                    }
                    futures::future::Either::Right((value, _)) => {
                        received(value, OpenQueues::Both(Queue::Jobs), OpenQueues::Jobs)
                    }
                },
            }
        }
        OpenQueues::Jobs => received(receiver.recv().await, OpenQueues::Jobs, OpenQueues::Closed),
        OpenQueues::Cleanup => received(
            discarded.recv().await,
            OpenQueues::Cleanup,
            OpenQueues::Closed,
        ),
        OpenQueues::Closed => WorkerEvent::ChannelClosed(OpenQueues::Closed),
    }
}

fn run_worker<G>(
    mut receiver: mpsc::Receiver<Job>,
    mut discarded: mpsc::Receiver<Job>,
    guard: G,
    #[cfg(feature = "export")] timer: Timer,
) {
    let mut state = WorkerState::default();
    let mut open = OpenQueues::Both(Queue::Cleanup);
    while !matches!(open, OpenQueues::Closed) {
        #[cfg(feature = "export")]
        state.views.expire();
        let receive = next_job(&mut receiver, &mut discarded, open);
        #[cfg(feature = "export")]
        let result = timer.block_on(async {
            let expiry = wait_deadline(state.views.deadline());
            futures::pin_mut!(receive, expiry);
            match futures::future::select(receive, expiry).await {
                futures::future::Either::Left((event, _)) => event,
                futures::future::Either::Right(_) => WorkerEvent::TimerTick,
            }
        });
        #[cfg(not(feature = "export"))]
        let result = futures::executor::block_on(receive);
        match result {
            WorkerEvent::Job { work, next } => {
                open = next;
                work(&mut state);
            }
            WorkerEvent::ChannelClosed(next) => open = next,
            #[cfg(feature = "export")]
            WorkerEvent::TimerTick => {}
        }
    }
    #[cfg(feature = "export")]
    drop(state);
    drop(discarded);
    drop(receiver);
    drop(guard);
    #[cfg(feature = "export")]
    drop(timer);
}

#[cfg(feature = "export")]
async fn wait_deadline(deadline: Option<std::time::Instant>) {
    match deadline {
        Some(at) => {
            let Some(remaining) = at.checked_duration_since(std::time::Instant::now()) else {
                return;
            };
            // Bound each timer registration, not the caller's lifetime: Tokio
            // rounds absolute deadlines upward and caps its internal tick range.
            tokio::time::sleep(remaining.min(std::time::Duration::from_secs(60))).await;
        }
        None => futures::future::pending().await,
    }
}

/// Last-handle drop closes admission without joining on an async thread.
/// The worker drains admitted jobs and releases its storage guard off-thread.
#[derive(Clone)]
pub struct Executor(Arc<Control>);

impl Executor {
    pub(crate) fn new<G: Send + 'static>(
        capacity: NonZeroUsize,
        cursor_capacity: NonZeroUsize,
        guard: G,
    ) -> Result<Self, FjallError> {
        let (sender, receiver) = mpsc::channel::<Job>(capacity.get());
        let (cleanup, discarded) = mpsc::channel::<Job>(cursor_capacity.get());
        let (done, finished) = oneshot::channel();
        #[cfg(feature = "export")]
        let timer = Timer::new()?;
        // This worker has a channel-based lifetime and explicit async shutdown;
        // a lexical thread scope cannot outlive the synchronous builder call.
        let _worker = std::thread::Builder::new()
            .name("mnesis-fjall-io".into())
            .spawn(move || {
                run_worker(
                    receiver,
                    discarded,
                    guard,
                    #[cfg(feature = "export")]
                    timer,
                );
                let _ = done.send(());
            })
            .map_err(FjallError::WorkerSpawn)?;
        Ok(Self(Arc::new(Control {
            sender,
            cleanup,
            finished,
        })))
    }

    pub(crate) async fn reserve(&self) -> Result<Slot<'_>, FjallError> {
        self.0
            .sender
            .reserve()
            .await
            .map(|permit| Slot { permit })
            .map_err(FjallError::WorkerQueueClosed)
    }

    /// Reserve cursor destruction capacity before opening an engine iterator.
    pub(crate) fn reserve_cursor(&self) -> Result<CursorSlot, FjallError> {
        self.0
            .cleanup
            .clone()
            .try_reserve_owned()
            .map(|permit| CursorSlot { permit })
            .map_err(|error| {
                FjallError::ScanCapacity(match error {
                    mpsc::error::TrySendError::Full(_) => mpsc::error::TrySendError::Full(()),
                    mpsc::error::TrySendError::Closed(_) => mpsc::error::TrySendError::Closed(()),
                })
            })
    }

    pub(crate) async fn shutdown(self) -> Result<(), FjallError> {
        let Control {
            sender,
            cleanup,
            finished,
        } = Arc::try_unwrap(self.0).map_err(|shared| FjallError::OutstandingHandles {
            count: Arc::strong_count(&shared),
        })?;
        drop(sender);
        drop(cleanup);
        finished.await.map_err(FjallError::WorkerResponseCanceled)
    }
}

pub struct Slot<'a> {
    permit: mpsc::Permit<'a, Job>,
}

impl Slot<'_> {
    pub(crate) fn submit<T: Send + 'static>(
        self,
        operation: impl FnOnce() -> T + Send + 'static,
    ) -> Completion<T> {
        self.submit_with(move |_| operation())
    }

    pub fn submit_with<T: Send + 'static>(
        self,
        operation: impl FnOnce(&mut WorkerState) -> T + Send + 'static,
    ) -> Completion<T> {
        let (sender, receiver) = oneshot::channel();
        self.permit.send(Box::new(move |state| {
            if sender.is_canceled() {
                return;
            }
            let result = catch_unwind(AssertUnwindSafe(|| operation(state))).map_err(|panic| {
                let message = panic
                    .downcast_ref::<&str>()
                    .copied()
                    .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
                    .unwrap_or("non-string panic payload");
                FjallError::WorkerPanicked {
                    message: crate::error::reason_label(&message),
                }
            });
            // If the caller disappeared after work started, the result and any
            // owned cursor are dropped here, on the worker, after bookkeeping.
            let _ = sender.send(result);
        }));
        Completion { receiver }
    }
}

pub struct Completion<T> {
    receiver: oneshot::Receiver<Result<T, FjallError>>,
}

impl<T> Future for Completion<T> {
    type Output = Result<T, FjallError>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.get_mut().receiver)
            .poll(cx)
            .map(|result| {
                result
                    .map_err(FjallError::WorkerResponseCanceled)
                    .and_then(core::convert::identity)
            })
    }
}

pub trait Cursor: Send + 'static {
    type Item: Send + 'static;
    fn next_row(&mut self) -> Option<(Result<Self::Item, FjallError>, usize)>;
}

/// One reserved cleanup slot follows a cursor through waiting, queued, running
/// and idle states. Drop never waits for queue space or performs engine cleanup.
pub struct ManagedCursor<C: Cursor> {
    owned: Option<(C, mpsc::OwnedPermit<Job>)>,
}

pub struct CursorSlot {
    permit: mpsc::OwnedPermit<Job>,
}

impl CursorSlot {
    pub(crate) fn attach<C: Cursor>(self, cursor: C) -> ManagedCursor<C> {
        ManagedCursor {
            owned: Some((cursor, self.permit)),
        }
    }

    #[cfg(feature = "export")]
    pub fn attach_cleanup(
        self,
        operation: impl FnOnce(&mut WorkerState) + Send + Sync + 'static,
    ) -> Cleanup {
        Cleanup {
            owned: Some((Box::new(operation), self.permit)),
        }
    }
}

#[cfg(feature = "export")]
type CleanupJob = Box<dyn FnOnce(&mut WorkerState) + Send + Sync>;

/// A reserved cleanup follows each session/cursor handle, including cancellation.
#[cfg(feature = "export")]
pub struct Cleanup {
    owned: Option<(CleanupJob, mpsc::OwnedPermit<Job>)>,
}

#[cfg(feature = "export")]
impl Cleanup {
    pub async fn finish(mut self) -> Result<(), FjallError> {
        let Some((job, permit)) = self.owned.take() else {
            return Ok(());
        };
        let (done, finished) = oneshot::channel();
        permit.send(Box::new(move |state| {
            job(state);
            let _ = done.send(());
        }));
        finished.await.map_err(FjallError::WorkerResponseCanceled)
    }
}

#[cfg(feature = "export")]
impl Drop for Cleanup {
    fn drop(&mut self) {
        if let Some((job, permit)) = self.owned.take() {
            permit.send(Box::new(move |state| job(state)));
        }
    }
}

impl<C: Cursor> ManagedCursor<C> {
    fn next_row(&mut self) -> Option<(Result<C::Item, FjallError>, usize)> {
        self.owned
            .as_mut()
            .and_then(|(cursor, _)| cursor.next_row())
    }
}

impl<C: Cursor> Drop for ManagedCursor<C> {
    fn drop(&mut self) {
        if let Some((cursor, permit)) = self.owned.take() {
            // Exactly one cleanup job per iterator, never one per row. The
            // owned reservation keeps admission open until this job is sent.
            permit.send(Box::new(move |_| drop(cursor)));
        }
    }
}

struct Batch<C: Cursor> {
    cursor: Option<ManagedCursor<C>>,
    rows: VecDeque<Result<C::Item, FjallError>>,
}

type BatchFuture<C> = Pin<Box<dyn Future<Output = Result<Batch<C>, FjallError>> + Send>>;

/// A lazy scan whose I/O and decoding execute in bounded worker batches.
/// Polling buffered rows performs no engine calls. EOF and errors are fused.
pub struct BlockingStream<C: Cursor> {
    cursor: Option<ManagedCursor<C>>,
    executor: Option<Executor>,
    config: BlockingConfig,
    rows: VecDeque<Result<C::Item, FjallError>>,
    pending: Option<BatchFuture<C>>,
}

impl<C: Cursor> Unpin for BlockingStream<C> {}

impl<C: Cursor> BlockingStream<C> {
    pub(crate) fn new(
        cursor: ManagedCursor<C>,
        executor: Executor,
        config: BlockingConfig,
    ) -> Self {
        Self {
            cursor: Some(cursor),
            executor: Some(executor),
            config,
            rows: VecDeque::new(),
            pending: None,
        }
    }
}

fn fetch<C: Cursor>(mut cursor: ManagedCursor<C>, config: BlockingConfig) -> Batch<C> {
    let mut rows = VecDeque::new();
    let mut bytes = 0usize;
    for _ in 0..config.scan_batch_rows.get() {
        let Some((row, size)) = cursor.next_row() else {
            return Batch { cursor: None, rows };
        };
        if row.is_err() {
            rows.push_back(row);
            return Batch { cursor: None, rows };
        }
        let Some(total) = bytes.checked_add(size) else {
            rows.push_back(Err(FjallError::ScanLengthOverflow));
            return Batch { cursor: None, rows };
        };
        rows.push_back(row);
        bytes = total;
        if bytes >= config.scan_batch_bytes.get() {
            break;
        }
    }
    Batch {
        cursor: Some(cursor),
        rows,
    }
}

impl<C: Cursor> Stream for BlockingStream<C> {
    type Item = Result<C::Item, FjallError>;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if let Some(row) = this.rows.pop_front() {
            return Poll::Ready(Some(row));
        }
        if this.pending.is_none() {
            let Some(cursor) = this.cursor.take() else {
                return Poll::Ready(None);
            };
            let Some(executor) = this.executor.clone() else {
                return Poll::Ready(None);
            };
            let config = this.config;
            this.pending = Some(Box::pin(async move {
                executor
                    .reserve()
                    .await?
                    .submit(move || fetch(cursor, config))
                    .await
            }));
        }
        let Some(pending) = this.pending.as_mut() else {
            return Poll::Ready(None);
        };
        let outcome = match pending.as_mut().poll(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(result) => result,
        };
        this.pending = None;
        match outcome {
            Ok(batch) => {
                this.cursor = batch.cursor;
                this.rows = batch.rows;
            }
            Err(error) => {
                this.cursor = None;
                this.rows.push_back(Err(error));
            }
        }
        if this.cursor.is_none() {
            this.executor = None;
        }
        Poll::Ready(this.rows.pop_front())
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions and injected operation panic"
)]
mod tests {
    use super::{BlockingConfig, BlockingStream, Cursor, Executor};
    use crate::FjallError;
    use futures::{FutureExt, StreamExt};
    use std::collections::VecDeque;
    use std::num::NonZeroUsize;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[cfg(feature = "export")]
    #[tokio::test]
    async fn unstarted_timer_can_be_dropped_inside_the_calling_runtime() {
        // A failed OS worker spawn would drop the captured timer here.
        let timer = super::Timer::new().unwrap();
        drop(timer);
    }

    fn nonzero(value: usize) -> NonZeroUsize {
        NonZeroUsize::new(value).unwrap()
    }

    // Dropping the release sender also unblocks the worker on assertion panic.
    async fn block_worker(
        executor: &Executor,
    ) -> (std::sync::mpsc::Sender<()>, super::Completion<()>) {
        let (release, wait) = std::sync::mpsc::channel();
        let (started, entered) = futures::channel::oneshot::channel();
        let completion = executor.reserve().await.unwrap().submit(move || {
            let _ = started.send(());
            let _ = wait.recv();
        });
        entered.await.unwrap();
        (release, completion)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn saturated_queue_and_cancellation_do_not_block_executor() {
        let executor = Executor::new(nonzero(1), nonzero(1), ()).unwrap();
        let (release, running) = block_worker(&executor).await;
        let calls = Arc::new(AtomicUsize::new(0));
        let queued_calls = Arc::clone(&calls);
        let queued = executor.reserve().await.unwrap().submit(move || {
            queued_calls.fetch_add(1, Ordering::SeqCst);
        });
        // A full queue suspends admission; it performs no storage work.
        let mut waiting = Box::pin(executor.reserve());
        assert!(waiting.as_mut().now_or_never().is_none());
        let unrelated = tokio::spawn(async { 123 });
        assert_eq!(unrelated.await.unwrap(), 123);
        drop(waiting);
        drop(queued);
        release.send(()).unwrap();
        running.await.unwrap();
        // FIFO fence: the canceled queued closure was visited before this job.
        executor
            .reserve()
            .await
            .unwrap()
            .submit(|| ())
            .await
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        executor.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn started_canceled_work_finishes_before_shutdown() {
        let executor = Executor::new(nonzero(1), nonzero(1), ()).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let operation_calls = Arc::clone(&calls);
        let (release, wait) = std::sync::mpsc::channel();
        let (started, entered) = futures::channel::oneshot::channel();
        let completion = executor.reserve().await.unwrap().submit(move || {
            let _ = started.send(());
            let _ = wait.recv();
            operation_calls.fetch_add(1, Ordering::SeqCst);
        });
        entered.await.unwrap();
        drop(completion);
        release.send(()).unwrap();
        executor.shutdown().await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn operation_panic_is_typed_and_worker_remains_available() {
        let executor = Executor::new(nonzero(1), nonzero(1), ()).unwrap();
        let failure = executor
            .reserve()
            .await
            .unwrap()
            .submit(|| panic!("injected"))
            .await;
        assert!(matches!(failure, Err(FjallError::WorkerPanicked { .. })));
        assert_eq!(
            executor
                .reserve()
                .await
                .unwrap()
                .submit(|| 7)
                .await
                .unwrap(),
            7
        );
        executor.shutdown().await.unwrap();
    }

    struct Rows {
        values: VecDeque<(Result<usize, FjallError>, usize)>,
        visited: Arc<AtomicUsize>,
    }

    impl Cursor for Rows {
        type Item = usize;
        fn next_row(&mut self) -> Option<(Result<usize, FjallError>, usize)> {
            self.visited.fetch_add(1, Ordering::SeqCst);
            self.values.pop_front()
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn scan_batches_are_lazy_bounded_ordered_and_fused() {
        for (row_limit, byte_limit, expected_first_batch) in [(2, 100, 2), (8, 5, 2)] {
            let executor = Executor::new(nonzero(1), nonzero(1), ()).unwrap();
            let visited = Arc::new(AtomicUsize::new(0));
            let cursor = Rows {
                values: (0..5).map(|value| (Ok(value), 3)).collect(),
                visited: Arc::clone(&visited),
            };
            let config = BlockingConfig {
                queue_capacity: nonzero(1),
                max_open_scans: nonzero(1),
                scan_batch_rows: nonzero(row_limit),
                scan_batch_bytes: nonzero(byte_limit),
            };
            let mut stream = BlockingStream::new(
                executor.reserve_cursor().unwrap().attach(cursor),
                executor.clone(),
                config,
            );
            assert_eq!(visited.load(Ordering::SeqCst), 0);
            assert_eq!(stream.next().await.unwrap().unwrap(), 0);
            assert_eq!(visited.load(Ordering::SeqCst), expected_first_batch);
            for expected in 1..5 {
                assert_eq!(stream.next().await.unwrap().unwrap(), expected);
            }
            assert!(stream.next().await.is_none());
            let at_eof = visited.load(Ordering::SeqCst);
            assert!(stream.next().await.is_none());
            assert_eq!(visited.load(Ordering::SeqCst), at_eof);
            // An exhausted stream must release its executor handle.
            executor.shutdown().await.unwrap();
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn scan_error_and_byte_overflow_stop_without_extra_rows() {
        for failure in [Err(FjallError::ScanLengthOverflow), Ok(1)] {
            let executor = Executor::new(nonzero(1), nonzero(1), ()).unwrap();
            let visited = Arc::new(AtomicUsize::new(0));
            let cursor = Rows {
                values: VecDeque::from([(Ok(0), 1), (failure, usize::MAX), (Ok(2), 1)]),
                visited: Arc::clone(&visited),
            };
            let config = BlockingConfig {
                scan_batch_rows: nonzero(2),
                scan_batch_bytes: nonzero(usize::MAX),
                ..BlockingConfig::default()
            };
            let mut stream = BlockingStream::new(
                executor.reserve_cursor().unwrap().attach(cursor),
                executor.clone(),
                config,
            );
            assert_eq!(stream.next().await.unwrap().unwrap(), 0);
            assert!(matches!(
                stream.next().await,
                Some(Err(FjallError::ScanLengthOverflow))
            ));
            assert!(stream.next().await.is_none());
            assert!(stream.next().await.is_none());
            assert_eq!(visited.load(Ordering::SeqCst), 2);
            executor.shutdown().await.unwrap();
        }
    }
    struct DropGuard(Option<futures::channel::oneshot::Sender<std::thread::ThreadId>>);

    impl Drop for DropGuard {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(std::thread::current().id());
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn last_handle_drop_drains_work_and_destroys_guard_off_executor() {
        let caller = std::thread::current().id();
        let (destroyed, observed) = futures::channel::oneshot::channel();
        let executor = Executor::new(nonzero(1), nonzero(1), DropGuard(Some(destroyed))).unwrap();
        let (release, running) = block_worker(&executor).await;
        drop(running);
        drop(executor);
        release.send(()).unwrap();
        assert_ne!(observed.await.unwrap(), caller);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn canceled_pending_scan_does_not_advance_cursor() {
        let executor = Executor::new(nonzero(1), nonzero(1), ()).unwrap();
        let (release, running) = block_worker(&executor).await;
        let visited = Arc::new(AtomicUsize::new(0));
        let cursor = Rows {
            values: VecDeque::from([(Ok(0), 1)]),
            visited: Arc::clone(&visited),
        };
        let mut stream = BlockingStream::new(
            executor.reserve_cursor().unwrap().attach(cursor),
            executor.clone(),
            BlockingConfig::default(),
        );
        assert!(stream.next().now_or_never().is_none());
        drop(stream);
        release.send(()).unwrap();
        running.await.unwrap();
        executor.shutdown().await.unwrap();
        assert_eq!(visited.load(Ordering::SeqCst), 0);
    }
    struct DropRows {
        destroyed: Option<futures::channel::oneshot::Sender<std::thread::ThreadId>>,
    }

    impl Cursor for DropRows {
        type Item = ();
        fn next_row(&mut self) -> Option<(Result<(), FjallError>, usize)> {
            Some((Ok(()), 1))
        }
    }

    impl Drop for DropRows {
        fn drop(&mut self) {
            if let Some(sender) = self.destroyed.take() {
                let _ = sender.send(std::thread::current().id());
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn idle_and_waiting_scan_cleanup_is_bounded_and_runs_on_worker() {
        for waiting_for_admission in [false, true] {
            let executor = Executor::new(nonzero(1), nonzero(1), ()).unwrap();
            let worker = executor
                .reserve()
                .await
                .unwrap()
                .submit(|| std::thread::current().id())
                .await
                .unwrap();
            let (release, running) = block_worker(&executor).await;
            let (destroyed, observed) = futures::channel::oneshot::channel();
            let cursor = executor.reserve_cursor().unwrap().attach(DropRows {
                destroyed: Some(destroyed),
            });
            let mut stream =
                BlockingStream::new(cursor, executor.clone(), BlockingConfig::default());
            assert!(matches!(
                executor.reserve_cursor(),
                Err(FjallError::ScanCapacity(_))
            ));
            let fence = executor.reserve().await.unwrap().submit(|| ());
            if waiting_for_admission {
                assert!(stream.next().now_or_never().is_none());
            }
            drop(stream);
            // Dropping is nonblocking, but its reserved cleanup capacity stays
            // occupied until the worker takes the cleanup job.
            assert!(matches!(
                executor.reserve_cursor(),
                Err(FjallError::ScanCapacity(_))
            ));
            assert_eq!(tokio::spawn(async { 9 }).await.unwrap(), 9);
            release.send(()).unwrap();
            assert_eq!(observed.await.unwrap(), worker);
            running.await.unwrap();
            fence.await.unwrap();
            drop(executor.reserve_cursor().unwrap());
            executor.shutdown().await.unwrap();
        }
    }
    struct WaitingRows {
        entered: Option<futures::channel::oneshot::Sender<()>>,
        release: std::sync::mpsc::Receiver<()>,
        destroyed: DropRows,
    }

    impl Cursor for WaitingRows {
        type Item = ();
        fn next_row(&mut self) -> Option<(Result<(), FjallError>, usize)> {
            if let Some(sender) = self.entered.take() {
                let _ = sender.send(());
            }
            let _ = self.release.recv();
            self.destroyed.next_row()
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn canceled_running_scan_destroys_cursor_on_worker() {
        let executor = Executor::new(nonzero(1), nonzero(1), ()).unwrap();
        let worker = executor
            .reserve()
            .await
            .unwrap()
            .submit(|| std::thread::current().id())
            .await
            .unwrap();
        let (started, entered) = futures::channel::oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let (destroyed, observed) = futures::channel::oneshot::channel();
        let cursor = executor.reserve_cursor().unwrap().attach(WaitingRows {
            entered: Some(started),
            release: wait,
            destroyed: DropRows {
                destroyed: Some(destroyed),
            },
        });
        let config = BlockingConfig {
            scan_batch_rows: nonzero(1),
            ..BlockingConfig::default()
        };
        let mut stream = BlockingStream::new(cursor, executor.clone(), config);
        assert!(stream.next().now_or_never().is_none());
        entered.await.unwrap();
        drop(stream);
        release.send(()).unwrap();
        assert_eq!(observed.await.unwrap(), worker);
        executor.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn canceling_shutdown_still_drains_and_releases_engine_guard() {
        let (destroyed, observed) = futures::channel::oneshot::channel();
        let executor = Executor::new(nonzero(1), nonzero(1), DropGuard(Some(destroyed))).unwrap();
        let (release, running) = block_worker(&executor).await;
        let mut shutdown = Box::pin(executor.shutdown());
        assert!(shutdown.as_mut().now_or_never().is_none());
        drop(shutdown);
        release.send(()).unwrap();
        running.await.unwrap();
        assert_ne!(observed.await.unwrap(), std::thread::current().id());
    }
}
