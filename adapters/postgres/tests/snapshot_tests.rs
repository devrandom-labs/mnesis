//! Native aggregate cache laws; projection checkpoints have a separate owner.
#![cfg(feature = "snapshot")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "native assertions"
)]

use mnesis::Version;
use mnesis_postgres::{Limits, PostgresError, PostgresStore};
use mnesis_store::state::{CodecSnapshotStore, EveryNEvents, Hydrated, SnapshotStore};
use mnesis_store::{JsonCodec, Repository, Snapshotting, Store, StoreError, StreamKey};
use mnesis_test_domains::{Counter, CounterEvent, CounterState, TestId};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use std::borrow::Cow;
use std::io::{Write, stderr, stdout};
use std::num::{NonZeroU32, NonZeroU64};
use std::process::Command;
use std::sync::atomic::Ordering;
use tokio::task::{JoinSet, yield_now};
use tokio::time::{Duration, timeout};

#[path = "support/memory.rs"]
mod memory;

#[path = "support/process.rs"]
mod process;

const CHILD: &str = "MNESIS_POSTGRES_SNAPSHOT_CHILD";
const ACK: &str = "MNESIS_POSTGRES_SNAPSHOT_ACK";

#[tokio::test]
async fn suppressed_snapshot_writes_fail_without_changing_cache_or_creating_keys() {
    if std::env::var("DATABASE_URL").is_err() {
        return;
    }
    let (store, pool) = fixture().await;
    let existing = StreamKey::from_slice(b"existing-cache");
    let missing = StreamKey::from_slice(b"missing-cache");
    let original = vec![1];
    store
        .commit(&existing, NonZeroU32::MIN, Version::INITIAL, &original)
        .await
        .unwrap();
    sqlx::raw_sql("CREATE FUNCTION mnesis.snapshot_skip() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RETURN NULL; END $$")
        .execute(&pool).await.unwrap();
    let mut results = Vec::new();
    for (statement, id) in [
        (
            "CREATE TRIGGER snapshot_skip BEFORE INSERT ON mnesis.snapshots FOR EACH ROW EXECUTE FUNCTION mnesis.snapshot_skip()",
            &missing,
        ),
        (
            "CREATE TRIGGER snapshot_skip BEFORE UPDATE ON mnesis.snapshots FOR EACH ROW EXECUTE FUNCTION mnesis.snapshot_skip()",
            &existing,
        ),
    ] {
        sqlx::raw_sql(statement).execute(&pool).await.unwrap();
        results.push(
            store
                .commit(id, NonZeroU32::MIN, Version::new(2).unwrap(), &vec![2])
                .await,
        );
        sqlx::raw_sql("DROP TRIGGER snapshot_skip ON mnesis.snapshots")
            .execute(&pool)
            .await
            .unwrap();
    }
    sqlx::raw_sql("DROP FUNCTION mnesis.snapshot_skip()")
        .execute(&pool)
        .await
        .unwrap();
    let absent = store.hydrate(&missing, NonZeroU32::MIN).await.unwrap();
    let unchanged = store.hydrate(&existing, NonZeroU32::MIN).await.unwrap();
    let keys: i64 = sqlx::query_scalar("SELECT count(*) FROM mnesis.keys")
        .fetch_one(&pool)
        .await
        .unwrap();
    for outcome in results {
        assert!(
            matches!(outcome, Err(PostgresError::Sqlx(sqlx::Error::RowNotFound))),
            "suppressed write must fail before COMMIT, got {outcome:?}"
        );
    }
    assert_eq!(absent, Hydrated::Absent);
    assert_eq!(
        unchanged,
        Hydrated::Found {
            position: Version::INITIAL,
            state: original
        }
    );
    assert_eq!(keys, 1);
}

