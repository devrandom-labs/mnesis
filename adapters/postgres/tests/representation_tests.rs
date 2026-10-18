//! Real-database representation and read-only startup boundaries.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions"
)]
#![allow(clippy::missing_panics_doc, reason = "test assertions")]

use futures::{StreamExt, TryStreamExt};
use mnesis::Version;
use mnesis_postgres::{PgAllPos, PostgresStore};
use mnesis_store::envelope::{PendingBatch, PendingEnvelope, pending_envelope};
use mnesis_store::{RawEventStore, StreamKey};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

async fn fixture() -> Option<(PostgresStore, PgPool)> {
    let url = std::env::var("DATABASE_URL").ok()?;
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .unwrap();
    PostgresStore::migrate(&pool).await.unwrap();
    let store = PostgresStore::from_pool(pool.clone()).await.unwrap();
    sqlx::query("TRUNCATE mnesis.events, mnesis.snapshots, mnesis.keys RESTART IDENTITY")
        .execute(&pool)
        .await
        .unwrap();
    Some((store, pool))
}

fn event(version: u64, kind: &'static str) -> PendingEnvelope {
    pending_envelope(Version::new(version).unwrap())
        .event_type(kind)
        .payload(b"value".as_slice())
        .build()
        .unwrap()
}

#[tokio::test]
async fn valid_unsigned_stream_bound_returns_empty_instead_of_narrowing() {
    let Some((store, _control)) = fixture().await else {
        return;
    };
    let stream = StreamKey::from_bytes("unsigned-bound");
    assert!(
        store
            .read_stream(&stream, Version::new(u64::MAX).unwrap())
            .await
            .unwrap()
            .next()
            .await
            .is_none()
    );
}

#[tokio::test]
async fn unsigned_global_sequence_still_includes_later_transactions() {
    let Some((store, _control)) = fixture().await else {
        return;
    };
    let before = StreamKey::from_bytes("before");
    let after = StreamKey::from_bytes("after");
    let rows = [event(1, "Position")];
    let first = store
        .append(&before, None, PendingBatch::new(&rows).unwrap())
        .await
        .unwrap();
    let second = store
        .append(&after, None, PendingBatch::new(&rows).unwrap())
        .await
        .unwrap();
    assert!(first.txid() < second.txid());
    let resumed: Vec<_> = store
        .read_all(Some(PgAllPos::new(first.txid(), u64::MAX)))
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(
        resumed.len(),
        1,
        "a high sequence only excludes its own transaction"
    );
    assert_eq!(resumed[0].0, second);
    assert_eq!(resumed[0].1, after);
    assert!(
        store
            .read_all(Some(PgAllPos::new(u64::MAX, u64::MAX)))
            .await
            .unwrap()
            .next()
            .await
            .is_none()
    );
}

#[tokio::test]
async fn utf8_event_type_with_nul_round_trips() {
    let Some((store, _control)) = fixture().await else {
        return;
    };
    let id = StreamKey::from_bytes("nul-type");
    let rows = [event(1, "A\0B")];
    store
        .append(&id, None, PendingBatch::new(&rows).unwrap())
        .await
        .unwrap();
    let read: Vec<_> = store
        .read_stream(&id, Version::INITIAL)
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].event_type(), "A\0B");
    assert_eq!(read[0].payload(), b"value");
}

#[tokio::test]
async fn incompressible_binary_key_beyond_notify_limit_round_trips() {
    let Some((store, control)) = fixture().await else {
        return;
    };
    let key: Vec<u8> = sqlx::query_scalar(
        "SELECT decode(string_agg(md5(n::text), ''), 'hex') FROM generate_series(1, 320) n",
    )
    .fetch_one(&control)
    .await
    .unwrap();
    assert_eq!(key.len(), 5120);
    let id = StreamKey::from_bytes(key);
    let rows = [event(1, "Key")];
    store
        .append(&id, None, PendingBatch::new(&rows).unwrap())
        .await
        .unwrap_or_else(|error| panic!("large raw key rejected: {error}"));
    let read: Vec<_> = store
        .read_all(None)
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].1, id);
    assert_eq!(read[0].2.payload(), b"value");
}

