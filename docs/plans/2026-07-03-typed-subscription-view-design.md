# Typed Subscription View — Design (#249)

**Status:** Design approved (brainstorming), pending implementation plan.
**Card:** #249 `[freeze]` — Subscriptions force hand-decoding: no ergonomic typed view that reuses the configured codec.
**Milestone:** 1 — Nexus: Pre-Freeze (1.0 blockers).
**Surfaced by:** #227 (end-to-end fjall example's hand-rolled `fold_balance`).

---

## 1. Problem

`Subscription` (and the raw `read_stream`/`read_all`) yield **raw `PersistedEnvelope`s**. A
consumer that already configured a codec on its repository cannot reuse it on the feed — it
re-implements decoding by hand, once per project:

```rust
fn fold_balance(balance: u64, env: &PersistedEnvelope) -> Result<u64, BoxErr> {
    let event: AccountEvent = serde_json::from_slice(env.payload())?;  // hand-decode
    …
}
```

Two decode paths (configured codec vs. this hand-rolled one) and a helper to hold the wart.

## 2. Root cause & the contract we keep

`Subscription` yields raw bytes **by design** — the distributed multi-consumer model has each
consumer hold its own codec (memory: `project_subscription_stays_raw`). That contract is **kept**.
The wart is that there is **no ergonomic typed layer on top**, so codec reuse is not the obvious
one-liner. We add a consumer-side layer; we do **not** make the core subscription typed.

## 3. Locked decisions (from brainstorming)

1. **Yield the event *and* its bookmark.** A subscription exists to be resumable; dropping the
   version/position would make the typed item un-checkpointable. Bare-`E` is rejected.
2. **A labeled box, not a bare tuple.** Named parts (`event`, `version`, `metadata`) read clearly
   and give the two feeds (`version` vs `$all` position) a consistent home. Rejected: bare pair.
3. **A snap-on stream adapter, not a new `subscribe_*` method.** One tool works on the live feed,
   the `$all` feed, and plain `read_stream`/`read_all`; it leaves `subscribe` untouched. This is
   the idiomatic Rust stream-adapter shape. Rejected: `subscribe_decoded(...)` per-case buttons.
4. **Two entry points — owning stream sugar *and* a borrowing fold — because subscriptions fold,
   they don't carry away.** See §5–§6.

## 4. Prior art (rule 0 — cited, no assumptions)

- **Akka Persistence `EventEnvelope`** carries `event`, `sequenceNr`, `offset`, `timestamp` — a
  typed envelope pairing the decoded event with its resume position. Direct precedent for
  `Decoded<T>` = `event` + `version`/position + `metadata`.
  <https://doc.akka.io/api/akka/current/akka/persistence/query/EventEnvelope.html>
- **EventStoreDB catch-up subscriptions** expose a typed/deserialized `ResolvedEvent` view layered
  over the raw stream, with the raw bytes still reachable — the "raw underneath, typed on top" shape.
  <https://developers.eventstore.com/clients/grpc/subscriptions.html>
- **serde `DeserializeOwned: for<'de> Deserialize<'de>`** — "owned vs borrowed is a *bound*, not a
  separate type." Precedent for expressing the owning-codec constraint as
  `for<'a> Decode::Output<'a> = E` rather than a second adapter type.
  <https://docs.rs/serde/latest/serde/de/trait.DeserializeOwned.html>
- **Internal iteration as the answer to the lending problem.** `futures::Stream::Item` has **no
  lifetime** (`type Item;`), so a stream cannot yield an item that borrows from the poll. Handing a
  closure to a `for_each`-style combinator — *internal* iteration — lets a borrowed value live only
  for the call and never escape, sidestepping the lending-stream requirement entirely.
  futures-core `Stream`: <https://docs.rs/futures-core/latest/futures_core/stream/trait.Stream.html>;
  GATs / lending-iterator limitation: <https://blog.rust-lang.org/2022/10/28/gats-stabilization/>.
