//! Conditional projection state; aggregate caches cannot write this table.

use std::num::{NonZeroU32, NonZeroU64};

use mnesis::{ErrorId, Id};
use mnesis_store::checkpoint::{
    CheckpointError, CheckpointHydrated, CheckpointStore, CheckpointWrite,
};
use sqlx::PgConnection;

use crate::{History, durability};

use super::{
    PgAllPos, PostgresError, PostgresStore, corrupt, corrupt_value, create_key, read_key,
    validate_bytes,
};

#[derive(sqlx::FromRow)]
struct Metadata {
    revision: String,
    schema_version: i64,
    txid: String,
    global_seq: String,
    history: [u8; 16],
}

impl Metadata {
    fn validate(
        &self,
        id: &impl Id,
        history: History,
    ) -> Result<(NonZeroU64, NonZeroU32, PgAllPos), PostgresError> {
        history.check(History::new(self.history))?;
        let label = ErrorId::from_display(id);
        let number = |value: &str| {
            value
                .parse()
                .map_err(|source| corrupt_value(label, &source))
        };
        let revision = NonZeroU64::new(number(&self.revision)?)
            .ok_or_else(|| corrupt(label, "zero checkpoint revision"))?;
        let schema = NonZeroU32::new(
            u32::try_from(self.schema_version).map_err(|source| corrupt_value(label, &source))?,
        )
        .ok_or_else(|| corrupt(label, "zero checkpoint schema"))?;
        let position = PgAllPos::new(history, number(&self.txid)?, number(&self.global_seq)?);
        Ok((revision, schema, position))
    }
}

#[derive(sqlx::FromRow)]
struct Row {
    #[sqlx(flatten)]
    metadata: Metadata,
    size: i64,
    state: Option<Vec<u8>>,
}

impl Row {
    fn hydrate(
        self,
        id: &impl Id,
        schema: NonZeroU32,
        limit: i64,
        history: History,
    ) -> Result<CheckpointHydrated<Vec<u8>, PgAllPos>, PostgresError> {
        let (revision, stored_schema, position) = self.metadata.validate(id, history)?;
        if stored_schema != schema {
            return Ok(CheckpointHydrated::Stale {
                revision,
                stored_schema,
            });
        }
        if self.size > limit {
            return Err(PostgresError::RowLimit {
                actual: self.size,
                limit,
            });
        }
        let state = self.state.ok_or_else(|| {
            corrupt(
                ErrorId::from_display(id),
                "matching checkpoint body is absent",
            )
        })?;
        Ok(CheckpointHydrated::Found {
            revision,
            position,
            state,
        })
    }
}

async fn lock(
    connection: &mut PgConnection,
    key: i64,
    id: &impl Id,
    history: History,
) -> Result<Option<(NonZeroU64, NonZeroU32, PgAllPos)>, PostgresError> {
    sqlx::query_as::<_, Metadata>(
        "SELECT revision::text AS revision, schema_version, txid::text AS txid, global_seq::text AS global_seq, history \
         FROM mnesis.checkpoints WHERE key = $1 FOR UPDATE",
    )
    .bind(key)
    .fetch_optional(connection)
    .await
    .map_err(PostgresError::Sqlx)?
    .map(|value| value.validate(id, history))
    .transpose()
}

const fn storage(source: sqlx::Error) -> CheckpointError<PostgresError> {
    CheckpointError::Store(PostgresError::Sqlx(source))
}

impl CheckpointStore<Vec<u8>, PgAllPos> for PostgresStore {
    type Error = PostgresError;

    async fn hydrate_checkpoint(
        &self,
        id: &impl Id,
        schema_version: NonZeroU32,
    ) -> Result<CheckpointHydrated<Vec<u8>, PgAllPos>, PostgresError> {
        let limit = i64::from(self.inner.limits.bytes.get());
        let row = sqlx::query_as::<_, Row>(
            "SELECT c.revision::text AS revision, c.schema_version, c.txid::text AS txid, c.global_seq::text AS global_seq, c.history, \
             octet_length(k.value)::bigint + octet_length(c.state)::bigint + 48 AS size, \
             CASE WHEN c.schema_version = $2 AND c.history = $4 AND \
                 octet_length(k.value)::bigint + octet_length(c.state)::bigint + 48 <= $3 \
                 THEN c.state END AS state \
             FROM mnesis.checkpoints c JOIN mnesis.keys k ON k.number = c.key \
             WHERE k.value OPERATOR(pg_catalog.=) $1",
        )
        .bind(id.as_ref()).bind(i64::from(schema_version.get())).bind(limit).bind(self.history().as_bytes())
        .fetch_optional(self.pool()).await.map_err(PostgresError::Sqlx)?;
        row.map_or_else(
            || Ok(CheckpointHydrated::Absent),
            |value| value.hydrate(id, schema_version, limit, self.history()),
        )
    }

    async fn commit_checkpoint(
        &self,
        id: &impl Id,
        write: CheckpointWrite<'_, Vec<u8>, PgAllPos>,
    ) -> Result<NonZeroU64, CheckpointError<PostgresError>> {
        self.history()
            .check(write.position.history())
            .map_err(CheckpointError::Store)?;
        validate_bytes(
            &[id.as_ref().len(), write.state.len(), 16],
            self.inner.limits,
        )
        .map_err(CheckpointError::Store)?;
        let mut tx = self.pool().begin().await.map_err(storage)?;
        let key = match read_key(&mut tx, id)
            .await
            .map_err(CheckpointError::Store)?
        {
            Some(number) => number,
            None => create_key(&mut tx, id)
                .await
                .map_err(CheckpointError::Store)?,
        };
        let current = lock(&mut tx, key, id, self.history())
            .await
            .map_err(CheckpointError::Store)?;
        let revision = write.next_revision(current)?;
        let statement = match current {
            Some(_) => {
                "UPDATE mnesis.checkpoints SET revision = $2::text::numeric, \
                schema_version = $3, txid = $4::text::numeric, global_seq = $5::text::numeric, \
                state = $6, history = $7 WHERE key = $1 RETURNING key"
            }
            None => {
                "INSERT INTO mnesis.checkpoints (key, revision, schema_version, txid, global_seq, state, history) \
                VALUES ($1, $2::text::numeric, $3, $4::text::numeric, $5::text::numeric, $6, $7) \
                ON CONFLICT (key) DO NOTHING RETURNING key"
            }
        };
        let saved = sqlx::query(statement)
            .bind(key)
            .bind(revision.get().to_string())
            .bind(i64::from(write.schema_version.get()))
            .bind(write.position.txid().to_string())
            .bind(write.position.seq().to_string())
            .bind(write.state)
            .bind(write.position.history().as_bytes())
            .fetch_optional(&mut *tx)
            .await
            .map_err(storage)?;
        if saved.is_none() {
            // A lost creation observes the winner. Suppression with no winner
            // must never acknowledge a write, even when revision policy permits it.
            write.next_revision(
                lock(&mut tx, key, id, self.history())
                    .await
                    .map_err(CheckpointError::Store)?,
            )?;
            return Err(storage(sqlx::Error::RowNotFound));
        }
        durability::commit(tx)
            .await
            .map_err(CheckpointError::Store)?;
        Ok(revision)
    }
}
