# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Mnesis is an event-sourcing and DDD kernel for Rust. It provides composable traits, derive macros, and infrastructure adapters (starting with fjall). The project is **experimental** with an unstable API. The 1.0 promise lives in [STABILITY.md](STABILITY.md) (#282): 1.0 tier = `mnesis`/`mnesis-macros`/`mnesis-store`/`mnesis-wake`/`mnesis-wake-nostd`, adapters + `mnesis-store-testing` stay 0.x; documented trait semantics are semver surface; on-disk = major-bounded write format with a one-major read overlap; MSRV bump = minor (pin-is-MSRV, no trailing floor); removals only at a major after ≥1 deprecated minor. Every freeze-affecting change must conform to it or amend it.

## Build & Development Commands

**Prerequisites:** Nix with flakes enabled. Use `nix develop` (or `direnv allow`) to enter the dev shell.

```bash
# Run ALL checks (clippy, fmt, tests, taplo, audit, deny, hakari)
nix flake check

# Run a single test by name
cargo test -p mnesis-store -- envelope_tests

# Apply formatting
cargo fmt --all
```

## Architecture

### Crate Dependency Graph

```
mnesis-store         --> mnesis (kernel)
mnesis-store-testing --> mnesis-store             (executable store-contract spec; every adapter's dev-dep; #281)
mnesis-wake          --> mnesis-store             (in-process wake registry; owns tokio/foldhash/parking_lot)
mnesis-wake-nostd    --> mnesis-store             (no_std global-eventcount wake; owns event-listener; #302)
mnesis-inmemory      --> mnesis-store, mnesis-wake (in-memory adapter; mnesis-store's test fixture via dev-dep cycle)
mnesis-fjall         --> mnesis-store, mnesis-wake
mnesis-postgres      --> mnesis-store, mnesis-wake
mnesis-macros       <-- mnesis (kernel, optional via "derive" feature)
```

