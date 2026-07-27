# #345 — Attributed projector fold: `Projector::apply_attributed`

**Issue:** [#345](https://github.com/joeldsouzax/nexus/issues/345) — `Projector::apply`
can't see the `$all` `StreamKey`; multi-stream folds smuggle attribution via a shim.
**Decision:** Option 1 from the issue, shaped as a defaulted second trait method.
**Milestone:** 1 — Pre-Freeze (1.0 blockers).

## Problem

#333 made stream attribution a store guarantee: every `$all` read/subscription item is
`(AllPosition, StreamKey, PersistedEnvelope)`, and `.decoded()` preserves the tag as
`(P, StreamKey, Decoded<E>)`. But `Projector::apply(state, &event)` has no channel for
the key, so the stepper's `Positioned::into_parts` drops it, and a multi-stream fold
whose events do not embed their aggregate id cannot attribute events to streams.

Proof of pain: `examples/signed-events` (`projection.rs`) — a KERI-shaped `Set` event
deliberately carries no register id (the stream *is* the identity), so `RegisterView`
carries a `route_to(id)` mutable shim the driver must call before every `apply`, plus a
`NoRoute` error variant for when it forgets. The attribution the store already
guarantees is smuggled out-of-band, defeating #333's rationale ("a store guarantee, not
a payload convention").

`examples/axum-todos` dodged the problem only because its payload embeds a todo uid.
KERI projections (the flagship IoT/mobile consumer) cannot dodge it.

## Design

### 1. `Projector` gains a defaulted attributed entry point

```rust
pub trait Projector: Send + Sync + 'static {
    // ... existing items unchanged ...

    /// Apply one event with its origin-stream attribution, when the item
    /// carries one.
    ///
    /// `key` is `Some` iff the event arrived off an `$all` read — the origin
    /// `StreamKey` the store stamps beside every `$all` item (#333). On a
    /// per-stream fold it is `None`: there the stream id is the query
    /// argument the caller already holds, and the item carries no tag.
    ///
    /// The default ignores the key and delegates to [`apply`](Self::apply),
    /// so a single-stream projector implements only `apply`. A multi-stream
    /// projector that routes by origin stream overrides this method instead.
    fn apply_attributed(
        &self,
        state: Self::State,
        key: Option<&StreamKey>,
        event: &Self::Event,
    ) -> Result<Self::State, Self::Error> {
        let _ = key;
        self.apply(state, event)
    }
}
```

- **Additive**: existing `Projector` impls compile unchanged; post-1.0 a defaulted
  method addition stays a minor change (this is the standard trait-evolution seam —
  `Hasher::write_u8`, tokio `poll_write_vectored`).
- **`Option<&StreamKey>` is honest** (rule 3): a per-stream item genuinely carries no
  attribution tag; `None` is "no tag on this item", not a sentinel.
- The default body is the dispatch seam, not ceremony: the stepper calls **only**
  `apply_attributed`; plain projectors fall through to `apply` (inlined, zero cost).

### 2. `Positioned` stops dropping the key

```rust
pub trait Positioned: sealed::Sealed {
    type Event;
    type Pos: Copy + Send;
    /// Split the item into its bookmark, its origin-stream attribution
    /// (`$all` items only), and the decoded box.
    fn into_parts(self) -> (Self::Pos, Option<StreamKey>, Decoded<Self::Event>);
}

impl<E> Positioned for Decoded<E> {
    // (self.version, None, self)
}

impl<E, P: AllPosition> Positioned for (P, StreamKey, Decoded<E>) {
    // (self.0, Some(self.1), self.2)
}
```

- Sealed trait with exactly two impls, consumed only by `Projection::advance` — the
  signature change is internal-shape, not surface breakage.
- `StreamKey` is `Bytes`-backed; moving it out is free (no copy).
- Doc rewrite: the current "the stepper **drops** the `StreamKey` … a key-aware fold is
  a `Projector` signature question deliberately out of #333's scope" paragraph
  (`projection.rs` `Positioned` docs and the duplicate note on the tuple impl) is
  replaced with: the tag flows to `Projector::apply_attributed`.

### 3. Stepper `advance` forwards the key

```rust
let (position, key, decoded) = item.into_parts();
let folded = self
    .projector
    .apply_attributed(state, key.as_ref(), &decoded.event)
    .map_err(ProjectionError::Apply)?;
```

Everything else in `advance` (trigger, commit, pending) is unchanged. Per-stream call
sites compile and behave identically (key is `None`, default delegates).

### 4. `examples/signed-events` becomes the proof

- Delete `RegisterView::route_to`, the `route: Option<RegisterId>` field, and the
  `ViewError::NoRoute` variant.
- `RegisterProjector` overrides `apply_attributed`: decode the `RegisterId` from the
  key bytes (`RegisterId` already has the raw-bytes constructor built for exactly this,
  `domain.rs:56`), fold into that register's entry. `apply` (keyless) becomes the
  error path for this projector: a multi-stream fold *requires* attribution, so plain
  `apply` returns a typed error (`ViewError::Unattributed`, replacing `NoRoute` — same
  failure domain, now structurally rare instead of ambient).
- Driver loop (`main.rs`): `view = projector.apply_attributed(view, Some(&key), &event)?;`
  — the shim call disappears.
- README + module docs updated: the "route through the driver" workaround section is
  replaced by the trait-level channel.

### 5. Docs

- `CLAUDE.md`: update the `projection.rs` architecture note (the stepper no longer
  "drops the attribution key"; `apply_attributed` is the key-aware fold) and the
  `Positioned` description.
- Issue #345 closed by the PR; #185's deferred-strain note resolved.

## Error handling

No new error types in `mnesis-store`. The fold's failure domain is unchanged
(`ProjectionError::Apply` wraps `P::Error` as before). In the example, `NoRoute`
(driver forgot to inject) becomes `Unattributed` (fold invoked without a key) — one
variant, one failure domain, now only reachable by calling the keyless `apply` on a
projector that requires attribution.

## Testing (rule 7 categories)

In `crates/store` (stepper tests live beside the existing `Projection` suite):

1. **Sequence/protocol** — drive `advance` over a mixed `$all` item sequence with a
   key-aware projector; assert per-key routing lands each event in the right bucket
   and checkpoints advance exactly as before.
2. **Boundary/defensive** — (a) plain projector (no override) fed `$all` items: key
   silently ignored, result equals the keyless fold — proves the default seam; (b)
   key-aware projector fed per-stream items (`key = None`): surfaces its typed error /
   fallback, proving `None` is observable, not swallowed.
3. **Lifecycle** — resume path: commit mid-sequence, reload via `Projection::load`,
   continue with attributed items; state + checkpoint agree.
4. **Linearizability** — not applicable beyond existing stepper coverage (stepper is
   single-consumer by construction; no new concurrency introduced).

Existing per-stream stepper tests passing **unmodified** is the recorded compatibility
proof (same bar as #327).

## Out of scope

- Any `Subscription`/`RawEventStore`/adapter change — the key already arrives on the
  item (#333).
- A key-aware `PersistTrigger` — trigger semantics untouched.
- Attribution on the per-stream path (the item has no tag by design; documented
  read-path asymmetry from #333 stands).
