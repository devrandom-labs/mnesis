//! Retained audit regressions from todo.md. Fixed cases run normally; unfinished
//! cases remain ignored and can be run explicitly with `-- --ignored`.
#![allow(clippy::unwrap_used, reason = "audit regression assertions")]

#[cfg(all(feature = "snapshot", feature = "projection"))]
use mnesis_store::checkpoint::{CheckpointError, CheckpointMode, CheckpointStore, CheckpointWrite};
#[cfg(all(feature = "snapshot", feature = "projection"))]
#[path = "support/checkpoint.rs"]
mod checkpoint;
use futures::{FutureExt, TryStreamExt};
use mnesis::Version;
use mnesis_fjall::{AllIndex, FjallError, FjallStore, MAX_STREAM_ID_LEN};
use mnesis_store::envelope::pending_envelope;
use mnesis_store::{PendingBatch, RawEventStore, StreamKey};
use std::panic::AssertUnwindSafe;

#[tokio::test]
#[cfg(all(feature = "import", feature = "snapshot", feature = "projection"))]
#[allow(
    clippy::too_many_lines,
    reason = "one ordered storage-lifecycle scenario covers all four write paths across policies and index modes"
)]
async fn durability_policies_cover_event_atomic_snapshot_and_projection_writes() {
    use mnesis_fjall::{Durability, GlobalSeq};
    use mnesis_store::import::{AtomicAppend, PlannedAppend};
    use mnesis_store::state::SnapshotStore;
    use std::num::NonZeroU32;

    for policy in [
        Durability::SyncAll,
        Durability::SyncData,
        Durability::Buffered,
    ] {
        for mode in [AllIndex::Disabled, AllIndex::Denormalized] {
            let dir = tempfile::tempdir().unwrap();
            let id = StreamKey::from_slice(b"a");
            let other = StreamKey::from_slice(b"b");
            let make = |v| {
                pending_envelope(Version::new(v).unwrap())
                    .event_type("E")
                    .payload(vec![7])
                    .build()
                    .unwrap()
            };
            let schema = NonZeroU32::new(1).unwrap();
            let state = vec![8, 9];
            let store = FjallStore::builder(dir.path())
                .durability(policy)
                .all_index(mode)
                .streams_config(|opts| opts)
                .events_config(|opts| opts)
                .open()
                .unwrap();
            assert_eq!(store.durability(), policy);
            assert_eq!(
                store
                    .append(&id, None, PendingBatch::of(&make(1)))
                    .await
                    .unwrap()
                    .as_u64(),
                1
            );
            let writes = [
                PlannedAppend {
                    target: id.clone(),
                    expected_version: Some(Version::INITIAL),
                    head: make(2),
                    tail: vec![],
                },
                PlannedAppend {
                    target: other.clone(),
                    expected_version: None,
                    head: make(1),
                    tail: vec![],
                },
            ];
            let position = store.atomic_append_many(&writes).await.unwrap().unwrap();
            assert_eq!(position.as_u64(), 3);
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
            store.flush().await.unwrap();
            store.close().await.unwrap();
            let reopened = FjallStore::builder(dir.path())
                .all_index(mode)
                .open()
                .unwrap();
            assert_eq!(reopened.durability(), Durability::SyncAll);
            let rows: Vec<_> = reopened
                .read_stream(&id, Version::INITIAL)
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap();
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[1].version().as_u64(), 2);
            assert_eq!(rows[1].payload(), &[7]);
            let other_rows: Vec<_> = reopened
                .read_stream(&other, Version::INITIAL)
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap();
            assert_eq!(other_rows.len(), 1);
            let snapshot =
                <FjallStore as SnapshotStore<Vec<u8>, Version>>::hydrate(&reopened, &id, schema)
                    .await
                    .unwrap()
                    .into_found()
                    .unwrap();
            assert_eq!(snapshot, (Version::new(2).unwrap(), state.clone()));
            let checkpoint = checkpoint::found(
                <FjallStore as CheckpointStore<Vec<u8>, GlobalSeq>>::hydrate_checkpoint(
                    &reopened, &id, schema,
                )
                .await
                .unwrap(),
            );
            assert_eq!(checkpoint, (std::num::NonZeroU64::MIN, position, state));
            assert_eq!(
                reopened
                    .append(&other, Some(Version::INITIAL), PendingBatch::of(&make(2)))
                    .await
                    .unwrap()
                    .as_u64(),
                4
            );
        }
    }
}

