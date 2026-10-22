#![allow(
    clippy::unwrap_used,
    reason = "example contract failures retain concrete observations"
)]

#[cfg(feature = "telemetry")]
use std::sync::Arc;
use std::time::Duration;

use futures::TryStreamExt;
use mnesis::Version;
use mnesis_command::{Acceptance, AcceptanceCodec};
use mnesis_inmemory::InMemoryStore;
use mnesis_store::{Decode, RawEventStore, Store, StreamKey};

use crate::domain::Changed;
use crate::{Summary, originals, run};
#[cfg(feature = "telemetry")]
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
#[cfg(feature = "telemetry")]
use opentelemetry_sdk::trace::{InMemorySpanExporter, SpanData, SpanExporter};
#[cfg(feature = "telemetry")]
use parking_lot::Mutex;
#[cfg(feature = "telemetry")]
use tracing::Instrument;

async fn observed(store: &Store<InMemoryStore>, summary: &Summary) {
    assert_eq!(summary.first, summary.duplicate);
    assert_eq!(summary.repeated, Some(summary.first.clone()));
    assert_eq!((summary.before, summary.after), (7, 7));
    assert_eq!(
        (summary.first.version, summary.first.events),
        (Version::INITIAL, 1)
    );
    assert_eq!(
        (Some(summary.noop.version), summary.noop.events),
        (Version::INITIAL.next(), 0)
    );
    let records: Vec<Acceptance<u64, Changed>> = store
        .raw()
        .read_stream(&StreamKey::from_slice(b"example"), Version::INITIAL)
        .await
        .unwrap()
        .map_ok(|row| AcceptanceCodec.decode(&row).unwrap())
        .try_collect()
        .await
        .unwrap();
    assert_eq!(records.len(), 2);
    for (record, original) in records.iter().zip(originals()) {
        assert_eq!(record.request(), &original);
    }
    assert_eq!(records[0].receipt(), &summary.first);
    assert_eq!(records[1].receipt(), &summary.noop);
    assert_eq!(records[0].facts(), &[Changed(7)]);
    assert_eq!(records[1].facts(), &[]);
}

#[cfg(not(feature = "telemetry"))]
#[tokio::test]
async fn tower_receipts_and_observed_resume_work_without_collectors() {
    let store = Store::new(InMemoryStore::new());
    let summary = tokio::time::timeout(Duration::from_secs(3), run(&store))
        .await
        .unwrap()
        .unwrap();
    observed(&store, &summary).await;
}

#[cfg(feature = "telemetry")]
#[derive(Clone, Copy, Debug)]
enum Destination {
    Available,
    Unavailable,
}
#[cfg(feature = "telemetry")]
#[derive(Debug)]
struct Exporter {
    inner: InMemorySpanExporter,
    destination: Arc<Mutex<Destination>>,
}
#[cfg(feature = "telemetry")]
impl SpanExporter for Exporter {
    async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
        let destination = *self.destination.lock();
        match destination {
            Destination::Available => self.inner.export(batch).await,
            Destination::Unavailable => Err(OTelSdkError::InternalFailure(
                "collector unavailable".into(),
            )),
        }
    }
    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.inner.shutdown_with_timeout(timeout)
    }
    fn set_resource(&mut self, resource: &opentelemetry_sdk::Resource) {
        self.inner.set_resource(resource);
    }
}

#[cfg(feature = "telemetry")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sdk_exports_context_and_unavailable_collection_preserves_receipts() {
    let inner = InMemorySpanExporter::default();
    let destination = Arc::new(Mutex::new(Destination::Available));
    let exporter = Exporter {
        inner: inner.clone(),
        destination: Arc::clone(&destination),
    };
    let (provider, metrics) = crate::telemetry::install(exporter).unwrap();
    let store = Store::new(InMemoryStore::new());
    let summary = tokio::time::timeout(
        Duration::from_secs(3),
        run(&store).instrument(tracing::info_span!("application")),
    )
    .await
    .unwrap()
    .unwrap();
    observed(&store, &summary).await;
    provider.force_flush().unwrap();
    exported(&inner.get_finished_spans().unwrap());
    counters(&metrics.render(), [3, 1, 2, 2, 2]);
    *destination.lock() = Destination::Unavailable;
    drop(tracing::info_span!("unavailable.probe"));
    assert!(matches!(
        provider.force_flush(),
        Err(OTelSdkError::InternalFailure(_))
    ));
    let unavailable = Store::new(InMemoryStore::new());
    let repeated = tokio::time::timeout(
        Duration::from_secs(3),
        run(&unavailable).instrument(tracing::info_span!("application")),
    )
    .await
    .unwrap()
    .unwrap();
    observed(&unavailable, &repeated).await;
    assert_eq!(repeated.first, summary.first);
    assert_eq!(repeated.noop, summary.noop);
    assert!(matches!(
        provider.force_flush(),
        Err(OTelSdkError::InternalFailure(_))
    ));
    counters(&metrics.render(), [6, 2, 4, 4, 4]);
    provider.shutdown().unwrap();
    assert!(inner.is_shutdown_called());
}

#[cfg(feature = "telemetry")]
fn exported(spans: &[SpanData]) {
    let roots = spans
        .iter()
        .filter(|span| span.name == "application")
        .collect::<Vec<_>>();
    assert_eq!(roots.len(), 1);
    let requests = spans
        .iter()
        .filter(|span| span.name == "mnesis.command.request")
        .collect::<Vec<_>>();
    assert_eq!(requests.len(), 6);
    let mut operations = Vec::new();
    for request in requests {
        assert_eq!(request.parent_span_id, roots[0].span_context.span_id());
        assert_eq!(
            request.span_context.trace_id(),
            roots[0].span_context.trace_id()
        );
        let field = |name| {
            request
                .attributes
                .iter()
                .find(|field| field.key.as_str() == name)
                .map(|field| field.value.to_string())
        };
        assert_eq!(field("outcome"), Some("ok".into()));
        operations.push(field("operation").unwrap());
    }
    operations.sort();
    assert_eq!(
        operations,
        ["read", "read", "retry", "submit", "submit", "submit"]
    );
    let workers = spans
        .iter()
        .filter(|span| span.name == "mnesis.command.worker")
        .collect::<Vec<_>>();
    assert_eq!(workers.len(), 2);
    assert!(
        workers
            .iter()
            .all(|span| span.parent_span_id == opentelemetry::trace::SpanId::INVALID)
    );
}

#[cfg(feature = "telemetry")]
fn counters(rendered: &str, [submit, retry, read, accepted, duplicate]: [u64; 5]) {
    for (name, expected) in [
        (
            "mnesis_command_submitted_total{operation=\"submit\"}",
            submit,
        ),
        ("mnesis_command_submitted_total{operation=\"retry\"}", retry),
        ("mnesis_command_submitted_total{operation=\"read\"}", read),
        ("mnesis_command_acceptances_total", accepted),
        ("mnesis_command_duplicates_total", duplicate),
    ] {
        let lines = rendered
            .lines()
            .filter(|line| line.starts_with(&format!("{name} ")))
            .collect::<Vec<_>>();
        assert_eq!(lines, [format!("{name} {expected}")]);
    }
    for name in ["queued", "reserved", "inflight", "unresolved"] {
        let expected = format!("mnesis_command_{name} 0");
        let lines = rendered
            .lines()
            .filter(|line| line.starts_with(&format!("mnesis_command_{name} ")))
            .collect::<Vec<_>>();
        assert_eq!(lines, [expected]);
    }
}
