//! Native conditional projection-state contracts, separate from aggregate caches.
#![cfg(feature = "projection")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "native checkpoint contract assertions"
)]

use std::borrow::Cow;
use std::io::{Write, stdout};
use std::num::{NonZeroU32, NonZeroU64};
use std::process::Command;

#[cfg(feature = "snapshot")]
use mnesis::Version;
use mnesis_postgres::{Limits, PgAllPos, PostgresError, PostgresStore};
use mnesis_store::StreamKey;
use mnesis_store::checkpoint::{
    CheckpointError, CheckpointHydrated, CheckpointMode, CheckpointRejection, CheckpointStore,
    CheckpointWrite,
};
#[cfg(feature = "snapshot")]
use mnesis_store::state::{Hydrated, SnapshotStore};
use sqlx::PgPool;
use std::sync::atomic::Ordering;
use tokio::task::{JoinSet, yield_now};
use tokio::time::{Duration, timeout};

#[path = "support/memory.rs"]
mod memory;

#[path = "support/process.rs"]
mod process;

mod support;

type Outcome = Result<NonZeroU64, CheckpointError<PostgresError>>;

#[tokio::test]
async fn acknowledged_checkpoint_pair_survives_client_process_termination() {
    const CHILD: &str = "MNESIS_POSTGRES_CHECKPOINT_CHILD";
    const ACK: &str = "MNESIS_POSTGRES_CHECKPOINT_ACK";
    let Ok(url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let id = StreamKey::from_slice(b"crashed-checkpoint\0");
    let schema = NonZeroU32::new(u32::MAX).unwrap();
    let position = PgAllPos::new(u64::MAX, u64::MAX);
    let state = b"saved before client SIGKILL".to_vec();
    match std::env::var(CHILD).as_deref() {
        Ok("write") => {
            let store = PostgresStore::connect(&url).await.unwrap();
            let revision = store
                .commit_checkpoint(
                    &id,
                    CheckpointWrite {
                        expected: None,
                        schema_version: schema,
                        position,
                        state: &state,
                        mode: CheckpointMode::Advance,
                    },
                )
                .await
                .unwrap();
            assert_eq!(revision, NonZeroU64::MIN);
            writeln!(stdout().lock(), "\n{ACK}").unwrap();
            stdout().flush().unwrap();
            std::future::pending::<()>().await;
        }
        Err(std::env::VarError::NotPresent) => {
            let (store, pool) = fixture().await.unwrap();
            drop(store);
            pool.close().await;
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "acknowledged_checkpoint_pair_survives_client_process_termination",
                    "--nocapture",
                ])
                .env(CHILD, "write");
            process::kill_after_marker(&mut command, ACK);
            let reopened = PostgresStore::connect(&url).await.unwrap();
            assert_eq!(
                reopened.hydrate_checkpoint(&id, schema).await.unwrap(),
                CheckpointHydrated::Found {
                    revision: NonZeroU64::MIN,
                    position,
                    state
                }
            );
        }
        wrong => panic!("invalid process fixture phase: {wrong:?}"),
    }
}

