# Postgres adapter completion

Status: active (PG4 isolation; PG7 measurements; PG8 security/operations; PG9 remain). Updated: 2026-10-19.
Owner: Mnesis Postgres adapter maintainers.
Baseline: `747aea2881938d6a7f10a85aec1475bc8b8289e3`.
Planning branch: `docs/postgres-prd`. Current slice: `work/postgres-durability`.
Dependencies and delivery states: [backlog](../prd-backlog/status.md).

## Problem

Mnesis needs a complete, supportable Postgres adapter for server applications.
The current adapter validates the basic event-store interface, but its read
paths load the entire matching history into memory, notification loss can leave
a live subscriber asleep indefinitely, and write errors do not preserve whether
the transaction outcome is known. Optional store capabilities, schema upgrades,
secure deployment, and operational recovery also need implementation or proof.

The target is an application that can append events, rebuild aggregates, follow
all streams, persist projection state, export and restore events, and recover
from connection and process failures without skipped events or unsafe retries.
Fixing bounded reads, missed notifications, and commit uncertainty is the first
delivery slice; it does not complete this PRD.

## Completion scope

All requirements PG0–PG9 below are required for the production release. Existing
capabilities count only when their acceptance evidence passes. Optional features
may remain opt-in, following the Fjall adapter's feature arrangement.

| Capability | Baseline source | Required result |
| --- | --- | --- |
| Single-stream append | Implemented in `store.rs` | Atomic version validation and writes, exact conflict classification, durable acknowledgement, explicit uncertain outcome. |
| Stream and `$all` reads | Eager `fetch_all`, followed by a second collection | Bounded, lazy reads with stable order, correct resume bounds, finite scans, and prompt resource release. |
| Live subscriptions | `PgListener` forwards hints to `StreamNotifiers` | Recovery without another notification or write; bounded connection and task use. |
| Atomic multi-stream writes | Not implemented | `AtomicAppend`, including whole-chunk import through the existing blanket implementation. |
| Aggregate snapshots | Not implemented | `SnapshotStore<Vec<u8>, Version>` using the existing codec wrapper. |
| Projection state | Not implemented | `CheckpointStore<Vec<u8>, PgAllPos>` conditionally saves revision, schema, state and position together. |
| Export and stream discovery | Per-stream export inherited; no `StreamLister` | Bounded stream enumeration, finite consistent export sessions and verified format-2 backup/import composition. |
| Schema lifecycle | `CREATE ... IF NOT EXISTS` on open | Versioned, validated migrations; ordinary runtime access without DDL privileges. |
| Operations | Shared pool; listener aborted on last store drop | Secure connection options, timeouts, readiness, shutdown, diagnostics, recovery instructions, and measured limits. |
| Delivery | Basic conformance and two out-of-order commit tests | Required real-database CI, complete feature coverage, migration and failure evidence, reviewed merge and verified publication. |

At the baseline, the [adapter README](../../adapters/postgres/README.md) claimed
snapshot and projection seams that its source and conformance registration did
not implement. This planning branch corrects that claim; update it when support
is verified. “Production grade” describes the tested deployment profile and
these capabilities; it does not promote the adapter out of its 0.x stability tier.

### Ownership boundaries

| Owner | Responsibility |
| --- | --- |
| `mnesis-store` | Append/read laws, `BatchSize`, subscription sequencing, codecs, import/export, snapshot and projection contracts. |
| `mnesis-postgres` | SQL, schema, positions, transaction outcomes, bounded cursors, listener recovery, Postgres resource ownership. |
| `mnesis-wake` | Existing in-process registration and wake mechanics. Extend only if a failing adapter test proves an absent capability. |
| `mnesis-store-testing` | Shared observable laws; adapter-only database fault tests stay in the adapter. |
| Application and database operator | Pool policy, secrets, database durability/replication, availability targets, backup schedule, consumer effect semantics. |
| Labcast controller | Durable command IDs, duplicate detection, stale-worker rejection, command reconciliation and external-effect ownership. |
| `mnesis-bombay` | Translation into its selected execution contracts; no assumption that its released Mnesis dependency contains this work. |

A separate named-subscriber checkpoint repository is PG10, a conditional adapter
addition requiring a selected consumer and explicit acknowledgement semantics.
It is distinct from the required atomic projection-state snapshot above. Neither
capability supplies command deduplication, worker fencing, or exactly-once external
effects. This PRD does not add an actor runtime, outbox framework, replication
system, generic transaction framework, or application controller.

## Locked dependency verification

