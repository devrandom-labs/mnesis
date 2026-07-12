# mnesis-postgres

PostgreSQL-backed event-store adapter for [Mnesis](https://github.com/devrandom-labs/mnesis) — the server-side store, built on `sqlx`. Implements `RawEventStore`, `WakeSource` (over `LISTEN`/`NOTIFY`), and the snapshot/projection seams. A reader-side `xid8` watermark keeps the `$all` stream gap-free under concurrent writers.

## Quickstart

```toml
[dependencies]
mnesis-postgres = "0.1"
```

Point it at a database via `DATABASE_URL`; integration tests boot a real PostgreSQL and skip cleanly when the variable is absent.

## MSRV & stability

MSRV **1.95**. Ships in the **0.x tier** — the wire/schema surface iterates without forcing kernel major bumps. See [STABILITY.md](../../STABILITY.md).

## License

Licensed under your choice of [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE).
