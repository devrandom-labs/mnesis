//! Retained read memory must depend on a page, not on the entire history.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test assertions")]
#![allow(clippy::missing_panics_doc, reason = "test assertions")]
#![allow(clippy::arithmetic_side_effects, reason = "bounded test measurements")]

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::{Write, stderr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use futures::{StreamExt, TryStreamExt};
use mnesis::Version;
use mnesis_postgres::{Limits, PostgresError, PostgresStore};
use mnesis_store::batch::MAX_BATCH;
use mnesis_store::{BatchSize, RawEventStore, StreamKey};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tokio::time::timeout;

struct Allocator;

static LIVE: AtomicUsize = AtomicUsize::new(0);

// Every allocation and matching deallocation contributes its exact layout.
// No other state is published through the counter; relaxed operations suffice.
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the allocator receives System's unchanged layout contract.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        // SAFETY: pointer and layout are forwarded from the matching allocation.
        unsafe { System.dealloc(ptr, layout) };
    }
}

#[global_allocator]
static ALLOCATOR: Allocator = Allocator;

#[tokio::test]
async fn first_event_does_not_retain_the_whole_backlog() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .unwrap();
    let store = PostgresStore::from_pool(pool.clone()).await.unwrap();
    sqlx::query("TRUNCATE events RESTART IDENTITY")
        .execute(&pool)
        .await
        .unwrap();
    let id = StreamKey::from_bytes("read-memory");
    let rows =
        std::env::var("MNESIS_TEST_ROWS").map_or(4096, |value| value.parse::<i64>().unwrap());
    sqlx::query(
        "INSERT INTO events (stream_id, version, event_type, schema_version, payload) \
         SELECT $1, n, 'Memory', 1, decode(repeat('42', 4096), 'hex') \
         FROM generate_series(1, $2) AS n",
    )
    .bind(id.as_bytes())
    .bind(rows)
    .execute(&pool)
    .await
    .unwrap();

    // Warm connection setup and statement preparation before measuring rows.
    drop(
        store
            .read_stream(&id, Version::new(u64::try_from(rows).unwrap() + 1).unwrap())
            .await
            .unwrap(),
    );
    drop(store.read_all(None).await.unwrap());
    let budget = isize::try_from(BatchSize::DEFAULT.get() * 4096 * 4).unwrap();

    let before_stream = isize::try_from(LIVE.load(Ordering::Relaxed)).unwrap();
    let stream_started = Instant::now();
    let mut stream = store.read_stream(&id, Version::INITIAL).await.unwrap();
    assert_eq!(
        stream.next().await.unwrap().unwrap().version(),
        Version::INITIAL
    );
    let stream_bytes = isize::try_from(LIVE.load(Ordering::Relaxed)).unwrap() - before_stream;
    let stream_elapsed = stream_started.elapsed();
    std::hint::black_box(&stream);
    drop(stream);

    let before_all = isize::try_from(LIVE.load(Ordering::Relaxed)).unwrap();
    let all_started = Instant::now();
    let mut all = store.read_all(None).await.unwrap();
    let (_, origin, event) = all.next().await.unwrap().unwrap();
    assert_eq!(origin, id);
    assert_eq!(event.version(), Version::INITIAL);
    let all_bytes = isize::try_from(LIVE.load(Ordering::Relaxed)).unwrap() - before_all;
    let all_elapsed = all_started.elapsed();
    std::hint::black_box(&all);
    writeln!(
        stderr().lock(),
        "{rows} rows x 4096 bytes: stream={stream_bytes} bytes/{stream_elapsed:?}, all={all_bytes} bytes/{all_elapsed:?}; budget={budget}"
    ).unwrap();
    assert!(
        stream_bytes <= budget && all_bytes <= budget,
        "first-event retained memory exceeds four default pages: stream={stream_bytes}, all={all_bytes}, budget={budget}"
    );
}

