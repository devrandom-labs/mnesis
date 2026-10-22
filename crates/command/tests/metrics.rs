#![cfg(feature = "metrics")]
#![allow(
    clippy::unwrap_used,
    reason = "metric contract failures retain exact observations"
)]

use std::collections::HashMap;
use std::io::ErrorKind;
use std::num::{NonZeroU64, NonZeroUsize};
use std::panic::panic_any;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use futures::TryStreamExt;
use metrics::atomics::AtomicU64;
use metrics::{
    Counter, Gauge, Histogram, HistogramFn, Key, KeyName, Label, Metadata, Recorder, SharedString,
    Unit,
};
use mnesis::{Events, Handle, Version};
use mnesis_command::{
    Acceptance, AcceptanceCodec, Cause, CodecError, CommandProcessor, Error, Limits, Receipt,
    Request,
};
use mnesis_inmemory::InMemoryStore;
use mnesis_store::{Decode, RawEventStore, Store, StreamKey};
use parking_lot::Mutex;
use tower::{Service, ServiceExt};

#[path = "support/counter.rs"]
mod counter;
#[path = "faults/mod.rs"]
#[allow(
    clippy::expect_used,
    reason = "shared controlled faults retain bounded diagnostics"
)]
mod faults;
#[path = "support/finish.rs"]
#[allow(
    clippy::expect_used,
    reason = "shared completion retains bounded diagnostics"
)]
mod shutdown;
use counter::{Counter as Aggregate, Increment, Operation, Rejection, Sum};
use faults::{Fault, FaultStore, Gate};

type Backend = FaultStore<InMemoryStore>;
type Host =
    CommandProcessor<Aggregate, Operation, NonZeroU64, 0, <Backend as RawEventStore>::Error>;
const WAIT: Duration = Duration::from_secs(2);