#[tokio::test]
async fn runtime_checkpoint_writer_needs_no_key_mutation_event_rewrite_or_ddl() {
    let Some((_store, pool)) = fixture().await else {
        return;
    };
    sqlx::raw_sql("DROP ROLE IF EXISTS mnesis_checkpoint_writer; CREATE ROLE mnesis_checkpoint_writer LOGIN; GRANT USAGE ON SCHEMA mnesis TO mnesis_checkpoint_writer; GRANT SELECT ON mnesis._sqlx_migrations TO mnesis_checkpoint_writer; GRANT SELECT, INSERT ON mnesis.keys TO mnesis_checkpoint_writer; GRANT USAGE ON SEQUENCE mnesis.keys_number_seq TO mnesis_checkpoint_writer; GRANT SELECT ON mnesis.events TO mnesis_checkpoint_writer; GRANT SELECT, INSERT, UPDATE ON mnesis.checkpoints TO mnesis_checkpoint_writer")
        .execute(&pool).await.unwrap();
    let runtime = sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .connect_with(
            pool.connect_options()
                .as_ref()
                .clone()
                .username("mnesis_checkpoint_writer"),
        )
        .await
        .unwrap();
    let identity: (String, String) =
        sqlx::query_as("SELECT current_user::text, session_user::text")
            .fetch_one(&runtime)
            .await
            .unwrap();
    let store = PostgresStore::from_pool(runtime.clone()).await.unwrap();
    let id = StreamKey::from_slice(b"restricted-checkpoint");
    let first = store
        .commit_checkpoint(&id, advance(None, PgAllPos::new(1, 1), &vec![1]))
        .await;
    let second = store
        .commit_checkpoint(
            &id,
            advance(Some(NonZeroU64::MIN), PgAllPos::new(2, 0), &vec![2]),
        )
        .await;
    let saved = store.hydrate_checkpoint(&id, NonZeroU32::MIN).await;
    let mut codes = Vec::new();
    for statement in [
        "UPDATE mnesis.keys SET value = 'changed'::bytea",
        "UPDATE mnesis.events SET payload = 'changed'::bytea",
        "DELETE FROM mnesis.events",
        "DELETE FROM mnesis.checkpoints",
        "CREATE TABLE mnesis.forbidden (number int)",
    ] {
        codes.push(
            sqlx::raw_sql(statement)
                .execute(&runtime)
                .await
                .err()
                .and_then(|error| {
                    error
                        .as_database_error()
                        .and_then(sqlx::error::DatabaseError::code)
                        .map(Cow::into_owned)
                }),
        );
    }
    drop(store);
    runtime.close().await;
    sqlx::raw_sql("DROP OWNED BY mnesis_checkpoint_writer; DROP ROLE mnesis_checkpoint_writer")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        identity,
        (
            "mnesis_checkpoint_writer".to_owned(),
            "mnesis_checkpoint_writer".to_owned()
        )
    );
    assert_eq!(first.unwrap(), NonZeroU64::MIN);
    assert_eq!(second.unwrap().get(), 2);
    assert_eq!(
        saved.unwrap(),
        CheckpointHydrated::Found {
            revision: NonZeroU64::new(2).unwrap(),
            position: PgAllPos::new(2, 0),
            state: vec![2],
        }
    );
    assert_eq!(codes, vec![Some("42501".to_owned()); 5]);
}

const fn advance(
    expected: Option<NonZeroU64>,
    position: PgAllPos,
    state: &Vec<u8>,
) -> CheckpointWrite<'_, Vec<u8>, PgAllPos> {
    CheckpointWrite {
        expected,
        schema_version: NonZeroU32::MIN,
        position,
        state,
        mode: CheckpointMode::Advance,
    }
}

fn rejected(result: Outcome, expected: &CheckpointRejection) {
    match result {
        Err(CheckpointError::Rejected(actual)) => assert_eq!(&actual, expected),
        other => panic!("expected exact rejection {expected:?}, got {other:?}"),
    }
}

async fn fixture() -> Option<(PostgresStore, PgPool)> {
    let url = std::env::var("DATABASE_URL").ok()?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(10)
        .connect(&url)
        .await
        .unwrap();
    sqlx::raw_sql("DROP SCHEMA IF EXISTS mnesis CASCADE; DROP VIEW IF EXISTS public.events")
        .execute(&pool)
        .await
        .unwrap();
    PostgresStore::migrate(&pool).await.unwrap();
    Some((PostgresStore::from_pool(pool.clone()).await.unwrap(), pool))
}

