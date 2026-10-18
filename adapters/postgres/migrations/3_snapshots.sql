SET LOCAL search_path = pg_catalog;
CREATE TABLE mnesis.snapshots (
    key BIGINT PRIMARY KEY REFERENCES mnesis.keys (number),
    schema_version BIGINT NOT NULL
        CHECK (schema_version > 0 AND schema_version <= 4294967295),
    version NUMERIC(20, 0) NOT NULL
        CHECK (version > 0 AND version <= 18446744073709551615),
    state BYTEA NOT NULL
);