This is source inspection for PRD preparation, not passing runtime evidence.
Recheck the selected lock and affected contracts before each implementation slice.

The table records planning-baseline versions. Subsequent protected slices select
Rust1.99.0 and SQLx0.9.0, with PostgreSQL17.10 local and18.6 native Linux evidence.
Exact dependency decisions and checks are in the linked research record; the
workspace manifests, toolchain file and locks remain authoritative.

| Selected owner | Version or revision | Verified surface and remaining evidence |
| --- | --- | --- |
| Mnesis workspace | 0.3.1 at the baseline above | `RawEventStore`, `AllPosition`, `AtomicAppend`, `StreamLister`, `SnapshotStore`, `BatchSize`, subscription loop, Postgres source and conformance tests inspected. |
| SQLx / sqlx-postgres | 0.8.6, `Cargo.lock` | Local registry `listener.rs` and `transaction.rs` inspected. Listener reserves a pool connection; receive errors are not limited to pool closure. Commit awaits a server response; connection loss cannot prove rollback. |
| Tokio / futures / thiserror | 1.53.1 / 0.3.33 / 2.0.19 | Existing runtime, stream, and error dependencies; no replacement runtime needed. |
| Rust | 1.95.0 | `rust-toolchain.toml` and workspace MSRV own the pin. |
| Nixpkgs | `64c08a7ca051951c8eae34e3e3cb1e202fe36786` | Locked package evaluation returns PostgreSQL 17.10. Existing Linux integration package uses the NixOS Postgres service. |
| Crane | `3d9df76e29656c679c744968b17fbaf28f0e923d` | Existing nextest archive, workspace checks, and integration package are the starting point. |
| mnesis-bombay consumer | Local lock selects registry Mnesis 0.3.1 | Consumer adoption must verify the new released adapter/core versions; sibling HEAD is not compatibility evidence. |

The workspace requests SQLx `runtime-tokio`, `postgres`, and `macros` with default
features disabled. It does not explicitly select TLS or migrations. Verify the
isolated package feature graph, then select supported SQLx features for those
capabilities; do not rely on unrelated workspace feature unification.

Reuse SQLx pooling, transactions, migration support and TLS, Tokio timers, existing
`futures` stream composition, and the conformance kit. Before changing dependency
selection, record maintained-version compatibility, license, advisory status,
feature/transitive cost, and build impact. Existing MIT/Apache-compatible choices
are candidates, not an assertion that today's locked versions are advisory-free.
Keep `cargo audit` and `cargo deny` evidence at the delivery revision.

### Contract decisions required by PG0

1. **Global order:** source documents call `(txid, seq)` “commit order”, but the
   existing no-skip test deliberately commits B before A and delivers A before B.
   Distinguish physical commit completion from the stable delivery order.
   Preserve no-skip, exclusive resume, and the returned-position barrier. Reconcile
   the wording in `AllPosition`, adapter docs and shared tests before changing it;
   if strict commit-completion order is required, the current algorithm needs a
   different design and migration. Do not silently relabel it as proven.
2. **Atomic failure:** `AtomicAppend` says any failure applies no writes. A remote
   commit can succeed while its acknowledgement is lost. The owner must distinguish
   definite rejection from unknown outcome while retaining all-or-none database
   atomicity. Update affected consumer/error tests and stability documentation if
   this changes the shared contract; never satisfy that sentence with a false claim.
3. **Representation:** core versions span nonzero `u64`; current SQL uses signed
   `BIGINT`. `PgAllPos::new` accepts values that current read casts cannot represent.
   Audit text NULs, binary IDs, frame limits and Postgres value limits as well.
   Select a lossless representation where the core requires it; otherwise establish
   an explicit backend limit and typed pre-write rejection with the contract owner.
   A documented limitation cannot waive an existing required capability.
4. **Finite reads:** record the visibility boundary captured for each one-shot
   scan. Paging must not chase continuous writes forever or change `CaughtUp`
   semantics. The selected boundary must not hold a long-lived read transaction
   that itself prevents the `$all` watermark from advancing.

## Requirements and acceptance

### PG0 Reproduce defects and settle contracts

Use real Postgres before production edits. Add native adapter regressions for
whole-backlog materialization and a dropped final notification while a subscriber
is already parked, with no later writes. Capture a baseline memory profile and
the exact failing test/trace at the baseline revision. Arrange notification loss
deterministically with the existing database/client facilities or a narrow private
test seam; a random sleep is not a fault oracle. Reproduce the watermark-only
progress case described in PG2 as well.