async fn seed(pool: &PgPool, id: &StreamKey, count: usize) {
    sqlx::query("TRUNCATE events RESTART IDENTITY")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO events (stream_id, version, event_type, schema_version, payload) \
         SELECT $1, n, 'Page', 1, 'payload'::bytea FROM generate_series(1, $2) AS n",
    )
    .bind(id.as_bytes())
    .bind(i64::try_from(count).unwrap())
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn page_edges_preserve_versions_and_exclusive_global_resume() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await
        .unwrap();
    let id = StreamKey::from_bytes("pages");
    for size in [1, 2, BatchSize::DEFAULT.get(), MAX_BATCH] {
        let batch = BatchSize::new(size).unwrap();
        let store = PostgresStore::from_pool_with_limits(
            pool.clone(),
            Limits {
                rows: batch,
                ..Limits::default()
            },
        )
        .await
        .unwrap();
        for count in [0, 1, size - 1, size, size + 1] {
            seed(&pool, &id, count).await;
            let expected: Vec<_> = (1..=u64::try_from(count).unwrap()).collect();
            let events: Vec<_> = store
                .read_stream(&id, Version::INITIAL)
                .await
                .unwrap()
                .map(|row| row.unwrap().version().as_u64())
                .collect()
                .await;
            assert_eq!(events, expected, "page={size}, count={count}");
            let all: Vec<_> = store
                .read_all(None)
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap();
            assert_eq!(
                all.iter()
                    .map(|(_, _, event)| event.version().as_u64())
                    .collect::<Vec<_>>(),
                expected
            );
            assert!(
                all.iter()
                    .all(|(_, origin, event)| origin == &id && event.payload() == b"payload")
            );
            assert!(all.windows(2).all(|pair| pair[0].0 < pair[1].0));
            for (index, (position, _, _)) in all.iter().enumerate().step_by(size) {
                let resumed: Vec<_> = store
                    .read_all(Some(*position))
                    .await
                    .unwrap()
                    .map(|row| row.unwrap().2.version().as_u64())
                    .collect()
                    .await;
                assert_eq!(resumed, expected[index + 1..]);
            }
            let from_second: Vec<_> = store
                .read_stream(&id, Version::new(2).unwrap())
                .await
                .unwrap()
                .map(|row| row.unwrap().version().as_u64())
                .collect()
                .await;
            assert_eq!(
                from_second,
                expected
                    .iter()
                    .copied()
                    .filter(|version| *version >= 2)
                    .collect::<Vec<_>>()
            );
        }
    }
}

#[tokio::test]
async fn finite_scans_exclude_later_appends_and_release_the_connection_between_pages() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .unwrap();
    let store = PostgresStore::from_pool_with_limits(
        pool.clone(),
        Limits {
            rows: BatchSize::new(2).unwrap(),
            ..Limits::default()
        },
    )
    .await
    .unwrap();
    let id = StreamKey::from_bytes("finite");
    seed(&pool, &id, 3).await;
    let mut stream = store.read_stream(&id, Version::INITIAL).await.unwrap();
    let mut all = store.read_all(None).await.unwrap();
    assert_eq!(
        stream.next().await.unwrap().unwrap().version(),
        Version::INITIAL
    );
    assert_eq!(
        all.next().await.unwrap().unwrap().2.version(),
        Version::INITIAL
    );
    timeout(
        Duration::from_secs(1),
        sqlx::query(
            "INSERT INTO events (stream_id, version, event_type, schema_version, payload) \
         VALUES ($1, 4, 'Later', 1, 'later'::bytea)",
        )
        .bind(id.as_bytes())
        .execute(&pool),
    )
    .await
    .expect("idle cursors must not retain the pool's only query connection")
    .unwrap();
    assert_eq!(
        stream
            .map(|row| row.unwrap().version().as_u64())
            .collect::<Vec<_>>()
            .await,
        [2, 3]
    );
    assert_eq!(
        all.map(|row| row.unwrap().2.version().as_u64())
            .collect::<Vec<_>>()
            .await,
        [2, 3]
    );
    assert_eq!(store.read_all(None).await.unwrap().count().await, 4);
}

#[tokio::test]
async fn corruption_on_a_later_page_terminates_without_skipping() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .unwrap();
    let store = PostgresStore::from_pool_with_limits(
        pool.clone(),
        Limits {
            rows: BatchSize::new(2).unwrap(),
            ..Limits::default()
        },
    )
    .await
    .unwrap();
    let id = StreamKey::from_bytes("corrupt-page");
    seed(&pool, &id, 4).await;
    sqlx::query("UPDATE events SET schema_version = 0 WHERE version = 3")
        .execute(&pool)
        .await
        .unwrap();
    let mut stream = store.read_stream(&id, Version::INITIAL).await.unwrap();
    let mut all = store.read_all(None).await.unwrap();
    for version in 1..=2 {
        assert_eq!(
            stream.next().await.unwrap().unwrap().version().as_u64(),
            version
        );
        assert_eq!(
            all.next().await.unwrap().unwrap().2.version().as_u64(),
            version
        );
    }
    assert!(matches!(
        stream.next().await,
        Some(Err(PostgresError::CorruptRow { .. }))
    ));
    assert!(matches!(
        all.next().await,
        Some(Err(PostgresError::CorruptRow { .. }))
    ));
    assert!(stream.next().await.is_none());
    assert!(all.next().await.is_none());
}

