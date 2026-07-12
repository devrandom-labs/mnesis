# Adapter Conformance Kit — Design (#281)

**Date:** 2026-07-10
**Issue:** [#281] Promote `mnesis-store-testing` into the executable store-contract spec
**Milestone:** 1 — Mnesis: Pre-Freeze (1.0 blockers)

## Problem

The 1.0 freeze pins the store adapter contract (`RawEventStore` + `WakeSource` +
`AtomicAppend` + subscription semantics). Today that contract lives implicitly in
`mnesis-store-testing` (two suites: per-stream `EventStream`, `$all` read path)
plus the fjall/postgres/in-memory test suites. Third parties writing adapters
after the freeze would re-discover every edge case (GlobalSeq semantics,
wake-loss races, inclusive-vs-exclusive cursor bounds) as production bugs.

## Goal

`mnesis-store-testing` becomes the **public, executable spec** of the store
adapter contract: a `conformance!` macro entry point over public per-check
functions, structured by the 4 test categories, documented by a "writing a
store adapter" rustdoc page, proven by a toy adapter written against only
that page.

## Design

### 1. Crate shape — 4 categories become modules

```
crates/store-testing/src/
  lib.rs             — adapter-guide rustdoc, macro definitions, re-exports
  row.rs             — ConformanceRow + shared drive/drain helpers
  sequence.rs        — Sequence/Protocol checks
  lifecycle.rs       — close→reopen checks (opt-in)
  boundary.rs        — Defensive Boundary checks
  linearizability.rs — concurrency/isolation checks
```

Every check is a public generic async fn
(`pub async fn check_<name><S, F, Fut>(factory: &F)`) so third parties can
cherry-pick. The existing `assert_event_stream_conformance` and
`assert_all_stream_conformance` suites dissolve into `sequence.rs`; their
checks are re-driven through the store factory (the kit appends rows and opens
`read_stream` itself), which deletes the adapters' stream-keep-alive
boilerplate (fjall's `OwnedFjallStream`).

### 2. Factory contract

Core factory: `Fn() -> Fut<Output = S>` with `S: RawEventStore + WakeSource` —
one fresh, empty store per check (the pattern the `$all` suite already
proved). `WakeSource` is **required in core**: subscription semantics are part
of the frozen contract, and a third party gets an in-process impl for free by
embedding `mnesis-wake::StreamNotifiers` (the guide teaches exactly this).

### 3. Macro entry points

One `macro_rules!` per capability, each generating one `#[tokio::test]` per
check under category modules, so nextest reports every contract rule as its
own named test:

```rust
mnesis_store_testing::conformance! {
    factory: || async { InMemoryStore::new() },
    // optional (postgres): skip_unless: || std::env::var("DATABASE_URL").is_ok(),
}
mnesis_store_testing::conformance_atomic_append! { factory: ... } // S: + AtomicAppend
mnesis_store_testing::conformance_snapshot! { factory: ... }      // feature "snapshot"
mnesis_store_testing::conformance_lifecycle! {                    // persistent adapters
    open:   || async { /* -> (store, ctx) */ },
    reopen: |ctx| async { /* drop handle, reopen same storage -> (store, ctx) */ },
}
```

- `skip_unless:` — generated tests return early (pass) when the guard is
  false; replaces postgres's hand-rolled skip-without-`DATABASE_URL`.
- `conformance_lifecycle!` is opt-in (`InMemoryStore` has nothing to reopen);
  the open/reopen closure pair over an adapter-chosen context type covers both
  fjall (tempdir path) and postgres (same database, fresh pool).
- Generated test names are public API: renaming/deleting a check after 1.0 is
  visible spec churn, by design.

### 4. Check matrix

**sequence.rs (Sequence/Protocol)**
- append → read round-trip via the store (absorbs the old per-stream suite:
  fused-after-None, strictly monotonic versions, byte-faithful
  event_type/schema_version/payload round-trip, insertion order, 1024-event
  drain)
- version conflict: wrong `expected_version` surfaces a typed conflict with
  expected/actual; retry with the corrected expectation succeeds
- `$all` (absorbs the #266 suite): position order across interleaved streams,
  strictly increasing positions, `read_all(Some(p))` exclusive, multi-resume
  reconstruction with no gap/dup/skip, inclusive-`read_stream` /
  exclusive-`read_all` asymmetry on one store
- subscription protocol (per-stream and `$all`): backlog delivered in order →
  `CaughtUp` exactly once → live events after; `from: None` = beginning;
  `from: Some(v)` strict-after with no duplicate on resume
- large backlog (> `CATCHUP_CHUNK` = 1024) crosses refill/scan seams with no
  gap or duplicate

**lifecycle.rs (Lifecycle, opt-in)**
- append → close → reopen: events, versions, and the GlobalSeq high-water mark
  survive (new appends after reopen get strictly higher positions)
- reopen → subscription catch-up delivers the pre-close backlog

**boundary.rs (Defensive Boundary)**
- conflict-rejected append leaves the store byte-identical (nothing lands)
- version-gap append (first incoming version ≠ next-expected) is rejected
  without landing
- metadata `None` round-trips as `None`; present metadata round-trips
  byte-for-byte, including the 1-byte minimum. (`Some(empty)` is
  unrepresentable by construction — `Metadata::from_bytes` rejects empty with
  `ValueError::MetadataEmpty`, and the wire reserves `u32::MAX` as the absent
  sentinel — so the absent-vs-empty confusion the original design targeted is
  already impossible at the type layer; finding from Task 4.)
- max-length event_type (`u16::MAX` bytes) round-trips
- binary (non-UTF-8) and Unicode stream ids round-trip; empty stream id is a
  documented carve-out (fjall rejects it), not asserted

**linearizability.rs (Linearizability/Isolation)**
- N concurrent appenders, same stream, same `expected_version` (real overlap:
  `tokio::spawn` + `Barrier`): exactly one wins, the rest surface conflicts,
  the store holds exactly the winner's events
- concurrent appends to distinct streams all land; `$all` contains all with
  strictly increasing positions
- wake-after-idle: subscribe, drain to `CaughtUp`, park; append from another
  task; the event arrives (bounded timeout)
- `CaughtUp`-boundary race: appends racing the catch-up→live transition are
  neither lost nor duplicated, `CaughtUp` still emitted exactly once

**atomic_append (capability, `conformance_atomic_append!`)**
- multi-stream `atomic_append_many` commits all runs or none
- a conflict in one run aborts the whole batch — no partial landing on any
  stream, `Conflict { index, actual }` identifies the offender

**snapshot (feature `snapshot`, `conformance_snapshot!`)**
- `hydrate` on fresh store → `Absent`; `commit` → `Found` with the same
  `(position, state)` pair (committed and loaded together)
- `hydrate` under a different schema version → `Stale { stored_schema }`

### 5. Contract ambiguities pinned (the pre-freeze payoff)

Found during design; the guide documents each, and the kit asserts only the
intersection:

- **Read visibility under concurrent append is unspecified.** fjall's
  `ScanCursor` is snapshot-pinned at open; `InMemoryStream` keyset-refills and
  observes later appends. The kit asserts: a finite read delivers *at least*
  everything appended before open, gap-free and byte-faithful.
- **GlobalSeq is strictly monotonic but never gapless** — asserted increasing,
  gaplessness never asserted.
- **Empty stream ids** are a permitted adapter limitation (fjall rejects
  them); the kit uses non-empty ids.
- **"Batch-bound respect"** (from the issue) is not externally observable
  (fjall deleted the knob); the observable contract is seam-correctness on
  large backlogs, asserted in `sequence.rs`.

Further ambiguities flushed out during PR1 get pinned in the guide the same
way.

### 6. Delivery — 3 PRs

1. **PR1 `feat(store-testing)`** — modules + checks + macros; the three
   adapters switch their conformance test files to the macros (additive
   elsewhere).
2. **PR2 `refactor(adapters)`** — delete adapter-local tests the kit now
   covers; anything an adapter tests that the kit doesn't becomes a kit check
   *first*, then the local copy dies. Adapter-specific suites (fjall
   crash-recovery, postgres watermark internals) stay.
3. **PR3 `docs(store-testing)`** — the "writing a store adapter" rustdoc page
   + a toy `HashMap` adapter in `store-testing/tests/` written against only
   that page, running the full `conformance!` matrix in CI. This is the
   issue's acceptance proof.

### 7. Publishing & CI

The crate is already a workspace member with description/keywords; it ships
with the workspace release. Adapter conformance tests and the toy adapter run
under the existing `nix flake check` nextest gate. Postgres conformance runs
in the nixosTest integration attribute as today (via `skip_unless`).

## Error handling

The kit is a test harness: every check panics with a descriptive,
contract-citing message (the crate-level allows for `unwrap`/`expect`/`panic`
already exist). Checks never swallow adapter errors — unexpected `Err` panics
with the debug repr; *expected* errors (conflicts) are matched structurally.

## Out of scope

- Export/import (`StreamLister`/`EventImporter`) conformance — separate
  capability, already proven on fjall via #220; can be a follow-up macro.
- A conformance suite for `WakeSource` implementations in isolation
  (`mnesis-wake-nostd`'s `GlobalWake` is exercised through its own crate's
  tests).
- Crash-injection / corruption lifecycle tests — adapter-specific by nature.
