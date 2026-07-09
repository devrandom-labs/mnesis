//! Relocated inline test mod of `src/decoded.rs` (nexus-inmemory is a
//! dev-dependency; type unification with it requires an integration test).

use nexus::Version;

use nexus_store::envelope::PersistedEnvelope;

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests")]
#[allow(clippy::expect_used, reason = "tests")]
mod tests {
    use super::*;
    use futures::StreamExt;
    use nexus_inmemory::InMemoryStore;
    use nexus_store::Store;
    use nexus_store::decoded::*;
    use nexus_store::pending_envelope;
    use nexus_store::store::RawEventStore;
    use nexus_store::stream_id::StreamKey;

    /// Build a real `PersistedEnvelope` by round-tripping through
    /// `InMemoryStore`: append a `PendingEnvelope`, then read it back. There is
    /// no test-only `PendingEnvelope -> PersistedEnvelope` constructor in
    /// `envelope.rs` (`PersistedEnvelope::for_decode` synthesizes an envelope
    /// but carries no version, so it cannot exercise the `version` field this
    /// module's tests need) — this mirrors the exact pattern used by
    /// `crates/nexus-store/tests/subscription_tests.rs`.
    async fn env(version: u64, meta: Option<&[u8]>) -> PersistedEnvelope {
        let store = Store::new(InMemoryStore::new());
        let id = StreamKey::from_slice(b"stream-1");

        // `append`'s `expected_version` is the CURRENT version (`None` = empty
        // stream); to land an event at an arbitrary target `version` the
        // stream must first be filled up to `version - 1`.
        let mut expected = None;
        for filler_version in 1..version {
            let filler = pending_envelope(Version::new(filler_version).expect("nonzero version"))
                .event_type("Filler")
                .payload(b"filler".to_vec())
                .build()
                .expect("valid envelope");
            store
                .append(&id, expected, &[filler])
                .await
                .expect("append succeeds");
            expected = Version::new(filler_version);
        }

        let mut builder = pending_envelope(Version::new(version).expect("nonzero version"))
            .event_type("E")
            .payload(b"payload".to_vec());
        if let Some(m) = meta {
            builder = builder.metadata(m.to_vec());
        }
        let envelope = builder.build().expect("valid envelope");
        store
            .append(&id, expected, &[envelope])
            .await
            .expect("append succeeds");

        let raw_stream = store
            .read_stream(&id, Version::new(version).expect("nonzero version"))
            .await
            .expect("read_stream succeeds");
        let mut stream = std::pin::pin!(raw_stream);
        stream
            .next()
            .await
            .expect("at least one event")
            .expect("read succeeds")
    }

    #[tokio::test]
    async fn retag_on_bare_envelope_is_identity_shape() {
        let e = env(3, Some(b"m")).await;
        let decoded = Decoded {
            event: 42u64,
            version: e.version(),
            metadata: e.metadata_bytes(),
        };
        let typed: Decoded<u64> = e.retag(decoded);
        assert_eq!(typed.event, 42);
        assert_eq!(typed.version, Version::new(3).expect("nonzero version"));
        assert_eq!(typed.metadata.as_deref(), Some(b"m".as_ref()));
    }

    #[tokio::test]
    async fn retag_on_tagged_item_copies_the_position_beside_the_box() {
        let e = env(1, None).await;
        let item = (99u64, e);
        let decoded = Decoded {
            event: 7u64,
            version: item.envelope().version(),
            metadata: None,
        };
        let (pos, typed): (u64, Decoded<u64>) = item.retag(decoded);
        assert_eq!(pos, 99);
        assert_eq!(typed.event, 7);
        assert_eq!(typed.version, Version::new(1).expect("nonzero version"));
    }

    #[test]
    fn error_variants_render_distinct_messages() {
        #[derive(Debug, thiserror::Error)]
        #[error("boom")]
        struct Boom;
        let read: DecodeStreamError<Boom, Boom> = DecodeStreamError::Read(Boom);
        let decode: DecodeStreamError<Boom, Boom> = DecodeStreamError::Decode(Boom);
        assert_eq!(read.to_string(), "subscription stream read failed");
        assert_eq!(decode.to_string(), "event decode failed");
    }
}
