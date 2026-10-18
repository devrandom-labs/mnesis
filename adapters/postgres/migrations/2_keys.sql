-- Maintenance upgrade: preserve event identity and transaction positions.
SET LOCAL search_path = pg_catalog;
CREATE TABLE mnesis.keys (
    number BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    value BYTEA NOT NULL,
    CONSTRAINT keys_value_key EXCLUDE USING hash (value WITH =)
);
INSERT INTO mnesis.keys (value) SELECT DISTINCT stream_id FROM mnesis.events;

-- Only an adopted legacy table has this compatibility guard.
DO $$
BEGIN
    IF to_regclass('public.events') IS NOT NULL THEN
        IF (SELECT relkind FROM pg_class WHERE oid = 'public.events'::regclass) <> 'v'
           OR regexp_replace(pg_get_viewdef('public.events'::regclass), '\s+', '', 'g')
               <> 'SELECTglobal_seq,stream_id,version,txid,event_type,schema_version,payload,metadataFROMmnesis.events;'
        THEN
            RAISE EXCEPTION 'public.events is not the legacy compatibility view; preserve or move the foreign relation before upgrading'
                USING ERRCODE = '55000';
        END IF;
        DROP VIEW public.events;
    END IF;
END
$$;
ALTER TABLE mnesis.events ADD COLUMN key_number BIGINT;
UPDATE mnesis.events e SET key_number = k.number FROM mnesis.keys k
    WHERE e.stream_id = k.value;
ALTER TABLE mnesis.events
    DROP CONSTRAINT events_stream_id_version_key,
    DROP COLUMN stream_id,
    ALTER COLUMN key_number SET NOT NULL,
    ALTER COLUMN version TYPE NUMERIC(20, 0),
    ALTER COLUMN event_type TYPE BYTEA USING convert_to(event_type, 'UTF8');
ALTER TABLE mnesis.events RENAME COLUMN key_number TO stream_id;
ALTER TABLE mnesis.events
    ADD CONSTRAINT events_stream_id_version_key UNIQUE (stream_id, version),
    ADD CONSTRAINT events_stream_id_fkey FOREIGN KEY (stream_id) REFERENCES mnesis.keys (number),
    ADD CONSTRAINT events_version_check CHECK (version > 0 AND version <= 18446744073709551615),
    ADD CONSTRAINT events_schema_version_check CHECK (schema_version > 0 AND schema_version <= 4294967295),
    ADD CONSTRAINT events_global_seq_check CHECK (global_seq > 0);
CREATE VIEW public.events AS SELECT * FROM mnesis.events;
