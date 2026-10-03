//! Real journal syscall failures in isolated children. This tests EIO handling,
//! not loss of the OS page cache or a storage device's power-loss behavior.
#![cfg(all(
    any(target_os = "linux", target_os = "macos"),
    feature = "import",
    feature = "snapshot",
    feature = "projection"
))]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "fault-injection test assertions"
)]

use mnesis_store::checkpoint::{CheckpointError, CheckpointMode, CheckpointStore, CheckpointWrite};
#[path = "support/checkpoint.rs"]
mod checkpoint;
use futures::{FutureExt, TryStreamExt};
use mnesis::Version;
use mnesis_fjall::{Durability, FjallError, FjallStore, GlobalSeq};
use mnesis_store::envelope::pending_envelope;
use mnesis_store::error::AppendError;
use mnesis_store::import::{AtomicAppend, AtomicAppendError, PlannedAppend};
use mnesis_store::state::SnapshotStore;
use mnesis_store::wake::{WakeRegistration, WakeSource};
use mnesis_store::{PendingBatch, RawEventStore, StreamKey};
use std::io::Write;
use std::num::NonZeroU32;
use std::path::Path;
use std::process::Command;

mod support;

const PATH_ENV: &str = "MNESIS_FAULT_DB";
const MARKER: &str = "MNESIS_FAULT_ASSERTIONS_PASSED";

#[test]
fn journal_write_and_sync_failures_are_reported_without_partial_transactions() {
    if let Ok(path) = std::env::var(PATH_ENV) {
        futures::executor::block_on(fault_child(Path::new(&path)));
        return;
    }
    let library_dir = tempfile::tempdir().unwrap();
    let library = compile_injector(library_dir.path());
    for operation in ["write", "sync"] {
        for action in ["append", "atomic", "snapshot", "projection", "flush"] {
            for policy in ["all", "data"] {
                if action == "flush" && operation == "write" {
                    continue;
                }
                let dir = tempfile::tempdir().unwrap();
                let mut command = Command::new(std::env::current_exe().unwrap());
                command
                    .args([
                        "--exact",
                        "journal_write_and_sync_failures_are_reported_without_partial_transactions",
                        "--nocapture",
                    ])
                    .env(PATH_ENV, dir.path())
                    .env("MNESIS_FAULT_MARKER", dir.path().join("armed"))
                    .env("MNESIS_FAULT_OPERATION", operation)
                    .env("MNESIS_FAULT_ACTION", action)
                    .env("MNESIS_FAULT_POLICY", policy);
                #[cfg(target_os = "macos")]
                command.env("DYLD_INSERT_LIBRARIES", &library);
                #[cfg(target_os = "linux")]
                command.env("LD_PRELOAD", &library);
                support::kill_after_marker(&mut command, MARKER);
                futures::executor::block_on(check_recovered(dir.path(), action, operation));
            }
        }
    }
}

fn compile_injector(dir: &Path) -> std::path::PathBuf {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/journal_faults.c");
    let library = dir.join("journal_faults.so");
    let mut command = Command::new("cc");
    command.args(["-Wall", "-Wextra", "-Werror"]);
    #[cfg(target_os = "macos")]
    command.arg("-dynamiclib");
    #[cfg(target_os = "linux")]
    command.args(["-shared", "-fPIC"]);
    command.arg(&source).arg("-o").arg(&library);
    #[cfg(target_os = "linux")]
    command.arg("-ldl");
    assert!(
        command.status().unwrap().success(),
        "compile syscall injector"
    );
    library
}

const fn schema() -> NonZeroU32 {
    NonZeroU32::new(1).unwrap()
}

fn event(version: u64, payload: Vec<u8>) -> mnesis_store::PendingEnvelope {
    pending_envelope(Version::new(version).unwrap())
        .event_type("E")
        .payload(payload)
        .build()
        .unwrap()
}

