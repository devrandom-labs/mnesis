# Structural / design-pattern review — `mnesis-fjall`

A **design review** (structure & responsibility decomposition), distinct from the
line-level idiomatic pass in `review-fjall-adapter.md`. Question: is the fjall
adapter decomposed with the right patterns so it stays clean and manageable — or is
there accidental complexity a pattern would tame, or ceremony a pattern is imposing
that Rust doesn't need?

Scope: `adapters/fjall/src/` — `store.rs`, `partition.rs`, `scan.rs`, `plan.rs`,
`snapshot.rs`, `global_seq.rs`, `wire_key.rs`, `builder.rs`, `lib.rs`.
(`error.rs` and `subscription_id.rs` are intentionally out of scope.)

Rubric: `./.auto/refs/patterns.md`. API correctness: `./.auto/refs/fjall.md`.
Settled/measured storage decisions (partition layout, per-partition LZ4, one atomic
`write_tx`, structural-not-fsync) are **not** revisited here.

---

## Findings table

| Module | Pattern intent | ✓/✗ | Finding | VERDICT + one-line why |
| --- | --- | --- | --- | --- |
| `store.rs` (`impl RawEventStore`, `WakeSource`, `SnapshotStore`) | **Adapter** — `mnesis-fjall` impl'ing kernel traits over fjall, fully translating errors | ✓ | Clean adapter: no fjall types leak through `RawEventStore`/`SnapshotStore`; every `FjallError` is wrapped at the boundary (`store.rs:161-162`, `partition.rs:200` `map_err(FjallError::Io)`). | **DON'T** — idiomatic adapter; nothing leaks. |
| `partition.rs` (`Partitions`) | **Facade** — one owner of the physical layout | ✓ | The crate's single owner of every partition + codec (`partition.rs:123-149`). All reads/writes route through `Partitions`; callers (`append`, snapshot, export lister) never name a partition or key format. | **DON'T** — real Facade, no caller reaches past it into `fjall::*`. |
| `plan.rs` (`plan_run`) + single `write_tx` in `store.rs` | **Command / Unit-of-Work** — stage then commit atomically | ✓ | `plan_run` (`plan.rs:82`) is the pure staged plan; both `append` (`store.rs:146`) and `atomic_append` (`store.rs:494`) stage via it and commit in **one** `write_tx`. Staging is never interleaved with commit; no per-item commit loop. | **DON'T** — rule-1 atomicity is structurally guaranteed; one tx per path. |
| `plan.rs` shared by `append` + `atomic_append` | **Template Method** — shared skeleton, vary steps | ✓ | The two write paths share the *one* encode core `plan_run`; they differ only in the pre-stage validation (`check_optimistic` vs `validate_atomic_writes`) and error mapping. No copy-pasted skeleton. | **DON'T** — shared fn is the Rust-native template-method realization. |
| `scan.rs` (`ScanCursor`) | **Iterator / Cursor** — lazy, zero-copy | ✓ | `ScanCursor<S>` is `impl futures::Stream` over one lazy `fjall::Iter` (`scan.rs:234`); snapshot pinned at `open` (`scan.rs:176-181`); `Slice → Bytes` zero-copy via `bytes_1` (`scan.rs:107`); poisoned-on-error, no silent skip (`scan.rs:210-228`). | **DON'T** — exactly the rubric's ✓ Iterator/Cursor; no eager `collect()`. |
| `scan.rs` (`ScanStrategy` + 2 impls) | **Strategy** — swap the scan's key/decode steps | ✓ | `ScanStrategy` (`scan.rs:20`) drives one `ScanCursor`; `StreamScan` and `GlobalScan` are the two genuinely-needed impls. A trait (not a `match on mode`) is the right call because the set has 2 real members. | **DON'T** — 2 impls ⇒ real Strategy; not one-impl ceremony. |
| `scan.rs` + `store.rs::stream_lister_impl` | **Template Method** — shared "poisoned lazy cursor" skeleton | ✗ (mild) | `ScanCursor` and `StreamIdCursor` duplicate the `fjall::Iter` + `poisoned: bool` + `poll_one: Option<Result<_, FjallError>>` + `futures::Stream` shape (`scan.rs:164-243`, `store.rs:573-605`). | **DON'T** — the two differ in *what they yield* (decoded row vs key-only `StreamKey`) and the id cursor is key-only (no value load). A shared generic adds a decode-or-identity closure for ~30 lines; ceremony. |
| `partition.rs` (`AllIndex` enum + `stage_event` branch) | **Strategy vs enum+match** | ✓ | `$all` denormalization is an **enum** (`AllIndex`, `partition.rs:109`) checked at the *single* dual-write site `stage_event` (`partition.rs:241-247`) plus one `read_all` guard (`store.rs:193`). No mode-match is repeated. | **DON'T** — canonical "enum+match beats a one-impl Strategy trait"; the branch is localized, not scattered. |
| `builder.rs` (`FjallStoreBuilder<S,E>`) | **Builder / Typestate** — staged validated construction, zero dyn-dispatch | ✓ | Typed config carried as `S`/`E` (`builder.rs:23`); `streams_config`/`events_config` (`builder.rs:62,85`) transition the type; `open` (`builder.rs:109`) finalizes with all defaults valid. | **DON'T** — correctly realized; `open` is always valid (all fields defaulted) so no missing typestate. |
| `partition.rs` (`KeyspaceConfig` sealed trait) | **Strategy via sealed trait + closure** — monomorphise config | ✓ | Only `()` and `F: FnOnce(..)` impl (`partition.rs:35-52`); eliminates `Box<dyn>` by monomorphising the builder (`builder.rs:112-121`). | **DON'T** — sealed trait + closure earns its place by killing dynamic dispatch; not ceremony. |
| `global_seq.rs` (`GlobalSeq` newtype) | **Smart constructor / newtype** — illegal state unrepresentable | ✓ | `GlobalSeq(NonZeroU64)` (`global_seq.rs:22`); `next()` uses `Option`/`checked` (`global_seq.rs:30`); `0` is unrepresentable. | **DON'T** — arithmetic-safe, makes `0` illegal at the type level. |
| `plan.rs` (`StagedRow`) | **Proof-carrying newtype** — validated+encoded row | ✓ | `StagedRow` (`plan.rs:36`) is proof the event passed the strict-sequential check and encoded cleanly; the IO shell is a mechanical insert loop (`store.rs:153-158`). | **DON'T** — borrow-free, makes the insert loop obviously safe. |
| `store.rs` (`append_plan_err` / `atomic_plan_err`) | **Error-domain separation** | ✗ (mild) | Two near-identical mappers turn neutral `PlanError` into `AppendError` / `AtomicAppendError` (`store.rs:82`, `store.rs:392`). They differ only in `Conflict` shape (`index` vs `expected/actual`). | **DON'T** — rule 3 (one variant = one failure domain) justifies a distinct mapper per target type; a shared core would thread a closure for one arm. |
| `store.rs` (two `SnapshotStore` impls) | **Template Method** — duplicate hydrate/commit skeleton | ✗ (mild) | `snapshot_impl` (`store.rs:227`) and `projection_impl` (`store.rs:306`) duplicate ~35-line hydrate/commit bodies; they differ only in partition (`read_snapshot`/`read_projection`) and Position newtype (`Version::new` vs `GlobalSeq::new`). | **DON'T** — distinct concepts (aggregate snapshot @ `Version` vs projection checkpoint @ `GlobalSeq`), feature-gated separately; a generic helper (closure for the one differing arm) is ceremony *now*. Seam to revisit at a 3rd impl. |
| `snapshot.rs` (`encode/decode_snapshot_value`) | **Facade for the snapshot blob codec** | ✓ | One `[u32 LE schema][u64 BE pos][payload]` codec (`snapshot.rs`) reused by both `SnapshotStore` impls; the `u64` position serves `Version` and `GlobalSeq` with no new shape. | **DON'T** — shared codec; no per-impl encoder duplication. |
| `wire_key.rs` (`encode/decode_*_key`) | **Pure wire-codec module** (no pattern needed) | ✓ | Key layout + `const`-size helpers + `checked`/`DecodeError` (`wire_key.rs`); no fjall coupling; unit-tested. | **DON'T** — a focused codec module; "no pattern needed" is the right verdict. |