#[derive(Default)]
struct Samples(Mutex<Vec<f64>>);
impl HistogramFn for Samples {
    fn record(&self, value: f64) {
        self.0.lock().push(value);
    }
}
#[derive(Default)]
struct Records {
    counters: Mutex<HashMap<Key, Arc<AtomicU64>>>,
    gauges: Mutex<HashMap<Key, Arc<AtomicU64>>>,
    histograms: Mutex<HashMap<Key, Arc<Samples>>>,
}
#[derive(Clone, Default)]
struct Recording(Arc<Records>);
impl Recorder for Recording {
    fn describe_counter(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}
    fn describe_gauge(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}
    fn describe_histogram(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}
    fn register_counter(&self, key: &Key, _: &Metadata<'_>) -> Counter {
        Counter::from_arc(Arc::clone(
            self.0.counters.lock().entry(key.clone()).or_default(),
        ))
    }
    fn register_gauge(&self, key: &Key, _: &Metadata<'_>) -> Gauge {
        Gauge::from_arc(Arc::clone(
            self.0.gauges.lock().entry(key.clone()).or_default(),
        ))
    }
    fn register_histogram(&self, key: &Key, _: &Metadata<'_>) -> Histogram {
        Histogram::from_arc(Arc::clone(
            self.0.histograms.lock().entry(key.clone()).or_default(),
        ))
    }
}
impl Recording {
    fn gauge(&self, name: &'static str) -> Option<f64> {
        self.0
            .gauges
            .lock()
            .get(&Key::from_static_name(name))
            .map(|value| f64::from_bits(value.load(Ordering::Acquire)))
    }
    fn count(&self, name: &'static str) -> Option<u64> {
        self.0
            .counters
            .lock()
            .get(&Key::from_static_name(name))
            .map(|value| value.load(Ordering::Acquire))
    }
    fn labeled(
        &self,
        metric: &'static str,
        labels: &[(&'static str, &'static str)],
    ) -> Option<u64> {
        self.0
            .counters
            .lock()
            .get(&key(metric, labels))
            .map(|value| value.load(Ordering::Acquire))
    }
    fn samples(&self, metric: &'static str, labels: &[(&'static str, &'static str)]) -> Vec<f64> {
        self.histogram(metric, labels).unwrap()
    }
    fn histogram(
        &self,
        metric: &'static str,
        labels: &[(&'static str, &'static str)],
    ) -> Option<Vec<f64>> {
        self.0
            .histograms
            .lock()
            .get(&key(metric, labels))
            .map(|samples| samples.0.lock().clone())
    }
    fn calls(&self) -> [Option<f64>; 4] {
        [
            self.gauge("mnesis_command_queued"),
            self.gauge("mnesis_command_reserved"),
            self.gauge("mnesis_command_inflight"),
            self.gauge("mnesis_command_unresolved"),
        ]
    }
    fn keys(&self) -> Vec<(&'static str, Key)> {
        let mut keys = self
            .0
            .counters
            .lock()
            .keys()
            .cloned()
            .map(|metric| ("counter", metric))
            .collect::<Vec<_>>();
        keys.extend(
            self.0
                .gauges
                .lock()
                .keys()
                .cloned()
                .map(|metric| ("gauge", metric)),
        );
        keys.extend(
            self.0
                .histograms
                .lock()
                .keys()
                .cloned()
                .map(|metric| ("histogram", metric)),
        );
        keys
    }
    async fn idle(&self) -> [Option<f64>; 4] {
        tokio::time::timeout(WAIT, async {
            while self.gauge("mnesis_command_inflight") != Some(0.0) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        self.calls()
    }
}
fn key(metric: &'static str, labels: &[(&'static str, &'static str)]) -> Key {
    Key::from_parts(
        metric,
        labels
            .iter()
            .map(|&(name, value)| Label::new(name, value))
            .collect::<Vec<_>>(),
    )
}
const fn request(id: u64, expected: Option<Version>, amount: u8) -> Request<Operation> {
    Request {
        id: NonZeroU64::new(id).unwrap(),
        expected,
        command: Operation::Add(amount),
    }
}
async fn rows(
    store: &InMemoryStore,
    stream: &'static str,
) -> Vec<Acceptance<Operation, Increment>> {
    store
        .read_stream(&StreamKey::from_slice(stream.as_bytes()), Version::INITIAL)
        .await
        .unwrap()
        .map_ok(|row| AcceptanceCodec.decode(&row).unwrap())
        .try_collect()
        .await
        .unwrap()
}
fn startup(
    store: &Store<Backend>,
    stream: &'static str,
    operation: Duration,
) -> (Host, mnesis_command::Worker<Operation>) {
    CommandProcessor::<Aggregate, Operation>::start(
        store,
        stream.into(),
        Limits {
            queue: NonZeroUsize::new(2).unwrap(),
            operation,
        },
    )
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn metrics_follow_native_admission_and_observed_acceptances() {
    let recording = Recording::default();
    assert!(metrics::set_global_recorder(recording.clone()).is_ok());
    tokio::time::timeout(WAIT, admission(&recording))
        .await
        .unwrap();
    tokio::time::timeout(WAIT, reconciliation(&recording))
        .await
        .unwrap();
    tokio::time::timeout(WAIT, abort(&recording)).await.unwrap();
    tokio::time::timeout(WAIT, panic(&recording)).await.unwrap();
    tokio::time::timeout(WAIT, invalid(&recording))
        .await
        .unwrap();
    labels(&recording);
    tokio::time::timeout(WAIT, stopping(&recording))
        .await
        .unwrap();
    accounting(&recording);
    tokio::time::timeout(WAIT, overlapping(&recording))
        .await
        .unwrap();
    labels(&recording);
}
async fn admission(recording: &Recording) {
    let started = Instant::now();
    let store = Store::new(FaultStore::new(InMemoryStore::new()));
    let gate = Gate::new();
    store.raw().arm(Fault::Delayed(Arc::clone(&gate)));
    let (mut host, worker) = startup(&store, "admission", WAIT);
    let first = tokio::time::timeout(WAIT, host.ready())
        .await
        .unwrap()
        .unwrap()
        .call(request(1, None, 7));
    gate.entered().await;
    let mut reserved = host.clone();
    tokio::time::timeout(WAIT, reserved.ready())
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(WAIT, reserved.ready())
        .await
        .unwrap()
        .unwrap();
    drop(reserved.clone());
    let queued = tokio::time::timeout(WAIT, host.ready())
        .await
        .unwrap()
        .unwrap()
        .call(request(2, Some(Version::INITIAL), 9));
    drop(queued);
    assert!(matches!(
        host.submit(request(3, None, 1)).await,
        Err(Error::QueueFull)
    ));
    let busy = recording.calls();
    let holding = Instant::now();
    tokio::time::sleep(Duration::from_millis(20)).await;
    let held = holding.elapsed();
    gate.release();
    gate.done().await;
    let receipt = tokio::time::timeout(WAIT, first).await.unwrap().unwrap();
    assert_eq!((receipt.version, receipt.events), (Version::INITIAL, 1));
    tokio::time::timeout(WAIT, async {
        while rows(&store.raw().inner, "admission").await.len() < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        tokio::time::timeout(WAIT, host.read())
            .await
            .unwrap()
            .unwrap(),
        (Version::new(2), Sum(16))
    );
    host.stop();
    assert_eq!(shutdown::finish(worker).await.uncertain(), None);
    let stopped = recording.calls();
    drop(reserved);
    let released = recording.calls();
    admission_records(&store.raw().inner).await;
    admission_observations(recording, [busy, stopped, released]);
    timing(recording, held, started.elapsed());
}
async fn admission_records(store: &InMemoryStore) {
    let accepted = rows(store, "admission").await;
    assert_eq!(accepted.len(), 2);
    assert_eq!(accepted[0].request(), &request(1, None, 7));
    assert_eq!(
        accepted[1].request(),
        &request(2, Some(Version::INITIAL), 9)
    );
    assert_eq!(accepted[0].facts(), &[Increment(7)]);
    assert_eq!(accepted[1].facts(), &[Increment(9)]);
}
fn admission_observations(recording: &Recording, [busy, stopped, released]: [[Option<f64>; 4]; 3]) {
    assert_eq!(busy, [Some(1.0), Some(1.0), Some(1.0), Some(1.0)]);
    assert_eq!(stopped, [Some(0.0), Some(1.0), Some(0.0), Some(0.0)]);
    assert_eq!(released, [Some(0.0); 4]);
    assert_eq!(recording.count("mnesis_command_acceptances_total"), Some(2));
    assert_eq!(
        recording.labeled("mnesis_command_submitted_total", &[("operation", "submit")]),
        Some(3)
    );
    assert_eq!(
        recording.labeled("mnesis_command_admitted_total", &[("operation", "submit")]),
        Some(2)
    );
    assert_eq!(
        recording.labeled(
            "mnesis_command_rejected_total",
            &[("operation", "submit"), ("outcome", "queue_full")]
        ),
        Some(1)
    );
}
async fn reconciliation(recording: &Recording) {
    let store = Store::new(FaultStore::new(InMemoryStore::new()));
    let gate = Gate::new();
    store.raw().arm(Fault::Delayed(Arc::clone(&gate)));
    let (host, worker) = startup(&store, "reconcile", Duration::from_millis(20));
    let original = request(1, None, 3);
    assert!(
        matches!(host.submit(original.clone()).await, Err(Error::Uncertain { command, source: None }) if command == NonZeroU64::MIN)
    );
    assert!(
        matches!(host.read().await, Err(Error::Uncertain { command, source: None }) if command == NonZeroU64::MIN)
    );
    let before = recording.count("mnesis_command_acceptances_total");
    let unresolved = recording.idle().await;
    gate.release();
    gate.done().await;
    assert_eq!(host.read().await.unwrap(), (Some(Version::INITIAL), Sum(3)));
    let observed = recording.count("mnesis_command_acceptances_total");
    let receipt = host.submit(original.clone()).await.unwrap();
    assert_eq!((receipt.version, receipt.events), (Version::INITIAL, 1));
    host.stop();
    assert_eq!(shutdown::finish(worker).await.uncertain(), None);
    let accepted = rows(&store.raw().inner, "reconcile").await;
    assert_eq!(accepted.len(), 1);
    assert_eq!(accepted[0].request(), &original);
    assert_eq!(accepted[0].receipt(), &receipt);
    assert_eq!(accepted[0].facts(), &[Increment(3)]);
    assert_eq!(before, Some(2));
    assert_eq!(observed, Some(3));
    assert_eq!(recording.count("mnesis_command_acceptances_total"), Some(3));
    assert_eq!(recording.count("mnesis_command_duplicates_total"), Some(1));
    assert_eq!(unresolved, [Some(0.0), Some(0.0), Some(0.0), Some(1.0)]);
    assert_eq!(recording.calls(), [Some(0.0); 4]);
    assert_eq!(
        recording.labeled(
            "mnesis_command_reconciliations_total",
            &[("outcome", "uncertain")]
        ),
        Some(1)
    );
    assert_eq!(
        recording.labeled("mnesis_command_reconciliations_total", &[("outcome", "ok")]),
        Some(1)
    );
}

async fn abort(recording: &Recording) {
    let store = Store::new(FaultStore::new(InMemoryStore::new()));
    let gate = Gate::new();
    *store.raw().read_gate.lock() = Some(Arc::clone(&gate));
    let (mut host, worker) = startup(&store, "abort", WAIT);
    let active = tokio::time::timeout(WAIT, host.ready())
        .await
        .unwrap()
        .unwrap()
        .call(request(1, None, 7));
    gate.entered().await;
    let queued = tokio::time::timeout(WAIT, host.ready())
        .await
        .unwrap()
        .unwrap()
        .call(request(2, None, 9));
    worker.abort();
    let report = tokio::time::timeout(WAIT, worker).await.unwrap();
    assert!(report.cause().unwrap().is_cancelled());
    assert_eq!(report.uncertain(), None);
    assert!(
        matches!(tokio::time::timeout(WAIT, active).await.unwrap(), Err(Error::Uncertain { command, source: None }) if command == NonZeroU64::MIN)
    );
    assert!(matches!(
        tokio::time::timeout(WAIT, queued).await.unwrap(),
        Err(Error::Closed)
    ));
    assert_eq!(rows(&store.raw().inner, "abort").await.len(), 0);
    assert_eq!(recording.calls(), [Some(0.0); 4]);
    assert_eq!(
        recording.labeled(
            "mnesis_command_worker_failures_total",
            &[("cause", "cancelled")]
        ),
        Some(1)
    );
}
impl Handle<u8> for Aggregate {
    #[allow(
        clippy::panic,
        reason = "prove metrics retain native worker failure and original request"
    )]
    fn handle(_: &Sum, _: u8) -> Result<Option<Events<Increment>>, Rejection> {
        panic_any(73_u32);
    }
}
async fn panic(recording: &Recording) {
    let store = Store::new(InMemoryStore::new());
    let (host, worker) = CommandProcessor::<Aggregate, u8>::start(
        &store,
        "panic".into(),
        Limits {
            queue: NonZeroUsize::MIN,
            operation: WAIT,
        },
    )
    .unwrap();
    let input = Request {
        id: NonZeroU64::MIN,
        expected: None,
        command: 7,
    };
    assert!(
        matches!(host.submit(input.clone()).await, Err(Error::Uncertain { command, source: None }) if command == input.id)
    );
    let mut report = tokio::time::timeout(WAIT, worker).await.unwrap();
    assert_eq!(report.uncertain(), Some(&input));
    assert_eq!(
        *report
            .take_cause()
            .unwrap()
            .into_panic()
            .downcast::<u32>()
            .unwrap(),
        73
    );
    assert_eq!(rows(store.raw(), "panic").await.len(), 0);
    assert_eq!(recording.calls(), [Some(0.0); 4]);
    assert_eq!(
        recording.labeled(
            "mnesis_command_worker_failures_total",
            &[("cause", "panic")]
        ),
        Some(1)
    );
}
async fn invalid(recording: &Recording) {
    let store = Store::new(InMemoryStore::new());
    let (mut host, worker) = CommandProcessor::<Aggregate, Operation, String>::start(
        &store,
        "invalid".into(),
        Limits {
            queue: NonZeroUsize::MIN,
            operation: WAIT,
        },
    )
    .unwrap();
    let oversized = Request {
        id: "x".repeat(4097),
        expected: None,
        command: Operation::Add(3),
    };
    let direct = host.submit(oversized.clone()).await;
    tokio::time::timeout(WAIT, host.ready())
        .await
        .unwrap()
        .unwrap();
    let service = host.call(oversized).await;
    for outcome in [direct, service] {
        assert!(
            matches!(outcome, Err(Error::Request(CodecError::Bytes { source: Some(ref error) })) if error.io_error_kind() == Some(ErrorKind::WriteZero))
        );
    }
    let valid = Request {
        id: "stable".to_owned(),
        expected: None,
        command: Operation::Add(3),
    };
    let receipt = host.submit(valid.clone()).await.unwrap();
    assert_eq!((receipt.version, receipt.events), (Version::INITIAL, 1));
    assert!(matches!(
        host.submit(Request {
            command: Operation::Add(4),
            ..valid.clone()
        })
        .await,
        Err(Error::Mismatch)
    ));
    assert!(matches!(
        host.submit(Request {
            id: "stale".to_owned(),
            ..valid.clone()
        })
        .await,
        Err(Error::Conflict)
    ));
    assert!(matches!(
        host.submit(Request {
            id: "rejected".to_owned(),
            expected: Some(Version::INITIAL),
            command: Operation::Reject
        })
        .await,
        Err(Error::Failed(ref cause)) if matches!(&**cause, Cause::Rejected(Rejection))
    ));
    let no_op = Request {
        id: "noop".to_owned(),
        expected: Some(Version::INITIAL),
        command: Operation::Ignore,
    };
    let confirmation = host.submit(no_op.clone()).await.unwrap();
    assert_eq!(
        (confirmation.version, confirmation.events),
        (Version::new(2).unwrap(), 0)
    );
    assert_eq!(host.read().await.unwrap(), (Version::new(2), Sum(3)));
    host.stop();
    assert_eq!(shutdown::finish(worker).await.uncertain(), None);
    invalid_records(store.raw(), &valid, &receipt, &no_op, &confirmation).await;
    assert_eq!(recording.calls(), [Some(0.0); 4]);
}
async fn invalid_records(
    store: &InMemoryStore,
    valid: &Request<Operation, String>,
    receipt: &Receipt<String>,
    no_op: &Request<Operation, String>,
    confirmation: &Receipt<String>,
) {
    let accepted: Vec<Acceptance<Operation, Increment, String>> = store
        .read_stream(&StreamKey::from_slice(b"invalid"), Version::INITIAL)
        .await
        .unwrap()
        .map_ok(|row| AcceptanceCodec.decode(&row).unwrap())
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    assert_eq!(accepted.len(), 2);
    assert_eq!(accepted[0].request(), valid);
    assert_eq!(accepted[0].receipt(), receipt);
    assert_eq!(accepted[0].facts(), &[Increment(3)]);
    assert_eq!(accepted[1].request(), no_op);
    assert_eq!(accepted[1].receipt(), confirmation);
    assert_eq!(accepted[1].facts(), &[]);
}
fn timing(recording: &Recording, held: Duration, elapsed: Duration) {
    for (samples, count) in [
        recording.samples("mnesis_command_queue_seconds", &[]),
        recording.samples(
            "mnesis_command_execution_seconds",
            &[("operation", "submit")],
        ),
        recording.samples(
            "mnesis_command_stage_seconds",
            &[("stage", "append"), ("outcome", "ok")],
        ),
    ]
    .into_iter()
    .zip([3, 2, 2])
    {
        assert_eq!(samples.len(), count);
        assert!(samples.iter().all(|sample| sample.is_finite()
            && *sample >= 0.0
            && *sample <= elapsed.as_secs_f64()));
        assert!(samples.iter().any(|sample| *sample >= held.as_secs_f64()));
    }
}
fn accounting(recording: &Recording) {
    assert_eq!(recording.count("mnesis_command_acceptances_total"), Some(6));
    assert_eq!(recording.count("mnesis_command_conflicts_total"), Some(1));
    assert_eq!(
        recording.labeled(
            "mnesis_command_rejected_total",
            &[("operation", "submit"), ("outcome", "request")]
        ),
        Some(2)
    );
    for (operation, submitted, admitted, unknown) in [("submit", 17, 14, 3), ("read", 4, 4, 1)] {
        for (name, expected) in [
            ("mnesis_command_submitted_total", submitted),
            ("mnesis_command_admitted_total", admitted),
            ("mnesis_command_uncertain_total", unknown),
        ] {
            assert_eq!(
                recording.labeled(name, &[("operation", operation)]),
                Some(expected)
            );
        }
    }
    assert_eq!(
        recording.samples("mnesis_command_queue_seconds", &[]).len(),
        18
    );
    for (operation, count) in [("submit", 12), ("read", 4)] {
        assert_eq!(
            recording
                .samples(
                    "mnesis_command_execution_seconds",
                    &[("operation", operation)]
                )
                .len(),
            count
        );
    }
    for (reason, count) in [
        ("queue_full", 1),
        ("closed", 2),
        ("mismatch", 1),
        ("conflict", 1),
        ("failed", 1),
    ] {
        assert_eq!(
            recording.labeled(
                "mnesis_command_rejected_total",
                &[("operation", "submit"), ("outcome", reason)]
            ),
            Some(count)
        );
    }
}
fn labels(recording: &Recording) {
    for (kind, metric) in recording.keys() {
        let expected: Option<&[&str]> = match (kind, metric.name()) {
            (
                "counter",
                "mnesis_command_submitted_total"
                | "mnesis_command_admitted_total"
                | "mnesis_command_uncertain_total",
            )
            | ("histogram", "mnesis_command_execution_seconds") => Some(&["operation"]),
            ("counter", "mnesis_command_rejected_total") => Some(&["operation", "outcome"]),
            ("counter", "mnesis_command_reconciliations_total") => Some(&["outcome"]),
            ("counter", "mnesis_command_worker_failures_total") => Some(&["cause"]),
            (
                "counter",
                "mnesis_command_acceptances_total"
                | "mnesis_command_duplicates_total"
                | "mnesis_command_conflicts_total",
            )
            | (
                "gauge",
                "mnesis_command_reserved"
                | "mnesis_command_queued"
                | "mnesis_command_inflight"
                | "mnesis_command_unresolved",
            )
            | ("histogram", "mnesis_command_queue_seconds" | "mnesis_command_shutdown_seconds") => {
                Some(&[])
            }
            ("histogram", "mnesis_command_stage_seconds") => Some(&["outcome", "stage"]),
            _ => None,
        };
        let mut names = metric.labels().map(Label::key).collect::<Vec<_>>();
        names.sort_unstable();
        assert_eq!(Some(names.as_slice()), expected);
        for label in metric.labels() {
            assert!(matches!(
                (label.key(), label.value()),
                ("operation", "submit" | "retry" | "read" | "original")
                    | ("cause", "panic" | "cancelled")
                    | (
                        "stage",
                        "admission" | "load" | "decision" | "append" | "reconcile" | "shutdown"
                    )
                    | (
                        "outcome",
                        "ok" | "interrupted"
                            | "failed"
                            | "mismatch"
                            | "conflict"
                            | "queue_full"
                            | "queue_limit"
                            | "deadline_zero"
                            | "closed"
                            | "scope"
                            | "runtime"
                            | "uncertain"
                            | "request"
                            | "corrupt"
                            | "timeout"
                    )
            ));
        }
    }
}
async fn stopping(recording: &Recording) {
    let store = Store::new(FaultStore::new(InMemoryStore::new()));
    let gate = Gate::new();
    store.raw().arm(Fault::Delayed(Arc::clone(&gate)));
    let (mut host, worker) = startup(&store, "stopping", WAIT);
    let active = tokio::time::timeout(WAIT, host.ready())
        .await
        .unwrap()
        .unwrap()
        .call(request(1, None, 7));
    gate.entered().await;
    let queued = tokio::time::timeout(WAIT, host.ready())
        .await
        .unwrap()
        .unwrap()
        .call(request(2, Some(Version::INITIAL), 9));
    let before = recording.histogram("mnesis_command_shutdown_seconds", &[]);
    let started = Instant::now();
    host.stop();
    let holding = Instant::now();
    tokio::time::sleep(Duration::from_millis(20)).await;
    host.stop();
    let held = holding.elapsed();
    gate.release();
    gate.done().await;
    let receipt = tokio::time::timeout(WAIT, active).await.unwrap().unwrap();
    assert!(matches!(
        tokio::time::timeout(WAIT, queued).await.unwrap(),
        Err(Error::Closed)
    ));
    let report = shutdown::finish(worker).await;
    assert_eq!(report.uncertain(), None);
    assert!(report.cause().is_none());
    let elapsed = started.elapsed();
    let records = rows(&store.raw().inner, "stopping").await;
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].request(), &request(1, None, 7));
    assert_eq!(records[0].receipt(), &receipt);
    assert_eq!(records[0].facts(), &[Increment(7)]);
    assert_eq!(recording.calls(), [Some(0.0); 4]);
    assert_eq!(before.as_ref().map(Vec::len), Some(3));
    let after = recording.samples("mnesis_command_shutdown_seconds", &[]);
    assert_eq!(after.len(), 4);
    assert!(*after.last().unwrap() >= held.as_secs_f64());
    assert!(*after.last().unwrap() <= elapsed.as_secs_f64());
}

