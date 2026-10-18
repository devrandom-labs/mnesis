//! A real server commits while a scoped transport suppresses its acknowledgement.

#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions"
)]
#![allow(clippy::missing_panics_doc, reason = "test assertions")]

use std::future::pending;
use std::io::{self, Write, stderr};
use std::path::PathBuf;
use std::pin::pin;
use std::str::FromStr;
use std::time::Duration;

use mnesis::Version;
use mnesis_postgres::{PostgresError, PostgresStore};
use mnesis_store::envelope::{PendingBatch, PendingEnvelope, pending_envelope};
use mnesis_store::{AppendError, RawEventStore, StreamKey};
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::watch;
use tokio::task::{JoinHandle, JoinSet, yield_now};
use tokio::time::timeout;

#[derive(Clone, Copy)]
enum Fault {
    Pass,
    DropBegin,
    DropInsert,
    DropCommit,
    HoldCommit,
    HoldHint,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Observation {
    Waiting,
    Begun,
    Inserted,
    Committed,
    Hint,
}

struct Proxy {
    options: PgConnectOptions,
    fault: watch::Sender<Fault>,
    seen: watch::Receiver<Observation>,
    task: JoinHandle<()>,
    _directory: TempDir,
}

impl Drop for Proxy {
    fn drop(&mut self) {
        // Aborting the owner drops its JoinSet, aborting every connection relay.
        self.task.abort();
    }
}

impl Proxy {
    fn start(options: &PgConnectOptions) -> Self {
        let target = options
            .get_socket()
            .expect("fault profile requires a Unix socket")
            .join(format!(".s.PGSQL.{}", options.get_port()));
        let directory = tempfile::tempdir_in("/tmp").unwrap();
        let listener = UnixListener::bind(
            directory
                .path()
                .join(format!(".s.PGSQL.{}", options.get_port())),
        )
        .unwrap();
        let (fault, mode) = watch::channel(Fault::Pass);
        let (observed, seen) = watch::channel(Observation::Waiting);
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (client, _) = accepted.unwrap();
                        let destination = target.clone();
                        let phase = mode.clone();
                        let notice = observed.clone();
                        connections.spawn(async move {
                            // EOF/errors close both sides; expected injected faults
                            // are observed separately at the exact command boundary.
                            let _ = forward(client, destination, phase, notice).await;
                        });
                    },
                    Some(result) = connections.join_next() => { result.unwrap(); },
                }
            }
        });
        Self {
            options: options
                .clone()
                .socket(directory.path())
                .ssl_mode(PgSslMode::Disable),
            fault,
            seen,
            task,
            _directory: directory,
        }
    }

    async fn observed(&mut self, expected: Observation) {
        timeout(
            Duration::from_secs(3),
            self.seen.wait_for(|phase| *phase == expected),
        )
        .await
        .expect("server command boundary was not observed")
        .unwrap();
    }
}

async fn forward(
    client: UnixStream,
    destination: PathBuf,
    mode: watch::Receiver<Fault>,
    observed: watch::Sender<Observation>,
) -> io::Result<()> {
    let server = UnixStream::connect(destination).await?;
    let (mut client_read, mut client_write) = client.into_split();
    let (mut server_read, mut server_write) = server.into_split();
    tokio::select! {
        result = tokio::io::copy(&mut client_read, &mut server_write) => result.map(|_| ()),
        result = async {
            loop {
                // Backend protocol: one-byte tag, four-byte BE length including
                // the length field, then its body. Bound test-only allocation.
                let tag = server_read.read_u8().await?;
                let length = server_read.read_u32().await?;
                let body_length = length.checked_sub(4).filter(|size| *size <= 1_048_576)
                    .ok_or_else(|| io::Error::other("invalid/oversized test frame"))?;
                let mut body = vec![0; usize::try_from(body_length).unwrap()];
                server_read.read_exact(&mut body).await?;
                if tag == b'C' && body == b"COMMIT\0" {
                    observed.send_replace(Observation::Committed);
                }
                let fault = *mode.borrow();
                let outcome = match (tag, body.as_slice(), fault) {
                    (b'C', b"BEGIN\0", Fault::DropBegin) => Some(Observation::Begun),
                    (b'C', b"COMMIT\0", Fault::DropCommit | Fault::HoldCommit) => Some(Observation::Committed),
                    (b'C', bytes, Fault::DropInsert) if bytes.starts_with(b"INSERT ") => Some(Observation::Inserted),
                    (b'C', b"SELECT 1\0", Fault::HoldHint)
                        if *observed.borrow() == Observation::Committed => Some(Observation::Hint),
                    _ => None,
                };
                if let Some(phase) = outcome {
                    observed.send_replace(phase);
                    if matches!(fault, Fault::HoldCommit | Fault::HoldHint) {
                        return pending::<io::Result<()>>().await;
                    }
                    return Ok(());
                }
                client_write.write_u8(tag).await?;
                client_write.write_u32(length).await?;
                client_write.write_all(&body).await?;
            }
        } => result,
    }
}

