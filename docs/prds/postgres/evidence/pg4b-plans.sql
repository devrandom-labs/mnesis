\set ON_ERROR_STOP on
TRUNCATE mnesis.events, mnesis.keys RESTART IDENTITY;
INSERT INTO mnesis.keys(value) VALUES ('byte-plans'::bytea);
INSERT INTO mnesis.events (stream_id, version, event_type, schema_version, payload)
SELECT (SELECT number FROM mnesis.keys WHERE value = 'byte-plans'::bytea), n, 'Plan', 1, decode(repeat('42', 4096), 'hex') FROM generate_series(1, 40960) n;
ANALYZE mnesis.events;
SELECT min(txid)::text AS txn FROM mnesis.events \gset
SET plan_cache_mode = force_generic_plan;
PREPARE byte_page(text, text, bigint, text, bigint) AS
WITH candidates AS (
 SELECT global_seq, version, txid,
 32 + octet_length(keys.value)::bigint + octet_length(event_type)::bigint +
 octet_length(payload)::bigint + COALESCE(octet_length(metadata)::bigint, 0) AS row_bytes
 FROM mnesis.events JOIN mnesis.keys ON keys.number = mnesis.events.stream_id WHERE txid < $1::text::xid8 AND (txid, global_seq) > ($2::text::xid8, $3)
 ORDER BY txid, global_seq LIMIT $4::text::bigint
), ranked AS (
 SELECT *, ROW_NUMBER() OVER w AS ordinal, SUM(row_bytes) OVER w AS total_bytes
 FROM candidates WINDOW w AS (ORDER BY txid, global_seq ROWS UNBOUNDED PRECEDING)
), settings AS (SELECT $5 AS byte_limit)
SELECT row_bytes, byte_limit, mnesis.events.version::text AS version, mnesis.events.schema_version, mnesis.events.global_seq,
 mnesis.events.txid::text AS txid,
 CASE WHEN row_bytes <= byte_limit THEN keys.value END AS stream_id,
 CASE WHEN row_bytes <= byte_limit THEN mnesis.events.event_type END AS event_type,
 CASE WHEN row_bytes <= byte_limit THEN mnesis.events.payload END AS payload,
 CASE WHEN row_bytes <= byte_limit THEN mnesis.events.metadata END AS metadata
FROM ranked JOIN mnesis.events USING(global_seq) JOIN mnesis.keys ON keys.number = mnesis.events.stream_id CROSS JOIN settings
WHERE total_bytes <= byte_limit OR ordinal = 1 ORDER BY mnesis.events.txid, mnesis.events.global_seq;
EXPLAIN (ANALYZE, BUFFERS, TIMING OFF) EXECUTE byte_page('18446744073709551615', :'txn', 0, '256', 16777216);
EXPLAIN (ANALYZE, BUFFERS, TIMING OFF) EXECUTE byte_page('18446744073709551615', :'txn', 40000, '256', 16777216);
DEALLOCATE byte_page;
ANALYZE mnesis.keys;
PREPARE stream_head(bytea) AS SELECT number, (SELECT MAX(version)::text FROM mnesis.events WHERE stream_id = number) FROM mnesis.keys WHERE value = $1;
EXPLAIN (ANALYZE, BUFFERS, TIMING OFF) EXECUTE stream_head('byte-plans'::bytea);
DEALLOCATE stream_head;
PREPARE stream_page(bigint, text, text, text, text, bigint) AS
WITH candidates AS (
 SELECT global_seq, version, txid,
 32 + octet_length(keys.value)::bigint + octet_length(event_type)::bigint +
 octet_length(payload)::bigint + COALESCE(octet_length(metadata)::bigint, 0) AS row_bytes
 FROM mnesis.events JOIN mnesis.keys ON keys.number = mnesis.events.stream_id WHERE stream_id = $1 AND version >= $2::text::numeric AND version <= $3::text::numeric AND version > $4::text::numeric
 ORDER BY version LIMIT $5::text::bigint
), ranked AS (
 SELECT *, ROW_NUMBER() OVER w AS ordinal, SUM(row_bytes) OVER w AS total_bytes
 FROM candidates WINDOW w AS (ORDER BY version ROWS UNBOUNDED PRECEDING)
), settings AS (SELECT $6 AS byte_limit)
SELECT row_bytes, byte_limit, mnesis.events.version::text AS version, mnesis.events.schema_version, mnesis.events.global_seq,
 mnesis.events.txid::text AS txid,
 CASE WHEN row_bytes <= byte_limit THEN keys.value END AS stream_id,
 CASE WHEN row_bytes <= byte_limit THEN mnesis.events.event_type END AS event_type,
 CASE WHEN row_bytes <= byte_limit THEN mnesis.events.payload END AS payload,
 CASE WHEN row_bytes <= byte_limit THEN mnesis.events.metadata END AS metadata
FROM ranked JOIN mnesis.events USING(global_seq) JOIN mnesis.keys ON keys.number = mnesis.events.stream_id CROSS JOIN settings
WHERE total_bytes <= byte_limit OR ordinal = 1 ORDER BY mnesis.events.version;
EXPLAIN (ANALYZE, BUFFERS, TIMING OFF) EXECUTE stream_page(1, '1', '40960', '0', '256', 16777216);
EXPLAIN (ANALYZE, BUFFERS, TIMING OFF) EXECUTE stream_page(1, '1', '40960', '40000', '256', 16777216);
DEALLOCATE stream_page;
