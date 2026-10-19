use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

use crate::Limits;
use crate::error::PostgresError;
use crate::schema;
use crate::store::PostgresStore;

impl PostgresStore {
    /// Install or upgrade the owned schema using an administrative pool.
    /// Stop old writers before adopting an unversioned database. Runtime open
    /// validates the installed schema without requiring DDL permissions.
    ///
    /// # Errors
    /// Returns typed migration, database or schema errors without resetting data.
    pub async fn migrate(pool: &PgPool) -> Result<(), PostgresError> {
        schema::migrate(pool).await
    }

    /// Connect to `url` and validate the explicitly migrated schema.
    ///
    /// # Errors
    ///
    /// Returns connection, capacity or schema-validation errors.
    pub async fn connect(url: &str) -> Result<Self, PostgresError> {
        let pool = PgPoolOptions::new()
            .connect(url)
            .await
            .map_err(PostgresError::Sqlx)?;
        Self::from_pool(pool).await
    }

    /// Build from an existing pool without changing the schema.
    ///
    /// Prefer this when the caller already holds a `PgPool` (e.g. a shared
    /// pool across multiple stores in an application). The pool must accommodate
    /// the dedicated listener alongside data queries.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresError::PoolCapacity`] for an undersized pool, or
    /// a database or schema-validation error if the installed schema is unusable.
    pub async fn from_pool(pool: PgPool) -> Result<Self, PostgresError> {
        Self::from_pool_with_limits(pool, Limits::default()).await
    }

    /// Build from a shared pool with explicit read/write resource limits.
    ///
    /// # Errors
    /// Returns [`PostgresError::PoolCapacity`] for an undersized pool, or
    /// a database or schema-validation error if the installed schema is unusable.
    #[cfg_attr(
        feature = "tracing",
        tracing::instrument(name = "mnesis.postgres.open", level = "debug", skip_all)
    )]
    pub async fn from_pool_with_limits(
        pool: PgPool,
        limits: Limits,
    ) -> Result<Self, PostgresError> {
        let capacity = pool.options().get_max_connections();
        if capacity < 2 {
            return Err(PostgresError::PoolCapacity { actual: capacity });
        }
        let history = schema::validate_pool(&pool).await?;
        Ok(Self::assemble(pool, limits, history))
    }
}
