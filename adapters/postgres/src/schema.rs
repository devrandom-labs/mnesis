use std::sync::LazyLock;

use mnesis::ErrorId;
use sqlx::migrate::Migrator;
use sqlx::{Connection, PgConnection, PgPool, Postgres, QueryBuilder};

use crate::error::PostgresError;
use crate::{History, durability, history};

static MIGRATIONS: LazyLock<Migrator> = LazyLock::new(|| {
    let mut migrator = sqlx::migrate!("./migrations");
    migrator.dangerous_set_table_name("mnesis._sqlx_migrations");
    migrator.create_schema("mnesis");
    migrator
});
const SHAPE: &str = include_str!("schema.sql");

/// Run explicit, checked-in migrations with a scoped administrative connection.
pub async fn migrate(pool: &PgPool) -> Result<(), PostgresError> {
    let mut connection = pool.acquire().await.map_err(PostgresError::Sqlx)?;
    // SQLx can return early before unlocking on errors. Never return a session
    // lock to the caller's shared pool, even on cancellation.
    connection.close_on_drop();
    durability::validate(&mut connection).await?;
    // SQLx serializes schema creation, validation and each migration transaction.
    MIGRATIONS
        .run_direct(None, &mut *connection, false)
        .await
        .map_err(PostgresError::Migration)?;
    validate(&mut connection).await.map(|_| ())
}

pub async fn validate_pool(pool: &PgPool) -> Result<History, PostgresError> {
    let mut connection = pool.acquire().await.map_err(PostgresError::Sqlx)?;
    connection.close_on_drop();
    validate(&mut connection).await
}

async fn validate(connection: &mut PgConnection) -> Result<History, PostgresError> {
    durability::validate(connection).await?;
    let mut view = connection
        .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .await
        .map_err(PostgresError::Sqlx)?;
    let history = validate_current(&mut view).await?;
    view.rollback().await.map_err(PostgresError::Sqlx)?;
    Ok(history)
}

async fn validate_current(connection: &mut PgConnection) -> Result<History, PostgresError> {
    let ready: bool =
        sqlx::query_scalar("SELECT pg_catalog.to_regclass('mnesis._sqlx_migrations') IS NOT NULL")
            .fetch_one(&mut *connection)
            .await
            .map_err(PostgresError::Sqlx)?;
    if !ready {
        return Err(incompatible(
            "schema is not migrated; run PostgresStore::migrate with the migration role",
        ));
    }
    // One extra byte/row preserves rejection of overlong digests and unknown versions.
    let checksum = MIGRATIONS
        .iter()
        .map(|migration| migration.checksum.len())
        .max()
        .and_then(|length| length.checked_add(1))
        .ok_or_else(|| incompatible("embedded migration checksums have no finite bound"))?;
    let rows = MIGRATIONS
        .iter()
        .count()
        .checked_add(1)
        .ok_or_else(|| incompatible("embedded migration count has no finite bound"))?;
    let mut query =
        QueryBuilder::<Postgres>::new("SELECT version, success, pg_catalog.substr(checksum, 1, ");
    query
        .push(checksum)
        .push(") FROM mnesis._sqlx_migrations ORDER BY version LIMIT ")
        .push(rows);
    let applied: Vec<(i64, bool, Vec<u8>)> = query
        .build_query_as()
        .fetch_all(&mut *connection)
        .await
        .map_err(PostgresError::Sqlx)?;
    if applied.len() != MIGRATIONS.iter().count()
        || !applied
            .iter()
            .zip(MIGRATIONS.iter())
            .all(|(actual, known)| {
                actual.0 == known.version && actual.1 && actual.2 == known.checksum.as_ref()
            })
    {
        return Err(incompatible(
            "migration version, checksum or completion differs; use a compatible binary or repair the upgrade",
        ));
    }
    validate_shape(connection, "mnesis.events").await?;
    let snapshots: bool = sqlx::query_scalar(include_str!("snapshot.sql"))
        .fetch_one(&mut *connection)
        .await
        .map_err(PostgresError::Sqlx)?;
    if !snapshots {
        return Err(incompatible(
            "snapshot columns, bounds, logging, primary key or identity reference differ",
        ));
    }
    let checkpoints: bool = sqlx::query_scalar(include_str!("checkpoint.sql"))
        .fetch_one(&mut *connection)
        .await
        .map_err(PostgresError::Sqlx)?;
    if !checkpoints {
        return Err(incompatible(
            "checkpoint columns, bounds, logging, primary key or identity reference differ",
        ));
    }
    history::load(connection).await
}

async fn validate_shape(connection: &mut PgConnection, table: &str) -> Result<(), PostgresError> {
    let valid: bool = sqlx::query_scalar(SHAPE)
        .bind(table)
        .fetch_one(connection)
        .await
        .map_err(PostgresError::Sqlx)?;
    if !valid {
        return Err(incompatible(
            "events columns, defaults, constraints or watermark index differ from the required schema",
        ));
    }
    Ok(())
}

fn incompatible(reason: &str) -> PostgresError {
    PostgresError::Schema {
        reason: ErrorId::from_display(&reason),
    }
}