#[tokio::test]
async fn sequential_append_crosses_the_signed_version_boundary() {
    let Some((store, control)) = fixture().await else {
        return;
    };
    let id = StreamKey::from_bytes("version-boundary");
    let number: i64 =
        sqlx::query_scalar("INSERT INTO mnesis.keys (value) VALUES ($1) RETURNING number")
            .bind(id.as_bytes())
            .fetch_one(&control)
            .await
            .unwrap();
    sqlx::query(
        "INSERT INTO mnesis.events (stream_id, version, event_type, schema_version, payload) \
         VALUES ($1, 9223372036854775807, 'Version', 1, 'old'::bytea)",
    )
    .bind(number)
    .execute(&control)
    .await
    .unwrap();
    let rows = [event(9_223_372_036_854_775_808, "Version")];
    store
        .append(
            &id,
            Version::new(9_223_372_036_854_775_807),
            PendingBatch::new(&rows).unwrap(),
        )
        .await
        .unwrap();
    let read: Vec<_> = store
        .read_stream(&id, rows[0].version())
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].version(), rows[0].version());
    assert_eq!(read[0].payload(), b"value");
}

#[tokio::test]
async fn full_unsigned_maximum_round_trips_and_overflow_leaves_it_unchanged() {
    let Some((store, control)) = fixture().await else {
        return;
    };
    let id = StreamKey::from_bytes("maximum-version");
    let number: i64 =
        sqlx::query_scalar("INSERT INTO mnesis.keys (value) VALUES ($1) RETURNING number")
            .bind(id.as_bytes())
            .fetch_one(&control)
            .await
            .unwrap();
    sqlx::query("INSERT INTO mnesis.events (stream_id, version, event_type, schema_version, payload) VALUES ($1, 18446744073709551614, 'Seed', 1, 'seed'::bytea)")
        .bind(number).execute(&control).await.unwrap();
    let last = pending_envelope(Version::new(u64::MAX).unwrap())
        .event_type("Maximum")
        .payload(b"maximum".as_slice())
        .schema_version(mnesis_store::SchemaVersion::new(std::num::NonZeroU32::MAX))
        .build()
        .unwrap();
    let position = store
        .append(
            &id,
            Version::new(u64::MAX - 1),
            PendingBatch::new(&[last]).unwrap(),
        )
        .await
        .unwrap();
    let rows: Vec<_> = store
        .read_stream(&id, Version::new(u64::MAX).unwrap())
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].version().as_u64(), u64::MAX);
    assert_eq!(rows[0].schema_version(), u32::MAX);
    assert_eq!(rows[0].payload(), b"maximum");
    let rejected = store
        .append(
            &id,
            Version::new(u64::MAX),
            PendingBatch::new(&[event(1, "Overflow")]).unwrap(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        rejected,
        mnesis_store::AppendError::Store(mnesis_postgres::PostgresError::CorruptRow { .. })
    ));
    let all: Vec<_> = store
        .read_all(None)
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[1].0, position);
    assert_eq!(all[1].2.version().as_u64(), u64::MAX);
    assert_eq!(all[1].2.payload(), b"maximum");
}

#[tokio::test]
async fn real_hash_collision_keeps_both_binary_identities() {
    let Some((store, control)) = fixture().await else {
        return;
    };
    let left = StreamKey::from_bytes("collision-44072");
    let right = StreamKey::from_bytes("collision-102013");
    let hashes: (i32, i32) =
        sqlx::query_as("SELECT hash_record(ROW($1::bytea)), hash_record(ROW($2::bytea))")
            .bind(left.as_bytes())
            .bind(right.as_bytes())
            .fetch_one(&control)
            .await
            .unwrap();
    assert_eq!(
        hashes.0, hashes.1,
        "fixture must be an actual native hash collision"
    );
    assert_ne!(left, right);
    let pending = [event(1, "Collision")];
    let (one, two) = tokio::join!(
        store.append(&left, None, PendingBatch::new(&pending).unwrap()),
        store.append(&right, None, PendingBatch::new(&pending).unwrap())
    );
    one.unwrap();
    two.unwrap();
    let mut plan = control.begin().await.unwrap();
    sqlx::query("SET LOCAL enable_seqscan = off")
        .execute(&mut *plan)
        .await
        .unwrap();
    let lines: Vec<String> = sqlx::query_scalar(
        "EXPLAIN (ANALYZE, COSTS OFF) SELECT number FROM mnesis.keys WHERE value = $1",
    )
    .bind(left.as_bytes())
    .fetch_all(&mut *plan)
    .await
    .unwrap();
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Rows Removed by Index Recheck: 1")),
        "both equal-hash entries must reach native equality recheck: {lines:?}"
    );
    plan.commit().await.unwrap();
    let all: Vec<_> = store
        .read_all(None)
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all.iter().filter(|row| row.1 == left).count(), 1);
    assert_eq!(all.iter().filter(|row| row.1 == right).count(), 1);
    for key in [&left, &right] {
        let actual: Vec<_> = store
            .read_stream(key, Version::INITIAL)
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(actual.len(), 1);
        assert_eq!(actual[0].payload(), b"value");
    }
}

