# mnesis-postgres

PostgreSQL-backed event-store adapter for [Mnesis](https://github.com/devrandom-labs/mnesis) — the server-side store, built on `sqlx`. Implements `RawEventStore` and `WakeSource` (over `LISTEN`/`NOTIFY`). A reader-side `xid8` watermark keeps the `$all` stream gap-free under concurrent writers.

The [Postgres completion PRD](../../docs/prds/postgres.md) defines the remaining
production requirements, including bounded reads, notification recovery, commit
uncertainty, atomic import, snapshots, projection state, migrations and operations.
See the [status index](../../docs/prd-backlog/status.md) for dependencies and delivery.

## Quickstart

```toml
[dependencies]
mnesis-postgres = "0.1"
```

Point it at a database via `DATABASE_URL`; integration tests boot a real PostgreSQL and skip cleanly when the variable is absent.

## MSRV & stability

MSRV **1.99**. Ships in the **0.x tier** — the wire/schema surface iterates without forcing kernel major bumps. See [STABILITY.md](../../STABILITY.md).

## License

Licensed under your choice of [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE).

## Migrations

Before runtime open, run `PostgresStore::migrate(&migration_pool).await` using an
administrative pool. `connect`, `from_pool` and `from_pool_with_limits` validate
migration checksums and the installed shape without changing the schema. They
reject an absent, incompatible or newer schema. Runtime roles need `USAGE` on
`mnesis`, `SELECT` on `mnesis._sqlx_migrations`, `SELECT, INSERT` on `mnesis.events`
and `USAGE` on `mnesis.events_global_seq_seq`; they need no schema creation rights.

Migrations live in the published crate and use SQLx transactions and locks.
Adoption of a verified legacy `public.events` table preserves its rows, positions
and identity sequence. A compatibility view prevents older constructors from
creating a second event table. Stop old writers for this upgrade; overlapping old
and new binaries are unsupported. Failed transactional steps retain old data and
can be rerun after repairing the reported cause. Never reset migration metadata
to conceal an error. Roll back through a verified backup under maintenance, or
repair forward with the matching migration version.
