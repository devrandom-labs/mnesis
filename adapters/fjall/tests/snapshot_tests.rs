#![cfg(feature = "snapshot")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::shadow_reuse,
    clippy::shadow_same,
    clippy::shadow_unrelated,
    clippy::as_conversions,
    reason = "test harness — relaxed lints for test code"
)]

use std::num::NonZeroU32;

use mnesis::Version;
use mnesis_fjall::FjallStore;
use mnesis_store::StreamKey;
use mnesis_store::state::{Hydrated, SnapshotStore};

const SV1: NonZeroU32 = NonZeroU32::MIN;

fn sk(s: &str) -> StreamKey {
    StreamKey::from_slice(s.as_bytes())
}

fn temp_store() -> (FjallStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = FjallStore::builder(dir.path().join("db")).open().unwrap();
    (store, dir)
}

/// Helper: append events to create a stream so snapshots have something to reference.
async fn setup_stream(store: &FjallStore, id: &StreamKey, event_count: u64) {
    use mnesis_store::envelope::pending_envelope;
    use mnesis_store::store::RawEventStore;

    let mut envs = Vec::new();
    for i in 1..=event_count {
        envs.push(
            pending_envelope(Version::new(i).unwrap())
                .event_type("TestEvent")
                .payload(format!("payload-{i}").into_bytes())
                .build()
                .expect("valid envelope"),
        );
    }
    store.append(id, None, &envs).await.unwrap();
}

// ── 3. Defensive Boundary Tests ────────────────────────────────────

#[tokio::test]
async fn hydrate_id_without_snapshot_returns_none() {
    let (store, _dir) = temp_store();
    let id = sk("agg-1");
    setup_stream(&store, &id, 3).await;

    let result = SnapshotStore::<Vec<u8>, Version>::hydrate(&store, &id, SV1)
        .await
        .unwrap();
    assert_eq!(result, Hydrated::Absent);
}

#[tokio::test]
async fn commit_without_event_stream_is_persisted() {
    let (store, _dir) = temp_store();
    // No event stream exists — commit writes the snapshot unconditionally.
    store
        .commit(&sk("nope"), SV1, Version::new(1).unwrap(), &vec![1])
        .await
        .unwrap();

    // And hydrate reads it back regardless of stream existence.
    let (version, state) = SnapshotStore::<Vec<u8>, Version>::hydrate(&store, &sk("nope"), SV1)
        .await
        .unwrap()
        .into_found()
        .unwrap();
    assert_eq!(version, Version::new(1).unwrap());
    assert_eq!(state, vec![1]);
}
