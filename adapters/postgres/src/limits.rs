use std::num::NonZeroU32;

use mnesis_store::BatchSize;

/// Resident page limits, including the maximum accepted individual row.
///
/// Bytes count stream ID, event type, payload, metadata and 32 scalar bytes.
/// Snapshot rows count ID, state and the same 32-byte scalar allowance.
/// A row exceeding `bytes` is rejected by writes and reported by reads without
/// fetching its body. Raise this explicitly to read larger existing rows.
/// Client memory also includes bounded per-row bookkeeping and frame decoding.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Maximum number of rows retained by one cursor page.
    pub rows: BatchSize,
    /// Maximum total row bytes in one page; also the individual row ceiling.
    pub bytes: NonZeroU32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            rows: BatchSize::DEFAULT,
            bytes: NonZeroU32::MIN.saturating_add(16_777_215),
        }
    }
}
