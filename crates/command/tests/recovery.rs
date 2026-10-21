//! Generic conflict and uncertainty witnesses at observable store boundaries.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test failures retain fault diagnostics"
)]
mod faults;

use std::convert::Infallible;
use std::error::Error as _;
use std::num::{NonZeroU64, NonZeroUsize};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use futures::TryStreamExt;
use mnesis::{Aggregate, AggregateState, DomainEvent, Events, Handle, Message, Version, events};
use mnesis_command::{CommandProcessor, Error, Limits, Request};
use mnesis_inmemory::InMemoryStore;
use mnesis_store::{RawEventStore, Store, StreamKey};
use serde::{Deserialize, Serialize};
use tower::{Service, ServiceExt};

use faults::{Fault, FaultStore, Gate};

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
async fn a_lost_commit_response_preserves_the_source_and_original_acceptance() {
    type Stored = mnesis_store::StoreError<
        faults::FaultError<<InMemoryStore as RawEventStore>::Error>,
        serde_json::Error,
        serde_json::Error,
    >;
    let store = Store::new(FaultStore::new(InMemoryStore::new()));
    store.raw().arm(Fault::CommitLost);
    let (controller, task) = Host::start(&store, String::from("counter"), limits()).unwrap();
    let original = request(1, 0, 7);
    let observed = controller.submit(original.clone()).await;
    let failure = observed.unwrap_err();
    assert!(matches!(
        failure,
        Error::Uncertain {
            source: Some(_),
            ..
        }
    ));
    let retained = failure.source().unwrap().downcast_ref::<Stored>().unwrap();
    assert!(matches!(
        retained,
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
    assert!(matches!(overload, Err(Error::Capacity)));
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
    let store = Store::new(InMemoryStore::new());
    let (one, first_task) = Host::start(&store, String::from("counter"), limits()).unwrap();
    let (two, second_task) = Host::start(&store, String::from("counter"), limits()).unwrap();
    let (first, duplicate) =
        tokio::join!(one.submit(request(1, 0, 7)), two.submit(request(1, 0, 7)));
    assert_eq!(first.unwrap(), duplicate.unwrap());
    let (left, right) = tokio::join!(one.submit(request(2, 1, 9)), two.submit(request(3, 1, 13)));
    let expected = match (left, right) {
        (Ok(receipt), Err(Error::Conflict)) => {
            assert_eq!(receipt.id.get(), 2);
            16
        }
        (Err(Error::Conflict), Ok(receipt)) => {
            assert_eq!(receipt.id.get(), 3);
            20
        }
        other => {
            assert!(
                matches!(
                    other,
                    (Ok(_), Err(Error::Conflict)) | (Err(Error::Conflict), Ok(_))
                ),
                "unexpected CAS outcomes: {other:?}"
            );
            return;
        }
    };
    let current = one.read().await.unwrap();
    assert_eq!(current, (Version::new(2), Total(expected)));
    assert_eq!(records(store.raw()).await.len(), 2);
    one.stop();
    two.stop();
    assert_eq!(finish(first_task).await.uncertain(), None);
    assert_eq!(finish(second_task).await.uncertain(), None);
}

#[tokio::test]
async fn dropping_all_handles_stops_the_observed_host_without_an_explicit_stop() {
    let store = Store::new(InMemoryStore::new());
    let (controller, task) = Host::start(&store, String::from("counter"), limits()).unwrap();
    drop(controller);
    let shutdown = tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .expect("closed admission did not stop the host")
        .unwrap();
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
    assert!(matches!(zero_timeout, Err(Error::Capacity)));
    let excessive_queue = Host::start(
        &store,
        String::from("counter"),
        Limits {
            queue: NonZeroUsize::new(tokio::sync::Semaphore::MAX_PERMITS + 1).unwrap(),
            ..limits()
        },
    );
    assert!(matches!(excessive_queue, Err(Error::Capacity)));
}

#[tokio::test]
async fn the_largest_supported_queue_is_valid_and_can_stop() {
    let store = Store::new(InMemoryStore::new());
    let (controller, task) = Host::start(
        &store,
        String::from("counter"),
        Limits {
            queue: NonZeroUsize::new(tokio::sync::Semaphore::MAX_PERMITS).unwrap(),
            ..limits()
        },
    )
    .unwrap();
    controller.stop();
    assert_eq!(finish(task).await.uncertain(), None);
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
    let lost_host = task.await.unwrap_err();
    assert!(lost_host.is_cancelled());
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
    assert!(matches!(
        failed,
        Error::Uncertain {
            source: Some(_),
            ..
        }
    ));
    let retained = failed
        .source()
        .unwrap()
        .downcast_ref::<Error<std::io::Error>>()
        .unwrap();
    assert!(matches!(retained, Error::Rejected(_)));
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
        other.call(request(1, 0, 7)).await,
        Err(Error::Closed)
    ));
}

#[tokio::test]
async fn tower_abort_before_first_worker_poll_closes_reserved_admission() {
    let store = Store::new(InMemoryStore::default());
    let (mut processor, task) = Host::start(&store, "counter".into(), limits()).unwrap();
    processor.ready().await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(matches!(
        processor.call(request(1, 0, 7)).await,
        Err(Error::Closed)
    ));
}

#[tokio::test]
async fn tower_worker_terminal_cause_remains_observable_through_its_join() {
    let store = Store::new(InMemoryStore::default());
    let (mut processor, task) =
        CommandProcessor::<Panics, u8>::start(&store, "counter".into(), limits()).unwrap();
    let response = processor.ready().await.unwrap().call(request(1, 0, 7));
    assert!(
        matches!(response.await, Err(Error::Uncertain { command, .. }) if command == NonZeroU64::MIN)
    );
    let failure = task.await.unwrap_err();
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