#[tokio::test]
async fn empty_key_returns_error_without_panicking() {
    check_keys(&[0]).await;
}

#[tokio::test]
async fn oversized_key_returns_error_without_panicking() {
    check_keys(&[1, 65517, 65518, 65525, 65526, 65535, 65536]).await;
}

async fn check_keys(lengths: &[usize]) {
    use mnesis_store::wake::{WakeRegistration, WakeSource};

    for mode in [AllIndex::Denormalized, AllIndex::Disabled] {
        for &len in lengths {
            let dir = tempfile::tempdir().unwrap();
            let store = FjallStore::builder(dir.path())
                .all_index(mode)
                .open()
                .unwrap();
            let id = StreamKey::from_bytes(vec![b'a'; len]);
            let env = pending_envelope(Version::INITIAL)
                .event_type("E")
                .payload(b"x".as_slice())
                .build()
                .unwrap();
            let registration = store.register(None).unwrap();
            let wake = registration.arm();
            let result = AssertUnwindSafe(store.append(&id, None, PendingBatch::of(&env)))
                .catch_unwind()
                .await;
            assert!(result.is_ok(), "key length {len} panicked inside Fjall");
            let outcome = result.unwrap();
            if (1..=MAX_STREAM_ID_LEN).contains(&len) {
                assert!(outcome.is_ok(), "maximum valid default-layout key rejected");
            } else {
                assert!(matches!(
                    outcome,
                    Err(mnesis_store::error::AppendError::Store(
                        FjallError::InvalidKey { .. }
                    ))
                ));
                assert!(wake.now_or_never().is_none(), "rejection woke subscribers");
                assert!(matches!(
                    store.read_stream(&id, Version::INITIAL).await,
                    Err(FjallError::InvalidKey { .. })
                ));
                // The rejected append neither poisoned the writer nor consumed a position.
                let position = store
                    .append(
                        &StreamKey::from_slice(b"valid"),
                        None,
                        PendingBatch::of(&env),
                    )
                    .await
                    .unwrap();
                assert_eq!(position.as_u64(), 1);
            }
        }
    }
}

#[cfg(all(feature = "import", feature = "export"))]
#[tokio::test]
async fn invalid_atomic_key_leaves_all_streams_and_counters_unchanged() {
    use mnesis_store::export::StreamLister;
    use mnesis_store::import::{AtomicAppend, AtomicAppendError, PlannedAppend};
    use mnesis_store::wake::{WakeRegistration, WakeSource};

    for mode in [AllIndex::Denormalized, AllIndex::Disabled] {
        for len in [0, MAX_STREAM_ID_LEN + 1] {
            let dir = tempfile::tempdir().unwrap();
            let store = FjallStore::builder(dir.path())
                .all_index(mode)
                .open()
                .unwrap();
            let valid = StreamKey::from_slice(b"valid");
            let envelope = pending_envelope(Version::INITIAL)
                .event_type("E")
                .payload(b"x".as_slice())
                .build()
                .unwrap();
            let writes = [valid.clone(), StreamKey::from_bytes(vec![b'x'; len])].map(|target| {
                PlannedAppend {
                    target,
                    expected_version: None,
                    head: envelope.clone(),
                    tail: vec![],
                }
            });
            let registration = store.register(None).unwrap();
            let wake = registration.arm();
            assert!(matches!(
                store.atomic_append_many(&writes).await,
                Err(AtomicAppendError::Store(FjallError::InvalidKey { .. }))
            ));
            assert!(wake.now_or_never().is_none());
            assert_eq!(
                store
                    .list_streams()
                    .await
                    .unwrap()
                    .try_collect::<Vec<_>>()
                    .await
                    .unwrap(),
                []
            );
            assert_eq!(
                store
                    .read_stream(&valid, Version::INITIAL)
                    .await
                    .unwrap()
                    .try_collect::<Vec<_>>()
                    .await
                    .unwrap()
                    .len(),
                0
            );
            if mode == AllIndex::Denormalized {
                assert_eq!(
                    store
                        .read_all(None)
                        .await
                        .unwrap()
                        .try_collect::<Vec<_>>()
                        .await
                        .unwrap()
                        .len(),
                    0
                );
            }
            assert_eq!(
                store
                    .append(&valid, None, PendingBatch::of(&envelope))
                    .await
                    .unwrap()
                    .as_u64(),
                1
            );
        }
    }
}

