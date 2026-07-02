# `execute` — one call for decide-then-save

**Issue:** #251
**Status:** design — research is settled, we build the combinator
**Date:** 2026-07-02

---

## The bug hiding in plain sight

Here is what a developer writes for every single command:

```rust
let account = repo.load(id).await?;
let decided  = account.handle(OpenAccount { owner })?;   // decide
repo.save(&mut account, &decided).await?;                // save — pass the SAME events, and don't forget
```

Look at the middle two lines. That is a hand-off. The developer decides events in one line, then carries them by hand into the next. Any hand-off can be dropped.

Two ways to drop this one:

1. **Forget the save.** The events were decided and then thrown away. No error. No trace. The write just vanishes.
2. **Save the wrong events.** Pass a stale `decided` from an earlier call, or hand-roll events the aggregate never agreed to. The store now holds a lie.

This is not hypothetical. The repetition is why `examples/fjall-end-to-end` grew a `seed_account` helper — a band-aid over a design gap (#227).

A hand-off the compiler does not check is a defect waiting for a bad day. Remove the hand-off.

## Is a combinator the right shape? Yes. Here is the proof, not the opinion.

The card demanded evidence, not taste (CLAUDE rule 0). The evidence is one-sided.

**The Decider pattern already answers this.** Chassaing's decider is three pure functions *plus one runner* — the runner reads events, rebuilds state, runs decide, and appends. The stitching is one sanctioned function. Nobody assembles it by hand per command. ([thinkbeforecoding](https://thinkbeforecoding.com/post/2021/12/17/functional-event-sourcing-decider))

**Functional Core, Imperative Shell says the same thing.** The pure core decides. The shell does the IO around it, and the core never knows the shell exists. The shell is written *once*. ([Chassaing, Functional Core Part 2](https://thinkbeforecoding.com/post/2018/02/01/functional-core-2))

Now the punchline. Our hand-written `handle()` then `save()` **is that shell — leaked into every call site.** FCIS exists to stop exactly this leak. So the two-step is not the principled form. It is the anti-pattern the theory was built to prevent.

The frameworks agree, but they are only witnesses, not authority: cqrs-es `execute`, Eventuous `CommandService`, Axon's unit-of-work — all fuse the call, all keep the decision pure. We do not need their vote.

**We already decided this. Twice.**

- `commit_persisted` (#212) fused `advance_version` and `apply_events`, then made the halves private. Reason given, in our own words: two steps a human threads is a desync bug, and the fix makes it *unrepresentable by construction*.
- `react_and_save` fused the saga shell already.

`handle` + `save` is the same bug, one floor up. Fixing it is consistency with ourselves. Leaving it is the thing that needs an excuse.

## Why not the clever half-measure

The card floated a second idea: keep two steps, but make `save` swallow the events as a move-only token so you cannot pass the wrong ones.

It is weaker, and here is precisely why. Rust gives you *affine* types — a value is used at most once. It does not give you *linear* types — used exactly once. A move stops you passing a token twice. It does not force you to pass it at all. `#[must_use]` only warns on a dropped token; the program still compiles and still loses the write.

So the token makes the bug *warned against*. Fusion makes it *impossible*. We do not settle for warned-against. Fuse it.

And fusion costs us nothing. `execute` calls `handle` inside itself, so the pure decision stays public and stays testable. The explicit two-step survives as the escape hatch for the rare case where you must see the events before they land. Default is safe. The sharp tool is still in the drawer. That is the same split as `commit_persisted` versus the private advance/apply.

## The design

One combinator. It mirrors `SagaRepository` down to the seams.

```rust
/// Decide-then-save as one transaction. Extends `Repository<A>`; every
/// repository gets it free (bare `EventStore` and the `Snapshotting` decorator).
pub trait CommandRepository<A: Aggregate>: Repository<A> {
    fn execute<C, const N: usize>(
        &self,
        root: &mut AggregateRoot<A>,
        command: C,
    ) -> impl Future<Output = Result<Events<EventOf<A>, N>, ExecuteError<A::Error, Self::Error>>> + Send
    where
        A: Handle<C, N>;
}

impl<A: Aggregate, R: Repository<A>> CommandRepository<A> for R {}
```

The whole body:

```rust
async move {
    let decided = root.handle::<C, N>(command).map_err(ExecuteError::Decide)?;
    self.save(root, &decided).await.map_err(ExecuteError::Store)?;
    Ok(decided)
}
```

Three lines. It invents nothing. `root.handle` is the pure decision that already exists. `self.save` already appends atomically and advances the root through `commit_persisted`. `execute` is the shell and only the shell.

**It returns the decided events.** That keeps the one good thing the two-step had — you can inspect what was decided — while killing the bad thing: the caller never *supplies* events, so the caller cannot supply the wrong ones. The new version is already on `root.version()`, so no wrapper struct. Return the bare `Events`. (The saga side needs a `Reaction` wrapper only because it mints version-pinned intent tokens. Commands owe nobody that. Do not copy ceremony you do not need.)

**Two errors, two domains. Never blur them (rule 3).**

```rust
#[derive(Debug, thiserror::Error)]
#[non_exhaustive] // public error enum — the freeze carve-out (#209)
pub enum ExecuteError<DecideErr, StoreErr> {
    /// The aggregate refused the command. Nothing was written.
    #[error("command rejected: {0}")]
    Decide(#[source] DecideErr),

    /// The save failed — adapter, codec, conflict, overflow.
    #[error(transparent)]
    Store(StoreErr),
}

impl<DecideErr, StoreErr: ConflictPredicate> ExecuteError<DecideErr, StoreErr> {
    #[must_use]
    pub fn is_conflict(&self) -> bool {
        matches!(self, Self::Store(e) if e.is_conflict())
    }
}
```

No `VersionOverflow` variant. `save` already reports overflow inside its `Store` error. Adding it here would split one domain across two variants — the exact smell rule 3 forbids. (`react_and_save` needs it only because it does its own version math after the save. We do none.)

`ConflictPredicate` already exists, sealed, in `saga.rs`. It moves to shared ground so `SagaError` and `ExecuteError` delegate to the one `StoreError::is_conflict`. One truth, two callers.

## Conflict: hand it back, do not retry

On a version conflict, `execute` returns `Err(ExecuteError::Store(..))` and `is_conflict()` is `true`. It stops there.

The textbook decider retries — re-read, re-decide, re-save. We do not, on purpose. Internal retry assumes a single writer, and that assumption belongs to the runtime (Agency), not to a storage kernel (rule 5). `react_and_save` already draws this line. `execute` draws it in the same place. The caller reloads and retries if the caller knows it should.

## Scope — and the discipline to stop there

- **Only** `execute(&mut root, cmd)`. No load-and-execute combo. The gap in the card is the decide→save hand-off. `load` is already its own line, and keeping it separate protects the load-once-decide-many flow and the fixture. A load+execute fusion is a *different* card if anyone ever wants it. YAGNI.
- `handle` and `save` do not change. `commit_persisted` does not change. The fixture does not change.
- No new persistence machinery. If this design adds a transaction, a partition, or a byte to the wire, it is wrong.

## Placement

New file `crates/nexus-store/src/execute.rs`, shaped like `saga.rs`: the trait, `ExecuteError`, the blanket impl, the tests. Wire `mod execute;` into `lib.rs` and re-export like the saga module. Move `ConflictPredicate` and its `sealed` module out of `saga.rs` to shared ground; `saga.rs` imports it back. Sagas do not change behavior.

## Tests — the four categories, first (rule 7)

1. **Sequence.** `load → execute → execute → …` down a chain of commands. Assert the returned events and `root.version()` step correctly. Reload from scratch and assert identical state.
2. **Lifecycle.** `execute` against `FjallStore`, close, reopen, load, prove the events are durable. Then prove a rejected command wrote *nothing*: reopen and the stream is untouched.
3. **Defensive boundary.** A command the aggregate refuses → `ExecuteError::Decide`, store untouched. A stale root, behind the store → `ExecuteError::Store`, `is_conflict()` true.
4. **Linearizability.** Two `execute` calls race on one id (spawn + barrier). One wins. The loser gets `is_conflict()`. The final stream holds only the winner's events.

Then the equivalence test that earns the whole design: `execute` must produce byte-identical persisted state to the manual `handle` + `save`. If it does not, it is not a pure shell and the design is broken. And `is_conflict()`: `false` for `Decide`, `true` only for a conflicting `Store`.

## Dogfood it — close #227 in the same stroke

`examples/fjall-end-to-end/src/lib.rs` threads the two-step in `seed_account` and three more places. Collapse every

```rust
let decided = x.handle(cmd)?;
repo.save(&mut x, &decided).await?;
```

into

```rust
repo.execute(&mut x, cmd).await?;
```

That proves the combinator on the real adapter and deletes the repetition that raised #227. Update the example prose to show `execute` as *the* way to run a command.

## What this does not do

- Does not retry. That is the runtime's job.
- Does not fuse `load`. Different concern.
- Does not touch `handle`, `save`, `commit_persisted`, or the fixture.
- Does not add one byte of persistence machinery. It reuses `save` whole.
