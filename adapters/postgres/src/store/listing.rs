//! Finite event-only enumeration without retaining a database lease between pages.

use std::collections::VecDeque;

use futures::StreamExt;
use futures::stream::{BoxStream, try_unfold};
use mnesis_store::StreamKey;
use mnesis_store::export::StreamLister;
use sqlx::PgPool;

use super::PostgresStore;
use crate::page::PageRow;
use crate::{Limits, PostgresError};

#[derive(sqlx::FromRow)]
struct Key {
    number: i64,
    value: Vec<u8>,
}

async fn page(
    pool: &PgPool,
    after: Option<i64>,
    snapshot: &str,
    limits: Limits,
) -> Result<VecDeque<PageRow<Key>>, PostgresError> {
    sqlx::query_as(
        "WITH candidates AS ( \
             SELECT k.number, 32 + octet_length(k.value)::bigint AS row_bytes \
             FROM mnesis.keys k WHERE ($1::bigint IS NULL OR k.number > $1) \
             AND EXISTS (SELECT FROM mnesis.events e WHERE e.stream_id = k.number \
                 AND pg_catalog.pg_visible_in_snapshot(e.txid, $2::text::pg_catalog.pg_snapshot)) \
             ORDER BY k.number LIMIT $3::text::bigint \
         ), ranked AS ( \
             SELECT *, ROW_NUMBER() OVER w AS ordinal, SUM(row_bytes) OVER w AS total_bytes \
             FROM candidates WINDOW w AS (ORDER BY number ROWS UNBOUNDED PRECEDING) \
         ), settings AS (SELECT $4::bigint AS byte_limit) \
         SELECT ranked.number, row_bytes, byte_limit, \
             CASE WHEN row_bytes <= byte_limit THEN k.value END AS value \
         FROM ranked JOIN mnesis.keys k USING (number) CROSS JOIN settings \
         WHERE total_bytes <= byte_limit OR ordinal = 1 ORDER BY ranked.number",
    )
    .bind(after)
    .bind(snapshot)
    .bind(limits.rows.get().to_string())
    .bind(i64::from(limits.bytes.get()))
    .fetch_all(pool)
    .await
    .map(VecDeque::from)
    .map_err(PostgresError::Sqlx)
}

impl StreamLister for PostgresStore {
    type StreamList = BoxStream<'static, Result<StreamKey, PostgresError>>;

    async fn list_streams(&self) -> Result<Self::StreamList, PostgresError> {
        // Frozen visibility includes committed writers beyond an older active
        // transaction. Immutable event rows cannot later enter this snapshot.
        let captured: String = sqlx::query_scalar("SELECT pg_catalog.pg_current_snapshot()::text")
            .fetch_one(self.pool())
            .await
            .map_err(PostgresError::Sqlx)?;
        let limits = self.inner.limits;
        let state = (self.pool().clone(), None, VecDeque::new(), captured);
        Ok(
            try_unfold(state, move |(pool, after, mut rows, snapshot)| async move {
                if rows.is_empty() {
                    rows = page(&pool, after, &snapshot, limits).await?;
                }
                let Some(buffered) = rows.pop_front() else {
                    return Ok(None);
                };
                let key = buffered.into_value()?;
                let number = key.number;
                Ok(Some((
                    StreamKey::from_bytes(key.value),
                    (pool, Some(number), rows, snapshot),
                )))
            })
            .boxed(),
        )
    }
}