#[cfg(all(feature = "snapshot", feature = "projection"))]
#[tokio::test]
async fn state_key_boundaries_return_errors_without_panics() {
    use mnesis_fjall::{GlobalSeq, MAX_KEY_LEN};
    use mnesis_store::state::SnapshotStore;
    use std::num::NonZeroU32;

    let dir = tempfile::tempdir().unwrap();
    let store = FjallStore::builder(dir.path()).open().unwrap();
    for len in [0, 1, MAX_KEY_LEN, MAX_KEY_LEN + 1] {
        let id = StreamKey::from_bytes(vec![b's'; len]);
        let snapshot = <FjallStore as SnapshotStore<Vec<u8>, Version>>::commit(
            &store,
            &id,
            NonZeroU32::MIN,
            Version::INITIAL,
            &vec![1],
        )
        .await;
        let projection = <FjallStore as CheckpointStore<Vec<u8>, GlobalSeq>>::commit_checkpoint(
            &store,
            &id,
            CheckpointWrite {
                expected: None,
                schema_version: NonZeroU32::MIN,
                position: GlobalSeq::INITIAL,
                state: &vec![2],
                mode: CheckpointMode::Advance,
            },
        )
        .await;
        let snap_read =
            <FjallStore as SnapshotStore<Vec<u8>, Version>>::hydrate(&store, &id, NonZeroU32::MIN)
                .await;
        let proj_read = <FjallStore as CheckpointStore<Vec<u8>, GlobalSeq>>::hydrate_checkpoint(
            &store,
            &id,
            NonZeroU32::MIN,
        )
        .await;
        if (1..=MAX_KEY_LEN).contains(&len) {
            snapshot.unwrap();
            projection.unwrap();
            assert_eq!(snap_read.unwrap().into_found().unwrap().1, vec![1]);
            let (revision, _, state) = checkpoint::found(proj_read.unwrap());
            assert_eq!(revision, std::num::NonZeroU64::MIN);
            assert_eq!(state, vec![2]);
        } else {
            assert!(matches!(snapshot, Err(FjallError::InvalidKey { .. })));
            assert!(matches!(
                projection,
                Err(CheckpointError::Store(FjallError::InvalidKey { .. }))
            ));
            assert!(matches!(snap_read, Err(FjallError::InvalidKey { .. })));
            assert!(matches!(proj_read, Err(FjallError::InvalidKey { .. })));
        }
    }
}

