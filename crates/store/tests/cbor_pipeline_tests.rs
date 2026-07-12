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
use mnesis_store::{ChunkWriter, decode_chunk};

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
                    core::slice::from_ref(&pe),
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
}
