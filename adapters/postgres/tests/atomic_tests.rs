//! Real-database atomic multi-stream append laws.
#![cfg(feature = "import")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "native test assertions"
)]

use futures::TryStreamExt;
use mnesis::Version;
use mnesis_postgres::PostgresStore;
use mnesis_store::envelope::pending_envelope;
use mnesis_store::import::{
    AbortReason, AtomicAppend, AtomicAppendError, Atomicity, EventImporter, ImportBlock,
    ImportError, PlannedAppend, StreamOutcome, StreamReport,
};
use mnesis_store::{PendingBatch, RawEventStore, Store, StreamKey};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use tokio::time::{Duration, timeout};

#[path = "support/sections.rs"]
mod sections;

#[tokio::test]
async fn import_policies_preserve_final_conflict_and_exact_partial_reports() {
    if std::env::var("DATABASE_URL").is_err() {
        return;
    }
    for policy in [Atomicity::WholeChunk, Atomicity::PerStream] {
        let (raw, pool) = fixture().await;
        let input = sections::pair();
        let occupied = StreamKey::from_slice(&input[1].origin);
        let seed = run("occupied");
        raw.append(&occupied, None, PendingBatch::from_parts(&seed.head, &[]))
            .await
            .unwrap();
        let store = Store::new(raw);
        let result = store.import(&input, StreamKey::from_slice, policy).await;
        let rows: Vec<_> = sqlx::query_as::<_, (Vec<u8>, String)>(
            "SELECT k.value, e.version::text FROM mnesis.events e JOIN mnesis.keys k ON k.number = e.stream_id ORDER BY k.value, e.version",
        ).fetch_all(&pool).await.unwrap();
        match policy {
            Atomicity::WholeChunk => {
                assert!(
                    matches!(result, Err(ImportError::Aborted { stream, reason: AbortReason::Mismatch { expected, got } })
                    if stream == occupied && expected == Version::new(2).unwrap() && got == Version::INITIAL)
                );
                assert_eq!(rows, [(input[1].origin.to_vec(), "1".to_owned())]);
            }
            Atomicity::PerStream => {
                assert_eq!(
                    result.unwrap().streams(),
                    [
                        StreamReport {
                            stream: StreamKey::from_slice(&input[0].origin),
                            outcome: StreamOutcome::Complete {
                                version: Version::new(2).unwrap()
                            }
                        },
                        StreamReport {
                            stream: occupied,
                            outcome: StreamOutcome::Mismatch {
                                reached: None,
                                got: Version::INITIAL
                            }
                        },
                    ]
                );
                assert_eq!(
                    rows,
                    [
                        (input[1].origin.to_vec(), "1".to_owned()),
                        (input[0].origin.to_vec(), "1".to_owned()),
                        (input[0].origin.to_vec(), "2".to_owned()),
                    ]
                );
            }
        }
    }
}

#[tokio::test]
async fn import_corrupt_prefix_reports_exact_persisted_progress() {
    if std::env::var("DATABASE_URL").is_err() {
        return;
    }
    let (raw, pool) = fixture().await;
    let store = Store::new(raw);
    let mut input = sections::pair();
    input[0].blocks.insert(1, ImportBlock::Corrupt);
    let result = store
        .import(&input, StreamKey::from_slice, Atomicity::WholeChunk)
        .await;
    assert!(
        matches!(result, Err(ImportError::Aborted { stream, reason: AbortReason::Corrupt }) if stream.as_bytes() == b"first")
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM mnesis.events")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    let report = store
        .import(&input, StreamKey::from_slice, Atomicity::PerStream)
        .await
        .unwrap();
    assert_eq!(
        report.streams(),
        [
            StreamReport {
                stream: StreamKey::from_bytes("first"),
                outcome: StreamOutcome::Corrupt {
                    reached: Some(Version::INITIAL)
                }
            },
            StreamReport {
                stream: StreamKey::from_slice(&input[1].origin),
                outcome: StreamOutcome::Complete {
                    version: Version::new(2).unwrap()
                }
            },
        ]
    );
    let rows: Vec<_> = store
        .read_all(None)
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(
        rows.iter()
            .map(|row| (&row.1, row.2.version().as_u64()))
            .collect::<Vec<_>>(),
        [
            (&StreamKey::from_bytes("first"), 1),
            (&StreamKey::from_slice(&input[1].origin), 1),
            (&StreamKey::from_slice(&input[1].origin), 2),
        ]
    );
    assert!(rows.iter().all(|row| row.2.event_type() == "Import\0"
        && row.2.payload() == b"payload"
        && row.2.metadata() == Some(b"metadata".as_slice())));
}

#[tokio::test]
async fn import_duplicate_routes_reject_before_any_catalog_or_event_write() {
    if std::env::var("DATABASE_URL").is_err() {
        return;
    }
    let (raw, pool) = fixture().await;
    let store = Store::new(raw);
    for policy in [Atomicity::WholeChunk, Atomicity::PerStream] {
        let result = store
            .import(&sections::pair(), |_| StreamKey::from_bytes("new"), policy)
            .await;
        assert!(matches!(result, Err(ImportError::InvalidRoute(_))));
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM mnesis.events")
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM mnesis.keys")
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );
    }
}