async fn overlapping(recording: &Recording) {
    let inner = Arc::new(InMemoryStore::new());
    let first_store = Store::new(FaultStore::shared(Arc::clone(&inner)));
    let second_store = Store::new(FaultStore::shared(Arc::clone(&inner)));
    let first_gate = Gate::new();
    let second_gate = Gate::new();
    first_store
        .raw()
        .arm(Fault::Delayed(Arc::clone(&first_gate)));
    second_store
        .raw()
        .arm(Fault::Delayed(Arc::clone(&second_gate)));
    let (mut first, first_worker) = startup(&first_store, "pending-a", Duration::from_millis(50));
    let (mut second, second_worker) =
        startup(&second_store, "pending-b", Duration::from_millis(50));
    let first_response = first.ready().await.unwrap().call(request(1, None, 7));
    let second_response = second.ready().await.unwrap().call(request(2, None, 9));
    first_gate.entered().await;
    second_gate.entered().await;
    for (response, id) in [(first_response, 1), (second_response, 2)] {
        assert!(
            matches!(response.await, Err(Error::Uncertain { command, source: None })
            if command == NonZeroU64::new(id).unwrap())
        );
    }
    assert_eq!(
        recording.idle().await,
        [Some(0.0), Some(0.0), Some(0.0), Some(2.0)]
    );
    let (fresh, fresh_worker) = startup(&first_store, "pending-fresh", WAIT);
    assert_eq!(recording.gauge("mnesis_command_unresolved"), Some(2.0));
    fresh.stop();
    assert_eq!(shutdown::finish(fresh_worker).await.uncertain(), None);
    assert_eq!(recording.gauge("mnesis_command_unresolved"), Some(2.0));
    assert!(rows(&inner, "pending-a").await.is_empty());
    assert!(rows(&inner, "pending-b").await.is_empty());
    first_gate.release();
    first_gate.done().await;
    assert_eq!(
        first.read().await.unwrap(),
        (Some(Version::INITIAL), Sum(7))
    );
    assert_eq!(recording.idle().await[3], Some(1.0));
    let receipt = first.submit(request(1, None, 7)).await.unwrap();
    first.stop();
    assert_eq!(shutdown::finish(first_worker).await.uncertain(), None);
    assert_eq!(recording.gauge("mnesis_command_unresolved"), Some(1.0));
    accepted(&inner, "pending-a", &request(1, None, 7), &receipt, 7).await;
    second_worker.abort();
    let report = tokio::time::timeout(WAIT, second_worker).await.unwrap();
    assert!(report.cause().unwrap().is_cancelled());
    assert_eq!(report.uncertain(), Some(&request(2, None, 9)));
    assert_eq!(recording.calls(), [Some(0.0); 4]);
    resume_pending(&second_store, &second_gate, &report, recording).await;
    overlap_accounting(recording);
}

