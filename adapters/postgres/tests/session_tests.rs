//! A consistent view spans all streams and releases native MVCC resources.
#![cfg(feature = "export")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "native export assertions"
)]

use std::str::FromStr;
#[cfg(feature = "import")]
use std::sync::Arc;

#[cfg(feature = "import")]
use futures::TryStreamExt;
use futures::{FutureExt, StreamExt};
use mnesis::Version;
use mnesis_postgres::PostgresStore;
#[cfg(feature = "import")]
use mnesis_store::Store;
use mnesis_store::export::{ConsistentExporter, ExportError, ExportSession};
#[cfg(feature = "import")]
use mnesis_store::import::{AtomicAppend, PlannedAppend};
use mnesis_store::{PendingBatch, PendingEnvelope, RawEventStore, StreamKey, pending_envelope};
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
#[cfg(feature = "import")]
use tokio::sync::Barrier;
use tokio::task::yield_now;
use tokio::time::{Duration, sleep, timeout};

fn application() -> String {
    format!("mnesis-view-{}", std::process::id())
}

async fn fixture() -> Option<(PostgresStore, PgPool)> {
    let url = std::env::var("DATABASE_URL").ok()?;
    let options = PgConnectOptions::from_str(&url)
        .unwrap()
        .application_name(&application());
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .test_before_acquire(false)
        .connect_with(options)
        .await
        .unwrap();
    sqlx::raw_sql("DROP SCHEMA IF EXISTS mnesis CASCADE; DROP VIEW IF EXISTS public.events")
        .execute(&pool)
        .await
        .unwrap();
    PostgresStore::migrate(&pool).await.unwrap();
    let store = PostgresStore::from_pool(pool.clone()).await.unwrap();
    timeout(Duration::from_secs(3), async {
        loop {
            let count: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE application_name = $1 AND state = 'idle' AND query LIKE 'LISTEN %mnesis_events%'")
                .bind(application()).fetch_one(&pool).await.unwrap();
            if count == 1 { break; }
            yield_now().await;
        }
    }).await.expect("the listener must own its connection before priming the next lease");
    // One deterministic poll can acquire without a network health probe before
    // spawning the owner, so canceled-opening tests reach actual view creation.
    timeout(Duration::from_secs(3), async {
        while pool.num_idle() == 0 {
            yield_now().await;
        }
    })
    .await
    .unwrap();
    Some((store, pool))
}

fn event(version: Version, payload: &'static [u8]) -> PendingEnvelope {
    pending_envelope(version)
        .event_type("E")
        .payload(payload)
        .build()
        .unwrap()
}

async fn append(store: &PostgresStore, id: &StreamKey, previous: Option<Version>) {
    let version = previous.map_or(Version::INITIAL, |value| value.next().unwrap());
    let pending = event(version, b"old");
    store
        .append(id, previous, PendingBatch::of(&pending))
        .await
        .unwrap();
}

async fn native_view(pool: &PgPool) -> i32 {
    timeout(Duration::from_secs(3), async {
        loop {
            let found: Option<i32> = sqlx::query_scalar("SELECT pid FROM pg_stat_activity WHERE application_name = $1 AND state = 'idle in transaction' AND backend_xmin IS NOT NULL LIMIT 1")
                .bind(application()).fetch_optional(pool).await.unwrap();
            if let Some(pid) = found { return pid; }
            yield_now().await;
        }
    }).await.expect("the actual database view must be pinned")
}