#[tokio::test]
async fn import_storage_error_preserves_only_previous_per_stream_transactions() {
    if std::env::var("DATABASE_URL").is_err() {
        return;
    }
    for policy in [Atomicity::WholeChunk, Atomicity::PerStream] {
        let (raw, pool) = fixture().await;
        sqlx::raw_sql("CREATE OR REPLACE FUNCTION mnesis.atomic_reject() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.version = 2 AND NEW.stream_id = (SELECT number FROM mnesis.keys WHERE value = decode('007365636f6e64', 'hex')) THEN RAISE EXCEPTION 'final import rejection' USING ERRCODE = '23514'; END IF; RETURN NEW; END $$; CREATE TRIGGER atomic_reject BEFORE INSERT ON mnesis.events FOR EACH ROW EXECUTE FUNCTION mnesis.atomic_reject()")
            .execute(&pool).await.unwrap();
        let store = Store::new(raw);
        let input = sections::pair();
        let result = store.import(&input, StreamKey::from_slice, policy).await;
        sqlx::query("DROP TRIGGER atomic_reject ON mnesis.events")
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            matches!(result, Err(ImportError::Store(mnesis_postgres::PostgresError::Sqlx(ref source)))
            if source.as_database_error().and_then(sqlx::error::DatabaseError::code).as_deref() == Some("23514"))
        );
        let rows: Vec<(Vec<u8>, String)> = sqlx::query_as("SELECT k.value, e.version::text FROM mnesis.events e JOIN mnesis.keys k ON k.number = e.stream_id ORDER BY e.global_seq")
            .fetch_all(&pool).await.unwrap();
        match policy {
            Atomicity::WholeChunk => assert_eq!(rows, Vec::<(Vec<u8>, String)>::new()),
            Atomicity::PerStream => assert_eq!(
                rows,
                [
                    (input[0].origin.to_vec(), "1".to_owned()),
                    (input[0].origin.to_vec(), "2".to_owned())
                ]
            ),
        }
        let keys: Vec<Vec<u8>> = sqlx::query_scalar("SELECT value FROM mnesis.keys ORDER BY value")
            .fetch_all(&pool)
            .await
            .unwrap();
        match policy {
            Atomicity::WholeChunk => assert_eq!(keys, Vec::<Vec<u8>>::new()),
            Atomicity::PerStream => assert_eq!(keys, [input[0].origin.to_vec()]),
        }
    }
}

async fn fixture() -> (PostgresStore, sqlx::PgPool) {
    let url = std::env::var("DATABASE_URL").unwrap();
    let options = url
        .parse::<PgConnectOptions>()
        .unwrap()
        .application_name(&format!("mnesis_atomic_{}", std::process::id()));
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect_with(options)
        .await
        .unwrap();
    PostgresStore::migrate(&pool).await.unwrap();
    sqlx::raw_sql("DROP TRIGGER IF EXISTS atomic_pause ON mnesis.events; DROP TRIGGER IF EXISTS atomic_reject ON mnesis.events; ALTER TABLE mnesis.events DROP CONSTRAINT IF EXISTS atomic_payload_key")
        .execute(&pool).await.unwrap();
    sqlx::query("TRUNCATE mnesis.events, mnesis.snapshots, mnesis.checkpoints, mnesis.keys RESTART IDENTITY")
        .execute(&pool)
        .await
        .unwrap();
    (PostgresStore::from_pool(pool.clone()).await.unwrap(), pool)
}

fn run(id: &str) -> PlannedAppend {
    let make = |version| {
        pending_envelope(Version::new(version).unwrap())
            .event_type("Atomic")
            .payload(b"value".as_slice())
            .build()
            .unwrap()
    };
    PlannedAppend {
        target: StreamKey::from_bytes(id.to_owned()),
        expected_version: None,
        head: make(1),
        tail: vec![make(2)],
    }
}

