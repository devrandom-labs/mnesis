# mnesis-postgres

PostgreSQL-backed event-store adapter for [Mnesis](https://github.com/devrandom-labs/mnesis) — the server-side store, built on `sqlx`. Implements `RawEventStore` and `WakeSource` (over `LISTEN`/`NOTIFY`). A reader-side `xid8` watermark keeps the `$all` stream gap-free under concurrent writers.

The [Postgres completion PRD](../../docs/prds/postgres.md) defines the remaining
production requirements, including bounded reads, notification recovery, commit
uncertainty, atomic import, snapshots, projection state, migrations and operations.
See the [status index](../../docs/prd-backlog/status.md) for dependencies and delivery.

Global reads use PostgreSQL's built-in snapshot functions and comparison operators
explicitly, preserving transaction cutoffs and exclusive resume with caller pool
search paths that put application schemas before `pg_catalog`.

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
All runtime opens also require `SELECT` on `mnesis.history` and `mnesis.events`,
including roles that only write cached state. No runtime role needs to update
or delete history metadata.

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

Version1 and legacy upgrades preserve existing numeric `(txid, seq)` components and the
next sequence. The second migration removes the redundant stream index and
retains the unique stream/version and global watermark indexes. Invalid stored
scalar values abort the upgrade transaction; repair the reported data under
maintenance before retrying. Foreign `public.events` views are rejected and
preserved. Never rewrite an applied migration checksum. Snapshot and
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
state uses the separate checkpoint capability below.

Migration3 installs the logged snapshot table regardless of compiled features.
Runtime cache writers need `SELECT, INSERT, UPDATE` on `mnesis.snapshots`, plus
the existing key lookup/insert and sequence grants. They need no key updates,
event rewrites or DDL. Startup validates all installed metadata in one brief
read-only database snapshot; canceling it releases that view.

State, schema and version are written in one transaction. A suppressed write is
an error and rolls back any new key. A schema mismatch
returns `Stale`; corrupted metadata is an error. The configured byte ceiling
counts ID, state and 32 scalar bytes. Stale or over-limit reads do not fetch the
body; writes reject over-limit input before beginning a transaction. Empty state
and the full nonzero unsigned version/schema ranges are supported.

`CommitUnknown` retains its SQLx cause, including through the codec wrapper.
Caller cancellation can follow an actual commit. Reload before making another
storage decision; aggregate cache failure never reverses an acknowledged event
append. Client SIGKILL recovery is tested; server restart, durability and release
acceptance remain tracked by the PRD.

## Projection checkpoints

The `projection` feature implements `CheckpointStore<Vec<u8>, PgAllPos>`. Compose
the existing `CodecSnapshotStore` with `Projection` for typed state, exclusive
resume, and explicit rebuild. The consumer drives the loop.

Migration4 installs the logged checkpoint table even when the feature is absent.
Runtime checkpoint writers need `SELECT, INSERT, UPDATE` on `mnesis.checkpoints`,
plus the migration metadata and key lookup/insert/sequence grants above. They
need no key updates, event rewrites, checkpoint deletion, or DDL. Startup rejects
incompatible columns, scalar guards, references, logging, and table inheritance.
Repair a failed migration forward without replacing foreign objects or changing
applied checksums; supported upgrades preserve event positions and cached state.

Each conditional write replaces revision, schema, position, and state together.
Only one concurrent writer can advance a revision. Position must increase;
schema replacement requires explicit rebuild and never resets the revision.
Revision overflow rejects without changing storage. Aggregate caches and
checkpoints remain independent even when they use the same raw ID.

Reads return absent, stale, or one complete saved record; invalid metadata is an
error even for a stale schema. The byte ceiling counts ID, state, and 32 scalar
bytes, plus 16 history bytes (48 fixed bytes in total). Stale and over-limit reads omit the body; oversized writes reject before
opening a transaction. Empty state and full unsigned positions are supported.

Failed or canceled checkpoint persistence requires projection reload. A lost
COMMIT acknowledgement preserves `CommitUnknown` through the codec; a caller
timeout can follow a successful commit and never proves rollback or safe retry.
Reload the saved pair before continuing. Application command IDs, duplicate
detection, and controller worker fencing remain outside this adapter.

## Stream listing

The `export` feature implements `StreamLister`. Each event stream appears once;
keys used only by snapshots or projection checkpoints are absent. Binary IDs
remain unchanged. Empty listings terminate, and exhausted cursors stay exhausted.

Listing captures committed event membership when it opens. Streams committed
before capture remain visible even behind an unrelated active transaction;
streams created or first appended afterward are excluded from that listing.
Pages follow internal key order; callers should rely on the trait's unordered
enumeration contract. Immutable event rows preserve this view without retaining
a database transaction or connection between pages.

The existing `Limits` bound page rows and bytes; listing bytes count ID plus 32
scalar bytes. An oversized ID returns `RowLimit` without transferring its body,
then ends the cursor. Memory includes the bounded page and Postgres's captured
transaction visibility record, whose size depends on active transactions.

Listing and ordinary per-stream exports capture independent views. Their
composition does not guarantee a consistent multi-stream backup.

## Consistent export sessions

With `export`, use `ConsistentExporter::open_export_session` for a single view
covering listing and every stream read. The read-only repeatable-read transaction
is established before open returns. A later atomic multi-stream commit is wholly
absent, including streams read for the first time after that commit.

Each session holds one connection from the caller's pool and one task on the
existing Tokio runtime. The shared listener also holds a connection, so session
admission shares the pool's capacity and acquisition timeout with other work.
No connection is created outside that pool. One queued request and one active
request bound internal work; each cursor retains at most its configured row/byte
page. Callers own the number of cursors and their yielded envelopes.

A session pins native MVCC history and can delay vacuum reclamation. Choose a
short, sufficient nonzero lifetime. Expiration and Drop release the view without
another caller poll, including idle or blocked reads; closing the caller's pool
also stops session ownership. Explicit `close` waits for rollback and connection
cleanup, and requires dropping borrowed cursors first. An expired cursor returns
`ExportError::Expired` and ends; owned envelopes already yielded remain valid.

The transaction uses a local one-second `client_connection_check_interval`,
preserving a caller's shorter nonzero interval. This lets supported PostgreSQL
servers detect socket closure during blocked SQL. It does not shorten query
execution or change write settings, and it does not establish a recovery bound
for network failures hidden from the server's kernel. Lifecycle and recovery
profiles remain tracked in the completion PRD. Any export error means the backup
is incomplete; normal import still requires completed format-2 chunks.

Compose the session with the existing `mnesis-store` CBOR feature. Keep listing
lazy, divide long streams into bounded batches, and reuse a fixed-capacity sink
for each `ChunkWriter`. Only `finish` produces a completed chunk. Choose batch
and sink limits that fit the configured maximum row, including encoding overhead;
sink exhaustion, source failure and cancellation invalidate that chunk.

Restore one bounded completed chunk at a time with `decode_chunk` and the existing
importer. Continuation chunks retain stream versions. `WholeChunk` applies to
each individual chunk; earlier completed chunks may already be committed when a
later chunk fails. Record the complete expected chunk inventory separately and
check it before declaring recovery complete: a valid chunk does not prove that
other files arrived. Apply the deployment's file flush, backup publication and
retention policy; `finish` does not sync a filesystem. Native tests cover
Postgres round trips, Fjall interchange after reopen, incomplete chunks, source
failure and cancellation. Logical restore rebuilds the existing projection
consumer, reopens its saved checkpoint and processes a later append. Operator
drills and physical projection restart acceptance remain in the PRD.

## History and recovery

`PgAllPos` contains a `History` identity and the transaction and sequence
components. Persist all three unchanged; reconstruct with
`PgAllPos::new(History::new(saved_bytes), saved_txid, saved_seq)`. Ordering and
exclusive resume apply within one history. Global reads and checkpoint writes
reject another history with `HistoryMismatch` before acquiring a connection.

Migration5 creates a 16-byte history identity, binds it to the native cluster
and database, and attaches it to existing checkpoint rows in the same migration
transaction. Migrations1–4 and their checksums remain unchanged. This is a
breaking adapter change for the next 0.x minor release: old binaries and the
two-argument constructor are incompatible. Stop old writers and save a verified
backup before upgrading. External two-number checkpoints have no saved identity;
verify their provenance before explicitly attaching the migrated store's identity.
Never attach a new identity merely to bypass a mismatch.

A coherent physical backup preserves the history, database identity, transaction
counter, events and checkpoint pairs. Native restore tests stop the source before
booting its copy, then verify state, exclusive resume and subsequent writes.
This does not establish cross-node failover or hardware power-loss guarantees.

Logical event reimport into a newly migrated database creates a new history and
assigns new positions. Rebuild projection state from those events. Copying native
`txid` values or old checkpoint pairs is unsupported: open rejects foreign
cluster/database bindings (`DatabaseHistory`) and event transaction IDs at or
above the native snapshot's upper bound (`FutureTransaction`). These checks
detect unsafe copies; they do not authenticate backups or repair arbitrary
administrative changes. Do not edit bindings, transaction counters or migration
metadata to force acceptance. Restore a verified coherent physical backup or
reimport events into a fresh history instead.

## Operator recovery drill

Choose the deployment's recovery point and recovery time objectives before
running this procedure. Record PostgreSQL/tool versions, storage, database
settings, backup start/end, the source cutoff and the time the recovered consumer
becomes ready. The native test below measures a small local fixture; its timings
are not production targets. Replication, cross-node failover and storage power
loss require their own tested profiles.

For a physical drill, use the matching PostgreSQL tools and a compatible adapter
binary. Keep the source's `fsync` and local commit-flush settings appropriate for
its durability claim. Create a coherent backup with all required WAL:

```sh
pg_basebackup --checkpoint=fast --wal-method=stream \
  --dbname="$SOURCE_DSN" --pgdata="$RESTORE_DATA" || exit 1
pg_verifybackup --format=plain "$RESTORE_DATA" || exit 1
```

Verify the manifest, file checksums and required WAL before boot; do not use
checksum/WAL skip flags. Native verification supplements the actual restore.
Keep the backup's complete directory and WAL together. Record the source history
identity, migration versions/checksums and the application's saved projection
position/state before the handover. A successful copy command alone does not
prove that the application can recover.

During the controlled handover, stop source writers through their owning
controller and stop the old primary. Require successful waited shutdown, then
the exact native status for a stopped server; an inaccessible data directory
is a different error:

```sh
pg_ctl -D "$SOURCE_DATA" -m fast -w stop || exit 1
if pg_ctl -D "$SOURCE_DATA" status; then
  exit 1
else
  test "$?" -eq 3 || exit 1
fi
```

Configure the target's paths, ports, authentication and TLS for its isolated
deployment before starting it. Preserve the physical history and database
identity; never edit adapter bindings to pass validation. Boot the target with
`pg_ctl -D "$RESTORE_DATA" -w start`, then open the existing pool through
`PostgresStore::from_pool`. Verify migration checksums, history and the saved
projection pair. Reload the actual projection consumer, resume strictly after
its checkpoint, and exercise a new append and checkpoint advance before routing
application work to the target. Record the observed restored point and elapsed
time. A timeout during any write still requires reconciliation.

The executable drill uses independent private clusters, checks old-primary
shutdown before target startup, and verifies actual repository/projection
progress after recovery:

```sh
nix develop --command cargo nextest run -p mnesis-postgres --all-features \
  --test restore_tests --test-threads=1 --success-output immediate \
  -E 'test(physical_operator_drill_restarts_the_saved_projection_and_advances)'
```

Supply the integration `DATABASE_URL`; without it the native tests return early.
For a logical event restore, install/migrate a fresh target, validate the complete
chunk inventory, import completed chunks with explicit routing and atomicity,
and rebuild projection state from the new event history. Old source positions
cannot be resumed against that history. Logical event chunks do not carry the
source's snapshot caches, checkpoints or native migration tables.
