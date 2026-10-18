SET LOCAL search_path = pg_catalog;
CREATE TABLE mnesis.checkpoints (
    key BIGINT PRIMARY KEY REFERENCES mnesis.keys (number),
    revision NUMERIC(20, 0) NOT NULL
        CHECK (revision > 0 AND revision <= 18446744073709551615),
    schema_version BIGINT NOT NULL
        CHECK (schema_version > 0 AND schema_version <= 4294967295),
    txid NUMERIC(20, 0) NOT NULL
        CHECK (txid >= 0 AND txid <= 18446744073709551615),
    global_seq NUMERIC(20, 0) NOT NULL
        CHECK (global_seq >= 0 AND global_seq <= 18446744073709551615),
    state BYTEA NOT NULL
);
