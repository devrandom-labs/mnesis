//! Listener recovery, pool ownership and notification boundaries on real Postgres.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test assertions")]
#![allow(clippy::missing_panics_doc, reason = "test assertions")]

use std::pin::pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use futures::StreamExt;
use mnesis::Version;
use mnesis_postgres::{PostgresError, PostgresStore};
use mnesis_store::envelope::{PendingBatch, pending_envelope};
use mnesis_store::wake::{WakeRegistration, WakeSource};
use mnesis_store::{RawEventStore, Step, Store, StreamKey, Subscription};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{Error as SqlxError, PgPool};
use tokio::sync::watch;
use tokio::task::yield_now;
use tokio::time::{sleep, timeout};

async fn pool(name: &str, capacity: u32) -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    let options = PgConnectOptions::from_str(&url)
        .unwrap()
        .application_name(name);
    Some(
        PgPoolOptions::new()
            .max_connections(capacity)
            .connect_with(options)
            .await
            .unwrap(),
    )
}

async fn open(name: &str) -> Option<(PostgresStore, PgPool)> {
    let backing = pool(name, 4).await?;
    let store = PostgresStore::from_pool(backing.clone()).await.unwrap();
    sqlx::query("TRUNCATE events RESTART IDENTITY")
        .execute(&backing)
        .await
        .unwrap();
    Some((store, backing))
}

async fn listener(backing: &PgPool, name: &str, previous: Option<i32>) -> i32 {
    timeout(Duration::from_secs(3), async {
        loop {
            let candidate: Option<i32> = sqlx::query_scalar(
                "SELECT pid FROM pg_stat_activity WHERE application_name = $1 \
                 AND state = 'idle' AND query LIKE 'LISTEN %mnesis_events%' \
                 AND ($2::int IS NULL OR pid <> $2) LIMIT 1",
            )
            .bind(name)
            .bind(previous)
            .fetch_optional(backing)
            .await
            .unwrap();
            if let Some(pid) = candidate {
                return pid;
            }
            yield_now().await;
        }
    })
    .await
    .expect("listener must establish its subscription")
}

async fn insert(backing: &PgPool, id: &StreamKey) {
    sqlx::query(
        "INSERT INTO events (stream_id, version, event_type, schema_version, payload) \
         VALUES ($1, 1, 'Final', 1, 'payload'::bytea)",
    )
    .bind(id.as_bytes())
    .execute(backing)
    .await
    .unwrap();
}

#[derive(Clone, Copy)]
enum Availability {
    Ready,
    Unavailable,
}

fn connect(availability: Availability, failures: &AtomicUsize) -> Result<(), SqlxError> {
    match availability {
        Availability::Ready => Ok(()),
        Availability::Unavailable => {
            failures.fetch_add(1, Ordering::SeqCst);
            Err(SqlxError::PoolTimedOut)
        }
    }
}

fn options(phase: watch::Receiver<Availability>, failures: &Arc<AtomicUsize>) -> PgPoolOptions {
    let reconnect_phase = phase.clone();
    let reconnect_failures = Arc::clone(failures);
    let acquire_failures = Arc::clone(failures);
    PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_millis(100))
        .before_acquire(move |_, _| {
            let outcome = connect(*phase.borrow(), &acquire_failures).map(|()| true);
            Box::pin(async move { outcome })
        })
        .after_connect(move |_, _| {
            let outcome = connect(*reconnect_phase.borrow(), &reconnect_failures);
            Box::pin(async move { outcome })
        })
}

