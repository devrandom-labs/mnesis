//! `PostgresStore` conformance against the executable store contract (#281).
//!
//! Skips (passes vacuously) when `DATABASE_URL` is unset — the native test hook
//! supplies a real URL in CI and runs serially (`--test-threads=1`), so each
//! test's fresh-store factory TRUNCATEs for isolation.

#![allow(clippy::unwrap_used, reason = "tests")]
#![allow(clippy::expect_used, reason = "tests")]
#![allow(clippy::missing_panics_doc, reason = "tests")]

use mnesis_postgres::{History, PgAllPos, PostgresStore};

fn have_db() -> bool {
    std::env::var("DATABASE_URL").is_ok()
}

/// Fresh store over a truncated events table. Only called when `have_db()`.
async fn open_fresh() -> (PostgresStore, ()) {
    let url = std::env::var("DATABASE_URL").expect("guarded by skip_unless");
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect(&url)
        .await
        .expect("connect pool");
    // Migrate explicitly FIRST, then validate and truncate: on a fresh
    // database the `events` table does
    // not exist yet, so truncating before creation would fail. Tests run
    // serially (the native hook invokes nextest with `--test-threads=1`), so
    // TRUNCATE gives each test a clean, isolated slate against the shared
    // table.
    PostgresStore::migrate(&pool).await.unwrap();
    let store = PostgresStore::from_pool(pool.clone())
        .await
        .expect("from_pool");
    sqlx::query("TRUNCATE mnesis.events RESTART IDENTITY")
        .execute(&pool)
        .await
        .expect("truncate events");
    (store, ())
}

// Optional atomic laws live in atomic_tests.rs with the import feature.
mnesis_store_testing::conformance! {
    factory: open_fresh,
    skip_unless: have_db,
}

// "Reopen" = a brand-new pool + store over the same database (no truncate!).
mnesis_store_testing::conformance_lifecycle! {
    open: open_fresh,
    reopen: |store: PostgresStore, (): ()| async move {
        drop(store);
        let url = std::env::var("DATABASE_URL").expect("guarded by skip_unless");
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect(&url)
            .await
            .expect("reconnect pool");
        let reopened = PostgresStore::from_pool(pool).await.expect("from_pool");
        (reopened, ())
    },
    skip_unless: have_db,
}

// ---------------------------------------------------------------------------
// `PgAllPos` ordering: locked by a compile-time test in `position.rs`, also
// verified here against the public API.
// ---------------------------------------------------------------------------

/// `PgAllPos` ord: lower `txid` wins regardless of `seq`.
/// Mirrors the lock test in `position.rs` but exercises the public API.
#[test]
fn pg_all_pos_ord_txid_first() {
    let history = History::new([1; 16]);
    assert!(PgAllPos::new(history, 1, 9) < PgAllPos::new(history, 2, 0));
    assert!(PgAllPos::new(history, 3, 1) < PgAllPos::new(history, 3, 2));
    assert_eq!(PgAllPos::new(history, 5, 5), PgAllPos::new(history, 5, 5));
}
