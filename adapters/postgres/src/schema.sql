-- Read-only validation, also used before adopting an unversioned legacy table.
WITH expected(position, name, kind, required, identity) AS (
    VALUES (1, 'global_seq', 'int8', true, 'a'),
           (2, 'stream_id', 'bytea', true, ''),
           (3, 'version', 'int8', true, ''),
           (4, 'txid', 'xid8', true, ''),
           (5, 'event_type', 'text', true, ''),
           (6, 'schema_version', 'int8', true, ''),
           (7, 'payload', 'bytea', true, ''),
           (8, 'metadata', 'bytea', false, '')
), relation AS (SELECT pg_catalog.to_regclass($1) AS id)
SELECT
    (SELECT count(*) FROM pg_catalog.pg_attribute, relation
     WHERE attrelid = id AND attnum > 0 AND NOT attisdropped) = 8
    AND (SELECT count(*) FROM expected JOIN pg_catalog.pg_attribute a
         ON a.attnum = expected.position AND a.attname = expected.name
         AND a.atttypid = pg_catalog.to_regtype('pg_catalog.' || expected.kind)
         AND a.attnotnull = expected.required AND a.attidentity::text = expected.identity,
         relation WHERE a.attrelid = id) = 8
    AND EXISTS (SELECT FROM pg_catalog.pg_class, relation
                WHERE oid = id AND relkind = 'r' AND NOT relrowsecurity)
    AND EXISTS (SELECT FROM pg_catalog.pg_constraint, relation
                WHERE conrelid = id AND conname = 'events_pkey'
                AND contype = 'p' AND conkey = ARRAY[1]::smallint[]
                AND convalidated AND NOT condeferrable)
    AND EXISTS (SELECT FROM pg_catalog.pg_constraint, relation
                WHERE conrelid = id AND conname = 'events_stream_id_version_key'
                AND contype = 'u' AND conkey = ARRAY[2, 3]::smallint[]
                AND convalidated AND NOT condeferrable)
    AND EXISTS (SELECT FROM pg_catalog.pg_attrdef, relation
                WHERE adrelid = id AND adnum = 4
                AND pg_catalog.pg_get_expr(adbin, adrelid) = 'pg_current_xact_id()')
    AND EXISTS (SELECT FROM pg_catalog.pg_index i
                JOIN pg_catalog.pg_class c ON c.oid = i.indexrelid
                JOIN pg_catalog.pg_am a ON a.oid = c.relam, relation
                WHERE i.indrelid = id AND a.amname = 'btree'
                AND i.indkey::text = '4 1' AND i.indisvalid AND i.indisready
                AND i.indpred IS NULL AND i.indexprs IS NULL)