#[tokio::test]
async fn initial_listener_acquisition_failure_recovers_after_service_returns() {
    let Some((setup, control)) = open("mnesis-startup-control").await else {
        return;
    };
    drop(setup);
    let (availability, phase) = watch::channel(Availability::Ready);
    let failures = Arc::new(AtomicUsize::new(0));
    let connection = control
        .connect_options()
        .as_ref()
        .clone()
        .application_name("mnesis-startup");
    let backing = options(phase, &failures)
        .connect_with(connection)
        .await
        .unwrap();
    let store = PostgresStore::from_pool(backing.clone()).await.unwrap();
    // This current-thread runtime cannot poll the spawned listener before this
    // synchronous transition. Every acquisition path then fails until restored.
    availability.send(Availability::Unavailable).unwrap();
    timeout(Duration::from_secs(1), async {
        while failures.load(Ordering::SeqCst) == 0 {
            yield_now().await;
        }
    })
    .await
    .expect("the initial listener attempt must hit the injected failure");
    // The observed attempt's configured 100ms acquisition deadline must expire.
    sleep(Duration::from_millis(200)).await;
    let active: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_stat_activity WHERE application_name = 'mnesis-startup' \
         AND query LIKE 'LISTEN %mnesis_events%'",
    )
    .fetch_one(&control)
    .await
    .unwrap();
    assert_eq!(active, 0);
    availability.send(Availability::Ready).unwrap();
    listener(&control, "mnesis-startup", None).await;
    let registration = store.register(None).unwrap();
    let wake = registration.arm();
    sqlx::query("SELECT pg_notify('mnesis_events', '73746172747570')")
        .execute(&control)
        .await
        .unwrap();
    timeout(Duration::from_millis(500), wake)
        .await
        .expect("restored LISTEN must deliver before fallback");
}

#[tokio::test]
async fn listener_subscription_failure_recovers_after_the_acquired_backend_dies() {
    let Some((setup, control)) = open("mnesis-listen-control").await else {
        return;
    };
    drop(setup);
    let (availability, phase) = watch::channel(Availability::Ready);
    let failures = Arc::new(AtomicUsize::new(0));
    let attempts = Arc::clone(&failures);
    let options = control
        .connect_options()
        .as_ref()
        .clone()
        .application_name("mnesis-listen-failure");
    let backing = PgPoolOptions::new()
        .max_connections(2)
        .before_acquire(move |connection, _| {
            let status = *phase.borrow();
            let completed = Arc::clone(&attempts);
            Box::pin(async move {
                if matches!(status, Availability::Unavailable) {
                    let result = sqlx::query("SELECT pg_terminate_backend(pg_backend_pid())")
                        .execute(connection)
                        .await;
                    assert!(matches!(result, Err(SqlxError::Database(ref error))
                        if error.code().as_deref() == Some("57P01")));
                    completed.fetch_add(1, Ordering::SeqCst);
                }
                Ok(true)
            })
        })
        .connect_with(options)
        .await
        .unwrap();
    // Holding one slot forces the listener to acquire the schema connection
    // after its return to the pool, so before_acquire is the exact fault point.
    let held = backing.acquire().await.unwrap();
    let store = PostgresStore::from_pool(backing.clone()).await.unwrap();
    availability.send(Availability::Unavailable).unwrap();
    timeout(Duration::from_secs(1), async {
        while failures.load(Ordering::SeqCst) == 0 {
            yield_now().await;
        }
    })
    .await
    .expect("the acquired backend must be terminated before LISTEN");
    availability.send(Availability::Ready).unwrap();
    listener(&control, "mnesis-listen-failure", None).await;
    drop(held);
    let registration = store.register(None).unwrap();
    let wake = registration.arm();
    sqlx::query("SELECT pg_notify('mnesis_events', '6c697374656e')")
        .execute(&control)
        .await
        .unwrap();
    timeout(Duration::from_millis(500), wake)
        .await
        .expect("recovered LISTEN must deliver a hint");
}

