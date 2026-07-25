//! fjall-private parameterization of a bounded keyset scan: the only parts
//! that differ between the per-stream (Version-keyed) and $all (GlobalSeq-keyed)
//! reads — the keyset bound bytes and how a stored row decodes into a
//! [`PersistedEnvelope`]. NOT exported; no other adapter shares fjall's on-disk
//! key layout, so this stays inside `mnesis-fjall`.

use bytes::Bytes;
use fjall::Slice;
use mnesis::{ErrorId, Version};
use mnesis_store::PersistedEnvelope;

use crate::error::{FjallError, reason_label};
use crate::global_seq::GlobalSeq;
use crate::subscription_id::OwnedStreamId;
use crate::wire_key::{
    GLOBAL_KEY_PREFIX_SIZE, decode_event_key, decode_global_key, encode_event_key,
    encode_global_key,
};
use mnesis_store::StreamKey;
use mnesis_store::wire;

/// The differing parts of a bounded keyset scan, factored so one cursor can
/// drive both the per-stream and `$all` reads.
pub trait ScanStrategy: Send {
    /// The position the scan opens from ([`Version`] for a stream, [`GlobalSeq`] for `$all`).
    type Position: Copy + Send;
    /// What one decoded row yields. Per-stream: a bare [`PersistedEnvelope`];
    /// `$all`: a `(GlobalSeq, StreamKey, PersistedEnvelope)` tagged with the
    /// key-derived position and origin stream (the envelope carries neither),
    /// matching the attributed `$all` stream contract (#333).
    type Item: Send;
    /// Keyset lower bound (inclusive) for opening at `from`.
    fn lower_key(&self, from: Self::Position) -> Result<Vec<u8>, FjallError>;
    /// Keyset upper bound (inclusive) — the end of this strategy's key range,
    /// or `None` when the scan is unbounded above: the `events_global`
    /// partition holds ONLY `$all` keys, and with a variable-length id inside
    /// the key no fixed-width maximum key exists, so [`GlobalScan`] scans to
    /// the partition's end.
    fn upper_key(&self) -> Result<Option<Vec<u8>>, FjallError>;
    /// Decode one stored row into this strategy's item, mapping malformed
    /// shapes to `FjallError`.
    fn decode(&self, key: &Slice, value: Slice) -> Result<Self::Item, FjallError>;
}

/// Per-stream scan: keyed by `[id_len][id_bytes][version]`, opens from a [`Version`].
pub struct StreamScan {
    pub id: OwnedStreamId,
    pub label: ErrorId,
}

/// `$all` scan: keyed by `[global_seq][id_len][id][version]` (#333, layout A2 —
/// the origin stream id rides in the KEY; the value stays the shared frame
/// bytes), opens from a [`GlobalSeq`].
pub struct GlobalScan;

/// Shared decode tail: validate the raw `version` and build the envelope,
/// mapping the two terminal failures identically for both strategies. The
/// `$all` position is **not** built here — it is the key-derived `GlobalSeq`
/// the [`GlobalScan`] pairs on top (the envelope no longer carries it).
///
/// `stream_id` is the diagnostic label stamped into every error: the per-stream
/// label for [`StreamScan`], the key-decoded origin-stream label for
/// [`GlobalScan`] (the `$all` key carries the id since #333; this helper is
/// just handed the ready-made label, never the key). `raw_version` is the
/// version decoded from the key; it appears verbatim in the error fields.
fn build_envelope(
    bytes_value: Bytes,
    decoded: wire::DecodedFrame,
    raw_version: u64,
    stream_id: ErrorId,
) -> Result<PersistedEnvelope, FjallError> {
    let version = Version::new(raw_version).ok_or(FjallError::CorruptValue {
        stream_id,
        version: Some(raw_version),
    })?;

    PersistedEnvelope::try_new(
        version,
        bytes_value,
        decoded.schema_version,
        decoded.offsets.event_type,
        decoded.offsets.payload,
        decoded.offsets.metadata,
    )
    .map_err(|source| FjallError::EnvelopeCorrupt {
        stream_id,
        version: raw_version,
        source,
    })
}

impl ScanStrategy for StreamScan {
    type Position = Version;
    type Item = PersistedEnvelope;