---

## Findings that map to the rubric's two failure modes

### Accidental complexity a pattern *would* tame

- **None that clears the bar.** The two mild duplications (the two cursors; the two
  `SnapshotStore` impls; the two plan-error mappers) are each gated by a *real*
  distinguishing factor — yielded item shape, feature flag + Position semantics, and
  distinct error domain respectively. A pattern would add a closure/closure-param
  indirection for 30–60 lines saved, which the rubric classifies as ceremony at this
  size. The strongest candidate (`SnapshotStore` impls) is explicitly the **seam to
  revisit when a 3rd impl appears** — not before (YAGNI / "seam at the 2nd impl").

### Ceremony a pattern is imposing that Rust dissolves

- **None.** Every trait present has ≥2 real impls (`ScanStrategy`×2, `KeyspaceConfig`×2)
  or is a sealed zero-cost config carrier, so none is a one-impl Strategy/Visitor
  smelling of over-abstraction. `AllIndex` is an **enum+match**, correctly *not* a
  Strategy trait. No abstraction layer wraps a single implementation.

### Rule-1 / zero-copy / settled-design spot-checks (from `fjall.md`)

- **Atomicity (rule 1):** exactly one `write_tx` per `append` (`store.rs:122`) and per
  `atomic_append_many` (`store.rs:535`); both drop-uncommitted on any `Err` so nothing
  lands across partitions. No second `write_tx`, no per-stream commit loop. ✓