#[tokio::test]
async fn a_pool_with_one_connection_is_rejected_before_it_can_starve_queries() {
    let Some(backing) = pool("mnesis-capacity", 1).await else {
        return;
    };
    let result = PostgresStore::from_pool(backing.clone()).await;
    assert!(matches!(
        result,
        Err(PostgresError::PoolCapacity { actual: 1 })
    ));
    assert_eq!(
        sqlx::query_scalar::<_, i32>("SELECT 1")
            .fetch_one(&backing)
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn listener_disconnect_and_a_final_unnotified_commit_recover() {
    let Some((source, backing)) = open("mnesis-reconnect").await else {
        return;
    };
    let previous = listener(&backing, "mnesis-reconnect", None).await;
    let store = Store::new(source);
    let id = StreamKey::from_bytes("reconnect");
    let subscription = Subscription::new(&store);
    let mut cursor = pin!(subscription.subscribe(&id, None).unwrap());
    assert!(matches!(cursor.next().await, Some(Ok(Step::CaughtUp))));
    assert!(
        timeout(Duration::from_millis(100), cursor.next())
            .await
            .is_err()
    );
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT pg_terminate_backend($1)")
            .bind(previous)
            .fetch_one(&backing)
            .await
            .unwrap()
    );
    insert(&backing, &id).await;
    let event = timeout(Duration::from_secs(3), cursor.next())
        .await
        .expect("final event must recover without another hint")
        .unwrap()
        .unwrap()
        .event()
        .unwrap();
    assert_eq!(event.version(), Version::INITIAL);
    assert_eq!(event.payload(), b"payload");
    let recovered = listener(&backing, "mnesis-reconnect", Some(previous)).await;
    assert_ne!(recovered, previous);
}

#[tokio::test]
async fn clones_share_one_listener_and_last_drop_preserves_the_callers_pool() {
    let Some(backing) = pool("mnesis-clones", 4).await else {
        return;
    };
    for _ in 0..8 {
        let source = PostgresStore::from_pool(backing.clone()).await.unwrap();
        let expected = listener(&backing, "mnesis-clones", None).await;
        let copies: Vec<_> = (0..128).map(|_| source.clone()).collect();
        let registrations: Vec<_> = (0..128)
            .map(|_| source.register(Some(b"shared")).unwrap())
            .collect();
        drop(source);
        assert_eq!(listener(&backing, "mnesis-clones", None).await, expected);
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity WHERE application_name = 'mnesis-clones' \
         AND query LIKE 'LISTEN %mnesis_events%'",
        )
        .fetch_one(&backing)
        .await
        .unwrap();
        assert_eq!(count, 1);
        drop(registrations);
        drop(copies);
        timeout(Duration::from_secs(3), async {
            loop {
                let remaining: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_stat_activity WHERE application_name = 'mnesis-clones' \
                 AND query LIKE 'LISTEN %mnesis_events%'",
            )
            .fetch_one(&backing)
            .await
            .unwrap();
                if remaining == 0 {
                    break;
                }
                yield_now().await;
            }
        })
        .await
        .expect("last clone must release LISTEN");
        assert_eq!(
            sqlx::query_scalar::<_, i32>("SELECT 1")
                .fetch_one(&backing)
                .await
                .unwrap(),
            1
        );
    }
}

#[tokio::test]
async fn terminal_receive_error_after_failed_reconnection_is_retried() {
    let Some((setup, control)) = open("mnesis-receive-control").await else {
        return;
    };
    drop(setup);
    let (availability, phase) = watch::channel(Availability::Ready);
    let failures = Arc::new(AtomicUsize::new(0));
    let connection = control
        .connect_options()
        .as_ref()
        .clone()
        .application_name("mnesis-receive");
    let backing = options(phase, &failures)
        .connect_with(connection)
        .await
        .unwrap();
    let store = PostgresStore::from_pool(backing).await.unwrap();
    let previous = listener(&control, "mnesis-receive", None).await;
    availability.send(Availability::Unavailable).unwrap();
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT pg_terminate_backend($1)")
            .bind(previous)
            .fetch_one(&control)
            .await
            .unwrap()
    );
    timeout(Duration::from_secs(1), async {
        while failures.load(Ordering::SeqCst) == 0 {
            yield_now().await;
        }
    })
    .await
    .expect("the active listener must attempt reconnecting");
    sleep(Duration::from_millis(200)).await;
    availability.send(Availability::Ready).unwrap();
    listener(&control, "mnesis-receive", Some(previous)).await;
    let registration = store.register(None).unwrap();
    let wake = registration.arm();
    sqlx::query("SELECT pg_notify('mnesis_events', '72656376')")
        .execute(&control)
        .await
        .unwrap();
    timeout(Duration::from_millis(500), wake)
        .await
        .expect("recovered receiver must deliver before fallback");
}

#[tokio::test]
async fn wake_between_arm_and_first_poll_is_preserved() {
    let Some((source, _backing)) = open("mnesis-arm").await else {
        return;
    };
    for target in [Some(b"armed".as_slice()), None] {
        let registration = source.register(target).unwrap();
        let armed = registration.arm();
        source.wake(b"armed");
        timeout(Duration::from_millis(100), armed)
            .await
            .expect("captured generation must beat fallback");
    }
}

