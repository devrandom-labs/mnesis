#![cfg(feature = "tracing")]
#![allow(
    clippy::unwrap_used,
    reason = "trace contract failures retain exact observations"
)]

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::io::ErrorKind;
use std::num::{NonZeroU64, NonZeroUsize};
use std::panic::panic_any;
use std::sync::Arc;
use std::time::Duration;

use futures::TryStreamExt;
use mnesis::{Events, Handle, Version};
use mnesis_command::{
    Acceptance, AcceptanceCodec, CodecError, CommandProcessor, Error, Limits, Request, Worker,
};
use mnesis_inmemory::InMemoryStore;
use mnesis_store::{Decode, RawEventStore, Store, StreamKey};
use parking_lot::Mutex;
use tower::{Service, ServiceExt};
use tracing::field::{Field, Visit};
use tracing::instrument::WithSubscriber;
use tracing::span::{Attributes, Id, Record};
use tracing::{Dispatch, Event, Instrument, Span as Caller, Subscriber, dispatcher, info_span};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;

#[path = "support/counter.rs"]
mod counter;
#[path = "faults/mod.rs"]
#[allow(
    clippy::expect_used,
    reason = "existing shared fault helpers retain timeout diagnostics"
)]
mod faults;
#[path = "support/finish.rs"]
#[allow(
    clippy::expect_used,
    reason = "existing shared completion helper retains timeout diagnostics"
)]
mod shutdown;
use counter::{Counter, Increment, Operation, Rejection, Sum};
use faults::{Fault, FaultStore, Gate};

const WAIT: Duration = Duration::from_secs(2);
type Backend = FaultStore<InMemoryStore>;
type Host = CommandProcessor<Counter, Operation, NonZeroU64, 0, <Backend as RawEventStore>::Error>;
type Response = <Host as Service<Request<Operation>>>::Future;