#[tokio::test]
async fn concurrent_large_key_writers_share_one_identity_and_one_winner() {
    let Some((store, control)) = fixture().await else {
        return;
    };
    let bytes: Vec<u8> = sqlx::query_scalar(
        "SELECT decode(string_agg(md5(n::text), ''), 'hex') FROM generate_series(1, 320) n",
    )
    .fetch_one(&control)
    .await
    .unwrap();
    let id = StreamKey::from_bytes(bytes);
    let pending = [event(1, "Concurrent")];
    let (first, second) = tokio::join!(
        store.append(&id, None, PendingBatch::new(&pending).unwrap()),
        store.append(&id, None, PendingBatch::new(&pending).unwrap())
    );
    let results: [_; 2] = (first, second).into();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, Err(mnesis_store::AppendError::Conflict { .. })))
            .count(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM mnesis.keys WHERE value = $1")
            .bind(id.as_bytes())
            .fetch_one(&control)
            .await
            .unwrap(),
        1
    );
    let rows: Vec<_> = store
        .read_all(None)
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].1, id);
    assert_eq!(rows[0].2.payload(), b"value");
}

#[tokio::test]
async fn scalar_constraints_reject_invalid_stored_values() {
    let Some((_store, control)) = fixture().await else {
        return;
    };
    let number: i64 = sqlx::query_scalar(
        "INSERT INTO mnesis.keys (value) VALUES ('invalid-scalars'::bytea) RETURNING number",
    )
    .fetch_one(&control)
    .await
    .unwrap();
    for invalid in ["0", "-1", "18446744073709551616", "NaN"] {
        let error = sqlx::query("INSERT INTO mnesis.events (stream_id, version, event_type, schema_version, payload) VALUES ($1, $2::text::numeric, 'Invalid', 1, 'value'::bytea)")
            .bind(number).bind(invalid).execute(&control).await.unwrap_err();
        assert!(matches!(
            error
                .as_database_error()
                .and_then(sqlx::error::DatabaseError::code)
                .as_deref(),
            Some("23514" | "22003")
        ));
    }
    for invalid in [0_i64, -1, 4_294_967_296] {
        let error = sqlx::query("INSERT INTO mnesis.events (stream_id, version, event_type, schema_version, payload) VALUES ($1, 1, 'Invalid', $2, 'value'::bytea)")
            .bind(number).bind(invalid).execute(&control).await.unwrap_err();
        assert_eq!(
            error.as_database_error().unwrap().code().as_deref(),
            Some("23514")
        );
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM mnesis.events")
            .fetch_one(&control)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn startup_rejects_missing_key_equality_and_scalar_guards() {
    let Some((_store, control)) = fixture().await else {
        return;
    };
    sqlx::query("ALTER TABLE mnesis.keys DROP CONSTRAINT keys_value_key")
        .execute(&control)
        .await
        .unwrap();
    let missing_key = PostgresStore::from_pool(control.clone()).await;
    sqlx::query(
        "ALTER TABLE mnesis.keys ADD CONSTRAINT keys_value_key EXCLUDE USING hash (value WITH =)",
    )
    .execute(&control)
    .await
    .unwrap();
    assert!(matches!(
        missing_key,
        Err(mnesis_postgres::PostgresError::Schema { .. })
    ));
    sqlx::query("ALTER TABLE mnesis.events DROP CONSTRAINT events_version_check")
        .execute(&control)
        .await
        .unwrap();
    let missing_version = PostgresStore::from_pool(control.clone()).await;
    sqlx::query("ALTER TABLE mnesis.events ADD CONSTRAINT events_version_check CHECK (version > 0 AND version <= 18446744073709551615)").execute(&control).await.unwrap();
    assert!(matches!(
        missing_version,
        Err(mnesis_postgres::PostgresError::Schema { .. })
    ));
    PostgresStore::from_pool(control).await.unwrap();
}