async fn fixture() -> Option<(Proxy, PostgresStore, PgPool)> {
    let url = std::env::var("DATABASE_URL").ok()?;
    let options = PgConnectOptions::from_str(&url)
        .unwrap()
        .application_name(&format!("mnesis-outcomes-{}", std::process::id()));
    let control = PgPoolOptions::new()
        .max_connections(4)
        .connect_with(options.clone())
        .await
        .unwrap();
    let setup = PostgresStore::from_pool(control.clone()).await.unwrap();
    sqlx::query("TRUNCATE events RESTART IDENTITY")
        .execute(&control)
        .await
        .unwrap();
    drop(setup);
    let proxy = Proxy::start(&options);
    let backing = PgPoolOptions::new()
        .max_connections(4)
        .connect_with(proxy.options.clone())
        .await
        .unwrap();
    let store = PostgresStore::from_pool(backing).await.unwrap();
    Some((proxy, store, control))
}

fn events() -> Vec<PendingEnvelope> {
    (1..=3)
        .map(|version| {
            pending_envelope(Version::new(version).unwrap())
                .event_type("Outcome")
                .payload(b"payload".as_slice())
                .build()
                .unwrap()
        })
        .collect()
}

async fn count(control: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM events")
        .fetch_one(control)
        .await
        .unwrap()
}

#[tokio::test]
async fn transparent_transport_preserves_confirmed_append() {
    let Some((_proxy, store, control)) = fixture().await else {
        return;
    };
    let id = StreamKey::from_bytes("transparent");
    let rows = events();
    let position = store
        .append(&id, None, PendingBatch::new(&rows).unwrap())
        .await
        .unwrap();
    assert_eq!(position.seq(), 3);
    assert_eq!(count(&control).await, 3);
}

#[tokio::test]
async fn loss_before_begin_response_cannot_commit_events() {
    let Some((mut proxy, store, control)) = fixture().await else {
        return;
    };
    proxy.fault.send_replace(Fault::DropBegin);
    let rows = events();
    let result = store
        .append(
            &StreamKey::from_bytes("before-begin"),
            None,
            PendingBatch::new(&rows).unwrap(),
        )
        .await;
    proxy.observed(Observation::Begun).await;
    assert!(matches!(
        result,
        Err(AppendError::Store(PostgresError::Sqlx(sqlx::Error::Io(ref cause)))) if cause.kind() == io::ErrorKind::UnexpectedEof
    ));
    assert_eq!(count(&control).await, 0);
}

#[tokio::test]
async fn loss_during_inserts_rolls_back_the_uncommitted_prefix() {
    let Some((mut proxy, store, control)) = fixture().await else {
        return;
    };
    proxy.fault.send_replace(Fault::DropInsert);
    let rows = events();
    let result = store
        .append(
            &StreamKey::from_bytes("during-insert"),
            None,
            PendingBatch::new(&rows).unwrap(),
        )
        .await;
    proxy.observed(Observation::Inserted).await;
    assert!(matches!(
        result,
        Err(AppendError::Store(PostgresError::Sqlx(sqlx::Error::Io(ref cause)))) if cause.kind() == io::ErrorKind::UnexpectedEof
    ));
    assert_eq!(count(&control).await, 0);
}

