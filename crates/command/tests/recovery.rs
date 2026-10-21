//! Generic conflict and uncertainty witnesses at observable store boundaries.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test failures retain fault diagnostics"
)]
mod faults;

use std::convert::Infallible;
use std::io::{ErrorKind, Write};
use std::num::{NonZeroU64, NonZeroUsize};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll, Wake, Waker};
use std::time::{Duration, Instant};

use futures::{TryStreamExt, poll};
use mnesis::{
    Aggregate, AggregateState, DomainEvent, Events, Handle, KernelError, Message, Version, events,
};
use mnesis_command::{
    Cause, CodecError, CommandProcessor, Error, Limits, Request, Shutdown, Worker,
};
use mnesis_inmemory::InMemoryStore;
use mnesis_store::{PendingBatch, RawEventStore, Store, StoreError, StreamKey, pending_envelope};
use serde::{Deserialize, Serialize};
use tower::{Service, ServiceExt};

use faults::{Fault, FaultStore, Gate};

#[path = "support/races.rs"]
mod races;
#[path = "support/finish.rs"]
mod shutdown;
use shutdown::finish;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Fact(u8);
impl Message for Fact {}
impl DomainEvent for Fact {
    fn name(&self) -> &'static str {
        "number"
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Total(u64);
impl AggregateState for Total {
    type Event = Fact;
    fn initial() -> Self {
        Self(0)
    }
    fn apply(self, fact: &Fact) -> Self {
        Self(self.0 + u64::from(fact.0))
    }
}
struct Counter;
impl Aggregate for Counter {
    type State = Total;
    type Error = Infallible;
    type Id = String;
}
impl Handle<u8> for Counter {
    fn handle(_: &Total, command: u8) -> Result<Option<Events<Fact>>, Infallible> {
        Ok(Some(events![Fact(command)]))
    }
}
type Host = CommandProcessor<Counter, u8>;

struct Text;
static TEXT_DECISIONS: AtomicUsize = AtomicUsize::new(0);
impl Aggregate for Text {
    type State = Total;
    type Error = Infallible;
    type Id = String;
}
impl Handle<String> for Text {
    fn handle(_: &Total, _: String) -> Result<Option<Events<Fact>>, Infallible> {
        TEXT_DECISIONS.fetch_add(1, Ordering::SeqCst);
        Ok(Some(events![Fact(7)]))
    }
}

#[tokio::test]
async fn record_byte_failure_does_not_spend_a_valid_request_identity() {
    TEXT_DECISIONS.store(0, Ordering::SeqCst);
    let store = Store::new(InMemoryStore::new());
    let (processor, worker) =
        CommandProcessor::<Text, String>::start(&store, "counter".into(), limits()).unwrap();
    let empty = Request {
        id: NonZeroU64::MIN,
        expected: None,
        command: String::new(),
    };
    let overhead = empty.encode().unwrap().len();
    let input = Request {
        command: format!("{}{}", "é".repeat(1024), "x".repeat(4096 - overhead - 2048)),
        ..empty
    };
    assert_eq!(input.encode().unwrap().len(), 4096);
    let exceeded = processor.submit(input.clone()).await;
    assert!(
        matches!(exceeded, Err(Error::Failed(cause)) if matches!(&*cause,
        Cause::Store(StoreError::Encode(CodecError::Bytes { source: Some(error) }))
            if error.io_error_kind() == Some(ErrorKind::WriteZero)))
    );
    assert_eq!(records(store.raw()).await, Vec::<serde_json::Value>::new());
    assert_eq!(TEXT_DECISIONS.load(Ordering::SeqCst), 1);

    let valid = Request {
        command: "é🦀\0\n".into(),
        ..input
    };
    let receipt = processor.submit(valid.clone()).await.unwrap();
    assert_eq!(
        (receipt.id, receipt.version, receipt.events),
        (valid.id, Version::INITIAL, 1)
    );
    assert_eq!(processor.submit(valid.clone()).await.unwrap(), receipt);
    assert_eq!(
        processor.original(valid.id).await.unwrap(),
        Some(valid.clone())
    );
    assert_eq!(
        processor.read().await.unwrap(),
        (Some(Version::INITIAL), Total(7))
    );
    assert_eq!(TEXT_DECISIONS.load(Ordering::SeqCst), 2);
    let committed = vec![serde_json::json!({
        "schema":2,"request":valid,"receipt":receipt,"facts":[7]
    })];
    assert_eq!(records(store.raw()).await, committed);

    let oversized = Request {
        id: NonZeroU64::MAX,
        expected: Some(Version::INITIAL),
        command: "x".repeat(4097),
    };
    assert!(matches!(
        processor.submit(oversized).await,
        Err(Error::Request(CodecError::Bytes { source: Some(_) }))
    ));
    assert_eq!(TEXT_DECISIONS.load(Ordering::SeqCst), 2);
    assert_eq!(records(store.raw()).await, committed);
    processor.stop();
    assert_eq!(finish(worker).await.uncertain(), None);
}

struct Callback<F>(F);
impl<F: Fn() + Send + Sync> Wake for Callback<F> {
    fn wake(self: Arc<Self>) {
        (self.0)();
    }
}

#[test]
fn timer_disabled_runtime_retains_the_native_failure() {
    if std::env::var_os("MNESIS_TIMER_CHILD").is_some() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let store = Store::new(InMemoryStore::new());
        return runtime.block_on(async {
            let (processor, worker) = Host::start(&store, "counter".into(), limits()).unwrap();
            assert!(matches!(processor.submit(request(1, 0, 7)).await,
                Err(Error::Uncertain { command, source: None }) if command == NonZeroU64::MIN));
            let mut report = worker.await;
            assert_eq!(report.uncertain(), None);
            let cause = report.take_cause().unwrap();
            assert!(cause.is_panic());
            let payload = cause.into_panic();
            let message = payload.downcast_ref::<&str>().copied().or_else(|| {
                payload.downcast_ref::<String>().map(String::as_str)
            });
            assert_eq!(message, Some("A Tokio 1.x context was found, but timers are disabled. Call `enable_time` on the runtime builder to enable timers."));
            assert_eq!(records(store.raw()).await, Vec::<serde_json::Value>::new());
            assert!(matches!(processor.read().await, Err(Error::Closed)));
        });
    }
    observe_child(
        "timer_disabled_runtime_retains_the_native_failure",
        "MNESIS_TIMER_CHILD",
    );
}