    fn lower_key(&self, from: Self::Position) -> Result<Vec<u8>, FjallError> {
        encode_event_key(self.id.as_ref(), from.as_u64()).map_err(|e| FjallError::InvalidInput {
            stream_id: self.label,
            version: from.as_u64(),
            reason: reason_label(&e),
        })
    }

    fn upper_key(&self) -> Result<Option<Vec<u8>>, FjallError> {
        encode_event_key(self.id.as_ref(), u64::MAX)
            .map(Some)
            .map_err(|e| FjallError::InvalidInput {
                stream_id: self.label,
                version: u64::MAX,
                reason: reason_label(&e),
            })
    }

    fn decode(&self, key: &Slice, value: Slice) -> Result<Self::Item, FjallError> {
        let (_id_bytes, version) = decode_event_key(key).map_err(|_| FjallError::CorruptValue {
            stream_id: self.label,
            version: None,
        })?;

        let bytes_value: Bytes = value.into();
        let decoded =
            wire::decode_frame(bytes_value.as_ref()).map_err(|_| FjallError::CorruptValue {
                stream_id: self.label,
                version: Some(version),
            })?;

        build_envelope(bytes_value, decoded, version, self.label)
    }
}

impl ScanStrategy for GlobalScan {
    type Position = GlobalSeq;
    type Item = (GlobalSeq, StreamKey, PersistedEnvelope);

    fn lower_key(&self, from: Self::Position) -> Result<Vec<u8>, FjallError> {
        // The empty-id lower bound sorts before every real key with the same
        // global_seq: fjall rejects an empty stream key at `append`, so every
        // stored key carries id_len >= 1 and compares greater at byte 9. The
        // encode error is unreachable (an empty id can never be over-long) but
        // stays typed, mirroring `StreamScan::lower_key` — never an unwrap.
        encode_global_key(from.as_u64(), b"", 0).map_err(|e| FjallError::InvalidInput {
            stream_id: ErrorId::default(),
            version: from.as_u64(),
            reason: reason_label(&e),
        })
    }

    fn upper_key(&self) -> Result<Option<Vec<u8>>, FjallError> {
        // Unbounded above: the events_global partition holds only `$all` keys,
        // and the variable-length id inside the key means no fixed-width max
        // key exists — scan to the partition's end.
        Ok(None)
    }

    fn decode(&self, key: &Slice, value: Slice) -> Result<Self::Item, FjallError> {
        let (key_global_seq, id_bytes, version_raw) =
            decode_global_key(key).map_err(|_| FjallError::CorruptValue {
                stream_id: ErrorId::default(),
                version: None,
            })?;
        let id_len = id_bytes.len();
        // The codec accepts id_len = 0 (it only describes bytes), but the
        // storage invariant is judged HERE: fjall rejects an empty stream id
        // at `append`, so a STORED row claiming an empty origin id is
        // corruption, never a valid empty `StreamKey`.
        if id_bytes.is_empty() {
            return Err(FjallError::CorruptValue {
                stream_id: ErrorId::default(),
                version: Some(version_raw),
            });
        }

        // Zero-copy: the key Slice is Arc-backed Bytes (bytes_1); subslice the
        // id out of it rather than copying. The decoded id doubles as the
        // diagnostic label for every error raised past this point.
        let key_bytes: Bytes = key.clone().into();
        let stream = StreamKey::from_bytes(
            key_bytes.slice(GLOBAL_KEY_PREFIX_SIZE..GLOBAL_KEY_PREFIX_SIZE + id_len),
        );
        let label = ErrorId::from_display(&stream);

        // The key is the authoritative `$all` position; an event is always
        // stamped with global_seq >= 1, so a 0 here is corruption.
        let position = GlobalSeq::new(key_global_seq).ok_or(FjallError::CorruptValue {
            stream_id: label,
            version: Some(version_raw),
        })?;

        let bytes_value: Bytes = value.into();
        let decoded =
            wire::decode_frame(bytes_value.as_ref()).map_err(|_| FjallError::CorruptValue {
                stream_id: label,
                version: Some(version_raw),
            })?;

        // Tag the envelope with the key-derived position and origin stream —
        // the attributed `$all` contract (#333). The frame stores neither, so
        // there is no redundant key/frame cross-check to perform.
        let env = build_envelope(bytes_value, decoded, version_raw, label)?;
        Ok((position, stream, env))
    }
}

