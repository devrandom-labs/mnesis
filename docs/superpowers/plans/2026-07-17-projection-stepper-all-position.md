# Projection Stepper `$all` Generalization Implementation Plan (#327 + #328)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Generalize `Projection` and `PersistTrigger` over the checkpoint position type so one stepper drives both per-stream (`Version`) and `$all` (`AllPosition`) projections, and delete the hand-rolled `$all` loop in `examples/axum-todos`.

**Architecture:** Three source-compatible generalizations in `mnesis-store`, all mirroring patterns the codebase already committed to: (1) `PersistTrigger<P = Version>` — defaulted trait type param (like `PartialEq<Rhs = Self>`), `AfterEventTypes` becomes position-generic, `EveryNEvents` stays `Version`-only (bucket arithmetic has no meaning on a composite `$all` position — decided, no scalar-view capability trait for 1.0); (2) a sealed `Positioned` item trait (mirrors `RawItem` in `decoded.rs`) so `advance` accepts both `Decoded<E>` (position = inner `version`) and `(P, Decoded<E>)` (position = the `$all` tag, exactly as `.decoded()` yields it) — the position/event pairing is structural and unforgeable; (3) `Projection<I, P, Trig, SS, Pos = Version>` — defaulted struct param, `checkpoint`/`pending` become `Option<Pos>`. Existing per-stream call sites compile unchanged (the existing test suite is the compatibility proof and is NOT modified). The axum-todos example then rewires `run` onto the stepper with a 4-line custom `EveryEvent` trigger — the acceptance evidence for #327.

**Tech Stack:** Rust edition 2024 (pinned stable toolchain via `nix develop`), mnesis-store (`projection` feature), mnesis-inmemory (`InMemorySnapshotStore<S, P>` + `InMemoryAllPos` — both already position-generic), mnesis-fjall (`GlobalSeq` + `SnapshotStore<Vec<u8>, GlobalSeq>` on the `projections` partition — already shipped).

**Non-goals (decided, recorded here so nobody re-litigates):**
- NO scalar-view capability trait for `EveryNEvents` on `$all` positions. Additive later beats frozen-wrong; postgres's `PgAllPos` composite genuinely has no bucket arithmetic. `$all` pacing for 1.0 = `AfterEventTypes` or a custom trigger.
- NO position slot added to `Decoded<T>`. The `(P, Decoded<E>)` tuple asymmetry is intentional (`decoded.rs:84-85`) and `Positioned` embraces it.
- NO change to `Snapshotting` (`snapshot.rs`) — the trait default param keeps its `T: state::PersistTrigger` bound meaning `PersistTrigger<Version>` verbatim.

**Project rules that bind every task (from CLAUDE.md / memory):**
- Never commit to main; branch off freshly-fetched `origin/main`.
- The pre-commit hook runs `nix flake check` on every commit — do NOT pre-run the full gate by hand; DO run the targeted test commands given per step.
- `git add` new files BEFORE committing (flake check ignores untracked files).
- Run `nix develop -c cargo fmt --all` after substantial edits, before staging.
- Workspace must be clippy-clean under `--all-features --all-targets` (Task 6 verifies; the flake gate only covers `--lib`).
- All test commands run inside the dev shell: prefix `nix develop -c`. `cargo nextest run -p mnesis-store` covers the `projection`-gated integration tests via the self dev-dependency (`crates/store/Cargo.toml:79`) — no feature flags needed, do not toggle feature sets between runs.
- Conventional commits with scope. gh CLI must use the `joeldsouzax` account.

---

## File Structure

