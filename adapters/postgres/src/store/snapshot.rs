//! Unconditional aggregate cache replacement, separate from projection revision CAS.

use std::num::NonZeroU32;

use mnesis::{ErrorId, Id, Version};
use mnesis_store::state::{Hydrated, SnapshotStore};

use crate::durability;

use super::{
    PostgresError, PostgresStore, corrupt, corrupt_value, create_key, read_key, validate_bytes,
};

#[derive(sqlx::FromRow)]
struct Row {
    schema_version: i64,
    version: String,
    size: i64,
    state: Option<Vec<u8>>,
}

impl Row {
    fn hydrate(
        self,
        id: &impl Id,
        schema: NonZeroU32,
        limit: i64,
    ) -> Result<Hydrated<Vec<u8>, Version>, PostgresError> {
        let label = ErrorId::from_display(id);
        let stored_schema = NonZeroU32::new(
            u32::try_from(self.schema_version).map_err(|source| corrupt_value(label, &source))?,
        )
        .ok_or_else(|| corrupt(label, "zero snapshot schema"))?;
        let raw = self
            .version
            .parse()
            .map_err(|source| corrupt_value(label, &source))?;
        let position = Version::new(raw).ok_or_else(|| corrupt(label, "zero snapshot version"))?;
        if stored_schema != schema {
            return Ok(Hydrated::Stale { stored_schema });
        }
        if self.size > limit {
            return Err(PostgresError::RowLimit {
                actual: self.size,
                limit,
            });
        }
        let state = self
            .state
            .ok_or_else(|| corrupt(label, "matching snapshot body is absent"))?;
        Ok(Hydrated::Found { position, state })
    }
}

impl SnapshotStore<Vec<u8>, Version> for PostgresStore {
    type Error = PostgresError;

    async fn hydrate(
        &self,
        id: &impl Id,
        schema_version: NonZeroU32,
    ) -> Result<Hydrated<Vec<u8>, Version>, PostgresError> {
        let limit = i64::from(self.inner.limits.bytes.get());
        let row = sqlx::query_as::<_, Row>(
            "SELECT s.schema_version, s.version::text AS version, \
             pg_catalog.octet_length(k.value)::bigint + pg_catalog.octet_length(s.state)::bigint + 32 AS size, \
             CASE WHEN s.schema_version = $2 AND \
                 pg_catalog.octet_length(k.value)::bigint + pg_catalog.octet_length(s.state)::bigint + 32 <= $3 \
                 THEN s.state END AS state \
             FROM mnesis.snapshots s JOIN mnesis.keys k ON k.number = s.key \
             WHERE k.value OPERATOR(pg_catalog.=) $1",
        )
        .bind(id.as_ref())
        .bind(i64::from(schema_version.get()))
        .bind(limit)
        .fetch_optional(self.pool())
        .await
        .map_err(PostgresError::Sqlx)?;
        row.map_or_else(
            || Ok(Hydrated::Absent),
            |value| value.hydrate(id, schema_version, limit),
        )
    }

    async fn commit(
        &self,
        id: &impl Id,
        schema_version: NonZeroU32,
        position: Version,
        state: &Vec<u8>,
    ) -> Result<(), PostgresError> {
        validate_bytes(&[id.as_ref().len(), state.len()], self.inner.limits)?;
        let mut tx = self.pool().begin().await.map_err(PostgresError::Sqlx)?;
        let key = match read_key(&mut tx, id).await? {
            Some(number) => number,
            None => create_key(&mut tx, id).await?,
        };
        sqlx::query(
            "INSERT INTO mnesis.snapshots (key, schema_version, version, state) \
             VALUES ($1, $2, $3::text::numeric, $4) ON CONFLICT (key) DO UPDATE \
             SET schema_version = EXCLUDED.schema_version, version = EXCLUDED.version, state = EXCLUDED.state RETURNING key",
        ).bind(key).bind(i64::from(schema_version.get())).bind(position.as_u64().to_string()).bind(state)
            .fetch_one(&mut *tx).await.map_err(PostgresError::Sqlx)?;
        durability::commit(tx).await
    }
}