Resolve the four contract decisions above, inventory every public adapter
capability against the shared traits, and record selected deployment assumptions.
Initially target the locked PostgreSQL 17 profile on a writable primary with
session-capable LISTEN connections. Other server majors, read replicas, transaction
pooling proxies and failover configurations are supported only after their named
matrix tests pass. A newer supported minor may be selected before release through
a measured lock update. No untested multi-version support claim is permitted.

Acceptance: recorded failing real-database witnesses, baseline workload, exact
dependency map, owner decisions, and a small PG1–PG3 change record. A source finding
alone does not count as a reproduced defect.

### PG1 Bounded reads and stable positions

Replace eager whole-history loading in both read paths. Reuse `BatchSize`
(default 256, accepted range 1–4096) rather than a second row-count type. Prefer
keyset pages: each query starts after the last delivered key, with no `OFFSET`.
Keep the initial stream bound inclusive and subsequent page bounds exclusive;
`$all` is always exclusive and keeps the full `(txid, seq)` checkpoint.

Preserve the safe visibility watermark on every global page and use an indexable
native `xid8` comparison instead of casting the indexed column to `bigint`.
Record `EXPLAIN (ANALYZE, BUFFERS)` for early and late checkpoints. Compare SQLx
streaming with keyset pages before selection: streaming is viable only if the
owned stream contract, connection occupancy and cancellation requirements hold.

Bound fetched and decoded rows and account for temporary frame copies. Row count
alone is not a practical byte bound: record the supported maximum row size,
per-cursor byte budget and behavior for an oversized accepted row. Enforce any
new limits consistently at append/import boundaries under PG0's contract decision.
The cursor must not silently discard events to meet a resource target.

Acceptance and negative tests:

- Page sizes 1, 2, default and maximum; histories of 0, 1, B−1, B, B+1 and many
  pages; inclusive stream reads, exclusive global reads, empty reads, and gaps
  after rollback. No duplicate or missing events across page boundaries.
- Multiple events in one transaction, reversed commit completion, concurrent
  streams, continuous writers and restart from every tested page boundary.
- Attribution, metadata, schema version and payload bytes survive each page.
- Corruption on a later page yields an error without advancing past the failing
  item; caller cancellation/drop releases queries and connections. An idle reader
  does not retain an exclusive connection between bounded page fetches.
- A consumer that reads one event does not fetch the full backlog. At fixed page
  and row size, increasing history 10× does not cause retained cursor memory to
  grow with history; record allocations and process memory separately.
- Preserve the subscription's single `CaughtUp` marker, `Send`/owned stream
  requirements and working subscription composition. End each one-shot scan at
  its documented finite boundary even under a continuous writer.

### PG2 Recovery from missing hints

Notifications accelerate scans; persisted rows determine correctness. Establish
an independent periodic rescan path for every live subscription, including `$all`.
Keep timing in the Postgres adapter; do not add Tokio to `mnesis-store` or require
each application to repair the listener. Reuse existing wake ownership and avoid
one background task or database connection per subscriber.

Select a positive rescan interval R (initial proposal: one second), with bounded
retry delay for listener recovery. Under a healthy database, available pool and
actively polled consumer, an eligible event must be offered within R plus the
measured bounded scan/scheduling allowance. Specify that allowance in the PG0
test profile and assert a deadline. After an outage, apply the bound from restored
query service and visibility, including the configured recovery delay.

Required independent cases:

- Subscriber is parked; writer commits once; the final hint is lost; no later
  writes or hints occur. Both stream and `$all` consumers receive the event.
- Listener disconnects, misses a commit, reconnects, and receives no subsequent
  notification. Initial LISTEN failure and later non-closure receive errors recover
  or expose explicit terminal failure instead of silently leaving a dead task.
- An older transaction holds the watermark. A newer event commits and its hint
  is consumed before it is eligible. The older transaction then commits or rolls
  back without a Mnesis notification. `$all` eventually delivers every eligible
  event. Include an unrelated transaction and a long-held snapshot.
- Notification arrives during startup, registration, confirm-scan, or cancellation.
  Duplicate, malformed and overlong payloads never corrupt routing or progress.
  Binary stream IDs remain supported even when their hex encoding exceeds the
  NOTIFY payload limit; use a bounded hint or fallback without rejecting a commit.
- Every clone shares listener ownership; idle subscribers, pool pressure and
  reconnect storms do not create unbounded tasks, connections or queued hints.

Long transactions can legitimately delay visibility under the watermark design.
Expose and document that delay. A timer cannot override the watermark safely.

### PG3 Transaction outcome and retry safety