#[tokio::test]
async fn lost_commit_acknowledgement_still_leaves_the_whole_batch_durable() {
    let Some((mut proxy, store, control)) = fixture().await else {
        return;
    };
    proxy.fault.send_replace(Fault::DropCommit);
    let rows = events();
    let result = store
        .append(
            &StreamKey::from_bytes("lost-commit"),
            None,
            PendingBatch::new(&rows).unwrap(),
        )
        .await;
    proxy.observed(Observation::Committed).await;
    assert_eq!(
        count(&control).await,
        3,
        "the independent control must prove real commit"
    );
    writeln!(
        stderr().lock(),
        "durable rows=3; suppressed COMMIT outcome={result:?}"
    )
    .unwrap();
    assert!(matches!(
        result,
        Err(AppendError::Store(PostgresError::CommitUnknown(sqlx::Error::Io(ref cause)))) if cause.kind() == io::ErrorKind::UnexpectedEof
    ));
}

#[tokio::test]
async fn caller_timeout_after_server_commit_never_proves_rollback() {
    let Some((mut proxy, store, control)) = fixture().await else {
        return;
    };
    proxy.fault.send_replace(Fault::HoldCommit);
    let rows = events();
    let id = StreamKey::from_bytes("cancel-commit");
    {
        let mut attempt = pin!(store.append(&id, None, PendingBatch::new(&rows).unwrap()));
        tokio::select! {
            result = &mut attempt => panic!("response must remain withheld: {result:?}"),
            () = proxy.observed(Observation::Committed) => {},
        }
        assert_eq!(count(&control).await, 3);
        assert!(
            timeout(Duration::from_millis(20), &mut attempt)
                .await
                .is_err()
        );
    }
    assert_eq!(count(&control).await, 3);
}

#[tokio::test]
async fn unrelated_unique_constraint_is_not_an_optimistic_conflict() {
    let Some((_proxy, store, control)) = fixture().await else {
        return;
    };
    sqlx::query("CREATE UNIQUE INDEX test_outcome_type ON events (event_type)")
        .execute(&control)
        .await
        .unwrap();
    let rows = events();
    let result = store
        .append(
            &StreamKey::from_bytes("other-unique"),
            None,
            PendingBatch::new(&rows).unwrap(),
        )
        .await;
    let persisted = count(&control).await;
    // Always remove the injected constraint before any assertion can panic.
    sqlx::query("DROP INDEX test_outcome_type")
        .execute(&control)
        .await
        .unwrap();
    assert_eq!(persisted, 0);
    assert!(
        matches!(result, Err(AppendError::Store(PostgresError::Sqlx(sqlx::Error::Database(ref error))))
        if error.constraint() == Some("test_outcome_type")),
        "wrong classification: {result:?}"
    );
}

#[tokio::test]
async fn stalled_hint_acknowledgement_cannot_hold_confirmed_success() {
    let Some((mut proxy, store, control)) = fixture().await else {
        return;
    };
    proxy.fault.send_replace(Fault::HoldHint);
    let rows = events();
    let position = timeout(
        Duration::from_secs(1),
        store.append(
            &StreamKey::from_bytes("slow-hint"),
            None,
            PendingBatch::new(&rows).unwrap(),
        ),
    )
    .await
    .expect("confirmed commit must not await an unbounded hint")
    .unwrap();
    proxy.observed(Observation::Hint).await;
    assert_eq!(position.seq(), 3);
    assert_eq!(count(&control).await, 3);
}