#[test]
fn readiness_release_allows_a_reentrant_stop_callback() {
    if std::env::var_os("MNESIS_REENTRANT_STOP_CHILD").is_some() {
        return tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
            .block_on(async {
                assert_eq!(finish(reentrant_stop()).await.uncertain(), None);
            });
    }
    observe_child(
        "readiness_release_allows_a_reentrant_stop_callback",
        "MNESIS_REENTRANT_STOP_CHILD",
    );
}

fn observe_child(test: &str, flag: &str) {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test, "--nocapture"])
        .env(flag, "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break Some(status);
        }
        if started.elapsed() >= Duration::from_secs(2) {
            child.kill().unwrap();
            break None;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let output = child.wait_with_output().unwrap();
    assert!(
        status.is_some(),
        "child did not complete: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.status.success(),
        "child failed: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let summary = stdout
        .lines()
        .find_map(|line| line.strip_prefix("test result: ")?.split(';').next());
    assert_eq!(summary, Some("ok. 1 passed"));
}

fn reentrant_stop() -> Worker<u8> {
    let store = Store::new(InMemoryStore::new());
    let (mut processor, task) = Host::start(&store, "counter".into(), limits()).unwrap();
    let mut assigned = processor.clone();
    let mut waiting = processor.clone();
    let mut context = Context::from_waker(futures::task::noop_waker_ref());
    assert!(matches!(
        processor.poll_ready(&mut context),
        Poll::Ready(Ok(()))
    ));
    assert!(matches!(assigned.poll_ready(&mut context), Poll::Pending));
    let stopped = processor.clone();
    let notifications = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&notifications);
    let waker = Waker::from(Arc::new(Callback(move || {
        observed.fetch_add(1, Ordering::SeqCst);
        std::io::stdout()
            .write_all(b"entered stop callback\n")
            .unwrap();
        stopped.stop();
    })));
    assert!(matches!(
        waiting.poll_ready(&mut Context::from_waker(&waker)),
        Poll::Pending
    ));
    drop(processor);
    assigned.stop();
    std::io::stdout()
        .write_all(b"releasing reserved capacity\n")
        .unwrap();
    assert!(matches!(
        assigned.poll_ready(&mut context),
        Poll::Ready(Err(Error::Closed))
    ));
    assert_eq!(notifications.load(Ordering::SeqCst), 1);
    task
}

#[tokio::test]
async fn stop_during_dequeue_rejects_the_queued_command_without_running_it() {
    let store = Store::new(FaultStore::new(InMemoryStore::new()));
    let gate = Gate::new();
    store.raw().arm(Fault::Delayed(Arc::clone(&gate)));
    let mut profile = limits();
    profile.operation = Duration::from_secs(2);
    let (mut processor, task) = Host::start(&store, "counter".into(), profile).unwrap();
    let first = processor.ready().await.unwrap().call(request(1, 0, 1));
    gate.entered().await;
    let second = processor.ready().await.unwrap().call(request(2, 1, 2));
    let mut waiting = processor.clone();
    let stopped = processor.clone();
    let notifications = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&notifications);
    let waker = Waker::from(Arc::new(Callback(move || {
        observed.fetch_add(1, Ordering::SeqCst);
        stopped.stop();
    })));
    assert!(matches!(
        waiting.poll_ready(&mut Context::from_waker(&waker)),
        Poll::Pending
    ));
    gate.release();
    let accepted = tokio::time::timeout(Duration::from_secs(2), first)
        .await
        .unwrap()
        .unwrap();
    let queued = tokio::time::timeout(Duration::from_secs(2), second)
        .await
        .unwrap();
    gate.done().await;
    processor.stop();
    assert_eq!(finish(task).await.uncertain(), None);
    assert_eq!(accepted.version, Version::INITIAL);
    assert_eq!(notifications.load(Ordering::SeqCst), 1);
    assert!(
        matches!(queued, Err(Error::Closed)),
        "queued request ran: {queued:?}"
    );
    let rows = records(&store.raw().inner).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["request"]["command"], serde_json::json!(1));
}

struct Panics;
impl Aggregate for Panics {
    type State = Total;
    type Error = Infallible;
    type Id = String;
}
impl Handle<u8> for Panics {
    #[allow(
        clippy::panic,
        reason = "exercise preservation of a worker's original panic through its join"
    )]
    fn handle(_: &Total, _: u8) -> Result<Option<Events<Fact>>, Infallible> {
        std::panic::panic_any(73_u32);
    }
}
const fn limits() -> Limits {
    Limits {
        queue: NonZeroUsize::MIN,
        operation: Duration::from_millis(100),
    }
}
const fn request(id: u64, previous: u64, command: u8) -> Request<u8> {
    Request {
        id: NonZeroU64::new(id).unwrap(),
        expected: Version::new(previous),
        command,
    }
}
async fn records(store: &InMemoryStore) -> Vec<serde_json::Value> {
    let rows = store
        .read_stream(&StreamKey::from_slice(b"counter"), Version::INITIAL)
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    rows.iter()
        .map(|row| serde_json::from_slice(row.payload()).unwrap())
        .collect()
}

#[tokio::test]
async fn a_confirmed_conflict_reloads_the_original_receipt() {
    let store = Store::new(FaultStore::new(InMemoryStore::new()));
    store.raw().arm(Fault::CompetingCommit);
    let (controller, task) = Host::start(&store, String::from("counter"), limits()).unwrap();
    let accepted = controller.submit(request(1, 0, 7)).await.unwrap();
    assert_eq!(
        (accepted.id, accepted.version, accepted.events),
        (NonZeroU64::MIN, Version::INITIAL, 1)
    );
    let raw = records(&store.raw().inner).await;
    assert_eq!(raw.len(), 1);
    assert_eq!(raw[0]["facts"], serde_json::json!([7]));
    let current = controller.read().await.unwrap();
    assert_eq!(current, (Some(Version::INITIAL), Total(7)));
    controller.stop();
    assert_eq!(finish(task).await.uncertain(), None);
}

