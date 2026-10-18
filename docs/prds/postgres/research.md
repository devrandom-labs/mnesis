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