#[derive(Default)]
struct Fields(BTreeMap<&'static str, String>);
impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        self.0.insert(field.name(), format!("{value:?}"));
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name(), value.to_owned());
    }
}
struct Span {
    name: &'static str,
    parent: Option<u64>,
    fields: Fields,
}
#[derive(Default)]
struct Log {
    spans: BTreeMap<u64, Span>,
    created: Vec<u64>,
    events: Vec<(&'static str, Option<u64>, Fields)>,
    closed: Vec<u64>,
}
#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Log>>);
impl<S> Layer<S> for Capture
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_new_span(&self, attributes: &Attributes<'_>, id: &Id, context: Context<'_, S>) {
        let mut fields = Fields::default();
        attributes.record(&mut fields);
        let parent = context
            .span(id)
            .unwrap()
            .parent()
            .map(|span| span.id().into_u64());
        let mut log = self.0.lock();
        log.created.push(id.into_u64());
        log.spans.insert(
            id.into_u64(),
            Span {
                name: attributes.metadata().name(),
                parent,
                fields,
            },
        );
    }
    fn on_record(&self, id: &Id, values: &Record<'_>, _: Context<'_, S>) {
        values.record(&mut self.0.lock().spans.get_mut(&id.into_u64()).unwrap().fields);
    }
    fn on_event(&self, event: &Event<'_>, context: Context<'_, S>) {
        let mut fields = Fields::default();
        event.record(&mut fields);
        let parent = context.event_span(event).map(|span| span.id().into_u64());
        self.0
            .lock()
            .events
            .push((event.metadata().name(), parent, fields));
    }
    fn on_close(&self, id: Id, _: Context<'_, S>) {
        self.0.lock().closed.push(id.into_u64());
    }
}
fn collector(capture: &Capture) -> Dispatch {
    Dispatch::new(tracing_subscriber::registry().with(capture.clone()))
}
fn startup(store: &Store<Backend>, subscriber: &Dispatch) -> (Host, Worker<Operation>) {
    dispatcher::with_default(subscriber, || {
        info_span!("startup").in_scope(|| {
            CommandProcessor::<Counter, Operation>::start(
                store,
                "trace".into(),
                Limits {
                    queue: NonZeroUsize::MIN,
                    operation: WAIT,
                },
            )
            .unwrap()
        })
    })
}
async fn admit(
    host: &mut Host,
    input: Request<Operation>,
    caller: Caller,
    subscriber: Dispatch,
) -> Response {
    tokio::time::timeout(WAIT, host.ready())
        .await
        .unwrap()
        .unwrap();
    dispatcher::with_default(&subscriber, || caller.in_scope(|| host.call(input)))
}
const fn request(id: u64, expected: Option<Version>, amount: u8) -> Request<Operation> {
    Request {
        id: NonZeroU64::new(id).unwrap(),
        expected,
        command: Operation::Add(amount),
    }
}
async fn committed(store: &InMemoryStore) -> Vec<Acceptance<Operation, Increment>> {
    store
        .read_stream(&StreamKey::from_slice(b"trace"), Version::INITIAL)
        .await
        .unwrap()
        .map_ok(|row| AcceptanceCodec.decode(&row).unwrap())
        .try_collect()
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn caller_context_survives_the_queue_and_an_unpolled_response_drop() {
    let first_log = Capture::default();
    let second_log = Capture::default();
    let first_dispatch = collector(&first_log);
    let second_dispatch = collector(&second_log);
    let store = Store::new(FaultStore::new(InMemoryStore::new()));
    let gate = Gate::new();
    store.raw().arm(Fault::Delayed(Arc::clone(&gate)));
    let (mut processor, worker) = startup(&store, &first_dispatch);
    let first_caller = dispatcher::with_default(&first_dispatch, || info_span!("first_caller"));
    let first_id = first_caller.id().unwrap().into_u64();
    let first = admit(
        &mut processor,
        request(1, None, 7),
        first_caller,
        first_dispatch.clone(),
    )
    .await;
    gate.entered().await;
    let second_caller = dispatcher::with_default(&second_dispatch, || info_span!("second_caller"));
    let second_id = second_caller.id().unwrap().into_u64();
    let second = admit(
        &mut processor,
        request(2, Some(Version::INITIAL), 9),
        second_caller,
        second_dispatch,
    )
    .await;
    drop(second);
    gate.release();
    gate.done().await;
    let receipt = tokio::time::timeout(WAIT, first).await.unwrap().unwrap();
    tokio::time::timeout(WAIT, async {
        while committed(&store.raw().inner).await.len() < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let third_caller = dispatcher::with_default(&first_dispatch, || info_span!("third_caller"));
    let third_id = third_caller.id().unwrap().into_u64();
    async {
        assert_eq!(processor.read().await.unwrap(), (Version::new(2), Sum(16)));
        assert_eq!(
            processor.submit(request(1, None, 7)).await.unwrap(),
            receipt
        );
    }
    .instrument(third_caller)
    .with_subscriber(first_dispatch)
    .await;
    processor.stop();
    assert_eq!(shutdown::finish(worker).await.uncertain(), None);
    let rows = committed(&store.raw().inner).await;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].request(), &request(1, None, 7));
    assert_eq!(rows[1].request(), &request(2, Some(Version::INITIAL), 9));
    assert_eq!(rows[0].facts(), &[Increment(7)]);
    assert_eq!(rows[1].facts(), &[Increment(9)]);
    context(
        &first_log.0.lock(),
        &second_log.0.lock(),
        [first_id, second_id, third_id],
    );
}
fn context(first: &Log, second: &Log, [first_id, second_id, third_id]: [u64; 3]) {
    verify(first, first_id, &["submit"]);
    verify(first, third_id, &["read", "submit"]);
    verify(second, second_id, &["submit"]);
    assert_eq!(accepted(first), 1);
    assert_eq!(accepted(second), 1);
    events(first, &["accepted", "duplicate"]);
    events(second, &["accepted", "reply_dropped"]);
    stages(
        first,
        &[
            "admission",
            "admission",
            "admission",
            "append",
            "decision",
            "load",
            "load",
            "load",
            "queue",
            "queue",
            "queue",
            "shutdown",
        ],
    );
    stages(
        second,
        &["admission", "append", "decision", "load", "queue"],
    );
    closed(first);
    closed(second);
    worker_spans(first, "ok");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn abort_closes_another_subscribers_active_and_queued_spans() {
    let startup_log = Capture::default();
    let caller_log = Capture::default();
    let subscriber = collector(&caller_log);
    let store = Store::new(FaultStore::new(InMemoryStore::new()));
    let gate = Gate::new();
    *store.raw().read_gate.lock() = Some(Arc::clone(&gate));
    let (mut processor, worker) = startup(&store, &collector(&startup_log));
    let caller = dispatcher::with_default(&subscriber, || info_span!("caller"));
    let first = admit(
        &mut processor,
        request(1, None, 7),
        caller.clone(),
        subscriber.clone(),
    )
    .await;
    gate.entered().await;
    let second = admit(&mut processor, request(2, None, 9), caller, subscriber).await;
    worker.abort();
    let report = tokio::time::timeout(WAIT, worker).await.unwrap();
    assert!(report.cause().unwrap().is_cancelled());
    assert_eq!(report.uncertain(), None);
    assert!(
        matches!(tokio::time::timeout(WAIT, first).await.unwrap(), Err(Error::Uncertain { command, .. }) if command == NonZeroU64::MIN)
    );
    assert!(matches!(
        tokio::time::timeout(WAIT, second).await.unwrap(),
        Err(Error::Closed)
    ));
    assert_eq!(committed(&store.raw().inner).await.len(), 0);
    let log = caller_log.0.lock();
    let outcomes = log
        .spans
        .values()
        .filter(|span| span.name == "mnesis.command.request")
        .map(|span| span.fields.0["outcome"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(outcomes, ["interrupted", "closed"]);
    closed(&log);
    drop(log);
    closed(&startup_log.0.lock());
    worker_spans(&startup_log.0.lock(), "interrupted");
}

impl Handle<u8> for Counter {
    #[allow(
        clippy::panic,
        reason = "assert tracing cleanup retains the original native panic"
    )]
    fn handle(_: &Sum, _: u8) -> Result<Option<Events<Increment>>, Rejection> {
        panic_any(73_u32);
    }
}
#[tokio::test]
async fn panic_preserves_its_native_cause_across_subscriber_cleanup() {
    let startup_log = Capture::default();
    let caller_log = Capture::default();
    let startup_dispatch = collector(&startup_log);
    let caller_dispatch = collector(&caller_log);
    let store = Store::new(InMemoryStore::new());
    let (mut host, worker) = dispatcher::with_default(&startup_dispatch, || {
        CommandProcessor::<Counter, u8>::start(
            &store,
            "trace".into(),
            Limits {
                queue: NonZeroUsize::MIN,
                operation: WAIT,
            },
        )
        .unwrap()
    });
    let input = Request {
        id: NonZeroU64::MIN,
        expected: None,
        command: 7,
    };
    tokio::time::timeout(WAIT, host.ready())
        .await
        .unwrap()
        .unwrap();
    let response = dispatcher::with_default(&caller_dispatch, || {
        info_span!("panic_caller").in_scope(|| host.call(input.clone()))
    });
    assert!(
        matches!(tokio::time::timeout(WAIT, response).await.unwrap(), Err(Error::Uncertain { command, .. }) if command == NonZeroU64::MIN)
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
    assert_eq!(committed(store.raw()).await.len(), 0);
    closed(&caller_log.0.lock());
    closed(&startup_log.0.lock());
    worker_spans(&startup_log.0.lock(), "interrupted");
}

fn verify(log: &Log, parent: u64, operations: &[&str]) {
    let requests = log
        .created
        .iter()
        .filter_map(|id| {
            let span = &log.spans[id];
            (span.name == "mnesis.command.request" && span.parent == Some(parent))
                .then_some((id, span))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        requests
            .iter()
            .map(|(_, span)| span.fields.0["operation"].as_str())
            .collect::<Vec<_>>(),
        operations
    );
    for (id, span) in requests {
        assert_eq!(span.fields.0["outcome"], "ok");
        assert_eq!(
            span.fields.0.keys().copied().collect::<Vec<_>>(),
            ["operation", "outcome"]
        );
        assert_eq!(
            log.spans
                .values()
                .filter(|stage| stage.parent == Some(*id)
                    && stage.fields.0.get("stage").map(String::as_str) == Some("load"))
                .count(),
            1
        );
    }
    for span in log
        .spans
        .values()
        .filter(|span| span.name.starts_with("mnesis.command"))
    {
        assert!(
            span.fields
                .0
                .keys()
                .all(|name| ["operation", "outcome", "stage"].contains(name))
        );
    }
    for (name, _, fields) in &log.events {
        if name.starts_with("mnesis.command") {
            assert!(
                fields
                    .0
                    .keys()
                    .all(|field| ["operation", "outcome", "stage"].contains(field))
            );
        }
    }
}
fn accepted(log: &Log) -> usize {
    log.events
        .iter()
        .filter(|(name, _, _)| *name == "mnesis.command.accepted")
        .count()
}

fn stages(log: &Log, expected: &[&str]) {
    let mut actual = log
        .spans
        .values()
        .filter_map(|span| {
            let stage = span.fields.0.get("stage")?;
            assert_eq!(
                span.fields.0.keys().copied().collect::<Vec<_>>(),
                ["outcome", "stage"]
            );
            assert_eq!(span.fields.0["outcome"], "ok");
            Some(stage.as_str())
        })
        .collect::<Vec<_>>();
    actual.sort_unstable();
    assert_eq!(actual, expected);
}
fn events(log: &Log, expected: &[&str]) {
    let actual = log
        .events
        .iter()
        .filter_map(|(name, parent, fields)| {
            let suffix = name.strip_prefix("mnesis.command.")?;
            assert_eq!(log.spans[&parent.unwrap()].name, "mnesis.command.request");
            assert_eq!(
                fields.0["outcome"],
                if suffix == "reply_dropped" {
                    "cancelled"
                } else {
                    "ok"
                }
            );
            Some(suffix)
        })
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
}
fn closed(log: &Log) {
    let mut closed = log.closed.clone();
    closed.sort_unstable();
    assert_eq!(closed, log.spans.keys().copied().collect::<Vec<_>>());
}
fn worker_spans(log: &Log, outcome: &str) {
    let workers = log
        .spans
        .values()
        .filter(|span| span.name == "mnesis.command.worker")
        .map(|span| (span.parent, span.fields.0["outcome"].as_str()))
        .collect::<Vec<_>>();
    assert_eq!(workers, [(None, outcome)]);
    let cleanup = log
        .spans
        .values()
        .filter(|span| span.fields.0.get("stage").map(String::as_str) == Some("shutdown"))
        .collect::<Vec<_>>();
    assert_eq!(cleanup.len(), 1);
    assert_eq!(cleanup[0].fields.0["outcome"], "ok");
    assert_eq!(
        log.spans[&cleanup[0].parent.unwrap()].name,
        "mnesis.command.worker"
    );
}

#[tokio::test]
async fn invalid_requests_are_observed_without_payloads_or_spending_admission() {
    let capture = Capture::default();
    let subscriber = collector(&capture);
    let store = Store::new(InMemoryStore::new());
    let (mut host, worker) = dispatcher::with_default(&subscriber, || {
        CommandProcessor::<Counter, Operation, String>::start(
            &store,
            "trace".into(),
            Limits {
                queue: NonZeroUsize::MIN,
                operation: WAIT,
            },
        )
        .unwrap()
    });
    let invalid = Request {
        id: "x".repeat(4097),
        expected: None,
        command: Operation::Add(3),
    };
    let valid = Request {
        id: "stable".to_owned(),
        ..invalid.clone()
    };
    let receipt = async {
        let direct = tokio::time::timeout(WAIT, host.submit(invalid.clone())).await.unwrap();
        tokio::time::timeout(WAIT, host.ready()).await.unwrap().unwrap();
        let service = tokio::time::timeout(WAIT, host.call(invalid)).await.unwrap();
        for result in [direct, service] {
            assert!(matches!(result, Err(Error::Request(CodecError::Bytes { source: Some(ref error) })) if error.io_error_kind() == Some(ErrorKind::WriteZero)));
        }
        let receipt = tokio::time::timeout(WAIT, host.submit(valid.clone())).await.unwrap().unwrap();
        host.stop();
        assert_eq!(shutdown::finish(worker).await.uncertain(), None);
        receipt
    }.with_subscriber(subscriber).await;
    assert_eq!(
        (&receipt.id, receipt.version, receipt.events),
        (&valid.id, Version::INITIAL, 1)
    );
    let rows = store
        .raw()
        .read_stream(&StreamKey::from_slice(b"trace"), Version::INITIAL)
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    let accepted: Acceptance<Operation, Increment, String> =
        AcceptanceCodec.decode(&rows[0]).unwrap();
    assert_eq!(accepted.request(), &valid);
    assert_eq!(accepted.receipt(), &receipt);
    assert_eq!(accepted.facts(), &[Increment(3)]);
    invalid_observations(&capture.0.lock());
}
fn invalid_observations(log: &Log) {
    let mut outcomes = log
        .spans
        .values()
        .filter(|span| span.name == "mnesis.command.request")
        .map(|span| {
            assert_eq!(
                span.fields.0.keys().copied().collect::<Vec<_>>(),
                ["operation", "outcome"]
            );
            assert_eq!(span.fields.0["operation"], "submit");
            span.fields.0["outcome"].as_str()
        })
        .collect::<Vec<_>>();
    outcomes.sort_unstable();
    assert_eq!(outcomes, ["ok", "request", "request"]);
    let mut stages = log
        .spans
        .values()
        .filter_map(|span| {
            let stage = span.fields.0.get("stage")?;
            assert_eq!(
                span.fields.0.keys().copied().collect::<Vec<_>>(),
                ["outcome", "stage"]
            );
            Some((stage.as_str(), span.fields.0["outcome"].as_str()))
        })
        .collect::<Vec<_>>();
    stages.sort_unstable();
    assert_eq!(
        stages,
        [
            ("admission", "ok"),
            ("admission", "request"),
            ("admission", "request"),
            ("append", "ok"),
            ("decision", "ok"),
            ("load", "ok"),
            ("queue", "ok"),
            ("shutdown", "ok")
        ]
    );
    assert_eq!(accepted(log), 1);
    closed(log);
    worker_spans(log, "ok");
}
