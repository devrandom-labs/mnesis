use mnesis::ErrorId;
use mnesis_store::envelope::EnvelopeError;

/// Errors produced by the postgres event store adapter.
///
/// Distinct failure domains (CLAUDE rule 3); diagnostic fields use
/// [`ErrorId`] to stay allocation-light on error paths.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PostgresError {
    /// Explicit migration failed; inspect the source before repairing or retrying.
    #[error("postgres migration failed: {0}")]
    Migration(#[source] sqlx::migrate::MigrateError),

    /// Runtime opening never repairs or changes an incompatible schema.
    #[error("incompatible postgres schema: {reason}")]
    Schema { reason: ErrorId<128> },

    /// A row exceeds the configured read/write byte ceiling. No row is skipped.
    #[error("row requires {actual} bytes, exceeding configured limit {limit}")]
    RowLimit { actual: i64, limit: i64 },

    /// Caller-provided byte lengths cannot be represented or added safely.
    #[error("input row byte count overflow")]
    SizeOverflow {
        #[source]
        source: Option<std::num::TryFromIntError>,
    },

    /// The shared pool cannot accommodate both LISTEN and a data query.
    #[error("pool capacity {actual} cannot accommodate the listener and data queries")]
    PoolCapacity { actual: u32 },

    /// A `sqlx`/postgres I/O, connection, or query error.
    #[error("postgres error: {0}")]
    Sqlx(#[source] sqlx::Error),

    /// COMMIT may have succeeded. Reconcile durable state before a new decision;
    /// this outcome never proves that retrying the same command is safe.
    #[error("commit outcome unknown; reconcile before retrying: {0}")]
    CommitUnknown(#[source] sqlx::Error),

    /// A persisted row decoded into bytes that fail envelope validation.
    #[error("envelope integrity error in stream '{stream_id}' at version {version}")]
    EnvelopeCorrupt {
        stream_id: ErrorId,
        version: u64,
        #[source]
        source: EnvelopeError,
    },

    /// A stored row has an out-of-range / corrupt scalar (e.g. `global_seq` <= 0,
    /// version <= 0, `schema_version` == 0).
    #[error("corrupt row in stream '{stream_id}': {reason}")]
    CorruptRow {
        stream_id: ErrorId,
        reason: ErrorId<128>,
    },

    /// Building the canonical wire frame for a row failed.
    #[error("wire frame build failed in stream '{stream_id}' at version {version}: {reason}")]
    Frame {
        stream_id: ErrorId,
        version: u64,
        reason: ErrorId<128>,
    },

    /// Wake registration over `LISTEN/NOTIFY` failed.
    #[error("listen/notify wake setup failed: {0}")]
    Wake(#[source] sqlx::Error),

    /// A consistent export request could not be admitted to its closed view.
    #[cfg(feature = "export")]
    #[error("export request unavailable: {0}")]
    ExportRequest(#[source] tokio::sync::mpsc::error::SendError<()>),

    /// The view stopped before returning the requested page or startup result.
    #[cfg(feature = "export")]
    #[error("export reply unavailable: {0}")]
    ExportReply(#[source] tokio::sync::oneshot::error::RecvError),

    /// The export owner stopped unexpectedly; retain its task failure.
    #[cfg(feature = "export")]
    #[error("export task failed: {0}")]
    ExportTask(#[source] tokio::task::JoinError),
}

impl PostgresError {
    /// Only explicit server rollbacks prove rejection while awaiting COMMIT.
    /// In particular class 40 includes `40003` (completion unknown).
    pub(crate) fn commit(source: sqlx::Error) -> Self {
        if let sqlx::Error::Database(error) = &source {
            let code = error.code();
            if code
                .as_deref()
                .is_some_and(|value| value.starts_with("23") || matches!(value, "40001" | "40P01"))
            {
                return Self::Sqlx(source);
            }
        }
        Self::CommitUnknown(source)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test code")]
mod tests {
    use super::*;

    /// Verify `PostgresError` satisfies the bounds required by `RawEventStore::Error`.
    #[test]
    fn satisfies_error_send_sync_static() {
        fn assert_bounds<T: std::error::Error + Send + Sync + 'static>() {}
        assert_bounds::<PostgresError>();
    }

    /// Defensive boundary: an over-long stream id in the `ErrorId<64>` field
    /// renders truncated and `…`-suffixed through the error's `Display`.
    #[test]
    fn corrupt_row_display_truncates_overlong_stream_id() {
        let err = PostgresError::CorruptRow {
            stream_id: ErrorId::from_display(&"s".repeat(200)),
            reason: ErrorId::from_display(&"bad version"),
        };
        let msg = err.to_string();
        assert!(msg.contains('…'), "display must signal truncation: {msg}");
        assert!(!msg.contains(&"s".repeat(100)));
    }
}