#[tokio::test]
async fn large_rows_obey_the_default_byte_budget() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .unwrap();
    let store = PostgresStore::from_pool(pool.clone()).await.unwrap();
    let id = StreamKey::from_bytes("byte-memory");
    sqlx::query("TRUNCATE events RESTART IDENTITY")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO events (stream_id, version, event_type, schema_version, payload) \
         SELECT $1, n, 'Large', 1, decode(repeat('42', 1048576), 'hex') \
         FROM generate_series(1, 24) AS n",
    )
    .bind(id.as_bytes())
    .execute(&pool)
    .await
    .unwrap();
    drop(store.read_all(None).await.unwrap());
    let before_stream = LIVE.load(Ordering::Relaxed);
    let mut stream = store.read_stream(&id, Version::INITIAL).await.unwrap();
    let event = stream.next().await.unwrap().unwrap();
    assert_eq!(event.payload().len(), 1_048_576);
    drop(event);
    let stream_bytes = LIVE.load(Ordering::Relaxed).saturating_sub(before_stream);
    drop(stream);
    let before_all = LIVE.load(Ordering::Relaxed);
    let mut all = store.read_all(None).await.unwrap();
    let (_, _, all_event) = all.next().await.unwrap().unwrap();
    assert_eq!(all_event.payload().len(), 1_048_576);
    drop(all_event);
    let all_bytes = LIVE.load(Ordering::Relaxed).saturating_sub(before_all);
    std::hint::black_box(&all);
    writeln!(
        stderr().lock(),
        "1MiB rows: stream={stream_bytes}, all={all_bytes}"
    )
    .unwrap();
    assert!(
        stream_bytes < 20 * 1_048_576 && all_bytes < 20 * 1_048_576,
        "row-count limit permits excessive resident bytes: stream={stream_bytes}, all={all_bytes}"
    );
}

async fn with_bytes(limit: u32) -> Option<(PostgresStore, PgPool)> {
    let url = std::env::var("DATABASE_URL").ok()?;
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .unwrap();
    let limits = Limits {
        bytes: std::num::NonZeroU32::new(limit).unwrap(),
        ..Limits::default()
    };
    let store = PostgresStore::from_pool_with_limits(pool.clone(), limits)
        .await
        .unwrap();
    Some((store, pool))
}

#[tokio::test]
async fn byte_edges_count_binary_keys_utf8_and_optional_metadata() {
    use mnesis_store::PendingBatch;
    let Some((store, pool)) = with_bytes(100).await else {
        return;
    };
    sqlx::query("TRUNCATE events RESTART IDENTITY")
        .execute(&pool)
        .await
        .unwrap();
    let id = StreamKey::from_bytes([0, 255, 1, 254, 2].as_slice());
    // 32 scalar + 5 key + 2 UTF-8 + 10 payload + 51 metadata = 100.
    let head = byte_event(1, 10, Some(51));
    let position = store
        .append(
            &id,
            None,
            PendingBatch::new(std::slice::from_ref(&head)).unwrap(),
        )
        .await
        .unwrap();
    let rows: Vec<_> = store
        .read_all(None)
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, position);
    assert_eq!(rows[0].1, id);
    assert_eq!(rows[0].2.event_type(), "é");
    assert_eq!(rows[0].2.payload(), head.payload());
    assert_eq!(rows[0].2.metadata(), head.metadata());
    let second = byte_event(2, 10, Some(52));
    assert!(matches!(
        store
            .append(
                &id,
                Some(Version::INITIAL),
                PendingBatch::new(std::slice::from_ref(&second)).unwrap()
            )
            .await,
        Err(mnesis_store::AppendError::Store(PostgresError::RowLimit {
            actual: 101,
            limit: 100
        }))
    ));
    let no_metadata = byte_event(2, 61, None);
    let small_metadata = byte_event(3, 60, Some(1));
    let tail = [no_metadata, small_metadata];
    store
        .append(
            &id,
            Some(Version::INITIAL),
            PendingBatch::new(&tail).unwrap(),
        )
        .await
        .unwrap();
    let events: Vec<_> = store
        .read_stream(&id, Version::INITIAL)
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(events.len(), 3);
    assert_eq!(events[1].metadata(), None);
    assert_eq!(events[2].metadata(), Some([255].as_slice()));
}