#[tokio::test]
async fn reopening_with_index_cannot_silently_omit_history() {
    let dir = tempfile::tempdir().unwrap();
    let id = StreamKey::from_slice(b"s");
    let env = pending_envelope(Version::INITIAL)
        .event_type("E")
        .payload(b"x".as_slice())
        .build()
        .unwrap();
    for stored in [AllIndex::Disabled, AllIndex::Denormalized] {
        let path = dir.path().join(format!("{stored:?}"));
        let original = FjallStore::builder(&path).all_index(stored).open().unwrap();
        original
            .append(&id, None, PendingBatch::of(&env))
            .await
            .unwrap();
        original.close().await.unwrap();
        let requested = match stored {
            AllIndex::Disabled => AllIndex::Denormalized,
            AllIndex::Denormalized => AllIndex::Disabled,
        };
        assert!(matches!(
            FjallStore::builder(&path).all_index(requested).open(),
            Err(FjallError::IndexModeMismatch { stored: actual, requested: wanted })
                if actual == stored && wanted == requested
        ));
        let store = FjallStore::builder(&path).all_index(stored).open().unwrap();
        let stream: Vec<_> = store
            .read_stream(&id, Version::INITIAL)
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(stream.len(), 1);
        if stored == AllIndex::Denormalized {
            let all: Vec<_> = store
                .read_all(None)
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap();
            assert_eq!(all.len(), stream.len());
        } else {
            assert!(matches!(
                store.read_all(None).await,
                Err(FjallError::AllIndexDisabled)
            ));
        }
    }
}

#[tokio::test]
async fn disk_payloads_preserve_promised_alignment() {
    let dir = tempfile::tempdir().unwrap();
    let id = StreamKey::from_slice(b"aligned");
    {
        let store = FjallStore::builder(dir.path()).open().unwrap();
        let rows: Vec<_> = (1..=100)
            .map(|v| {
                pending_envelope(Version::new(v).unwrap())
                    .event_type("Aligned")
                    .payload(vec![0u8; 16])
                    .build()
                    .unwrap()
            })
            .collect();
        store
            .append(&id, None, PendingBatch::new(&rows).unwrap())
            .await
            .unwrap();
    }
    {
        let db = fjall::SingleWriterTxDatabase::builder(dir.path())
            .open()
            .unwrap();
        db.keyspace("events", fjall::KeyspaceCreateOptions::default)
            .unwrap()
            .inner()
            .rotate_memtable_and_wait()
            .unwrap();
        db.keyspace("events_global", fjall::KeyspaceCreateOptions::default)
            .unwrap()
            .inner()
            .rotate_memtable_and_wait()
            .unwrap();
    }
    let store = FjallStore::builder(dir.path()).open().unwrap();
    let rows: Vec<_> = store
        .read_stream(&id, Version::INITIAL)
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    for row in rows {
        assert_eq!(
            row.payload().as_ptr().align_offset(16),
            0,
            "unaligned payload at {}",
            row.version()
        );
        assert_eq!(*bytemuck::try_from_bytes::<u128>(row.payload()).unwrap(), 0);
    }
    let all: Vec<_> = store
        .read_all(None)
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(all.len(), 100);
    for (_, _, row) in all {
        assert_eq!(*bytemuck::try_from_bytes::<u128>(row.payload()).unwrap(), 0);
    }
}

#[tokio::test]
async fn borrowing_decoders_work_in_memtables_ssts_and_compacted_reopened_stores() {
    for compressed in [false, true] {
        check_borrowing_storage(compressed).await;
    }
}

