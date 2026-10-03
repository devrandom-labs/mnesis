//! Revision-checked checkpoint records and single-transaction replacement.

use core::num::{NonZeroU32, NonZeroU64};
use fjall::{Readable, SingleWriterTxKeyspace};
use mnesis::{ErrorId, Id, Version};
use mnesis_store::checkpoint::{
    CheckpointError, CheckpointHydrated, CheckpointStore, CheckpointWrite,
};

use crate::partition::Partitions;
use crate::store::Storage;
use crate::{FjallError, GlobalSeq};

/// `[schema:u32 LE][revision:u64 BE][position:u64 BE][payload]`.
pub const HEADER_SIZE: usize = 20;

trait Position: Copy + Ord + Send {
    fn keyspace(partitions: &Partitions) -> &SingleWriterTxKeyspace;
    fn from_u64(value: u64) -> Option<Self>;
    fn as_u64(self) -> u64;
}

impl Position for Version {
    fn keyspace(partitions: &Partitions) -> &SingleWriterTxKeyspace {
        partitions.checkpoint_stream()
    }
    fn from_u64(value: u64) -> Option<Self> {
        Self::new(value)
    }
    fn as_u64(self) -> u64 {
        self.as_u64()
    }
}
impl Position for GlobalSeq {
    fn keyspace(partitions: &Partitions) -> &SingleWriterTxKeyspace {
        partitions.checkpoint_global()
    }
    fn from_u64(value: u64) -> Option<Self> {
        Self::new(value)
    }
    fn as_u64(self) -> u64 {
        self.as_u64()
    }
}

fn corrupt(id: &impl Id) -> FjallError {
    FjallError::CorruptValue {
        stream_id: ErrorId::from_display(id),
        version: None,
    }
}

