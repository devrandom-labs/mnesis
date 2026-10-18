use mnesis_store::BatchSize;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

use crate::error::PostgresError;
use crate::schema::ensure_schema;
use crate::store::PostgresStore;

impl PostgresStore {
    /// Connect to `url`, create a connection pool, and ensure the schema exists.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresError::Sqlx`] on connection failure or schema DDL
    /// failure.
    pub async fn connect(url: &str) -> Result<Self, PostgresError> {
        let pool = PgPoolOptions::new()
            .connect(url)
            .await
            .map_err(PostgresError::Sqlx)?;
        Self::from_pool(pool).await
    }

    /// Build from an existing pool and ensure the schema exists.
    ///
    /// Prefer this when the caller already holds a `PgPool` (e.g. a shared
    /// pool across multiple stores in an application). The pool must accommodate
    /// the dedicated listener alongside data queries.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresError::PoolCapacity`] for an undersized pool, or
    /// [`PostgresError::Sqlx`] if the schema DDL fails.
    pub async fn from_pool(pool: PgPool) -> Result<Self, PostgresError> {
        Self::from_pool_with_batch_size(pool, BatchSize::DEFAULT).await
    }

    /// Build from a shared pool with a validated maximum resident row count.
    ///
    /// # Errors
    /// Returns [`PostgresError::PoolCapacity`] for an undersized pool, or
    /// [`PostgresError::Sqlx`] if schema creation fails.
    pub async fn from_pool_with_batch_size(
        pool: PgPool,
        batch_size: BatchSize,
    ) -> Result<Self, PostgresError> {
        let capacity = pool.options().get_max_connections();
        if capacity < 2 {
            return Err(PostgresError::PoolCapacity { actual: capacity });
        }
        ensure_schema(&pool).await?;
        Ok(Self::assemble(pool, batch_size))
    }
}