async fn check_borrowing_storage(compressed: bool) {
    let dir = tempfile::tempdir().unwrap();
    let ids = [
        StreamKey::from_slice(b"s"),
        StreamKey::from_slice(&[b's'; 97]),
    ];
    let store = FjallStore::builder(dir.path())
        .events_config(|opts: fjall::KeyspaceCreateOptions| {
            if compressed {
                opts
            } else {
                opts.data_block_compression_policy(fjall::config::CompressionPolicy::disabled())
            }
        })
        .open()
        .unwrap();
    for id in &ids {
        let rows: Vec<_> = (1u64..=24)
            .map(|v| {
                let payload = if v % 2 == 0 {
                    v.to_le_bytes().to_vec()
                } else {
                    rkyv::to_bytes::<rkyv::rancor::Error>(&vec![v; 3])
                        .unwrap()
                        .to_vec()
                };
                pending_envelope(Version::new(v).unwrap())
                    .event_type(if v % 2 == 0 { "Pod" } else { "ArchivedVector" })
                    .payload(payload)
                    .metadata(vec![b'm'; usize::try_from(v).unwrap()])
                    .build()
                    .unwrap()
            })
            .collect();
        store
            .append(id, None, PendingBatch::new(&rows).unwrap())
            .await
            .unwrap();
    }
    check_borrowing_reads(&store, &ids).await;
    store.close().await.unwrap();
    for compact in [false, true] {
        {
            let db = fjall::SingleWriterTxDatabase::builder(dir.path())
                .open()
                .unwrap();
            for name in ["events", "events_global"] {
                let keyspace = db
                    .keyspace(name, fjall::KeyspaceCreateOptions::default)
                    .unwrap();
                keyspace.inner().rotate_memtable_and_wait().unwrap();
                if compact {
                    keyspace.inner().major_compact().unwrap();
                }
            }
        }
        let reopened = FjallStore::builder(dir.path()).open().unwrap();
        check_borrowing_reads(&reopened, &ids).await;
    }
}

async fn check_borrowing_reads(store: &FjallStore, ids: &[StreamKey]) {
    let mut rows = Vec::new();
    for id in ids {
        let stream: Vec<_> = store
            .read_stream(id, Version::INITIAL)
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(stream.len(), 24);
        rows.extend(stream);
    }
    let all: Vec<_> = store
        .read_all(None)
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(all.len(), 48);
    rows.extend(all.into_iter().map(|(_, _, row)| row));
    // Drop the cursor before decoding: each envelope must retain its owner.
    for row in rows {
        assert!(row.payload().as_ptr().addr().is_multiple_of(16));
        let v = row.version().as_u64();
        if v % 2 == 0 {
            assert_eq!(*bytemuck::try_from_bytes::<u64>(row.payload()).unwrap(), v);
        } else {
            let archived =
                rkyv::access::<<Vec<u64> as rkyv::Archive>::Archived, rkyv::rancor::Error>(
                    row.payload(),
                )
                .unwrap();
            assert_eq!(archived.len(), 3);
            assert!(archived.iter().all(|entry| *entry == v));
        }
    }
}

#[tokio::test]
#[cfg(all(feature = "import", feature = "export"))]
async fn multistream_export_requires_one_snapshot() {
    use mnesis_store::export::{ConsistentExporter, ExportSession};
    use mnesis_store::import::{AtomicAppend, PlannedAppend};

    for mode in [AllIndex::Denormalized, AllIndex::Disabled] {
        let dir = tempfile::tempdir().unwrap();
        let store = FjallStore::builder(dir.path())
            .all_index(mode)
            .open()
            .unwrap();
        let ids = [StreamKey::from_slice(b"a"), StreamKey::from_slice(b"b")];
        let runs = |version: u64| {
            ids.iter()
                .map(|id| PlannedAppend {
                    target: id.clone(),
                    expected_version: Version::new(version - 1),
                    head: pending_envelope(Version::new(version).unwrap())
                        .event_type("E")
                        .payload(b"x".as_slice())
                        .build()
                        .unwrap(),
                    tail: vec![],
                })
                .collect::<Vec<_>>()
        };
        store.atomic_append_many(&runs(1)).await.unwrap();
        let session = store
            .open_export_session(std::time::Duration::from_secs(60))
            .await
            .unwrap();
        let first: Vec<_> = session
            .export_stream(&ids[0], Version::INITIAL)
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        store.atomic_append_many(&runs(2)).await.unwrap();
        let second: Vec<_> = session
            .export_stream(&ids[1], Version::INITIAL)
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(first.len(), second.len(), "backup split an atomic commit");
        assert_eq!(first.len(), 1);
        for events in [&first, &second] {
            assert_eq!(events[0].version(), Version::INITIAL);
            assert_eq!(events[0].payload(), b"x");
        }
        session.close().await.unwrap();
        store.close().await.unwrap();
    }
}