- **Internal evidence against a lending stream.** This codebase already shipped a GAT-lending
  `EventStream` layer and **deleted ~1370 lines** of it once the owned-`Bytes` envelope removed the
  lifetime cliff (`CLAUDE.md`, `nexus-store/src/stream.rs` entry). Re-introducing a lending stream
  for the typed view would re-open exactly that cliff and forfeit `StreamExt`/`.next().await`.

## 5. The zero-copy boundary (why two entry points)

A `futures::Stream` item must **own its data or be `'static`** — it cannot hand out a borrowed
window (`Decode::Output<'a> = &'a Archived<E>`) because the window borrows the per-item envelope,
a temporary inside the poll. Therefore:

- **Owning codecs** (JSON, bincode, postcard — `Output<'a> = E`) can be a **stream of carry-away
  items**.
- **Zero-copy codecs** (rkyv, bytemuck — `Output<'a> = &'a Archived<E>` / `&'a E`) cannot be a
  stream of items — but a subscription **folds** state (e.g. KERI's KEL projection reads each event,
  updates running state, moves on; it never stashes the decoded event). Peek-and-use-in-place is
  exactly what zero-copy is for. So zero-copy is served by an **internal-iteration fold** that hands
  the borrowed window to a closure, valid only for that call.

This is a **first-class, stated use case**: KERI (built on nexus, memory: `project_agency_layering`)
will use rkyv on subscriptions. The design serves it directly, not as an afterthought.

## 6. API surface

New module `crates/nexus-store/src/decoded.rs`, re-exported at the crate root. It depends only on
`Decode` + `PersistedEnvelope` + `futures::Stream` — **not** subscription-gated, so it also sugars
plain `read_stream`/`read_all`.

### 6.1 The `Decoded<T>` box (one type, generic over payload)

```rust
/// A raw envelope, un-packed: the decoded event plus its resume bookmark and metadata.
/// `T` is the owned event (`E`) on the stream path, or the borrowed window
/// (`Decode::Output<'a>`) inside the fold closure.
pub struct Decoded<T> {
    pub event: T,
    pub version: Version,
    pub metadata: Option<Bytes>,   // Bytes is cheap-owned (Arc), no lifetime — works both ways
}
```

Mirrors a `PersistedEnvelope` field-for-field, minus the raw payload bytes. One type serves both
paths: `Decoded<E>` (owned) on the stream, `Decoded<C::Output<'a>>` (window) in the closure.

### 6.2 Owning stream sugar — `.decoded(codec)`

```rust
// bound: for<'a> C: Decode<E, Output<'a> = E>   → owning codecs only, enforced by the type system
let typed = subscription.subscribe(&id, None)?.decoded(codec);
// Stream<Item = Result<Decoded<E>, DecodeStreamError<ReadErr, CodecErr>>>
```

A zero-copy codec **cannot satisfy** `Output<'a> = E`, so the compiler steers rkyv users to
`.for_each_decoded()` — the boundary is a compile error, not a runtime check or a footgun.

### 6.3 Borrowing fold — `.for_each_decoded(codec, f)`

```rust
subscription.subscribe(&id, None)?
    .for_each_decoded(codec, |d: Decoded<&Archived<KelEvent>>| {   // window; never escapes
        kel_state.apply(d.event);
        checkpoint = d.version;
        Ok(())     // Result<(), HandlerErr>
    }).await?;
```

Works for **owning *and* zero-copy** codecs (no `Output<'a> = E` bound). Plain `futures::Stream`
underneath — no lending stream, no `yoke`/`ouroboros`.

### 6.4 Per-stream vs `$all` — one method, bookmark rides where it already does

A tiny internal item-shape trait (`DecodableItem`, `pub(crate)`, sealed) lets one `.decoded()` /
`.for_each_decoded()` cover both raw shapes, preserving the `$all` position tag beside the box:

| feed | raw item | typed item |
|---|---|---|
| `subscribe` / `read_stream` | `PersistedEnvelope` | `Decoded<E>` |
| `subscribe_all` / `read_all` | `(AllPosition, PersistedEnvelope)` | `(AllPosition, Decoded<E>)` |