| File | Change |
|---|---|
| `crates/store/src/state.rs` | `PersistTrigger` → `PersistTrigger<P = Version>`; `AfterEventTypes` impl becomes generic over `P`; `EveryNEvents` impl unchanged (`P = Version`) |
| `crates/store/src/projection.rs` | Add sealed `Positioned` trait + 2 impls; `Projection` gains defaulted `Pos = Version` param; `advance` takes `impl Positioned`; `checkpoint`/`pending`/`flush`/`commit` retyped `Version` → `Pos`; docs get the `$all` assembly example |
| `crates/store/src/lib.rs` | Export `Positioned` from the `projection` re-export line |
| `crates/store/tests/projection_tests.rs` | Add: `AfterEventTypes` position-genericity test (`InMemoryAllPos`) |
| `crates/store/tests/projection_stepper_tests.rs` | Add: `$all` sequence / lifecycle / flush tests (`InMemoryAllPos` + local `Always` trigger). Existing per-stream tests untouched = source-compat proof |
| `examples/axum-todos/src/index.rs` | Delete hand-rolled fold/commit loop; `run(store, tx)` drives `Projection::load`/`advance`; add `EveryEvent` trigger; keep `hydrate` (spawn_app's seed + resume oracle); update tests |
| `examples/axum-todos/src/lib.rs` | `spawn_app`: call `index::run(projection_store, tx)` (drop `seed`/`checkpoint` args) |
| `examples/axum-todos/README.md` | Mark findings rows 1 & 2 resolved |
| `CLAUDE.md` | Update `state.rs` + `projection.rs` architecture bullets with the position-generic design and the EveryNEvents-stays-Version decision |

---

### Task 0: Branch setup

- [ ] **Step 1: Fetch and branch off origin/main**

```bash
git fetch origin
git switch -c feat/327-projection-stepper-all-position origin/main
```

Expected: `branch 'feat/327-projection-stepper-all-position' set up to track 'origin/main'` (or plain switch output). `git status` clean.

---

### Task 1: Generalize `PersistTrigger<P = Version>` (#328)

**Files:**
- Modify: `crates/store/src/state.rs:153-214` (trait + the two impls)
- Test: `crates/store/tests/projection_tests.rs` (append after the existing `AfterEventTypes` tests)

- [ ] **Step 1: Write the failing test**

Append to `crates/store/tests/projection_tests.rs` (top of file already has the lint-allow header; add the import next to the existing `use` block at ~line 16-22):

```rust
use mnesis_inmemory::InMemoryAllPos;
```

Append at the end of the file:

```rust
// ── PersistTrigger is generic over the position type (#328) ─────────
// AfterEventTypes never inspects positions, so it must accept an
// adapter's $all position, not only Version.

#[test]
fn after_event_types_fires_by_name_for_all_positions() {
    let trigger = AfterEventTypes::new(&["Closed"]);
    let old = InMemoryAllPos::new(3);
    let new = InMemoryAllPos::new(7).unwrap();
    assert!(trigger.should_persist(old, new, std::iter::once("Closed")));
    assert!(!trigger.should_persist(old, new, std::iter::once("Opened")));
}

#[test]
fn after_event_types_ignores_position_gaps() {
    // $all positions are monotonic but NOT gapless — the trigger must not care.
    let trigger = AfterEventTypes::new(&["Closed"]);
    let old = InMemoryAllPos::new(1);
    let new = InMemoryAllPos::new(1_000_000).unwrap();
    assert!(trigger.should_persist(old, new, std::iter::once("Closed")));
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
nix develop -c cargo nextest run -p mnesis-store --test projection_tests
```

Expected: COMPILE ERROR — `expected 'Version', found 'InMemoryAllPos'` (or trait-bound mismatch) on the new tests. That is the failure mode for a type-level feature: the test cannot pass until the trait is generic.

- [ ] **Step 3: Generalize the trait and the `AfterEventTypes` impl**

In `crates/store/src/state.rs`, replace the trait definition (lines 153-169):

```rust
/// Strategy for deciding when to persist state.
///
/// Used by both projection steppers (when to checkpoint projection state)
/// and snapshot decorators (when to snapshot aggregate state).
///
/// Generic over the position type `P` (default [`Version`]) for the same
/// reason [`SnapshotStore`] is: a per-stream caller paces on [`Version`],
/// an `$all` projection paces on the adapter's
/// [`AllPosition`](crate::AllPosition). Position-agnostic triggers
/// ([`AfterEventTypes`]) implement it for every `P`; arithmetic triggers
/// ([`EveryNEvents`]) only for [`Version`] — a composite `$all` position
/// (e.g. postgres `(txid, seq)`) deliberately has no bucket arithmetic, so
/// an `$all` pacer is a custom impl on the adapter's concrete position.
pub trait PersistTrigger<P = Version>: Send + Sync {
    /// Whether state should be persisted now.
    ///
    /// - `old_position`: position before the operation (`None` for first run)
    /// - `new_position`: position after the operation
    /// - `event_names`: names of events just processed
    fn should_persist(
        &self,
        old_position: Option<P>,
        new_position: P,
        event_names: impl Iterator<Item: AsRef<str>>,
    ) -> bool;
}
```

Leave `impl PersistTrigger for EveryNEvents` (lines 175-187) byte-for-byte unchanged — with the default param it now reads as `PersistTrigger<Version>`, which is the decision (Version-only, see Non-goals).

Replace `impl PersistTrigger for AfterEventTypes` (lines 205-214) with the generic impl:

```rust
impl<P> PersistTrigger<P> for AfterEventTypes {
    fn should_persist(
        &self,
        _old_position: Option<P>,
        _new_position: P,
        mut event_names: impl Iterator<Item: AsRef<str>>,
    ) -> bool {
        event_names.any(|name| self.types.iter().any(|t| *t == name.as_ref()))
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass (and nothing regressed)**

```bash
nix develop -c cargo nextest run -p mnesis-store --test projection_tests --test snapshot_tests --test snapshot_integration_tests --test projection_stepper_tests
```

Expected: PASS, including every pre-existing `EveryNEvents`/`AfterEventTypes`/`Snapshotting`/stepper test — the proof that the defaulted param is source-compatible (the `Snapshotting` bound `T: state::PersistTrigger` at `snapshot.rs:70` now elaborates to `PersistTrigger<Version>` with zero edits).

- [ ] **Step 5: Format and commit**

```bash
nix develop -c cargo fmt --all
git add crates/store/src/state.rs crates/store/tests/projection_tests.rs
git commit -m "feat(store): generalize PersistTrigger over the position type (#328)"
```

Expected: pre-commit hook runs `nix flake check` and passes; commit lands.

---

### Task 2: Sealed `Positioned` trait + `Projection<…, Pos = Version>` (#327)

**Files:**
- Modify: `crates/store/src/projection.rs` (imports ~line 1-7, struct docs+fields 56-113, impl block 115-256)
- Modify: `crates/store/src/lib.rs:167-168` (export `Positioned`)
- Test: `crates/store/tests/projection_stepper_tests.rs` (append `$all` section)

- [ ] **Step 1: Write the failing tests**

In `crates/store/tests/projection_stepper_tests.rs`, extend the imports inside `mod tests` (currently lines 15-22):

```rust
    use core::num::{NonZeroU32, NonZeroU64};

    use mnesis::{DomainEvent, Message, Version, version};

    use mnesis_inmemory::{InMemoryAllPos, InMemorySnapshotStore};
    use mnesis_store::decoded::Decoded;
    use mnesis_store::projection::{Projection, ProjectionError, Projector};
    use mnesis_store::state::{AfterEventTypes, EveryNEvents, Hydrated, PersistTrigger, SnapshotStore};
```

Append inside `mod tests`, after the existing tests (do NOT touch the existing per-stream tests — their unchanged compile is the source-compat proof):

```rust
    // ── $all duality (#327): the SAME stepper drives an $all projection ─────
    //
    // The $all position rides beside the item as `(P, Decoded<E>)` — exactly
    // what `.decoded()` yields on a subscribe_all stream — and feeds `advance`
    // whole. `InMemoryAllPos` stands in for fjall's `GlobalSeq`.

    /// `$all` trigger that always fires — the per-event-commit dual of
    /// `EveryNEvents(1)`, which is deliberately `Version`-only (#328).
    struct Always;
    impl PersistTrigger<InMemoryAllPos> for Always {
        fn should_persist(
            &self,
            _old_position: Option<InMemoryAllPos>,
            _new_position: InMemoryAllPos,
            _event_names: impl Iterator<Item: AsRef<str>>,
        ) -> bool {
            true
        }
    }

    fn all_pos(v: u64) -> InMemoryAllPos {
        InMemoryAllPos::new(v).expect("nonzero position")
    }

    fn all_store() -> InMemorySnapshotStore<CountState, InMemoryAllPos> {
        InMemorySnapshotStore::new()
    }

    // ── 1. Sequence/Protocol ($all): advance folds and checkpoints the tag ──

    #[tokio::test]
    async fn all_advance_accepts_tuple_items_and_checkpoints_the_all_tag() {
        let ss = all_store();
        let id = TestId("all");
        let (mut p, mut state) =
            Projection::load(id.clone(), CountingProjector, Always, &ss, NonZeroU32::MIN)
                .await
                .unwrap();
        assert_eq!(p.checkpoint(), None);

        // Gappy positions on purpose: $all is monotonic but NOT gapless
        // (aborted appends burn values) — the stepper checkpoints whatever
        // tag arrives. Inner versions are per-stream (two streams, both v1).
        state = p
            .advance(state, (all_pos(3), decoded(TestEvent::Added(10), 1)))
            .await
            .unwrap();
        state = p
            .advance(state, (all_pos(7), decoded(TestEvent::Added(20), 1)))
            .await
            .unwrap();

        assert_eq!(p.checkpoint(), Some(all_pos(7)));
        assert_eq!(
            state,
            CountState {
                count: 2,
                total: 30
            }
        );

        // Persisted together, atomically, under the $all position type.
        let (pos, st) = ss
            .hydrate(&id, NonZeroU32::MIN)
            .await
            .unwrap()
            .into_found()
            .unwrap();
        assert_eq!(pos, all_pos(7));
        assert_eq!(
            st,
            CountState {
                count: 2,
                total: 30
            }
        );
    }

    // ── 2. Lifecycle ($all): commit → reload → resume from the $all tag ─────

    #[tokio::test]
    async fn all_load_resumes_state_and_checkpoint_from_snapshot() {
        let ss = all_store();
        let id = TestId("all");
        {
            let (mut p, state) =
                Projection::load(id.clone(), CountingProjector, Always, &ss, NonZeroU32::MIN)
                    .await
                    .unwrap();
            let _ = p
                .advance(state, (all_pos(9), decoded(TestEvent::Added(10), 1)))
                .await
                .unwrap();
        }

        let (mut p2, state2) =
            Projection::load(id, CountingProjector, Always, &ss, NonZeroU32::MIN)
                .await
                .unwrap();
        assert_eq!(
            p2.checkpoint(),
            Some(all_pos(9)),
            "resume point is the $all tag, not a Version"
        );
        assert_eq!(
            state2,
            CountState {
                count: 1,
                total: 10
            }
        );

        // Resume folds onto the restored state at a later (gappy) position.
        let resumed = p2
            .advance(state2, (all_pos(12), decoded(TestEvent::Added(5), 2)))
            .await
            .unwrap();
        assert_eq!(p2.checkpoint(), Some(all_pos(12)));
        assert_eq!(
            resumed,
            CountState {
                count: 2,
                total: 15
            }
        );
    }

    // ── flush semantics ($all) + AfterEventTypes genericity in the stepper ──

    #[tokio::test]
    async fn all_flush_commits_tail_under_a_generic_after_event_types_trigger() {
        let ss = all_store();
        let id = TestId("all");
        // AfterEventTypes is position-generic (#328); "Removed" never arrives,
        // so only flush persists.
        let (mut p, mut state) = Projection::load(
            id.clone(),
            CountingProjector,
            AfterEventTypes::new(&["Removed"]),
            &ss,
            NonZeroU32::MIN,
        )
        .await
        .unwrap();

        state = p
            .advance(state, (all_pos(2), decoded(TestEvent::Added(10), 1)))
            .await
            .unwrap();
        assert_eq!(p.checkpoint(), None, "trigger must not have fired");
        assert_eq!(
            ss.hydrate(&id, NonZeroU32::MIN).await.unwrap(),
            Hydrated::Absent
        );

        p.flush(&state).await.unwrap();
        assert_eq!(p.checkpoint(), Some(all_pos(2)));
        let (pos, st) = ss
            .hydrate(&id, NonZeroU32::MIN)
            .await
            .unwrap()
            .into_found()
            .unwrap();
        assert_eq!(pos, all_pos(2));
        assert_eq!(
            st,
            CountState {
                count: 1,
                total: 10
            }
        );
    }
```

Notes for the implementer: (a) the defensive `Apply`-error path is NOT re-tested for `$all` — the monomorphized code path is identical and the per-stream test `advance_surfaces_projector_apply_error` is the canonical single location (CLAUDE rule 8: each invariant tested once); (b) no concurrency test — the stepper is `&mut self` single-threaded by design; (c) inference works without turbofish because `InMemorySnapshotStore<CountState, InMemoryAllPos>` pins `Pos` concretely.

- [ ] **Step 2: Run the tests to verify they fail**

```bash
nix develop -c cargo nextest run -p mnesis-store --test projection_stepper_tests
```

Expected: COMPILE ERROR — `advance` does not accept `(InMemoryAllPos, Decoded<TestEvent>)` and `SS: SnapshotStore<CountState, Version>` is unsatisfied by `InMemorySnapshotStore<CountState, InMemoryAllPos>`.

- [ ] **Step 3: Implement `Positioned` and generalize `Projection`**

In `crates/store/src/projection.rs`:

**(a)** Extend the imports (lines 1-7):

```rust
use core::iter;
use core::num::NonZeroU32;

use mnesis::{DomainEvent, Id, Version};

use crate::decoded::Decoded;
use crate::state::{Hydrated, PersistTrigger, SnapshotStore};
use crate::store::AllPosition;
```

**(b)** Insert after the `Projector` trait (after line 50), before the `Projection` section:

```rust
// ═══════════════════════════════════════════════════════════════════════════
// Positioned — the stepper's input contract over both stream item shapes
// ═══════════════════════════════════════════════════════════════════════════

mod sealed {
    pub trait Sealed {}
}

/// A decoded stream item carrying the position the stepper checkpoints at.
///
/// The two shapes a decoded subscription yields (the typed duals of
/// [`RawItem`](crate::decoded::RawItem)):
///
/// - [`Decoded<E>`] (per-stream) — the bookmark is the `version` *inside*
///   the box; `Pos = `[`Version`].
/// - `(P, Decoded<E>)` (`$all`) — the bookmark is the
///   [`AllPosition`](crate::AllPosition) tag riding *beside* the box,
///   exactly as `.decoded()` yields it; `Pos = P`.
///
/// Sealed on purpose: the pairing of position and event is **structural**.
/// A caller can never hand [`Projection::advance`] a position that did not
/// arrive with the event, so a committed checkpoint always describes the
/// state it is saved with — the same illegal-states-unrepresentable bet as
/// the atomic [`SnapshotStore::commit`].
pub trait Positioned: sealed::Sealed {
    /// The decoded event type carried by the item.
    type Event;
    /// The position type the stepper checkpoints at.
    type Pos: Copy + Send;
    /// Split the item into its bookmark and the decoded box.
    fn into_parts(self) -> (Self::Pos, Decoded<Self::Event>);
}

impl<E> sealed::Sealed for Decoded<E> {}
impl<E> Positioned for Decoded<E> {
    type Event = E;
    type Pos = Version;
    fn into_parts(self) -> (Version, Self) {
        (self.version, self)
    }
}

impl<E, P: AllPosition> sealed::Sealed for (P, Decoded<E>) {}
impl<E, P: AllPosition> Positioned for (P, Decoded<E>) {
    type Event = E;
    type Pos = P;
    fn into_parts(self) -> (P, Decoded<E>) {
        self
    }
}
```

**(c)** Retype the struct (fields at lines 98-113). Replace the struct declaration line and the two position fields; every other field and its doc comment stays:

```rust
pub struct Projection<I, P: Projector, Trig, SS, Pos = Version> {
    id: I,
    projector: P,
    trigger: Trig,
    snapshot_store: SS,
    schema_version: NonZeroU32,
    /// Last position durably committed together with the state.
    checkpoint: Option<Pos>,
    /// Folded-but-not-yet-persisted tail position, flushed on shutdown.
    pending: Option<Pos>,
    /// `Some(old_schema)` iff `load` discarded a snapshot under a different
    /// schema version — the projection is re-folding from scratch. Surfaced via
    /// [`rebuilding_from`](Projection::rebuilding_from) so a host can distinguish
    /// a costly schema-bump rebuild from an ordinary fresh start.
    rebuilt_from: Option<NonZeroU32>,
}
```

**(d)** Retype the impl block header (lines 115-121):

```rust
impl<I, P, Trig, SS, Pos> Projection<I, P, Trig, SS, Pos>
where
    I: Id,
    P: Projector,
    Trig: PersistTrigger<Pos>,
    SS: SnapshotStore<P::State, Pos>,
    Pos: Copy + Send,
{
```

**(e)** `load` — body unchanged; only the types generalize (no edit needed beyond the impl header: `checkpoint`/`position` are inferred as `Pos`).

**(f)** `checkpoint` accessor (lines 182-186) — retype and re-doc:

```rust
    /// The last durably-committed position — pass to `subscribe` (per-stream,
    /// `Pos = Version`) or `subscribe_all` (`Pos` = the adapter's
    /// [`AllPosition`](crate::AllPosition)) as the resume point. `None` means
    /// "from the beginning".
    pub const fn checkpoint(&self) -> Option<Pos> {
        self.checkpoint
    }
```

(If the compiler rejects `const` for copying a generic — it should not, implicit `Copy` is a compiler builtin, not a trait call — drop `const` and note it in the commit body. Do not add workarounds.)

**(g)** `advance` (lines 201-221) — replace signature and the position extraction; the trigger/commit body is unchanged:

```rust
    /// Fold one decoded event, then commit `(state, position)` together if the
    /// [`PersistTrigger`] fires. Returns the new state.
    ///
    /// Accepts either item shape a decoded stream yields (see [`Positioned`]):
    /// a bare [`Decoded<E>`] from a per-stream subscription (the position is
    /// its `version`), or the `(position, Decoded<E>)` tuple from an `$all`
    /// subscription — fed whole, no unpacking. The item's position becomes the
    /// candidate checkpoint. On a commit the checkpoint advances and the
    /// pending tail clears; otherwise the position is remembered as `pending`
    /// for the next [`flush`](Projection::flush).
    ///
    /// # Errors
    ///
    /// - [`ProjectionError::Apply`] if the projector rejects the event. The
    ///   consumed state is not recoverable (the fold owns it by value), so a
    ///   failed `advance` ends the projection — reload to resume.
    /// - [`ProjectionError::Commit`] if the snapshot commit fails.
    pub async fn advance<It>(
        &mut self,
        state: P::State,
        item: It,
    ) -> Result<P::State, ProjectionError<P::Error, SS::Error>>
    where
        It: Positioned<Event = P::Event, Pos = Pos>,
    {
        let (position, decoded) = item.into_parts();
        let folded = self
            .projector
            .apply(state, &decoded.event)
            .map_err(ProjectionError::Apply)?;

        if self
            .trigger
            .should_persist(self.checkpoint, position, iter::once(decoded.event.name()))
        {
            self.commit(position, &folded).await?;
        } else {
            self.pending = Some(position);
        }
        Ok(folded)
    }
```

**(h)** `flush` and the private `commit` (lines 232-255) — only the parameter type changes: `position: Version` → `position: Pos` in `commit`; `flush` body unchanged (its `self.pending` is already `Option<Pos>`).

**(i)** Struct docs — replace the single Assembly example (lines 80-94) with both shapes:

```rust
/// # Assembly (consumer-owned loop)
///
/// Per-stream (`Pos` defaults to [`Version`]):
/// ```ignore
/// let (mut proj, mut state) =
///     Projection::load(id, projector, trigger, &snapshots, schema).await?;
/// let stream = subscription
///     .subscribe(proj.id(), proj.checkpoint())?
///     .events()
///     .decoded(codec);
/// tokio::pin!(stream);
/// while let Some(item) = stream.next().await {
///     state = proj.advance(state, item?).await?;
/// }
/// proj.flush(&state).await?;
/// ```
///
/// `$all` (`Pos` = the adapter's [`AllPosition`](crate::AllPosition)) is the
/// **same loop** — the `(position, Decoded)` tuple `.decoded()` yields feeds
/// [`advance`] whole; only the subscribe call and the snapshot store's
/// position type differ:
/// ```ignore
/// let (mut proj, mut state) =
///     Projection::load(id, projector, trigger, &snapshots, schema).await?;
/// let stream = subscription
///     .subscribe_all(proj.checkpoint())?
///     .events()
///     .decoded(codec);
/// tokio::pin!(stream);
/// while let Some(item) = stream.next().await {
///     state = proj.advance(state, item?).await?;
/// }
/// proj.flush(&state).await?;
/// ```
```

**(j)** In `crates/store/src/lib.rs` line 168, export the new trait:

```rust
pub use projection::{Positioned, Projection, ProjectionError, Projector};
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
nix develop -c cargo nextest run -p mnesis-store
```

Expected: PASS — all new `$all` tests AND every pre-existing per-stream stepper test with zero edits to them (that is the compatibility claim, verified).

- [ ] **Step 5: Format and commit**

```bash
nix develop -c cargo fmt --all
git add crates/store/src/projection.rs crates/store/src/lib.rs crates/store/tests/projection_stepper_tests.rs
git commit -m "feat(store): generalize Projection stepper over \$all positions (#327)"
```

Expected: hook passes, commit lands.

---

### Task 3: Rewire `examples/axum-todos` onto the stepper (acceptance evidence)

**Files:**
- Modify: `examples/axum-todos/src/index.rs` (imports, delete loop body of `run`, add `EveryEvent`, update tests)
- Modify: `examples/axum-todos/src/lib.rs:85-94` (`spawn_app` wiring)

The example's `hydrate` **stays**: `spawn_app` needs `(seed, checkpoint)` synchronously to seed the watch channel and to populate the `resumed_from` lifecycle oracle before the loop task exists. `run` now performs its own authoritative `Projection::load` — one extra startup point-read, and the canonical stepper usage.

- [ ] **Step 1: Rewrite `index.rs` — trigger + `run`**

Replace the import block (lines 3-15) with:

```rust
use std::fmt;
use std::num::NonZeroU32;

use futures::StreamExt;
use mnesis_fjall::{FjallStore, GlobalSeq};
use mnesis_store::state::{CodecSnapshotStore, Hydrated, PersistTrigger, SnapshotStore};
use mnesis_store::store::Store;
use mnesis_store::{DecodedStreamExt, JsonCodec, Projection, Projector, StepStreamExt, Subscription};
use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use uuid::Uuid;

use crate::domain::TodoEvent;
```

Insert after the `INDEX_SCHEMA` const (line 149):

```rust
/// Commit `(state, position)` after every event.
///
/// The watch channel publishes per event, so the durable checkpoint tracks
/// the published state 1:1 — a restart never replays an event readers have
/// already observed. `EveryNEvents` is deliberately `Version`-only (bucket
/// arithmetic has no meaning on a composite `$all` position — #328), so an
/// `$all` per-event pacer is this four-line custom trigger.
struct EveryEvent;

impl PersistTrigger<GlobalSeq> for EveryEvent {
    fn should_persist(
        &self,
        _old_position: Option<GlobalSeq>,
        _new_position: GlobalSeq,
        _event_names: impl Iterator<Item: AsRef<str>>,
    ) -> bool {
        true
    }
}
```

Replace `run` (lines 171-214) — doc comment and body:

```rust
/// Fold the `$all` stream into the index forever, committing
/// `(state, position)` atomically per event and publishing each new state.
///
/// Driven by the position-generic [`Projection`] stepper (#327): `load`
/// hydrates `(state, checkpoint)` from fjall's `projections` partition, and
/// the `(GlobalSeq, Decoded)` tuple the subscription yields feeds
/// [`Projection::advance`] whole — no hand-rolled fold/commit loop. The
/// [`EveryEvent`] trigger commits every fold, so there is no pending tail
/// (a `flush` would be a no-op) and `send_replace` pays one clone per event
/// — the price of the deliberately no-`Clone` fold, at the seam where
/// another task must see the state.
///
/// Die-on-error contract: any `Err` (hydrate, register, read, decode, fold,
/// or commit) ends the loop and drops `tx`, so receivers observe
/// `changed() -> Err` as the death signal. A deterministic fold error (e.g.
/// [`IndexError::UnknownTodo`]) is a permanent crash-loop across restarts —
/// the committed checkpoint sits just before the poisoned event, so every
/// resume re-reads it; recovery is a rebuild (schema bump), never a retry.
/// The caller must seed the watch channel with the hydrated state (see
/// [`hydrate`]), or readers serve a stale default until the first event
/// arrives.
pub async fn run(store: Store<FjallStore>, tx: watch::Sender<TodosIndex>) -> Result<(), BoxErr> {
    let snapshots = CodecSnapshotStore::new(store.raw(), JsonCodec::default());
    let (mut proj, mut state) =
        Projection::load(IndexId, TodosProjector, EveryEvent, snapshots, INDEX_SCHEMA).await?;
    let stream = Subscription::new(&store)
        .subscribe_all(proj.checkpoint())?
        .events()
        .decoded(JsonCodec::default());
    tokio::pin!(stream);

    while let Some(item) = stream.next().await {
        state = proj.advance(state, item?).await?;
        tx.send_replace(state.clone());
    }
    Ok(())
}
```

Update `hydrate`'s doc comment (prepend one sentence to the existing comment at lines 158-162):

```rust
/// Load the persisted `(state, checkpoint)` pair, if any — `spawn_app`'s
/// synchronous peek for the watch-channel seed and the `resumed_from`
/// oracle; [`run`]'s own `Projection::load` re-reads the same snapshot as
/// its authoritative starting point (one extra startup point-read).
///
/// `Stale` (schema bump) folds from scratch, exactly like `Absent` — for this
/// consumer the two collapse; a host that must anticipate a costly rebuild
/// would branch here.
```

- [ ] **Step 2: Update `spawn_app` in `examples/axum-todos/src/lib.rs`**

Replace lines 85-94:

```rust
    let (seed, checkpoint) = index::hydrate(&store).await?;
    // The watch channel is seeded with the hydrated state, so reads are
    // served immediately after a reopen — no catch-up wait. The loop's own
    // `Projection::load` re-reads the same snapshot as its starting point.
    let (tx, rx) = watch::channel(seed);
    let projection_store = store.clone();
    let projection = tokio::spawn(async move {
        if let Err(error) = index::run(projection_store, tx).await {
            tracing::error!("projection loop stopped: {error}");
        }
    });
```

(`seed` is no longer moved into `run`, so `watch::channel(seed)` drops the `.clone()`.)

- [ ] **Step 3: Update the tests in `index.rs`**

`drive_until_contains` (lines 357-381) — drop the `checkpoint` parameter (run self-loads); keep `seed` (it seeds the channel, matching production wiring):

```rust
    /// Spawn [`run`] and await until the published index contains `id`;
    /// returns the receiver holding the folded state. `run` loads its own
    /// checkpoint from the store — the same path production takes.
    async fn drive_until_contains(
        store: &Store<FjallStore>,
        seed: TodosIndex,
        id: Uuid,
    ) -> watch::Receiver<TodosIndex> {
        let (tx, mut rx) = watch::channel(seed);
        let loop_store = store.clone();
        let task = tokio::spawn(async move {
            let _ = run(loop_store, tx).await;
        });
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if rx.borrow_and_update().contains(id) {
                    break;
                }
                rx.changed().await.expect("loop alive");
            }
        })
        .await
        .expect("loop folds the write");
        task.abort();
        let _ = task.await;
        rx
    }