async fn released(pool: &PgPool, pid: i32) {
    timeout(Duration::from_secs(5), async {
        loop {
            let pinned: bool = sqlx::query_scalar("SELECT EXISTS (SELECT FROM pg_stat_activity WHERE pid = $1 AND backend_xmin IS NOT NULL)")
                .bind(pid).fetch_one(pool).await.unwrap();
            if !pinned { break; }
            sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("cleanup must release the native view without a caller poll");
}

#[cfg(feature = "import")]
#[tokio::test]
async fn one_view_includes_no_part_of_a_later_atomic_commit_even_before_its_first_read() {
    let Some((raw, _pool)) = fixture().await else {
        return;
    };
    let first = StreamKey::from_slice(b"a");
    let second = StreamKey::from_slice(b"b");
    let created = StreamKey::from_slice(b"c\0");
    append(&raw, &first, None).await;
    append(&raw, &second, None).await;
    let store = Store::new(raw.clone());
    let session = store
        .open_export_session(Duration::from_secs(60))
        .await
        .unwrap();
    let gate = Arc::new(Barrier::new(2));
    let enter = Arc::clone(&gate);
    let writes = [
        (first.clone(), Some(Version::INITIAL), b"new-a"),
        (second.clone(), Some(Version::INITIAL), b"new-b"),
        (created.clone(), None, b"new-c"),
    ]
    .map(|(target, expected_version, payload)| PlannedAppend {
        target,
        expected_version,
        head: event(
            expected_version.map_or(Version::INITIAL, |value| value.next().unwrap()),
            payload,
        ),
        tail: Vec::new(),
    });
    let writing = tokio::spawn(async move {
        enter.wait().await;
        raw.atomic_append_many(&writes).await.unwrap()
    });
    gate.wait().await;
    assert!(writing.await.unwrap().is_some());
    let listed: Vec<_> = session
        .list_streams()
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    let mut listed_bytes: Vec<_> = listed.iter().map(StreamKey::as_bytes).collect();
    listed_bytes.sort();
    assert_eq!(listed_bytes, [first.as_bytes(), second.as_bytes()]);
    for id in [&first, &second] {
        let rows: Vec<_> = session
            .export_stream(id, Version::INITIAL)
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].version(), Version::INITIAL);
        assert_eq!(rows[0].payload(), b"old");
        let current: Vec<_> = store
            .read_stream(id, Version::INITIAL)
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(current.len(), 2);
        assert_eq!(current[1].version(), Version::new(2).unwrap());
    }
    assert!(
        session
            .export_stream(&created, Version::INITIAL)
            .await
            .unwrap()
            .try_collect::<Vec<_>>()
            .await
            .unwrap()
            .is_empty()
    );
    session.close().await.unwrap();
}

#[tokio::test]
async fn idle_expiration_releases_a_view_with_buffered_and_unpolled_cursors() {
    let Some((store, pool)) = fixture().await else {
        return;
    };
    let id = StreamKey::from_slice(b"buffered");
    append(&store, &id, None).await;
    append(&store, &id, Some(Version::INITIAL)).await;
    let session = store
        .open_export_session(Duration::from_secs(1))
        .await
        .unwrap();
    let mut buffered = Box::pin(session.export_stream(&id, Version::INITIAL).await.unwrap());
    let mut unread = Box::pin(session.list_streams().await.unwrap());
    let owned = buffered.next().await.unwrap().unwrap();
    let pid = native_view(&pool).await;
    // Only native observation follows; neither cursor nor the session is polled.
    released(&pool, pid).await;
    assert!(matches!(
        buffered.next().await,
        Some(Err(ExportError::Expired))
    ));
    assert!(buffered.next().await.is_none());
    assert!(matches!(
        unread.next().await,
        Some(Err(ExportError::Expired))
    ));
    assert!(unread.next().await.is_none());
    assert!(matches!(
        session.list_streams().await,
        Err(ExportError::Expired)
    ));
    drop(buffered);
    drop(unread);
    session.close().await.unwrap();
    assert_eq!(owned.version(), Version::INITIAL);
    assert_eq!(owned.payload(), b"old");
}

#[tokio::test]
async fn canceling_open_after_native_view_creation_releases_the_unreceived_session() {
    let Some((store, pool)) = fixture().await else {
        return;
    };
    let mut opening = Box::pin(store.open_export_session(Duration::from_secs(60)));
    assert!(opening.as_mut().now_or_never().is_none());
    let pid = native_view(&pool).await;
    drop(opening);
    released(&pool, pid).await;
    store
        .open_export_session(Duration::from_secs(60))
        .await
        .unwrap()
        .close()
        .await
        .unwrap();
}

#[tokio::test]
async fn invalid_lifetimes_reject_before_a_closed_pool_is_touched() {
    let Some((store, pool)) = fixture().await else {
        return;
    };
    pool.close().await;
    for lifetime in [Duration::ZERO, Duration::MAX] {
        assert!(matches!(store.open_export_session(lifetime).await,
            Err(ExportError::InvalidLifetime { lifetime: actual }) if actual == lifetime));
    }
}

#[tokio::test]
async fn explicit_close_and_drop_release_native_views_and_allow_reopen() {
    let Some((store, pool)) = fixture().await else {
        return;
    };
    let session = store
        .open_export_session(Duration::from_secs(60))
        .await
        .unwrap();
    let pid = native_view(&pool).await;
    session.close().await.unwrap();
    let pinned: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT FROM pg_stat_activity WHERE pid = $1 AND backend_xmin IS NOT NULL)",
    )
    .bind(pid)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!pinned, "successful close must wait for native cleanup");
    let another = store
        .open_export_session(Duration::from_secs(60))
        .await
        .unwrap();
    let dropped = native_view(&pool).await;
    drop(another);
    released(&pool, dropped).await;
    store
        .open_export_session(Duration::from_secs(60))
        .await
        .unwrap()
        .close()
        .await
        .unwrap();
}

