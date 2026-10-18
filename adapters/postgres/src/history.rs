//! A persisted history namespace, separate from controller command identities.

use sqlx::PgConnection;

use crate::PostgresError;

/// Identity of one event history. Persist these bytes with every resume position.
/// Logical reimport creates a new history; coherent physical recovery retains it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct History([u8; 16]);

impl History {
    /// Reconstruct an identity from a saved checkpoint's unchanged bytes.
    #[must_use]
    pub const fn new(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Bytes to persist alongside the transaction and sequence components.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    pub(crate) fn check(self, actual: Self) -> Result<(), PostgresError> {
        if self == actual {
            Ok(())
        } else {
            Err(PostgresError::HistoryMismatch {
                expected: self,
                actual,
            })
        }
    }
}

pub async fn load(connection: &mut PgConnection) -> Result<History, PostgresError> {
    let valid: bool = sqlx::query_scalar(include_str!("history.sql"))
        .fetch_one(&mut *connection)
        .await
        .map_err(PostgresError::Sqlx)?;
    if !valid {
        return Err(PostgresError::DatabaseHistory);
    }
    let bytes = sqlx::query_scalar("SELECT value FROM mnesis.history")
        .fetch_one(&mut *connection)
        .await
        .map_err(PostgresError::Sqlx)?;
    let future: bool = sqlx::query_scalar(
        "SELECT COALESCE((SELECT txid >= pg_catalog.pg_snapshot_xmax(pg_catalog.pg_current_snapshot()) \
         FROM mnesis.events ORDER BY txid DESC, global_seq DESC LIMIT 1), false)",
    )
    .fetch_one(connection)
    .await
    .map_err(PostgresError::Sqlx)?;
    if future {
        return Err(PostgresError::FutureTransaction);
    }
    Ok(History(bytes))
}