fn decode<'a, P: Position>(
    bytes: &'a [u8],
    id: &impl Id,
) -> Result<(NonZeroU64, NonZeroU32, P, &'a [u8]), FjallError> {
    if bytes.len() < HEADER_SIZE {
        return Err(corrupt(id));
    }
    let schema = NonZeroU32::new(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        .ok_or_else(|| corrupt(id))?;
    let revision = NonZeroU64::new(u64::from_be_bytes([
        bytes[4], bytes[5], bytes[6], bytes[7], bytes[8], bytes[9], bytes[10], bytes[11],
    ]))
    .ok_or_else(|| corrupt(id))?;
    let position = P::from_u64(u64::from_be_bytes([
        bytes[12], bytes[13], bytes[14], bytes[15], bytes[16], bytes[17], bytes[18], bytes[19],
    ]))
    .ok_or_else(|| corrupt(id))?;
    Ok((revision, schema, position, &bytes[HEADER_SIZE..]))
}

impl<P: Position> CheckpointStore<Vec<u8>, P> for Storage {
    type Error = FjallError;

    async fn hydrate_checkpoint(
        &self,
        id: &impl Id,
        schema_version: NonZeroU32,
    ) -> Result<CheckpointHydrated<Vec<u8>, P>, Self::Error> {
        crate::limits::validate_key(id.as_ref(), crate::MAX_KEY_LEN)?;
        let Some(bytes) = P::keyspace(&self.partitions).get(id.as_ref())? else {
            return Ok(CheckpointHydrated::Absent);
        };
        let (revision, schema, position, payload) = decode::<P>(&bytes, id)?;
        if schema != schema_version {
            return Ok(CheckpointHydrated::Stale {
                revision,
                stored_schema: schema,
            });
        }
        Ok(CheckpointHydrated::Found {
            revision,
            position,
            state: payload.to_vec(),
        })
    }

    async fn commit_checkpoint(
        &self,
        id: &impl Id,
        write: CheckpointWrite<'_, Vec<u8>, P>,
    ) -> Result<NonZeroU64, CheckpointError<Self::Error>> {
        crate::limits::validate_key(id.as_ref(), crate::MAX_KEY_LEN)
            .map_err(CheckpointError::Store)?;
        let len = crate::limits::checkpoint_value_len(write.state.len())
            .map_err(CheckpointError::Store)?;
        // Prepare the complete value before acquiring the serialized writer.
        // Only the validated revision bytes are filled while holding the lock.
        let mut bytes = Vec::with_capacity(len);
        bytes.extend_from_slice(&write.schema_version.get().to_le_bytes());
        bytes.extend_from_slice(&0_u64.to_be_bytes());
        bytes.extend_from_slice(&write.position.as_u64().to_be_bytes());
        bytes.extend_from_slice(write.state);
        let keyspace = P::keyspace(&self.partitions);
        let revision = {
            let mut tx = self.write_tx();
            let current = tx
                .get(keyspace, id.as_ref())
                .map_err(FjallError::Io)
                .map_err(CheckpointError::Store)?;
            let metadata = current
                .as_ref()
                .map(|value| {
                    decode::<P>(value, id)
                        .map(|(revision, schema, position, _)| (revision, schema, position))
                })
                .transpose()
                .map_err(CheckpointError::Store)?;
            let next = write.next_revision(metadata)?;
            bytes[4..12].copy_from_slice(&next.get().to_be_bytes());
            tx.insert(keyspace, id.as_ref(), bytes);
            tx.commit()
                .map_err(FjallError::Io)
                .map_err(CheckpointError::Store)?;
            next
        };
        Ok(revision)
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "checkpoint corruption and overflow assertions"
)]
mod tests {
    use super::{FjallError, GlobalSeq, NonZeroU32, NonZeroU64, Position, Version};
    use crate::FjallStore;
    use mnesis_store::StreamKey;
    use mnesis_store::checkpoint::{
        CheckpointError, CheckpointHydrated, CheckpointMode, CheckpointRejection, CheckpointStore,
        CheckpointWrite,
    };

    fn record(schema: u32, revision: u64, position: u64) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(21);
        bytes.extend_from_slice(&schema.to_le_bytes());
        bytes.extend_from_slice(&revision.to_be_bytes());
        bytes.extend_from_slice(&position.to_be_bytes());
        bytes.push(42);
        bytes
    }

    #[tokio::test]
    async fn corrupt_checkpoint_metadata_remains_corrupt_across_failed_writes_and_reopen() {
        let values = [
            vec![],
            vec![1; 19],
            record(0, 1, 1),
            record(1, 0, 1),
            record(1, 1, 0),
        ];
        for value in values {
            check_corruption::<Version>(&value).await;
            check_corruption::<GlobalSeq>(&value).await;
        }
    }

    async fn check_corruption<P: Position>(value: &[u8])
    where
        FjallStore: CheckpointStore<Vec<u8>, P, Error = FjallError>,
    {
        let directory = tempfile::tempdir().unwrap();
        let store = FjallStore::builder(directory.path()).open().unwrap();
        let id = StreamKey::from_slice(b"corrupt-checkpoint");
        // White-box injection bypasses the public API to exercise persisted
        // corruption. Public storage operations still run on the I/O worker.
        P::keyspace(&store.storage.partitions)
            .insert(id.as_ref(), value)
            .unwrap();
        for schema in [NonZeroU32::MIN, NonZeroU32::new(2).unwrap()] {
            assert!(matches!(
                <FjallStore as CheckpointStore<Vec<u8>, P>>::hydrate_checkpoint(
                    &store, &id, schema
                )
                .await,
                Err(FjallError::CorruptValue { .. })
            ));
        }
        let state = vec![99];
        let proposal = CheckpointWrite {
            expected: None,
            schema_version: NonZeroU32::MIN,
            position: P::from_u64(2).unwrap(),
            state: &state,
            mode: CheckpointMode::Advance,
        };
        assert!(matches!(
            store.commit_checkpoint(&id, proposal).await,
            Err(CheckpointError::Store(FjallError::CorruptValue { .. }))
        ));
        let unchanged = P::keyspace(&store.storage.partitions)
            .get(id.as_ref())
            .unwrap()
            .unwrap();
        assert_eq!(unchanged.as_ref(), value);
        store.flush().await.unwrap();
        store.close().await.unwrap();
        let reopened = FjallStore::builder(directory.path()).open().unwrap();
        assert!(matches!(
            <FjallStore as CheckpointStore<Vec<u8>, P>>::hydrate_checkpoint(
                &reopened,
                &id,
                NonZeroU32::MIN
            )
            .await,
            Err(FjallError::CorruptValue { .. })
        ));
        let recovered = P::keyspace(&reopened.storage.partitions)
            .get(id.as_ref())
            .unwrap()
            .unwrap();
        assert_eq!(recovered.as_ref(), value);
        reopened.close().await.unwrap();
    }

    #[tokio::test]
    async fn checkpoint_revision_exhaustion_cannot_wrap_even_during_schema_rebuild() {
        check_exhaustion::<Version>().await;
        check_exhaustion::<GlobalSeq>().await;
    }

    async fn check_exhaustion<P: Position>()
    where
        FjallStore: CheckpointStore<Vec<u8>, P, Error = FjallError>,
    {
        let directory = tempfile::tempdir().unwrap();
        let store = FjallStore::builder(directory.path()).open().unwrap();
        let id = StreamKey::from_slice(b"revision-ceiling");
        let original = record(1, u64::MAX, 7);
        P::keyspace(&store.storage.partitions)
            .insert(id.as_ref(), original.as_slice())
            .unwrap();
        let state = vec![99];
        for (schema, mode, position) in [
            (1, CheckpointMode::Advance, 8),
            (2, CheckpointMode::Rebuild, 1),
        ] {
            let proposal = CheckpointWrite {
                expected: Some(NonZeroU64::MAX),
                schema_version: NonZeroU32::new(schema).unwrap(),
                position: P::from_u64(position).unwrap(),
                state: &state,
                mode,
            };
            assert!(matches!(
                store.commit_checkpoint(&id, proposal).await,
                Err(CheckpointError::Rejected(
                    CheckpointRejection::RevisionOverflow
                ))
            ));
        }
        store.flush().await.unwrap();
        store.close().await.unwrap();
        let reopened = FjallStore::builder(directory.path()).open().unwrap();
        let found = reopened
            .hydrate_checkpoint(&id, NonZeroU32::MIN)
            .await
            .unwrap();
        assert!(
            matches!(found, CheckpointHydrated::Found { revision: NonZeroU64::MAX, position, state: stored_state } if position == P::from_u64(7).unwrap() && stored_state == [42])
        );
        let recovered = P::keyspace(&reopened.storage.partitions)
            .get(id.as_ref())
            .unwrap()
            .unwrap();
        assert_eq!(recovered.as_ref(), original);
        reopened.close().await.unwrap();
    }
}