#[tokio::test]
async fn closing_the_callers_pool_completes_with_a_live_subscription() {
    let Some((source, backing)) = open("mnesis-close").await else {
        return;
    };
    listener(&backing, "mnesis-close", None).await;
    let store = Store::new(source);
    let subscription = Subscription::new(&store);
    let mut cursor = pin!(subscription.subscribe_all(None).unwrap());
    assert!(matches!(cursor.next().await, Some(Ok(Step::CaughtUp))));
    timeout(Duration::from_secs(1), backing.close())
        .await
        .expect("pool close must release its listener");
    let next = timeout(Duration::from_secs(3), cursor.next())
        .await
        .expect("closed pool must surface");
    assert!(matches!(
        next,
        Some(Err(PostgresError::Sqlx(SqlxError::PoolClosed)))
    ));
}

#[tokio::test]
async fn overlong_binary_notification_payload_does_not_reject_a_committed_append() {
    let Some((source, backing)) = open("mnesis-long-id").await else {
        return;
    };
    let store = Store::new(source);
    let id = StreamKey::from_bytes(vec![0xab; 5000]);
    let subscription = Subscription::new(&store);
    let mut cursor = pin!(subscription.subscribe(&id, None).unwrap());
    assert!(matches!(cursor.next().await, Some(Ok(Step::CaughtUp))));
    assert!(
        timeout(Duration::from_millis(100), cursor.next())
            .await
            .is_err()
    );
    let pending = pending_envelope(Version::INITIAL)
        .event_type("LongId")
        .payload(b"payload".as_slice())
        .build()
        .unwrap();
    store
        .append(&id, None, PendingBatch::from_parts(&pending, &[]))
        .await
        .unwrap();
    let event = timeout(Duration::from_secs(3), cursor.next())
        .await
        .expect("overlong hint must fall back to a durable scan")
        .unwrap()
        .unwrap()
        .event()
        .unwrap();
    assert_eq!(event.version(), Version::INITIAL);
    assert_eq!(event.payload(), b"payload");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM events")
            .fetch_one(&backing)
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn malformed_duplicate_and_unrelated_hints_do_not_forge_events_or_markers() {
    let Some((source, backing)) = open("mnesis-routing").await else {
        return;
    };
    listener(&backing, "mnesis-routing", None).await;
    let store = Store::new(source);
    let id = StreamKey::from_bytes("route-a");
    let subscription = Subscription::new(&store);
    let mut cursor = pin!(subscription.subscribe(&id, None).unwrap());
    assert!(matches!(cursor.next().await, Some(Ok(Step::CaughtUp))));
    for hint in ["GG", "0", "6f74686572", "726f7574652d61", "726f7574652d61"] {
        sqlx::query("SELECT pg_notify('mnesis_events', $1)")
            .bind(hint)
            .execute(&backing)
            .await
            .unwrap();
    }
    assert!(
        timeout(Duration::from_millis(100), cursor.next())
            .await
            .is_err()
    );
    insert(&backing, &id).await;
    let event = timeout(Duration::from_secs(3), cursor.next())
        .await
        .expect("routing noise must not lose the final event")
        .unwrap()
        .unwrap()
        .event()
        .unwrap();
    assert_eq!(event.version(), Version::INITIAL);
    assert_eq!(event.payload(), b"payload");
    assert!(
        timeout(Duration::from_millis(100), cursor.next())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn all_rescans_after_a_long_held_snapshot_commits_without_a_hint() {
    let Some((source, backing)) = open("mnesis-snapshot").await else {
        return;
    };
    let mut older = backing.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *older)
        .await
        .unwrap();
    sqlx::query("SELECT pg_current_xact_id(), pg_current_snapshot()")
        .execute(&mut *older)
        .await
        .unwrap();
    let store = Store::new(source);
    let subscription = Subscription::new(&store);
    let mut cursor = pin!(subscription.subscribe_all(None).unwrap());
    assert!(matches!(cursor.next().await, Some(Ok(Step::CaughtUp))));
    let id = StreamKey::from_bytes("snapshot");
    insert(&backing, &id).await;
    assert!(
        timeout(Duration::from_millis(100), cursor.next())
            .await
            .is_err()
    );
    older.commit().await.unwrap();
    let (_, origin, event) = timeout(Duration::from_secs(3), cursor.next())
        .await
        .expect("settled snapshot must permit progress without another write")
        .unwrap()
        .unwrap()
        .event()
        .unwrap();
    assert_eq!(origin, id);
    assert_eq!(event.version(), Version::INITIAL);
    assert_eq!(event.payload(), b"payload");
}
