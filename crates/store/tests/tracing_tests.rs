#![cfg(feature = "tracing")]
//! Integration tests for the `tracing` feature (issue #136): exact span
//! names/levels/nesting at the facade seams and the single INFO `caught_up`
//! event at the subscription boundary.

use std::pin::pin;
use std::sync::Arc;

use futures::StreamExt;
use mnesis::{Events, Handle};
use mnesis_inmemory::InMemoryStore;
use mnesis_store::{CommandRepository, Execution, Repository, Step, Store, Subscription};
use mnesis_test_domains::{Counter, CounterError, CounterEvent, CounterState, TestId};
use parking_lot::Mutex;
use tracing::{Level, span};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::{Context, SubscriberExt};

/// One captured span or event: name, level, and the parent span name (if any).
#[derive(Debug)]
struct Captured {
    name: &'static str,
    level: Level,
    parent: Option<&'static str>,
}

/// Capturing `tracing` layer that records span creation and event emission.
///
/// The layer is `Clone` so it can be installed into the default subscriber while
/// the test retains a handle to inspect recorded data.
#[derive(Clone, Default)]
struct Capture {
    spans: Arc<Mutex<Vec<Captured>>>,
    events: Arc<Mutex<Vec<Captured>>>,
}

impl<S> Layer<S> for Capture
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    fn on_new_span(&self, _attrs: &span::Attributes<'_>, id: &span::Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else {
            return;
        };
        let meta = span.metadata();
        let parent = span.parent().map(|p| p.metadata().name());
        self.spans.lock().push(Captured {
            name: meta.name(),
            level: *meta.level(),
            parent,
        });
    }

    fn on_event(&self, event: &tracing::Event<'_>, ctx: Context<'_, S>) {
        let meta = event.metadata();
        let parent = ctx
            .current_span()
            .id()
            .and_then(|id| ctx.span(id))
            .map(|span| span.metadata().name());
        self.events.lock().push(Captured {
            name: meta.name(),
            level: *meta.level(),
            parent,
        });
    }
}

/// Local command that sets the counter to a fixed value.
struct Set(i64);

impl Handle<Set> for Counter {
    fn handle(
        _state: &CounterState,
        cmd: Set,
    ) -> Result<Option<Events<CounterEvent>>, CounterError> {
        Ok(Some(Events::new(CounterEvent::Set(cmd.0))))
    }
}

fn subscriber(capture: Capture) -> impl tracing::Subscriber + Send + Sync {
    tracing_subscriber::registry().with(capture)
}

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

/// `load` → `execute` emits exactly three spans, with `save` nested under
/// `execute`.
#[tokio::test]
async fn execute_emits_nested_spans() -> TestResult {
    let capture = Capture::default();
    let _guard = tracing::subscriber::set_default(subscriber(capture.clone()));

    let store = Store::new(InMemoryStore::new());
    let repo = store.repository::<Counter>().json().build();
    let id = TestId::new("trace-execute");

    let mut root = repo.load(id).await?;
    let executed = repo.execute(&mut root, Set(1)).await?;
    assert!(matches!(executed, Execution::Executed { .. }));

    // Copy the captured data out and release the parking_lot guards before
    // any assertion runs.
    let spans = capture.spans.lock();
    let names: Vec<&'static str> = spans.iter().map(|c| c.name).collect();
    let save_parent = spans
        .iter()
        .find(|c| c.name == "mnesis.aggregate.save")
        .and_then(|c| c.parent);
    let levels: Vec<Level> = spans.iter().map(|c| c.level).collect();
    drop(spans);
    let event_count = capture.events.lock().len();

    assert_eq!(
        names,
        vec![
            "mnesis.aggregate.load",
            "mnesis.aggregate.execute",
            "mnesis.aggregate.save",
        ]
    );
    assert!(levels.iter().all(|level| *level == Level::DEBUG));
    assert_eq!(save_parent, Some("mnesis.aggregate.execute"));
    assert_eq!(event_count, 0);

    Ok(())
}

/// The `mnesis.subscription.caught_up` INFO event fires exactly once when a
/// subscription reaches the backlog→live boundary.
#[tokio::test]
async fn caught_up_event_emitted_exactly_once() -> TestResult {
    let capture = Capture::default();
    let _guard = tracing::subscriber::set_default(subscriber(capture.clone()));

    let store = Store::new(InMemoryStore::new());
    let id = TestId::new("trace-subscription");

    // Seed one event so the subscription has a backlog to drain.
    let repo = store.repository::<Counter>().json().build();
    let mut root = repo.load(id.clone()).await?;
    let executed = repo.execute(&mut root, Set(1)).await?;
    assert!(matches!(executed, Execution::Executed { .. }));

    let subscription = Subscription::new(&store);
    let sub_stream = subscription.subscribe(&id, None)?;
    let mut cursor = pin!(sub_stream);

    while let Some(item) = cursor.next().await {
        match item? {
            Step::CaughtUp => break,
            Step::Event(_) => {}
        }
    }

    let events = capture.events.lock();
    let caught_up_levels: Vec<Level> = events
        .iter()
        .filter(|c| c.name == "mnesis.subscription.caught_up")
        .map(|c| c.level)
        .collect();
    drop(events);

    assert_eq!(caught_up_levels.len(), 1);
    assert_eq!(caught_up_levels[0], Level::INFO);

    Ok(())
}