async fn resume_pending(
    store: &Store<Backend>,
    gate: &Gate,
    report: &mnesis_command::Shutdown<Operation>,
    recording: &Recording,
) {
    let (resumed, worker) = CommandProcessor::<Aggregate, Operation>::resume(
        store,
        "pending-b".into(),
        Limits {
            queue: NonZeroUsize::new(2).unwrap(),
            operation: WAIT,
        },
        report,
    )
    .unwrap();
    assert_eq!(recording.gauge("mnesis_command_unresolved"), Some(1.0));
    assert!(rows(&store.raw().inner, "pending-b").await.is_empty());
    gate.release();
    gate.done().await;
    assert_eq!(
        resumed.read().await.unwrap(),
        (Some(Version::INITIAL), Sum(9))
    );
    assert_eq!(recording.idle().await, [Some(0.0); 4]);
    let receipt = resumed.submit(request(2, None, 9)).await.unwrap();
    resumed.stop();
    assert_eq!(shutdown::finish(worker).await.uncertain(), None);
    assert_eq!(recording.calls(), [Some(0.0); 4]);
    accepted(
        &store.raw().inner,
        "pending-b",
        &request(2, None, 9),
        &receipt,
        9,
    )
    .await;
}

async fn accepted(
    store: &InMemoryStore,
    stream: &'static str,
    original: &Request<Operation>,
    receipt: &Receipt,
    amount: u8,
) {
    let records = rows(store, stream).await;
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].request(), original);
    assert_eq!(records[0].receipt(), receipt);
    assert_eq!((receipt.version, receipt.events), (Version::INITIAL, 1));
    assert_eq!(records[0].facts(), &[Increment(amount)]);
}