**Workspace layout (#309)** — directory names drop the `mnesis-` prefix; **published package names keep it** (paths and package names are independent in cargo). Two folders carry the architecture's load-bearing boundary:

```
crates/            # core: everything an adapter builds on
  mnesis/           → mnesis            macros/          → mnesis-macros
  store/           → mnesis-store      store-testing/   → mnesis-store-testing
  wake/            → mnesis-wake       wake-nostd/      → mnesis-wake-nostd
  nostd-smoketest/ → mnesis-nostd-smoketest              workspace-hack/
adapters/          # infrastructure impls of the store seams
  fjall/           → mnesis-fjall      postgres/        → mnesis-postgres
  inmemory/        → mnesis-inmemory
```

The wake crates live in `crates/` (core), not `adapters/`, **by dependency direction**: `mnesis-wake` is a lib dependency of all three store adapters (their `WakeSource` impls delegate to `StreamNotifiers`), so it is something adapters build on — and `mnesis-wake-nostd` stays beside its sibling. Decided explicitly per #309's classification call.

`mnesis-store` itself has **no** tokio/foldhash/parking_lot dependency (#300): the
subscription loop is generic over the `WakeSource` trait, and the tokio-backed
in-process impl (`StreamNotifiers`) lives in `mnesis-wake`. `mnesis-store`'s own
tests reach `InMemoryStore` through a **path-only dev-dependency on
`mnesis-inmemory`** (a legal dev-dep cycle) — which unifies types **only for
integration tests in `tests/`**; the lib-test target recompiles the crate under
`cfg(test)` as a distinct crate, so inline white-box test mods that need a store
(`catchup.rs`, `subscription_cursor.rs`) use the in-crate `test_support.rs`
double instead.

**`mnesis-store-testing` — the executable store-contract spec (#281).** The
adapter seam (`RawEventStore` + `WakeSource`, optionally `AtomicAppend` /
`SnapshotStore`) has a contract far beyond "the trait compiles" — inclusive vs.
exclusive read bounds, conflict rejection with nothing landing, catch-up→live
ordering, lost-wakeup defense — and this crate pins it as **runnable checks**:
the four rule-7 category modules (`sequence`, `boundary`, `linearizability`,
`lifecycle`) plus two opt-in capability modules (`atomic` behind
`atomic-append`, `snapshot` behind `snapshot`). Four macros invoke it —
`conformance!` (the core matrix), `conformance_atomic_append!`,
`conformance_snapshot!` (takes `positions`/`extremes` sample pairs),
`conformance_lifecycle!` (`open`/`reopen` closures, persistent adapters only) —
each generating one named `#[tokio::test]` per check over a shared **factory
contract**: `|| async { (store, guard) }`, fresh store per test, the guard
(e.g. `TempDir`, or `()`) keeping backing resources alive; `skip_unless:` gates
environment-dependent adapters (postgres without `DATABASE_URL`). The crate
docs carry the **writing-a-store-adapter guide** (what you implement, the
append/read/wake contracts, running the kit, pinned contract notes), proven by
the toy-adapter acceptance test (`tests/toy_adapter.rs`): a HashMap
`RawEventStore + WakeSource + AtomicAppend` written against **only** the guide
under an enforced no-adapter-source restriction, 34/34 first run. WHY: the
freeze's executable spec — a third-party adapter is verifiable against the
contract with no tribal knowledge and no reading of fjall/postgres source. All
three shipped adapters consume the kit as a dev-dep (their local suites were
deduped into it, PR #313).

### Kernel contracts

The kernel uses `core` in production; `testing` adds allocator-backed fixtures.
`AggregateState::apply` consumes and returns state and requires no `Clone`.
`AggregateRoot` owns optional state, version and replay-work count. Application
panics leave a poisoned root; state access, commands and reuse return typed
errors until a new root is loaded from committed history.

Use `replay(version, event)` for strictly ordered rehydration and
`commit_persisted(events)` after a successful append. The latter derives and
checks the committed version range from the nonempty batch before folding;
callers cannot supply an arbitrary final version. Replay limits count successful
replayed events, not the absolute version of a restored snapshot.
`Handle` and `React` return `Option<Events>` for legitimate no-ops; root dispatch
keeps application rejection distinct from kernel availability errors.
`Saga::intent_for` derives outgoing intents from the saga's own recorded events.
See the contracts in `aggregate.rs`, `saga.rs` and `testing.rs`.

### Store contracts

`RawEventStore::append` takes a nonempty `PendingBatch`, checks the expected
committed head and returns the last assigned global position. `read_stream`
starts at an inclusive `Version`; `read_all` resumes strictly after its exclusive
position and yields `(position, raw stream key, envelope)`. Global ordering is
an adapter capability; disabled indexes return an explicit unavailable error.
`Store<S>` forwards raw operations and supported export/import capabilities.

`Encode` returns owned bytes. `Decode` has an output GAT supporting owned or
borrowed results; `OwningCodec` names the owning-output bound where required.
Envelopes carry validated frame offsets, schema, metadata and payload, with
neither an origin key nor a global position. Transformed decode preserves the
original event's version and metadata. Verification precedes transformation;
errors retain their typed sources. Plain upcast functions consume/return
`EventMorsel`; `EventStore::load_with` accepts them.

`SnapshotStore` is an unconditional cache. It is separate from `CheckpointStore`,
whose conditional writes replace revision, schema, position and state together.
A projection owns its matching state/position pair, validates order before
folding, and uses revision CAS. Failed or canceled persistence requires reload;
application panics poison the projection. Incompatible schema replacement uses
explicit `Projection::rebuild` and preserves revisions. The consumer drives the
loop; the library supplies primitives, not a runtime runner.

Atomic append validates distinct raw targets and complete run shapes before
storage work. Import validates all routing once before any writes, including
empty/corrupt sections. `WholeChunk` applies one atomic request; `PerStream` can
commit earlier valid runs before a later storage error. Ordinary exports read
independently. Consistent backup uses one finite-lifetime `ExportSession` for
listing and every stream read; borrow rules require releasing cursors before
close. Format-2 CBOR backups require `ChunkWriter::finish` and completion
validation before normal import. Partial/legacy recovery uses explicit salvage
with retained provenance; see [backup format](docs/backup-format.md).
See [store README](crates/store/README.md) and the trait docs for limits,
subscription cancellation/wake protocols, codec and persistence requirements.

### Fjall contracts

The adapter uses one bounded worker for synchronous engine work and cleanup.
Opening/recovery is synchronous startup; async calls and cursor refills use the
worker. Admission, cursor count and row/byte budgets are bounded. Dropping a
future can skip unstarted work; started writes can finish. Explicit close waits
for shutdown after other handles are released.

One writer transaction spans head checks, event/index staging and counters.
Checkpoint CAS and replacement use one writer transaction in separate keyspaces
from snapshot caches. The selected persistence policy applies to all writes;
`SyncAll` is the default. A storage error can leave an uncertain commit outcome.
Wakes follow successful persistence. Process-kill and syscall EIO tests establish
their tested boundaries, not hardware power-loss survival.

The layout manifest records version and index mode; incompatible/unmarked
nonempty stores reject on open. Key/value bounds are validated before staging.
Fjall checks actual payload addresses and realigns when needed; already aligned
buffers are reused. One export session retains a cross-keyspace snapshot on the
worker, with physical cleanup at the first worker turn after expiry even for
idle callers. Every session/cursor reserves cleanup admission; retained views
can delay database-wide reclamation. See [adapter README](adapters/fjall/README.md)
for exact limits, migration, durability and lifetime contracts.

### Macros

`DomainEvent` supports generic enums. `aggregate` validates marker declarations
and emits the `Aggregate` implementation and constructor. `transforms` emits
plain `upcast` / `current_version` functions; schema graph validation rejects
cycles, ambiguity and invalid u32 schema values. Known undeclared schemas return
`TransformError::UnsupportedSchema`; user failures preserve their source.
Compiler fixtures pin intended diagnostics and are reviewed on toolchain bumps.

### Examples

- **`inmemory`** — Pure in-memory event sourcing, no persistence (bank account domain)
- **`store-inmemory`** — Demonstrates all `mnesis-store` traits with `InMemoryStore`, including codec, upcasting, schema evolution, and the "substrate path" (Step 7) — driving `Store::raw()` directly with `futures::StreamExt::map_err` + `futures::TryStreamExt::try_fold` instead of going through the typed repository facade
- **`store-and-kernel`** — Full lifecycle integrating kernel + store: create → decide → encode → persist → read → decode → rehydrate

## Mandatory Rules

These rules are non-negotiable. Every one exists because of a real bug found in this codebase.

### 0 · 2 · 3 · 4 · 5 · 6 · 8 — Shared devrandom rules (EXTREMELY IMPORTANT)

Rules **0 (No Assumptions, No Opinions — Facts Only), 2 (Arithmetic Safety), 3 (Error Handling), 4 (API Design), 5 (Concurrency Safety), 6 (Functional-First, Allocate-Last), 8 (Test Quality)** are the shared devrandom engineering rules and apply here in full. Canonical text lives in the user-global `~/.claude/CLAUDE.md` ("Engineering rules — EXTREMELY IMPORTANT"). The original numbering is preserved because the architecture notes above cite rules by number (e.g. "rule 2", "rule 3"). Only the mnesis-specific rules are spelled out below.

### 1. Database Atomicity

Every database interaction in store adapters MUST be atomic:

- **Reads touching multiple partitions/keys**: single read transaction or shared snapshot — NEVER two independent reads
- **Writes**: write transactions
- **Read-then-write**: single transaction spanning both
- **Projection checkpoints**: conditional checkpoint writes are separate from event append. Revision, schema, position and state share one transaction; failed or canceled writes require reload. Aggregate snapshots remain best-effort caches. No combined event/checkpoint transaction is promised by the current interfaces.

If a public method does 2+ database calls without a shared transaction, it is a bug.

Mnesis-specific addenda to the shared rules:

- **Rule 3 addendum — `#[non_exhaustive]` is banned on enums, except public *error* enums at the 1.0 freeze.** During experimental development it adds friction without value; on non-error enums it stays banned forever (exhaustive matching catches real bugs). The carve-out (#209): at the 1.0 API freeze, every publicly-reachable **error** enum takes `#[non_exhaustive]` so adding a variant stays a minor change. `KernelError`/`StoreError`/`FjallError` et al. carrying it do **not** contradict the rule — do not "fix" them by stripping it. Private-mod error enums and non-error enums are out of scope.
- **Rule 3 addendum — adapter error types must be distinct from facade error types** (`InMemoryStore::Error = StoreError` causes double-wrapping); provide `From` impls at known crate-boundary mapping points; `ErrorId` truncation must be visually signaled.
- **Rule 4 addendum** — typestate builder intermediates are not independently constructable (seal or `pub(crate)` constructors); redundant data eliminated or validated (silent mismatch = corruption); type asymmetries across read/write paths intentional and documented; `pub(crate)` fields get a constructor so adding a field doesn't break construction sites.
- **Rule 6 addendum** — extension traits for GAT streams (`EventStreamExt`) keep call sites composable; collapse redundant branches that agree at the boundary value.

### 7. Testing — 4 Cross-Cutting Categories (highest priority)

Every new feature MUST include tests in these 4 categories BEFORE any other test methodology:

1. **Sequence/Protocol Tests** — Multi-step interactions on the same object. Test all valid operation sequences, not just individual operations in isolation.
2. **Lifecycle Tests** — Create, close, corrupt, reopen. If it persists state, test write-close-reopen-verify, write-corrupt-reopen-detect, and write-crash-reopen-recover.
3. **Defensive Boundary Tests** — Feed each crate inputs that violate its upstream crate's guarantees.
4. **Linearizability/Isolation Tests** — Concurrent readers and writers with snapshot consistency assertions.

After the 4 categories above, apply the 21 testing methodologies (see test strategy docs).

### 8. Test Quality Rules

Shared rule — see the global CLAUDE.md. Mnesis addenda: don't write a custom `ProbeStore` when `InMemoryStore`/`FjallStore` exist; state-machine tests include concurrent mode if the SUT is concurrent; no `Box::leak` in proptest without documentation and bounded iteration counts.

### 9. Architectural Decisions — Measure, Question, Decide (principal-engineer bar)

Refactors and adapter/kernel design are not "make it compile cleaner." Every
load-bearing design choice must be **surfaced, measured, and decided against the
real target** — not inherited by default. This rule exists because the
`mnesis-fjall` `$all` index (#270) stored a full second copy of every event
*implicitly*, and the cost was never stated or measured.

- **Don't take the premise (or the existing design) at face value.** A task that
  says "this is bloated / badly designed" is a hypothesis to verify, not a fact to
  execute. Measure first: the fjall "1754-LOC bloat" was ~61% inline tests; the
  production code was already clean. Report the honest reframe.
- **Surface the implicit load-bearing decision.** Every adapter hides one or two
  choices that dominate its cost/behavior (here: the `$all` denormalization —
  `events_global` stores a full frame copy to buy a
  point-read-free `$all` scan). Make it explicit in code + docs; a silent
  architectural trade must be documented.
- **Measure the fork, don't assert it.** Design choices ship with workload,
  implementation/version, host and command attached to measured results.
  `adapters/fjall/benches/all_index_layout.rs` compares historical layouts;
  rerun it before applying its figures to changed production code. "Facts only"
  applies to design tradeoffs as well as APIs.
- **Weight by the real target: `IoT`/mobile-first.** Decide by the *primary*
  deployment's binding constraint (flash-write + storage on-device), not the
  convenient server/default case. Ask "what does this cost on the constrained
  device?" — and whether the benefit is even *used* there (a produce-and-sync
  device that never reads `$all` pays the index cost for nothing). See
  [Target Platforms].
- **Make genuine either-way forks configurable, with a safe default.** When two
  real deployments have opposite optima (projection-heavy server wants the `$all`
  index; produce-sync `IoT` device wants it gone), expose a knob
  (`AllIndex::{Denormalized default, Disabled}`) rather than hardcoding one — and
  keep the default safe/least-surprising.
- **A "keep it as-is" conclusion is legitimate — but only once measured.** The
  answer can be "the existing design is right for the domain," but you must have
  the numbers and the explicit rationale, not a shrug.

## Key Conventions

- **Rust edition 2024** with `rustfmt` edition 2024
- **Toolchain & MSRV**: pinned to an exact **stable** release in `rust-toolchain.toml` (`channel`), which is the single source of truth for both `rustup` users and the Nix flake (consumed via fenix `fromToolchainFile`). **No nightly** — the crates carry no `#![feature(...)]` gates and the whole workspace (build, test, clippy, fmt, coverage, examples) runs on the pinned stable. MSRV policy: `rust-version` in `[workspace.package]` **equals** the pinned toolchain; when bumping Rust, bump `rust-toolchain.toml` *and* `rust-version` together (the pin tracks the latest stable we actually build/test against — no separate lower floor is claimed unless a CI job verifies it). All publishable crates opt in via `rust-version.workspace = true`.
- **Strict clippy**: `all`, `pedantic`, `nursery` denied; `unwrap_used`, `expect_used`, `panic`, `todo`, `as_conversions`, `shadow_*`, `allow_attributes_without_reason` all denied
- **Workspace dependencies**: all dependency versions declared in root `Cargo.toml` `[workspace.dependencies]`; crate-level Cargo.toml files use `workspace = true`
- **workspace-hack crate**: managed by `cargo-hakari` for build optimization; run `cargo hakari generate` after dependency changes
- **Commit style**: conventional commits (`feat:`, `fix:`, `docs:`, `refactor:`) with optional scope (e.g. `feat(fjall):`, `fix(store):`)
- **Dual license**: MIT OR Apache-2.0
- **Property-based tests** via `proptest` in kernel, store, and fjall crates
- **CI checks** (via Nix flake): clippy (deny warnings), fmt, taplo fmt, cargo-audit, cargo-deny, nextest, tarpaulin coverage (Linux only), hakari verification
