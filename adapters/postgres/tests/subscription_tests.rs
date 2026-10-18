//! Durable rows must remain discoverable when the final notification is lost.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test assertions")]
#![allow(clippy::missing_panics_doc, reason = "test assertions")]

use std::pin::pin;
use std::time::Duration;

use futures::{Stream, StreamExt};
use mnesis::Version;
use mnesis_postgres::{PostgresError, PostgresStore};
use mnesis_store::envelope::{PendingBatch, pending_envelope};
use mnesis_store::store::RawEventStore;
use mnesis_store::wake::{WakeRegistration, WakeSource};
use mnesis_store::{Step, Store, StreamKey, Subscription};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tokio::time::timeout;

async fn fresh() -> Option<(Store<PostgresStore>, PgPool)> {
    let url = std::env::var("DATABASE_URL").ok()?;
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await
        .unwrap();
    let store = PostgresStore::from_pool(pool.clone()).await.unwrap();
    sqlx::query("TRUNCATE events RESTART IDENTITY")
        .execute(&pool)
        .await
        .unwrap();
    Some((Store::new(store), pool))
}

async fn park<T>(cursor: &mut (impl Stream<Item = Result<Step<T>, PostgresError>> + Unpin)) {
    assert!(matches!(cursor.next().await, Some(Ok(Step::CaughtUp))));
    // Cancelling `next()` preserves the stream's in-progress unfold future.
    // Drive its confirming empty scan to the parked wait before committing.
    assert!(
        timeout(Duration::from_millis(100), cursor.next())
            .await
            .is_err()
    );
}

async fn commit_without_hint(pool: &PgPool, id: &StreamKey) {
    sqlx::query(
        "INSERT INTO events (stream_id, version, event_type, schema_version, payload) \
         VALUES ($1, 1, 'Final', 1, 'payload'::bytea)",
    )
    .bind(id.as_bytes())
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn stream_recovers_the_final_missing_hint() {
    let Some((store, pool)) = fresh().await else {
        return;
    };
    let id = StreamKey::from_bytes("final-stream");
    let subscription = Subscription::new(&store);
    let mut cursor = pin!(subscription.subscribe(&id, None).unwrap());
    park(&mut cursor).await;
    commit_without_hint(&pool, &id).await;
    let next = timeout(Duration::from_secs(3), cursor.next()).await;
    let step = next
        .expect("final missed hint must recover")
        .unwrap()
        .unwrap();
    let event = step.event().expect("expected the committed event");
    assert_eq!(event.version(), Version::INITIAL);
    assert_eq!(event.payload(), b"payload");
}

#[tokio::test]
async fn all_recovers_the_final_missing_hint() {
    let Some((store, pool)) = fresh().await else {
        return;
    };
    let id = StreamKey::from_bytes("final-all");
    let subscription = Subscription::new(&store);
    let mut cursor = pin!(subscription.subscribe_all(None).unwrap());
    park(&mut cursor).await;
    commit_without_hint(&pool, &id).await;
    let next = timeout(Duration::from_secs(3), cursor.next()).await;
    let step = next
        .expect("final missed hint must recover")
        .unwrap()
        .unwrap();
    let (_, origin, event) = step.event().expect("expected the committed event");
    assert_eq!(origin, id);
    assert_eq!(event.version(), Version::INITIAL);
    assert_eq!(event.payload(), b"payload");
}

#[tokio::test]
async fn all_recovers_after_an_unrelated_transaction_releases_the_watermark() {
    let Some((store, pool)) = fresh().await else {
        return;
    };
    timeout(Duration::from_secs(3), async {
        loop {
            let ready: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT FROM pg_stat_activity \
                 WHERE datname = current_database() AND state = 'idle' \
                 AND query LIKE 'LISTEN %mnesis_events%')",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            if ready {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("listener must subscribe");

    let mut older = pool.begin().await.unwrap();
    sqlx::query("SELECT pg_current_xact_id()")
        .execute(&mut *older)
        .await
        .unwrap();
    let subscription = Subscription::new(&store);
    let mut cursor = pin!(subscription.subscribe_all(None).unwrap());
    park(&mut cursor).await;

    let id = StreamKey::from_bytes("watermark");
    let registration = store.raw().register(None).unwrap();
    let hint = registration.arm();
    let event = pending_envelope(Version::INITIAL)
        .event_type("Final")
        .payload(b"payload".as_slice())
        .build()
        .unwrap();
    let position = store
        .raw()
        .append(&id, None, PendingBatch::from_parts(&event, &[]))
        .await
        .unwrap();
    timeout(Duration::from_secs(3), hint)
        .await
        .expect("the actual hint must arrive");
    assert!(
        timeout(Duration::from_millis(100), cursor.next())
            .await
            .is_err()
    );

    // No event and no NOTIFY come from this unrelated transaction's rollback.
    older.rollback().await.unwrap();
    let step = timeout(Duration::from_secs(3), cursor.next())
        .await
        .expect("watermark release must recover")
        .unwrap()
        .unwrap();
    let (seen, origin, persisted) = step.event().expect("expected the committed event");
    assert_eq!(seen, position);
    assert_eq!(origin, id);
    assert_eq!(persisted.version(), Version::INITIAL);
}
