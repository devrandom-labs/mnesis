//! Required operational evidence comes from actual adapter work.
#![cfg(feature = "tracing")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "native diagnostics assertions"
)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use mnesis::Version;
use mnesis_postgres::PostgresStore;
use mnesis_store::wake::{WakeRegistration, WakeSource};
use mnesis_store::{AppendError, PendingBatch, RawEventStore, StreamKey, pending_envelope};
use parking_lot::Mutex;
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use tokio::task::yield_now;
use tokio::time::timeout;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::{Layer, Registry};

#[derive(Clone, Default)]
struct Records(Arc<Mutex<Vec<BTreeMap<String, String>>>>);

#[derive(Default)]
struct Fields(BTreeMap<String, String>);

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}"));
    }
}

impl<S: Subscriber> Layer<S> for Records {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        if event.metadata().target().starts_with("mnesis_postgres") {
            let mut fields = Fields::default();
            fields
                .0
                .insert("event".to_owned(), event.metadata().name().to_owned());
            event.record(&mut fields);
            self.0.lock().push(fields.0);
        }
    }

    fn on_new_span(&self, attributes: &Attributes<'_>, _id: &Id, _context: Context<'_, S>) {
        if attributes
            .metadata()
            .target()
            .starts_with("mnesis_postgres")
        {
            let mut fields = Fields::default();
            attributes.record(&mut fields);
            self.0.lock().push(fields.0);
        }
    }
}

async fn observed(records: &Records, name: &str, outcome: &str) -> BTreeMap<String, String> {
    let completion = timeout(Duration::from_secs(3), async {
        loop {
            let captured = records
                .0
                .lock()
                .iter()
                .find(|fields| {
                    fields.get("event").is_some_and(|value| value == name)
                        && fields
                            .get("outcome")
                            .is_some_and(|value| value == &format!("{outcome:?}"))
                })
                .cloned();
            if let Some(fields) = captured {
                break fields;
            }
            yield_now().await;
        }
    })
    .await;
    assert!(
        completion.is_ok(),
        "missing {name}/{outcome}: {:?}",
        records.0.lock()
    );
    completion.unwrap()
}

async fn open(url: &str) -> (PostgresStore, PgPool, i32) {
    let options = url
        .parse::<PgConnectOptions>()
        .unwrap()
        .application_name("mnesis-diagnostics");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
        .unwrap();
    PostgresStore::migrate(&pool).await.unwrap();
    sqlx::query("TRUNCATE mnesis.events RESTART IDENTITY")
        .execute(&pool)
        .await
        .unwrap();
    let store = PostgresStore::from_pool(pool.clone()).await.unwrap();
    let ready =
        timeout(Duration::from_secs(3), async {
            loop {
                let backend: Option<i32> = sqlx::query_scalar(
                "SELECT pid FROM pg_stat_activity WHERE application_name = 'mnesis-diagnostics' \
                 AND state = 'idle' AND query LIKE 'LISTEN %mnesis_events%' LIMIT 1",
            ).fetch_optional(&pool).await.unwrap();
                if let Some(pid) = backend {
                    break pid;
                }
                yield_now().await;
            }
        })
        .await
        .unwrap();
    (store, pool, ready)
}

#[tokio::test]
async fn actual_listener_scan_and_commit_work_emit_bounded_operational_fields() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let records = Records::default();
    tracing::subscriber::set_global_default(Registry::default().with(records.clone())).unwrap();
    let (store, pool, ready) = open(&url).await;
    let key = StreamKey::from_bytes("diagnostic-key-probe");
    let event = pending_envelope(Version::INITIAL)
        .event_type("Probe")
        .payload(b"diagnostic-payload-probe".as_slice())
        .build()
        .unwrap();
    store
        .append(&key, None, PendingBatch::of(&event))
        .await
        .unwrap();
    let conflict = store.append(&key, None, PendingBatch::of(&event)).await;
    assert!(matches!(conflict, Err(AppendError::Conflict { .. })));
    assert_eq!(
        store
            .read_stream(&key, Version::INITIAL)
            .await
            .unwrap()
            .next()
            .await
            .unwrap()
            .unwrap()
            .version(),
        Version::INITIAL
    );
    assert_eq!(
        store
            .read_all(None)
            .await
            .unwrap()
            .next()
            .await
            .unwrap()
            .unwrap()
            .2
            .version(),
        Version::INITIAL
    );
    let registration = store.register(Some(key.as_bytes())).unwrap();
    timeout(Duration::from_secs(3), registration.arm())
        .await
        .unwrap();
    let terminated: bool = sqlx::query_scalar("SELECT pg_terminate_backend($1)")
        .bind(ready)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(terminated);
    observed(&records, "mnesis.postgres.listener", "ready").await;
    observed(&records, "mnesis.postgres.listener", "reconnected").await;
    observed(&records, "mnesis.postgres.commit", "committed").await;
    observed(&records, "mnesis.postgres.append", "conflict").await;
    let stream = observed(&records, "mnesis.postgres.page", "stream").await;
    let all = observed(&records, "mnesis.postgres.page", "all").await;
    observed(&records, "mnesis.postgres.rescan", "deadline").await;
    timeout(Duration::from_secs(3), pool.close()).await.unwrap();
    observed(&records, "mnesis.postgres.listener", "closed").await;
    drop(store);
    let fields = records.0.lock();
    for page in [stream, all] {
        assert_eq!(page.get("rows").map(String::as_str), Some("1"));
        assert!(page.contains_key("elapsed"));
    }
    for name in fields.iter().flat_map(BTreeMap::keys) {
        assert!(
            [
                "event", "outcome", "rows", "elapsed", "cutoff", "kind", "sqlstate", "message"
            ]
            .contains(&name.as_str())
        );
    }
    for field in fields.iter().flat_map(BTreeMap::values) {
        assert!(!field.contains("diagnostic-key-probe"));
        assert!(!field.contains("diagnostic-payload-probe"));
    }
}