Model transaction outcome separately from the error cause. The names below are
semantic categories, not a preselected public enum or new retry framework.

| Evidence at the boundary | Outcome | Caller consequence |
| --- | --- | --- |
| Validation failed or pool acquisition failed before a transaction could write | Definitely not committed | Preserve cause; retry only if caller policy permits it. |
| Version conflict or a verified server rejection that aborts the transaction | Definitely not committed | Conflict may permit reload and a new decision; other causes keep their own classification. |
| Commit response confirms success | Committed | Return the assigned last/highest position. A later hint failure cannot reverse success. |
| COMMIT may have reached the server, but its response is lost, times out, or is cancelled | Unknown | Reconcile through durable application identity; never report “safe to retry”. |
| Caller drops the future and receives no result | No outcome delivered | Cancellation alone proves no rollback; document the phase-dependent uncertainty. |

Retain the original SQLx/server cause, SQLSTATE and transaction phase where needed.
Classify a unique violation as an optimistic conflict only for the intended
stream/version constraint. An unrelated unique constraint must not become a
retry-eligible conflict. Deadlock, serialization failure, permission failure,
overflow, corruption, pool timeout and uncertain commit are distinct facts.
SQLSTATE alone is insufficient for every phase; record the evidence needed for
each mapping. No adapter or wrapper automatically retries an unknown append.

Bound post-commit notification work so an acknowledged commit is not held behind
an unbounded auxiliary operation. Compare transactional NOTIFY with bounded
post-commit hints and periodic recovery; neither removes commit uncertainty.
Cover append, atomic append and snapshot commit with the same outcome discipline.

Acceptance: fault tests before BEGIN, during inserts, before COMMIT, after server
commit but before client acknowledgement, and after acknowledgement before notify.
Use an independent connection to inspect rows after settlement. The after-commit
test must prove rows committed while the response was suppressed; terminating a
backend at an arbitrary time is insufficient. Keep any fault controls private to
tests and use supported facilities before adding a test-only transport seam.
Assert no partial batch, no false conflict, and uncertainty preserved through
`StoreError`, repository execution, import, and projection consumers. An immediate
empty reconciliation read while the original transaction is unresolved is not
proof that retry is safe.

### PG4 Schema, migrations and representation

Replace silent create-if-absent assumptions with versioned schema ownership.
Prefer SQLx's migrator with checked-in SQL over a custom migration engine. Provide
explicit migration and validation paths; an application using an existing schema
and runtime role must not require CREATE/ALTER permissions on every open.

Define safe adoption of the currently unversioned `events` table, validate its
columns and constraints, preserve existing events and compatible checkpoints,
and reject unknown newer or incompatible layouts with an actionable error.
Concurrent migration attempts serialize safely. Interrupted upgrades leave a
recognizable recoverable state. Record supported old/new binary overlap and the
rollback or forward-repair procedure; no destructive automatic reset.

Audit constraint names, binary keys, numeric ranges, null/empty metadata,
schema-version range, text encoding and payload sizes. Eliminate redundant
indexes only after measuring query plans and upgrade behavior; the stream/version
unique constraint already creates an index. Keep the watermark/resume index
suited to PG1 queries. Fix search-path ambiguity through one owned schema policy,
with safe identifier handling and collision tests, rather than tenant machinery.

Acceptance: fresh install, populated legacy upgrade, repeated migration,
concurrent startup, injected migration failure, missing/wrong constraint,
newer-schema rejection, least-privilege runtime open, numeric extremes and
corrupt persisted values. Existing data and resume positions survive a supported
upgrade. Schema-changing releases include the migration in the published crate.

### PG5 Atomic writes and import

Implement existing `AtomicAppend::atomic_append_many` under the `import` feature.
One database transaction covers all runs. Follow the current owning contract:
validate distinct raw targets and complete run shapes before storage work;
duplicate routes and invalid runs are input rejections, never optimistic conflicts.
Validate each accepted run against its committed head. Return the highest assigned
`PgAllPos`; empty input returns `None`. Reuse single-stream validation and SQL
without introducing a second transaction or repository abstraction.

Acceptance: enable the shared atomic conformance suite, plus conflicting final
run, malformed middle run, repeated target with valid and stale versions (both
must reject as invalid routes before writing),
concurrent overlapping batches, reversed target order/deadlocks, cancellation,
and uncertain commit. A definite rejection leaves every target unchanged; an
unknown outcome is still all-or-none in the database. Conflict index and actual
head follow the owning trait. Notifications cover every affected subscription.
Exercise both existing import atomicity policies and their partial-progress
reports; whole-chunk import must not degrade into separate commits.

