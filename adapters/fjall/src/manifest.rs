use fjall::{KeyspaceCreateOptions, PersistMode, Readable, SingleWriterTxDatabase};

use crate::{AllIndex, FjallError};

const KEYSPACE: &str = "mnesis_meta";
const KEY: &[u8] = b"layout";
const LAYOUT_VERSION: u8 = 2;

/// Initialize only an empty database; otherwise require its persisted layout.
#[allow(
    clippy::significant_drop_tightening,
    reason = "writer lock spans manifest validation and conditional initialization; commit consumes the transaction"
)]
pub fn validate(db: &SingleWriterTxDatabase, requested: AllIndex) -> Result<(), FjallError> {
    let metadata = db.keyspace(KEYSPACE, KeyspaceCreateOptions::default)?;
    let mut tx = db.write_tx().durability(Some(PersistMode::SyncAll));
    if let Some(bytes) = tx.get(&metadata, KEY)? {
        let [version, mode] = bytes.as_ref() else {
            return Err(FjallError::InvalidManifest);
        };
        if *version != LAYOUT_VERSION {
            return Err(FjallError::UnsupportedLayout { version: *version });
        }
        let stored = match mode {
            0 => AllIndex::Disabled,
            1 => AllIndex::Denormalized,
            _ => return Err(FjallError::InvalidManifest),
        };
        if stored != requested {
            return Err(FjallError::IndexModeMismatch { stored, requested });
        }
        drop(tx);
        return Ok(());
    }

    for name in db.list_keyspace_names() {
        let keyspace = db.keyspace(&name, KeyspaceCreateOptions::default)?;
        if !tx.is_empty(&keyspace)? {
            return Err(FjallError::UnmarkedDatabase);
        }
    }
    let mode = match requested {
        AllIndex::Disabled => 0,
        AllIndex::Denormalized => 1,
    };
    tx.insert(&metadata, KEY, [LAYOUT_VERSION, mode]);
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "manifest storage assertions")]
mod tests {
    use super::*;

    #[test]
    fn empty_database_mode_survives_reopen_and_rejected_mode_change() {
        for stored in [AllIndex::Disabled, AllIndex::Denormalized] {
            let dir = tempfile::tempdir().unwrap();
            let requested = match stored {
                AllIndex::Disabled => AllIndex::Denormalized,
                AllIndex::Denormalized => AllIndex::Disabled,
            };
            {
                let db = SingleWriterTxDatabase::builder(dir.path()).open().unwrap();
                validate(&db, stored).unwrap();
            }
            let db = SingleWriterTxDatabase::builder(dir.path()).open().unwrap();
            assert!(matches!(
                validate(&db, requested),
                Err(FjallError::IndexModeMismatch { .. })
            ));
            validate(&db, stored).unwrap();
        }
    }

    #[test]
    fn unmarked_empty_database_initializes_but_nonempty_requires_migration() {
        for has_data in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let db = SingleWriterTxDatabase::builder(dir.path()).open().unwrap();
            let events = db
                .keyspace("events", KeyspaceCreateOptions::default)
                .unwrap();
            if has_data {
                events.insert(b"legacy", b"event").unwrap();
            }
            let result = validate(&db, AllIndex::Denormalized);
            if has_data {
                assert!(matches!(result, Err(FjallError::UnmarkedDatabase)));
                assert_eq!(events.get(b"legacy").unwrap().unwrap().as_ref(), b"event");
                let metadata = db
                    .keyspace(KEYSPACE, KeyspaceCreateOptions::default)
                    .unwrap();
                assert!(metadata.get(KEY).unwrap().is_none());
            } else {
                result.unwrap();
                validate(&db, AllIndex::Denormalized).unwrap();
            }
        }
    }

    #[test]
    fn malformed_and_future_manifests_are_rejected() {
        for value in [
            &[][..],
            &[1][..],
            &[1, 1][..],
            &[2, 7][..],
            &[3, 1][..],
            &[2, 1, 0][..],
        ] {
            let dir = tempfile::tempdir().unwrap();
            let db = SingleWriterTxDatabase::builder(dir.path()).open().unwrap();
            let metadata = db
                .keyspace(KEYSPACE, KeyspaceCreateOptions::default)
                .unwrap();
            metadata.insert(KEY, value).unwrap();
            assert!(matches!(
                validate(&db, AllIndex::Denormalized),
                Err(FjallError::InvalidManifest | FjallError::UnsupportedLayout { .. })
            ));
            assert_eq!(metadata.get(KEY).unwrap().unwrap().as_ref(), value);
        }
    }
}