#[tokio::test]
async fn server_rejections_at_commit_preserve_the_code_and_outcome() {
    for code in ["23514", "40001", "40P01", "40003"] {
        let Some((_proxy, store, control)) = fixture().await else {
            return;
        };
        // Inject exact server error codes in a DEFERRED trigger. This verifies
        // commit-stage mapping; it does not claim a real SSI/deadlock campaign.
        sqlx::raw_sql(&format!(
            "CREATE FUNCTION test_commit_reject() RETURNS trigger LANGUAGE plpgsql AS \
             $$ BEGIN RAISE EXCEPTION USING ERRCODE = '{code}', MESSAGE = 'commit rejection witness'; END $$; \
             CREATE CONSTRAINT TRIGGER test_commit_reject AFTER INSERT ON events \
             DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION test_commit_reject();"
        )).execute(&control).await.unwrap();
        let rows = events();
        let result = store
            .append(
                &StreamKey::from_bytes("deferred"),
                None,
                PendingBatch::new(&rows).unwrap(),
            )
            .await;
        let persisted = count(&control).await;
        sqlx::raw_sql(
            "DROP TRIGGER test_commit_reject ON events; DROP FUNCTION test_commit_reject();",
        )
        .execute(&control)
        .await
        .unwrap();
        assert_eq!(persisted, 0);
        let source = match (code, result) {
            ("40003", Err(AppendError::Store(PostgresError::CommitUnknown(source))))
            | ("23514" | "40001" | "40P01", Err(AppendError::Store(PostgresError::Sqlx(source)))) => {
                source
            }
            (_, other) => panic!("incorrect commit-stage outcome for {code}: {other:?}"),
        };
        assert!(
            matches!(source, sqlx::Error::Database(error) if error.code().as_deref() == Some(code))
        );
    }
}

#[tokio::test]
async fn identity_sequence_collision_is_not_an_optimistic_conflict() {
    let Some((_proxy, store, control)) = fixture().await else {
        return;
    };
    let rows = events();
    store
        .append(
            &StreamKey::from_bytes("first"),
            None,
            PendingBatch::new(&rows).unwrap(),
        )
        .await
        .unwrap();
    sqlx::query("ALTER SEQUENCE events_global_seq_seq RESTART WITH 1")
        .execute(&control)
        .await
        .unwrap();
    let result = store
        .append(
            &StreamKey::from_bytes("second"),
            None,
            PendingBatch::new(&rows).unwrap(),
        )
        .await;
    // Repair the deliberately rewound sequence before assertions.
    sqlx::query("SELECT setval('events_global_seq_seq', 3)")
        .execute(&control)
        .await
        .unwrap();
    assert!(
        matches!(result, Err(AppendError::Store(PostgresError::Sqlx(sqlx::Error::Database(ref error))))
        if error.constraint() == Some("events_pkey")),
        "incorrect identity failure: {result:?}"
    );
    assert_eq!(count(&control).await, 3);
}

struct Increment;

impl mnesis::Handle<Increment> for mnesis_test_domains::Counter {
    fn handle(
        _state: &mnesis_test_domains::CounterState,
        _command: Increment,
    ) -> Result<
        Option<mnesis::Events<mnesis_test_domains::CounterEvent>>,
        mnesis_test_domains::CounterError,
    > {
        Ok(Some(mnesis::events![
            mnesis_test_domains::CounterEvent::Incremented
        ]))
    }
}

#[tokio::test]
async fn uncertain_commit_survives_repository_execution_and_requires_reload() {
    use mnesis_store::{CommandRepository, ExecuteError, Repository, Store, StoreError};
    use mnesis_test_domains::{Counter, TestId};
    let Some((mut proxy, raw, control)) = fixture().await else {
        return;
    };
    proxy.fault.send_replace(Fault::DropCommit);
    let store = Store::new(raw);
    let repository = store.repository::<Counter>().json().build();
    let id = TestId::new("uncertain-execution");
    let mut root = mnesis::AggregateRoot::<Counter>::new(id.clone());
    let Err(error) = repository.execute(&mut root, Increment).await else {
        panic!("a suppressed commit response cannot confirm execution");
    };
    proxy.observed(Observation::Committed).await;
    assert!(
        !error.is_conflict(),
        "wrapping must not make uncertainty retryable"
    );
    assert!(matches!(error,
        ExecuteError::Store(StoreError::Adapter(PostgresError::CommitUnknown(sqlx::Error::Io(cause))))
        if cause.kind() == io::ErrorKind::UnexpectedEof));
    assert_eq!(count(&control).await, 1);
    assert_eq!(root.version(), None);
    assert_eq!(root.state().unwrap().value, 0);
    proxy.fault.send_replace(Fault::Pass);
    let reloaded = repository.load(id).await.unwrap();
    assert_eq!(reloaded.version(), Some(Version::INITIAL));
    assert_eq!(reloaded.state().unwrap().value, 1);
}