- **Lazy scans:** `ScanCursor` and `StreamIdCursor` are both lazy `impl Stream` over a
  single `fjall::Iter`; no `.collect()` of a large scan anywhere. ✓
- **Zero-copy on the hot path:** event-scan value is `Slice → Bytes` (`scan.rs:107`,
  `bytes_1`); keys stay borrowed `Slice`. The only `.to_vec()` is in snapshot `hydrate`
  (`store.rs:264,347`) — a point-read forced by the kernel's `SnapshotStore<Vec<u8>, _>`
  owned return type, **not** the event-scan hot path. ✓
- **No `unwrap`/`expect` on fjall `Result`:** all error paths use
  `.map_err(FjallError::Io)?` / `decode_*().map_err(..)`. ✓
- **Arithmetic safety:** every version/`GlobalSeq` step uses `checked_add`
  (`plan.rs:96,104`; `store.rs:451,459`; `global_seq.rs:30`) — no `saturating_*`,
  no `unwrap_or(u64::MAX)`. ✓
- **Settled design untouched:** partitions, LZ4, structural-not-fsync, and the
  `AllIndex` denormalization decision are all preserved; no finding touches them. ✓

---
## Addendum — cross-adapter finding: the kernel `store` module pushes the append-version contract onto every adapter

Scope widened per review follow-up: compare the kernel `RawEventStore` (`crates/store/src/store.rs`) against the **fjall** (`adapters/fjall/src/{store,plan}.rs`) and **postgres** (`adapters/postgres/src/store.rs`) adapters, plus the in-memory reference (`adapters/inmemory/src/lib.rs`). The concern: does the kernel `store` module make adapters *take decisions* they shouldn't — logic that belongs once, in the kernel, re-implemented per adapter?

### The redundant decision

`RawEventStore::append` (`crates/store/src/store.rs:159-194`) specifies the contract **in prose only**: optimistic concurrency (`expected_version` must equal current, else reject); strict-sequential (`expected_version+1, +2, …`, no gaps/dupes); atomic check+insert (warns against SELECT-then-INSERT). It provides **no default impl and no shared helper**, so three adapters each re-implement the *same pure, DB-free* validation:

| Adapter | Where the contract is re-implemented | Duplicated logic |
| --- | --- | --- |
| fjall | `check_optimistic` (`store.rs:50`) + `plan_run`'s running `checked_add(1)` loop (`plan.rs:89-103`) | optimistic + strict-sequential + overflow |
| postgres | `prepare_inserts` (`store.rs:281-333`) | optimistic + strict-sequential + overflow (narrowed to `i64`) |
| inmemory | `impl RawEventStore` append — doc "Includes optimistic concurrency and sequential version validation" (`lib.rs:139`) | same contract, under a `Mutex` |

That is the rubric's **Template Method** anti-pattern ("✗ copy-pasted skeleton across impls"): the validate-then-stage skeleton is shared by 3 impls, but the *shared* part (validation) is copy-pasted while only the *varied* step (storage staging) is left to each adapter.

### The drift it already caused

The two production adapters don't encode the contract identically — the hazard the rubric warns about:

- **fjall** special-cases new streams: `current == 0` requires `expected_version.is_none()`, else `Conflict` (`store.rs:51-66`).
- **postgres** folds it into one check: `expected.map_or(0) != current → Conflict` (`store.rs:288-294`). No `current == 0` branch.

Behaviour is currently equivalent (`Version` is `NonZero`, so `Some(0)` is unconstructable), but the *code paths diverge*. Any future contract tweak must be edited in 3 places and can silently drift.

### What is correctly adapter-specific (do NOT pull into the kernel)

