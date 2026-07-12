# Stability

What pinning a `1.x` mnesis crate buys you. This document takes effect at the
1.0 release of the core crates; until that release every crate is 0.x and this
is the policy being frozen, not yet a promise in force.

## Crate tiers

| Tier | Crates | Promise |
|------|--------|---------|
| **1.0 (frozen)** | `mnesis`, `mnesis-macros`, `mnesis-store`, `mnesis-wake`, `mnesis-wake-nostd` | Full semver: breaking changes only at a major release |
| **0.x (evolving)** | `mnesis-inmemory`, `mnesis-fjall`, `mnesis-postgres`, `mnesis-store-testing` | Breaking changes allowed in any `0.(x+1)` release, called out in release notes |
| **Unpublished** | `workspace-hack`, `mnesis-nostd-smoketest`, examples | No promise |

Coupling rules across the tiers:

- `mnesis-macros` is version-locked to `mnesis` (the serde/serde_derive
  pattern): they release together, always at the same version.
- A `mnesis` major forces a `mnesis-store` major — kernel types (`Version`,
  `Events`, the aggregate traits) appear in store APIs.
- A `mnesis-store` major forces a major of both wake crates — they implement
  its public `WakeSource` contract in their public API.
- Adapters are consumers: a core major forces nothing on a 0.x crate beyond a
  routine dependency bump.
- No 0.x type appears in the public API of a 1.0-tier crate, with one
  acknowledged exception listed under
  [Public dependencies](#public-dependencies).

## Semver surface

The public API of the 1.0-tier crates is:

- Every documented `pub` item — types, traits, functions, macros, and
  feature-gated items when their feature is enabled.
- **Documented trait semantics** — behavior, not just signatures. The store
  contract's promises (inclusive `read_stream` bound, exclusive `read_all`
  bound — an intentional asymmetry — strict-after subscription resume,
  `CaughtUp` emitted exactly once,
  conflicting appends rejected with nothing landing, spurious wakes
  permitted) are API; the `mnesis-store-testing` conformance kit is their
  executable form. Changing one is a breaking change even if every signature
  still compiles.
- Documented `Send`/`Sync` bounds on public types and on returned streams and
  futures.

Explicitly **not** public API:

- Anything `#[doc(hidden)]`.
- Sealed traits' internals (`RawItem`, `ConflictPredicate`, `KeyspaceConfig`,
  …): implementing them outside this workspace is unsupported.
- The exact text of `Display`/`Debug` output, including `ErrorId` truncation
  rendering. Match on error *variants*, never on strings.
- Adapter internals: on-disk key layouts, partition names, and
  connection/session behavior of the 0.x adapters.
- The conformance kit's check list: new checks may be added in any release. A
  new check failing an adapter is the kit doing its job, not a breaking
  change.

### Public dependencies

Three external crates appear in 1.0-tier public APIs; their major version is
part of our contract, so a semver-incompatible bump of any of them is a
mnesis major:

- `bytes` (1.x) — `Bytes` in envelope and codec APIs.
- `futures-core` (0.3) — `Stream` in stream and subscription bounds. It is
  0.x upstream but de-facto frozen; we treat `0.3` as its major and
  acknowledge it as the one 0.x type in a 1.0 surface.
- `minicbor` (2.x, behind the `cbor` feature) — the backup box's sink trait
  (`minicbor::encode::Write` bounds on `ChunkWriter`/`SectionWriter`) and
  encode errors (`WriteError::Encode`).

Everything else (arrayvec, tokio, parking_lot, foldhash, event-listener,
fjall, sqlx, …) is an implementation detail and may change in any release
(#208 sealed the leaks).

## On-disk format

The event frame (`mnesis-store`'s `wire` module) carries a leading
format-version byte (#205); the CBOR backup box carries a `format_version`
header field. The promise, for both:

- **Within 1.x**: the default write format never changes (frame v2 today),
  and every 1.x release reads everything any 1.x release wrote.
- **Across one major**: 2.x may introduce and default to a new format, but
  must still *read* every format 1.x wrote — an in-place upgrade never needs
  a migration step.
- **Beyond one major**: export → CBOR box → import is the supported migration
  path.
- Unknown format versions always fail with a typed error
  (`DecodeError::UnsupportedFrameVersion` for the frame, `ChunkError` for the
  box) — never a misparse.

Adapters are 0.x: their key layouts and partitions may change in a 0.x
release, but any such change ships with a documented migration path
(export/import at minimum). The CBOR box is the durable interchange format;
raw store bytes are not.

## MSRV

`rust-version` in every published Cargo.toml equals the pinned stable
toolchain we build and test with (currently **1.95.0**). Raising the MSRV is
a **minor** change, never a patch. We claim no trailing floor: the declared
MSRV is the only supported toolchain lower bound, and cargo enforces it.

## Feature flags

All cargo features of the 1.0-tier crates are **additive**: enabling a
feature never changes or removes an existing item's signature (the #211
unification guarantee). Adding a feature is a minor change; changing a
crate's default feature set is a major change. There are no unstable features
at 1.0; if one is ever introduced it will be named `unstable-*` and excluded
from this promise.

## Enums

- Public **error** enums carry `#[non_exhaustive]` (#209): adding a variant
  is a minor change. Always match errors with a wildcard arm.
- Public **non-error** enums (`Step`, `Hydrated`, `Atomicity`, …) are
  deliberately exhaustive: adding a variant is a major change. Exhaustive
  matching that catches new domain states at compile time is a promised
  feature, not an oversight.

## Deprecation

Nothing is removed silently:

- An item is removed only at a major release, and must have shipped with
  `#[deprecated(note = "…")]` — naming its replacement — in at least one
  published minor release before that major.
- 0.x crates: deprecated in at least one 0.x release before removal.
