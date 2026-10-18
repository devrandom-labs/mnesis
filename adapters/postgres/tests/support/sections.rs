//! Shared import inputs; assertions belong to the native adapter tests.

use bytes::Bytes;
use mnesis::Version;
use mnesis_store::SchemaVersion;
use mnesis_store::envelope::PersistedEnvelope;
use mnesis_store::import::{ImportBlock, StreamSection};

pub fn pair() -> [StreamSection; 2] {
    [b"first".as_slice(), b"\0second"].map(|origin| StreamSection {
        origin: Bytes::copy_from_slice(origin),
        blocks: [1, 2]
            .map(|number| {
                ImportBlock::Event(
                    PersistedEnvelope::try_new(
                        Version::new(number).unwrap(),
                        Bytes::from_static(b"Import\0payloadmetadata"),
                        SchemaVersion::INITIAL,
                        0..7,
                        7..14,
                        Some(14..22),
                    )
                    .unwrap(),
                )
            })
            .to_vec(),
    })
}