async fn fault_child(path: &Path) {
    let policy = std::env::var("MNESIS_FAULT_POLICY").unwrap();
    let operation = std::env::var("MNESIS_FAULT_OPERATION").unwrap();
    let action = std::env::var("MNESIS_FAULT_ACTION").unwrap();
    let store = FjallStore::builder(path.join("db"))
        .durability(if policy == "all" {
            Durability::SyncAll
        } else {
            Durability::SyncData
        })
        .open()
        .unwrap();
    let id = StreamKey::from_slice(b"a");
    store
        .append(&id, None, PendingBatch::of(&event(1, vec![7])))
        .await
        .unwrap();
    let old = vec![8];
    <FjallStore as SnapshotStore<Vec<u8>, Version>>::commit(
        &store,
        &id,
        schema(),
        Version::INITIAL,
        &old,
    )
    .await
    .unwrap();
    <FjallStore as CheckpointStore<Vec<u8>, GlobalSeq>>::commit_checkpoint(
        &store,
        &id,
        CheckpointWrite {
            expected: None,
            schema_version: schema(),
            position: GlobalSeq::INITIAL,
            state: &old,
            mode: CheckpointMode::Advance,
        },
    )
    .await
    .unwrap();
    let registration = store.register(None).unwrap();
    let wake = registration.arm();
    // Opening an empty marker arms faults without performing a write syscall.
    std::fs::File::create(path.join("armed")).unwrap();
    let failure = attempt(&store, &id, &action).await.expect_err(&format!(
        "{operation}/{action}/{policy} must hit the injected syscall"
    ));
    if operation == "sync" {
        assert!(
            matches!(failure, FjallError::Io(fjall::Error::Poisoned)),
            "real persistence failure must poison Fjall: {failure:?}"
        );
    } else {
        assert!(
            matches!(failure, FjallError::Io(fjall::Error::Io(_))),
            "large frame must fail during journal write_batch: {failure:?}"
        );
    }
    assert!(
        wake.now_or_never().is_none(),
        "failed writes must not announce success"
    );
    // The failed transaction was not published to live readers, even if its
    // complete journal entry can be recovered after a failed sync.
    let rows: Vec<_> = store
        .read_stream(&id, Version::INITIAL)
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    let mut output = std::io::stdout().lock();
    writeln!(output, "{MARKER}").unwrap();
    output.flush().unwrap();
    drop(output);
    loop {
        std::thread::park();
    }
}

#[allow(
    clippy::result_large_err,
    clippy::panic,
    reason = "adapter error type and diagnostics for unexpected test fixture validation failures"
)]
async fn attempt(store: &FjallStore, id: &StreamKey, action: &str) -> Result<(), FjallError> {
    let state = fault_payload();
    match action {
        "append" => store
            .append(
                id,
                Some(Version::INITIAL),
                PendingBatch::of(&event(2, state)),
            )
            .await
            .map(|_| ())
            .map_err(|error| match error {
                AppendError::Store(source) => source,
                other => panic!("unexpected validation failure: {other:?}"),
            }),
        "atomic" => {
            let writes = [
                PlannedAppend {
                    target: id.clone(),
                    expected_version: Some(Version::INITIAL),
                    head: event(2, state.clone()),
                    tail: vec![],
                },
                PlannedAppend {
                    target: StreamKey::from_slice(b"b"),
                    expected_version: None,
                    head: event(1, state),
                    tail: vec![],
                },
            ];
            store
                .atomic_append_many(&writes)
                .await
                .map(|_| ())
                .map_err(|error| match error {
                    AtomicAppendError::Store(source) => source,
                    other => panic!("unexpected validation failure: {other:?}"),
                })
        }
        "snapshot" => {
            <FjallStore as SnapshotStore<Vec<u8>, Version>>::commit(
                store,
                id,
                schema(),
                Version::new(2).unwrap(),
                &state,
            )
            .await
        }
        "projection" => <FjallStore as CheckpointStore<Vec<u8>, GlobalSeq>>::commit_checkpoint(
            store,
            id,
            CheckpointWrite {
                expected: Some(std::num::NonZeroU64::MIN),
                schema_version: schema(),
                position: GlobalSeq::new(2).unwrap(),
                state: &state,
                mode: CheckpointMode::Advance,
            },
        )
        .await
        .map(|_| ())
        .map_err(|error| match error {
            CheckpointError::Store(source) => source,
            other => panic!("unexpected checkpoint rejection: {other:?}"),
        }),
        "flush" => store.flush().await,
        _ => panic!("unknown fault action"),
    }
}

