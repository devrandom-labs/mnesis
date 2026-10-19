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

## Verified connections

The adapter enables SQLx's Rustls/ring TLS backend with native system trust roots.
Build a caller-owned pool with `PgSslMode::VerifyFull` and the expected server
hostname. A configured `ssl_root_cert` adds its CA to the system roots; it does
not replace those roots. Verification rejects an untrusted certificate or a name
missing from the certificate. Encryption alone does not verify server identity.
The ordinary URL constructor follows SQLx's requested SSL mode; compiling TLS
support does not change a caller's `Prefer` or `Disable` setting.

The [secure example](examples/secure.rs) requires `DATABASE_URL` and
`PGSSLROOTCERT`, explicitly selects `VerifyFull`, and reads an already migrated
database. It never falls back to plaintext. Use the runtime role below rather
than the migration account:

```sh
nix develop --command cargo run -p mnesis-postgres --example secure
```

Configure the server for SSL and restrict remote runtime access to `hostssl`
entries with appropriate authentication, such as `scram-sha-256`. Keep migration
access under the operator's separate policy. Use the matching hostname in the
connection options; changing it to an IP requires that IP in the certificate.
Provision certificates and passwords through the deployment's existing secret
management. Never log connection URLs, passwords, or `PgConnectOptions`, including
its `Debug` representation. The example emits a generic failure without printing
those inputs; applications should record only selected safe error fields.

Authentication retains the native SQLSTATE and cause; certificate/name failures
retain their Rustls cause through SQLx. Native tests pin authentication `28P01`,
plaintext rejection `28000`, and both certificate failures. They also check that
returned error display/debug text omit the supplied test passwords; this does
not establish redaction for arbitrary application logs or connection inputs.

Create the login through your existing role/secret administration. For an
already migrated database, this is the complete event runtime grant profile
(replace `mnesis_runtime` with that login):

```sql
GRANT USAGE ON SCHEMA mnesis TO mnesis_runtime;
GRANT SELECT ON mnesis._sqlx_migrations, mnesis.history,
    mnesis.keys, mnesis.events TO mnesis_runtime;
GRANT INSERT ON mnesis.keys, mnesis.events TO mnesis_runtime;
GRANT USAGE ON SEQUENCE mnesis.keys_number_seq,
    mnesis.events_global_seq_seq TO mnesis_runtime;
```

Snapshot writers additionally need `SELECT, INSERT, UPDATE` on
`mnesis.snapshots`; projection writers need those rights on `mnesis.checkpoints`.
All runtime opens validate the installed schema, including these tables, without
DDL. Runtime users need no event `UPDATE`, `DELETE`, `TRUNCATE`, schema creation,
table ownership, superuser or history metadata write rights. Native TLS tests
verify denied event rewrites/schema changes, atomic competing appends, state
replacement and saved checkpoint recovery. The role must not inherit broader
rights from another membership or own the schema/tables.

Caller pool acquisition timeouts bound waiting for a pool lease. They do not
establish a statement/lock deadline or prove rollback after a write timeout.
Keep at least two connections available for the shared listener and data work;
the adapter rejects a pool configured with only one. Follow the transaction
outcome rules below for any interrupted write. Capacity and remaining operational
profiles stay under the completion PRD.

## Deadlines and shutdown

Configure deadlines on the caller's pool and native connection options:

```rust,ignore
let options = options.options([
    ("statement_timeout", "5s"),
    ("lock_timeout", "1s"),
]);
let pool = PgPoolOptions::new()
    .max_connections(8)
    .acquire_timeout(Duration::from_secs(2))
    .connect_with(options)
    .await?;
```

These values illustrate configuration; choose deployment limits from measured
workloads. Acquisition covers pool admission and connection establishment.
Statement timeout covers server work; lock timeout covers each lock wait.
Set a shorter lock limit when distinguishing lock contention from statement
execution matters. Configure migration deadlines separately for maintenance.

Native tests exercise exhausted pools, blocked startup and appends, and a
statement deadline after an insert has been staged. Those errors occur before
COMMIT and roll back their event/key changes. Their classification does not
apply to an interrupted COMMIT or a caller timeout after commit: reconcile those
outcomes under the rules below. Read errors may be native connection failures
before an acquisition deadline expires.

Opening validates the schema before returning. The listener starts in the
background; opening does not promise its first LISTEN has completed. Reads and
subscription rescans work while notifications are unavailable. Clones share
one listener; last-handle drop aborts it. Drop active cursors and store handles
before calling `pool.close().await` when the application owns the pool.
Dropping a store leaves that pool usable. SQLx pool closure waits for outstanding
leases, so application tasks must release their own leases and export sessions.
Native tests verify bounded close even while the server remains stopped.
## Diagnostics

