//! `SnapshotStore` capability conformance: state and position commit and
//! hydrate together; a schema change reads back as `Stale`, never as decode
//! garbage.

use core::fmt::Debug;
use core::future::Future;
use std::num::NonZeroU32;

use nexus_store::state::{Hydrated, SnapshotStore};

use crate::row::SubId;

const SCHEMA_1: NonZeroU32 = NonZeroU32::new(1).expect("1 is non-zero");
const SCHEMA_2: NonZeroU32 = NonZeroU32::new(2).expect("2 is non-zero");

/// Fresh store hydrates `Absent`; after commit the same `(position, state)`
/// pair comes back `Found`.
pub async fn check_snapshot_absent_then_commit_then_found<SS, P, C, F, Fut>(
    factory: &F,
    p1: P,
    _p2: P,
) where
    SS: SnapshotStore<Vec<u8>, P>,
    P: Copy + Ord + Debug + Send + Sync + 'static,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (SS, C)> + Send,
{
    let (store, _guard) = factory().await;
    let id = SubId::new("snap");
    match store
        .hydrate(&id, SCHEMA_1)
        .await
        .unwrap_or_else(|e| panic!("hydrate failed: {e:?}"))
    {
        Hydrated::Absent => {}
        other => panic!("fresh store must hydrate Absent, got {other:?}"),
    }

    let state = vec![1u8, 2, 3];
    store
        .commit(&id, SCHEMA_1, p1, &state)
        .await
        .unwrap_or_else(|e| panic!("commit failed: {e:?}"));
    match store
        .hydrate(&id, SCHEMA_1)
        .await
        .unwrap_or_else(|e| panic!("hydrate failed: {e:?}"))
    {
        Hydrated::Found {
            position,
            state: got,
        } => {
            assert_eq!(position, p1, "position must hydrate exactly as committed");
            assert_eq!(got, state, "state must hydrate byte-for-byte");
        }
        other => panic!("expected Found after commit, got {other:?}"),
    }
}

/// Hydrating under a different schema version yields `Stale` carrying the
/// stored schema — never `Found` with undecodable bytes, never `Absent`.
pub async fn check_snapshot_stale_on_schema_change<SS, P, C, F, Fut>(factory: &F, p1: P, _p2: P)
where
    SS: SnapshotStore<Vec<u8>, P>,
    P: Copy + Ord + Debug + Send + Sync + 'static,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (SS, C)> + Send,
{
    let (store, _guard) = factory().await;
    let id = SubId::new("snap-stale");
    store
        .commit(&id, SCHEMA_1, p1, &vec![1u8])
        .await
        .unwrap_or_else(|e| panic!("commit failed: {e:?}"));
    match store
        .hydrate(&id, SCHEMA_2)
        .await
        .unwrap_or_else(|e| panic!("hydrate failed: {e:?}"))
    {
        Hydrated::Stale { stored_schema } => {
            assert_eq!(
                stored_schema, SCHEMA_1,
                "Stale must carry the stored schema version"
            );
        }
        other => panic!("schema mismatch must hydrate Stale, got {other:?}"),
    }
}

/// A second commit fully replaces the first — latest `(position, state)` wins.
pub async fn check_snapshot_overwrite_latest_wins<SS, P, C, F, Fut>(factory: &F, p1: P, p2: P)
where
    SS: SnapshotStore<Vec<u8>, P>,
    P: Copy + Ord + Debug + Send + Sync + 'static,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (SS, C)> + Send,
{
    assert!(
        p1 < p2,
        "kit misuse: positions must be supplied in ascending order"
    );
    let (store, _guard) = factory().await;
    let id = SubId::new("snap-latest");
    store
        .commit(&id, SCHEMA_1, p1, &vec![1u8])
        .await
        .unwrap_or_else(|e| panic!("first commit failed: {e:?}"));
    store
        .commit(&id, SCHEMA_1, p2, &vec![2u8])
        .await
        .unwrap_or_else(|e| panic!("second commit failed: {e:?}"));
    match store
        .hydrate(&id, SCHEMA_1)
        .await
        .unwrap_or_else(|e| panic!("hydrate failed: {e:?}"))
    {
        Hydrated::Found { position, state } => {
            assert_eq!(position, p2, "latest committed position must win");
            assert_eq!(state, vec![2u8], "latest committed state must win");
        }
        other => panic!("expected Found, got {other:?}"),
    }
}