#[tokio::test]
async fn reopen_preserves_every_run_and_the_highest_position() {
    if std::env::var("DATABASE_URL").is_err() {
        return;
    }
    let (store, pool) = fixture().await;
    let writes = [run("first"), run("second")];
    let position = store.atomic_append_many(&writes).await.unwrap().unwrap();
    drop(store);
    let reopened = PostgresStore::from_pool(pool).await.unwrap();
    let rows: Vec<_> = reopened
        .read_all(None)
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(rows.len(), 4);
    assert_eq!(rows.last().unwrap().0, position);
    assert!(rows.iter().all(|row| row.0.txid() == position.txid()));
    assert_eq!(
        rows.iter()
            .map(|row| (&row.1, row.2.version().as_u64()))
            .collect::<Vec<_>>(),
        [
            (&writes[0].target, 1),
            (&writes[0].target, 2),
            (&writes[1].target, 1),
            (&writes[1].target, 2)
        ]
    );
    assert!(rows.iter().all(|row| row.2.payload() == b"value"));
    assert!(
        reopened
            .read_all(Some(position))
            .await
            .unwrap()
            .try_collect::<Vec<_>>()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn rejected_final_insert_rolls_back_all_events_and_new_keys() {
    if std::env::var("DATABASE_URL").is_err() {
        return;
    }
    let (store, pool) = fixture().await;
    sqlx::raw_sql("CREATE OR REPLACE FUNCTION mnesis.atomic_reject() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.event_type = 'Reject'::bytea THEN RAISE EXCEPTION 'native atomic rejection' USING ERRCODE = '23514'; END IF; RETURN NEW; END $$; CREATE TRIGGER atomic_reject BEFORE INSERT ON mnesis.events FOR EACH ROW EXECUTE FUNCTION mnesis.atomic_reject()")
        .execute(&pool).await.unwrap();
    let mut last = run("second");
    last.tail[0] = pending_envelope(Version::new(2).unwrap())
        .event_type("Reject")
        .payload(b"rejected".as_slice())
        .build()
        .unwrap();
    let result = store.atomic_append_many(&[run("first"), last]).await;
    sqlx::query("DROP TRIGGER atomic_reject ON mnesis.events")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        matches!(result, Err(AtomicAppendError::Store(mnesis_postgres::PostgresError::Sqlx(ref error)))
        if error.as_database_error().and_then(sqlx::error::DatabaseError::code).as_deref() == Some("23514"))
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM mnesis.events")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM mnesis.keys")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn readers_cannot_observe_the_staged_prefix() {
    if std::env::var("DATABASE_URL").is_err() {
        return;
    }
    let (store, pool) = fixture().await;
    sqlx::raw_sql("CREATE OR REPLACE FUNCTION mnesis.atomic_pause() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.version = 2 THEN PERFORM pg_advisory_xact_lock(319875); END IF; RETURN NEW; END $$; CREATE TRIGGER atomic_pause AFTER INSERT ON mnesis.events FOR EACH ROW EXECUTE FUNCTION mnesis.atomic_pause()")
        .execute(&pool).await.unwrap();
    let mut held = pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(319875)")
        .execute(&mut *held)
        .await
        .unwrap();
    let writer = store.clone();
    let append = tokio::spawn(async move {
        writer
            .atomic_append_many(&[run("first"), run("second")])
            .await
    });
    timeout(Duration::from_secs(3), async {
        loop {
            let waiting: bool = sqlx::query_scalar("SELECT EXISTS (SELECT FROM pg_locks l JOIN pg_stat_activity a ON a.pid = l.pid WHERE l.locktype = 'advisory' AND l.objid = 319875 AND NOT l.granted AND a.datname = current_database() AND a.application_name = $1 AND a.query LIKE 'INSERT INTO %events %')")
                .bind(format!("mnesis_atomic_{}", std::process::id())).fetch_one(&pool).await.unwrap();
            if waiting { break }
            tokio::task::yield_now().await;
        }
    }).await.expect("writer must reach the AFTER INSERT trigger on its second row");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM mnesis.events")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert!(
        store
            .read_all(None)
            .await
            .unwrap()
            .try_collect::<Vec<_>>()
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .read_stream(&StreamKey::from_bytes("first"), Version::INITIAL)
            .await
            .unwrap()
            .try_collect::<Vec<_>>()
            .await
            .unwrap()
            .is_empty()
    );
    held.commit().await.unwrap();
    let assigned = timeout(Duration::from_secs(3), append)
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .unwrap();
    sqlx::query("DROP TRIGGER atomic_pause ON mnesis.events")
        .execute(&pool)
        .await
        .unwrap();
    let rows: Vec<_> = store
        .read_all(None)
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(rows.len(), 4);
    assert_eq!(rows.last().unwrap().0, assigned);
    assert!(rows.iter().all(|row| row.0.txid() == assigned.txid()));
}

#[tokio::test]
async fn invalid_requests_are_rejected_before_a_closed_pool_is_touched() {
    if std::env::var("DATABASE_URL").is_err() {
        return;
    }
    let (original, pool) = fixture().await;
    drop(original);
    let limits = mnesis_postgres::Limits {
        rows: mnesis_store::BatchSize::DEFAULT,
        bytes: std::num::NonZeroU32::new(64).unwrap(),
    };
    let store = PostgresStore::from_pool_with_limits(pool.clone(), limits)
        .await
        .unwrap();
    pool.close().await;
    assert!(store.atomic_append_many(&[]).await.unwrap().is_none());
    assert!(matches!(
        store.atomic_append_many(&[run("same"), run("same")]).await,
        Err(AtomicAppendError::InvalidRoute(_))
    ));
    let mut malformed = run("bad");
    malformed.tail[0] = pending_envelope(Version::new(4).unwrap())
        .event_type("Gap")
        .payload(b"gap".as_slice())
        .build()
        .unwrap();
    assert!(matches!(
        store.atomic_append_many(&[run("first"), malformed]).await,
        Err(AtomicAppendError::InvalidRun(_))
    ));
    let mut oversize = run("large");
    oversize.tail[0] = pending_envelope(Version::new(2).unwrap())
        .event_type("Size")
        .payload(vec![0; 100])
        .build()
        .unwrap();
    assert!(matches!(
        store.atomic_append_many(&[run("first"), oversize]).await,
        Err(AtomicAppendError::Store(
            mnesis_postgres::PostgresError::RowLimit { .. }
        ))
    ));
}

mnesis_store_testing::conformance_atomic_append! {
    factory: fixture,
    skip_unless: || std::env::var("DATABASE_URL").is_ok(),
}

#[tokio::test]
async fn missing_expected_head_cannot_create_a_version_gap_or_commit_a_prefix() {
    if std::env::var("DATABASE_URL").is_err() {
        return;
    }
    let (store, pool) = fixture().await;
    let missing = PlannedAppend {
        target: StreamKey::from_bytes("missing-head"),
        expected_version: Some(Version::INITIAL),
        head: pending_envelope(Version::new(2).unwrap())
            .event_type("Missing")
            .payload(b"value".as_slice())
            .build()
            .unwrap(),
        tail: Vec::new(),
    };
    let result = store
        .atomic_append_many(&[run("fresh-first"), missing])
        .await;
    assert!(
        matches!(
            result,
            Err(AtomicAppendError::Conflict {
                index: 1,
                actual: None
            })
        ),
        "wrong missing-head rejection: {result:?}"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM mnesis.events")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM mnesis.keys")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn unrelated_unique_failure_on_an_existing_head_is_not_a_conflict() {
    if std::env::var("DATABASE_URL").is_err() {
        return;
    }
    let (store, pool) = fixture().await;
    let initial = run("existing");
    store
        .append(
            &initial.target,
            None,
            PendingBatch::from_parts(&initial.head, &[]),
        )
        .await
        .unwrap();
    sqlx::query("ALTER TABLE mnesis.events ADD CONSTRAINT atomic_payload_key UNIQUE (payload)")
        .execute(&pool)
        .await
        .unwrap();
    let next = PlannedAppend {
        target: initial.target.clone(),
        expected_version: Some(Version::INITIAL),
        head: initial.tail[0].clone(),
        tail: Vec::new(),
    };
    let result = store.atomic_append_many(&[next]).await;
    sqlx::query("ALTER TABLE mnesis.events DROP CONSTRAINT atomic_payload_key")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        matches!(result, Err(AtomicAppendError::Store(mnesis_postgres::PostgresError::Sqlx(ref source)))
        if source.as_database_error().and_then(sqlx::error::DatabaseError::constraint) == Some("atomic_payload_key"))
    );
    let rows: Vec<_> = store
        .read_stream(&initial.target, Version::INITIAL)
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].version(), Version::INITIAL);
    assert_eq!(rows[0].payload(), b"value");
}