#[tokio::test]
async fn an_aborted_worker_returns_the_exact_unresolved_request_for_resumption() {
    let store = Store::new(FaultStore::new(InMemoryStore::new()));
    let gate = Gate::new();
    store.raw().arm(Fault::Delayed(Arc::clone(&gate)));
    let (mut processor, mut task) = Host::start(&store, "counter".into(), limits()).unwrap();
    let original = request(1, 0, 7);
    let response = processor.ready().await.unwrap().call(original.clone());
    gate.entered().await;
    assert!(poll!(&mut task).is_pending());
    task.abort();
    let joined = tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap();
    let observed = tokio::time::timeout(Duration::from_secs(2), response).await;
    gate.release();
    gate.done().await;
    assert!(
        matches!(observed.unwrap(), Err(Error::Uncertain { command, .. }) if command == original.id)
    );
    let committed = records(&store.raw().inner).await;
    assert_eq!(committed.len(), 1);
    assert_eq!(
        committed[0]["request"],
        serde_json::to_value(&original).unwrap()
    );
    let mut report = joined;
    assert!(report.cause().unwrap().is_cancelled());
    assert!(report.take_cause().unwrap().is_cancelled());
    assert!(report.cause().is_none());
    assert_eq!(report.uncertain(), Some(&original));
    let (unstarted, resumption) =
        Host::resume(&store, "counter".into(), limits(), &report).unwrap();
    resumption.abort();
    let resumed_report = tokio::time::timeout(Duration::from_secs(2), resumption)
        .await
        .unwrap();
    assert!(resumed_report.cause().unwrap().is_cancelled());
    assert_eq!(resumed_report.uncertain(), Some(&original));
    assert!(matches!(unstarted.read().await, Err(Error::Closed)));
    let (resumed, worker) =
        Host::resume(&store, "counter".into(), limits(), &resumed_report).unwrap();
    let accepted = resumed.submit(original.clone()).await.unwrap();
    assert_eq!(
        (accepted.id, accepted.version, accepted.events),
        (original.id, Version::INITIAL, 1)
    );
    assert_eq!(records(&store.raw().inner).await, committed);
    resumed.stop();
    assert_eq!(finish(worker).await.uncertain(), None);
}

#[tokio::test]
#[allow(
    clippy::panic,
    reason = "report an unexpected structured error in this negative test"
)]
async fn a_lost_commit_response_preserves_the_source_and_original_acceptance() {
    type Stored = mnesis_store::StoreError<
        faults::FaultError<<InMemoryStore as RawEventStore>::Error>,
        CodecError,
        CodecError,
    >;
    let store = Store::new(FaultStore::new(InMemoryStore::new()));
    store.raw().arm(Fault::CommitLost);
    let (controller, task) = Host::start(&store, String::from("counter"), limits()).unwrap();
    let original = request(1, 0, 7);
    let observed = controller.submit(original.clone()).await;
    let failure = observed.unwrap_err();
    let Error::Uncertain {
        command,
        source: Some(source),
    } = failure
    else {
        panic!("lost commit response must preserve its typed cause");
    };
    assert_eq!(command, NonZeroU64::MIN);
    let Cause::Store(retained) = *source else {
        panic!("lost commit response must preserve the store failure");
    };
    let typed: Stored = retained;
    assert!(matches!(
        typed,
        mnesis_store::StoreError::Adapter(faults::FaultError::Lost)
    ));
    let raw = records(&store.raw().inner).await;
    assert_eq!(raw.len(), 1);
    assert_eq!(raw[0]["request"]["command"], serde_json::json!(7));
    let receipt = controller.submit(original).await.unwrap();
    assert_eq!((receipt.version, receipt.events), (Version::INITIAL, 1));
    assert_eq!(records(&store.raw().inner).await, raw);
    controller.stop();
    assert_eq!(finish(task).await.uncertain(), None);
}

#[tokio::test]
async fn empty_reads_and_shutdown_do_not_erase_a_write_that_can_still_commit() {
    let store = Store::new(FaultStore::new(InMemoryStore::new()));
    let gate = Gate::new();
    store.raw().arm(Fault::Delayed(Arc::clone(&gate)));
    let (controller, task) = Host::start(&store, String::from("counter"), limits()).unwrap();
    let original = request(1, 0, 7);
    let result = controller.submit(original.clone()).await;
    gate.entered().await;
    assert!(matches!(result, Err(Error::Uncertain { source: None, .. })));
    assert_eq!(
        records(&store.raw().inner).await,
        Vec::<serde_json::Value>::new()
    );
    let repeated = Gate::new();
    store.raw().arm(Fault::Delayed(Arc::clone(&repeated)));
    let retry = controller.submit(original.clone()).await;
    assert!(matches!(retry, Err(Error::Uncertain { .. })));
    repeated.entered().await;
    let current = controller.read().await;
    assert!(matches!(current, Err(Error::Uncertain { .. })));
    let absent = controller.original(NonZeroU64::MIN).await;
    assert!(matches!(absent, Err(Error::Uncertain { .. })));
    controller.stop();
    let shutdown = finish(task).await;
    assert_eq!(shutdown.uncertain(), Some(&original));
    let wrong = Host::resume(&store, String::from("other"), limits(), &shutdown);
    assert!(matches!(wrong, Err(Error::Scope)));
    assert_eq!(shutdown.uncertain(), Some(&original));
    let (resumed, observed) =
        Host::resume(&store, String::from("counter"), limits(), &shutdown).unwrap();
    let dependent = resumed.submit(request(2, 0, 9)).await;
    assert!(matches!(dependent, Err(Error::Uncertain { .. })));
    gate.release();
    gate.done().await;
    repeated.release();
    repeated.done().await;
    let accepted = resumed.submit(original).await.unwrap();
    assert_eq!(
        (accepted.id, accepted.version, accepted.events),
        (NonZeroU64::MIN, Version::INITIAL, 1)
    );
    let restored = resumed.read().await.unwrap();
    assert_eq!(restored, (Some(Version::INITIAL), Total(7)));
    assert_eq!(records(&store.raw().inner).await.len(), 1);
    resumed.stop();
    assert_eq!(finish(observed).await.uncertain(), None);
}