/// A bounded read cursor over a single lazy `fjall::Iter`.
///
/// `fjall::Keyspace::range` returns a lazy k-way-merge cursor over LSM blocks
/// (it pulls the next block from disk only when the current one drains), so a
/// single `Iter` already bounds memory — no batching/refill needed. Holding it
/// pins one consistent snapshot for the read's duration (repeatable-read).
pub struct ScanCursor<S: ScanStrategy> {
    iter: fjall::Iter,
    strategy: S,
    /// Once an error is yielded the cursor is poisoned: subsequent polls return
    /// `None` rather than silently skipping corrupt rows.
    poisoned: bool,
}

impl<S: ScanStrategy> ScanCursor<S> {
    /// Open a bounded scan from `from` (inclusive). Fallible only because the
    /// keyset bound keys may fail to encode (e.g. an over-long id).
    ///
    /// The snapshot is taken **at `open` time**, not at first poll: the returned
    /// cursor reads a consistent point-in-time view as of `open`, so events
    /// appended after `open()` but before/while polling are **not** observed.
    /// Long-lived use therefore pins the GC watermark — a bounded read completes
    /// promptly, but a never-ending subscription must open a **fresh**
    /// [`ScanCursor`] per refill rather than hold one for its whole life.
    pub fn open(
        keyspace: &fjall::SingleWriterTxKeyspace,
        strategy: S,
        from: S::Position,
    ) -> Result<Self, FjallError> {
        let lower = strategy.lower_key(from)?;
        let iter = match strategy.upper_key()? {
            Some(upper) => keyspace.inner().range(lower..=upper),
            None => keyspace.inner().range(lower..),
        };
        Ok(Self {
            iter,
            strategy,
            poisoned: false,
        })
    }

    /// Open an intentionally **empty** cursor — the `$all` ceiling case, where
    /// nothing is strictly after the maximum position so the exclusive resume
    /// has no successor. Uses a canonical **empty half-open** bound
    /// (`[0] .. [0]`: `start == end`, so no key satisfies it) — not a *reversed*
    /// inclusive bound, whose emptiness would depend on fjall's undocumented
    /// handling of `start > end` (a future upgrade could panic there instead).
    /// Infallible (the bound is constant), unlike [`open`](Self::open).
    pub fn open_empty(keyspace: &fjall::SingleWriterTxKeyspace, strategy: S) -> Self {
        let iter = keyspace.inner().range(vec![0u8]..vec![0u8]);
        Self {
            iter,
            strategy,
            poisoned: false,
        }
    }

    fn poll_one(&mut self) -> Option<Result<S::Item, FjallError>> {
        if self.poisoned {
            return None;
        }
        let guard = self.iter.next()?;
        let (key, value) = match guard.into_inner() {
            Ok(kv) => kv,
            Err(e) => {
                self.poisoned = true;
                return Some(Err(FjallError::Io(e)));
            }
        };
        // Poison on a decode error, then surface the result as-is.
        Some(
            self.strategy
                .decode(&key, value)
                .inspect_err(|_| self.poisoned = true),
        )
    }
}

// `get_mut()` in `poll_next` requires `Self: Unpin`; `fjall::Iter` is already
// `Unpin`, so `S` is the only field that isn't `Unpin` by default — hence the
// `S: Unpin` bound (also relied on by the generic live loop in `mnesis-store`).
impl<S: ScanStrategy + Unpin> futures::Stream for ScanCursor<S> {
    type Item = Result<S::Item, FjallError>;

