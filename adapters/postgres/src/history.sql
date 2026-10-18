WITH relation AS (SELECT pg_catalog.to_regclass('mnesis.history') AS id)
SELECT
    EXISTS (SELECT FROM pg_catalog.pg_class, relation
            WHERE oid = id AND relkind = 'r' AND relpersistence = 'p' AND NOT relrowsecurity)
    AND NOT EXISTS (SELECT FROM pg_catalog.pg_inherits, relation
                    WHERE inhparent = id OR inhrelid = id)
    AND (SELECT count(*) FROM pg_catalog.pg_attribute a, relation
         WHERE a.attrelid = id AND a.attnum > 0 AND NOT a.attisdropped) = 3
    AND (SELECT count(*) FROM pg_catalog.pg_attribute a, relation,
         (VALUES ('value', 'bytea'), ('system_id', 'int8'), ('database_id', 'oid')) AS expected(name, kind)
         WHERE a.attrelid = id AND a.attname = expected.name
         AND a.atttypid = pg_catalog.to_regtype('pg_catalog.' || expected.kind)
         AND a.attnotnull AND a.attgenerated = '' AND a.attidentity = '') = 3
    AND (SELECT count(*) FROM (SELECT FROM mnesis.history LIMIT 2) bounded) = 1
    AND EXISTS (SELECT FROM mnesis.history h CROSS JOIN pg_catalog.pg_control_system() s
                JOIN pg_catalog.pg_database d ON d.datname = pg_catalog.current_database()
                WHERE h.system_id = s.system_identifier AND h.database_id = d.oid
                AND pg_catalog.octet_length(h.value) = 16)
    AND EXISTS (SELECT FROM pg_catalog.pg_constraint c, relation
                WHERE c.conrelid = id AND c.conname = 'history_pkey' AND c.contype = 'p'
                AND c.conkey = ARRAY[1]::smallint[] AND c.convalidated AND NOT c.condeferrable)
    AND EXISTS (SELECT FROM pg_catalog.pg_constraint c, relation
                WHERE c.conrelid = id AND c.conname = 'history_value_check' AND c.contype = 'c'
                AND c.convalidated AND pg_catalog.pg_get_constraintdef(c.oid) = 'CHECK ((octet_length(value) = 16))')
    AND EXISTS (SELECT FROM pg_catalog.pg_index i, relation
                WHERE i.indrelid = id AND i.indisunique AND i.indisvalid AND i.indisready
                AND i.indkey::text = '0'
                AND i.indpred IS NULL AND pg_catalog.pg_get_expr(i.indexprs, i.indrelid) = 'true')