```

In `loop_folds_writes_and_reopen_resumes_from_checkpoint`, update the two call sites:
- line 401: `drive_until_contains(&store, seed, first_id).await;` (drop `checkpoint` — the preceding `hydrate` assertions at 394-396 stay: they still verify the fresh store finds nothing)
- line 426: `let rx = drive_until_contains(&store, seed, second_id).await;` (drop `checkpoint`)

Every assertion stays byte-for-byte — including `checkpoint == GlobalSeq::new(1)` after reopen (line 411-417) and the exact-ids resume assertion (line 428-439). The resume proof is now MORE end-to-end: `run` reads its checkpoint from disk instead of being handed it.

- [ ] **Step 4: Run the example's tests**

```bash
nix develop -c cargo nextest run -p mnesis-example-axum-todos
```

Expected: PASS — all unit fold tests plus the lifecycle test (fold → reopen → resume, no re-fold of event 1).

- [ ] **Step 5: Update the README findings table**

In `examples/axum-todos/README.md` lines 84-85, replace the two rows:

```markdown
| 1 | The `Projection` stepper cannot drive an `$all` projection — its `SnapshotStore` bound and checkpoint are `Version`-typed (per-stream); the example hand-rolled the loop in `index.rs`. **Resolved:** the stepper is generic over the position (`Pos = Version` default), and `index.rs::run` now drives it | #327 |
| 2 | `PersistTrigger` is `Version`-typed and `Decoded<T>` has no position slot — the `$all` position rides in a tuple beside the item; no shipped trigger could accept it. **Resolved:** the trigger is `PersistTrigger<P = Version>`; `advance` takes the `(position, Decoded)` tuple whole. `EveryNEvents` stays `Version`-only by decision (no bucket arithmetic on composite positions) — the example's per-event pacer is a 4-line custom trigger | #328 |
```

- [ ] **Step 6: Format and commit**

```bash
nix develop -c cargo fmt --all
git add examples/axum-todos/src/index.rs examples/axum-todos/src/lib.rs examples/axum-todos/README.md
git commit -m "feat(examples): drive the axum-todos \$all index with the generalized stepper (#327)"
```

Expected: hook passes, commit lands.

---

### Task 4: CLAUDE.md architecture notes

**Files:**
- Modify: `CLAUDE.md` (the `state.rs` and `projection.rs` bullets in the mnesis-store section)

Capture the WHY so the decision survives (memory rule: design intent lives in CLAUDE.md).

- [ ] **Step 1: Update the `state.rs` bullet**

Find the sentence in the `state.rs` bullet: `` `PersistTrigger` trait: `EveryNEvents(N)` (bucket-crossing), `AfterEventTypes(&[&str])` (semantic). Used by both projection runners and the snapshot decorator. `` and replace with:

```markdown
`PersistTrigger<P = Version>` trait (position-generic since #328, defaulted so `Snapshotting`'s `T: PersistTrigger` bound and every existing impl compile unchanged): `EveryNEvents(N)` (bucket-crossing, **`Version`-only by decision** — bucket arithmetic has no meaning on a composite `$all` position like postgres `(txid, seq)`, and no scalar-view capability trait ships at 1.0: additive later beats frozen-wrong; an `$all` pacer is `AfterEventTypes` or a custom impl on the adapter's concrete position), `AfterEventTypes(&[&str])` (semantic, implemented for every `P` — it never reads positions). Used by both projection steppers and the snapshot decorator.
```

- [ ] **Step 2: Update the `projection.rs` bullet**

In the `projection.rs` bullet, find the sentence beginning `` `Projection<I, P, Trig, SS>` (#255) — an inert per-event **stepper** `` and change the type to `Projection<I, P, Trig, SS, Pos = Version>`, then append to the end of that bullet:

```markdown
**Position-generic since #327** (the axum-todos port proved the first real `$all` read model had to hand-roll the loop #255 deleted): `checkpoint`/`pending` are `Option<Pos>`, the impl is bounded `SS: SnapshotStore<P::State, Pos>` + `Trig: PersistTrigger<Pos>`, and `advance` takes any sealed **`Positioned`** item — `Decoded<E>` (per-stream, position = its `version`) or the `(AllPosition, Decoded<E>)` tuple exactly as an `$all` `.decoded()` stream yields it — so the position/event pairing is structural and a committed checkpoint can never describe a state it didn't arrive with (same unrepresentability bet as `commit_persisted` #212). Per-stream call sites compile unchanged via the `Pos = Version` default; the pre-#327 per-stream test suite passing unmodified is the recorded compatibility proof.
```

- [ ] **Step 3: Commit**

```bash
git add CLAUDE.md
git commit -m "docs: record the position-generic stepper/trigger design (#327) (#328)"
```

Expected: hook passes, commit lands.

---

### Task 5: Workspace-wide lint sweep

- [ ] **Step 1: Clippy under the full matrix (the flake gate only covers --lib)**

```bash
nix develop -c cargo clippy --workspace --all-features --all-targets
```

Expected: zero warnings. If any lint fires in non-test code, FIX it — never allow/suppress (memory: strict clippy is the project default; lint-driven fixes are not deviations). Commit any fixes as `fix(store): <lint> in <place>` or fold into a `style:` commit.

- [ ] **Step 2: Doc build sanity (new intra-doc links in projection.rs/state.rs)**

```bash
nix develop -c cargo doc -p mnesis-store --no-deps
```

Expected: no `broken_intra_doc_links` warnings for `Positioned`/`AllPosition`/`SnapshotStore::commit` links.

---

### Task 6: PR

- [ ] **Step 1: Confirm gh account and push**

```bash
gh auth status
git push -u origin feat/327-projection-stepper-all-position
```

Expected: `joeldsouzax` is the active account for this repo (switch with `gh auth switch -u joeldsouzax` if not); push succeeds.

- [ ] **Step 2: Open the PR**

```bash
gh pr create \
  --title "feat(store): generalize Projection stepper + PersistTrigger over \$all positions" \
  --body "$(cat <<'EOF'
Closes #327. Closes #328.

Three source-compatible generalizations, each mirroring a pattern the codebase already committed to (`SnapshotStore<S, P>`, `RawItem`, adapter-defined positions):

- **`PersistTrigger<P = Version>`** — defaulted trait param: `Snapshotting`'s bound and every existing impl compile unchanged. `AfterEventTypes` is now position-generic (it never reads positions). **`EveryNEvents` stays `Version`-only by decision**: bucket arithmetic has no meaning on a composite `$all` position (postgres `(txid, seq)`), and no scalar-view capability trait ships at 1.0 — additive later beats frozen-wrong.
- **Sealed `Positioned` item trait** — `advance` accepts `Decoded<E>` (per-stream, position = its `version`) or `(AllPosition, Decoded<E>)` exactly as an `$all` `.decoded()` stream yields it. The position/event pairing is structural: a checkpoint can never be committed against a state it didn't arrive with.
- **`Projection<I, P, Trig, SS, Pos = Version>`** — `checkpoint`/`pending` are `Option<Pos>`; the `$all` and per-stream assemblies are the same three-line loop.

**Acceptance evidence:** `examples/axum-todos/src/index.rs::run` — the hand-rolled `$all` fold/commit loop (#327's Evidence section) is deleted; the example now drives the stepper with a 4-line custom per-event trigger, and its lifecycle test proves resume-from-checkpoint through the stepper's own `load`.

**Compatibility proof:** the pre-existing per-stream stepper/trigger/snapshot test suites pass with zero edits.

New tests: `$all` sequence + lifecycle + flush over `InMemoryAllPos` (gappy positions on purpose — `$all` is monotonic-not-gapless), and `AfterEventTypes` position-genericity.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

Expected: PR URL printed. CI ("Nix Flake Check") must pass before squash-merge (`gh pr merge --squash --delete-branch`, only after review).

---

## Self-Review (completed at plan time)

- **Spec coverage:** #327 stepper generalization → Task 2; #328 trigger + tuple-item acceptance → Tasks 1–2; evidence loop deletion → Task 3; freeze-record of the decision → Task 4 + PR body. The "possible direction" from #327 (caller-supplied position) was upgraded to the sealed-trait variant per the design discussion (unforgeable pairing) — recorded in Task 2's docs.
- **Type consistency:** `Positioned::{Event, Pos}` names match between trait def (Task 2 Step 3b), `advance` bound (3g), and all test call sites; `PersistTrigger<P>`'s `should_persist(old_position, new_position, event_names)` matches the `Always`/`EveryEvent` impls in Tasks 2–3.
- **Placeholder scan:** none — every code step carries the full code; every command carries its expected outcome.
