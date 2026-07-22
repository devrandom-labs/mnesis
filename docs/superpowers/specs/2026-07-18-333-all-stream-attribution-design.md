# Stream attribution on the `$all` path (#333)

**Status:** approved design, pre-implementation
**Issue:** [#333](https://github.com/devrandom-labs/mnesis/issues/333) (supersedes #217)
**Milestone:** 1 — Pre-Freeze (1.0 blockers)
**Date:** 2026-07-18

## Problem

An `$all` consumer receives `(AllPosition, PersistedEnvelope)` and cannot tell
which stream an event came from. The store knows the stream id at append time
and drops it on the `$all` read path. Today the only workaround is embedding
the aggregate id in every event payload — the axum-todos port (#326-7) showed
this is boilerplate carried through every command and event variant, and an
event that forgets it is silently unroutable in every downstream projection.

## Decision

Stream attribution on `$all` is a **store guarantee**, not a domain
convention. The `$all` item carries the stream's `StreamKey` structurally.

Reasons, in order of weight:

1. **Freeze asymmetry.** Adding attribution after 1.0 changes the `AllStream`
   item shape — a major bump. Shipping it now and having it be redundant costs
   `id_len` bytes per `$all` row. The error is irreversible in one direction
   only; take the reversible side. (Same logic as the frame-format-version
   byte, #205.)
2. **The convention already failed its field trial.** #326-7 is the
   experiment: a careful author with the findings list open still carried the
   id as boilerplate in four event variants and three commands.
3. **Routing before decoding.** With id-in-payload, attributing an event
   requires decoding it, so routers/multi-stream projections must hold the
   domain codec just to move bytes. The subscription layer deliberately stays
   raw (consumer holds the codec); a structural tag keeps routing on raw
   bytes. KERI self-addressing does not help: the prefix is inside the SAD,
   so you still parse to route.
4. **The redundancy counterweight is bounded and already has a knob.** In the
   Agency stack ids ride in payloads anyway, but the tag costs `id_len` bytes
   per `$all` row only on fjall, and a produce-and-sync device runs
   `AllIndex::Disabled` and pays zero.

Precedent: wire V2 dropped `global_seq` from the frame and `PersistedEnvelope`;
positional/identity facts ride **beside** the envelope (tuple tag, key, query
argument), never inside the stored value. This design follows that direction.

## Contract change (`mnesis-store::store`)

`RawEventStore::AllStream`'s item becomes the flat 3-tuple:

```text
(Self::AllPosition, StreamKey, PersistedEnvelope)
```

- `StreamKey` already exists (`stream_id.rs`, a `Bytes` newtype spoken by
  `append`/`read_stream`/`StreamLister`/import). **No new types.** Clones are
  one Arc bump.
- A named `AllItem<P>` struct was considered and rejected: it carries no
  invariant the tuple doesn't, and either shape is equally frozen at 1.0
  (`#[non_exhaustive]` on non-error types is banned).
- `PersistedEnvelope` is untouched.
- The per-stream path is untouched: there the id is the query argument and
  every returned envelope belongs to it by construction. This read-path
  asymmetry is intentional and gets documented on the trait (rule 4 addendum).

## Ripple through the store crate (shape-following, mechanical)

- `subscription.rs` — `subscribe_all` yields
  `Step<(P, StreamKey, PersistedEnvelope)>`.
- `catchup.rs` — `AllCatchup::position_of` keeps reading the first tuple
  element.
- `decoded.rs` — the `RawItem` impl for the `$all` tuple maps to
  `(P, StreamKey, Decoded<E>)`: position **and** key preserved beside the box.
- `projection.rs` — the sealed `Positioned` impl follows the same tuple, so an
  `$all` stepper call site gets routing with no change to `advance`'s
  signature.
- `store.rs` docs — item contract, attribution guarantee, and the per-stream
  asymmetry.

## fjall on-disk layout (the measured fork)

Two candidate layouts for the `events_global` partition:

- **A2 — id in the key (lean):**
  `[u64 BE global_seq][u16 BE id_len][id][u64 BE version]`.
  The 8-byte `global_seq` prefix is unique per event, so it fully determines
  sort order; the id/version behind it are payload smuggled into the key,
  never order-determining. Resume bounds remain prefix ranges on
  `global_seq`. The value stays the **same shared `Slice`** as the `events`
  partition: zero extra allocation, zero extra value bytes.
- **A1 — id wrapped into the value:** `[u16 id_len][id][frame]`.
  Breaks the shared-value-bytes optimization, one extra buffer per append,
  `2 + id_len` bytes per row.

Both are benchmarked (append throughput + on-disk size, extending the
existing `#270`-era `$all`-index bench) before the layout is committed; the
numbers go in the PR (rule 9).

**Migration posture:** clean break, pre-1.0, documented — same posture as the
wire V1→V2 transition. No migration machinery: the old 16-byte key
(`[gs][version]`) is structurally distinguishable from the new ≥19-byte key
(`8 + 2 + id_len + 8`, `id_len ≥ 1`; fjall rejects empty keys so an empty id
cannot make 16), so decode rejects it as a typed `CorruptValue` rather than
misparsing. `events_global` is derived data, rebuildable from `events` if
ever needed.

## Other adapters

- **postgres** — rows already store `stream_id`; `read_all`'s SELECT adds the
  column. Free.
- **in-memory** — clones the id `Bytes` it already holds.
- **toy-adapter guide** (`mnesis-store-testing` crate docs) — updated to show
  the stamp.

## Error handling

- fjall: an `$all` row whose key is too short / whose `id_len` overruns the
  key is a typed `CorruptValue` (corruption domain, distinct from input
  validation — rule 3). Adapters defend their own boundary; none trusts the
  writer.
- No new error variants elsewhere: the tuple change is shape-only.

## Testing

- **Conformance kit (the freeze-proof):** new check in the core matrix —
  append to two interleaved streams, `read_all`/`subscribe_all`, assert each
  item's `StreamKey` equals its append target, in `AllPosition` order.
  Existing `$all` checks updated for the new shape. The toy adapter must pass
  it from the guide alone.
- **fjall white-box:** key codec round-trip incl. boundary ids (1 byte,
  `u16::MAX`-length), old-16-byte-key rejection as `CorruptValue`, proptest
  ranges include the boundaries (rule 8).
- The 4 cross-cutting categories (rule 7) are covered by the kit's
  sequence/linearizability/lifecycle updates plus the fjall boundary tests.

## Evidence loop

`examples/axum-todos`' `$all` read model routes by the tag; the #326-7 doc
comments in `domain.rs` note the store-level answer (domain events *may*
still carry ids — a domain choice now, not a framework obligation).

## Out of scope

- **(b) metadata channel at the typed save seam** — a separate feature (the
  HLC/signature carry-not-index channel); its own card if Agency needs it.
- Any per-stream item change; export/import (per-stream, already labelled).
- A scalar-view or filtered-`$all` capability (#215 territory).

## Delivery

One PR, branch `feat/333-all-stream-attribution` off `origin/main`. Gate:
`nix flake check` via the pre-commit hook, plus a by-hand
`cargo clippy --all-features --all-targets` pass. Conventional commit scope:
`feat(store):`.