#[tokio::test]
async fn runtime_role_writes_cache_without_mutating_keys_events_or_schema() {
    if std::env::var("DATABASE_URL").is_err() {
        return;
    }
    let (_store, pool) = fixture().await;
    sqlx::raw_sql("DROP ROLE IF EXISTS mnesis_snapshot_writer; CREATE ROLE mnesis_snapshot_writer LOGIN; GRANT USAGE ON SCHEMA mnesis TO mnesis_snapshot_writer; GRANT SELECT ON mnesis._sqlx_migrations TO mnesis_snapshot_writer; GRANT SELECT, INSERT ON mnesis.keys TO mnesis_snapshot_writer; GRANT USAGE ON SEQUENCE mnesis.keys_number_seq TO mnesis_snapshot_writer; GRANT SELECT, INSERT, UPDATE ON mnesis.snapshots TO mnesis_snapshot_writer")
        .execute(&pool).await.unwrap();
    let runtime = PgPoolOptions::new()
        .max_connections(4)
        .connect_with(
            pool.connect_options()
                .as_ref()
                .clone()
                .username("mnesis_snapshot_writer"),
        )
        .await
        .unwrap();
    let identity: (String, String) =
        sqlx::query_as("SELECT current_user::text, session_user::text")
            .fetch_one(&runtime)
            .await
            .unwrap();
    let store = PostgresStore::from_pool(runtime.clone()).await.unwrap();
    let id = StreamKey::from_slice(b"restricted-cache");
    let first = store
        .commit(&id, NonZeroU32::MIN, Version::INITIAL, &vec![1])
        .await;
    let second = store
        .commit(&id, NonZeroU32::MIN, Version::new(2).unwrap(), &vec![2])
        .await;
    let saved = store.hydrate(&id, NonZeroU32::MIN).await;
    let mut codes = Vec::new();
    for statement in [
        "UPDATE mnesis.keys SET value = 'changed'::bytea",
        "UPDATE mnesis.events SET payload = 'changed'::bytea",
        "DELETE FROM mnesis.events",
        "CREATE TABLE mnesis.forbidden (number int)",
    ] {
        let rejected = sqlx::raw_sql(statement).execute(&runtime).await;
        codes.push(rejected.err().and_then(|error| {
            error
                .as_database_error()
                .and_then(sqlx::error::DatabaseError::code)
                .map(Cow::into_owned)
        }));
    }
    drop(store);
    runtime.close().await;
    sqlx::raw_sql("DROP OWNED BY mnesis_snapshot_writer; DROP ROLE mnesis_snapshot_writer")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        identity,
        (
            "mnesis_snapshot_writer".to_owned(),
            "mnesis_snapshot_writer".to_owned()
        )
    );
    first.unwrap();
    second.unwrap();
    assert_eq!(
        saved.unwrap(),
        Hydrated::Found {
            position: Version::new(2).unwrap(),
            state: vec![2]
        }
    );
    assert_eq!(
        codes,
        [
            Some("42501".to_owned()),
            Some("42501".to_owned()),
            Some("42501".to_owned()),
            Some("42501".to_owned())
        ]
    );
}

#[tokio::test]
async fn runtime_open_rejects_missing_reference_nullable_state_and_unlogged_cache() {
    if std::env::var("DATABASE_URL").is_err() {
        return;
    }
    let (_store, pool) = fixture().await;
    for (alter, restore) in [
        (
            "ALTER TABLE mnesis.snapshots DROP CONSTRAINT snapshots_key_fkey",
            "ALTER TABLE mnesis.snapshots ADD CONSTRAINT snapshots_key_fkey FOREIGN KEY (key) REFERENCES mnesis.keys (number)",
        ),
        (
            "ALTER TABLE mnesis.snapshots ALTER COLUMN state DROP NOT NULL",
            "ALTER TABLE mnesis.snapshots ALTER COLUMN state SET NOT NULL",
        ),
        (
            "ALTER TABLE mnesis.snapshots SET UNLOGGED",
            "ALTER TABLE mnesis.snapshots SET LOGGED",
        ),
    ] {
        sqlx::raw_sql(alter).execute(&pool).await.unwrap();
        let rejected = PostgresStore::from_pool(pool.clone()).await;
        sqlx::raw_sql(restore).execute(&pool).await.unwrap();
        assert!(
            matches!(rejected, Err(PostgresError::Schema { .. })),
            "unsafe cache layout accepted after {alter}"
        );
    }
    PostgresStore::from_pool(pool).await.unwrap();
}

