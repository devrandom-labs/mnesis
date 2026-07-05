#![cfg(feature = "projection")]
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

//! `SnapshotStore<Vec<u8>, GlobalSeq>` — the fjall-backed projection-state
//! adapter (issue #164). Projection state and the `$all` [`GlobalSeq`] it was
//! folded up to are committed together into the dedicated `projections`
//! partition. These are the public-API tests; the corrupt-bytes defensive test
//! is a white-box unit test in `src/store.rs` (it needs raw partition access).

use std::num::NonZeroU32;

use nexus_fjall::{FjallStore, GlobalSeq};
use nexus_store::StreamKey;
use nexus_store::state::SnapshotStore;
// Only the cross-partition collision test (both features) needs `Version`.
#[cfg(feature = "snapshot")]
use nexus::Version;

const SV1: NonZeroU32 = NonZeroU32::MIN;

fn pk(s: &str) -> StreamKey {
    StreamKey::from_slice(s.as_bytes())
}

const fn gs(v: u64) -> GlobalSeq {
    GlobalSeq::new(v).unwrap()
}

fn temp_store() -> (FjallStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = FjallStore::builder(dir.path().join("db")).open().unwrap();
    (store, dir)
}

// ── 1. Sequence/Protocol Tests ─────────────────────────────────────

#[tokio::test]
async fn commit_then_hydrate_roundtrips() {
    let (store, _dir) = temp_store();
    let id = pk("proj-1");

    SnapshotStore::<Vec<u8>, GlobalSeq>::commit(&store, &id, SV1, gs(7), &vec![1, 2, 3])
        .await
        .unwrap();

    let (pos, state) = SnapshotStore::<Vec<u8>, GlobalSeq>::hydrate(&store, &id, SV1)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pos, gs(7));
    assert_eq!(state, vec![1, 2, 3]);
}

#[tokio::test]
async fn commit_overwrites_previous_projection() {
    let (store, _dir) = temp_store();
    let id = pk("proj-1");

    SnapshotStore::<Vec<u8>, GlobalSeq>::commit(&store, &id, SV1, gs(5), &vec![1])
        .await
        .unwrap();

    let sv2 = NonZeroU32::new(2).unwrap();
    SnapshotStore::<Vec<u8>, GlobalSeq>::commit(&store, &id, sv2, gs(42), &vec![2, 3])
        .await
        .unwrap();

    let (pos, state) = SnapshotStore::<Vec<u8>, GlobalSeq>::hydrate(&store, &id, sv2)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pos, gs(42));
    assert_eq!(state, vec![2, 3]);

    // Old schema version is filtered at the store level → absent.
    assert!(
        SnapshotStore::<Vec<u8>, GlobalSeq>::hydrate(&store, &id, SV1)
            .await
            .unwrap()
            .is_none()
    );
}

// ── 2. Lifecycle Tests ─────────────────────────────────────────────

#[tokio::test]
async fn projection_persists_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("db");
    let id = pk("proj-1");

    // First session: commit projection state at GlobalSeq 9.
    {
        let store = FjallStore::builder(&db_path).open().unwrap();
        SnapshotStore::<Vec<u8>, GlobalSeq>::commit(&store, &id, SV1, gs(9), &vec![42, 43, 44])
            .await
            .unwrap();
    }

    // Second session: reopen and hydrate the exact same (GlobalSeq, state).
    {
        let store = FjallStore::builder(&db_path).open().unwrap();
        let (pos, state) = SnapshotStore::<Vec<u8>, GlobalSeq>::hydrate(&store, &id, SV1)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(pos, gs(9));
        assert_eq!(state, vec![42, 43, 44]);
    }
}

// ── 3. Defensive Boundary Tests ────────────────────────────────────

