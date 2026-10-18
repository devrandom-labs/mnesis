WITH relation AS (SELECT pg_catalog.to_regclass('mnesis.checkpoints') AS id),
expected(position, name, kind) AS (
    VALUES (1, 'key', 'int8'), (2, 'revision', 'numeric'), (3, 'schema_version', 'int8'),
           (4, 'txid', 'numeric'), (5, 'global_seq', 'numeric'), (6, 'state', 'bytea')
)
SELECT
    (SELECT count(*) FROM pg_catalog.pg_attribute, relation
     WHERE attrelid = id AND attnum > 0 AND NOT attisdropped) = 6
    AND (SELECT count(*) FROM expected JOIN pg_catalog.pg_attribute a
         ON a.attnum = expected.position AND a.attname = expected.name
         AND a.atttypid = pg_catalog.to_regtype('pg_catalog.' || expected.kind)
         AND a.attnotnull AND a.attidentity = '' AND a.attgenerated = '', relation WHERE a.attrelid = id) = 6
    AND EXISTS (SELECT FROM pg_catalog.pg_class, relation
                WHERE oid = id AND relkind = 'r' AND NOT relrowsecurity AND relpersistence = 'p')
    AND NOT EXISTS (SELECT FROM pg_catalog.pg_inherits, relation
                    WHERE inhparent = id OR inhrelid = id)
    AND (SELECT count(*) FROM pg_catalog.pg_attribute, relation
         WHERE attrelid = id AND attnum IN (2, 4, 5)
         AND pg_catalog.format_type(atttypid, atttypmod) = 'numeric(20,0)') = 3
    AND EXISTS (SELECT FROM pg_catalog.pg_constraint c
                JOIN pg_catalog.pg_index i ON i.indexrelid = c.conindid, relation
                WHERE c.conrelid = id AND c.conname = 'checkpoints_pkey'
                AND c.contype = 'p' AND c.conkey = ARRAY[1]::smallint[]
                AND c.convalidated AND NOT c.condeferrable AND i.indisvalid AND i.indisready)
    AND EXISTS (SELECT FROM pg_catalog.pg_constraint, relation
                WHERE conrelid = id AND conname = 'checkpoints_key_fkey' AND contype = 'f'
                AND conkey = ARRAY[1]::smallint[] AND confkey = ARRAY[1]::smallint[]
                AND confrelid = pg_catalog.to_regclass('mnesis.keys')
                AND convalidated AND NOT condeferrable AND confupdtype = 'a' AND confdeltype = 'a')
    AND (SELECT count(*) FROM pg_catalog.pg_constraint c, relation,
         (VALUES
             ('checkpoints_revision_check', 'CHECK (((revision > (0)::numeric) AND (revision <= ''18446744073709551615''::numeric)))'),
             ('checkpoints_schema_version_check', 'CHECK (((schema_version > 0) AND (schema_version <= ''4294967295''::bigint)))'),
             ('checkpoints_txid_check', 'CHECK (((txid >= (0)::numeric) AND (txid <= ''18446744073709551615''::numeric)))'),
             ('checkpoints_global_seq_check', 'CHECK (((global_seq >= (0)::numeric) AND (global_seq <= ''18446744073709551615''::numeric)))')
         ) AS checks(name, definition)
         WHERE c.conrelid = id AND c.conname = checks.name AND c.contype = 'c'
         AND c.convalidated AND pg_catalog.pg_get_constraintdef(c.oid) = checks.definition) = 4
