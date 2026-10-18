SET LOCAL search_path = pg_catalog;
CREATE TABLE mnesis.history (
    value BYTEA PRIMARY KEY CHECK (octet_length(value) = 16),
    system_id BIGINT NOT NULL,
    database_id OID NOT NULL
);
CREATE UNIQUE INDEX history_singleton ON mnesis.history ((true));
INSERT INTO mnesis.history
SELECT uuid_send(gen_random_uuid()), system_identifier, d.oid
FROM pg_control_system() CROSS JOIN pg_database d WHERE d.datname = current_database();
ALTER TABLE mnesis.checkpoints ADD COLUMN history BYTEA;
UPDATE mnesis.checkpoints SET history = (SELECT value FROM mnesis.history);
ALTER TABLE mnesis.checkpoints ALTER COLUMN history SET NOT NULL;
ALTER TABLE mnesis.checkpoints ADD CONSTRAINT checkpoints_history_fkey
    FOREIGN KEY (history) REFERENCES mnesis.history (value);
