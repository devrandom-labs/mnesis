# Conformance Kit PR3 Implementation Plan (#281) — adapter guide + toy-adapter acceptance

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development or superpowers:executing-plans, task-by-task.

**Goal:** Ship the "writing a store adapter" rustdoc guide in `nexus-store-testing`, prove it with a toy HashMap adapter written against ONLY that guide (the issue's acceptance criterion), and close #281.

**Branch:** `feat/281-conformance-kit-pr3` off `e5e0078`.

**Acceptance (from #281):** "A toy third-party adapter (e.g. a HashMap store written against only the rustdoc) passes the kit without reading fjall/postgres source." Operationalized: the toy-adapter implementer agent is FORBIDDEN from reading `adapters/*/src`; it may read only `nexus-store-testing`'s crate docs (the guide), `nexus-store`'s public rustdoc/source of the traits it implements, and `nexus-wake`'s public API. Every place it gets stuck is a GUIDE GAP: the fix is a guide edit, never adapter-source access.

**Rules recap:** full flake-check hook on every commit (never bypass); fmt before staging; deviation log below.

---

## Task 1: The "writing a store adapter" guide

**Files:** `crates/store-testing/src/lib.rs` (crate-level `//!` docs — expand the existing kit docs into the full guide).

- [ ] Structure (top-level sections in the crate docs):
  1. **What you implement** — `RawEventStore` (assoc types incl. `Stream`/`AllStream`/`AllPosition`; `Error: Error + Send + Sync + 'static`) + `WakeSource`. Capability traits: `AtomicAppend` (at `nexus_store::import::`, behind nexus-store's `import` feature), `SnapshotStore<Vec<u8>, P>` (behind `snapshot`).
  2. **The append contract** — optimistic head check against `expected_version` (`None` = fresh stream); envelope versions must run sequentially from `expected + 1` (a gap/duplicate/out-of-order batch is `AppendError::Conflict`, and NOTHING lands); `Conflict` carries the actual head; stamp a store-local `AllPosition` on every event, strictly monotonic across ALL streams in commit order, NOT required gapless. Empty batch = Ok no-op.
  3. **The read contract** — `read_stream(id, from)`: `from` INCLUSIVE, ascending versions, terminates `None`, absent stream = empty stream (not an error), fused after `None`. `read_all(from)`: `from` EXCLUSIVE (strictly after), position-tagged items, `Ord`-based resume (no successor fn). The intentional asymmetry and why (single stream has a gapless successor; composite `$all` position does not).
  4. **The wake contract** — `register(Option<&[u8]>)` (`None` = `$all`) BEFORE the caller's re-scan; `wake(stream)` after every durable commit (per-stream AND `$all` observers); spurious wakes are permitted (cost = one empty re-scan); lost wakes are NOT (the kit's wake-after-idle and boundary-race checks catch them). In-process adapters: embed `nexus_wake::StreamNotifiers` and delegate — show the ~10-line snippet.
  5. **Running the kit** — factory contract `(store, guard)`, the four macros, `skip_unless:`, snapshot `positions`/`extremes`, lifecycle `open`/`reopen`.
  6. **Contract notes** (keep/extend the PR1 list): read visibility under concurrent append unspecified; positions monotonic-not-gapless; empty stream ids a permitted limitation; `Some(empty)` metadata unrepresentable; error enums `#[non_exhaustive]` (match `Conflict` + wildcard).
- [ ] Facts only: every semantic claim must be checked against `crates/store/src/{store,wake,subscription,import,state}.rs` doc comments — the guide RESTATES the contract, it must not invent or drift. Where the trait docs already say it, the guide may link (`[RawEventStore::append]`) and summarize.
- [ ] `nix develop -c cargo doc -p nexus-store-testing --all-features --no-deps` builds clean (workspace denies rustdoc warnings via the flake's doc check).
- [ ] fmt; commit `docs(store-testing): the writing-a-store-adapter guide (#281)` (full hook).

## Task 2: Toy adapter acceptance test

**Files:** `crates/store-testing/tests/toy_adapter.rs` (new); `crates/store-testing/Cargo.toml` (dev-dep `nexus-wake`).

- [ ] Dispatched to an agent that has NOT seen the adapter sources, with the read restriction above. Deliverable: a `ToyStore` — `parking_lot`-or-tokio-mutexed `HashMap<Vec<u8>, Vec<StoredEvent>>` + a global `u64` position counter + embedded `nexus_wake::StreamNotifiers` — implementing `RawEventStore + WakeSource` (+ `AtomicAppend` if the guide alone suffices; skip `SnapshotStore` — a toy second store type adds nothing).
- [ ] The file runs `conformance! { factory: || async { (ToyStore::new(), ()) } }` (+ `conformance_atomic_append!` if implemented) — the FULL core matrix must pass.
- [ ] Every question the implementer cannot answer from the guide = a guide gap: it reports the gap, the controller amends the guide (Task 1 file), and the implementer retries. Gaps and their fixes go in the deviation log — they are the acceptance findings.
- [ ] Note: `tokio::sync::Mutex` or `parking_lot::Mutex` only (`std::sync::Mutex` is banned by clippy `disallowed_types`); test-target code, crate-level allows apply.
- [ ] fmt; commit `test(store-testing): toy HashMap adapter passes the kit — #281 acceptance (full hook)`.

## Task 3: CLAUDE.md + close-out

- [ ] CLAUDE.md: update the workspace-layout/store sections to describe `nexus-store-testing` as the executable store-contract spec (4-category modules, capability modules, the four macros, factory contract, toy-adapter acceptance, guide location). Capture the WHY (freeze's executable spec; adapter authors need no tribal knowledge).
- [ ] `nix develop -c cargo clippy --workspace --all-features --all-targets` clean; hakari no-op check.
- [ ] Commit `docs: conformance kit in CLAUDE.md (#281)`; push; PR titled `docs(store-testing): adapter guide + toy-adapter acceptance (PR3 of #281)`, body summarizing guide + acceptance proof + guide-gap findings, with `Closes #281`. Attribution line per repo convention.

---

## Deviation log

| # | Task | Deviation | Why | Impact |
|---|------|-----------|-----|--------|