#[tokio::test]
async fn hydrate_unknown_id_returns_none() {
    let (store, _dir) = temp_store();
    let result: Option<(GlobalSeq, Vec<u8>)> =
        SnapshotStore::<Vec<u8>, GlobalSeq>::hydrate(&store, &pk("nope"), SV1)
            .await
            .unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn hydrate_schema_mismatch_returns_none() {
    let (store, _dir) = temp_store();
    let id = pk("proj-1");
    SnapshotStore::<Vec<u8>, GlobalSeq>::commit(&store, &id, SV1, gs(3), &vec![9])
        .await
        .unwrap();

    let other = NonZeroU32::new(2).unwrap();
    assert!(
        SnapshotStore::<Vec<u8>, GlobalSeq>::hydrate(&store, &id, other)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn commit_at_initial_global_seq_roundtrips() {
    let (store, _dir) = temp_store();
    let id = pk("proj-1");
    // Boundary: the very first GlobalSeq (1).
    SnapshotStore::<Vec<u8>, GlobalSeq>::commit(&store, &id, SV1, GlobalSeq::INITIAL, &vec![0])
        .await
        .unwrap();

    let (pos, state) = SnapshotStore::<Vec<u8>, GlobalSeq>::hydrate(&store, &id, SV1)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pos, GlobalSeq::INITIAL);
    assert_eq!(state, vec![0]);
}

#[tokio::test]
async fn commit_at_max_global_seq_roundtrips() {
    let (store, _dir) = temp_store();
    let id = pk("proj-1");
    // Boundary: the largest GlobalSeq (u64::MAX).
    SnapshotStore::<Vec<u8>, GlobalSeq>::commit(&store, &id, SV1, gs(u64::MAX), &vec![7])
        .await
        .unwrap();

    let (pos, state) = SnapshotStore::<Vec<u8>, GlobalSeq>::hydrate(&store, &id, SV1)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pos, gs(u64::MAX));
    assert_eq!(state, vec![7]);
}

#[tokio::test]
async fn commit_empty_state_roundtrips() {
    let (store, _dir) = temp_store();
    let id = pk("proj-1");
    SnapshotStore::<Vec<u8>, GlobalSeq>::commit(&store, &id, SV1, gs(4), &Vec::new())
        .await
        .unwrap();

    let (pos, state) = SnapshotStore::<Vec<u8>, GlobalSeq>::hydrate(&store, &id, SV1)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pos, gs(4));
    assert!(state.is_empty());
}

// ── 4. Isolation Tests ─────────────────────────────────────────────

#[tokio::test]
async fn different_projections_are_independent() {
    let (store, _dir) = temp_store();
    let id1 = pk("proj-1");
    let id2 = pk("proj-2");

    SnapshotStore::<Vec<u8>, GlobalSeq>::commit(&store, &id1, SV1, gs(5), &vec![1])
        .await
        .unwrap();
    SnapshotStore::<Vec<u8>, GlobalSeq>::commit(&store, &id2, SV1, gs(10), &vec![2])
        .await
        .unwrap();

    let (pos1, state1) = SnapshotStore::<Vec<u8>, GlobalSeq>::hydrate(&store, &id1, SV1)
        .await
        .unwrap()
        .unwrap();
    let (pos2, state2) = SnapshotStore::<Vec<u8>, GlobalSeq>::hydrate(&store, &id2, SV1)
        .await
        .unwrap()
        .unwrap();

    assert_eq!((pos1, state1), (gs(5), vec![1]));
    assert_eq!((pos2, state2), (gs(10), vec![2]));
}

/// The reason the `projections` partition is separate from `snapshots`: an
/// aggregate snapshot and a projection checkpoint stored under the **same id
/// bytes** must not clobber each other. This is the one test that defends the
/// partition split — it FAILS if the two `SnapshotStore` impls ever share a
/// keyspace (a merged partition would let the second `commit` overwrite the
/// first, so one `hydrate` would decode the other's value). Requires both
/// features, hence the `snapshot` gate on top of the file's `projection` gate.
#[cfg(feature = "snapshot")]
#[tokio::test]
async fn snapshot_and_projection_with_same_id_do_not_collide() {
    let (store, _dir) = temp_store();
    let id = pk("shared-id");

    // Same id bytes, two distinct stores, two distinct position types.
    SnapshotStore::<Vec<u8>, Version>::commit(
        &store,
        &id,
        SV1,
        Version::new(5).unwrap(),
        &vec![1, 2, 3],
    )
    .await
    .unwrap();
    SnapshotStore::<Vec<u8>, GlobalSeq>::commit(&store, &id, SV1, gs(9), &vec![9, 9, 9])
        .await
        .unwrap();

    // Each hydrates its own value untouched — proof the keyspaces are disjoint.
    let (v, snap) = SnapshotStore::<Vec<u8>, Version>::hydrate(&store, &id, SV1)
        .await
        .unwrap()
        .unwrap();
    let (g, proj) = SnapshotStore::<Vec<u8>, GlobalSeq>::hydrate(&store, &id, SV1)
        .await
        .unwrap()
        .unwrap();

    assert_eq!((v, snap), (Version::new(5).unwrap(), vec![1, 2, 3]));
    assert_eq!((g, proj), (gs(9), vec![9, 9, 9]));
}
