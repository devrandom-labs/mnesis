//! Full export → CBOR box → import pipeline over the in-memory adapter —
//! relocated from `src/cbor.rs` (mnesis-inmemory is a dev-dependency; type
//! unification with it requires an integration test).

#![cfg(feature = "cbor")]

use futures::StreamExt;
use mnesis::Version;
use mnesis_inmemory::InMemoryStore;
use mnesis_store::envelope::pending_envelope;
use mnesis_store::import::{Atomicity, EventImporter};
use mnesis_store::store::RawEventStore;
use mnesis_store::stream_id::StreamKey;
use mnesis_store::{ChunkError, ChunkWriter, ImportBlock, decode_chunk};

#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code asserts exact values"
)]
mod pipeline {
    use super::*;

    #[tokio::test]
    async fn export_box_import_round_trip_byte_equal_modulo_global_seq() {
        // Seed a source store with two streams.
        let src = InMemoryStore::new();
        for (sid, count) in [("task-1", 3u64), ("task-2", 2)] {
            for v in 1..=count {
                let pe = pending_envelope(Version::new(v).expect("nonzero"))
                    .event_type("E")
                    .payload(format!("{sid}-{v}").into_bytes())
                    .build()
                    .expect("valid envelope");
                src.append(
                    &StreamKey::from_slice(sid.as_bytes()),
                    Version::new(v - 1),
                    mnesis_store::PendingBatch::of(&pe),
                )
                .await
                .expect("append");
            }
        }

        // Export → box-encode into one chunk.
        let mut w = ChunkWriter::new(Vec::new(), Some(b"src")).expect("writer");
        for sid in ["task-1", "task-2"] {
            let s = src
                .read_stream(&StreamKey::from_slice(sid.as_bytes()), Version::INITIAL)
                .await
                .expect("read");
            w.section(sid.as_bytes())
                .expect("section")
                .try_extend(s)
                .await
                .expect("extend");
        }
        let chunk = w.into_sink();

        // Box-decode → import into a fresh store under origin-namespaced ids.
        let sections = decode_chunk(&chunk).expect("decode");
        let dst = InMemoryStore::new();
        let route = |origin: &[u8]| {
            StreamKey::from_slice(format!("src:{}", String::from_utf8_lossy(origin)).as_bytes())
        };
        let report = dst
            .import(&sections, route, Atomicity::PerStream)
            .await
            .expect("import");
        assert!(report.all_complete());

        // Verify byte-equality of payloads/versions modulo global_seq.
        for sid in ["task-1", "task-2"] {
            let target = StreamKey::from_slice(format!("src:{sid}").as_bytes());
            let got: Vec<(u64, Vec<u8>)> = dst
                .read_stream(&target, Version::INITIAL)
                .await
                .expect("read")
                .map(|r| {
                    let e = r.expect("no err");
                    (e.version().as_u64(), e.payload().to_vec())
                })
                .collect()
                .await;
            let expected: Vec<(u64, Vec<u8>)> = (1..=if sid == "task-1" { 3u64 } else { 2 })
                .map(|v| (v, format!("{sid}-{v}").into_bytes()))
                .collect();
            assert_eq!(got, expected, "stream {sid} round-trips");
        }
    }

    /// A minimal valid chunk (header + one section + one Event block) plus the
    /// length of a header-only chunk — torn-tail cuts start past the header,
    /// which must always decode.
    async fn one_event_chunk() -> (Vec<u8>, usize) {
        let src = InMemoryStore::new();
        let pe = pending_envelope(Version::INITIAL)
            .event_type("E")
            .payload(b"hi".to_vec())
            .build()
            .expect("valid envelope");
        src.append(
            &StreamKey::from_slice(b"s"),
            None,
            mnesis_store::PendingBatch::of(&pe),
        )
        .await
        .expect("append");

        let header_len = ChunkWriter::new(Vec::new(), Some(b"src"))
            .expect("writer")
            .into_sink()
            .len();

        let mut w = ChunkWriter::new(Vec::new(), Some(b"src")).expect("writer");
        let stream = src
            .read_stream(&StreamKey::from_slice(b"s"), Version::INITIAL)
            .await
            .expect("read");
        w.section(b"s")
            .expect("section")
            .try_extend(stream)
            .await
            .expect("extend");
        (w.into_sink(), header_len)
    }