#[tokio::test]
async fn known_receipts_remain_readable_without_clearing_an_unrelated_uncertainty() {
    let store = Store::new(FaultStore::new(InMemoryStore::new()));
    let (controller, task) = Host::start(&store, String::from("counter"), limits()).unwrap();
    let original = request(1, 0, 7);
    let first = controller.submit(original.clone()).await.unwrap();
    let gate = Gate::new();
    store.raw().arm(Fault::Delayed(Arc::clone(&gate)));
    let pending = request(2, 1, 9);
    let uncertain = controller.submit(pending).await;
    assert!(matches!(uncertain, Err(Error::Uncertain { .. })));
    gate.entered().await;
    let duplicate = controller.submit(original.clone()).await.unwrap();
    let known = controller.original(NonZeroU64::MIN).await.unwrap();
    assert_eq!(duplicate, first);
    assert_eq!(known, Some(original));
    let dependent = controller.submit(request(3, 1, 4)).await;
    assert!(matches!(dependent, Err(Error::Uncertain { .. })));
    let blocked = controller.read().await;
    assert!(matches!(blocked, Err(Error::Uncertain { .. })));
    assert_eq!(records(&store.raw().inner).await.len(), 1);
    gate.release();
    gate.done().await;
    let restored = controller.read().await.unwrap();
    assert_eq!(restored, (Version::new(2), Total(16)));
    controller.stop();
    assert_eq!(finish(task).await.uncertain(), None);
}

#[tokio::test]
async fn an_advancing_competitor_proves_a_delayed_cas_cannot_later_win() {
    let store = Store::new(FaultStore::new(InMemoryStore::new()));
    let gate = Gate::new();
    store.raw().arm(Fault::Delayed(Arc::clone(&gate)));
    let (old, old_task) = Host::start(&store, String::from("counter"), limits()).unwrap();
    let original = request(1, 0, 7);
    let uncertain = old.submit(original.clone()).await;
    assert!(matches!(uncertain, Err(Error::Uncertain { .. })));
    gate.entered().await;
    let (new, new_task) = Host::start(&store, String::from("counter"), limits()).unwrap();
    let winner = new.submit(request(2, 0, 9)).await.unwrap();
    assert_eq!(winner.version, Version::INITIAL);
    let loser = old.submit(original).await;
    assert!(matches!(loser, Err(Error::Conflict)));
    gate.release();
    gate.done().await;
    let current = old.read().await.unwrap();
    assert_eq!(current, (Some(Version::INITIAL), Total(9)));
    let raw = records(&store.raw().inner).await;
    assert_eq!(raw.len(), 1);
    assert_eq!(raw[0]["request"]["id"], serde_json::json!(2));
    old.stop();
    new.stop();
    assert_eq!(finish(old_task).await.uncertain(), None);
    assert_eq!(finish(new_task).await.uncertain(), None);
}

#[tokio::test]
async fn overload_and_shutdown_reject_queued_calls_before_acceptance() {
    let store = Store::new(FaultStore::new(InMemoryStore::new()));
    let gate = Gate::new();
    *store.raw().read_gate.lock() = Some(Arc::clone(&gate));
    let (controller, task) = Host::start(
        &store,
        String::from("counter"),
        Limits {
            operation: Duration::from_secs(2),
            ..limits()
        },
    )
    .unwrap();
    let reader = controller.clone();
    let in_flight = tokio::spawn(async move { reader.read().await });
    gate.entered().await;
    let queued = controller.submit(request(1, 0, 7));
    let mut waiting = std::pin::pin!(queued);
    assert!(futures::poll!(waiting.as_mut()).is_pending());
    let overload = controller.submit(request(2, 0, 9)).await;
    assert!(matches!(overload, Err(Error::QueueFull)));
    controller.stop();
    gate.release();
    let completed = in_flight.await.unwrap().unwrap();
    assert_eq!(completed, (None, Total(0)));
    let rejected = waiting.await;
    assert!(matches!(rejected, Err(Error::Closed)));
    let shutdown = finish(task).await;
    assert_eq!(shutdown.uncertain(), None);
    assert_eq!(
        records(&store.raw().inner).await,
        Vec::<serde_json::Value>::new()
    );
}

#[tokio::test]
async fn caller_cancellation_does_not_cancel_an_admitted_write() {
    let store = Store::new(FaultStore::new(InMemoryStore::new()));
    let gate = Gate::new();
    store.raw().arm(Fault::Delayed(Arc::clone(&gate)));
    let (controller, task) = Host::start(
        &store,
        String::from("counter"),
        Limits {
            operation: Duration::from_secs(2),
            ..limits()
        },
    )
    .unwrap();
    let caller = controller.clone();
    let response = tokio::spawn(async move { caller.submit(request(1, 0, 7)).await });
    gate.entered().await;
    response.abort();
    let cancelled = response.await.unwrap_err();
    assert!(cancelled.is_cancelled());
    gate.release();
    gate.done().await;
    let receipt = controller.submit(request(1, 0, 7)).await.unwrap();
    assert_eq!((receipt.version, receipt.events), (Version::INITIAL, 1));
    assert_eq!(records(&store.raw().inner).await.len(), 1);
    controller.stop();
    assert_eq!(finish(task).await.uncertain(), None);
}

