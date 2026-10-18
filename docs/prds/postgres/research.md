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