- **Wire encoding**: fjall → one `[schema][position][payload]` frame (`wire::encode_frame` in `plan_run`); postgres → normalized columns; inmemory → `StoredFrame`. Different on-disk shapes — adapter-owned. (`AllPosition` is already adapter-defined: `crates/store/src/store.rs:330-361`.)
- **`$all` position assignment**: fjall stamps `GlobalSeq`; postgres reads a DB-assigned `(txid, seq)`; inmemory bumps `InMemoryAllPos`. Genuinely adapter-specific.

So the right extraction is **only the pure version-validation core**, not a full Template Method that stages — over-abstracting staging would be the ceremony the rubric warns against.

### VERDICT: **APPLY — raise as an issue**

Add one pure, adapter-agnostic validator to `mnesis-store` (e.g. `store.rs` or a `store::validate` submodule):

```rust
/// Validate the append contract once, for every adapter.
/// `current` = stream's max version (0 = new stream). Returns `Conflict`
/// on optimistic mismatch or a non-sequential batch, `Store(VersionOverflow)`
/// on `u64::MAX` overflow — never a silent sentinel.
pub fn validate_append_versions<E>(
    current: u64,
    expected: Option<Version>,
    envelopes: &[PendingEnvelope],
) -> Result<(), AppendError<E>>
where
    E: core::error::Error + Send + Sync + 'static;
```

(Or return the validated sequence `[current+1, current+2, …]` so adapters stop re-deriving `expect`.) Each adapter calls it *inside* its tx/lock, immediately before the storage-specific staging loop — fjall at `store.rs:124-147`, postgres at `store.rs:428-429`, inmemory inside its `Mutex` critical section.

- **Coupling removed:** the contract lives in one place; 3 copies of the `checked_add` loop disappear; the new-stream divergence collapses to one branch.
- **Atomicity preserved:** validation still runs inside the single `write_tx` (fjall) / `PgTransaction` (postgres) / `Mutex` (inmemory) — before any insert. No second transaction, no per-item commit.
- **Zero-copy preserved:** pure; allocates only a `checked_add` counter (and optionally the validated-sequence `Vec`); no value copies.
- **Settled design untouched:** partition layout, LZ4, structural-not-fsync, `AllPosition`, and on-disk wire format are unchanged — the refactor moves *validation*, not *storage*.
- **Low risk:** a pure `Result` fn with the exact `Conflict`/`VersionOverflow` semantics the adapters already produce; each call site is a drop-in pre-stage guard, gated by existing tests (`plan.rs` tests, postgres `prepare_inserts` tests, inmemory `bounded_read_tests`).

This is the one structural change worth making. Everything else in the adapter is clean.

---

## Overall verdict

**The `mnesis-fjall` adapter is genuinely clean — neither over-abstracted nor under-structured. But the kernel `store` module has one cross-adapter structural defect: it documents the append-version contract but implements none of it, so every adapter re-derives it (see Addendum).**

The fjall adapter decomposes along exactly the seams Rust-native patterns suggest: a **Facade** (`Partitions`) hides the physical layout, an **Adapter** (`FjallStore`) translates the kernel traits, a pure **Command/Unit-of-Work** (`plan.rs`) is the shared encode core committed in one atomic `write_tx`, **Strategy** (`ScanStrategy`) and **Builder/typestate** (`FjallStoreBuilder`) are realized idiomatically, and **newtypes** (`GlobalSeq`, `StagedRow`) make illegal states unrepresentable. enum+match is correctly preferred over Strategy for the binary `AllIndex` decision.

No accidental complexity *within the adapter* crosses the "measurably cuts complexity" bar, and no ceremony is present. The three mild duplications are each deliberate, semantically distinct, and would be *more* complex to unify at this size.

### Top changes actually worth making

1. **APPLY (raise as issue) — extract the append-version contract into the kernel.** See the Addendum. A pure `validate_append_versions` in `mnesis-store`, called by fjall/postgres/inmemory before their storage-specific staging. One source of truth for the contract; removes the 3× copy-paste and the new-stream code-path drift.
2. *(Optional, not recommended)* If a **3rd** `SnapshotStore` impl is ever added, unify `snapshot_impl`/`projection_impl` behind a generic `hydrate`/`commit` helper taking the partition reader + a `u64 → Position` constructor — that is the moment the Template-Method seam pays off. Until then, leave them.
3. *(Optional, not recommended)* The two `plan_err` mappers could share a `map_overflow → Store` core, but rule-3's one-domain-per-variant discipline justifies keeping the mappers separate.

**Bottom line:** within `mnesis-fjall`, apply nothing. The one real refactor is *cross-adapter* and owned by the kernel `store` module — raise it as an issue; don't over-abstract the two near-duplicate trait impls and cursors.