#[test]
fn startup_without_a_runtime_returns_a_typed_error() {
    let store = Store::new(InMemoryStore::new());
    let unavailable = Host::start(&store, String::from("counter"), limits());
    assert!(matches!(unavailable, Err(Error::Runtime(_))));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_hosts_preserve_one_original_receipt_and_one_cas_winner() {
    for kind in [
        races::Kind::Duplicate,
        races::Kind::Conflict,
        races::Kind::Mismatch,
    ] {
        for winner in [races::Winner::First, races::Winner::Second] {
            for id in [
                NonZeroU64::MIN,
                NonZeroU64::new(2).unwrap(),
                NonZeroU64::new(u64::MAX - 1).unwrap(),
                NonZeroU64::MAX,
            ] {
                for amount in [0, 1, u8::MAX - 1, u8::MAX] {
                    for prefix in [None, Some(7)] {
                        races::run(kind, winner, id, amount, prefix).await;
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn dropping_all_handles_stops_the_observed_host_without_an_explicit_stop() {
    let store = Store::new(InMemoryStore::new());
    let (controller, task) = Host::start(&store, String::from("counter"), limits()).unwrap();
    drop(controller);
    let shutdown = tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .expect("closed admission did not stop the host");
    assert!(shutdown.cause().is_none());
    assert_eq!(shutdown.uncertain(), None);
}

#[test]
fn invalid_limits_reject_before_creating_a_host() {
    let store = Store::new(InMemoryStore::new());
    let zero_timeout = Host::start(
        &store,
        String::from("counter"),
        Limits {
            operation: Duration::ZERO,
            ..limits()
        },
    );
    assert!(matches!(zero_timeout, Err(Error::DeadlineZero)));
    let excessive_queue = Host::start(
        &store,
        String::from("counter"),
        Limits {
            queue: NonZeroUsize::new(tokio::sync::Semaphore::MAX_PERMITS + 1).unwrap(),
            ..limits()
        },
    );
    assert!(
        matches!(excessive_queue, Err(Error::QueueLimit { requested,maximum }) if requested==tokio::sync::Semaphore::MAX_PERMITS+1 && maximum==tokio::sync::Semaphore::MAX_PERMITS)
    );
}

#[tokio::test]
async fn the_largest_supported_queue_is_valid_and_can_stop() {
    for capacity in [
        tokio::sync::Semaphore::MAX_PERMITS - 1,
        tokio::sync::Semaphore::MAX_PERMITS,
    ] {
        let store = Store::new(InMemoryStore::new());
        let (controller, task) = Host::start(
            &store,
            String::from("counter"),
            Limits {
                queue: NonZeroUsize::new(capacity).unwrap(),
                ..limits()
            },
        )
        .unwrap();
        controller.stop();
        assert_eq!(finish(task).await.uncertain(), None);
    }
}

#[tokio::test]
async fn deadline_extremes_preserve_immediately_ready_store_operations() {
    let minimum = Duration::from_nanos(1);
    let below = Duration::MAX.checked_sub(minimum).unwrap();
    for operation in [minimum, below, Duration::MAX] {
        let store = Store::new(InMemoryStore::new());
        let profile = Limits {
            operation,
            ..limits()
        };
        let (processor, worker) = Host::start(&store, "counter".into(), profile).unwrap();
        let receipt = processor.submit(request(1, 0, 7)).await.unwrap();
        let expected = (NonZeroU64::MIN, Version::INITIAL, 1);
        assert_eq!((receipt.id, receipt.version, receipt.events), expected);
        let current = processor.read().await.unwrap();
        assert_eq!(current, (Some(Version::INITIAL), Total(7)));
        assert_eq!(records(store.raw()).await.len(), 1);
        processor.stop();
        assert_eq!(finish(worker).await.uncertain(), None);
    }
}

#[tokio::test]
async fn a_read_timeout_does_not_clear_an_earlier_unknown_write() {
    let store = Store::new(FaultStore::new(InMemoryStore::new()));
    let server = Gate::new();
    store.raw().arm(Fault::Delayed(Arc::clone(&server)));
    let (controller, task) = Host::start(&store, String::from("counter"), limits()).unwrap();
    let original = request(1, 0, 7);
    let unknown = controller.submit(original.clone()).await;
    assert!(matches!(unknown, Err(Error::Uncertain { .. })));
    server.entered().await;
    let reader = Gate::new();
    *store.raw().read_gate.lock() = Some(Arc::clone(&reader));
    let unavailable = controller.read().await;
    reader.entered().await;
    assert!(matches!(unavailable, Err(Error::Timeout)));
    assert_eq!(
        records(&store.raw().inner).await,
        Vec::<serde_json::Value>::new()
    );
    controller.stop();
    let shutdown = finish(task).await;
    assert_eq!(shutdown.uncertain(), Some(&original));
    reader.release();
    server.release();
    server.done().await;
    let (resumed, observed) =
        Host::resume(&store, String::from("counter"), limits(), &shutdown).unwrap();
    let accepted = resumed.submit(original).await.unwrap();
    assert_eq!(accepted.version, Version::INITIAL);
    resumed.stop();
    assert_eq!(finish(observed).await.uncertain(), None);
}

#[tokio::test]
async fn abrupt_host_loss_keeps_the_caller_unknown_and_exact_retry_arbitrates_the_late_write() {
    let store = Store::new(FaultStore::new(InMemoryStore::new()));
    let server = Gate::new();
    store.raw().arm(Fault::Delayed(Arc::clone(&server)));
    let (controller, task) = Host::start(
        &store,
        String::from("counter"),
        Limits {
            operation: Duration::from_secs(2),
            ..limits()
        },
    )
    .unwrap();
    let original = request(1, 0, 7);
    let retained = original.clone();
    let waiting = tokio::spawn(async move { controller.submit(retained).await });
    server.entered().await;
    task.abort();
    let lost_report = task.await;
    assert!(lost_report.cause().unwrap().is_cancelled());
    assert_eq!(lost_report.uncertain(), Some(&original));
    drop(lost_report);
    let reply = waiting.await.unwrap();
    assert!(
        matches!(reply, Err(Error::Uncertain { command, source:None }) if command == NonZeroU64::MIN)
    );
    assert_eq!(
        records(&store.raw().inner).await,
        Vec::<serde_json::Value>::new()
    );
    let (replacement, observed) = Host::start(&store, String::from("counter"), limits()).unwrap();
    // Without an observed Shutdown, retain/retry the exact original before
    // dependent work; a fresh empty snapshot is not a rollback certificate.
    let accepted = replacement.submit(original).await.unwrap();
    assert_eq!(
        (accepted.id, accepted.version, accepted.events),
        (NonZeroU64::MIN, Version::INITIAL, 1)
    );
    server.release();
    server.done().await;
    let raw = records(&store.raw().inner).await;
    assert_eq!(raw.len(), 1);
    assert_eq!(raw[0]["request"]["command"], serde_json::json!(7));
    let current = replacement.read().await.unwrap();
    assert_eq!(current, (Some(Version::INITIAL), Total(7)));
    replacement.stop();
    assert_eq!(finish(observed).await.uncertain(), None);
}

#[tokio::test]
async fn an_exact_conditional_retry_can_win_without_waiting_for_a_delayed_original() {
    let store = Store::new(FaultStore::new(InMemoryStore::new()));
    let server = Gate::new();
    store.raw().arm(Fault::Delayed(Arc::clone(&server)));
    let (controller, task) = Host::start(&store, String::from("counter"), limits()).unwrap();
    let original = request(1, 0, 7);
    let uncertain = controller.submit(original.clone()).await;
    assert!(matches!(uncertain, Err(Error::Uncertain { .. })));
    server.entered().await;
    let retry = controller.submit(original).await;
    server.release();
    server.done().await;
    let current = controller.read().await.unwrap();
    controller.stop();
    let shutdown = finish(task).await;
    let accepted = retry.unwrap();
    assert_eq!(
        (accepted.id, accepted.version, accepted.events),
        (NonZeroU64::MIN, Version::INITIAL, 1)
    );
    assert_eq!(current, (Some(Version::INITIAL), Total(7)));
    assert_eq!(shutdown.uncertain(), None);
    let raw = records(&store.raw().inner).await;
    assert_eq!(raw.len(), 1);
    assert_eq!(
        raw[0]["request"],
        serde_json::json!({"id":1,"expected":null,"command":7})
    );
}

#[tokio::test]
async fn an_unclassified_rollback_can_be_resolved_by_the_same_host_and_identity() {
    let store = Store::new(FaultStore::new(InMemoryStore::new()));
    store.raw().arm(Fault::Rollback);
    let (controller, task) = Host::start(&store, String::from("counter"), limits()).unwrap();
    let original = request(1, 0, 7);
    let unknown = controller.submit(original.clone()).await;
    assert!(matches!(
        unknown,
        Err(Error::Uncertain {
            source: Some(_),
            ..
        })
    ));
    assert_eq!(
        records(&store.raw().inner).await,
        Vec::<serde_json::Value>::new()
    );
    let unrelated = controller.submit(request(2, 0, 9)).await;
    assert!(matches!(unrelated, Err(Error::Uncertain { .. })));
    let changed_payload = controller.submit(request(1, 0, 8)).await;
    assert!(matches!(changed_payload, Err(Error::Mismatch)));
    let changed_base = controller.submit(request(1, 1, 7)).await;
    assert!(matches!(changed_base, Err(Error::Mismatch)));
    assert_eq!(
        records(&store.raw().inner).await,
        Vec::<serde_json::Value>::new()
    );
    let accepted = controller.submit(original).await.unwrap();
    assert_eq!(
        (accepted.id, accepted.version, accepted.events),
        (NonZeroU64::MIN, Version::INITIAL, 1)
    );
    let raw = records(&store.raw().inner).await;
    assert_eq!(raw.len(), 1);
    assert_eq!(
        raw[0]["request"],
        serde_json::json!({"id":1,"expected":null,"command":7})
    );
    controller.stop();
    assert_eq!(finish(task).await.uncertain(), None);
}

static RETRY_DECISIONS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
struct Transient;
impl Aggregate for Transient {
    type State = Total;
    type Error = std::io::Error;
    type Id = String;
}
impl Handle<u8> for Transient {
    fn handle(_: &Total, command: u8) -> Result<Option<Events<Fact>>, std::io::Error> {
        if RETRY_DECISIONS.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 1 {
            return Err(std::io::Error::other("controlled retry decision failure"));
        }
        Ok(Some(events![Fact(command)]))
    }
}
#[tokio::test]
#[allow(
    clippy::panic,
    reason = "report an unexpected structured error in this negative test"
)]
async fn a_retry_rejection_cannot_erase_the_older_append_that_can_still_commit() {
    type TransientHost = CommandProcessor<Transient, u8>;
    let store = Store::new(FaultStore::new(InMemoryStore::new()));
    let server = Gate::new();
    store.raw().arm(Fault::Delayed(Arc::clone(&server)));
    let (controller, task) =
        TransientHost::start(&store, String::from("counter"), limits()).unwrap();
    let original = request(1, 0, 7);
    let first = controller.submit(original.clone()).await;
    assert!(matches!(first, Err(Error::Uncertain { .. })));
    server.entered().await;
    let failed = controller.submit(original.clone()).await.unwrap_err();
    let Error::Uncertain {
        command,
        source: Some(source),
    } = failed
    else {
        panic!("failed exact retry must preserve the original uncertain request");
    };
    assert_eq!(command, NonZeroU64::MIN);
    let Cause::Rejected(retained) = *source else {
        panic!("failed exact retry must preserve its typed decision cause");
    };
    assert_eq!(retained.kind(), std::io::ErrorKind::Other);
    assert_eq!(retained.to_string(), "controlled retry decision failure");
    let blocked = controller.read().await;
    assert!(matches!(blocked, Err(Error::Uncertain { .. })));
    controller.stop();
    let shutdown = finish(task).await;
    assert_eq!(shutdown.uncertain(), Some(&original));
    server.release();
    server.done().await;
    let (recovered, observed) =
        TransientHost::resume(&store, String::from("counter"), limits(), &shutdown).unwrap();
    let accepted = recovered.submit(original).await.unwrap();
    assert_eq!(accepted.version, Version::INITIAL);
    assert_eq!(records(&store.raw().inner).await.len(), 1);
    assert_eq!(RETRY_DECISIONS.load(std::sync::atomic::Ordering::SeqCst), 2);
    recovered.stop();
    assert_eq!(finish(observed).await.uncertain(), None);
}

#[tokio::test]
async fn recovery_retries_only_a_retained_original_and_returns_its_stable_receipt() {
    let store = Store::new(FaultStore::new(InMemoryStore::new()));
    let (controller, task) = Host::start(&store, String::from("counter"), limits()).unwrap();
    let absent = controller.retry(NonZeroU64::MIN).await.unwrap();
    assert_eq!(absent, None);
    store.raw().arm(Fault::Rollback);
    let original = request(1, 0, 7);
    let unknown = controller.submit(original.clone()).await;
    assert!(matches!(unknown, Err(Error::Uncertain { .. })));
    let unrelated = controller.retry(NonZeroU64::new(2).unwrap()).await;
    assert!(matches!(
        unrelated,
        Err(Error::Uncertain {
            command: NonZeroU64::MIN,
            ..
        })
    ));
    let receipt = controller.retry(NonZeroU64::MIN).await.unwrap().unwrap();
    assert_eq!(receipt.version, Version::INITIAL);
    let retried = controller.retry(NonZeroU64::MIN).await.unwrap();
    assert_eq!(retried, Some(receipt.clone()));
    let duplicate = controller.submit(original).await.unwrap();
    assert_eq!(duplicate, receipt);
    assert_eq!(
        controller.read().await.unwrap(),
        (Some(Version::INITIAL), Total(7))
    );
    assert_eq!(records(&store.raw().inner).await.len(), 1);
    controller.stop();
    assert_eq!(finish(task).await.uncertain(), None);
}

#[tokio::test]
async fn receipt_identity_is_scoped_to_the_actual_stream() {
    let store = Store::new(InMemoryStore::new());
    let (one, first_task) = Host::start(&store, String::from("one"), limits()).unwrap();
    let (two, second_task) = Host::start(&store, String::from("two"), limits()).unwrap();
    one.submit(request(1, 0, 7)).await.unwrap();
    let separate = two.original(NonZeroU64::MIN).await.unwrap();
    assert_eq!(separate, None);
    two.submit(request(1, 0, 9)).await.unwrap();
    assert_eq!(one.read().await.unwrap().1, Total(7));
    assert_eq!(two.read().await.unwrap().1, Total(9));
    one.stop();
    two.stop();
    let report = finish(first_task).await;
    finish(second_task).await;
    let wrong = Host::resume(&store, String::from("two"), limits(), &report);
    assert!(matches!(wrong, Err(Error::Scope)));
}

#[tokio::test]
#[ignore = "optimized workload measurement, run explicitly with --ignored --nocapture"]
#[allow(
    clippy::print_stdout,
    reason = "explicit measurement records raw elapsed times"
)]
async fn measured_reload_and_duplicate_workload() {
    let store = Store::new(InMemoryStore::new());
    let (controller, task) = Host::start(&store, String::from("counter"), limits()).unwrap();
    for id in 1..=128 {
        controller.submit(request(id, id - 1, 1)).await.unwrap();
    }
    let original = controller.submit(request(1, 0, 1)).await.unwrap();
    for sample in 0..5 {
        let start = std::time::Instant::now();
        for _ in 0..100 {
            let duplicate = controller.submit(request(1, 0, 1)).await.unwrap();
            assert_eq!(duplicate, original);
        }
        println!(
            "sample={sample} duplicates=100 records=128 elapsed_us={}",
            start.elapsed().as_micros()
        );
    }
    assert_eq!(controller.read().await.unwrap().1, Total(128));
    controller.stop();
    finish(task).await;
}

#[tokio::test]
async fn tower_reserved_readiness_cannot_submit_after_observed_stop() {
    let store = Store::new(InMemoryStore::default());
    let (mut processor, task) = Host::start(&store, "counter".into(), limits()).unwrap();
    processor.ready().await.unwrap();
    processor.stop();
    finish(task).await;
    let result = tokio::time::timeout(Duration::from_secs(2), processor.call(request(1, 0, 7)))
        .await
        .unwrap();
    assert!(matches!(result, Err(Error::Closed)));
    assert_eq!(records(store.raw()).await.len(), 0);
}

#[tokio::test]
async fn tower_clones_do_not_duplicate_readiness_or_block_shutdown() {
    let store = Store::new(InMemoryStore::default());
    let (mut processor, task) = Host::start(&store, "counter".into(), limits()).unwrap();
    processor.ready().await.unwrap();
    let mut other = processor.clone();
    let mut context = Context::from_waker(futures::task::noop_waker_ref());
    assert!(matches!(other.poll_ready(&mut context), Poll::Pending));
    drop(processor);
    other.ready().await.unwrap();
    other.stop();
    finish(task).await;
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(2), other.call(request(1, 0, 7)))
            .await
            .unwrap(),
        Err(Error::Closed)
    ));
}