### PG6 Aggregate snapshots and projection state

Implement aggregate `SnapshotStore<Vec<u8>, Version>` and projection
`CheckpointStore<Vec<u8>, PgAllPos>` under `snapshot` and `projection` features.
Reuse the existing codec wrappers and owning `CheckpointWrite::next_revision` rule.
Save ID, schema version, state and its exact position in one atomic operation.
Keep aggregate and projection identities/position forms from colliding.
`Hydrated::Absent`, `Stale` and `Found` remain distinct; corruption is an error.

Acceptance: shared snapshot conformance for `Version` and checkpoint contract
tests for `PgAllPos`, empty
state, maximum supported positions and schema versions, overwrite behavior,
concurrent readers during writes, process restart, schema mismatch/rebuild and
failed/uncertain commits. Readers observe the old complete pair or new complete
pair, never a mixed state and position. Exercise real aggregate rehydration and
the existing projection stepper through these implementations. Preserve the
owner's revision comparison, strictly increasing positions, schema replacement
through explicit rebuild and revision-overflow rejection. Failed or cancelled
checkpoint writes require reload. Storage revision checks do not replace Labcast's
controller command/worker fencing.

### PG7 Export, backup and restore

Implement bounded `StreamLister` and `ConsistentExporter` under `export`;
enumerate each stream once for the documented scan visibility boundary. A
finite-lifetime `ExportSession` pins one database view for listing and every
stream read, including across concurrent atomic multi-stream commits. Expiration
must release the view without another caller poll; explicit close waits for
cleanup, and cursors borrow the session. Bound session count and retained resources.
Compose the existing exporter, completed format-2 CBOR
format and importer. Preserve versions, event types, schema versions, payloads,
metadata and caller-selected routing. Import assigns destination global positions;
source `$all` checkpoints are not portable across a logical restore.

Acceptance: Postgres export/import round trip and Fjall/Postgres interchange;
empty database, many streams, binary IDs, schema variants, malformed/truncated
backup, duplicate target routing and conflict near the end of a chunk. Exercise
bounded reads and failure reports instead of loading a backup into memory.
Test session cancellation, idle expiration, close, corruption and incomplete
backup rejection. Ordinary independent live stream exports keep their independent
views; consistent backup must use the session contract and completed chunks.

Provide an operator restore drill for events plus saved state and migration
metadata. Distinguish physical database recovery (which preserves transaction
history when restored consistently) from logical restore (which may invalidate
positions). In particular, copied `xid8` values from another cluster must not be
assumed compatible with its new transaction counter. Specify reimport/rebuild or
an evidenced translation procedure; verify subsequent append, `$all` progress
and projection restart after restore. Never resume an old checkpoint silently
against a different history. Document backup consistency, recovery point and
recovery time as deployment choices, with measured drill results.

The active history slice adds migration5 and a persisted16-byte history identity
to `PgAllPos` and projection checkpoint rows. Its breaking constructor requires
all three saved components. Native tests reject copied cluster/database bindings,
future transaction IDs (including the exact upper bound), and foreign positions
before pool acquisition; populated-v4 migration rollback/backfill and coherent
physical recovery are exercised. This slice merged in PR387. Completed bounded
CBOR interchange, projection restart and the full operator drill remain required.

### PG8 Secure operation, lifecycle and capacity

Keep caller-provided `PgPool` composition. Use SQLx's supported connection and
TLS options; select a TLS implementation explicitly and test certificate and
hostname verification. The production example must not silently fall back to
plaintext. Test authentication failure, wrong CA/hostname and secret redaction.
Specify a runtime role that cannot alter schema or rewrite/delete committed
events, with separate migration privileges and the required snapshot permissions.

Specify startup completion, listener readiness/degraded operation, pool close,
last-clone drop, active cursor ownership and bounded shutdown. A one-connection
pool must either work or fail configuration explicitly; a permanently reserved
listener must not starve all data queries. Test repeated open/drop, reconnect,
database restart, query cancellation, slow consumers, exhausted pools, lock
contention and a permanently unavailable server. Caller-owned pools remain owned
by callers. Define query/acquisition/lock deadlines through the owning SQLx and
Postgres controls, with PG3 preserving outcome uncertainty.

Use the existing tracing conventions and database statistics. Expose enough
evidence to distinguish append latency/conflicts/unknown outcomes, scan rows and
duration, listener outage/reconnect, fallback scans, pool wait and watermark
stalls. Bound label cardinality; omit secrets and payloads. Show an operator how
to investigate a stalled projection and identify long transactions without
granting every application a superuser role.

