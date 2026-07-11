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
        .into_found()
        .unwrap();
    let (g, proj) = SnapshotStore::<Vec<u8>, GlobalSeq>::hydrate(&store, &id, SV1)
        .await
        .unwrap()
        .into_found()
        .unwrap();

    assert_eq!((v, snap), (Version::new(5).unwrap(), vec![1, 2, 3]));
    assert_eq!((g, proj), (gs(9), vec![9, 9, 9]));
}