fn overlap_accounting(recording: &Recording) {
    for (name, count) in [
        ("mnesis_command_acceptances_total", 8),
        ("mnesis_command_duplicates_total", 3),
    ] {
        assert_eq!(recording.count(name), Some(count));
    }
    for (operation, submitted, admitted, unknown) in [("submit", 21, 18, 5), ("read", 6, 6, 1)] {
        for (name, count) in [
            ("mnesis_command_submitted_total", submitted),
            ("mnesis_command_admitted_total", admitted),
            ("mnesis_command_uncertain_total", unknown),
        ] {
            assert_eq!(
                recording.labeled(name, &[("operation", operation)]),
                Some(count)
            );
        }
    }
    for (operation, count) in [("submit", 16), ("read", 6)] {
        assert_eq!(
            recording
                .samples(
                    "mnesis_command_execution_seconds",
                    &[("operation", operation)]
                )
                .len(),
            count
        );
    }
    assert_eq!(
        recording.samples("mnesis_command_queue_seconds", &[]).len(),
        24
    );
    assert_eq!(
        recording
            .samples("mnesis_command_shutdown_seconds", &[])
            .len(),
        7
    );
    assert_eq!(
        recording.labeled(
            "mnesis_command_worker_failures_total",
            &[("cause", "cancelled")]
        ),
        Some(2)
    );
}