fn fault_payload() -> Vec<u8> {
    // Deterministic incompressible bytes exceed the journal's 8 KiB buffer.
    let mut value = 0x1234_5678_9abc_def0_u64;
    (0..65_536)
        .map(|_| {
            value ^= value << 13;
            value ^= value >> 7;
            value ^= value << 17;
            value.to_le_bytes()[0]
        })
        .collect()
}

async fn check_recovered(path: &Path, action: &str, operation: &str) {
    let store = FjallStore::builder(path.join("db")).open().unwrap();
    let id = StreamKey::from_slice(b"a");
    let other = StreamKey::from_slice(b"b");
    let all: Vec<_> = store
        .read_all(None)
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    let rows: Vec<_> = store
        .read_stream(&id, Version::INITIAL)
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    let other_rows: Vec<_> = store
        .read_stream(&other, Version::INITIAL)
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    check_events(&all, &rows, &other_rows, action, operation);
    let snapshot = <FjallStore as SnapshotStore<Vec<u8>, Version>>::hydrate(&store, &id, schema())
        .await
        .unwrap()
        .into_found()
        .unwrap();
    let projection = checkpoint::found(
        <FjallStore as CheckpointStore<Vec<u8>, GlobalSeq>>::hydrate_checkpoint(
            &store,
            &id,
            schema(),
        )
        .await
        .unwrap(),
    );
    check_state(
        snapshot.0.as_u64(),
        &snapshot.1,
        action == "snapshot" && operation == "sync",
    );
    assert_eq!(projection.0.get(), projection.1.as_u64());
    check_state(
        projection.1.as_u64(),
        &projection.2,
        action == "projection" && operation == "sync",
    );
    let next = store
        .append(
            &id,
            Version::new(u64::try_from(rows.len()).unwrap()),
            PendingBatch::of(&event(u64::try_from(rows.len() + 1).unwrap(), vec![9])),
        )
        .await
        .unwrap();
    assert_eq!(next.as_u64(), u64::try_from(all.len() + 1).unwrap());
}

fn check_events(
    all: &[(GlobalSeq, StreamKey, mnesis_store::PersistedEnvelope)],
    rows: &[mnesis_store::PersistedEnvelope],
    other_rows: &[mnesis_store::PersistedEnvelope],
    action: &str,
    operation: &str,
) {
    let id = StreamKey::from_slice(b"a");
    let other = StreamKey::from_slice(b"b");
    assert_eq!(rows[0].payload(), &[7], "acknowledged event must survive");
    match action {
        "append" => {
            assert!(rows.len() == 1 || rows.len() == 2);
            assert!(other_rows.is_empty());
        }
        "atomic" => assert!(
            matches!((rows.len(), other_rows.len()), (1, 0) | (2, 1)),
            "recover the entire transaction or none of it"
        ),
        _ => {
            assert_eq!(rows.len(), 1);
            assert!(other_rows.is_empty());
        }
    }
    if operation == "write" {
        assert_eq!(rows.len(), 1);
        assert!(other_rows.is_empty());
    }
    for (i, row) in rows.iter().enumerate() {
        assert_eq!(row.version().as_u64(), u64::try_from(i + 1).unwrap());
    }
    if rows.len() == 2 {
        assert_eq!(rows[1].payload(), fault_payload());
    }
    if let Some(row) = other_rows.first() {
        assert_eq!(row.version(), Version::INITIAL);
        assert_eq!(row.payload(), fault_payload());
    }
    assert_eq!(all.len(), rows.len() + other_rows.len());
    for (i, (position, _, _)) in all.iter().enumerate() {
        assert_eq!(position.as_u64(), u64::try_from(i + 1).unwrap());
        let (_, stream, envelope) = &all[i];
        let source = if stream == &id {
            rows
        } else {
            assert_eq!(stream, &other);
            other_rows
        };
        let index = usize::try_from(envelope.version().as_u64() - 1).unwrap();
        assert_eq!(envelope.payload(), source[index].payload());
    }
}

fn check_state(position: u64, state: &[u8], may_recover_new: bool) {
    if may_recover_new && position == 2 {
        assert_eq!(state, fault_payload());
    } else {
        assert_eq!(position, 1);
        assert_eq!(state, &[8]);
    }
}
