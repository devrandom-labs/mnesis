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