Enable the optional `tracing` feature and install the application's existing
tracing subscriber. `mnesis_postgres=debug` includes operation spans and page
events; span-close events provide operation duration. No subscriber is installed
by the adapter. Configure the subscriber's filters and output in the application.

Listener events distinguish ready, automatic reconnect, failed retry and pool
closure. Retry fields contain a fixed error kind and optional SQLSTATE, without
raw error messages. Page events report stream/all, actual rows and elapsed time;
watermark events report the finite transaction cutoff. Rescan events distinguish
an arriving hint from the independent deadline. Commit events distinguish
committed, explicit rejection and uncertain completion. An unknown outcome still
requires reconciliation; logging does not make a timeout safe to retry.
A missing terminal event does not prove rollback. Reconcile an interrupted write
using durable state rather than inferring completion from logs.

Event names, outcomes and error kinds form the bounded label set. Row counts,
durations and transaction cutoffs are values, not metric labels. Adapter spans
skip arguments; events omit keys, payloads, credentials and connection options.
SQLx has separate pool acquisition timing/slow-wait settings on `PgPoolOptions`;
use those supported settings with pool size/idle counts to distinguish pool wait
from query work. Server lock/activity statistics identify the blocking owner.

For a stalled projection, compare saved progress with the reported cutoff and
inspect long transactions and blockers from an operator connection:

```sql
SELECT pid, application_name, state, wait_event_type, wait_event,
       xact_start, backend_xmin, pg_blocking_pids(pid)
FROM pg_stat_activity
WHERE datname = current_database()
ORDER BY xact_start NULLS LAST;
```

An old transaction can hold the global cutoff back after newer commits. Identify
its owning application before ending that transaction. Ordinary roles see their
own activity; use the operator's existing statistics permissions for other roles.
Applications need no superuser grant. End unnecessary transactions through their
owner; preserve the cutoff and projection checkpoint rules while investigating.

## Durability

The local durable-write profile requires logged tables, `fsync=on`,
`full_page_writes=on`, and `synchronous_commit` other than `off`. Runtime opening
and explicit migration reject incompatible settings with
`PostgresError::Durability`, naming the setting. The adapter preserves configured
`on`, `local`, `remote_write`, and `remote_apply` modes; it never silently replaces
them with a weaker policy. Replication guarantees require a separate tested
deployment profile.

Every event, atomic import, snapshot and checkpoint write runs deferred
constraints before checking its transaction's settings and issuing COMMIT. This
also checks replacement pool connections and settings changed by write triggers.
A durability rejection rolls back the write and its new keys. The caller's pool
settings are not changed. Keep the server and storage profile fixed while writers
run; these checks do not certify administrative changes or storage hardware.

An error before issuing COMMIT leaves this owned transaction uncommitted.
After COMMIT is sent, `CommitUnknown` retains the original cause and requires
reconciliation; it never proves that retrying is safe. A caller timeout can follow
an actual commit. Reload saved state before continuing after a failed or canceled
checkpoint write. Command identity, duplicate detection and worker fencing remain
application responsibilities.

Native tests verify immediate primary crash/restart recovery of acknowledged
events and saved pairs, including subsequent append and exclusive resume. This
is process-crash evidence for the tested local profile. Hardware power loss,
cross-node failover, deployment recovery targets and capacity remain separate.

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

Runtime validation bounds metadata transfer using the migrations embedded in the
binary. Unknown versions, incomplete records and differing checksums still reject;
full checksum equality remains required.

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
append. Client SIGKILL and immediate primary restart recovery are tested for the
local profile above; complete release acceptance remains tracked by the PRD.

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

Keep an independent inventory of every intended chunk and its source cutoff.
Before importing, check every inventory entry for existence, the selected file
size cap and successful normal decoding, one bounded chunk at a time. A valid
completion record in the remaining files cannot reveal an entirely missing
file. Keep verified files immutable while importing. This preflight does not
make a multi-chunk restore atomic: storage failure during import may leave prior
chunks committed under either policy.

Import reports describe progress committed by that request. A mismatch with
`reached: None` means that section committed nothing; the target may already
contain events. Inspect its history before reconciling. Partial salvage requires
an explicit choice, retention of producer/completion evidence, and examination
of corrupt blocks and the resulting per-stream report.