#[tokio::test]
async fn tower_abort_before_first_worker_poll_closes_reserved_admission() {
    let store = Store::new(InMemoryStore::default());
    let (mut processor, task) = Host::start(&store, "counter".into(), limits()).unwrap();
    processor.ready().await.unwrap();
    task.abort();
    let report = task.await;
    assert!(report.cause().unwrap().is_cancelled());
    assert_eq!(report.uncertain(), None);
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(2), processor.call(request(1, 0, 7)))
            .await
            .unwrap(),
        Err(Error::Closed)
    ));
}

#[tokio::test]
async fn tower_worker_terminal_cause_remains_observable_through_its_join() {
    let store = Store::new(InMemoryStore::default());
    let (mut processor, task) =
        CommandProcessor::<Panics, u8>::start(&store, "counter".into(), limits()).unwrap();
    let response = processor.ready().await.unwrap().call(request(1, 0, 7));
    let result = tokio::time::timeout(Duration::from_secs(2), response)
        .await
        .unwrap();
    assert!(matches!(result, Err(Error::Uncertain { command, .. }) if command == NonZeroU64::MIN));
    let mut report = tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap();
    assert_eq!(report.uncertain(), Some(&request(1, 0, 7)));
    let failure = report.take_cause().unwrap();
    assert!(failure.is_panic());
    assert_eq!(*failure.into_panic().downcast::<u32>().unwrap(), 73);
    assert_eq!(records(store.raw()).await.len(), 0);
}

