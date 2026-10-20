# Postgres characterization and contract decisions

Baseline source: `747aea2881938d6a7f10a85aec1475bc8b8289e3`.
Date: 2026-10-18. Requirements: [PG0 and dependent work](../postgres.md).

## Environment and commands

Native aarch64 Darwin, Mac16,7, 14 logical CPUs, 24 GiB RAM. Rust 1.95.0,
unoptimized test profile with debuginfo. PostgreSQL 17.10 from the locked
Nixpkgs; fresh local cluster, UTF-8, C locale, Unix socket, trust authentication
for an isolated throwaway database. `fsync`, `synchronous_commit` and
`full_page_writes` are all `on`. This is local characterization, not the required
Linux release evidence or a network/TLS capacity profile.

The repository dev shell initially started building its optional editor server.
Use this temporary supported Nix override for test commands; it retains the
exact pinned toolchain and checks and does not change repository setup:

```sh
nix develop --impure --expr '
  let f = builtins.getFlake "git+file:///Users/joel/orca/workspaces/nexus/main";
  in f.devShells.${builtins.currentSystem}.default.overrideAttrs (old: {
    nativeBuildInputs = builtins.filter
      (p: !(f.inputs.nixpkgs.lib.hasPrefix "rust-analyzer" (p.name or "")))
      old.nativeBuildInputs;
  })
' --command env DATABASE_URL='postgres:///mnesis_pg0?host=/tmp&port=55432' \
  cargo nextest run -p mnesis-postgres --no-fail-fast --test-threads=1 --no-capture
```

The Nix store Postgres path is
`/nix/store/k5mjkij9d9fcikm1pshcads63fsyf08r-postgresql-17.10`.
Cluster: `/tmp/mnesis-postgres-pg0`; database: `mnesis_pg0`; socket: `/tmp`;
port: 55432; no TCP listener. Initialize with that package's `initdb` using
`-A trust --no-locale --encoding=UTF8`, then start with `pg_ctl` options
`-k /tmp -p 55432 -c listen_addresses=''` and create the database with `createdb`.
Do not reuse another application's database: the current tests truncate events.

Nextest runs serially with `--no-capture`; it reports that the additional
`--test-threads=1` flag is ignored in this mode. Tests are distinct subprocesses.

## Original defect witnesses

Production source was unchanged for all runs below. The new tests are native
adapter tests; no production fault seam or new dependency was introduced.

| Witness | Actual original result | Evidence |
| --- | --- | --- |
| Existing adapter suite plus first three regressions | 59 run: 56 existing pass, 3 regressions fail, 0 skipped, 7.453 seconds | `/tmp/mnesis-pg0-baseline.log` |
| Read first event from 4,096 rows, each with a 4,096-byte payload | Stream retains 17,919,936 bytes; `$all` retains 17,969,152 bytes; test budget is 4,194,304 bytes | `/tmp/mnesis-pg0-regressions.log`; `tests/read_tests.rs` |
| Stream subscriber parked before a commit without NOTIFY | Three-second delivery deadline expires | `/tmp/mnesis-pg0-wakes.log`; `stream_recovers_the_final_missing_hint` |
| `$all` subscriber parked before a commit without NOTIFY | Three-second delivery deadline expires | Same log; `all_recovers_the_final_missing_hint` |
| Actual append hint delivered while older unrelated transaction holds watermark; older transaction rolls back | Three-second deadline expires after watermark release | Same log; `all_recovers_after_an_unrelated_transaction_releases_the_watermark` |

The measured read sample took 23.760 ms to open/read the first stream event and
28.707 ms for `$all` after connection/statement warmup. These are individual
characterization samples, not comparative performance claims. Payload bytes total
16 MiB; retained values include envelopes, frames, keys and incidental client
state. The counting allocator measures live client allocations, not RSS, server
memory or total allocation traffic. It uses exact matching allocation layouts
and has no gate that could misattribute later deallocations.

The missing-hint tests use direct committed SQL to model the durable-write/hint
gap without a later write. `park` drives the actual generic subscription, first
observing `CaughtUp`, then preserving its pending unfold future through a timed
`next`. The watermark test additionally arms the real wake registration and
confirms the adapter's actual NOTIFY arrives before releasing the older
transaction. Thus notification transport can succeed while progress still fails.
No test claims that a subscription timeout means the write was absent. Captured
test logs are checked in under `evidence/`; the unrelated shell greeting is
omitted from nextest logs.

The 10× baseline input (40,960 rows) retains 179,236,800 bytes for the stream and
179,691,520 bytes for `$all`. `/usr/bin/time -l` reports maximum RSS of 50,970,624
bytes for 4,096 rows and 419,790,848 bytes for 40,960 rows. These separate process
runs demonstrate history-dependent client memory; setup and warmup are included
in process RSS. Raw original samples:
[small](evidence/original-memory-small.log) and
[large](evidence/original-memory-large.log).

## Selected contract decisions

1. Retain the existing lexicographic `(txid, seq)` stable delivery order and safe
   watermark. This is a linearization order consistent with non-overlapping
   appends, not the wall-clock order of overlapping commit acknowledgements.
   The existing reversed-commit test is direct evidence. Correct the shared and
   adapter wording without changing existing positions. A raw sequence cursor
   remains rejected because it can skip late commits permanently.
2. Retain all-or-none database atomicity. A conflict or proven server rejection
   means no writes from that transaction committed. A transport failure during
   commit is an adapter error preserving unknown outcome, not a conflict.
   Correct `AtomicAppend`'s unconditional “on any failure, no write is applied”
   wording; its generic `Store(E)` can retain the adapter's distinction without
   a new core outcome abstraction. Error wrappers already preserve typed E.
3. Keep full core `u64` versions and `u32` schema versions. PG4 must supply a
   lossless numeric representation for versions and stop narrowing transaction
   IDs through signed bigint. Use binary storage for UTF-8 event-type bytes so
   valid NUL-containing names are not silently excluded. Native Postgres field
   size limits remain explicit backend limits; resource limits need typed errors
   before writes and symmetric reads, never conflict/corruption misclassification.
4. A finite scan captures a stable upper boundary. Per-stream append-only
   history can be bounded by its committed head; global history can be bounded
   by a safe transaction watermark. Under the adapter's immutable-event rules,
   neither can gain an earlier row later. Paging therefore reconstructs the
   same frozen history without retaining a transaction that blocks vacuum or
   the watermark. Record this structural proof when reconciling the current
   broad “2+ database calls require one transaction” documentation. Mutable
   state and any read-then-write operation still require actual transactions.
5. Select a one-second fallback wake interval for the first recovery experiment,
   with a two-second allowance in this healthy local three-second deadline.
   Implement fallback in the Postgres registration's existing armed future;
   use Tokio's supported timer and wake race. This adds a timer per waiting
   subscription, not a spawned task or connection. Listener retry and pool
   lifecycle stay with the existing one-per-store task. No kernel runtime dep.

These decisions preserve the demonstrated user contracts and identify explicit
owner-document corrections. Shared position/atomic-failure wording and the
append-only scan exception are reconciled in owning source documentation.
PG3 still needs acknowledged-commit fault evidence before retaining a write
outcome implementation. Release-scale throughput and latency distributions,
compile/dependency cost comparison and two fresh final audits remain required.

## Initial read and recovery experiments

The Postgres registration now races its existing generation-safe wake with a
one-second Tokio timer. All three original wake regressions pass in 3.197 seconds
total. No task, connection, registry or kernel runtime dependency was added.
Listener retry and the remaining PG2 failure/resource campaigns are still open.

Reads now capture a finite committed head or safe transaction cutoff and use
bounded native-key comparisons. `futures::try_unfold` owns the asynchronous
scan, with one `BoxStream` allocation and no custom public cursor framework.
Rows are decoded as delivered; a failed page/item terminates the scan. A new
constructor accepts the existing `BatchSize`; no duplicate row-limit type.

The 63-test real-database suite passes, with zero skips: all 56 previous cases,
four original-defect witnesses and three new read campaigns. These cover page
sizes 1/2/256/4096, empty and boundary histories, inclusive stream bounds,
exclusive global resume at page edges, finite history across later appends,
later-page corruption, and a two-connection pool proving idle cursors release
the sole data-query connection. [Test output](evidence/bounded-tests.log).
Adapter test Clippy passes with warnings denied.

Retained first-event allocations are 1,086,920 bytes per-stream and 1,088,997 bytes
globally for both baseline sizes. Maximum process RSS is 11,288,576 bytes at
4,096 rows and 11,354,112 bytes at 40,960 rows.
[Paired measurements](evidence/bounded-memory.log). These are native debug samples,
not release throughput claims. Query plans, the practical byte budget, workload
distributions and release Linux evidence remain open.

Fresh `cargo audit` loaded 1,338 advisories and scanned 366 dependencies. It
reports one allowed unsoundness warning for event-listener 5.4.1,
[RUSTSEC-2026-0221](https://rustsec.org/advisories/RUSTSEC-2026-0221).
No dependency or advisory policy was changed. Assess its exact applicability
to the existing `Event<()>` use before release; do not call the lock warning-free.

Current changed Rust source files: +213/−161/net +52 including documentation;
core executable changes: zero. Public surface: one registration associated type
and one `BatchSize` constructor overload. Native tests: 465 lines across two
files, including 189 read-campaign lines versus the 180-line estimate after
formatting. The first experiments are committed separately as `af6a43b4` (wake)
and `755ec8eb` (reads), after the matching local gate passed. They are not merged
or released, and remaining feature acceptance is open.

The query-plan follow-up exposed two additional defects in the first read
experiment: a nullable resume predicate prevents a generic prepared plan from
using the resume key as an index condition, and `ORDER BY txid` selects the
cast output alias rather than the native table column. On 40,960 rows at a
40,000-row checkpoint, the nullable plan removes 40,000 rows by filtering and
touches 868 buffers. A strict resume predicate improves this; qualifying
`events.txid` also removes the sort, yielding an index scan that visits exactly
256 rows and 9 buffers. Early resume visits 256 rows and 7 buffers.
[Comparison](evidence/optional-resume-plans.log),
[native-column plans](evidence/native-resume-plans.log). These are controlled
`force_generic_plan` diagnostic samples, not throughput benchmarks; log padding
is trimmed. Source now uses SQLx's supported `QueryBuilder` to bind values and
emit separate initial/resume shapes without duplicated column definitions.

The fixed safe transaction cutoff already proves every paged row eligible;
recomputing the snapshot watermark per page is redundant and removed. Native
numeric output conversion remains pending PG4. Full source still fits the first
slice's production budget. Final formatting and targeted Clippy pass for this
refinement. The current real-DB rerun passes all 63 tests, zero skips, in 6.545
seconds.

The first full `nix flake check -L` passed on aarch64-darwin: all 12 configured
checks, including 1,358 passing workspace tests, four skips and fuzz replay.
Nextest took 315.695 seconds. It captured source before the query refinement and
does not certify the newer source. Known rustdoc warnings remain visible;
passing the gate does not mean the repository is warning-free. Log:
`/tmp/mnesis-postgres-flake-check.log`.

The final source-matched run passed in unified exec session 38340, logging to
`/tmp/mnesis-postgres-flake-final.log`. All 12 configured Darwin checks pass.
The SHA-256 of the Rust source diff under
`adapters/postgres/src` and `crates/store/src` at invocation is
`18dda4e3770c6f357ec1c4043e22654c444f1af5c3aff3f58ae5cc2cc5d4f762`.
This establishes the first read/wake experiment's local gate, not Linux delivery
or completion of PG1/PG2's remaining acceptance criteria.
Temporary shell invocations use an explicit Git source to avoid copying ignored
`target/` data. `/tmp/mnesis-postgres-shell` roots the realized Nix environment.

## Listener lifecycle experiment

New native tests reproduce two failures against `755ec8eb`: a one-connection
pool is accepted despite its dedicated listener, and a failed initial listener
acquisition never recovers after service returns. Six original lifecycle tests
run: four pass, two fail, zero skips in 5.380 seconds.
[Original output](evidence/original-lifecycle.log).

SQLx 0.8.6 source shows `PgListener::connect_with` acquires a pool connection;
its destructor issues `UNLISTEN *` before returning that connection. It retries
some transport disconnects internally, but setup errors and failed reconnect
acquisition can escape to the owner. The adapter now retries a complete listener
session after one second, or exits on the pool's existing close event. It rejects
pool capacity below two before DDL. Clones still share one existing task and
registry; no dependency, public type or additional task was introduced.

Eleven native lifecycle campaigns now pass as part of 74 passing adapter tests,
zero skips, 12.273 seconds. They cover acquisition and LISTEN setup failures,
terminal receive/reconnect failure, a disconnected final unnotified commit,
eight open/drop cycles with 128 clones and registrations, pool closure, hints
arriving before the first armed poll, malformed/duplicate/unrelated hints,
overlong binary IDs, and release of a held snapshot without a later hint.
[Current output](evidence/lifecycle-tests.log). Formatting and targeted Clippy
pass. The setup fault uses supported SQLx callbacks and a 100ms acquisition
deadline; the LISTEN fault returns a terminated backend from `before_acquire`,
proving the failure occurs after acquisition. Restored hints must arrive within
500ms, before the one-second fallback can mask a dead listener.

Stage Rust source delta: +37/−23/net +14 including docs; native tests: +539
lines, within the refined 600-line budget. Cumulative Rust source including the
first experiment: +250/−184/net +66. Public surface remains one registration
associated type and one batch-size constructor; this stage adds one typed
capacity-error alternative. The full source-matched Darwin gate passed in
session 45611: all 12 configured checks, 1,369 passing workspace tests and four
skips; nextest took 279.821 seconds. [Gate output](evidence/lifecycle-gate.log).
Full log: `/tmp/mnesis-postgres-lifecycle-flake.log`. Its Rust
source/test diff SHA-256 at invocation is
`5eda6d45949078d950302945d31ce21aa6bdac689aba6b4919882752337a2707`.
This retains the lifecycle experiment. Final production capacity evidence,
diagnostics and Linux release verification remain PG8/PG9 obligations.

Additional primary sources checked 2026-10-18:
[SQLx listener lifecycle](https://docs.rs/sqlx/0.8.6/sqlx/postgres/struct.PgListener.html)
and locked `listener.rs`/`pool/options.rs`;
[Tokio spawn](https://docs.rs/tokio/1.53.1/tokio/task/fn.spawn.html), which does not
poll a newly spawned task synchronously. This makes the current-thread startup
fault transition deterministic before the first listener attempt.

## Existing upstream work and facility comparison

- [Issue 348](https://github.com/devrandom-labs/mnesis/issues/348) already owns
  bounded keyset reads and batched append; reuse its scope and ownership.
- [Issue 268](https://github.com/devrandom-labs/mnesis/issues/268) owns TLS support
  and records the earlier certificate-license and feature decisions. Verify
  selected maintained TLS features and actual licenses before adopting one.
- [Issue 351](https://github.com/devrandom-labs/mnesis/issues/351) owns the live
  Postgres mutation campaign. Its absence from current mutation gates is a
  verification gap, not evidence that the adapter is mutation-verified.
- SQLx 0.8.6 `PgListener::try_recv` exposes reconnection via `None`; `recv` hides
  that transition. Neither supplies periodic scan progress. Tokio timer/select
  composition within the existing registration is sufficient for the proven gap;
  a scheduler service or new wake registry would add a second owner.
- SQLx transactions and migrator remain the selected facilities. Keyset pages
  must be compared with SQLx streaming for ownership and connection occupancy;
  do not invent a public cursor framework before the concrete experiment.
- PG4 preparation inspected locked SQLx `migrate.rs` and `migrator.rs`:
  `run_direct` owns a session advisory lock, while each Postgres migration owns
  its DDL/bookkeeping transaction. Error returns can precede unlock. Use an
  explicitly scoped connection that closes on drop when integrating migrations;
  do not return a session carrying an advisory lock or changed search path to
  the caller's pool. Do not wrap the whole migrator in an outer transaction that
  remains uncommitted after its advisory unlock. Concurrent and failed-migration
  tests must verify this integration. This is a facility constraint, not a
  completed migration implementation.

Primary facility sources and versions are linked in the parent PRD. Source and
advisory verification must be refreshed at the delivery revision.

## PG1 byte-limit experiment (pre-edit)

Threat: a bounded number of legal envelope rows still retains arbitrarily large
payloads and metadata. The default witness uses 24 rows of 1MiB each and requires
first-event retained memory below 20MiB for both scan paths. Original row-only
pages must fail it. Proposed default: 16MiB of row bytes per page; configurable
positive budget, row ceiling remains `BatchSize`. Count raw stream ID, event
type, payload, metadata and 32 bytes of scalar fields. Reject any individual row
exceeding the configured budget before append begins; legacy oversized rows
produce a typed limit error at their exact place, without fetching their body
or silently advancing. Consumers may reopen with an explicitly larger budget.

Compare supported Postgres window functions with a two-query header/body scan.
Select a bounded header candidate set, compute its ordered running byte sum,
then join only the fitting prefix to immutable events within one SQL statement.
Return a size diagnostic for an oversized first row, with byte columns NULL.
This reuses native SQL window/CASE evaluation and SQLx FromRow rather than a
custom public cursor framework or new dependency. Falsifiers: byte retention
still scales with page row count; metadata/keys escape accounting; read order,
resume, source error or connection-release laws regress; writes partially commit
before size rejection; or query plans scan the backlog.

Budget: existing store/builder/error/lib plus private limits/page owners, no more
than +260/−45/net +215 production lines, one configuration type replacing the
unpublished batch-only constructor, no dependency; native tests at most +240
lines. This refines cumulative first-slice budget to net +500 production lines
and 14 implementation/test files, explicitly including practical byte bounds.
Record actual delta and query plans before retention.

Primary sources checked 2026-10-18:
[Postgres 17 window functions](https://www.postgresql.org/docs/17/functions-window.html),
[conditional evaluation](https://www.postgresql.org/docs/17/functions-conditional.html),
and locked SQLx 0.8.6 `FromRow`/`QueryBuilder` implementations. Window sums operate
on header rows; CASE guards actual byte columns. No literal-error CASE branches
or aggregate expressions inside guarded branches are used.

Pre-implementation witness failed on `d4624e58`: 26,203,952 retained stream
bytes and 24,125,055 global bytes, versus a 20MiB test ceiling. See
[original byte witness](evidence/original-byte-limit.log). No row was individually
oversized; this proves the missing aggregate page bound.

Test budget refined before adding cancellation evidence: +350 lines, including
readable exact-edge, metadata/key accounting, atomic tail rejection, oversized
legacy-body exclusion and pool-wait cancellation witnesses. The earlier +240
estimate omitted Rust formatting and the independent cancellation law. Production
budget and dependency/public-type ceilings are unchanged.

Byte-limited implementation passes all 79 native Postgres tests, zero skips,
12.601 seconds; targeted Clippy and formatting pass. On the 24 x 1MiB witness,
retained stream/global bytes are 15,718,465/15,735,472. Small 4096-byte rows retain
1,088,719/1,090,052 bytes, preserving the earlier row-count bound. A raw legacy
8MiB row is reported exactly at version 3, with less than 1MiB extra retained
memory and a terminal error; explicitly raising the budget reads all four rows.
Binary keys, UTF-8 byte lengths, absent/nonempty metadata, exact acceptance and
one-byte rejection, whole-batch rejection and dropped pool requests pass.
[Native output](evidence/byte-tests.log).

The header-window query retains index seeks with forced generic plans. At early
and 40,000-row checkpoints in 40,960 rows, header selection visits 256 rows and
7/9 buffers. Payload lookup adds 256 primary-key seeks (768 buffers) and a bounded
256-row sort; execution samples are 0.272/0.279ms. This is a real extra lookup
cost, accepted provisionally for a single-statement byte bound that never fetches
rejected bodies. These diagnostic samples do not establish throughput or latency
capacity; PG8 must measure the tradeoff on representative data and load.
[Plans](evidence/byte-plans.log).

Actual byte-stage Rust source delta: +209/−85/net +124 including docs; tests
+347/−10. Cumulative production net +190, no dependencies; two public types
(Registration and Limits). `Limits` replaces the unpublished batch-only overload
and owns row/byte facts together. The matched full Darwin gate passed in
session 46146: all 12 checks, 1,374 passing workspace tests and four skips;
nextest took 301.257 seconds. [Gate output](evidence/byte-gate.log).
Full log: `/tmp/mnesis-postgres-byte-flake.log`, source/test diff SHA-256
`0644165e71d4a033df307670ae3027ddf38f8a5b0422f9e99e17d277a6222c5f`.
This retains the byte-limit experiment; Linux CI and delivery remain pending.

## PG3 transaction-outcome experiment (pre-edit)

Threat: all unique violations currently become retry-eligible conflicts, and
transport failure awaiting COMMIT has no distinct outcome. Reproduce unrelated
unique failure and suppress the server's actual `CommandComplete(COMMIT)` response,
then inspect committed rows using an independent control connection. Compare
ordinary SQLx callbacks and backend termination first: neither can selectively
hide a successful COMMIT response. Use a private test-only Unix socket forwarder
with existing Tokio I/O and workspace-managed tempfile; parse only the documented
backend frame boundary and command tag, forwarding all other bytes unchanged.
A normal forwarding control and before-BEGIN failure must establish the oracle.
No production fault seam, transport framework or new runtime.

Model unknown commit with a source-preserving Postgres error alternative. Keep
ordinary pre-commit errors and verified server rollbacks as definite failures;
never infer rollback from class 40 wholesale (`40003` means completion unknown).
Qualify stream/version conflicts by constraint identity. Bound post-commit hint
work and wake local registrations after confirmed commit. Cancellation delivers
no result, and an application timeout remains insufficient evidence of rollback.
Do not add retry policy or application command IDs to the adapter.

Budget: existing store/error owners plus a private transaction helper if needed,
at most +100/−25/net +75 production lines, zero public types and one existing
workspace dev dependency, at most 650 native test lines including the scoped
forwarder. Falsifiers: the proxy cannot prove its phase; durable rows exist after
a definite failure; any unknown outcome becomes conflict; an unrelated constraint
becomes conflict; a dropped future leaks a task/connection; or auxiliary hint
work can hold acknowledged success indefinitely. Atomic import and snapshot
paths must reuse the outcome rule when added in PG5/PG6.

Primary sources refreshed 2026-10-18: locked SQLx 0.8.6
`sqlx-postgres/src/transaction.rs` and `sqlx-core/src/transaction.rs` (COMMIT awaits
server responses; dropping queues rollback and does not undo an already committed
transaction), [Postgres error codes](https://www.postgresql.org/docs/17/errcodes-appendix.html)
(distinct `40001`, `40P01`, `40003`, `08007` and constraint-name fields), and
[backend message format](https://www.postgresql.org/docs/17/protocol-message-formats.html)
(CommandComplete includes a NUL-terminated tag). These support the planned fault
oracle and conservative classification, not a completed experiment.

Protected delivery inspection: main uses active ruleset 7146669 (classic branch
protection endpoint is absent). Pull requests, squash merges, resolved threads
and Nix Flake Check / Compile-fail tests / Feature isolation are required;
no bypass actors. The separate native-Postgres CI job must also pass by this PRD.

The current byte-limited source also passes the 40,960-row x 4,096-byte
allocation witness: retained stream/global bytes are exactly 1,088,719/1,090,052,
matching the 4,096-row workload. This run overlaps the full gate, so its elapsed
read times are not controlled latency evidence. [Output](evidence/byte-large-history.log).

PG4 preparation notes (candidate, not implementation): compare owned `mnesis`
schema with public tables and application-shared SQLx metadata. An owned schema
can isolate migration identity and qualify runtime SQL without mutating caller
search paths. SQLx's session lock must also cover first-time schema bootstrap;
`CREATE SCHEMA IF NOT EXISTS` alone is not proof against concurrent catalog
creation. Any nested use of SQLx's reentrant advisory lock must release by closing
the scoped connection, with a native concurrent-start witness. Runtime open must
validate without DDL, unknown versions/checksums must fail, and explicit migration
must adopt only a verified legacy shape. Do not build a second migration engine.

Lossless candidates: numeric(20,0) with positive/u64-range checks for versions,
bytea for UTF-8 event-type bytes (including NUL), and text parsing of native xid8
on output. Keep indexed native columns uncast. A supplied resume sequence above
native bigint capacity still has a valid lexicographic meaning: exclude that
transaction and seek later transaction IDs, rather than fail signed conversion.
These choices require exact round-trip, upgrade, rollback, least-privilege and
plan evidence before selection. Long incompressible stream keys remain a separate
credible native-index limitation to measure, not solved by event-type conversion.

## Reconciliation with the released 0.4.0 core

PR #376 initially targeted the older 0.3.1 baseline. Main advanced to `6faddb7f`
through #373/#374 and release #375. Incorporation preserves those changes and
Rust 1.99.0. One documentation conflict in AtomicAppend is resolved by keeping
both current distinct-target/run validation and remote commit uncertainty.
The PRD now follows the current owning projection checkpoint CAS and consistent
export-session/completed format-2 backup contracts; these remain required PG6/PG7
work, and standalone subscriber checkpoints remain conditional PG10.

The uncommitted PG3 experiment is preserved in Git stash
`postgres PG3 outcome experiment before PR376 base integration` on
`fix/postgres-outcomes`. It passed nine native phase tests on its pre-merge base;
reapply and verify after this integration. No phase experiment is included in
this read/wake PR. Incoming `scripts/check-features.py` and the existing NixOS
Python harness require replacement with maintained Cargo/Nix facilities in PG9;
preserve their actual coverage and required check names. Cargo-hack's supported
`--each-feature --no-dev-deps` is a candidate (default, no-default, all and each
feature); verify locked Nixpkgs fitness before adopting it. No replacement custom
script or validation framework is authorized.

The integrated 0.4.0 / Rust 1.99.0 source passes formatting, strict targeted
Clippy and all 79 native adapter tests, zero skips, 12.568 seconds.
[Output](evidence/core04-native.log). The matched full gate is running in unified
exec session 48147, `/tmp/mnesis-postgres-core04-flake.log`. Source/core-contract
Rust diff SHA-256 relative to `origin/main` (`6faddb7f`) at invocation:
`1bdafa8dde90f40343ba98e283404906edf55dc4f1a0296ca113b4c8ac186153`.

Native representation diagnostics on the integrated legacy schema reproduce four
concrete limits without retaining mutations (savepoints and final rollback):
u64::MAX version and xid8 signed output both fail SQLSTATE22003; UTF-8 text NUL
fails22021; a deterministic incompressible 5,120-byte stream key fails54000 because
the unique B-tree tuple is5,144 bytes versus2,704 allowed. The earlier 5,000-byte
hint witness is compressible and cannot certify arbitrary binary-key support.
[Exact server output](evidence/representation-diagnostics.log). These are storage
facility diagnostics, not yet adapter-level PG4 regression tests.

PG4 must resolve the key limitation without using a digest as identity. Candidate:
a native hash index over a stream table's raw byte key for equality lookup (hash
collisions are rechecked by Postgres), a compact assigned stream number for the
unique event/version arbiter, and transaction advisory locking of lookup/create.
Hash collisions may serialize unrelated creations but must never merge identities.
Compare its ownership, migration, query/insert cost and native collision/concurrency
proof against alternatives before selecting it. Keep one authoritative raw key;
do not add a duplicate event key plus surrogate without an enforced relationship.
Preserve sequence/global positions and adopt legacy data atomically through SQLx
migrations. The current target permits binary IDs beyond NOTIFY's size ceiling,
so an arbitrary small key limit cannot silently waive that acceptance criterion.

A better maintained facility emerged before implementing that advisory-lock
candidate: Postgres17 supports `EXCLUDE USING hash (key WITH =)` as an equality
constraint. A native temporary-table probe accepts the incompressible5,120-byte
key, rejects an exact duplicate with23P01, and supports `ON CONFLICT ON CONSTRAINT
key_probe_key_excl DO NOTHING` without altering data. Crucially, two distinct
keys with the same actual index hash both insert: collision-44072 and
collision-102013 share -2078099230. An index query returns the correct one and
reports one row removed by index recheck, proving identity remains raw-byte
equality. [Probe and plan](evidence/hash-exclusion-probe.log); only the generated
large-key error detail is trimmed.

This supersedes a custom advisory-lock stream-creation protocol as the preferred
PG4 candidate: use native exclusion for raw-key uniqueness and compact numeric
identity for event/version B-tree lookup, then measure full adapter fitness.
No new extension or library is needed. The probe is not a completed migration or
adapter concurrency proof. `DO UPDATE` does not accept exclusion arbiters; select
or insert with exact-constraint `DO NOTHING` and handle creation races explicitly
without replaying writes. PostgreSQL17 CREATE TABLE exclusion documentation and
its index recheck behavior were checked2026-10-18:
https://www.postgresql.org/docs/17/sql-createtable.html#SQL-CREATETABLE-EXCLUDE .
The first attempt to call hashvarlena(bytea) directly failed because its SQL
argument is internal. The retained probe uses `hash_record(ROW(bytea))` solely to
find candidates and verifies the collision through the actual index recheck;
production does not depend on either hash value or internal hash function API.

## Three-pass case review after every test run

User direction added2026-10-18: review test cases three times after every run.
A passing command is insufficient; retain the three separate reviews and any
findings before using a run as acceptance evidence. New source changes require
another run and the three reviews again. Do not lower assertions or skip a case
to clear a failure. Workspace runs without DATABASE_URL are not native database
proof; their vacuous adapter returns must stay explicit.

Latest completed native run: integrated0.4.0,79 cases, zero skips,12.568s,
`evidence/core04-native.log`.

1. Contract review: inspect raw read inclusive/exclusive bounds, finite visibility,
   attribution, row/byte budgets, pool ownership and independent notification
   progress against PG1/PG2. New small-budget tests cover keys, UTF-8 bytes,
   metadata, exact acceptance, oversized tails and terminal legacy errors. Full
   capacity/delivery and PG3+ remain open; passing79 cannot certify them.
2. Falsifier review: original whole-backlog/large-row regressions and final-hint
   failures were recorded against old production. Hints are omitted by raw SQL
   commits, not randomized sleeps; short hint deadlines cannot be satisfied by
   the one-second fallback. Setup faults use SQLx callbacks/terminated backends;
   restored service must establish LISTEN again. The existing query plans also
   verify strict native index seeks at late checkpoints.
3. Loophole review FOUND gaps to strengthen: the oversized-body assertion counts
   retained allocation only, so a fetch-then-free implementation could evade it.
   Track peak allocation during that request as well. The cancellation fixture
   currently accepts any LISTEN backend when proving pool occupancy; give its
   own pool a unique application name and select that listener. Neither finding
   permits weakening the contract; strengthen and rerun after collecting the
   current integration gate. Do not describe retained-memory evidence alone as
   a peak/transfer measurement.

Review of the preserved PG3 cases also requires matching the actual EOF source
on both definite pre-commit failures and CommitUnknown, and proving propagation
through real StoreError/repository consumers. The deferred-trigger tests are
code-mapping tests, not real deadlock/SSI tests. These remain explicit follow-ups
when the stash is reapplied on the reconciled base.

Three-pass review of `hash-exclusion-probe.log` (completed native facility probe):
1. Contract: this tests equality uniqueness and large-key representation only;
   it cannot certify migrations, adapter transaction races or least-privilege
   deployment. All objects are temporary and disappear with the connection.
2. Falsifier: the same5,120-byte key that fails B-tree insertion succeeds here;
   its duplicate fails23P01 and exact-constraint DO NOTHING leaves count1. Two
   different keys sharing an actual hash insert independently and remain separate.
3. Loopholes: candidate hashes alone would be insufficient, so the retained plan
   proves two index hits and one equality recheck rejection. No digest substitutes
   identity. The superuser probe establishes no permission claim, and its single
   latency sample establishes no capacity claim. Adapter-level races and recovery
   remain required. No false acceptance claim is retained from this probe.

Test-strengthening pre-edit record after the three-pass review: add peak live
allocation tracking to the existing native read-test allocator and reset its
peak around the oversized-body request. Require both peak and retained deltas
below1MiB for an8MiB rejected body. Give the byte-test pool a per-process unique
application name and query that exact name before occupying its remaining slots.
Test-only budget +25/−5 lines, no production/dependency/public API change. Native
negative control: temporarily remove the payload CASE guard, confirm that the
same test rejects the fetched-and-discarded oversized body, then restore source.
Review cases three times after that expected failing run and after the repaired
run; retain the failure output. Do not change the memory threshold to make it pass.

The integrated0.4.0 full Darwin gate passed all12 configured checks,
1,524 workspace tests/four compiler exclusions, nextest453.504s.
[Output](evidence/core04-gate.log). Three-pass review:
1. Coverage: workspace suite covers the integrated owners, but adapter cases
   without DATABASE_URL are vacuous; native79 proof stays separate. Compiler
   exclusions require the protected Compile-fail tests job, not a waiver.
2. Regression: read/wake original failures remain recorded and native79 passed
   on this exact core. This supports integration only, not PG3–PG9 completion.
3. Loopholes: the earlier peak-memory and listener-identity findings remain open.
   Strengthen those cases next and repeat verification. This gate does not
   certify the strengthened source that has not yet been written.

Original PR376 head674ed905 passed Linux Nix Flake Check and Postgres Integration
in [run39103105302](https://github.com/devrandom-labs/mnesis/actions/runs/39103105302).
Three-pass review: this is old-head coverage only; the workspace summary is1,374
passes/four compiler exclusions; native VM script completes in52.41s. Its pipeline
can mask a failing nextest exit and the normal log does not show individual native
verdicts, so job success alone is insufficient final native acceptance. Preserve
native local79 evidence; replace the harness with fail-closed standard Nix/Postgres
commands in PG9, preserve required CI status names and verify exact native counts.

Strengthened read cases:9 native cases passed, zero skips,0.773s; strict targeted
Clippy/fmt passed (`/tmp/mnesis-read-oracles.log`). Three-pass review:
1. Acceptance: both retained and peak allocation are bounded for the rejected8MiB
   body; cancellation waits for the pool's unique application-name listener.
2. Falsifier: page/resume/finite/order and byte-edge assertions still execute;
   original failure evidence is unchanged. A payload-guard mutation is next, so
   instrumentation sensitivity is not yet claimed from this passing run alone.
3. Loopholes: peak resets after fixture insertion, directly before the two failing
   read requests; it includes temporary allocations and cannot be satisfied by
   freeing the body before the assertion. Per-process test isolation and current-
   thread runtime bound unrelated allocations. Neither threshold nor skip guard
   was weakened. This remains targeted evidence pending mutation/full checks.

Payload-guard mutation: the real native case fails as required,0 passes/1 expected
failure,0.120s. [Output](evidence/body-guard-mutant.log). Three-pass review:
1. Acceptance: both requests still report RowLimit; the failed invariant is actual
   body memory, not a SQL syntax/setup failure or changed expected error.
2. Falsifier: only the production payload CASE guard was removed. The driver
   retains16,747,385 extra bytes and the test rejects the mutation.
3. Loopholes: this particular driver's retained buffer already trips the older
   assertion before peak is checked. It does NOT prove peak sensitivity by itself.
   Add a small freed-allocation calibration to the same measurement fixture, then
   reset before real requests. Restore the guard exactly; keep both thresholds.
   Refine this test-only stage budget to+40/−5 lines to include that calibration
   and explicit metric output. No production change is retained from the mutant.

Restored guard plus strengthened oracles: all79 native cases pass, zero skips,
12.654s; fmt/strict targeted Clippy pass. [Output](evidence/strong-native.log).
Actual test-only stage+35/−13 lines; production byte guard restored unchanged.
The rejected8MiB row adds zero retained bytes and5,904 peak bytes; the fixture
first observes a2MiB allocation after that allocation has been freed.
Three-pass review after this run:
1. Contract: inclusive/exclusive page edges, finite order, attribution, byte-edge
   writes and terminal read rejection still execute across all79 cases. Both
   read paths remain below the unchanged1MiB peak/retained error budget.
2. Falsifier: removing the real payload guard fails the real native case; the
   freed-buffer calibration would fail if peak updates stopped. Exact listener
   identity replaces the old any-listener condition. No production fault seam.
3. Loopholes: default byte budget is reused from Limits rather than duplicated;
   thresholds, native guard and all order/error checks remain. Filtered counts in
   mutant runs are not claimed as native passes. The full source-matched gate
   must be repeated because these assertions changed after the integration gate.

Strengthened source-matched full gate passed in session98721: all12 Darwin
checks,1,524 workspace passes/four compiler exclusions,470.744s nextest.
[Output](evidence/strong-gate.log). Source diff relative6faddb7f:
`3f6f3b4697bbaa5d4788b4eb1304f0449c64764f93f88b899bdc844df405087c`.
Three-pass review:
1. Matrix/contract: all configured checks passed; real79-case native proof is
   separate from DB-free workspace returns. Compiler exclusions still require
   the protected compiler job. No secure-operation/full-capability claim follows.
2. Regression: the exact source includes the calibrated peak/own-listener cases;
   the real payload-guard mutant was killed and then reverted. All original
   read/wake witnesses and independent native run remain linked above.
3. Loopholes: full-gate source hash matches the retained Rust; no thresholds,
   cases or CI requirements were removed. Worktree PG3 changes were isolated and
   cannot be credited to this gate. Linux checks must rerun on the new PR head.
   No additional gate-local test weakness was found in this pass; later capacity,
   fail-closed CI and PG3–PG9 acceptance remain explicitly open.

## Native mutation CI correction (pre-edit)

PR376 mutation job failed in run39105034067:27 candidates,14 missed,13 unviable;
its unmutated baseline reports0s tests. The job supplies no DATABASE_URL and uses
`--exclude mnesis-postgres`, yet --in-diff still selects Postgres source. These
results cannot grade its native cases. Preserve the failed job as evidence,
start real locked Postgres in the existing Nix environment, remove the exclusion,
and serialize nextest cases sharing the database. Keep cargo-mutants' measured
timeouts and existing aggregate verdict. Do not clear red by excluding source,
ignoring survivors, lowering thresholds or calling timeouts caught mutants.

Budget: at most+45/−10 workflow lines,+1 Nix development-package entry, no Rust,
new script, runtime, framework or dependency. Use standard initdb/pg_ctl/createdb,
a fresh Unix-only socket directory and trap cleanup. Run the same in-diff sweep
locally against real Postgres before retention; review every result under three
lenses. First verify the Nix-provided executable: `cargo mutants` locally selected
an unrelated broken~/.cargo/bin plugin (missing historical libiconv). Invoke the
existing Nix cargo-mutants binary directly, as the archive job already does for
cargo-nextest; do not install/repair host tools or change personal Cargo state.

Real-database diff sweep completed: unmutated baseline79/79, zero skips,12.613s;
27 candidates in2m:14 caught,13 unviable, zero missed/timeouts.
[Individual verdicts and failing cases](evidence/native-mutation.log).
Three post-run case reviews:
1. Acceptance: all79 native cases really ran; each of14 viable mutations failed
   an observable capacity, ordering, byte-bound, listener or wake assertion.
2. Falsifiers: exact byte equality rejects both excessive and overstrict limits;
   removed size validation is caught by rejected-tail atomicity. Disabled wake
   fails the captured-generation deadline; disabled listener fails its exact
   lifecycle witness. SQL payload-guard mutation remains a separate manual test
   because cargo-mutants does not rewrite SQL string bodies.
3. Loopholes: all13 unviable cases fail Rust compilation (missing Default,
   private constructors or mismatched types); none counts as caught. Fail-fast
   cancellations after the first genuine assertion are not skipped baseline
   tests. No survivor, timeout, filter change or reduced assertion was hidden.

CI-only full Darwin gate passes with all12 outputs reused unchanged. Three
reviews: source/test hash is unchanged; native mutation proof is separately
recorded; cached checks are not claimed as a fresh execution or Linux proof.
The first local startup smoke command failed due to quoting added in that
one-off command, before starting a server. Validate the workflow's exact heredoc
instead, preserving its original correct `-h ''` argument and cleanup trap.

Exact workflow heredoc validation on the locked default Postgres18.6 succeeded:
fresh cluster, Unix-only socket, new empty database,27 mutants in2m:14 caught,
13 compile-unviable,zero missed/timeouts; baseline79/79,zero skips.
[Output and individual assertions](evidence/fresh-mutation.log). Server stopped
and temporary cluster directory removed by the original EXIT trap.
Three post-run case reviews:
1. Acceptance: the fresh database actually executes all79 native cases; every
   viable mutant fails. This independently checks the CI startup path and newer
   locked default server, beyond the earlier17.10 existing-cluster run.
2. Falsifiers: exact caught-mutant list equals the17.10 run; inspected first
   failures still exercise capacity/byte-limit/order/listener/generation laws,
   not connectivity errors or an unavailable database.
3. Loopholes: zero missed/timeouts and identical13 compiler rejections; compile
   failures are never counted as caught. Only local diff/output paths changed
   when extracting the CI heredoc. No test source, assertion, timeout threshold
   or skip behavior changed. Native Linux CI still must execute the correction.
PG3 original-source results at `674ed905`: five of six phase tests pass, one
fails. The unrelated event-type unique constraint becomes `AppendError::Conflict`,
misclassifying a permanent schema rejection as optimistic contention. The scoped
Unix forwarder observes and suppresses the real COMMIT tag; an independent
connection counts all three rows while append returns ordinary `Sqlx(Io(...))`.
Dropping BEGIN/INSERT acknowledgements leaves zero visible rows; transparent
forwarding succeeds. Holding a successful COMMIT response permits an application
timeout while all three rows remain durable. [Output](evidence/original-outcomes.log).
The retained acknowledgement assertion will require the explicit unknown variant.

PG3 targeted implementation currently passes all nine outcome tests, zero skips,
0.577 seconds, with formatting and strict targeted Clippy. Added confirmed-success
versus stalled hint acknowledgement, unrelated identity-sequence collision, and
deferred server-code mapping. The latter injects `23514`/`40001`/`40P01`/`40003`
in a deferred trigger to verify commit-phase error classification; it does not
claim a true SSI/deadlock workload. All durable-state checks use an independent
control connection. Full merged-base verification remains required.

Delivery integration: PR #376 opened at `674ed905`. Main advanced through the
non-Postgres audit (#373/#374) and v0.4.0 (#375) after this branch's baseline;
GitHub reports merge conflicts. Preserve the uncommitted PG3 experiment in a
named Git stash while incorporating current main into the read/wake PR, then
reapply PG3 on the reconciled base and rerun its tests. Preserve the upstream
contracts and release versions. No overwrite of upstream audit work is intended.

PG3 wrapper-test pre-edit refinement on the isolated0.4.0 checkout: reuse the
existing Counter domain (path-only mnesis-test-domains dev dependency with serde)
and existing Store JSON feature for a real typed repository execute/reload case.
No fake store, codec or new registry crate. Budget adds one existing test-domain
dependency and one existing codec feature edge beyond tempfile, at most+70 test
lines within the650-line outcome budget. Require identical EOF cause before and
after COMMIT, preserved CommitUnknown through ExecuteError/StoreError, false
is_conflict, unchanged in-memory root despite durable data, and explicit reload
that reconciles that root. Do not resubmit the command to demonstrate recovery.

Integrated PG3 native run:89 cases pass, zero skips,39.512s; fmt/strict targeted
Clippy pass. [Output](evidence/pg3-native.log). Three-pass review:
1. Contract: exact EOF cause is retained in definite BEGIN/INSERT failures and
   CommitUnknown; independent control proves0 versus3 committed rows. Real typed
   execute preserves uncertainty/is_conflict=false and leaves root unchanged;
   explicit load restores the one durable event without replaying the command.
2. Falsifier: original unrelated-unique case failed against674ed905; intended
   constraint and PK-sequence failures now stay SQLx errors. Actual COMMIT tag is
   suppressed and all rows remain visible independently; delayed hint response
   cannot hold confirmed success. Deferred SQLSTATE injection is a mapping test,
   not a true deadlock/SSI workload. All prior79 raw/wake laws still pass.
3. Loopholes FOUND: BEGIN-response loss is during BEGIN, not a pool failure before
   any BEGIN. Add closed-data-pool coverage explicitly. The timeout case keeps a
   pinned append in scope until function end; put it in a nested scope so the
   underlying future is actually dropped before the final durable-state query.
   These strengthen phase/cancellation evidence without weakening expectations.

PG3 strengthened integrated run:90 native cases passed, zero skips,15.498s;
formatting/strict targeted Clippy pass. [Output](evidence/pg3-final-native.log).
Three-pass review:
1. Acceptance: closed pool proves a definite failure before BEGIN; lost BEGIN
   and INSERT responses preserve exact EOF, with zero durable rows. Lost COMMIT
   response preserves the same EOF under CommitUnknown, with all3 rows durable.
   Unrelated constraints never become conflict. Typed execute and reload preserve
   the unknown outcome and restore the one durable event without a duplicate.
2. Falsifier: actual server COMMIT tags and independent row counts establish the
   phase; no arbitrary backend termination substitutes for successful commit.
   Stalled-hint case must observe the held hint response, so skipping the hint
   entirely cannot satisfy it. Original false-conflict and payload-guard mutation
   failures remain recorded. All prior79 cases still pass on0.4.0.
3. Loopholes: the pinned append now ends its nested scope before the final query,
   actually dropping its underlying future. The known closed-pool cause differs
   from transport uncertainty. Deferred-trigger codes remain mapping-only evidence;
   true deadlock/SSI, atomic import/projection reuse and full gate are still open.
   No test threshold, error expectation or skip guard was weakened.

The reused pinned1.99.0 dev environment is rooted at
`/tmp/mnesis-postgres-shell-199`; `nix develop <profile> --command ...` is the
standard Nix facility and was verified to run rustc1.99.0. It avoids repeated
flake evaluation for one-off checks without changing repository setup. The
isolated source still uses the same locked inputs and Cargo facilities.

PG3 full Darwin gate passes all12 checks,1,535 workspace tests/four compiler
exclusions,453.086s nextest. [Output](evidence/pg3-gate.log). Phase Rust/test/manifest
and lock diff SHA-256 relative to its frozen read/wake baseline:
`3f681b11f5b4c68b9986d70adf952e66c998a774bff3ba0de1c40a581bbcbc1c`.
Three-pass review:
1. Matrix/contract:11 outcome cases are included (1,524→1,535);90 real native
   passes are separate from workspace DB-free returns. PG5/PG6 outcome reuse,
   real deadlock/SSI and delivery remain required before PG3 feature acceptance.
2. Falsifier: acknowledged-COMMIT suppression, exact EOF, independent row counts,
   false-conflict failures, scoped cancellation, bounded hints and real repository
   reload remain operative. The original unrelated-constraint failure is retained.
3. Loopholes: source diff matches the completed gate; schema/CI work in separate
   checkouts cannot be credited here. No new gate-local weakness was found after
   the last two native-case improvements. Keep the original named stash until
   the focused delivery commit is safely retained on the reconciled base.

PG3 delivery checkout reuses all12 Darwin outputs after applying only the phase
patch onto reconciled headc5a5ac34. Three post-check reviews: (1) source, tests,
manifest and lock hash remains exactly3f681b11f5b4c68b9986d70adf952e66c998a774bff3ba0de1c40a581bbcbc1c;
(2) real90-case phase/regression proof remains unchanged, including failed
original unique classification and acknowledged-COMMIT suppression; (3) cached
checks execute no cases and are not additional native or Linux evidence. Both
append-only research histories were preserved when integrating the phase patch.
PR376 head8357c0c2d53edcdc91f6de7795e6ec8ef44b6632:40 checks passed, one
expected nightly-only job skipped; GitHub merge state CLEAN, no reviews/threads
blocking. Three post-CI case reviews before protected merge:
1. Acceptance: protected Linux Nix gate1524 passes/four compiler exclusions;
   separate compiler jobs execute their compile-fail cases successfully. Feature
   isolation, native integration, deny, fuzz and CodeQL jobs pass.
2. Falsifiers: corrected CI mutation path starts/stops a real fresh Postgres,
   baseline14s tests;27 mutations yield14 caught/13 compiler-unviable, no survivors
   or timeout. Same source already passed all79 local native laws and the manual
   SQL payload-guard mutation. CI fix changes no Rust assertions or limits.
3. Loopholes: the existing NixOS integration output is cached and its tee pipeline
   can mask a future nextest failure; it is not fresh fail-closed native evidence.
   PG9 must replace that harness and verify exact native counts. Independent
   fresh-database mutation CI uses strict shell failure handling. This merge
   delivers only the bounded-read/recovery slice; PG3–PG9 and publication remain.

Initial read/wake slice delivered by protected squash PR376 on2026-10-18 at
07:23:40UTC, merge7db7cf83f74d9ae53b45ca9aae6c548572c73e91. CI record:
[evidence](evidence/pr376-ci.log), workflow run39106192841 and native mutation
run39106192660. No admin bypass, direct main write or protection change.
PG3 rebased as4b8f3ba9eee25d52317e17be60180063001ff6b3; its adapter/manifest/lock
diff hash relative merged main remains3f681b11f5b4c68b9986d70adf952e66c998a774bff3ba0de1c40a581bbcbc1c.
PG1/PG2 final capacity/recovery acceptance remains open underPG8/PG9. No version
publication or clean registry-consumer proof is credited to this initial merge.

Post-rebase PG3 gate passes with all12 outputs reused. Three reviews: (1) exact
adapter/test/manifest/lock hash remains3f681b11f5b4c68b9986d70adf952e66c998a774bff3ba0de1c40a581bbcbc1c;
(2) unchanged native90-case evidence and original COMMIT/constraint failures still
apply; (3) cached checks are not a fresh native run, and new Linux PR377 jobs
remain required. PR376 CI/delivery records are added without changing that code.

PR377 real-database mutation CI run39107773089 FAILED:10 candidates,6 caught,
3 unviable,1 missed: replace is_stream_conflict with false. Three post-run case
reviews: (1) negative unrelated-unique cases pass, but no forced native positive
uniqueness collision grades the classifier; (2) existing concurrent writers can
serialize before their head reads, yielding a preflight conflict and missing the
actual INSERT collision; (3) do not exclude the survivor or soften the gate.
Add a real uncommitted competing row, observe SUT INSERT waiting on its native
transaction lock under a unique process application name, then commit the winner.
Check both empty and existing heads, exact conflict metadata and unchanged durable
winner/prefix. Budget+110 native test lines within650 total, production+10/-10
only if this exposes incorrect diagnostics. Run the new case before production
changes, retain its failure, then require full native/mutation/gate proof again.

Forced positive-collision characterization FAILS on the current PR code: after
observing the actual INSERT blocked on the winner's transaction, it returns
Conflict but expectedSome(Version1) rather than the caller's expectedNone. Three
post-run reviews: (1) the empty-head metadata assertion is the exact failure;
(2) independent durable rows contain only the committed winner and the native
lock guard rules out serial preflight conflict; (3) the existing-head loop was
not reached after this first failure and is not credited. Preserve known caller
head in native conflict diagnostics; actual remains unknown until reconciliation.
Pass that known value through the existing private insert helper (+4 lines),
then rerun both head scenarios and the missed classifier mutation unchanged.

Native after preserving caller expected head:91 pass/zero skips,16.787s. Three
post-run reviews: (1) both empty and existing heads reach the actual INSERT lock,
return exact caller expectedNone/Some1 with unknown actual, and retain only the
winner plus known prefix; (2) original metadata witness failed on Some1 versus
None and the independent lock/data oracle rejects serial preflight substitutes;
(3) strict Clippy still rejects the new case length95>80 and one redundant
closure. Extract the native waiting observation into its own private test helper,
retain every assertion and both iterations, then falsify the previously missed
classifier replacement and rerun strict/full verification. This native pass
alone does not authorize retention while Clippy/gates are red.

The previously missed false-classifier mutant now fails on actual23505 from
public.events/events_stream_id_version_key (0/1,11 filtered,0.044s), after observing
the INSERT's native wait. Three reviews: (1) failure is Store(Sqlx23505) versus
required Conflict, not setup/build; (2) exact table/constraint and unchanged
independent winner rows prove the intended positive path; (3) source restored
byte-for-byte, no exclusion/threshold change, and filtered cases not credited.
Test helper extraction keeps both scenarios and every assertion; strict Clippy
now passes. Actual file640<=650 native-test budget. Full native91 and complete
automatic in-diff sweep must rerun before retaining this correction.

Refactored native run:91 pass/zero skips,13.671s. Automated diff sweep:10 mutants,
7 caught/3 compile-unviable,zero missed/timeouts; baseline91 pass in12.867s.
Three post-run reviews (each run):
1. Acceptance: every native case ran; both expected-head scenarios preserve exact
   caller metadata and independent committed rows. Mutation verdicts identify
   actual compilation versus assertion failure, never classify unviable as caught.
2. Falsifiers: false match guard reaches the new forced-collision case; false
   classifier happens to hit actual23505 in the older concurrent law this time.
   True classifier/guard fails unrelated PK, commit-code boolean mutation fails
   rejection mapping, and missing hints fail the actual fault-observation guard.
   Manual false-classifier proof specifically reached the new deterministic case.
3. Loophole FOUND: I ran these two suites concurrently in different databases of
   ONE cluster. The cluster-wide snapshot watermark can cause interference even
   with separate tables. A native probe confirms a held22861 in mnesis_pg0 pins
   mnesis_pg_mutants cutoff exactly22861; releasing it advances the cutoff. All
   cases passed and inspected mutant failures are relevant, but use a fresh
   isolated-cluster sweep as primary evidence and rerun the91 suite without a
   competing cluster peer. Never run independent native suites concurrently on
   one cluster again. CI jobs already initialize separate clusters. This also
   creates an explicitPG8 shared-cluster lag/operations obligation; no unsafe
   watermark shortcut is authorized.

Cross-database facility probe completed successfully: held22861 pins another
DB's cutoff22861; after rollback cutoff22862, exact booleans asserted by psql.
Three post-run reviews: (1) writer and observer use different named DBs in the
same17.10 cluster; (2) observation waits for the writer's actual PgSleep after
assigning its XID, and pin disappears on rollback; (3) this is cluster-isolation/
operations evidence, not an adapter recovery acceptance test. Neither competing
native suite was running during the probe. Keep the safe cluster-wide watermark.
The re-run sweep uses a NEW18.6 cluster and fixed explicit mnesis test role; the
91-case suite uses17.10 alone. They no longer share transaction state.

Isolated-cluster proof:17.10 native91/91,zero skips/no filter,22.453s; fresh18.6
mutation cluster baseline91/91,16.527s,10 candidates/7 caught/3 compile-unviable,
zero missed/timeouts. Fresh server stopped and its temporary cluster disappeared.
Three post-run reviews for EACH native run and full sweep:
1. Acceptance: both native baselines include12 outcome cases. Exact empty and
   existing caller heads, unknown actual, native INSERT wait and winner rows
   pass. All previous phase/source/repository/read/wake laws remain present.
2. Falsifiers: fresh false-classifier mutant fails the new deterministic case;
   false guard is caught by real native23505 in the concurrent law, true paths
   fail unrelated PK, commit-code boolean fails mapping and absent hints fail
   actual injected-fault observation. Each first failure was inspected.
3. Loopholes: these runs use physically separate clusters, not just different
   DBs. No compiler rejection is counted caught; no skipped baseline, survivor,
   timeout, assertion/deadline reduction or modified notification policy is hidden.

Matched PG3 full Darwin gate passes all12 checks,1,536 workspace cases/four
compiler exclusions,485.246s nextest. Source/test/manifest/lock diff SHA256:
1f0994b7d03fafcad465e1f347b2f9f3690a1ff9c289cf249acaa7efe3af8fac.
Three post-run reviews: (1) the new case adds one suite case with two actual
head scenarios; native91 is separate from DB-free returns; (2) original bad
metadata and false-classifier failures are preserved and the fresh sweep has no
viable survivor; (3) source hash is unchanged, no thresholds or exclusions were
altered, and protected Linux jobs must re-run on the corrected PR377 head.
## PG4 representation and startup experiment (pre-edit)

Threats already reproduced in native facility probes: signed versions/positions,
text NUL, long incompressible keys and write-required open. First add six adapter
regressions against the still-unmigrated0.4.0 production shape: inclusive read
fromu64::MAX; global resume with a high sequence but a later real transaction;
UTF-8 NUL event type; incompressible5,120-byte key; sequential append crossing
signed bigint maximum; and read-only open of a prepared schema. The last fixture
will move preparation to explicit migration when that API exists; runtime open
must remain read-only. Use a separate mnesis_pg4 database on the same real pinned
Postgres17.10 server; do not mutate the active PG3 test database.

Characterization budget:+220 native test lines, production+0. After failures and
three case reviews, implementation budget at most+700/−200/net+500 production
Rust lines and+350 SQL lines over current owners: schema/migrations, builder,
error, row conversion/query/insert and a small private stream-key lookup. Reuse
SQLx Migrator and native constraints; no migration engine, identity protocol,
framework, runtime or tool. Normalize event stream identity if the native hash
exclusion probe survives adapter concurrency/migration tests; no hash substitutes
raw identity. Track actual production/test/SQL/public surface costs per retained
sub-experiment. Later PG4 acceptance adds migration/version/checksum/structural,
concurrent/failed-upgrade, role and full unsigned boundary cases.

PG4 native characterization: all6 targeted cases fail against the unchanged
representation/startup code,0 passes/6 failures/zero runtime skips,1.319s.
[Output](evidence/pg4-original.log). Three-pass review:
1. Contract: failures are the intended54000 long-key limit,25006 DDL-on-read-only
   open, signed-version rejection, signed resume-sequence rejection,22021 text
   NUL, and unsigned inclusive-bound rejection. No setup/connection failure
   substitutes for the product defect. Test delta203 lines, production+0.
2. Falsifier: high-sequence resume includes a later real transaction, preventing
   a fake always-empty result from passing. The version test seeds the largest
   representable old head rather than materializing billions of prior rows;
   it tests the documented head/next contract, not a complete historical replay.
   Long keys use deterministic incompressible bytes, not the earlier compressible
   hint witness. The NUL envelope is accepted by the owning value constructor.
3. Loopholes: the existing-ready-schema fixture is legacy preparation; when the
   explicit migration API exists, prepare through it before testing read-only
   runtime open. Keep the no-DDL assertion unchanged and add separate legacy
   adoption/rejection cases. These six failures do not cover migration metadata,
   least privileges, concurrent upgrades or full unsigned checkpoint persistence;
   those remain required.90 filtered cases are not credited as passes here.

PG4a smaller migration slice before representation changes: explicit SQLx migrate
and read-only validating open, owned mnesis schema, fully qualified runtime SQL,
verified legacy adoption and published SQL source inclusion. Keep current legacy
column representation in migration1; migration2 will own the unsigned/binary-key
changes after this independently reviewable slice. Budget+260/-50 Rust,+140 SQL,
+250 tests, one existing SQLx feature, no registry dependency or migration engine.
First characterize missing/wrong optimistic constraint acceptance; runtime open
must reject either, without silently repairing or writing. The existing six
representation failures remain open until their respective changes land.

Both missing/wrong constraint witnesses fail against original startup (0/2,
zero skips,0.032s). Three post-run reviews: (1) both reach successful runtime
open despite missing or wrongly targeted uniqueness; (2) assertions fail for
those exact accepted corrupt schemas, not connection/setup errors; (3) each
fixture restores the original constraint before asserting, so later failures
cannot be artifacts of prior DDL. Initial generic is_err will become the exact
schema error once that public error alternative exists. Existing tests remain.

Initial PG4a selected native run:3 pass,5 intentionally filtered representation
cases (those remain open),0.095s. Three reviews: (1) prepared read-only open now
passes, missing/wrong constraints fail with the exact Schema alternative; (2)
the two new assertions failed against original startup and both restore their
DDL before checking; (3) this selected run does not establish fresh install,
concurrent/failed migration, checksum or role safety. Those tests must be added
before retention; no filtered unsigned/key/NUL defect is marked fixed.
Dependency review found SQLx's migrate feature expands lockfile-only optional
MySQL/SQLite resolution (+14 packages), despite neither being present in the
actual normal/build tree. Investigate advisories/license gates; no new advisory
ignore or handwritten migration implementation is justified by this discovery.

PG4a acceptance expansion pre-edit: raise the native-case budget from250 to360
lines to cover distinct fresh/repeat, populated legacy, concurrent migration,
transactional failure/retry, checksum/newer metadata, namespace collision and
least-privilege obligations. These exercise separate observable failures; no
framework/helper binary is introduced. Reuse one dedicated native test database
and existing serial test discipline. Immutable legacy SQL fixture is extracted
from released6faddb7f, not regenerated from the new implementation.

PG4a first integrated run:89 pass/11 fail,5 unsigned/key/NUL representation
cases deliberately filtered,12.367s. Three post-run reviews:
1. Acceptance: fresh/repeated, concurrent, failed/repairable migration, checksum/
   newer metadata, schema collision and no-CREATE runtime role cases pass. The
   legacy positive fixture fails on empty metadata (invalid by the owning value
   contract); two outcome fixtures fail on still-unqualified index/sequence SQL.
2. Falsifiers: missing/wrong constraints reject specifically; injected42P07 rolls
   back the legacy move and leaves zero migration records. The remaining8 read
   failures are fallout from the failed index cleanup, not new page regressions.
3. Loopholes FOUND: qualify all index/sequence fixture cleanup as well as table
   queries. Use valid nonempty metadata for the positive legacy read, and KEEP
   the empty-byte witness separately: adoption must preserve it, reads must
   reject it, and NULL must remain distinct. No assertion is removed or softened.
   Rustfmt expanded the nine migration cases to446 lines, beyond360; revise the
   test budget to500 for these explicit obligations plus the separate corruption
   case. Production Rust remains below260 and SQL below140; no test framework.

PG4a corrected integrated native run:101 pass/zero failures,5 explicitly open
representation cases filtered,12.856s; strict all-target Clippy passes. Three
post-run reviews:
1. Acceptance: all90 existing raw/wake/outcome cases still pass, plus ten migration
   laws and read-only prepared startup. Legacy positions, next sequence, schema
   version maximum, payload and nonempty metadata survive; empty metadata remains
   exact corruption and NULL remains absence without skipping the corrupt row.
2. Falsifiers: original missing/wrong constraint cases rejected; failed legacy
   ALTER retains old data and zero migration records, explicit repair succeeds;
   runtime role provably lacks CREATE and still appends/reads. Fixture index/
   sequence cleanup is fully qualified and no longer contaminates later cases.
3. Loophole FOUND: spawn+Barrier starts overlapping migration calls, but does not
   prove both requests reached native lock contention. Strengthen that witness
   by holding SQLx's own lock, observing both native ungranted locks, then releasing
   it. Cancellation while migration DDL is blocked is also not yet tested; add
   it now instead of inferring it from error cleanup. No representation/fullPG4
   claim follows from the five filtered cases. Test budget484 lines currently;
   pre-edit increment+120 lines (total620) for these concrete contention/cancel
   witnesses. Keep the production budget unchanged.

PG4a stronger contention run:101 pass/1 fail,5 open representation cases filtered,
15.970s. Three reviews: (1) both migrations are now observed waiting on SQLx's
native advisory lock before release; one metadata record and zero leaked locks
follow; (2) cancellation's exact ALTER-phase witness times out, so no cancellation
acceptance is credited; (3) the fixture's ACCESS EXCLUSIVE table lock also blocks
preflight relation lookup, before ALTER. Replace it with ACCESS SHARE, which
allows preflight reads but still conflicts with ALTER's ACCESS EXCLUSIVE. Keep
both the actual ALTER query witness and unchanged3s deadline. This corrects the
fault phase; it does not weaken the assertion or substitute arbitrary termination.

Selected contention/cancellation rerun:2 pass,9 filtered migration cases,0.058s.
Three post-run reviews: (1) exact native ALTER query is now waiting, abort reports
cancelled, old positions and zero migration records survive, retry installs one
record; both concurrent calls also reach native lock contention; (2) ACCESS SHARE
correctly targets ALTER rather than preflight and retains the strict phase oracle;
(3) NEW loophole: retry alone could reuse the same session's leaked reentrant
lock. Assert zero native advisory locks before retry as well, sharing the exact
existing zero-lock oracle with the concurrency case. Then falsify that assertion
by removing close_on_drop temporarily. Keep source restoration and raw failure.

Controlled native close_on_drop removal is killed: cancellation reaches the
actual ALTER phase, then fails the exact zero-advisory-lock assertion in3.047s.
Source restored byte-for-byte from its pre-mutation snapshot. Three post-run
reviews: (1) the intended leak, not setup or transport failure, causes rejection;
(2) no reentrant successful retry can mask it because the assertion precedes
retry; (3) the mutation is not retained, test deadline/phase/error expectations
are unchanged, and10 filtered tests are not credited. This manual mutation is
independent from the earlier cargo-mutants diff sweeps.

Split the migration delivery from the still-unimplemented representation work:
move the existing read-only startup case unchanged into migration_tests.rs.
The five unsigned/NUL/long-key witnesses remain in the PG4 research checkout
for the next representation slice and retain their original failures/evidence.
They cannot be included in or credited to PG4a's delivery suite. PG4 remains
active until that slice passes and all migration criteria are delivered.

PG4a restored-source native run:102/102 pass,13.039s. Nextest reports zero case
skips but ONE binary is explicitly excluded: the five still-open representation
witnesses. This is migration-slice evidence only, not full adapter acceptance.
Three post-run reviews:
1. Acceptance: all90 previous cases plus12 migration/startup laws pass, including
   actual native contention, exact DDL cancellation, zero locks before retry,
   metadata/data rollback, populated upgrade, least privileges and read-only open.
2. Falsifiers: the lock cleanup mutant fails before retry, restored source matches
   its saved bytes, and original wrong/missing constraint witnesses remain red
   against released startup. Existing ordering/byte/hint/outcome assertions remain.
3. Loopholes: no case assertion or deadline was lowered. Five unsigned/NUL/key
   tests are not delivered by PG4a and remain required in PG4b. The focused
   delivery checkout must run without any test filter; the final production
   adapter must pass all those later witnesses too. Actual migration tests597
   lines <=620 revised budget; production+125/-54, SQL migration25+validation40
   lines, one public migrate method and two error alternatives, no new framework.

Focused PG4a delivery checkout: all102 native cases pass with NO filter and zero
skips,13.226s. Three reviews: (1) all previous laws and12 migration/startup cases
are present; (2) exact lock/ALTER/cleanup oracles and original failure witnesses
are unchanged; (3) this checkout contains only migration implementation, so the
five required later representation cases remain in the research checkout and
are not credited to this count. No completePG4 claim follows.

Full Darwin gate FAILED on RUSTSEC-2023-0071: SQLx0.8.6's migrate facade feature
resolves unused MySQL/SQLite drivers, including rsa0.9.10. cargo-deny's actual
dependency tree check passed, but CargoAudit grades the lockfile and rejects it.
No ignore or changed gate is allowed. Native proof does not override this failure.
Source/test/manifest/lock/Nix diff hash at failed gate:
bb9afcd4a0d5df7ef2fc4d1bdf84ffd3d49e9fc21395d2f589749e11e83da64d.

Dependency correction pre-edit: evaluate maintained SQLx0.9.0 (Rust1.94 minimum,
compatible with pinned1.99, MIT OR Apache-2.0). Its MySQL RSA support is separately
opt-in and disabled by default. Preserve existing SQLx/Tokio/driver ownership;
no replacement runtime/framework or handwritten migration source/registry.
Upstream SQLx#3196 and Cargo#13078 explain optional-driver lockfile resolution.
Compare native laws, audit/license gates, dependency tree/lock costs and startup
behavior before retention. Budget: one workspace SQLx version update, generated
workspace-hack manifest/lock changes, at most+80/-80 adapter compatibility lines,
no new public API or dependency crate family. Keep all assertions and gates.
A manual embedded MigrationSource on sqlx-core0.8 would duplicate migration
version/description authority and add custom registry work; prefer supported
current facade capability if compatibility and cost checks pass.

SQLx0.9 first native run:15 unit cases pass/87 native setup failures,zero skips,
0.950s. Three post-run reviews: (1) no native migration law passed here; all87
fail before storage on role anonymous, SQLSTATE28000; (2) SQLx0.9 changed OS-user
lookup via whoami2, and its supported fallback is anonymous when lookup fails in
this Darwin Nix environment; (3) supply the intended role explicitly in local
URLs and both CI owners instead of creating anonymous or weakening assertions.
The upstream parser supports the user query parameter and tests it directly.
Use fixed initdb role mnesis in fresh mutation clusters; the VM already creates
root, so its URL names root. Production connection policy remains SQLx-owned.

SQLx0.9 also provides schema creation and a schema-qualified migration table
under its native migration lock. Reuse those methods and remove our CREATE
SCHEMA/SET search_path statements. Preflight legacy validation still holds a
SQLx reentrant lock until scoped connection close; actual contention/cancellation
witnesses remain mandatory. Generic run still fails Send/Acquire compilation;
retain upstream run_direct(None, connection, false), now its0.9 signature. Dynamic
DDL in the existing SQLSTATE test uses AssertSqlSafe solely around formatting of
its four compile-time literal codes; no user input or production SQL is exempted.

SQLx0.9 with explicit intended username:102 native passes,zero skips/no filter,
13.940s. Three post-run reviews:
1. Acceptance: fresh/legacy/repeat/concurrent/cancel/failed upgrades, least rights,
   read-only runtime and all prior raw/wake/commit-uncertainty laws pass. The
   existing0.8 candidate migration checksum is accepted before clean fixtures;
   native0.9 schema creation uses the owned migration table without search_path
   mutation. No unsafe-retry or row-bound assertions changed.
2. Falsifiers: both requests still reach native advisory contention; cancelled
   actual ALTER leaves zero migration records and zero locks before retry.
   COMMIT suppression preserves exact EOF and independent durable counts.
3. Loopholes: username is explicitly the existing authorized role, not an
   anonymous superuser created to hide failed identity selection. Host-user
   inference is no production guarantee. Five representation witnesses remain
   outside this focused slice. Full matched Nix gates and dependency/performance
   comparisons still required; the earlier running gate precedes import ordering
   and CI-role configuration corrections and cannot certify their final bytes.

SQLx0.9 capacity spot check:40,960x4,096-byte events, one case passed/8 filtered,
7.364s. Retained stream1,089,173B/global1,090,271B versus unchanged4,194,304B
ceiling; first-event timings2.325/2.829ms are single samples under competing Nix
build load, not a throughput/noise-floor result. Three reviews: (1) workload and
byte ceiling equal original10x-backlog witness; (2) actual SUT first stream/global
rows and correct origin/version are asserted, not just allocator output; (3)
no smaller workload, threshold increase or untested throughput claim. Native
schema/driver change preserves bounded retention; full capacity remainsPG8.

Native CI correction pre-edit: replace the existing Python NixOS testScript with
locked nixpkgs' postgresqlTestHook + stdenv.mkDerivation and existing nextest
archive. Source: pkgs/by-name/po/postgresqlTestHook/{package.nix,test.nix,
postgresql-test-hook.sh} at locked nixpkgs. The hook owns initdb, Unix-only service,
pre/post/failure cleanup and test identity. It is Linux-supported (badPlatforms
Darwin), matching the existing package's Linux scope. Budget+50/-70 Nix/CI lines,
no Rust test assertion or new framework/runtime/tool. Standard stdenv unpack
provides writable sources; preserve the same archive/remap/serial cases, strict
pipefail and durable run.log. CI prints the artifact log even on a cache hit.
Hook setup role uses SUPERUSER only for existing administrative fixtures (as the
previous root fixture did); the runtime least-privilege case remains distinct.
This removes the edited owner’s project Python instead of retaining it. Feature
script Python remains a separate requiredPG9 removal through supported cargo-hack.

SQLx0.9 initial Darwin gate completed all12 checks,1547 workspace passes/four
compiler exclusions,503.047s. Three reviews: (1) existing102 native cases are
separately proved, DB-free returns are not new native evidence; (2) RSA is gone
and audit/deny pass without a new ignore; (3) this source precedes import ordering,
CI role/hook changes and the forced-collision correction, so it is not the final
matched delivery gate. Retain only as dependency-candidate evidence.

Next migration-artifact audit: legacy shape verification currently occurs only
in Rust before SQLx runs migration1, so direct execution of the shipped SQL can
omit that obligation. Characterize that bypass with an incompatible legacy
constraint, then put the immutable legacy shape check inside migration1's SQLx
transaction and remove the Rust preflight/reentrant bootstrap lock. SQLx0.9
already serializes native schema/metadata creation. Current runtime shape and
legacy-version shape are distinct authorities once representation migration2
lands; no stored validation function, second migration engine or extra public
adoption API is justified. Budget+55 SQL/-25 Rust,+50 tests (670 total migration
case cap); preserve and test cancellation with actual AccessExclusiveLock on the
legacy relation, not a long query-string substring truncated by pg_stat_activity.
Add SQLx's documented standard Cargo build.rs rerun-if-changed=migrations so adding
migration2 is observed on stable Rust, and enforce LF SQL checkout for checksums.
No custom development tool: this is Cargo's supported compile dependency hook.

Migration-artifact bypass reproduces:1 selected case FAILS (12 filtered),0.031s;
raw checked-in migration1 accepts a legacy table missing its optimistic unique
constraint. Three post-run reviews: (1) actual result is Ok(PgQueryResult), not
requiredSQLSTATE55000; (2) the real SQL artifact executes with Rust preflight
omitted, and explicit rollback/fixture repair occurs before assertion; (3) data
remains unchanged and later tests are not contaminated. Fix the artifact itself,
then remove the bypassable Rust preflight and bootstrap reentrant acquisition.

Self-validating migration artifact run:13/13 native cases pass,zero skips,24.207s;
strict all-target Clippy passes. Three post-run reviews:
1. Acceptance: direct SQL rejects missing legacy uniqueness with55000; normal
   legacy upgrade preserves rows/positions and metadata; every existing migration,
   role, checksum, contention and cancellation case passes.
2. Falsifiers: shipped-script witness was red before SQL guard; actual native
   AccessExclusiveLock on the legacy relation proves cancellation reaches ALTER
   without relying on a query string that may be truncated. Zero locks before
   retry and zero migration records remain asserted. SQLx owns bootstrap locking.
3. Loopholes: first fixture took23.896s while others <=40ms; no latency/SLO claim
   is made from this run. Record and investigate that tail duringPG8 capacity;
   do not drop it from results. The new build.rs is only Cargo's documented
   migration-directory dependency instruction, not a validation/tool suite.

PG4a rebased onto corrected PG3b26f0556:104/104 native cases pass,zero skips/no
filter,14.576s. Three post-run reviews:
1. Acceptance: all91 existing cases plus13 migration laws run; source contains
   caller-head preservation and both native race scenarios. Read-only runtime,
   legacy/empty metadata, SQL artifact self-validation and cleanup still pass.
2. Falsifiers: the original bypass is rejected by the artifact itself; actual
   AccessExclusiveLock, not a potentially truncated query, drives cancellation.
   Zero retained advisory locks and unchanged independent rows remain asserted.
3. Loopholes: no case dropped, deadline relaxed or result reclassified. Five
   unsigned/NUL/long-key requirements remain in the representation research
   checkout and are not claimed fixed here. The new first-fixture latency is
   recorded; this run makes no performance/SLO claim.

Local x86_64 Linux build request could not resolve the configured nixos.local
builder and was cancelled before tests. No native-Linux proof is credited.
The configured aarch64-linux builder is actively building the same supported
hook/archive integration path. Its first snapshot predates recent SQL guard/
PG3 changes; use it for harness validation only, then require matched current
native artifacts and a controlled failing-pipeline proof before delivery.

PG3 protected delivery: PR377 merged 2026-10-18T08:42:47Z as
f87afc12dd6dabe4acd7847e98756858f5fb4c86, headb26f0556fa74d45f2e072fd93269e9c04913e589.
All40 checks green;1 expected scheduled mutation-sweep skip, zero unresolved
threads. CI39110924458 and mutation39110924430 inspected after completion.
Three post-run reviews: (1) full1536 workspace/4 separate compiler suites and
feature matrix pass, native baseline is real; (2) fresh-cluster10 mutants include
7 caught/3 compilation-unviable, zero missed/timeouts; (3) scheduled-only skip is
not credited, NixOS cached integration is not fresh native evidence, no threshold
changed. Local91-case isolated native proof supplements that older harness.
No publication or complete PG3/adapter acceptance claim follows this focused merge.

Native Nix hook candidate, aarch64-linux derivation
/nix/store/ipyv7sp5mh6s59pmxzq3whjjfvzfygxd-mnesis-postgres-integration:
fresh PostgreSQL18.6,102/102 pass,zero skips,16.614s, clean server shutdown.
Three post-run reviews: (1) all then-current102 cases ran through native hook;
(2) real COMMIT-loss/notification/migration fixtures run, not DB-free returns;
(3) this precedes latest PG3 collision and migration-script guard: harness candidate
only, not matched current source. Current104-case artifact and a deliberately
failing pipeline proof are required. Offline x86 builder ran no tests; no credit.

Matched PG4a gates complete, unchanged source SHA256d59835191badbc8b17869597e7b6d0339f6604c1a2513b97a0ffe0bb4b0e3613:
12 Darwin checks pass (audit/TOML reused;10 rebuilt),1549 workspace cases pass,
four compiler exclusions owned by separate Linux jobs,467.299s nextest. Native
ARM Linux supported hook:104/104 pass,zero skips,13.409s, fresh18.6 cluster and
clean shutdown; /nix/store/fb9lkf51i60m2ylv848slzy6ikp7plkq-mnesis-postgres-integration.
Three post-run reviews for EACH gate:
1. Contract: all13 migration cases plus91 prior cases retained. Runtime requires
   only read/insert role, explicit admin migration, repeatable legacy preservation.
2. Falsifiers: direct SQL bypass, native cancellation/zero-lock-before-retry,
   forced uniqueness wait, commit acknowledgement loss and recovery all execute.
3. Oracle:104 includes no skips or filtering; workspace DB-free returns are not
   double-counted as native proof. Exact source hash frozen; dependency audit
   passed without adding RSA ignore. Previous102-case harness is superseded.

Controlled pipeline failure proof: same matched archive and supported native
hook, ONLY DATABASE_URL role replaced with mnesis_intentionally_missing via
Nix overrideAttrs (one-off command, no repository tooling). Native SQLSTATE28000
fails first database test;14 DB-free passes,1 failure,89 not run,zero skips.
Nix build exits1 instead of accepting tee success. Hook shuts fresh server down
on the failure path. Three post-run reviews: (1) expected failure is actual
nextest setup failure propagated by pipefail; (2) original masking pipeline
would return tee success for exactly this output; (3) intentionally unrun89 are
not credited, no product assertion/filter changed and no failure artifact was
installed. This proves harness fail-closed behavior, not a storage-fault law.

Normal signed commit hook repeated all12 matched checks from cache successfully.
Three reviews: acceptance/source hash unchanged, native proof remains separately
104 cases, cached workspace checks are not counted as fresh execution. New build
transcripts retain diagnostic content with trailing spaces removed for diff hygiene;
raw temporary originals are preserved. No production source changed after gates.

PG4b baseline on the frozen PG4a tree341eba45b88d066c183548e4fd2a4f515481533b:
PostgreSQL17.10, explicit userjoel, five selected representation cases:0 pass,
5 fail,0 skips,0.865s. Raw output /tmp/mnesis-pg4b-original.log.
Three post-run reviews:
1. Contract: nonzero unsigned versions, arbitrary UTF8 event types including NUL,
   arbitrary binary keys and lexicographic global resume remain core obligations.
2. Original defects: actual Btree tuple5144>2704 rejection, BIGINT narrowing,
   SQLSTATE22021 NUL rejection and high-sequence resume rejection all reach storage
   or adapter conversion. No setup or compiler failure is mistaken for a witness.
3. Oracle: each case asserts returned content/position or exact empty boundary;
   the later transaction is real and has a strictly greater transaction ID. All
   five ran with a real URL and no concurrent native suite on their cluster.
   Read-only startup moved unchanged to migration tests; no contract was deleted.

PG4b experiment budget: retain PG4 total +700/-200/net500 production Rust,
+350 SQL; target compact internal binary-key catalog, native full-u64 numeric
versions, BYTEA event types, native xid8 comparison and unsigned resume handling.
No extra runtime, numeric codec, command retry or custom hash identity. Migration1
stays frozen; migration2 must preserve existing rows, positions and sequence.
Remove the obsolete PreparedInsert allocation and its four implementation-only
unit tests only together with native batch/schema-range coverage. Core append
validation remains authoritative; a stored maximum version must reject overflow.
Falsifiers include real hash collisions, large-key races, migration preservation,
late-page corruption and all earlier outcome/read/wake regressions.

PG4b first candidate: five original native representation witnesses now pass,
zero skips/no filter,0.123s. Three post-run reviews:
1. Contract: full unsigned read bounds and lexicographic resume, NUL UTF8 type,
   large binary origin and signed-boundary append preserve exact results.
2. Falsifiers: original raw failure log remains; only boundary seed SQL now uses
   the compact foreign key, all original assertions remain unchanged.
3. Gaps FOUND: current latest-shape validator does not yet verify the new keys
   table, numeric precision/checks/FK or type generation; full-u64 maximum and real
   equal-hash/concurrent-key cases remain missing. The five-case pass is not full
   adapter or migration acceptance. Add those before retaining this candidate.

PG4b full first native run:105 cases,98 pass/7 fail,zero skips,13.205s.
Three post-run reviews: (1) existing read/wake/outcome behavior executes, but
migration and collision acceptance is not yet green; (2) actual failures are
old one-migration expectations, old raw-key/numeric decoding and runtime role
missing new catalog grants, not SUT data-loss suppression; (3) preserve every
assertion's meaning while updating independent SQL inspection for the new schema,
assert both migration records and unknown version3, grant only SELECT/INSERT
plus sequence use on new keys. Late-page corruption now injects invalid UTF8
bytes because new scalar CHECK prevents the former schema_version0 injection;
its original termination/no-skip assertions are unchanged. Add explicit scalar
CHECK negatives rather than count that stronger boundary as tested already.

PG4b second native run:110 cases,109 pass/1 fail,zero skips,13.497s.
Three post-run reviews: (1) all13 updated migration laws and forced-conflict law
now pass, maximum-u64/u32 append and overflow immutability pass; (2) scalar invalid
values are rejected and missing catalog/version guards prevent startup; (3) new
collision fixture FAILED before SUT because hashvarlena(bytea) is not a callable
SQL signature. Do not count it as a caught product defect or collision proof.
Use the original native probe's hash_record(ROW(bytea)) plus an actual index
recheck witness, retaining exact binary identity assertions. No oracle is relaxed.

PG4b third native run:110/110 pass,zero skips/no filter,13.439s.
Three post-run reviews:
1. Contract: full unsigned maximum/version overflow, schema maximum, exact raw
   keys, native hash collisions, concurrency and all prior regressions pass.
2. Falsifiers: collision actually removes one index entry during native equality
   recheck; large-key race has one success/one Conflict/one catalog identity;
   invalid scalars leave zero events; missing required guards reject startup.
3. Remaining gap: versioned-v1-to-v2 upgrade failure/repair is not yet independently
   tested, and migration2 currently drops public.events without proving ownership.
   Add a foreign-view collision witness before changing that behavior. This pass
   does not certify the upgrade or pending capacity/query-plan evidence.

Upgrade characterization:3 native cases,2 pass/1 fail,zero skips,0.109s.
Three post-run reviews: (1) genuine SQLx version1 installation upgrades with exact
positions, schema/metadata/body and increasing next sequence; invalid legacy
version aborts all migration2 DDL and remains repairable under version1 metadata;
(2) foreign-view witness fails because migration returns Ok and replaces a view
it does not own, an actual destructive-ownership defect in this candidate;
(3) setup uses supported Migrator::with_migrations from the embedded immutable
version1, not manually fabricated checksums. Preserve failure and add fail-closed
view ownership check under a catalog-only migration search_path before DROP.

PG4b fourth native run:113 cases,62 pass/51 fail,zero skips,5.863s.
Three post-run reviews: (1) all three actual upgrade cases pass, including the
previously destructive foreign-view scenario; (2) each of51 earlier failures is
VersionMismatch(2): this disposable DB still had the earlier UNPUBLISHED candidate
migration2 checksum. Checksum rejection is correct; never rewrite recorded metadata
or weaken validation. The migration suite then creates fresh owned fixtures and
last upgrade leaves the current checksum. (3) 62 passes are not full-suite proof;
rerun all113 against that freshly installed current test schema, retaining this
failed run. Published migration1 has not changed. Candidate2 will freeze before
its focused PR. No application database or production migration metadata changed.

PG4b fifth native run:113/113 pass,zero skips/no filter,13.411s.
Three post-run reviews:
1. Contract: all original outcomes/read/wake/migration cases plus13 representation/
   versioned-upgrade laws pass. Native count changes from104 are -4 obsolete pure
   scalar-conversion mirror tests,+10 representation,+3 actual upgrade cases.
2. Falsifiers: foreign view is preserved/rejected then upgrade succeeds after
   explicit removal; negative old version leaves v1 bytes and metadata untouched
   before repair; positions/sequence and subsequent binary type append survive.
3. Loopholes: no checksum was rewritten, no assertion loosened, invalid UTF8
   retains later-page corruption termination. New scalar checks have independent
   real rejection cases. Earlier failures remain logged. Still require native
   query-plan/capacity/mutation and full cross-platform delivery proof.

PG4b capacity probe: selected native first-event case,40,960x4096B,1 pass/8 filtered,
0.633s. Retained stream1,094,103B/all1,100,611B, unchanged4,194,304B ceiling.
Three post-run reviews: (1) same tenfold history and four-page bound as retained
baseline; (2) peak allocator calibration and original retained-backlog falsifier
remain, actual large database workload ran; (3) only one case credited, no filtered
case counted. Single latency samples5.164/14.101ms occurred while independent
mutation compilation ran; not a fair throughput or latency comparison. All-scan
join cost needs native plans and repeated isolated measurements before retention.

PG4b native prepared generic-plan study, PostgreSQL17.10,40,960x4096B:
all-page seeks at0/40000 each examine exactly256 event headers via native
watermark index,256 primary-key body lookups,1 memoized key lookup per side;
779/781 shared buffer hits,0.381/0.350ms server execution. Stream seeks use
native numeric version/compact-key unique index, exactly256 headers and bodies,
780/779 buffer hits,0.379/0.330ms. Head uses backward unique index-only LIMIT1,
0 heap fetches,0.064ms; one-row keys relation is appropriately sequential.
Three post-run reviews for EACH SQL study: (1) row/body prefix and ordering match
current page.rs including raw key accounting and nullable oversized bodies;
(2) generic plans at deep resume do not scan40,960 histories or cast indexed
columns, redundant stream index is absent; (3) SQL copies were compared to source,
prepared parameters include explicit casts on parameters only. These single server
samples are evidence of bounded access, not latency SLOs or broad cardinality proof.
Many-key cardinality, capacity/soak and final profile remain PG8 obligations.

Fresh independent PostgreSQL18.6 mutation baseline113/113 pass,zero skips,
13.545s. Three reviews: acceptance includes new upgrade laws; original fault and
collision assertions are present; cluster physically separate from17.10 memory/
plan workload, no same-cluster interference. Sweep19 candidates still running;
no final mutation verdict until every result and first failure is inspected.

PG4b automated native diff sweep complete:19 candidates,14 caught/5 compile-
unviable,zero missed/timeouts, fresh18.6 cluster removed after server stop.
Three post-run reviews for the sweep and each candidate outcome:
1. Acceptance: unmutated113/zero skips baseline; each viable mutant has a real
   native test failure, all five unviable results have actual missing-Default
   compiler diagnostics and are not counted caught.
2. Falsifiers: omitted page finishing fails real out-of-order resume; absent or
   false heads fail payload replay/conflict immutability; fabricated key numbers
   fail actual append/read identity. First failing case in every log inspected.
3. Loopholes: no exclusions, timeouts, survivors or changed oracles. SQL string
   contents are not mutated by cargo-mutants; original representation failures,
   guard-removal native tests and upgrade failure witnesses cover those separately.
   Do not claim this sweep exhausts SQL behavior or final adapter mutation coverage.

PG4b matched ARM Linux package /nix/store/iw2bpshrp0slzl0954l5ikwmk6i1qdcp-mnesis-postgres-integration:
fresh18.6,113/113 pass,zero skips,16.066s, normal supported-hook server shutdown.
Three post-run reviews: (1) all old and new native cases ran including3 genuine
version1 upgrades; (2) collision/index recheck, foreign-view preservation, corrupt
row/rollback and real outcome faults execute; (3) no filtered cases, source matches
the staged candidate used by the live Darwin gate, cache is not claimed fresh.
Candidate native direct SQL mutation attempt initially aborted before test because
rustfmt split query.push across lines; no test/result credited. Corrected exact
.push string replacement is running in an archived copy; live delivery source
and its matching checks stay untouched.

Controlled SQL resume mutation, archived staged tree only: replace high-sequence
branch txid > with txid >=. Native exclusive-resume case FAILS as required with
2 rows instead of1,0/1 pass,9 filtered,exit100,0.658s. Three post-run reviews:
(1) exact contract is high sequence excludes only its own transaction; (2) two
real committed transactions exist and the original assertion catches reintroduced
first transaction, not setup/timing; (3) mutation was confined to an archived
copy, delivery source unchanged,9 filtered not credited. Control wrapper exits
success only for the observed nextest failure100. No claim of full test coverage
from this selected controlled mutant. Raw output retained.

PG4b MATCHED Darwin gate complete:all12 checks pass,1558 workspace cases pass,
four compiler exclusions require separate Linux compiler jobs,461.040s nextest.
Three post-run reviews: (1) workspace count changes by9 vsPG4a as documented,
all feature compilation/doc/audit/license/style gates pass; (2) original five
representation failures, foreign-view failure and controlled resume mutation
remain evidence, native113 proof separately executes real storage; (3) no filter
or threshold change, DB-free returns are not native proof, source hash remains
7ea6c9a1220b94a201ad49269a91de014f517e44aca622b7e3f2207a43f39d79.
Separate native ARM package passed113 without skips. Complete release gates and
unimplemented capabilities remain open. Property tests ran normally; no timeout
or process was suppressed to obtain this result.

Normal PG4b signed-commit hook also passes all12 checks, fresh1558 cases/four
compiler exclusions,456.756s; adapter source bytes unchanged, only new SQL evidence
entered the broad Nix source filter. Three reviews: complete feature/style/audit
matrix retained, native113 proof separate, no skipped compiler or cached run
counted as fresh native execution. Initial commitd044228227148ecbded3269b2790174fe34611d8.
PG4a protected PR379 is MERGED2026-10-18T09:16:24Z as
e1ee1dc29c30b0116da14e4cd595a4d8c0c7beb1; head a0f422178ab45f086512f58205445837defa4505
had40 green/1 scheduled skip, native104 zero skips,1549 workspace and21 mutation
candidates17caught/4unviable,zero misses/timeouts. See CI39112792409 and39112792431.
No publication or full-adapter acceptance claim follows these focused deliveries.

Post-PR cross-capability review found a PG4b test loophole: DropInsert could now
intercept new catalog insertion before an event existed. Key setup is committed
before enabling that fault, so the next native INSERT necessarily stages an event
inside the append transaction. Tightened selected case passes1/1,11 filtered,
zero runtime skips credited,0.057s. Three reviews: (1) transaction source error
and zero committed event assertion remain; (2) actual event statement now reaches
CommandComplete before response loss, restoring the intended prefix rollback
falsifier; (3) preseeded key does not bypass event writes and filtered cases are
not counted. Earlier113-run evidence remains valid for its actual weaker fault
point; it is not proof of event-prefix transport loss. Rerun full113 and commit
this stronger oracle before PR380 merge. No production policy changed.

PR380 stronger-event-prefix full native run:113/113 pass,zero skips/no filter,
13.573s. Three post-run reviews: (1) all representation/migration/read/wake/outcome
laws remain; (2) DropInsert now necessarily acknowledges an event statement before
loss because its key already exists and no other INSERT precedes events; (3) no
production behavior/filter/deadline changed, selected1-run not double-counted,
all original failures retained. Normal signed hook and fresh protected CI must
pass this stronger head before merge.

PG5 shared atomic conformance characterization: compilation fails with E0277,
PostgresStore does not implement AtomicAppend. Zero runtime cases ran; no pass or
caught runtime defect is claimed. Three reviews: (1) all5 shared atomic laws are
registered under the intended import feature; (2) missing real capability prevents
building each invocation, not missing database setup; (3) macro skip requires a
URL only for ordinary DB-free workspace checks, native CI must use all features
and a real URL. Add owner-local boundary/lifecycle/isolation/fault cases before
implementation. Existing whole-request validation functions remain the authority.

PG5 contract check: AtomicAppend documents actual committed head in Conflict.
A native unique collision occurs after the transaction aborts; unlike append's
explicitly optional unknown actual, do not silently report a fresh head for an
unknown racer. Resolve by rolling back and reading the monotonic committed head
before returning atomic Conflict, or retain the original typed database error as
Store if the extra observation cannot be made. Never turn a COMMIT failure into
Conflict or assume timeout rollback. Record native forced-race evidence for this
choice before closing the slice; no new command identity or retry protocol.

PG4a protected CI completed: head a0f422178ab45f086512f58205445837defa4505,
40 successes/1 scheduled mutation-sweep skip, no unresolved threads. CI39112792409:
1549 workspace/four compiler exclusions (separate compiler jobs pass), native104
on fresh Linux PostgreSQL,zero skips,13.308s, clean shutdown. Installed run.log
printed a second time is the SAME run and not another104 tests. Mutation CI
39112792431:21 candidates,17 caught/4 compilation-unviable,zero missed/timeouts,
real fresh native baseline. Three post-run reviews: (1) complete selected native
set and feature/compiler matrix pass; (2) migration/real lock/SQL artifact and
phase fault cases execute; (3) cached/replayed output and scheduled skip not
credited as new execution, no exclusions or softened assertions. Protected merge
requested with exact matching head after all40 checks completed; collect exact
merge SHA/time before calling delivery verified. No release or full-PG4 claim.

PG5 first native run:9/9 pass,zero skips,8.938s; strict all-target Clippy passes.
Three post-run reviews:
1. Contract:5 shared laws plus whole-batch reopen/highest-position, final-insert
   real SQL rejection with zero events/keys, native blocked-prefix readers and
   pre-storage malformed/duplicate/oversize rejection against a closed pool.
2. Falsifiers: actual AFTER INSERT trigger blocks on second row; independent
   control and both scan interfaces observe no uncommitted prefix, then4 rows
   share one transaction. Empty input does not touch a closed pool.
3. Gap FOUND: advisory-wait witness filters database/object but not exact caller
   process/query. Tag the pool and observe its INSERT wait to exclude unrelated
   sessions. Isolation case took8.170s despite scoped3s waits passing; preserve
   this unexplained tail for PG8, do not label it a latency pass. Forced race/head
   ordering, true commit uncertainty and import composition still pending.

PG5 revised draft budget before retention: <=+210/-60/net+170 production Rust,
<=420 native test lines, <=20 feature/manifest/archive lines. Original+190 bound
was5 lines short of current shared insert/head parsing and required real-head
reconciliation after a unique rejection. Actual first draft +195/-41/net154:
129 private child-module lines +66/-41 parent. No public API or error type added.
Whole-batch hint wait is bounded once, not once per target. Additional native
schedule/import/SSI acceptance is a separate small verification slice, remains
required and cannot be omitted from PG5 completion. Existing project Python is
not extended; supported cargo-hack replacement/complete CI matrix remains PG9.

PG5 second native run:125/125 pass,zero skips/no filter,15.991s; strict all-target
Clippy passes. Three post-run reviews:
1. Contract:113 existing cases plus9 atomic laws plus3 real transport fault laws
run. Acknowledged atomic COMMIT loss leaves6 rows under one transaction ID with
exact UnexpectedEof source in CommitUnknown, followed by explicit read-only
reconciliation. Caller timeout occurs after confirmed backend commit and retains
all6 rows, never interpreted as rollback.
2. Falsifiers:16-target hint fault is observed after actual commit, then success
must complete within900ms measured FROM first held hint, rejecting multiplication
of250ms per target.48 committed rows and lastseq48 independently checked. The
advisory wait now names the exact process and INSERT, preserving prefix isolation.
3. Remaining gaps: native unique-collision reconciliation (including failed head
observation), all-heads-before-XID ordering schedule, actual SSI/deadlock and
import composition need their own next verification slice. No claim of PG5
completion follows125 passes. Earlier8.170s isolation tail remains recorded;
its later result must be inspected without silently deleting that outlier.

PG5 feature execution policy: native archive and in-diff mutation CI explicitly
select all advertised features. Without this, import's implementation and tests
would be compiled out in mutation/native delivery. The existing feature matrix
still uses project Python; do not extend it here. Its supported cargo-hack
replacement and complete isolated-feature CI remain a required PG9 slice.
Tokio time is selected directly because this owner uses timers, rather than
relying on another crate's feature unification. No new dependency family/runtime.

PG5 mutation evidence REOPENS verification: first completed viable survivors are
prepare->Ok(None) and rejected's &&->||. Each ran125 native cases successfully.
Three reviews per survived run: (1) current five shared laws only test an occupied
stream claimed fresh, not a missing stream claimed present; (2) uniqueness can
reject the occupied case late, so missing head validation escaped. New shared
law must reject a valid version2 run whose expected1 does not exist, and preserve
all streams. (3) &&->|| survives because unrelated injected errors hit fresh
streams; after rollback no existing head is observed, hiding misclassification.
Add a Postgres unique-payload rejection on an EXISTING stream so the wrong guard
would invent Conflict. These are meaningful survivors, never exclusions/equivalent
mutants or passing evidence. Current125-case full gates are candidate-only until
both gaps are fixed and the entire fresh native sweep is rerun.
PG5 verification refinement: +40 shared law/+7 macro lines and <=65 adapter-negative
lines are a separate narrowly budgeted test pass. No production API/contract
change. Existing pending ordering/real-race/import/SSI acceptance remains required.

First PG5 sweep FINAL:24 candidates,8 caught/14 compile-unviable/2 MISSED,
zero timeouts; fresh cluster stopped. Three post-run reviews: (1) actual125 native
baseline,13 child-module candidates included and --all-features verified; (2)
both survivors explained by missing-head and existing-head unrelated-error gaps,
not marked equivalent; (3) fourteen compilation failures are not kills. Add owner
cases and rerun full fresh sweep before any retention/delivery claim. Candidate
Nix/native gates on125 cases are not sufficient and are superseded as final proof.

PG5 initial all-feature Linux candidate:125/125 native cases pass,zero skips,
22.743s on fresh PostgreSQL18.6, supported-hook cleanup completes; artifact
/nix/store/fcx5dzazwb3x89ky9by2kzfgdzqfzgd7-mnesis-postgres-integration.
Three post-run reviews: (1) all features really selected and125 cases run; (2)
committed/timeout/hint and native prefix witnesses execute; (3) this snapshot
precedes the two mutation-gap cases and CANNOT be final feature proof. Mutation
failure still blocks retention. New shared-law Clippy requires equality assertions
that print unexpected rows; retain full empty-row invariant while fixing style.

Oracle ownership/minimization review: keep the missing-head falsifier with the
Postgres adapter, where omitting prepare permits a native version2 insert into a
fresh stream. The existing shared AtomicAppend law already defines the contract;
adding a new public test-kit helper is unnecessary for this adapter defect. Move
the SAME input/error/whole-batch assertions into native atomic_tests and add an
independent zero-keys check. No coverage or contract is removed; the shared-law
draft was only compiled, never credited as runtime proof, and is preserved at
/tmp/mnesis-pg5-shared-law-draft.diff. No mutation exclusions or fake-store
calibration tests added. Refined test-only guard pass budget <=100 native lines,
zero production/public-surface changes, replacing +40 shared/+7 macro proposal.

PG5 guard cases native:2/2 pass,9 filtered,0.930s; strict all-target Clippy passes.
Three post-run reviews: (1) missing declared head returns Conflict(index1,actualNone)
with independently zero events AND keys; existing-head unrelated unique-payload
failure retains that exact constraint in Store(Sqlx) and version1 remains intact;
(2) these inputs distinguish both survived mutations' behavior directly; (3) no
filtered case credited, no mutation config changed, and no public test-kit helper
or calibration wrapper retained. Full127 native and complete fresh sweep required.

PR380 event-prefix correction signed headf4f4a1bdbfbb3770559d834f7aea9b0c332c0bce:
normal hook12 checks pass,1558 workspace/four compiler exclusions,613.324s.
Three post-run reviews: (1) the only functional change is the stronger fault
placement; native113 passes separately; (2) event statement rather than key setup
is now suppressed; (3) no threshold/filter change, slower whole-workspace property
run retained honestly. Source/test diff vs frozenPG4a tree SHA256
e095e0f0798d1a338c279f0e00bd7d9e237d01c3238a530be9fc15088b558af7.
Protected CI must finish again on this corrected head before merge.

PG5 full guard baseline:127/127 native17.10 pass,zero skips,50.657s. Fresh18.6
mutation baseline127/127 pass,zero skips,42.739s; full sweep24:10 caught/14 compile-
unviable,zero missed/timeouts. Each of the two prior survivors is now killed by
its exact new native case. Three post-run reviews for EACH full run and every
mutation outcome: (1) all old and new cases execute under import, no filtered or
DB-free case counted; (2) missing-head mutation fails its new gap/prefix law,
wrong boolean guard fails existing-head unique-payload negative; native hints
and actual-head comparison mutations fail their corresponding fault/conflict
laws. First failure in every log inspected. (3)14 actual missing constructors/
Default/type-inference compiler failures remain unviable, never kills; no config
exclusions/threshold changes. Physically separate clusters avoid watermark
interference. Concurrent Nix builds make these timings unsuitable for SLO claims.

Initial125-source Darwin candidate gate also passes12 checks,1558 DEFAULT-feature
workspace tests/four compiler exclusions,502.511s. Three reviews: (1) this nextest
workspace gate does NOT select the new optional import tests; Clippy separately
compiles all features and native archive explicitly does; (2)125 native proof and
failed-first/fixed127 mutation evidence remain separate; (3) this old snapshot
precedes new guard tests and cannot certify final source. Keep default/feature
claims precise and rerun matched source checks before retaining the feature.

PG5 ordering schedule native:1/1 pass,15 filtered,0.042s; strict Clippy passes.
Three post-run reviews: (1) actual frontend Bind is held before second-head
snapshot, independent server observation shows no writer xid, real controller
commits B1 then atomic A1+B2 follows; (2) full delivery has3 rows, Bversions[1,2],
returned position beyond predecessor and equal to delivered highest; (3) this
is not an inside-query lock delay or uncontrolled task race, but still requires
an archived early-write mutant to prove the test rejects actual reversed order.
Default proxy relay remains unframed; only selected test mode parses frontend.
Linux Nix check/package derivations evaluate identically, so the existing protected
Nix check now includes the SAME supported native package on Linux. A psql preflight
makes absence of DATABASE_URL/live database fail before nextest. Final matched
native all-feature/check proof remains required after these changes.

PR380 corrected headf4f4a1bdbfbb3770559d834f7aea9b0c332c0bce protected CI:
40green/1scheduled skip, clean merge state, zero unresolved threads. CI39116344952
has1558 default workspace/four separate compiler cases and native113/0skips,
17.578s; installed run.log replay is not another113 executions. Fresh mutation
CI39116345048:19 candidates14caught/5compile-unviable/0missed/0timeouts.
Three post-run reviews: (1) full selected native/migration/representation and
compiler/feature matrix runs; (2) actual event-prefix transport witness retained
alongside original value/ordering/upgrade falsifiers; (3) no exclusion or threshold
change, replayed/cached output and scheduled skip not credited. Matching-head
protected squash merge may proceed; exact merge SHA/time must be collected.

Controlled EARLY-WRITE ordering mutant, archived source only:
insert first A run before preparing B's head, then continue normally. Native test
FAILS with actually delivered Bversions[2,1] versus required[1,2]; independent
before-head writer xid was30738.0/1 pass,15filtered,0.038s,nextestexit100.
Three reviews: (1) actual committed predecessor and successor are out of order,
not a synthetic error-code assertion; (2) first assertion names the reversed
versions, before later no-xid/highest-position assertions; (3) delivery source
unchanged, only archived /tmp/mnesis-pg5-order-control.94pFqI mutated, no skipped
case credited. Original body moved early, not deleted to force a setup failure.
This proves the all-heads-before-any-write rule protects stream/global order.

PG4 representation protected PR380 is MERGED2026-10-18T10:05:58Z as
49ffe78f166b7d6a787ea898dfba454218a697e9, exact headf4f4a1bd.
The source/value/migration PG4 acceptance is feature-complete at this focused
merge. Deployment/capacity, later capability migrations and final publication
remain required PG6–PG9. No complete-adapter or registry delivery claim.

PG5 native race/reconciliation cases:2/2 pass,16 filtered,0.090s. Three reviews:
1. Actual final INSERT waits on winner while first run is staged. Releasing winner
   returns Conflict(index1,actual1), only winner remains and new prefix key is gone.
   Removing key SELECT on the dedicated runtime role yields Store with ORIGINAL
   23505/expected constraint, preserving definite rejection without invented head.
2. Native wait and independent version/payload/key queries reject partial commit;
   no synthetic SQLSTATE or fake adapter. Role is verified with current_user and
   permissions restored/pool closed/owned role dropped before result assertion.
3. Tighten permission proof by issuing a separate runtime SELECT and checking42501
   while writer waits. Further classifier audit finds owned schema is not checked:
   another schema's events with the same constraint name can falsely become OCC.
   Reproduce it through a real native trigger before changing the shared classifier.

Native foreign-schema reproduction FAILS as intended:0/1 pass,18filtered,26.139s.
Both real calls falsely return Conflict (raw actualNone, atomic actualSome1) when
an auditing trigger violates another schema's events_stream_id_version_key.
Three reviews: (1) definite unrelated rejection must retain Store/native source,
not OCC; (2) schema/table/constraint are real Postgres objects and duplicate rows,
not fabricated SQLSTATE/context; (3) cleanup occurs before assertion, original
owned event remains1, no query/filter/deadline relaxed.26.134s case tail is another
unexplained latency outlier recorded for PG8, not counted as a timing SLO pass.
Add independent native schema/error-field probe, then require owned mnesis schema
in the shared classifier via supported SQLx native downcast/schema accessor.

Scoped-classifier fix native:4/4 pass,15filtered,24.495s; strict Clippy passes.
Three post-run reviews: (1) foreign-schema raw/atomic errors retain exact native
23505/schema/table/constraint, while true owned-schema collisions still classify
correctly including caller expected metadata and actual atomic head; (2) a direct
native duplicate probe independently confirms foreign object error fields, and
runtime SELECT denial now independently proves42501 before finishing the race;
(3) no replacement error/message parser or retry inference. Role case24.356s is
an outlier, namespace case0.034s. Need investigate setup versus storage latency.
Upstream SQLx0.9 options/mod.rs67–74 calls whoami2 before URL overrides unless
PGUSER is set. Local explicit URL user does not avoid that default construction.
This is a candidate explanation for Darwin tails, not a proven storage slowdown.
Set supported PGUSER in an isolated local comparison and record option-parse
elapsed time; native Nix hook already supplies PGUSER=mnesis. No library/global
environment mutation or new user-lookup machinery is proposed.

Actual server failures:2/2 native cases pass,19filtered,24.682s. Real wait-cycle
forces SUT deadlock40P01 (3.047s from native3s detector) and only controller rows
persist. Actual SERIALIZABLE read/write cycle produces SUT40001, no owned A row,
only controller B durable. No RAISE for either code. Three post-run reviews:
1. Both exact server codes stay Store(Sqlx), never OCC or CommitUnknown; independent
   data rejects partial/uncertain classification.2. Native lock gates ensure both
   actual cycles, controller commits, and only SUT failure satisfies assertions.
3. SSI case21.633s tail remains unexplained. PGUSER made option parsing0.532ms,
   so user lookup alone does NOT explain this tail. Add phase timing before any
   lifecycle change; do not call this a latency SLO pass or weaken timeouts.
Fixture cleanup now removes only owned fault triggers/constraint before each case
so a failed mutant cannot leave native DDL that falsely kills the next candidate.

SSI timing follow-up: strict Clippy initially rejected diagnostic additions
(semicolon and 81/80 lines); no native cases ran on those attempts. Corrected
without lint exceptions, then selected real SSI case passed1/1,132 filtered,
4.267s. Option parsing1.080ms, entire proxy fixture29.197ms, native insert gate
4.244s, completed close4.245s. The long phase is after the proxy fixture and before
its blocked INSERT, not migration, option parsing or final pool close. Cause
remains open for PG8; no latency improvement or SLO is claimed.
Three reviews after this run: (1) exact40001 remains source-preserved Sqlx and
only the independent controller row persists; (2) real read/write cycle and
native advisory gate remain unchanged, so diagnostic code cannot supply the
failure oracle; (3) only1 selected case executed,132 filtered are not successes;
all prior assertions/deadlines remain. Diagnostic print statements are temporary.

PG5 import composition pre-edit budget: <=230 native test lines, zero production
changes/dependencies/public interfaces. Exercise the existing EventImporter on
Store<PostgresStore> with both policies: final conflict and corrupt-prefix exact
reports, partial persistence on later SQL failure, and suppressed real COMMIT
acknowledgement retaining typed ImportError::Store(CommitUnknown). Independent
server rows and transaction IDs reject whole-chunk degradation. Reuse current
atomic fixture and Unix protocol fault fixture. Full format-2 interchange stays
PG7. Native Nix gate budget <=12 lines: nonempty URL, supported nextest exact
atomic smoke test with --no-tests=fail before complete archive; same derivation
already aliases into Linux checks. No new checker or baseline calibration.

Import composition native5/5 pass,133 filtered,0.166s; strict all-target Clippy
passes after correcting ordinary style findings (no native run on lint failures).
Three reviews after this run:
1. Public Store<PostgresStore>/EventImporter executes both policies. Final conflict
   returns exact offender/versions versus exact partial reports. Corrupt-prefix
   reports agree with native rows and byte/type/metadata preservation. Repeated
   routes write neither catalog nor events. Final native23514 rolls back every
   whole-chunk row/key, while per-stream retains only its previous transaction.
2. Whole-chunk changed to per-stream would leave the first run on conflict/error;
   independent complete row sets reject that defect. Protocol proxy sees actual
   COMMIT before losing its response; exact ImportError::Store(CommitUnknown(Io
   UnexpectedEof)) survives the facade under both policies, one durable XID and
   exact rows prove no automatic resubmission or later per-stream write.
3. Five selected tests execute;133 filtered are not credited. No report is
   fabricated after storage errors. Native RAISE23514 is deliberate insert-failure
   coverage, not a claimed real SSI/deadlock. Shared routing/report planner laws
   retain ownership in core; these only prove native adapter composition.
Import test budget revised to <=320 lines before retention: explicit complete row
sets, facade fault assertions and a separate route case needed ~300 formatted
lines versus estimated230. No production line/public interface/dependency added.
Temporary timing instrumentation removed after retaining its raw evidence.

PG5 complete selected native suite:138/138 pass,0 skipped/no filter,28.741s on
local PostgreSQL17.10, following alignment to protected main49ffe78f. Production
Rust diff199 added/43 removed, net156, within declared210/60/net170. Parent PR380
records and stronger event-prefix evidence preserved; root worktree untouched.
Three reviews after this run:
1. Contract: all113 existing cases plus25 import/atomic/schedule/real fault cases
   pass. Highest-position/reopen, exact route/run/size rejection, native isolation,
   all-heads-before-XID, collision metadata, error namespace and both import modes
   remain exercised; ordinary append retains its documented expected-head data.
2. Falsifiers: archived early-write mutant produced B[2,1]; corrected head gives
   B[1,2]. Both prior automated survivors now have direct native rejection cases.
   Server COMMIT loss/caller cancellation leave full durable batches, true SSI/
   deadlock native errors remain exact; no uncertain outcome is made retry-safe.
3. No ignored/filtered case counted; ten ordinary unit cases are part of138,
   not138 independently database-using laws. Import error case9.107s is another
   unexplained setup tail, while selected identical case0.037s and SSI0.076s in
   this run. Do not invent a storage SLO; PG8 timing investigation remains open.

Feature-absence negative check: default/no-import package with the EXACT native
atomic smoke selector and --no-tests=fail exits4: zero tests, 'no tests to run'.
Three reviews: (1) advertised import absence cannot satisfy required atomic suite;
(2) production archive includes --all-features and gate uses same selector before
full unfiltered suite; (3) zero cases are expected FAILURE evidence, no vacuous
passes credited. The missing-database pipeline's prior native28000/exit1 evidence
remains valid; new final Linux gate must pass both smoke and full suite.

Default/no-import real Postgres suite113/113 passes,zero skipped/no filter,25.249s.
Three reviews: (1) common append/read/wake/migration/value laws run without import,
including changed shared insert_run and namespace predicate compilation; (2) native
prefix rollback and original read/wake regressions remain; (3) optional25 cases
are absent by requested feature selection, not counted as passing. This is native
execution, separate from DB-free default workspace results and all-feature138.

Fresh Linux ARM PostgreSQL18.6 gate passes at current source:
/nix/store/9ja47b32ws7r6lrb7rs95dkvhini3ndr-mnesis-postgres-integration.
Feature smoke1/1 passes0.333s; complete138/138 passes28.637s,zero skipped. Actual
fresh server initialization/start, SQL preflight, tests and shutdown recorded.
Three reviews of smoke: exact shared atomic law executes, native rows back its
assertions,14 filtered plus10 excluded binaries are not successes. Three reviews
of full run: (1) all required selected native cases present, including optional
import; (2) real scheduling/lock/transport faults retain their independent server
oracles and all original regressions; (3) no filter/skip, smoke law is a repeated
subset and NOT added to138. Both feature.log/run.log are installed. Linux check
aliases the existing package; Darwin's default workspace is not native evidence.

Post-138 adversarial review found missing direct evidence for waking EVERY local
atomic target before hints finish. Existing shared atomic laws inspect data, and
held-hint batch law proves one time budget but not each registration. Required
PG5 follow-up budget <=65 private native test lines, zero production changes:
reuse HoldHint proxy, arm each stream plus global registration, start atomic
write, observe first held native hint after commit, then require all armed waits
already ready before one-second fallback can possibly fire. Explicit elapsed<1s
is an oracle-validity check, not a new throughput target. Missing any local wake
must fail while native hint is held. No sleep-based fault injection or test seam.
The138 gates remain honest evidence of that snapshot; final suite will be139.
Before running the new wake case, source review caught another oracle hazard:
holding a pg_notify RESPONSE alone does not prevent its notification waking the
first target. HoldHint now discards native NotificationResponse frames in the
existing private proxy as well; the first hint still genuinely executes, but it
cannot satisfy ANY local registration. No new adapter seam. A first-target local
wake omission control must fail; elapsed fallback guard remains required. Initial
Clippy similar_names finding prevented execution, then ordinary local rename.

Local wake law1/1 native passes0.392s,138 filtered; strict Clippy passes.
Three reviews: (1) three distinct targets plus global arm before actual atomic
commit; all ready while first native hint response is held;9 rows really durable;
(2) native NotificationResponse frames are deliberately dropped, so server hint
cannot hide omitted local wakes; (3) elapsed<1s rejects fallback-timer false pass,
no filtered cases credited, pending task owned by JoinSet. Run archived
first-target wake omission control before accepting this stronger oracle.

Archived first-target wake omission control truly FAILS the new case:exit100,
0/1,138 filtered,0.219s. Failure is 'every stream and global wait must already be
ready', NOT fallback-validity/time/setup. Three reviews: (1) exactly first local
wake is omitted, remaining production behavior unchanged in archived tree;
(2) native notifications are dropped and observed hint confirms actual committed
write, so the missing wake itself fails; (3) elapsed<1s passes, excluding timer
fallback; current source remains untouched and positive selected case passes.

Final production-diff automated mutation sweep (snapshot138 cases):26 completed,
12 caught,14 compilation-unviable,zero missed/timeout; baseline138 passes21s,
auto test timeout107s, fresh physically separate18.6 cluster shut down cleanly.
All first failures and compile errors inspected. Three reviews for each outcome:
1. Contract mapping: head_version=None, prepare=Some(0/1/-1), prepare!= inversion
   and atomic=Ok(None) fail exact shared conflict/head laws; prepare=None fails
   missing expected-head1/actualNone/zero-prefix law. Classifier=true fails exact
   preserved23514 import outcome; classifier=false fails real concurrent winner
   law. Rejected&&->|| fails existing-head unrelated-unique source law.
2. Meaningful original survivors prepare=None and rejected|| are now killed by
   their specific native cases, not unrelated fixture failures. Two hint removal
   outcomes are fault-entry guards (observed command absent), not independent
   data-corruption witnesses; do not inflate their strength. Separate new archived
   local-wake omission control fails the observable ready-wait invariant itself.
3. All14 unviable are actual missing Default/new/from_iter or type-inference
   compilation errors, never credited as caught. Fail-fast unrun cases are not
   successes. No exclusion, calibration wrapper, synthetic substitute or relaxed
   deadline. New139th test changes no production bytes; final CI reruns the diff
   with that stronger test set before merge.

Final139 native suite passes139/139,zero skipped/no filter,66.891s on17.10 under
concurrent whole-workspace disk load. Three reviews: (1) all previously138 laws
plus direct all-target wake law execute; (2) omitted-first-wake archived control
fails its readiness assertion0.219s, positive case0.428s keeps fallback guard
valid; actual deadlock3.200s and SSI0.139s retain real native codes/data; (3) no
skips or softened assertions, ten ordinaryunit cases remain within139. Import
storage case14.697s is further setup/load noise evidence, not a production SLO.
Production diff SHA256 d21bb61135c2f6abb82cc1ceef6e1bea979817cea0c38c5e706d00b97a992941
is unchanged from the completed26-candidate mutation sweep; only test coverage
strengthened. FinalLinux139 and normal signed hook must pass before delivery.

Final fresh Linux ARM PostgreSQL18.6 suite139/139 passes21.938s,zero skips, plus
one separately counted feature smoke0.449s. Artifact:
/nix/store/h1afzq97r02p31k35pnzly9s4gpyi7c7-mnesis-postgres-integration.
Three reviews of each run: (1) smoke selects exact shared atomic case and full run
includes all139; (2) stronger HoldHint drops native hints, and positive local-wake
law runs alongside real native errors, transport uncertainty and prior regressions;
(3) filtered smoke cases are not successes, full run has no filter/skips, repeated
smoke is not added to139, clean fresh server shutdown and installed logs retained.
Package/check evaluation produced identical drv paths; Linux protected flake
check now owns this same complete native derivation. Darwin verification is
separate. Full signed-commit hook and protected PR/CI/review/merge remain required.

Earlier138-source full Nix gate's default nextest run completes1558 passed,
4 explicit compiler exclusions,545.710s (14 slow existing Fjall cases). Three
reviews after this run: (1) default workspace/conformance/property/state-machine
coverage runs normally; optional import runtime laws are separately139 native,
not inferred from this default feature run; (2) no regression or propertycase
was cut short to accelerate the gate; (3) four compile-fail exclusions are NOT
successes and require their existing separate CI jobs, DB-free adapter returns
are NOT native evidence. Source strengthened after this snapshot; normal commit
hook is running the final139-source gate separately. Earlier gate install/fixup
completion still being collected, no failure hidden.

Normal signed commit493f7e68361e5815c5927699bef6be5ac84ddfc7 passes all12 Darwin
flake checks (two unchanged checks cached,10 built), including fresh1558 default
workspace cases/4 explicit compiler exclusions,447.021s. Earlier138-source gate
also completes12 checks/1558 cases,545.710s; it is not counted as final139 proof.
Three reviews after final hook: (1) source matches final139 native proof, strict
Clippy all targets/features, formatting, dependency/advisory and no_std/wasm/doc/
fuzz gates preserved; (2) ordinary full256-case properties/state-machine laws
run, no slow case suppressed; (3) four compiler exclusions require separate CI,
optional runtime laws rely on real139 suite, cached checks and DB-free returns
are not counted as fresh native execution. Exact reviewed source requires CI at
its pushed head before merge. Documentation-only amendment uses the same tested
source and still runs the normal hook; no bypass or fresh-test claim on cache.

PG5 PR381 opened at signed head3d310e316cc72fb6af439e3b59da678d6b1c1ef6:
https://github.com/devrandom-labs/mnesis/pull/381. Protected CI/review is pending;
no merge or release claimed. Documentation amendment's12 cached checks pass.
Three reviews: same final139 production/test source; no changed assertions/fault
oracles; zero fresh execution credited from cached hook. Root worktree preserved.

PG6 native key-lock probe rejects the proposed raw-key NO KEY UPDATE lock with
actual42501 under the existing SELECT/INSERT runtime permissions. The whole
CREATE ROLE/grants/SET ROLE transaction is rolled back, so no role remains.
Three reviews: (1) native lock requires extra UPDATE permission on an immutable
key catalog; (2) real restricted role executes the SELECT, expected exact42501
is checked by psql and wrong code exits1; (3) zero adapter runtime cases credited,
no permissive grant or native error is hidden. Do not expand key mutation rights
just to implement checkpoint CAS. Prefer the checkpoint row's own native lock;
resolve absent-row creation with a full valid INSERT ON CONFLICT DO NOTHING and
observe the actual winning revision before returning conflict. Research and prove
that protocol rather than add placeholder rows, application locks or retries of
uncertain writes. Aggregate cache upsert needs no conditional protocol.

PG6a snapshot pre-edit budget: <=180 added/15 removed production Rust, <=100 SQL
lines for migration3 plus runtime validation, <=380 native owner test lines and
<=80 existing transport-fault lines. One snapshot feature forwarded to the owning
core; shared snapshot conformance is the primary executable law. Keep optional
projection checkpoint implementation in the subsequent PG6b slice; PG6 is not
complete after snapshots alone. Versioned snapshot table always installed and
validated independently of compiled features. Reuse raw catalog identity, byte
limits, SQLx transactions and existing CommitUnknown classification. Prove actual
reopen, corruption/stale boundaries, unsigned extrema, complete-pair isolation,
unknown commit and real Snapshotting/codec composition before retention. No new
framework/runtime/tool/dependency/public adapter method. Add tests before code.

PG6a shared snapshot characterization: compilation fails E0277, PostgresStore
lacks SnapshotStore<Vec<u8>,Version>, zero runtime tests. Three reviews: shared
five law signatures match the existing owning trait; the missing implementation
is the actual compiler failure rather than absent database setup; no skipped or
unrun runtime case is credited. Add owner-local lifecycle, byte/corruption and
actual concurrent pair isolation tests before implementing migration/trait.

PG5 protected merge381 completed2026-10-18T11:33:00Z:
be2e52ec3468657d36684506eed53fe060bef080, matching head3d310e316cc72fb6af439e3b59da678d6b1c1ef6.
Forty green checks/one scheduled skip, CLEAN protected merge, no review threads.
CI39122783424 passes1558 default workspace/4 compiler exclusions, both separate
compiler jobs pass their fixtures. Two actual native139 runs18.900s/22.353s,
feature smokes separately; installed22.353s log replay is SAME run. Push workflow
39122630749 also passes1558 and native13919.093s/22.069s on fresh jobs. Mutation
39122783429:26 candidates12caught14unviable0missed/timeout, actual native baseline.
Three reviews per CI run: (1) full selected139 and feature smoke remain present,
compiler exclusions have passing owner jobs, advertised feature jobs pass;
(2) original schedules/fault guards and stronger omitted-wake oracle retained,
no survivor masked; (3) scheduled skip, filtered smoke, repeated installed logs
and DB-free default workspace returns are not extra native successes. Separate
push/PR jobs count as separate executions only where actual starts/logs show it.
No publication or complete-adapter acceptance claimed. PG6 remains required.

PG6a first native9/9 pass0.267s,zero skips; all-target/all-feature Clippy passes.
Three reviews: (1) shared5 plus reopen/fullunsigned/unconditionaloldercache,
byte/stale/corrupt metadata and real blocked UPDATE pair isolation execute;
(2) actual native advisory INSERT-wait proves reader overlaps staged UPDATE;
(3) GAPS FOUND: that schedule does not force a commit BETWEEN independent reader
queries, and tiny8-byte stale/oversize state cannot reject whole-body loading.
Do not call those bounded-memory or split-read falsifiers yet. Keep these cases
and add stronger oracles before retention, no production change is needed.
Follow-up verification budget <=100 native lines plus <=25 existing proxy lines:
reuse backend DataRow hold so independent update commits between first read
response capture and reader continuation; archived split-read mutant must yield
mixed pair and fail. Add real1MiB-state allocation bound using existing read-test
allocation pattern; stale and RowLimit reads must not hydrate body, and an archived
CASE-body omission must fail. This strengthens missing acceptance evidence rather
than replacing or relaxing original cases. No new framework/runtime/tool.

Strengthened native2/2 pass,148 filtered,0.100s; all-target Clippy passes.
Three reviews: (1) captured old DataRow is held while an independent production
cache commit completes, then reader must return old complete pair; next read is
new complete pair; (2)1MiB stored body read under37-byte budget allocates11,306
bytes for stale and9,561 for RowLimit, both below262,144; TOTAL counts temporary
allocations even if freed or baseline buffers are released; (3) private HoldSnapshot
selects the unique seeded old state, no unrelated startup row can satisfy the
boundary. Archived split-read and full-body counterfactuals still required; this
selected run is not full150-suite proof. Existing read allocation fixture is
shared, its LIVE/PEAK semantics unchanged; one total-allocation counter added.

Archived controls both reject actual defects (working source unchanged):
- Split-read control:exit100,0/1,149 filtered,0.054s. Actual position1/NEW state
  versus expected position1/OLD state. Three reviews: only second independent
  SELECT added; real held DataRow plus independent committed update forces the
  interleaving; actual data assertion fails rather than synchronization/timeout.
  Narrow state-marker fixture rejects other query shapes at synchronization
  rather than claiming a semantic kill without observing a captured old row.
- Unconditional body-fetch control:exit100,0/1,149 filtered,0.037s. Stale1MiB read
  allocates6,294,809 bytes versus262,144 ceiling. Three reviews: only CASE ELSE
  changed to return body; real byte-store query allocates the unwanted body;
  failure is allocation invariant, not decoding/setup/timing. No threshold raised.
Both selected positive cases remain2/2 with~11KiB/~9KiB allocations. These controls
close the two concrete weaknesses found after the initial9-case run.

PG6a real-consumer verification pre-edit budget <=200 native test lines: use
existing Counter domain, JSON codec and Snapshotting decorator. Prove a valid
cache skips a deliberately undecodable historical prefix (plain repository must
fail Decode), stale-schema cache state is ignored, and rejected cache replacement
cannot undo acknowledged event append or overwrite the old cache pair. Restore
faults before assertions. No probe store, added kernel getter or new interface.
Existing codec transport tests cover typed uncertain outcome and caller timeout.

Source-provenance correction: earlier body control reported0.06s/no compilation
using shared target after a different archived mutation. CWD/manifest and source
ELSE are correct but not enough to establish executed library identity. BOTH
prior snapshot counterfactuals are PROVISIONAL until fresh-target repetitions
with explicit adapter recompilation paths. No bound or assertion is weakened.
Fresh proof target is recorded in /tmp/mnesis-pg6a-proof-target. Current source
and normal positive tests are separate. Keep previous logs as superseded evidence.

Consumer selection failed honestly:10 passed,1 failed,2 unrun,142 filtered.
Failure is Events capacity N=0 before the snapshot scenario; valid2/3-event seeds
need additional-event capacities1/2. Three reviews: correct setup cardinality,
no adapter failure inferred from this panic, no consumer pass or unrun success
credited. Fixing inputs preserves every downstream assertion. Earlier compile/
Clippy failures ran zero cases. Fresh native verification remains required.

Fresh-target body control confirms actual unconditional-fetch defect: adapter
compiled explicitly from /private/tmp/mnesis-pg6a-body-control.0MsmOm into fresh
/tmp/mnesis-pg6a-proof-target.z1jmzk (all dependencies compiled,12.35s). Test
exit100,0/1,149 filtered,0.063s: stale allocation6,280,373 >262,144. Three reviews:
exact CASE ELSE difference; failure is unwanted real allocation; actual compile
path and distinct target now verified, no stale artifact credited. This fresh
result supersedes earlier provisional body control.

Fresh split control explicitly removes806.6MiB adapter artifacts, recompiles
mnesis-postgres from /tmp/mnesis-pg6a-split-control.P92ykM into the isolated proof
target, then fails actual position1/NEW-state assertion:exit100,0/1,149 filtered,
0.039s. Three reviews: only second SELECT differs from working source; independent
committed update really yields mixed pair; explicit compile path and target are
verified and no synchronization/timeout failure counted. Fresh body and split
results now supersede both provisional shared-target results. Future archived
controls must use isolated targets and force adapter rebuild on source-root changes.

Corrected consumer selection13/13 passes1.383s,142 filtered; strict Clippy passes.
Three reviews: (1) exact real JSON Counter cache and tail values, stale999 cache
ignored, rejected cache replacement leaves four acknowledged events and original
cache pair intact; (2) plain repository fails Decode on deliberately damaged
prefix while cached repository succeeds, and byte/uncertainty cases run actual
adapter; (3) corrected Events capacities match seed counts without weakening
assertions, faults restored before assertions, no filtered/unrun cases credited.
This is selected proof, not full155-suite delivery.

PG6a process-recovery pre-edit budget <=110 native test/fixture lines. Reuse the
existing Fjall std-process SIGKILL-after-marker pattern (inspected
adapters/fjall/tests/crash_recovery.rs and support/mod.rs), keeping the Postgres
fixture with its adapter. No new runtime/dependency/tool/public library helper.
One existing libtest binary self-invocation writes through actual cache API,
flushes ACK marker after successful commit, remains alive with handles retained;
parent verifies SIGKILL9 then hydrates exact pair from a reopened store. This
establishes client-process crash recovery, not server/power-loss survival (PG8).

Client-process SIGKILL case1/1 passes0.066s,155 filtered; strict Clippy passes.
Three reviews: (1) child acknowledges actual cache commit, parent kills with9 and
reopens exact full-u64/schema/binary-ID/state pair; (2) exact pipe marker prevents
zero selected child tests from passing, explicit signal rejects test panic or
graceful shutdown as crash proof; (3) RAII cleanup covers timeout/panic and reaps
child, no extra child stage credited as a second passing test. This covers client
process death only; primary restart and storage durability remain PG8.

PG6a schema/privilege follow-up budget <=150 native lines: real LOGIN role with
only metadata/key lookup/insert and snapshot select/insert/update, no key UPDATE,
event mutation or schema creation. Capture actual login identities and42501
failures; close runtime pool/drop grants and role before assertions. Also reject
missing snapshot FK, nullable body and unlogged table, restoring every mutation.
No production permission expansion or new roles in migrations.

Mutable-read audit found runtime schema validation reads migration metadata and
multiple table shapes without one shared transaction/snapshot. The PG6 snapshot
table adds another such read. Standing database rule requires one coherent view.
Before fixing, add a native held-validation-row witness and inspect real backend
state/xmin; it must show idle-in-transaction with a retained view, not autocommit
idle. Budget <=25 production Rust/<=65 native+12 existing proxy lines; reuse
SQLx supported READ ONLY REPEATABLE READ and scoped connection cleanup, no new
public interface/dependency. Explicit rollback ends a read-only view without
introducing write commit uncertainty. Verify ordinary readonly-role open and all
migration/lifecycle laws afterward; no extra long-lived cursor transaction.

Validation characterization truly fails0/1,158 filtered,0.059s. Native backend is
('idle',false) after first metadata response, expected ('idle in transaction',true).
Three reviews: (1) no shared catalog snapshot spans subsequent checks;
(2) exact binary boolean DataRow hold plus unique application/backend lookup is
actual server evidence, and cleanup finishes before assertion; (3) no timeout or
unrelated startup failure counted. backend_xmin retained while idle distinguishes
REPEATABLE READ from READ COMMITTED's statement snapshots. Explicit READ ONLY is
verified by source statement plus existing restricted-readonly-pool regressions.

Coherent validation plus role/layout selection4/4 passes0.183s,155 filtered;
strict all-target Clippy passes. Three reviews: (1) metadata view now retains xmin
while idle, old readonly-pool open still works, role uses REAL LOGIN identities
and only intended cache permissions; (2) wrong FK/nullable body/unlogged table
reject, each restored before assertion; four denied operations return42501 while
cache insert/overwrite/hydrate succeeds without key UPDATE; (3) exact identities,
SQLSTATEs and full pair are asserted, all role sessions/grants removed, no filtered
case credited. Brief READ ONLY REPEATABLE READ validation explicitly rolls back;
scoped pool lease closes on cancellation, not caller-owned pool. Production
removed-line budget increases15→25 because shared byte validation/key lookup
remove duplicated code; added Rust remains within180. No acceptance is dropped.

PG6a upgrade follow-up budget <=90 native lines: build exact frozen versions1/2
using supported SQLx Migrator, populate event, collide with foreign snapshots
relation, require migration3 rollback/no metadata advancement/no data loss; rename
foreign relation and repair forward, verify preserved old position and next event
sequence. Extend existing upgrade fixture with explicit last-known version.

Version2→snapshot upgrade1/1 passes0.052s,159 filtered; strict Clippy passes.
Three reviews: (1) real SQLx versions1/2 and existing event position/sequence are
preserved across3; (2) foreign table causes actual42P07 with only[1,2] recorded,
its data survives failure and explicit rename/forward repair; (3) original row
and exclusive resume/new sequence verified, no destructive reset or weakened
legacy assertions, failed fixture is repaired before checking outcomes.

Fresh READ COMMITTED control (only BEGIN statement changed) recompiles adapter
in isolated proof target after cleaning adapter artifacts, then FAILS expected
retained-view assertion0/1,159 filtered,0.068s. Actual ('idle in transaction',false)
rejects merely grouping checks in a READ COMMITTED transaction. Three reviews:
correct coherent-view contract; actual backend xmin proves statement snapshot
released; source compilation/cleanup verified, no setup/timeout or unrun success
credited. Current REPEATABLE READ source passes that same native witness.

First full160 native selection FAILS:74 passed,1 failed,85 unrun,6.617s. Fresh
mutation baseline fails same case (7.078s), so ZERO mutants tested; its server
shuts down cleanly. Three reviews of each: (1) intended LISTEN failure never
occurs; (2) fixture assumed schema connection returns idle, but new scoped
validation correctly closes it, so listener creates a fresh connection bypassing
before_acquire hook; (3) not a recovery pass, unrun cases not credited, deadline
not increased. Preserve exact native57P01 injection and LISTEN recovery assertion:
prewarm two idle sessions with one held at capacity3, leaving an actual idle
session after schema validation closes its own. Only fixture setup changes.
This preserves the original acquired-backend-before-LISTEN fault; no production
cleanup is weakened to make the prior fixture assumption true. Rerun selected,
full suite, fresh mutation and final Linux gate at corrected test source.

Repaired LISTEN fault fixture1/1 passes1.091s,159 filtered; strict Clippy passes.
Three reviews: real idle acquired backend still terminated with exact57P01 before
LISTEN; recovery must establish actual LISTEN and deliver the500ms hint (not1s
fallback); only prewarmed test capacity changes2→3, original deadlines/assertions
stay. No success credited to failed full/mutation baselines. Rerun complete proof.

Scoped-view cleanup verification pre-edit budget <=60 native lines, zero product
changes. Cancel runtime opening while the first catalog response is held: prove
actual backend xmin is pinned before abort, then no view remains pinned without
releasing that response; caller pool must still answer SELECT1. Observe view
lifetime rather than require a particular number of idle connections. This tests
the new close-on-drop cancellation claim, not a synthetic delay or write retry.

Second full160 fails81 passed/1 failed/78 unrun,31.626s. Fresh mutation baseline
fails the same assertion (23.436s), ZERO mutants tested and clean server shutdown.
Three reviews of each: all earlier rollback/data assertions pass, but final count
hardcodes2 migrations although3 is now required; no adapter rollback defect is
inferred; no unrun cases or mutation ratio credited. Compare final count to the
checked-in SQLx migration list rather than a stale literal. The cancelled state
must still have ZERO applied rows and preserve exact legacy data/position.
Earlier Linux candidate fails its known old LISTEN fixture and shuts down server;
that candidate is not final gate evidence.

Canceled runtime-open case1/1 passes0.068s,160 filtered; Clippy passes. Three
reviews: real xmin pinned before abort and released while response remains held;
JoinSet cancellation actually drops constructor future and caller pool still
answers SELECT1; no write/cancel retry inference or idle-connection-count oracle.

Third native161 run81 passed/1 failed/79 unrun8.883s: first stale count was fixed,
but a second occurrence in cancelled-migration final verification remained2.
Three reviews: exact3-versus2 setup expectation, all preceding cancellation/zero
metadata/data preservation checks still pass, no acceptance waived. Both final
count assertions now use checked-in SQLx migration count; zero-applied rollback
assertions remain exact0. Rerun complete matrix after this correction.

Fourth full161:85 passed,1 failed,75 unrun9.103s. Fresh-install test still listed
only1/2 and checked checksum lengths. Three reviews: actual metadata has all three
correct successful rows; this is another stale fixture expectation; unrun tests
are not successes. Audited migration tests for remaining fixed current-version
counts/lists. Fresh-install now compares exact version/success/checksum BYTES to
all checked-in SQLx migrations, strengthening its former48-byte length check.
Frozen legacy1/2 fixtures and zero-after-cancel assertions stay frozen/exact.
Next full run uses --no-fail-fast to expose remaining failures in one pass.

Complete corrected161/161 native passes19.203s,zero skips/no filter, --no-fail-fast;
strict Clippy passes. Three reviews: (1) existing139 plus22 snapshot/coherence/
size/upgrade laws all run, including actual SIGKILL and typed codec consumers;
(2) fresh-target split/body/READ COMMITTED controls reject their specific defects,
original read/wake and real SSI/deadlock/commit fault witnesses remain; (3) stale
migration lists now compare exact known checksum bytes and full count, rollback
zero/data assertions unchanged, LISTEN fixture still requires57P01 and500ms hint,
no unrun/skipped case credited. Eleven ordinary unit cases are within161, not161
independently database-using laws. Final Linux/mutations/feature isolation and
matching signed full gate still required before focused PR/delivery.

Default native115/115 passes15.533s, zero skips/no filter. Three reviews: (1)
feature-independent migration3 and existing event contracts execute without
snapshot/import; (2) original ordering/read/wake/recovery witnesses still execute;
(3) eleven ordinary unit cases included, no optional-feature absence counted as
capability proof. Final isolated import/snapshot runs remain required.

Fresh ARM Linux18.6 candidate161/161 passes22.859s, zero skips/no filter, plus
separate exact atomic smoke1/1 at0.213s and snapshot smoke1/1 at0.194s. Three
reviews of EACH run: (1) expected full archive and exact capability selectors
execute with nonempty real DATABASE_URL; (2) complete native fault, migration,
role, process and coherence laws pass; (3) smoke selections intentionally filter
other cases, are not added to161; installed run.log is replay of this same
execution, not a second pass. Artifact
/nix/store/d6yfgrmkmwv53i8cmqc92dn9g9zijrgs-mnesis-postgres-integration.

Earlier Darwin candidate gate12/12 checks passes1560 default workspace cases,
499.728s, four compiler exclusions. Three reviews: (1) whole-workspace checks
remain green; (2) default runs without DATABASE_URL and do NOT prove native
database behavior; (3) source predates final fixture fixes, so this is not the
matching final signed gate; four compiler cases still require separate CI.

REJECTED mutation result after three reviews: fresh baseline161/161 passes
19.895s, but nominal23=21caught/2unviable is NOT credible. First mutant disabling
validate_pool fails the checksum-rejection assertion BEFORE restoring corrupted
migration metadata; following mutants mostly fail unrelated VersionMismatch(1)
during first database setup. Only the first validation rejection and the byte
overflow unit failure demonstrate their own defects. Two E0277 unviable outcomes
remain compile failures, never caught. (1) contract review requires behavior
oracles; (2) actual first-failure inspection exposes persistent fixture poison;
(3) no nominal ratio will be credited or used for delivery. Pre-edit test-only
budget <=65 added/25 removed lines: separate checksum/future-version cases,
capture observations, restore exact metadata BEFORE any behavioral assertions.
Rerun full native and physically isolated fresh mutation sweep; no exclusions,
weakened assertions or schema resets in product code. Earlier user-facing nominal
mutation summary is explicitly superseded by this diagnosis.

Corrected complete native162/162 passes43.725s, zero skips/no filter, strict
Clippy passes. Three reviews: (1) both checksum and future-version rejections keep
exact original Schema/Migration error and untouched-metadata assertions; (2)
faulty migration metadata is restored before assertions, without weakening the
rejection oracle; (3) all native fault/consumer/corruption/upgrade laws execute,
eleven ordinary units included, no filtered/unrun success credited. Runtime
outlier23.100s in import-storage fixture remains measured PG8 investigation,
not erased or used as a deployment bound. Mutation, isolated features and
matching Linux/full gates still pending.

Isolated snapshot-only native136/136 passes15.470s, zero skips/no filter. Three
reviews: (1) snapshot feature works without import, including its real consumers,
SIGKILL, schema/byte boundaries and complete-pair laws; (2) original event/wake/
phase regressions still execute; (3) feature-independent checksum/future tests
remain exact, absent import laws are not credited, no workspace feature
unification used. Fresh mutation baseline162/162 passes19.609s: three reviews
confirm real separate18.6 cluster, complete expected suite, no skipped cases;
baseline alone establishes no mutation coverage. First mutant failures still
must be inspected individually before crediting the sweep.

Isolated import-only native142/142 passes41.972s, zero skips/no filter. Three
reviews: (1) atomic/import and feature-independent migration3/coherent validation
work without snapshot implementation; (2) complete original transaction/wake/
representation fault laws execute; (3) native URL and exact source verified,
snapshot-only consumer laws absent and not credited. Missing-snapshot exact
selector under import-only build fails exit4, zero cases. Three reviews: intended
absence of advertised capability is rejected by --no-tests=fail; exact required
selector is identical to Nix smoke; no test or database behavior is claimed by
this expected-negative guard, and no command failure is masked.

Corrected-source ARM Linux18.6 gate162/162 passes29.821s, zero skips/no filter;
separate exact atomic smoke1/1 at2.702s and snapshot smoke1/1 at8.373s. Three
reviews of EACH: (1) required feature selectors and complete archive execute
against native server with explicit URL; (2) updated failure-cleanup cases and
all native contract/fault witnesses execute; (3) smoke filters not added to162,
installed log is same execution, setup tails retained rather than hidden.
Final artifact /nix/store/kmvfxnly8y4dbhgcz3k0hn5zv9x2qgb7-mnesis-postgres-integration.

Final-source default native116/116 passes28.593s, zero skips/no filter. Three
reviews: (1) both independent migration metadata negatives and coherent runtime
validation execute without import/snapshot; (2) original ordering, bounded-read,
commit and missed-final-hint defects remain covered; (3) native URL is explicit,
eleven ordinary units included, optional suites absent and not credited. Together
default116/import142/snapshot136/all162 cover every current feature combination;
PG6b/PG7 and final downstream/full feature coverage remain required.

Disk pressure: standard Cargo clean removed only the accidentally generated
state-worktree target (545.9MiB). Cargo refused the isolated proof target because
its CACHEDIR.TAG was absent; left it intact. Source controls and raw evidence are
preserved. No user work, active shared target or broad Nix store cleanup performed.

PG6a source minimization review: no new public constructor/method, dependency,
runtime or runner; existing SnapshotStore/CodecSnapshotStore/Snapshotting own the
contracts. The raw key lookup is private and reused by key creation; append's
combined head query remains one query rather than splitting a coherent read.
Validation uses SQLx's supported Connection::begin_with and rollback, no custom
transaction machinery. Whole Rust diff187 added/22 removed includes17 lines of
host-only overflow-test scaffolding; production170/22 stays within180/25. SQL43
within100. Native verification growth is the separately declared pair/allocation,
real-consumer, SIGKILL, role/view, upgrade, cancellation and failure-cleanup passes;
it is not disguised as production size or a single original test budget.
Production source diff SHA256 (including embedded validation SQL)
d614c2afe3d6ee297fa22ec9681e3194593dee9344e58f3049b82f4bf72e5c32.
Final whole-repository minimization/audits remain mandatory PG9 work.

Corrected mutation outcomes:23 candidates,21 caught,2 E0277 compile-unviable,
zero missed/timeouts. Three separate reviews performed for EVERY candidate run:
(1) match the changed contract to the observed error/value; (2) inspect the actual
first failure and its native fault/data evidence; (3) reject leftover-state,
setup-source, filtering and timing explanations. Exact first failures are kept
in pg6a-clean-mutation-first-failures.log; outcomes JSON retains every phase.
The three removed validation functions now fail the intended corrupt-checksum
rejection AFTER repair, not a poisoned successor. Negated shape check rejects a
valid schema; size helper mutations reject exact overflow/oversize/edge bounds.
Wrong key lookups cause actual FK rejection, inability to reuse an existing key,
or missing data under the requested raw ID; these prove key/commit failures,
not independent COMMIT-uncertainty behavior. Snapshot schema comparison returns
Stale instead of the exact captured pair; size changes violate oversize or exact
edge behavior. Snapshot commit no-op is caught by the required native COMMIT
fault-entry guard (returns Ok before actual COMMIT); do NOT describe that catch
as an independent post-commit persistence oracle. The shared direct cache
commit/hydrate laws execute in the full native suite. Fresh split/body/view
controls supply separate behavioral falsifiers. Two Default substitutions cannot
compile because Hydrated has no Default and receive no caught credit.
No VersionMismatch(1) contamination, timeout, assertion weakening or exclusion
supplied any credited catch. Final sweep process cleanup/end_time and native
server shutdown still need verification before recording complete run delivery.

Corrected23-candidate sweep exits0; fresh Postgres18.6 server stopped cleanly.
Three final cleanup reviews: all23 completed with zero missed/timeout; no DB-free
or filtered success credited; temporary source/build cleanup and server stop
completed. Runtime13min under concurrent disk-heavy workspace gate is recorded,
not a production performance number. Original poisoned sweep remains rejected.

Matching source Darwin nix flake check passes all12 checks:1561 default-workspace
cases,557.266s, four compiler-case exclusions. Three reviews: (1) all12 checks
cover strict Clippy/format/audit/deny/hakari/docs/fuzz/wasm/no_std plus tests at
retained source; (2) real database evidence remains the separate native feature
matrix and ARM gate, not default workspace DB-free returns; (3) no filter or
retry changed to hide a failure, all1561 ran, four compiler exclusions require
separate CI. Slow Fjall properties preserved, no deadlines or assertions changed.
First signed commit1886d1cb3af63293a335019d0dc5070a407aa8ae has valid signature.
Its NORMAL pre-commit hook passes all12: eleven already built, nextest waits on
this SAME fresh derivation. Three hook reviews: required gate not bypassed;
matching derivations; cached/shared results not counted as another fresh run.
Documentation/evidence amendment will use the same normal hook, with cached
results explicitly distinguished. Raw fault logs retain nextest's trailing spaces;
source/document whitespace checks exclude only raw evidence and pass.

The first docs amendment was INTERRUPTED, not passed: checked-in raw outcome JSON
unexpectedly matched the existing global Nix JSON source filter, invalidating all
build identities despite unchanged product sources. Preserved identical tool
output as pg6a-clean-mutation-outcomes.log and rerun the NORMAL signed hook.
No hook bypass or source-filter exception added. Three reviews of aborted attempt:
no completed test result credited; no source/test/SQL assertion changed; prior
matching full/native/mutation proofs retained. PG9 filter-minimization hypothesis
must cover unrelated evidence JSON as well as unrelated SQL. Nix will verify
final retained source through the normal gate; cancellation alone is no success.

Corrected amendment normal hook PASSES all12: eleven cached checks (including
1561-case nextest), one rebuilt no_std store check with the SAME derivation as
prior full gate. Three reviews: normal hook retained; exact nextest derivation
fxd7c7scy2w6c2d49nwxavw1in2b752p matches fresh557.266s run; no extra native or
workspace execution credited to cached results. Final Linux derivation
 i382xb9bjyjkh1fl341gz5jnzr7pzhm3 matches recorded162-case artifact. The aborted
JSON-only rebuild log stops in compilation and supplies no test execution proof.
Final evidence-only amendment retains identical Rust/SQL/test/flake inputs and
again uses the normal gate; all-cache checks are reuse, not another fresh run.

PG6a reopened during PG6b design, BEFORE further implementation: SnapshotStore
uses execute() and ignores affected-row count. A valid native BEFORE trigger can
return NULL, producing successful command status with zero rows saved. Hypothesis:
current commit incorrectly returns Ok and may commit a newly created raw key.
Pre-edit budget <=5 production lines plus <=75 native lines. Reproduce BOTH
suppressed INSERT and suppressed ON CONFLICT UPDATE against real17.10. Preserve
all fixture cleanup before assertions. Expected contract: definite pre-COMMIT
storage failure, old pair retained, missing pair absent and no orphan committed
new key. Compare SQLx supported fetch_one with RETURNING against custom count/error
machinery; no new public error or retry classification. PR382 must not merge
until actual red/fix and final native/mutation/Linux/full gates rerun. PG6b source
is preserved separately; this discovery is not dismissed as a cache optimization.

Suppressed-write characterization first attempt fails E0277 during compilation:
SQLx0.9 SqlSafeStr rejects a formatted dynamic CREATE TRIGGER string. Zero runtime
cases. Three reviews: contract oracle not exercised; supported SQL safety boundary
caught test-fixture construction, not an adapter defect; no red reproduction
credited. Replace two fixed operations with two static SQL literals, not a safety
escape wrapper. Then rerun the original unchanged adapter.

Native suppressed-write reproduction FAILS as intended:0passed/1failed/15filtered,
24.922s, actual public commit returned Ok after trigger suppressed storage.
Three reviews: (1) valid cache replacement requires a stored pair or an error;
(2) actual BEFORE INSERT and BEFORE UPDATE both executed with cleanup BEFORE
assertions; native independent facts (confirmed after the test process completed)
show existing old pair, missing cache, TWO committed raw keys; (3) no fixture,
timeout or unrun case credited, real incorrect Ok is the failure. Use supported
SQLx fetch_one + RETURNING key: zero-row command becomes typed RowNotFound before
COMMIT, rolling back any newly created key, without a new error/check framework.
This addresses zero affected rows; it does not promise defense against a malicious
database operator rewriting arbitrary returned values. No retry-safety weakening.

Suppressed INSERT/UPDATE regression passes1/1,15filtered,14.736s. Three reviews:
(1) both native suppressions return exact Sqlx(RowNotFound) before COMMIT; (2)
missing cache stays absent, existing complete pair stays old, and raw key count
is exactly1, proving failed creation rolled back; (3) triggers/function cleaned
before assertions, original incorrect Ok really failed the same law, no skipped
case or constructor latency excused. Private SQL UPSERT adds RETURNING key and
supported fetch_one, no new variant/policy. Native setup tail14.7s remains unexplained
PG8 evidence even outside the full workspace test run; do not claim a causal
explanation or tune deadlines merely to pass. Full163 now includes this law.

Corrected complete163/163 native passes45.275s, zero skips/no filter. Three
reviews: (1) new native suppressed-write rollback law and all prior event/snapshot
contracts execute; (2) actual original false Ok is rejected, exact missing cache/
old pair/new-key rollback are asserted, COMMIT uncertainty still has native
ACK-loss/timeout witnesses; (3) no timeout, filter, assertion or fixture cleanup
weakened, eleven ordinary units included, setup tails retained as unresolved PG8
measurement. Strict all-target/all-feature Clippy passes. Final production source
diffSHA4862f765357b6549c691337c51b4353cffcc0b3dd16d2b9c5329f88a9081ccd1.
Fresh separate18.6 mutation baseline163/163 passes20s, three reviews confirm real
server/full expected suite/no skip; baseline alone proves no mutation coverage.

Updated snapshot-only137/137 passes21.659s, zero skips/no filter. Three reviews:
new suppression failure/rollback law works without import; original snapshot
consumer/pair/uncertainty/corruption laws remain; no filtered/import proof credited.
Updated ARM Linux18.6 full163/163 passes20.867s, zero skips; exact atomic1/1 at
0.146s and snapshot1/1 at0.133s. Three reviews of EACH: expected complete feature
archive and selectors; real native fault/consumer/new suppression evidence;
smokes are separate filtered runs and installed run.log is same execution.
Artifact/nix/store/1lh77qhvkks0fyhyjnflr9lrpbfm307v-mnesis-postgres-integration.

Candidate PR382 head475a73ff CI native162/162 passes24.891s; exact atomic1/0.080s,
snapshot1/0.058s separately. Three reviews of EACH: real database and expected
feature suite, original native laws, installed-log replay counted once; this
CANDIDATE lacks the newly found suppression law and is NOT final-source evidence.
Candidate compiler job117685117671 passes four functions (2/108.20s,1/3.90s,
1/11.69s). Three reviews of EACH run: actual intended compile-pass/fail fixtures;
diagnostics enforced by trybuild; four functions excluded from ordinary nextest
are covered, not added to its count. Candidate mutationCI23=21caught2unviable,
baseline25s. Three reviews: real freshDB/full feature baseline; source matches
local corrected first-failure proofs; CI summary alone lacks per-case logs, so
it cannot independently prove which assertion killed each mutation.

Pre-edit CI evidence budget <=9 YAML lines: preserve mutants.out using SAME
stock actions/upload-artifact@v4 and always/if-no-files-found pattern already
owned by fuzz.yml. No new runtime/tool/framework/custom validator. Mutation exit
status still owns gating; a docs-only PR may have no mutations. For this code PR,
require downloading outcomes/log/diff and reviewing every CI first failure before
merge. Missing expected artifact prevents our acceptance, never papered over by
summary ratio. This implements the user's three-review requirement on CI cases.

Updated default native116/116 passes25.883s, zero skips/no filter. Three reviews:
current feature-independent migration/view/event contracts; original read/wake/
commit faults; no optional snapshot capability claimed from disabled code and
no cached or filtered result counted. Snapshot-only/full native separately
cover the changed UPSERT. Updated import-only follows serially on localcluster.

Updated import-only142/142 passes39.200s, zero skips/no filter. Three reviews:
atomic/import feature works without snapshot impl; original transaction/event/
recovery laws remain; no snapshot capability credited from disabled code. All
four current combinations now pass at corrected source (116/142/137/163).
Fresh final163 native mutation baseline is20.190s; all23 finish in7min and server
stops cleanly.21 caught/2 E0277unviable/zero missed/timeout. Three reviews for
EVERY run: match mutated contract to actual first failure; inspect actual native
values/errors; check source, fixture cleanup, filtering and timing loopholes.
All first failures retained separately; no VersionMismatch contamination. Exact
checksum/valid-layout rejection, overflow/oversize/edge behavior, valid raw-key
commit, snapshot pair/schema/byte boundaries fail as intended. No-op commit
still dies at COMMIT fault-entry guard, not an independent persistence oracle;
our new original-source suppressed-write red/fix supplies actual zero-row storage
proof. Existing direct cache shared laws execute in full suite. Neither of the
Default substitutions compiles and neither counts caught. No exclusions or
assertion weakening. Updated normal full gate and CI at pushed head remain.

Candidate head475a CI39130711194 completes green; PR382 has40 green/one scheduled
skip at that head. Complete CI logs inspected: native162/24.897s in Nix check is
a separate actual run from native162/24.891s in integration job; installed replay
of24.891s is not third run. Nix workspace1561/121.259s with4 exclusions, separately
covered four compiler functions. Three reviews for EACH native/smoke/workspace/
compiler and feature-matrix run: intended source/feature contract; actual original
regressions/diagnostics; exact counts, expected empty harnesses, filters and replay
limits. Core feature-matrix successes do not prove Postgres optional capabilities;
its native feature matrix is separate. Whole candidate remains superseded by
new163-case suppression fix, which still needs new-head CI and artifact review.
CI feature jobs include expected zero-case library harnesses: compile evidence,
not new behavioral coverage. No four-exclusion or installed-log double credit.

PG9 dependency audit follow-up: candidate crate-audit log emits inability to
check registry yanked status (`No such crate in crates.io index`) despite green
vulnerability gate. Do not describe that as verified yanked-package status.
Inspect owner/tool behavior and use maintained Cargo/audit/registry facilities
for final delivery evidence; no custom checker or new security ignore. Full
vulnerability/license and clean registry-consumer proofs remain required.

Hardened-source NORMAL signed hook passes ALL12:1561 workspace tests/489.257s,
four compiler exclusions; two cached checks/ten built. Signed followup
 e2367cd0dfb15a84422453f4eb33f63cce85119a has valid signature. Three reviews:
(1) required complete workspace/check matrix at corrected source; (2) native
163-case/feature/Linux/mutation proofs remain separately required and passed;
(3) no shortened workload, deadline, filter or source exception, no four-case
compiler or DB-free/installed-log double credit. Native startup tails remain
PG8 measurement, not hidden by this passing gate. Raw log
/tmp/mnesis-pg6a-final163-workspace.log; nextest derivation
 w25gn62nfxians5f3dl73sqvqcxawl94-mnesis-nextest-0.4.0.
Final evidence amendment uses the same normal hook; cached reuse adds zero
fresh runs. Then push corrected head, rerun ALL CI, inspect uploaded CI mutation
outcomes/logs/diffs and review threads before protected merge. Original candidate
475a's40 green checks never substitute for that new-head delivery verification.
PG6a focused PR382 opened at signed475a73ff8e72566d029515e6c03f744e16623695:
https://github.com/devrandom-labs/mnesis/pull/382. Final normal hook12/12 allcached.
Three reviews: existing required gate executed without bypass; nextest/native
source identities match retained fresh proofs; cached checks add zero fresh test
runs. CI39130711194 and mutation39130711227 queued; all review/CI/protected merge
still required. PG6b starts separately while CI runs, not a claim PG6a merged.

PG6b pre-edit locked ownership: existing checkpoint.rs next_revision owns revision
comparison, schema/rebuild/position validation and overflow. Reuse it under actual
Postgres row lock; no duplicated predicate policy in SQL. Migration4 checkpoint
record uses separate table/raw key number, schema/revision/txid/global_seq/state;
all u64 scalars remain lossless NUMERIC(20,0), zero position is valid, revision
nonzero. All fields read/replaced together, metadata validated before Stale,
byte limits before BEGIN and body materialization. Existing SQLx/Tokio facilities
only; no dependency/framework/runtime/new public checkpoint API.
Budget recorded BEFORE implementation in HYPOTHESES:260/30 productionRust,
100SQL,500 native laws,100 existing proxy lines; later consumer/recovery passes
need separate explicit budgets. First four test categories precede implementation:
protocol rejection/rebuild, persisted reopen/extremes, corruption/overflow, and
native overlapping creators/advancers. Preseed raw key for true absent checkpoint
race; otherwise key creation serialization could conceal broken checkpointCAS.
Hold native BEFORE INSERT/UPDATE trigger with an administrative advisory lock,
observe both writers blocked in their actual checkpoint operations, then release.
Creation loser must observe exact winner revision; update loser exact next one.
Use isolated mnesis_pg6b DB on local17.10, serial suites; no PG6a local suite runs
concurrently. Real PG6a CI owns separate clusters. All failed/filtered/native runs
receive three reviews; E0277 missing implementation is zero runtime evidence.

PG6b initial five native law cases fail compilation:22 E0599 errors all identify
missing CheckpointStore methods/trait bound. Zero runtime cases. Three reviews:
(1) tests invoke existing public contract, not a replacement test model; (2)
current adapter demonstrably lacks this capability; (3) no syntax/feature typo or
native result credited. Native protocol/lifecycle/boundary/overlapping-writer
laws exist before implementation. No production checkpoint code written yet.

Work pauses on PG6b implementation to handle a PG6a finding on its OWN branch:
execute() does not prove snapshot UPSERT actually changed a row if BEFORE trigger
returns NULL. Native red case underway in /private/tmp/mnesis-postgres-state.
Preserve this worktree's feature/test/docs changes; do not mix parent fix here
before focused retention/review. Parent PR382 must await corrected evidence.

PG6b resumes after hardened parent source retained/pushed as signedc72f38ca in
PR382. Final parent amendment normal gate12/12 passes:11cached, same-input WASM
rebuilt; three reviews confirm required gate/source identity/no fresh test credit.
Parent new-head CI still required, including downloading `mutants` artifact.
Child work saved in named stash `PG6b initial laws before snapshot hardening
c72f38ca`, OIDb51e18ce53f0a3e9326676e014d3bdab26778392; stash KEPT. Fast-forwarded
child to c72f38ca, applied stash, resolved single append/append research conflict
by retaining BOTH complete records. Parent hardened source/test/workflow files
have zero diff against parent HEAD. Initial law source and tracked patch also
saved in /tmp/mnesis-pg6b-initial-laws.rs and before-parent-alignment.patch.
Before implementation, extend corruption lifecycle with actual reopen rejection
and require zero-row checkpoint writes to fail: absent no-op is NOT an actual
creation-race winner; it must not fabricate Conflict(None,None) or commit a key.
Use supported INSERT RETURNING optional for legitimate race, then require actual
locked winner or RowNotFound; UPDATE RETURNING requires a row. Initial test
budget500 still applies; no helper framework or repeated write policy.

Migration4 DDL introspection against actual migrated mnesis.keys on local17.10
runs entirely inside BEGIN/ROLLBACK; exact native constraint definitions recorded
in pg6b-native-shape.log. Three reviews: current migration SQL is exercised;
actual native definitions (including zero-valid position bounds) drive independent
read-only shape validation; rollback preserves parent schema/metadata/data and
this is NOT checkpoint runtime-behavior evidence. Metadata FromRow flatten is a
supported pinned SQLx0.9 feature. Initial product uses one native row lock and
sole next_revision policy; zero-row INSERT observes winner underlock, suppression
with no winner returns RowNotFound. SQL static INSERT/UPDATE share all field
bindings; existing-row zero update likewise cannot be acknowledged. No retry.
Checkpoint metadata/nullable selected body use different types rather than dummy
fields or mode flags. Migration4 validates even when projection is disabled.

First PG6b implementation attempt fails E0382 compilation before runtime: adding
the next catalog query reused a moved &mut PgConnection. Three reviews: shared
validation transaction contract retained; actual compiler detects missing
reborrow, not a checkpoint behavioral failure; zero runtime cases/no coverage
credit. Reborrow the snapshot query's connection explicitly, preserving the same
brief read-only repeatable-read transaction and cancellation cleanup.

First six native checkpoint laws PASS6/6,27.555s, zero skips. Three reviews:
(1) exact creation/advance/rebuild/stale/position/overflow/error/reopen contracts;
(2) actual two INSERT contenders with preseeded key and two advancing writers
have one complete winner and exact loser revision, corruption stays corrupt,
suppressed creation/update return storage failure with no orphan key; (3) all
cleanup precedes assertions, no unrelated error accepted as CAS conflict, no
runtime proof credited to the earlier compile failures. First cold fixture27.350s
remains unresolved PG8; other laws0.036–0.048s. This is targeted capability proof,
not whole PG6/delivery. Case review found the overlap wait depends on SQL casing.
Replace query text matching with native blocking PID relationships: creation
requires two DIRECT waiters on the trigger gate; advancement allows a row-lock
waiter behind the first writer. Preserve the actual creation race; a key-catalog
serialization must not masquerade as two creators. No deadline increase.

Strict initial Clippy rejects the private SQLx-to-checkpoint error constructor
because it can be const. No runtime test failure and no lint relaxation: make
that existing constructor const, then rerun strict all-target/all-feature checks.

PG6a final headc72f38ca CI39133566393 passes all40 checks/one scheduled skip.
Native163/25.347s integration job and163/20.601s Nix-check job are TWO actual runs;
installed25.347s replay is not third. Their exact atomic/snapshot smoke pairs
1/0.071s+1/0.057s and1/0.045s+1/0.040s are separate filtered proofs, not added to163.
Workspace1561/88.605s with four compiler exclusions; separate compiler functions
2/108.46s,1/3.88s,1/12.19s all pass at SAME head. Three distinct reviews of EACH
native/smoke/workspace/compiler/feature-matrix run: source/expected contracts;
actual original and new suppression regressions/diagnostics; skips, empty harness,
feature and installed-log limits. No failed/filtered/DB-free coverage credited.

CI mutation39133566425 artifact`mutants`,ID12139461392 downloaded and reviewed
locally, all outcomes/log/diff available. Baseline163/24.886s;23candidates21caught,
2E0277unviable,0missed/timeout. Three reviews for EVERY candidate run: changed
contract and exact first failure; actual native error/value/fault; residual-state,
source/timing/filter loopholes. No poison. CI key-lookup mutations differ in valid
race interleaving: one produces TWO concurrent winners[0,2], others RowNotFound
or actualFK23503, all reject the mapping/valid-write contract. Snapshot schema/
size flips reject exact pair/limits; no-op commit caught at native COMMIT-entry
guard, not described as independent persistence proof. New native zero-row red/
fix plus direct cache laws supply separate storage evidence. Unviable never
counted caught. Artifact upload uses existing stock action and succeeded.
PR merge statusCLEAN/MERGEABLE at reviewedc72, zero reviews/threads to resolve;
normal protected squash with matching head next, no admin/bypass or early release.

PG6a DELIVERED through protected squash PR382 on2026-10-18T14:14:32Z:
merge090504ae3b2081c03cfd8960f6edc5c0058fb3b2, reviewed headc72f38cab09df220f2c1f78aad96757c51b7ab0c.
All40 checks green/one scheduled skip; no unresolved review threads; matching
head, no admin/bypass. CI39133566393, mutation39133566425 and artifact12139461392
are final-head evidence. Publication/full PG6 and adapter acceptance stay OPEN.

Second strict Clippy pass finds similar local state/stale names and a test helper
needlessly owning expected rejection. Rename the mismatch result and borrow the
expected rejection, retaining ALL exact comparisons. No lint level changes or
runtime assertion weakening. Private error constructor now const as requested.

Strict third all-target/all-feature Clippy passes. Native blocking-PID race
selection1/1 passes27.773s, five filtered. Three reviews: both real absent-row
creators directly wait on the trigger gate; existing-row advancement has actual
lock overlap with exact winner/loser/full pair; no query-casing dependence,
deadline or assertion weakening, filtered cases not credited. Cold fixture tail
near27s repeats independently of full workspace tests and remains PG8 research.
Next align child safely onto merged090504ae; parent and reviewed c72 trees are
identical by git diff. Save named stash and apply index so all tracked/untracked
product/test/evidence and prior staging survive; no mixed reset of source files.

Child safely aligned onto merged090504ae using KEPT stashfbcc7d919c5df03ee3216065de84a63ddb825992,
empty-range rebase fromc72 and apply --index; source trees equal, all staged/
untracked work restored without conflict or negative parent changes.

PG6b bounded-state verification pass BEFORE edits: <=220 additional native lines,
zero production changes, reuse existing allocation fixture. Cover1MiB stale and
oversize reads without body allocation, exact byte edge/pre-BEGIN closed-pool
rejection, invalid rebuilds and same-ID aggregate/projection separation. Then
wire-pair/uncertainty and real-consumer verification receive their own explicit
budgets; no initial500-line test-budget concealment. These reject observable
state loss/resource behavior, not a reimplementation of next_revision.

Before verification/retention, bounded-state test budget refined220→260 lines:
rustfmt gives251 added including four full laws, shared allocation import and
a real bounded-store fixture. Missing-record CAS and both directions of namespace
independence are included rather than weakening coverage to meet an estimate.
Actual product remains unchanged203/1Rust,49SQL. Strict all-target/all-feature
Clippy passes. This is a transparent pre-verification budget revision, not a
claim the original220 estimate was met. Next run all ten native checkpoint laws.

All ten native checkpoint laws PASS10/10,3.017s, zero skips. Three reviews:
(1) missing/stale revisions, invalid rebuilds, byte limits and namespace independence
match owner contracts; (2)1MiB stale/oversize reads stay below262144 allocated
bytes, exact37-byte edge succeeds,38-byte input rejects even on closed pool,
snapshot and checkpoint updates leave each other intact; (3) actual allocation
counter warmed before sampling, no println-as-assert/filter/skip or backend-error
alias accepted. All ten include the original six with strengthened blocking-PID
probe. Strict Clippy passes; no full-suite or release credit yet.

PG6b transport verification budget BEFORE editing: <=230 new native test lines,
<=35 changed existing proxy/fixture lines, zero production changes. Reuse held
state DataRow and actual COMMIT marker: capture old complete checkpoint while
independent real update commits; after release require old record, then new.
Native ACK loss through existing codec retains exact CommitUnknown/UnexpectedEof,
actual caller timeout after COMMIT preserves full revision/schema/position/body.
Rename private shared snapshot probe to state and enable it for either feature;
no new transport runtime/framework. Reset events AND both state tables before
outcome cases so isolated repeats cannot inherit prior checkpoint revisions.

Projection-only native transport3/3 PASS0.156s,12filtered; strict all-target/
all-feature Clippy passes. Three reviews: (1) full old revision/position/state
captured while a real independent replacement commits; native ACK loss propagates
exact typed CommitUnknown/UnexpectedEof through codec; caller timeout follows
actual COMMIT and preserves all fields; (2) independent native queries verify
revision1/full unsigned positions/schema/state and zero event writes, no retry;
(3) actual fault-entry markers observed, future truly dropped after20ms held
reply, subset/filtered cases not credited, new fixture resets both state tables
for repeat isolation. Existing snapshot probe behavior still needs full regression
coverage with renamed shared marker. No whole-PG6/delivery claim.

PG6b real-consumer verification BEFORE edits: <=300 native consumer lines plus
<=80 shared test fixture lines and <=100 existing outcome-case lines; zero product
changes/dependencies. Use existing Counter/CounterEvent/CounterState/CounterError,
a small checked Projector, real repository-produced events, RawEventStore read+
DecodedStreamExt, CodecSnapshotStore and actual Projection load/advance/rebuild.
Verify exclusive resume, schema mismatch/rebuild preserving revisions, a real
stale writer requiring reload, and actual caller cancellation after COMMIT making
the stepper unusable until reload. Shared pure projector/event fixture is used by
consumer and wire case; it is not a fake store, framework or runtime runner.

Real-consumer strict compile check caught one unused test import, zero runtime;
removed the unused PostgresError import without lint changes. Consumer source
214lines/shared fixture62 within300/80 budget. Native cancellation case is added
through the same real projector/repository/decoder fixture: after actual COMMIT,
drop stepper future, require ReloadRequired on access/advance, then independently
reload exact committed state/revision/position. No resubmission after timeout.

Pre-run cancellation case review found an old COMMIT watch value from seeding
history through the proxy. Mere equality would not prove THIS checkpoint committed.
Mark the old version seen BEFORE arming, then require a NEW watch change and exact
Committed phase before the20ms timeout. Preserve3s fault-entry bound. Other new
raw timeout/ACK-loss cases arm before first proxy commit; pair case waits for
State, not the earlierCommitted value. No stale marker accepted as actual fault.

Cancellation freshness compile check failed E0615 because the existing receiver
field is `seen`, while `observed` is a method. Zero runtime; use the actual
receiver, preserving fresh-version semantics and exact phase assertion. No
suppression, annotation workaround or claimed passing consumer case.

Real projection consumers3/3 PASS18.079s, zero skips. Three reviews: (1) real
repository-produced events decoded from native read_all drive existing stepper/
codec; exact persisted state/global position resumes exclusively, explicit schema
rebuild preserves revision1→4; (2) actual stale writer returns exact conflict and
is unusable until reload, resumed state cannot double-apply old event; (3) fixed
two-event collection supports replay/duplicate consumers rather than unbounded
product loading, exact remaining tuple and revision counts prevent Set(4) from
masking duplicate replay. First fixture18.001s remains PG8 observation. This is
real adapter integration, no fake store/new kernel API/runtime.

PG6b least-privilege and migration verification BEFORE edits: <=110 native lines
each, zero product changes. Real LOGIN checkpoint writer gets metadata/key lookup,
key INSERT/sequence USAGE and checkpoint SELECT/INSERT/UPDATE, no key mutation,
event rewrite/delete, checkpoint delete or DDL. Exact42501 denials and true
current_user/session_user identity; cleanup before assertions. Frozen migration3
fixture contains actual event plus full-range snapshot: foreign checkpoints table
must fail migration4 transaction with42P07, preserve exact metadata/data/position,
then explicit rename and forward repair retain next sequence/exclusive resume.
Do not rewrite applied migration SQL/checksums or discard a foreign object.

Cancellation consumer native1/1 PASS0.094s,15filtered. Three reviews after this
run: (1) actual Projection/codec consumer becomes ReloadRequired after canceled
persistence and cannot advance until reload; (2) a fresh watch change proves THIS
checkpoint COMMIT preceded timeout, independent reload yields revision1/state1/
first position and native history remains exactly two events; (3) old seeding
COMMIT is marked seen before arming, held reply future actually drops, exact pair
and blocked-access assertions remain,15filtered cases receive no runtime credit.
No retry is issued after uncertainty. Log /tmp/mnesis-pg6b-cancelled-consumer-native.log.

Restricted-role Clippy check rejected86/80 function lines; zero runtime. Consolidate
only the repeated five identical expected permission codes into vec![Some(...);5].
All five independent denied operations and exact42501 assertions remain. No lint
allow, deadline, privilege or production change.

Restricted-role native1/1 PASS3.581s,9filtered, projection-only. Three reviews:
(1) real LOGIN/current_user/session_user proves the supported runtime grants can
create and conditionally advance checkpoints without key UPDATE or DDL;
(2) exact persisted revision2/position(2,0)/state[2] follows real INSERT then UPDATE,
while five native privilege probes fail exact42501; (3) expected codes have five
entries matching the five attempted operations, privileged setup is separate,
runtime pool closes and role cleanup precedes all assertions. Filtered cases are
not credited. Strict all-feature Clippy passes, no lint/privilege relaxation.

Migration compile check rejects107/80 function lines, zero runtime. Split the
history/next-sequence/exclusive-resume assertions into a private test helper;
keep the entire collision/metadata/cache/foreign-object sequence together.
Before runtime verification/retention, migration test budget110→150 formatted
lines: the original estimate omitted three exact checksum snapshots and full
unsigned/binary cache preservation. No production budget change; all acceptance
and failure assertions stay. No applied migration or lint change.

PG6b process-termination verification BEFORE edits: <=90 native lines, zero
production changes. Reuse existing support/process.rs kill-after-real-ACK fixture;
child commits full-range revision/schema/position/state through actual adapter,
retains handles, parent confirms native SIGKILL and independently reopens exact
record. Client crash evidence only, no server/power-loss durability claim.

Migration3→4 native1/1 PASS0.057s,4filtered (default features). Three reviews:
(1) actual frozen migration3 with event and unsigned/binary snapshot hits native
42P07 at migration4, retains exact existing checksums/success flags, then explicit
rename repairs forward; (2) exact full-range cache and foreign row remain, event
position unchanged, next sequence is old+1 and exclusive resume returns only new
version2; (3) repair occurs before assertions, no applied SQL/checksum edits or
foreign-object deletion, default-feature execution proves validation does not
require capability feature unification. Only this one case receives runtime credit.
Stronger follow-up captures cache immediately after failure as well, before repair,
to directly establish rollback preservation independently of forward repair.

Added direct post-failure cache capture: strict compile81/80 lines, zero runtime;
name the existing versions vector before the unchanged exact[1,2,3] assertion.
No helper framework, lint relaxation or reduced checks. Crash fixture writes
revision1 with maximum schema/position, rather than claiming a maximum revision.

First complete PG6b native all-feature regression:183/183 PASS20.158s,zero skips,
14 binaries. Three reviews after this run:
1. Contract: native CAS, full unsigned state/revision/positions, explicit rebuild,
real projection resume/reload, bounds and namespace separation all run alongside
existing append/atomic/import/snapshot/migration/wake contracts.
2. Defect/fault: actual overlap gate admits one complete winner; real ACK loss and
post-COMMIT timeout preserve uncertainty and saved pair; new child acknowledges
revision1/full-range schema+position then native SIGKILL, independent reopen exact;
migration collision retains cache both immediately after failure and after repair,
checksums unchanged, exact next sequence/exclusive resume remain.
3. Oracle/provenance: actual DATABASE_URL PG17.10, serial shared cluster,0skips,
source compiled from checkpoint worktree; both renamed snapshot wire regressions
and new checkpoint holds pass, all four real consumer cases execute, role cleanup
and corruption repair precede assertions. No installed-log replay, filtered cases,
cache or compile failures credited. Existing snapshot process test also runs.
Log: evidence/pg6b-full-native.log. This is not Linux/full-flake/CI/merge/release.

PG6b shape falsifier BEFORE edits: <=90 native test lines, <=4 SQL validation
lines,0 Rust production. Threat: native inheritance reads descendants but parent
PK/FK do not enforce descendant uniqueness, so a validator checking only relkind,
columns and parent constraints could accept multiple checkpoint rows for one ID.
Create a real inherited child containing a duplicate key with distinct saved
state; require startup Schema rejection, cleanup child before assertions. First
run must reject current source if shape guard is missing. Then reject either
parent/child inheritance through existing pg_catalog validation, no new runtime,
API, migration/checksum rewrite or per-query workaround. Other catalog owners
remain subject to final whole-repository audit rather than a silent readiness claim.

Inheritance ORIGINAL native0/1 FAIL0.052s,14filtered at exact startup-rejection
assertion. Three reviews: (1) supported checkpoint profile requires unique rawID
record and coherent CAS; (2) native query actually returns two distinct saved
states under ONE key from parent+child, but current startup accepts the schema;
(3) exact[parent,child] assertion executes first, child and inserted parent row
are removed BEFORE failure assertion, no timeout/fixture/compile failure or
missing feature passed as the intended red. Default-feature SUT freshly compiled
from this worktree. Actual original unsafe-schema acceptance reproduced.

Inheritance FIXED native1/1 PASS0.059s,14filtered. Three reviews: (1) startup now
rejects unsupported inheritance using existing read-only coherent schema view;
(2) same exact duplicate native[parent,child] rows that original source accepted
are rejected as Schema after two-line pg_inherits check; (3) default-feature
fresh compile path recorded, cleanup still BEFORE assertions, no ONLY query that
could silently hide foreign rows or metadata repair. Only this case credited;
full183 run predates this production fix and must be repeated at final source.

PG6b remaining metadata boundary verification BEFORE edits: <=180 native lines,
zero product changes. Extend existing corruption law to revision0/>MAX,
schema0/negative/>MAX, txid/sequence negative/>MAX. Native DROP guard+UPDATE,
actual matching/stale reads and conditional write must preserve CorruptRow,
not Stale/Conflict or a raw SQLx failure. Capture exact full corrupted row before
and after attempted write; restore value and original CHECK before assertions.
Keep tests against actual adapter metadata validation, not replicated parser or
next_revision. The original revision0 case remains among the same canonical laws.

Metadata-helper compile check E0521: SQLx0.9 accepts static literal SQL without
unsafe assertion, and Id helper accidentally added an extra reference. Zero
runtime. State literal arguments explicitly &'static str and pass existing
borrowed StreamKey directly. No dynamic SQL safety escape, ownership/API change
or lint relaxation.

PG6b CI/capability wiring BEFORE edits: <=7 Nix lines in existing integration
check, one --no-tests=fail projection consumer selector before full archive run;
<=35 README lines documenting exact grants/CAS/reload/migration4 behavior and
<=8 changelog lines. No new job, framework, package or runtime. Verify selector
with projection absent fails, so --all-features cannot silently omit capability.

Expanded metadata+both inheritance directions native2/2 PASS0.149s,24filtered.
Three reviews: (1) all nine corrupt scalar witnesses return CorruptRow on matching
AND stale reads and store-wrapped CorruptRow on conditional writes, missing guards
reject startup; (2) exact full native corrupted row remains unchanged and final
repaired checkpoint is revision1/position(1,1)/state[7], inherited descendants and
an inherited parent both reject Schema; (3) each mutation repairs original value/
CHECK BEFORE assertions, both inheritance edges detached/dropped before assertions,
zero skips among selected cases, no raw SQLx/Conflict/Stale alias accepted. Initial
E0521 compilation gets no runtime credit. Strict Clippy current source passes.

Final-source default native118/118 PASS14.182s,0skips. Three reviews: (1) default
feature intentionally has no checkpoint/snapshot/import API but validates all four
migrations and both checkpoint inheritance edges; (2) original read/wake/commit
fault laws and3→4 collision/cache/checksum/resume preservation actually run;
(3) DATABASE_URL realPG17.10/serial, capability-only binaries correctly contain
zero cases rather than falsepasses, no selective filter, all118 named cases
reviewed; source compiled current worktree. This is default-only evidence.

Final-source import-only native144/144 PASS18.901s,0skips. Three reviews:
(1) all default118 plus26 import/atomic cases execute without snapshot/projection
feature unification; (2) native atomic overlap, rejected final write, whole/per-stream
progress and ACK-loss/timeout/real-deadlock preservation run alongside schema4
upgrade and original read/wake regressions; (3) all144 named cases in real serial
PG17.10, no missing selected feature, repeated log credit or ignored failure.
Filtered capability binaries are absent, not claimed as exercised.

Final-source snapshot-only native139/139 PASS15.168s,0skips. Three reviews:
(1) unconditional snapshot semantics and all five shared conformance laws remain
present without import/projection features; (2) renamed held-old-state read still
forces independent replacement, native suppression/ACK-loss/caller-timeout and
SIGKILL recovery execute,3→4 migration preserves caches; (3) all139 named real
serial PG17.10 cases reviewed, no projection API credited from feature unification,
no threshold/skip/oracle changes. Snapshot-only evidence, not final all-features.

Fresh isolated lock-omission control native0/1 FAIL0.042s,10filtered. Three reviews:
(1) removing only FOR UPDATE violates one-winner revision CAS; (2) actual overlap
returns BOTH writes Ok(2), expected loser is exact Conflict{Some(1),Some(2)},
failure is semantic, not gate/deadline/setup; (3) isolated proof target adapter
artifacts explicitly cleaned837.5MiB and adapter recompiled from control manifest
/tmp/mnesis-pg6b-lock-control.m6lSZn before running, compile path retained in log.
No stale shared target credit; working product untouched, no exclusions/relaxation.

Evidence whitespace housekeeping: strip trailing spaces only in committed copies
of native catalog/failure extracts, retaining original /tmp logs unchanged. No
case names, errors, values, timings or verdicts changed. git diff --check had
correctly identified terminal-output spaces, not production/test defects.

Final-source projection-only native136/136 PASS15.449s,0skips. Three reviews:
(1) all118 default plus18 projection cases execute without snapshot/import;
(2) actual two-writer overlap, nine corrupted metadata witnesses, suppressed writes,
1MiB allocation/byte edges, real stepper rebuild/stale/canceled reload, full pair
hold/ACK loss, LOGIN and SIGKILL laws execute; (3) all136 named cases reviewed,
namespace law intentionally requires snapshot and is not credited here, literal
SQL source/manifest/current feature prove isolated compilation. Zero skipped
selected tests; no deadline/threshold/expectation reduction.

Fresh isolated unconditional-body control native0/1 FAIL0.055s,10filtered. Three
reviews: (1) bounded stale reads must omit state body before transfer/allocation;
(2) changing only CASE ELSE to return c.state makes actual1MiB stale hydration
allocate6,306,290 bytes, failing original262,144-byte ceiling; (3) isolated proof
target adapter cleaned106.7MiB then explicitly recompiled from body-control
manifest, no stale/shared artifact, SQL source and compile log retained. Failure
is allocation assertion, not setup/timeout/decoding, threshold unchanged, working
source untouched. Oversized branch is unrun after first assertion and NOT credited.

Final-source import+snapshot native165/165 PASS20.139s,0skips. Three reviews:
(1) default118/import26/snapshot21 compose without projection unification;
(2) shared atomic/snapshot laws and independent wire COMMIT/old-pair/rollback/
SIGKILL/migration preservation actually run together; (3) all165 named native
PG17.10 cases reviewed, zero omitted selected cases, projection absent and not
credited, DB-free paths/cached compile not counted as fresh tests.

Fresh isolated split-read control native0/1 FAIL0.045s,15filtered. Three reviews:
(1) hydration must return one coherent revision/position/state record; (2) adding
only an independent body SELECT yields actual revision1/position(1,9)/NEW state,
expected same old revision/position with OLD state, while independent writer
commits revision2; (3) failure is exact record assertion after real held DataRow,
not fault-entry/timeout, proof target adapter cleaned106.7MiB and recompiled from
explicit split-control manifest, no shared stale artifact credit. Working product
unchanged; next positive full source run remains required.

Final-source import+projection native162/162 PASS21.371s,0skips. Three reviews:
(1) default118/import26/projection18 compose without snapshot; (2) checkpoint
CAS and independent projection persistence run beside atomic/import outcome
faults and exact migration/cache preservation; (3) all162 named real serial
PG17.10 cases reviewed, no snapshot/namespace credit, no filtered selected case,
all source compiled current worktree. Separate physical PG18.6 mutation cluster
started for16 generated changed-source candidates; baseline/outcomes pending.

PG18.6 mutation BASELINE native184/184 PASS21.461s,0skips,14binaries (8.52s build).
Three reviews: (1) complete selected adapter suite with projection API present;
(2) strengthened metadata/inheritance and all actual overlap/transport/consumer/
SIGKILL/upgrade laws execute on fresh independent physical cluster; (3) actual
native database, no default-return skips, cloned source compiled by cargo-mutants
with maintained default timeout109s derived from measured baseline. Baseline is
not a mutant caught;16 generated candidates remain to review independently.

ARM Linux18.6 existing integration derivation PASS, native184/18427.815s,0skips.
Three reviews: (1) all four capability/contract families compiled and selected;
(2) native full suite includes new inheritance/metadata witnesses, real consumers,
uncertain commits, childSIGKILL and migration4 repair alongside original laws;
(3) actual remote builder executed fresh archive,14binaries,zero selected skips,
installed log is this SAME run, never an extra execution. Full flake/CI still due.
Separate Linux smoke1 atomic1/1 PASS0.199s: reviews(1) import feature selector;
(2) actual multi-stream all-or-none native conformance; (3) --no-tests=fail and
pipefail remain,14 filtered cases/13binaries do not receive runtime credit.
Separate Linux smoke2 snapshot1/1 PASS0.403s: reviews(1) snapshot selector;
(2) absent→commit→exact found real store law; (3) --no-tests=fail/pipefail,15filtered/
13binaries not extra tests. Separate smoke3 projection1/1 PASS0.278s: reviews
(1) projection selector; (2) real repository/decoder/codec stepper saves/resumes
exact pair exclusively; (3) --no-tests=fail/pipefail,2filtered/13binaries not
credited, no DB-free skip or fake consumer. Missing-feature negative still due.

Final-source snapshot+projection native158/158 PASS16.442s,0skips. Three reviews:
(1) default118/snapshot21/projection18 plus one bidirectional namespace law;
(2) both state stores with identical rawID remain independent in BOTH directions,
actual old-pair/commit uncertainty and all consumer/rebuild/reload laws execute;
(3) all158 named cases reviewed on real serial PG17.10, import absent/not credited,
zero selected skips, capability combination is actual compilation rather than
workspace feature unification. Last remaining local combination is all-features.

Generated mutation cases reviewed independently as results arrived. Each row
records three reviews: threatened contract; actual first-failure falsifier; oracle/
provenance limit. Full native baseline is184; fail-fast unrun cases are NOT passes.

| Candidate | Review1 contract | Review2 observed failure | Review3 oracle/provenance |
| --- | --- | --- | --- |
| schema47 returnOk | Startup rejects missing guards | Corrupt metadata test reopens unsupported shape and does not get Schema | Repair occurs BEFORE failed assertion; actual native case32,152unrun, no poisoned checksum fixture. |
| schema89 delete! | Valid schema opens | Native out-of-order setup receives Schema for valid checkpoints | First native open refusal,12/184 run,172unrun; valid-input rejection catch, not corruption acceptance proof. |
| metadata26 default | Preserve decoded metadata | E0277 PgAllPos lacks Default | Unviable build only,0native, never caught credit. |
| row58 default | Hydration result preserves state | E0277 CheckpointHydrated lacks Default | Unviable build only,0native, never caught credit. |
| row59 !=→== | Matching schema returns Found | Native SIGKILL/reopen returns Stale for matchingMAX schema | Exact full-pair assertion after actual acknowledged commit/processkill,29run155unrun. |
| row65 >→== | Exact byte limit accepted | Actual37/limit37 returns RowLimit | Existing exact edge unwrap rejects false error,31run153unrun; no bound relaxation. |
| row65 >→< | Below-limit row accepted | Actual78/limit16777216 returns RowLimit after SIGKILL | Exact durable Found expected,29run155unrun; process cleanup complete before failure. |
| row65 >→>= | Exact byte limit accepted | Actual37/limit37 returns RowLimit | Same exact-edge oracle,31run153unrun; independent source mutation, no invented extra workload. |
| lock90 None | Existing revision observed | Valid namespace checkpoint advance gets Conflict{Some1,None} | Actual update fails expected success,30run154unrun; separate native FOR UPDATE control proves true two-winner race. |
| lock90 SomeDefault | Saved position valid | E0277 PgAllPos lacks Default | Unviable build only,0native. |
| storage103 new() | Typed store source retained | E0599 missing constructor | Unviable build only,0native, no error-path coverage claim. |
| storage103 from_iter | Typed store source retained | E0599 missing constructor | Unviable build only,0native. |
| storage103 new(default) | Typed store source retained | E0599 missing constructor | Unviable build only,0native. |
| storage103 from(default) | Typed store source retained | E0283 ambiguous From | Unviable build only,0native. |
| hydrate114 default | Complete hydration result | E0277 CheckpointHydrated lacks Default | Unviable build only,0native. |
| commit136 returnOk1 | Acknowledged checkpoint must persist | Actual child receives success/ACK and dies by SIGKILL, reopen returns Absent | Exact full record assertion,29run155unrun; independent data proof, not merely COMMIT-entry guard. |

Final mutation summary16:8caught8unviable0missed0timeout. All16 first outcomes
reviewed three ways above; eight type errors are NOT successful runtime evidence.
No exclusions, cap/timeout changes, replacement or weakened tests. SQL-specific
FOR UPDATE/body/split invariants independently falsified by fresh controls.
Final artifact copies use .log suffix for JSON evidence to preserve existing Nix
source filtering; originals retained /tmp/mnesis-pg6b-mutants/mutants.out.

Final-source all-features native184/184 PASS20.811s,0skips,14binaries. Three reviews:
(1) every default/import/snapshot/projection contract and namespace law runs;
(2) latest both-direction inheritance, nine metadata faults, true CAS overlap,
wire-pair/uncertainty, real stepper consumers, LOGIN/SIGKILL and upgrade4 cases
execute with unchanged original read/notification/atomic laws; (3) actual serial
PG17.10, current source explicitly compiled, all expected184 present, no filtered
case/compiled-only/cache/log-replay credit. All EIGHT feature combinations now
have fresh native evidence:118,144,139,136,165,162,158,184; all0skips.

Missing-projection negative exits4:0tests/no tests to run under SAME required
selector with feature absent. Three reviews: (1) integration must fail if projection
suite absent; (2) default-feature binary actually selects0 and --no-tests=fail
returns nonzero, paired with actual Linux selected1/1; (3) no compile/DB/setup
error mistaken for absence,0runtime successes and no native evidence credited.
No feature or selector loosened. Log evidence/pg6b-missing-projection.log.

PG6b pre-commit minimization: product203added/1removedRust,51SQL,one Cargo feature,
five existing-gate Nix lines; within260/30Rust100SQL budgets. Zero new public
methods/types, dependency/lock changes, runtime tasks, framework or tools.
Existing next_revision is sole revision/schema/position policy; native row lock
only supplies serialized current facts. Existing key helper and commit classifier
own identity/uncertainty. SQLx flatten removes duplicate metadata decoder; static
INSERT/UPDATE share bindings. Two-line inheritance guard retained only after real
original red. All applied prior migrations untouched. Native test budgets preserve
readable laws and explicit negative evidence, no test-only production seam.
Full normal signed commit hook/CI/protected delivery still pending. Changed Rust
diff SHA4061f1d5647320d21429f71eb52d20d6dabcd100bc0e60e5d2866c728a013f50.

Full-gate fuzz replay reviews (logs /tmp/mnesis-pg6b-gate-fuzz.log), each AFTER
execution, no Postgres-native credit:
- CBOR header/chunk2/2 PASS1.00s: review1 completion/parse boundaries; review2
actual existing decoder fuzz targets/corpus, no replacement decoder; review3
0filtered/ignored, short retained replay is not an extended fuzz campaign or
format interchange evidence. Existing assertions/targets unchanged.
- Event type1/1 PASS1.00s: review1 value boundary contract; review2 actual owned
value fuzz target with existing corpus; review3 no skips/filters, no database or
full payload correctness inferred from this target.
- Wire frame1/1 PASS1.00s: review1 wire decoder boundaries; review2 actual canonical
frame decoder/fuzz fixture; review3 no skips/filters, no new tests/threshold or
claim that short replay establishes every adapter behavior.

Normal signed commit full hook passes at d1b9c387574acfa7f3136c4aab9283c521a4da13,
G signature verification, no bypass. All12 named Darwin checks green (audit cached,
11 fresh). Workspace nextest1563/1563 PASS464.283s,13slow,4explicit compiler-function
exclusions. Three reviews: (1) whole workspace gate includes adapter compilation,
core/store/Fjall/inmemory/wake contracts, unchanged compiler owners have dedicated
CI jobs still required; (2) new source/schema/capabilities compiled with strict
all-feature/all-target Clippy and actual independent native184/Linux184 evidence
above, original source controls remain failing; (3) workspace run has NO native
DATABASE_URL and returns from DB-dependent tests, so1563 is NOT new database
coverage;4excluded functions are not successes here. Existing property tests
completed unchanged (last random-version case388.944s), no workload/deadline edits.
Fuzz replay reviews recorded separately above. Fresh complete hook output retained
/tmp/mnesis-pg6b-commit.log and native artifacts already committed; docs-only
amendment reruns normal hook and any cached checks receive no extra runtime credit.

PG6b docs-only amendment7599588e3a0f1bbe318209accb3b831efac58f04 is signed G,
normal hook passes all12 named checks from cache,0fresh runtime credit. Pushed
work/postgres-checkpoints and openedPR383 https://github.com/devrandom-labs/mnesis/pull/383.
CI39141316095, mutations39141316085, pushCI39141295836 in progress; reviewed
artifacts/allchecks/threads/protectedmerge still required. Release378 remainsopen.

PG7a begins in separate work/postgres-export from reviewed7599588e while parent
CI runs. Exact pre-edit budget/falsifiers in HYPOTHESES; no production edits yet.
Bounded event-only listing precedes consistent sessions and restore. Existing
export/RR/pool facilities and native PG7 test workloads inspected; preparation
notes/tmp/mnesis-pg7-pre-edit.md retain unresolved lifetime/foreign-history issues.
No public history API, hidden SQLx constructor or detached pool connection chosen.

PG7a first characterization: selected export native target does not compile,
6E0599 missing list_streams methods,0runtime. Three reviews: (1) required existing
StreamLister capability absent; (2) this proves API absence only, not a failing
boundedness/visibility algorithm; (3) correct trait/feature selected, no unrelated
errors, no test silently skipped by cfg. Native source/path and full compiler log
/tmp/mnesis-pg7a-original.log retained. Five laws254lines within450; production0
before implementation. Next bounded-keyset candidate within180Rust budget.

PG7a initial implementation strict Clippy rejects shadow_unrelated on cursor
cutoff binding,0runtime. Rename carried value ceiling consistently with existing
read_all owner pattern. No lint allow/API/SQL behavior or test expectation change.

PG7a next strict compile catches shadow_unrelated on second test lease name,
0runtime. Give idle and buffered acquisition checks distinct local names; both
3s acquisition assertions remain, no lint or workload change.

PR383 CI mutation39141316085 completed; artifact downloaded to
/tmp/mnesis-pg6b-ci-mutants-39141316085, baseline184/18429.787s,0skips. Three
baseline reviews: complete native selected suite; real new state/overlap/fault
laws on fresh physical cluster; actual workflow revision7599588e, no cached/
installed-log replay credit. All16 artifact first failures inspected independently.
For EACH case, three reviews against the canonical mutation table above:
(1) exact source candidate and owner contract match the table; (2) native first
failure matches the recorded oracle, including true Absent after acknowledged
checkpoint no-op and37/37 exact-edge rejection; (3) phase results distinguish eight
runtime catches from eight E0277/E0599/E0283 unviable builds, native fail-fast
unrun cases not passes, fixture repairs BEFORE assertions, no checksum poison or
hidden skips. All16 observed first-failure mappings match the table;8caught8
unviable0survivors/timeouts. These are CI executions, distinct from local sweep.

PG7a first native5/5 PASS0.297s,0skips. Three reviews: (1) empty/state-only IDs
absent, binary IDs once across one-row pages/reopen, exact37byte edge, terminal
oversized ID, finite visibility and released leases match declared listing contract;
(2) actual native staged event writer with tokio::spawn+Barrier overlaps capture,
old cache key gaining committed event and new key after capture stay out;20,000
native keys/events first-item allocations and1MiB ID body omission meet262,144
bound, sole data connection remains acquirable while cursor idle/buffered;
(3) real PG17.10/serial dedicatedmnesis_pg7a, listener PID identified by per-process
application name BEFORE lease assertion, exact row counts/membership/results,
no cfg skip/unrelated compiler success or backend-error alias credited. Strict
Clippy passes with names fixed, no lint/bound/deadline changes. Strong independent
unbounded-list/body/cutoff controls remain required before retention.

Additional PG9 inventory finding from existing fuzz.yml: embedded python3/tomllib
AFL-version selector also conflicts with no-project-Python, beyond the already
recorded scripts/check-features.py. Remove through maintained Cargo metadata+
existing ecosystem tools under established Nix ownership; do not retain snippet
or translate into new custom tooling. Audit existing AFL tee/crash gate evidence
before claiming crashes reliably fail required CI; current source uploads crash
artifacts even on exit0 and that alone is not a failing gate. No speculative
production change now; verify actual behavior/official shell contract first.

PR383 compiler owner job117716163410 completed: kernel2/2 PASS71.72s, macro1/1
PASS2.63s, store1/1 PASS7.88s. Three reviews for EACH run: (1) original diagnostic/
misuse contract owner and full feature/locked arguments selected; (2) actual
trybuild fixtures and version-zero/macro/store compile-fail functions pass, all
four functions excluded from Nix workspace accounted for; (3)0skipped/filtered,
normal checkout paths, unchanged .stderr snapshots/fixtures, no native-database
or extra function credit. Raw job log/tmp/mnesis-pg6b-ci-compiler.log.

PR383 pushCI39141295836 native job117716106025:184/184 PASS22.242s,0skips.
Three reviews: (1) complete compiled184/14binaries and all capabilities; (2) native
state/wake/atomic/uncertainty/consumer/upgrade laws execute; (3) actual independent
physical cluster with required psql check, second identical22.242s installed
run.log display is SAME execution, not another native run. Three separate feature
smokes atomic1/0.051s,snapshot1/0.041s,projection1/0.049s; EACH reviewed against
its feature contract, real native conformance/consumer assertions, and strict
--no-tests=fail/pipefail/filtered counts (14/15/2) not extra native coverage.
PR CI's own separate native/full-flake runs remain in progress.

PG7a fresh unbounded-list control native0/1 FAIL0.243s,4filtered: first item
allocates3,415,412>262,144 bytes for20,000 actualkeys/events. Three reviews:
(1) listing memory must depend on page, not catalogue; (2) replacing only row LIMIT
with bound-parameter OFFSET0 fetches whole candidate catalogue and exact allocation
assertion fails; (3) isolated proof adapter artifacts explicitly cleaned4.0GiB,
source recompiled from/tmp/mnesis-pg7a-unbounded-control.xBPZbI, failure is real
allocation, not SQL binding/setup/gate/timeout. Positive source untouched; raw log
retained. Final source must retain equivalent falsification after any later fix.

PG7a additional visibility falsifier BEFORE editing: <=70 native lines (total
still<=450),0productyet. StreamLister promises every stream in selected committed
view; unlike ordered read_all, it need not withhold already committed higher-xid
streams behind an unrelated held transaction. tokio::spawn+Barrier holds assigned
native lowxid while actual adapter appends higherxid; listing must include that
committed stream. Capture output and release/rollback held transaction BEFORE
assertions. Reject safe-xmin candidate if it omits a committed-at-open stream;
never weaken trait/PRD by silently declaring this omission acceptable.

PG7a visibility ORIGINAL native0/1 FAIL0.041s,5filtered. Three reviews:
(1) listing must include committed streams at capture even when global ordered
reader intentionally withholds them; (2) real spawned/Barrier-held lowerxid overlaps
higher-xid adapter commit, actual listing[] instead of exact one committed ID;
(3) native IDs prove higher>held, writer rollback/join BEFORE assertion, failure
is membership omission rather than gate/timeout/setup. Fresh source compile.
Reject xmin-only listing candidate; do NOT weaken StreamLister/PRD to accept it.

Compare supported native full pg_snapshot visibility before any custom filter:
pg_current_snapshot plus pg_visible_in_snapshot preserve frozen committed set
without pinning read transaction or discarding committed higher-xid streams.
Stored event xid8 comes from pg_current_xact_id (top-level); subtransaction-ID
caveat must stay explicit. Snapshot representation costs active-xid list, bounded
by database active transaction configuration rather than event backlog; PG8
capacity must include that cost. Within existing180Rust budget, replace only
snapshot capture/filter, no Rust parser, new type, dependency, runtime or API.

PG7a full-snapshot FIXED native6/6 PASS0.362s,0skips. Three reviews: (1) listing
includes committed-at-capture higher-xid stream while excluding active/future
streams, retains binary/once/reopen/bounds/lease laws; (2) exact original empty→one
membership defect fixed through maintained pg_snapshot visibility, real native
Barrier-held transactions and first-item/body allocation laws still pass; (3)
cleanup before original assertion, no parser/framework,0selected skips, unchanged
allocation262144/deadline3s, no global read_all semantics changed. All six actual
native cases from current export worktree; this supersedes five-case candidate.

PR383 PR-CI native job117716163539 passes184/18427.936s,0skips, independently of
push22.242s and local/Linux runs. Three reviews: complete14binary/capability suite;
actual native regression/uncertainty/consumer/migration laws; installed27.936s log
is SAME execution, no extra native credit. Separate native feature smokes1each:
atomic0.078s,snapshot0.068s,projection0.076s. EACH reviewed for matching trait,
actual conformance/consumer oracle, strict --no-tests=fail/pipefail and14/15/2
filtered not credited. One full-flake CI check still pending, reviewthreads0.

PG7a search-path falsifiers BEFORE edits: <=100 native lines (total<=450),
<=2 production SQL name qualifications. Caller-supplied pool search_path can put
foreign schema before explicit pg_catalog. Two fresh native pools separately
shadow visibility and snapshot functions; both must still list exact committed
ID through canonical builtins. Existing profile fixture prepares valid storage,
close/drop foreign schema before assertions. Run original first, retain only real
membership red; don't infer unsupported SQLx setup failure as correct falsification.

PG7a search-path ORIGINAL native0/2 FAIL0.114s,6filtered. Three reviews for EACH
native case: (1) caller pool search_path must not redirect canonical visibility
or capture; (2) separate native false-visibility function and fake1:1 snapshot
EACH yield actual[] instead of exact committed ID; (3) both selected execute via
--no-fail-fast, fresh pools prevent prepared-plan reuse, each schema/reader pool
cleaned BEFORE assertion, failures are exact membership, not constructor/setup/
SQL error or missing feature. No fixture poison, deadline or assertion changes.
Fix uses2 changed SQL lines qualifying2functions plus snapshot type (3canonical
identifiers), within180productionRust budget, no lookup wrapper/runtime/tool.

PR383 final CI39141316095 full-flake review:1563/1563 workspace97.334s,4 compiler
functions excluded (all pass their owner job above); fresh Nix native184/23.503s,
plus separate feature smokes1/0.057s,1/0.050s,1/0.059s. Three reviews for workspace:
whole selected contract owners; actual unchanged property/adversarial cases;
4exclusions separately accounted and DB-free adapter returns NOT native credit.
Three reviews for Nix native run: all184/14binaries/capabilities; actual new
state/concurrency/uncertainty/consumer/migration regression cases;0skips and distinct
execution from27.936s integration job, whose installed replay is counted ONCE.
Each native smoke reviewed for its matching contract, real conformance/consumer
oracle, and --no-tests=fail/pipefail with filtered cases not credited. Three fuzz
replay runs2/1/1 at1.00s reviewed individually for decoder/value contract, actual
existing corpus targets, and finite replay/0skips/no native or long-fuzz claim.

All24 isolated unit harness runs (8packages×default/no-default/all) reviewed THREE
ways EACH: (1) actual requested library/features and owning unit contract;
(2) real existing unit assertions with unchanged expected outputs; (3) no skips,
zero-case harnesses are compile-only, and dev-dependency unification means these
are not standalone no_std/dependency/adapter-native proofs. Separate library-only
checks cover every advertised flag through existing workflow. Detailed command/
count records in/tmp/mnesis-pg6b-ci.log; Python selector remains explicit PG9 debt.

All8 deep-fuzz runs reviewed THREE ways EACH: wire/header/chunk/value owners for
both libFuzzer and AFL; actual bounded engine execution (libFuzzer6,661,135/
1,847,064/1,265,285/5,684,485 runs at61s, AFL60s with0crashes/0timeouts);
actual revision/corpus/ASan or CMPLOG confirmation and empty crash artifacts,
finite smokes do not prove every input or reliability of the existing AFL gate.
No native-adapter credit. Existing crash-gate/Python debt retained for PG9 audit.

Before protected merge: head7599588e3a0f1bbe318209accb3b831efac58f04 unchanged,
40SUCCESS/1scheduledSKIPPED, CLEAN/MERGEABLE, reviewthreads0/reviews0. Required
mutation artifact12142160368 sha256ffcd8a971a0cb86ce9422e3261e7cbe63635d28fd30dfae76a48c5082b21ac67
was DOWNLOADED and all16 first failures reviewed above. No admin/bypass/auto-merge.

PG7a canonical snapshot FIXED native8/8 PASS0.481s,0skips. Three reviews:
(1) both caller search-path pools use actual canonical snapshot/visibility while
retaining full committed-set, finite pages, event-only/binary/reopen contracts;
(2) BOTH original[] function-shadow witnesses now yield exact committed ID,
held-earlier-xid omission also stays fixed;20k-key/1MiB-body bounds and sole data
connection checks still run; (3) original foreign schemas/pools cleaned before
assertions, strict all-target/all-feature Clippy passes, no source hook or fake
store/new parser/framework/API, unchanged3s and262144 thresholds.8selectedactual
native cases, no filtered/cached/compiler/native replay credit.

Protected PR383 merged2026-10-18T15:55:19Z:
c482fb6a3762c74d7b10416029de22a642de2b71, matching reviewed signedhead7599588e.
All40requiredgreen/1scheduledskip,0reviews/threads, CLEAN/MERGEABLE immediately
before normal squash --match-head-commit; no admin/bypass/auto. Main tree identical
to reviewed source. Mutationartifact12142160368 downloaded/per-case reviewed;
CI39141316095 and mutations39141316085 exact delivery linked above. Projection
checkpoint feature delivery complete; PG7–PG9, publication and clean registry
consumer remain. Remote feature branch already automatically deleted on merge;
explicit redundant delete reported absent, verify ls-remote before any cleanup.

PG7a base aligned to merged checkpoints c482fb6a after verifying tree equality
against reviewed7599588e. Named stash65981423d8263e5a00d6df11e62668b3ed4cd2b6
retained, empty-range rebase then apply--index restores ALL tracked/untracked
work without conflicts. Parent worktree stays clean at7599; root branch preserved.
No source content change from rebase, no fresh runtime credit. Root progress now
points to current export worktree, eight-law candidate and remaining controls.

PG7a FINAL-source unbounded control native0/1 FAIL0.256s,7filtered. Three reviews:
(1) whole catalogue must not be fetched for first item; (2) same current canonical
full-snapshot source with ONLY LIMIT→OFFSET0 allocates3,415,431>262,144 bytes on
20k real rows; (3) adapter-specific proof clean99.8MiB/recompile exact isolated
manifest/tmp/mnesis-pg7a-final-unbounded.Z4Oa2z, failure is allocation invariant,
not SQL/setup/timeout. This supersedes prior xmin-candidate control as FINAL
source evidence; prior result retained. Positive product remains unchanged.

PG7a wiring/documentation BEFORE edits: <=5 lines existing Nix integration gate
for required listing selector, <=25 README lines/<=6 changelog/<=3 Limits doclines.
No new job/tool/package/runtime; existing native archive remains all-features.
Assert missing export selector exits4 with --no-tests=fail, full suite remains
required. State captured visibility and ID+32 accounting/connection release;
ordinary listing+per-stream export do not promise shared multi-stream backup.

PG7a FINAL-source body control native0/1 FAIL0.074s,7filtered. Three reviews:
(1) oversized IDs must error without body transfer; (2) only CASE ELSE returns
stored1MiB key, original exact RowLimit still matches but actual allocation
5,235,056>262,144 fails; (3) proof adapter clean103.8MiB/explicit recompile from
/tmp/mnesis-pg7a-final-body.RwmWNg, first accepted37byte item unchanged, failure
is unwanted allocation not decoding/setup/deadline, threshold untouched. Terminal
poll assertions after this failure are unrun and not credited for mutant.

PG7a FINAL-source visibility-omission control native0/1 FAIL0.042s,7filtered.
Three reviews: (1) frozen listing excludes active/future stream membership;
(2) removing ONLY native snapshot predicate returns exact forbiddenID'late'
instead of'original' after real Barrier-held writer commits; (3) proof adapter
clean103.8MiB/recompile isolated/tmp/mnesis-pg7a-final-visibility.IB3vFx, failure
is membership equality, not overlap/pool/SQL setup or deadline. Later terminal/
current-list assertions unrun in mutant are not successes. Allthree final-source
controls now independently reject actual defects. Strict Clippy passes unchanged.

PG7a complete all-feature native192/192 PASS20.846s,0skips,15binaries. Three
reviews: (1) all prior184 contracts plus8 listing laws execute with import/state
features present; (2) original bounded read/wake/atomic/commit uncertainty,
projection reload/SIGKILL/upgrade and new visibility/search-path/byte/lease laws
all run; (3) actual current export worktree compiled, dedicated realPG17.10 serial
cluster,192 expected selected tests present,0skips, no installed replay, filtered
or DB-free credit. Production88added/1removedRust including2doclines,1feature,
5existingNixgatelines within budgets;334native within450. Linux gate still building.

PG7a export-only native126/126 PASS15.041s,0skips. Three reviews: (1) default118
plus8 export listing laws compile/run without import/snapshot/projection flags;
(2) key-only saved state exclusion uses real installed tables independently of
feature unification, all8 native visibility/body/allocation/lease/reopen laws and
original default laws execute; (3)126 expected cases/15 compiled binaries with
capability-only binaries empty as intended, selected tests0skips, exact standalone
feature source/path. No optional capability runtime credit. Raw evidence retained.

PG7a default native118/118 PASS14.857s,0skips. Three reviews: (1) installed schema4
and original default contracts remain available with export disabled; (2) original
bounded read/final-hint/watermark/outcome/native race/upgrade witnesses execute;
(3)118 expected actual native cases/serialPG17.10, optional listing/state/import
cases correctly absent and not successes, no feature unification or silent skip
credited. No production change after last all/export-only tests.

PG7a missing-export negative exits4/no tests:0runtime. Three reviews: (1) required
listing gate must fail if feature/test absent; (2) same exact Linux selector under
default feature yields0selected and nonzero4, paired with positive export-only
binary/reopen law; (3) no compile/DB/setup error mistaken for absent suite,
0skips/0successes, --no-tests=fail unchanged. No native runtime credit.

PG7a mutation baseline native192/192 PASS21.730s,0skips, fresh separatePG18.6.
Three reviews: complete15binary capability suite; actual8 listing+184prior laws;
real cloned source/profile, no failed baseline, cache/DB-free/replay credit.
All6 generated outcomes individually reviewed THREE ways:
| Candidate | Review1 contract | Review2 first failure | Review3 provenance/limit |
| --- | --- | --- | --- |
| page27 empty | Committed IDs enumerated | Actual[] versus exact one held-xid committed stream | Native92/192,100unrun not passes, writer cleanup BEFORE assertion. |
| page27 PageRow::new() | Typed page values | E0599 no such constructor | Unviable build,0native/no caught credit. |
| page27 PageRow::from_iter | Typed page values | E0599 no such method | Unviable build,0native. |
| page27 PageRow::new(default) | Typed page values | E0599 no such constructor | Unviable build,0native. |
| page27 PageRow::from(default) | Typed page values | E0277 PageRow no Default | Unviable build,0native. |
| listing59 default stream | Actual ID stream | E0277 dynStream no Default | Unviable build,0native. |

Six=1caught5unviable0missed/timeout. Generated mutations cannot substitute for
SQL-specific budgets/frozen membership: three fresh manual SQL controls provide
those actual negative results. Default111s mutation timeout measured from native
baseline, no exclusions/test/threshold changes. Exact artifacts retained.

PG7a first Linux18.6 integration PASS192/19221.014s,0skips. Three reviews:
(1) complete15binary adapter suite and all capabilities; (2) actual native new
visibility/search-path/bounds/lease laws with original state/uncertainty/wake/
upgrade; (3) freshly built archive on ARM builder, installed log SAME execution,
0skipped selected tests. Artifact/nix/store/dwpj7m01m62f15239d35dz0wz58k9r1b-mnesis-postgres-integration.
Four separate smokes atomic0.288s/snapshot0.324s/projection0.374s/listing0.388s,
EACH3reviews: correct requested feature; real native conformance/consumer or
binary/reopen listing law; --no-tests=fail/pipefail unchanged, filtered14/15/2/7
and14otherbinaries not credited. No full-gate/CI/merge/publication claim.

Final boundary audit: existing StreamKey constructors and keys BYTEA NOTNULL
allow an empty rawID. Add emptyID to the existing binary/page/reopen expected
set (within450testbudget),0product change. Expected vector4items, no sentinel
empty-key treatment. Prior Linux/source test version has3IDs, not empty-ID
coverage; fresh selected/native Linux verification required after this refinement.

Empty-ID refinement native8/8 PASS0.530s,0skips. Three reviews: (1) full raw-byte
ID domain includes empty ID independently of end-of-stream; (2) actual append of
empty plus NUL/binary/8KiB IDs twice each, one-row listing and reopen preserve
exact FOUR IDs without duplicates, all prior7 laws remain; (3) no sentinel special
case/new production code or relaxed assertion, selected realPG17.10 cases all
run,4-ID source path compiled, prior3-ID Linux result not claimed to cover empty.
Fresh Linux artifact rebuilding same product with this stricter test source.

PG7a FINAL test-source Linux18.6 integration192/192 PASS21.137s,0skips.
Three reviews: (1) complete15binary/capability matrix with FOUR binary IDs incl
empty in canonical page/reopen law; (2) native empty-ID refinement and all earlier
actual visibility/search-path/bounds/lease plus184 prerequisite laws run;
(3) fresh archive+execution source, not first21.014s run/installed replay, actual
ARM builder/current production hash unchanged7bdaaad5. Artifact
/nix/store/canfglrla50h2zr0y1vy46k943zfxyqf-mnesis-postgres-integration.
Four separate feature smokes atomic0.266s/snapshot0.312s/projection0.374s/
listing0.368s EACH reviewed3ways: exact trait/feature selector, real native
conformance/consumer or FOUR-ID/reopen assertions, --no-tests=fail/pipefail plus
14/15/2/7filtered and14otherbinaries not counted. No cache/replay credit.

PG7a final minimization:88added/1removedRust(including2doclines),1Cargo feature,
5Nixlines,339native lines. Existing BoxStream/try_unfold, SQLx, Limits/PageRow,
key table and immutable event membership reused; zero dependency/lock/runtime/
public-type changes. No snapshot parser, origin API, generic framework or new
worker. Native full-snapshot visibility retained after real xmin counterexample;
canonical function/type names after2real shadow counterexamples. Three fresh
SQL controls plus actual generated mutation first failures retained; no exclusions
or engineered percentage. ProductdiffSHA7bdaaad5165e36b38bfe88738acaac9df09f01ac28deaa7bb95a194ef89a8925.
No applied migration change. Consistent multi-stream session/restore remainsPG7b,
operations/capacityPG8 and full releasePG9. Normal full signed hook now required.

Normal full signed hook passes at3ca0eaa15ed821c1801e2dacd2d2f1fb942c36d8 (G).
Workspace1563/1563 PASS489.402s,14slow,4compiler-function exclusions. Three reviews:
(1) entire default workspace contract owners execute; (2) existing property/
resilience/state-machine and owner tests run unchanged while strict all-feature/
all-target Clippy compiles current listing; (3) DB-free adapter early returns are
NOT native evidence,4compiler functions assigned to existing CI owner job rather
than counted successes, no slowed workload/deadline adjustments. Real192/Linux
native and source controls remain independent evidence. Full unmodified gate
passes12namedchecks, audit cached/11fresh, no bypass. Fuzz replay3runs2/1/1pass
1.00s EACH reviewed3ways: matching header/chunk/value/frame owner; actual existing
corpus decoder assertions;0filtered/ignored, finite replay not deep fuzz or native
adapter evidence. Raw full gate and logs retained. Docs-only amendment runsnormal
hook; cached checks receive0extra runtime credit.

PG7a docs-only signed amendmentf1380837e4b52beb3d963b6b6fabd95c75ce7d58 passes
normalfullhook12cached,0newruntimecredit. Pushedwork/postgres-export, openedPR384
https://github.com/devrandom-labs/mnesis/pull/384; exactCI/artifact/review/protected
merge still required. PG7b starts separately inwork/postgres-views from reviewed
f138source while parentCI runs. New pre-edit budget/falsifiers in HYPOTHESES;
read/tmp/mnesis-pg7-pre-edit.md for supported API research, no mechanism chosen
without native cleanup evidence. Existing immutable contract/protocol retained.

PG7b first compile fails10errors,0runtime: required session trait absent plus two
fixture mistakes (StreamKey is Eq/Hash, not Ord; PendingEnvelope slice input needs
static lifetime or ownership). Three reviews: (1) five initial laws target real
adapter/shared-view/idle/cancel/closed-pool contracts; (2) no actual native cleanup
or consistency evidence from failed compilation; (3) first result is NOT credited
as clean API-absence characterization until fixture errors fixed. Compare sorted
raw byte slices, and mark literal-only payload helper &'static[u8]. Preserve all
assertions/deadlines, no lint relaxation, producer/storage inputs unchanged.

PG7b corrected initial characterization:8E0599 missing ConsistentExporter /
open_export_session methods,0runtime. Three reviews: (1) required consistent
session capability absent; (2) missing trait is ONLY API absence, no native
atomicity/expiry/cancellation evidence; (3) fixture Ord/lifetime defects fixed,
no unrelated compile error, all five laws within600-line budget. Original and
corrected logs retained separately. Next smallest candidate uses existing Tokio
bounded requests/owned task and SQLx borrowed read-only transaction; actively
driven owner needed to avoid unpolled query retaining cleanup mutex. Drop-only
oneshot signal cancels owner independently of queued/unpolled requests; pool
leases remain admission bound. Exact native laws, not assumed cleanup, decide
retention. No raw cancellation protocol, new crate/runtime/framework or hidden
SQLx constructor.3typed dispatch/task variants preserve original sources.

PG7b initial candidate compiles library but strict Clippy rejects owner6arguments
(limit5),0runtime. Represent each view's immutable deadline+page Limits together
as a private Window product shared by session and owner; this owns their common
bounds rather than an arbitrary tuple/allow. Existing lifetime and byte decisions
stay, no public knob, lint relaxation or test change. Candidate291modulelines,
shared-query31/17 and3typed errors15lines stay within450/80 production budget.

PG7b second Clippy rejects83/80 lines in atomic-view test,0runtime. Replaced
three repeated PlannedAppend literals with fixed-array map, same targets, versions,
payloads and all assertions; no lint/limit relaxation. Third strict Clippy passes.
First native attempt run2928c955-d79f-41f8-a764-158562930e1c:0/1 failed3.051s,
four UNRUN. Three reviews: (1) required five session laws still unverified;
(2) failure is fixture priming at line56 before opening any export session, NOT
proof of a session defect; (3) unchanged3s timeout waiting pool.num_idle with
cooperative yield does not itself drive SQLx idle-return work; investigate actual
pool ownership before changing fixture. Do not credit other four as passing.
Raw /tmp/mnesis-pg7b-first-native.log retained. No timeout increased.

PR384 actual downloaded mutation artifact12143007654 digest
39ab9563a2313d030929765dad2e0f83230f6cd3a97fee988b2600785ecdf154,headf1380837,
/tmp/mnesis-pg7a-ci-mutants-39145116667/mutants. Baseline192/28.814s all native,
0skips. Three baseline reviews: contract owners execute actual PostgreSQL18.6;
all eight listing laws included; no ignored/filtered or installed replay credit.
Each generated case reviewed THREE ways (contract; falsifier; oracle/provenance):
| Mutation | Contract | First failure/falsifier | Oracle review |
| --- | --- | --- | --- |
| page=>empty | committed streams must be listed | actual held-transaction law exact[] vs committed-after-held-transaction ID | native assertion at listing_tests296;91passed1failed100UNRUN,0skips,not100successes |
| page=>PageRow::new() | real page construction retained | E0599 missing enum constructor | build-only,0runtime,NOTcaught |
| page=>PageRow::from_iter(default) | real page construction retained | E0599 missing associated constructor | build-only,0runtime,NOTcaught |
| page=>PageRow::new(default) | real page construction retained | E0599 missing enum constructor | build-only,0runtime,NOTcaught |
| page=>PageRow::from(default) | real page representation retained | E0277 PageRow lacks Default | build-only,0runtime,NOTcaught |
| list_streams=>default | valid bounded stream required | E0277 dynStream lacks Default | build-only,0runtime,NOTcaught |
Total6=1caught5unviable,0missed/timeout; no engineered percentage. Fresh final
SQL controls in previous records separately reject unbounded scan/body fetch/
missing snapshot predicate. Full current CI native runs192/28.621s and192/24.836s
are separate; installed28.621s log is replay of FIRST run, not third execution.

PG7b fixture diagnosis CORRECTION: the sole existing listener permanently takes
the only initial idle lease, so polling num_idle alone cannot create a second
lease. SQLx drop DOES spawn return work; that was an unproven earlier suggestion,
not the cause. Reuse existing listing fixture native LISTEN-readiness query before
priming idle count; it acquires a separate observer lease and waits for actual
listener ownership. Same3s bound, same SUT and assertions. This is fixture setup,
not product rescue. Second strict Clippy passes; native b590c7f2-b31c-4239-ae93-fa789e41b528
5/5 PASS1.210s,0skips. Three reviews after run:
1. Contract: one frozen view across listing/both old streams/new stream and actual
atomic commit before first read; close waits native rollback; idle expiration and
canceled startup release actual backend_xmin; invalid ZERO/MAX before storage.
2. Falsifier: live reads independently see both committed second events while
session exact old payload/version remains; native pg_stat_activity proves a real
view existed before cancellation/expiry and disappears without cursor/session
poll. No synthetic reimplementation or successful error-string matching.
3. Assertions/setup: all5 actually ran with DATABASE_URL,0skip/ignored; identical
1s lifetime,3s startup and5s cleanup bounds; cancellation drops pinned BOX future,
not a reference; no detach/leak workaround. These tests do NOT yet prove blocked
SQL cancellation, bounded session admission, caller pool close, all byte/corrupt
paths or backup composition. Add adversaries before retaining candidate.

PG7b actual adversarial run3d416f36-ec1b-4a1a-ac75-7edcbd537d48:7run5pass2FAIL
9.402s0skip. StrictClippypasses. Three reviews after run:
1. Contract: pool.close must terminate outstanding view ownership before60s;
1s expiry must release native backend_xmin despite an abandoned lock-waiting read.
2. Falsifiers: pool.close actually times out3.039s, then succeeds after session
Drop; native pg_stat_activity confirms Lock wait and still-pinned xmin after5s
5.084s, independent table blocker rolled back BEFORE failure. These are real
resource-contract defects, not fixture setup or predicted driver behavior.
3. Oracle/setup: all7 ran, same prior5pass,0skips; timeout bounds unchanged,
blocked next/session/cursor unpolled during observation; native rollback cleanup
precedes assertions, no wire proxy. Owner task + connection.close alone is
INSUFFICIENT for active blocked SQL. Do not credit this candidate as retained.
Research supported server transaction timeout (PG17+ supported profile) and SQLx
pool close event; no custom PostgreSQL cancellation protocol or new dependency.

PR384 CI reviews after completed actual runs (headf138,run39145116648 and push
39145081397). Each summary retained separately; installed native run.log replay
not credited again. Three independent reviews for EVERY listed run:
| Runs | Contract review | Fault review | Assertions/provenance review |
| --- | --- | --- | --- |
| Native192/28.621s,192/24.836s,192/28.320s,192/28.447s | all adapter capabilities and8listing laws | original xmin/search_path and bounded behavior assertions execute | four distinct native executions,0skips; duplicated installed summaries not extra |
| Atomic smoke1 each .079/.063/.075/.078s | import feature actual commit | conformance asserts complete streams |14filtered per run, not14passes |
| Snapshot smoke1 each .062/.074/.062/.067s | snapshot feature | absent/commit/found exact state |15filtered each, only1run |
| Projection smoke1 each .077/.055/.078/.075s | checkpoint feature actual Projection | reload exact state/position/resume |2filtered each, only1run |
| Listing smoke1 each .103/.071/.095/.095s | export feature actual binary IDs |4IDs exactly once incl empty +reopen |7filtered each, only1run |
| Workspace1563/98.446s and1563/120.776s | unchanged workspace contracts | properties/resilience/model owners still run |4compiler functions skipped intentionally with own jobs below; DB-free returns NOTnative |
| Kernel compiler2/107.53s and2/67.16s | static misuse rejects | existing compiler fixtures exact diagnostics |0filtered/ignored; complements4workspace exclusions |
| Macro compiler1/3.82s and1/2.45s | generated misuse rejects | actual trybuild diagnostics |0filtered/ignored, no fixture blessing |
| Store compiler1/11.74s and1/7.26s | static store misuse rejects | actual trybuild diagnostic owner |0filtered/ignored |
| Features,3runs each job in EACH CI: mnesis15,macros1,store217,wake17,fjall131,inmemory25/25/30 | selected no-default/default/all contract owners | unchanged unit properties/errors execute under each feature set |0filtered/ignored; native absence NOT claimed as adapter runtime |
| Feature wake-nostd and store-testing,3each perCI,0tests | isolated compilation only | no runtime falsifier |0test result explicitly no runtime credit |
| Fuzz replay2/1/1 each1s in EACH CI | header/chunk/value/frame corpus owners | real decoder corpus assertions |0filtered/ignored; finite replay not new deep fuzz |
| LibFuzzer chunk1245257, value5831023, wire6580783, header2955147 iterations,61s EACH | four production decoder targets | actual fuzz inputs/no crash | bounded smoke only, not proof of completeness or adapter runtime |
| AFL wire60s,chunk61s,header60s,value60s | same actual four decoder owners |0crashes/timeouts; corpus new0/12/1/0 | actual CMPLOG validation; artifact absence not proof current crash gate catches injected crash, PG9 remains |
40green/1scheduledskip, CLEAN/MERGEABLE, exactheadf138,0reviews/0threads verified.
Mutation artifact per-case reviewed above BEFORE merge. Normal protected squash
now permitted by existing user authorization. No approval bypass/admin merge.

PG7b supported connection-check +pool-close experiment run93da317b-eb5d-470d-9d70-02deb5052781:
7/7 PASS2.348s,0skips, strictClippyPass. Three reviews after run:
1. Contract: unchanged five original laws and both newly-red cleanup laws now
execute successfully; fixed consistent snapshot, exact events, expiry and close
semantics preserved. No lifetime validation/range or public API change.
2. Falsifiers: SAME actual native Lock wait now loses pinned xmin in1.046s
without releasing independent blocker; same60s idle session permits caller close
.033s rather than original3s timeout. Supported LOCAL socket checking addresses
actual blocked-query mechanism, existing pool close_event drives idle ownership.
3. Oracle/provenance: all7actual native,0skip; independent blocker rolled back
BEFORE assertions; lifetime1s/cleanup5s/startup3s unchanged, no ignored/adapted
expected errors. Narrower caller interval preservation/drop while long-lived SQL
blocked/cursor bounds/storage-error source/backup still need their own witnesses.
No inference about packet loss/proxies: socket-close check only sees kernel state.

PR384 merged through normal protected squash at2026-10-18T16:52:53Z,
88a4575b3a3d0c5db8aaac2b92241c76b19f9a60, reviewedheadf1380837e4b52beb3d963b6b6fabd95c75ce7d58.
All40green1scheduledskip/CLEAN,0reviewthreads, downloaded mutation per-case reviews
preceded merge. Remote work/postgres-export absent by exact ls-remote verification.
Parent worktree preserved clean at reviewedhead; no push/recreation. Listing
feature complete; consistent session/backup/restore and PG8/PG9 still outstanding.

PG7b resource-law attempt first stopped at strictClippy due explicit test panic,
0runtime/no native credit. Replaced match/panic with exact source+SQLSTATE
assertion; added fused-none assertion, no lint allow. Native run4ee4a694-7d1a-42f2-a8c8-f614828042e5
11/11 PASS4.663s0skips; strictClippyPass. Three reviews after run:
1. Contract: original7 plus long-lived Drop/blocked SQL, pool-bound admission,
original SQLSTATE, inclusive read/terminal row limits. One borrowed native view,
no writes/commit classification change. Export-only still needs separate feature run.
2. Falsifiers: actual60s view Drop removes Lock-pinned xmin1.040s; pool max2
(listener+view) rejects second open with ORIGINAL PoolTimedOut then reopens after
close; native pg_stat_activity EXACT2connections; real pg_terminate_backend gives
original57P01;37-byte exact event then74-byte overflow, listing33 then72 limit.
3. Assertions/provenance:11actualnative0skip, all prior deadlines unchanged;
blocker released before failing assertions, error cursor fused and cleanup awaited;
no source string remapping/defaults.568native lines <=600budget;317modulelines,
shared SQL/Error helpers still <=450/80production. Allocation proof still owned
by existing shared query controls; this test proves result/order, not RSS or
network partition safety. Caller-stricter connection-check setting remains to
prove with independent wire/source control or supported native metadata.

PG7b full native all-feature203/203 PASS24.551s0skips. Three reviews:
1. Contract: all previous192 adapter owner laws plus11new session laws execute,
including reads/wakes/outcomes/migrations/binary/import/snapshot/projection.
2. Falsifier: original no-hint/native transaction rollback/actual COMMIT-watch/
process-kill and new real active lock-pinned session cleanup assertions remain;
shared stream SQL refactor preserves original bounds/corruption/read properties.
3. Oracle/provenance: actual17.10 DATABASE_URL,serialsamecluster,0filtered/ignored,
no setup-only success claimed, no shortened workload/loosened deadlines. New
11focused source unchanged since4.663s pass; lifetime/static error constraints
still pending controls/current isolated features/Linux/gate/mutations.203pass is
not PG7 complete or merged/released delivery.

PG7b export-only136/136 PASS18.969s0skips. Three reviews:
1. Contract: all default owner118 plus8listing and10session laws run without
import/projection/snapshot capabilities, required11th atomic-session law is
import-gated by its real consumer, not silently counted.
2. Falsifier: real pool-bound/57P01/native lock cleanup/page byte limits and
original default reads/wakes/migrations still execute. No all-feature unification
credited to this isolated build.
3. Oracle/setup: actual17.10 DATABASE_URL0skip, serialphysicalcluster. Four unused
imports from import-only atomic case warn (Arc/TryStreamExt/Store/Barrier);
fix with narrow matching cfg imports, not allow. Native assertions unchanged;
strict isolated Clippy must pass before final gate.136 green isn't warning-free
readiness. Raw run retained, not replaced by corrected result.

PG7b fresh read-committed source control /tmp/mnesis-pg7b-read-committed.i5KkHe,
owned proof target explicit adapter clean/recompile from THAT path:0/1 FAIL.072s,
actualrows2 expected1 at session_tests163. Three reviews:
1. Contract: atomic multi-stream commit must be absent from the established view.
2. Falsifier: ONLY transaction isolation lowered, listing's captured visibility
still applies; actual stream data includes forbidden second row, not API absence.
3. Oracle/provenance: clean+explicit compile path in raw log,1run10filteredNOT10
successes; no fixture/lint/bound changes. Control source before import-cfg tidy
and equivalent local-timeout parameter binding; no final-source claim yet. Need
final current-source controls because later source changed (semantic-neutral
cfg imports and one SQL parameter owning1000 once). Preserve this real witness.
Minimization binds1000_i64 once instead of three duplicated interval constants;
current all/default/export strict Clippy runs, no lints relaxed.

PG7b FINAL-source socket-check control /tmp/mnesis-pg7b-disconnect-disabled.LqroYB:
ONLY native interval bound1000_i64=>0_i64, owned proof adapter clean+explicit
source recompile, d663f243-0223-4e46-822d-98bcc823d71c0/1 FAIL5.062s. Three reviews:
1. Contract: canceled/dropped60s session must release actual native read view.
2. Falsifier: kernel-close detection disabled; actual Lock-waiting backend_xmin
persists beyond5s and original cleanup assertion fails. Independent blocker
rollback precedes assertion; normal1s interval same testpassed1.040s.
3. Oracle/provenance: final parameterized SQL/current test-importsource, explicit
compilepath/clean in raw log;1run10filteredNOT10passes, no deadline/expectation
change. Pure supported timeout causal control, not test-only driver simulation.
Strict current all/default/export all-target Clippy each passes warning-free;
no runtime credit from compilation. Native136 earlier raw warnings retained.

PG7b current default-only118/118 PASS15.435s0skip. Three reviews:
1. Contract: raw adapter default owners remain available without export capability.
2. Falsifier: unchanged read/all bounds, no-hint recovery, commit uncertainty,
migrations/legacy upgrades/corrupt schema/property boundary laws still execute.
3. Oracle/provenance: actual native17.10,serialphysicalcluster,0skip/filtered;
export API absence is NOT tested by118 and gets no session runtime credit.
Exact absent feature selector --no-tests=fail still required separately.

PG7b FINAL-source isolation control /tmp/mnesis-pg7b-final-isolation.qxqkA0:
explicit proof adapter clean/recompile, same current parameterized SQL/cfg imports,
ONLY RR=>READCOMMITTED.0/1 FAIL.057s, exact actual2 expected1. Three reviews:
1. Contract: one cross-stream view includes no part of later atomic commit.
2. Falsifier: actual forbidden second event despite frozen listing; independent
live raw store confirms commit, no synthetic source/API-only failure.
3. Oracle/provenance: explicit compile path/clean,1run10filtered, zero loosened
limits/oracles. Supersedes earlier pre-parameter control for final source proof;
both original logs retained. Final production stays unchanged during controls.

PG7b FINAL current export-only136/136 PASS19.915s0skip after native interval
parameterization and cfg-import fixes. Three reviews:
1. Contract: actual118default+8listing+10session laws with onlyexport enabled.
2. Falsifier: native blocked-query expiry/drop/pool-close/resource saturation/
original57P01/terminal37-byte bounds and all default defects execute unchanged.
3. Oracle/provenance: current source warning-free, real17.10,0skips/filtered;
import-only atomic-view case excluded by actual capability, not credited here.
All-feature203 prior parameterization remains earlier evidence; final Linux
current203 running separately. Default118 current passing. No final-release claim.

PG7b missing-export selector run39a1f513-60da-480e-995a-c70310e0a639 exits4,
0tests. Three reviews: (1) required native session smoke must not succeed when
feature absent; (2) exact session_tests close/drop selector unavailable yields
no-tests error via existing nextest --no-tests=fail; (3)0runtime/15binariesfiltered,
not an export behavioral pass. Final native Linux package succeeds at
/nix/store/0vv8gv9db3m8ikrml0pqrzmlh2s7335f-mnesis-postgres-integration. Review
separate5feature smokes and203whole native run below; installedlognotextraexecution.

PG7b Linux current203/203 PASS27.703s0skip, source diff
SHA256d5e7e35be8b81624f892832f7979b9075405987ffd829b919e145c68841941c3.
Three reviews after EACH native run:
| Run | Contract | Actual falsifier | Oracle/provenance |
| --- | --- | --- | --- |
| atomic1/.313s | import actual multi-stream transaction | exact all targets committed |14filtered not14successes |
| snapshot1/.256s | saved cache composition | absent/commit/found exactpair |15filtered,1actual |
| projection1/.204s | actual stepper resume | exact state/nativeposition |2filtered,1actual |
| listing1/.271s | export binary discovery |4exactIDs+reopen |7filtered,1actual |
| session1/.106s | export native view lifecycle | actual xmin/close ACK/Drop/reopen |10filtered,1actual |
| all203/27.703s | all adapter owners+11session laws | real blocked SQL/pool/drop/native error/bounds |0skips,18.6 ARM Linux fresh nativeexecution; installed log not extra |
Raw /tmp/mnesis-pg7b-linux.log and immutable result retained; strictClippy all/
export/default compiled warning-free. Five native smokes required with missing
selector exits4. No inferred partition/durability/capacity acceptance.

PG7b first22-candidate mutation attempt REJECTED baseline failure:12/203run,
11unitpass1nativefailure,191UNRUN. Three reviews: (1) no successful native
baseline,0mutants tested; (2) actual failure is database role28000 'joel does not
exist', not adapter regression; (3) selected existing18.6 mutation cluster belongs
to mnesis, ordinary17.10 cluster usesjoel. Correct CONNECTION USER only, preserve
all test/source/bounds and original failed logs. No survivor/caught count claimed.
Reused test cluster automatic crash replay is not a controlled durability drill.

PG7b corrected current22 generated mutations:9caught13unviable0missed/timeout,
203/26.294s baseline0skips. Every first failure inspected before retention;
/tmp/mnesis-pg7b-corrected-mutants/mutants.out preserved, outcomes/first failures
copied as.log. Three reviews after EVERY run/case:
| Case | Contract | First falsifier | Oracle/provenance |
| --- | --- | --- | --- |
| baseline | all203actual owners | unchanged native regressions execute |18.6 independentcluster,0skips,correctusermnesis |
| stream_query=>new() | valid native stream query required | E0061missingarg |0runtime,unviableNOTcaught |
| stream_query=>from_iter | same query ownership | E0599missingmethod |0runtime,unviable |
| stream_query=>new(default) | same bound query | E0283inference |0runtime,unviable |
| stream_query=>from(default) | committed history cannot disappear | actual[]vsone ConformanceRow(version1,E,payload9) atomic duplicate-target preservation |14pass1fail188UNRUN, data failureNOTtypeerror |
| raw read_stream=>default | valid stream required | E0277dynStreamDefault |0runtime,unviable |
| listing page=>empty | committed streams discoverable | actual[]vsheld-transaction committedID |91pass1fail111UNRUN, real listing assertion |
| listing page=>new() | page representation valid | E0599PageRownew |0runtime,unviable |
| listing page=>from_iter | page representation valid | E0599PageRowfrom_iter |0runtime,unviable |
| listing page=>new(default) | page representation valid | E0599PageRownew |0runtime,unviable |
| listing page=>from(default) | page representation valid | E0277PageRowDefault |0runtime,unviable |
| Windowlive=>Ok | expired buffered data must stop | actual second buffered item notExpired at session211 |174pass1fail28UNRUN; actual native idle expiry |
| Windowlive>= to< | available pre-expiry reads permitted | list creation incorrectlyExpired at session482 |168pass1fail34UNRUN; entry guard,NOTbackendSQLSTATE proof |
| Windowunavailable=>default | typed original errors required | E0277ExportErrorDefault |0runtime,unviable |
| Windowunavailable>= to< | expired pending reads must reportExpired | native lock-read terminal error wrong at session339 |172pass1fail30UNRUN, real expiry assertion |
| request=>default | typed reply cannot invent value | E0277TDefault |0runtime,unviable |
| open=>default | actual Session must exist | E0277SessionDefault |0runtime,unviable |
| open lifetime OR=>AND | ZERO/MAX reject before closed pool | exact InvalidLifetime assertion fails session258 |175pass1fail27UNRUN; actual input/storage precedence |
| own=>Ok | opening must establish real ready view | originalExportReplyRecvError onopen at474 |168pass1fail34UNRUN; startup guard,NOTcleanup proof |
| serve=>Ok | same real native startup | originalExportReplyRecvError onopen at474 |168pass1fail34UNRUN; startup guard,NOTcleanup proof |
| sessionlist=>default | valid borrowed stream required | E0277dynStreamDefault |0runtime,unviable |
| sessionevents=>default | valid borrowed events required | E0277dynStreamDefault |0runtime,unviable |
| close=>Ok | cleanup failures cannot be acknowledged | actual57P01 owner shutdown incorrectly returnsOk, closed.is_err fails493 |168pass1fail34UNRUN; failure propagation,NOTtemporalACK proof |
No new exclusions/config changes or engineered percentage. Explicit normal-close
native xmin oracle and source controls remain separate; robust pending-close
cancellation and strictly shorter caller interval profiles remain credible
follow-up lifecycle/security work (PG8), not claimed by generated totals.
Current product371added24removedRust,572native lines,5Nix lines;1Sessiontype,
3typederror variants;0deps/runtime/framework/tools. Final source diff SHA256
d5e7e35be8b81624f892832f7979b9075405987ffd829b919e145c68841941c3.

PG7b full flake gate IN PROGRESS, source unchanged. Three reviews immediately
after each completed replay run: (1)2-case CBOR corpus replay targets actual
header/chunk decoder owner; actual decoder assertions run;0ignored/filtered1.00s,
not deepfuzz/native adapter evidence. (2)1-case value corpus replay targets actual
value validation; original corpus assertions execute;0ignored/filtered1.00s,
not capacity evidence. (3)1-case wire corpus replay targets real frame decoder;
actual frame checks execute;0ignored/filtered1.00s, not new input coverage proof.
These are three distinct contract/fault/oracle reviews PER run; replay counts
2/1/1 retained in raw full gate. Owned mutation cluster stoppedFAST/ACK, data kept.

PG7b full unmodified nixflakecheck passes12namedchecks:10fresh, audit+toml-fmt
cached. Workspace1563/1563 PASS519.852s,13slow,4compiler-function exclusions.
Three reviews after workspace run:
1. Contract: unchanged whole-workspace kernel/store/adapters/examples/property/
state-machine/resilience owners execute, including existing persistent backup
composition; current session code separately compiled under all-feature Clippy.
2. Falsifier: actual pre-existing property/recovery workloads retained; no cases,
loops, deadlines, persistence settings or exclusions altered. Sourcecontrols and
203nativeLinux remain independent original-defect rejection evidence.
3. Oracle/provenance:4compiler functions NOT counted passes, assigned existing
CI owner jobs; DB-free Postgres returns NOT native runtime.13slow completed
unchanged; timing difference vs prior489s not treated as adapter capacity result.
Replays2/1/1each1s separately reviewed above. NativeLinux203/27.703s+5smokes was
actual earlier final-source execution, not repeated by Darwin gate. Incompatible
architecture warning not additional platform verification. Raw fullgate retained
/tmp/mnesis-pg7b-full-gate.log; concise extracted summary in evidence. Strict
source/test/Nix whitespace check passes; original raw negative logs preserve
nextest trailing whitespace. No gate bypass, lints relaxed, test allowances added
or Cargo/lockfile changes. Signed normal commit hook still required and will
receive0fresh runtime credit for cached checks.

Protocol chronology correction: the5-line native session smoke budget note was
written immediately AFTER that small gate edit, not before as its wording said.
The prior450/80Rust and600native-line budgets were recorded before production
implementation; final371/24Rust and572native stay within them. No source or gate
relaxation is justified by this record correction. Before further scope changes,
record the new budget and falsifier first.

PG7b signed conventional commitcd3ec0355299b574c5fefe3dfa50830b20bcf987 (G)
passes normal unmodified pre-commit nixflakecheck,12cached checks,0new runtime
credit. Full actual gate1563/519.852s recorded above. Docs-only record amendment
uses same normal hook; source/native controls/LINUX/mutation evidence unchanged.
No source change or bypass; focused protected PR still required before delivery.
User-directed release repair while PG7bPR385 runs CI. ReleasePR378 head8836a37c,
failedrun39147192934:4AFLtargets each `cargo afl build --release --locked` exits101
stale fuzz-afl/Cargo.lock;0fuzz executions. Three reviews: (1) required locked
harness builds blocked; (2) actualCargo lock mismatch, not decoder crash;
(3) all4buildfailures retained, nonecounted runtime/caught input, fuzz-gate correctly
fails. LibFuzzer successes don't rescue absent AFL runs. CodSpeed check117734118722
also reports3decodecases-10.42% and differentruntimeenvironments; cause UNPROVEN,
no performance acknowledgement or gate relaxation. Branch-local repair budget in
HYPOTHESES, PG7bsource kept separate. Publication remainswhole-adapter-gated.

Release lock reproduction at original8836a37c: actual Cargo metadata--locked
--offline exits101 with same stale-lock error as four AFLbuilds,0runtime. Three
reviews:1lockedgraph must resolve;2exactCargo config rejection not a decoder crash;
3nativeCargo/SUTmanifestactualhead, no --locked removal. Initial selective update
attempt incorrectly included mnesis-macros (not in either harness's selected
feature graph), aborts/no file changes; corrected actual graph has only mnesis
and mnesis-store local packages to update. Native Cargo selected updates produce
ONLY4version edits across2locks (.4.0=>.4.1), allregistry/checksum pins unchanged.
Both corrected Cargo metadata--locked--offline pass. Three reviews each graph:
1same locked harness graph resolves;2original version-mismatch rejected by original
control now corrected;3resolver-only,0AFLbuild/fuzz runtime credit, no dep upgrade.

Release workflow candidate45lines, no Rust/dependency/tool/script additions. Reuse
supported release-plz outputs; upstream v0.5 source shows missing PR is {} and
prs_created=false, so gate BOTH post-steps on that true output. Actual current
GitHub schema confirms CommittableBranch.branchName (not older docs' refName).
Separate release-pr checkout leaves root publisher on triggering main commit;
manual dispatch job also requires refs/heads/main. Cargo updates only2local paths
in each isolated workspace; GitHub App createCommitOnBranch retains PR metadata,
produces verified commits and expectedHeadOid prevents overwrites. Full original
--locked AFL builds/gates untouched. Existing user's Cesr uses same App/signing
owner; no new bot, runner or framework. Official release-plz output and GitHub
GraphQL primary docs inspected; actual schema checked against live endpoint.

Native workflow controls use EXACT YAML-extracted shell body, not reimplemented
logic. Fresh release-head probe8836 runs its Cargo/JQ body, emits valid exact
2lock payload/4version edits. Only expectedHeadOid deliberately changed to stale
88a4575b; REAL GitHub API returns STALE_DATA, createCommitOnBranch=null, and
remote PR378 head remains8836. Three reviews:1concurrent head must never overwrite;
2actual conditional write rejected at server;3only changed fault input, no mock,
no external write/nativefuzz credit, API response+exactremotehead recorded.
Already-current main88a4575b runs FULL exact workflow body: Cargo locks0updates,
exit0 before payload/API; no release-locks.json created, both locks unchanged,
rootHEAD unchanged. Three reviews:1idempotent refresh;2actual no-change path makes
no write;3actualshell/Cargo, no mocked API; not live publisher proof. Locked
nixpkgs actionlint passes candidate and corrected final workflow; three reviews:
valid native action/schema references; existing publish/gate steps retained;
static check only, no live Actions or publication credit.45<=80budget.

CodSpeed public report's ACTUAL environment data: release8836 on AMD EPYC7763,
base88a4575 on EPYC9V74, equal recorded CPU flag lists. Compiler1.99.0b940084d7,
LLVM23.1.1, cargo-codspeed5.0.3 and Ubuntu20261012.334.1 match both logs. Three
pure decode cases report501.7ns=>560ns (-10.42%); root release diff only versions/
changelogs, no decoder source change. Different CPU is EVIDENCE of environment
change, NOT yet proof of entire cause. No threshold, workload, mode, binary or
acknowledgement changed. Raw publicHTML/JSON kept outside repo (contains unrelated
page metadata); selected environment facts in.log. Actual trace/counter comparison
or controlled same-environment reproduction still needed before closing this
failure. No registered repo self-hosted runner, no new infrastructure selected.

PG7b protected CI reviewed at exact signed head
f94333c7649a1a69e875728bc4a5553355f11fa7. All40checks SUCCESS,1 scheduled sweep
SKIPPED,0reviews/threads; mergeability currently UNKNOWN (not yet merged).
PR39149415319/push39149411578 actual native203 each in28.025/33.572 and
35.239/35.414s respectively. EACH of FOUR executions reviewed THREE ways:
1complete all-capability native contract;2all11 new real session adversaries plus
192 prior laws;3actual fresh execution,203 selected0skips, installed duplicate
summary is replay, no additional run. Each of20 feature smokes reviewed THREE
ways: exact atomic/snapshot/projection/listing/session capability; actual native
owner assertion; --no-tests=fail/pipefail retained,1 actual each,14/15/2/7/10
filtered not passes. Timing/counts in pg7b-ci-summaries.log.

Both full CI workspace runs1563/1563 PASS120.722/107.856s,4compiler functions
excluded. EACH reviewed THREE ways: workspace owner contracts; actual unchanged
property/resilience/state-machine assertions; DB-free early returns not native,
compiler exclusions covered by separate owner job. Compiler owner jobs EACH
reviewed THREE ways: exact intended diagnostic contract; real rejected fixture
compilation; kernel2/macro1/store1 functions,0skips and no fixture rewrites.
PR109.07/3.94/11.84s, push109.11/3.92/11.69s. Each of six replay runs2/1/1 per
CI invocation reviewed THREE ways: actual decoder owner; corpus assertions;
finite replay,0filtered, not new deep fuzz or Postgres evidence. Feature-unit
matrix summaries retained; EACH run reviewed THREE ways: isolated owning feature
contract; actual unit assertion or compilation obligation; 0-test harnesses are
compilation-only and receive0runtime credit, none substitute for native adapter.
No gate, workload, expected output, timeout or lint weakened.

Deep-fuzz39149415450 four LibFuzzer executions PASS61s each: wire6528881,
header3230413,chunk2229169,value10361584 inputs. EACH reviewed THREE ways:
matching production decoder contract; actual mutated-input execution no crash;
finite campaign, not exhaustive or native database evidence. Four actual AFL
runs PASS60s, wire37.25%/header47.61%/chunk43.39%/value8.31% reported bitmap
coverage,0crashes/timeouts each. EACH reviewed THREE ways: same required target;
actual locked build and execution reaches time limit; percentages are AFL bitmap
coverage, not product line coverage; known separate crash-exit gate gap remains
PG9, not erased by these no-crash campaigns.

PG7b actual CI mutation artifact12144836614/run39149415324 SHA256
74c563874ac9918daa08b868a7e56cb6b91afcb26102ae0ea480d22d1ac12648,
22=9caught13unviable0missed/timeout. Downloaded actual outcomes AND every first
failure inspected; not inferred from aggregate green. Native baseline203/203
PASS29.328s reviewed THREE ways: whole-capability contract; real203 actual laws;
0skips and native source, not local-baseline replay. Per-outcome THREE reviews:

| Candidate | Review1 contract | Review2 actual first fault | Review3 provenance/limit |
| --- | --- | --- | --- |
| stream_query562 new() | Valid query construction | E0061 missing argument | Unviable,0runtime/caught credit. |
| stream_query562 from_iter | Valid query construction | E0599 unsupported function | Unviable,0runtime. |
| stream_query562 new(default) | Typed query construction | E0283 ambiguous type | Unviable,0runtime. |
| stream_query562 from(default) | Committed event fidelity | Actual[] vs exact v1/E/payload9 |14pass1fail188UNRUN, actual atomic conformance row oracle. |
| read_stream634 default | Event stream type | E0277 dynStream no Default | Unviable,0runtime. |
| listing27 empty | Committed stream enumeration | Actual[] vs exact committed-after-held-transaction ID |91pass1fail111UNRUN; actual visibility oracle. |
| listing27 new() | Typed page | E0599 PageRow::new missing | Unviable,0runtime. |
| listing27 from_iter | Typed page | E0599 PageRow::from_iter missing | Unviable,0runtime. |
| listing27 new(default) | Typed page | E0599 PageRow::new missing | Unviable,0runtime. |
| listing27 from(default) | Typed page | E0277 PageRow no Default | Unviable,0runtime. |
| live59 Ok | Expiry includes buffered rows | Actual buffered.next fails exact Expired match |174pass1fail28UNRUN;1.060s actual expiry. |
| live59 >= to < | Live view permits reads | Premature Expired at pre-termination list.unwrap |168pass1fail34UNRUN; entry guard, NOT57P01 fault coverage. |
| unavailable67 default | Original error type | E0277 ExportError no Default | Unviable,0runtime. |
| unavailable67 >= to < | Expired abandoned read reports expiry | Actual cursor.next wrong error |172pass1fail30UNRUN;1.061s native blocked-read law. |
| request80 default | Generic request response | E0277 T no Default | Unviable,0runtime. |
| open98 default | Session type | E0277 Session no Default | Unviable,0runtime. |
| open98 OR to AND | Reject zero/MAX lifetime before pool | Exact InvalidLifetime match fails |175pass1fail27UNRUN; original boundary assertion retained. |
| own149 Ok | Owner must establish view | ExportReply RecvError at session startup |168pass1fail34UNRUN; startup guard, NOT native termination cleanup. |
| serve170 Ok | Owner must serve view | ExportReply RecvError at session startup |168pass1fail34UNRUN; startup guard only. |
| list236 default | Listing stream type | E0277 dynStream no Default | Unviable,0runtime. |
| export262 default | Event stream type | E0277 dynStream no Default | Unviable,0runtime. |
| close303 Ok | Cleanup errors preserved | Actual57P01 close incorrectly acknowledged: closed.is_err fails |168pass1fail34UNRUN; error fidelity, NOT temporal rollback-ACK proof. |

Separate fresh READCOMMITTED and disabled-native-disconnect controls remain the
SQL/cleanup defect witnesses. No generated percentage substitutes for those
or the blocked pool-close/drop laws. Artifact and summary records retained;
normal protected exact-head merge still pending GitHub mergeability result.

PG7b DELIVERY UPDATE: actual GitHub event reports PR385 merged by joeldsouzax at
2026-10-18T17:41:10Z, merge12fd839c7b08e2eda1a8020bae9f52dd898c1d15, exact signed
headf94333c7649a1a69e875728bc4a5553355f11fa7; remote branch deletion17:41:12Z.
This agent discovered the merge while inspecting UNKNOWN mergeability and did
not issue that merge request. All40green/1scheduledskip and downloaded artifact
reviews above refer to exact delivered head. Parent views worktree kept clean.
Workflow repair rebased onto actual main12fd, preserving both append histories;
pre-rebase work retained in named stash (OID in progress). No publication claim.
ReleasePR378 refreshed to9aab7b894f21637ffdf7aad924183a6a58b54b20; same AFL locked
build failure repeats before execution. Original8836 reproduction remains valid
for diagnosed stale isolated lock graph. CodSpeed new report still in progress.

Release repair pre-rebase gate at88a finished unmodified12checks PASS; actual
workspace1563/1563 PASS508.004s,14slow4compiler functions excluded. Three reviews:
1whole default workspace contract;2actual property/resilience/state-machine owner
assertions, original workloads;3native database early returns NOT native proof,
compiler functions remain separately owned CI, this older source gate is not
claimed for rebased12fd. Eight named checks cached, four final named checks fresh
(clippy/nextest/store-nostd/wasm) plus shared dependencies; evaluation's '5 checks'
includes dependencies, not five new test executions. No new replay/runtime credit
from cached fuzz check. First signed normal-hook commit111d0b8a396ea5f85e9fe77c2bb62f761433e365(G)
on12fd passes full12checks,9cached+3fresh(clippy/nostd/wasm). THREE reviews:
1entire unchanged gate retained;2actual source compilation and strict lints;
3all runtime tests cached,0fresh runtime/cross-architecture credit. No bypass.

CodSpeed trace discovery resolves one part of the diagnosis: actual public
FindBenchmarkCallGraph query (from CodSpeed's shipped client) retrieves base and
head large-decode callgraphs. Their complete node self totals match reported
501.6667/560ns. Instruction contribution IDENTICAL35ns; L1 miss contribution
22.2222=>25ns and last-level miss444.4444=>500ns explain ALL58.3333ns delta.
Actual decode_frame self instruction contribution IDENTICAL7.7778ns; one extra
simulated L1/last-level miss pair accounts for delta.10nodes9edges both, no
allocation or syscall contribution. This establishes a cold-cache cost difference
rather than extra decoder instructions for this benchmark. Hardware/layout root
cause still not separated; it does NOT justify a manual acknowledgement or
threshold change. Other payload traces and a causal retained improvement pending.
Raw callgraphs retained without expiring signed download URLs. Initial read-only
GraphQL query rejected wrong enum GitHub; native schema response says GITHUB,
corrected query succeeds. Initial jq on gzip bytes rejected; proper gzip decode
used. Neither inspection rejection is a native test or performance proof.
PG7c initial fresh-cluster fixture REJECTED at compilation: Bytes cannot compare
with &[u8;8] (E0277). THREE reviews:1restore coherence invariant not exercised;
2fixture assertion type mistake, NOT evidence of copied-xid defect;3zero runtime/
Postgres startup/test count, no caught-defect credit. Correct only slice coercion
and rerun same test; no original source/expected history assertions changed.
PG7c first actual independent-cluster native run FAIL1/1,1.900s. THREE reviews:
1real migrated source append plus native full-schema pg_dump/psql restore;
2actual from_pool rejects Schema(events columns/defaults/constraints/index), so
this is NOT the claimed xid-watermark fault;3real PG18.6 source/target clusters,
0skips, no history/read assertions reached. Current schema validator hardcodes
stream_id attnum9 after migration dropped an earlier column; pg_dump recreates
only live columns and can renumber them. That cause is a source-backed hypothesis
pending catalog comparison, not accepted explanation. Fail-closed behavior
retained. Next use independently migrated exact target schema and native DATA-
ONLY dump excluding already-installed migration row data. This preserves actual
schema/validation and isolates copied xid data, rather than bypassing auditor
or weakening expected stream/global coherence. Both distinct paths remain in
required logical-restore contract decisions; no raw-schema support claimed.
PG7c isolated DATA-ONLY logical restore produces actual original defect:
FAIL1/1,1.557s, actual global.len()0 versus expected1 after PostgresStore opened
successfully. THREE reviews:1real source/target independent PG18.6 clusters,
source migration+128 actual allocated transaction IDs, actual adapter append,
native pg_dump data-only +psql into separately migrated target;2actual adapter
stream read yields exactly1 original payload, stored source txid strictly exceeds
actual fresh target counter, then actual read_all(None) omits it;3no auditor
bypass (target schema installed and validated by same migrator),0filtered/skips,
no original product changes, all earlier exact payload/counter assertions reached,
server cleanup before final failing assertion. This is real xid-history mismatch,
not the earlier full-schema rejection or compile fixture failure. Both prior
rejected/other-path results retained. No supported logical restore policy or
history/checkpoint context API chosen yet. Need native evidenced completed-CBOR
reimport/rebuild and foreign checkpoint rejection, plus operator drill; current
probe remains RED and uncommitted until contract/implementation resolves it.

Release repairPR386 CI owner jobs inspected: two real native203/203 PASS
32.902/35.986s,0skips. EACH reviewed THREE ways:1whole adapter contract current
12fd production;2all actual session/listing/import/checkpoint/snapshot and prior
fault laws;3fresh native job, installed duplicate summary replay not another run.
Each of ten native feature smokes reviewed THREE ways: exact requested capability;
real native conformance/consumer assertions;1 actual each,14/15/2/7/10filtered not
passes, --no-tests=fail and pipefail retained. Two compiler jobs2/1/1 functions
PASS71.37/2.61/7.67s and109.80/3.95/12.83s. EACH function run reviewed THREE
ways: intended diagnostic owner; actual rejected compiler fixtures;0skips,
no stderr rewrite or native/runtime overclaim. Full Nix jobs still pending.

Release repair mutation39151448951 actual log 'No source changes in this PR —
nothing to mutate', API0artifacts. THREE reviews:1workflow-only production diff;
2native diff selector truly selects0Rustmutations;3green is gate/selection only,
0baseline/runtime/caught/survivor credit, no nonexistent artifact inferred.
Fuzz39151449004 four LibFuzzer61s runs wire6613345/header1763132/chunk1264851/
value5603167 PASS. EACH reviewed THREE ways: unchanged production decoder owner;
actual mutations execute without crash; finite campaign, no adapterDB proof.
Four AFL60s runs PASS with0crashes/timeouts:wire37.25%/header47.61%/chunk43.52%/
value8.31% bitmap coverage. EACH reviewed THREE ways: same target and locked
build; actual native fuzz runs to original time limit; bitmap not line coverage,
known independent crash-exit propagation gap still PG9. No altered workload,
compiler diagnostics, test exclusions, timeout, threshold or gate.
Native17.10 history-capability inspection: pg_control_system().system_identifier
and current pg_database.oid readable by freshly created unprivileged NOLOGIN role
with no added grants. Original speculation that this necessarily needs monitoring
or superuser grants is disproved. Actual transaction rolls role creation back.
THREE reviews:1minimum runtime-read capability;2actual SET LOCAL ROLE invokes
native builtin, not admin-only privilege inquiry;3PG17.10 proof only,18.6 and
physical/logical context semantics not inferred. No API/migration or extra grant
retained. System/DB identity is a candidate needing bounded representation and
checkpoint/history compatibility decision, not an automatically selected design.
Release repair full CI final exact49c1b1c6c0b18e1775071d8249cd4927af2ce541:
40SUCCESS1scheduledSKIP,CLEAN/MERGEABLE,0reviews/threads. Two full workspace runs
1563 PASS115.763/110.972s,4compiler-function exclusions separately verified above.
EACH reviewed THREE ways: entire default workspace owner; actual unchanged
property/resilience assertions; DB-free returns not native and4excluded functions
not passes. Remaining two full-gate native203 PASS34.909/30.928s,0skips EACH
reviewed THREE ways: all current capability contracts; actual session/ordering/
state/uncertainty/wake regressions; fresh actual execution, no installed replay
credit. Each remaining ten feature smokes reviewed THREE ways: owning capability;
actual native law;1selected14/15/2/7/10filtered not passes. Each six replay runs
2/1/1 per gate reviewed THREE ways: decoder owner; actual corpus assertions;
finite replay,0filtered, not additional deep fuzz/native credit. Each feature-unit
matrix run in retained summary reviewed THREE ways: actual isolated capability;
original owner assertions/compilation obligation;0-test harnesses compilation-only,
not native or fabricated runtime successes. No new generated mutation candidates
or artifact;0Rustdiff proven by actual selector/API. No gate/expectation weakened.
Normal protected exact-head merge authorized; live workflow still needs proof.

PG7c fixture refactor (native dump/restore moves to Cluster::copy_into, under
220line test budget) preserves original data-only scenario and every oracle.
Actual fresh native FAIL1/1,2.106s on global0vs1 again. THREE reviews:1same actual
adapter/history invariant;2same exact-payload and source-counter preconditions
passed before final expected1actual0;3only fixture extraction/formatting, no
product/auditor/assertion/timeout relaxation,0filtered/skips. Red remains pending
supported restore/history contract decision, not a passing readiness claim.

Release workflow repair DELIVERY: PR386 normal protected squash at exact signed
head49c1b1c6c0b18e1775071d8249cd4927af2ce541,40success/1scheduledskip,
CLEAN/MERGEABLE0reviewthreads. Merge87e87d6e00b0d7703a93cf577ec4893a9d49d35e
2026-10-18T18:09:28Z; actual normal CLI issued successfully, remote branch absent,
clean worktree retains signed reviewed head. Explicit Release MAIN dispatch
39152877808 at18:09:40Z uses actual merged87e87d6e and contains separate PR
checkout/synchronization. Root main remains0.4.0 (already released2026-10-03),
releasePR378 remainsOPEN9aab7b8 at dispatch; no unmerged PR publication. Live
App-lock commit/AFL proof still pending, independent CodSpeed failure still OPEN.
PG7c current native characterization with context capability assertions FAIL1/1
1.699s on SAME actual global0vs1. THREE reviews:1actual adapter and unchanged
restore/payload/counter invariant;2before final red, separate fresh18.6 clusters
have DISTINCT native system_identifiers but SAME postgres databaseOID, both
readable under newly created NOLOGIN role without grants;3both role transactions
rolled back before actual scan,0skips/filtered,189test lines<=220 budget,
0product changes. This disproves blanket privilege worries for18.6 too and
shows databaseOID alone cannot distinguish these histories. No UUID dependency,
public history/position API or migration chosen yet. Initial patch context
mismatch changed no file; corrected exact source patch then native run above.

Release live workflow39152877808 SUCCESS, ROOTmain87e87d6e,18:09:40–18:11:46Z.
Actual GitHubApp verified(valid) commitdaefe06097f7310ef21a596ca8ee1bb7a5049994,
authordevrandom-release-plz[bot],parent9556f2c985d22fca12ef50424ea411e307ea2b39,
headline chore(release): synchronize fuzz workspace locks. EXACTLY two changed
files fuzz/Cargo.lock and fuzz-afl/Cargo.lock,4local version edits0.4.0=>0.4.1,
no unrelated registry pins/source/workloads changed. THREE reviews:1required
locked fuzz graphs track release versions;2real dispatched workflow/App API
conditional commit succeeds and triggers required checks;3actual API verified
signature and precise parent/file diff, not just resolver/no-op control.
Root publisher logs explicitly skip ALLnine already-published0.4.0 crates; no
0.4.1/incomplete adapter publication. THREE reviews:1separate-checkout publication
boundary;2actual root version/tag skip logs, not branch-ref inference;3release378
remainsOPEN with newhead,0new publication credit. Fuzzrun39153035060 actual four
AFL jobs executing; no repaired fuzz-runtime credit until their real completions.
CodSpeed on refreshedhead fails independently again; no gate waiver or change.
PG9 user-directed release repair continues while focused workflowPR386 head
49c1b1c6c0b18e1775071d8249cd4927af2ce541 runs protectedCI. No workflow delivery or
successful release check yet claimed. Actual CodSpeed traces for ALL3payloads
reconcile exact reported regression: base/head instruction35ns unchanged;
L1/LLmiss22.2222/444.4444=>25/500ns. Exactly58.3333ns difference from simulated
cache behavior, not added instructions, no syscall/allocator cost. THREE reviews
EACH comparison:1same real production decoder and unchanged input;2actual profile
self totals reproduce501.6667=>560ns and decode_frame self accounts for delta;
3profile source IDs from failed8836/base88a, not runtime hardware causation,
not a post-fix result. Live refreshed release9aab repeats identical3regressions.
Pre-edit supported inline-hint hypothesis/budget/falsifier recorded before edit;
actual unchanged native benchmark baseline now compiling. No product edit yet,
no metric/threshold/workload/source-policy change. Native ARM alone insufficient
for x86 release proof. Review after docs-only amended hook: full12checks cached,
0fresh runtime; unchanged complete gate, no source assertion/lint relaxation.

PG9 native unchanged benchmark first run PASS3actual decode workloads, pinned
native environment in evidence, releaseprofile,30samples each,1s warmup+3s
measurement, 64/1024/65536byte payloads. THREE reviews:1actual production decoder
through original owning Fjall benchmark;2no surrogate function/setup in timing,
original bytes/data unchanged;3warm native ARM timing NOT cold x86 simulation,
all three selected, other benchmark groups excluded from this experiment rather
than counted successes. Raw confidence intervals/outliers retained. Repeat same
command twice to measure noise before single-line experiment; no attribute yet.
Native unchanged benchmark second run PASS3actual workloads. THREE reviews:
1same owning decoder and payloads;2actual original binary/command unchanged,
30samples each/no data changes;3confidence intervals and outliers retained,
noise claims use repeated native samples, never simulation equivalence. No
post-change improvement claimed; second run still before any product edit.
Native unchanged benchmark third run PASS3actual workloads. THREE reviews:
1same production owner/input sizes;2actual original compiled binary, no source
change yet;3all30samples per case and full CIs/outliers retained, native noise
baseline only. Three original-run logs now recorded before the one-line hint.

PG9 native single #[inline] candidate REJECTED: all three workloads regress
significantly, e.g. large payload3.4080–3.5063ns versus baseline2.6427–2.7147ns,
Criterion+25.845%..+29.417%. THREE reviews:1same production decoder and original
three inputs/30samples;2actual one-line source hypothesis produces worse native
runtime, falsifier met;3no changed benchmark/threshold/deadline/feature graph,
full raw CIs and outliers preserved. Do not keep to chase a possibly greener
CodSpeed result. No semantic test suite claimed. Revert ONLY new attribute and
run saved original compiled binary as A/B/A control; no need three redundant
candidate campaigns after this clear falsifier.
Native A/B/A original-binary control PASS3 actual workloads; large2.5579–2.5837ns,
restoring the original baseline after candidate3.4080–3.5063ns. THREE reviews:
1actual same decoder/input owner;2saved pre-edit original executable, source
attribute reverted and git diff confirms zero production changes;3same pinned
shell/hardware/options, full CIs/outliers retained, not x86 simulation evidence.
Binary file sizes original6704128/candidate6704640 (+512bytes, includes all file
sections, not a claimed text-only size). Both runtime and file size reject hint.
No retained product/workload change; failed experiment remains documented.
Release0.4.1 actual owning benchmark PASS3workloads on same pinned M4Pro/Rust
with unchanged workload/options: CIs small2.6005–2.6195ns,medium2.5884–2.6213ns,
large2.5975–2.6178ns. THREE reviews:1actual original release8836 source and local
0.4.1 Cargo graph, locked build;2actual production source/benchmark diff versus
12fd for kernel/store/Fjall is EMPTY, all three inputs execute;3native warm ARM
is not x86 cold-cache simulation and cannot waive that gate. Criterion's printed
-21..26% 'improvement' compares its SAVED FAILED INLINE CANDIDATE baseline from
the shared target, not original0.4.0. Do NOT attribute that comparison to version
bump. Original/reversion run logs give correct direct comparison; no retained
product or benchmark change and release performance failure stays OPEN.

Release lock failure VERIFIED FIXED on real release headdaefe06097f7310ef21a596ca8ee1bb7a5049994:
actual deep-fuzz39153035060 SUCCESS including required fuzz-gate and four original
`cargo afl build --release --locked` builds followed by actual60s AFL campaigns.
Wire/header/chunk/value all PASS0crashes/timeouts, bitmap37.25/47.61/43.52/8.31%.
EACH reviewed THREE ways:1same required production target/workload/locked graph;
2original pre-fuzz stale-lock failure removed and actual native fuzzing executes;
3finite campaign, bitmap not line coverage, no dropped --locked, deadline or
crash-gate claim (separate known propagation gap remainsPG9). Four LibFuzzer61s
campaigns PASS wire6533505/header1917920/chunk1476989/value9883659 inputs. EACH
reviewed THREE ways: decoder owner; actual original fuzz assertions with no crash;
finite campaign not native Postgres or exhaustive proof. All exact log summaries
retained. This confirms the workflow fix rather than only a passing resolver.

Independent CodSpeed check117751101287 still FAIL-10.42% on repaired headdaefe,
three same decode workloads. THREE reviews:1unchanged required workload/threshold;
2actual external failed check with same reported501.7=>560ns, not dismissed by
successful AFL;3old downloaded8836 traces explain old same-cost profile, not
falsely claimed new-head traces; native ARM cannot waive x86 cold simulation.
No acceptable retained performance repair/waiver yet. PR378 remainsOPEN and no
new adapter publication is claimed. Persistent PG0–PG9 goal remainsACTIVE.

Research consolidation: backup branch rebased from12fd to actual merged87e87d6e,
all append conflicts resolved by retaining main plus exact new experiment blocks.
Performance-only rejected experiment/three original traces/native results merged
into this record/evidence; original performance worktree kept unchanged. New
restore test remains189lines and RED,0production changes. No new checks credited
for mechanical rebase/record consolidation. Named stash retained in progress.

PG7c contract selection: raw copied xid8 histories must reject before runtime use;
logical restore is completed-CBOR reimport with new positions/history and projection
rebuild. The original desired coherence row1 actual0 remains in immutable red logs.
The retained regression now requires exact typed startup rejection for SAME unsafe
copied data, not expected0 or ignored data. New history metadata is deliberately
excluded from the data-copy probe to isolate the future-xid guard; copied metadata
has its own native rejection law. SQLx0.9 supports fixed16byte arrays natively;
no UUID dependency/hand parser. History is persisted generation+native database
binding. Scope changes are an explicit documented0.x breaking minor under actual
STABILITY.md, not an unscoped compatibility fallback. All legacy numeric fields
and state/revisions stay unchanged on explicit migration, scoped under newhistory.
PG7c first all-feature library cargo check PASS5.22s. THREE reviews:1new typed
scope/checkpoint SQL library compiles;2selected Rust/SQLx array capabilities used;
3compilation-only0runtime/native proof, no complete-gate claim. Original schema
and copied-xid native regressions not yet rerun on new implementation.
PG7c first new-history native PASS1/1,1.730s. THREE reviews:1same actual unsafe
copied futurexid dataset from original global0vs1, now exact FutureTransaction
startup rejection;2original payload/sourcecounter/native context preconditions
still pass, no zero-row-success assertion or ignore;3real independent18.6
clusters/migrator5/native tools and SQLx SUT,0skips, no whole-suite/physical recovery
or foreign cursor claim. Need separate guards/counterfactuals and regressions.
PG7c native four laws PASS4/4,4.511s,0skips. EACH reviewed THREE ways:
- settled-above-xmin: stable order/open contract; actual spawn+Barrier commits B
  while native A holds older xid, second store opens before rollback; exact same
  history/one row/assigned position after release, not a relaxed xmin check.
- futurecopy: original dataset still rejects FutureTransaction; actual migrate5/
  native data-only copy and sourcecounter/payload assertions;0missing/skipped laws.
- physicalbinding: actual other-cluster systemid and other-database OID each
  reject DatabaseHistory; stored generation stays exact original bytes; no repair,
  no extra privileges inferred, no physical restore credit yet.
- foreignscope: real same-database schema recreation gives NEW generation;
  read_all and checkpoint write reject exact old/new HistoryMismatch AFTER pool
  closure; proves validation precedes acquisition, not a generic connection error.
Native fixture now uses fsync default (removed -F); setup initdb --no-sync is not
hardware power-loss evidence. No complete capacity/lifecycle/backup claim.
PG7c all-test discovery rejected at compilation E0382: one runtime-role assertion
asked a dropped store for its scope. THREE reviews:1actual native grant law not
executed;2fixture constructor adaptation error, not history-fault evidence;
3zero runtime, no flaky retry/skip credited. Use retained administrative fixture's
history for the expected record, preserving store drop and role cleanup ordering.
PG7c native five laws PASS5/5,5.904s,0skips. EACH reviewed THREE ways:
1same four actual scope/binding/future/held-xmin contracts plus physical recovery;
2original four exact typed/assigned-position assertions repeat, and REAL
pg_basebackup--wal-method=stream copies fsync-on database after application pools
close, original primary stops before clone boots, startup validates migrated
metadata, cache+checkpoint exact states/revision/position survive, new append/
exclusive global resume/CAS advance succeed;
3fresh independent18.6 processes/no installed-log replay,1.350s physical law,
not a power-loss/cross-node failover or real projection-stepper restart claim.
Those broader recovery requirements remain pending.
Strict all-target Clippy rejects one private-module redundant pub(crate) on
history::load. THREE reviews: original lint law unchanged; visibility spelling
only (module stays private, only History reexported); compilation-only0runtime,
no suppression or changed lint policy. Correct to pub within private module.
Second strict Clippy run rejects physical drill88/80lines; THREE reviews:
original function-length contract; actual diagnostic only, no runtime result;
no suppression/limit change. Reuse a private checkpoint fixture constructor for
its two identical Advance/schema fields, preserving every input and assertion.
Next strict Clippy rejects const-eligible fixture constructor and original runtime
role law expanded81/80lines. THREE reviews:1same const/function bounds;2static
source diagnostics,0runtime;3no lint relaxation. Make fixture constructor const;
extract existing actual SQL permission probes and ADD two exact42501 history
UPDATE/DELETE probes. All five original permission/state/role cleanup assertions
stay; the first209 native campaign already compiled still tests the original five
probes, and later targeted/final runs must execute seven. No new grant permits
identity mutation. Snapshot role gains SELECTevents for the new bounded startup
check; no event write/delete right added. Full runtime evidence still pending.
PG7c first complete native209/209 PASS44.268s,0skips (17.10 main fixture plus
independent18.6 restore clusters). THREE reviews:1entire adapter default/optional
contracts including original bounded reads/final hints/atomicity/outcome/state/
listing/session/upgrade laws;2actual new physical/history/future safeguards and
exact37-byte floor/53-byte edge/54-byte overflow and megabyte omission execute;
3no native early-return/filtered/cache credit. This compiled cohort has the
original five role-denial probes, not subsequently added two history denials.
One import test takes12.387s versus0.051s in recorded PG7b203/24.551s baseline.
The new5restore laws add5.9s, but elapsed total is not a throughput benchmark.
Cause of that import outlier UNPROVEN; retain/investigate, no timeout/workload
relaxation or performance-parity claim. Prior cold fixture variability alone does
not establish its cause. Final targeted/repeated native evidence still required.
Fourth strict all-target Clippy PASS5.96s. THREE reviews: original lint rules;
actual current library/all test targets compile warning-free;0runtime credit,
no default/isolated-feature or complete Nix-gate claim.
PG7c populated-v4 upgrade first run REJECTED fixture setup: native42804 state
integer[] rather than bytea from untyped [0,255,1] bind. THREE reviews:1backfill/
collision law NOT reached;2actual SQLx generic bind defaults integer element,
not product migration failure;3native2old tests pass/1fixturefails/3UNRUN,0skips,
no backfill credit. Correct only element type to u8 and retain all state bytes.
Import outlier repeat1 nativePASS1/1 11.835s,14filtered. THREE reviews:1same actual
import error/partial-state contract;2original23514 and exact WholeChunk/PerStream
state assertions pass but latency regression REPRODUCES;3not noise-dismissed or
weakened deadline, baseline0.051s remains. Investigate native query plan/activity
before keeping performance claim. No fixture delay removed to hide result.
PG7c corrected populated-v4 upgrade cohort PASS6/6,24.667s,0skips. THREE reviews:
1migration5 data adoption/rollback/backfill/reopen plus all prior upgrade laws;
2new actual4schema row has MAXrevision/schema/txid/seq and binary0/255/1 state,
foreign history table collision yields exact42P07/step5 without added column or
changed checksums/state, repair preserves all1–4checksums/foreign data and exact
scoped saved state across reopen;3actual17.10 native, no copying new metadata onto
unknown old cursor, external2scalar adoption remains explicit operator choice.
First old upgrade law takes24.424s; retain as observed timing outlier.
Import repeated cohort PASS1/1 6.531s (14filtered), then paired observer cohort
PASS1/1 0.060s (14filtered). EACH reviewed THREE ways: same original import
failure/partial commit contract; actual23514 and exact per-policy stored rows;
no source/deadline/expected-value changes, unselected14not passes. A psql observer
initially rejected empty query buffer; corrected watch missed fast episode, so
it is NOT wait-event evidence. Three actual schema query plans run under1ms
execution (history0.559,checkpoint0.406,snapshot0.281), not proof against all
client overhead. SQLx actual locked source constructs default options using
whoami/getpwuid_r BEFORE explicit URL username; possible host lookup cost, NOT
proved causal. Native Mac sample of another slow12.260s upgrade invocation
captures _dyld_start and loader debugger-notification Mach waits before Rust;
this narrows part of observed interval but sampling can perturb loader timing.
Do not attribute all latency to lookup or dismiss it as noise. Raw source/stack/
plan evidence retained; libtest body timing diagnostic pending. No gate replacement.
Diagnostic libtest runs SAME real import law PASS1/1 6.15s,14filtered. THREE
reviews: same23514/partial-state assertions; actual in-process harness timing
still slow; not substituted for nextest/Nix gate and disproves a loader-only
explanation. Need per-stage/late stack evidence. Host-user lookup hypothesis
remains unproved. Previous12.260s sampled upgrade PASS1/1 5filtered reviewed3ways:
original upgrade/checksum/history invariant; actual old law unchanged;
initial3s sample saw loader only, cannot attribute all12s or dismiss body cost.
Explicit PGUSER=joel control runs same actual import law, same DATABASE_URL
including user=joel, with no product/workload/deadline change. Actual1/1PASS,
14filtered (timing retained). THREE reviews: original error/partial-state contract;
unchanged actual database identity and SQLx graph; environment default lookup
only is candidate causal variable, not a weaker DB/user or passing threshold.
Need reverse/stack evidence before assigning root cause or changing setup.

PG7c timing diagnostic closeout: explicit PGUSER=joel libtest PASS1/1 in0.05s;
reverse unset-PGUSER libtest PASS1/1 in0.04s, each14filtered. THREE reviews EACH:
(1) unchanged actual import23514 and exact policy-dependent committed rows;
(2) neither fault workload, credentials, assertions nor source changed;
(3) reverse did NOT reproduce slowdown, so PGUSER causality remains unproved.
Late sampler found no still-running test and produced no stack: zero stack credit.
Retain timing outliers without a performance-parity claim; do not alter project
configuration or spend another diagnostic loop without a slow native witness.

PG7c followup BEFORE edit: the future-XID guard's exact equality boundary is
not yet falsified. Add <=40 native-test lines (cumulative native additions <=620,
previous580 plus40), no additional product/API/dependency budget. Set an existing
copied event to the next native transaction ID in one admin statement, assert it
equals the new snapshot xmax, then require exact FutureTransaction. A >= to >
counterfactual must fail. Keep the original128-ahead source/copy/payload oracle.
Rollback: remove only this boundary fixture, never weaken the guard or old tests.
Current strict Clippy PASS6.94s before equality helper edit. THREE reviews:
(1) all existing deny levels and80-line rule retained; (2) populated-v4 upgrade
and seven denial probes compile in all-target/all-feature scope; (3) zero runtime
credit and no current complete-gate claim. Boundary helper still needs fresh check.
PG7c equality-boundary cohort PASS5/5 native5.696s,0skips. THREE reviews:
(1) original payload/counter, physical recovery, mismatched positions and valid
held-xmin/open contracts remain; (2) copied far-future row AND exactly-xmax row
both reject FutureTransaction, with actual equality asserted before open;
(3) independent18.6 clusters, fsync-on physical recovery, no filtered/skipped
case credited and no timeout/expected result weakened. Manual falsifiers pending.
PG7c fresh equality counterfactual (separate source worktree, package-clean owned
target) native FAIL1/1 in1.634s,4unselected. THREE reviews: (1) exactly-xmax
transaction must reject; (2) >= changed only to >, original far-future/payload
assertions pass and exact boundary assertion fails at restore_tests:257;
(3) runtime failure, not build failure, timeout, skipped case or fixture poisoning.
The production guard remains >=. This is a meaningful caught boundary defect.
PG7c fresh xmin counterfactual native FAIL1/1 in0.784s,4unselected. THREE reviews:
(1) newer committed rows behind an unrelated older writer remain valid on open;
(2) replacing only the future guard's xmax with xmin rejects that valid history
with exact FutureTransaction, after real Barrier/spawn overlap; (3) native
assertion failure at restore_tests:359, no timeout/build rejection credit.
Current strict Clippy PASS9.10s. THREE reviews: (1) unchanged deny policy;
(2) all targets/all features including equality helper compile warning-free;
(3) compilation only, no extra runtime or feature-isolation credit.
PG7c fresh disabled-future counterfactual native FAIL1/1 in1.556s,4unselected.
THREE reviews: (1) independently copied committed events must not open under a
lower transaction counter; (2) bypassing only future rejection accepts the unsafe
copy and fails exact FutureTransaction after the unchanged payload/source-counter
oracles; (3) actual runtime assertion at restore_tests:242, no build/skip credit.
Production remains unchanged. Each control uses isolated private clusters, so
concurrent main-cluster full-suite verification cannot move their counters.
PG7c fresh disabled-namespace counterfactual native FAIL1/1 in0.916s,4unselected.
THREE reviews: (1) old positions after same-database schema recreation must return
exact identities before pool acquisition; (2) bypassing History::check fails the
read_all exact mismatch assertion with the caller pool already closed;
(3) current schema generation really differs, source control compiled freshly,
no guessed mismatch, skip/build-error credit or changed workload. The production
method still checks both global resumes and checkpoint writes.
PG7c current full native PASS210/210 in37.703s,0skips. THREE reviews:
(1) original203 event/notification/outcome/atomic/cache/checkpoint/listing/session
laws plus five restore laws, the original37-byte floor and populated-v4 upgrade;
(2) seven exact42501 restricted-role probes now execute, physical backup retains
scope/state and future equality is checked with real counters; (3) actual17.10
and independent18.6 clusters, no filter/skip/cached execution or whole-release
claim. Timing alone is not capacity or CodSpeed evidence.
PG7c system-binding counterfactual native FAIL1/1 in1.411s,4unselected. THREE
reviews: (1) foreign actual cluster identity must reject; (2) replacing only its
comparison with self-equality accepts it, failing exact DatabaseHistory at318;
(3) SQL source recompiled after package clean, runtime assertion not compile
failure, database binding guard and all other checks remain active.

PG7c fresh audit BEFORE edit: no law mutates a checkpoint's saved history after
open. Input mismatch is covered but does not prove metadata rejection before
stale handling/body transfer. Add <=70 new native lines, cumulative <=690
(previous620 plus70), zero product/SQL/dependency budget. Use admin fault injection
only to remove the FK and change16bytes on a real megabyte checkpoint, then
require exact HistoryMismatch for matching/stale reads and conditional writes,
unchanged complete pair, and the existing262144-byte allocation threshold.
A fresh SQL control omitting only the body history predicate must fail that bound.
This closes a read corruption/transfer gap; do not weaken original tests or limits.
PG7c database-binding counterfactual native FAIL1/1 in1.539s,4unselected. THREE
reviews: (1) another database OID must reject even on the same native cluster;
(2) bypassing only database-ID comparison leaves foreign-cluster rejection
working, then fails the second exact DatabaseHistory assertion at329;
(3) native runtime failure after fresh compile, no unviable/build/skip credit.
PG7c saved-history corruption law native PASS1/1 in4.580s,13unselected. THREE
reviews: (1) exact HistoryMismatch precedes stale-schema handling and conditional
replacement; (2) actual one-megabyte saved state, FK removed by admin only,
16-byte history mutated, both matching/stale schema reads and a CAS write reject,
complete revision/schema/position/state/history tuple stays unchanged;
(3) two read allocations remain below original262144 ceiling; no input limit,
workload or assertion weakened, unselected13not credited. Timing is not a benchmark.
PG7c fresh foreign-body SQL control native FAIL1/1 in0.057s,13unselected. THREE
reviews: (1) foreign metadata must not transfer its state body even when its
schema matches; (2) only c.history=$4 becomes $4::bytea=$4::bytea, retaining the
four native bind parameters and all other guards, exact HistoryMismatch still
passes but allocation is6289652 > original262144 at checkpoint_tests:549;
(3) actual runtime body-transfer witness, not parameter-count/protocol rejection,
no workload/limit/oracle change and no filtered-case credit. Restore the isolated
control copy; production predicate never changed.
PG7c default feature run REJECTED:11pass/1fail/110UNRUN in0.135s. THREE reviews:
(1) unchanged default no-skip law requires a valid installed schema;
(2) independent native catalog confirms checkpoints_history_fkey ABSENT and saved
history unequal to its singleton, left by the targeted admin corruption fixture;
(3) real fixture poisoning, not product acceptance failure or feature pass.
Fix fixture to collect actual read/write/unchanged-row outcomes, repair only its
injected history/FK changes, then assert all original outcomes/allocation bounds.
This preserves negative oracles and cleans even an assertion-failing control.
Do not waive validation, reset migration checksums, skip the old law or credit110
unrun cases. Initial6289652-byte control remains real, but rerun final fixture.
PG7c repaired fixture native PASS1/1 in25.357s,13unselected. THREE reviews:
(1) exact foreign metadata read/write and allocation oracles remain;
(2) independent catalog after run reports one restored history FK and all saved
history values equal to singleton; (3) source inspection finds the control copy
had already been restored to production predicate, so THIS is a positive run,
NOT a failed-to-kill mutant or final counterfactual evidence. Do not infer a
control from its log filename. Slow elapsed time retained without causal claim.
Rerun the deliberate predicate replacement explicitly, with fresh package build.
PG7c FINAL fixture SQL counterfactual native FAIL1/1 in0.131s,13unselected.
THREE reviews: (1) foreign saved metadata must reject without transferring body;
(2) inspected source explicitly replaces only c.history=$4 with bind-preserving
self-equality, exact mismatch oracles pass and actual allocation6289652 exceeds
original262144 at574; (3) fresh package build, runtime failure, independent
catalog afterward reports FK count1 and valid history=t EVEN AFTER assertion
failure. This final fixture no longer poisons subsequent tests. Production stays
unchanged; its earlier25.357s positive run is separately labeled above.
Evidence preservation correction: matrix reused the original failed default
/tmp log filename. Retained tool-transcript excerpt and independent catalog
output in evidence/pg7c-default-poisoning-original-excerpt.log are explicitly
labeled incomplete; do NOT claim the original full raw log survives. The native
failure/11pass/110unrun and catalog cause remain recorded. Future attempts use
unique names. No failed result is relabeled passing or removed from this record.
PG7c isolated default matrix cohort PASS122/122 in20.334s,0skips. THREE reviews:
(1) default append/read/subscription/outcome/migration/history contracts;
(2) both future counter boundaries, valid held-xmin open, physical binding and
foreign global resume run against native17.10/private18.6; original no-skip law
now passes after fixture repair; (3) no projection/snapshot/export/import runtime
credit, no DB-free early return/filtered-case or capacity claim. Optional API
absence still needs the separate downstream feature checks required by PG9.
Linux selected-feature smokes EACH reviewed THREE ways: all-feature archived
atomic1/1 PASS2.101s (14unselected), snapshot1/1 PASS2.157s (15unselected), real
projection1/1 PASS2.753s (2unselected), listing1/1 PASS2.073s (7unselected), session
close/drop1/1 PASS2.263s (10unselected). Pass1: correct optional capability laws;
Pass2: native18.6 actual database and owned archived source with new migration5,
fixed corruption-fixture cleanup; Pass3: these are filtered smoke executions,
NOT five feature-isolated builds and unselected cases get zero credit. Full211
native run is separate and still pending; no replay/publication claim.
Four existing fuzz replay assertions (2/1/1 cases) pass under full Nix gate.
THREE reviews EACH: original wire/CBOR/value corpus laws; actual stored seeds
replayed, not a new fuzz campaign; no AFL/deep-fuzz/native-DB or coverage credit.
Slow pre-cleanup full-gate Fjall key law eventually PASS93.098s (14store opens);
readonly host sampling was denied by process permissions, no stack evidence and
no escalation. Unchanged Fjall workload and all assertions retained. Do not
attribute timings to Postgres or claim a measured root cause/performance parity.
PG7c Linux full native REJECTED175pass/1fail/35UNRUN,51.934s,0skips. THREE reviews:
(1) SQLSTATE preservation must retain the cause actually delivered by SQLx;
(2) unchanged session law kills an IDLE backend then sends its FIRST query,
Linux reports original typed Sqlx::Io(BrokenPipe32), not Database57P01;
(3) no adapter error erasure was shown and accepting any error/waiving57P01 is
not justified. Keep exact raw Linux red, not a green retry. Make the intended
server-SQLSTATE witness deterministic: existing native lock observation must
prove query in flight BEFORE termination, then still demand exact57P01 and fused
cursor. Keep idle-death typed-I/O uncertainty as PG8 lifecycle evidence, not a
claim of server SQLSTATE availability when client writes a closed socket.
Pre-edit budget <=50 changed native fixture lines,0production/API/SQL budget,
reuse existing pg_stat_activity native Lock/3s poll pattern and same60s lifetime.
No deadline/limit/lint relaxed. Rollback only this fault-order fixture change.
Verification resource decision: cancel the obsolete pre-cleanup full Nix gate
(PID26280 verified nix flake check) and run the COMPLETE unchanged gate on current
fixed fixtures. Its1567 selected workspace tests had1557 completed with ten slow
Fjall property/state tests still running; no failure was waived and no whole-gate
pass is credited. Three reviews: (1) original workspace laws/workloads/lints
unchanged; (2) source snapshot lacks two subsequent fixture repairs so completion
would not validate the current tree; (3) explicitly canceled partial run,4compiler
cases unselected and DB-free Postgres checks are not native evidence. Keep raw
log; no timeout/sample-count/durability adjustment. Current whole gate starts
separately and normal signed-commit hook remains mandatory.
PG7c isolated export cohort PASS140/140 in246.059s,0skips. THREE reviews:
(1) default laws plus bounded listing/session capture, resources and cleanup;
(2) actual far-future/equality/native binding/foreign resume and original eleven
session laws execute; (3) this binary predates the deterministic in-flight57P01
fixture followup, so its idle-kill pass does NOT resolve the Linux red. Long
private-cluster timings19–58s retained without performance-parity claim.
PG7c isolated import cohort PASS148/148 in41.699s,0skips. THREE reviews:
(1) default plus actual atomic/whole-chunk/per-stream import contracts;
(2) native ambiguous commit, exact transaction ordering, validation-before-write,
real races and raw-restore guard laws all run; (3) export/snapshot/projection are
absent here, so no credit for their runtime laws or API-absence compilation.
Current full-gate corpus replays EACH again reviewed THREE ways (2/1/1 cases):
unchanged wire/CBOR/value seed contracts; actual existing seeds, no new fuzzing;
no native database/AFL/campaign or mutation credit. Old interrupted gate receives
zero whole-run pass credit; current normal gate is independently in progress.
PG7c isolated projection cohort PASS143/143 in24.137s,0skips. THREE reviews:
(1) default plus conditional-state/CAS/scalar/consumer/uncertainty laws;
(2) new corrupted saved history rejects matching/stale reads and CAS without
megabyte transfer, repaired FK/pair survives; populated-v4 collision/backfill,
foreign input pre-acquire and seven42501 role probes execute;
(3) no export/import/cache or actual physical-Projection restart credit, no skips,
limits/assertions unchanged. Complete delivery and capacity evidence pending.
PG7c isolated snapshot cohort PASS143/143 in23.670s,0skips. THREE reviews:
(1) default plus unconditional cache/codec/repository/scalar/uncertainty laws;
(2) history-bound startup under restricted cache LOGIN succeeds with SELECTonly
history/events and retained mutation/DDL prohibitions; original stale/oversized
body and migration/cache/event isolation assertions execute; (3) no projection,
export/import or physical checkpoint-restart credit; all original limits stay.
PG7c CURRENT Linux native PASS211/211 in51.024s,0skips, artifact
/nix/store/045sg6v2lhkvj0bd94pbyfxh9fynmic4-mnesis-postgres-integration.
THREE reviews: (1) full default and all four optional adapter contracts;
(2) current corrupted-history/FK cleanup, real physical recovery, populated-v4
rollback/backfill, and IN-FLIGHT native57P01/fused-cursor/failedclose all execute;
(3) real PostgreSQL18.6 on aarch64-linux, no retries/filtered/DB-free cases credited
as full suite; original Linux175/1/35UNRUN red retained independently. No workload,
SQLSTATE assertion, durability, deadline or limit relaxed. This is a focused
slice pass, not PG7/8/9 delivery or publication.
CURRENT Linux smokes EACH also THREE-reviewed: atomic1/1.445s(14unselected),
cache1/.394s(15), projection1/.371s(2), listing1/.197s(7), session1/.381s(10).
Pass1 original capability law; Pass2 actual current archived native source;
Pass3 filtered smokes not isolated builds, no skipped-case credit.
PG7c isolated export+import cohort PASS167/167 in32.612s,0skips. THREE reviews:
(1) default plus consistent export/listing, atomic/import and facade composition;
(2) corrected IN-FLIGHT57P01 law and fresh history/future/binding laws execute
alongside whole-chunk/per-stream validation and outcomes; (3) no projection/cache
or physical checkpoint-restart credit, original assertion/limit policy retained.
PG7c isolated export+projection PASS161/161 in27.712s,0skips. THREE reviews:
(1) default plus listing/session and conditional-state capability laws;
(2) foreign saved history, scoped CAS, populated-v4 upgrade and corrected native
in-flight57P01 all execute together; (3) no atomic import/cache/physical pair
restart credit, no skipped cases or source/limit/expected-value relaxation.
PG7c isolated export+snapshot PASS161/161 in42.642s,0skips. THREE reviews:
(1) default plus consistent export/listing and unconditional cache laws;
(2) new history startup under restricted cache LOGIN, original cache/body bounds,
session in-flight57P01 and foreign global resumes execute together;
(3) no projection/import/physical checkpoint restart credit, no skipped cases or
source/limit/expected-value relaxation. Current full Clippy is warning-free:
allworkspace/allfeatures/alltargets49.95s under unchanged deny policy; three
reviews are original strict contracts, actual current fixtures compile, zero
runtime/whole-gate credit. Full workspace test phase still running separately.
PG7c exact minimization count at frozen product source: Rust186added/84removed
(including position docs/unit changes), SQL51added/3removed, within original
300/100 and110SQL budgets. New restore file483lines + populated-v4 test~84 +
foreign-metadata test71 + floor22 ~=660, plus a few permission/import lines stays
within690new-native budget. Existing constructor callsite edits47 (source diff),
plus grants/raw SQL fixtures and the separately budgeted22-line session fix;
no lint/limit/kernel/dependency/runtime/framework/tool changes.
Next PG7 composition source inspection (NOT implementation/pass): LOCKEDminicbor
2.2.3 Write for &mut[u8] supports a fixed-capacity sink directly; no custom sink,
new dependency or minicbor public reexport needed to bound encoded chunk bytes.
Existing ChunkWriter/finish + decode_chunk still materializes ONE completed chunk.
Use small bounded chunks/lazy session enumeration in the next native experiment;
never call fs::read on an arbitrary whole backup and claim bounded resources.
PG7c isolated import+projection PASS169/169 in188.271s,0skips. THREE reviews:
(1) default/atomic/import plus conditional checkpoint and consumer laws;
(2) scoped CAS/unsigned bounds/metadata corruption/no-body transfer, exact import
outcomes/partial state, populated-v4 and restore guard laws execute together;
(3) no export/cache/physical checkpoint restart credit, no skipped cases or
weakened deadlines/limits. Long19–37s private restore and20.619s old upgrade
latencies remain observations, not performance parity or a demonstrated cause.
PG9 future tooling inspection only: the existing Nix shell does NOT provide
cargo-hack (help exits with no such command); do not imperatively install it or
claim an existing tool. Inspect locked nixpkgs package/upstream before selection.
Python remains in scripts/check-features.py and fuzz.yml AFL-version extraction;
remove via supported ecosystem facilities in a separate focused slice. No current
product/gate source change. Fixed-slice full gate and matrix continue.
PG7c isolated import+snapshot PASS169/169 in141.587s,0skips. THREE reviews:
(1) default/atomic/import plus unconditional cache/repository laws;
(2) binary/unsigned/migration/cache-event isolation and import error/partial-state
contracts execute under the same history-bound startup; (3) no projection/export
or physical checkpoint restart credit, no unselected-case/deadline/limit waiver.
Large host timing variation is retained without a performance-parity conclusion.
PG7c isolated projection+snapshot PASS166/166 in37.941s,0skips. THREE reviews:
(1) default plus independent cache/checkpoint persistence and actual consumers;
(2) new metadata mismatch/no-body transfer, scoped CAS/v4backfill, original cache
contracts, and physical backup retaining event/cache/checkpoint pairs all execute;
(3) physical pair/append/resume/CAS are not an actual restarted Projection stepper
or failover/power-loss proof; no import/export credit, no skips or relaxed oracles.
PG7c current normal Nix gate EXIT0. Workspace1567/1567PASS624.962s,14slow,
4compiler cases deliberately unselected. THREE reviews: (1) unchanged full
workspace laws,256-case Fjall random-version workload and all other properties;
(2) last properties429.759/527.870s pass with original samples/assertions, no
sampling/timeout/durability relaxation; (3) Postgres cases under this Darwin gate
are DB-free and receive ZERO native credit; compiler4not run here, not passes.
Native Linux211 remains separate. Static Clippy/fmt/docs-excluding-postgres/
no_std/wasm/hakari/deny and corpus gates pass. Nix fixup emits a Bash pipeline
SIGSEGV in audit-tmpdir.sh AFTER completed Rust tests but Nix exits0; retain raw
diagnostic and triage in PG9, no suppression and no claim of a clean packaging
audit from this message. Actual Linux native artifact fixup succeeds separately.

Dedicated public Postgres rustdoc REJECTED two PRIVATE links (builder/Inner).
THREE reviews: (1) warning-free public API docs; (2) actual -D warnings check,
no --document-private-items or allow; (3) existing source-doc links, not missing
History API or runtime failure. Fix only public prose to link public constructors
and describe shared ownership. <=8doc lines, within existing Rust budget; no
new tests or runtime changes. Fresh normal hook gate required after this fix.
PG7c isolated export+import+projection PASS188/188 in34.539s,0skips. THREE
reviews: (1) default plus atomic/import, sessions and actual projection consumers;
(2) exact scoped positions and checkpoint pairs survive import/read/consumer
composition, metadata faults and in-flight57P01 remain covered; (3) no cache or
physical checkpoint restart credit, no skips or changed workloads/assertions.
PG7c isolated export+import+snapshot PASS188/188 in35.997s,0skips. THREE reviews:
(1) default/atomic/import/session/cache/repository contracts; (2) independent
cache/event state and byte/routing/position laws execute with new history startup;
(3) no projection or physical checkpoint restart credit, no skips or relaxed bounds.
PG7c isolated export+projection+snapshot PASS184/184 in28.930s,0skips. THREE
reviews: (1) default/session/raw checkpoint/cache contracts; (2) physical-copy
coherence retains event/cache/checkpoint records, scope and subsequent progress;
(3) no atomic/import-gated actual Projection consumer or operator fencing/restart
credit, no skips or relaxed bounds. Fourteen native combinations now complete.
PG7c isolated import+projection+snapshot PASS192/192 in28.550s,0skips. THREE
reviews: (1) default/atomic/import plus cache, checkpoints and real consumers;
(2) native metadata no-body faults, physical-copy event/state/pair progress,
unsigned preservation and v4upgrade execute together; (3) no export-session
capability credit, no operator fencing/restarted Projection or publication claim.
PG7c isolated all4 PASS211/211 in32.791s,0skips. THREE reviews: (1) complete
current adapter native capability cohort; (2) all new history/future/binding/
metadata/physical-copy/v4upgrade laws and original203 contract laws execute;
(3) no skipped/filtered/DB-free case credit, no source/limit/deadline/expected-value
relaxation. ALL16native feature combinations PASS,2652 executed cases total,
0skips; these are repeated laws under configurations, not2652unique laws.
Only subsequent public-doc prose changes, no implementation changes; final
protected CI must execute exact signed head. Preserve each raw per-feature log.
Dedicated public adapter rustdoc now PASS2.36s with -Dwarnings. THREE reviews:
(1) public History/position/store docs and constructor links; (2) original two
private links fixed with prose/public links, modules/Inner remain private;
(3) compilation only, no doctest/runtime credit and no warning suppression.
Old16suite+firstLinux head behavior unchanged by four added/six removed DOC lines.
Final normal signed commit hook must run unchanged full gate on final source.
PG7c first SIGNED commit attempt REJECTED by unchanged normal hook: noStd build
syn fails native OS28 No space left on device. THREE reviews: (1) full gate still
required after doc-only fix; (2) disk exhaustion occurs before target product
code, no Rust contract/lint failure is proved; (3) no commit created, other checks
canceled and cached fuzz/deny are not fresh runtime passes. Preserve raw hook log.
Recovery uses supported `cargo clean -p mnesis-postgres` with explicit owned
manifest/shared target after ALL16native suites complete. Removes generated
package artifacts268342files/36.2GiB; free space now32GiB. No source/Git/database/
evidence/other-package deletion, no raw rm/cache purge or hook bypass. Another
Nix process is independently building Labcast; verified its cwd, left it alone.
Retry SAME signed commit and unchanged full gate; no parallelism/test/workload/
limit/lint configuration change.
# Continuation, history delivery and global-read search path — 2026-10-18

Fresh scalar-resume negative control in isolated worktree
/tmp/mnesis-path-controls.FsopcM, package-clean proof target, native control DB:
reverted ONLY the u64-sequence fallback comparison to unqualified txid >.
The unchanged resume-operator law fails0.051s at the final large-sequence oracle:
actual[] vs exact later event. THREE reviews: (1) a position with u64::MAX seq
still permits a later transaction; (2) actual SUT compiled fresh and the other
function/row/cutoff qualifications remain intact, so the independent scalar
branch is falsified; (3) the two-event positive control and ordinary row-resume
assertions pass before the final failure, cleanup precedes assertions, five
other tests are filtered, not skipped runtime cases. Production source remains
fully qualified and unchanged. Raw /tmp/mnesis-pg8-path-scalar-control.log.

Current full native all-feature suite:215/215pass42.586s,0skips; strict
all-feature/all-target adapter Clippy passes without lint changes. THREE reviews:
(1) the four default-core query regressions join the complete previous211cases;
(2) actual local Postgres17.10 plus independent18.6restore processes run; all
history/migration/cancel/uncertain-commit/atomic/cache/checkpoint/session owners
remain selected; (3) exact-output oracles and previous numeric/byte/deadline
boundaries are retained, no skip credit, Linux execution remains independently
pending. Product delta9added/8removed lines and fixture156added/4removed remain
within the pre-edit20/200budgets. No API/migration/dependency/runtime changes.

PR387 protected normal squash merged2026-10-18T20:23:02Z by joeldsouzax,
exact merge2b181b12d1040f1527d6a28375fd5db91a9d1129 from reviewed signedb2410ad.
All40checks passed and one scheduled-only sweep skipped; no review threads.
Actual final CI native suites:211/211 in40.890s,35.104s,42.118s (two workflows
and separate package/check executions, not three distinct feature matrices).
Every suite reviewed THREE ways: (1) all advertised features and full native law
selection; (2) real PostgreSQL supplied and211actual cases execute; (3) zero skips,
raw logs match reviewed revision; same laws repeated do not multiply coverage.
Each associated five one-case smokes reviewed THREE ways: owning capability
present; selected native law actually runs; all other cases filtered, no isolated
feature-build credit. Full CI workspace1567/1567 in80.641s and121.218s likewise
reviewed THREE ways: owning complete workspace gate, real nextest execution,
four compiler cases unselected and DB-free cases contribute zero native proof.
Existing per-package feature jobs retained original default/none/all unit runs;
for EACH run the three reviews are: exact package and configured feature set,
actual unit bodies execute, zero-body harnesses/compile-only checks receive no
behavior proof and no native database proof. Feature isolation source checks are
not additional runtime executions. Compile-fail jobs verify intended diagnostics.
Four original LibFuzzer campaigns completed61s each with counts6636851,
9917547,1854424,1219752 for wire,value,header,chunk. Four locked AFL campaigns
hit original time limits and had no crash artifacts. EACH campaign THREE reviews:
owning real decoder/value target; actual campaign after successful locked build;
no reduced duration/corpus/assertions, no artifact indicates observed crashes,
but this does not establish the still-pending zero-exit crash-gate fault proof.
Raw /tmp/mnesis-pr387-ci.log, -ci-call.log, -fuzz.log and mutants artifact retained.

Path slice strict Clippy first rejected three fixture issues: two shadowed names
and large returned adapter error. THREE reviews: contract/oracles unchanged;
compile-only failure means planned full native run DID NOT start; rename fixture
locals and box the concrete error solely to retain failures through cleanup,
without type erasure, product error/API edits, or lint relaxation.

Qualified-query native run: six/six pass0.270s, zero skips. THREE reviews:
(1) both original no-skip laws and four new exact-output laws are selected;
(2) actual native functions/operators remain injected while read_all runs, with
two genuinely overlapping writers and the same original-red oracles;
(3) no errors are ignored, injections are cleaned before assertion, full drain
and both exclusive-resume branches are asserted. Default positive control also
returns the exact two events. This is targeted proof, not whole-adapter delivery.

Original-source global ordering run: four native laws, four failures in0.208s;
two unrelated existing laws filtered (not runtime skips). THREE reviews:
(1) exact no-skip cutoff, full ordered output, row resume and unsigned sequence
resume remain the contract; (2) current-snapshot substitution hides both events,
xmin and cutoff-operator substitutions release the later commit while the older
transaction is held, and the resume-operator substitution produces native0A000
when refilling a row comparison; (3) each law executed actual read_all, actual
spawned append overlapping a held older transaction, positive default-pool
control and cleanup before assertions. No deadline/workload/oracle is weakened.
The resume error arises even in a full drain's second page; it is not evidence
that the scalar large-sequence branch has been independently falsified yet.
Raw /tmp/mnesis-pg8-path-original-native.log is retained. Minimal fix qualifies
the two snapshot functions and three comparisons at their existing owning query.

PR387 final downloaded mutation artifact12149346087, run39161571503, exact
headb2410ad4480ec1382fb50cc1c82b688e2b51aca3: baseline211/211native passed36.111s,
27candidates,11caught,16unviable,0survivors/timeouts. Baseline THREE reviews:
(1) unchanged complete selected feature suite; (2) native database configured in
the mutation job and bodies execute; (3) no runtime skips, source head matches,
subsequent fail-fast mutants do not receive211-case pass credit.
Every runtime mutant's FIRST failure reviewed in three distinct ways below.

| Mutant | Contract review | Actual fault/falsifier review | Oracle and scope review |
| --- | --- | --- | --- |
| History::check -> Ok | Foreign checkpoint history must reject | foreign_saved_history exact HistoryMismatch assertion fails | Fixture repairs injected history/FK before assertion;33passed/177unrun |
| History::check == -> != | Same history resumes must work | exclusive-resume law gets HistoryMismatch with identical bytes | SUT fails at open;11passed/199unrun, not foreign-history coverage |
| history::load delete ! | Valid cluster binding must open | unchanged no-skip setup gets DatabaseHistory | Startup false rejection;11passed/199unrun, not copied-binding proof |
| migrate -> Ok | Explicit migration must install schema | process-checkpoint fixture gets exact unmigrated Schema error | Native open failure;28passed/182unrun, not a crash-survival run |
| insert_one -> (0,0) | Conflicting run must reject atomically | shared conflict law unexpectedly gets Some position(0,0) | Actual append bypass detected;13passed/197unrun |
| insert_one -> (0,1) | Conflicting run must reject atomically | same shared law gets Some position(0,1) | Actual append bypass detected;13passed/197unrun |
| insert_one -> (1,0) | Conflicting run must reject atomically | same shared law gets Some position(1,0) | Actual append bypass detected;13passed/197unrun |
| insert_one -> (1,1) | Conflicting run must reject atomically | same shared law gets Some position(1,1) | Actual append bypass detected;13passed/197unrun |
| atomic_append_many -> None | Nonempty conflict cannot acknowledge | same shared conflict law gets None success | Owning atomic SUT called;13passed/197unrun |
| checkpoint lock -> None | Existing revision remains visible to CAS | aggregate/cache independence law gets Conflict expected1 actualNone | Observable revision loss;29passed/181unrun |
| commit_checkpoint -> revision1 | Acknowledged pair must persist | process law reads Absent instead of exact Found pair | Child actually exits after SIGKILL;28passed/182unrun; no power-loss claim |

All16unviable compile diagnostics independently inspected. THREE reviews for each:
(1) these replacements target typed construction, not executable behavior;
(2) Default is absent for store/history/position/stream/hydrated types, or const
as_bytes cannot call Box::new; (3) builds fail before tests, zero runtime catches.
Affected sites: builder56; history22(two),38; position34; schema39,49;
store99,121,600,685; atomic74(SomeDefault); checkpoint33,67,100(SomeDefault),124.
The native manual SQL/identity controls recorded earlier supplement these limits;
they are not replaced by a manufactured mutation percentage.

PR387 at review:40SUCCESS/1scheduledSKIPPED, mergeStateCLEAN, no review threads.
No required check bypass or release publication is authorized by a local pass.

The previous status turn was a verified wait: both identified PR387 Nix jobs
were confirmed live; it was not implementation progress. Goal service now reports
active after resumption. Independent native regression work is proceeding.

History final signed commit b2410ad4480ec1382fb50cc1c82b688e2b51aca3 opened PR387.
Normal unmodified commit gate passed on its second attempt after supported
package-cache cleanup. Workspace nextest1567/1567 passed504.323s,14slow and
four compiler cases unselected. THREE reviews: (1) complete owning workspace
gate and unchanged property workloads; (2) actual nextest bodies ran, but absent
DATABASE_URL means this Darwin gate contributes zero native Postgres proof;
(3) no deadline, durability, lint, or assertion relaxation, and cached derivations
are not fresh runtime evidence. Separate actual Linux211 and native16-feature
matrix evidence remains authoritative for database execution.
Darwin audit-tmpdir emitted a classifier crash despite Nix exit0. Exact current
rooted output has two files only: coverage text (SF:/ bytes) and target.tar.zst
(28b52ffd). No ELF/shebang candidate occurs in this output file set. This scopes
the diagnostic for this artifact only; it does not fix the upstream helper or
audit compressed contents. Linux native output fixup was clean.

Global-read characterization first attempt did not compile: SQLx0.9 raw_sql
requires static SQL here; helper accepted an unconstrained borrowed string.
THREE reviews: (1) the four laws still require exact withheld/ordered/resumed
events from real overlapping writers; (2) E0521 is fixture compilation failure,
zero native tests executed and zero product-defect credit; (3) repair only the
helper lifetime to &'static str, matching all literal callers; preserve all
oracles, injections, SUT calls and cleanup. Raw /tmp/mnesis-pg8-path-original.log
preserved separately before rerun.
# PG7 bounded chunk composition — 2026-10-18

Combined source now includes query parent19e706242412f31a8505b88c51340a4024104f2a
(PR388 OPEN) and all seven backup laws. Strictall/all ClippyPASS1.26s and full
native222/222PASS32.681s,0skips. THREE reviews: (1) complete combined owning suite
211previous+4query+7backup, all optional capabilities enabled; (2) actual local
17.10 and independent18.6restore processes execute, including blocked native
faults, both adapter interchanges and actual logical consumer restart; (3) no
filtered/ignored cases, all exact version/field/history/state/uncertainty oracles
retained, no workload/deadline/limit/lint weakening. Earlier217suite is not
misreported as current222. This is not whole PG7–PG9 delivery or capacity proof.
Actual budget574newtestlines/650, devdependency2entry edits, lock1localedge/5,
production0. README documents per-chunk atomicity, whole-inventory obligation
and file-sync ownership. Current Linux verification remains in progress.

Current projection-enabled backup source strict all-feature/all-target Clippy
PASS1.15s. Separate export,import without projection native6/6PASS2.426s,0skips.
THREE reviews: (1) all six non-projection laws remain available without the
projection capability; (2) actual distinct feature build and native bodies run,
including both blocked-query faults; (3) entire file-selected suite has zero
skips, projection law excluded at compile time, no default/all16feature claim.

Path normal SIGNED commit hook completed unchanged full Nix gate. Actual current
workspace1571/1571PASS535.364s,14slow,four compiler cases unselected. THREE reviews:
(1) full owning workspace laws include four added query regressions; (2) actual
nextest execution, but absent DATABASE_URL still means zero native database
credit from this Darwin gate; (3) unchanged property workload/deadlines/durability,
no checks skipped/bypassed. Exact current nextest artifact
/nix/store/xgdwqv92darvg56ya6r3ar1zjaqn9n2n-mnesis-nextest-0.4.0 has clean fixup
with no classifier crash in its retrieved full log, unlike older recorded outputs.
Do not generalize this to an upstream helper repair. Source artifact archives
4.28GiB to1.01GiB under unchanged gate. New signed commit19e70624; exact full head
and PR delivery will be checked before any merge claim.

Actual logical projection recovery law1/1PASS0.237s,six other backup cases
filtered. THREE reviews: (1) logical import must discard source checkpoint
provenance and permit a fresh state rebuild, saved-pair restart and later progress;
(2) existing actual Counter repository/projector saves source state4, exports two
completed bounded chunks, fresh-history imports, confirms checkpointAbsent,
rebuilds4, closes/reopens the actual pool, reloads the exact rebuilt position/state,
then appends through the real repository and advances to5; (3) source/destination
history differ, destination checkpoint retains its identity and strictly increases
after append. No mock consumer, no copied source checkpoint, no physical-recovery
or deployment RPO/RTO claim. Current574testlines fit650budget. This new law was
not part of the earlier217-case full run; combined final suite is still required.

Six-law source full native suite217/217PASS135.905s,0skips, strictall/all
ClippyPASS. THREE reviews: (1) owning entire selected feature suite including
new six backup laws and previous211; (2) native17.10 plus independent18.6restore
processes actually execute; (3) source predates the next consumer-restart test,
no capacity-parity claim from this slower concurrent-build run, no deadline/
workload/oracle reduction. Parent path's four tests are on its separate branch,
so combined acceptance will require222 after the additional seventh backup law.
Before adding real logical projection restart, test-only budget550->650.

Strict Clippy rejected one fixture pattern shadowing the expected-field map with
an error's expected-history field. THREE reviews: exact HistoryMismatch bytes
must still match; this is lint-only failure and full suite did not run; rename
the pattern binding to current without changing assertions or lint policy.

Current six native backup laws6/6PASS15.842s,0skips. THREE reviews:
(1) retain all four exact roundtrip/empty/truncation laws, plus incomplete chunk
on source failure and cancellation; (2) both new faults use actual session
cursors, actual ACCESS EXCLUSIVE contention and pg_stat_activity Lock proof
before terminating the in-flight backend or dropping the pending drain;
(3) failure retains native57P01, both writers reject finish with exact Poisoned,
and cancellation releases the actual backend view before its60slifetime. The
independent blocker rolls back before final assertions. No synthetic failing
stream substitutes for the adapter. Current477testlines fit revised550budget.
Raw /tmp/mnesis-pg7d-backup-six.log retained; full owning suite/strict Clippy,
inventory/missing-file acceptance, mapping/final-conflict recovery and operator/
Projection restart evidence remain. Release PR378 currenthead65ddfc0cc1a6024011de18cb62260830b85a3963
again has actual CodSpeed FAILURE; no publication or waiver.

Previous path slice Linux native integration completed:215/215PASS52.938s,
0skips; /nix/store/qb2rhvdhvb2qkh8hslg4v1r4mrk51v4m-mnesis-postgres-integration.
THREE reviews: (1) complete selected all-feature suite includes all four new
query laws and previous history/atomic/checkpoint/session tests; (2) independent
native18.6server and current adapter archive actually execute on aarch64-linux;
(3) zero runtime skips, exact source queries qualified, no deadline/workload/
assertion changes, clean output fixup. Each of five one-case feature smokes also
reviewed THREE ways: intended capability law, actual native execution, other
cases filtered and no independent-feature-build credit. Parent path signed
commit's unchanged full Nix hook remains running; no delivery claim yet.

Corrected native first four laws4/4PASS8.001s,0skips. THREE reviews:
(1) exact62-event field maps,39completed bounded chunks and both import policies,
binary IDs, schema1/MAX and metadata absence/binary are compared;
(2) actual Postgres->Postgres new-history restore and Postgres->Fjall(close/reopen)
->Postgres execute, plus every truncation of a complete single-event artifact;
(3) backup lists lazily, uses two-event batches and one16KiB fixed sink with4KiB
source row bound; restore reads <=16KiB plus one reject byte. Assertion maps
materialize only the62-event fixture, not the backup path. This is not capacity,
operator recovery-point/time, missing-whole-chunk inventory or projection restart
proof. Empty backup has one completed artifact, zero sections, and zero writes.
Raw /tmp/mnesis-pg7d-backup-native-second.log retained. Before adding two actual
blocked-query fault cases, test-only budget refined500->550lines; no product edits.

First actual native run:4laws in2.730s,2pass/2fail/0skips. THREE reviews:
(1) empty completed backup and every truncated prefix correctly reject/restore
through the actual adapters; (2) both populated cases stop at PendingEnvelope
construction with exact Value(MetadataEmpty), before export/import, so these
are invalid-fixture failures, not adapter defects; (3) the owning Metadata type
forbids Some(empty), not a supported serialized state. Remove only invalid empty
metadata generation, retain absent/nonempty binary metadata, both schema
extremes,3000-byte payloads, fixed bounds, exact field comparisons and both
atomicity policies. Original native red remains /tmp/mnesis-pg7d-backup-native-first.log.

Pre-edit budget is in HYPOTHESES:500test lines,2existing-workspace dev-dependency
entries/feature edits,5lock-edge lines,0production changes expected. Reuses locked
minicbor2.2.3 and Fjall3.1.8, existing MIT/Apache workspace adapters and CBOR box;
no new production dependency or crate-version selection. Root lock changes only
one local mnesis-fjall dev edge. First compile rejected a mistaken fixture call
to SchemaVersion::new: this constructor takes NonZeroU32 and is infallible.
THREE reviews: (1) all serialized fields and version continuations remain exact;
(2) this is fixture E0308/E0599, zero executed native laws and zero product-defect
credit; (3) use the owning constructor, keep every oracle, and broaden fixture
fields before the first native run to initial/MAX schemas, absent/empty/binary
metadata and both four-byte and3000-byte payloads under4096-byte row bounds.
Raw /tmp/mnesis-pg7d-backup-first.log retained, not overwritten.
PR388 final required gates independently downloaded: workspace1571/1571PASS
121.690s/124.201s,4slow/4compiler cases filtered in each. EACH THREE reviews:
owning workspace laws; actual nextest bodies execute; database-free workspace
passes supply no native database credit and preserve all assertions/workloads.
The same jobs actually build native integration215/215PASS44.012s/44.042s,
zero skips. EACH THREE reviews: full advertised suite; fresh native derivation
and real database; exact query source and no deadline/assertion reduction.
Each of ten one-case native feature smokes also reviewed THREE ways: selected
capability contract; native body executes; remaining tests filtered, no isolated
feature-build claim. Each of eight bounded corpus-replay invocations (2/1/1 cases
per job) reviewed THREE ways: owning decoder/value boundary; actual replay executes;
one-second bounded replay, not a deep-fuzz or database proof.
Downloaded /tmp/mnesis-pr388-flake-ci.log and -flake-call.log retained.

PR388 deep-fuzz39164331910: wire/header/chunk/value LibFuzzer campaigns each61s,
6,692,894/3,297,859/1,633,785/5,901,327 executions, no observed failure. EACH THREE
reviews: actual owning decoder or value contract; instrumented real production
target executes; unchanged4096 input cap and61s duration, finite campaign only.
Corresponding four AFL++ campaigns each60s, zero saved crashes/timeouts. EACH
THREE reviews: production owning target; actual instrumented campaign and locked
build succeed; finite coverage and existing crash gate limitations remain PG9
work, no exhaustive absence claim. Raw /tmp/mnesis-pr388-fuzz.log retained.
GraphQL reviewThreads empty; all40checksSUCCESS/one scheduled sweepSKIPPED at
signed19e706242412f31a8505b88c51340a4024104f2a. Reviewed mutant artifact remains
zero runtime catches, one compile-unviable candidate; original native controls
supply query falsification. Normal protected exact-head merge is now authorized.

Backup slice normal signed hook completed0 at470d2ca229207fb97b1c59300c74075bb83c7f22,
signatureG; workspace1571/1571PASS528.503s,14slow/4compiler cases filtered.
THREE reviews: owning workspace and new backup test compilation; actual fresh
nextest runs; absent DATABASE_URL means zero native proof, unchanged gate and no
fixup crash in retrieved log. Independent Linux actual222/222PASS69.380s,0skips,
artifact /nix/store/2wkr7bb03mnvlx4888rwxgqv4dgxb5gk-mnesis-postgres-integration.
THREE reviews: complete seven-backup-law source plus four query laws and prior
capabilities; independent native Linux server actually executes; zero skipped
tests, no weakened data/limits/deadlines. Each of five one-case native smokes
reviewed THREE ways: selected capability; actual body; other cases filtered and
no independent feature-build credit. Raw /tmp/mnesis-pg7d-hook-final.log and
/tmp/mnesis-pg7d-backup-combined-linux.log retained. Source is frozen pending
parent protected merge and source-equivalent child rebase, not full delivery.


PR388 protected squash merged2026-10-18T20:59:59Z at
651aabf35e94b7b0e98e9e87eff19f6964df95f3 from reviewed signed19e70624.
All40checksSUCCESS, one scheduled sweepSKIPPED, reviewThreads empty. No admin
or performance waiver. Child backup rebased only above19e onto that squash;
new signed00c5f67d2e58dc1766e8d1b946cdd5902d6ed4fa has identical adapter/core/
manifest/lock/Nix/CI source to470d2ca2 (git diff --exit-code). Documentation now
records final delivery evidence. Normal unchanged signed hook remains required
for amended reviewable commit; no fresh native run claimed from a source-equivalent
rebase or cached gate. PG7/PG8/PG9 and publication remain incomplete.

Source-equivalent signed amendment unchanged Nix gate completed0 using existing
outputs, not a new test execution. THREE reviews: all owning gate obligations
remain selected; reused derivations have the previously recorded actual native/
workspace evidence; cache reuse is not additional runtime coverage. Checked-in
full-gate/Linux logs normalize terminal escapes, carriage-return progress and
trailing whitespace only; original raw files above remain untouched. Initial
whitespace review found raw terminal whitespace; repaired before PR publication.
# PG7 physical operator recovery — 2026-10-18

Current explicit-plain physical subset3/3PASS3.254s,4other laws filtered;
strictall/all ClippyPASS0.69s. THREE reviews: (1) backup verification, actual
stopped source, saved migration/history/pair and later consumer progress remain
the required contracts; (2) native18.6 now emits exactly checksum-mismatch and
manifest-present-but-not-on-disk diagnostics, both exit1; restoring the original
PG_VERSION makes verification succeed before boot; (3) no tar autodetection,
checksum/WAL skip, weaker malformed input or missing-file credit from the earlier
wrong-reason green. All three current physical laws execute, not the other four.
One actual fixture sample: backup210.22575ms, stopped-primary-to-reloaded-consumer
136.721917ms; source consumer4 resumes and advances5 in the same history with a
strictly later position. These are drill measurements, not deployment targets.
Raw /tmp/mnesis-pg7e-physical-plain-current.log retained alongside both rejected
oracles. Current full combined native/Linux and unchanged signed gate still due.

PR388 final required gates independently downloaded: workspace1571/1571PASS
121.690s/124.201s,4slow/4compiler cases filtered in each. EACH THREE reviews:
owning workspace laws; actual nextest bodies execute; database-free workspace
passes supply no native database credit and preserve all assertions/workloads.
The same jobs actually build native integration215/215PASS44.012s/44.042s,
zero skips. EACH THREE reviews: full advertised suite; fresh native derivation
and real database; exact query source and no deadline/assertion reduction.
Each of ten one-case native feature smokes also reviewed THREE ways: selected
capability contract; native body executes; remaining tests filtered, no isolated
feature-build claim. Each of eight bounded corpus-replay invocations (2/1/1 cases
per job) reviewed THREE ways: owning decoder/value boundary; actual replay executes;
one-second bounded replay, not a deep-fuzz or database proof.
Downloaded /tmp/mnesis-pr388-flake-ci.log and -flake-call.log retained.

PR388 deep-fuzz39164331910: wire/header/chunk/value LibFuzzer campaigns each61s,
6,692,894/3,297,859/1,633,785/5,901,327 executions, no observed failure. EACH THREE
reviews: actual owning decoder or value contract; instrumented real production
target executes; unchanged4096 input cap and61s duration, finite campaign only.
Corresponding four AFL++ campaigns each60s, zero saved crashes/timeouts. EACH
THREE reviews: production owning target; actual instrumented campaign and locked
build succeed; finite coverage and existing crash gate limitations remain PG9
work, no exhaustive absence claim. Raw /tmp/mnesis-pr388-fuzz.log retained.
GraphQL reviewThreads empty; all40checksSUCCESS/one scheduled sweepSKIPPED at
signed19e706242412f31a8505b88c51340a4024104f2a. Reviewed mutant artifact remains
zero runtime catches, one compile-unviable candidate; original native controls
supply query falsification. Normal protected exact-head merge is now authorized.

Backup slice normal signed hook completed0 at470d2ca229207fb97b1c59300c74075bb83c7f22,
signatureG; workspace1571/1571PASS528.503s,14slow/4compiler cases filtered.
THREE reviews: owning workspace and new backup test compilation; actual fresh
nextest runs; absent DATABASE_URL means zero native proof, unchanged gate and no
fixup crash in retrieved log. Independent Linux actual222/222PASS69.380s,0skips,
artifact /nix/store/2wkr7bb03mnvlx4888rwxgqv4dgxb5gk-mnesis-postgres-integration.
THREE reviews: complete seven-backup-law source plus four query laws and prior
capabilities; independent native Linux server actually executes; zero skipped
tests, no weakened data/limits/deadlines. Each of five one-case native smokes
reviewed THREE ways: selected capability; actual body; other cases filtered and
no independent feature-build credit. Raw /tmp/mnesis-pg7d-hook-final.log and
/tmp/mnesis-pg7d-backup-combined-linux.log retained. Source is frozen pending
parent protected merge and source-equivalent child rebase, not full delivery.

First explicit-plain run correctly reached missing-file reporting, but an
incorrect expected diagnostic added the word "file".1pass/1fail/1unrun in10.752s,
four unrelated laws filtered. THREE reviews: original complete physical pair
passes; actual missing PG_VERSION is now reported by manifest checking rather
than tar detection; the exact expected text must follow pinned REL_18_6
pg_verifybackup.c:979–981 (no leading "file"). Correct that literal only, retain
all malformed input/status/repair assertions, and rerun owning physical subset.
Raw /tmp/mnesis-pg7e-physical-plain.log kept, no successful-current-run credit.

Verified-physical subset3/3PASS27.886s,4other laws filtered; strictall/all Clippy
PASS0.70s. THREE reviews: (1) actual saved pair/consumer/checksums/explicit old
primary stop remain required; (2) native verifier actually executes, recovery
sample backup3.180546625s and stopped-primary-to-consumer136.978875ms;
(3) REJECT missing-file coverage credit: deleting PG_VERSION made verifier
auto-detect tar and fail with pg_waldump-cannot-read-tar, so exit1 alone did not
prove the intended missing-file path. Keep original raw green, select supported
explicit plain format matching pg_basebackup and require exact native checksum
and missing-file diagnostics as well as exit1/repaired-success. No skip flags,
weaker input, lint change or pass credit for the wrong error. Raw
/tmp/mnesis-pg7e-physical-verified.log retained before current rerun.

Strict Clippy rejected println diagnostics (print_stdout); the planned verified
native run did not start. THREE reviews: all behavioral oracles unchanged;
compile-only failure grants zero native proof; use the repository's existing
fallible Write/stderr diagnostic pattern and handle its I/O error, retaining
timing evidence without any lint allow or policy change.

First six native restore laws6/6PASS180.715s,0skips. THREE reviews: (1) retain
all five original history/restore laws plus actual physical projection consumer;
(2) independent native18.6source/copy processes use fsync=on and synchronous_commit=on,
waited shutdown0 then actual status3, exact history/checksums for migrations1–5,
real saved consumer4 reload and later repository append/consumer5; (3) zero skips,
no datatype/byte/deadline/durability weakening, one fixture sample backup7.462912333s
and stopped-primary-to-reloaded-projection509.218042ms is not a deployment RPO/RTO
target. Old source stops before copy boots; no cross-node fencing or power-loss
claim. Other laws are slower during concurrent builds; no equal-performance claim.
Raw /tmp/mnesis-pg7e-physical-first.log retained. Before adding supported native
manifest/WAL integrity and corruption/missing-file case, new-test budget180->220.

Previous goal turn classified PROGRESS: protected history merge, query regression
fix/PR388 and seven actual bounded-backup/recovery laws with222native checks.
Current two inherited handles55320(backup signed-hook) and62488(Linux) were
re-polled live; pending CI does not block independent operator acceptance work.

Fresh stopped-primary negative control in /tmp/mnesis-recovery-control.5Pvmnv,
package-clean proof target, swaps only waited stop for successful pg_ctl--version.
Native selected drill fails51.854s at exact status assertion:0running vs3stopped,
before target startup. THREE reviews: (1) old primary must be stopped before the
copy can serve; (2) real native source remains live and pg_ctl reports its actual
PID and data directory, so this is a runtime falsifier, not a mock/compile error;
(3) five other laws filtered, no full-suite credit, fallback Drop cleans the
source on failure, production fixture still calls the real waited stop. Original
red /tmp/mnesis-pg7e-skipped-stop-control.log retained. No fencing/failover claim.

PR388 actual mutation artifact12149868735 reviewed:215baselinePASS44.492s/0skips,
one compile-unviable read_all Default substitution, ZERO runtime catches/survivors/
timeouts. THREE reviews of baseline: full selected suite; actual native bodies;
matching source and zero skips, no independent matrix claim. THREE reviews of
mutant: scan contract; E0277 before execution; typed stream cannot Default,
therefore no runtime-falsifier credit. Automatic tool never substitutes SQL
strings; actual four original reds and scalar native control remain necessary.
No review threads at GraphQL check. Downloaded native CI jobs independently
show BUILDINGintegration and215/215PASS39.718s/43.661s,0skips. EACH reviewed
THREE ways: complete advertised capability suite; real fresh native derivation
build rather than cached log alone; no filtered cases or relaxed assertions.
Each accompanying five one-case native smokes reviewed THREE ways: intended
capability selected; actual body runs; others filtered, no isolated-build credit.
Normal CI full gates still pending, so no merge claim yet.

Default-feature strict Clippy fails only unused Write/stderr imports in the new
physical test owner. THREE reviews: all-feature physical behaviors and exact
native oracles remain unchanged; this compile-only failure runs zero tests and
catches missing feature scoping; gate those two diagnostic imports with the same
projection+snapshot condition as their only callers, retaining warnings-as-errors
and every feature boundary. No lint allow or dependency change is permitted.
Raw /tmp/mnesis-pg7e-default-clippy-first.log retained. Combined native224 source
is already compiled/running; do not restart it or claim it includes a later edit.

Combined native224/224PASS35.764s,0skips; strictall/all ClippyPASS2.07s before
feature-import-only correction. THREE reviews: (1) all prior211 laws, four query,
seven backup and two new physical laws execute with the shared real consumer;
(2) local17.10 plus independent18.6 physical clusters actually run, exact corrupt/
missing-file errors and stopped-primary status3 are asserted; (3) zero skipped
or filtered cases, no changed workload/deadline/oracle. One physical fixture
sample backup186.344041ms, stopped-primary-to-consumer143.636667ms, exact4->5
progress/samehistory/laterposition. Not production RPO/RTO or failover proof.
Raw /tmp/mnesis-pg7e-combined-native.log retained; later import-only cfg repair
requires strict feature checks and current Linux/source gate, not mislabeling
this older compiled source as a second fresh execution.

After the import-only cfg repair, strict warnings-as-errors Clippy passes default,
export+import+projection without snapshot, and all features, all targets. EACH
THREE reviews: the selected compile boundary keeps physical diagnostics scoped;
actual compiler/linter runs against the final shared-helper source; no native
execution or independent dependency-unification claim, no lint policy relaxation.
Raw /tmp/mnesis-pg7e-features-clippy.log retained. Both logical and physical
consumers now call one private helper; each retains exact saved4/reload4/advance5
oracles. Rebase saved stashfd00ca76deae61cf2550415c57c0ea37dd71c6f4 is preserved;
verified unchanged base prefixes before concatenating complete parent+child
append-only records. No user root research edit touched.

Physical slice signed d4a10c175cf082eeb9eb43429e3cdf1baf5d3d1b, signatureG,
normal unchanged full Nix hook0. Actual workspace1571/1571PASS532.082s,14slow,
4compiler cases filtered, clean fixup. THREE reviews: owning full workspace and
current feature source compile; actual nextest execution; no DATABASE_URL means
zero native credit, no weakened gate/workload/oracles. Independent current Linux
224/224PASS85.858s,0skips, current source includes cfg import repair. THREE reviews:
complete selected feature suite including both actual consumers; native fresh
18.6 integration derivation executes, exact errors and stop status assertions;
zero skips/filtered cases, no timing/limit/assertion weakening. Five one-case
native smokes EACH THREE-reviewed: intended capability; actual body executes;
other tests filtered, no independent feature-build claim. Linux source/archive
and normal local gate both completed0, not an observation timeout/restart.
Raw /tmp/mnesis-pg7e-combined-linux.log and /tmp/mnesis-pg7e-hook-final.log retained.

PR389 final reviewed signedheadd30129c3d24d543bd773ae3bbd29252b5f38aeb6:
all40checksSUCCESS/one scheduledsweepSKIPPED, GraphQLreviewThreads empty.
Actual mutants-in-diff39165938664 reports No mutants to filter; upload path has
no files and artifactsAPI returnsnone. THREE reviews: test-only/dev-feature diff
has no changed production expression; zero mutant/baseline tests executed;
zero runtime catches and no fabricated artifact/percentage. Original retained
native backup/source/cancellation proofs remain the behavioral evidence.

Actual CI39165938658 and reusable39165934416 downloaded whole logs. Workspace
1571/1571PASS124.926s/121.481s,4slow/4compiler cases filtered EACH THREE-reviewed:
owning workspace laws; actual nextest executes; DB-free pass does not prove native
bodies. Native222/222PASS45.541s/46.347s/47.499s/47.677s,0skips EACH THREE-reviewed:
complete selected capability/backup suite; actual building native derivations and
bodies; no weakened source, limits, timing or assertions. Repeated final-summary
lines do not count as additional runs. Twenty one-case native smokes EACH reviewed
THREE ways: selected capability contract; actual native execution; other cases
filtered, no isolated feature-build credit. Six replay invocations EACH THREE
reviews: owning decoder/value invariants; actual2/1/1 bounded case replay perjob;
one-second replay is not a deep fuzz/database proof. Each job's three standalone
compiler-fixture runs (2/1/1 cases) EACH reviewed THREE ways: expected compile
accept/reject contract; actual compiler fixture runs; compiler diagnostics only.

Each feature-job run (three each for core15,store217,wake17,Fjall131,inmemory25/30,
macros1,store-testing0,wake-nostd0 in both workflows) THREE-reviewed: owning selected
feature behavior; actual compiler/test bodies and zero-case cases distinguished;
these exclude the missing independent Postgres feature CI required by PG9 and
supply no native proof. Actual logs /tmp/mnesis-pr389-ci.log and -call.log retained.
Fuzz39165938688 wire/header/chunk/value LibFuzzer runs EACH61s,
12,072,578/2,526,624/2,312,309/5,775,359 iterations. EACH THREE reviews: owning
production boundary; actual instrumented execution; unchanged4096byte cap and
finite61s, not exhaustive proof. Four corresponding AFL++ campaigns EACH60s,
zero savedcrashes/timeouts; EACH THREE reviews: real selected target; instrumented
locked build+campaign execute; finite coverage/current crash-gate limitations
remain PG9 work, no stronger guarantee. Raw /tmp/mnesis-pr389-fuzz.log retained.
Normal protected exact-head squash is now authorized; publication still gated.

PR389 normal protected squash merged2026-10-18T23:09:09Z at
49b32374041843a87980fc6f74a945f51f0b9ea3, all40checksSUCCESS and zero unresolved
threads from signedd30129c3. Physical child rebase ONLY above d301 onto49b yields
signed56007a30537d283a9276600ed3b79760aafbff5d; git diff --exit-code confirms
adapter/core/manifests/locks/Nix/CI identical to reviewed f431999b. No native rerun
or whole-goal delivery claimed by rebase. Final source-equivalent doc amendment
uses unchanged gate, cached outputs are not additional runtime execution.

# PG7 completed chunk routes and failures — 2026-10-18

First strictall/all compile fails two shadow-unrelated bindings in final-conflict
fixture. THREE reviews: exact final mismatch and unchanged-or-earlier policy
state remain the contract; Clippy fails before any native run, zero product-defect
credit; rename only expected-pattern to next and local key bytes to id, retaining
strict lints, actual import calls and every oracle. Before execution source review
also removed a redundant final assertion that assigned the actual destination
back to its own expected variable; independent original/source/seed maps remain
all oracles. No pass credit from self-comparison. Original compile red retained
/tmp/mnesis-pg7f-routes-first.log.

First native routes run fails before SUT assertions:0passed/1failed/4unrun,
6other cases filtered,0.163s. THREE reviews: completed-chunk route/conflict/view
oracles remain unexecuted; actual fixture migrate raises native58P01 cannot load
$libdir/plpgsql, no adapter-defect credit; process13259 still runs exact17.10
binary but its /nix/store/k5mjkij9d9fcikm1pshcads63fsyf08r-postgresql-17.10
package is absent and Nix reports invalidpath. Restore exact supported package
with a Nix root before rerun; do not alter migrations, bypass schema validation,
weaken assertions, silently switch server major or restart solely on observation
timeout. Raw /tmp/mnesis-pg7f-routes-current.log retained before next attempt.
Strictall/all Clippy passed0.82s first. This is environment repair, not productfix.

Exact17.10 package restoration via nix-store --realise --add-root completed0,
fetching original binary and lib outputs,7.3MiB. New explicit package root
/tmp/mnesis-postgres-pg0-package preserves the running server's tool/library
closure. No server restart, major-version/data change or migration edit. Actual
Postgres version/native PLpgSQL probe retained separately; next native route run
must establish behavioral evidence from unchanged compiled test source.

After exact package repair and successful native version/PLpgSQL probe, routes
run0pass/1fail/4unrun0.539s,6other cases filtered. THREE reviews: final-conflict
WholeChunk branch and unchanged target pass before PerStream report assertion;
actual importer returns Mismatch reachedNone,got1 while target already has1;
resolve against owner before changing any oracle. crates/store/tests/
import_inline_tests.rs::per_stream_partial_overlap_is_picky_rejected and existing
Postgres atomic_tests explicitly pin reachedNone for a section that committed
nothing. StreamOutcome docs also say None=untouched, but introductory "where the
stream sits" is misleading: this is this import request's committed progress,
not a current target-head read (concurrent writers can advance it). Preserve the
owner's established None report and independently assert exact existing seed and
all newly imported fields. Do not return source versions as target progress or
claim that None means an empty target. Original mismatch red retained at
/tmp/mnesis-pg7f-routes-native.log. This is a fixture/wording correction, not a
product fix or a passing original defect witness. Native probe reviews: exact
17.10 target; actual DO executes the restored library; no restart/schema bypass,
zero backup-law proof from that probe alone.

Corrected owning-contract source5/5nativePASS2.982s,6other backup laws filtered,
strictall/all ClippyPASS4.14s. THREE reviews: (1) four new completed-chunk laws plus
existing empty law preserve exact routes/fields/late conflict and one sessionview;
(2) actual native binary targets, both policies, fresh+nonempty late duplicate,
Barrier/spawn atomic writer commits before later session reads, actual source
advances3/3/3 while restored source view remains2/2/2; (3) report reachedNone is
established request-progress semantics, exact independent seed and prior source
maps verify physical target state separately. No self-derived expected map,
no skip/deadline/lint reduction, filtered subset is not the full suite. Raw
/tmp/mnesis-pg7f-routes-established.log retained. Need fault controls, missing/
oversize inventory, malformed/corrupt salvage, combined owning/native/Linux/gate.

Expanded backup compile-only run rejected one108line salvage fixture against
unchanged80line policy. THREE reviews: exact parser provenance and native import
outcomes remain required; Clippy stops before native execution, zero test credit;
move pure damage/decode/provenance checks into a private typed SalvagedChunk
helper, keep actual native import workflow in its test, no allow/lint/policy
change or duplicate decoder implementation. Raw /tmp/mnesis-pg7f-backup-all-first.log
retained. Original planned new650/45helper/doc15 budgets remain unchanged.

Helper extraction's compile run fails needless-borrow at decode_chunk(&bytes),
where bytes is now a slice. THREE reviews: exact two-block positive control stays;
compile-only/zero native runs; remove that redundant reference only, no policy
or oracle change. Raw /tmp/mnesis-pg7f-backup-all-second.log retained.

Current completed-backup native13/13PASS19.973s,0skips; strictall/all ClippyPASS
0.68s. THREE reviews: (1) all seven earlier plus six new laws preserve exact event
fields, real route evaluation and policy progress, finite snapshot view and
explicit damage recovery; (2) actual native final-file absence/size overflow
reject before any writes under both policies, exact NotFound/FileTooLarge;
repair restores all6events; same-length tail corruption yields exact completion
Checksum and [Event1,Corrupt], deliberately chosen partial import reports Corrupt
reached1 and exact original prefix; malformed headings reject both decoders and
torn completion retains Missing plus producer identity; (3) zero skips, immutable
owned file inventory explicitly required, one bounded file per preflight/import
pass, no whole-backup materialization/atomicity/powerloss/authentication claim.
Actual default-source view advances3/3/3 under Barrier/spawn atomic commit while
completed CBOR restores2/2/2. Raw /tmp/mnesis-pg7f-backup-all-current.log retained.
Actual additions503/removals23 fit650new/45helper edit budgets; core docs10+/8-
clarify existing progress contract only, no executable core change.

Fresh isolated session-composition control actualFAIL0.108s,1case/12filtered.
THREE reviews: (1) same-view completed CBOR must exclude the overlapping atomic
commit from every stream; (2) replacing ONLY later session exports with supported
ordinary EventExporter compiles and restores exact2/3/3 instead of source oracle
2/2/2 after actual spawned writer commits3/3/3; (3) runtime map assertion rejects
mixed native view, not setup/timeout/compile error; correct source still uses the
session. No fabricated mutator catch. Raw /tmp/mnesis-pg7f-independent-export-control.log
and exact control source /tmp/mnesis-pg7f-independent-export-control.rs retained.

Second fresh isolated control actualFAIL0.093s,1case/12filtered. THREE reviews:
independent caller inventory must reject a missing final file before any import;
remove ONLY preflight loop, retain ordinary imports, and native target contains
exact earlier two streams rather than empty after NotFound; runtime exact-map
oracle rejects actual early writes, not a compiler/environment/timeout error.
This falsifies the caller's tested restore procedure, not a new adapter file API.
Controlworktree /tmp/mnesis-backup-controls.9MH4i5 intentionally keeps this mutant;
raw /tmp/mnesis-pg7f-no-inventory-control.log retained. Both controls terminal;
no concurrent local native suite was started against the shared physicalcluster.

Owned WIP saved by exact stash5a7ca8a2f2f4374ef732ed69228c3a500efa9b81 and
reapplied after rebase onto physicalPR390signed5b717bde. Verified exact unchanged
base prefixes before preserving full parent+child research/HYPOTHESES additions;
backup import-only conflict keeps new typed imports and parent's shared consumer,
never revives private duplicate. Logical restore still awaits fallible helper
and reuses consumer::resume. Need current combined230-suite proof, not combining
counts from separate source revisions. User root research edit remains untouched.

Current combined source now includes physicalPR390 and all six new chunk laws.
Full native230/230PASS42.979s,0skips. THREE reviews: complete selected capability
suite (224parent+6new); actual local17.10 plus independent18.6 physical drills,
shared logical/physical consumer and exact new native failures execute; zero
filtered/ignored cases and no changed bytes/limits/deadlines/oracles. Separate
export+import without projection12/12PASS12.571s,0skips. THREE reviews: all12
non-projection backup laws remain available; actual isolated feature compilation
and native test bodies execute; projection law excluded at compile-time, not
misreported as an all-feature suite. Strict default/EXIM/allfeature all-target
ClippyPASS4.35/2.60/1.79s EACH THREE-reviewed: selected feature surface; actual
compiler/linter run; no native proof or weakened lint policy. Final matrix57179
terminal0. Raw /tmp/mnesis-pg7f-*-native.log and *-clippy.log retained.

PR390 final actual evidence reviewed at signed5b717bde9813ed776efcd23394fecc88ae4b7875:
all40checksSUCCESS/one scheduledsweepSKIPPED, GraphQLreviewThreads empty.
Mutants-in-diff39175498203 actual log No mutants to filter, no upload files and
artifactsAPI empty. THREE reviews: no changed production expression; zero baseline/
mutant tests execute; zero runtime catches/percentage, original native control
remains the stopped-primary falsifier. No imaginary artifact or mutation score.

Actual downloaded fullCI39175498308/reusable39175495005: workspace1571/1571PASS
84.505s/121.933s,2/4slow/4compiler cases filtered. EACH THREE reviews: owning
workspace laws; actual nextest runs; absentDATABASE_URL is no native proof.
Four native224/224PASS41.841/42.403/44.291/48.779s,0skips EACH THREE reviews:
complete current physical+logical/previous suite; actual native builds/bodies;
zero skips/filtered cases and unchanged durability/deadline/oracles. Duplicate
summary echoes are not extra runs. Twenty one-case native smokes EACH THREE
reviews: selected capability; real native body; other cases filtered/no independent
feature-build credit. Six bounded replay invocations (2/1/1 per job) EACH THREE
reviews: owning decoder/value law; actual replay; one-second corpus run only.
Six compiler-fixture invocations (2/1/1 per job) EACH THREE reviews: compile accept/
reject contract; actual fixture compiler; diagnostics only, no database credit.
Each of three feature runs per core15/store217/wake17/Fjall131/inmemory25/30/
macros1/store-testing0/wake-nostd0 in both jobs THREE-reviewed: selected owner;
actual bodies or explicitly zero-case compile; no native/omittedPostgresfeature
CI credit. Raw /tmp/mnesis-pr390-ci.log and -call.log retained.

Fuzz39175498286 actual wire/header/chunk/value LibFuzzer EACH61s:
6,635,619/2,910,224/1,306,205/5,774,047 executions without observed failure. EACH
THREE reviews: real owning production target; instrumented runs; unchanged4096
input cap/61s finite campaign, not exhaustive proof. Corresponding four AFL++
EACH60s,0savedcrashes/timeouts. EACH THREE reviews: target boundary; actual locked
instrumented build/campaign; finite coverage and existing crash-gate limitations
stayPG9 work. Raw /tmp/mnesis-pr390-fuzz.log retained. Normal exacthead protected
merge authorized; whole PG0–PG9 and publication remain incomplete.

Final backup slice normalSIGNEDhook26864completed0 at41c6732eb457953b7f29c3a17a6ef622c8a0a60d,
signatureG. Workspace1571/1571PASS522.470s,13slow/4compiler cases filtered,
cleanfixup. THREE reviews: full owning workspace and new source compile; actual
fresh nextest execution; DATABASE_URL absent means no native test credit, no gate/
workload/assertion weakening. Independent Linux64778completed0:230/230PASS58.457s,
0skips, /nix/store/5y75jgpyxzg9xwpljkm2pfvmd0kdghvs-mnesis-postgres-integration.
THREE reviews: full combined physical/backup suite; actual fresh native18.6server;
zero skips/filtered/relaxed bounds, exact native errors and final maps asserted.
Five one-case native smokes EACH THREE-reviewed: intended feature; actual body;
other cases filtered, no independent feature-build proof. Raw
/tmp/mnesis-pg7f-hook-final.log and -combined-linux.log retained. No new execution
claimed from subsequent source-equivalent parent squash rebase or cached gate.

PR390 protected squash merged2026-10-18T23:37:42Z at
2247c739525f9d8765179b7d9f7f67c6b43bd32f from reviewed5b717bde,
all40checksSUCCESS/one scheduledskip/no reviewthreads. No admin/waiver.
Final backup source41c6732e already passed actual230native/Linux and unchanged
full gate; append final evidence/docs, then source-equivalent child-only rebase
onto physicalsquash. Cached subsequent gates are not extra runtime executions.

## PG9 decoder boundary baseline — 2026-10-19

On protected parent49104117, actual original owning decoder benchmark executes
all three original workloads twice,30samples/case,1s warmup/3s measurement,
Rust1.99.0/LLVM23.1.1 on M4Pro/aarch64Darwin. First95%intervals small2.5093–2.5722,
medium2.5021–2.5101,large2.5008–2.5076ns. Secondsmall2.5089–2.5157,
medium2.5423–2.6182,large2.6419–2.7294ns. THREE reviews for EACH run: same actual
production decoder and unchanged three payloads; exact existing benchmark/input
and compiled binary138285fb91733059, setup outside timing, all30samples execute;
raw outliers/intervals retained and named Criterion baselines avoid comparison
against prior rejected inline results. Noise/drift is material (roughly9% across
large intervals); native warm ARM does NOT waive deterministic x86 simulation.
No product edit preceded these baselines, no concurrent own native suite during
timing. /tmp/mnesis-decoder-parent-{first,second}.log, -environment.log.

Official CodSpeed CPU, regression-causes and reducing-variance docs accessed
2026-10-19 explain weighted instructions/cache costs and possible code-layout,
runner and allocator variance. Existing observed CPU-model mismatch is evidence
of environment difference, not proof of this regression's cause. Do not change
benchmark isolation/metric/cache behavior to erase the original gate. Candidate
remains the recorded single production-owner move; reject if correctness or
native performance regresses beyond measured variation, require actual CI.

First decoder-owner candidate16Rustlines churn (+6/-10, within18) actually runs
all3original workloads with30samples: small2.7052–2.7155ns (+7.81..9.00%vssecond
namedparent), medium2.7173–2.7247 (+3.35..6.19%),large2.7168–2.7274 (+0.66..3.12%).
THREE reviews: actual production decoder/unchanged payloads and error algorithm;
only header-size choice moved to its using body owner, original command/named
baseline, no benchmark/gate/hint change; raw intervals/outliers retained, small
regression exceeds its own parent interval variation though earlier large-run
drift exists. Not accepted or called improvement. Both whole binaries6704128bytes
(file size, not text-only). Run saved original executable as A/B/A control before
causal conclusion; no source minimization justifies a real runtime regression.
/tmp/mnesis-decoder-candidate-first.log. Initial stat command used incompatible
BSD flag in GNU environment; original output kept -parent-size-first.log, actual
byte count corrected with wc and independently compared. No result fabricated.

Saved-binary control invocation initially ran Criterion TEST mode:3inputs print
Testing/Success and complete without timing. THREE reviews: A/B/A requires actual
measurement; real original executable only does three decode smoke cases; no
30sample/interval/timing or A/B/A credit. Keep /tmp/mnesis-decoder-parent-return.log.
Exact installed Criterion5source923/998+ confirms standalone invocation needs
--bench (Cargo supplies it automatically). Repeat with that supported flag to
match original cargo bench mode, same input/filter/warmup/samples. No source or
benchmark alteration. Rename smoke log separately instead of overwriting it.

Actual measured saved-original A/B/A control executes all3original workloads,
30samples/1swarmup/3smeasurement; original intervals return to approximately
2.5–2.64ns versus candidate2.705–2.727ns. THREE reviews: same production owner/
original input sizes; exact saved pre-edit executable in actual --bench mode,
not earlier smoke; full raw intervals/outliers retained, native ARM only and
not a CodSpeed waiver. Small/medium return below candidate, so reject the body-
boundary move for real runtime cost. Revert ONLY owned16line wire change;
gitdiff versus parentwire is empty. Exact rejected source saved
/tmp/mnesis-decoder-body-candidate.rs, binaries preserved; no production change
retained, no easier benchmark/gate/threshold/inline attribute used. Native
current noise does not justify accepting a consistent slower source candidate.
/tmp/mnesis-decoder-parent-return-measured.log is authoritative, not its smoke log.

Cold overflow candidate49Rustline churn(+16/-33) runs same three actual workloads,
30samples, unchanged benchmark/metric/input. Intervals small2.6180–2.7108,
medium2.6059–2.6916,large2.5975–2.6660ns; named parent-return comparison reports
+2.88..5.55/+3.29..6.46/+1.48..5.56%. THREE reviews: same production decoder,
exact error fields/priority and original three inputs; actual supported cold/lazy
construction candidate builds and executes, native whole binary6704128bytes;
all CIs/outliers retained, no improvement claimed or gate waived. Native drift
still overlaps previous large baseline, while small/medium show slower reports.
Not accepted: inspect generated owner code and saved-original control before
keeping any source change. /tmp/mnesis-decoder-cold-first.log.

Cold factory rejected, source reverted to exact parent (gitdiff0). Actual pinned
LLVM owner disassembly at0x10015c930..0x10015cc10 has190lines each, changes branch
layout but does not eliminate the normal header work; no emitted separate cold
factory symbol. Initial symbol-filter commands produced only warnings/two-line
headers: NO instruction-equivalence credit. Address-based dump actually contains
instructions and is retained. Same whole file size is not equal machine code.
Measured candidate gives no speed improvement and reports slower small/medium;
smaller source alone is insufficient. Exact rejected source/binary preserved
/tmp/mnesis-decoder-cold-candidate.rs and -cold-benchmark. No retained production
change or claimed release repair. Raw -parent/-cold-addresses.log show actual
code, not empty -assembly/-instructions attempts.

Exact-header array candidate formattingPASS, strict store all-feature/all-target
ClippyPASS9.57s,40actual wire lawsPASS0.256s/177filtered. THREE reviews for EACH:
unchanged header/V1/V2/endian/metadata/error priority/range contract; actual
maintained core::slice::first_chunk (stable1.77 primarydocs) and exact inferred
array pattern compile on pinned1.99, all40selected existing wire laws execute;
filtered177 are not successes/full-store proof, no new unsafe/unchecked arithmetic,
assertion/lint/error weakening or original benchmark change. Source26+/34-=60
churn within100, two obsolete V1offsets removed, V2offsets remain for existing test
fixtures only after workspace reference inspection. /tmp/mnesis-decoder-array-{clippy,wire}.log.
Official https://doc.rust-lang.org/core/primitive.slice.html#method.first_chunk
returns borrowed fixed array or None on short input, no allocation. Missing local
rust-src package is not primary-source verification; official docs plus exact
compiler execution are the evidence. Native performance still unproven.

Exact-array actual benchmarkPASS all3original30sample workloads: small1.5489–1.5590,
medium1.5691–1.6777,large1.5553–1.6242ns versus current original2.5–2.64ns; named
comparison improvements35.68–40.30%. THREE reviews: same actual production decoder,
original payloads, native error algorithm and ownership; ONLY safe fixed-header
view/pattern replaces repeated indexed loads, benchmark/metric/gates/hints untouched;
all raw intervals/outliers retained, improvement exceeds current native drift,
but NOT x86/CI/release proof. Wholefile6704128bytes unchanged; decoder symbol span
old0x10015c930..0x10015cc10=736bytes, new0x10015cab0..0x10015cc98=488bytes,
not a claimed wholebinary text-size reduction. Need actual new instruction dump,
complete owning tests/fuzz, no_std/platform matrix, normal gate and protectedCI.
/tmp/mnesis-decoder-array-first.log. Candidate remains uncommitted.

Complete store owning run651/651PASS17.808s, zero skips. THREE reviews: all
existing store/wire/value/import/export/checkpoint/consumer and diagnostic laws
must preserve their contract; actual complete selected all-feature target set
executes, including compiler fixture owner and original malformed/header/range
cases; no filtered cases or new allowances, no PostgreSQL proof inferred from
inmemory/store tests, no API/error priority change accepted. Source unchanged
since40wire laws/native benchmark. /tmp/mnesis-decoder-array-store-native.log.

Actual full LLVM bounded decoder dump128lines has ZERO panic_bounds_check call
sites; original bounded190line owner has8 (earlier hypothesis guessed6). New
compiled owner488vs736bytes; applies to inspected AArch64 artifact only. This
supports the specific eliminated bounds machinery, not a guess about x86 cache
miss causation or a wholebinary size saving. Failed symbol-only empty attempts
remain excluded, address dumps contain real instructions. No unsafe introduced.

Full Fjall owning run232/232PASS550.736s,12slow,0skips. THREE reviews: persisted
wire decode/read/import/export and all shared store laws must preserve behavior;
actual native engine/corruption/journal fault/property/state-machine cases execute,
including slow64/128/256case owners without lowering their budgets; exact existing
assertions unchanged, no filtered/timeout/ignored success or hardwarepower-loss
credit. Existing proptest SourceParallel persistence-path warnings retained;
not an assertion failure or new fix. /tmp/mnesis-decoder-array-fjall-native.log.
Store doctest command exits0 but ALL12examples are existing ignored cases (11+1).
THREE reviews: documentation code must not gain false execution credit; actual
rustdoc invocation0.79s/mergedcompile0.54s; ZERO examples execute, no doctest
behavior proof. Full existing compiler fixtures execute separately above.

Independent aarch64 Linux native Postgres package
/nix/store/fivsg16crwp8dfqjgrv7vmsxq91f9nx7-mnesis-postgres-integration
passes230/230in43.840s,0skips, plus five filtered one-case capability smokes
1.818/3.113/3.139/2.364/0.776s. THREE reviews for EACH actualrun: current parent
adapter/backup/restore behavior must compose with changed core decoder; actual
PostgreSQL/tools and source archive execute including Fjall interchange; full
suite0skips, smokes deliberatelyfiltered, no TLS/durability additions on THIS
branch or count extrapolation. /tmp/mnesis-decoder-array-linux.log. Normal
signed full gate and actual x86 CI/performance/release proof still pending.

Normal SIGNED full-hook commit61368a255ee05fed3763674c569dea109042b219(G)
completed0, unchanged nix flake check, all12Darwin checks successful. Fresh
workspace1571PASS568.764s,13slow/4compiler-fixture exclusions; replay actual
CBOR2/value1/wire1 each1.00s. THREE reviews for EACH gate run:
1. Contract: wire/header error precedence, all existing store/kernel/adapter
   consumers, no_std/wasm and public/feature surfaces remain unchanged.
2. Execution/falsifier: actual pinned strict release all-target Clippy, nextest,
   native replay corpus and target builds execute. Full workspace native DB
   guards return early WITHOUT DATABASE_URL: no fresh PostgreSQL behavior credit
   from this Darwin gate. Independent Linux230 proof recorded separately.
3. Oracles/limits: no test/input/assertion/filter/lint/benchmark/gate changed;
   four compiler owners deliberately excluded here, not missing successes.
   Replay2/1/1 are short corpus checks, not long fuzz campaigns. Nix audit scans
   locked1280advisories/364deps yet emits missing registry/yank index errors and
   existing allowed event-listener warning; derivation0 DOES NOT mean complete
   clean live security/yank audit. Deny preserves existing policy/duplicates and
   license warnings. Incompatible platform warning retained; no x86 CI proof yet.
Raw fullgate /tmp/mnesis-decoder-array-hook.log, extracted nextest/replay and
nine other derivation logs retained under /tmp/mnesis-decoder-array-*gate*.log.

Initial staged whitespace check reported a new blank line at EOF in eight
benchmark evidence copies; shell did not stop the signed hook, so whitespace
was NOT initially clean. After terminal commit, normalize only those owned
checked-in copies to exactly one final newline (original /tmp logs unchanged),
record full gate and perform normal signed evidence-only amendment. Executable
source must compare byte-identical to61368a25 before push. No bypass/cached run
claimed as a new full test execution.

## Caller query names: original stream-isolation failure

Protected parent49104117, Rust1.99, native PostgreSQL17.10, existing Unix socket
cluster55432, unchanged SQLx0.9 default statement cache. Exact command:
`cargo nextest run -p mnesis-postgres --all-features --test names_tests --test-threads=1 --success-output immediate`
inside existing Nix shell, DATABASE_URL set to mnesis_pg7b. Actual1run/1FAIL,
0PASS/0skip,0.088s summary (case0.083s), not an inferred source finding. Original
raw `/tmp/mnesis-pg8-names-original.log` is preserved unchanged.

Three independent reviews after this run:
1. Contract: raw second stream identity must select its own exact payload and
   INITIAL version; returning first-body is cross-stream disclosure, not a
   legitimate order or visibility choice. PG4 acceptance is reopened.
2. Execution/falsifier: actual healthy migrate+two acknowledged appends, healthy
   second-body read, native LISTEN idle readiness, two-lease caller pool, real
   application bytea operator. Native ordinary equality TRUE and explicit
   pg_catalog equality FALSE establish actual search-path selection. Actual SUT
   then returns first-body at final payload assertion. No production changes.
3. Oracle/source limits: exact1row and INITIAL assertions pass before exact bytes
   fail (left first-body/right second-body). Injected schema dropped BEFORE final
   result assertions, no downstream fixture poisoning. DATABASE_URL exists and
   test body executes;0skips. This proves read_head bytea routing only, not every
   query name, authorization bypass or arbitrary SQL injection. Default statement
   cache retained. Head-forgery/state-write follow-ups separately budgeted before
   editing; no blanket completion claim.

Follow-up original run1 exited101 at compilation, zero tests executed. Three
reviews: (1) no contract behavior exercised; (2) no native fault/aggregate/state
execution, no catch credited; (3) literal byte array is not Into<Bytes>, while
supported &str is. Correct the test payload input only, preserve compiler log
`/tmp/mnesis-pg8-names-followup-original.log` before rerunning. Production unchanged.

Follow-up2 ORIGINAL actual3FAIL/0PASS/0skip,0.433s. Three reviews:
1. Contract: original read disclosure repeats; snapshot second write overwrites
   first's acknowledged state (only first key remains with second-state), and
   forged numeric MAX makes expected999/event1000 return Ok over actual head1.
   These violate identity, cache ownership and contiguous committed versions.
2. Execution: each uses fresh migrated/truncated native store; no-fail-fast and
   one test thread execute all3. Healthy first/second event seeding precedes
   injection. Real maximum selection control yields (999,1); actual append
   returns PgAllPos, not a setup failure. Actual independent native state query
   after dropping injected schema returns wrong first-key/second-state pair.
3. Oracles/limits: exact payload/state assertions reject actual wrong values;
   max test captures independent native rows BEFORE its result assertion but
   assertion panics first on unexpected Ok, so persisted-gap assertion was not
   reached. Move existing exact rows assertion earlier and repeat original for
   direct row-level proof before repair. No production edits, no mutation credit;
   retain both raw originals. Schemas cleaned before assertions in all3. Native
   equality control is explicit in read law, maximum control in head law; snapshot
   state law independently tests shared read_key's write path. This is not yet
   coverage for snapshot/checkpoint read WHERE clauses; add state-read witness
   within320-line test budget before editing those SQL owners.

Follow-up3 ORIGINAL actual4FAIL/0PASS/0skip,1.032s. Three reviews:
1. Contract: unchanged stream and state-write failures repeat; added state-read
   law asks second and requires exact second-state/INITIAL. MAX law now first
   checks independent settled native rows; original actually inserted version1000
   and must-not-land after two version1 events. No contiguous-version guarantee.
2. Execution: all four actual SUT paths execute with real DB under same caller
   pool; independent catalog-qualified MAX control distinguishes999 from1. Four
   test processes sequential, all schemas removed before final oracles. Exact
   original payload evidence is retained, not replaced by post-fix-only tests.
3. Assertions/limits: state-read final exact pair fails, same schema and version
   do not hide wrong body; state-write exact two rows fails, read exact body fails,
   head persisted-row assertion fails before typed conflict oracle.0skips. No
   nonnative controls or timing assumptions substitute for DB execution. This
   does not prove checkpoint read WHERE, integer joins, page byte accounting or
   schema validation. Preserve those remaining hypotheses, not an all-SQL claim.

Read-only release audit: live PR378 now head9d5ccbb386f41397e8cd53e4172954334909be80
explicitly selects0.5.0 for all nine crates and lists PgAllPos::new2->3 arity as
breaking. Prior concern about shipping this as0.4.1 is disproved by current actual
release output; no version/policy edit warranted. This is not publication proof.

Follow-up4 ORIGINAL actual5FAIL/0PASS/0skip,0.258s. Three reviews:
1. Contract: original four failures repeat; independent checkpoint read must
   return second's exact revision/position/state, never first's saved pair.
   It instead returns first's position/state despite matching revision/schema.
2. Execution: actual read_all positions from two real acknowledged seed events
   feed production commit_checkpoint calls BEFORE operator injection; production
   hydrate_checkpoint(second) executes while caller path is active. Native
   schema removed before final exact enum comparison. No fixture setup failure.
3. Source/oracles: checkpoint full Found enum distinguishes pair mixing/disclosure;
   MAX row-level assertion again sees actual1000/must-not-land. All5sequential
   native bodies run,0skips, not unit/mocked/equivalence-only evidence. Existing
   tests' result assertions retained. Scoped repair now justified for read_key,
   read_head MAX+identity, snapshot/cache and checkpoint lookup WHERE; integer
   joins/bounds/catalog validation remain separate untested risks.

Minimal query repair actual5/5PASS0.281s,0skips. Three reviews:
1. Contracts unchanged: exact own event/cache/checkpoint pair, rejection of
   forged999 with actual1, no inserted1000. All five ORIGINAL red laws pass with
   identical inputs/faults/oracles; no retry/commit mapping changed.
2. Execution: actual same17.10 DB and supported caller pool/statement cache,
   application objects remain selected for native controls; SUT now uses explicit
   pg_catalog bytea operations/MAX, preserving caller search_path. Native settled
   two-row query verifies no gap and both independent snapshot states survive.
3. Source/oracles:8added/5removed=13existing Rust churn in3SQL owners,35budget;
   newtest317/360lines. No API/schema/deps/session mutation/long transactions or
   allowances. Result-type conflict exactexpected999/actualINITIAL passes after
   row oracle. This is targeted5law proof, not complete matrix or capacity proof.

Strict adapter all-feature/all-target ClippyPASS4.70s. Three reviews: typed SQL
changes preserve public contracts and compile all call sites; actual pinned
compiler/lints execute existing and new targets; no lint allowances added or
thresholds changed. This does not count as native DB proof. Names full native
run continues. Initial Linux command started before new names_tests.rs was
staged, so Nix Git source excludes that untracked file: DO NOT credit it as
new5law native proof even if terminal green. Stage names owner then run exact
new-source derivation separately; retain initial command/result honestly.

Complete current all-feature native run235/235PASS57.439s,0skips. Three reviews:
1. Contracts: previous230 plus all5identity/head/state laws preserve ordering,
   byte bounds, snapshots/CAS, atomic import, finite export, migration/recovery,
   and uncertainty across actual wrappers. No stronger durability/TLS claim.
2. Execution: actual17.10 shared cluster, one testthread, DATABASE_URL present;
   fault transport/independent rows, private physical18.6 recovery tools and
   actual Fjall interchange execute; no concurrent localnative suite.
3. Assertions/source: original red oracles unchanged after13line SQL repair;
   all235execute0skips, no filtered cases hidden. This branch does NOT contain
   TLS5 or durability10:235count coinciding with TLS branch is not combinedproof.
   No result inferred for Linux/fullNix/CI/mutations until actual completion.

Isolated-feature native names runs complete default2/2(0.102s),snapshot4/4(0.191s),
projection3/3(0.141s),import2/2(0.097s),export2/2(0.084s),all0skips. THREE reviews
for EACH actualrun:1same exact identity/head/state invariants within its feature;
2native DB+actual productionSUT executes with original operator/aggregate faults;
3cfg-disabled laws are absent rather than credited, selected suite counts exact,
no workspaceallfeatures results substituted. These are targeted names laws, not
complete perfeature adapter/nativeCI or isolated downstream consumer proof.

Initial Linux run before staging new test owner exited0: protected230/230PASS
59.022s,0skips plus5filtered one-case smokes0.354/0.500/0.431/0.301/0.548s,
/nix/store/gir2s697dm4y6xpanhpnwrkfws61caf5-mnesis-postgres-integration.
Three reviews for EACH actualrun:1existing native contracts still apply to13line
SQLcandidate;2real independent Linux18.6 engine/tools execute existing230;
3new5names cases are ABSENT from first Nix Git archive, so zero new-law Linux
credit and not matched complete sourceproof. Retain initial actual result/log
without replacing it. Correctly staged new-source Linux kd12f4/9qshfm remains
running. No completion inferred until actual235includesnamesowner in its log.

Correct staged intermediate Linux source executes actual235/235PASS54.267s,
0skips and5one-case filtered smokes0.945/0.622/0.825/0.703/0.850s. THREE reviews
for EACH run:1all previous230+five names contracts compose;2actual18.6 native
Linux archive includes all5 names_tests cases (not omitted first archive);
3exact unweakened original oracles pass, all235run0skips, smokes deliberately
filtered. Additional caller search_path preservation assertion was added after
this archive snapshot: this is intermediate5law proof, NOT that extra oracle's
final Linux result. Current final Linux78554 remainsrunning; preserve both.

Final path-preservation local source native235/235PASS48.365s,0skips.
THREE reviews:1same5identity/head/pair contracts plus unchanged caller pool GUC;
2all actual production/fault/migration/recovery laws execute on native17.10,
SHOW captures the only available data lease after SUT, cleanup precedes exact
path and original result assertions;3new helper only consolidates test cleanup,
original5oracles remain, no SQLrepair change or weaker assertions/filters.
No new feature-matrix or Linux credit inferred from this local full run.

Final strict all-feature/all-target adapter ClippyPASS2.99s. Three reviews:
unchanged product/feature contracts; actual final path-preservation test/helper
compiles through pinned lint policy; no allow/threshold changes, no native proof
from compilation. Actual wc reports final testowner314/360lines after cleanup
deduplication, not a source-budget estimate.

Caller-path assertion Linux run235/235PASS38.043s0skips,5filteredsmokes
1.440/2.629/1.943/3.202/2.057s. THREE reviews for EACH:1original5invariants AND
unchanged caller GUC now asserted;2actual18.6 Linux SUT executes all235;3archive
contains the helper but its two chained await lines precede rustfmt normalization.
Bytewise cmp reports difference atline70; actual diff is ONLY four line breaks
in test helper, no assertion/input/product change. Do not claim byte-identical
final source; a final formatted-source Linux run follows after staging is stable.
Preserve /tmp/mnesis-pg8-names-final-linux.log and exact derivation JSON; root
progress's premature cmp-success text corrected immediately after inspecting
actual failed cmp. No gate result fabricated from that comparison.

Before signed full gate normalize ONLY owned checked-in evidence copies' trailing
spaces and final newlines; all raw /tmp originals remain unmodified. This is
presentation normalization, not discarded failures/control output. Final source
and test assertions unchanged. Final formatted Linux command separate; normal
signed full Nix gate must complete without bypass before push/merge.

Query original normal SIGNED full gate67804DONE0 at77bef05b8b4190e0329494567769d941f8f0c4c6(G):
all12Darwinchecks, workspace1573PASS563.191s14slow/4compilerexclusions, short
replay2/1/1each1s. THREE reviews for EACH actualrun:1unchanged existing owner
contracts+two unconditional names laws;2actual pinnedrelease/coverage/no_std/
wasm/lints and corpus checks execute;3DBguard earlyreturn without DATABASE_URL
means extra registered laws are NOT native DB credit; no changed assertions/
filters/allowances, compilerexcludedseparateCI, audit/yank blindness retained.

Final formatted predecoder Linux actual235PASS92.338s0skips plus5filteredonecase
smokes1.215/1.938/1.099/0.956/12.058s, artifact
/nix/store/5zar4rqafm3qskdsv9s7vypl39n47dg0-mnesis-postgres-integration.
THREE reviews for EACH actualrun:1all230+five identity/head/pair/pathpreservation
contracts;2actual18.6 native tools and all5 names SUT cases execute;3source
src2ljf2lpy0kp6g3vkyc7qbrz3llc7nbnj names_tests bytecmp FINAL formatted filePASS,
zero runtime skips, capabilitysmokes intentionallyfiltered, no extrapolated
TLS/durability/decoder-combined proof. Earlier cmpfailed archive preserved as
format-only intermediate, never relabeled. This final run predates decoderbase.

Protected decoderPR393 mergef207923a6131ea0270e0e8b22810d8cb3afb7adc is now parent.
Signedrebase resolved ONLY five record conflicts: verified exact oldbase prefix
in both parent+child for append-only experiments/hypotheses/sources/research,
kept fullparent then fullchild suffix. Progress preserves exact106-line common
prefix and both fullsuffixes. No evidence discarded or product source rewritten.
Adapter/manifests/locks/Nix/workflows compare byte-identical to77bef05b after
rebase; changed parent core decoder now requires actual combined validation.
Use one combined native suite, unchanged full signed Nix amendment, and required
CI's real Linux execution; do not add redundant standalone Linux reruns when
those CI runs establish the exact combined source. No release/completionclaim.

## Decoder PR393 final CI review before protected merge

Reviewed exact signed9f81cbddd15b86cdb797b247ee9303d252595591 against protected
49104117. Current40SUCCESS/1scheduledSKIP,0threads,MERGEABLE/CLEAN. CI39185929403
and39185948529; fuzz39185948594; mutation39185948595; CodSpeed39185948558.
Raw /tmp/mnesis-pr393-{ci-call,ci,fuzz,mutants,performance}.log plus API checks,
threads and downloaded artifact retained. THREE reviews for EACH actual run:

| Actual evidence | Contract review | Execution/falsifier review | Oracle/source/limit review |
| --- | --- | --- | --- |
| Workspace1571 twice116.523/125.395s,4compiler exclusions each | Whole existing Rust owner contracts unchanged | Actual Linux release/coverage runs | Same source/assertions/filters;2/4slow retained, compiler cases separately2/1/1 |
| Native230 four runs49.643/53.328/49.584/50.059s | All protected parent adapter/storage/recovery laws compose | Four distinct realPostgres executions,0skips | Duplicate installed summaries excluded; no TLS/querynames/durability counts added |
| Twenty filtered native one-case capability smokes | Each registered capability exists | Actual selected named test executes | Other cases intentionallyfiltered, not added to full230 totals |
| Compiler2/1/1 each CI,84.52/3.23/9.25s and110.73/4.05/12.42s | Existing compile rejection contracts | Actual trybuild owners execute | Four fullgate exclusions accounted separately, no missing-test success |
| Feature jobs each CI: kernel15/15/15,macros1/1/1,store217/217/217,wake17/17/17,inmemory25/25/30,Fjall131/131/131 | Existing default/none/all library contracts | Actual selected library checks and unit harnesses | Nostd/store-testing0/0/0 compile-only, PG omitted from this matrix remains PG9 |
| Replay2/1/1 each1s in both CI | CBOR/value/frame error and roundtrip invariants | Actual short seeded replay owners | Short corpus coverage, not long random or nativeDB evidence |
| LibFuzzer61s each wire13,630,546/header1,899,447/chunk1,338,760/value5,929,371 | Original malformed/header/codec invariants | Actual four ASan campaigns on unchanged fuzz owners | No crash, no changed seeds/time limits/oracles; finite campaigns not exhaustive proof |
| AFL4x60s,0savedcrash/timeout each,CMPLOGconfirmed | Same four failure domains | Actual instrumentation and native fuzz runs | Calibration warnings retained; existing zero-exit crash-gate fault-proof still PG9, no invented catch |
| Mutation baseline650PASS10.605s,1compiler exclusion; solemutant UNVIABLE | Existing store laws preserve behavior | Baseline actually executes; replacement needs nonexistent FrameHeader::Default | ZERO catches/zero viable mutants, not runtime catch. Downloaded12158550134 digest272a25fc4ce9319df781e50fed6ae02342900916e15cf50365b5eb63ff703d0c, diff+firstcompiler error inspected |
| CodSpeed PASS all3changed decode sizes501.7->397.2ns,+26.29%reported efficiency | Same existing actual production decoder and original3payloads | Simulation changed source at exact head, gate passes without waiver |40untouched/3skippedbaselinefallback, runtime-environment and17syscall warnings retained; simulation not wallclock, native A/B/A independently recorded |

Only executable diff remains26+/34- header owner; benchmark/workload/metric/input/
gates/dependencies untouched, no unsafe or added hint. Completed signed localgate,
sourceequivalent evidenceamend and fullbasewhitespace checks retained. Remaining
whole-adapter acceptance/publication/registryconsumer NOT claimed. Post-CI evidence
is an owned uncommitted documentation append, not a new executable/reviewed head.

Protected normal squash PR393 MERGED2026-10-19T01:54:59Z exact reviewed signed
head9f81cbddd15b86cdb797b247ee9303d252595591, merge
f207923a6131ea0270e0e8b22810d8cb3afb7adc. All40checks pass,1scheduledskip,
0threads, downloaded mutation artifact explicitly0runtimecatches/1unviable.
CodSpeed501.7->397.2ns at3existingdecode sizes, warnings/3unrelatedskips retained.
No admin/bypass/thresholdreset/manualack. This fixes demonstrated decoder
regression at PR393; releasehead/TLS/newsource still require their actual gates.
No whole-adapter completion/publication inferred.

Combined protected decoder+query source native235/235PASS53.423s0skips.
THREE reviews:1all original ordering/checkpoint/atomicity/outcome/identity/head/
saved-pair contracts preserved after newcore parent;2actual17.10 complete suite,
private18.6 physical tools and five names laws run on signed rebased source;
3exact original assertions/faults/feature selection unchanged,0skips, newdecoder
is actual protected owner not a sibling source assumption. This branch still
has neither TLS5nor durability10;235is not their combined total. Normal signed
amendment follows and requiredCI establishes exactcombined Linux source.

Combined normal signed amendment attempt5226 FAILED exit1. Actual workspace
1573run:1572PASS/1FAIL/4compilerexclusions,13slow,623.143s. Last Fjall property
owner fails536.809s at native StorageFull/os28 (minimal generated input[1],168
successful cases), and LLVM profile write also reports No space left on device.
THREEreviews:1original storage/version invariant still required, no test changed;
2actual complete selected workspace executes under concurrent local Nix gates,
real disk exhaustion reaches owner and profiling writes;3FAILED is retained,
not credited as passing fullgate or waived as harmless infrastructure. DBguards
still not native credit. No product/test/workload/lint/timeout change justified
by this resource failure. Original logs /tmp/mnesis-pg8-names-combined-hook.log
and -combined-nextest-diskfull.log retained. Signed commit remains b8531258,
so no failed amendment/push/delivery claimed.

Both failed Nix builds have discharged temporary trees; actual df now22GiB free.
No broad GC/raw cache deletion, source reset or unrelated job termination. Run
one full signed gate at a time, keep all original failure evidence, preserve
original proptest budgets. Other sharedtargets/performance/proof artifacts stay.

Serial signed-gate retry59677 completed successfully at signed head
95a00bc15d72cd04ee79ab25600ec321e03b019e (good GPG signature). All12 local
Nix checks passed. Fresh nextest1573/1573PASS523.520s,14slow,4compiler exclusions;
fresh short fuzz replay2/1/1PASS, each1.00s. Original failed623.143s run remains
above, without waiver. Logs /tmp/mnesis-pg8-names-serial-{hook,nextest,replay}.log.
THREE independent reviews after this run:
1 Contract: workspace storage/version/order/atomicity and decoding invariants
remain unchanged; the query repairs preserve native identity/head and saved-pair
contracts. The actual original names assertions and cleanup owner were reread.
2 Execution/falsifier: fresh nextest and three replay commands actually executed;
all original Fjall property budgets remain. This gate's DATABASE_URL guards can
return early, so its PostgreSQL registrations add NO native proof; separate235
native53.423s and required combined-source Linux CI supply that boundary.
3 Assertions/source/limits: executable source compares exactly to b8531258,
including tests, manifests/locks, Nix and workflows. Four compiler exclusions are
retained, replay durations are short smoke evidence rather than a new long fuzz
campaign. No counts combine TLS5 or durability10. Whitespace/GPG checks pass.
Post-gate documentation is kept locally for the next necessary evidence commit;
no fresh execution is claimed for cached checks or a later documentation edit.

PR394 exact95a00bc1 first CI push run39195623984 completed-job review (workflow
still running). Direct GitHub job-log API downloaded the actual finished logs;
`gh run view --job --log` refused until whole-workflow completion, so those initial
empty files supplied no evidence. Original full logs now /tmp/mnesis-pr394-push-*.log.
Each listed command run received these THREE independent reviews:

| Actual command runs | 1 Contract | 2 Execution and falsifier | 3 Assertions, selection and limits |
| --- | --- | --- | --- |
| Kernel compiler2PASS72.06s, macro compiler1PASS2.81s, store compiler1PASS8.01s | Required invalid construction, version literal and live export borrow rejection remain. | Actual trybuild fixture lines are all ok, including valid nonzero control and invalid zero codegen case; no snapshot blessing command. | Reread all three runner sources: kernel2 registrations, others1 each; count registrations separately from fixtures. No exclusions/ignored cases or DB credit. |
| Kernel lib default/no-default/all:15/15/15PASS | Kernel aggregate/version laws unchanged. | Three actual cargo test invocations, no ignored/filtered tests. | Full lib selection at each mode; integration/compiler fixtures are separate, dev-dependency unification remains a PG9 limit. |
| Macros lib default/no-default/all:1/1/1PASS | Parsing/expansion contract unchanged. | Three actual unit commands; compiler-rejection suites run separately above. | Zero ignored/filtered; one lib unit is not complete macro API evidence. |
| Store lib default/no-default/all:217/217/217PASS,3.22/2.98/3.30s | Wire/value/store contracts unchanged by SQL-only repair. | All three actual suites execute; protected wire decoder source retained. | Full lib suites, zero skips; separate integration/fuzz/native evidence remains required. |
| Wake lib default/no-default/all:17/17/17PASS, each0.15s | Arming, routing, generation and cancellation contract remains. | Actual three unit commands; sources retain concrete Tokio tests. | Zero ignored/filtered; not a live PostgreSQL listener/reconnect proof. |
| Inmemory lib default/no-default/all:25/25/30PASS, each0.05s | Inmemory persistence/optional capability contracts unchanged. | Actual3 commands show all-feature adds5 tests, no artificial common total. | Zero ignored/filtered; this adapter's laws cannot substitute for Postgres. |
| Fjall lib default/no-default/all:131/131/131PASS,2.51/2.49/2.50s | Fjall storage/order contract unchanged. | Actual3 commands and seven preceding lib checks are visible in downloaded job log. | Unit-only feature checks, zero ignored/filtered; property/integration owners are separate. |
| Wake-nostd lib default/no-default/all:0/0/0; store-testing lib default/no-default/all:0/0/0 | Both must compile under advertised selections. | All six real harness invocations finish but execute ZERO tests. | Compile-only evidence, not six behavior passes; downstream conformance and no_std targets remain separate. |

These first completed jobs do not establish native Postgres, exact-head CodSpeed,
fuzz, mutation, final CI or protected delivery. Current CI still running; no merge.

PR394 first current-combined Linux native job117877686721 in PR workflow
39195637612 is SUCCESS. Actual complete suite235/235PASS42.711s,0skips,19binaries;
all FIVE names laws execute (0.042/0.038/0.040/0.040/0.040s). Five preceding
one-case capability smokes also execute. The later cat of installed run.log is
an ECHO of the same42.711s run, not another execution. Raw downloaded job log
/tmp/mnesis-pr394-pr-postgres.log.
THREE reviews for the full run and each of its five smoke commands:
1 Contract: native stream identity, exact committed head, separate snapshot keys,
and checkpoint saved-pair invariants align with original failing witnesses; all
old order/resume/atomicity/migration/recovery laws stay selected. Smoke owners
remain atomic/snapshot/projection/listing/session rather than proxy assertions.
2 Execution/falsifier: required native Nix hook opens the real database and actual
235 archive tests run serially, including each hostile caller-name control. The
five no-tests=fail smoke commands execute their concrete owners. An attempted
earlier fetch of still-running push-job logs returned404 and gives no proof.
3 Assertions/source/limits: final314-line names source was reread with exact body,
settled rows BEFORE Conflict, unchanged SHOW search_path, and complete saved-pair
oracles. Full suite0skips; smoke filters intentionally exclude others, so their
skips are selection rather than total-suite gaps. This source combines protected
decoder with query repair, NOT TLS/durability; no extra count from echoed logs.
Current workflow's remaining checks, mutation evidence and protected merge remain.

PR394 deep-fuzz run39195637553 SUCCESS, actual full log
/tmp/mnesis-pr394-fuzz.log downloaded (terminal CLI session89316exit0).
Each of the eight actual campaigns received THREE independent reviews:
1 Contract: unchanged shared fuzz-common owners exercise production wire/header/
chunk/value decoding; no query-name, TLS, database or recovery coverage inferred.
2 Execution/falsifier: LibFuzzer four61s campaigns actually complete wire11,230,479,
header1,744,944, chunk2,132,677 and value7,046,139 runs. Four original AFL60s
campaigns each show CMPLOG engaged and0saved crashes/0saved timeouts; discoveries
wire0/header2/chunk14/value0. These are campaign reports, not exit-code-only claims.
3 Assertions/source/limits: reread Bolero wrappers and existing AFL workflow;
shared production targets unchanged and neither input sets nor workloads reduced.
Calibration instability, redundant seed/instrumentation warnings and AFL variable
warning remain in raw evidence. AFL's zero-exit-on-crash behavior still needs
PG9's explicit crash-gate falsifier; these clean runs cannot establish it. No
mutation catches or native database results inferred from fuzzing success.

PR394 mutation review REJECTS raw14caught as adequate evidence. Artifact12161164145
SHA25668224bde5803893cd4183e9364e963ed98c3ee7b029355aef063fc1e51a7f0af actually
matches downloaded zip. Baseline235PASS49.299s.16variants report14caught/2unviable,
0survivors/timeouts. All first failures inspected: read_key4runtime failures,
read_headNone actual Conflict-vs-Store oracle, read_head(0,None) actual FK23503 in
spawned append; followingEIGHT read_head variants only fail42P06 schema ordering
already exists. They are contaminated, NOT validated catches. Two hydrate
Default variants are compiler E0277 unviable, no runtime credit. Three reviews
for baseline/each variant: contract still demands real order/head/key behavior;
actual faults separate six own-SUT runtime failures from eight fixture failures;
assertion/source review finds append.unwrap/join.unwrap BEFORE DROP in existing
shadowed_ordering. Fail-fast excludes remaining cases per mutant; not235 passes
per variant. Original artifact/logs remain in /tmp/mnesis-pr394-mutants-reviewed
and -mutants.zip. Cleanup repair and repeated independent controls required;
current green mutation check alone is rejected as merge evidence.

Cleanup original-source native control first run63448exit100: one FAIL6.338s,
5filtered. Fresh dedicated database mnesis_pg4_cleanup_xrkk8n, isolated control
worktree /tmp/mnesis-query-cleanup.xRKk8n at95a00bc1; ONLY read_head replaced by
Ok(Some((0,None))) with explicit unused-argument binding. THREEreviews:1the
ordinary append must succeed under healthy source and order/resume contract
unchanged;2actual Pg17.10 append fails FK23503, exact key0 missing, spawned task
panics then join unwrap also panics;3reread source shows native probe cleanup
lies after both unwraps, so the actual first SUT failure can poison following
runs. This intentionally failing test is NOT a production regression or passing
control. Original log /tmp/mnesis-pg4-cleanup-original-first.log retained. Repeat
identical mutant/test/database WITHOUT cleanup to falsify independence next.

Cleanup original-source repeated control exit100: one FAIL, same real database,
source/mutant/test selection unchanged and no intervening cleanup. THREEreviews:
1independence requires the second run to reach its own append fault;2actual second
run instead fails SQLSTATE42P06 schema ordering already exists, before SUT;
3fivefiltered tests remain explicit, this is actual native contamination matching
the eight rejected CI catches, not a compiler-only or theoretical concern.
Raw /tmp/mnesis-pg4-cleanup-original-second.log retained. Proceed with <=8test-line
cleanup repair; no product SQL or regression assertion is relaxed.

Cleanup candidate first control3624exit100: one intentional FAIL0.105s,5filtered.
Owned stale probe from original red explicitly dropped ONCE before this candidate
run. THREEreviews:1same bad native head0 still rejected, no assertion removed;
2actual append fails FK23503 at AFTER-cleanup result unwrap, independent catalog
query afterward reports ordering namespace count0;3same spawn+Barrier overlap,
real Pg17.10 and every order/resume assertion retained, rawfirst logs preserved.
This is expected failing-control evidence, not a passing suite. Source inspection
also catches a shadowed local name against unchanged Clippy policy; rename that
captured result before the repeated control, without adding any allowance.

Cleanup candidate repeated control57068exit100: intentional one FAIL0.064s,
5filtered. NO cleanup between the two candidate runs; only captured local name
renamed to comply with existing shadow lints. THREEreviews:1same head0 fault
must remain rejected, no production repair or weakened oracle;2actual second run
reaches its OWN FK23503 at post-cleanup unwrap and independent namespace query
again returns0;3unchanged Barrier/spawn/read/resume/body assertions, no42P06 and
no compile-only credit. Control source stays isolated, query production remains
13line native-operation repair. Actual retained test delta2+/2-=4churn<=8budget.
Original repeatedred0.045s, candidatefirst0.105s and second0.064s all retained.
Raw /tmp/mnesis-pg4-cleanup-{original,fixed}-{first,second}.log plus fixed catalog
logs; no runtime catch credit for compiler-unviable or contaminated variants.

Cleanup healthy current-source native72223exit0:235/235PASS78.426s,0skips.
THREEreviews:1original stream/global order, resume, atomicity, snapshots and saved
pairs remain required with test cleanup before failure;2actual complete native
suite executes on healthy query-worktree product source, including five names
and four shadowed-ordering laws, no injected read_head stub in this worktree;
3all existing payload/settled-row/typed-error/boundary oracles unchanged, tests
not dropped or added to inflate count, no feature filtering and0skips. The
contended-host import_storage_error case took41.777s; total is validation timing,
not capacity or latency-target evidence. Exact4test-line diff stays within budget;
production8+/5- unchanged. Original and both failed-control schedules retained.

Cleanup final strict all-feature/all-target Clippy31554exit0,6.49s; unchanged
warnings, shadow and line policies, no allowance. Format check passed before
healthy native suite. This is compilation/lint evidence, not extra test executions.
Post-cleanup source is now frozen for normal signed gate; native failures and
healthy235 proof copied below with whitespace-only presentation normalization,
original /tmp logs unchanged. Exact mutation artifact digest and rejected first
verdict remain available; fresh CI must establish independent catches.

# PG8 verified TLS first witness — 2026-10-18

First strictall/all fixture compile fails redundant-pub-crate inside private test
module,84/80line preparation and shadowed rows binding. THREE reviews: verified
TLS lifecycle/exact event/pool reopen oracles remain; compiler/linter only, zero
native execution or TLS-defect credit; private enclosing module remains the
visibility boundary, split certificate preparation from server boot, rename
recovered result and retain all checks. No allow/lint policy/dependency change.
Raw /tmp/mnesis-pg8-tls-first-clippy.log retained. SQLx0.9 SqlSafeStr supports
AssertSqlSafe for owned strings; fixed test PASSWORD is interpolated only in a
native role utility statement, never untrusted data. Other setup SQL is literal.

Current extracted-fixture strictall/all ClippyPASS1.98s. THREE reviews: private
module visibility and <=80line functions preserve shared native lifecycle;
actual compiler/linter passes, zero native laws yet; no lint relaxation, new
runtime or public adapter API. After that run the fixture gained an independent
native psql VerifyFull positive control with exact ssl=t before SQLx connection.
Nix environment21614terminal0 selects actual OpenSSL3.5.8 storebinary, Rust1.99.0,
PostgreSQL18.6; this tool-version probe is not a passing TLS adapter test.

Actual original-feature native TLS witness FAIL12.684s,1case/0skips; strictall/all
ClippyPASS1.87s. THREE reviews: VerifyFull runtime lifecycle must work against a
valid trusted native SSL server; independent OpenSSL chain/hostname check,
Postgres ssl=on and native psql VerifyFull returns exactssl=t before SQLx; actual
client then fails exact SQLxTls "TLS upgrade required by connect options but SQLx
was built without TLS support enabled", before adapter writes, not bad server/
certificate/credentials. Keep all lifecycle/content/reopen assertions unchanged;
select supported SQLx TLS only after dependency review. Original raw
/tmp/mnesis-pg8-tls-original-native.log retained; this is actual missing-capability
proof, not a green test that accepts absence. No production Cargo feature changed.

First supported Rustls/WebPKI candidate cargo-checkPASS12.69s (wall12.722s,
user12.739/sys3.072) with actual new16packages,150+/12-locklines and no existing
package version updates (Windows edges only gain disambiguating versions).
THREE reviews: required TLS backend now compiles; actual Rustls0.23.46/ring0.17.14
are patched, Rustls-webpki0.103.15/pki-types1.15.1 fit pinnedRust; compile proof
only, no runtime/capacity claim. Raw /tmp/mnesis-pg8-tls-first-build.log kept.

Actual cargo-deny licensesFAIL4: webpki-roots1.0.9 is CDLA-Permissive-2.0, absent
from current allowlist. Initial MPL expectation was stale; exact resolved
Cargo.toml is authoritative. THREE reviews: retain existing license policy;
actual license gate rejects only this root-data dependency, no TLS runtime tests;
do not allow a new license or pin an old trust bundle merely to pass. Use SQLx's
supported Rustls/ring native-roots alternative with explicit custom CA in tested
profile and same certificate/name verification. Native OS trust replaces bundled
Mozilla roots; do not claim identical trust sets or CA-file-only trust. Both
append configured root certs; app deployment owns its trust source. Ring LICENSE
now refers Apache2/ISC BoringSSL and once_cell licenses, Cargo metadata Apache2
ANDISC; license gate did not reject it. Leave deny policy/clarification unchanged.
Raw /tmp/mnesis-pg8-tls-license-first.log retained; no native pass claimed yet.

Supported Rustls/ring native-roots selection actualcargo-checkPASS6.80s
(wall6.856,user7.414,sys1.639), licensesOK under unchanged deny policy, strict
all-feature/all-target ClippyPASS3.89s. EACH THREE reviews: selected feature/
license boundary; actual command executes against resolved lock; compilation/
license approval is not native behavior proof. Actual totalnew22packages,
207+/12-locklines, no existing package version updates, within revised24/350.
Resolved Rustls0.23.46/ring0.17.14/webpki0.103.15/pki-types1.15.1/native-certs0.8.4;
platform SecurityFramework3.7.0/sys2.17.0/core-foundation0.10.1 use permitted
MIT/Apache terms and MSRV<=1.85, fit pinned1.99. Retain exact downloaded Cargo
metadata as license authority; initial WebPKI-root MPL assumption was rejected,
current native-roots eliminates that root-data crate without old-version pins.

Actual unchanged positive TLS lifecycle native1/1PASS2.145s,0skips after original
MissingTLS red. THREE reviews: same VerifyFull open/append/exact position-key-body/
close/reopen contract; real private18.6SSL server+CA, native psql VerifyFull first,
actual SQLx encrypted session and restricted runtime store execute; zero skips,
no reduced cert/name verification, no alias for TLS capability absence. This
single law does not establish wrongCA/hostname/auth/redaction, lifecycle races,
Linux, full feature matrix or production readiness. Raw
/tmp/mnesis-pg8-tls-native-roots.log retained. Shared cluster extraction still
needs all original native restore laws after feature change.

First four-law TLS run0pass/1fail/3unrun1.177s,0skips; strictall/all ClippyPASS
24.21s. THREE reviews: healthy direct-options TLS control passed first; actual
public-connect auth case fails wrong reason SqlxIoNotFound before authentication,
so NO bad-password/CA/hostname credit; inspect exact dependency before changing
oracles. SQLx0.9 PgConnectOptions::build_url serializes CertificateInput::File
using Display "file: /tmp/...", while parse_from_url treats that value as a raw
filename. Thus options.to_url_lossy is not a valid file-root roundtrip here.
Keep the red/raw /tmp/mnesis-pg8-tls-four-first.log. Construct proper URL using
supported URL query builder with raw certificate path, from common transport
options without file cert; add a healthy public-URL adapter control BEFORE its
negative cases. Keep exact28P01/28000 and native TLS cause assertions, do not
accept NotFound, bypass verification, or drop publicconnect coverage. This
upstream serialization finding does not authorize a SQLx fork or custom parser.

Proper-URL fixture attempt stops at Clippy needless-pass-by-value for borrowed
PgConnectOptions; native tests did not start. THREE reviews: preserve exact auth/
issuer/name/error-redaction assertions; compile-only zero runtime proof; borrow
options in the cold URL helper and keep supported builder/raw CA path, no new
parser or lint allowance. Raw /tmp/mnesis-pg8-tls-four-current.log retained.

Proper-URL native run1pass/1fail/2unrun2.488s,0skips; strictall/all ClippyPASS1.71s.
THREE reviews: native auth28P01 and plaintext28000 with exact messages/redaction
now pass; healthy public URL also passes before badCA, whose actual retained
cause is SqlxIo(InvalidData,InvalidCertificate(UnknownIssuer)), not the assumed
SqlxTls category. Locked SQLx0.9 tls_rustls.rs:192 performs complete_io().await?
with From<Io>, preserving the native source. Require exact IoKindInvalidData AND
present underlying certificate cause/message for CA/name failures; do not broaden
to arbitrary errors or accept prior NotFound fixture. Bad hostname has not yet
executed. Raw /tmp/mnesis-pg8-tls-four-verified.log kept. No production remapping,
error erasure, TLS weakening or native-failure credit from the wrong error tag.

Native TLS run3pass/1fail6.012s,0skips. THREE reviews: auth/plaintext exact
SQLSTATE/messages, proper public-URL healthy control, both wrongCA/hostname
exact IoInvalidData+present native certificate cause, secret redaction and normal
lifecycle all pass; actual Barrier/spawn race has one success but conflict
metadata assertion fails before permission probes; its assertion omitted error
contents, so cause remains unproven. Keep the exact oracle, add diagnostic only,
rerun only that case before deciding whether this is fixture assumption or product
defect. Do not accept arbitrary failures or infer unseen actual metadata.
Raw /tmp/mnesis-pg8-tls-four-causes.log retained.

Diagnostic-only TLS race rerun actualFAIL1.796s,1case/3filtered, exact retained
error Conflict{stream_id:"contended",expected:None,actual:None}. THREE reviews:
one actual success/one definite conflict and exact durable event are required;
real native UNIQUE collision yields unknown actual in established append mapping,
not permission/network/unknown-commit failure; PR377 forced native collision laws
and research:852 explicitly pin unknown actual, while earlier head observation
can returnSome1. Correct the fixture's unjustified Some1-only assumption, retain
exact stream/expected, restrict actual to unknown or observed1 and independent
exact winner data. No broad is_err, production mapping change, artificial schedule
or missing-error acceptance. Raw /tmp/mnesis-pg8-tls-race-diagnostic.log retained.
Clarify the existing optional observation in shared error docs; None is no proof
that target is empty. This is not a newly repaired adapter defect.

Current verified TLS native4/4PASS5.918s,0skips. THREE reviews: normal directpool
and proper publicURL, exact wrongCA/name source errors, auth28P01/plaintext28000,
redaction, close/reopen and actual Barrier/spawn one-winner race remain required;
all actual private18.6 SSL/native psql controls and SUT operations execute,
UPDATE/DELETE/TRUNCATE/ALTER/CREATE each native42501 and exact winner bytes remain;
zero skips/filters/relaxed cert policy, race permits only established unknown or
observed1 metadata with exact callerID/expectedNone. No Store failure accepted.
Readability docs now explain optional observation once at AppendError owner.
Raw /tmp/mnesis-pg8-tls-four-established.log retained. Need explicit state-update
permissions, native negative controls and owning full/Linux/feature/audit gates.

State-role test compile stops at83/80lines, no native case executed. THREE
reviews: two actual event/cache/CAS advances and exact reopened pairs remain;
compile-only zero state-role proof; reuse one private open helper across three
native consumers instead of repeating pool construction, no lint allowance.
Complete runtime profile grants INSERT+UPDATE only to snapshot/checkpoint tables,
while committed events remain INSERT-only. Initial TLS fixture role was an event
publisher; required state permissions are explicit fixture/profile additions,
not a production adapter bug or hidden test outcome. Same negative event/schema
mutation assertions remain. Raw /tmp/mnesis-pg8-tls-state-role-first.log retained.

Current five native TLS laws5/5PASS7.263s,0skips. THREE reviews: verified direct
pool/publicURL operations, exact CA/name/auth/plaintext causes, redaction and
forbidden event/schema mutation remain; actual native SSL/server/psql controls
plus real two-writer race execute, snapshot cache and checkpoint CAS each update
through two acknowledged event positions/revision1->2 and reopen exact state/
position pairs using only explicit state-table grants; zero skips/filters or
weaker verification, no whole-event/checkpoint transaction/fencing claim. Private
open helper now serves three actual consumers, state test stays under80line
policy. Raw /tmp/mnesis-pg8-tls-five-current.log retained. Need native weakened-
verification controls, original seven restore laws after extraction, current
combined source/full Linux/feature/audit gates and documentation before delivery.

Fresh native weakened-verification control1 actuallyFAIL1.165s,1case/4filtered.
THREE reviews: wrong CA must be rejected by verified client; ONLY change private
connection default VerifyFull->Require, real server/native psql healthy control
still pass and actual adapter accepts wrongCA, so expected rejection panics;
compile succeeded and runtime failure is exactly wrongCA-must-reject, not setup,
timeout or library absence. Correct branch stays VerifyFull; native cause and
redaction assertions unchanged. Raw /tmp/mnesis-pg8-tls-require-control.log and
exact source /tmp/mnesis-pg8-tls-require-control.rs retained. No mutator percentage.

Existing cargo-hakari generate+verify bothPASS; THREE reviews: selected workspace
feature ownership; actual supported generator/verification runs; no native proof
or manual generated-policy edit. Managed file6changedlines within25 budget;
Cargo.lock after generation208+/12-, total22newpackages, within24/350. Current
Nix native check adds one existing-archive --no-tests=fail TLS smoke (5lines),
retaining complete suite, plus OpenSSL in two tool lists. No custom tooling.

Fresh independent hostname control actualFAIL1.470s,1case/4filtered. THREE
reviews: hostname verification is independently required; ONLY change connection
VerifyFull->VerifyCa, healthy native/public controls and wrongCA rejection pass,
then actual adapter accepts the certificate lacking an IP SAN and exact hostname
rejection expect fails; this is runtime falsification of name checking, not setup,
compilation, timeout or broad-error acceptance. Correct source stays VerifyFull.
Controlworktree /tmp/mnesis-tls-controls.aLjhvT intentionally retains this mutant;
Require control exact source/log retained separately. Both control handles
66360/64453 terminal100 expected; no production change or mutation score claimed.
Current five-law strictall/all Clippy alsoPASS2.62s before managed manifest
regeneration; compile-only, retained exact native5/5 proof scoped above.

### TLS owning regressions and backup delivery review

After Hakari generation, default-feature strict all-target Clippy PASS3.67s,
rustfmt check PASS. THREE reviews: isolated adapter default capability must
compile; actual supported checks execute at the selected lock; these are compile
and format checks, not database or feature-matrix evidence.

Extracted cluster owner restore run7/7PASS7.957s, zero skips. THREE reviews:
history/foreign-copy rejection and coherent recovery must remain unchanged;
all seven actual independent native clusters execute, including exact corrupt
and missing PG_VERSION failures, repaired verification, waited primary shutdown,
migration checksums and saved consumer restart/advance; source retains native
argv/status oracles and no early-return credit, with fixture backup216.464ms and
stopped-primary-to-projection132.697ms not deployment performance targets.
Raw /tmp/mnesis-pg8-tls-restore-native.log.

Default-feature native security4/4PASS5.414s, zero skips. THREE reviews: the
unconditional verified transport and runtime event permissions must work without
state features; all four actual SSL/psql/SUT cases execute with wrong CA/name,
authentication/plaintext SQLSTATEs, redaction, overlap and reopen; all-feature
state law is deliberately absent by cfg and receives no credit from this run,
while both runtime verification-weakening controls remain failures. Raw
/tmp/mnesis-pg8-tls-default-native.log. Combined parent+security suite still due.

PR391 signed head07c15a59faa3676db0dd2e9343857e24c6a7872a has40SUCCESS,
one scheduled mutation sweep skip, zero unresolved review threads and CLEAN
protected merge state. Downloaded CI39177525744 and39177528258, fuzz39177528178,
mutants39177528241; raw /tmp/mnesis-pr391-{ci,ci-call,fuzz,mutants}.log.
For EACH actual run the following three reviews apply independently:

| Actual run | Contract review | Execution/fault review | Oracle/coverage review |
| --- | --- | --- | --- |
| Workspace1571/1571 in86.727/121.603s | Existing workspace laws preserved | Actual full gate passes,1/4slow respectively | Four compiler tests excluded here and tested separately; DB-free adapter results are not native proof |
| Native230/230 in43.132/49.667/51.006/47.704s | Full backup/restore/order/outcome suite retained | Real native database and independent restore tools execute each time | Zero skips; repeated printed47.704 summary is not a fifth execution; no TLS laws on this head |
| Twenty one-case feature smokes across four native builds | Advertised capability registration required | Each actual filtered case passes | Nonselected cases remain filtered; these do not replace full feature matrix |
| Compiler2/1/1 in92.47/3.54/10.28s and66.91/2.47/7.30s | Diagnostic contract preserved | Actual compiler cases execute outside excluded workspace group | Exact cases pass; no database evidence |
| Feature jobs eachnone/default/all, both workflows | Existing isolation contracts retained | Observed mnesis15, macros1, store217, wake17; inmemory25/25/30; fjall131 per combination | nostd/store-testing zero-case unit harnesses give compile-only credit; existing CI still omits native Postgres matrix |
| Replay2/1/1 in1s each in both workflows | Existing wire/chunk corpus remains valid | Actual replay entries pass | Finite corpus only, not deep fuzz or transport evidence |
| LibFuzzer wire/header/chunk/value | Decoder/payload invariants retained | Each61s:6,664,578/1,948,617/1,670,254/8,485,201 executions, no reported crash | Finite budgets, no proof of exhaustiveness or native adapter behavior |
| Four AFL++ campaigns | Existing production decoder targets retained | Each60s, CMPLOG observed; final native statistics0saved crashes/0timeouts | Calibration instability warnings retained; zero-exit crash-gate fault proof remains PG9 |
| Mutation diff and gate | No executable production change in this slice | Actual log says No mutants to filter; upload says no files; artifacts API total0 | Zero runtime mutation catches/baseline executions, not a fabricated mutation score; two independent native controls documented above |

This evidence permits protected backup-slice delivery, not full PG7–PG9 or
publication. Runtime TLS source remains uncommitted on the separate branch.

PR391 protected-squash merged49104117b87010206fb4b25be1b4d3d6e6635b22 at
2026-10-19T00:07:36Z. Security branch rebased onto this exact parent, with full
owned WIP including untracked files saved in retained stash3b4e83faef631d3796700f2af1f8cc503f13909e.
Append-only document conflicts were resolved by byte-verifying the old common
prefix in both parent and child and preserving parent plus the complete child
suffix. No source conflict or existing user edit lost.

Combined owning run194PASS/1FAIL/40UNRUN of235,34.971s; strictall/all Clippy
PASS3.86s. THREE reviews: combined native suite must include original backup and
new verified TLS laws; real earlier194cases execute, then TLS fixture fails before
server setup because the invoking bash -lc resets Nix PATH to host LibreSSL
without x509-copy_extensions; wrong-tool setup is zero TLS/whole-suite proof,
not product/certificate failure. Preserve /tmp/mnesis-pg8-tls-combined-native.log
and correct invocation to bash -c retaining Nix OpenSSL/PG/Rust paths. Do not
change cert policy, fixture flags, production code or acceptance to pass.
Audit/deny commands after failed nextest did not execute and get no credit.

Corrected Nix-owned invocation proves current combined source235/235nativePASS
68.032s, zero skips; all five TLS laws actually execute in1.093/1.386/1.896/
1.797/1.429s alongside merged backup/recovery laws. THREE reviews: complete
ordering/outcome/state/restore/security contract retained; actual shared17.10
database and private18.6clusters execute with NixOpenSSL3.5.8 and Rust1.99.0;
exact existing assertions unchanged, no early-return/filtered cases or weaker
verification, first wrong-tool failure retained separately. Raw
/tmp/mnesis-pg8-tls-combined-current.log. This precedes the new example, which
needs its own all-target compilation and unchanged full gate.

Actual cargo audit exits0 with1338advisories loaded and386lock dependencies
scanned; selected TLS packages have no reported vulnerability. THREE reviews:
selected versions must not introduce a known security defect; real advisory-db
fetch/index/scan executes on currentlock; the EXISTING event-listener5.4.1
RUSTSEC-2026-0221 unsound warning remains, so exit0 is not an advisory-free
workspace or completion claim. Existing ignore/policy unchanged; fix belongs to
separate PG9 dependency slice. /tmp/mnesis-pg8-tls-audit.log.

Actual cargo deny check exits0: advisories/bans/licenses/sources all OK under
UNCHANGED current policy. THREE reviews: compatible maintained TLS selection
and existing source/license gates; actual full lock check executes including
native root readers; existing event-listener ignore, unmatched MPL allowance
and duplicate-version warnings remain, no clean-security/absence-of-warning claim
and no policy relaxation. /tmp/mnesis-pg8-tls-deny.log. Rejected CDLA-root
candidate remains rejected; selected22newpackages/208+/12lock lines within budget.

Secure example first strict checkFAIL: large216-byte typed error, stdout/stderr
macros and single-pattern match lints. FormattingPASS; default/Hakari commands
not run. THREE reviews: normal verified connection example must compile under
unchanged lints; actual Clippy reaches the new example and rejects four exact
issues before any native execution; zero example runtime proof or adapter
regression claim. Keep /tmp/mnesis-pg8-tls-final-clippy.log. Box the concrete
PostgresError only on the error path, retaining its type/source; use supported
fallible Write and if-let instead of lint allowances. No test/oracle change.

Example second ClippyFAIL solely equatable_if_let; formatPASS, no subsequent
default/Hakari/native execution. THREE reviews: unchanged lint contract; actual
example compiles far enough to reject unit-pattern predicate style; change only
to Result::is_ok, retain typed source/no-secret output and normal capability.
/tmp/mnesis-pg8-tls-example-clippy.log retained. No allowance or test change.

Current example62/70lines; public connection/grant docs63/120addedlines. Format
PASS, strict adapter all-feature/all-target ClippyPASS1.08s, default/all-target
PASS1.88s, Hakari verifyPASS. THREE reviews for EACH command: unchanged formatting,
warning and generated-dependency contracts; actual example and all adapter target
compilation executes on both configurations and generator verification confirms
managed feature owner; no lint relaxation, native proof, feature-combination or
example execution credit from compilation. All source types/causes retained;
example emits only version or a generic failure, not DSNs/options/passwords.
Raw /tmp/mnesis-pg8-tls-example-current-{format,clippy,default,hakari}.log.
Locked SQLx stream.rs routes both TCP and Unix sockets through MaybeUpgradeTls;
tls.rs Require/VerifyCa/VerifyFull reject native SSL refusal and VerifyFull checks
certificate and hostname. The example uses that exact supported path without
parsing its own URL or silently weakening verification.

Evidence copies under evidence/mnesis-pg8-tls-*.log retain every original red and
current result. Only trailing horizontal whitespace is removed from checked-in
copies; originals in /tmp remain unchanged. Initial git diff whitespace check
correctly rejected those raw copied spaces; normalized copies pass the same
check. THREE reviews: evidence formatting contract only; actual diff checks
execute; no result/exit code/assertion text is changed or test success inferred.
Current Linux integration session64404 running on supported remote aarch64 builder;
source frozen, with TLS smoke and new secure example staged in Nix source.

### TLS signed gate and independent Linux result — 2026-10-19

Signed implementation commit e065a0a4d52f8c2357b59cb94121fee7e98a7cde (G)
passes normal unchanged pre-commit nix flake check, terminal0, no bypass.
Independent aarch64 Linux native package
/nix/store/s1216vcpnykcj10j04n5imj0a3qf6s63-mnesis-postgres-integration
passes235/235 in96.943s, zero skips. THREE reviews: combined existing adapter,
backup/restore and all5verified TLS laws required; actual native PostgreSQL18.6,
OpenSSL and SQLx on Linux execute including exact causes/roles/state and physical
restart; same assertions as Darwin, zero missing DB/early return credit, source
archive includes current security tests. Six separate one-case smokes each pass
in5.539/4.769/4.145/4.881/3.398/7.137s. EACH THREE-reviewed: required capability
exists; actual corresponding native case executes; other tests deliberately
filtered, not six full runs or isolated feature combinations. Source absence is
an error through --no-tests=fail. No native count extrapolation.

Hook nextest actual1575/1575PASS613.939s,15slow,4compiler cases excluded. THREE
reviews: existing workspace contracts preserved; actual default workspace run
executes; +4unconditional TLS registrations return early without DATABASE_URL
and get NO TLS/database credit, fifth state law not in this configuration,
compiler cases require separate CI. All-feature Linux235 is the native proof.
Actual supported replay2/1/1cases PASS1s each. EACH THREE-reviewed: locked decoder
corpus contract; actual replay runs; finite corpus only, no fuzz-exhaustiveness or
native transport claim. All12Nixchecks retained: fmt/toml,Clippy,docs,Hakari,
no_std/store-no_std/wasm,deny,audit,nextest,replay. For EACH compile/check:
contract unchanged; actual selected derivation executes or exact cached output
is reused; compilation/cached result does not mean a fresh native test execution.
Gate derivation map /tmp/mnesis-pg8-tls-gate-derivations.json; actual logs
/tmp/mnesis-pg8-tls-gate-*.log, hook raw /tmp/mnesis-pg8-tls-hook-first.log.

Adversarial inspection retains a pre-existing Nix cargoAudit blind spot: its
registry-index yanked checks print missing-crate errors despite derivation0.
Existing event-listener warning/deny ignore also remains. No clean supply-chain
claim follows Nix0. The separate current live cargo audit earlier actually updated
its registry/advisory DB, scanned386dependencies and reported only that existing
unsound warning; current deny passes unchanged policy. PG9 must repair the
supported Nix index check, remove obsolete ignore and update the vulnerable
existing dependency. These issues do not become waived by this focused TLS PR.
No publication, capacity/durability profile or complete-adapter claim.

### Final PR392 CI review (head03052633, performance failure retained)

All checks terminal:39SUCCESS,1FAILURE(CodSpeed),1scheduledskip; zero unresolved
review threads. No protected merge attempted. Current expected decoder failure
501.7->560ns, all3sizes10.42%regression, actual report
/tmp/mnesis-pr392-performance.json; separate decoder source experiment underway.
CI39181438275/39181441862, fuzz39181441835, mutations39181441919 actual logs
/tmp/mnesis-pr392-{ci,ci-call,fuzz,mutants}.log. For EACH actual run the three
independent reviews below apply, not a generic green-check inference:

| Actual run | Contract | Actual execution/fault | Oracle and limits |
| --- | --- | --- | --- |
| Workspace1575twice145.923/133.904s | Existing workspace laws intact | Actual suite passes,4slow each | Four compiler cases excluded here; DB-free adapter registrations not native evidence |
| Native235fourtimes55.628/60.115/53.884/57.306s | All selected adapter/security/restore laws required | Real database and independent tools execute each time | Zero skips; two echoed integration summaries not additional executions; no unmerged durability laws |
| Twenty-four one-case smokes | Capability binary/test must exist | Each actual atomic/snapshot/projection/list/session/TLS case passes | Other cases filtered; not full isolated-feature coverage |
| Compiler2/1/1 in118.61/14.82/18.21 and110.71/14.45/17.61s | Existing diagnostics enforced | Actual compiler fixture executions | Separate from excluded workspace group, no database credit |
| Each none/default/all feature job in both workflows | Advertised existing crate isolation | mnesis15,macros1,store217,wake17,inmemory25/25/30,fjall131 per config | nostd/store-testing zero-case unit harnesses compile-only; native Postgres feature matrix still PG9 |
| Replay2/1/1each1s in both | Exact existing decoder corpus retained | Each actual replay executes | Finite corpus, no complete-fuzz claim |
| LibFuzzer wire/header/chunk/value61s each | Real existing decoder/value targets |6,441,316/1,812,151/1,221,349/6,125,501iterations, no reported crash | Finite budgets; no native TLS/protocol/coverage exhaustiveness claim |
| Four AFL++60s each | Same actual target bodies and locked builds | CMPLOG confirmed, final statistics0savedcrashes/0timeouts each | Calibration instability warnings retained; zero-exit crash-gate falsifier remains PG9 |
| Mutation diff/gate | No executable production Rust change in TLS slice | Actual No mutants to filter, no upload files, artifacts API0 | ZERO runtime catches/baseline executions, not a fabricated score; independent two TLS runtime controls are separate proof |
| CodSpeed | Original performance gate unchanged | Actual three failures, environment warning retained | No reset/waiver/acknowledgement; cannot count39green as merge readiness |

All original failures and independent control artifacts stay preserved. Evidence
review does not make PG8/PG9 complete or permit publishing an unmerged adapter.

TLS signed rebase onto protected decoderparentf207923a completed at
3fa39a4d0ff8d4b9d6eefac5993d8a6b74b03df0(G). Four record conflicts preserve exact
oldbase plus fullparent+fullchildsuffixes. Mutable progress keeps complete child
snapshot and current parent pointer; originalparent124lines preserved in Git and
/tmp/mnesis-tls-parent-progress-before-rebase.md. Owned saved CI suffix26lines
from stashae465010869acc9a6fe794e85564c199c0494d19 restored by exactprefix check,
stash remains. Adapter/coredocs/manifests/locks/Nix/workflows compare byte-identical
to03052633 EXCEPT protectedwire.rs; wire.rs separately compares EXACT origin/main.
No original TLS implementation/test rewrite, no dependency policy weakening.
Need actual combined native+normal signedfullgate+leasedpush/currentCI before
claiming old CodSpeed failure cleared at this newTLS head.

Combined protecteddecoder+TLS native235/235PASS68.239s,0skips. THREEreviews:
1unchanged verifiedTLS/auth/role/callerpool/cache/checkpoint and previous230adapter
contracts;2actual17.10 maincluster plus private18.6TLS/restore tools run against
3fa39a4d with protecteddecoder source;3same5securityoracles and all old fault
assertions/limits unchanged, no filters/skips, no query5/durability10 added to
this coinciding235count. Prior Linux/TLS source was predecoder; current required
CI must prove combined Linux and CodSpeed before protected delivery.

ActualreleasePR378 now all40SUCCESS/1scheduledskip at
c5ec958221a3f488af3258532e8c8ef2c4480d74, including CodSpeed. APIcompare to
f207923a reports behind0/ahead2 with verified release0.5.0 commit2315ca3d and
verified lock-sync c5ec958, only expected manifests/changelogs/locks16files.
This clears reported releaseperformance failure at its actual head, not final
PG0-PG9 acceptance/publication. Required remaining adapter work still gatesrelease.

Combined normal signed TLS amendment28690 FAILED exit1 during concurrent local
Nix gate disk exhaustion. nextest derivation fails COMPILATION with os28/no space
writing rmeta and object files; ZERO workspace tests execute. Clippy actual strict
compilation finishes45.65s but output archive install fails (derivationexit70).
THREEreviews for each:1all contracts and warnings policy remain required;
2actual compiler succeeds Clippy but build/archive/storage fails, nextest never
reaches execution;3no fullgate or test success inferred, no native/newtestcredit,
original logs retained (/tmp/mnesis-pg8-tls-decoder-{hook,nextest-diskfull,clippy-diskfull}.log).
Prior235native68.239s remains distinct valid evidence, not substitute for gate.
Signed head remains3fa39a4d, failed amendment not pushed. Nix cleared failed temp
builds,22GiB free; serialize query gate before TLS retry, no broad cache deletion
or weakened tests. Existing staging remains owned and preserved.

Serial signed TLS retry34027 completed0, good GPG head
276bbdd2c5530317a81f954052f895f9999a8136. All12 local Nix checks passed; cached
audit explicitly identified. Actual fresh nextest1575/1575PASS536.315s,15slow,
4compiler exclusions; short replay2/1/1PASS each1.00s. THREE reviews for the
workspace and each replay run:1unchanged TLS/auth/role/cache/checkpoint and core
contracts, no new product assertions or policy weakened;2actual nextest/replay
commands execute to terminal, prior disk-full compile failure retained, database
registrations can early-return and add NO native proof;3exact source equivalent
to3fa39a4d across adapters/core/manifests/locks/Nix/workflows, exclusions retained,
short replay not long campaign, combined native23568.239s remains separate. No
query5/durability10 or Linux credit inferred. Logs /tmp/mnesis-pg8-tls-serial-
{hook,nextest,replay}.log. Post-gate notes remain local for next necessary evidence
commit, without another fresh-run claim. Normal exact-lease branch update follows.

TLS actual276bbdd2 CodSpeedcheck117882169089 SUCCESS vs protectedf207923a.
All3 decoder sizes426.4->368.1ns, reported+15.85% simulated efficiency;40untouched
and3skipped-baselinefallback. Syscall/comparison-environment warnings remain in
/tmp/mnesis-pr392-performance-276.md. Same source, inputs, benchmarks and gate;
no threshold change or manual acknowledgement. This clears the old TLS-head
performance failure at the actual new head, not a wall-clock capacity claim.
Mutation job117881036476 run39196739506 SUCCESS has no eligible mutants and
no mutants.out artifact; actual log /tmp/mnesis-pr392-mutants-276.log. THREEreviews:
1TLS functionality still needs native certificate/auth/role laws;2actual tool
finds no eligible production changes, runs no mutant baseline/cases;3ZERO runtime
catches credited, missing artifact explicitly reported rather than invented.
Current native/full CI still required before protected delivery. Postgate notes
stay working-only and preserve current signed source/lease evidence.

TLS current276bbdd2 real-Linux standalone CI jobs117881024861/117881036173
complete235/235PASS51.415s/59.003s,0skips. Each also executes SIX one-case smokes
(atomic/snapshot/projection/listing/session/verifiedTLS), not merely test discovery.
Actual source combines protecteddecoder+TLS, NOT query5/durability10. Downloaded
/tmp/mnesis-pr392-native-276-{push,pr}.log includes actual Pg18.6 init and all five
security cases. Installed run.log repeats each summary; no duplicate-run credit.
THREE reviews for each full suite and each of the twelve smoke commands:
1 Contract: verified certificate+hostname/auth/plaintext/permission boundaries,
caller-owned pools, exact conflict and reopened saved pairs remain alongside the
previous230adapter laws; native tests retain original bad-CA/hostname falsifiers.
2 Execution: real private server/certificates/restricted LOGIN and supported
construction paths execute at this signed head; no guards/skips in full suite,
smokes use no-tests=fail and actual TLS owner. Prior oldhead failure retained.
3 Assertions/source/limits: unchanged five native laws, exact typed error causes,
settled permissions/state assertions and real Barrier/spawn conflict retained;
no Debug-password-redaction or CA-file-only-trust claim. No projected240/245total
from sibling suites and no complete feature/native matrix claim. Current remaining
Nix CI, fresh final integration parent and protected delivery remain required.

TLS fuzz run39196739408 SUCCESS (raw /tmp/mnesis-pr392-fuzz-276.log). THREEreviews
for each of eight campaigns:1same production wire/CBOR/value owners, no database
coverage inferred;2actual four61s LibFuzzer counts wire14,156,536/header1,803,711/
chunk1,203,945/value7,580,337, and four60s AFL campaigns each CMPLOG confirmed,
0savedcrashes/0savedtimeouts; discoveries0/2/13/0 respectively;3unchanged target
wrappers/input workload and preserved calibration/instrumentation/environment
warnings, no runtime mutation credit and explicit AFL crash-gate falsifier still
PG9. These campaign results do not certify TLS or final whole-adapter delivery.

TLS stacking pre-verification: owned postgate notes saved in stash
9a3d9449e8fc0a47356e00c19986570ba67fafea (retained, not popped), full originals
and patch /tmp/mnesis-tls-stack.jxcsdf. Rebase old276bbdd2 from protectedf207923a
onto query4ac45d46 has ONLY documentation conflicts. For experiments/hypotheses/
sources/research, both child records compare exact common base byte prefixes;
result retains complete query record followed by TLS-only suffix. Exact oldTLS
prefix also verified before restoring its entire working postgate suffix.
Progress records retained in full with an authoritative current pointer. PRD
keeps reopened isolation acceptance and TLS progress; duplicate PR390/391 labels
removed without dropping facts. No immutable contract or original evidence edits.
Product diff from oldTLS276 equals EXACT query diff f207->4ac across all adapters,
core, manifests/locks, Nix and workflows (cmpPASS). No guessed source union.
Normal signed rebase then real combined native and normal signed full gate follow;
old235/235 sibling counts are not a combined240 result. Saved goal remainsblocked,
ordinary authorized work continues; no safeguard/goal-storage manipulation.

Query post-gate evidence restored after exact4ac prefix verification:

PR394 original95a00bc1 push native job117877643172 now downloaded successfully:
complete235/235PASS55.220s,0skips; five one-case smokes0.185/0.061/0.144/0.151/
0.071s. THREEreviews for each command:1same atomic/snapshot/projection/listing/
session and235 original contracts;2native Pg18.6 hook actually executes each
selected owner, required database present;3fullsuite0skips, smokes explicitly
filtered and installed log echo is not a second run. Raw
/tmp/mnesis-pr394-push-postgres.log. These are BEFORE the4linecleanup correction;
no current-source mutation independence inferred. Full current-source native
78.426s above remains separate. Original95a00bc1 CodSpeed actualcheck117879363243
is SUCCESS vs protectedf207923a:43untouched/3skippedbaselinefallback,17syscall
warnings retained. This simulation measures owning Fjall benchmarks and supplies
no Postgres wall-clock/SQL capacity proof. Current amended head needs its own CI.

Original95a00bc1 PR394 CI now terminal40SUCCESS/1scheduledSKIP, zero review
threads. Both whole logs downloaded: /tmp/mnesis-pr394-ci-{push,pr}.log, runs
39195623984/39195637612. This result is BEFORE cleanup correction and the raw
mutation verdict remains rejected, so it does NOT authorize merging oldsource.
THREE independent reviews of each additional completed command:
1 Contract: same workspace, native database and per-feature owning laws remain;
compiler fixtures retain exact diagnostics and valid/invalid literal controls.
2 Execution: both Nix workspace suites execute1573 each; each workflow's full
Nix gate additionally executes235 native tests beyond its standalone integration
job. Actual PG starts/nextest summaries are visible, not inferred from green.
PR compiler2/1/1PASS88.70/3.40/10.20s, no ignored cases. PR lib matrix repeats
kernel15/15/15, macros1/1/1, store217/217/217, wake17/17/17, inmemory25/25/30,
Fjall131/131/131, nostd0/0/0, store-testing0/0/0. Three replay suites per Nix gate
are2/1/1, each1s. Full selected native suites have0skips.
3 Assertions/source/limits: exact old95 source selected; four compiler exclusions
remain in workspace suites and covered separately. Native log echoes are not
extra executions; filtered one-case smokes are not full feature matrix. Zero-test
harnesses remain compile-only, Postgres absent from feature job remains PG9, and
none of this oldhead evidence proves corrected mutation independence. No waiver,
fixture-poison credit, missing test count or benchmark threshold change accepted.

Cleanup signed fullgate53526 completed0 at GOOD GPG
4ac45d4676fda35cfd36ff7b6ca5929643df6ecc. All12local checks pass. Fresh workspace
1573/1573PASS498.736s,14slow,4compiler exclusions; short replay2/1/1PASS each1s.
THREE independent reviews of workspace and each replay command:1original storage,
order/resume/atomicity/wire invariants unchanged, cleanup retains actual overlap
and all final oracles;2actual complete nextest and replay commands execute, guards
still add NO native credit, previous NoSpace and contaminated CI results retained;
3executablesource diff against95 matches frozen pre-gate SHA256
ede683bd9e62cb38441a43d53235b45f59c3ff87addaad6b91b65432e3b7de06 exactly.
Only4testline cleanup beyond prior13production-line repair; no new counts,
exclusions/filters/workload/policy/dependencies or framework. Compiler cases remain
separate. Good signature and fullbase whitespace verified. Logs
/tmp/mnesis-pg4-cleanup-{hook,nextest,replay}.log. Latest post-stage95CI notes and
this gate review remain working-only for next necessary evidence commit. Normal
leased push updates PR394; current-head independent native/mutation CI still due.

Actual combined TLS+query native96829exit0 on signed59290a0e:240/240PASS87.470s,
0skips. THREE independent reviews:1all previous230adapter laws plus actualfive
names/fiveTLS cases execute together, preserving order/resume/atomicity/uncertainty
and exact saved-pair semantics;2real Pg17.10 shared cluster and private18.6TLS/
recovery tools execute all selected cases serially, corrected ordering fixture
included, no source stub or no-DB early-return credit;3exact union cmp against
oldTLS276 and queryf207->4ac PASS, original assertions/Barriers/faults unchanged,
0skip/0filter and no durability10 count added. Formatting passes before run.
This is the first actual240 combined proof, not sibling total arithmetic.
Native validation timings are not capacity targets; earlier41.777s import fault
case deserves PG8 latency/lifecycle investigation before deployment claims.

Normal signed full gate follows with unchanged hook/checks/test workloads. Per-run
Nix max-jobs2 bounds simultaneous derivation builds to preserve disk margin after
the retained real NoSpace failures; no test concurrency/case count/filter/deadline
is weakened. Current14GiBfree, no other local fullgate/native job active. Actual
combined-source CI and independently reviewed mutation evidence remain required.

TLS old276bbdd2 CI now terminal40SUCCESS/1scheduledSKIP,0reviewthreads. Actual
whole logs /tmp/mnesis-pr392-ci-276-{push,pr}.log, workflows39196735894/39196739404.
This is BEFORE stacking query4ac, not current59290 combined-source CI. THREEreviews
for each actual command run:1unchanged owning kernel/store/wake/Fjall/inmemory and
TLS/native order/atomicity/saved-pair contracts, compiler fixture diagnostics intact;
2fresh workspace1575PASS112.033/133.373s (3/4slow), plus both Nix gates' native235
PASS53.685/56.444s0skip in addition to standalone51.415/59.003s, each with SIX
one-case capability/TLS smokes. Two compiler groups2/1/1PASS127.82/16.37/19.49s
and103.11/13.34/15.85s. Replay2/1/1each1s in each gate. Feature lib commands in
BOTH workflows execute default/no-default/all:kernel15/15/15,macros1/1/1,
store217/217/217,wake17/17/17,inmemory25/25/30,Fjall131/131/131,nostd0/0/0,
store-testing0/0/0;3zero test harnesses compile-only, four compiler exclusions
retained in workspace and separately covered, filtered smokes not full feature
matrix, echoed installed logs not extra executions, native235 has0skips and no
query5/durability10. Current combined240native proof is separate; existing feature
job omitsPostgres and remains PG9. No waiver, fixture-failure credit or publication
inferred from old276 CI40green. Latest old-source review remains working-only
while combined signed fullgate31558 has frozen index/source.

PG8 timing clarification for later quiescent profile: native import_storage_error
case measured12.373s(oldTLS decoder),15.282s(oldquery decoder),41.777s(cleanup235)
and27.896s(combined240), while full suites pass. The41.777s run overlapped a local
full gate; contention was observed, but its causal contribution is NOT established.
Do not label this a proven adapter regression, dismiss it as infrastructure, tune
a deadline, or turn these validation timings into capacity targets. Existing PG8
lifecycle/latency evidence must identify the wait using supported database/pool
statistics at equal durability. No source/test/budget change follows this note.

Combined TLS+query signed fullgate31558 completed0 at GOOD GPG
0ac0c21cfcc3f0f3e81b9a183131ad422b4b4ff6. All12checks pass; audit/tomlfmt cached
explicitly. Fresh1577/1577PASS476.059s,14slow,4compiler exclusions; replay2/1/1
PASS1.01/1.03/1.00s. THREE independent reviews of workspace and each replay:
1same core/order/atomicity/identity/head/TLS/cache/checkpoint contracts, unchanged
fault/oracle owners;2actual nextest/replay execute together on combined source,
no DBguard nativecredit and no use of sibling totals;3all product/manifests/locks/
Nix/workflows byteequal59290a0e, fullbasewhitespace and goodGPG pass. Four compiler
exclusions remain separately covered by CI; replay short, not fuzz campaign.
Actual240native87.470s above remains independent. Per-run Nixmaxjobs2 changes
scheduler only; no test selection/concurrency/budgets relaxed. Original NoSpace
and contaminated mutation evidence retained. Raw
/tmp/mnesis-pg8-tls-queries-{hook,nextest,replay}.log. Query4ac protected-merged
as5ef3e742 tree-identical4ac; now rebase only parent metadata onto that protected
commit, verify entiretree equal before restoring postgate notes, then normal
cached signed gate. No fresh-run credit for later cached checks.

Additional query CI/mutation/protected delivery evidence, exact saved-prefix delta:

PR394 corrected4ac mutation run39197816436 now provides artifact12163146175,
SHA256edc92fc8eb44985e834ffc47bb6fab0adacabacec4e92a1c322205f43c8b9410;
actual downloaded ZIP digest VERIFIED, files /tmp/mnesis-pr394-mutants-4ac-reviewed.
Baseline235/235PASS50.084s,0skips.16mutants:14real runtime catches,2compiler
unviable,0survivors/0timeouts. ALL16diffs and each actual FIRST failure reviewed;
NO schema-already-exists error anywhere. Original contaminated artifact remains
rejected, unchanged and separately retained.
THREE independent reviews for baseline and EACH case:

| Cases | 1 Contract | 2 Actual execution/falsifier | 3 Assertions/source/limits |
| --- | --- | --- | --- |
| Baseline235 | Complete native adapter contract. | Actual real-DB235 pass50.084s. | Zero skips; unchanged4acsource, not1573 guarded unit proof. |
| read_key None | Active native drain cannot finish while blocked on its real view. | Runtime assertion rejects prematurely completed empty cursor at backup_tests543. | Actual production function replaced, no fixture/schema error. |
| read_key Some0/Some-1 | Import must preserve final-conflict policy reports. | Each runtime test rejects fabricated empty successful ImportReport at backup_tests745. | Both cases independently reach their own wrong result; no poison credit. |
| read_key Some1 | Restored stream bodies must match exact source bytes. | Runtime payload-pair equality fails at backup_tests779. | Exact differing byte payloads visible, not count-only validation. |
| read_head None | Real optimistic conflict retains its typed classification. | Runtime atomic conformance rejects Store23505 instead of Conflict. | Actual owner at atomic.rs135, no fixture bootstrap failure. |
| read_head (0,None)/(-1,None) | Overlapping valid append must succeed with its real key. | Both independently reach own FK23503, keys0/-1 absent. | Result unwrap at all_noskip207 occurs AFTER DROP; no42P06. |
| read_head six Some(empty/xyzzy) variants across0/1/-1 | Valid append must preserve real stored head interpretation. | Each independently returns CorruptRow invalid stored version and fails after cleanup. | All SIX first failure logs inspected, same production fault/unchanged final oracle. |
| read_head (1,None) | Global event order includes exact originating stream identity. | Runtime exact row equality fails at all_noskip212. | Real overlapping writes survive through cleanup; own payload/routing oracle, no timeout. |
| snapshot hydrate Default / checkpoint hydrate Default | Concrete found/absent/stale contracts remain typed. | Both compile E0277 because Hydrated/CheckpointHydrated lack Default. | TWO UNVIABLE, ZERO runtime catch credit. |

Fail-fast means later native cases do not execute in each failing mutant; no235
passes per mutant are claimed. Mutation tool does not mutate SQL string names;
original five native caller-name failures remain the direct qualifier regressions.
Corrected cleanup only repairs case independence; no test/gate/filter/corpus/timeout
or mutation exclusion change. Remaining exact-head full CI/CodSpeed/review and
protected merge still required.

PR394 final4ac45d46 CI40SUCCESS/1scheduledSKIP,0unresolvedthreads, CLEAN merge
state. Complete current logs /tmp/mnesis-pr394-ci-4ac-{push,pr}.log, workflows
39197812041/39197816451. Actual per-run THREEreviews:

| Actual executions | 1 Contract | 2 Execution/falsifier | 3 Assertions/selection/limits |
| --- | --- | --- | --- |
| Workspace1573PASS120.934s/91.622s | All prior workspace laws retained. | Two fresh Nix suites,4/3slow; no failure/timeout. | Four compiler exclusions each, guarded PG tests NOT native credit. |
| Native235PASS42.599/52.407 standalone,51.185/42.917 Nix | All adapter order/resume/identity/head/state/atomic/recovery laws. | FOUR actual Pg18.6 native suites, each five names+four ordering cases and0skips. | Source4ac includes cleanup; echoed run.log not another execution; noTLS/durability total. |
| Twenty one-case capability smokes | Atomic/snapshot/projection/listing/session owners. | Five no-tests=fail selected commands in each of four native derivations. | Explicitly filtered smoke evidence, not isolated default/each-feature matrix. |
| Compiler2/1/1PASS76.34/2.81/8.61s and85.45/3.14/9.66s | Invalid construction/schema/literal/borrow rejection unchanged. | Actual fixtures all ok, no snapshot blessing. | Registration counts separate from fixture counts,0ignored/filter. |
| Feature lib default/no-default/all in BOTH workflows:kernel15/15/15,macros1/1/1,store217/217/217,wake17/17/17,inmemory25/25/30,Fjall131/131/131,nostd0/0/0,store-testing0/0/0 | Same concrete owner contracts under advertised library selections. | Every reported harness result verified in full logs. | Zero-test harnesses compile-only; PG absent here remainsPG9, no fake matrix completion. |
| Replay2/1/1each1s in BOTH Nix gates | Wire/CBOR/value contracts. | Six actual short owner invocations execute. | Short replay, not eight long campaigns or DB proof. |

Corrected4ac fuzz39197816495 SUCCESS; THREEreviews of each of eight campaigns:
1unchanged production wire/CBOR/value targets;2actual61s LibFuzzer wire7,522,520/
header2,343,907/chunk2,112,044/value5,784,943 runs; four60s AFL eachCMPLOG confirmed,
0savedcrashes/timeouts, discoveries0/1/9/0;3calibration/instrumentation warnings
retained in /tmp/mnesis-pr394-fuzz-4ac.log, no SQL/TLS/native claims, explicit
zero-exit AFL crash-gate faultproof remainsPG9. CurrentCodSpeed117885258078
SUCCESS at4ac vs protectedf207,43untouched/3skippedbaselinefallback; syscall/
environment warnings preserved in -performance-4ac.md, no Postgres capacity claim.
Mutation artifact12163146175 independently reviewed above, no fixture-failure
credit. Exact protected squash with match-head4ac follows; wholePG0-PG9 delivery,
publication and registry consumer still incomplete.

PR394 protected delivery COMPLETE: normal squash --match-head-commit
4ac45d4676fda35cfd36ff7b6ca5929643df6ecc merged2026-10-19T04:25:55Z as
5ef3e742c63a2f4078ec28057559b5439af28234. Before action: exactsignedhead,
40SUCCESS/1scheduledmutants-sweepSKIP, CLEAN state,0unresolvedthreads; actual
current CI/fuzz/CodSpeed/artifact reviews above. No admin/waiver/baseline reset.
Fetched protected merge compares ENTIRE TREE IDENTICAL to reviewed4ac (gitdiff0).
Latest /tmp/mnesis-pr394-delivery.json and protected-checks/protected-threads
snapshots retain exact evidence. This is17th focused protected merge, not full
Postgres acceptance or publication. Remaining byte/integer/catalog hypotheses,
TLS/durability/operations/capacity/recovery timing/PG9 delivery remain required.

Protected-parent rebase completed GOOD GPGf95f9451694e905ea2f85a58742823cd33d5c7e3
onto5ef3e742. ENTIRE repository tree compares equal to tested0ac0c21c before
restoring records; only parent/signature metadata differs. Owned postgate stash
7f06e58bcb8d1212b1a9cf4d98a3d8393e32d55e retained with snapshot/patch
/tmp/mnesis-tls-delivery-rebase-path. Restored postgate TLS evidence and latest
query mutation/CI/merge suffix only after exact prefix comparisons. Backlog now
records protected PR394 without closing remaining query policy/PG8/PG9 acceptance.
No product/manifests/locks/Nix/workflow change. Normal signed evidence amendment
uses unchanged full hook; cached checks must be labelled0fresh executions.

## PG8 durability original witness — 2026-10-19

Actual original-source runtime-open witnessFAIL0.047s, one case, zero skips.
THREE reviews: asynchronous commits must not silently inherit the durable runtime
profile; healthy explicit-on native SHOW and actual migrated store open succeed,
explicit-off native SHOW confirms off and the SAME from_pool succeeds incorrectly,
so expected rejection fails; this is actual SUT acceptance, not missing database,
setup, compiler, crash or data-loss evidence. /tmp/mnesis-pg8-durability-original.log
retained. Initial coarse rejection oracle is characterization only; final typed
setting error and no-write assertions are required before acceptance.

Five-law extension initially does not compile: three envelope builders omitted
required typestate event_type/payload steps (E0599). THREE reviews: real valid
writes must be attempted after replacement; zero native execution because
compiler rejects fixture construction; use the established complete envelope
builder, no SUT change or defect credit. Keep
/tmp/mnesis-pg8-durability-five-original.log. First startup-only native red remains
valid; this failed compile cannot substitute for replaced-connection evidence.

Current original five laws all execute and FAIL0.229s, zero skips. THREE reviews
for EACH run/case: durable runtime startup and each acknowledged write must reject
unsupported async settings; healthy native migrated control and acknowledged
seed append succeed, native LISTEN is established, capacity2 has one data slot,
SQLx future options are changed, original connection is closed and native new PID
is distinct with synchronous_commit exactlyoff; actual SUT then incorrectly
returns success for append0.063s/atomic0.040s/checkpoint0.035s/snapshot0.042s
(startup0.048s), causing each corresponding rejection expectation to fail. No
compiler/setup/timeout/connection error counted as runtime proof. Final-green
oracles still require exact typed setting errors and independent settled database
contents; baseline uses coarse expected-rejection only to show absent protection.
Raw /tmp/mnesis-pg8-durability-five-current-original.log retained. No production
change preceded these observed reds, and no data-loss-on-crash claim follows.

Native profile probe returns exact on|on|on for fsync/full_page_writes/
synchronous_commit. THREE reviews: establish selected server settings before
profile experiment; actual supported psql query executes on native17.10; this
is configuration observation, not crash/flush/power-loss verification. Raw
/tmp/mnesis-pg8-durability-native-profile.log. First fix ClippyFAIL shadow_reuse;
formatPASS, zero native execution. THREE reviews: unchanged warning contract;
actual private validation owner rejected before tests; rename only the extracted
unsafe-setting binding, no allow/assertion change. Keep
/tmp/mnesis-pg8-durability-fixed-clippy.log.

Second fix check formatPASS, ClippyFAIL only fixture option shadowing and
err().expect() on Debug unit result; zero native execution. THREE reviews:
unchanged warning/test contracts; actual compiler/linter rejects these exact
fixture-only issues after private production guard compiles; rename runtime
options and use expect_err for unit result, no lint/behavior/assertion weakening.
/tmp/mnesis-pg8-durability-current-clippy.log retained.

Current five durability lawsPASS1.951s, zero skips, strictall/all ClippyPASS2.37s
and formatPASS. THREE reviews for EACH check/case: startup and all four write
boundaries must reject exact synchronous_commit configuration; actual replacement
backend/native settings/healthy seed execute, and every write returns the exact
Durability setting through its own wrapper; after dropping the store and waiting
for caller-pool closure, independent admin reads assert exact (events,keys,caches,
checkpoints)=(1,1,0,0), original native transaction/sequence and payload unchanged.
No arbitrary failure, unknown commit, skipped case or uncommitted-only read counts
as acceptance. Compile-only checks provide no extra database credit. Raw
/tmp/mnesis-pg8-durability-verified-{clippy,five}.log. Existing PG3 regression suite,
unsafe server profiles, stronger modes, trigger changes and omission controls
still required; five laws do not complete durability acceptance.

Seven durability lawsPASS13.893s, zero skips; strictall/all ClippyPASS2.42s,
default/all-target ClippyPASS6.91s and formattingPASS. THREE reviews for EACH
command/case: original five exact rejection/settled-state laws retained, plus
all four non-off modes must remain unchanged and a native AFTER-INSERT setting
change must reject before COMMIT; actual new-backend/healthy seed controls execute,
four actual mode writes acknowledge and retain their exact mode and native row
map after pool closure, trigger uses transaction-local off and exact wrapper
rejects with subsequent native setting restoredon; no replication/power-loss
claim, arbitrary error, skipped case, no-fail false green, or lint relaxation.
All prior reds remain. Sources test354/450lines, private product32/80 and existing
source29/35churn. Raw /tmp/mnesis-pg8-durability-{seven-native,seven-clippy,default-clippy}.log.
Fresh independent ignore-check and commit-then-return-rejection controls will
challenge both error and settled-storage assertions before full acceptance.

Fresh isolated ignore-check control actuallyFAIL1.034s, one case/six filtered.
THREE reviews: replacement async append must return exact durable-profile error;
ONLY private commit guard's validation result is deliberately ignored, startup
checks remain, actual native replacement/off controls pass and SUT returns
Ok(position); runtime assertion fails at exact returned result, not compilation,
setup, timeout or synthetic model. Correct source unchanged. Separate detached
/tmp/mnesis-durability-controls.7o1ffu, proof target package-clean before run;
/tmp/mnesis-pg8-durability-ignore-control.log retained. No mutation percentage.
Fresh commit-then-return-rejection control actuallyFAIL1.109s, one case/six
filtered. THREE reviews: a definite configuration rejection must commit nothing;
ONLY private commit owner retains the validation error, nevertheless commits,
and returns that error after COMMIT; exact error/wrapper assertion PASSES, then
after pool cleanup the independent native oracle finds (2,2,0,0) instead of
(1,1,0,0), so runtime storage assertion rejects the false rejection. This is
actual native committed data despite a matching error, not compile/setup failure
or a mirrored implementation test. Correct source unchanged. Raw
/tmp/mnesis-pg8-durability-false-rejection.log retained. This directly falsifies
returning a safe-looking error after successful commit; no retry-safety weakening.

Locked Nix postgresqlTestHook inspected through exact evaluated derivation: it
initializes native default settings and appends postgresqlExtraSettings, with
no fsync-off override. Earlier concern about an implicit unsafe fast profile
is disproved for this selected hook. No Nix config edit needed; native actual
server-profile/restart tests still required. No source inference gets runtime
flush/crash credit.

Startup-only control initiallyFAIL0.047s, one case/six filtered, at exact runtime
startup rejection expectation. THREE reviews: runtime startup must reject off;
actual healthy native on/migrated open then off SHOW execute and unsafe open
succeeds; however invocation ran in control checkout and attempted self-copies,
leaving the earlier commit-then-error mutation alongside startup-check omission.
That commit path is not called by this test, but this is NOT clean single-change
control evidence. Keep /tmp/mnesis-pg8-durability-startup-control.log and rerun
with exact correct commit owner restored from absolute product path. Correct
product checkout unaffected. Record setup mistake rather than credit or erase it.

Clean startup-only control actuallyFAIL0.086s, one case/six filtered. THREE
reviews: unsupported startup profile must reject; exact correct private commit
owner restored byte-for-byte, diff verifies ONLY startup validator result is
ignored, real healthy on/open and native off controls pass then unsafe open
succeeds; failure is exact expected-rejection assertion after native execution,
not compile/setup/timeout or leftover commit behavior. Previous mixed-setup run
is retained separately and gets no clean-control credit. Raw
/tmp/mnesis-pg8-durability-startup-clean-control.log. Control worktree intentionally
retains ONLY this startup mutant, correct product source unchanged.

Server/restart extension formatPASS then strict ClippyFAIL solely90/80line
restart test; zero new native cases executed. THREE reviews: process-crash and
unsafe profile contract retained; actual linter prevents native run; extract
native immediate-stop/status assertions into one private lifecycle helper and
keep each state capability's save/check helpers feature-local to avoid unused
position obligations in snapshot-only builds. No lint relaxation, new public seam
or hidden test skip. /tmp/mnesis-pg8-durability-restart-clippy.log retained.

Restart helper revision formatPASS, ClippyFAIL at81/80lines, zero native/default/
snapshot-only cases run. THREE reviews: all restart/state assertions retained;
actual linter counts remaining testbody; remove a one-use envelope local by
constructing the same envelope at append, no compressed formatting, allowance,
assertion removal or capability skip. Keep
/tmp/mnesis-pg8-durability-nine-current-clippy.log. Shared helper/state separation
remains meaningful and cfg-local, not an alternate runtime/framework.

Nine durability lawsPASS26.030s,0skips. Strictall/all Clippy1.20s, default/all-target
0.31s, snapshot-only durability target1.33s and formattingPASS. THREE reviews for
EACH command/case: prior seven invariants retained plus unsafe native server
settings and process-crash persistence; actual independent18.6servers first
migrate healthy, restart with fsyncoff/full_page_writesoff, SHOW exactoff, runtime
AND migrator reject exact namedsetting while versions1–5 stay; immediate -m
shutdown actually succeeds and status3 precedes restart, acknowledged event,
history, snapshot and checkpoint pair reopen exactly and later append/resume
advance. All observable assertions remain, no genericerror/skip/setup credit or
hardware/failover claim; compilation-only configurations are not native feature
coverage. Test591/600lines plus exact reused shared66line cluster owner; actual
source budgets unchanged. /tmp/mnesis-pg8-durability-nine-verified-*.log retained.

New guard audit witness actuallyFAIL0.063s, one case/nine filtered, expected
rejection sees actualOk(position). THREE reviews: caller search_path must not
redirect durability facts; actual existing healthy startup/acknowledged seed and
replacement off controls pass, native text equality is proven redirectedfalse,
then actual SUT wrongly commits while only guard comparisons are overridden;
cleanup drops injected schema BEFORE assertion, so failure is neither poisoned
schema validation nor setup/SQLsyntax/generic error. Current candidate guard
introduced this unqualified-operator gap; do not accept its earlier9greens as
completion. /tmp/mnesis-pg8-durability-path-original.log retained. Minimal fix
qualifies exactly three equality operators, within revised35source churn.

Qualified-guard formatPASS then ClippyFAIL fixture shadow_unrelated: native
setting local and match field both named setting. THREE reviews: exact hostile
path oracle unchanged; compiler/linter rejects naming before native execution;
rename only native-control local, no policy/oracle/lint weakening. Original
runtime red remains valid, but zero fixed/native credit yet. Keep
/tmp/mnesis-pg8-durability-ten-clippy.log. Actual source budgets remain private32
lines (operator qualification changes bytes inside this NEW file), existing
18+/11-=29churn; previous forecast35 conservatively counted qualifications as
existing lines, not actual growth.

Qualified guard ten-law runPASS35.565s,0skips; strictall/all ClippyPASS2.84s,
formatPASS. THREE reviews for EACH: all previous startup/replacement/mode/trigger/
server/crash/settled-state invariants plus hostile text comparison; actual native
on/off/newPID and injectedoperatorfalse controls execute, exact Durability wrapper
rejects and settled independent rows remain unchanged; no filtered/earlyreturn/
arbitraryerror/lint waiver or power-loss credit. New operator gap is corrected
before delivery, original red preserved. /tmp/mnesis-pg8-durability-ten-current-*.log.
Source budget32private lines and existing29churn,629/650testlines +reused66helper.
Need current source default/state feature native checks, full regression and cost
comparison, protected TLS parent rebase/complete Linux/gate/PR delivery.

Full current durability branch native240/240PASS105.061s, zero skips. THREE
reviews: all existing adapter/order/commit-uncertainty/migration/atomic/state/
backup/restore laws plus ten durability laws retained; actual17.10shared database
and independent18.6servers execute complete all-feature set; no filtered cases,
no skipped database, original commit-loss/timeout classification regressions
still pass, no TLS laws on THIS parent (TLS235 is a separate source and must not
be added as if combined). New source remains uncommitted. Raw
/tmp/mnesis-pg8-durability-full-native.log. Final TLS/decoder parent rebase still
requires combined-source native/Linux/gate and protected delivery.

Deferred-trigger original attempt1 exited2 in bash before nextest: unquoted
filter parentheses are shell syntax. THREE reviews: no durability contract
execution; no native trigger/fault or mutation catch; format command succeeded
but ZERO tests started. Preserve /tmp/mnesis-pg8-durability-deferred-original-call.log
and use quoted supported nextest expression in separate attempt2. Production
module remains byte-identical to /tmp/mnesis-durability-predeferred.rs.

Deferred-trigger attempt2 exits101 at compilation, ZERO native tests. THREE
reviews: no durability invariant exercised; no immediate/deferred native catch;
SQLx0.9 intentionally rejects dynamic format String as SqlSafeStr. Keep literal
function SQL and two full static trigger statements, not AssertSqlSafe bypass.
Preserve /tmp/mnesis-pg8-durability-deferred2-original.log. Product guard unchanged.

Deferred-trigger attempt3 ACTUAL native1FAIL4.554s,9filtered/0pass. THREEreviews:
1. A claimed precommit durability rejection must not acknowledge or persist an
   asynchronous event. Settled independent counts are2events/2keys instead of
   healthy1/1; cache/checkpoint counts stay0. Current precommit guard is insufficient.
2. Actual healthy seed+replacement backend/off control then on mode, real AFTER
   INSERT trigger, later DEFERRABLE INITIALLY DEFERRED constraint trigger. Existing
   immediate schedule completes before the second deferred schedule in the same
   registered law. Native schema cleanup precedes independent settlement; store
   dropped and caller pool closed before querying rows. Product module compared
   byte-identical to saved predeferred source. This is actual storage failure,
   not compiler/unviable mutation evidence.
3. Exact settled counts fail BEFORE typed rejection oracle, so a safe-looking
   returned error cannot hide a committed write. Error/setting checks remain,
   no weakened expectations.9other laws filtered, not passing/nativefullcredit.
   Deferred phase attribution follows sequential source/control execution; raw
   failure labels the shared settled-count helper, not a distinct named test.
   Preserve raw /tmp/mnesis-pg8-durability-deferred3-original.log.

Primary PostgreSQL18 SET CONSTRAINTS docs (accessed2026-10-19) state that switching
ALL to IMMEDIATE retroactively executes pending deferred constraint triggers in
this owned transaction; CREATE TRIGGER docs describe commit-time constraint
triggers. Use native supported statement before final setting validation, after
all write statements. No changes to COMMIT's existing uncertainty mapping, no
configuration override, and no changes outside this transaction. Additional
native roundtrip cost must be measured in PG8 equal-durability capacity profile.

Deferred-trigger fixed focused run10/10PASS89.180s,1slow,0skips. THREEreviews:
1all exact durability startup/replacement/writer/profile/crash laws remain,
immediate+deferred schedule now both preserve independent settled1/1/0/0 counts
and exact Durability{synchronous_commit} rejection with restoredon session;
2real17.10 application cluster and private18.6 restart/processcrash tools execute,
native SET CONSTRAINTS precedes final profile validation in owned transaction;
3existing error mappings/positive4commitmodes/original typed sources unchanged,
no timeout/skip/filteredcase accepted as passing, deferred actualred retained.
This is focused10law proof only; final fullsuite/combinedTLS/currentLinux, full
gate and delivery remain. New native roundtrip cost must be measured, not assumed.

Deferred-trigger retained source strict all-feature/all-target ClippyPASS6.09s.
THREEreviews:1all four write-owner error/public contracts remain;2pinnedcompiler
checks actual native static SQL query and typed Transaction ownership;3no lint
allowances/threshold changes, zero native behavior credit from compilation.
Actual budgets37/80new private production lines,634/650native-testlines and
existing18added/11removed=29churn<=35. Native original storage red and fixed10
laws recorded above; prior full240 predates deferred-drain addition, so never
claim current full240/combinedTLS/query/Linux/protected delivery from it.

TLS final protected-parent cached-gate proof restored by exact parent prefix:

Final protected-parent signed evidence amend34064 completed0 at GOOD GPG
f2cfeaa186f08282201d56a30791bb92a7501471. All12checks are PREVIOUSLY BUILT,
ZERO fresh test runs. THREEreviews:1unchanged complete contracts;2actual hook
executes and reuses exact prior derivations, no invented fresh/native pass;
3all product/test/manifests/locks/Nix/workflows byteequal tested0ac0c21c,
fullbasewhitespace and GPG pass, latest only records/status changes. Prior actual
combined240native87.470s and1577workspace476.059s remain the behavioral proof;
current-head requiredCI must execute or explicitly disclose cache reuse. Normal
exactlease againstremote276bbdd2 follows. No protected TLS merge/publication yet.

Durability integration pre-run: original tracked+untracked work preserved in
stashb2e13e44d4288a2fc4711302c9c5c6585a0d5054 (retained), snapshot
/tmp/mnesis-durability-integration.ljilXn. Branch moved from49104117 onto tested
TLS/queryf2cfeaa1 while its finalCI runs; prerequisite must merge before durability
delivery. Stash apply conflicts ONLY in five records, no executable conflict.
Four append records merged after exact491base prefix comparisons on both sides;
complete old progress and parent progress retained. Existing cluster helper now
owned by TLSparent and compares byte-identical, no duplicate new helper credit.
New private durability.rs37lines and durability_tests.rs634lines compare exact
saved candidate; existing seven Rust owners18+/11-=29churn, unchanged budgets
80private/35existing/650tests. Full merged patch reread: four writes call SAME
precommit drain/guard and original commit-uncertainty mapper, startup+migrate guard,
no modified native lookup repair/TLS/core/manifest/lock/Nix/workflow. No new method,
dependency, framework, runtime or test assertion. Actual combined fullsuite now
required; old focused10pass and old240predeferred suite are not combined proof.

Integrated durability full-native41420 FAILED exit100:174/250 executed,
173PASS/1FAIL39.183s,0skips,76not run duefail-fast. Original unchanged owner
server_rejections_at_commit_preserve_the_code_and_outcome fails0.138s: deferred
SQLSTATE40003 is returned as Sqlx instead of CommitUnknown. THREEreviews:
1PG3 explicitly forbids treating completion-unknown as verified rollback and all
old uncertainty assertions remain required;2actual real-server deferred trigger
now executes during new SET CONSTRAINTS finalization, before explicit COMMIT,
independent persisted rows0 and exact40003 retained;3this is an integration defect
in new error mapping, not proof of lost COMMIT acknowledgement or a test to weaken.
Actual TCP after-server-commit suppression/timeout owners remain separate. Original
/tmp/mnesis-pg8-durability-integrated-native.log retained; no250pass claim.

Integration correction keeps the unchanged four-code oracle and TCP commit-loss/
timeout laws. Deferred drain now reuses existing PostgresError::commit finalization
policy; no new classifier, cause erasure, retry or excluded test. This is explicitly
CONSERVATIVE before literal COMMIT: an unknown finalization error never proves a
commit occurred, and cannot become a safe retry merely because checks moved.
Public error docs now state this boundary. Existing-owner churn34/35 with two
replaced doc lines/three new; private module37/80 unchanged. No assertion or
workload changed. This tradeoff may require reconciliation for earlier finalization
errors; no precision or no-rollback claim is invented. Target entire old outcome
suite, then complete combined native250 again.

Unchanged complete outcome suite51526PASS32/32 in16.593s,0skips after using the
existing finalization mapper for the deferred drain. THREEreviews:1all old outcome,
atomic/import/repository/cache propagation and retry-uncertainty laws remain;
2actual32 native cases execute, including four literal deferred codes AND real
TCP after-server-COMMIT response suppression/timeout with independent committed
rows;3outcome_tests.rs byteequal TLSparentf2cfeaa1, no assertion/control/filter or
timeout change. Four-code trigger proves conservative classification, not that
COMMIT was sent; actual transport owners separately prove committed uncertainty.
Source preserves original SQLSTATE/cause and all-or-none settlement. This is
selected32 proof, not full250 or currentLinux proof. Raw
/tmp/mnesis-pg8-durability-outcome-fixed.log retained alongside original integratedred.

Final wording states the same conservative boundary in two public doc lines and
updates the private mapper's phase wording, without tests or logic changes.
Actual existing-owner churn35/35 (21added14removed), private37/80, tests634/650.
No budget relaxation; full combined native rerun follows on this exact source.

Corrected integrated full native74472PASS250/250 in60.177s,0skips. THREEreviews:
1all previous230adapter laws plus actual5names+5TLS+10durability registrations
execute together, preserving order/resume/atomicity/exact saved pairs and conservative
unknown finalization;2real Pg17.10 plus private18.6TLS/restore/fsync/fullpage/crash
fixtures execute; both immediate/deferred setting-trigger schedules and four
preserved supported commit modes run under their existing registrations, not
inflated counts;3source includes only budgeted guards/newerror/docs and original
uncertainty mapper reuse, outcome_tests.rs unchanged, all exact causes/settledrows/
Barriers/oracles retained,0filter/skip, no hardware-powerloss/failover/capacity claim.
Original174/250 failure and selected32fixed proof retained independently; failed
first full run is not relabelled. Current-source Linux/native feature and full
signed gate/CI still due. Raw
/tmp/mnesis-pg8-durability-integrated-fixed-native.log; full defaults/optional-owner
focused laws follow before staging the final formatted source for Linux archive.

Integrated no-default-features durability27522PASS7/7 in14.686s,0skips. THREEreviews:
1default append/startup/trigger/mode/restart/native-setting contracts still required;
2actual seven native cases execute, including both trigger schedules, real server
restart and two unsafe server settings;3cfg-gated atomic/snapshot/checkpoint cases
are NOT compiled/credited here, so7 is not10, and this selected owner is not full
per-feature conformance. Same finalizer/error docs and old exact oracles, no filter
inside durability binary. Native saved-state coverage remains in actual full250.

Integrated snapshot-only durability12835PASS8/8 in27.708s,0skips. THREEreviews:
1adds the snapshot write's real off-setting rejection and restarted saved pair to
seven common laws;2actual native snapshot paths execute with default/projection/
import disabled, supported modes and immediate/deferred schedules retained;
3atomic/checkpoint cases not compiled/credited, same oracles and no internal
filter, this focused8 is not the full snapshot conformance matrix. Raw
/tmp/mnesis-pg8-durability-integrated-snapshot.log retained. No source change.

Integrated projection-only durability76715PASS8/8 in4.562s,0skips. THREEreviews:
1common seven laws plus real checkpoint off-setting rejection and exact recovered
saved revision/position/state;2actual native projection owner runs with optional
snapshot/import disabled, real private restart and preserved modes/triggers;
3no extra atomic/snapshot credit, same source and independent settled-row oracles,
no filter/ignored cases. Focused8 remains distinct from full projected conformance
and currentLinux. Raw /tmp/mnesis-pg8-durability-integrated-projection.log.

Integrated import-only durability34814PASS8/8 in16.023s,0skips. THREEreviews:
1common seven plus actual atomic off-setting rejection/all-or-none settlement;
2real native atomic write owner executes with snapshot/projection disabled and
existing overlap/restart/mode/trigger laws retained;3same exact causes/settled
rows, no added filter or artificial cases, and focused8 does not replace full
atomic/import conformance. Raw /tmp/mnesis-pg8-durability-integrated-import.log.

Integrated export-only durability87521PASS7/7 in25.565s,0skips. THREEreviews:
1export feature keeps common startup/write/profile/restart laws;2actual seven
native cases run with atomic/snapshot/projection disabled, no test binary absence;
3same original assertions and no internal filter/ignored case, no export session
or backup conformance credit from this focused owner. All five focused feature
runs are separately recorded; complete CI feature matrix still PG9.

Durability CI registration pre-edit budget: add exactly5lines to the existing
native integration smokes, selecting asynchronous_connection_settings_reject_before_runtime_open
with --no-tests=fail. Reuse current Nix/nextest/Postgres facility and complete
all-feature suite. This makes an absent durability test owner fail the required
job, without a new checker/tool/framework or fixed arbitrary test-count gate.
Actual Linux execution is requiredCI; no redundant manualLinux rerun is needed
before the same required CI commands. Preserve complete-suite and smoke evidence
separately. No lints/workloads/exclusions change.

Integrated strict all-feature/all-target Clippy70994PASS7.48s, formatPASS;
no lint allowance or policy change. Compile/lint evidence adds no test runs.
README actual33added4removed/45line budget; changelog2added/3budget; existing
Rust35/35, private37/80, test634/650, Nixsmoke5/5. No publicmethod/dependency/
framework/runtime/tooling addition. Final source staged only AFTER formatting
and native/feature proofs. Complete new source inclusion required for Nix archive;
current required LinuxCI will execute seven smokes plus fullsuite, not native
proof from Darwin guards. Source will remain frozen during normal signed gate.

Integrated signed gate72759 FAIL before nextest execution: output51gyycg...drv
compilation hitNoSpace while writing instrumented dependencies. SIX completed
checks doc/fmt/replay/deny/hakari/clippy; nostd/store-nostd/wasm cancelled, no full
gate credit and no test pass invented. THREEreviews:1no changed contract or
weakened workload;2compiler failure is storage, zero nextest runtime/catch;
3same frozen source4fc4dd... remained, original raw hook log preserved at
/tmp/mnesis-pg8-durability-integrated-hook.log. Build concurrency alreadymax2,
no second local full gate; next retry must address actual space without deleting
raw baselines or bypassing hook. Replay fresh log will be inspected separately.

Reconsideration:250green conservative-finalization candidate is NOT accepted as
final PG3 policy. Deferred40003 is raised before literalCOMMIT now; owned phase
proves no commit, so mapping that error Unknown throws away information. Record
new phase falsifier before editing, retain prior reds/greens and reject candidate
based on contract rather than preserving old test assumptions. Existing commit
mapper/lost-ack/timeout invariants must remain unchanged. Follow existing relay
instead of adding a test framework. See new HYPOTHESES/SOURCES entry and150line
existing-test budget. No rewrite of append-only experiments.

Failed full gate replay log independently inspected: fresh2/1/1PASS1.01/1/1s.
THREEreviews:1existing replay inputs/contracts unchanged;2actual test bodies ran
under preserved gate source, unlike nextest compile failure;3not nativeDB proof,
no added corpus or coverage claim. Raw integrated-replay.log retained.

Final TLS f2cfeaa CI review (2026-10-19): push39199782445 and PR39199786060
both terminalSUCCESS. THREE independent reviews for each actual run:
1contract: workspace1577PASS117.517/137.262s with4compiler cases separately
selected; four REAL Linux native240PASS55.873/58.250 standalone and58.505/57.313
Nix,0skips, all5identity plus5TLS laws visibly executed. Installed-log echoes are
not extra executions. Each native job's six one-case smokes passed separately.
2fault/falsifier: original TLS absence/wrongCA/hostname controls remain retained;
actual native cases assert exact causes, settled event/state pairs, restricted
role privileges, overlapping writer result, caller-pool/reopen semantics. The
main workspace guards are not counted as real-database proof. Existing query
cleanup fixture now precedes assertion; no schema-poisoning false catches.
3selection/assertions: no source/test/filter/gate changes since signedf2;
compiler2/1/1 passed95.63/16.24/13.23s push and93.71/12.95/14.54s PR; each
feature unit matrix remains15/15/15kernel,1/1/1macros,217/217/217store,
17/17/17wake,25/25/30inmemory,131/131/131Fjall,0/0/0nostd andstore-testing.
Zero-case harnesses are compilation only; matrix still omitsPostgres (PG9).
Replay each2/1/1PASS1s; no new replay corpus or coverage credit invented.

Fuzz39199786217 THREEreviews:1same four input invariants/real production entry
points;2four61s libFuzzer campaigns wire7,544,825/header2,613,663/chunk2,254,914/
value5,959,047 and four60s AFL++CMPLOG campaigns;3actual AFL savedcrashes0/
timeouts0, discoveries wire0/header1/chunk9/value0, no clean-run-only AFL crash
gate falsifier credit (stillPG9). Mutants39199786177 actualNoMutantsToFilter,
no artifact/0variants/0runtimecatches, not mutation coverage of TLS. CodSpeed
check117891822169SUCCESS:3decode sizes426.4→368.1ns(+15.85%reported),40untouched,
3skippedbaselinefallback; syscalls/environment warnings retained, no PG latency
claim. Raw logs /tmp/mnesis-tls-f2-{push-ci,pr-ci,fuzz,mutants}.log and performance
JSON. All40requiredSUCCESS/1scheduledsweepSKIP,CLEAN,0reviewthreads at exactf2;
normal exact-head protected merge next, no admin/waiver/publication claim.

TLS PR392 MERGED2026-10-19T05:09:59Z at8f03ae8553ab6410fdfbe7660ab84ec0873efae0, normal exact-head squashf2cfeaa,40requiredSUCCESS/1scheduledSKIP/0threads. Fetched protected merge ENTIRETREE equals reviewedf2; root user branch/work untouched. No publication claim.

Phase falsifier45039 actualRED1FAIL26.422s (31nonselected), compiledfresh4.77s.
THREEreviews:1before-COMMIT failures must preserve known rollback, regardless
of40003string;2same native deferredtrigger raises40003 during drain, independent
count0 and trigger cleanup occurred BEFORE error assertion;3specific received
CommitUnknown vs requiredSqlx fails, unchanged exactSQLSTATE/0row assertions,
no skipped DB guard or synthetic-only error. Raw phase-original.log retained.
This rejects the conservative mapper candidate even though earlier250green.

Phase-fixed75087 actual2PASS94.794s,1slow,31nonselected. THREEreviews:
1contract: real deferred40003 beforeCOMMIT nowSqlx and zero independentrows,
while same code after OBSERVED clientCOMMIT remainsCommitUnknown; otherthree
codes preserve exact typed native causes;2actualserver raises both kinds, new
relay observes actualQ/COMMIT and substitutesDO, no false actual-server-COMMIT
claim, allfourcodeslooped;3specific original source/row assertions retained,
no guard/ignored/filter expansion credit.94.662s deferred case is a slow observed
sample, causeUNESTABLISHED (coincident local command stalls), not dismissed as
contention or accepted latency. NewCOMMIT boundary case0.130s. No othercases
credit; existing post-server-COMMIT lost-ack/timeout still require fullregression.

Tool responsiveness restored05:15; diagnostic/echo completed, scopedduPID75667
already exited before queuedkill, killreturnedNoSuchProcess and touched nothing.
No native jobs now. Next wrong-final-COMMIT mapper control in isolated worktree.

Wrong final-COMMIT mapper control14923 native1FAIL15.544s at exact40003Sqlx
insteadUnknown,32nonselected; compilation7.70s, expected now-unused commit helper
warning is confined to rejected mutant. THREEreviews:1client-sentCOMMIT requires
uncertainty except proven server rejection;2actual clientQ observed, nativeDO
error preserves40003, zero independently visible rows, error assertion FAILS;
3unchanged literal code/typed source expectation rejects mutant, compiler warning
is NOT catch, no skip/timeouts credited. Isolated /tmp/mnesis-durability-phase.Wqpt5d
preserved with sole wrong final mapper; product source remains correct. Raw
phase-commit-control.log. New dual-phase law demonstrably rejects both bad mappers.

Scoped cache maintenance: query found rooted old check outputgjks... and explicitly
EXCLUDED it. Four old nextest outputs9ngh/xgdw/qh3/5rwi each1.086–1.087GB have
zero roots AND zero referrers immediately checked; original build logs saved
before per-path supported nix store delete. No broadGC or raw baseline/target/
worktree deletion. Operation89647 currently waits for existing daemon-owned
GC(max1GB) lock; do not kill unrelated daemon or claim freed space until done.

Phase source strictall/allClippy25107PASS3.19s and no-default/all-targetClippy
PASS3.43s, formatPASS. No allowances/lint relaxations, compile-only proof not test
run. Existing source29/35churn; outcome relay/tests72/150. Starting complete current
native suite unfiltered; prior250 is superseded conservative candidate evidence.

Final phase-correct full native77886 PASS251/25175.943s,0skipped. THREEreviews:
1contract: all prior240 plus10native durability laws and the new COMMIT-boundary
law visibly execute together; no inferred sibling arithmetic. Event order,
bounded reads/missing-final-hint/watermark recovery, atomic/import, snapshot/CAS,
migrations/history/backup and TLS identities remain actual regressions.
2fault/falsifier: original10 durability reds/controls retained; native deferred
setting-off rejected beforecommit, independent settled data/state/key assertions;
phase deferred40003Sqlx0.143s and client-COMMIT40003Unknown0.166s both pass.
Actual post-server-COMMIT lost-ack0.047s and caller-timeout0.075s still prove
committed rows and uncertain/no delivered outcome; not replaced by DO injection.
3assertions/source: no ignored/filter/guard exclusions in251, all source frozen
during run, wrong drain and wrong final mapper controls actually fail. Exact
native causes/counts/state pairs/privilege checks retained. Prior slow deferred
sample94.662s is preserved; current0.143s does not establish its cause. Full run
75.943s is correctness evidence, not capacity/parity/soak or power-loss proof.

Phase-correct default32215 nativePASS140/14030.952s0skip. THREEreviews:
1complete no-default package actually runs core/read/wake/migration/TLS and7
unconditional durability laws plus both phase laws, no optional capability credit;
2native failures/rows/trigger boundaries are real, same wrong-mapper controls
reject mismatches, actual lost-ack/timeout retained;3no DB guard skips or nextest
filter, all140visible, disabled feature owners absent by cfg rather than claimed
passed. Strict no-default/all-targetClippy already passed. Prior focused5feature
samples retained as earlier-candidate evidence, not full current feature matrix.

Protected-parent integration: retained stash recorded in
/tmp/mnesis-durability-phase-protected-parent-stash; git rebase skipped identical
TLSf2 patch already protected as8f03ae85. StashAPPLIED--index, not popped/dropped.
ENTIRE pending binarydiff before/after comparesidentical, including records and
source, not merely selected files. Native251/140 proofs remain exact source.
Raw phase/failure/replay logs copied to evidence with trailing presentation spaces
normalized only; originals retained in/tmp. Next normalSIGNEDgate maxjobs1 to
reduce simultaneous disk peaks, unchanged hook/commands/workload/exclusions.

Replay completed before whole gate: inspected current c3j3...derivation log,
fresh compilation4.63s then actual2/1/1PASS1s each. THREE reviews:1unchanged
existing corpus/input invariants/production entry points;2actual replay bodies
executed, not copied baseline/cached run;3git diff confirms zero fuzz source/lock/
corpus changes, no new coverage/nativeDB/capacity credit. Raw phase-gate-replay.log.
Keep source/index frozen during still-running signed gate99009; merge this review
into durable records only after terminal gate, without pretending added docs were
part of the previous source snapshot.

Instrumented workspace nextest finished:1585PASS494.308s,15slow,4compilerexcluded,
136binaries; release compilation2min, coverage report emitted. THREEreviews:
1all selected workspace contracts/fixtures run under frozen source; workspace
feature union is NOT adapter --all-features; count increases by7unconditional
native guard cases plus1phase law, no assumed1588;2actual test execution includes
existing Fjall/properties/faults, while DB-guarded adapter returns are compilation/
harness proof ONLY, separate actual native251 and no-default140 remain DB proof;
3unchanged compiler exclusion4, unchanged fixture assertions/source/workloads,
no new skip/timeout/percentage waiver, native all-feature coverage not invented.
Current log /tmp/mnesis-pg8-durability-phase-gate-nextest.log. Full gate99009 still
running later no_std/wasm checks; do not credit complete gate/commit yet.

FINAL signed gate99009PASS all12 checks at signed64f325f670e49de74e3bae259976f13f760ac32a
(GPGgood), parent protected8f03ae85. Workspace1585PASS494.308s15slow4compilerexcluded
and replay2/1/1each1s independently THREEreviewed above; no nativeDB credit from
workspace guard returns. Audit/TOML were alreadycached, other10 rebuilt; no false
claim of12 fresh runs. no_std/store-no_std/Wasm compilation checks passed.
ENTIRE committed productdiff compares exact frozen8e21c659d8b1401bd0c2bf2697a2e0d7c2a3d0be18872d31b319be9f963ee506,
whitespacePASS, clean worktree before record-only updates. No gate/workload/lint/
threshold changes. Evidence-only normal signed amend will run unchanged hook
again; unchanged outputs are cached and do NOT represent new test executions.

Release PR378 regenerated at2475b90f840f78f604a5f663fd59508a9b34a821 (App6fd3d574
plus existing lock-sync2475b90f) now40requiredSUCCESS/1scheduledSKIP and remainsOPEN.
Original reported pipeline failure fixed, no final publication/acceptance claim;
remaining adapter prerequisites still block merging release. Current checks
snapshot /tmp/mnesis-release-2475-checks.json. Final release evidence must use
its eventual reviewed head and actual registry consumer, not this intermediate.
frozen8e21c659... PASS, whitespacePASS, clean beforepush.
Focused PR395 https://github.com/devrandom-labs/mnesis/pull/395 OPEN at exactefa,
normalpush (no force/admin). Required actual LinuxCI/artifact/review checks next.
Scoped cache deletion89647 COMPLETE four exact unrooted/referrerfree oldnextest
outputs, ~1GiB each; original build logs retained before removals. Rooted old
proofgjks untouched; no broadGC/target/rawbaseline/worktree deletion. No localjobs.

Next focused accounting branchfix/postgres-accounting starts at signedefaedc28,
while prerequisiteDurabilityPR395 CI runs. No other local builds/native suites.
Research append copied only after exact committed-prefix verification; user root
branch/research/worktrees unchanged. Read-only source findings in next hypotheses
entry are NOT acceptance. Scope/budgets and healthy→application-name falsifier
recorded BEFORE test edits. Existing standard caller pool, SQLx and fixture are
sufficient; no framework/runtime or projectPython. All runs THREEreviewed.

Accounting original event witness8362 actual1FAIL0.090s,5nonselected. THREEreviews:
1reader64 rejects healthy valid larger-writer row with exactRowLimit300/64;
2native application octet_length returns0 vs qualified1 on same bytea, original
reader instead delivers two valid envelopes including256byte body;3normal larger
writer persisted it (no corrupt/native row rewriting), stream/all results captured
before probeDROP, cleanup and path assertion precede failure, no poisoned schema
left. First failing stream assertion proves this owner defect; all-result captured
but its later assertion did not execute, so no separate original-all catch claimed.
Original production unchanged. Raw /tmp/mnesis-pg8-accounting-event-original.log.
Need original separate state/listing witnesses before those SQL edits. Testfile
400/650lines, healthy fixture retains existing lifecycle readiness and callerpool.

Snapshot witness command29632 compileFAIL101 E0599, ZERO runtime tests. Fixture
mistakenly calledsave_snapshot; actual existing SnapshotStore method iscommit,
confirmed intrait/adapter/existingnames law. THREEreview checks:1intended same
native body/byte contract;2compiler failure is NOT original-defect catch;3no test
assertion/exclusion/workload relaxed; correct only call name, preserve rawfailed
command /tmp/mnesis-pg8-accounting-snapshot-original.log, retry separately named.
Original production remains unchanged; sourceexisting4capability witnesses571/650lines.

Snapshot original native87581 actual1FAIL0.308s,8nonselected. THREEreviews:
1normal larger-limit writer stores validVersion1/256byte state, reader64 healthy
exactRowLimit293/64;2real application0/qualified1 lengths verified, originalhydrate
instead returnsFoundVersion1+all256bytes;3same typed exact limit oracle, cleanup
and search_path assertion precede failure, no skip/compiler-failure credit or
schema contamination. Raw snapshot-native-original.log, earlier compile failure
kept separately. No production SQL edits yet. Next independent checkpoint owner.

Checkpoint original native command actual1FAIL0.153s,8nonselected. THREEreviews:
1valid larger-writer revision1/position/state256 has healthy reader64 exactlimit
309/64;2native shadowlength0 vsqualified1 makes originalhydrate returnFound with
same revision/position and all256bytes;3exact typed RowLimit oracle, application
schema removed and callerpath asserted BEFORE failure, no compile/skip credit,
state/position untouched. Raw checkpoint-original.log. No production fix yet.

Listing original native actual1FAIL0.159s,8nonselected. THREEreviews:1normal
larger-writer accepts128byte key, healthy reader64 exactRowLimit160/64;2real same
length0/qualified1 probe makes original list_streams return allthree keys including
128bytekey;3typed limit oracle, cleanup/path assertion BEFORE failure, no guard
skip or fixture poisoning. Raw listing-original.log. Allfourcapability owners now
have actual original native reds before any production edits. Snapshot wrongcall
compile failure remains separate with ZERO runtimecredit. Initial added-test
strictClippy8.19sPASS (before three extra capability cases), not final lintproof.

PR395 final exactefa CI review: push39205684104/PR39205687726 terminalSUCCESS.
THREE independent reviews for each actual run:1contract workspace1585PASS136.712/
94.054s4/1slow4compilerexcluded, nativefour251PASS54.698/57.378 standalone and
61.606/56.293Nix0skip; all10durability+two phase laws visibly run. Seven smokes
pernativejob pass; installed-log echoes are not more executions.2execution/fault:
real unsafe settings/replacement connections/deferred triggers/process restart,
exact independent settled counts/state pairs and original lostack/timeout actual
postserverCOMMIT remain;3assertion/selection sourceunchanged, no new exclusion or
waiver. Compiler2/1/1PASS118.84/14.64/18.31s push and97.62/12.98/16.05s PR;
unitfeature matrix each15/15/15kernel,1/1/1macros,217/217/217store,17/17/17wake,
25/25/30inmemory,131/131/131Fjall,0/0/0nostd/store-testing.0harness compile-only,
Postgres feature matrix stillPG9. Replay both2/1/1PASS1s, unchanged sources/corpus.

Mutation39205687658 actual artifact12165771927 ZIP sha256
c71439878264678b9d7565f50e536d80859a1b64e40bc2f2f09ced2e285770b4 verified equals
GitHubdigest and exactefa metadata. Baseline251PASS55.985s0skip.9variants:
6runtimecaught/3compilerunviable/0survivor/timeout. ALL9diffs inspected (exact
one added replacementreturn each), all6first failures/source/selection reviewed:
validateOk bypass commits nativecounts2/2 vs1/1; commitOk falselyacknowledges
missingglobalrows; migrateOk fails fresh native initializationSchemaNotMigrated;
atomicNone fails required conflictError; snapshotOk givesAbsent vsFoundVersion2;
checkpointOk1 givesAbsent vsFoundrevision1/maxposition/state. Each matches own
mutated contract and actual native failure, no42P06 fixturepoison. ThreeDefault
History/PgAllPosE0277 are compilationonly, zero runtimecatches. Raw full logs/diffs
retained /tmp/mnesis-durability-efa-mutants, originalZIP/artifactJSON retained.

Fuzz39205687702 THREEreviews:1same four production input contracts;2actual61s
libFuzzer wire8,281,696/header2,533,822/chunk1,286,315/value5,794,139; four60s
AFL++CMPLOG campaigns;3savedcrashes0/timeouts0, discoverieswire0/header1/chunk11/
value0. No new corpus/source/gate change; clean runs NOT AFLzeroexitcrashgate
faultproof, stillPG9. CodSpeedcheck117909348385SUCCESS43untouched3baselinefallback
skipped,17syscallwarnings retained; no PGwalltime/capacity proof claimed.

Exactefa40requiredSUCCESS/1scheduledsweepSKIP,CLEAN,0reviewthreads. Raw
/tmp/mnesis-durability-efa-{push-ci,pr-ci,fuzz}.log/performanceJSON/checksJSON/
threadsJSON retained. Normal exact-head protected merge next, no admin/waiver.

Length fix13210 nativePASS9/90.390s0skip, compiled2.05s. THREEreviews:
1all five existing identity/head laws plus four separate byte-limit capability
laws pass together;2actual original reds retained event0.090/snapshot0.308/
checkpoint0.153/listing0.159s, normal accepted larger-writer data, exact healthy
and shadowed RowLimit300/293/309/160 vs64, native/control length0/1;3cleanup before
assertions and callerpath unchanged, no exclusions/guards, both stream/all exact
limit assertions nowexecute/pass (no separate original-all catch invented).
Productiononly7SQLline substitutions(14churn) plus one32→48checkpointRustdoc
correction(2churn), total16/20allowedSQLchurn+docbudget; no new API/dependency.
SUM/window/integer/catalog/metadata findings still unproven and unresolved.

DurabilityPR395 protectedMERGED2026-10-19T16:08:47Z at
f7d390d4c50936b2912207c2d417a8c24a899e0c, exactefa reviewedhead40requiredSUCCESS/
1scheduledSKIP/0threads, no admin/waiver. Fetched ENTIRE protectedtree equals
reviewedefa. User correctly challenged long delivery delay; do not leave green
focused changes waiting while expanding another investigation. Finish current
four proved byte-accounting repairs through full gate/CI/protected delivery.

Accounting strictall/allClippy7.99s and no-default/allClippy3.98sPASS, formatPASS;
no lint policy/allowance changes, compile-only not native proof. Source changes
14SQLchurn+2Rustdoc, names571/650, no new APIs/dependencies/runtime/framework.

Accounting full current native24832 PASS255/25567.963s0skip, compilation13.93s.
THREEreviews:1all protected251 regressions plus four new body/key limit laws
actually run together, preserving ordering/resume/recovery/atomicity/uncertainty/
state pairs/migrations/backup/TLS/durability;2allfour original native reds and
healthy exact-limit controls retained, new queries only13native length-call
qualifications, actual new failures not simulated pureSQL-only tests;3all255
selected/no ignored/guard exclusion, unchanged test assertions/workloads, no
inferred siblings. No capacity, power-loss or all-SQL-names-safety claim. Source
entire pending binarydiff compares identical before/after protectedparentrebase,
retained stash in/tmp/mnesis-accounting-protected-parent-stash, not dropped.

Accounting no-default names80116 PASS3/30.139s0skip, compilation6.53s.
THREEreviews:1both existing unconditional identity/head laws and new event row
limit law run with optional capabilities disabled;2actual native shadow/control
and original red preserved, exacttyped300/64 checks forstream/all;3no filter or
DBskip, cfg-disabled state/listing cases NOT counted. Focused3doesnotreplace full
nativefeature matrix (stillPG9). Completeall255 and strictall/defaultClippy remain
separate proofs. Preparing normalSIGNED fullgate; no new localtest/build jobs.

Accounting fullgate replay actualfresh2/1/1PASS1s each, compilation4.05s.
THREEreviews:1existing four production decoder/value contracts unchanged;2actual
replay test bodies ran, no cached/echo executioncredit;3fuzz source/locks/corpus
unchanged, no added exclusions/coverage/nativeDB/capacity claim. Current rawlog
/tmp/mnesis-pg8-accounting-gate-replay.log, h871...derivation. Keep source/index
frozen until complete signedgate75846; append this review to durable records only
after terminal gate, explicitly preserving the prior frozen product snapshot.

Accounting instrumented workspace actual1586PASS495.606s15slow4compilerexcluded,
release compilation1m13s, coverage report emitted. THREEreviews:1all selected
workspace invariants/old fault/property suites pass plus new unconditional event
law;2actual runtime tests verified, while Postgres environment guards here are
compilation/harness evidence ONLY, separate255native proof remains;3unchanged
fourcompiler exclusions/feature union/workloads/assertions, cfg-disabled state/
listing cases not falsely counted, no added skips or waivers. Raw sala...log at
/tmp/mnesis-pg8-accounting-gate-nextest.log. Fullsignedgate75846 stillfinishing
no_std/store-no_std/Wasm; no complete gate/commit claim untilterminal.

Accounting normalSIGNEDfullgate75846PASS all12 at
f3b2a6312087539661553f83072ea13406ec3cae GPGgood. Actualworkspace1586PASS495.606s
15slow4compilerexcluded and replay2/1/1each1s reviewedTHREEtimes above; no nativeDB
credit from guarded workspace. Full no_std/store-no_std/Wasm checks pass.
ENTIRE committed productdiff matches frozen
0d2e801dd8c9a626caaf8797133023a128988b4ed54c7318628bfc81e32f6230, whitespacePASS,
clean worktree before recordupdates. Gate/workloads/assertions/filters/policies
unchanged. Record-only signedamend next; unchanged cachedoutputs are ZERO new
runtime tests. Then focusedPR and actualLinuxCI/artifact/protecteddelivery.
# PG9 patched wake dependency — baseline, 2026-10-19

Patched complete actual native Postgres suite PASS251/251,0skipped,65.600s,
--locked --all-features --test-threads=1 --no-tests=fail, using the existing
PG17 socket55432 mnesis_pg7b and private native cluster fixtures. Raw
/tmp/mnesis-pg9-advisory-native.log. This worktree is still protectedf7 parent,
so it does NOT include PR396's four unmerged accounting laws.
Three reviews immediately after this run:
1. Contract: existing native ordering, bounded reads, recovery, transaction phase
   and actual post-COMMIT uncertainty, migrations, writes, saved state, backup,
   TLS/permissions and durability laws all remain selected and pass.
2. Execution/falsifier: all251 named cases actually executed with DATABASE_URL,
   one suite/cluster, zero skipped; retained original advisory red plus unchanged
   existing regressions provide distinct evidence. No guarded no-DB credit.
3. Assertions/source/selection: no test source, skip guard, filters, timeout,
   expected result or byte/count limit changed. Inspect actual native logs, not
   only exit0; full feature selection remains explicit. These correctness timings
   are not capacity, equal-durability performance or30-minute soak acceptance.

Patched wake suite: cargo nextest --locked -p mnesis-wake-nostd --all-features
--no-tests=fail PASS11/11,0skipped,0.163s (compile9.03s), run
e7a340dc-fcb5-4ed5-adc7-233554a70ccd. Raw /tmp/mnesis-pg9-advisory-wake.log.
Three reviews immediately after this run:
1. Contract: actual GlobalWake arm/wake, fresh rearm, unrelated/global fanout,
   future/handle lifetime and cross-thread subscriber delivery remain required.
2. Execution/falsifier:11 named existing runtime tests executed, including two
   actual Tokio barrier/task concurrency cases and Embassy catch-up/live flow;
   this is neither a zero-test library harness nor a database test.
3. Assertions/source/selection: unchanged tests park listeners before fanout,
   assert no-wake pending, bound required deliveries5s, and check exact ordered
   Embassy events1/2/CaughtUp/3. Fifty parked-wake races and16x100churn retain
   original workload. No excludes/skips/relaxed limits or new test implementation.
   Host tests do not prove bare-metal build or all possible memory interleavings.

Patched policy/graph checks: native cargo-deny advisories+licenses PASS without
any ignore; cargo-audit PASS separately over root385/fuzz70/AFL41dependencies,
current database1338advisories. Unmatched license allowances remain warnings,
not advisories. No runtime tests in these checks. Raw logs:
/tmp/mnesis-pg9-advisory-patched-deny.log and
/tmp/mnesis-pg9-advisory-audit-{.,fuzz,fuzz-afl}.log.
Three reviews after each check: (1) scope is that exact owned lock and unwaived
policy, not whole release acceptance; (2) actual zero exits/current database and
the retained original0221 red distinguish a repaired gate from a suppressed
finding; (3) source diff updates only5.4.1→5.4.2, removes concurrent-queue and its
three edges, adds zero packages and does not change either fuzz lock or tests.
Supported hakari regeneration removes its two obsolete edges, with diff now0;
the initial hakari diff1 is retained as required generated-owner evidence.
Exact upstream patched listen/notify/Notify::fence and T:Send bounds inspected.

Isolated fix/postgres-advisory worktree starts at protectedf7d390d4; accounting
PR396b0 remains frozen. Existing ledgers resumed with canonical init, skill and
full protocol; user root modifications preserved. Budget recorded before edits.

Original unwaived cargo-deny returned1, `advisories FAILED`, identifying actual
selected event-listener5.4.1 and RustSec0221. Only a temporary copy of deny.toml
had its exception removed; original manifest/lock/policy remained unchanged.
Raw: /tmp/mnesis-pg9-advisory-original-deny.log; temporary directory retained at
the path in /tmp/mnesis-pg9-advisory-control-path. No runtime tests executed.

Three independent reviews of the baseline check:
1. Contract: production acceptance requires the selected dependency to be patched
   and the advisory exception removed, preserving wake semantics and no_std.
2. Execution/falsifier: actual Cargo-resolved root graph with the current advisory
   database rejects5.4.1 under the unwaived policy; this is an advisory gate red,
   not a demonstrated runtime exploit or concurrency-test failure.
3. Assertions/source/selection: inspect the exact advisory/package/version and
   failure exit, not merely a nonzero command. Both other owned locks lack the
   affected package. No dependency changes, test excludes or baseline resets.

Accounting evidence-only signed amend22308 PASSall12previouslybuilt outputs,
ZERO fresh runtime runs; current signedb0d613eaed11daf2d475c937d4c3d675ca284661 GPGgood.
ENTIRE productdiff stillcmp frozen0d2e801d...PASS, whitespacePASS. Normalpush
created focusedPR396 https://github.com/devrandom-labs/mnesis/pull/396 at exactb0,
no force/admin. RequiredCI started (initialpushCI39285646413); no localjobs.
Next actualnativeLinuxCI/artifact/CodSpeed/threads then protectedmerge; don'tleave
greenfocusedchanges waiting or falselymark wholeadapter delivered.
# PG8 accounting current CI review — signed b0d613ea, 2026-10-19

DELIVERED: normal protected exact-head squash PR396 merged2026-10-19T16:49:55Z,
commit766394cd508046dbce3ae6893a1b9c9da805c941. All40requiredSUCCESS/1scheduledSKIP,
0threads; complete merged TREE equals reviewed signedb0, not only source paths.
Delivery/check/thread/performance/mutation snapshots and raw actual logs retained
under /tmp/mnesis-accounting-b0-*. No waiver, admin merge or publication.
Whole PG7/8/9 evidence and full release/clean-registry consumer remain required.

Final push Nix job118170676378 completed SUCCESS: workspace1586PASS137.601s4slow
(four established compiler exclusions); native255PASS61.805s0skip,7smokes;
replay2/1/1PASS1s each. Three independent reviews after EACH run: (1) compare its
actual selected workspace/native/replay contract with the immutable head and
required optional owners; (2) fresh build/execution and complete native set,
not reused outputs or echoed runs; (3) unchanged assertion source, filters,
limits/corpus/feature selection, distinguishing no-DB workspace guards from
actual native database proof. No PG8 capacity credit from these timings.
Final exact signedb0 source SHA0d2e801dd8c9a626caaf8797133023a128988b4ed54c7318628bfc81e32f6230
matches frozen pre-gate product byte-for-byte; whitespace/GPGgood. Final API
snapshot CLEAN,40SUCCESS/1scheduledSKIP,0threads. Current final check/thread JSON
retained; normal protected exact-head merge next, no bypass or threshold change.


Completed PR full Nix job118170709132: actual workspace1586PASS102.346s2slow,
four established compiler-fixture exclusions; actual additional native255PASS
57.835s0skip plus seven one-law smokes; replay2/1/1PASS1s each. Three reviews
after EACH run: (1) full selected workspace contract and optional native owners,
with default workspace feature union kept distinct from all-feature DB archive;
(2) actual fresh build/run logs, zero native skips, separate counted executions;
(3) unchanged exclusions/guards/assertions/limits, no no-DB pass claimed native,
existing fuzz source/corpus/workloads unchanged. Current signed code hashes and
source scope remain unchanged. Push full Nix job118170676378 is still QUEUED.

PR compiler118170709336: actual2/1/1PASS117.45/14.45/17.94s, all three reviews
repeated for each compiler owner as above, unchanged exact stderr cases/no
ignored/filtered/failed. Final unreviewed matrix modes now reviewed individually:
push kernel15/15/15, store217/217/217, store-testing0/0/0 and PR wake-nostd0/0/0.
For each actual harness the contract/source, execution and assertion/selection
reviews agree with the earlier matrix review; zero harnesses prove compilation
only. All selected unit assertions are unchanged and retain their full scope.


PR native job118170709358 actual255PASS61.633s0skip and seven one-case smokes;
all three reviews above repeated independently against this job's raw log and
current head, not credited from the push run. Its second printed255summary is
the installed-log echo, zero extra executions. Native jobs still required inside
the two full Linux Nix checks, whose current handles remain live.

All four actual AFL60s/CMPLOG campaigns reviewed: wire0/header2/chunk18/value0
new corpus items; each reports0crashes/0timeouts and the time-limit termination.
Three reviews for EACH: (1) unchanged target parsing/value laws and input set;
(2) actual end statistics, CMPLOG confirmation and clean uploads, not inferred
from workflowgreen; (3) current source/duration/no changed failure thresholds.
This clean evidence does NOT prove the known zero-exit saved-crash gate rejects
a fault; that independent PG9 acceptance remains required. Both fuzz engines'
actual raw job logs retained under /tmp/mnesis-accounting-b0-job-<id>.log.


Completed push native job118170676951 actual255PASS62.313s0skip, plus seven actual
one-law feature smokes. Its installed run.log echo repeats the same255run and is
NOT an additional execution. Three reviews after each smoke and full suite:
1. Contract: selected atomic/snapshot/projection/listing/session/TLS/durability
   smoke owners prove optional cases are present; complete255 includes all four
   new accounting laws and existing native contracts, not only smoke selection.
2. Execution/falsifier: live isolated Linux Postgres, no-tests=fail on seven
   smokes, explicit nonempty URL and SQLprobe, actual compile/archive/execution;
   full255zero skipped. Smoke skips are unrelated cases excluded by selection.
3. Assertions/source/selection: reviewed immutable signedb0 source and current
   archived set, original caps64/exacterrors unchanged, no pipeline/test/timeout
   edits or database guards counted as native evidence. Seven selected smokes
   and one complete run retained distinctly from the echoed installed log.

Push compiler job118170676765 actual2/1/1PASS87.71/11.16/13.57s. Three reviews
after each owner run: (1) existing malformed-domain/schema/API diagnostics stay
required; (2) actual trybuild cases run in plain checkout, not omitted by the
Nix compiler-fixture exclusion; (3) unchanged sources/expected stderr/feature
selection and0failed/ignored/filtered. Compiler fixtures are not database laws.

Four actual libFuzzer campaigns finish61s each: wire8,982,140/header2,841,252/
chunk2,272,854/value9,601,392 executions, no crash failure. Three reviews after
each: (1) unchanged parser/value invariants; (2) actual terminal count and ASan/
value-profile logs; (3) unchanged sources/duration/corpus/settings, no Postgres
or omitted-crash-gate acceptance inferred. AFL legs still being collected.


Further completed matrix jobs downloaded individually under
/tmp/mnesis-accounting-b0-job-<jobid>.log, current job metadata retained for both
push39285646413 and PR39285656117. Newly inspected runs: push Fjall131/131/131,
wake17/17/17, inmemory25/25/30; PR store-testing0/0/0, store217/217/217,
inmemory25/25/30, kernel15/15/15, Fjall131/131/131 (default/no-default/all).
Three reviews for EACH actual run: (1) exact unit-owner contracts stay unchanged,
and optional inmemory laws appear only in all-features; (2) actual runtime counts,
0failed/ignored/filtered in each log, with store-testing0 getting no runtime
credit; (3) unchanged selected assertions/feature commands and no test/gate
edits, preserving the distinction between library checks, unit tests, integration
laws and native Postgres. These do not close PG9's downstream/feature gap.

CodSpeed check118173905265 SUCCESS, actual run39285656109, against protectedf7:
43untouched/3skipped with baseline fallback;17system-call warnings retained.
Raw /tmp/mnesis-accounting-b0-codspeed.log includes actual benchmark execution,
plus restore/save cache-service warnings (job still succeeds), not a benchmark
regression. Three reviews: (1) existing kernel/Fjall/wire suite coverage only,
not Postgres capacity; (2) actual report and revision comparison, not inference
from checkgreen; (3) unchanged benchmark sources/thresholds, baseline-substituted
three cases and unmeasured system calls explicitly excluded from new evidence.
Current review-thread query returns0; recheck exact head and checks before merge.


Actual mutation artifact12202629030, exact headb0/run39285656132 downloaded and
SHA256 VERIFIED against GitHub0a18b86e21e665fea68b9dcae23da713f4efa790fb8f9771a77f1331e1575730.
ZIP /tmp/mnesis-accounting-b0-mutants.zip and unpacked directory retained.
Baseline255PASS59.181s0skip. Sevenvariants:1runtimecaught/6compilerunviable,
0survivors/timeouts. ALL seven diffs/first outcomes reviewed: each substitutes
one return only. Snapshot/checkpoint Default(E0277) and four PageRow constructors/
Default(E0599/E0277) are compiler-only and get ZERO runtime catch credit.
The one runtime variant returns empty listing; actual firstfailure is the
completed-chunk final-conflict law observing missing listed stream, not a poisoned
schema.34cases executed33PASS/1FAIL3.105s, no42P06. No claim this automated
mutation proves each SQL literal guard; the four retained original native reds do.
Three reviews after baseline and after the one mutant runtime run: (1) baseline
selects complete actual DB suite; mutant threatens real event inventory contract;
(2) correct-source all255pass and the bad empty-return actuallyfails on observable
missing inventory; (3) actual failure source/assertion, seven exact diffs, no skip/
filter change or contaminated fixture, six compile failures kept distinct.


Live exact-head CI is running/queued; no all-green/merge claim yet. Downloaded
actual completed job logs118170676922(push wake-nostd),118170676800(push macros),
118170709561(PR wake),118170709612(PR macros), kept under
/tmp/mnesis-accounting-b0-{push-wake-nostd,push-macros,pr-wake,pr-macros}.log.
For EACH of their default/no-default/all test runs (twelve harness runs):
1. Contract review: existing selected lib-test owners are unaffected by the SQL
   change. Macros1/1/1 and wake17/17/17 check their existing unit contracts;
   wake-nostd0/0/0 proves only compilation, not its integration wake protocol.
2. Execution/falsifier review: actual logs include three cargo test invocations
   per job and zero exit with0failed/ignored/filtered. No database runs or native
   Postgres credit. These are executions, unlike reused gate outputs.
3. Assertion/source/selection review: unchanged assertions and package selections
   retain existing checks; lib-only scope omits external wake-nostd integration
   laws. Feature-isolation and actual optional Postgres suites remain PG9 work;
   green0-test harnesses do not satisfy them. No test filters or skips changed.

## PG9 patched wake dependency — first signed gate and protected integration

Final combined normal signed gate28513 exits0, signed0f407cce G; all12 named host
checks PASS (10rebuilt/2previouslybuilt per hook). Actual1586workspacePASS622.698s
15slow4compilerexcluded; replay2/1/1PASS1.00/1.00/1.02s; no_std/store-no_std/Wasm
complete. Native255PASS90.562s0skip is distinct actual DB proof. Final signed
product diff remains byte-identical2803SHA over protected766; GPG/whitespacePASS.
Raw integrated-hook/nextest/replay/nostd logs and three reviews retained at
/tmp/mnesis-pg9-advisory-integrated-*.
Three reviews after each remaining compile-only owner: supported pinned targets
and wake/store ownership, actual completed build logs rather than host runtime
substitution, unchanged source/feature gates and no warning/lint relaxation.
No native or capacity claim from build-only checks. User steering: keep remaining
work tied to explicit PG0–PG9 requirements, prioritize operations/CI/docs/release,
and stop expanding security edge-case research. Original required TLS/role/
dependency checks remain covered. Current metadata work is ordinary startup
allocation/correctness with two reproduced failures, not new security scope.
One record-only normal signed amend next; cache status must be inspected, not
assumed. Then focused PR/current CI/protected delivery; no full-release claim.


Combined protected-parent native suite68930 exited0:255PASS90.562s0skip,
compile11.50s, run68940e29-f124-4436-a04d-dcc178a03c78. Raw integrated-native.log.
Three independent reviews immediately after this run:
1. Contract: dependency patch composes with ALL four newly merged byte-accounting
   laws and the existing native ordering, reads, uncertainty, migration, saved
   state, backup, TLS/permissions and durability contracts.
2. Execution/falsifier: all255 actual named native cases executed with explicit
   database URL, --locked/--all-features/one thread/--no-tests=fail; zero skips,
   not inferred from the earlier251 or an empty/guarded workspace harness.
3. Assertions/source/selection: exact original signed accounting source now
   composes with unchanged2803dependency diff; all tests/expected errors/byte
   limits/case selection/timeouts unchanged. Complete actual log inspected, no
   stopped/suppressed cases.90.562s is correctness timing, not capacity proof or
   an established diagnosis of variation from prior67.963s sample.


Normal signed full gate70705 exits0; original signeddb52f645ff2d0959c1f37c039407f338a0acab6b
retained at work/postgres-advisory-f7-proof. All12 named host checks pass, including
bare-metal/store-no_std/Wasm; actual workspace1585PASS603.626s15slow4compiler
exclusions and replay2/1/1PASS1s each. Full raw logs and three reviews retained at
/tmp/mnesis-pg9-advisory-{hook,gate-nextest,gate-replay}.log and gate-reviews.md.
All tests/assertions/workloads unchanged; cause of slower timing unestablished.

Three reviews after each compiled no_std/Wasm owner: supported pinned targets and
unchanged wake feature ownership; actual completed native build logs, not host
runtime substitution; source/config/selection unchanged, no warnings or std gate
relaxation. Unwaived Nix audit/deny and supported hakari logs show successful
commands. A later path-info inspection finds an absent Hakari cache output, while
its derivation identity matches the exact completed successful gate log. Cause
not established; this is not an additional test run or a failed test verdict.
Raw inspection failure retained; no claim that every output remains in cache.

PR396 protected-merged766394cd at16:49:55Z after40requiredgreen/0threads; complete
merge tree equals reviewedb0. Dependency branch now signed3861c0b8d7c10201546e87e79da13fe5ad255481
on that parent. Five RESEARCH-ONLY rebase conflicts resolved using Git's ordinary
union of both histories. Protected EXPERIMENTS prefix byte-identical and JSONL
valid. Full product diff over the new parent STILL byte-identical to frozen
2803e43f13db6d0fbb127dfa5bc7947ba03eaf42714daaa0d5e8041ff27f9c18; no Rust/test
conflicts. Original signed commit and all conflict stages preserved at paths in
/tmp/mnesis-pg9-advisory-rebase-path. Accounting postgate/CI/delivery working
records also merged with the ordinary three-way union; original worktree intact.
Combined revision still requires actual255native and normal signed full gate,
current Linux CI, focused PR/protected merge; first251 onf7 is isolated proof.
PG9 dependency signed gate70705: completed runtime review before the final build outputs finish; source/index remain frozen.

Actual instrumented workspace:1585PASS603.626s15slow,4 established compiler-fixture exclusions. Fresh completed derivation log ggrp9py35physffh8fsvkhwxzqz3jakl, not cached/no-DB native evidence. Cause of the slower duration versus earlier495s sample remains unestablished.

Three independent reviews after the workspace run:
1. Contract: preserve all selected kernel/store/adapter/wake invariants; existing complete native251 run and unwaived advisory check remain distinct. Workspace feature unification does not prove every isolated Postgres feature or actual DB execution.
2. Execution/falsifier: actual1585 named cases passed, not an empty harness or inferred exit. Fifteen slow cases finished; the last two Fjall version properties retain256 generated cases and unchanged bounds. No process was restarted or test omitted to complete the gate.
3. Assertions/source/selection: source and index remained frozen at productSHA2803e43f13db6d0fbb127dfa5bc7947ba03eaf42714daaa0d5e8041ff27f9c18. No tests, expected results, filters, limits, lints or workloads changed. Four compile fixtures stay in their established separate plain-checkout jobs. Guarded no-DB adapter passes get no native credit; full PG8 performance/capacity/soak remain pending.

Actual replay:2/1/1PASS, each1.00s, fresh yp7sf0s7phzw7ll7g94pmg6g0h4yvmn0 derivation. Three reviews after EACH harness: (1) unchanged wire/CBOR corpus and bounded-random invariants; (2) actual existing harness executes and succeeds; (3) exact source/corpus/filter/duration unchanged, no coverage-guided campaign or Postgres proof inferred.

Readonly one-second OS sampling was unavailable for a Nix build-user process without additional privileges; no escalation, process change or causal diagnosis. Raw /tmp/mnesis-pg9-advisory-slow-property-sampling.log retained. Completed test process and live overall gate were distinguished; no missing child was treated as a reason to restart a live gate.
Combined protected-parent gate28513 runtime reviews, before remaining build-only outputs finish. Dependency source/index remain frozen.

Actual complete instrumented workspace1586PASS622.698s15slow,4 established compiler-fixture exclusions, across136binaries. Actual native255PASS90.562s is separate proof. No metadata-probe files from the independent worktree are part of this gate source.
Three reviews immediately after the workspace run:
1. Contract: preserve all selected workspace contracts and compose the reviewed accounting fix with the patched dependency. Default workspace feature union does not substitute for isolated adapter features or the native database suite.
2. Execution/falsifier: actual fresh azbj8qv7 derivation executes1586 cases with0failures; existing slow Fjall property workloads finish intact. No restart, lowered proptest count, timeout waiver or omitted test.
3. Assertions/source/selection: exact2803dependency product diff and reviewed766accounting source; no test/assertion/cap/filter/baseline changes. Four compiler fixtures remain in their existing plain-checkout jobs. No guarded no-DB pass is credited as native proof. Concurrent independent metadata characterization used a separate target and process-local allocator; correctness timings are not a capacity comparison.

Actual fresh replay qlj4acd7:2/1/1PASS1.00/1.00/1.02s. Three reviews after EACH harness: unchanged corpus/wire/CBOR invariants, actual completed execution/counts, unchanged source/selection/duration. Not a coverage-guided campaign or Postgres capacity result.

Both the isolated first gate603.626s and combined622.698s are retained. These noisy whole-workspace timings do not establish a dependency performance regression or improvement; no such claim is made. Ordinary tests and current source records remain authoritative.
# PG4 bounded runtime metadata — original witness, 2026-10-19

Complete current native suite86557 exits0:257PASS74.826s0skip, --locked/
--all-features/one thread/--no-tests=fail, supported --success-output immediate.
Raw /tmp/mnesis-pg4-metadata-native.log includes successful numeric measurements.
Three independent reviews after this actual run:
1. Contract: all existing migration/ordering/read/recovery/transaction/atomic/
   state/backup/TLS/durability cases compose with the two bounded-startup laws.
2. Execution/falsifier: all257 actual native cases execute with zero skips, and
   the original two reds plus two independent wrong-bound reds remain retained.
3. Assertions/source/selection: original caps, inputs, checksum/version equality,
   expected errors and restoration unchanged; no excluded cases or baseline/lint
   changes. Successful stdout exposes measurements from the same full run rather
   than adding a measurement-only execution. Correctness timing is not capacity.


Independent wrong-prefix control: same isolated worktree, row limit restored to
correct candidate; ONLY checksum checked_add(1)→checked_add(0). Actual1FAIL0.405s,
compile11.51s, run28262655-a1cc-4330-9ae4-844b3fd60ba0;16unselected. It accepts
the correct48byte prefix of a33,554,480byte checksum and allocates22,115peak bytes.
Original Schema assertion fails AFTER restoring the exact original checksum and
successful repaired open. Three reviews: (1) bounded memory alone must not make
an overlong digest valid; (2) actual bad native query opens the store, failing the
correctness assertion rather than a setup/compile problem; (3) exactly one bound
token changed, original input/cap/assertions/cleanup intact and no poisoned schema.
Candidate source remains unchanged; control state/diffs/log retained separately.
Next complete native suite will also expose successful allocation measurements
using nextest's supported success output, without an extra measurement-only run.


Independent wrong-count control: isolated retained worktree
/tmp/mnesis-metadata-controls.E8INXH differs from candidate ONLY row-bound
checked_add(1)→checked_add(0). Actual1FAIL1.123s (compile19.85s),16unselected,
run36f83274-38ba-44d6-a718-b3a19b1c5cf0. It accepts100,005rows while allocating
only22,115peak bytes; Schema assertion fails AFTER exact metadata restoration and
successful repaired open. Raw wrong-count.log retained. Three reviews:
1. Contract: a small-memory implementation must still reject an unsupported
   migration, so the input/type/error oracle remains independently required.
2. Execution/falsifier: actual bad native query returns a successfully opened
   store and fails the original Schema assertion, not compile/setup or memory.
3. Assertions/source/selection: one bound-token change only, unchanged native
   data/count/memory cap and cleanup;16focused omissions are unrelated cases.
   The candidate worktree remains correct and untouched. No fixture poisoning.


Minimal fixed query passes both actual native regressions:2PASS3.181s,15focused
unselected, compile28.99s, runce20aad7-3811-4376-9e8e-197989681223. No assertion,
cap, input or test selection changed. Source24added/7removed lines (within40/10),
zero new private/public type or API. SQLx QueryBuilder renders compiler-owned
numeric bounds derived from MIGRATIONS with checked addition; native substr and
LIMIT return at most one byte/row beyond the known shape. Existing exact digest,
version and success checks remain unchanged. Explicit SQLx administrative
migration behavior is unchanged; this slice bounds runtime schema validation.

Three independent reviews after this actual run:
1. Contract: reject wrong/unsupported metadata while bounding runtime startup
   allocation; normal metadata remains accepted after each exact repair.
2. Execution/falsifier: the same two previously failing native laws now pass,
   actually invoking from_pool, including schema rejection and repaired reopening;
   original134MB/11MB reds retained. Successful pass proves the fixed2MiB upper
   bound; hidden success stdout gives no exact smaller allocation claim yet.
3. Assertions/source/selection: exact original test file/caps/workload/cleanup
   unchanged,15unselected from focus only; no DB guards counted as native. Extra
   byte/row preserves rejection, with independent bad-bound controls next.


Base protected766394cd; production schema.rs unchanged. Existing migration owner
adds125test lines,821/876budget, reuses allocation accounting and real from_pool.
Original native run935406e5-bcc4-4a53-9263-8bcf6cdde47b exits100: two actualFAIL,
4.963s (compile40.71s), --no-fail-fast/one thread with real PG17.15unselected
cases/20other binaries omitted by the focused filter, not runtime skips.
100,005metadata rows allocate11,028,315incremental peak bytes; a33,554,480byte
checksum allocates134,204,367bytes. Both reject Schema, preserve metadata exactly,
restore their mutation and successfully reopen BEFORE allocation assertions.
Raw original /tmp/mnesis-pg4-metadata-original.log retained. Limit2,097,152bytes
was recorded before execution and remains unchanged. This is ordinary startup
memory/correctness scope; no new security acceptance or public feature is added.

Three independent reviews after this actual run:
1. Contract: runtime opening must reject unsupported metadata without memory
   scaling with its row count or checksum body; checksum/version exactness stays.
2. Execution/falsifier: two actual named native cases fail at the allocation
   assertions, not compile/setup or a different returned error. No-fail-fast lets
   both execute; source/error/unchanged-data assertions and repaired open passed
   before each allocation panic. Original source remains unchanged until now.
3. Assertions/source/selection: actual counters wrap only from_pool, fixture
   generation occurs before measurement, and process-local peak deltas reject
   both original defects. Correct digest prefix plus tail will also reject a fix
   that silently truncates to exact digest length; extra rows reject a limit that
   hides unknown versions. Cleanup BEFORE assertions avoids mutation poisoning.
   Existing host Nix gate has no DATABASE_URL/impure env and a distinct target;
   no simultaneous native suite. Timings under host load get no capacity credit.

## PG4 startup bounds — final native measurements and compiler checks

The same complete 257-case native run records incremental peak allocations of
2,813 bytes for 100,005 metadata rows and 2,814 bytes for a 33,554,480-byte
checksum, below the original 2,097,152-byte cap. This measures the caller's
startup allocation window, not server memory, retained RSS or capacity.
Strict all-feature/all-target Clippy passed in 9.36s; strict no-default/all-target
Clippy passed in 11.96s and workspace formatting passed. Compiler/format checks
execute zero product tests and earn no runtime credit. Three reviews: contract
and feature combinations unchanged; actual compiler exit0 under existing denied
warnings; no lint, fixture, cap, selection or assertion changes.

Frozen four-file product diff SHA256:
da2c52f4fdb1de272cfc09b5c052eb2c63269145b6536128c796394a3af1d103.
Next integrate onto reviewed dependency PR397 before its first signed full gate;
preserve the entire WIP and verify the product diff byte-for-byte after integration.
The dependency update is required delivery maintenance; user steering limits
further security work to explicit PG0–PG9 acceptance. Operational and release
requirements remain the priority after the current startup-memory fix.

## PG4 combined reviewed dependency integration

Combined source on reviewed dependency205b01c4 passes the entire real-Postgres
suite:257PASS63.461s,0skips, compile12.81s. Same original migration inputs record
2,813/2,814 incremental peak bytes. Raw
/tmp/mnesis-pg4-metadata-integrated-native.log is retained with the original reds,
fixed run and independent wrong-count/wrong-prefix controls. Product diff remains
byte-identical da2c52f4; no dependency/API/test/threshold changes during integration.
The entire WIP stash is retained. Four research-only conflicts used ordinary
Git union; the protected parent's JSONL prefix is byte-identical and JSONL valid.
An initial shell inspection used zsh's reserved path variable and failed before
merging files or executing tests; the corrected /bin/sh command uses record.
This setup error earns zero execution or product-defect credit.

Three independent reviews after the actual combined run:
1. Contract: every selected migration, ordering, uncertainty, atomic/import,
   state, backup, recovery and lifecycle law passes with bounded startup.
2. Execution/falsifier: all257 native cases actually execute,0skips; original
   two allocation reds and two wrong-bound Schema failures remain independently
   retained. A compiled or guarded workspace run is not substituted.
3. Assertions/source/selection: source diff byte-identical to frozen candidate;
   original checksum/count/cap/Schema/cleanup/reopen assertions intact, no omitted
   selected laws or weakened lints/workloads. No capacity claim from this timing.

Next normal signed full Nix gate freezes source/index until terminal. Delivery
requires actual Linux CI, review and protected merge after the dependency parent.

Eight native/control/compiler logs are committed under the existing evidence
folder as pg4-metadata-*.log. Copies remove terminal trailing blanks only for
Git whitespace checks; exact unmodified raw /tmp logs remain retained. This
formatting does not alter test selection, assertions, inputs or result counts.

## PG4 signed full gate and protected parent

Normal signed gate18512 exits0, first signedfe050eb29898ef80857956cb8c6c7e0df93fb9ff G:
all12hostchecksPASS (10built/2cached). Actual workspace1588PASS566.907s15slow,
4existing compiler-fixture exclusions,136binaries. For this actual run, three
reviews: unchanged workspace contracts plus two guarded startup regressions;
actual executed1588 with native257 separately proven (no DB guard credit here);
unchanged filters/workloads/assertions/lints and all exclusions explicit.
Actual replay2/1/1PASS1.00/1.00/1.01s: EACH invocation three-reviewed for owning
corpus contracts, actual execution, and unchanged corpus/oracles/selection.
Actual no_std/store-no_std/Wasm builds complete; no hardware runtime credit.
Docs/Clippy/fmt/TOML/Hakari/unwaived audit/deny checks pass. Raw normal gate and
per-owner logs retained under/tmp/mnesis-pg4-metadata-* and mnesis-pg4-*-nostd/wasm.

PR397 protected merge3313567f8714f655c7ed65f0fb9be62caa463b0a at17:47:45Z,
exact reviewed205b01c4 entire tree byte-identical.40SUCCESS/1scheduledSKIP,
0threads.21focusedprotectedmerges, not published. All final Linux job evidence
and three reviews are retained in the operations research record pending its
integration, with exact delivery snapshot/tmp/mnesis-pg9-advisory-delivery.json.

Metadata signed rebase onto that protected parent givesc565230c G; ENTIREtree
matches original reviewedfe050, product diff remains exactda2c52f4. Original
signed proof branch work/postgres-metadata-205b-proof retained. No merge conflicts
or source mutation. This does not create another test execution. Record-only
normal signed amend next; inspect actual cached/runtime status rather than assume.
Then normal push/focused PR and Linux CI/protected delivery; no approval pending.

Evidence-only signed amend55938 exits0 at c33c6e0c G: all12checks cached,
running0flakechecks, ZERO new runtime tests. Frozen four-file product stillda2c52f4.
Correct the PRD's dependency pointer from in-CI to merged; final normal signed
record amend preserves the product and all verification policies. Publish after
terminal/sourcecmp/signature/actual cache inspection, without another source edit.

## PG8 supported deadline characterization — preparation

The prior goal turn made delivery progress: dependency PR397 opened and metadata
combined257native proof completed. Its signed gate remains live18512; operations
work is isolated in /tmp/mnesis-postgres-operations. Scope is explicitly required
PG8 acquisition/statement/lock/permanent-outage behavior using existing controls.
First strict Clippy attempt exits101 on one unused import, one redundant closure
and three explicit panic macros in the new test fixture. No native tests execute.
Three reviews: original production untouched; actual compiler failure gets zero
runtime/falsifier credit; repair fixture syntax under the unchanged denied lints,
with exact error/rollback/cleanup oracles preserved. No lint allowance is added.

Strict Clippy repair passes2.49s: zero product tests executed. Three reviews:
existing public controls unchanged; actual all-feature/all-target compiler pass;
original warnings remain denied, no selection/assertion relaxation. Before the
first native run, strengthen rollback checks to compare both actual event and
key inventories across each rejected write, with healthy follow-up append after
cleanup. This checks the preregistered no-partial-write invariant, not a new API.

First five-case native run:4PASS/1FAIL12.178s,0skip, compile5.12s;
strict Clippy before it PASS2.24s. Actual acquisition, lock, startup and statement
oracles pass; permanent-outage test assumes every native failure is PoolTimedOut
and fails that assertion. The actual error was not included in the first panic.
No production-defect credit is claimed for that unjustified fixture assumption.
Before changing the expected type, retain the red and rerun the same oracle with
its actual typed error in the assertion message. Three reviews: (1) native
unavailability can reach an existing socket as well as acquisition, so timeout
classification requires evidence; (2) five actual native executions, not guards,
with the one exact failing assertion retained; (3) watchdogs, native SQLSTATE
checks, inventory pairs, restoration and healthy follow-up appends stay intact.
No cap, lints, runtime or production code changes. Private-cluster RAII stops the
outage fixture even on this assertion failure; shared-schema mutations restored.

Same permanent-outage oracle with diagnostic text fails again:1FAIL0.969s,
4focused omissions, compile timing in raw outage-type.log. Actual error is
Sqlx(Io(Os { code:2, kind:NotFound, message:"No such file or directory" })); the
native Unix socket has been removed by the actual stopped server. Three reviews:
(1) bounded unavailable-server reads need not wait for an acquisition timeout;
(2) actual native typed error proves the fixture assumption wrong, not a product
bug; (3) no error-code/cap/source/selection changes in this diagnostic run.
Correct the expected type specifically to native Io(NotFound), additionally prove
socket exists before stop and is absent after it. Keep three attempts, watchdogs,
pool-size/close oracles and server lifetime unchanged. This is a documented
fixture correction based on observed native behavior, not arbitrary error success.

Configured native suite now5PASS41.005s0skip, strictall/allClippyPASS1.73s,
compile1.35s. This proves supported controls on unchanged production, not a new
adapter timeout policy. Slow fixture startup/cleanup under current host load
is not a latency or capacity result. Three reviews:
1. Contract: actual pre-COMMIT statement/lock rejections retain57014/55P03;
   failed admission and rejected inserts preserve both event/key inventories;
   metadata unchanged after bounded startup rejection; stopped-server reads and
   shutdown remain bounded and poolsize returns0.
2. Execution/falsifier: five real named laws execute,0skip; private Unix socket
   exists before native stop and disappears afterward; three reads require the
   specific observed Io(NotFound), with arbitrary success/error types rejected.
3. Assertions/source/selection: watchdogs/configs/healthy post-cleanup appends and
   original typed checks intact; the narrowly corrected native-error expectation
   is documented with two retained reds and no production-defect claim. Existing
   lost-ack and after-COMMIT timeout laws stay unchanged; no safe-retry inference.

Next controls: copy the exact five-case fixture to an isolated retained worktree.
First suppress only native startup settings using take(0), leaving all three
native error oracles/cleanup intact; those three laws must fail. Then restore
settings and lengthen only the owning pool's acquisition deadline beyond the
unchanged watchdog; admission law must fail. Do not merge either wrong config.

Native-option suppression control executes3cases:3FAIL17.206s,2focused
omissions, compile17.74s. Only change is options(settings.iter().take(0).copied())
in the isolated fixture. Each lock/startup/statement case reaches its original
exact55P03/57014 oracle and fails after releasing the held lock or trigger,
capturing unchanged inventories/metadata and successful repaired use. Three
reviews: (1) supported native controls, not watchdog cancellation, must produce
the claimed native errors; (2) all three real named failures reject their disabled
configuration, not compile/setup/poisoning; (3) same watchdogs, original oracles,
inputs, native cleanup and healthy use, with two unrelated filtered laws unrun.
Raw native-control.log and one-line diff retained, no production mutations.

Acquisition wrong-option control: restore native settings exactly, change ONLY
owning pool acquire_timeout200ms→5s. Actual1FAIL6.396s,4focused omissions,
compile4.50s. Same three-second watchdog expires on blocked read/append; after
release, inventory comparison and healthy append pass, then the original read
PoolTimedOut assertion fails. The later append assertion is not reached, so no
independent negative-control catch is credited to that assertion. Three reviews:
(1) caller pool policy, not a new adapter timeout, owns admission; (2) one actual
named runtime failure at the intended oracle, not compile/setup; (3) single-line
control diff, original assertions/watchdogs/cleanup and workloads unchanged.
Wrong configs stay isolated in /tmp/mnesis-deadline-controls.XRdWk1, never merged.
All five correct-config native laws already passed; full integrated suite and
protected delivery remain pending. Product Rust/API/dependencies unchanged.

## PR397 completed Linux owner checks at signed205b01c4

Current head205b01c4bbe6f022702c482d13e2f7f9d6a4ffcd; two Nix flake jobs still
live,38checksSUCCESS/1scheduledSKIP; zero review threads, independently queried.
Completed logs fetched from the actual job API, retained at
/tmp/mnesis-pg9-advisory-job-<id>.log. Review covers EACH actual invocation below,
not just its green aggregate. Final protected delivery is still pending.

Standalone native push118201421475:255PASS62.147s0skip, seven distinct native
one-case capability smokes; PR118201438653:255PASS61.076s0skip plus seven smokes.
Their installed run.log echo repeats the same summary, not a second native run.
For each of these16 actual invocations, three reviews: (1) unchanged full selected
ordering/transaction/state/backup/lifecycle contract or named smoke capability;
(2) actual PG18 fixture and compiled5.4.2 dependency execute, no no-DB guard credit;
(3) original test sources/oracles and all-feature archive selection unchanged,
full runs0skip, smoke omissions explicit and no isolated-feature coverage claim.

Push and PR each run default/no-default/all LIB matrices on eight packages:
kernel15/15/15, macros1/1/1, store217/217/217, wake17/17/17,
inmemory25/25/30, Fjall131/131/131, wake-nostd0/0/0 and store-testing0/0/0.
For EACH of48 library invocations, three reviews: owning feature contract;
actual --lib execution and counts (zero-case binaries get zero runtime credit);
unchanged flags/assertions and no ignored/filtered cases. These jobs omit the
Postgres package and cannot prove its advertised feature isolation; PG9 remains.
Two Feature isolation jobs merely aggregate matrix success, ZERO further runs.
Compiler fixtures each2/1/1PASS: push119.93/14.76/17.89s, PR115.66/15.39/18.17s.
Each of six invocations reviewed for owning diagnostics contract, actual compiler
fixture execution and unchanged expected diagnostics/selection. No native proof.

LibFuzzer jobs wire/header/chunk/value each61s:13,075,390/2,552,820/2,161,682/
9,438,340 iterations, no finding. AFL counterparts each60s/CMPLOG:
0/3/13/0 new items,0crashes/0timeouts. EACH of eight campaigns has three reviews:
unchanged target contracts/corpora; actual locked campaigns and duration;
unchanged failing-exit pipeline/inputs and no threshold relaxation. Clean AFL
campaigns do NOT prove the required saved-crash-with-zero-exit gate, still PG9.
Runner warnings about low-value seed cases remain retained in raw logs.

Mutation job118201437149 says No mutants to filter: executable Rust is unchanged.
Actual artifacts endpoint has0artifacts. No baseline, mutant execution or runtime
catch is credited; this zero-applicable result is distinct from prior PR396.
CodSpeed check118203806844 SUCCESS against205b01c4/main766:43untouched,3skipped
with baseline fallback,17syscall-instrumentation warnings. Existing four suite
job118201437526 completed, no benchmarks or thresholds changed. Three reviews:
unchanged benchmark contracts; actual owner run/report; skips and simulation
limits explicit, ZERO Postgres capacity claim. Cache service and Node-version
warnings are retained, without unrelated workflow changes in this slice.

Strict no-default/all-target Clippy passes15.70s and formatting passes after the
final five-law test source. Three reviews: supported no-feature adapter contract;
actual compiler completion with ZERO runtime tests; unchanged denied lints,
feature graph and all assertions. No capacity/timing claim. New deadline owner
297/380lines, existing README38/45lines, changelog2/3; zero production Rust,
API, dependency, migration or runtime changes, within preregistered budget.
Evidence copies remove trailing terminal blanks only; /tmp raw logs stay exact.
Metadata gate18512 remains live/frozen, dependency397 two live Nix jobs remain;
operations integration/full-suite proof waits for reviewed metadata parent.

## PR397 final Nix/CI review

Both final Nix Linux jobs nowPASS at exact205b01c4. Push118201421591 actual
workspace1586PASS133.949s3slow4compilerexclusions; PR118201438615 actual
workspace1586PASS140.900s4slow4compilerexclusions. Each run independently reviewed
three times: unchanged owning workspace contracts; actual executed counts with
compile fixtures separately exercised; unchanged filters/oracles and no no-DB
workspace guard counted as native evidence.
Each Nix job executes seven distinct native smokes plus full255native:
push69.309s0skip, PR61.742s0skip. EACH of these16 actual native invocations gets
three reviews: same selected capability/full transaction/order/state/recovery
contract; actual PG18 and patched listener graph; unchanged test bodies and
archive selection, smoke omissions explicit. Combined with standalone jobs,
four255-case Linux runs and28smokes execute. No echoed log is counted again.
Each job's replay2/1/1PASS1s each is THREE reviewed for unchanged corpus contracts,
actual bounded replay execution, and unchanged assertions/corpus/selection; no
capacity or saved-AFL-crash gate credit. All other named Nix checks PASS.

Final current snapshot:40SUCCESS/1scheduledSKIP/0threads, head205b01c4 GPGgood,
merge stateCLEAN. Normal exact-head protected squash next; no admin/waiver or
threshold changes. Sourcehash2803 unchanged, published delivery still pending.

## PG8 operations integrated with bounded startup

Metadata PR398 OPEN at reviewed signedb60fdf4929337654d7a785fe93d524d66371211f;
normal full gate12PASS including1588workspace566.907s,257native separately,
record amendments all12cached/0fresh runs. Dependency397 is already protected.
Operations WIP stash retained (pointer/tmp/mnesis-pg8-deadline-integration-stash).
Rebased onto reviewedb60; six research-only conflicts use ordinary Git union.
All parent append-only JSONL lines remain an exact prefix; JSONL validates.
Original deadline test source byte-identical to frozen297line candidate. Whole
product patch is byte-identical after removing only Git index hash headers:
parent metadata legitimately changed the README/changelog base-file hashes;
all owned hunk text, offsets, changes and test bytes stay exact. Original and
integrated patches remain retained, no weakened source comparison claim.

Integrated strictall/allClippy9.62sPASS (0runtime tests). Full actual native
262PASS63.889s0skip, compile10.77s,22binaries. Original five deadline laws run
alongside both startup bounds and all prior ordering/uncertainty/atomic/state/
backup/recovery/lifecycle laws. EACH actual run has three reviews:
1. Contract: original native SQLSTATEs and no-partial-event/key writes, healthy
   post-cleanup reuse, bounded startup and unavailable-server release compose
   with the existing commit uncertainty and durable recovery contract.
2. Execution/falsifier: all262 actual selected native laws execute,0skip; four
   wrong-config failures and two incorrect fixture-error assumptions remain
   retained and separately classified, not fabricated product defects.
3. Assertions/source/selection: original test source and owned hunk changes
   unchanged after parent integration, strict existing lints, no test omissions,
   watchdog/budget relaxations or new runtime/framework/dependency. Correctness
   timing here is not a capacity result. Original after-COMMIT timeout laws pass.

Normal signed full gate next, maxjobs1/source-index freeze until terminal.
Focused PR/current Linux/protected merge follow after parent398; do not stop at
this test slice, diagnostics and remaining PG7/8/9 delivery remain required.

## PR398 final Linux owners and delivery review

Exact signed headb60fdf4929337654d7a785fe93d524d66371211f now40SUCCESS/
1scheduledSKIP. Full downloaded joblogs /tmp/mnesis-pg4-metadata-job-<id>.log.
Standalone push118213729683 actual257PASS55.518s0skip; PR118213750325 actual
257PASS63.856s0skip, each plus seven distinct native capability smokes. Push
Nix118213729688 actual1588workspacePASS113.428s1slow4compilerexclusions,
257nativePASS58.006s0skip plus seven smokes/replay2/1/1each1s. PRNix118213750017
actual1588workspacePASS147.040s4slow4compilerexclusions,257nativePASS63.739s0skip
plus seven smokes/replay2/1/1each1s. Each native/workspace/replay invocation has
three independent reviews: selected owner contract and new startup-memory laws;
actual PG/native counts and compiler boundaries; unchanged tests/oracles/caps/
archive/filter policies with exclusions explicit. Installed run.log echoes are
ZERO new runs. Four257native runs/28smokes are distinct; workspace guarded tests
are not database evidence. All remaining named Nix owners passed.

Push and PR each execute three library unit modes on eight packages:
15/15/15kernel,1/1/1macros,217/217/217store,17/17/17wake,25/25/30inmemory,
131/131/131Fjall,0/0/0wake-nostd and store-testing. EACH of48invocations gets
three reviews: actual feature-owner contract; real --lib counts (zero cases earn
zero runtime credit); unchanged defaults/no-default/all flags and assertions,
no ignored/filtered tests. These still omit isolated Postgres feature execution.
Compile fixtures each2/1/1PASS: push121.80/15.32/18.57s, PR118.81/14.88/18.07s.
EACH of six invocations reviewed for intended diagnostics, actual compiler
execution and unchanged expected messages/test selection; no native credit.

LibFuzzer wire/header/chunk/value each61s:7,541,739/3,219,430/2,061,857/6,806,864
iterations, no finding. AFL/CMPLOG counterparts each60s:0/1/15/0new items,
0crashes/0timeouts. EACH of eight campaigns gets three reviews: unchanged target
contract/corpus; actual locked execution/time; unchanged failure pipeline/input
and no threshold/corpus relaxation. Clean campaigns do not prove the outstanding
saved-crash-with-zero-exit gate. CodSpeed118216049442 SUCCESS:43untouched,
3skipped/baselinefallback,17syscallwarnings. Actual four-suite job118213750495
completes; three reviews preserve benchmark contracts/execution/simulation and
skip limits, with ZERO Postgres capacity claim. Owner CI cache/deprecation
warnings remain retained. Mutation artifact baseline/sole E0277unviable review
is in diagnostics research; no runtime catch is credited from that artifact.

Final zero-thread/current-head/merge-state check then normal protected exact-head
squash; no admin/waiver/threshold changes. Full PG7/8/9 release acceptance remains.

## PG8 operations signed full gate and protected parent

Normal signed gate28287 exits0 at4fed170499a36911e4a51f2abf049db26a8aaffa G;
all12hostchecksPASS(10built2cached). Actual1593workspacePASS616.035s15slow,
4established compiler exclusions/137binaries. Three reviews: owning unchanged
workspace contracts plus five guard-returning tests; actual1593 execution with
262native proof separately retained, no guarded-host database credit; unchanged
workloads/assertions/filters and all exclusions explicit. Each actual replay
2/1/1PASS1s gets three reviews for corpus contract, actual bounded execution and
unchanged inputs/oracles/selection. Actual no_std/store/Wasm builds and named
docs/Clippy/fmt/TOML/Hakari/unwaived audit/deny checks complete; no hardware claim.
Raw owner logs/tmp/mnesis-pg8-operations-gate-* and mnesis-pg8-*-nostd/wasm retained.

Metadata PR398 protected-merged8df11b218d959622a26c743c31ca68e0b8782388 at18:14:56Z,
40SUCCESS/1scheduledSKIP/0threads. Entire reviewedb60tree exact;22focused merges,
not published. Signed operations rebase onto that protected parent givesf37a7492 G,
ENTIREtree equal first signed4fed proof; original proof branch and postgate stash
preserved. No source/behavior change and zero additional executions from rebase.
Normal evidence-only signed amend follows, inspect actual cache/runtime status,
then push focused PR/current Linux CI/protected delivery promptly. No approval
pending. Product remains297line test +README38/changelog2, zero executableRust,
API/migration/dependency/runtime/framework changes. Full diagnostics/capacity,
recovery measurements and PG9/release/registry requirements stay required.

## Required diagnostics slice started

Previous goal turn was progress: five required native deadline laws and four
wrong-config catches, not just a wait. Work is now isolated in
/tmp/mnesis-postgres-diagnostics at reviewed205b01c4; metadata full gate18512 stays
live/frozen and operations WIP remains untouched. Hypothesis/budget recorded
before code. No additional security edge-case scope. Existing tracing owners and
SQLx try_recv implementation inspected; collect original adapter absence before
adding production instrumentation. Protected dependency397 has39SUCCESS, one
Nixjob still live. Completed push Nix runtime reviewed separately in operations
record; final CI/merge still pending.

Original-production diagnostics fixture compiler preparation: cargo check12.81s
warns unused PgPool; strict Clippy rejects that import, std::sync::Mutex and95line
case. ZERO native tests execute. Three reviews: production unchanged and no
original-defect credit; actual compiler exit101; repair fixture under unchanged
80line/mutex/lint policy, not by allowances. Reuse existing parking_lot for the
synchronous subscriber callback and extract native opening into the fourth
planned collector/fixture helper. No new dependency package. Native observation
oracles, timeout and key/payload probes stay unchanged before first execution.

Original production diagnostics characterization executes1actual native case:
1FAIL4.062s0skip, run9d60e674-9903-4ad0-a0bf-830127e79889, compile5.31s. Native
append/conflict, both real reads, deadline arm and backend termination execute;
the first required readiness observation times out. Later event observations
are not reached and get no independent original-source catch credit. Three
reviews: (1) PG8 requires observable adapter work without payload/key fields;
(2) actual server operations pass their original value/conflict oracles before
missing-event failure, not a compiler/no-DB failure; (3) collector filters only
adapter target events/spans and exact event/outcome pairs, three-second bound,
no changed assertions or selection. Prior fixture-only compiler failure remains
zero runtime credit. Production instrumentation starts only after this red.

First instrumented compiler checkPASS11.49s; supported Hakari --diff needs no
manifest regeneration. Strict Clippy then fails on one redundant method closure
and read_all82/80lines. ZERO instrumented native tests execute. Three reviews:
existing feature/type/ordering contracts remain; actual compile pass versus lint
failure classified separately without runtime credit; repair under unchanged
lints. Remove the pre-existing redundant start=from alias and express the same
optional typed history validation with map/transpose (no boolean/error erasure),
so no helper/framework is needed. Replace the method closure by its owning trait
method. Original event oracles and test file stay byte-identical.

Instrumented strictall/allClippy passes (timing in fixed-native.log), then actual
1FAIL4.055s0skip. A required event is still missing; the shared helper did not
include its expected name/outcome, so do not guess which observation failed or
claim instrumentation complete. Three reviews: native semantics remain checked;
actual native failure is retained, not converted to compile/no-DB success; all
seven original observation oracles and inputs unchanged. Improve diagnostic text
only, then rerun the same event oracles before correcting any expected behavior.

The newly supported tracing feature is opt-in. Register its test only when that
feature is enabled; otherwise disabled instrumentation is not a promised output.
All-features native runs still execute the same observation case. Off-feature
empty diagnostics binaries get ZERO runtime credit; the preliminary original
production characterization deliberately enabled its fixture before adding the
feature. Preserve that exact186line fixture separately. This registration change
is explicit feature ownership, not an exclusion from the supported tracing mode.

Diagnostic-only rerun retains the same seven event expectations and fails
1case4.073s0skip. Captured native evidence identifies the wrong expectation:
ready, committed, conflict, both pages, deadline, and reconnected are present;
retry is absent. SQLx eagerly reconnects and returns None as its supported API
specifies. Three reviews: (1) distinguish successful automatic reconnection
from a failed retry attempt; (2) actual captured fields prove the missing pair
is listener/retry, not a missing readiness/page/fallback implementation; (3)
no oracle/timeout/input changed in this diagnostic run, full snapshot retained.
Correct that expectation specifically to reconnected, not arbitrary listener
activity. Also observe caller-pool closure while the store remains alive, naming
that terminal outcome closed rather than falsely calling it retry. Require the
actual native termination boolean and keep all value/no-payload assertions.
This adds explicit shutdown observation within the original test-line budget;
original missing-readiness red and both wrong-retry expectation reds remain.

Correct instrumented native casePASS1.615s0skip (compile3.07s), strictall/all
ClippyPASS15.35s. Run30c21de6-24d6-410a-a52d-b9b8d5028d3e. Three reviews:
1. Contract: real readiness, successful native reconnect, caller-pool terminal
   close, commit success/conflict, both scans and independent rescan are visible;
   native value/conflict/termination and no-key/payload checks still hold.
2. Execution/falsifier: actual one-case native path executes all eight exact
   observations; original missing-readiness red remains. The two wrong-retry
   expectation failures are fixture errors, not fabricated product defects.
3. Assertions/source/selection: same input/value/event oracles, except the
   documented correction to SQLx's actual reconnected signal and added terminal
   close oracle; enabled tracing mode executes the case, disabled feature gets
   no trace-contract/test credit. Default store/query/commit semantics unchanged.

Strengthen field assertions before the negative control: observed pages must
report the actual single row and an elapsed field; emitted field names stay in
the documented fixed set, with key/payload probe rejection retained. This tests
PG8 output values, not just presence. Original event cap and native workload stay.
Production102added/15removed lines within160/30, zero new helper/type/publicAPI,
optional reuse of existing tracing; test193/260 before this field check. No new
package. Hakari --diff0, no generated manifest mutation.

Stronger field-check preparation fails strictClippy at88/80lines; ZERO native
execution. Three reviews: exact row/elapsed/field-set contract preserved;
actual compiler rejection gets no runtime credit; no lints or assertion removal.
Have the existing observation helper return its actual captured field map, then
assert values from the two observed page records. This removes duplicate lookup
work and keeps the original four collector/fixture helpers and80line limit.

Final field-owning native casePASS1.175s0skip, compile4.79s; strictall/allClippy
PASS9.25s. Run328faa5a-d8b9-4b7c-9242-7b853c76923b. Three reviews:
1. Contract: actual page row counts are1, elapsed fields exist, emitted field
   names stay in the fixed documented set, and all eight exact observations plus
   value/conflict/native termination/pool-close/no-key/payload oracles pass.
2. Execution/falsifier: actual enabled-tracing native scenario executes; the
   existing helper returns observed records rather than reconstructing outputs.
   Original native absence red and wrong-expectation/compiler failures retained.
3. Assertions/source/selection: no weakened event/time/input oracle, four fixture
   helpers remain, disabled feature remains explicit0diagnostic runtime credit;
   body bounded under existing80line lint. No new production helper/API/state.

Next isolated negative control changes ONLY the all-page outcome label. The
unchanged native observation must reject it after actual reads and reconnection,
not from setup or compile failure. Candidate source and test copies retained;
never merge the wrong label. Existing full-scope integration remains required.

Isolated single-label control is a real runtime catch:1FAIL5.292s0skip at the
unchanged mnesis.postgres.page/all observation. Exact sole source change:
outcome all→missing; native stream/all data, commit/conflict, rescan and reconnect
still execute and captured fields prove the incorrect label. Three reviews:
(1) correct operational labels are part of PG8 output; (2) actual named native
failure at intended oracle, not compile/setup; (3) same candidate test/oracles/
workload, one-line control diff, no source mutation in the candidate worktree.
Control/tmp/mnesis-diagnostics-control.c5Zloj intentionally stays wrong, never
merge. Candidate original/corrected/field runs and all control files retained.
No independent negative-control credit is claimed for later field/close checks
that are not reached after this missing-page assertion.

Metadata PR398 current mutation artifact12209477724 downloaded and digest-checked:
36e3994e2285a64424ba4479ef3c826f495e5d3ec48dc2204fd413fd3b81f66c.
Actual baseline257PASS56.193s0skip (build45.703s); sole generated mutant replaces
validate_current by Ok(Default::default()). Reviewed its full diff and first
failure: E0277 History has no Default.1compiler-unviable/0runtimecatches/
0survivors/timeouts. This is not a scratch/build-baseline failure and earns ZERO
runtime mutation credit. Direct native original-allocation reds and independent
wrong-count/wrong-prefix controls remain the meaningful metadata guards. Three
reviews: actual native baseline contract; actual compiler-only mutant and exact
artifact/head; unchanged tests/selection/error/byte caps and no invented catches.
Current CI38SUCCESS/1scheduledSKIP/twoNixjobs live; protected merge still pending.

Complete candidate native suite on original205b parentPASS256cases124.100s,
0skip/22binaries, compile16.25s. Strict no-default/all-targetClippyPASS8.84s;
TOMLformat and Rustfmt pass. Three reviews after EACH actual run:
1. Contract: tracing instrumentation composes with all existing ordered reads,
   notification recovery, transaction outcomes, atomic writes, saved state,
   backup/recovery and lifecycle laws; no change to uncertain commit policy.
2. Execution/falsifier: all256 actual native cases run,0skip; original absence
   red and wrong-label control retained. The off-feature diagnostics binary
   earns0runtime credit. Compiler checks are not native executions.
3. Assertions/source/selection: unchanged non-diagnostic test sources and full
   selected feature suite, original native fault oracles preserved; fields/key/
   payload checks apply to enabled tracing. This parent lacks the two pending
   metadata and five deadline cases; integrated263 proof is still required.
No capacity conclusion follows elapsed time under the concurrent host gate.

PR398 is now protected-merged8df11b218d959622a26c743c31ca68e0b8782388,
18:14:56Z after40requiredsuccess/1scheduledskip/zero review threads.22focused
merges, not published. Its reviewedb60tree matches protected merge exactly.
The operations parent is undergoing its final record-only normal signed amend;
wait for that exact head before integrating this diagnostics candidate. No native
or other diagnostic compiler jobs remain active. Source/fields/control evidence
is ready for parent integration; full combined263 native and all delivery gates
remain mandatory. Do not add more security hypotheses while waiting.

Diagnostics integration onto final operations482de3c4 preserves the exact Rust/
manifest/lock patch b931c6cdcec36c9068518c9425d0359b28518219579575e5596aa7f55edd739b
and exact final test bytes. Four research conflicts and README/changelog use
ordinary three-way union; native deadline and migration sections remain intact.
A stricter added-lines comparison noticed the pre-existing separator blank is
shared rather than newly added. Full actual Diagnostics section (including its
final blank) compares byte-identical to the original, not just a text subset.
This benign patch-context difference caused ZERO native execution. Parent JSONL
prefix is exact/valid. Original entire-WIP stash and all conflict stages retained
via /tmp/mnesis-pg8-diagnostics-integration-{stash,path} pointers. Next actual
combined263native and strictchecks, then one normal signed full Nix gate.

## PG8 diagnostics combined native proof

Integrated onto operations482de3c4: strictall/allClippyPASS13.64s, native263PASS
62.368s0skip/23binaries, compile9.21s. Three reviews after EACH actual run:
1. Contract: all existing event order/checkpoint/atomicity/uncertain completion,
   bounded startup, configured deadlines, recovery and diagnostics laws compose.
2. Execution/falsifier: all263 selected native cases actually run,0skip; original
   missing-event red and isolated wrong-label runtime catch remain retained.
   This includes the two metadata and five deadline laws absent from the first
   256-case candidate-base run, not a second echo or guarded-host substitute.
3. Assertions/source/selection: exact Rust/manifest patch b931c6cd and final test
   bytes unchanged after integration; no caps/assertions/workloads/filters/lints
   relaxed. Parent JSONL prefix exact and full README section preserved. No
   capacity/soak claim from this correctness timing.

After integration proof, add two README lines clarifying missing terminal logs
cannot prove rollback; no source/test change or extra runtime claim. Final added
README40/45, changelog2/3, production102+/15- within160/30, test<=260, no helper/
new package/runtime/framework/tool. Normal signed full Nix gate follows with
source/index frozen until terminal. Parent399 must merge before delivery.

## PR399 final CI review

Exact482de3c4979d2dfb5ec05baca485d764289b89e6 G now40SUCCESS/1scheduledSKIP/
0threads. Downloaded actual owner logs/tmp/mnesis-pg8-operations-job-<id>.log.
Standalone push118227022658 actual262PASS65.667s0skip, PR118227999232
262PASS64.412s0skip; each seven distinct native smokes. Push Nix118227022954
1593workspacePASS140.742s4slow4compilerexclusions and262nativePASS67.239s0skip;
PRNix118227998891 1593workspacePASS104.977s2slow4compilerexclusions and262native
PASS58.575s0skip; each seven smokes and replay2/1/1PASS1s each. For EACH actual
native/workspace/replay invocation, three reviews: owning selected contracts and
five deadline laws; real executed counts/PG18database boundaries; unchanged
caps/assertions/workloads/selection and explicit compiler/smoke omissions. Four
native runs/28smokes are distinct; echoed run.log summaries earn zero new credit.
Host guarded tests are not native evidence. All named Nix owners passed.

Push/PR each default/no-default/all library modes: kernel15/15/15, macros1/1/1,
store217/217/217, wake17/17/17, inmemory25/25/30, Fjall131/131/131,
wake-nostd0/0/0/store-testing0/0/0. EACH of48invocations three-reviewed for owner
feature contract, actual --lib counts (zero cases get0runtime), unchanged flags/
assertions/selection. These do not prove isolated Postgres feature combinations.
Compiler fixtures each2/1/1PASS: push121.76/14.94/18.98s, PR122.86/16.48/19.89s;
EACH of six reviewed for diagnostics contract, actual compiler execution and
unchanged expected messages/selection, not native credit.

LibFuzzer wire/header/chunk/value each61s:9,841,073/2,082,531/1,568,423/5,675,868
iterations. AFL/CMPLOG each60s:0/2/15/0new corpus items,0crashes/0timeouts.
EACH of eight campaigns three-reviewed for unchanged target/corpus contract,
actual locked runs/durations, unchanged failure pipeline/inputs/thresholds.
Clean campaigns do not close the saved-crash-with-zero-exit gate. Mutation job
118227999611 says No mutants to filter (only tests/docs changed), no baseline or
runtime mutation is credited. CodSpeed118230145987 SUCCESS:43untouched,
3skipped/baselinefallback,17syscallwarnings; actual four-suite job118227998061
completed. Three reviews retain benchmark contract/execution/instrument limits;
no Postgres capacity or soak proof. Owner cache/deprecation warnings remain raw.

All four direct wrong-configuration catches and the corrected original fixture
assumptions remain in the committed research record. Normal exact-head protected
squash next, no admin/waivers/threshold changes; full PG7/8/9 acceptance remains.

## Diagnostics signed gate and release-parent integration

Normal signed commit eb2c73410985e2705af62b90a7f6cf12b97a6bc0 completed the
unchanged twelve-check Nix gate successfully. Actual workspace nextest:1593PASS,
532.590s,15slow,4 compiler-fixture exclusions,138 binaries. Replay2/1/1PASS,
1s each; no_std/store-no_std/Wasm checks built successfully. Raw local gate,
nextest and replay logs remain /tmp/mnesis-pg8-diagnostics-{hook,gate-nextest,
gate-replay}.log. Three reviews for EACH executed suite: (1) owning workspace
and replay contracts; (2) actual counts/exclusions and completed derivations;
(3) unchanged flags/assertions/selection. The ordinary workspace gate does not
enable Postgres tracing: its empty diagnostics binary earns zero tracing-runtime
credit. The separate263-case real-database suite supplies that evidence. No
capacity or soak credit follows from correctness-suite timing.

The user merged PR399 as e7bb9f4560ddb05a3148a8d5841312ec2ef3afb0, then
release PR378 as787d61c7a75d4b8b3e4a867fbe352751d4344469. Preserve the0.5.0
versions, internal requirements, three locks and changelogs. Signed diagnostics
rebased to f92f8abd; exact production Rust patch and diagnostics fixture compare
identically against retained pre-rebase copies. This comparison is NOT a new
execution; rerun native/strict/full checks for the changed0.5 graph.

Release39305353720 job118240859105 SUCCESS published all nine crates and tags.
Native release log explicitly confirms mnesis-postgres0.5.0. crates.io API403
provided no registry proof; the primary sparse registry index independently lists
0.5.0, not yanked, checksum
00cd69105598a23f428b6bf02bf46476bd60e58551304d33efc9c78debbdfbbc.
Release tag published2026-10-19T18:49:44Z. Raw /tmp/mnesis-release-0.5.0.log.
This is verified publication of the delivered subset, not whole-PRD completion;
remaining diagnostics/soak/capacity/recovery/feature/fuzz/docs acceptance and final
clean registry consumer still apply.

Current0.5 graph strict all-features/all-targets ClippyPASS7.39s, no-default/
all-targets ClippyPASS4.89s; Rustfmt/Taplo/Hakari diffPASS. Three reviews for
each compiler invocation: feature/target contract; completed real compiler
execution, zero test-runtime credit; unchanged deny-level lints, selected targets
and assertions. Exact pre-release production patch and diagnostics fixture still
compare identically. Native0.5 suite is independently running; no aggregate
pass is inferred from compiler or partial results.

Current released-parent native run70753fdf-23cf-4148-a834-3994d8770883:
263PASS147.944s,0skip,23binaries,17.17s compilation excluded. All features select
real Postgres through DATABASE_URL. Three independent reviews:
1. Contract: actual diagnostics eight outcomes/rows/timing and native reconnect/
   close pass; ordering/resume, definite rejection versus unknown finalization,
   atomic import, saved pairs, migration and recovery regressions remain selected.
2. Execution/falsifier:263 actual executed cases, zero guard-only substitutions;
   original readiness and isolated all-label runtime failures remain retained.
   Slower147.944s (including one21.841s import-failure case) is recorded, not
   called capacity, parity, or dismissed as measured noise. It supplies no SLO.
3. Assertions/source/selection: production patch/test bytes identical across
   release rebase; original deadlines/caps/expected outcomes and full suite
   flags unchanged. No filter/exclusion/lint/threshold weakened. No uncertain
   commit becomes retry-safe; no capacity/soak claim from correctness timings.
Move the new changelog entry under Unreleased:0.5.0 is already published. No
published release history is altered. Current Rust patch checksum625c798bf11891f8
(full /tmp/mnesis-pg8-diagnostics-current-rust.patch). Normal SIGNED full Nix gate
next; freeze source/index until its actual terminal outcome.
Replay current0.5 gate: actual CBOR2PASS/header+chunk, value1PASS and wire1PASS,
1.00s each. THREE reviews separately for each invocation: owning decoder/value
contract; actual locked native test execution/counts/duration, no empty harness;
unchanged target/fixtures/inputs/assertions/filter. No PG database, long fuzz
campaign, AFL saved-crash gate or capacity credit from these replay executions.
Raw /tmp/mnesis-pg8-diagnostics-release-gate-replay.log. Gate still running;
records prepared outside frozen checkout and will be appended after terminal.

Current0.5 workspace: actual1593PASS672.998s,18slow,4compiler exclusions;
three reviews: (1) owning full workspace contracts, including unchanged long
Fjall property law; (2) completed actual release/coverage native execution,
no echo counted twice, guarded Postgres cases supply zero real-DB credit;
(3) unchanged not-test(compile_fail) selector/source/assertions/deadlines/lints,
no reduced property cases or workload. Empty tracing-disabled diagnostic binary
gets zero trace-runtime credit. Separate263native suite supplies Postgres proof.
No capacity/parity claim from the longer host regression runtime. Do not treat
four excluded compiler fixtures as native Postgres skips; CI owns those fixtures.

Signed4e7e636408d7a22d77d0c615dda0d6514cd6593a G completed the unchanged
normal twelve-check Nix gate, terminal0. Both no_std owners and Wasm completed;
those compile-target checks earn zero test-runtime credit. Current workspace
and replay counts/reviews above are actual new0.5 executions, not old0.4 logs.
Raw/tmp/mnesis-pg8-diagnostics-release-{hook,gate-nextest,gate-replay}.log.
Exact Rust patch remains625c798bf11891f839fcb1b10f5c67c5968e367476921c2fa8551895ad918d88.
Normal record-only signed amend next; inspect cache results and credit zero new
runtime when outputs are reused. Then focused PR and Linux review/protected
merge promptly. Capacity/soak/recovery/PG9/final registry acceptance remain open.
## PG8 soak benchmark preparation

Standalone benchmark follows existing Fjall Cargo reporting targets, reusing
Postgres cluster/allocation owners unchanged. Private database, no caller URL
or data is used. First strictall/allClippyPASS11.71s, Rustfmt/TOMLformatPASS,
436/500Rust lines and manifest6lines, zero production/dependency changes.
Three reviews: declared workload/ownership contract; actual compiler-only pass
with ZERO soak runtime credit; unchanged lints and no hidden skipped native test.
The first explicit10s native run is a smoke test only. Full1800s acceptance waits
for final diagnostics source integration and local gate quiescence. Native
failure control will disable only the rescan interval and must hit the original
3s no-notification oracle; an incorrect ordinal must fail the actual fold.

First actual native soak smokePASS: requested10s,10.002s steady workload/
10.268s including close,4,458rounds/20,070events/one fault-restart cycle,
44connections<=52. Baseline1,085,359allocator bytes, peak1,871,689,
retained103,910after pool/server-session quiescence;10,904,246,495 allocated
bytes during measurement (NOT allocation count or RSS). Native PG18.6 settings
fsync/full_page_writes/synchronous_commit allon, fixed128rows/1MiB pages.
Optimized benchmark compilation20.14s, excluded from steady-work timing.

Three independent reviews after this actual run:
1. Contract: each real competing round has one winner/15conflicts; all16native
   state/position pairs, strict event ordinals/positions, global count and cleanup
   assertions pass. Native unnotified final event resumes within3s with no later
   write; fresh reader and actual terminated/reconnected listener are exercised.
2. Execution/falsifier: actual private Postgres cluster and release executable,
   no URL guard or skipped test. Declared10s earns smoke-only credit; zero30min,
   full capacity/quantile/RSS or deployment-SLO credit.
3. Assertions/source/selection: all preregistered deadlines/resource caps and
   native error classification intact; no unknown write is retried. Original
   source/control copies will be retained before each independent falsifier.

First isolated native falsifier exits101 at the intended3s unnotified-final-
commit oracle. Sole library change RESCAN_INTERVAL1s→3600s; release build5.67s.
The private server/real competing warmup and saved consumers initialize, then
missing-notification recovery times out rather than relying on another write.
Three reviews: (1) independent rescans are necessary for the stated recovery
bound; (2) actual native runtime timeout, not compile/setup/empty execution;
(3) original benchmark,3s deadline/caps/workload unchanged, exact one-line diff.
Candidate worktree/source untouched. Restore only that interval in the control
and change only the produced ordinal; actual folded values must reject it.

Second isolated native falsifier restores the correct rescan source, then shifts
only the produced ordinal by+1. Actual benchmark exits101 at the real consumer
fold: byte-encoded2received versus1expected, before any recovery-phase credit.
Release compilation5.31s. Three reviews: exact payload/state sequence contract;
actual native read/validation failure (not compile/setup/timeout); unchanged
benchmark assertions/caps/deadlines and sole ordinal-expression diff. The main
JoinError faithfully propagates the worker assertion. Candidate source remains
unchanged, controls stay isolated/wrong and are never merged. Neither short
control supplies30-minute or capacity acceptance. Raw logs/diffs retained.

Upstream main advanced concurrently beyond operational merge399 to a0.5.0
release version change. Preserve that work; inspect exact release/publication
state and rebase diagnostics, then this benchmark onto the resulting reviewed
source. Prior0.4checks remain evidence for those revisions, not an invented
0.5rerun. Full1800s execution still waits for current-source integration.

Soak source integrated onto signed diagnostics f92f8abd and released0.5.0
requirements. Three research-file append conflicts resolved by standard
three-way union, retaining both independent sections; no conflict in production
or fixture code. Benchmark compares byte-for-byte against the pre-integration
copy. Manifest retains0.5requirements/optional tracing and adds only the five
Cargo reporting-target lines. Earlier smoke/controls stay exact evidence for
the earlier source; fresh current-source smoke and full1800s follow. Original
WIP stash and all conflict stages retained under /tmp/mnesis-soak-union-*.
README adds the ordinary native command and distinguishes short smoke credit
from the default full30min. No native execution follows from this integration.

Released-parent0.5 current-source smokePASS: strict all-features/all-targets
Clippy11.64s (compiler only), formatting/TOML pass, optimized build5.29s excluded.
Actual private PG18.6/on/on/on,10.002514s steady work/10.382710s through close,
3,659rounds/16,478events/one real fault cycle,44connections<=52. Baseline
1,083,422/peak1,858,314/retained104,102 allocator bytes;8,950,005,035 allocated
bytes measured, not count or RSS. This run overlaps the independent host Nix
gate and makes no throughput comparison, noise-floor or capacity claim.

Three reviews after this actual run: (1) exact competing winner/conflict counts,
16reloaded state/position/revision pairs, ordinals/global order/count and real
listener/restart/no-hint recovery/cleanup assertions; (2) actual private database
and release executable ran10s with zero guard/skip substitution, so smoke-only
credit and ZERO30min/full-capacity credit; (3) exact benchmark bytes unchanged
from preregistered smoke/control source, original3s bounds/resource caps/input
workload/uncertainty policy unchanged. Compiler-only checks receive zero native
credit. Earlier isolated rescan/ordinal controls remain meaningful failures.

Full1800s waits only for active normal diagnostics Nix gate32709 to finish.
Do not broaden this slice into unrelated security work. Standard Cargo bench
runs the existing adapter under a recorded profile; it is not a new test/tool
framework. Fourteen README lines and five manifest lines stay within budgets.

Full soak preregistered execution: integrate onto final signed diagnostics
ca88dbb6c15fa6d7b0ee8611a3ed1e1cb20ff54f(PR402). Only two research append
conflicts resolved by standard union; parent JSONL prefix exact and valid. Exact
benchmark/support bytes compare to prior0.5 smoke; no production Rust/lock change.
Normal diagnostics full gate and record-only cached amend are terminal; no native
suite/build gate remains active. Begin the default1800s native benchmark with
projection enabled. Source/support checksum manifest and UTC start are retained
under /tmp/mnesis-pg8-soak-full-*. No full-soak credit before terminal assertions.

Diagnostics PR402 opened at signedca88dbb6. The record-only signed amend passed
all twelve checks from cache: no build entries and ZERO new runtime executions.
Full1800s soak session13029 is now actively executing the optimized benchmark;
actual PG18.6 settingson/on/on, first real fault cycle passed at1.020468s,
44connections. This is partial progress only, not acceptance. Native compilation
3.38s excluded. Do not change benchmark/support/product source or start another
native suite/full build gate until terminal. Raw/tmp/mnesis-pg8-soak-full.log,
source hashes/tmp/mnesis-pg8-soak-full-source.sha256 and UTC start retained.

## Full soak failure, retained before any fixture correction

Session13029 TERMINAL101 before1800s. Last progress61.608546s/28,913rounds/
130,122events/two faults/44connections; later actual consumers assert183,745
versus183,753 at original advance count check. More than one consumer fails at
the same eight-event boundary; main propagates JoinError. All raw output and
exact pre-correction source retained under/tmp/mnesis-pg8-soak-{full-first-failure.log,
first-failure.rs}; checked-in log copy strips trailing blanks only. No full-soak
or capacity acceptance. Private cluster cleaned after panic, so the cause of this
specific delayed boundary is not independently settled from its database state.

Three reviews after this actual failed run:
1. Contract: exact payload/position/CAS laws remain; finite read_all captures a
   safe cutoff and can legitimately exclude acknowledged events while an older
   transaction holds visibility. Append returns a token that a consumer awaits,
   not a promise every finite scan includes it immediately (store trait source).
2. Execution/falsifier: real optimized native benchmark, not compiler/setup/guard;
   actual model-count assertion fails, no new checkpoint from those failed
   consumers. Two completed minute faults do not supply30-minute acceptance.
3. Assertions/source/selection: exact frozen436line source and original3s recovery,
   10s cohort/resource assertions; no bound or workload was relaxed. The finite
   scan assumption is stronger than the owning contract. Pending conflict rollback
   is a candidate cause, not proven from the cleaned failed database. Reproduce
   the owning watermark case natively before selecting a consumer correction.

Actual owning-contract reproduction: existing native subscription test1PASS
1.167s,2explicitly unselected cases, compile10.00s excluded. It holds an older
real transaction, obtains a successful append token while no event is eligible,
releases the transaction without a Mnesis hint, and receives the exact event/
position through the real subscription within3s. Three reviews: exact owner
visibility/barrier and no-later-write contract; actual one native execution,
not guards or all3cases; unchanged assertions/deadlines/selection and source.

Select the existing Subscription consumer for soak advance, rather than a
single finite read_all. Await the exact known event count under the EXISTING3s
recovery deadline; retain strict ordinals/positions, exact expected count before
checkpoint, full saved-pair/CAS assertions, original10s cohort watchdog, final
native count,30min/fault/resource caps. No production change, timeout increase,
unknown retry or new runner. A permanently missing event must still time out;
duplicates/reordering fail ordinals or final native count. This corrects the
proven stronger-than-contract finite-scan assumption; it does not assert the
specific first failed database's pending-rollback cause was proven. Budget stays
500benchmark lines/15helpers; code/control copies and red run remain preserved.

First subscription-fixture strict check fails at drop(Pin<&mut Stream>), which
releases only the reference, not the pinned future. No smoke runtime occurred.
Three reviews: owned cursor lifetime contract; actual compiler-only red/zero
runtime; unchanged lint policy/assertions. Use a lexical scope so the actual
pinned future drops before checkpoint writes, without another allocation or lint
allowance.450/500lines before the scope formatting; no production change.

Subscription consumer current10s smokePASS:10.002456s steady/10.168734s through
close,4,886rounds/21,996events/one actual fault,44connections. Baseline1,075,292,
peak1,845,649,retained104,102 allocator bytes;10,373,383,414allocated bytes,
not count/RSS. Optimized compile2.81s excluded. Strict all/all check and format
pass with original lints; native PG18.6 settingson/on/on.450-ish/500source lines.
Three reviews: (1) exact event/position/state/CAS/count and recovery/quiescence
contracts; (2) actual10s private native executable/zero guards, no30-minute or
capacity credit; (3) original3s deadlines,10s cohort watchdog, resource/fault
caps/count/ordinal assertions retained. Live consumption now follows the owning
contract rather than equating finite cutoff with completed writes. Rerun the
rescan/ordinal falsifiers against this exact new fixture before the full1800s.

Current subscription-fixture rescan control: actual native TERMINAL101, optimized
build5.39s, expected3s unnotified-final-commit timeout. Warmup winner/conflicts/
all16subscription checkpoint consumers initialize successfully; the deliberate
one-line1s→3600s rescan change blocks the no-later-write recovery. Three reviews:
original3s recovery contract; actual native intended timeout rather than compiler/
setup/empty run; exact new subscription fixture, unchanged assertions/inputs/caps
and sole library interval diff. Candidate product untouched. Restore that one
control source and shift only the producer ordinal for a second native falsifier.

Updated ordinal control native TERMINAL101 at real subscription fold: byte-
encoded2received versus1expected, optimized compile5.50s. Correct rescan source
restored byte-exact; only produced ordinal shifts by+1. Three reviews: strict
state/event sequence contract; actual native assertion and propagated JoinError,
not compile/setup/timeout; exact new fixture assertions/inputs/caps/deadlines,
sole ordinal-expression diff and no candidate production mutation. Recovery
later in this control remains unexecuted/zero credit. Both new controls remain
isolated and intentionally wrong; never merge. Candidate451/500lines, existing
3s/10s bounds unchanged. Strict compiler-only check2.84s had zero runtime credit.

Restart full1800s using the corrected real subscription consumers, source frozen.
The earlier terminal101/log/source and contract reproduction remain explicit.
New raw/tmp/mnesis-pg8-soak-subscription-full.log, source hash manifest and UTC
start under the same subscription-full prefix. No full acceptance until actual
terminal invariants, resource release and thirty-minute/fault counts pass.

## Diagnostics PR402 completed CI review

Reviewed signedca88dbb6c15fa6d7b0ee8611a3ed1e1cb20ff54f:40SUCCESS, one
scheduled mutation sweepSKIP, zero review threads and no further thread page.
Actual logs retained /tmp/mnesis-pg8-diagnostics-job-<id>.log. Standalone native
push118259623506263PASS58.814s and PR118259647311263PASS73.385s, both0skip.
Nix push118259623480:1593workspace105.016s2slow/4compiler exclusions and
263native60.242s0skip. Nix PR118259647624:1593workspace143.854s4slow/4excluded
and263native69.203s0skip. Each of four native executions also passes seven
one-case smokes; each Nix owner runs replay2/1/1PASS1s each. Echoed run.log is
not another native execution. For EACH actual native/workspace/replay/smoke,
three reviews: owning selected contracts including actual tracing case; real
counts/database or explicit guard/compiler boundaries; unchanged source/flags/
assertions/caps/selection, no omitted enabled case. No capacity timing claim.

Six compiler fixtures2/1/1 each:push117.06/14.35/17.84s,PR125.50/15.94/19.20s.
Each three-reviewed for intended diagnostics, actual compiler execution/counts,
unchanged expected output and selection.16 library jobs each default/no-default/
all: kernel15/15/15,macros1/1/1,store217/217/217,wake17/17/17,inmemory25/25/30,
Fjall131/131/131,wake-nostd0/0/0,store-testing0/0/0. EACH of48invocations reviewed
for owner feature contract, actual --lib execution/counts (zero gets0runtime),
and unchanged flags/assertions/selection. These do not close PG9's missing
Postgres isolated native feature matrix or dev-unification proof.

LibFuzzer wire/value/chunk/header each61s:11,127,830/8,370,834/1,286,392/
2,437,707iterations. AFL/CMPLOG wire/header/chunk/value each60s:0/2/13/0new
items,0crashes/0timeouts. EACH campaign three-reviewed: actual selected target/
corpus contract; executed locked campaign/duration/counts; unchanged failure
pipeline/inputs/thresholds. A clean campaign does not prove the saved-crash/
zero-exit gate. CodSpeed actual four-suite job118259646872 SUCCESS; analysis
43untouched/3baseline-fallback skips/17syscall warnings. Three reviews preserve
source/benchmark contract, actual simulation run and declared instrument limits;
zero Postgres capacity or native walltime credit.

Mutation artifact12215538740 ZIP checksum47361bcba42dd00166df5bdf9dc14c15b26b412eaedc5dec344de52946ac94fa
matches metadata. Retained ZIP/extraction /tmp/mnesis-pg8-diagnostics-mutants*.
Actual native baseline263PASS66.668s0skip.17mutants:2runtime caught,15compiler
unviable,0missed/timeouts. commit→Ok fails real returned history1vs2 at14/263:
13PASS1FAIL0.254s, remainder canceled. listen→Ok fails real listener acquisition
at100/263:99PASS1FAIL14.931s, remainder canceled; new tracing case105notreached,
so do not attribute those catches to it. Each baseline/mutated runtime three-
reviewed for exact storage/readiness invariant, actual native failure/counts,
unchanged source/oracle/selection. All15unviable logs inspected: unsupported
Default/new/from_iter/from bodies are compiler failures, ZERO runtime catches.
Direct wrong all-label tracing control remains its independent oracle evidence.

Main advanced through user release PR4010525d517 after PR4009f7116c7. These
change versions/locks/changelogs, not Rust implementation. Preserve all of that
work. PR402 tests the recorded0.5.0 graph; no invented0.5.1 rerun. Protected
ruleset does not require latest-base checks; use ordinary exact-head squash,
verify the resulting tree against the standard three-way merge and retain final
current-graph verification/registry consumer as whole-goal requirements.

## Second long-soak red — configured lock rejection

Interrupted session88724 now confirmed TERMINAL101. Last actual progress
1269.6655805s/459,808rounds/2,069,166events/22faults/44connections; subsequent
append returns Sqlx SQLSTATE55P03 while checking keys exclusion constraint under
unchanged1s lock_timeout. No full1800s/final-quiescence acceptance. Raw/source
preserved before edits under/tmp/mnesis-pg8-soak-{subscription-full-lock-failure.log,
lock-failure.rs}; checked-in log copy strips trailing blanks only.

Three reviews: (1) no ordering/state/CAS violation was reported, but original
benchmark rejects this configured definite pre-COMMIT failure rather than a
Conflict; (2) actual native1269+s run and exact error, not compile/guard; later
assertions unexecuted get zero credit; (3) original deadlines/workload/unknown-
error rejection unchanged. Verify native exclusion-lock rollback and the owning
phase mapping before accepting a separate known-rejection counter. Never relabel
it Conflict, increase timeouts, retry uncertainty, or erase the red to pass a run.

Native exclusion characterization2PASS8.732s (37explicitly unselected), compile
10.21s excluded: exact55P03/zero keys+events after independent blocker rollback,
healthy subsequent append and inventory(1,1), plus existing four-code COMMIT
boundary test. Three reviews: exact pre-COMMIT exclusion/settlement contract;
actual two native tests and explicit omissions; unchanged deadlines/assertions/
classification and zero uncertain retries. Source review shows current COMMIT
code set did NOT include55P03; do not claim that code's runtime coverage yet.

Extend the two existing phase tests by55P03 (<=5existing test lines), retaining
all original codes/outcomes: before COMMIT known Sqlx, after COMMIT Unknown.
No production mapper change. Reproduce this exact phase difference before the
soak permits a separately reported pre-COMMIT lock rejection.500benchmark/
380deadline-file budgets unchanged; no timeout or acceptance bound increase.

Diagnostics PR402 merged2026-10-19T23:05:15Z through ordinary exact-head protected
squash:5b8680280854c397c957140784ba7795bcb488a0. All40checksSUCCESS/one scheduled
SKIP/no threads. Entire resulting tree equals standard merge-tree
e1fd9b70203e80892480cfa38ee552fb27024a38, preserving user0.5.1 release401
0525d517 and prior400changelog work alongside the reviewed diagnostics. This is
the24th focused adapter fix; full PG0–PG9 acceptance remains incomplete.

Exact phase characterization now3PASS11.783s (36explicitly unselected), compile
6.61s excluded. Both five-code loops execute55P03: deferred rejection before
COMMIT remains Sqlx with zero rows; after COMMIT submission remains CommitUnknown.
Native exclusion blocking separately returns exact55P03 before creating event/key,
then independent rollback inventory is zero and a healthy append creates(1,1).
Three reviews: exact phase/error/settled-data contract; three real native cases
and explicit omissions, not only the previously four-code test; unchanged original
cases/assertions/deadlines/unknown outcome and no automatic retry. All reds stay.

Retain1s lock and5s statement/3s recovery/10s cohort/1800s soak limits. Count only
typed Sqlx/database55P03 as a separate known rejection; never label it Conflict.
One exact winner remains required; all other requests must be Conflict or that
verified pre-COMMIT rejection. Every other error, especially CommitUnknown with
any SQLSTATE, remains fatal. Native consumer/global counts still require exactly
the one winning batch, so a falsely rejected committed write cannot disappear.
Report rejection counts with workload counters rather than claiming zero failures.
This addresses a configured deadline under load, not a storage correctness change
or an increased timeout. Preregistered original zero-invariant requirement stays.

## Current protected parent and release verification

Soak WIP preserved in a retained stash before integration onto protected
5b8680280854c397c957140784ba7795bcb488a0 (.5.1); stash applied without conflicts.
Only benchmark counters/error reporting change; production Rust stays exact.
Initial current-source format check failed on import order/wrapping before any
lint/runtime execution. Formatting corrected through pinned rustfmt; strict
all-feature/all-target Clippy passes15.64s. Zero runtime evidence from those checks.

Release39312731245 independently reports published mnesis-postgres0.5.1. Primary
sparse registry confirms not yanked with checksum
 aa483fa702ad0dd67a0ba941704bc7b196cfbc5925ac8088cc6bda40ea61e7cb.
Raw/tmp/mnesis-release-0.5.1.log. Publication covers that revision, not remaining
PG8/PG9 or the later diagnostics merge; final registry consumer still required.

Current protected .5.1 phase rerun:3PASS5.194s/36explicitly unselected, compile
6.79s excluded, nextest a48e9a2b-cf8c-4f9f-b0eb-4426d436e866. Raw
/tmp/mnesis-pg8-soak-current-lock-phases.log. Review1: independent inventory after
real exclusion-blocker rollback proves zero keys/events, healthy append proves
continued availability; exact phase tests keep55P03 known before COMMIT and
unknown after submission. Review2: all three native cases actually execute,
.152/4.882/.158s with DATABASE_URL set; five literal codes each phase, native
observation CommitSent and row counts, not empty/guarded binaries. Review3:
compared owning source/diff, original four SQLSTATEs/assertions/deadlines remain;
production Rust unchanged,36unselected cases earn no rerun credit. No unknown
retry introduced. Counter-only fixture475/500lines; every prior stress red retained.

Current .5.1 counter-policy smoke completed exit0:10.003503542s steady,
10.160068458s through close;4818rounds/21690events/1fault/0known lock rejections,
44connections, baseline1104546/peak1843444/retained103974bytes,
10228637897allocated bytes; release compile5.70s excluded. Raw
/tmp/mnesis-pg8-soak-known-rejections-smoke.log. Review1: exact ordinal fold,
strict positions, revision/state/position readback, native final count and cleanup
assertions all reached. Review2: real privatePG18.6 on/on/on,16producers/16consumers,
actual listener replacement/kill/reconnect/no-hint event; smoke is NOT1800s
acceptance and no lock rejection occurred, so that branch relies on separately
executed native phase laws, not this smoke. Review3: source diff retains all
1s/5s/3s/10s/1800s and resource bounds, no unknown error acceptance/retry, production
source unchanged. Counter-source SHA256
61d3f49305c17c97d6baa616aaa616c7c3abd4a54318dc27f19420ca11197206.
Start full default1800s on this frozen source; every prior red remains retained.

Backlog delivery pointer updated to24 protected changes through402 and verified
published0.5.1, preserving all existing dependency edges and historical evidence.
PG9 active reflects the verified, preregistered CI implementation now underway;
its required PG5–PG8 delivery prerequisites remain visible and unsatisfied until
final acceptance. No whole-adapter release or clean registry consumer claimed.

## Full current-parent native soak passes

Session71259 TERMINAL0. Actual steady work exceeds1800s: final cohort/fault
report1812.763109s; independent native final-count report1812.797935834s;
through complete client/backend/private-cluster close1814.44915625s. Native
PG18.6, fsync/full_page_writes/synchronous_commit on/on/on.591768rounds,
2662995events including8warmup/31unnotified fault events,31actual reader-store
replacements/listener kills/reconnections/fallback recoveries/rebuilds;0known
lock rejections this run. Observed periodic backend high46 (configured max52).
Baseline1071253/peak1879613/retained103974bytes,1294346952265allocated bytes
of client traffic. Allocation traffic is not retained memory or process RSS.
No deployment throughput/latency SLO is claimed from this correctness soak.

Three independent reviews AFTER the actual terminal:
1. Contract: each round has exactly one winner and15conflicts/rejections; every
   consumer validates strictly increasing positions and exact ordinal payloads,
   saves revision+1/state/position and independently reads that exact pair. Every
   fault arms before the final no-NOTIFY commit and must recover within3s with
   no later write. Exact native event count, >=30faults, >=1800s, pool size0 and
   zero owned backend sessions within original3s are reached before completion.
2. Execution/falsifier: real optimized native process exits0 and reaches final
   close/retained assertions, not a smoke/compile/guard/echo. Model arithmetic is
   295884*(1+8)+8+31=2662995, matching the native assertion. Both prior long reds,
   finite-watermark law and real rescan-disabled/ordinal failures remain. This
   successful run has0lock rejections, so the new typed rejection branch gets
   phase-law evidence, not fabricated stress execution credit.
3. Source/assertions/selection: benchmark SHA256 still
   61d3f49305c17c97d6baa616aaa616c7c3abd4a54318dc27f19420ca11197206, verified by
   recorded pre-run checksum.475/500lines and14/15helpers; deadline file339/380.
   Production Rust unchanged; all1s/5s/3s/10s/1800s/fault/resource bounds and
   unknown-error rejection remain. CommitUnknown is always fatal; no retry.

Raw full output/start/end/checksum under/tmp/mnesis-pg8-soak-known-rejections-full*;
checked-in full log retained. This establishes this native soak workload only;
complete current regression/native CI, signed normal gate, capacity/recovery,
final acceptance/minimization/publication/registry consumer remain required.

Delivery minimization decision before edits: carry the two public index lines and
quickstart correction in this workload parent rather than making a second docs
patch. Revised adapterREADME budget20added/4removed (14workload +4quickstart,
plus wording for reported lock rejection); rootREADME<=2added. These describe the
same verified adapter/workload, zero production/interface/runtime changes.
Keep their identical copies in the PG9 WIP; three-way parent integration should
retain the single source fact without duplication. One focused workload PR first,
then combined PG9 CI; no extra full-gate rerun just for duplicate doc edits.

Complete current-parent native regression run finishes264PASS73.287s/0skip across
23binaries, compile21.25s excluded. Raw/tmp/mnesis-pg8-soak-full-native.log,
nextest invocation includes --all-features --no-tests=fail --test-threads=1 and
actual DATABASE_URL. Three reviews after terminal: (1) original ordering/resume,
missing final hints, commit/timeout uncertainty, migrations, atomic import,
snapshots/checkpoints, backup/restore and lifecycle laws all remain selected;
new exclusion law and both five-code phases pass; (2) actual264 completed native
suite cases, not compile/guard/echo, no new correctness or timing claim from the
separate1800s result; (3) production source unchanged, old four SQLSTATE cases and
all assertions/deadlines preserved, new typed55P03 never becomes Conflict or a
retry of CommitUnknown. Original rescan/ordinal/finite-scan/lock-timeout reds stay.

PG7 measurement refinement before edits: existing physical drill already reports
backup and stopped-primary-to-reloaded-projection time; existing logical drill
asserts restore/rebuild/reopen/advance but emits no timing. Add observation only
(backup_tests.rs<=12added lines, no assertion/selection/timeout change), then
measure both unchanged scenarios three times. Explicit profile: two events in
two streams, projection state4, then one increment→5; do NOT call that four events
or a large deployment recovery benchmark. RPO0 applies only to this quiesced
source fixture; measured readiness excludes external routing/provisioning and
controller drain. No target SLO is invented. Existing native fixtures and test
owners suffice; no new harness/framework/tool/runtime/production change.

Logical timing observation adds9lines within12budget, no existing assertion or
branch changes. Current strict all-feature/all-target ClippyPASS17.03s/Rustfmt
PASS. Three reviews: selected test/feature compilation; actual compiler-only
completion with zero runtime/measurement credit; unchanged lint levels,
assertions/deadlines and nine observation-only lines. Full264-case result above
precedes this test-only observation; current modified owner is measured next and
complete current Linux suite remains a delivery requirement.

Recovery measurement1:2PASS6.213s/18explicitly unselected; compile3.81s excluded.
Logical PG17.10 on/on/on: backup2.680417ms, fresh-target-to-reloaded-pair48.200084ms;
physical nativePG18.6: verified backup376.025125ms, stopped-primary-to-ready
141.374167ms. Full per-test durations4.671s/1.540s are distinct from those defined
windows (setup, original-source close, final append and cleanup excluded).
Three reviews:1 logical gets new history/rebuild and resumes pair4 before5,
physical preserves history/metadata/pair then advances;2 actual both native cases
run in printed START/PASS order, --no-capture forces serial and reports that the
redundant --test-threads flag is ignored, not hidden overlap;18unselected get no
credit;3 exact9observation lines only, no old assertions/limits/selection changed.
Raw/tmp/mnesis-pg7-recovery-repeat-1.log. RPO0 only for quiesced2-event fixture;
no deployment SLO or large-data recovery claim.

Recovery measurement2:2PASS6.504s/18unselected, compile.29s excluded. Logical
backup2.092292ms/readiness45.876ms; physical verified backup403.584208ms/readiness
140.4845ms. Three reviews: same exact logical new-history/state4→5 and physical
preserved-metadata/checkpoint invariants; actual two serial native cases with
fresh physical cluster/new logical history and no guard/compile substitutions;
unchanged9observational lines, all previous assertions/selection/deadlines.
Raw/tmp/mnesis-pg7-recovery-repeat-2.log. Defined readiness windows are not total
outage or test elapsed; initial setup/source close/external routing remain outside.

Recovery measurement3:2PASS23.422s/18unselected, compile.30s excluded. Logical
backup2.133125ms/readiness41.92975ms; physical backup413.460084ms/readiness
140.917959ms. Three reviews: same exact data/history/metadata/pair/advance
invariants; actual two serial native cases and separate real timings, recording
the larger23.422s full invocation rather than calling it46ms or noise; unchanged
source/assertions/selection/deadlines and observation-only9lines. Defined
readiness windows omit other setup/source-close/final-cleanup work; these results
must not be presented as total downtime or deployment recovery targets. Raw
/tmp/mnesis-pg7-recovery-repeat-3.log. Three complete measured runs/6cases now
retained, each independently three-reviewed. Current full signed gate next.

## Workload normal signed gate

Signed d231769bf044730e4998a3fe1437b005690be0aa (G) completes the unchanged
pre-commit Nix gate, exit0. Actual workspace1594PASS476.417s/16slow/4compiler
exclusions across138binaries, compile7m47s excluded; coverage output retained.
Three reviews: full selected workspace contracts including original long Fjall
law;1594 actual completions with four compiler cases explicitly owned by CI and
no DB credit for guarded host tests; unchanged source/assertions/property inputs/
not-test(compile_fail) selector and strict lints/thresholds. Independent264real
Postgres cases and six operator cases above supply actual database evidence.

Three actual replay invocations pass CBOR2/header+chunk, value1 and wire1, each
1.00s. EACH reviewed three times: owning parser/value contract; actual completed
locked corpus/replay execution/counts with no empty harness or echoed credit;
unchanged source/inputs/assertions/selection. No long AFL/capacity/native-DB proof
from replay. Cross-target Wasm/no_std owners compile successfully, zero runtime
credit. Raw/tmp/mnesis-pg8-soak-{hook,gate-nextest,gate-replay}.log. Determinate
Nix reports76 evaluated flake checks; host build owners remain the existing gate.

Concurrent user release403 merged9e5ceeb3af789cff282136e635f6e295f3bf9e58:
versions/internal requirements/three locks/Postgres changelog only, no Rust
implementation changes. Release39342443941 explicitly publishes0.5.2; primary
registry confirms not yanked, checksum
39f9288298e4c54847505db231f581b75979cc4c21da1757ff70243aed38ab45.
Preserve this work. This workload tested the recorded0.5.1 graph, not an invented
0.5.2 rerun. Existing protected rules allow ordinary exact-head merge against
concurrent release; verify resulting standard merge tree. Current graph/final
registry consumer remain later requirements. Record-only signed amendment next;
credit no new runtime if cached. Open focused PR promptly after terminal.

The record-only amendment actually reran the required gate after the interrupted
session; it was NOT cached runtime. Actual workspace1594PASS531.789s/15slow/
4compiler exclusions, release compilation2m20s excluded. Three reviews: full
unchanged owning workspace contracts and original long property laws;1594 actual
new completions rather than echoed first-run476.417s, no guarded DB/runtime
credit; identical source/assertions/property cases/not-test(compile_fail)
selector/lints. Longer elapsed is recorded, not labeled a measured noise floor.
Replay again executes2/1/1 cases,1.00s each; EACH three-reviewed for exact parser/
value owner, actual completed invocation, unchanged inputs/assertions/selection.
No new capacity/native/AFL proof from those replays. Cross-target compile owners
pass. The same full normal hook exits0. Raw
/tmp/mnesis-pg8-soak-record-{hook,nextest,replay}.log. Final evidence-only amendment
immediately afterward must still use the normal signed gate; inspect actual
cache result before claiming zero additional runtime. Functional source stays.

## PG9 feature checker characterization

Supported cargo-hack0.6.45 help confirms each-feature includes no/default/all and
individual cases. Initial Nix shell contained the tool but no Cargo; no checks
ran. Pinned repository environment supplies Cargo1.99.0 correctly. First actual
isolated-library attempt fails before compilation: --no-dev-deps removes manifest
edges, so Cargo must prune Cargo.lock and correctly refuses --locked. Original
manifest and lock restored exactly by the failed tool invocation.

Three reviews: (1) actual intended dev-free library graph is necessary, ordinary
unit dev-unification is not proof; (2) compiler/dependency-setup red, zero library
or runtime passes; (3) no source/lock/assertion/lint changes hidden, original
locked command retained as rejected approach. Use supported cargo fetch --locked
first, then offline dev-free checks with an explicitly disposable lock copy and
restoration before original locked unit modes. Inspect resulting lock changes for
pruning/unchanged versions before retaining. This records the necessary graph
change rather than pretending --locked works. Workflow budget40added lines
(refined before edits), zero production/public API/tool-suite changes. An isolated
downstream graph remains required independently. Existing real-DB suite adds
individual adapter-feature archives through the same native hook and full tests.

Offline Postgres dev-free compilation passes7 actual selections:all,no-default,
export,import,projection,snapshot,tracing (10.67/1.11/1.73/1.69/1.29/1.26/1.12s;
read exact raw/tmp/mnesis-feature-postgres-offline-check.log for all times).
Cargo-hack itself restores BOTH manifests and Cargo.lock byte-exactly; the
proposed manual backup/restore wrapper is unnecessary and will not be added.
Review1: selected dev-free adapter graph compiles, but source audit finds existing
workspace-hack explicitly enables Tokio full, so these passes alone do NOT prove
published dependency isolation. Review2: seven compiler-only invocations, zero
runtime/native tests; package has no default feature so cargo-hack deduplicates
that equivalent selection, ordinary default unit mode remains explicit.
Review3: all original manifests/lock restored exactly; no weakened lint/selection
or production change. Refine using existing cargo hakari disable/generate owners
around the check and require git diff --exit-code afterward. This matches the
established publishing preparation; do not invent another dependency editor.
Nix archive factory mechanical reindent is36added/15removed vs65/10budget; retain
same commands/integration owner, revised removed-line budget20 before further
edits. New advertised-feature names come from adapter Cargo.toml, not a second
hand-maintained list. Real native archive executions still pending.

Correction to the preceding first offline pass timing: no-default was2.00s
(raw authoritative), not1.11s; all other six times match. No runtime credit.

Published-shape Postgres compile with Hakari disabled and dev dependencies
removed now passes7/7:35.42/1.13/1.07/1.06/1.03/1.03/.58s, all/no-default and
five individual features. Three reviews for EACH compiler invocation:
1 selected dependency/adapter-feature contract with no generated third-party
unification and no dev edges;2 actual compiler completions in raw
/tmp/mnesis-feature-postgres-unified-check.log, zero runtime/DB evidence;
3 unchanged source/lints/feature list, Hakari generation and cargo-hack restore
all manifests/Cargo.lock exactly (independent cmp passes). No manual lock editor.
The earlier seven checks with Hakari enabled remain limited evidence, not reused.

Cargo-metadata parser run through pinned Nix emits exact locked0.18.2; own metadata
format-version1 and jq require exactly one AFL package. Three reviews: engine
version owner remains fuzz-afl/Cargo.lock; actual Cargo/jq execution, not a Python
translation; all manifests/locks unchanged and pipeline failure preserved.
Native individual archives and downstream evidence are still pending.

Feature-falsifier setup preserved in detached/tmp/mnesis-feature-control at5b86:
private compile-only use of Tokio's multi-thread builder without declaring that
library feature. First intended original-control invocation ran in the ordinary
feature worktree instead (source path in log proves it);4.86s compiler pass gets
ZERO falsifier credit. Three reviews catch the mismatch: correct compiler contract
but wrong source; actual ordinary compile only; source/locks unchanged. Repeat
against the explicit control cwd before any control conclusion. Control never
merges; its source and logs remain retained.

Actual original-check control now runs in detached feature-control: Cargo check
all-features accepts the private unsupported Tokio multi-thread builder use.
Three reviews: owning declared library graph should not need test-only/full
Tokio; actual compiler pass in intended control cwd, not prior ordinary source;
only the five-line private compile probe differs, original flags/lock/Hakari
retained. This deliberately wrong source never merges. Correct dev-free/Hakari-
disabled command must now reject the same source for the missing constructor.

Workflow static check finds SC2016 on the single-quoted Bash-c body. Variables
were intentionally deferred to the child, but replace it with the ordinary
quoted here-document form rather than suppressing ShellCheck. Initial static
red retained/tmp/mnesis-feature-actionlint.log. Nix devShell and Linux integration
checkPhase evaluate; the initial devShell attribute omitted .default and was
corrected, zero build/runtime credit. Nix checkPhase contains all seven complete
native feature archives plus unchanged mandatory all-feature smokes, pipefail,
DATABASE_URL and live server guard. Actual archive execution still pending.

The Tokio compile probe SURVIVES all7 dev-free selections (2.93/1.17/1.52/1.24/
1.23/1.19/.88s), wrapper exits1 because expected compiler101 was not observed.
Three reviews: actual root workspace explicitly declares Tokio full, so the probe
is LEGITIMATE published-shape code; it is NOT evidence of checker failure or an
unreported dependency defect. Real compiler passes cannot be relabeled catches.
Owning source invalidates the proposed falsifier; preserve rejected source diff
/tmp/mnesis-feature-tokio-probe-rejected.diff and logs. Assertions/flags unchanged,
no runtime/native credit. Select the actual mnesis-store JsonCodec seam instead:
Postgres enables json/cbor only in dev dependencies, its declared production store
edge enables subscription. Verify that source fact before the next control.

JsonCodec control also rejects the proposed original-false-green claim: ordinary
Cargo --lib --locked --all-features already fails E0433 at the undeclared json
codec. Three reviews: declared production graph lacks json (exact intended
boundary); actual original compiler rejects it, so ZERO new-checker catch credit;
only private control differs and assertions/flags unchanged. The old Python
wrapper's --lib check already excludes this dev-only seam. Do not manufacture
another probe to obtain a claimed defect. Earlier general feature-leak concern
is NOT a reproduced current Cargo1.99 bug. Retain replacement on its demonstrated
maintenance value (delete custom Python, supported cargo-hack feature enumeration)
and published-shape graph checks; describe coverage limits precisely, not as an
existing compiler defect repair. Individual real-DB archives and final isolated
registry consumer supply separate required evidence. Both rejected controls stay.

Both JsonCodec checks reject the same intended compiler E0433. The cargo-hack
wrapper returns1 although its child Cargo returns101; the ad-hoc expectation of
wrapper101 was wrong and its outer command exits1. Three reviews distinguish
actual compiler rejection from wrapper status, preserve exact source/flags and
award no new-checker superiority or runtime/native credit. No rerun is needed to
manufacture a green control summary; raw logs/source stay. Real production-shape
7/7 valid-source compiles above remain separate positive evidence.

Feature archives now require one exact owned native law before the full suite,
with --no-tests=fail so a missing feature's test cannot silently pass. Default
uses real append/read conformance; other selectors use existing atomic/listing/
projection/snapshot/diagnostics cases. Reuse those selector facts in the existing
all-feature smoke commands. Current48added/19removed Nixlines remains within
revised65/20budget. Live database preflight and pipefail unchanged. No runtime
coverage inferred from evaluated shell text.

## PG9 AFL shell characterization and first gate

Existing Bash-e pipeline actually returns0 with a failing producer; no explicit
pipefail in the AFL step. Existing workflow uploads saved crashes but has no
verdict, so successful bounded exit cannot establish a clean target. Add only
explicit pipefail and regular crash-file rejection (README.txt excluded),12lines;
keep original target/duration/CMPLOG/corpus/upload owners unchanged.

Pinned Nix-shell direct extracted-workflow checks: missing and README-only pass,
default saved input rejects; original false|tee exits0/fixed exits1. Raw
/tmp/mnesis-afl-verdict-cases.log. Review1: exact file/pipeline policy, no engine
or product correctness claim from constructed artifacts. Review2: inspection
found the worker-directory case still had the prior default crash, so it does
NOT independently prove worker-directory rejection; reject that coverage claim
and rerun isolated directories. Review3: source diff confirms only verdict and
pipefail, unchanged failing conditions/target set/uploads always()/CMPLOG.
These are shell checks, not native fuzz campaigns or PG8 work. Original red and
fixture limitation retained. Cargo metadata independently resolves locked afl
0.18.2 with no Python parser; future tool-removal slice remains separate.

Corrected isolated gate run: missing/README-only exit0; default crash with spaces
and independent worker/sample crash each exit1. Raw
/tmp/mnesis-afl-verdict-isolated-cases.log. Review1: exact supported artifact
policy covers any regular saved input without treating informational README as
failure. Review2: each case has its own fresh directory; worker failure now
names out/worker/crashes/sample, no prior default file can mask coverage. These
remain constructed artifacts, not real-engine crash evidence. Review3: extracted
script equals workflow body,12added lines only, original CMPLOG/always-upload/
targets/durations preserved; original failing producer proof retained. Full Nix
signed gate must wait until native soak terminal, not run concurrently.

Locked actionlint1.7.12 completes exit0, zero diagnostics for fuzz.yml. Static
workflow analysis only; no test/fuzz execution credited. All artifact cases and
producer pipeline checks above remain the separate shell-runtime evidence.

Combine the two PG9 CI changes in one focused verification slice: feature archive/
checker ownership and AFL verdict, no product implementation. Original isolated
AFL worktree remains preserved. Actual combined Nix delta53added/19removed, not
the earlier estimated48; within recorded65/20budget. Nix shell evaluation contains
all-feature and six default/individual archives with exact mandatory selectors.
Heredoc workflow passes pinned actionlint with no diagnostics. All still need
normal signed full Nix gate and actual Linux CI; no native/compiler result is
inferred from static evaluation. Goal remains active and native soak sole DB run.

Owned devShell builds through the repository flake with cargo-hack0.6.45,
cargo-hakari0.9.39 and jq1.8.2. Other eight library checks complete40 actual
compiler invocations: mnesis6, macros1, store17, store-testing4, wake1,
wake-nostd1, inmemory4, Fjall6. Together with Postgres7,47 complete successfully.
For EACH invocation three distinct reviews: declared package/feature graph with
Hakari disabled and no dev edges; actual matching running/completed child logs
(no runtime/native credit); unchanged lints/source/selection and independent
byte-exact lock/manifest restoration after supported generation. Exact individual
feature flags/durations remain /tmp/mnesis-feature-<package>-check.log; whole
serial driver/tmp/mnesis-feature-workspace-check.log. Zero library unit tests or
native database cases are inferred from these compiler checks. Cross-target
no_std checks remain owned by the normal full Nix gate.

Combined actionlint passes with zero diagnostics. Nixfmt check requests ordinary
formatting; apply the supported formatter rather than suppressing its verdict.
No product assertions, test filters or budgets change. Current public README
corrections list Postgres and existing verified-connection example; obsolete0.1
quickstart becomes independently published0.5. Local missing-DATABASE_URL tests
are explicitly not database evidence.

After supported formatting, nixfmt and combined actionlint complete with zero
diagnostics; static analysis only. Final measured Nix delta58added/19removed,
within65/20. Existing native all-feature laws remain selected with shared
selectors; six individual/default complete suites have their own required law.
No new native workload has run while the frozen1800s soak is active. Latest
observed1393.575682917s/504484rounds/2270210events/24faults/0knownrejections/
45connections. This is progress, not completed30-minute acceptance.

## Independent published dependency graph

Temporary standalone/tmp/mnesis-pg9-registry-compile-052 has only the exact
published mnesis-postgres0.5.2 dependency and five forwarded feature names. No
workspace/path/patch/development dependency. Supported cargo-hack --locked
compiles all/no-default/export/import/projection/snapshot/tracing7/7:
45.74/1.94/2.01/1.86/1.79/1.85/1.74s. Cargo metadata independently confirms all
four Mnesis crates are registry0.5.2, not local paths. Each compiler invocation
three-reviewed: owning isolated published graph/API export; actual completed
compiler execution (zero native/runtime credit); unchanged exact version/lock/
feature flags and no unifying workspace/development graph. Cargo deduplicates
equivalent default/no-default because adapter has no declared default features.
No full registry consumer claim yet: separate ordinary test is prepared at
/tmp/mnesis-pg9-registry-native-052, requires an actual private DATABASE_URL and
will fail if absent. It checks binary-key appends, saved checkpoint reopen,
stream reads and exclusive global resume. No runtime before active fullgate ends.
Raw /tmp/mnesis-pg9-registry-compile-052{.log,-metadata.json}; both consumer
projects remain outside the repository, no new project tool/framework/runtime.

Standalone native consumer compiles against published0.5.2 in36.12s with
cargo test --locked --no-run; Cargo metadata confirms every Mnesis package is
registry0.5.2 (four packages), no path/workspace graph. Three reviews: actual
public append/checkpoint/reopen/exclusive-resume API composition; completed
compiler only, ZERO executed tests; exact pinned versions, no DATABASE_URL guard
that can pass, no automatic retry or local overrides. Native runtime still
required after active local gate. Raw/tmp/mnesis-pg9-registry-native-052-build.log
and matching metadata.json. This is outside-repository consumer evidence, not
another project runner or test framework.

PR405 current177 head push CI feature logs obtained directly from completed jobs
because gh run --log waits for the entire run. Push native unit modes(default/
no-default/all): kernel15/15/15, macros1/1/1, store217/217/217, Fjall131/131/131,
inmemory25/25/30, wake17/17/17, wake-nostd0/0/0, store-testing0/0/0. Each of these
24actual invocations three-reviewed: selected owning library/mode contract;
actual completed commands/counts with all zero-case binaries receiving ZERO
runtime credit; unchanged source/assertions/feature selections/lints. Compiler
checks in those jobs are separate, no Postgres-isolation proof from this old
matrix (PG9 replacement still required). Raw direct-job logs under
/tmp/mnesis-pg8-soak-direct-job-<id>.log, ids118699976514/563/621/783,
118699977075/153/178/813. Mutants39443682812/118699853743 has NO mutants to filter
and uploads no artifact; ZERO new runtime catch/baseline credit. This is expected
for a test/benchmark/doc-only production diff; existing real wrong-rescan and
wrong-ordinal controls remain the workload oracle evidence. Scheduled sweep skip
is not a failed required check. Linux/native/PR-feature/fuzz logs still pending.

Independent registry consumer .5.2 executes against a fresh privatePG18.6
on/on/on:1PASS.057s/0skip, nextest1f608d24-caed-44db-a074-86521365fed0. Three
reviews: (1) real binary-key append, checkpoint commit, all handles/pool close,
fresh runtime open preserving exact revision/position/state/history, second
append, inclusive stream reads and exclusive global resume; (2) actual one
public-consumer test with no environment success guard, migrations from the
registry archive and no workspace/path/patch graph, waited native server
shutdown succeeds; (3) exact published versions/locks/unchanged assertions and
5sclose deadlines, unknown writes fail rather than retry. This verifies released
0.5.2, not every still-unmerged test/doc/CI change or a future version. Real native
consumer source/lock/metadata and database directory are retained outsideproject;
raw/tmp/mnesis-pg9-registry-native-052-run.log. Do not call whole goal complete.

PR405 current177 fuzz logs: libFuzzer wire13266301/value5692739/header2407556/
chunk2358227 actual runs,61s each. AFLwire/header/chunk/value60s each, CMPLOG
confirmed,0saved crashes/0timeouts; new corpus items0/2/15/0. EACH of eight
campaigns three-reviewed: selected parser/value invariant; actual engine output,
counts/duration and no saved failures rather than job-green inference; unchanged
source/targets/durations/assertions/CMPLOG/corpus handling. Clean campaigns supply
ZERO saved-crash/zero-exit gate proof; required PG9 negative control remains.
Raw direct-job logs ids118699853619,118699854124/183/252/295/348/371/425.

PR405 exact177 standalone Linux integration actual264PASS66.868s(push) and
264PASS67.369s(PR),0skips/23binaries each; second printed run.log is an ECHO,
zero additional executions. Each preceded seven actual one-case feature smokes
with explicit remaining cases unselected (14/15/2/7/10/4/9). EACH of these16
invocations three-reviewed: exact native owner/order/state/commit/metadata/lifecycle
law and complete-suite selection; actual process/count/duration/live database,
not echoed or guarded substitute; unchanged original source/assertions/filter/
deadlines, new55P03 phase preserved. Compile1m04s/1m02s excluded. Other default/
individual advertised adapter features still require PG9 expansion, not these
all-feature smokes. Raw direct-job118699976624/118700026089 logs retained.

Compile-fail cases also actually run: push kernel2PASS122.70s/macros1PASS15.51s/
store1PASS18.57s; PR2PASS81.54s/1PASS10.78s/1PASS13.95s. EACH of six invocations
three-reviewed: exact owning compile-time misuse contract; actual fixture/pass
counts and compiler subprocess success, not ordinary DB/runtime proof; unchanged
source/expected stderr/toolchain/flags and four normal nextest exclusions still
owned here. Raw direct-job118699976677/118700026522. Final Nix-host native and
workspace logs plus CodSpeed still need independent review before protected merge.

2026-10-20 PR405 final177 PR-event feature jobs reviewed individually: wake
118700026341=17/17/17; Fjall118700026367=131/131/131; inmemory118700026529=25/25/30;
kernel118700026618=15/15/15; store-testing118700026620=0/0/0;
macros118700026789=1/1/1; store118700026999=217/217/217;
wake-nostd118700027020=0/0/0. Each triplet is default/no-default/all-feature
unit execution. EACH of the24 invocations received three reviews: (1) owning
crate's exact unit laws and advertised selection; (2) actual log test counts,
failures/ignored/filtered all0, with empty binaries receiving ZERO runtime
coverage; (3) unchanged source/assertions/feature flags/deadlines at177 and no
Postgres native isolation credit. All24 actual invocations pass. Raw direct-job
logs retained, not a second credit for summaries. This finishes the PR-event
counterpart of the previously reviewed24 push-event invocations.

Push-event final177 Nix job118699976130:1594PASS135.887s/4slow/4compile-fixture
exclusions; actual native264PASS66.991s/0skip plus seven separate one-law smokes
(14/15/2/7/10/4/9unselected); replay2/1/1PASS each1.00s. EACH of these12 actual
invocations three-reviewed: (1) workspace or selected native/parser owner
contract, precise DB selection and held-watermark/outcome semantics; (2) real
process/count/duration and live native database, not echoed run.log or compilation;
(3) unchanged assertions/filters/statement deadlines/feature flags, with four
compile fixtures separately exercised in owned jobs. Build times excluded. All
other completed host checks are static/compile evidence, not new runtime tests.

CodSpeed final177 job118700056095 actually completes four unchanged suites
(kernel_bench, store_bench, fjall_benchmarks, all_index_layout), instrumented
Simulation mode and uploaded data; analysis succeeds. Three reviews: exact
existing owner/workload selection; actual measured entries and four-suite finish
rather than green/cache inference; unchanged benchmark source/thresholds/tool
configuration. Cache restore/save warnings are retained. No native Postgres
capacity or deployment latency claim follows this instrumented non-Postgres run.

PR405 final177 PR-event Nix118700026661 completes:1594PASS137.186s/4slow/
4compile exclusions,264nativePASS66.414s/0skip,seven one-law smokes with
14/15/2/7/10/4/9unselected, corpus replay2/1/1each1.00s. EACH of12actual
invocations independently three-reviewed against source/owner contract, actual
native process/count/falsifier and unchanged selection/assertions/deadlines. No
compilation/echo/guard credit. Both push and PR Nix jobs therefore actually ran
the complete current native suite. Exact177 all40SUCCESS/1scheduledSKIP and
0review threads; expected ordinary merge treebd9d9ce3152aa4a6ccd12045961fe0dd31856ac8
preserves concurrent0.5.2 release. Protected exact-head merge follows.

Protected workload/recovery PR405 merged at8b0ee9778e13d87dc3d45ff23f26c60d285fe6cc,
2026-10-20T15:19:38Z. Actual merge tree equals expected
bd9d9ce3152aa4a6ccd12045961fe0dd31856ac8, including .5.2 release. Current PG9
source integrated byte-identically onto this parent; four append-only records
preserve HEAD prefix plus saved candidate suffix. All stashes retained. Public
README fixes are already in protected parent and disappear from this diff.
Final minimization:0productionRust/API/migrations/dependencylock changes,
44linePython wrapper removed, no replacement project runner. Nix feature names
come from manifest, selectors reused for existing mandatory smokes, native full
suites require nonempty execution, all pipelines preserve exit status. Fuzz
version is selected once from owning lock via supported Cargo metadata/jq;
saved-crash negative verdict supplements clean campaigns. Real engine control
and actual Linux selected suites remain required. Freeze source for normal
signed commit/full gate; no threshold, lint or hook bypass.
PG9 normal signed gate88620 actual corpus replay2/1/1PASS, each1.00s,0failed/
ignored/filtered. Each of three invocations reviewed separately: (1) existing
stable parser/CBOR fuzz owner and checked-in corpus; (2) actual test process and
named tests in replay log, not compilation/cache output; (3) unchanged fuzz source,
corpus, durations, assertions and selection relative protected8b0e. This proves
stable replay only, not real AFL saved-crash verdict or Postgres native execution.
Workspace nextest actual1594PASS557.312s/14slow/4compile-fixture exclusions,
138binaries; compilation2m30s excluded. Three reviews: (1) current .5.2 kernel,
store, wake and adapter contracts, including original property/ordering/batch
and checkpoint law owners; (2) actual complete execution/pass counts and slow
process completion, all unexecuted database-success guards provide ZERO native
credit on this Darwin gate and compiler fixtures remain owned by CI; (3) source,
assertions, property workloads, filters, limits and deadlines unchanged, only
ordinary all-target build/gate selection. No reduction for long property tests.
Native default/each/all and real AFL saved-crash control remain separately due.

Normal signed commit1013b6a3 passes all12host Nix checks. Final intended staging
command initially named the already-staged deleted Python file and returned a
pathspec error before staging its other arguments; the shell then entered the
normal unchanged hook with the implementation already staged. No source or gate
was altered. Retain that first signed commit, stage remaining owned records/raw
logs and amend through the same normal signed hook. Product/configuration diff
is unchanged; raw/document records cannot supply runtime credit. The final signed
head will be verified before push. No project-owned Python remains in workflows
or the removed scripts directory; Cargo metadata owns AFL version selection.

PR406 real AFL control FIRST RED retained: dispatch39450659063,
job118724284510 based signeda30c4b34, actual10s/CMPLOG,3savedcrashes/0timeouts,
engine stepSUCCESS(0), verdictFAILURE(1). Upload then FAILS because
/id:000000,sig:06,... contains colon rejected by actions/upload-artifact. No
artifact proof claimed. THREEreviews: owning saved-crash/retained-evidence
contract; actual initial A calibration, live engine10s/3files/zeroexit followed
by exact verdict1 and failed artifact; unchanged normal verdict/timebox/CMPLOG/
source outside isolated intentional injection. This catches a real evidence
pipeline defect, not an adapter security issue. Standard archive packaging is
the smallest supported repair; preregistered budget/falsifier inHYP.

PR406 PR-event wake118722980838 compiler1/unit17/17/17 and macros118722981185
compiler1/unit1/1/1: EACH compiler/unit invocation three-reviewed against
owning feature or unit contract, actual counts/process, unchanged assertion/
selection with supported Hakari/dev-free offline graph and exact restoration.
PR compile-fail118722980848 actuallykernel2PASS81.44s/macros1PASS11.91s/
store1PASS14.51s; EACH three-reviewed against exact misuse fixture, actual counts
and diagnostics, unchanged source/flags/toolchain. No native runtime credit.

Checked-in first real-crash job log removes trailing spaces only for repository
whitespace checks; the exact original /tmp/mnesis-pg9-afl-real-crash-job.log is
preserved. Initial whitespace check rejected raw runner blank-space fields;
no test, workload, original raw file or gate was changed to hide that failure.
## Capacity preparation compiler baseline

First strict all/all check exits101: actual E0308 BatchSize expectsusize, and
SQLx0.9 requires its supported dynamic-query builder instead of an ordinary
String argument. No native benchmark ran. Three reviews: owning selected API/
target contracts; actual compiler-only failures with zero runtime evidence;
unchanged lints/selection/assertions. Rustfmt643lines remains within700budget.
Original shell creation command initially lacked the benches directory and wrote
no file; directory creation corrected before source addition, zero test credit.
Defer native capacity until the failed soak's finite-visibility assumption is
resolved and the full soak succeeds. Original reds remain raw, not suppressed.

Second capacity strict check reaches linting and fails writes96/80lines and one
missing statement semicolon. Three reviews: selected strict shape/ownership;
actual compiler-only red/zero benchmark runtime; no lint/assertion/selection
relaxation. Extract the real concurrent writer cohort for reuse in warmup and
measurements, retaining unknown-error rejection and exact winner counts. The
soak's visibility finding also requires seed quiescence before finite-history
measurements: wait for the native cutoff to exceed the newest seeded transaction,
record the wait separately, never assume acknowledgement implies eligibility.
This is setup only, not a stronger production visibility or latency guarantee.

Capacity third strict all-feature/all-target checkPASS2.88s,693/700Rustlines.
Three reviews: exact advertised compilation/target contract; actual compiler
execution only, zero native capacity runtime; unchanged strict lint levels and
all assertions/inputs. Shared writer cohort now validates exact winners and
rejects every non-conflict error, and is reused for untimed warmup. Seed setup
waits for native eligibility, separately from measurements. Still prepare docs,
source integration, current native smoke/falsifiers/full matrix and delivery.

Capacity WIP preserved in retained stash58c04f27bc49e8ba7872534c9c816ed9a2f41f31
before integration onto protected5b868028 (.5.1). Only append/append research
conflict: retain complete protected prefix byte-identically, followed by all
capacity compiler evidence; no production source changes. Current bench685/700
lines after sharing nearest-rank percentile authority, reporting operations/sec
and actual native row growth per append window. README16/18lines documents
matrix and measurement limits. No new capacity test/benchmark run yet; full
native soak remains the sole active local database workload. Stash retained.

Current protected capacity strict check fails only missing_const_for_fn on the
new shared percentile helper. Three reviews: selected strict target/API contract;
actual compiler-only red with zero native measurement; original lints and all
workloads/assertions preserved. Mark that pure helper const as requested by
pinned Clippy; no suppression or threshold change. Raw
/tmp/mnesis-pg8-capacity-protected-check.log. No capacity benchmark has run yet.

Current protected capacity compilation nowPASS13.42s with shared const percentile
helper. Three reviews: actual selected .5.1 all-feature/all-target contract;
completed compiler execution only, zero native measurement/capacity credit;
unchanged strict lints, matrix inputs/sample thresholds/model/memory bounds and
no production/API/dependency change.685/700source, README17/18lines, existing
allocator4churnlines. Native10k smoke then full matrix must follow sole active
soak. Rejected compiler red remains retained. Raw
/tmp/mnesis-pg8-capacity-const-check.log.

Capacity optimized benchmark compiles11.48s with --no-run on protected.5.1,
no benchmark execution. Three reviews: actual default adapter benchmark target
and selected APIs; real optimized compilation/artifact only (zero capacity or
runtime credit); source6ea3e0da5bc6c90b82f40c9bca0866dcfcb79d744ddbdc46df7d3ba823815a55,
unchanged assertions/limits/profile. Artifact reported by Cargo JSON:
/Users/joel/orca/workspaces/nexus/main/target/release/deps/capacity-4117bc1a0d839cb6.
Compile output/tmp/mnesis-pg8-capacity-{artifacts.jsonl,release-build.log}.
Read actual page/store SQL before measurement: initial stream text matches
observer's WHERE stream_id prefix; native global-resume tuple matches observer;
continuation-only stream shape is explicitly excluded. Plans still require all
three real prepared shapes at runtime. RSS snapshots/model overhead labeled;
external OS time scope is whole-command resources, not isolated client peak.

Capacity preserved in retained stash before moving from5b86 to concurrent user
release4039e5ceeb3af789cff282136e635f6e295f3bf9e58 (.5.2). Rebase/stash apply
succeed without conflicts. Existing benchmark SHA2566ea3e0da5bc6c90b82f40c9bca0866dcfcb79d744ddbdc46df7d3ba823815a55
still compares exactly; production Rust matches protected main. New version/lock/
changelog work preserved. Old .5.1 optimized artifact is NOT current .5.2 build
or runtime evidence; compile current graph after workload full gate terminal,
then native smoke/full matrix. Still ZERO native capacity measurements.

Current .5.2 capacity source passes fmt/strict all-feature/all-targets20.19s and
optimized --no-run5.71s. Three reviews for EACH compiler command: current selected
version/target graph; actual compiler completion only (zero benchmark/native
credit); unchanged source6ea3..., lints/inputs/assertions/limits and preserved
.5.2 release metadata. Cargo artifact reports capacity-d4d8b8efa57b1ad6. Begin
MNESIS_CAPACITY_EVENTS=10000 native smoke after local gate terminal; all other
axes/windows/sample counts stay. This is not the full10k/100k/1m matrix.

## First native capacity smoke completes

Current protected.5.2 process31754 TERMINAL0. Four10k-event fixtures cover100/
10000-event streams and32/4096-byte noncompressible templates, PG18.6 on/on/on,
fixed128rows/1MiB/24pool.144read windows/144append windows/16actual EXPLAIN
outputs, all3repeated windows with>=100samples. Four full-global drains return
exactly10000events:27.160833/25.121708/73.800458/73.272291ms. Each close reaches
pool0; retained allocator bytes107022/103822/105166/103822. Native append before/
after counts include growth; no assertion/error/unknown retry observed.

Three reviews after actual terminal:
1 Contract: native seed eligibility precedes finite scans; every read validates
   full payload/version/key/position; concurrent writers validate exact winners
   and independent committed-row deltas; client peak/retained bounds and pool
   close are reached. All three actual cached query shapes pass diagnostic replay.
2 Execution: four real private clusters,144+144completed windows and16plans,
   not compiler/guard/empty binary. Only10k history is exercised, so100k/1m and
   full capacity acceptance remain unproven. RSS snapshots and allocation stats
   retain their defined scopes; no deployment SLO from this smoke.
3 Source/assertions/profile: frozen6ea3e0da5bc6c90b82f40c9bca0866dcfcb79d744ddbdc46df7d3ba823815a55
   still matches; rows/bytes/pool/deadlines/input axes/sample counts/model and
   unknown-error rejection unchanged. No optimization/noise threshold introduced.
Raw /tmp/mnesis-pg8-capacity-052-smoke.log and matchingstart/checksum. Begin full
unchanged12-fixture matrix next; no other local native suite/fullgate during
measurement. OS time wraps the compiled benchmark, not Cargo or Nix startup;
its whole-command resource scope can include native child tools, not client-only
peak RSS. Current program remains685/700lines, no production/API change.

## Full native capacity matrix completes

Session45723 TERMINAL0, unchanged .5.2 source6ea3...verified. All12fixtures
(10k/100k/1m ×100/history-length streams ×32/4096B),432read windows,432append
windows and48real native EXPLAIN outputs complete;12pool closes succeed. Every
window preserves3repetitions and>=100response/success denominators. Full command
343.78s real/153.81user/115.91sys; OS whole-command maximumRSS56885248bytes,
peak memory footprint32702896bytes,0swaps (scope includes launch/child tools;
not client-only peak). No error, invariant failure or unknown-write retry.

Three reviews after terminal:
1 Contract: exact seed/native count and eligibility precede scans; full drains
   validate every payload/key/position, and page probes also validate versions;
   all page/read/writer cohorts
   retain real overlap, exact winners and independent persisted deltas. Fixed
   rows/bytes/pools/deadlines, bounded peak/retained allocations and12close
   assertions are reached. Every actual prepared shape is observed and replayed.
2 Execution: actual12native fixtures/432+432windows/48plans, including both
   million-event4KiB histories; no smoke/compile substitution. Million-event
  4KiB full drains8.987108541s(short)/8.326797875s(long) with client allocation
   peaks840990/810003bytes (baseline223421/196037), showing the tested page-bound
   retention; allocation TRAFFIC remains~9.11GB per drain, not retained memory.
3 Source/selection: pre-run SHA still matches exactly; all input axes, rows128/
   bytes1MiB/pool24,30sstatement/acquisition and5slock deadlines, model assertions
   and sample/window counts unchanged. No algorithm/dependency/API/production
   change. Append fixtures report native row growth rather than pretend a fixed
   writer history; timed reads include validation overhead. No deployment SLO or
   small-difference optimization accepted from these measurements. Analyze the
   repeated-window spread, query plans and saturation before retaining delivery.
Raw/tmp/mnesis-pg8-capacity-052-full.log/start/checksum. No localfullgate/native
suite overlapped. Existing OS-time scope is recorded, not compiler/Nix RSS.

Plan observer refinement: actual timed million-row initial reads remain roughly
0.4–0.7ms at one client, while forced-generic diagnostics include~80ms scans.
That is a mode mismatch, not evidence that auto-mode timings regress by80ms.
Record native auto plans alongside generic stress plans and selected session's
prior generic/custom counts; preserve original baseline output and do not tune
library SQL from the forced-mode number. Current source690/700lines, existing
plan function remains within strict80line limit; no new helper/API/production
code. Rustfmt/strictall/all PASS6.42s. Three reviews: observed supported native
plan/counter APIs; actual compiler-only pass/zero new runtime; unchanged workload,
model, sample/windows, quotas and all deadline/resource/error assertions. Full
unchanged matrix rerun is justified by the observer-mode correction, not by a
weakened passing condition. New output must distinguish auto/generic labels.

Corrected observer optimized compilePASS3.56s (--no-run), no new runtime credit.
Three reviews: current .5.2 benchmark/auto+generic observer APIs; actual optimized
compiler completion only; unchanged measurements/model/bounds/inputs and
690/700source with no production/API changes. Run the full same12fixture matrix,
now expecting96distinct mode-tagged EXPLAIN outputs. Original343.78s baseline
remains retained; this rerun corrects observer evidence, not a performance claim.

2026-10-20 corrected observer full capacity matrix76942 TERMINAL0. Native PG18.6
on/on/on, published0.5.2 code; capacity.rs source SHA256
582662e92362290754a5cf2f1a604a25a9b56f7cdb35fb4876bbaf05b90ed54a exact before/after.
Three independent reviews: (1) source/model validates full drain count, key,
payload ordinal/tail and increasing global positions; timed page probes also
validate each version, and native count deltas validate all measured appends;
(2) all12fixtures,432read/432append windows,96 actual auto/generic query plans and
12pool-zero closes must be counted from retained raw log below before merge;
terminal0 reaches final fixture's quiescent cleanup, all128-client shared cohorts
have exactly one winner, no swallowed non-conflict/unknown errors; (3) original
matrix/sample counts,128row/1MiB bounds,30s/5s deadlines, allocator assertions and
seed/measurement separation unchanged by observer refinement. No source tuning.
Actual compiled-binary command time361.55real/159.03user/117.29sys seconds,
maximumRSS57737216B/peakfootprint35291544B/0swaps. This command-level OS scope is
not an isolated allocator peak or production target. Source log and measurement
analysis follow; prior343.78s generic-only full baseline remains, not overwritten.

Full corrected matrix actual counts independently parsed:12profiles/432reads/
432appends/96plans/12closes,12full drains. Source remains582662e9... after
protected405 integration; production .5.2 unchanged. capacity.md records exact
scope, short-window dispersion (median7.35%,p95 66.21%,max624.20%), all retained
outliers and same-product baseline, not a manufactured tiny improvement. Native
plan review distinguishes one-row keys scans from event scans: ordinary auto
1m initial global .170–.238ms/0prior generic, versus forced68.997–78.303ms with
outer body scan1m despite indexed candidates. This selected profile never forces
generic; no unsupported auto regression or performance guarantee. One-off local
Ruby2.6 analysis initially called unavailable filter_map; corrected to supported map/grouping,
no native rerun/catch credit and no project tool added. Table entries derive from
actual three-window rows; raw copies retained for every other matrix cell.

Capacity source minimized after protected405 union:690/700benchmark lines,
manifest4/6added,README19/20added, existing allocator3added/1removed. No new
public adapter API, production Rust, migration or dependency. Borrowed prepared
statement inspection reuses owning SQL rather than duplicating query policy;
fixed enums and one shared percentile function report the existing workload.
Full original/source-corrected matrices already passed; final normal gate needs
compilation/original allocator-owner regression only, without repeating the
native matrix absent a material source change. Selected full source SHA remains
582662e9. Large log artifacts are raw evidence, not a custom validation tool.

PG9 final candidate f3633983953a53488ae323f91498d0af911ab5e5 signedG, PR406
open. Normal final record-only amend passes all12previously-built checks, ZERO
new runtime (1594actual cases remain first gate evidence, not recounted). Proof
branch a30c4b341810bcd8302f11b6139971cf0e18dee1 signedG has normal unchanged
cached12check hook, ZEROnewruntime, only temporaryworkflow/doc diff. Candidate
assertions/verdict/source remain unchanged; actual Linux proof dispatch follows.
Targeted locked actionlint for changed checks/fuzz passes; broader legacy action
input findings in unchanged dependency-update workflows are retained in proof
record, no whole-repository actionlint success claimed. Not a native test.
