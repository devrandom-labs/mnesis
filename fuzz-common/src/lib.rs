//! Shared fuzz-target bodies for the `mnesis-store` untrusted-byte parse surface.
//!
//! Each `pub fn` takes raw bytes and drives one decoder that must survive
//! hostile input. A panic is a finding: parsing untrusted input never panics —
//! that would be a DoS bug (a persisted frame or a backup-box chunk arrives from
//! disk/network and may be truncated, tampered, or bit-rotted). These functions
//! are the single source of truth for both engines: the bolero crate (`fuzz/`)
//! calls them from `check!().for_each(...)`, and the afl.rs crate (`fuzz-afl/`)
//! calls them from `afl::fuzz!(...)`.

use mnesis_store::bytes::Bytes;
use mnesis_store::{EventType, cbor, wire};

/// The canonical on-disk frame decoder — the irreversible wire format every
/// adapter reads. Truncation, a bad length prefix, or a corrupt version byte
/// must surface as a typed `DecodeError`, never a panic.
pub fn wire_decode_frame(data: &[u8]) {
    let _ = wire::decode_frame(data);
}

/// The backup-box CBOR chunk header (magic, format version, optional origin).
pub fn cbor_decode_header(data: &[u8]) {
    let _ = cbor::decode_header(data);
}

/// The backup-box CBOR chunk body: per-stream sections of crc32c-checked blocks.
/// A checksum failure is a non-error `Corrupt` block, not a panic; a malformed
/// structure is a typed `ChunkError`.
pub fn cbor_decode_chunk(data: &[u8]) {
    let _ = cbor::decode_chunk(data);
}

/// The event-type value newtype: UTF-8 validity plus the `u16::MAX` length cap.
/// The only non-trivial validator among the envelope value fields (the boundary
/// where a non-UTF-8 or oversized event-type name must be rejected, not panic).
pub fn value_event_type(data: &[u8]) {
    let _ = EventType::from_bytes(Bytes::copy_from_slice(data));
}

#[cfg(test)]
mod tests {
    use super::*;

    // Proves every shared body is wired to a real mnesis-store decoder (not a
    // stub) and returns without panic on empty input — the boundary case both
    // engines hit first. Each call fails the build if the underlying symbol were
    // renamed or removed.
    #[test]
    fn all_targets_accept_empty_without_panic() {
        wire_decode_frame(&[]);
        cbor_decode_header(&[]);
        cbor_decode_chunk(&[]);
        value_event_type(&[]);
    }
}
