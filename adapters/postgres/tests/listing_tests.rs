//! Event-only finite listing, bounded transfer and released pool leases.
#![cfg(feature = "export")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "native listing assertions"
)]

use std::num::NonZeroU32;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use futures::{StreamExt, TryStreamExt};
use mnesis::Version;
use mnesis_postgres::{Limits, PostgresError, PostgresStore};
use mnesis_store::export::StreamLister;
use mnesis_store::{BatchSize, PendingBatch, RawEventStore, StreamKey, pending_envelope};
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use tokio::sync::{Barrier, oneshot};
use tokio::task::yield_now;
use tokio::time::{Duration, timeout};

#[path = "support/memory.rs"]
mod memory;
mod support;

async fn fixture(limits: Limits) -> Option<(PostgresStore, PgPool)> {
    let url = std::env::var("DATABASE_URL").ok()?;
    let name = format!("mnesis-listing-{}", std::process::id());
    let options = PgConnectOptions::from_str(&url)
        .unwrap()
        .application_name(&name);
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect_with(options)
        .await
        .unwrap();
    sqlx::raw_sql("DROP SCHEMA IF EXISTS mnesis CASCADE; DROP VIEW IF EXISTS public.events")
        .execute(&pool)
        .await
        .unwrap();
    PostgresStore::migrate(&pool).await.unwrap();
    let store = PostgresStore::from_pool_with_limits(pool.clone(), limits)
        .await
        .unwrap();
    timeout(Duration::from_secs(3), async {
        loop {
            let count: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE application_name = $1 AND state = 'idle' AND query LIKE 'LISTEN %mnesis_events%'")
                .bind(&name).fetch_one(&pool).await.unwrap();
            if count == 1 { break; }
            yield_now().await;
        }
    }).await.expect("the sole shared listener must hold one of the two leases");
    Some((store, pool))
}

async fn append(store: &PostgresStore, id: &StreamKey, previous: Option<Version>) {
    let version = previous.map_or(Version::INITIAL, |value| value.next().unwrap());
    let event = pending_envelope(version)
        .event_type("E")
        .payload(b"p".as_slice())
        .build()
        .unwrap();
    store
        .append(id, previous, PendingBatch::of(&event))
        .await
        .unwrap();
}

async fn ids(store: &PostgresStore) -> Vec<Vec<u8>> {
    let rows: Vec<_> = store
        .list_streams()
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    let mut bytes: Vec<_> = rows.into_iter().map(|id| id.as_bytes().to_vec()).collect();
    bytes.sort();
    bytes
}

async fn shadowed(statement: &'static str) {
    let Some((store, pool)) = fixture(Limits::default()).await else {
        return;
    };
    let id = StreamKey::from_slice(b"canonical-listing");
    append(&store, &id, None).await;
    sqlx::raw_sql("DROP SCHEMA IF EXISTS listing_shadow CASCADE; CREATE SCHEMA listing_shadow")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(statement).execute(&pool).await.unwrap();
    let runtime = PgPoolOptions::new()
        .max_connections(4)
        .connect_with(
            pool.connect_options()
                .as_ref()
                .clone()
                .options([("search_path", "listing_shadow,pg_catalog")]),
        )
        .await
        .unwrap();
    let reader = PostgresStore::from_pool(runtime.clone()).await.unwrap();
    let listed = ids(&reader).await;
    drop(reader);
    runtime.close().await;
    sqlx::raw_sql("DROP SCHEMA listing_shadow CASCADE")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(listed, [id.as_bytes().to_vec()]);
}

#[tokio::test]
async fn an_untrusted_search_path_cannot_forge_transaction_visibility() {
    shadowed("CREATE FUNCTION listing_shadow.pg_visible_in_snapshot(pg_catalog.xid8, pg_catalog.pg_snapshot) RETURNS boolean LANGUAGE SQL AS $$ SELECT false $$").await;
}

#[tokio::test]
async fn an_untrusted_search_path_cannot_replace_the_captured_snapshot() {
    shadowed("CREATE FUNCTION listing_shadow.pg_current_snapshot() RETURNS pg_catalog.pg_snapshot LANGUAGE SQL AS $$ SELECT '1:1:'::pg_catalog.pg_snapshot $$").await;
}

