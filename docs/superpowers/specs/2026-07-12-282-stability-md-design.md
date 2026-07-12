# #282 — STABILITY.md: the 1.0 promise (design)

**Issue:** [#282](https://github.com/joeldsouzax/nexus/issues/282) · **Milestone:** 1 — Pre-Freeze (1.0 blockers)
**Deliverable:** a root-level `STABILITY.md` recording what the 1.0 freeze promises a consumer, linked from README and CLAUDE.md, plus an audit pass over the open freeze cards.

## Why

The freeze milestone lands API decisions, but nothing states what those decisions promise. Bombay (M2 adapter) is the first serious dependent; external adopters follow via the B1 OSS funnel. Without a written policy every future change relitigates "is this breaking?", and writing the doc is itself an audit — any clause that can't be written cleanly points at an unfinished freeze card.

## Decisions (made 2026-07-12)

| Fork | Decision |
|------|----------|
| Crate tiers | **Core + wake at 1.0**: `nexus`, `nexus-macros`, `nexus-store`, `nexus-wake`, `nexus-wake-nostd`. **0.x**: `nexus-inmemory`, `nexus-fjall`, `nexus-postgres`, `nexus-store-testing`. Unpublished: `workspace-hack`, `nostd-smoketest`, examples. |
| MSRV | Bump = **minor**, never patch. `rust-version` = the pinned tested toolchain (currently 1.95.0). **No trailing-floor claim** — stated explicitly (no CI job verifies a floor; claiming one would violate rule 0). |
| Deprecation | Removal only at a major; the item must have shipped `#[deprecated(note = "use X")]` naming its replacement in **at least one published minor** first. 0.x crates: at least one 0.x release. |
| On-disk format | **Major-bounded + one-major read overlap**: within 1.x the default write format stays frame v2 and every 1.x reads everything any 1.x wrote; 2.x may write a new format but must still read 1.x's. Beyond one major: export → CBOR box → import. Unknown format bytes fail typed (`UnsupportedFrameVersion`), never misparse. Same policy for the CBOR box `format_version`. |
| Doc shape | Single root `STABILITY.md`, clause-per-section. No rustdoc mirrors — #280's per-crate READMEs carry the docs.rs-facing links instead. |

## STABILITY.md contents (section by section)

1. **Crate tiers** — the table above, plus coupling rules:
   - `nexus-macros` is version-locked to `nexus` (serde/serde_derive precedent).
   - A `nexus` major forces a `nexus-store` major (kernel types are re-exported in store APIs).
   - A `nexus-store` major forces majors of both wake crates (they implement its public traits in their public API).
   - Invariant making the split sound: **no 0.x type may appear in a 1.0-tier crate's public API**, with one acknowledged exception — `futures-core` 0.3 (`Stream` in public bounds), de-facto frozen upstream and named in §2 as a public dependency whose semver-incompatible bump forces our major (cargo cannot check this invariant; the audit pass does).
2. **Semver surface** —
   - **In**: every documented `pub` item of 1.0-tier crates; *documented trait semantics* (inclusive `read_stream` / exclusive `read_all` bounds, strict-after resume, `CaughtUp` exactly once, conflict-rejects-atomically — behavior, not just signatures); documented `Send`/`Sync` bounds; the three acknowledged public dependencies `bytes`, `futures-core`, and `minicbor` (cbor feature) — a major bump of any forces our major; everything else was sealed by #208.
   - **Out**: `#[doc(hidden)]` items; sealed-trait internals (`RawItem`, `ConflictPredicate`, `KeyspaceConfig`); exact `Display`/`Debug` strings; `ErrorId` truncation rendering; adapter internals; the conformance kit's check list (kit is 0.x — a new check failing an adapter is the kit working, not a break).
3. **On-disk format** — the decided promise above, plus: adapters are 0.x so their key layouts may change in 0.x, but any such change must ship a documented migration path (export/import at minimum).
4. **MSRV** — the decided policy above.
5. **Feature flags** — all features of 1.0-tier crates are **additive** (enabling one never changes or removes an existing item's signature; #211 fixed the one violation, making this claimable). New feature = minor; changing the default set = major. No unstable features at 1.0; any future one must be named `unstable-*`.
6. **Enum policy** — public **error** enums carry `#[non_exhaustive]` (#209's conscious override of the CLAUDE.md ban): new variant = minor, consumers match with a wildcard arm. Public **non-error** enums (`Step`, `Hydrated`, `Atomicity`, …) stay exhaustive deliberately: new variant = major.
7. **Deprecation** — the decided policy above; nothing is ever removed silently.

## Audit pass (part of acceptance)

- Link STABILITY.md from README and CLAUDE.md.
- Sweep open freeze cards (#280, #221, #239, #223, #222, #185): each either conforms or gets a comment naming the clause it changes. #280 gets the tier-table decision (its "version strategy" checkbox is resolved by this doc).
- Verify no 0.x or private type leaks in the five 1.0-tier public APIs.
- **`nexus-wake` finding**: it still carries both the watch-generation path and the legacy `Notify`/`wake_all` transition path. Promoting it to 1.0 freezes that public surface — resolve as either a pre-freeze cleanup card or an explicit "intentional, frozen" note in the doc.

## Acceptance (from the issue)

STABILITY.md merged, linked from README + CLAUDE.md; every open freeze card either conforms to it or has a comment saying which clause it changes.

## Non-goals

- No rustdoc `stability` modules (deferred to #280's per-crate READMEs).
- No crates.io name reservation, docs.rs config, or release mechanics (#280, #221).
- No behavior or API changes — this PR is documentation plus issue comments; the only code-adjacent outcome is possibly filing a new cleanup card for `nexus-wake`.
