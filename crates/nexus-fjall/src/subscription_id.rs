//! Private module: the `pub` items below are crate-local by containment (no
//! external API leak). It exists so the per-stream `scan.rs` strategy has an
//! owned [`OwnedStreamId`] satisfying the [`Id`](nexus::Id) `'static` bound
//! across the re-reads the generic subscription loop performs.

use nexus_store::StreamKey;

/// Owned byte-key wrapper to satisfy the [`Id`](nexus::Id) trait's `'static`
/// bound when re-reading from the store during subscription refills.
///
/// It is an `Id` for free via the blanket impl — it carries `Clone`, `Debug`,
/// `Hash`, `Eq`, `Display`, and `AsRef<[u8]>` below, with no `impl Id` block.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct OwnedStreamId(Vec<u8>);

impl OwnedStreamId {
    /// Create from any stream key by capturing its byte representation.
    pub fn from_id(id: &StreamKey) -> Self {
        Self(id.as_ref().to_vec())
    }
}

impl std::fmt::Display for OwnedStreamId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match std::str::from_utf8(&self.0) {
            Ok(s) => f.write_str(s),
            Err(_) => write!(f, "<{} bytes>", self.0.len()),
        }
    }
}

impl AsRef<[u8]> for OwnedStreamId {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}
