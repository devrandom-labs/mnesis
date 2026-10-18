//! History-scoped, transaction-ordered Postgres resume positions.

use crate::History;

/// A resume position within one event history.
///
/// Persist all three components: [`history`](Self::history), [`txid`](Self::txid)
/// and [`seq`](Self::seq). Positions from another history are rejected before a
/// scan. Logical reimport creates a new history; coherent physical recovery
/// preserves it. Legacy two-number checkpoints require explicit provenance
/// verification before attaching the migrated store's history identity.
///
/// Within one history, ordering is lexicographic `(txid, seq)` and matches
/// `ORDER BY txid, global_seq`. The xmin barrier prevents skipped late commits;
/// this is stable delivery order, not commit-acknowledgement timestamps.
/// Ordering between different histories has no consumer meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PgAllPos {
    history: History,
    txid: u64,
    seq: u64,
}

impl PgAllPos {
    /// Reconstruct a checkpoint from all its unchanged persisted components.
    #[must_use]
    pub const fn new(history: History, txid: u64, seq: u64) -> Self {
        Self { history, txid, seq }
    }

    /// History identity to persist with the numeric components.
    #[must_use]
    pub const fn history(self) -> History {
        self.history
    }

    /// The writing transaction ID component.
    #[must_use]
    pub const fn txid(self) -> u64 {
        self.txid
    }

    /// The global sequence component.
    #[must_use]
    pub const fn seq(self) -> u64 {
        self.seq
    }
}

impl mnesis_store::AllPosition for PgAllPos {}

#[cfg(test)]
mod tests {
    use super::*;

    const HISTORY: History = History::new([1; 16]);

    #[test]
    fn ord_is_txid_first_then_seq() {
        assert!(PgAllPos::new(HISTORY, 1, 9) < PgAllPos::new(HISTORY, 2, 0));
        assert!(PgAllPos::new(HISTORY, 3, 1) < PgAllPos::new(HISTORY, 3, 2));
        assert_eq!(PgAllPos::new(HISTORY, 5, 5), PgAllPos::new(HISTORY, 5, 5));
    }

    #[test]
    fn accessors_round_trip() {
        let history = History::new([7; 16]);
        let pos = PgAllPos::new(history, 42, 7);
        assert_eq!(pos.history().as_bytes(), &[7; 16]);
        assert_eq!(pos.txid(), 42);
        assert_eq!(pos.seq(), 7);
        assert_eq!(core::mem::size_of::<PgAllPos>(), 32);
    }
}