#[tokio::test]
async fn acknowledged_cache_pair_survives_client_process_termination() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let id = StreamKey::from_slice(b"crashed-cache\0");
    let schema = NonZeroU32::new(u32::MAX).unwrap();
    let position = Version::new(u64::MAX).unwrap();
    let state = b"saved before client SIGKILL".to_vec();
    match std::env::var(CHILD).as_deref() {
        Ok("write") => {
            let store = PostgresStore::connect(&url).await.unwrap();
            store.commit(&id, schema, position, &state).await.unwrap();
            writeln!(stdout().lock(), "\n{ACK}").unwrap();
            stdout().flush().unwrap();
            std::future::pending::<()>().await;
        }
        Err(std::env::VarError::NotPresent) => {
            let (store, pool) = fixture().await;
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "acknowledged_cache_pair_survives_client_process_termination",
                    "--nocapture",
                ])
                .env(CHILD, "write");
            process::kill_after_marker(&mut command, ACK);
            drop(store);
            pool.close().await;
            let reopened = PostgresStore::connect(&url).await.unwrap();
            assert_eq!(
                reopened.hydrate(&id, schema).await.unwrap(),
                Hydrated::Found { position, state }
            );
        }
        wrong => panic!("invalid process fixture phase: {wrong:?}"),
    }
}

#[tokio::test]
async fn real_snapshotting_repository_skips_the_cached_prefix() {
    if std::env::var("DATABASE_URL").is_err() {
        return;
    }
    let (raw, pool) = fixture().await;
    let store = Store::new(raw.clone());
    let repository = Snapshotting::new(
        store.repository::<Counter>().json().build(),
        CodecSnapshotStore::new(&raw, JsonCodec::default()),
        EveryNEvents(NonZeroU64::new(2).unwrap()),
        NonZeroU32::MIN,
        false,
    );
    let id = TestId::new("cached-counter");
    let mut root = repository.load(id.clone()).await.unwrap();
    repository
        .save::<1>(
            &mut root,
            &mnesis::events![CounterEvent::Incremented, CounterEvent::Incremented],
        )
        .await
        .unwrap();
    let codec = CodecSnapshotStore::new(&raw, JsonCodec::default());
    assert_eq!(
        codec.hydrate(&id, NonZeroU32::MIN).await.unwrap(),
        Hydrated::Found {
            position: Version::new(2).unwrap(),
            state: CounterState { value: 2 }
        }
    );
    let plain = store.repository::<Counter>().json().build();
    plain
        .save::<0>(&mut root, &mnesis::events![CounterEvent::Incremented])
        .await
        .unwrap();
    let original: Vec<u8> = sqlx::query_scalar("SELECT e.payload FROM mnesis.events e JOIN mnesis.keys k ON k.number = e.stream_id WHERE k.value = $1 AND e.version = 1")
        .bind(id.as_ref()).fetch_one(&pool).await.unwrap();
    sqlx::query("UPDATE mnesis.events SET payload = decode('ff', 'hex') WHERE stream_id = (SELECT number FROM mnesis.keys WHERE value = $1) AND version = 1")
        .bind(id.as_ref()).execute(&pool).await.unwrap();
    let uncached = plain.load(id.clone()).await;
    let cached = repository.load(id.clone()).await;
    sqlx::query("UPDATE mnesis.events SET payload = $1 WHERE stream_id = (SELECT number FROM mnesis.keys WHERE value = $2) AND version = 1")
        .bind(original).bind(id.as_ref()).execute(&pool).await.unwrap();
    assert!(matches!(uncached, Err(StoreError::Decode(_))));
    let loaded = cached.unwrap();
    assert_eq!(loaded.version(), Version::new(3));
    assert_eq!(loaded.state().unwrap().value, 3);
}

