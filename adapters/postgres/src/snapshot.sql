WITH relation AS (SELECT pg_catalog.to_regclass('mnesis.snapshots') AS id),
expected(position, name, kind) AS (
    VALUES (1, 'key', 'int8'), (2, 'schema_version', 'int8'),
           (3, 'version', 'numeric'), (4, 'state', 'bytea')
)
SELECT
    (SELECT count(*) FROM pg_catalog.pg_attribute, relation
     WHERE attrelid = id AND attnum > 0 AND NOT attisdropped) = 4
    AND (SELECT count(*) FROM expected JOIN pg_catalog.pg_attribute a
         ON a.attnum = expected.position AND a.attname = expected.name
         AND a.atttypid = pg_catalog.to_regtype('pg_catalog.' || expected.kind)
         AND a.attnotnull AND a.attidentity = '', relation WHERE a.attrelid = id) = 4
    AND EXISTS (SELECT FROM pg_catalog.pg_class, relation
                WHERE oid = id AND relkind = 'r' AND NOT relrowsecurity AND relpersistence = 'p')
    AND EXISTS (SELECT FROM pg_catalog.pg_attribute, relation
                WHERE attrelid = id AND attname = 'version'
                AND pg_catalog.format_type(atttypid, atttypmod) = 'numeric(20,0)')
    AND EXISTS (SELECT FROM pg_catalog.pg_constraint c
                JOIN pg_catalog.pg_index i ON i.indexrelid = c.conindid, relation
                WHERE c.conrelid = id AND c.conname = 'snapshots_pkey'
                AND c.contype = 'p' AND c.conkey = ARRAY[1]::smallint[]
                AND c.convalidated AND NOT c.condeferrable AND i.indisvalid AND i.indisready)
    AND EXISTS (SELECT FROM pg_catalog.pg_constraint, relation
                WHERE conrelid = id AND conname = 'snapshots_key_fkey' AND contype = 'f'
                AND conkey = ARRAY[1]::smallint[] AND confkey = ARRAY[1]::smallint[]
                AND confrelid = pg_catalog.to_regclass('mnesis.keys')
                AND convalidated AND NOT condeferrable AND confupdtype = 'a' AND confdeltype = 'a')
    AND (SELECT count(*) FROM pg_catalog.pg_constraint c, relation,
         (VALUES
             ('snapshots_schema_version_check', 'CHECK (((schema_version > 0) AND (schema_version <= ''4294967295''::bigint)))'),
             ('snapshots_version_check', 'CHECK (((version > (0)::numeric) AND (version <= ''18446744073709551615''::numeric)))')
         ) AS checks(name, definition)
         WHERE c.conrelid = id AND c.conname = checks.name AND c.contype = 'c'
         AND c.convalidated AND pg_catalog.pg_get_constraintdef(c.oid) = checks.definition) = 2
