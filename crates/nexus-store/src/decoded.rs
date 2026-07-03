//! Consumer-side typed view over the raw envelope streams (#249).
//!
//! [`Subscription`](crate::Subscription), [`read_stream`](crate::RawEventStore::read_stream)
//! and [`read_all`](crate::RawEventStore::read_all) yield **raw**
//! [`PersistedEnvelope`]s by design (the distributed multi-consumer contract:
//! each consumer holds its own codec). This module adds an ergonomic layer on
//! top — it does **not** make the core subscription typed.
//!
//! - [`DecodedStreamExt::decoded`] — for **owning** codecs (JSON, bincode): a
//!   stream of carry-away [`Decoded<E>`] items.
//! - [`DecodedStreamExt::for_each_decoded`] — for **owning and zero-copy**
//!   codecs (rkyv, bytemuck): an internal-iteration fold that hands the borrowed
//!   window to a closure, so no lending stream is needed.

use bytes::Bytes;

use crate::envelope::PersistedEnvelope;
use nexus::Version;

/// A raw envelope, un-packed: the decoded event plus its resume bookmark and
/// metadata. `T` is the owned event (`E`) on the stream path, or the borrowed
/// window (`Decode::Output`) inside a fold closure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded<T> {
    /// The decoded event (owned `E`, or a borrowed window).
    pub event: T,
    /// The per-stream version — the per-stream resume bookmark.
    pub version: Version,
    /// The event's metadata, if any (cheap Arc-shared bytes).
    pub metadata: Option<Bytes>,
}

/// A read from the raw stream failed, or the codec failed to decode the payload.
/// The two failure domains never share a variant (CLAUDE rule 3).
#[derive(Debug, thiserror::Error)]
pub enum DecodeStreamError<R, D> {
    /// The underlying raw stream yielded an error item.
    #[error("subscription stream read failed")]
    Read(#[source] R),
    /// The codec rejected the payload.
    #[error("event decode failed")]
    Decode(#[source] D),
}

/// [`DecodedStreamExt::for_each_decoded`] failure: a raw read, a decode, or the
/// consumer's own fold closure. Three distinct domains (CLAUDE rule 3).
#[derive(Debug, thiserror::Error)]
pub enum FoldDecodedError<R, D, H> {
    /// The underlying raw stream yielded an error item.
    #[error("subscription stream read failed")]
    Read(#[source] R),
    /// The codec rejected the payload.
    #[error("event decode failed")]
    Decode(#[source] D),
    /// The consumer's fold closure returned an error.
    #[error("decoded-event handler failed")]
    Handler(#[source] H),
}

mod sealed {
    pub trait Sealed {}
}

/// A raw stream item that carries a [`PersistedEnvelope`] and can be re-tagged
/// with its decoded counterpart.
///
/// Implemented for the two raw shapes the store yields:
/// - `PersistedEnvelope` (per-stream / `read_stream`) → typed item `Decoded<T>`;
///   the bookmark (`version`) lives **inside** the box.
/// - `(P, PersistedEnvelope)` (`$all` / `read_all`) → typed item
///   `(P, Decoded<T>)`; the `$all` position bookmark stays **beside** the box.
///
/// The asymmetry is intentional (CLAUDE rule 4): each bookmark rides exactly
/// where it lives in the raw layer. Sealed — not implementable downstream.
pub trait RawItem: sealed::Sealed {
    /// The typed item once the envelope is decoded to `Decoded<T>`.
    type Typed<T>;
    /// Borrow the carried envelope (to decode it).
    fn envelope(&self) -> &PersistedEnvelope;
    /// Re-attach the decoded box into this item's shape (copying any tag).
    fn retag<T>(&self, decoded: Decoded<T>) -> Self::Typed<T>;
}

impl sealed::Sealed for PersistedEnvelope {}
impl RawItem for PersistedEnvelope {
    type Typed<T> = Decoded<T>;
    fn envelope(&self) -> &PersistedEnvelope {
        self
    }
    fn retag<T>(&self, decoded: Decoded<T>) -> Decoded<T> {
        decoded
    }
}

impl<P: Copy> sealed::Sealed for (P, PersistedEnvelope) {}
impl<P: Copy> RawItem for (P, PersistedEnvelope) {
    type Typed<T> = (P, Decoded<T>);
    fn envelope(&self) -> &PersistedEnvelope {
        &self.1
    }
    fn retag<T>(&self, decoded: Decoded<T>) -> (P, Decoded<T>) {
        (self.0, decoded)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests")]
#[allow(clippy::expect_used, reason = "tests")]
mod tests {
    use super::*;
    use crate::Store;
    use crate::pending_envelope;
    use crate::store::RawEventStore;
    use crate::stream_id::StreamKey;
    use crate::testing::InMemoryStore;
    use futures::StreamExt;

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