Document the durability profile: logged tables, appropriate WAL and storage
settings, local commit flush, and separately specified replication/failover
guarantees. Validate or reject incompatible session settings for the claimed
profile. An asynchronous durability setting must not silently receive the same
guarantee. Primary restart is required evidence; cross-node failover is a separate
support claim requiring its own tested profile.

Capacity evidence uses fixed page sizes and 10k/100k/1m-event histories, 1/16/128
concurrent consumers, short and long streams, small and large accepted payloads,
single-event and multi-event appends, and competing writers. Record hardware,
database settings, warmup, samples, p50/p95/p99, throughput, allocations, peak and
retained memory, connections, query plans and saturation. Include a 30-minute
mixed-workload soak with reconnects and consumer restarts. Require zero invariant
violations, bounded resources after quiescence, and the PG2 recovery bound.
Set deployment throughput/latency targets from PG0 measurements before tuning;
never invent passing numbers. Investigate repeatable regressions above the measured
noise floor and record explicit tradeoffs at equal durability.

### PG9 Verification and delivery

Keep tests with their existing owners. Add focused properties for page/resume and
error mapping, plus real database concurrency, corruption, cancellation, crash
and upgrade tests. Demonstrate that original source fails the new read and wake
regressions. Reuse existing fuzz/property facilities where useful; no new test
framework or orchestration binary. Mutation checks target meaningful invariants,
not a percentage manufactured by excluding the Postgres paths.

Required commands and CI evidence:

```sh
nix flake check -L
# Existing Linux integration output; must run the complete selected feature suite.
nix build .#postgres-integration -L
# Targeted development run in the Nix environment with an isolated test database.
nix develop --command cargo nextest run -p mnesis-postgres --all-features --test-threads=1
```

The current adapter tests return success without `DATABASE_URL`; that is not
database evidence. Make the required integration job fail if the database,
feature suite or expected test set is absent. Inspect the existing `tee` pipeline
so a failing nextest command cannot be masked. Add the database integration gate
to Linux flake checks through the existing derivation; retain the package entry
point. Darwin compilation cannot stand in for Linux database execution. Exercise
default, each advertised feature, all features, and an isolated downstream crate
so workspace unification cannot hide missing dependencies.

Use Nix for tools, Postgres and checks, preserving the repository-owned
`direnv allow` entry point. Reuse Crane/Fenix and supported Nix facilities. The
current NixOS harness embeds a Python test-driver snippet; record this existing
conflict with the standing no-project-Python rule and select supported Nix package
checks or small native commands when extending the gate. Add no Python files,
snippets, custom validation suite or replacement orchestration tool.

Ship a concise public Postgres example and operator documentation covering
configuration, migrations, limits, failure/retry semantics, snapshots, projection
rebuild, import/export, backup/restore and supported deployment profiles.
Examples demonstrate normal use; fault injection and assertions belong in tests.
Update adapter docs, public capability tables, changelog and any affected shared
contract docs together. Production readiness does not require a new kernel API
unless PG0 demonstrates a contract gap.

Release gate: every PG0–PG8 acceptance item links to its exact test or measurement;
all required checks pass at the reviewed revision; the final ownership/interface
audit removes redundant state, duplicate SQL policy and unjustified public types.
Perform two fresh audits under the research protocol. Any credible untested
high-value finding reopens the relevant item. Then record the PR, review, CI,
merge commit, published version and a clean registry consumer smoke test. A local
build, open PR, or publication of a different revision does not complete delivery.

## Hypotheses and falsifiers

| Hypothesis | Evidence that rejects it | Rollback boundary |
| --- | --- | --- |
| H1 Keyset pages preserve order while bounding memory | Missing/duplicated row, endless scan, resume query rescans history, memory scales with backlog | Read implementation and its index/query changes. |
| H2 Adapter-owned periodic recovery makes hints optional | Final missed hint or watermark release remains stuck; resources scale as one listener per subscriber | Wake transport/recovery changes only. |
| H3 Phase-aware errors preserve commit facts | Server-committed write reaches a retry-safe classification; wrappers erase uncertainty | Error contract and its affected mappings together. |
| H4 SQLx transactions and migrator cover needed persistence | Partial visible batch/state, unsafe legacy adoption, unsupported schema accepted | Atomic/snapshot or migration slice and matching SQL. |
| H5 Existing store traits compose the complete adapter | A real consumer cannot express a required operation without duplicating authority | Record the exact core gap before adding an interface. |

