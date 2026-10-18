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

The representation upgrade adds `mnesis.keys` and `mnesis.keys_number_seq` to
those runtime grants: `SELECT, INSERT` on keys and `USAGE` on its sequence.
Binary keys use exact native equality with a hash exclusion index; hash collisions
remain distinct keys. Events reference compact internal key numbers, keeping
large raw keys out of the stream/version Btree. Internal key numbers are not
application command IDs or subscriber identities. Versions use `NUMERIC(20,0)`
to preserve the full nonzero `u64` range; event types use UTF8 bytes, including
NUL. Global identity remains positive `BIGINT`, a database capacity limit;
a caller's valid larger resume sequence still permits later transactions.

Version1 and legacy upgrades preserve existing `(txid, seq)` positions and the
next sequence. The second migration removes the redundant stream index and
retains the unique stream/version and global watermark indexes. Invalid stored
scalar values abort the upgrade transaction; repair the reported data under
maintenance before retrying. Foreign `public.events` views are rejected and
preserved. Never rewrite an applied migration checksum. Snapshot and future
projection capabilities share raw key identity; a key alone does not prove
that an event stream exists.

## Atomic writes

The `import` feature implements `AtomicAppend` and the existing blanket importer.
It validates every route, version run and row size before storage, then checks
all heads before any key or event write. One transaction covers every target.
Success returns the highest position; empty input touches no database. This
preserves transaction delivery order even when a batch follows another stream's
concurrent predecessor. A definite failure commits no event or new key.

A lost COMMIT acknowledgement returns `CommitUnknown` with the SQLx source.
A caller timeout can occur after the whole batch committed. Reload committed
history before deciding what to do; never infer rollback or retry safety from
that timeout. Command reconciliation and deduplication remain application-owned.
A native stream/version race reports the observed committed head after rollback;
if that observation fails, the original database error remains a store error.
All local wakes precede one bounded native-hint deadline for the whole batch.
The completion PRD still tracks required import, recovery and capacity evidence.

## Aggregate snapshots

The `snapshot` feature implements `SnapshotStore<Vec<u8>, Version>`. Use the
existing `CodecSnapshotStore` and `Snapshotting` wrappers from `mnesis-store` for
typed aggregate state. This is an unconditional cache: the last committed pair
replaces the previous one, including an older version. Conditional projection
state is a separate capability tracked in the completion PRD.

Migration3 installs the logged snapshot table regardless of compiled features.
Runtime cache writers need `SELECT, INSERT, UPDATE` on `mnesis.snapshots`, plus
the existing key lookup/insert and sequence grants. They need no key updates,
event rewrites or DDL. Startup validates all installed metadata in one brief
read-only database snapshot; canceling it releases that view.

State, schema and version are written in one transaction. A schema mismatch
returns `Stale`; corrupted metadata is an error. The configured byte ceiling
counts ID, state and32 scalar bytes. Stale or over-limit reads do not fetch the
body; writes reject over-limit input before beginning a transaction. Empty state
and the full nonzero unsigned version/schema ranges are supported.

`CommitUnknown` retains its SQLx cause, including through the codec wrapper.
Caller cancellation can follow an actual commit. Reload before making another
storage decision; aggregate cache failure never reverses an acknowledged event
append. Client SIGKILL recovery is tested; server restart, durability and release
acceptance remain tracked by the PRD.