Intentional asymmetry (rule 4): per-stream's bookmark is `version` *inside* the box; `$all`'s is the
tag *beside* it — because that is exactly where each lives in the raw layer. Documented on the trait.

### 6.5 Error domains (rule 3 — distinct variants, `thiserror`)

```rust
#[derive(thiserror::Error, Debug)]
pub enum DecodeStreamError<R, D> {          // stream path
    #[error("stream read failed")]  Read(#[source] R),
    #[error("event decode failed")] Decode(#[source] D),
}

#[derive(thiserror::Error, Debug)]
pub enum FoldDecodedError<R, D, H> {        // fold path adds the handler domain
    #[error("stream read failed")]   Read(#[source] R),
    #[error("event decode failed")]  Decode(#[source] D),
    #[error("handler failed")]       Handler(#[source] H),
}
```

Read vs decode vs handler are three separate failure domains and never share a variant. At the 1.0
freeze both take `#[non_exhaustive]` per the standing public-error-enum carve-out (memory:
`feedback_no_non_exhaustive`).

## 7. Testing (4 mandatory categories first — memory: `feedback_exhaustive_testing`)

1. **Sequence/protocol.** Catch-up then live over `.decoded()`: N seeded events fold to the expected
   value, then a live append is observed and folded; `.for_each_decoded()` over the same sequence
   yields the identical running state. Assert exact final state and per-step versions.
2. **Lifecycle.** Corrupt a persisted payload, then decode it through the view → surfaces the
   `Decode` variant (never a panic, never mis-tagged as `Read`). Write-close-reopen a fjall store,
   then `.decoded()` resumes from a checkpoint at the correct version.
3. **Defensive boundary.** Feed a payload the codec rejects → `Decode`. Inject a read-error item
   (adapter `Err`) → `Read`. Return `Err` from the fold closure → `Handler`. Each maps to exactly
   its variant, asserted via `matches!` on the specific arm.
4. **Linearizability/isolation.** Concurrent writer appends while a `.decoded()` feed folds
   (`tokio::spawn` + `Barrier` for real overlap): no lost/duplicated events, bookmark strictly
   monotonic, snapshot-consistent count.

Then the standard methodologies (property tests over payload/boundary sizes, etc.).

## 8. Acceptance

- A consumer reads a subscription as typed events by reusing the configured codec, with **no**
  hand-written `from_slice`/`access::<…>()`.
- The raw stream remains available unchanged.
- The #227 `fold_balance` helper is deleted; the example uses `.decoded()` (owning JSON path).
- Zero-copy (rkyv) subscription folding is expressible via `.for_each_decoded()` without a lending
  stream.

## 9. File structure

| File | Change |
|---|---|
| `crates/nexus-store/src/decoded.rs` | **New.** `Decoded<T>`, `DecodedStreamExt` (`.decoded`, `.for_each_decoded`), `DecodableItem` (sealed `pub(crate)`), `DecodeStreamError`, `FoldDecodedError`. |
| `crates/nexus-store/src/lib.rs` | `mod decoded;` + `pub use` of the public items; module docs. |
| `examples/fjall-end-to-end/src/lib.rs` | Delete `fold_balance`; use `.decoded()`. |
| `crates/nexus-store/tests/decoded_view_tests.rs` | **New.** The 4-category suite. |
| `CLAUDE.md` | Document `decoded.rs` in the store-crate map. |

## 10. Open questions for the implementation plan

- Exact trait/method names (`DecodedStreamExt` / `.decoded` / `.for_each_decoded` vs `.typed` /
  `.fold_decoded`) — provisional, revisit at review.
- Whether `.for_each_decoded` should also offer a `try_fold_decoded(init, f) -> Acc` accumulator
  form (returns the folded value) in addition to the unit `for_each` form. Likely yes; confirm scope.
- `ControlFlow`/early-break support for the fold on an infinite live tail (checkpoint-and-stop).