Compare bounded post-commit hints with transactional NOTIFY, keyset pages with
SQLx streaming, and the existing watermark with alternatives only if measured
blocking makes it unsuitable. Do not replace the established order with a raw
identity sequence: reversed commits can permanently skip lower sequence values.
Do not add a CDC service or serialize every writer merely to avoid investigating
the present contract. Each selected alternative needs maintained dependency,
correctness, migration and operational-cost evidence.

## Change budget and implementation sequence

PG0 pre-edit record: characterize retained read memory and lost final hints using
the real adapter and database. Add `tests/read_tests.rs` and
`tests/subscription_tests.rs`; production +0/−0, at most 300 test lines and zero
new public types. The read witness reuses the existing allocation-counting test
pattern; wake witnesses commit through SQL without a notification and drive the
actual subscription until parked. No production fault seam or new dependency.
The original read/wake code must fail before an implementation is selected.

Preparation uses the locked dev shell with its optional rust-analyzer package
filtered from `nativeBuildInputs` through `overrideAttrs`, leaving the pinned
Rust and verification tools intact. This avoids building an editor server solely
to execute tests. Postgres comes from the same locked Nixpkgs. No project setup
changes are made for this local preparation.

PG2 first experiment pre-edit record: the two final-hint and watermark-release
regressions fail on original source. Change `src/wake.rs` to wrap the existing
`WakeReg` and race its armed wake with a one-second Tokio timer. Expected
production +30/−8/net +22; one associated registration type, no new public
methods, dependencies, tasks or connections. Existing registry owns routing and
generation capture. The adapter wrapper owns only the independent rescan
deadline that the registry cannot express. Revert this file if the regressions
remain stuck or existing wake laws fail. Listener retry, resource lifecycle and
other PG2 cases remain required after this experiment.

PG1 read experiment pre-edit record: both read paths retain ten times more memory
for ten times more rows. Change `store.rs` and `builder.rs` plus native read tests;
expected production +130/−120/net +10, zero new public types, one constructor
overload accepting the existing `BatchSize`. Freeze a committed head or safe
transaction cutoff, then fetch one bounded keyset page at a time. Reuse
`futures::stream::try_unfold` and `BoxStream`: one explicit cursor allocation
avoids a custom public cursor/state-machine framework and releases pool leases
between pages. It erases only the database scan future, not typed events/errors
or codecs. Measure this choice; revert if memory, resume, finite visibility or
connection occupancy laws fail. Full byte-budget enforcement remains required.
Test budget: extend `read_tests.rs` by at most 180 lines for page edges, resume,
finite visibility, corruption and connection release, reusing existing database
fixtures and standard streams. Keep shared contract wording corrections scoped
to the recorded PG0 decisions; no core executable change.

PG2 lifecycle experiment pre-edit record: the first fallback experiment is
committed as `af6a43b4`; bounded reads are `755ec8eb`. Add native lifecycle
regressions before changing the listener. Prove initial acquisition failure with
SQLx's supported connection callbacks and a bounded acquisition deadline, then
restore service and require LISTEN to return. Also cover disconnect plus a final
unnotified commit, one shared listener across clones, last-clone cleanup,
caller-owned pool closure, malformed/duplicate hints and an overlong binary ID.
Reject a one-connection pool explicitly: SQLx's listener reserves a connection.
Expected changes: `store.rs`, `builder.rs`, `error.rs`, and a new
`tests/lifecycle_tests.rs`; at most +90/−30/net +60 production lines, no new public
type or dependency. Refined native-test budget: at most 600 lines for the
independent connection, LISTEN, snapshot, routing and cleanup cases with readable
formatting; the original 450-line estimate omitted that separation. Reuse the one listener task,
SQLx reconnect and close events, the wake registry and Tokio timers. Retry setup
and terminal receive errors with a one-second bounded delay until the pool closes.
No per-subscriber tasks or new lifecycle framework. Preserve the original errors
in diagnostics when PG8 adds the existing tracing feature.

