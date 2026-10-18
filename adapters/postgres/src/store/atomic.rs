//! All heads precede writes so a successor never receives an earlier transaction ID.

use mnesis_store::import::{
    AtomicAppend, AtomicAppendError, PlannedAppend, validate_atomic_runs, validate_distinct_targets,
};
use sqlx::{PgConnection, Postgres, Transaction};

use super::{
    PgAllPos, PostgresError, PostgresStore, create_key, head_version, insert_run,
    is_stream_conflict, read_head, validate_sizes,
};

async fn prepare(
    connection: &mut PgConnection,
    index: usize,
    write: &PlannedAppend,
) -> Result<Option<i64>, AtomicAppendError<PostgresError>> {
    let head = read_head(connection, &write.target)
        .await
        .map_err(AtomicAppendError::Store)?;
    let actual = head_version(head.as_ref(), &write.target).map_err(AtomicAppendError::Store)?;
    if actual != write.expected_version {
        return Err(AtomicAppendError::Conflict { index, actual });
    }
    Ok(head.map(|value| value.0))
}

async fn insert(
    connection: &mut Transaction<'_, Postgres>,
    write: &PlannedAppend,
    key: Option<i64>,
) -> Result<PgAllPos, PostgresError> {
    let number = match key {
        Some(number) => number,
        None => create_key(connection, &write.target).await?,
    };
    insert_run(connection, &write.target, number, write.batch()).await
}

impl PostgresStore {
    /// Preserve the real collision source if rollback/head observation is unavailable.
    async fn rejected(
        &self,
        tx: Transaction<'_, Postgres>,
        index: usize,
        write: &PlannedAppend,
        error: PostgresError,
    ) -> AtomicAppendError<PostgresError> {
        if matches!(&error, PostgresError::Sqlx(source) if is_stream_conflict(source))
            && tx.rollback().await.is_ok()
        {
            let observed = async {
                let mut connection = self.pool().acquire().await.map_err(PostgresError::Sqlx)?;
                let head = read_head(&mut connection, &write.target).await?;
                head_version(head.as_ref(), &write.target)
            }
            .await;
            if let Ok(Some(actual)) = observed {
                return AtomicAppendError::Conflict {
                    index,
                    actual: Some(actual),
                };
            }
        }
        AtomicAppendError::Store(error)
    }
}

impl AtomicAppend for PostgresStore {
    async fn atomic_append_many(
        &self,
        writes: &[PlannedAppend],
    ) -> Result<Option<PgAllPos>, AtomicAppendError<PostgresError>> {
        validate_distinct_targets(writes.iter().map(|write| &write.target))?;
        validate_atomic_runs(writes)?;
        for write in writes {
            validate_sizes(&write.target, write.batch(), self.inner.limits)
                .map_err(AtomicAppendError::Store)?;
        }
        let Some((first, tail)) = writes.split_first() else {
            return Ok(None);
        };
        let mut tx = self
            .pool()
            .begin()
            .await
            .map_err(PostgresError::Sqlx)
            .map_err(AtomicAppendError::Store)?;
        let first_key = prepare(&mut tx, 0, first).await?;
        let mut prepared = Vec::with_capacity(tail.len());
        for (index, write) in (1..).zip(tail) {
            let key = prepare(&mut tx, index, write).await?;
            prepared.push((index, write, key));
        }
        // No key insert or event default assigned xid8 before every head passed.
        let staged = async {
            let mut assigned = insert(&mut tx, first, first_key)
                .await
                .map_err(|error| (0, first, error))?;
            for (index, write, key) in prepared {
                assigned = insert(&mut tx, write, key)
                    .await
                    .map_err(|error| (index, write, error))?;
            }
            Ok::<_, (usize, &PlannedAppend, PostgresError)>(assigned)
        }
        .await;
        let assigned = match staged {
            Ok(position) => position,
            Err((index, write, error)) => {
                return Err(self.rejected(tx, index, write, error).await);
            }
        };
        tx.commit()
            .await
            .map_err(PostgresError::commit)
            .map_err(AtomicAppendError::Store)?;
        for write in writes {
            self.wake_registry().wake(write.target.as_bytes());
        }
        let hints = async {
            for write in writes {
                self.publish_hint(write.target.as_bytes()).await;
            }
        };
        let _ = tokio::time::timeout(super::HINT_TIMEOUT, hints).await;
        Ok(Some(assigned))
    }
}