#[tokio::test]
async fn missing_revision_and_invalid_rebuild_do_not_create_or_replace_state() {
    let Some((store, pool)) = fixture().await else {
        return;
    };
    let id = StreamKey::from_slice(b"invalid-rebuild");
    let original = vec![1];
    rejected(
        store
            .commit_checkpoint(
                &id,
                advance(Some(NonZeroU64::MIN), PgAllPos::new(0, 0), &original),
            )
            .await,
        &CheckpointRejection::Conflict {
            expected: Some(NonZeroU64::MIN),
            actual: None,
        },
    );
    let rebuild = |expected| CheckpointWrite {
        expected,
        schema_version: NonZeroU32::MIN,
        position: PgAllPos::new(0, 0),
        state: &original,
        mode: CheckpointMode::Rebuild,
    };
    rejected(
        store.commit_checkpoint(&id, rebuild(None)).await,
        &CheckpointRejection::InvalidRebuild,
    );
    assert_eq!(
        store
            .hydrate_checkpoint(&id, NonZeroU32::MIN)
            .await
            .unwrap(),
        CheckpointHydrated::Absent
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM mnesis.keys")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    let revision = store
        .commit_checkpoint(&id, advance(None, PgAllPos::new(1, 1), &original))
        .await
        .unwrap();
    rejected(
        store.commit_checkpoint(&id, rebuild(Some(revision))).await,
        &CheckpointRejection::InvalidRebuild,
    );
    assert_eq!(
        store
            .hydrate_checkpoint(&id, NonZeroU32::MIN)
            .await
            .unwrap(),
        CheckpointHydrated::Found {
            revision,
            position: PgAllPos::new(1, 1),
            state: original,
        }
    );
}

#[cfg(feature = "snapshot")]
#[tokio::test]
async fn aggregate_cache_and_checkpoint_with_identical_ids_remain_independent() {
    let Some((store, _pool)) = fixture().await else {
        return;
    };
    let id = StreamKey::from_slice(b"same\0id");
    store
        .commit(&id, NonZeroU32::MIN, Version::INITIAL, &vec![1])
        .await
        .unwrap();
    let first = PgAllPos::new(9, 1);
    let revision = store
        .commit_checkpoint(&id, advance(None, first, &vec![2]))
        .await
        .unwrap();
    store
        .commit(&id, NonZeroU32::MIN, Version::new(2).unwrap(), &vec![3])
        .await
        .unwrap();
    assert_eq!(
        store
            .hydrate_checkpoint(&id, NonZeroU32::MIN)
            .await
            .unwrap(),
        CheckpointHydrated::Found {
            revision,
            position: first,
            state: vec![2],
        }
    );
    let next = store
        .commit_checkpoint(&id, advance(Some(revision), PgAllPos::new(9, 2), &vec![4]))
        .await
        .unwrap();
    assert_eq!(next.get(), 2);
    assert_eq!(
        store.hydrate(&id, NonZeroU32::MIN).await.unwrap(),
        Hydrated::Found {
            position: Version::new(2).unwrap(),
            state: vec![3],
        }
    );
}

async fn bounded(pool: PgPool) -> PostgresStore {
    PostgresStore::from_pool_with_limits(
        pool,
        Limits {
            bytes: NonZeroU32::new(37).unwrap(),
            ..Limits::default()
        },
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn byte_edges_and_oversized_input_preserve_the_checkpoint_before_storage() {
    let Some((store, pool)) = fixture().await else {
        return;
    };
    let id = StreamKey::from_slice(b"k");
    let revision = store
        .commit_checkpoint(&id, advance(None, PgAllPos::new(1, 1), &b"old".to_vec()))
        .await
        .unwrap();
    let limited = bounded(pool.clone()).await;
    assert!(matches!(
        limited
            .commit_checkpoint(
                &id,
                advance(Some(revision), PgAllPos::new(2, 0), &b"above".to_vec())
            )
            .await,
        Err(CheckpointError::Store(PostgresError::RowLimit {
            actual: 38,
            limit: 37
        }))
    ));
    assert_eq!(
        limited
            .hydrate_checkpoint(&id, NonZeroU32::MIN)
            .await
            .unwrap(),
        CheckpointHydrated::Found {
            revision,
            position: PgAllPos::new(1, 1),
            state: b"old".to_vec(),
        }
    );
    let next = limited
        .commit_checkpoint(
            &id,
            advance(Some(revision), PgAllPos::new(2, 0), &b"edge".to_vec()),
        )
        .await
        .unwrap();
    assert_eq!(next.get(), 2);
    assert_eq!(
        limited
            .hydrate_checkpoint(&id, NonZeroU32::MIN)
            .await
            .unwrap(),
        CheckpointHydrated::Found {
            revision: next,
            position: PgAllPos::new(2, 0),
            state: b"edge".to_vec(),
        }
    );
    pool.close().await;
    assert!(matches!(
        limited
            .commit_checkpoint(
                &id,
                advance(Some(next), PgAllPos::new(3, 0), &b"above".to_vec())
            )
            .await,
        Err(CheckpointError::Store(PostgresError::RowLimit {
            actual: 38,
            limit: 37
        }))
    ));
}

#[tokio::test]
async fn stale_and_oversized_reads_omit_the_megabyte_checkpoint_body() {
    let Some((store, pool)) = fixture().await else {
        return;
    };
    let id = StreamKey::from_slice(b"k");
    let revision = store
        .commit_checkpoint(&id, advance(None, PgAllPos::new(1, 1), &vec![7; 1_048_576]))
        .await
        .unwrap();
    let limited = bounded(pool).await;
    limited
        .hydrate_checkpoint(&StreamKey::from_slice(b"missing"), NonZeroU32::MIN)
        .await
        .unwrap();
    let before = memory::TOTAL.load(Ordering::Relaxed);
    let mismatch = limited
        .hydrate_checkpoint(&id, NonZeroU32::new(2).unwrap())
        .await;
    let mismatched = memory::TOTAL
        .load(Ordering::Relaxed)
        .checked_sub(before)
        .unwrap();
    assert_eq!(
        mismatch.unwrap(),
        CheckpointHydrated::Stale {
            revision,
            stored_schema: NonZeroU32::MIN
        }
    );
    assert!(
        mismatched < 262_144,
        "stale checkpoint allocated {mismatched} bytes"
    );
    let matching_before = memory::TOTAL.load(Ordering::Relaxed);
    let oversized = limited.hydrate_checkpoint(&id, NonZeroU32::MIN).await;
    let matching = memory::TOTAL
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
        matching < 262_144,
        "oversized checkpoint allocated {matching} bytes"
    );
}

#[tokio::test]
async fn stale_schema_and_position_rejections_preserve_the_complete_pair() {
    let Some((store, _pool)) = fixture().await else {
        return;
    };
    let id = StreamKey::from_slice(b"projection\0");
    let initial = vec![1];
    let latest = vec![2];
    let first = store
        .commit_checkpoint(&id, advance(None, PgAllPos::new(0, 0), &initial))
        .await
        .unwrap();
    assert_eq!(first, NonZeroU64::MIN);
    let position = PgAllPos::new(1, 9);
    let second = store
        .commit_checkpoint(&id, advance(Some(first), position, &latest))
        .await
        .unwrap();
    assert_eq!(second.get(), 2);
    for expected in [None, Some(first)] {
        rejected(
            store
                .commit_checkpoint(&id, advance(expected, PgAllPos::new(2, 0), &initial))
                .await,
            &CheckpointRejection::Conflict {
                expected,
                actual: Some(second),
            },
        );
    }
    for previous in [position, PgAllPos::new(1, 8), PgAllPos::new(0, u64::MAX)] {
        rejected(
            store
                .commit_checkpoint(&id, advance(Some(second), previous, &initial))
                .await,
            &CheckpointRejection::NonIncreasingPosition,
        );
    }
    assert_eq!(
        store
            .hydrate_checkpoint(&id, NonZeroU32::MIN)
            .await
            .unwrap(),
        CheckpointHydrated::Found {
            revision: second,
            position,
            state: latest,
        }
    );
}

#[tokio::test]
async fn explicit_schema_rebuild_preserves_revision_and_rejects_stale_replacement() {
    let Some((store, _pool)) = fixture().await else {
        return;
    };
    let id = StreamKey::from_slice(b"rebuild");
    let initial = vec![1];
    let first = store
        .commit_checkpoint(&id, advance(None, PgAllPos::new(9, 9), &initial))
        .await
        .unwrap();
    let schema = NonZeroU32::new(2).unwrap();
    assert_eq!(
        store.hydrate_checkpoint(&id, schema).await.unwrap(),
        CheckpointHydrated::Stale {
            revision: first,
            stored_schema: NonZeroU32::MIN,
        }
    );
    let state = vec![2];
    let proposal = |mode| CheckpointWrite {
        expected: Some(first),
        schema_version: schema,
        position: PgAllPos::new(0, 0),
        state: &state,
        mode,
    };
    rejected(
        store
            .commit_checkpoint(&id, proposal(CheckpointMode::Advance))
            .await,
        &CheckpointRejection::SchemaMismatch {
            stored: NonZeroU32::MIN,
            requested: schema,
        },
    );
    let next = store
        .commit_checkpoint(&id, proposal(CheckpointMode::Rebuild))
        .await
        .unwrap();
    assert_eq!(next.get(), 2);
    rejected(
        store
            .commit_checkpoint(&id, proposal(CheckpointMode::Rebuild))
            .await,
        &CheckpointRejection::Conflict {
            expected: Some(first),
            actual: Some(next),
        },
    );
    assert_eq!(
        store.hydrate_checkpoint(&id, schema).await.unwrap(),
        CheckpointHydrated::Found {
            revision: next,
            position: PgAllPos::new(0, 0),
            state,
        }
    );
}

#[tokio::test]
async fn reopen_preserves_unsigned_extremes_empty_state_and_revision_overflow() {
    let Some((store, pool)) = fixture().await else {
        return;
    };
    let id = StreamKey::from_slice(b"maximum\0");
    let schema = NonZeroU32::new(u32::MAX).unwrap();
    let position = PgAllPos::new(u64::MAX, u64::MAX);
    let state = Vec::new();
    let revision = store
        .commit_checkpoint(
            &id,
            CheckpointWrite {
                expected: None,
                schema_version: schema,
                position,
                state: &state,
                mode: CheckpointMode::Advance,
            },
        )
        .await
        .unwrap();
    assert_eq!(revision, NonZeroU64::MIN);
    drop(store);
    let reopened = PostgresStore::from_pool(pool.clone()).await.unwrap();
    assert_eq!(
        reopened.hydrate_checkpoint(&id, schema).await.unwrap(),
        CheckpointHydrated::Found {
            revision,
            position,
            state
        }
    );
    sqlx::query("UPDATE mnesis.checkpoints SET revision = 18446744073709551615")
        .execute(&pool)
        .await
        .unwrap();
    let maximum = NonZeroU64::new(u64::MAX).unwrap();
    let replacement = vec![3];
    rejected(
        reopened
            .commit_checkpoint(
                &id,
                CheckpointWrite {
                    expected: Some(maximum),
                    schema_version: NonZeroU32::MIN,
                    position: PgAllPos::new(0, 0),
                    state: &replacement,
                    mode: CheckpointMode::Rebuild,
                },
            )
            .await,
        &CheckpointRejection::RevisionOverflow,
    );
    assert_eq!(
        reopened.hydrate_checkpoint(&id, schema).await.unwrap(),
        CheckpointHydrated::Found {
            revision: maximum,
            position,
            state: Vec::new(),
        }
    );
}

#[tokio::test]
async fn corrupt_metadata_is_not_stale_and_cannot_be_replaced() {
    let Some((store, pool)) = fixture().await else {
        return;
    };
    let id = StreamKey::from_slice(b"corrupt-revision");
    let state = vec![7];
    store
        .commit_checkpoint(&id, advance(None, PgAllPos::new(1, 1), &state))
        .await
        .unwrap();
    for (mutation, repair) in [
        (
            "ALTER TABLE mnesis.checkpoints DROP CONSTRAINT checkpoints_revision_check; UPDATE mnesis.checkpoints SET revision = 0",
            "UPDATE mnesis.checkpoints SET revision = 1; ALTER TABLE mnesis.checkpoints ADD CONSTRAINT checkpoints_revision_check CHECK (revision > 0 AND revision <= 18446744073709551615)",
        ),
        (
            "ALTER TABLE mnesis.checkpoints DROP CONSTRAINT checkpoints_revision_check; UPDATE mnesis.checkpoints SET revision = 18446744073709551616",
            "UPDATE mnesis.checkpoints SET revision = 1; ALTER TABLE mnesis.checkpoints ADD CONSTRAINT checkpoints_revision_check CHECK (revision > 0 AND revision <= 18446744073709551615)",
        ),
        (
            "ALTER TABLE mnesis.checkpoints DROP CONSTRAINT checkpoints_schema_version_check; UPDATE mnesis.checkpoints SET schema_version = 0",
            "UPDATE mnesis.checkpoints SET schema_version = 1; ALTER TABLE mnesis.checkpoints ADD CONSTRAINT checkpoints_schema_version_check CHECK (schema_version > 0 AND schema_version <= 4294967295)",
        ),
        (
            "ALTER TABLE mnesis.checkpoints DROP CONSTRAINT checkpoints_schema_version_check; UPDATE mnesis.checkpoints SET schema_version = -1",
            "UPDATE mnesis.checkpoints SET schema_version = 1; ALTER TABLE mnesis.checkpoints ADD CONSTRAINT checkpoints_schema_version_check CHECK (schema_version > 0 AND schema_version <= 4294967295)",
        ),
        (
            "ALTER TABLE mnesis.checkpoints DROP CONSTRAINT checkpoints_schema_version_check; UPDATE mnesis.checkpoints SET schema_version = 4294967296",
            "UPDATE mnesis.checkpoints SET schema_version = 1; ALTER TABLE mnesis.checkpoints ADD CONSTRAINT checkpoints_schema_version_check CHECK (schema_version > 0 AND schema_version <= 4294967295)",
        ),
        (
            "ALTER TABLE mnesis.checkpoints DROP CONSTRAINT checkpoints_txid_check; UPDATE mnesis.checkpoints SET txid = -1",
            "UPDATE mnesis.checkpoints SET txid = 1; ALTER TABLE mnesis.checkpoints ADD CONSTRAINT checkpoints_txid_check CHECK (txid >= 0 AND txid <= 18446744073709551615)",
        ),
        (
            "ALTER TABLE mnesis.checkpoints DROP CONSTRAINT checkpoints_txid_check; UPDATE mnesis.checkpoints SET txid = 18446744073709551616",
            "UPDATE mnesis.checkpoints SET txid = 1; ALTER TABLE mnesis.checkpoints ADD CONSTRAINT checkpoints_txid_check CHECK (txid >= 0 AND txid <= 18446744073709551615)",
        ),
        (
            "ALTER TABLE mnesis.checkpoints DROP CONSTRAINT checkpoints_global_seq_check; UPDATE mnesis.checkpoints SET global_seq = -1",
            "UPDATE mnesis.checkpoints SET global_seq = 1; ALTER TABLE mnesis.checkpoints ADD CONSTRAINT checkpoints_global_seq_check CHECK (global_seq >= 0 AND global_seq <= 18446744073709551615)",
        ),
        (
            "ALTER TABLE mnesis.checkpoints DROP CONSTRAINT checkpoints_global_seq_check; UPDATE mnesis.checkpoints SET global_seq = 18446744073709551616",
            "UPDATE mnesis.checkpoints SET global_seq = 1; ALTER TABLE mnesis.checkpoints ADD CONSTRAINT checkpoints_global_seq_check CHECK (global_seq >= 0 AND global_seq <= 18446744073709551615)",
        ),
    ] {
        corrupt_checkpoint(&store, &pool, &id, mutation, repair).await;
    }
    assert_eq!(
        store
            .hydrate_checkpoint(&id, NonZeroU32::MIN)
            .await
            .unwrap(),
        CheckpointHydrated::Found {
            revision: NonZeroU64::MIN,
            position: PgAllPos::new(1, 1),
            state
        }
    );
}

async fn corrupt_checkpoint(
    store: &PostgresStore,
    pool: &PgPool,
    id: &StreamKey,
    mutation: &'static str,
    repair: &'static str,
) {
    sqlx::raw_sql(mutation).execute(pool).await.unwrap();
    let before: (String, i64, String, String, Vec<u8>) = sqlx::query_as(
        "SELECT revision::text, schema_version, txid::text, global_seq::text, state FROM mnesis.checkpoints",
    ).fetch_one(pool).await.unwrap();
    let matching = store.hydrate_checkpoint(id, NonZeroU32::MIN).await;
    let mismatch = store
        .hydrate_checkpoint(id, NonZeroU32::new(2).unwrap())
        .await;
    let write = store
        .commit_checkpoint(id, advance(None, PgAllPos::new(2, 0), &vec![9]))
        .await;
    let reopened = PostgresStore::from_pool(pool.clone()).await;
    let unchanged: (String, i64, String, String, Vec<u8>) = sqlx::query_as(
        "SELECT revision::text, schema_version, txid::text, global_seq::text, state FROM mnesis.checkpoints",
    ).fetch_one(pool).await.unwrap();
    sqlx::raw_sql(repair).execute(pool).await.unwrap();
    assert!(matches!(matching, Err(PostgresError::CorruptRow { .. })));
    assert!(matches!(mismatch, Err(PostgresError::CorruptRow { .. })));
    assert!(matches!(
        write,
        Err(CheckpointError::Store(PostgresError::CorruptRow { .. }))
    ));
    assert_eq!(unchanged, before);
    assert!(matches!(reopened, Err(PostgresError::Schema { .. })));
}

#[tokio::test]
async fn suppressed_creation_and_update_fail_without_fabricating_a_revision_conflict() {
    let Some((store, pool)) = fixture().await else {
        return;
    };
    let existing = StreamKey::from_slice(b"existing-checkpoint");
    let missing = StreamKey::from_slice(b"missing-checkpoint");
    let original = vec![1];
    let position = PgAllPos::new(1, 1);
    let revision = store
        .commit_checkpoint(&existing, advance(None, position, &original))
        .await
        .unwrap();
    sqlx::raw_sql("CREATE FUNCTION mnesis.checkpoint_skip() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RETURN NULL; END $$")
        .execute(&pool).await.unwrap();
    let mut outcomes = Vec::new();
    for (statement, id, expected) in [
        (
            "CREATE TRIGGER checkpoint_skip BEFORE INSERT ON mnesis.checkpoints FOR EACH ROW EXECUTE FUNCTION mnesis.checkpoint_skip()",
            &missing,
            None,
        ),
        (
            "CREATE TRIGGER checkpoint_skip BEFORE UPDATE ON mnesis.checkpoints FOR EACH ROW EXECUTE FUNCTION mnesis.checkpoint_skip()",
            &existing,
            Some(revision),
        ),
    ] {
        sqlx::raw_sql(statement).execute(&pool).await.unwrap();
        outcomes.push(
            store
                .commit_checkpoint(id, advance(expected, PgAllPos::new(2, 0), &vec![2]))
                .await,
        );
        sqlx::raw_sql("DROP TRIGGER checkpoint_skip ON mnesis.checkpoints")
            .execute(&pool)
            .await
            .unwrap();
    }
    sqlx::raw_sql("DROP FUNCTION mnesis.checkpoint_skip()")
        .execute(&pool)
        .await
        .unwrap();
    let absent = store
        .hydrate_checkpoint(&missing, NonZeroU32::MIN)
        .await
        .unwrap();
    let unchanged = store
        .hydrate_checkpoint(&existing, NonZeroU32::MIN)
        .await
        .unwrap();
    let keys: i64 = sqlx::query_scalar("SELECT count(*) FROM mnesis.keys")
        .fetch_one(&pool)
        .await
        .unwrap();
    for result in outcomes {
        assert!(
            matches!(
                result,
                Err(CheckpointError::Store(PostgresError::Sqlx(
                    sqlx::Error::RowNotFound
                )))
            ),
            "zero-row write without a winner must be a storage failure: {result:?}"
        );
    }
    assert_eq!(absent, CheckpointHydrated::Absent);
    assert_eq!(
        unchanged,
        CheckpointHydrated::Found {
            revision,
            position,
            state: original
        }
    );
    assert_eq!(keys, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn overlapping_creators_and_advancers_have_one_complete_winner() {
    let Some((store, pool)) = fixture().await else {
        return;
    };
    let id = StreamKey::from_slice(b"racing-checkpoint");
    support::key(&pool, id.as_bytes()).await;
    sqlx::raw_sql("CREATE FUNCTION mnesis.checkpoint_pause() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_advisory_xact_lock(319881); RETURN NEW; END $$; CREATE TRIGGER checkpoint_pause BEFORE INSERT OR UPDATE ON mnesis.checkpoints FOR EACH ROW EXECUTE FUNCTION mnesis.checkpoint_pause()")
        .execute(&pool).await.unwrap();
    let first = race(&store, &pool, &id, None, [1, 2]).await;
    assert_eq!(first.get(), 1);
    let next = race(&store, &pool, &id, Some(first), [3, 4]).await;
    assert_eq!(next.get(), 2);
    drop(store);
    let reopened = PostgresStore::from_pool(pool).await.unwrap();
    let saved = reopened
        .hydrate_checkpoint(&id, NonZeroU32::MIN)
        .await
        .unwrap();
    let CheckpointHydrated::Found {
        revision,
        position,
        state,
    } = saved
    else {
        panic!("reopen must retain the complete winning record");
    };
    assert_eq!(revision, next);
    assert!(position == PgAllPos::new(3, 0) || position == PgAllPos::new(4, 0));
    assert_eq!(state, position.txid().to_be_bytes());
}

async fn race(
    store: &PostgresStore,
    pool: &PgPool,
    id: &StreamKey,
    expected: Option<NonZeroU64>,
    positions: [u64; 2],
) -> NonZeroU64 {
    let mut gate = pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(319881)")
        .execute(&mut *gate)
        .await
        .unwrap();
    let blocker: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *gate)
        .await
        .unwrap();
    let mut writers = JoinSet::new();
    for value in positions {
        let client = store.clone();
        let target = id.clone();
        writers.spawn(async move {
            let state = value.to_be_bytes().to_vec();
            let position = PgAllPos::new(value, 0);
            (
                client
                    .commit_checkpoint(&target, advance(expected, position, &state))
                    .await,
                position,
                state,
            )
        });
    }
    timeout(Duration::from_secs(3), async {
        loop {
            let (direct, total): (i64, i64) = sqlx::query_as(
                "WITH waits AS (SELECT pid, pg_blocking_pids(pid) AS blockers FROM pg_stat_activity \
                    WHERE datname = current_database() AND wait_event_type = 'Lock') \
                 SELECT count(*) FILTER (WHERE $1 = ANY(w.blockers)), count(*) FROM waits w \
                 WHERE $1 = ANY(w.blockers) OR EXISTS \
                    (SELECT FROM waits parent WHERE parent.pid = ANY(w.blockers) AND $1 = ANY(parent.blockers))",
            ).bind(blocker).fetch_one(pool).await.unwrap();
            match expected {
                None if direct == 2 => break,
                Some(_) if total == 2 => break,
                _ => yield_now().await,
            }
        }
    }).await.expect("both actual checkpoint writes must overlap before release");
    gate.rollback().await.unwrap();
    let left = writers.join_next().await.unwrap().unwrap();
    let right = writers.join_next().await.unwrap().unwrap();
    let (winner, loser) = match (left, right) {
        ((Ok(revision), position, state), (failure, _, _))
        | ((failure, _, _), (Ok(revision), position, state)) => {
            ((revision, position, state), failure)
        }
        other => panic!("exactly one checkpoint CAS must win: {other:?}"),
    };
    rejected(
        loser,
        &CheckpointRejection::Conflict {
            expected,
            actual: Some(winner.0),
        },
    );
    assert_eq!(
        store.hydrate_checkpoint(id, NonZeroU32::MIN).await.unwrap(),
        CheckpointHydrated::Found {
            revision: winner.0,
            position: winner.1,
            state: winner.2
        }
    );
    winner.0
}
