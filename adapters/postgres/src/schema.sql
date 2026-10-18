-- Read-only current schema validation; migration1 retains the legacy contract.
WITH expected(position, name, kind, required, identity) AS (
    VALUES (1, 'global_seq', 'int8', true, 'a'),
           (9, 'stream_id', 'int8', true, ''),
           (3, 'version', 'numeric', true, ''),
           (4, 'txid', 'xid8', true, ''),
           (5, 'event_type', 'bytea', true, ''),
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
                AND contype = 'u' AND conkey = ARRAY[9, 3]::smallint[]
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
    AND EXISTS (SELECT FROM pg_catalog.pg_attribute, relation
                WHERE attrelid = id AND attname = 'version' AND atttypmod = 1310724)
    AND EXISTS (SELECT FROM pg_catalog.pg_constraint, relation
                WHERE conrelid = id AND conname = 'events_stream_id_fkey'
                AND contype = 'f' AND conkey = ARRAY[9]::smallint[]
                AND confrelid = pg_catalog.to_regclass('mnesis.keys')
                AND confkey = ARRAY[1]::smallint[] AND convalidated AND NOT condeferrable
                AND confupdtype = 'a' AND confdeltype = 'a')
    AND (SELECT count(*) FROM pg_catalog.pg_constraint c, relation,
         (VALUES
             ('events_global_seq_check', 'CHECK ((global_seq > 0))'),
             ('events_schema_version_check', 'CHECK (((schema_version > 0) AND (schema_version <= ''4294967295''::bigint)))'),
             ('events_version_check', 'CHECK (((version > (0)::numeric) AND (version <= ''18446744073709551615''::numeric)))')
         ) AS checks(name, definition)
         WHERE c.conrelid = id AND c.conname = checks.name AND c.contype = 'c'
         AND c.convalidated AND pg_catalog.pg_get_constraintdef(c.oid) = checks.definition) = 3
    AND (SELECT count(*) FROM pg_catalog.pg_attribute
         WHERE attrelid = pg_catalog.to_regclass('mnesis.keys')
         AND attnum > 0 AND NOT attisdropped) = 2
    AND (SELECT count(*) FROM pg_catalog.pg_attribute a,
         (VALUES (1, 'number', 'int8', 'a'), (2, 'value', 'bytea', '')) AS keys(position, name, kind, identity)
         WHERE a.attrelid = pg_catalog.to_regclass('mnesis.keys')
         AND a.attnum = keys.position AND a.attname = keys.name
         AND a.atttypid = pg_catalog.to_regtype('pg_catalog.' || keys.kind)
         AND a.attnotnull AND a.attidentity::text = keys.identity) = 2
    AND EXISTS (SELECT FROM pg_catalog.pg_class
                WHERE oid = pg_catalog.to_regclass('mnesis.keys') AND relkind = 'r' AND NOT relrowsecurity)
    AND EXISTS (SELECT FROM pg_catalog.pg_constraint
                WHERE conrelid = pg_catalog.to_regclass('mnesis.keys') AND conname = 'keys_pkey'
                AND contype = 'p' AND conkey = ARRAY[1]::smallint[]
                AND convalidated AND NOT condeferrable)
    AND EXISTS (SELECT FROM pg_catalog.pg_constraint c
                JOIN pg_catalog.pg_index i ON i.indexrelid = c.conindid
                JOIN pg_catalog.pg_class t ON t.oid = i.indexrelid
                JOIN pg_catalog.pg_am a ON a.oid = t.relam
                WHERE c.conrelid = pg_catalog.to_regclass('mnesis.keys')
                AND c.conname = 'keys_value_key' AND c.contype = 'x'
                AND c.conkey = ARRAY[2]::smallint[] AND c.convalidated AND NOT c.condeferrable
                AND a.amname = 'hash' AND i.indisvalid AND i.indisready
                AND i.indpred IS NULL AND i.indexprs IS NULL
                AND pg_catalog.pg_get_constraintdef(c.oid) = 'EXCLUDE USING hash (value WITH =)')
