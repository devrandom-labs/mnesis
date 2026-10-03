//! Process termination tests. These establish process-crash recovery, not
//! power-loss survival or behavior of a device that ignores sync requests.
#![cfg(all(feature = "import", feature = "snapshot", feature = "projection"))]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test assertions and child-process handshake"
)]

use mnesis_store::checkpoint::{CheckpointMode, CheckpointStore, CheckpointWrite};
#[path = "support/checkpoint.rs"]
mod checkpoint;
use futures::TryStreamExt;
use mnesis::Version;
use mnesis_fjall::{AllIndex, Durability, FjallStore, GlobalSeq};
use mnesis_store::envelope::pending_envelope;
use mnesis_store::import::{AtomicAppend, PlannedAppend};
use mnesis_store::state::SnapshotStore;
use mnesis_store::{PendingBatch, RawEventStore, StreamKey};
use std::io::Write;
use std::num::NonZeroU32;
use std::process::Command;

mod support;

const CHILD_PATH: &str = "MNESIS_RECOVERY_TEST_PATH";
const ACK: &str = "MNESIS_WRITES_ACKNOWLEDGED";

#[test]
fn acknowledged_writes_survive_process_termination() {
    if let Ok(path) = std::env::var(CHILD_PATH) {
        futures::executor::block_on(write_child(&path));
        return;
    }
    for policy in ["all", "data", "buffered", "buffered-flush"] {
        for mode in ["enabled", "disabled"] {
            let dir = tempfile::tempdir().unwrap();
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "acknowledged_writes_survive_process_termination",
                    "--nocapture",
                ])
                .env(CHILD_PATH, dir.path())
                .env("MNESIS_RECOVERY_TEST_POLICY", policy)
                .env("MNESIS_RECOVERY_TEST_INDEX", mode);
            support::kill_after_marker(&mut command, ACK);
            futures::executor::block_on(check_recovered(dir.path(), mode));
        }
    }
}

async fn write_child(path: &str) {
    let policy = std::env::var("MNESIS_RECOVERY_TEST_POLICY").unwrap();
    let mode = std::env::var("MNESIS_RECOVERY_TEST_INDEX").unwrap();
    let durability = match policy.as_str() {
        "all" => Durability::SyncAll,
        "data" => Durability::SyncData,
        _ => Durability::Buffered,
    };
    let store = FjallStore::builder(path)
        .all_index(index_mode(&mode))
        .durability(durability)
        .open()
        .unwrap();
    let id = StreamKey::from_slice(b"a");
    let first = event(1);
    store
        .append(&id, None, PendingBatch::of(&first))
        .await
        .unwrap();
    let writes = [
        PlannedAppend {
            target: id.clone(),
            expected_version: Some(Version::INITIAL),
            head: event(2),
            tail: vec![],
        },
        PlannedAppend {
            target: StreamKey::from_slice(b"b"),
            expected_version: None,
            head: event(1),
            tail: vec![],
        },
    ];
    let position = store.atomic_append_many(&writes).await.unwrap().unwrap();
    let schema = NonZeroU32::new(1).unwrap();
    let state = vec![4, 5, 6];
    <FjallStore as SnapshotStore<Vec<u8>, Version>>::commit(
        &store,
        &id,
        schema,
        Version::new(2).unwrap(),
        &state,
    )
    .await
    .unwrap();
    <FjallStore as CheckpointStore<Vec<u8>, GlobalSeq>>::commit_checkpoint(
        &store,
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
    if policy == "buffered-flush" {
        store.flush().await.unwrap();
    }
    let mut output = std::io::stdout().lock();
    writeln!(output, "{ACK}").unwrap();
    output.flush().unwrap();
    drop(output);
    // Keep every handle alive until the parent kills us. The pipe handshake
    // prevents graceful drop and avoids timing-based ordering.
    loop {
        std::thread::park();
    }
}

fn index_mode(mode: &str) -> AllIndex {
    if mode == "enabled" {
        AllIndex::Denormalized
    } else {
        AllIndex::Disabled
    }
}

fn event(version: u64) -> mnesis_store::PendingEnvelope {
    pending_envelope(Version::new(version).unwrap())
        .event_type("E")
        .payload(vec![7])
        .build()
        .unwrap()
}

async fn check_recovered(path: &std::path::Path, mode: &str) {
    let store = FjallStore::builder(path)
        .all_index(index_mode(mode))
        .open()
        .unwrap();
    let id = StreamKey::from_slice(b"a");
    let other = StreamKey::from_slice(b"b");
    for (stream, count) in [(&id, 2), (&other, 1)] {
        let rows: Vec<_> = store
            .read_stream(stream, Version::INITIAL)
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(rows.len(), count);
        for (i, row) in rows.iter().enumerate() {
            assert_eq!(row.version().as_u64(), u64::try_from(i + 1).unwrap());
            assert_eq!(row.payload(), &[7]);
        }
    }
    if mode == "enabled" {
        let all: Vec<_> = store
            .read_all(None)
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(
            all.iter()
                .map(|(pos, _, _)| pos.as_u64())
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
    }
    let schema = NonZeroU32::new(1).unwrap();
    let snapshot = <FjallStore as SnapshotStore<Vec<u8>, Version>>::hydrate(&store, &id, schema)
        .await
        .unwrap()
        .into_found()
        .unwrap();
    assert_eq!(snapshot, (Version::new(2).unwrap(), vec![4, 5, 6]));
    let checkpoint = checkpoint::found(
        <FjallStore as CheckpointStore<Vec<u8>, GlobalSeq>>::hydrate_checkpoint(
            &store, &id, schema,
        )
        .await
        .unwrap(),
    );
    assert_eq!(
        checkpoint,
        (
            std::num::NonZeroU64::MIN,
            GlobalSeq::new(3).unwrap(),
            vec![4, 5, 6]
        )
    );
    assert_eq!(
        store
            .append(&other, Some(Version::INITIAL), PendingBatch::of(&event(2)))
            .await
            .unwrap()
            .as_u64(),
        4
    );
}