#[tokio::test]
async fn tower_unpolled_reply_cancellation_preserves_persisted_order_and_receipts() {
    let store = Store::new(FaultStore::new(InMemoryStore::default()));
    let gate = Gate::new();
    store.raw().arm(Fault::Delayed(Arc::clone(&gate)));
    let configuration = Limits {
        operation: Duration::from_secs(2),
        ..limits()
    };
    let (mut processor, task) = Host::start(&store, "counter".into(), configuration).unwrap();
    let first = processor.ready().await.unwrap().call(request(1, 0, 1));
    gate.entered().await;
    let second = processor.ready().await.unwrap().call(request(2, 1, 2));
    drop(first);
    drop(second);
    assert_eq!(records(&store.raw().inner).await.len(), 0);
    gate.release();
    let third = processor
        .ready()
        .await
        .unwrap()
        .call(request(3, 2, 3))
        .await
        .unwrap();
    gate.done().await;
    assert_eq!(third.version, Version::new(3).unwrap());
    assert_eq!(processor.read().await.unwrap(), (Version::new(3), Total(6)));
    let saved = records(&store.raw().inner).await;
    assert_eq!(saved.len(), 3);
    assert_eq!(
        saved
            .iter()
            .map(|row| row["request"]["id"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    let original = processor
        .ready()
        .await
        .unwrap()
        .call(request(1, 0, 1))
        .await
        .unwrap();
    assert_eq!(original.version, Version::INITIAL);
    assert_eq!(records(&store.raw().inner).await, saved);
    processor.stop();
    finish(task).await;
}

static FOLD_DECISIONS: AtomicUsize = AtomicUsize::new(0);
#[derive(Debug, PartialEq, Eq)]
struct FoldFailure(u8);
#[derive(Clone, Debug)]
struct BrokenState;
impl AggregateState for BrokenState {
    type Event = Fact;
    fn initial() -> Self {
        Self
    }
    #[allow(
        clippy::panic,
        reason = "exercise a real application fold panic after append and during reconstruction"
    )]
    fn apply(self, fact: &Fact) -> Self {
        std::panic::panic_any(FoldFailure(fact.0));
    }
}
struct Fragile;
impl Aggregate for Fragile {
    type State = BrokenState;
    type Error = Infallible;
    type Id = String;
}
impl Handle<u8> for Fragile {
    fn handle(_: &BrokenState, command: u8) -> Result<Option<Events<Fact>>, Infallible> {
        FOLD_DECISIONS.fetch_add(1, Ordering::SeqCst);
        Ok(Some(events![Fact(command)]))
    }
}
struct Recovered;
impl Aggregate for Recovered {
    type State = Total;
    type Error = Infallible;
    type Id = String;
}
impl Handle<u8> for Recovered {
    fn handle(_: &Total, command: u8) -> Result<Option<Events<Fact>>, Infallible> {
        FOLD_DECISIONS.fetch_add(1, Ordering::SeqCst);
        Ok(Some(events![Fact(command)]))
    }
}
async fn assert_fold_failure(task: Worker<u8>) -> Shutdown<u8> {
    let mut report = tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap();
    let failure = report.take_cause().unwrap();
    assert!(failure.is_panic());
    assert_eq!(
        *failure.into_panic().downcast::<FoldFailure>().unwrap(),
        FoldFailure(7)
    );
    report
}

#[tokio::test]
async fn committed_acceptance_survives_fold_panic_and_failed_reconstruction() {
    let store = Store::new(InMemoryStore::new());
    let original = request(1, 0, 7);
    let (broken, task) =
        CommandProcessor::<Fragile, u8>::start(&store, "counter".into(), limits()).unwrap();
    let response = broken.submit(original.clone()).await;
    assert!(
        matches!(response, Err(Error::Uncertain { command, source: None }) if command == NonZeroU64::MIN)
    );
    let report = assert_fold_failure(task).await;
    assert_eq!(report.uncertain(), Some(&original));
    assert_eq!(FOLD_DECISIONS.load(Ordering::SeqCst), 1);
    let committed = vec![serde_json::json!({
        "schema":2,
        "request":{"id":1,"expected":null,"command":7},
        "receipt":{"id":1,"version":1,"events":1},
        "facts":[7]
    })];
    assert_eq!(records(store.raw()).await, committed);

    let (rebuilding, observed) =
        CommandProcessor::<Fragile, u8>::resume(&store, "counter".into(), limits(), &report)
            .unwrap();
    assert!(matches!(rebuilding.read().await, Err(Error::Closed)));
    let reconstructed = assert_fold_failure(observed).await;
    assert_eq!(reconstructed.uncertain(), Some(&original));
    assert_eq!(records(store.raw()).await, committed);
    assert_eq!(FOLD_DECISIONS.load(Ordering::SeqCst), 1);

    let (healthy, completed) = CommandProcessor::<Recovered, u8>::resume(
        &store,
        "counter".into(),
        limits(),
        &reconstructed,
    )
    .unwrap();
    let receipt = healthy.submit(original.clone()).await.unwrap();
    assert_eq!(receipt.id, original.id);
    assert_eq!((receipt.version, receipt.events), (Version::INITIAL, 1));
    assert_eq!(
        healthy.read().await.unwrap(),
        (Some(Version::INITIAL), Total(7))
    );
    assert_eq!(FOLD_DECISIONS.load(Ordering::SeqCst), 1);
    assert_eq!(records(store.raw()).await, committed);
    healthy.stop();
    assert_eq!(finish(completed).await.uncertain(), None);
}

#[tokio::test]
async fn history_capacity_preserves_old_receipts_and_typed_kernel_failures() {
    let store = Store::new(InMemoryStore::new());
    let batch = (1..=4097_u64)
        .map(|number| {
            let record = serde_json::json!({
                "schema":2,
                "request":{"id":number,"expected":Version::new(number - 1).map(Version::as_u64),"command":0},
                "receipt":{"id":number,"version":number,"events":0},
                "facts":[]
            });
            pending_envelope(Version::new(number).unwrap())
                .event_type("controller.accepted.v2")
                .payload(serde_json::to_vec(&record).unwrap())
                .build()
                .unwrap()
        })
        .collect::<Vec<_>>();
    store
        .raw()
        .append(
            &StreamKey::from_slice(b"counter"),
            None,
            PendingBatch::new(&batch[..4095]).unwrap(),
        )
        .await
        .unwrap();
    let mut profile = limits();
    profile.operation = Duration::from_secs(2);
    let (processor, task) = Host::start(&store, "counter".into(), profile).unwrap();
    assert_eq!(
        processor.read().await.unwrap(),
        (Version::new(4095), Total(0))
    );
    let last = processor.submit(request(4096, 4095, 0)).await.unwrap();
    assert_eq!(
        (last.id.get(), last.version.as_u64(), last.events),
        (4096, 4096, 1)
    );
    let before = records(store.raw()).await;
    assert_eq!(before.len(), 4096);
    assert_eq!(
        processor.read().await.unwrap(),
        (Version::new(4096), Total(0))
    );
    let old = processor.submit(request(1, 0, 0)).await.unwrap();
    assert_eq!(
        (old.id, old.version, old.events),
        (NonZeroU64::MIN, Version::INITIAL, 0)
    );
    let exhausted = processor.submit(request(5000, 4096, 0)).await;
    assert!(matches!(exhausted, Err(Error::Failed(cause)) if matches!(*cause, Cause::HistoryFull)));
    assert_eq!(records(store.raw()).await, before);
    store
        .raw()
        .append(
            &StreamKey::from_slice(b"counter"),
            Version::new(4096),
            PendingBatch::new(&batch[4096..]).unwrap(),
        )
        .await
        .unwrap();
    let unsupported = records(store.raw()).await;
    assert_eq!(unsupported.len(), 4097);
    let failure = processor.read().await;
    assert!(
        matches!(failure, Err(Error::Failed(cause)) if matches!(*cause,
        Cause::Store(StoreError::Kernel(KernelError::RehydrationLimitExceeded { max:4096 }))))
    );
    assert_eq!(records(store.raw()).await, unsupported);
    processor.stop();
    assert_eq!(finish(task).await.uncertain(), None);
}
