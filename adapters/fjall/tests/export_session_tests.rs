//! Actual session paging and a persistent backup across intervening commits.
#![cfg(all(feature = "export", feature = "import"))]
#![allow(
    clippy::unwrap_used,
    reason = "persistent session regression assertions"
)]

use futures::{StreamExt, TryStreamExt};
use mnesis::Version;
use mnesis_fjall::{AllIndex, BlockingConfig, FjallError, FjallStore};
use mnesis_store::cbor::{ChunkWriter, decode_chunk};
use mnesis_store::export::{ConsistentExporter, ExportSession, StreamLister};
use mnesis_store::import::{AtomicAppend, Atomicity, EventImporter, PlannedAppend};
use mnesis_store::value::SchemaVersion;
use mnesis_store::{PersistedEnvelope, RawEventStore, StreamKey, pending_envelope};
use std::num::NonZeroUsize;
use std::time::Duration;

async fn append_runs(store: &FjallStore, ids: &[StreamKey], version: u64) {
    let writes = ids
        .iter()
        .map(|id| PlannedAppend {
            target: id.clone(),
            expected_version: Version::new(version - 1),
            head: pending_envelope(Version::new(version).unwrap())
                .event_type("Saved")
                .payload(format!("v{version}").into_bytes())
                .schema_version(SchemaVersion::from_u32(2).unwrap())
                .metadata(b"original metadata".as_slice())
                .build()
                .unwrap(),
            tail: vec![],
        })
        .collect::<Vec<_>>();
    store.atomic_append_many(&writes).await.unwrap();
}

fn assert_history(rows: &[PersistedEnvelope], first: u64, last: u64) {
    assert_eq!(rows.len(), usize::try_from(last - first + 1).unwrap());
    for (event, version) in rows.iter().zip(first..=last) {
        assert_eq!(event.version(), Version::new(version).unwrap());
        assert_eq!(event.schema_version(), 2);
        assert_eq!(event.event_type(), "Saved");
        assert_eq!(event.payload(), format!("v{version}").as_bytes());
        assert_eq!(event.metadata(), Some(b"original metadata".as_slice()));
    }
}

#[tokio::test]
async fn paging_keeps_old_heads_and_ids_after_atomic_append_and_stream_creation() {
    for mode in [AllIndex::Denormalized, AllIndex::Disabled] {
        let directory = tempfile::tempdir().unwrap();
        let store = FjallStore::builder(directory.path())
            .all_index(mode)
            .blocking(BlockingConfig {
                scan_batch_rows: NonZeroUsize::new(2).unwrap(),
                ..BlockingConfig::default()
            })
            .open()
            .unwrap();
        let ids = [
            StreamKey::from_slice(b"a"),
            StreamKey::from_slice(&[0xff, 0]),
            StreamKey::from_slice(b"c"),
            StreamKey::from_slice(b"d"),
            StreamKey::from_slice(b"e"),
        ];
        for version in 1..=5 {
            append_runs(&store, &ids[..2], version).await;
        }
        append_runs(&store, &ids[2..], 1).await;
        let session = store
            .open_export_session(Duration::from_secs(60))
            .await
            .unwrap();
        let mut listing = session.list_streams().await.unwrap();
        let mut captured = vec![listing.next().await.unwrap().unwrap()];
        let mut events = session
            .export_stream(&ids[0], Version::new(2).unwrap())
            .await
            .unwrap();
        let mut first_rows = vec![events.next().await.unwrap().unwrap()];
        append_runs(&store, &ids[..2], 6).await;
        let later = StreamKey::from_slice(b"aa");
        append_runs(&store, std::slice::from_ref(&later), 1).await;
        captured.extend(listing.by_ref().try_collect::<Vec<_>>().await.unwrap());
        captured.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
        let mut expected = ids.to_vec();
        expected.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
        assert_eq!(captured, expected);
        assert!(listing.next().await.is_none());
        first_rows.extend(events.by_ref().try_collect::<Vec<_>>().await.unwrap());
        assert_history(&first_rows, 2, 5);
        assert!(events.next().await.is_none());
        let second = session
            .export_stream(&ids[1], Version::INITIAL)
            .await
            .unwrap()
            .try_collect::<Vec<_>>()
            .await
            .unwrap();
        assert_history(&second, 1, 5);
        for (id, from) in [
            (&later, Version::INITIAL),
            (&ids[0], Version::new(u64::MAX).unwrap()),
        ] {
            let mut empty = session.export_stream(id, from).await.unwrap();
            assert!(empty.next().await.is_none());
            assert!(empty.next().await.is_none());
        }
        drop(events);
        drop(listing);
        session.close().await.unwrap();
        let live = store
            .read_stream(&ids[1], Version::INITIAL)
            .await
            .unwrap()
            .try_collect::<Vec<_>>()
            .await
            .unwrap();
        assert_history(&live, 1, 6);
        store.close().await.unwrap();
    }
}