#[tokio::test]
async fn expiry_releases_a_native_view_while_an_abandoned_read_is_blocked() {
    let Some((store, pool)) = fixture().await else {
        return;
    };
    let id = StreamKey::from_slice(b"blocked");
    append(&store, &id, None).await;
    let session = store
        .open_export_session(Duration::from_secs(1))
        .await
        .unwrap();
    let mut cursor = Box::pin(session.export_stream(&id, Version::INITIAL).await.unwrap());
    let pid = native_view(&pool).await;
    let mut lock = pool.begin().await.unwrap();
    sqlx::query("LOCK TABLE mnesis.events IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *lock)
        .await
        .unwrap();
    let mut reading = Box::pin(cursor.next());
    assert!(reading.as_mut().now_or_never().is_none());
    let blocked = timeout(Duration::from_secs(3), async {
        loop {
            let waiting: bool = sqlx::query_scalar("SELECT EXISTS (SELECT FROM pg_stat_activity WHERE pid = $1 AND wait_event_type = 'Lock')")
                .bind(pid).fetch_one(&pool).await.unwrap();
            if waiting { break; }
            yield_now().await;
        }
    }).await;
    drop(reading);
    let cleaned = timeout(Duration::from_secs(5), async {
        loop {
            let pinned: bool = sqlx::query_scalar("SELECT EXISTS (SELECT FROM pg_stat_activity WHERE pid = $1 AND backend_xmin IS NOT NULL)")
                .bind(pid).fetch_one(&pool).await.unwrap();
            if !pinned { break; }
            sleep(Duration::from_millis(10)).await;
        }
    }).await;
    // Release the independent blocker even when the adapter cleanup failed.
    lock.rollback().await.unwrap();
    blocked.expect("the actual read must wait on the native table lock");
    cleaned.expect("expiry must release a view while SQL is blocked, without caller polls");
    assert!(matches!(
        cursor.next().await,
        Some(Err(ExportError::Expired))
    ));
    assert!(cursor.next().await.is_none());
    drop(cursor);
    session.close().await.unwrap();
}

#[tokio::test]
async fn closing_the_callers_pool_releases_an_idle_session_before_its_lifetime() {
    let Some((store, pool)) = fixture().await else {
        return;
    };
    let session = store
        .open_export_session(Duration::from_secs(60))
        .await
        .unwrap();
    let _pid = native_view(&pool).await;
    let closed = timeout(Duration::from_secs(3), pool.close()).await;
    drop(session);
    // Cleanup before assertion also makes the original failing witness finite.
    timeout(Duration::from_secs(3), pool.close()).await.unwrap();
    closed.expect("caller-owned pool closure must stop idle session ownership");
}

#[tokio::test]
async fn dropping_a_long_lived_session_releases_its_blocked_query() {
    let Some((store, pool)) = fixture().await else {
        return;
    };
    let id = StreamKey::from_slice(b"drop-blocked");
    append(&store, &id, None).await;
    let session = store
        .open_export_session(Duration::from_secs(60))
        .await
        .unwrap();
    let mut cursor = Box::pin(session.export_stream(&id, Version::INITIAL).await.unwrap());
    let pid = native_view(&pool).await;
    let mut lock = pool.begin().await.unwrap();
    sqlx::query("LOCK TABLE mnesis.events IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *lock)
        .await
        .unwrap();
    assert!(cursor.next().now_or_never().is_none());
    let blocked = timeout(Duration::from_secs(3), async {
        loop {
            let waiting: bool = sqlx::query_scalar("SELECT EXISTS (SELECT FROM pg_stat_activity WHERE pid = $1 AND wait_event_type = 'Lock')")
                .bind(pid).fetch_one(&pool).await.unwrap();
            if waiting { break; }
            yield_now().await;
        }
    }).await;
    drop(cursor);
    drop(session);
    let cleaned = timeout(Duration::from_secs(5), async {
        loop {
            let pinned: bool = sqlx::query_scalar("SELECT EXISTS (SELECT FROM pg_stat_activity WHERE pid = $1 AND backend_xmin IS NOT NULL)")
                .bind(pid).fetch_one(&pool).await.unwrap();
            if !pinned { break; }
            sleep(Duration::from_millis(10)).await;
        }
    }).await;
    lock.rollback().await.unwrap();
    blocked.expect("the actual query must be blocked before Drop");
    cleaned.expect("Drop must release native blocked SQL before the60s lifetime");
}

#[tokio::test]
async fn sessions_use_the_callers_pool_bound_and_acquisition_error_without_extra_connections() {
    let Some((_store, observer)) = fixture().await else {
        return;
    };
    let name = format!("{}-bounded", application());
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_millis(100))
        .connect_with(
            observer
                .connect_options()
                .as_ref()
                .clone()
                .application_name(&name),
        )
        .await
        .unwrap();
    let store = PostgresStore::from_pool(pool.clone()).await.unwrap();
    timeout(Duration::from_secs(3), async {
        loop {
            let listeners: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE application_name = $1 AND state = 'idle' AND query LIKE 'LISTEN %mnesis_events%'")
                .bind(&name).fetch_one(&observer).await.unwrap();
            if listeners == 1 { break; }
            yield_now().await;
        }
    }).await.unwrap();
    let session = store
        .open_export_session(Duration::from_secs(60))
        .await
        .unwrap();
    assert!(matches!(
        store.open_export_session(Duration::from_secs(60)).await,
        Err(ExportError::Store(mnesis_postgres::PostgresError::Sqlx(
            sqlx::Error::PoolTimedOut
        )))
    ));
    assert_eq!(pool.size(), 2);
    let actual: i64 =
        sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE application_name = $1")
            .bind(&name)
            .fetch_one(&observer)
            .await
            .unwrap();
    assert_eq!(actual, 2);
    session.close().await.unwrap();
    timeout(
        Duration::from_secs(3),
        store.open_export_session(Duration::from_secs(60)),
    )
    .await
    .unwrap()
    .unwrap()
    .close()
    .await
    .unwrap();
    pool.close().await;
}

#[tokio::test]
async fn a_native_backend_failure_preserves_its_sqlstate_and_ends_the_cursor() {
    let Some((store, pool)) = fixture().await else {
        return;
    };
    let session = store
        .open_export_session(Duration::from_secs(60))
        .await
        .unwrap();
    let pid = native_view(&pool).await;
    let mut lock = pool.begin().await.unwrap();
    sqlx::query("LOCK TABLE mnesis.events IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *lock)
        .await
        .unwrap();
    let mut cursor = Box::pin(session.list_streams().await.unwrap());
    let mut next = Box::pin(cursor.next());
    assert!(next.as_mut().now_or_never().is_none());
    let blocked = timeout(Duration::from_secs(3), async {
        loop {
            let waiting: bool = sqlx::query_scalar("SELECT EXISTS (SELECT FROM pg_stat_activity WHERE pid = $1 AND wait_event_type = 'Lock')")
                .bind(pid).fetch_one(&pool).await.unwrap();
            if waiting { break; }
            yield_now().await;
        }
    }).await;
    let terminated: bool = sqlx::query_scalar("SELECT pg_catalog.pg_terminate_backend($1)")
        .bind(pid)
        .fetch_one(&pool)
        .await
        .unwrap();
    let pending = timeout(Duration::from_secs(3), next).await;
    lock.rollback().await.unwrap();
    blocked.expect("the actual query must be in flight before native termination");
    assert!(terminated);
    let failure = pending.unwrap().unwrap();
    assert!(cursor.next().await.is_none());
    drop(cursor);
    let closed = session.close().await;
    assert!(
        matches!(&failure,
        Err(ExportError::Store(mnesis_postgres::PostgresError::Sqlx(sqlx::Error::Database(source))))
        if source.code().as_deref() == Some("57P01")),
        "original native termination error required: {failure:?}"
    );
    assert!(closed.is_err());
}

#[tokio::test]
async fn session_pages_preserve_inclusive_versions_and_terminal_row_limits() {
    let Some((raw, pool)) = fixture().await else {
        return;
    };
    let id = StreamKey::from_slice(b"a");
    append(&raw, &id, None).await;
    append(&raw, &id, Some(Version::INITIAL)).await;
    let third = event(Version::new(3).unwrap(), &[b'x'; 40]);
    raw.append(
        &id,
        Some(Version::new(2).unwrap()),
        PendingBatch::of(&third),
    )
    .await
    .unwrap();
    let big = StreamKey::from_slice(&[b'z'; 40]);
    append(&raw, &big, None).await;
    let limited = PostgresStore::from_pool_with_limits(
        pool,
        mnesis_postgres::Limits {
            rows: mnesis_store::BatchSize::new(1).unwrap(),
            bytes: std::num::NonZeroU32::new(37).unwrap(),
        },
    )
    .await
    .unwrap();
    let session = limited
        .open_export_session(Duration::from_secs(60))
        .await
        .unwrap();
    let mut events = Box::pin(
        session
            .export_stream(&id, Version::new(2).unwrap())
            .await
            .unwrap(),
    );
    let second = events.next().await.unwrap().unwrap();
    assert_eq!(second.version(), Version::new(2).unwrap());
    assert_eq!(second.payload(), b"old");
    assert!(matches!(
        events.next().await,
        Some(Err(ExportError::Store(
            mnesis_postgres::PostgresError::RowLimit {
                actual: 74,
                limit: 37
            }
        )))
    ));
    assert!(events.next().await.is_none());
    assert!(events.next().await.is_none());
    let mut listing = Box::pin(session.list_streams().await.unwrap());
    assert_eq!(listing.next().await.unwrap().unwrap(), id);
    assert!(matches!(
        listing.next().await,
        Some(Err(ExportError::Store(
            mnesis_postgres::PostgresError::RowLimit {
                actual: 72,
                limit: 37
            }
        )))
    ));
    assert!(listing.next().await.is_none());
    assert!(listing.next().await.is_none());
    assert!(matches!(
        session.export_stream(&big, Version::INITIAL).await,
        Err(ExportError::Store(
            mnesis_postgres::PostgresError::RowLimit {
                actual: 72,
                limit: 37
            }
        ))
    ));
    drop(events);
    drop(listing);
    session.close().await.unwrap();
}
