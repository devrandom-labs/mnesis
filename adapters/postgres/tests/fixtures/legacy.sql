CREATE TABLE IF NOT EXISTS events (
    global_seq     BIGINT GENERATED ALWAYS AS IDENTITY,
    stream_id      BYTEA    NOT NULL,
    version        BIGINT   NOT NULL,
    txid           xid8     NOT NULL DEFAULT pg_current_xact_id(),
    event_type     TEXT     NOT NULL,
    schema_version BIGINT   NOT NULL,
    payload        BYTEA    NOT NULL,
    metadata       BYTEA,
    PRIMARY KEY (global_seq),
    UNIQUE (stream_id, version)
);
CREATE INDEX IF NOT EXISTS events_stream_idx    ON events (stream_id, version);
CREATE INDEX IF NOT EXISTS events_watermark_idx ON events (txid, global_seq);