async fn backup_across_commit(store: &FjallStore, ids: &[StreamKey]) -> Vec<u8> {
    let session = store
        .open_export_session(Duration::from_secs(60))
        .await
        .unwrap();
    let mut listed = session
        .list_streams()
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    listed.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    assert_eq!(listed, ids);
    let mut writer = ChunkWriter::new(Vec::new(), None).unwrap();
    for (index, id) in listed.iter().enumerate() {
        let events = session.export_stream(id, Version::INITIAL).await.unwrap();
        writer
            .section(id.as_bytes())
            .unwrap()
            .try_extend(events)
            .await
            .unwrap();
        if index == 0 {
            append_runs(store, ids, 6).await;
            append_runs(store, &[StreamKey::from_slice(b"later")], 1).await;
        }
    }
    session.close().await.unwrap();
    writer.finish().unwrap()
}

async fn restore_and_check(mode: AllIndex, policy: Atomicity) {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("source");
    let destination_path = directory.path().join("destination");
    let ids = [
        StreamKey::from_slice(b"a"),
        StreamKey::from_slice(&[0xff, 0]),
    ];
    let source = FjallStore::builder(&source_path)
        .all_index(mode)
        .open()
        .unwrap();
    for version in 1..=5 {
        append_runs(&source, &ids, version).await;
    }
    let bytes = backup_across_commit(&source, &ids).await;
    let file = directory.path().join("backup.nxch");
    std::fs::write(&file, bytes).unwrap();
    source.close().await.unwrap();
    let decoded = decode_chunk(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(decoded.len(), 2);
    let destination = FjallStore::builder(&destination_path)
        .all_index(mode)
        .open()
        .unwrap();
    let report = destination
        .import(&decoded, StreamKey::from_slice, policy)
        .await
        .unwrap();
    assert!(report.all_complete());
    destination.close().await.unwrap();
    let reopened = FjallStore::builder(&destination_path)
        .all_index(mode)
        .open()
        .unwrap();
    for id in &ids {
        let events = reopened
            .read_stream(id, Version::INITIAL)
            .await
            .unwrap()
            .try_collect::<Vec<_>>()
            .await
            .unwrap();
        assert_history(&events, 1, 5);
    }
    let mut absent = reopened
        .read_stream(&StreamKey::from_slice(b"later"), Version::INITIAL)
        .await
        .unwrap();
    assert!(absent.next().await.is_none());
    drop(absent);
    assert_global_import_tags(&reopened, &ids, mode).await;
    append_runs(&reopened, std::slice::from_ref(&ids[0]), 6).await;
    let after = reopened
        .read_stream(&ids[0], Version::INITIAL)
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    assert_history(&after, 1, 6);
    reopened.close().await.unwrap();
}

async fn assert_global_import_tags(store: &FjallStore, ids: &[StreamKey], mode: AllIndex) {
    match mode {
        AllIndex::Denormalized => {
            let all = store
                .read_all(None)
                .await
                .unwrap()
                .try_collect::<Vec<_>>()
                .await
                .unwrap();
            let actual = all
                .iter()
                .map(|(pos, id, event)| (pos.as_u64(), id.as_bytes(), event.version().as_u64()))
                .collect::<Vec<_>>();
            let expected = (1..=5)
                .map(|version| (version, ids[0].as_bytes(), version))
                .chain((1..=5).map(|version| (version + 5, ids[1].as_bytes(), version)))
                .collect::<Vec<_>>();
            assert_eq!(actual, expected);
        }
        AllIndex::Disabled => assert!(matches!(
            store.read_all(None).await,
            Err(FjallError::AllIndexDisabled)
        )),
    }
}

#[tokio::test]
async fn session_backup_restores_one_complete_view_after_source_close_and_destination_reopen() {
    for mode in [AllIndex::Denormalized, AllIndex::Disabled] {
        for policy in [Atomicity::WholeChunk, Atomicity::PerStream] {
            restore_and_check(mode, policy).await;
        }
    }
}

#[tokio::test]
async fn every_backup_cut_rejects_before_writes_and_reopens_empty() {
    for mode in [AllIndex::Denormalized, AllIndex::Disabled] {
        for policy in [Atomicity::PerStream, Atomicity::WholeChunk] {
            let directory = tempfile::tempdir().unwrap();
            let source = FjallStore::builder(directory.path().join("source"))
                .all_index(mode)
                .open()
                .unwrap();
            let ids = [StreamKey::from_slice(b"a"), StreamKey::from_slice(b"b")];
            for version in 1..=5 {
                append_runs(&source, &ids, version).await;
            }
            let bytes = backup_across_commit(&source, &ids).await;
            source.close().await.unwrap();
            let destination_path = directory.path().join("destination");
            let destination = FjallStore::builder(&destination_path)
                .all_index(mode)
                .open()
                .unwrap();
            for cut in 0..bytes.len() {
                // Exercise the restore boundary: if decoding ever accepts a cut,
                // run the actual importer so the exact-storage assertion catches it.
                let decoded = decode_chunk(&bytes[..cut]);
                let rejected = decoded.is_err();
                if let Ok(sections) = decoded {
                    destination
                        .import(&sections, StreamKey::from_slice, policy)
                        .await
                        .unwrap();
                }
                assert!(
                    rejected,
                    "normal restore accepted cut {cut}/{}",
                    bytes.len()
                );
            }
            assert_eq!(
                destination
                    .list_streams()
                    .await
                    .unwrap()
                    .try_collect::<Vec<_>>()
                    .await
                    .unwrap(),
                []
            );
            destination.close().await.unwrap();
            let reopened = FjallStore::builder(&destination_path)
                .all_index(mode)
                .open()
                .unwrap();
            for id in &ids {
                assert!(
                    reopened
                        .read_stream(id, Version::INITIAL)
                        .await
                        .unwrap()
                        .try_collect::<Vec<_>>()
                        .await
                        .unwrap()
                        .is_empty()
                );
            }
            if mode == AllIndex::Denormalized {
                assert!(
                    reopened
                        .read_all(None)
                        .await
                        .unwrap()
                        .try_collect::<Vec<_>>()
                        .await
                        .unwrap()
                        .is_empty()
                );
            }
            assert_first_commit_after_rejected_backup(&reopened, &ids, mode).await;
            reopened.close().await.unwrap();
        }
    }
}

async fn assert_first_commit_after_rejected_backup(
    store: &FjallStore,
    ids: &[StreamKey],
    mode: AllIndex,
) {
    append_runs(store, ids, 1).await;
    for id in ids {
        let rows = store
            .read_stream(id, Version::INITIAL)
            .await
            .unwrap()
            .try_collect::<Vec<_>>()
            .await
            .unwrap();
        assert_history(&rows, 1, 1);
    }
    if mode == AllIndex::Denormalized {
        let rows = store
            .read_all(None)
            .await
            .unwrap()
            .try_collect::<Vec<_>>()
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
        for (index, row) in rows.iter().enumerate() {
            assert_eq!(row.0.as_u64(), u64::try_from(index).unwrap() + 1);
        }
    }
}
