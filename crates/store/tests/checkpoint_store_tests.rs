#![cfg(feature = "json")]
#![allow(clippy::unwrap_used, reason = "checkpoint codec protocol assertions")]

use core::num::NonZeroU32;
use mnesis_inmemory::InMemoryCheckpointStore;
use mnesis_store::checkpoint::{
    CheckpointError, CheckpointHydrated, CheckpointMode, CheckpointRejection, CheckpointStore,
    CheckpointWrite,
};
use mnesis_store::state::{CodecSnapshotStore, CodecSnapshotStoreError};
use mnesis_store::{JsonCodec, StreamKey};

#[tokio::test]
async fn codec_roundtrip_preserves_revision_position_and_conditional_rejection() {
    let bytes = InMemoryCheckpointStore::<Vec<u8>, u64>::new();
    let codec = CodecSnapshotStore::new(&bytes, JsonCodec::default());
    let id = StreamKey::from_slice(b"codec");
    let schema = NonZeroU32::MIN;
    let revision = codec
        .commit_checkpoint(
            &id,
            CheckpointWrite {
                expected: None,
                schema_version: schema,
                position: 5,
                state: &123_u64,
                mode: CheckpointMode::Advance,
            },
        )
        .await
        .unwrap();
    let restored: CheckpointHydrated<u64, u64> =
        codec.hydrate_checkpoint(&id, schema).await.unwrap();
    assert_eq!(
        restored,
        CheckpointHydrated::Found {
            revision,
            position: 5,
            state: 123
        }
    );
    assert!(matches!(
        codec
            .commit_checkpoint(
                &id,
                CheckpointWrite {
                    expected: None,
                    schema_version: schema,
                    position: 6,
                    state: &999_u64,
                    mode: CheckpointMode::Advance,
                }
            )
            .await,
        Err(CheckpointError::Rejected(
            CheckpointRejection::Conflict { .. }
        ))
    ));
    let after: CheckpointHydrated<u64, u64> = codec.hydrate_checkpoint(&id, schema).await.unwrap();
    assert_eq!(after, restored);
}

#[tokio::test]
async fn stale_schema_preserves_revision_without_decoding_incompatible_bytes() {
    let bytes = InMemoryCheckpointStore::<Vec<u8>, u64>::new();
    let id = StreamKey::from_slice(b"stale-codec");
    let old = NonZeroU32::MIN;
    let new = NonZeroU32::new(2).unwrap();
    let malformed = b"not JSON".to_vec();
    let revision = bytes
        .commit_checkpoint(
            &id,
            CheckpointWrite {
                expected: None,
                schema_version: old,
                position: 50,
                state: &malformed,
                mode: CheckpointMode::Advance,
            },
        )
        .await
        .unwrap();
    let codec = CodecSnapshotStore::new(&bytes, JsonCodec::default());
    let stale: CheckpointHydrated<u64, u64> = codec.hydrate_checkpoint(&id, new).await.unwrap();
    assert_eq!(
        stale,
        CheckpointHydrated::Stale {
            revision,
            stored_schema: old
        }
    );
    let incompatible: Result<CheckpointHydrated<u64, u64>, _> =
        codec.hydrate_checkpoint(&id, old).await;
    assert!(matches!(
        incompatible,
        Err(CodecSnapshotStoreError::Decode(_))
    ));
    let next = codec
        .commit_checkpoint(
            &id,
            CheckpointWrite {
                expected: Some(revision),
                schema_version: new,
                position: 1,
                state: &10_u64,
                mode: CheckpointMode::Rebuild,
            },
        )
        .await
        .unwrap();
    assert_eq!(next.get(), 2);
    let restored: CheckpointHydrated<u64, u64> = codec.hydrate_checkpoint(&id, new).await.unwrap();
    assert_eq!(
        restored,
        CheckpointHydrated::Found {
            revision: next,
            position: 1,
            state: 10
        }
    );
}