#[tokio::test]
async fn empty_and_saved_state_keys_do_not_create_event_streams() {
    let Some((store, pool)) = fixture(Limits::default()).await else {
        return;
    };
    let mut empty = store.list_streams().await.unwrap();
    assert!(empty.next().await.is_none());
    assert!(empty.next().await.is_none());
    let cache = support::key(&pool, b"cache-only").await;
    let checkpoint = support::key(&pool, b"checkpoint-only").await;
    sqlx::query("INSERT INTO mnesis.snapshots (key, schema_version, version, state) VALUES ($1, 1, 1, 'cache'::bytea)")
        .bind(cache).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO mnesis.checkpoints (key, revision, schema_version, txid, global_seq, state, history) VALUES ($1, 1, 1, 0, 0, 'checkpoint'::bytea, (SELECT value FROM mnesis.history))")
        .bind(checkpoint).execute(&pool).await.unwrap();
    assert_eq!(ids(&store).await, Vec::<Vec<u8>>::new());
    let id = StreamKey::from_slice(b"events");
    append(&store, &id, None).await;
    append(&store, &id, Some(Version::INITIAL)).await;
    assert_eq!(ids(&store).await, [b"events".to_vec()]);
}

#[tokio::test]
async fn binary_ids_list_once_across_pages_and_reopen() {
    let Some((store, pool)) = fixture(Limits {
        rows: BatchSize::new(1).unwrap(),
        ..Limits::default()
    })
    .await
    else {
        return;
    };
    let mut expected = vec![
        Vec::new(),
        b"\0/a".to_vec(),
        b"a\0b".to_vec(),
        vec![b'z'; 8192],
    ];
    for raw in &expected {
        let id = StreamKey::from_slice(raw);
        append(&store, &id, None).await;
        append(&store, &id, Some(Version::INITIAL)).await;
    }
    expected.sort();
    assert_eq!(ids(&store).await, expected);
    drop(store);
    pool.close().await;
    let reopened = PostgresStore::connect(&std::env::var("DATABASE_URL").unwrap())
        .await
        .unwrap();
    assert_eq!(ids(&reopened).await, expected);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn listing_freezes_event_presence_before_first_poll_and_releases_pool_leases() {
    let Some((store, pool)) = fixture(Limits {
        rows: BatchSize::new(1).unwrap(),
        ..Limits::default()
    })
    .await
    else {
        return;
    };
    let late = support::key(&pool, b"late").await;
    let original = StreamKey::from_slice(b"original");
    append(&store, &original, None).await;
    let writer = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .unwrap();
    let gate = Arc::new(Barrier::new(2));
    let release = Arc::clone(&gate);
    let (staged, ready) = oneshot::channel();
    let writing = tokio::spawn(async move {
        let mut tx = writer.begin().await.unwrap();
        sqlx::query("INSERT INTO mnesis.events (stream_id, version, event_type, schema_version, payload) VALUES ($1, 1, 'Late', 1, 'late'::bytea)")
            .bind(late).execute(&mut *tx).await.unwrap();
        staged.send(()).unwrap();
        release.wait().await;
        tx.commit().await.unwrap();
        writer.close().await;
    });
    ready.await.unwrap();
    let mut listing = store.list_streams().await.unwrap();
    gate.wait().await;
    writing.await.unwrap();
    let idle = timeout(Duration::from_secs(3), pool.acquire())
        .await
        .expect("an idle listing must release the sole data connection")
        .unwrap();
    drop(idle);
    append(&store, &StreamKey::from_slice(b"later"), None).await;
    assert_eq!(listing.next().await.unwrap().unwrap(), original);
    let buffered = timeout(Duration::from_secs(3), pool.acquire())
        .await
        .expect("a buffered listing must release its page connection")
        .unwrap();
    drop(buffered);
    assert!(listing.next().await.is_none());
    assert!(listing.next().await.is_none());
    assert_eq!(
        ids(&store).await,
        [b"late".to_vec(), b"later".to_vec(), b"original".to_vec()]
    );
}

#[tokio::test]
async fn first_id_does_not_load_the_whole_catalog() {
    let Some((store, pool)) = fixture(Limits::default()).await else {
        return;
    };
    assert!(store.list_streams().await.unwrap().next().await.is_none());
    let keys = sqlx::query("INSERT INTO mnesis.keys (value) SELECT convert_to('stream-' || lpad(n::text, 5, '0'), 'UTF8') FROM generate_series(1, 20000) n")
        .execute(&pool).await.unwrap();
    let events = sqlx::query("INSERT INTO mnesis.events (stream_id, version, event_type, schema_version, payload) SELECT number, 1, 'Memory', 1, 'p'::bytea FROM mnesis.keys")
        .execute(&pool).await.unwrap();
    assert_eq!(keys.rows_affected(), 20000);
    assert_eq!(events.rows_affected(), 20000);
    let before = memory::TOTAL.load(Ordering::Relaxed);
    let mut listing = store.list_streams().await.unwrap();
    let first = listing.next().await.unwrap().unwrap();
    let allocated = memory::TOTAL
        .load(Ordering::Relaxed)
        .checked_sub(before)
        .unwrap();
    assert!(
        allocated < 262_144,
        "first listing item allocated {allocated} bytes"
    );
    let present: i64 = sqlx::query_scalar("SELECT count(*) FROM mnesis.keys k WHERE k.value = $1 AND EXISTS (SELECT FROM mnesis.events WHERE stream_id = k.number)")
        .bind(first.as_bytes()).fetch_one(&pool).await.unwrap();
    assert_eq!(present, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unrelated_held_transaction_does_not_hide_committed_streams() {
    let Some((store, pool)) = fixture(Limits::default()).await else {
        return;
    };
    let other = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .unwrap();
    let gate = Arc::new(Barrier::new(2));
    let release = Arc::clone(&gate);
    let (started, ready) = oneshot::channel();
    let holding = tokio::spawn(async move {
        let mut tx = other.begin().await.unwrap();
        let assigned: String = sqlx::query_scalar("SELECT pg_current_xact_id()::text")
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        started.send(assigned).unwrap();
        release.wait().await;
        tx.rollback().await.unwrap();
        other.close().await;
    });
    let held = ready.await.unwrap().parse::<u64>().unwrap();
    let id = StreamKey::from_slice(b"committed-after-held-transaction");
    append(&store, &id, None).await;
    let committed: String = sqlx::query_scalar("SELECT txid::text FROM mnesis.events")
        .fetch_one(&pool)
        .await
        .unwrap();
    let listed = ids(&store).await;
    gate.wait().await;
    holding.await.unwrap();
    assert!(committed.parse::<u64>().unwrap() > held);
    assert_eq!(listed, [id.as_bytes().to_vec()]);
}

#[tokio::test]
async fn byte_edges_and_oversized_ids_are_terminal_without_fetching_the_body() {
    let Some((store, pool)) = fixture(Limits::default()).await else {
        return;
    };
    let exact = StreamKey::from_slice(b"abcde");
    append(&store, &exact, None).await;
    let enormous = StreamKey::from_bytes(vec![7; 1_048_576]);
    append(&store, &enormous, None).await;
    drop(store);
    let limited = PostgresStore::from_pool_with_limits(
        pool,
        Limits {
            rows: BatchSize::new(1).unwrap(),
            bytes: NonZeroU32::new(37).unwrap(),
        },
    )
    .await
    .unwrap();
    let mut listing = limited.list_streams().await.unwrap();
    assert_eq!(listing.next().await.unwrap().unwrap(), exact);
    let before = memory::TOTAL.load(Ordering::Relaxed);
    let oversized = listing.next().await.unwrap();
    let allocated = memory::TOTAL
        .load(Ordering::Relaxed)
        .checked_sub(before)
        .unwrap();
    assert!(matches!(
        oversized,
        Err(PostgresError::RowLimit {
            actual: 1_048_608,
            limit: 37
        })
    ));
    assert!(
        allocated < 262_144,
        "oversized listing allocated {allocated} bytes"
    );
    assert!(listing.next().await.is_none());
    assert!(listing.next().await.is_none());
}
