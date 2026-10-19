//! `$all` no-skip linearizability test (Task 8, Step 7).
//!
//! This file proves that the composite `(txid, global_seq)` position plus the
//! `pg_snapshot_xmin` watermark make the `$all` read gap-free **by construction**
//! under concurrent writers with controlled out-of-order commits.
//!
//! ## The hazard the old scalar position had
//!
//! With a bare `global_seq` resume scalar:
//! - Transaction A `INSERT`s to stream X, claims a **lower** `global_seq`, but
//!   has not yet committed.
//! - Transaction B `INSERT`s to stream Y, claims a **higher** `global_seq`, and
//!   **commits**.
//! - A `$all` cursor running `WHERE global_seq > $last ORDER BY global_seq`
//!   would consume B, checkpoint B's seq, then — when A finally commits — skip A
//!   forever because `A.global_seq < B.global_seq <= $last`.
//!
//! ## How the composite position + watermark closes the gap (#266)
//!
//! The `read_all` query has two predicates:
//!
//! 1. **Watermark**: `txid < pg_snapshot_xmin(pg_current_snapshot())` — only
//!    deliver rows whose writing txn is provably committed (its txid is below
//!    the snapshot's xmin, i.e., every lower txid is also committed). While A
//!    is still open, `xmin ≤ A.txid`; since `B.txid > A.txid ≥ xmin`, B's rows
//!    satisfy `B.txid ≥ xmin`, so they are withheld too.
//!
//! 2. **Exclusive `Ord` resume**: `(txid::text::bigint, global_seq) > ($1, $2)`.
//!    Once both A and B commit, `xmin` advances past both, and the next
//!    `read_all(None)` delivers both in `(txid, global_seq)` order — A first,
//!    B second — with no gap.
//!
//! ## Test structure
//!
//! 1. Open transaction A (`pool.begin()`), INSERT to stream X.
//! 2. Open transaction B, INSERT to stream Y, COMMIT B immediately.
//! 3. Run `read_all(None)` — assert it delivers **nothing** (watermark holds).
//! 4. COMMIT transaction A.
//! 5. Run `read_all(None)` again — assert both events appear in `(txid, seq)`
//!    order with A (lower txid) first. This is the event the old scalar cursor
//!    would have skipped.
//!
//! # Skip-without-DATABASE_URL
//!
//! If `DATABASE_URL` is unset the test returns immediately (passes). The
//! nixosTest supplies the URL in CI.

#![allow(clippy::unwrap_used, reason = "tests")]
#![allow(clippy::expect_used, reason = "tests")]
#![allow(clippy::missing_panics_doc, reason = "tests")]
#![allow(clippy::panic, reason = "tests")]

mod support;

use std::sync::Arc;

use futures::{StreamExt, TryStreamExt};
use mnesis::Version;
use mnesis_postgres::{History, PgAllPos, PostgresError, PostgresStore};
use mnesis_store::store::RawEventStore;
use mnesis_store::{PendingBatch, StreamKey, pending_envelope};
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Postgres, Transaction};
use tokio::sync::Barrier;

// ---------------------------------------------------------------------------
// Infrastructure
// ---------------------------------------------------------------------------

/// Connect + truncate. Returns `None` (skip) if `DATABASE_URL` is unset.
async fn setup() -> Option<(PostgresStore, PgPool)> {
    let url = std::env::var("DATABASE_URL").ok()?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(10)
        .connect(&url)
        .await
        .expect("connect pool");
    // Create the schema FIRST, then truncate (see conformance_tests::setup — a
    // fresh DB has no `events` table yet; tests run serially via nextest
    // `--test-threads=1` so TRUNCATE isolates each one).
    PostgresStore::migrate(&pool).await.unwrap();
    let store = PostgresStore::from_pool(pool.clone())
        .await
        .expect("from_pool");
    sqlx::query("TRUNCATE mnesis.events RESTART IDENTITY")
        .execute(&pool)
        .await
        .expect("truncate");
    Some((store, pool))
}