Start with PG0 and the real failing witnesses. The first production slice covers
PG1–PG3 after their contract decisions: expected owners are Postgres `store.rs`,
`wake.rs`, `builder.rs`, `error.rs`, `position.rs`, adapter tests and manifests.
Touch shared error/contract or wake files only when PG0 proves the need.
PG1 byte-limit pre-edit refinement: add a private page-query owner and a `Limits`
configuration alongside the existing store/builder/error owners. Budget +260/−45,
net +215 production lines, one configuration type replacing the unpublished
batch-only overload, zero dependencies. Native tests: at most +350 lines for
large-row memory, exact byte edges, metadata/key accounting, whole-batch rejection,
oversized legacy rows and cancellation. Full falsifiers, comparison and actual
deltas are in [the research](postgres/research.md#pg1-byte-limit-experiment-pre-edit).

Revised first-slice budget: at most 14 changed implementation/test files, net +500 production
lines, and two new public types; record actual additions/deletions before edits
and at each checkpoint. Cursor associated types count in the public-surface audit.
Reuse existing position, batch, wake, pool and stream contracts. No new crate.

Do not disguise full completion as one small patch. After the first slice, write
an exact file/line/API budget for migrations, atomic import, snapshots, export and
operations before each production change. Keep both per-slice and cumulative
totals visible; exceeding the estimate requires an explained revised record and
scope review, not dropping acceptance criteria or moving code into a new tool.
Separate independent fixes into reviewable commits and migrations into explicit
compatibility boundaries. Do not use speculative wrappers to hide growth.

The [status index](../prd-backlog/status.md) is the sole dependency/status table.
Completion evidence stays below and in feature-local artifacts linked from it.

## Evidence and remaining work

[Characterization and contract decisions](postgres/research.md) records the
actual original failures, database profile, commands and selected first experiment.

| Evidence | Result on this branch |
| --- | --- |
| Source and contract inspection | Completed at the baseline revision; concrete findings and ownership recorded above. |
| Document verification | Local links resolve across the PRD, backlog, research records and adapter README; all 11 backlog rows have reciprocal dependency edges; whitespace checks pass. |
| Locked Nix package evaluation | `nix eval --raw --expr 'let f = builtins.getFlake (toString ./.); p = import f.inputs.nixpkgs { system = builtins.currentSystem; }; in p.postgresql.version' --impure` returned `17.10`. |
| Native defect reproduction | Existing 56 tests pass; new bounded-read, stream/global final-hint and watermark-release tests fail against original production source. See characterization. |
| Broader baseline and dependency audit | Pending PG0/PG8; individual read-memory and timing samples are characterization, not capacity evidence. |
| Production implementation and acceptance gates | Read/wake/outcome/migration/value/atomic/snapshot/checkpoint/listing/session/history, backup/recovery failure, query identity, and verified TLS/runtime role slices merged. PG7 operator measurements and PG8–PG9 remain; durability verification is active. Exact acceptance is tracked in the backlog and research record. |
| Merge and publication | Focused protected merges are recorded in the backlog. No whole-adapter release or registry-consumer verification yet. |

## Primary sources

Accessed 2026-10-18. Local source and locks above establish selected behavior;
upstream documents establish facility contracts, not passing adapter tests.

- [SQLx 0.8.6 PgListener](https://docs.rs/sqlx/0.8.6/sqlx/postgres/struct.PgListener.html): reconnect behavior, lost notifications, and pool closure. Supports the recovery requirement, not a guarantee that the current loop recovers.
- [PostgreSQL 17 LISTEN](https://www.postgresql.org/docs/17/sql-listen.html): listener startup ordering. [NOTIFY](https://www.postgresql.org/docs/17/sql-notify.html): transaction delivery and payload limits. Neither is a durable event log.
- [PostgreSQL 17 snapshot functions](https://www.postgresql.org/docs/17/functions-info.html#FUNCTIONS-PG-SNAPSHOT): `xid8`, transaction assignment and snapshot visibility. Does not establish physical commit ordering for the adapter's position.
- [PostgreSQL 17 error codes](https://www.postgresql.org/docs/17/errcodes-appendix.html): structured SQLSTATE classification. Transaction phase and observed server response remain necessary evidence.
- [SQLx 0.8.6 Migrator](https://docs.rs/sqlx/0.8.6/sqlx/migrate/struct.Migrator.html): existing migration facility to evaluate with the legacy schema.
- [SQLx 0.8.6 PgSslMode](https://docs.rs/sqlx/0.8.6/sqlx/postgres/enum.PgSslMode.html): supported verification modes; the selected TLS feature and real certificate tests remain required.
- [PostgreSQL 18 WAL settings](https://www.postgresql.org/docs/18/runtime-config-wal.html): durability depends on flush and replication configuration. Reference material; verify the selected server profile during PG0/PG8.
- [PostgreSQL 18 SQL dump](https://www.postgresql.org/docs/18/backup-dump.html): native logical backup facilities. This does not prove Mnesis checkpoint portability or safe restore of copied transaction IDs.