#[tokio::test]
async fn closed_data_pool_fails_before_begin_with_no_committed_rows() {
    let Some((_proxy, _fixture, control)) = fixture().await else {
        return;
    };
    let backing = PgPoolOptions::new()
        .max_connections(4)
        .connect_with(control.connect_options().as_ref().clone())
        .await
        .unwrap();
    let store = PostgresStore::from_pool(backing.clone()).await.unwrap();
    backing.close().await;
    let rows = events();
    let result = store
        .append(
            &StreamKey::from_bytes("before-begin"),
            None,
            PendingBatch::new(&rows).unwrap(),
        )
        .await;
    assert!(matches!(
        result,
        Err(AppendError::Store(PostgresError::Sqlx(
            sqlx::Error::PoolClosed
        )))
    ));
    assert_eq!(count(&control).await, 0);
}

#[tokio::test]
async fn a_native_unique_collision_preserves_the_callers_expected_head() {
    for expected in [None, Some(Version::INITIAL)] {
        let Some((_proxy, store, control)) = fixture().await else {
            return;
        };
        let key = StreamKey::from_bytes("contender");
        let prefix = events();
        if expected.is_some() {
            store
                .append(&key, None, PendingBatch::new(&prefix[..1]).unwrap())
                .await
                .unwrap();
        }
        let number = expected.map_or(0, Version::as_u64).checked_add(1).unwrap();
        let mut winner = control.begin().await.unwrap();
        sqlx::query(
            "INSERT INTO events (stream_id, version, event_type, schema_version, payload) \
            VALUES ($1, $2, 'Winner', 1, 'winner'::bytea)",
        )
        .bind(key.as_bytes())
        .bind(i64::try_from(number).unwrap())
        .execute(&mut *winner)
        .await
        .unwrap();
        let contender = tokio::spawn(async move {
            let event = pending_envelope(Version::new(number).unwrap())
                .event_type("Contender")
                .payload(b"loser".as_slice())
                .build()
                .unwrap();
            store
                .append(&key, expected, PendingBatch::new(&[event]).unwrap())
                .await
        });
        wait_for_insert(&control).await;
        winner.commit().await.unwrap();
        let result = contender.await.unwrap();
        let stored: Vec<(i64, Vec<u8>)> =
            sqlx::query_as("SELECT version, payload FROM events ORDER BY version")
                .fetch_all(&control)
                .await
                .unwrap();
        let wanted = match expected {
            None => vec![(1, b"winner".to_vec())],
            Some(_) => vec![(1, b"payload".to_vec()), (2, b"winner".to_vec())],
        };
        assert_eq!(
            stored, wanted,
            "the rejected contender must not persist any row"
        );
        match result {
            Err(AppendError::Conflict {
                stream_id,
                expected: observed,
                actual,
            }) => {
                assert_eq!(stream_id.to_string(), "contender");
                assert_eq!(
                    observed, expected,
                    "report the caller's head, not its proposed next version"
                );
                assert_eq!(
                    actual, None,
                    "the post-race head is unknown without reconciliation"
                );
            }
            other => panic!("native stream/version collision misclassified: {other:?}"),
        }
    }
}

async fn wait_for_insert(control: &PgPool) {
    let name = control
        .connect_options()
        .get_application_name()
        .unwrap()
        .to_owned();
    timeout(Duration::from_secs(3), async {
        loop {
            let waiting: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_locks l \
                JOIN pg_stat_activity a ON a.pid = l.pid \
                WHERE l.locktype = 'transactionid' AND NOT l.granted \
                AND a.application_name = $1 AND a.query LIKE 'INSERT INTO %events %'",
            )
            .bind(&name)
            .fetch_one(control)
            .await
            .unwrap();
            if waiting == 1 {
                break;
            }
            yield_now().await;
        }
    })
    .await
    .expect("the actual INSERT must wait on the uncommitted winning row");
}