/// Drain a full `read_all(from)` into `(PgAllPos, event_type, stream_id_bytes)`
/// triples — the third field is the origin [`StreamKey`]'s raw bytes, surfaced
/// so tests can assert per-event attribution (#333).
async fn drain_all(
    store: &PostgresStore,
    from: Option<PgAllPos>,
) -> Vec<(PgAllPos, String, Vec<u8>)> {
    let stream = store.read_all(from).await.expect("open read_all");
    futures::pin_mut!(stream);
    let mut out = Vec::new();
    while let Some(item) = stream.next().await {
        let (pos, key, env) = item.expect("read_all item must be Ok");
        out.push((pos, env.event_type().to_owned(), key.as_bytes().to_vec()));
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

async fn held_event(pool: &PgPool, history: History) -> (Transaction<'_, Postgres>, PgAllPos) {
    let number = support::key(pool, b"earlier").await;
    let mut transaction = pool.begin().await.unwrap();
    let (txid, seq): (String, i64) = sqlx::query_as(
        "INSERT INTO mnesis.events (stream_id, version, event_type, schema_version, payload) \
         VALUES ($1, 1, $2, 1, $3) RETURNING txid::text, global_seq",
    )
    .bind(number)
    .bind(b"Earlier".as_slice())
    .bind(b"first".as_slice())
    .fetch_one(&mut *transaction)
    .await
    .unwrap();
    let position = PgAllPos::new(history, txid.parse().unwrap(), u64::try_from(seq).unwrap());
    (transaction, position)
}

async fn read_outcome(
    store: &PostgresStore,
    from: Option<PgAllPos>,
) -> Result<Vec<(PgAllPos, String, Vec<u8>)>, Box<PostgresError>> {
    store
        .read_all(from)
        .await
        .map_err(Box::new)?
        .map_ok(|(pos, key, event)| (pos, event.event_type().to_owned(), key.as_bytes().to_vec()))
        .try_collect()
        .await
        .map_err(Box::new)
}

async fn shadowed_ordering(statement: &'static str) {
    let Some((store, pool)) = setup().await else {
        return;
    };
    sqlx::raw_sql("CREATE SCHEMA ordering")
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
                .options([("search_path", "ordering,pg_catalog")]),
        )
        .await
        .unwrap();
    let opened = PostgresStore::from_pool(runtime.clone()).await;
    if opened.is_err() {
        runtime.close().await;
        sqlx::raw_sql("DROP SCHEMA ordering CASCADE")
            .execute(&pool)
            .await
            .unwrap();
    }
    let reader = opened.unwrap();
    let (older, first) = held_event(&pool, store.history()).await;
    let barrier = Arc::new(Barrier::new(2));
    let later = {
        let writer_barrier = Arc::clone(&barrier);
        let writer = store.clone();
        tokio::spawn(async move {
            writer_barrier.wait().await;
            let event = pending_envelope(Version::INITIAL)
                .event_type("Later")
                .payload(b"second".as_slice())
                .build()
                .unwrap();
            writer
                .append(
                    &StreamKey::from_slice(b"later"),
                    None,
                    PendingBatch::of(&event),
                )
                .await
        })
    };
    barrier.wait().await;
    let result = later.await;
    let held = read_outcome(&reader, None).await;
    older.commit().await.unwrap();
    let all = read_outcome(&reader, None).await;
    let resumed = read_outcome(&reader, Some(first)).await;
    let boundary = PgAllPos::new(store.history(), first.txid(), u64::MAX);
    let beyond = read_outcome(&reader, Some(boundary)).await;
    drop(reader);
    runtime.close().await;
    sqlx::raw_sql("DROP SCHEMA ordering CASCADE")
        .execute(&pool)
        .await
        .unwrap();
    let second = result.unwrap().unwrap();
    let expected = vec![
        (first, "Earlier".to_owned(), b"earlier".to_vec()),
        (second, "Later".to_owned(), b"later".to_vec()),
    ];
    assert_eq!(read_outcome(&store, None).await.unwrap(), expected);
    assert_eq!(held.unwrap(), Vec::new(), "must withhold the newer commit");
    assert_eq!(all.unwrap(), expected);
    assert_eq!(resumed.unwrap(), expected[1..]);
    assert_eq!(beyond.unwrap(), expected[1..]);
}

#[tokio::test]
async fn all_ordering_ignores_shadowed_current_snapshot() {
    shadowed_ordering(
        "CREATE FUNCTION ordering.pg_current_snapshot() RETURNS pg_catalog.pg_snapshot \
         LANGUAGE sql AS $$ SELECT '1:1:'::pg_catalog.pg_snapshot $$",
    )
    .await;
}

#[tokio::test]
async fn all_ordering_ignores_shadowed_snapshot_xmin() {
    shadowed_ordering(
        "CREATE FUNCTION ordering.pg_snapshot_xmin(pg_catalog.pg_snapshot) \
         RETURNS pg_catalog.xid8 LANGUAGE sql \
         AS $$ SELECT '18446744073709551615'::pg_catalog.xid8 $$",
    )
    .await;
}

#[tokio::test]
async fn all_ordering_ignores_shadowed_cutoff_operator() {
    shadowed_ordering(
        "CREATE FUNCTION ordering.less(pg_catalog.xid8, pg_catalog.xid8) \
         RETURNS pg_catalog.bool LANGUAGE sql AS $$ SELECT true $$; \
         CREATE OPERATOR ordering.< (FUNCTION = ordering.less, \
         LEFTARG = pg_catalog.xid8, RIGHTARG = pg_catalog.xid8)",
    )
    .await;
}

#[tokio::test]
async fn all_ordering_ignores_shadowed_resume_operator() {
    shadowed_ordering(
        "CREATE FUNCTION ordering.greater(pg_catalog.xid8, pg_catalog.xid8) \
         RETURNS pg_catalog.bool LANGUAGE sql AS $$ SELECT false $$; \
         CREATE OPERATOR ordering.> (FUNCTION = ordering.greater, \
         LEFTARG = pg_catalog.xid8, RIGHTARG = pg_catalog.xid8)",
    )
    .await;
}

/// The `$all` no-skip linearizability test under out-of-order commits.
///
/// This is the freeze evidence that the composite position + watermark
/// (`#266`) makes the live `$all` subscription gap-free by construction.
#[tokio::test]
async fn all_noskip_out_of_order_commits() {
    let Some((store, pool)) = setup().await else {
        return;
    };

    let stream_x = StreamKey::from_slice(b"stream-x");
    let stream_y = StreamKey::from_slice(b"stream-y");

    // ── Phase 1: Transaction A opens and inserts to stream X but does NOT commit. ──
    //
    // We use an explicit `sqlx::Transaction` so we can hold it open across awaits.
    // `pool.begin()` acquires a connection and issues `BEGIN`.
    let x_number = support::key(&pool, stream_x.as_bytes()).await;
    let y_number = support::key(&pool, stream_y.as_bytes()).await;
    let mut tx_a = pool.begin().await.expect("BEGIN tx_a");

    // INSERT to stream X inside tx_a. This claims a `txid` (lower, because A
    // opened first) but the row is not visible until tx_a commits.
    sqlx::query(
        "INSERT INTO mnesis.events (stream_id, version, event_type, schema_version, payload) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(x_number)
    .bind(1_i64)
    .bind(b"XEvent".as_slice())
    .bind(1_i32)
    .bind(b"payload-x".as_slice())
    .execute(&mut *tx_a)
    .await
    .expect("INSERT stream X in tx_a");

    // ── Phase 2: Transaction B inserts to stream Y and COMMITS immediately. ──
    //
    // B commits before A, so B's row becomes visible first. Under the old
    // scalar-`global_seq` resume, B would be consumed and its seq checkpointed;
    // then when A commits (lower seq), A's event would be missed forever.
    let mut tx_b = pool.begin().await.expect("BEGIN tx_b");
    sqlx::query(
        "INSERT INTO mnesis.events (stream_id, version, event_type, schema_version, payload) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(y_number)
    .bind(1_i64)
    .bind(b"YEvent".as_slice())
    .bind(1_i32)
    .bind(b"payload-y".as_slice())
    .execute(&mut *tx_b)
    .await
    .expect("INSERT stream Y in tx_b");
    tx_b.commit().await.expect("COMMIT tx_b");

    // ── Phase 3: `read_all(None)` while A is still in-flight. ──
    //
    // Assert: the watermark withholds everything because A's txid is still open.
    // `pg_snapshot_xmin` will be ≤ A's txid; the predicate `events.txid < xmin`
    // is false for every row (A's txid ≥ xmin, B's txid > A's txid ≥ xmin).
    let phase3 = drain_all(&store, None).await;
    assert!(
        phase3.is_empty(),
        "Phase 3: while tx_a is open, read_all must deliver nothing — \
         the watermark withholds B (B.txid > A.txid ≥ xmin). Got: {phase3:?}"
    );

    // ── Phase 4: Commit transaction A. ──
    tx_a.commit().await.expect("COMMIT tx_a");

    // ── Phase 5: `read_all(None)` — both events must appear in (txid, seq) order. ──
    //
    // A.txid < B.txid because `pg_current_xact_id()` assigns xids as transactions
    // start (BEGIN), and A started before B. Lexicographic `Ord` on `PgAllPos`
    // sorts by txid first, so A's event must sort before B's event — exactly the
    // order a gap-free cursor must deliver. The old scalar cursor would have
    // delivered B (higher seq, already committed) and then skipped A forever.
    let phase5 = drain_all(&store, None).await;
    assert_eq!(
        phase5.len(),
        2,
        "Phase 5: both events must be delivered after both txns commit. Got: {phase5:?}"
    );

    // Positions must be strictly increasing.
    assert!(
        phase5[1].0 > phase5[0].0,
        "Phase 5: positions must be strictly increasing: {:?} then {:?}",
        phase5[0].0,
        phase5[1].0
    );

    // The first delivered event must be A's (XEvent) — lower txid sorts first.
    assert_eq!(
        phase5[0].1, "XEvent",
        "Phase 5: X's event (lower txid = A opened first) must sort before Y's event. \
         Got first={:?}, second={:?}",
        phase5[0].1, phase5[1].1
    );
    assert_eq!(
        phase5[1].1, "YEvent",
        "Phase 5: Y's event (higher txid = B opened after A) must sort after X's event"
    );

    // Attribution (#333): each yielded item carries the origin stream id beside
    // the envelope, in position order — X's event is attributed to stream-x, Y's
    // to stream-y — so a consumer can route without decoding the payload.
    assert_eq!(
        phase5[0].2,
        stream_x.as_bytes(),
        "Phase 5: first item ($all pos 0) must be attributed to stream-x"
    );
    assert_eq!(
        phase5[1].2,
        stream_y.as_bytes(),
        "Phase 5: second item ($all pos 1) must be attributed to stream-y"
    );
}

/// Resuming from the position of the first event (X) must yield only the
/// second event (Y) — confirming exclusive `Ord`-based resume is correct
/// after out-of-order commits (no duplicate X on the next scan).
#[tokio::test]
async fn all_noskip_exclusive_resume_after_out_of_order_commits() {
    let Some((store, pool)) = setup().await else {
        return;
    };

    let stream_x = StreamKey::from_slice(b"resume-x");
    let stream_y = StreamKey::from_slice(b"resume-y");

    // Set up: A inserts X (opens first, lower txid), B inserts Y and commits,
    // then A commits.
    let x_number = support::key(&pool, stream_x.as_bytes()).await;
    let y_number = support::key(&pool, stream_y.as_bytes()).await;
    let mut tx_a = pool.begin().await.expect("BEGIN tx_a");
    sqlx::query(
        "INSERT INTO mnesis.events (stream_id, version, event_type, schema_version, payload) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(x_number)
    .bind(1_i64)
    .bind(b"XEvent2".as_slice())
    .bind(1_i32)
    .bind(b"x".as_slice())
    .execute(&mut *tx_a)
    .await
    .expect("INSERT X in tx_a");

    let mut tx_b = pool.begin().await.expect("BEGIN tx_b");
    sqlx::query(
        "INSERT INTO mnesis.events (stream_id, version, event_type, schema_version, payload) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(y_number)
    .bind(1_i64)
    .bind(b"YEvent2".as_slice())
    .bind(1_i32)
    .bind(b"y".as_slice())
    .execute(&mut *tx_b)
    .await
    .expect("INSERT Y in tx_b");
    tx_b.commit().await.expect("COMMIT tx_b");
    tx_a.commit().await.expect("COMMIT tx_a");

    // Both committed. Full scan: X sorts before Y (lower txid).
    let full = drain_all(&store, None).await;
    assert_eq!(full.len(), 2);

    let pos_of_x = full[0].0;
    assert_eq!(full[0].1, "XEvent2", "X (lower txid) must sort first");
    assert_eq!(full[1].1, "YEvent2");

    // Resume strictly after X's position → only Y.
    let resumed = drain_all(&store, Some(pos_of_x)).await;
    assert_eq!(
        resumed.len(),
        1,
        "resuming after X must yield only Y, got: {resumed:?}"
    );
    assert_eq!(resumed[0].1, "YEvent2");
    assert!(
        resumed[0].0 > pos_of_x,
        "resumed position must be strictly after the checkpoint"
    );
}