#[tokio::test]
async fn stale_cache_and_rejected_replacement_do_not_change_committed_aggregate_history() {
    if std::env::var("DATABASE_URL").is_err() {
        return;
    }
    let (raw, pool) = fixture().await;
    let store = Store::new(raw.clone());
    let plain = store.repository::<Counter>().json().build();
    let id = TestId::new("stale-counter");
    let mut root = plain.load(id.clone()).await.unwrap();
    plain
        .save::<2>(
            &mut root,
            &mnesis::events![
                CounterEvent::Incremented,
                CounterEvent::Incremented,
                CounterEvent::Incremented
            ],
        )
        .await
        .unwrap();
    let codec = CodecSnapshotStore::new(&raw, JsonCodec::default());
    codec
        .commit(
            &id,
            NonZeroU32::MIN,
            Version::new(2).unwrap(),
            &CounterState { value: 999 },
        )
        .await
        .unwrap();
    let repository = Snapshotting::new(
        plain,
        &codec,
        EveryNEvents(NonZeroU64::MIN),
        NonZeroU32::new(2).unwrap(),
        false,
    );
    let mut loaded = repository.load(id.clone()).await.unwrap();
    assert_eq!(loaded.version(), Version::new(3));
    assert_eq!(loaded.state().unwrap().value, 3);
    sqlx::raw_sql("CREATE FUNCTION mnesis.snapshot_reject() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'cache rejection' USING ERRCODE = '23514'; END $$; CREATE TRIGGER snapshot_reject AFTER INSERT OR UPDATE ON mnesis.snapshots FOR EACH ROW EXECUTE FUNCTION mnesis.snapshot_reject()")
        .execute(&pool).await.unwrap();
    let result = repository
        .save::<0>(&mut loaded, &mnesis::events![CounterEvent::Incremented])
        .await;
    sqlx::raw_sql(
        "DROP TRIGGER snapshot_reject ON mnesis.snapshots; DROP FUNCTION mnesis.snapshot_reject()",
    )
    .execute(&pool)
    .await
    .unwrap();
    let position = result.unwrap();
    assert_eq!(loaded.version(), Version::new(4));
    assert_eq!(loaded.state().unwrap().value, 4);
    let persisted: (i64, String, i64) = sqlx::query_as(
        "SELECT count(*), max(txid::text::numeric)::text, max(global_seq) FROM mnesis.events",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(persisted.0, 4);
    assert_eq!(
        position,
        mnesis_postgres::PgAllPos::new(
            persisted.1.parse().unwrap(),
            u64::try_from(persisted.2).unwrap()
        )
    );
    let saved: Hydrated<CounterState, Version> = codec.hydrate(&id, NonZeroU32::MIN).await.unwrap();
    assert_eq!(
        saved,
        Hydrated::Found {
            position: Version::new(2).unwrap(),
            state: CounterState { value: 999 }
        }
    );
    let reopened = repository.load(id).await.unwrap();
    assert_eq!(reopened.version(), Version::new(4));
    assert_eq!(reopened.state().unwrap().value, 4);
}

#[tokio::test]
async fn stale_and_oversized_snapshot_reads_do_not_allocate_the_stored_body() {
    if std::env::var("DATABASE_URL").is_err() {
        return;
    }
    let (store, pool) = fixture().await;
    let id = StreamKey::from_slice(b"k");
    store
        .commit(&id, NonZeroU32::MIN, Version::INITIAL, &vec![7; 1_048_576])
        .await
        .unwrap();
    let bounded = PostgresStore::from_pool_with_limits(
        pool,
        Limits {
            bytes: NonZeroU32::new(37).unwrap(),
            ..Limits::default()
        },
    )
    .await
    .unwrap();
    bounded
        .hydrate(&StreamKey::from_slice(b"missing"), NonZeroU32::MIN)
        .await
        .unwrap();
    let before = memory::TOTAL.load(Ordering::Relaxed);
    let stale = bounded.hydrate(&id, NonZeroU32::new(2).unwrap()).await;
    let stale_bytes = memory::TOTAL
        .load(Ordering::Relaxed)
        .checked_sub(before)
        .unwrap();
    assert_eq!(
        stale.unwrap(),
        Hydrated::Stale {
            stored_schema: NonZeroU32::MIN
        }
    );
    assert!(
        stale_bytes < 262_144,
        "stale snapshot allocated {stale_bytes} bytes"
    );
    let matching_before = memory::TOTAL.load(Ordering::Relaxed);
    let oversized = bounded.hydrate(&id, NonZeroU32::MIN).await;
    let matching_bytes = memory::TOTAL
        .load(Ordering::Relaxed)
        .checked_sub(matching_before)
        .unwrap();
    assert!(matches!(
        oversized,
        Err(PostgresError::RowLimit {
            actual: 1_048_609,
            limit: 37
        })
    ));
    assert!(
        matching_bytes < 262_144,
        "oversized snapshot allocated {matching_bytes} bytes"
    );
    writeln!(stderr().lock(), "snapshot1MiB stale_allocated={stale_bytes} oversized_allocated={matching_bytes} ceiling=262144").unwrap();
}

async fn fixture() -> (PostgresStore, sqlx::PgPool) {
    let url = std::env::var("DATABASE_URL").unwrap();
    let options = url
        .parse::<PgConnectOptions>()
        .unwrap()
        .application_name(&format!("mnesis_snapshot_{}", std::process::id()));
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect_with(options)
        .await
        .unwrap();
    sqlx::raw_sql("DROP SCHEMA IF EXISTS mnesis CASCADE; DROP VIEW IF EXISTS public.events")
        .execute(&pool)
        .await
        .unwrap();
    PostgresStore::migrate(&pool).await.unwrap();
    (PostgresStore::from_pool(pool.clone()).await.unwrap(), pool)
}

#[tokio::test]
async fn reopen_preserves_unsigned_extremes_and_unconditional_cache_replacement() {
    if std::env::var("DATABASE_URL").is_err() {
        return;
    }
    let (store, pool) = fixture().await;
    let id = StreamKey::from_slice(b"\0cache");
    let schema = NonZeroU32::new(u32::MAX).unwrap();
    let position = Version::new(u64::MAX).unwrap();
    store
        .commit(&id, schema, position, &Vec::new())
        .await
        .unwrap();
    drop(store);
    let reopened = PostgresStore::from_pool(pool).await.unwrap();
    assert_eq!(
        reopened.hydrate(&id, schema).await.unwrap(),
        Hydrated::Found {
            position,
            state: Vec::new()
        }
    );
    assert_eq!(
        reopened.hydrate(&id, NonZeroU32::MIN).await.unwrap(),
        Hydrated::Stale {
            stored_schema: schema
        }
    );
    reopened
        .commit(&id, schema, Version::INITIAL, &b"older cache".to_vec())
        .await
        .unwrap();
    assert_eq!(
        reopened.hydrate(&id, schema).await.unwrap(),
        Hydrated::Found {
            position: Version::INITIAL,
            state: b"older cache".to_vec()
        }
    );
}

#[tokio::test]
async fn byte_ceiling_precedes_storage_and_stale_bodies_are_not_fetched() {
    if std::env::var("DATABASE_URL").is_err() {
        return;
    }
    let (store, pool) = fixture().await;
    let id = StreamKey::from_slice(b"k");
    store
        .commit(
            &id,
            NonZeroU32::MIN,
            Version::INITIAL,
            &b"oversize".to_vec(),
        )
        .await
        .unwrap();
    let bounded = PostgresStore::from_pool_with_limits(
        pool.clone(),
        Limits {
            bytes: NonZeroU32::new(37).unwrap(),
            ..Limits::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        bounded
            .hydrate(&id, NonZeroU32::new(2).unwrap())
            .await
            .unwrap(),
        Hydrated::Stale {
            stored_schema: NonZeroU32::MIN
        }
    );
    assert!(matches!(
        bounded.hydrate(&id, NonZeroU32::MIN).await,
        Err(PostgresError::RowLimit {
            actual: 41,
            limit: 37
        })
    ));
    bounded
        .commit(&id, NonZeroU32::MIN, Version::INITIAL, &b"edge".to_vec())
        .await
        .unwrap();
    assert_eq!(
        bounded.hydrate(&id, NonZeroU32::MIN).await.unwrap(),
        Hydrated::Found {
            position: Version::INITIAL,
            state: b"edge".to_vec()
        }
    );
    pool.close().await;
    assert!(matches!(
        bounded
            .commit(&id, NonZeroU32::MIN, Version::INITIAL, &b"above".to_vec())
            .await,
        Err(PostgresError::RowLimit {
            actual: 38,
            limit: 37
        })
    ));
}

#[tokio::test]
async fn corrupt_position_is_an_error_even_when_the_requested_schema_is_stale() {
    if std::env::var("DATABASE_URL").is_err() {
        return;
    }
    let (store, pool) = fixture().await;
    let id = StreamKey::from_slice(b"corrupt");
    store
        .commit(&id, NonZeroU32::MIN, Version::INITIAL, &vec![1])
        .await
        .unwrap();
    sqlx::raw_sql("ALTER TABLE mnesis.snapshots DROP CONSTRAINT snapshots_version_check; UPDATE mnesis.snapshots SET version = 0")
        .execute(&pool).await.unwrap();
    let matching = store.hydrate(&id, NonZeroU32::MIN).await;
    let stale = store.hydrate(&id, NonZeroU32::new(2).unwrap()).await;
    sqlx::raw_sql("UPDATE mnesis.snapshots SET version = 1; ALTER TABLE mnesis.snapshots ADD CONSTRAINT snapshots_version_check CHECK (version > 0 AND version <= 18446744073709551615)")
        .execute(&pool).await.unwrap();
    assert!(matches!(matching, Err(PostgresError::CorruptRow { .. })));
    assert!(matches!(stale, Err(PostgresError::CorruptRow { .. })));
}

#[tokio::test]
async fn concurrent_hydration_sees_complete_old_or_new_pairs() {
    if std::env::var("DATABASE_URL").is_err() {
        return;
    }
    let (store, pool) = fixture().await;
    let id = StreamKey::from_slice(b"pair");
    store
        .commit(&id, NonZeroU32::MIN, Version::INITIAL, &b"old".to_vec())
        .await
        .unwrap();
    sqlx::raw_sql("CREATE OR REPLACE FUNCTION mnesis.snapshot_pause() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_advisory_xact_lock(319880); RETURN NEW; END $$; CREATE TRIGGER snapshot_pause AFTER UPDATE ON mnesis.snapshots FOR EACH ROW EXECUTE FUNCTION mnesis.snapshot_pause()")
        .execute(&pool).await.unwrap();
    let mut gate = pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(319880)")
        .execute(&mut *gate)
        .await
        .unwrap();
    let client = store.clone();
    let target = id.clone();
    let mut pending = JoinSet::new();
    pending.spawn(async move {
        client
            .commit(
                &target,
                NonZeroU32::MIN,
                Version::new(2).unwrap(),
                &b"new".to_vec(),
            )
            .await
    });
    wait_for_snapshot(&pool).await;
    let old = store.hydrate(&id, NonZeroU32::MIN).await;
    gate.commit().await.unwrap();
    pending.join_next().await.unwrap().unwrap().unwrap();
    sqlx::raw_sql(
        "DROP TRIGGER snapshot_pause ON mnesis.snapshots; DROP FUNCTION mnesis.snapshot_pause()",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        old.unwrap(),
        Hydrated::Found {
            position: Version::INITIAL,
            state: b"old".to_vec()
        }
    );
    assert_eq!(
        store.hydrate(&id, NonZeroU32::MIN).await.unwrap(),
        Hydrated::Found {
            position: Version::new(2).unwrap(),
            state: b"new".to_vec()
        }
    );
}

async fn wait_for_snapshot(pool: &sqlx::PgPool) {
    timeout(Duration::from_secs(3), async {
        loop {
            let waiting: bool = sqlx::query_scalar("SELECT EXISTS (SELECT FROM pg_locks l JOIN pg_stat_activity a ON a.pid = l.pid WHERE l.locktype = 'advisory' AND l.objid = 319880 AND NOT l.granted AND a.datname = current_database() AND a.application_name = $1 AND a.query LIKE 'INSERT INTO mnesis.snapshots %')")
                .bind(format!("mnesis_snapshot_{}", std::process::id())).fetch_one(pool).await.unwrap();
            if waiting { break; }
            yield_now().await;
        }
    }).await.expect("native snapshot UPDATE must block after staging its new pair");
}

mnesis_store_testing::conformance_snapshot! {
    factory: fixture,
    positions: (Version::new(5).unwrap(), Version::new(9).unwrap()),
    extremes: (Version::INITIAL, Version::new(u64::MAX).unwrap()),
    skip_unless: || std::env::var("DATABASE_URL").is_ok(),
}