#[tokio::test]
async fn oversized_tail_rejects_the_entire_append_before_writing() {
    use mnesis_store::PendingBatch;
    use mnesis_store::envelope::pending_envelope;
    let Some((store, pool)) = with_bytes(70).await else {
        return;
    };
    let id = StreamKey::from_bytes("byte-write");
    sqlx::query("TRUNCATE events RESTART IDENTITY")
        .execute(&pool)
        .await
        .unwrap();
    let events = [
        pending_envelope(Version::INITIAL)
            .event_type("Byte")
            .payload(vec![1; 10])
            .build()
            .unwrap(),
        pending_envelope(Version::new(2).unwrap())
            .event_type("Byte")
            .payload(vec![1; 25])
            .build()
            .unwrap(),
    ];
    assert!(matches!(
        store
            .append(&id, None, PendingBatch::new(&events).unwrap())
            .await,
        Err(mnesis_store::AppendError::Store(PostgresError::RowLimit {
            actual: 71,
            limit: 70
        }))
    ));
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM events")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        count, 0,
        "an invalid later row must leave no committed prefix"
    );
}

#[tokio::test]
async fn oversized_legacy_row_stops_without_fetching_its_body_or_skipping() {
    let Some((store, pool)) = with_bytes(70).await else {
        return;
    };
    let id = StreamKey::from_bytes("limit");
    seed(&pool, &id, 4).await;
    sqlx::query(
        "UPDATE events SET payload = decode(repeat('42', 8388608), 'hex') WHERE version = 3",
    )
    .execute(&pool)
    .await
    .unwrap();
    let mut stream = store.read_stream(&id, Version::INITIAL).await.unwrap();
    let mut all = store.read_all(None).await.unwrap();
    for expected in 1..=2 {
        assert_eq!(
            stream.next().await.unwrap().unwrap().version().as_u64(),
            expected
        );
        assert_eq!(
            all.next().await.unwrap().unwrap().2.version().as_u64(),
            expected
        );
    }
    let before = LIVE.load(Ordering::Relaxed);
    assert!(matches!(
        stream.next().await,
        Some(Err(PostgresError::RowLimit {
            actual: 8_388_649,
            limit: 70
        }))
    ));
    assert!(matches!(
        all.next().await,
        Some(Err(PostgresError::RowLimit {
            actual: 8_388_649,
            limit: 70
        }))
    ));
    let retained = LIVE.load(Ordering::Relaxed).saturating_sub(before);
    assert!(
        retained < 1_048_576,
        "an 8MiB rejected body was retained: {retained}"
    );
    assert!(stream.next().await.is_none());
    assert!(all.next().await.is_none());
    let larger = PostgresStore::from_pool_with_limits(
        pool,
        Limits {
            bytes: std::num::NonZeroU32::new(16_777_216).unwrap(),
            ..Limits::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        larger
            .read_stream(&id, Version::INITIAL)
            .await
            .unwrap()
            .try_collect::<Vec<_>>()
            .await
            .unwrap()
            .len(),
        4
    );
}

fn byte_event(
    version: u64,
    payload: usize,
    metadata: Option<usize>,
) -> mnesis_store::PendingEnvelope {
    let builder = mnesis_store::envelope::pending_envelope(Version::new(version).unwrap())
        .event_type("é")
        .payload(vec![42; payload]);
    match metadata {
        Some(length) => builder.metadata(vec![255; length]).build().unwrap(),
        None => builder.build().unwrap(),
    }
}

#[tokio::test]
async fn cancelling_reads_waiting_for_pool_capacity_releases_their_requests() {
    let Some((store, pool)) = with_bytes(1000).await else {
        return;
    };
    let id = StreamKey::from_bytes("cancel");
    seed(&pool, &id, 3).await;
    timeout(Duration::from_secs(3), async {
        loop {
            let listening: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() \
                 AND state = 'idle' AND query LIKE 'LISTEN %mnesis_events%'",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            if listening > 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let mut stream = store.read_stream(&id, Version::INITIAL).await.unwrap();
    let mut all = store.read_all(None).await.unwrap();
    let first = pool.acquire().await.unwrap();
    let second = pool.acquire().await.unwrap();
    let third = pool.acquire().await.unwrap();
    assert!(
        timeout(Duration::from_millis(20), stream.next())
            .await
            .is_err()
    );
    assert!(
        timeout(Duration::from_millis(20), all.next())
            .await
            .is_err()
    );
    drop((stream, all, first, second, third));
    timeout(Duration::from_secs(1), async {
        let rows: Vec<_> = store
            .read_all(None)
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(rows.len(), 3);
    })
    .await
    .expect("cancelled pool requests must not leak capacity");
}
