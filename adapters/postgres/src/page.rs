//! One-statement header selection bounds both rows and bytes before transfer.

use sqlx::postgres::PgRow;
use sqlx::{FromRow, Postgres, QueryBuilder, Row};

use crate::{Limits, PostgresError};

// Scalar accounting: version, schema version, transaction and sequence (8 each).
// Widen lengths before addition; metadata NULL and empty remain distinct bodies.
pub const START: &str = "WITH candidates AS (SELECT global_seq, version, txid, \
    32 + octet_length(stream_id)::bigint + octet_length(event_type)::bigint + \
    octet_length(payload)::bigint + COALESCE(octet_length(metadata)::bigint, 0) \
    AS row_bytes FROM mnesis.events WHERE ";

#[derive(Clone, Copy)]
pub enum Order {
    Stream,
    All,
}

pub fn finish(query: &mut QueryBuilder<Postgres>, order: Order, limits: Limits) {
    let ordering = match order {
        Order::Stream => "version",
        Order::All => "txid, global_seq",
    };
    query
        .push(" ORDER BY ")
        .push(ordering)
        .push(" LIMIT ")
        .push_bind(limits.rows.get().to_string())
        .push(
            "::text::bigint), ranked AS (SELECT *, ROW_NUMBER() OVER w AS ordinal, \
               SUM(row_bytes) OVER w AS total_bytes FROM candidates WINDOW w AS (ORDER BY ",
        )
        .push(ordering)
        .push(" ROWS UNBOUNDED PRECEDING)), settings AS (SELECT ")
        .push_bind(i64::from(limits.bytes.get()))
        .push(
            " AS byte_limit) SELECT row_bytes, byte_limit, events.version, \
               events.schema_version, events.global_seq, events.txid::text::bigint AS txid, \
               CASE WHEN row_bytes <= byte_limit THEN events.stream_id END AS stream_id, \
               CASE WHEN row_bytes <= byte_limit THEN events.event_type END AS event_type, \
               CASE WHEN row_bytes <= byte_limit THEN events.payload END AS payload, \
               CASE WHEN row_bytes <= byte_limit THEN events.metadata END AS metadata \
               FROM ranked JOIN mnesis.events USING (global_seq) CROSS JOIN settings \
               WHERE total_bytes <= byte_limit OR ordinal = 1 ORDER BY ",
        );
    query.push(match order {
        Order::Stream => "events.version",
        Order::All => "events.txid, events.global_seq",
    });
}

/// Reject an oversized first row before `SQLx` decodes any NULL body fields.
pub enum PageRow<T> {
    Value(T),
    Oversized { actual: i64, limit: i64 },
}

impl<'row, T: FromRow<'row, PgRow>> FromRow<'row, PgRow> for PageRow<T> {
    fn from_row(row: &'row PgRow) -> Result<Self, sqlx::Error> {
        let actual = row.try_get("row_bytes")?;
        let limit = row.try_get("byte_limit")?;
        if actual > limit {
            Ok(Self::Oversized { actual, limit })
        } else {
            T::from_row(row).map(Self::Value)
        }
    }
}

impl<T> PageRow<T> {
    pub fn into_value(self) -> Result<T, PostgresError> {
        match self {
            Self::Value(row) => Ok(row),
            Self::Oversized { actual, limit } => Err(PostgresError::RowLimit { actual, limit }),
        }
    }
}
