//! A real server commits while a scoped transport suppresses its acknowledgement.

#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions"
)]
#![allow(clippy::missing_panics_doc, reason = "test assertions")]

mod support;

#[cfg(feature = "projection")]
#[path = "support/projection.rs"]
mod projection;

#[cfg(feature = "import")]
#[path = "support/sections.rs"]
mod sections;

#[cfg(feature = "import")]
use futures::{FutureExt, TryStreamExt};
use std::future::pending;
use std::io::{self, Write, stderr};
#[cfg(any(feature = "snapshot", feature = "projection"))]
use std::num::NonZeroU32;
#[cfg(feature = "projection")]
use std::num::NonZeroU64;
use std::path::PathBuf;
use std::pin::pin;
use std::str::FromStr;
use std::time::Duration;

use mnesis::Version;
#[cfg(feature = "projection")]
use mnesis_postgres::PgAllPos;
use mnesis_postgres::{PostgresError, PostgresStore};
#[cfg(any(feature = "snapshot", feature = "projection"))]
use mnesis_store::JsonCodec;
#[cfg(feature = "projection")]
use mnesis_store::checkpoint::{
    CheckpointError, CheckpointHydrated, CheckpointMode, CheckpointStore, CheckpointWrite,
};
use mnesis_store::envelope::{PendingBatch, PendingEnvelope, pending_envelope};
#[cfg(feature = "projection")]
use mnesis_store::state::AfterEventTypes;
#[cfg(any(feature = "snapshot", feature = "projection"))]
use mnesis_store::state::{CodecSnapshotStore, CodecSnapshotStoreError};
#[cfg(feature = "snapshot")]
use mnesis_store::state::{Hydrated, SnapshotStore};
use mnesis_store::{AppendError, RawEventStore, StreamKey};
#[cfg(feature = "projection")]
use mnesis_store::{Projection, ProjectionError, ProjectionStateError};
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
    RejectCommit(&'static str),
    HoldHint,
    #[cfg(feature = "import")]
    HoldSecondHead,
    #[cfg(any(feature = "snapshot", feature = "projection"))]
    HoldState,
    #[cfg(feature = "snapshot")]
    HoldValidation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Observation {
    Waiting,
    Begun,
    Inserted,
    Committed,
    CommitSent,
    Hint,
    #[cfg(feature = "import")]
    SecondHead,
    #[cfg(any(feature = "snapshot", feature = "projection"))]
    State,
    #[cfg(feature = "snapshot")]
    Validation,
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
        Self::start_with(options, Fault::Pass)
    }

    fn start_with(options: &PgConnectOptions, initial: Fault) -> Self {
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
        let (fault, mode) = watch::channel(initial);
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
        result = forward_commands(&mut client_read, &mut server_write, mode.clone(), observed.clone()) => result,
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
                #[cfg(feature = "snapshot")]
                if tag == b'D' && matches!(fault, Fault::HoldValidation)
                    && body == [0, 1, 0, 0, 0, 1, 1] {
                    observed.send_replace(Observation::Validation);
                    let mut resume = mode.clone();
                    resume.wait_for(|phase| matches!(phase, Fault::Pass)).await.map_err(io::Error::other)?;
                }
                #[cfg(any(feature = "snapshot", feature = "projection"))]
                if tag == b'D' && matches!(fault, Fault::HoldState)
                    && body.windows(STATE_OLD.len()).any(|part| part == STATE_OLD) {
                    observed.send_replace(Observation::State);
                    let mut resume = mode.clone();
                    resume.wait_for(|phase| matches!(phase, Fault::Pass)).await.map_err(io::Error::other)?;
                }
                if tag == b'A' && matches!(fault, Fault::HoldHint) {
                    continue;
                }
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

#[cfg(any(feature = "snapshot", feature = "projection"))]
const STATE_OLD: &[u8] = b"snapshot-old-081";

#[cfg(feature = "snapshot")]
#[tokio::test]
async fn runtime_schema_validation_keeps_one_read_only_database_view() {
    let Some((mut proxy, _store, control)) = fixture().await else {
        return;
    };
    let name = format!("mnesis-validation-{}", std::process::id());
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect_with(proxy.options.clone().application_name(&name))
        .await
        .unwrap();
    proxy.fault.send_replace(Fault::HoldValidation);
    let backing = pool.clone();
    let mut pending = JoinSet::new();
    pending.spawn(async move { PostgresStore::from_pool(backing).await });
    proxy.observed(Observation::Validation).await;
    let facts = timeout(Duration::from_secs(3), async {
        loop {
            let row: (String, bool) = sqlx::query_as("SELECT state, backend_xmin IS NOT NULL FROM pg_stat_activity WHERE application_name = $1 AND datname = current_database()")
                .bind(&name).fetch_one(&control).await.unwrap();
            if row.0 != "active" { break row; }
            yield_now().await;
        }
    }).await.unwrap();
    proxy.fault.send_replace(Fault::Pass);
    let opened = pending.join_next().await.unwrap().unwrap().unwrap();
    drop(opened);
    pool.close().await;
    assert_eq!(facts, ("idle in transaction".to_owned(), true));
}

#[cfg(feature = "snapshot")]
#[tokio::test]
async fn cancelling_runtime_open_releases_its_view_and_preserves_the_callers_pool() {
    let Some((mut proxy, _store, control)) = fixture().await else {
        return;
    };
    let name = format!("mnesis-validation-cancel-{}", std::process::id());
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect_with(proxy.options.clone().application_name(&name))
        .await
        .unwrap();
    proxy.fault.send_replace(Fault::HoldValidation);
    let backing = pool.clone();
    let mut pending = JoinSet::new();
    pending.spawn(async move { PostgresStore::from_pool(backing).await });
    proxy.observed(Observation::Validation).await;
    let pinned: bool = sqlx::query_scalar("SELECT EXISTS (SELECT FROM pg_stat_activity WHERE application_name = $1 AND backend_xmin IS NOT NULL)")
        .bind(&name).fetch_one(&control).await.unwrap();
    pending.abort_all();
    assert!(matches!(pending.join_next().await, Some(Err(error)) if error.is_cancelled()));
    timeout(Duration::from_secs(3), async {
        loop {
            let retained: bool = sqlx::query_scalar("SELECT EXISTS (SELECT FROM pg_stat_activity WHERE application_name = $1 AND backend_xmin IS NOT NULL)")
                .bind(&name).fetch_one(&control).await.unwrap();
            if !retained { break; }
            yield_now().await;
        }
    }).await.expect("canceled constructor must release its actual native view");
    proxy.fault.send_replace(Fault::Pass);
    assert!(
        pinned,
        "constructor must hold a real view before cancellation"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i32>("SELECT 1")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    pool.close().await;
}

#[cfg(feature = "snapshot")]
#[tokio::test]
async fn snapshot_hydration_cannot_mix_a_captured_version_with_a_later_state() {
    let Some((mut proxy, store, control)) = fixture().await else {
        return;
    };
    let id = StreamKey::from_bytes("snapshot-read-pair");
    store
        .commit(&id, NonZeroU32::MIN, Version::INITIAL, &STATE_OLD.to_vec())
        .await
        .unwrap();
    proxy.fault.send_replace(Fault::HoldState);
    let client = store.clone();
    let target = id.clone();
    let mut pending = JoinSet::new();
    pending.spawn(async move { client.hydrate(&target, NonZeroU32::MIN).await });
    proxy.observed(Observation::State).await;
    let independent = PostgresStore::from_pool(control).await.unwrap();
    independent
        .commit(
            &id,
            NonZeroU32::MIN,
            Version::new(2).unwrap(),
            &b"snapshot-new-081".to_vec(),
        )
        .await
        .unwrap();
    proxy.fault.send_replace(Fault::Pass);
    let captured = pending.join_next().await.unwrap().unwrap().unwrap();
    assert_eq!(
        captured,
        Hydrated::Found {
            position: Version::INITIAL,
            state: STATE_OLD.to_vec()
        }
    );
    assert_eq!(
        store.hydrate(&id, NonZeroU32::MIN).await.unwrap(),
        Hydrated::Found {
            position: Version::new(2).unwrap(),
            state: b"snapshot-new-081".to_vec()
        }
    );
}

#[cfg(feature = "snapshot")]
#[tokio::test]
async fn uncertain_snapshot_commit_preserves_the_native_source_through_the_codec() {
    let Some((mut proxy, raw, control)) = fixture().await else {
        return;
    };
    let store = CodecSnapshotStore::new(raw, JsonCodec::default());
    let id = StreamKey::from_bytes("snapshot-unknown");
    let state = mnesis_test_domains::CounterState { value: 7 };
    let position = Version::new(u64::MAX).unwrap();
    proxy.fault.send_replace(Fault::DropCommit);
    let result = store.commit(&id, NonZeroU32::MIN, position, &state).await;
    proxy.observed(Observation::Committed).await;
    assert!(
        matches!(result, Err(CodecSnapshotStoreError::Store(PostgresError::CommitUnknown(sqlx::Error::Io(ref cause))))
        if cause.kind() == io::ErrorKind::UnexpectedEof)
    );
    let row: (String, i64, Vec<u8>) = sqlx::query_as("SELECT s.version::text, s.schema_version, s.state FROM mnesis.snapshots s JOIN mnesis.keys k ON k.number = s.key WHERE k.value = $1")
        .bind(id.as_bytes()).fetch_one(&control).await.unwrap();
    assert_eq!(row, (u64::MAX.to_string(), 1, b"{\"value\":7}".to_vec()));
    assert_eq!(count(&control).await, 0);
    proxy.fault.send_replace(Fault::Pass);
    assert_eq!(
        store.hydrate(&id, NonZeroU32::MIN).await.unwrap(),
        Hydrated::Found { position, state }
    );
}

#[cfg(feature = "snapshot")]
#[tokio::test]
async fn caller_timeout_after_snapshot_commit_does_not_erase_the_saved_pair() {
    let Some((mut proxy, store, control)) = fixture().await else {
        return;
    };
    let id = StreamKey::from_bytes("snapshot-cancel");
    let state = b"committed".to_vec();
    proxy.fault.send_replace(Fault::HoldCommit);
    {
        let mut attempt = pin!(store.commit(&id, NonZeroU32::MIN, Version::INITIAL, &state));
        tokio::select! {
            result = &mut attempt => panic!("snapshot commit response must remain held: {result:?}"),
            () = proxy.observed(Observation::Committed) => {},
        }
        assert!(
            timeout(Duration::from_millis(20), &mut attempt)
                .await
                .is_err()
        );
    }
    let row: (String, Vec<u8>) = sqlx::query_as("SELECT s.version::text, s.state FROM mnesis.snapshots s JOIN mnesis.keys k ON k.number = s.key WHERE k.value = $1")
        .bind(id.as_bytes()).fetch_one(&control).await.unwrap();
    assert_eq!(row, ("1".to_owned(), state));
    assert_eq!(count(&control).await, 0);
}

#[cfg(feature = "import")]
const SECOND_HEAD: &[u8] = b"pg5-order-second";

#[cfg(feature = "projection")]
#[tokio::test]
async fn checkpoint_hydration_keeps_the_captured_revision_position_and_state_together() {
    let Some((mut proxy, store, control)) = fixture().await else {
        return;
    };
    let id = StreamKey::from_slice(b"checkpoint-pair");
    let original = PgAllPos::new(store.history(), 1, 9);
    let revision = store
        .commit_checkpoint(
            &id,
            CheckpointWrite {
                expected: None,
                schema_version: NonZeroU32::MIN,
                position: original,
                state: &STATE_OLD.to_vec(),
                mode: CheckpointMode::Advance,
            },
        )
        .await
        .unwrap();
    proxy.fault.send_replace(Fault::HoldState);
    let client = store.clone();
    let target = id.clone();
    let mut pending = JoinSet::new();
    pending.spawn(async move { client.hydrate_checkpoint(&target, NonZeroU32::MIN).await });
    proxy.observed(Observation::State).await;
    let independent = PostgresStore::from_pool(control).await.unwrap();
    let next = PgAllPos::new(store.history(), 2, 0);
    let replacement = b"checkpoint-new".to_vec();
    let updated = independent
        .commit_checkpoint(
            &id,
            CheckpointWrite {
                expected: Some(revision),
                schema_version: NonZeroU32::MIN,
                position: next,
                state: &replacement,
                mode: CheckpointMode::Advance,
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.get(), 2);
    proxy.fault.send_replace(Fault::Pass);
    assert_eq!(
        pending.join_next().await.unwrap().unwrap().unwrap(),
        CheckpointHydrated::Found {
            revision,
            position: original,
            state: STATE_OLD.to_vec(),
        }
    );
    assert_eq!(
        store
            .hydrate_checkpoint(&id, NonZeroU32::MIN)
            .await
            .unwrap(),
        CheckpointHydrated::Found {
            revision: updated,
            position: next,
            state: replacement,
        }
    );
}

#[cfg(feature = "projection")]
#[tokio::test]
async fn codec_checkpoint_commit_retains_unknown_outcome_after_native_commit() {
    let Some((mut proxy, raw, control)) = fixture().await else {
        return;
    };
    let history = raw.history();
    let store = CodecSnapshotStore::new(raw, JsonCodec::default());
    let id = StreamKey::from_slice(b"checkpoint-unknown");
    let position = PgAllPos::new(history, u64::MAX, u64::MAX);
    let state = mnesis_test_domains::CounterState { value: 7 };
    proxy.fault.send_replace(Fault::DropCommit);
    let result = store
        .commit_checkpoint(
            &id,
            CheckpointWrite {
                expected: None,
                schema_version: NonZeroU32::MIN,
                position,
                state: &state,
                mode: CheckpointMode::Advance,
            },
        )
        .await;
    proxy.observed(Observation::Committed).await;
    assert!(matches!(result,
        Err(CheckpointError::Store(CodecSnapshotStoreError::Store(PostgresError::CommitUnknown(sqlx::Error::Io(ref source)))))
        if source.kind() == io::ErrorKind::UnexpectedEof));
    let saved: (String, i64, String, String, Vec<u8>) = sqlx::query_as(
        "SELECT c.revision::text, c.schema_version, c.txid::text, c.global_seq::text, c.state \
         FROM mnesis.checkpoints c JOIN mnesis.keys k ON k.number = c.key WHERE k.value = $1",
    )
    .bind(id.as_bytes())
    .fetch_one(&control)
    .await
    .unwrap();
    assert_eq!(
        saved,
        (
            "1".to_owned(),
            1,
            u64::MAX.to_string(),
            u64::MAX.to_string(),
            b"{\"value\":7}".to_vec()
        )
    );
    assert_eq!(count(&control).await, 0);
    proxy.fault.send_replace(Fault::Pass);
    assert_eq!(
        store
            .hydrate_checkpoint(&id, NonZeroU32::MIN)
            .await
            .unwrap(),
        CheckpointHydrated::Found {
            revision: NonZeroU64::MIN,
            position,
            state,
        }
    );
}

#[cfg(feature = "projection")]
#[tokio::test]
async fn caller_timeout_after_checkpoint_commit_preserves_every_saved_field() {
    let Some((mut proxy, store, control)) = fixture().await else {
        return;
    };
    let id = StreamKey::from_slice(b"checkpoint-cancel");
    let position = PgAllPos::new(store.history(), 7, 9);
    let state = b"committed".to_vec();
    let schema = NonZeroU32::new(u32::MAX).unwrap();
    let write = CheckpointWrite {
        expected: None,
        schema_version: schema,
        position,
        state: &state,
        mode: CheckpointMode::Advance,
    };
    proxy.fault.send_replace(Fault::HoldCommit);
    {
        let mut attempt = pin!(store.commit_checkpoint(&id, write));
        tokio::select! {
            result = &mut attempt => panic!("checkpoint COMMIT response must remain held: {result:?}"),
            () = proxy.observed(Observation::Committed) => {},
        }
        assert!(
            timeout(Duration::from_millis(20), &mut attempt)
                .await
                .is_err()
        );
    }
    let saved: (String, i64, String, String, Vec<u8>) = sqlx::query_as(
        "SELECT c.revision::text, c.schema_version, c.txid::text, c.global_seq::text, c.state \
         FROM mnesis.checkpoints c JOIN mnesis.keys k ON k.number = c.key WHERE k.value = $1",
    )
    .bind(id.as_bytes())
    .fetch_one(&control)
    .await
    .unwrap();
    assert_eq!(
        saved,
        (
            "1".to_owned(),
            i64::from(u32::MAX),
            "7".to_owned(),
            "9".to_owned(),
            state
        )
    );
    assert_eq!(count(&control).await, 0);
}

#[cfg(feature = "projection")]
#[tokio::test]
async fn cancelled_projection_reloads_the_committed_pair_before_continuing() {
    let Some((mut proxy, raw, control)) = fixture().await else {
        return;
    };
    let events = projection::events(&raw).await;
    let id = StreamKey::from_slice(b"cancelled-projector");
    let codec = CodecSnapshotStore::new(&raw, JsonCodec::default());
    let mut stepper = Projection::load(
        id.clone(),
        projection::Counter,
        AfterEventTypes::new(&["Incremented", "Set"]),
        &codec,
        NonZeroU32::MIN,
    )
    .await
    .unwrap();
    drop(proxy.seen.borrow_and_update());
    proxy.fault.send_replace(Fault::HoldCommit);
    {
        let mut attempt = pin!(stepper.advance(events[0].clone()));
        tokio::select! {
            result = &mut attempt => panic!("stepper COMMIT response must remain held: {result:?}"),
            status = timeout(Duration::from_secs(3), proxy.seen.changed()) => status.unwrap().unwrap(),
        }
        assert_eq!(*proxy.seen.borrow(), Observation::Committed);
        assert!(
            timeout(Duration::from_millis(20), &mut attempt)
                .await
                .is_err()
        );
    }
    assert!(matches!(
        stepper.state(),
        Err(ProjectionStateError::ReloadRequired)
    ));
    assert!(matches!(
        stepper.advance(events[1].clone()).await,
        Err(ProjectionError::State(ProjectionStateError::ReloadRequired))
    ));
    let independent = PostgresStore::from_pool(control.clone()).await.unwrap();
    let saved = CodecSnapshotStore::new(independent, JsonCodec::default());
    let resumed = Projection::load(
        id.clone(),
        projection::Counter,
        AfterEventTypes::new(&["Incremented", "Set"]),
        &saved,
        NonZeroU32::MIN,
    )
    .await
    .unwrap();
    assert_eq!(resumed.checkpoint(), Some(events[0].0));
    assert_eq!(
        resumed.state().unwrap(),
        &mnesis_test_domains::CounterState { value: 1 }
    );
    assert_eq!(
        saved
            .hydrate_checkpoint(&id, NonZeroU32::MIN)
            .await
            .unwrap(),
        CheckpointHydrated::Found {
            revision: NonZeroU64::MIN,
            position: events[0].0,
            state: mnesis_test_domains::CounterState { value: 1 },
        }
    );
    assert_eq!(count(&control).await, 2);
}

async fn forward_commands(
    client: &mut tokio::net::unix::OwnedReadHalf,
    server: &mut tokio::net::unix::OwnedWriteHalf,
    mode: watch::Receiver<Fault>,
    observed: watch::Sender<Observation>,
) -> io::Result<()> {
    // SSL is disabled for this Unix-only fixture. Startup has no message tag.
    let startup = client.read_u32().await?;
    let size = startup
        .checked_sub(4)
        .filter(|n| *n <= 1_048_576)
        .ok_or_else(|| io::Error::other("invalid test startup packet"))?;
    let mut parameters = vec![0; usize::try_from(size).unwrap()];
    client.read_exact(&mut parameters).await?;
    server.write_u32(startup).await?;
    server.write_all(&parameters).await?;
    loop {
        let tag = client.read_u8().await?;
        let length = client.read_u32().await?;
        let body_size = length
            .checked_sub(4)
            .filter(|n| *n <= 1_048_576)
            .ok_or_else(|| io::Error::other("invalid test frontend packet"))?;
        let mut body = vec![0; usize::try_from(body_size).unwrap()];
        client.read_exact(&mut body).await?;
        #[cfg(feature = "import")]
        if tag == b'B'
            && body
                .windows(SECOND_HEAD.len())
                .any(|part| part == SECOND_HEAD)
            && matches!(*mode.borrow(), Fault::HoldSecondHead)
        {
            observed.send_replace(Observation::SecondHead);
            let mut resume = mode.clone();
            resume
                .wait_for(|phase| !matches!(phase, Fault::HoldSecondHead))
                .await
                .map_err(io::Error::other)?;
        }
        if tag == b'Q' && body == b"COMMIT\0" {
            let fault = *mode.borrow();
            if let Fault::RejectCommit(code) = fault {
                observed.send_replace(Observation::CommitSent);
                // A real server supplies the injected error at the client's
                // COMMIT boundary. This does not execute a server COMMIT.
                body = format!(
                    "DO $$ BEGIN RAISE EXCEPTION USING ERRCODE='{code}', MESSAGE='commit boundary witness'; END $$\0"
                ).into_bytes();
            }
        }
        server.write_u8(tag).await?;
        server
            .write_u32(u32::try_from(body.len()).unwrap().checked_add(4).unwrap())
            .await?;
        server.write_all(&body).await?;
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
    PostgresStore::migrate(&control).await.unwrap();
    sqlx::raw_sql("DROP TRIGGER IF EXISTS ssi_read ON mnesis.events; DROP TRIGGER IF EXISTS foreign_unique ON mnesis.events; DROP FUNCTION IF EXISTS mnesis.ssi_read(); DROP FUNCTION IF EXISTS mnesis.foreign_unique(); DROP SCHEMA IF EXISTS mnesis_foreign CASCADE")
        .execute(&control).await.unwrap();
    let setup = PostgresStore::from_pool(control.clone()).await.unwrap();
    sqlx::query("TRUNCATE mnesis.events, mnesis.snapshots, mnesis.checkpoints RESTART IDENTITY")
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
    sqlx::query_scalar("SELECT count(*) FROM mnesis.events")
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
    let id = StreamKey::from_bytes("during-insert");
    // The suppressed INSERT must be an event, after key setup has committed.
    support::key(&control, id.as_bytes()).await;
    proxy.fault.send_replace(Fault::DropInsert);
    let rows = events();
    let result = store
        .append(&id, None, PendingBatch::new(&rows).unwrap())
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
    sqlx::query("CREATE UNIQUE INDEX test_outcome_type ON mnesis.events (event_type)")
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
    sqlx::query("DROP INDEX mnesis.test_outcome_type")
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
async fn deferred_rejections_before_commit_preserve_the_code_and_known_outcome() {
    for code in ["23514", "40001", "40P01", "40003"] {
        let Some((_proxy, store, control)) = fixture().await else {
            return;
        };
        // SET CONSTRAINTS runs these checks before the adapter sends COMMIT.
        // The known phase proves rollback even for a raised 40003 SQLSTATE.
        // Only the four literal SQLSTATEs above enter this test DDL.
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "CREATE FUNCTION test_commit_reject() RETURNS trigger LANGUAGE plpgsql AS \
             $$ BEGIN RAISE EXCEPTION USING ERRCODE = '{code}', MESSAGE = 'commit rejection witness'; END $$; \
             CREATE CONSTRAINT TRIGGER test_commit_reject AFTER INSERT ON mnesis.events \
             DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION test_commit_reject();"
        ))).execute(&control).await.unwrap();
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
            "DROP TRIGGER test_commit_reject ON mnesis.events; DROP FUNCTION test_commit_reject();",
        )
        .execute(&control)
        .await
        .unwrap();
        assert_eq!(persisted, 0);
        let source = match result {
            Err(AppendError::Store(PostgresError::Sqlx(source))) => source,
            other => panic!("incorrect pre-COMMIT outcome for {code}: {other:?}"),
        };
        assert!(
            matches!(source, sqlx::Error::Database(error) if error.code().as_deref() == Some(code))
        );
    }
}

#[tokio::test]
async fn server_errors_after_sending_commit_preserve_the_code_and_outcome() {
    for code in ["23514", "40001", "40P01", "40003"] {
        let Some((mut proxy, store, control)) = fixture().await else {
            return;
        };
        proxy.fault.send_replace(Fault::RejectCommit(code));
        let rows = events();
        let result = store
            .append(
                &StreamKey::from_bytes("commit-boundary"),
                None,
                PendingBatch::new(&rows).unwrap(),
            )
            .await;
        proxy.observed(Observation::CommitSent).await;
        assert_eq!(count(&control).await, 0);
        let source = match (code, result) {
            ("40003", Err(AppendError::Store(PostgresError::CommitUnknown(source))))
            | ("23514" | "40001" | "40P01", Err(AppendError::Store(PostgresError::Sqlx(source)))) => {
                source
            }
            (_, other) => panic!("incorrect COMMIT boundary outcome for {code}: {other:?}"),
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
    sqlx::query("ALTER SEQUENCE mnesis.events_global_seq_seq RESTART WITH 1")
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
    sqlx::query("SELECT setval('mnesis.events_global_seq_seq', 3)")
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
        let key_number = support::key(&control, key.as_bytes()).await;
        let mut winner = control.begin().await.unwrap();
        sqlx::query(
            "INSERT INTO mnesis.events (stream_id, version, event_type, schema_version, payload) \
            VALUES ($1, $2, 'Winner', 1, 'winner'::bytea)",
        )
        .bind(key_number)
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
            sqlx::query_as("SELECT version::bigint, payload FROM mnesis.events ORDER BY version")
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

#[cfg(feature = "import")]
fn atomic_writes(count: usize) -> Vec<mnesis_store::import::PlannedAppend> {
    let mut rows = events().into_iter();
    let head = rows.next().unwrap();
    let tail: Vec<_> = rows.collect();
    (0..count)
        .map(|number| mnesis_store::import::PlannedAppend {
            target: StreamKey::from_bytes(format!("atomic-{number}")),
            expected_version: None,
            head: head.clone(),
            tail: tail.clone(),
        })
        .collect()
}

#[cfg(feature = "import")]
#[tokio::test]
async fn lost_atomic_commit_ack_preserves_the_whole_transaction_and_source() {
    use mnesis_store::import::{AtomicAppend, AtomicAppendError};
    let Some((mut proxy, store, control)) = fixture().await else {
        return;
    };
    proxy.fault.send_replace(Fault::DropCommit);
    let result = store.atomic_append_many(&atomic_writes(2)).await;
    proxy.observed(Observation::Committed).await;
    assert_eq!(
        count(&control).await,
        6,
        "both complete runs really committed"
    );
    assert!(
        matches!(result, Err(AtomicAppendError::Store(PostgresError::CommitUnknown(sqlx::Error::Io(ref source)))) if source.kind() == io::ErrorKind::UnexpectedEof)
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(DISTINCT txid::text) FROM mnesis.events")
            .fetch_one(&control)
            .await
            .unwrap(),
        1
    );
    proxy.fault.send_replace(Fault::Pass);
    let reconciled: Vec<_> = store
        .read_all(None)
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(reconciled.len(), 6);
    for key in ["atomic-0", "atomic-1"] {
        assert_eq!(
            reconciled
                .iter()
                .filter(|row| row.1.as_bytes() == key.as_bytes())
                .map(|row| row.2.version().as_u64())
                .collect::<Vec<_>>(),
            [1, 2, 3]
        );
    }
}

#[cfg(feature = "import")]
#[tokio::test]
async fn import_wrapper_preserves_uncertainty_for_both_policies_without_resubmission() {
    use mnesis_store::Store;
    use mnesis_store::import::{Atomicity, EventImporter, ImportError};
    for policy in [Atomicity::WholeChunk, Atomicity::PerStream] {
        let Some((mut proxy, raw, control)) = fixture().await else {
            return;
        };
        let input = sections::pair();
        let store = Store::new(raw);
        proxy.fault.send_replace(Fault::DropCommit);
        let result = store.import(&input, StreamKey::from_slice, policy).await;
        proxy.observed(Observation::Committed).await;
        assert!(
            matches!(result, Err(ImportError::Store(PostgresError::CommitUnknown(sqlx::Error::Io(ref source))))
            if source.kind() == io::ErrorKind::UnexpectedEof),
            "lost import acknowledgement: {result:?}"
        );
        let rows: Vec<(Vec<u8>, String)> = sqlx::query_as(
            "SELECT k.value, e.version::text FROM mnesis.events e JOIN mnesis.keys k ON k.number = e.stream_id ORDER BY e.global_seq",
        ).fetch_all(&control).await.unwrap();
        let first = [
            (input[0].origin.to_vec(), "1".to_owned()),
            (input[0].origin.to_vec(), "2".to_owned()),
        ];
        match policy {
            Atomicity::WholeChunk => assert_eq!(
                rows,
                [
                    first[0].clone(),
                    first[1].clone(),
                    (input[1].origin.to_vec(), "1".to_owned()),
                    (input[1].origin.to_vec(), "2".to_owned())
                ]
            ),
            Atomicity::PerStream => assert_eq!(rows, first),
        }
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(DISTINCT txid::text) FROM mnesis.events")
                .fetch_one(&control)
                .await
                .unwrap(),
            1
        );
    }
}

#[cfg(feature = "import")]
#[tokio::test]
async fn every_local_atomic_target_wakes_before_the_first_native_hint_finishes() {
    use mnesis_store::import::AtomicAppend;
    use mnesis_store::wake::{WakeRegistration, WakeSource};
    let Some((mut proxy, store, control)) = fixture().await else {
        return;
    };
    let writes = atomic_writes(3);
    let mut registrations: Vec<_> = writes
        .iter()
        .map(|write| store.register(Some(write.target.as_bytes())).unwrap())
        .collect();
    registrations.push(store.register(None).unwrap());
    let start = std::time::Instant::now();
    let waiting: Vec<_> = registrations.iter().map(WakeRegistration::arm).collect();
    proxy.fault.send_replace(Fault::HoldHint);
    let client = store.clone();
    let mut pending = JoinSet::new();
    pending.spawn(async move { client.atomic_append_many(&writes).await });
    proxy.observed(Observation::Hint).await;
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "the periodic fallback must not satisfy the local-wake oracle"
    );
    for wait in waiting {
        assert!(
            wait.now_or_never().is_some(),
            "every stream and global wait must already be ready while native hints are blocked"
        );
    }
    assert_eq!(count(&control).await, 9);
    proxy.fault.send_replace(Fault::Pass);
    assert!(pending.join_next().await.unwrap().unwrap().is_ok());
}

#[cfg(feature = "import")]
#[tokio::test]
async fn caller_timeout_after_atomic_commit_is_not_rollback() {
    use mnesis_store::import::AtomicAppend;
    let Some((mut proxy, store, control)) = fixture().await else {
        return;
    };
    proxy.fault.send_replace(Fault::HoldCommit);
    let writes = atomic_writes(2);
    let timed_out = {
        let mut save = pin!(store.atomic_append_many(&writes));
        tokio::select! {
            () = proxy.observed(Observation::Committed) => {},
            result = &mut save => panic!("commit acknowledgement was not held: {result:?}"),
        }
        assert_eq!(count(&control).await, 6);
        timeout(Duration::from_millis(100), save).await.is_err()
    };
    assert!(timed_out);
    assert_eq!(
        count(&control).await,
        6,
        "dropping the timed-out future cannot undo this commit"
    );
    proxy.fault.send_replace(Fault::Pass);
    assert_eq!(
        store
            .read_all(None)
            .await
            .unwrap()
            .try_collect::<Vec<_>>()
            .await
            .unwrap()
            .len(),
        6
    );
}

#[cfg(feature = "import")]
#[tokio::test]
async fn atomic_hint_deadline_covers_every_target_together() {
    use mnesis_store::import::AtomicAppend;
    let Some((mut proxy, store, control)) = fixture().await else {
        return;
    };
    proxy.fault.send_replace(Fault::HoldHint);
    let writes = atomic_writes(16);
    let mut save = pin!(store.atomic_append_many(&writes));
    tokio::select! {
        () = proxy.observed(Observation::Hint) => {},
        result = &mut save => panic!("first native hint acknowledgement was not held: {result:?}"),
    }
    let committed = timeout(Duration::from_millis(900), save)
        .await
        .expect("native hint waits must not multiply by the16 targets")
        .unwrap()
        .unwrap();
    assert_eq!(count(&control).await, 48);
    assert_eq!(committed.seq(), 48);
}

#[cfg(feature = "import")]
#[tokio::test]
async fn every_head_is_read_before_an_atomic_writer_can_obtain_its_transaction_id() {
    use mnesis_store::import::{AtomicAppend, PlannedAppend};
    let Some((old_proxy, old_store, control)) = fixture().await else {
        return;
    };
    drop(old_store);
    drop(old_proxy);
    let controller = PostgresStore::from_pool(control.clone()).await.unwrap();
    let mut proxy = Proxy::start_with(&control.connect_options(), Fault::HoldSecondHead);
    let backing = PgPoolOptions::new()
        .max_connections(4)
        .connect_with(proxy.options.clone())
        .await
        .unwrap();
    let writer = PostgresStore::from_pool(backing).await.unwrap();
    let first = StreamKey::from_bytes("pg5-order-first");
    let second = StreamKey::from_bytes(SECOND_HEAD);
    let make = |target, expected_version, version| PlannedAppend {
        target,
        expected_version,
        head: pending_envelope(Version::new(version).unwrap())
            .event_type("Order")
            .payload(b"value".as_slice())
            .build()
            .unwrap(),
        tail: Vec::new(),
    };
    let requests = [
        make(first.clone(), None, 1),
        make(second.clone(), Some(Version::INITIAL), 2),
    ];
    let mut append = pin!(writer.atomic_append_many(&requests));
    tokio::select! {
        () = proxy.observed(Observation::SecondHead) => {},
        result = &mut append => panic!("second head never reached the observed Bind boundary: {result:?}"),
    }
    let before_xid: Option<String> = sqlx::query_scalar("SELECT backend_xid::text FROM pg_stat_activity WHERE datname = current_database() AND application_name = $1 AND state = 'idle in transaction'")
        .bind(control.connect_options().get_application_name().unwrap()).fetch_one(&control).await.unwrap();
    let seed = events();
    let predecessor = controller
        .append(&second, None, PendingBatch::new(&seed[..1]).unwrap())
        .await
        .unwrap();
    proxy.fault.send_replace(Fault::Pass);
    let assigned = timeout(Duration::from_secs(3), append)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let delivered: Vec<_> = writer
        .read_all(None)
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(delivered.len(), 3);
    let versions: Vec<_> = delivered
        .iter()
        .filter(|row| row.1 == second)
        .map(|row| row.2.version().as_u64())
        .collect();
    assert_eq!(
        versions,
        [1, 2],
        "a successor must not obtain a transaction id before its committed predecessor; prior writer xid={before_xid:?}"
    );
    assert!(
        before_xid.is_none(),
        "head inspection allocated xid before the first write: {before_xid:?}"
    );
    assert!(assigned > predecessor);
    assert_eq!(delivered.last().unwrap().0, assigned);
    assert_eq!(delivered[0].0, predecessor);
    assert_eq!(delivered.iter().filter(|row| row.1 == first).count(), 1);
}

#[cfg(feature = "import")]
struct Collision {
    pending: JoinSet<
        Result<
            Option<mnesis_postgres::PgAllPos>,
            mnesis_store::import::AtomicAppendError<PostgresError>,
        >,
    >,
    winner: sqlx::Transaction<'static, sqlx::Postgres>,
    prefix: StreamKey,
}

#[cfg(feature = "import")]
impl Collision {
    async fn start(store: PostgresStore, control: &PgPool) -> Self {
        use mnesis_store::import::AtomicAppend;
        let target = StreamKey::from_bytes(format!("atomic-race-final-{}", std::process::id()));
        let number = support::key(control, target.as_bytes()).await;
        let mut winner = control.begin().await.unwrap();
        sqlx::query("INSERT INTO mnesis.events (stream_id, version, event_type, schema_version, payload) VALUES ($1, 1, 'Winner', 1, 'winner'::bytea)")
            .bind(number).execute(&mut *winner).await.unwrap();
        let prefix = StreamKey::from_bytes(format!("atomic-race-prefix-{}", std::process::id()));
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM mnesis.keys WHERE value = $1")
                .bind(prefix.as_bytes())
                .fetch_one(control)
                .await
                .unwrap(),
            0
        );
        let mut requests = atomic_writes(2);
        requests[0].target = prefix.clone();
        requests[1].target = target;
        let mut pending = JoinSet::new();
        pending.spawn(async move { store.atomic_append_many(&requests).await });
        wait_for_insert(control).await;
        Self {
            pending,
            winner,
            prefix,
        }
    }

    async fn finish(
        mut self,
        control: &PgPool,
    ) -> mnesis_store::import::AtomicAppendError<PostgresError> {
        self.winner.commit().await.unwrap();
        let result = self
            .pending
            .join_next()
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        let rows: Vec<(String, Vec<u8>)> =
            sqlx::query_as("SELECT version::text, payload FROM mnesis.events")
                .fetch_all(control)
                .await
                .unwrap();
        assert_eq!(rows, [("1".to_owned(), b"winner".to_vec())]);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM mnesis.keys WHERE value = $1")
                .bind(self.prefix.as_bytes())
                .fetch_one(control)
                .await
                .unwrap(),
            0
        );
        result
    }
}

#[cfg(feature = "import")]
#[tokio::test]
async fn final_atomic_unique_race_rolls_back_prefix_and_reports_the_observed_head() {
    use mnesis_store::import::AtomicAppendError;
    let Some((_proxy, store, control)) = fixture().await else {
        return;
    };
    let race = Collision::start(store, &control).await;
    let result = race.finish(&control).await;
    assert!(
        matches!(result, AtomicAppendError::Conflict { index: 1, actual: Some(version) } if version == Version::INITIAL),
        "wrong native race outcome: {result:?}"
    );
}

#[cfg(feature = "import")]
async fn restricted_writer(control: &PgPool) -> (PostgresStore, PgPool) {
    sqlx::raw_sql("DO $$ BEGIN IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'mnesis_atomic_writer') THEN CREATE ROLE mnesis_atomic_writer; END IF; END $$; GRANT USAGE ON SCHEMA mnesis TO mnesis_atomic_writer; GRANT SELECT ON mnesis._sqlx_migrations, mnesis.history TO mnesis_atomic_writer; GRANT SELECT, INSERT ON mnesis.keys, mnesis.events TO mnesis_atomic_writer; GRANT USAGE ON SEQUENCE mnesis.keys_number_seq, mnesis.events_global_seq_seq TO mnesis_atomic_writer")
        .execute(control).await.unwrap();
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .after_connect(|connection, _| {
            Box::pin(async move {
                sqlx::query("SET ROLE mnesis_atomic_writer")
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect_with(control.connect_options().as_ref().clone())
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT current_user::text")
            .fetch_one(&pool)
            .await
            .unwrap(),
        "mnesis_atomic_writer"
    );
    let store = PostgresStore::from_pool(pool.clone()).await.unwrap();
    (store, pool)
}

#[cfg(feature = "import")]
#[tokio::test]
async fn failed_atomic_head_reconciliation_retains_the_original_unique_source() {
    use mnesis_store::import::AtomicAppendError;
    let Some((_proxy, old_store, control)) = fixture().await else {
        return;
    };
    drop(old_store);
    let (store, pool) = restricted_writer(&control).await;
    let race = Collision::start(store, &control).await;
    sqlx::query("REVOKE SELECT ON mnesis.keys FROM mnesis_atomic_writer")
        .execute(&control)
        .await
        .unwrap();
    let denied = sqlx::query_scalar::<_, i64>("SELECT number FROM mnesis.keys LIMIT 1")
        .fetch_optional(&pool)
        .await
        .unwrap_err();
    assert_eq!(
        denied.as_database_error().unwrap().code().as_deref(),
        Some("42501")
    );
    let result = race.finish(&control).await;
    sqlx::query("GRANT SELECT ON mnesis.keys TO mnesis_atomic_writer")
        .execute(&control)
        .await
        .unwrap();
    pool.close().await;
    sqlx::raw_sql("DROP OWNED BY mnesis_atomic_writer; DROP ROLE mnesis_atomic_writer")
        .execute(&control)
        .await
        .unwrap();
    assert!(
        matches!(result, AtomicAppendError::Store(PostgresError::Sqlx(ref source))
        if source.as_database_error().and_then(sqlx::error::DatabaseError::code).as_deref() == Some("23505")
        && source.as_database_error().and_then(sqlx::error::DatabaseError::constraint) == Some("events_stream_id_version_key")),
        "original native unique source was replaced or misclassified: {result:?}"
    );
}

#[cfg(feature = "import")]
fn assert_foreign_unique(error: PostgresError) {
    let PostgresError::Sqlx(sqlx::Error::Database(database)) = error else {
        panic!("foreign unique source was not retained: {error:?}")
    };
    let native = database
        .try_downcast_ref::<sqlx::postgres::PgDatabaseError>()
        .unwrap();
    assert_eq!(native.schema(), Some("mnesis_foreign"));
    assert_eq!(native.table(), Some("events"));
    assert_eq!(native.constraint(), Some("events_stream_id_version_key"));
    assert_eq!(native.code(), "23505");
}

#[cfg(feature = "import")]
#[tokio::test]
async fn identical_constraint_names_in_a_foreign_schema_are_not_optimistic_conflicts() {
    use mnesis_store::import::{AtomicAppend, AtomicAppendError, PlannedAppend};
    let Some((_proxy, store, control)) = fixture().await else {
        return;
    };
    let id = StreamKey::from_bytes("foreign-schema");
    let rows = events();
    store
        .append(&id, None, PendingBatch::new(&rows[..1]).unwrap())
        .await
        .unwrap();
    sqlx::raw_sql("CREATE SCHEMA mnesis_foreign; CREATE TABLE mnesis_foreign.events (stream_id bigint, version bigint, CONSTRAINT events_stream_id_version_key UNIQUE (stream_id, version)); INSERT INTO mnesis_foreign.events VALUES (1, 1); CREATE FUNCTION mnesis.foreign_unique() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN INSERT INTO mnesis_foreign.events VALUES (1, 1); RETURN NEW; END $$; CREATE TRIGGER foreign_unique BEFORE INSERT ON mnesis.events FOR EACH ROW EXECUTE FUNCTION mnesis.foreign_unique()")
        .execute(&control).await.unwrap();
    let native = sqlx::query("INSERT INTO mnesis_foreign.events VALUES (1, 1)")
        .execute(&control)
        .await
        .unwrap_err();
    assert_foreign_unique(PostgresError::Sqlx(native));
    let raw = store
        .append(
            &id,
            Some(Version::INITIAL),
            PendingBatch::new(&rows[1..2]).unwrap(),
        )
        .await;
    let atomic = store
        .atomic_append_many(&[PlannedAppend {
            target: id.clone(),
            expected_version: Some(Version::INITIAL),
            head: rows[1].clone(),
            tail: Vec::new(),
        }])
        .await;
    sqlx::raw_sql("DROP TRIGGER foreign_unique ON mnesis.events; DROP FUNCTION mnesis.foreign_unique(); DROP SCHEMA mnesis_foreign CASCADE")
        .execute(&control).await.unwrap();
    assert_eq!(count(&control).await, 1);
    match (raw, atomic) {
        (Err(AppendError::Store(first)), Err(AtomicAppendError::Store(second))) => {
            assert_foreign_unique(first);
            assert_foreign_unique(second);
        }
        wrong => panic!("foreign-schema unique errors misclassified: {wrong:?}"),
    }
}

#[cfg(feature = "import")]
#[tokio::test]
async fn real_serialization_failure_preserves_definite_rejection_and_no_owned_event() {
    let Some((_proxy, unused, control)) = fixture().await else {
        return;
    };
    drop(unused);
    let first = StreamKey::from_bytes("ssi-a");
    let second = StreamKey::from_bytes("ssi-b");
    let first_number = support::key(&control, first.as_bytes()).await;
    let second_number = support::key(&control, second.as_bytes()).await;
    sqlx::raw_sql("CREATE OR REPLACE FUNCTION mnesis.ssi_read() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM count(*) FROM mnesis.events WHERE stream_id = (SELECT number FROM mnesis.keys WHERE value = 'ssi-b'::bytea); PERFORM pg_advisory_xact_lock(319878); RETURN NEW; END $$; CREATE TRIGGER ssi_read AFTER INSERT ON mnesis.events FOR EACH ROW WHEN (NEW.event_type = 'Ssi'::bytea) EXECUTE FUNCTION mnesis.ssi_read()")
        .execute(&control).await.unwrap();
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .after_connect(|connection, _| {
            Box::pin(async move {
                sqlx::query("SET default_transaction_isolation = 'serializable'")
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect_with(control.connect_options().as_ref().clone())
        .await
        .unwrap();
    let store = PostgresStore::from_pool(pool.clone()).await.unwrap();
    let mut gate = control.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(319878)")
        .execute(&mut *gate)
        .await
        .unwrap();
    let mut other = control.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *other)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM mnesis.events WHERE stream_id = $1")
            .bind(first_number)
            .fetch_one(&mut *other)
            .await
            .unwrap(),
        0
    );
    let mut pending = JoinSet::new();
    pending.spawn(async move {
        let event = pending_envelope(Version::INITIAL)
            .event_type("Ssi")
            .payload(b"sut".as_slice())
            .build()
            .unwrap();
        store
            .append(&first, None, PendingBatch::new(&[event]).unwrap())
            .await
    });
    wait_for_ssi_insert(&control).await;
    sqlx::query("INSERT INTO mnesis.events (stream_id, version, event_type, schema_version, payload) VALUES ($1, 1, 'Other', 1, 'other'::bytea)")
        .bind(second_number).execute(&mut *other).await.unwrap();
    other.commit().await.unwrap();
    gate.commit().await.unwrap();
    let result = timeout(Duration::from_secs(3), pending.join_next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    sqlx::raw_sql("DROP TRIGGER ssi_read ON mnesis.events; DROP FUNCTION mnesis.ssi_read()")
        .execute(&control)
        .await
        .unwrap();
    pool.close().await;
    assert!(
        matches!(result, Err(AppendError::Store(PostgresError::Sqlx(ref source))) if source.as_database_error().and_then(sqlx::error::DatabaseError::code).as_deref() == Some("40001")),
        "actual serialization outcome: {result:?}"
    );
    let rows: Vec<(i64, Vec<u8>)> = sqlx::query_as("SELECT stream_id, payload FROM mnesis.events")
        .fetch_all(&control)
        .await
        .unwrap();
    assert_eq!(rows, [(second_number, b"other".to_vec())]);
}

#[cfg(feature = "import")]
#[tokio::test]
async fn real_deadlock_aborts_the_atomic_writer_without_optimistic_reclassification() {
    use mnesis_store::import::{AtomicAppend, AtomicAppendError};
    let Some((_proxy, unused, control)) = fixture().await else {
        return;
    };
    drop(unused);
    let requests = atomic_writes(2);
    let first = support::key(&control, requests[0].target.as_bytes()).await;
    let second = support::key(&control, requests[1].target.as_bytes()).await;
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .after_connect(|connection, _| {
            Box::pin(async move {
                sqlx::query("SET deadlock_timeout = '3s'")
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect_with(control.connect_options().as_ref().clone())
        .await
        .unwrap();
    let store = PostgresStore::from_pool(pool.clone()).await.unwrap();
    let mut other = control.begin().await.unwrap();
    sqlx::query("SET LOCAL deadlock_timeout = '30s'")
        .execute(&mut *other)
        .await
        .unwrap();
    sqlx::query("INSERT INTO mnesis.events (stream_id, version, event_type, schema_version, payload) VALUES ($1, 1, 'Control', 1, 'control'::bytea)")
        .bind(second).execute(&mut *other).await.unwrap();
    let mut pending = JoinSet::new();
    pending.spawn(async move { store.atomic_append_many(&requests).await });
    wait_for_insert(&control).await;
    timeout(Duration::from_secs(10), sqlx::query("INSERT INTO mnesis.events (stream_id, version, event_type, schema_version, payload) VALUES ($1, 1, 'Control', 1, 'control'::bytea)")
        .bind(first).execute(&mut *other)).await.expect("native detector must break the real wait cycle").unwrap();
    other.commit().await.unwrap();
    let result = pending.join_next().await.unwrap().unwrap();
    pool.close().await;
    assert!(
        matches!(result, Err(AtomicAppendError::Store(PostgresError::Sqlx(ref source))) if source.as_database_error().and_then(sqlx::error::DatabaseError::code).as_deref() == Some("40P01")),
        "actual deadlock outcome: {result:?}"
    );
    let rows: Vec<Vec<u8>> = sqlx::query_scalar("SELECT payload FROM mnesis.events")
        .fetch_all(&control)
        .await
        .unwrap();
    assert_eq!(rows, [b"control".to_vec(), b"control".to_vec()]);
}

#[cfg(feature = "import")]
async fn wait_for_ssi_insert(control: &PgPool) {
    let name = control
        .connect_options()
        .get_application_name()
        .unwrap()
        .to_owned();
    timeout(Duration::from_secs(3), async {
        loop {
            let waiting: bool = sqlx::query_scalar("SELECT EXISTS (SELECT FROM pg_locks l JOIN pg_stat_activity a ON a.pid = l.pid WHERE l.locktype = 'advisory' AND l.objid = 319878 AND NOT l.granted AND a.datname = current_database() AND a.application_name = $1 AND a.query LIKE 'INSERT INTO %events %')")
                .bind(&name).fetch_one(control).await.unwrap();
            if waiting { break }
            yield_now().await;
        }
    }).await.expect("SUT must read B then block in its actual A insert");
}