    #[tokio::test]
    async fn torn_tail_decodes_to_valid_prefix_never_errors() {
        // A chunk truncated mid-item (end-of-input) must stop cleanly and return
        // the valid prefix — every `is_end_of_input()` guard in decode_block /
        // decode_chunk rides on this. If any guard stopped treating EOF as a torn
        // tail, some cut would surface `Err(Malformed)` instead of `Ok`.
        let (full, header_len) = one_event_chunk().await;
        for cut in header_len..full.len() {
            assert!(
                decode_chunk(&full[..cut]).is_ok(),
                "chunk torn at byte {cut} must decode to a valid prefix, not error"
            );
        }
        // The intact chunk yields exactly one section carrying one Event block.
        let sections = decode_chunk(&full).expect("intact chunk decodes");
        assert_eq!(sections.len(), 1, "one section");
        assert_eq!(sections[0].blocks.len(), 1, "one block");
        assert!(
            matches!(sections[0].blocks[0], ImportBlock::Event(_)),
            "the block is a crc-valid Event"
        );
    }

    #[tokio::test]
    async fn malformed_block_crc_slot_is_error_not_torn_tail() {
        // A block array whose crc slot holds a text string (not a u32) is a
        // structural violation, NOT end-of-input — `decode_block` must surface
        // `Err(Malformed)`, not swallow it as a torn tail (`Ok(None)`).
        let mut w = ChunkWriter::new(Vec::new(), Some(b"src")).expect("writer");
        w.section(b"s").expect("section"); // heading only; SectionWriter dropped here
        let mut bytes = w.into_sink();
        // CBOR: array(2)=0x82, text "x"=0x61 0x78, bstr[0x00]=0x41 0x00.
        bytes.extend_from_slice(&[0x82, 0x61, 0x78, 0x41, 0x00]);
        let err = decode_chunk(&bytes).expect_err("non-u32 crc slot must be Malformed");
        assert!(
            matches!(err, ChunkError::Malformed("malformed block crc")),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn malformed_block_body_slot_is_error_not_torn_tail() {
        // A block array whose body slot holds an integer (not a byte string) is a
        // structural violation, not EOF.
        let mut w = ChunkWriter::new(Vec::new(), Some(b"src")).expect("writer");
        w.section(b"s").expect("section");
        let mut bytes = w.into_sink();
        // array(2)=0x82, u32(0)=0x00, uint(1)=0x01 (a non-bytes body).
        bytes.extend_from_slice(&[0x82, 0x00, 0x01]);
        let err = decode_chunk(&bytes).expect_err("non-bytes body slot must be Malformed");
        assert!(
            matches!(err, ChunkError::Malformed("malformed block body bytes")),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn malformed_section_heading_is_error_not_torn_tail() {
        // A heading map whose stream-id value is an integer (not bytes) fails to
        // decode as a structural violation, not EOF.
        let bytes = {
            let mut b = ChunkWriter::new(Vec::new(), Some(b"src"))
                .expect("writer")
                .into_sink();
            // map(1)=0xA1, key 0=0x00, uint(5)=0x05 (stream_id must be bytes).
            b.extend_from_slice(&[0xA1, 0x00, 0x05]);
            b
        };
        let err = decode_chunk(&bytes).expect_err("non-bytes heading value must be Malformed");
        assert!(
            matches!(err, ChunkError::Malformed("malformed section heading")),
            "got {err:?}"
        );
    }
}