    fn poll_next(
        self: core::pin::Pin<&mut Self>,
        _cx: &mut core::task::Context<'_>,
    ) -> core::task::Poll<Option<Self::Item>> {
        core::task::Poll::Ready(self.get_mut().poll_one())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test code")]
#[allow(clippy::panic, reason = "test code")]
mod tests {
    use super::*;
    use crate::store::FjallStore;
    use crate::store::read_test_helpers::{sk, temp_store};
    use futures::StreamExt;
    use mnesis_store::PendingBatch;
    use mnesis_store::StreamKey;
    use mnesis_store::envelope::pending_envelope;
    use mnesis_store::store::RawEventStore;
    use mnesis_store::value::{EventType, Payload, SchemaVersion};
    use mnesis_store::wire;

    /// Build a wire-frame event-value row via the real production encoder
    /// (`wire::encode_frame` + the `mnesis_store::value` newtypes), for the
    /// row-decode tests below. `schema_version` is always 1 and there is no
    /// metadata — the cases this test mod exercises. The `$all` position is not
    /// in the value (V2); the `events_global` key carries it.
    fn test_row_value(event_type: &str, payload: &[u8]) -> Vec<u8> {
        let sv = SchemaVersion::from_u32(1).unwrap();
        let et = EventType::from_bytes(Bytes::copy_from_slice(event_type.as_bytes())).unwrap();
        let pl = Payload::from_bytes(Bytes::copy_from_slice(payload)).unwrap();
        wire::encode_frame(sv, &et, &pl, None)
            .unwrap()
            .value
            .to_vec()
    }

    async fn append_versions(
        store: &FjallStore,
        id: &StreamKey,
        versions: std::ops::RangeInclusive<u64>,
    ) {
        for v in versions {
            let env = pending_envelope(Version::new(v).unwrap())
                .event_type("E")
                .payload(format!("v{v}").into_bytes())
                .build()
                .unwrap();
            store
                .append(id, Version::new(v - 1), PendingBatch::of(&env))
                .await
                .unwrap();
        }
    }

    #[tokio::test]
    async fn scan_cursor_yields_rows_in_order() {
        let (store, _dir) = temp_store();
        let id = sk("s");
        append_versions(&store, &id, 1..=3).await;

        let cursor = ScanCursor::open(
            store.partitions.events(),
            StreamScan {
                id: OwnedStreamId::from_id(&id),
                label: ErrorId::from_display(&id),
            },
            Version::INITIAL,
        )
        .unwrap();

        let versions: Vec<u64> = cursor
            .map(|item| item.unwrap().version().as_u64())
            .collect()
            .await;
        assert_eq!(versions, vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn scan_cursor_opens_from_midpoint() {
        let (store, _dir) = temp_store();
        let id = sk("s");
        append_versions(&store, &id, 1..=5).await;

        let cursor = ScanCursor::open(
            store.partitions.events(),
            StreamScan {
                id: OwnedStreamId::from_id(&id),
                label: ErrorId::from_display(&id),
            },
            Version::new(3).unwrap(),
        )
        .unwrap();

        let versions: Vec<u64> = cursor
            .map(|item| item.unwrap().version().as_u64())
            .collect()
            .await;
        assert_eq!(versions, vec![3, 4, 5]);
    }

    #[tokio::test]
    async fn scan_cursor_global_yields_ascending_global_seq() {
        let (store, _dir) = temp_store();
        let a = sk("a");
        let b = sk("b");

        // Interleave appends across two streams so global_seq order differs
        // from per-stream version order.
        append_versions(&store, &a, 1..=1).await; // global_seq 1
        append_versions(&store, &b, 1..=1).await; // global_seq 2
        append_versions(&store, &a, 2..=2).await; // global_seq 3
        append_versions(&store, &b, 2..=2).await; // global_seq 4

        let cursor = ScanCursor::open(
            store.partitions.events_global(),
            GlobalScan,
            GlobalSeq::INITIAL,
        )
        .unwrap();

        // The `$all` scan is position- AND stream-tagged; both ride on the
        // key-derived tags, so the interleaving order [a, b, a, b] is
        // observable without decoding a payload.
        let items: Vec<(u64, Vec<u8>)> = cursor
            .map(|item| {
                let (pos, stream, _env) = item.unwrap();
                (pos.as_u64(), stream.as_bytes().to_vec())
            })
            .collect()
            .await;
        assert_eq!(
            items,
            vec![
                (1, b"a".to_vec()),
                (2, b"b".to_vec()),
                (3, b"a".to_vec()),
                (4, b"b".to_vec()),
            ],
        );
    }

    fn stream_scan(id_bytes: &[u8], label: &str) -> StreamScan {
        StreamScan {
            id: OwnedStreamId::from_id(&label_id(id_bytes)),
            label: ErrorId::from_display(&label),
        }
    }

    /// A [`StreamKey`] over borrowed bytes, used only to feed
    /// [`OwnedStreamId::from_id`] in tests.
    fn label_id(bytes: &[u8]) -> StreamKey {
        StreamKey::from_slice(bytes)
    }

    fn row(id: &[u8], version: u64, et: &str, payload: &[u8]) -> (Slice, Slice) {
        let key = encode_event_key(id, version).unwrap();
        let val = test_row_value(et, payload);
        (Slice::from(key), Slice::from(val))
    }

    fn global_row(
        global_seq: u64,
        id: &[u8],
        version: u64,
        et: &str,
        payload: &[u8],
    ) -> (Slice, Slice) {
        // `global_seq` + origin id ride in the `$all` index KEY (#333, A2);
        // the value (frame) holds neither.
        let key = encode_global_key(global_seq, id, version).unwrap();
        let val = test_row_value(et, payload);
        (Slice::from(key), Slice::from(val))
    }

    #[test]
    fn stream_decode_yields_envelope() {
        let (k, v) = row(b"user-1", 7, "Created", b"data");
        let scan = stream_scan(b"user-1", "user-1");
        let env = scan.decode(&k, v).unwrap();
        assert_eq!(env.version(), Version::new(7).unwrap());
        assert_eq!(env.event_type(), "Created");
        assert_eq!(env.payload(), b"data");
    }

    #[test]
    fn stream_decode_rejects_truncated_value() {
        let k = Slice::from(encode_event_key(b"corrupt", 1).unwrap());
        let v = Slice::from(&[0u8, 1, 2][..]);
        let scan = stream_scan(b"corrupt", "corrupt");
        match scan.decode(&k, v).unwrap_err() {
            FjallError::CorruptValue { stream_id, version } => {
                assert_eq!(stream_id.as_str(), "corrupt");
                assert_eq!(version, Some(1));
            }
            other => panic!("expected CorruptValue, got {other:?}"),
        }
    }

    #[test]
    fn stream_decode_rejects_non_utf8_event_type() {
        // `wire::decode_frame` does not UTF-8-validate `event_type`; the
        // read path's `PersistedEnvelope::try_new` does, surfacing it as
        // `FjallError::EnvelopeCorrupt`. Build a valid frame, then overwrite
        // the `event_type` bytes in place (same length) with invalid UTF-8.
        let (k, v) = row(b"user-1", 7, "ABC", b"data");
        let mut raw = v.to_vec();
        // Derive the event_type start offset from a publicly-decoded
        // `FrameOffsets` rather than a private wire header-size constant —
        // `decode_frame` is the adapter-facing read path and already exposes
        // every offset a decoder needs.
        let et_start =
            usize::try_from(wire::decode_frame(&raw).unwrap().offsets.event_type.start).unwrap();
        // 0xFF is never a valid UTF-8 byte; keep the 3-byte length intact.
        raw[et_start] = 0xFF;
        raw[et_start + 1] = 0xFE;
        raw[et_start + 2] = 0xFF;
        let corrupt = Slice::from(raw);

        let scan = stream_scan(b"user-1", "user-1");
        match scan.decode(&k, corrupt).unwrap_err() {
            FjallError::EnvelopeCorrupt {
                stream_id, version, ..
            } => {
                assert_eq!(stream_id.as_str(), "user-1");
                assert_eq!(version, 7);
            }
            other => panic!("expected EnvelopeCorrupt, got {other:?}"),
        }
    }

    #[test]
    fn global_decode_yields_position_tagged_envelope() {
        let (k, v) = global_row(42, b"user-1", 7, "Created", b"data");
        // `$all` decode pairs the key-derived position AND origin stream with
        // the envelope (#333).
        let (pos, stream, env) = GlobalScan.decode(&k, v).unwrap();
        assert_eq!(pos, GlobalSeq::new(42).unwrap());
        assert_eq!(stream.as_bytes(), b"user-1");
        assert_eq!(env.version(), Version::new(7).unwrap());
        assert_eq!(env.event_type(), "Created");
        assert_eq!(env.payload(), b"data");
    }
}
