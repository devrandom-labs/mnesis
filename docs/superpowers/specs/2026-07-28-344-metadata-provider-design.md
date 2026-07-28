# #344 — Builder-Level Metadata Provider for the Typed `EventStore` Facade

**Date:** 2026-07-28
**Issue:** [#344](https://github.com/devrandom-labs/mnesis/issues/344)
**Status:** Approved (design), pending implementation
**Branch:** `feat/344-metadata-provider`

## Problem

The typed `EventStore` facade write path (`save_events`, `crates/store/src/repository.rs`)
never calls `PendingEnvelope`'s existing `.metadata()` builder step, so every event
persisted through `Repository::save` / `EventStore::save_with` lands with
`metadata = None`. A consumer that wants signature bytes / attachments / HLC stamps in
envelope metadata (KERI's placement for indexed signatures; the sanctioned
carry-not-index home for HLC/causal metadata) must abandon the typed facade for the raw
`RawEventStore::append` seam, losing codec/upcaster/aggregate ergonomics.

The read side is already metadata-complete: `Decoded.metadata` surfaces
`Option<Bytes>` on every read/subscription path. The gap is write-only, and the fix
site is one envelope-build chain.

## Decision

**Option 1 of #344 — a builder-level metadata provider**, chosen over blessing the raw
seam (option 2: any HLC-stamping app abandons the facade entirely) and over
payload-embedded attachments (option 3: contradicts the standing carry-not-index
architecture decision).

Builder-level (configured once at `RepositoryBuilder`) rather than per-call
(`save_with_metadata`): `SagaRepository::react_and_save`,
`CommandRepository::execute`, and `Snapshotting::save` all write through trait
`Repository::save` internally — call sites the application author never makes. A
provider carried by the facade rides through all of them for free; a per-call
parameter reaches none of them.

## Design

### Trait + no-op default

```rust
/// Write-path metadata producer. Called once per event, post-encode.
pub trait MetadataProvider<E: ?Sized>: Send + Sync {
    fn metadata(&self, version: Version, event: &E, payload: &Payload) -> Option<Metadata>;
}

/// No-op default — the facade's `M = ()` slot. Mirrors `Upcaster` for `()`.
impl<E: ?Sized> MetadataProvider<E> for () { /* always None */ }

/// Closure blanket impl — users write a plain closure, never name the trait.
impl<E: ?Sized, F> MetadataProvider<E> for F
where
    F: Fn(Version, &E, &Payload) -> Option<Metadata> + Send + Sync,
{ /* delegate */ }
```

Decided points:

- **`version` parameter included** (fork resolved 2026-07-28): the `Version` is
  trivially in hand at the call site (`encode_at`), position-bound metadata ("sign
  this payload *at* version 5") becomes possible, and a trait-method parameter list
  is breaking to extend after the 1.0 freeze — one shot, taken now.
- **Infallible, returns `Option<Metadata>`**: HLC stamping is infallible;
  `ed25519_dalek` signing is infallible once the key is held (the KERI bridge case).
  A fallible provider would cost a fourth generic on `StoreError<A, C, U>` plus a new
  variant, updating every bound site in the workspace, for zero concrete consumers.
  Escape hatches if fallibility ever materializes: pre-compute metadata before
  `save`, or an additive trait at a minor.
- **Returns the validated `Metadata` newtype, not raw bytes**: the
  `MAX_METADATA_LEN` cap lives in `Metadata` construction (`value.rs`), handled at
  the provider author's layer. The facade never re-validates; envelope `build()`
  still backstops. Same once-at-the-type discipline as `SchemaVersion`.
- **Statefulness via interior mutability**: the provider is called through `&self`
  (an HLC clock uses atomics/`parking_lot`), same contract `WakeSource` already
  carries. Documented on the trait.

### Threading

- `EventStore<S, C, A>` becomes `EventStore<S, C, A, M = ()>`. The default type
  parameter keeps every existing call site compiling unchanged.
- `RepositoryBuilder` gains a `.metadata(provider)` step, order-independent like
  `.snapshot()`, slot default `()`.
- `save_events` receives the provider, calls it once per event after encoding, and
  on `Some` adds `.metadata(m)` to the envelope chain.
- The facade holds the provider `Arc`-wrapped, matching the codec/upcaster capture
  pattern.
- **Zero changes** to `Repository` (trait), `saga.rs`, `execute.rs`, `snapshot.rs`,
  envelope, wire format, adapters. Saga reactions, command execution, and the
  snapshot decorator inherit metadata stamping through the facade's trait impl.

### History guard — why this is not the old `M` generic

The 2026-05-27 audit deleted a metadata generic `M` threaded through 10+ traits and
hardcoded `()` everywhere. This design re-enters metadata at exactly **one** site, as
a defaulted parameter, now that the first concrete consumer (KERI bridge, agency#137)
exists — rule-4 YAGNI's "add at the second concrete use" working as intended.

### Documented tension — upcasting vs. byte-level signatures

Metadata is never upcasted. A signature over payload bytes stops verifying if an
upcaster rewrites the payload. Raw subscription paths see pre-upcast bytes
(verification works); the facade `load` path replays typed events (consumer never
sees bytes). KERI handles schema evolution via digest chains, not byte stability.
A doc note on `MetadataProvider` states: *signing payload bytes couples signature
validity to the frozen payload encoding*. Matches signed-events README finding 3.

## Conformance kit gap (same PR)

`mnesis-store-testing` must pin metadata round-trip as an adapter-contract check:
append an envelope carrying metadata → read back byte-identical, on **both**
`read_stream` and `read_all`. Without it a third-party adapter can drop metadata and
still pass the kit. Verify whether existing checks cover this; add if absent.

## Tests (rule 7, four categories first)

1. **Sequence/protocol**: save with provider → read back → metadata bytes exact
   match; multi-event batch → per-event metadata distinct; provider sees the correct
   ascending `Version` per event.
2. **Lifecycle**: fjall write → close → reopen → metadata survives byte-identical.
3. **Defensive boundary**: provider returning max-len metadata (cap boundary);
   mixed batch (`Some`/`None` per event); empty-capable provider on `()` default.
4. **Linearizability/isolation**: concurrent conflicting saves — the losing
   writer's metadata never lands (nothing of the losing batch lands).

Plus inheritance proof: saga `react_and_save` through a metadata-carrying facade
stamps the saga's own events.

## Out of scope

- Read-side typed metadata decoding (deliberately raw `Option<Bytes>` —
  carry-not-index; consumer parses).
- Fallible providers (see escape hatches above).
- Raw-seam documentation changes beyond the `MetadataProvider` doc notes.
