//! Check the native durability profile without weakening stronger commit modes.

use mnesis::ErrorId;
use sqlx::{PgConnection, Postgres, Transaction};

use crate::PostgresError;

pub async fn validate(connection: &mut PgConnection) -> Result<(), PostgresError> {
    let setting: Option<String> = sqlx::query_scalar(
        "SELECT CASE \
         WHEN pg_catalog.current_setting('fsync') OPERATOR(pg_catalog.=) 'off' THEN 'fsync' \
         WHEN pg_catalog.current_setting('full_page_writes') OPERATOR(pg_catalog.=) 'off' THEN 'full_page_writes' \
         WHEN pg_catalog.current_setting('synchronous_commit') OPERATOR(pg_catalog.=) 'off' THEN 'synchronous_commit' \
         END",
    )
    .fetch_one(connection)
    .await
    .map_err(PostgresError::Sqlx)?;
    if let Some(unsafe_setting) = setting {
        return Err(PostgresError::Durability {
            setting: ErrorId::from_display(&unsafe_setting),
        });
    }
    Ok(())
}

#[cfg_attr(
    feature = "tracing",
    tracing::instrument(name = "mnesis.postgres.commit", level = "debug", skip_all)
)]
pub async fn commit(mut transaction: Transaction<'_, Postgres>) -> Result<(), PostgresError> {
    // Deferred triggers can still change settings at COMMIT. Run them first.
    sqlx::query("SET CONSTRAINTS ALL IMMEDIATE")
        .execute(&mut *transaction)
        .await
        .map_err(PostgresError::Sqlx)?;
    // Check after all writes and triggers that can change session settings.
    // A failed check drops this uncommitted transaction and schedules rollback.
    validate(&mut transaction).await?;
    transaction.commit().await.map_err(PostgresError::commit)?;
    #[cfg(feature = "tracing")]
    tracing::debug!(name: "mnesis.postgres.commit", outcome = "committed");
    Ok(())
}
