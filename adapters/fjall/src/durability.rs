use fjall::PersistMode;

/// Persistence required before a successful write acknowledgment.
///
/// Atomic consistency is unchanged by this choice. Survival of power loss
/// also depends on the filesystem and storage device honoring sync requests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Durability {
    /// Sync journal data and metadata (`fsync`). The safe default.
    #[default]
    SyncAll,
    /// Sync journal data (`fdatasync`); use only where that is sufficient.
    SyncData,
    /// Flush to OS buffers; acknowledgment does not promise power-loss survival.
    /// Call [`crate::FjallStore::flush`] to synchronously persist completed writes.
    Buffered,
}

impl Durability {
    pub(crate) const fn persist_mode(self) -> PersistMode {
        match self {
            Self::SyncAll => PersistMode::SyncAll,
            Self::SyncData => PersistMode::SyncData,
            Self::Buffered => PersistMode::Buffer,
        }
    }
}
