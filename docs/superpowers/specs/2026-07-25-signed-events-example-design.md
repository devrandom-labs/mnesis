# signed-events example — design (#185)

**Date:** 2026-07-25
**Issue:** #185 — Example: signed + content-addressed event aggregate (kernel-generality validation)
**Status:** approved for implementation

## Goal

Prove a **signed, content-addressed, hash-chained** aggregate fits nexus's existing
traits with **zero kernel/store changes**. The pattern is *general* (KERI-compatible,
not KERI): no KEL, SAID, rotation, witnesses, or CESR. KERI is the first consumer about
to bet on "nexus needs no kernel changes for signed content-addressed events"; this
example is the cheap proof of that claim, and a template for Task/Inventory too.

Exercises the KERI-shaped trait surface:
- **signed events** — ed25519 over a deterministic preimage
- **content-addressed id** — `id = blake3(pubkey)`, variable-length `AsRef<[u8]>`
- **hash-chain** — each event carries the prior event's digest (tamper-evidence)
- **crypto in `Handle`** — state-dependent verification before emitting
- **pure fold in `apply`** — signed event folds to state, no side effects
- **`Projector`** — re-verifies sigs read-side, folds external signed events into a view

## Strain point already found (file as separate issue)

The typed `EventStore` facade `save`/`save_with` (`crates/store/src/repository.rs:503`)
builds `pending_envelope(version).event().payload().schema_version().build()` — **no
`.metadata()`**. So the high-level repository path always writes `metadata = None`. The
`PendingEnvelope` builder supports metadata; the facade does not plumb it. A signed
consumer that wants signatures/attachments in **envelope metadata** (KERI's actual
placement) must drop to the raw `RawEventStore::append` seam and hand-build
`PendingEnvelope`. File this as the acceptance-criteria strain issue. **Decision for this
example: signature lives inside the event payload**, so the example stays on the blessed
typed path and the metadata gap is documented + filed, not hidden.

## Crate

`examples/signed-events`, package `mnesis-example-signed-events`, `publish = false`.
Deps: `mnesis` (derive, testing dev), `mnesis-store` (json, projection, subscription),
`mnesis-fjall` (projection), `ed25519-dalek`, `blake3`, `serde`, `serde_json`,
`thiserror`, `tokio`, `tempfile`, plus `rand_core` (dev, keygen), `workspace-hack`.
New `[workspace.dependencies]`: `ed25519-dalek`, `blake3`, `rand_core` — added via `cargo add`.

## Domain — `SignedRegister`

```
Id:      RegisterId([u8; 32])              // = blake3(pubkey). AsRef<[u8]> = &self.0. Display = hex.
State:   RegisterState {
             entries: HashMap<String,String>,
             last_digest: Option<[u8; 32]>,  // chain head; None before inception
             owner: Option<VerifyingKey>,    // seeded by Inception; None at initial()
         }
Event (DomainEvent, serde):
  Inception { owner_pubkey: [u8; 32], sig: [u8; 64] }        // genesis, prior = None
  Set       { key: String, val: String,
              prior_digest: [u8; 32], sig: [u8; 64] }        // prior = chain head
Commands:
  Incept    { signing_key }                                  // caller holds private key
  SubmitSet { key, val, signing_key }
Error (thiserror):
  RegisterError { BadSignature, BrokenChain{expected,actual}, Unauthorized, NotIncepted, AlreadyIncepted }
```

### Preimages (what the sig covers) & digests
- Inception signed bytes: `blake3(b"incept" ‖ owner_pubkey)`.
- Set signed bytes: `blake3(b"set" ‖ key ‖ 0x00 ‖ val ‖ prior_digest)`.
- **Event digest** (chain link) = `blake3(canonical serde bytes of the event)`.
  Canonical = the same JSON payload the codec persists, so digest is reproducible read-side.

### `Handle<Incept>` (fresh aggregate)
1. `state.owner.is_some()` → `Err(AlreadyIncepted)`.
2. Verify `sig` over inception preimage by `owner_pubkey` → else `BadSignature`.
3. Caller must build the `AggregateRoot` with `id == blake3(owner_pubkey)` — assert; mismatch is a programmer bug in the example wiring, not untrusted input.
4. `Ok(Some(events![Inception{..}]))`.

### `Handle<SubmitSet>`
1. `let owner = state.owner.ok_or(NotIncepted)?`.
2. `prior_digest = state.last_digest.ok_or(NotIncepted)?`.
3. Verify `sig` over set preimage by `owner` → else `BadSignature`.
4. `Ok(Some(events![Set{ prior_digest, .. }]))`.

### `apply(state, &event)` — pure fold
- `Inception` → set `owner`, `last_digest = Some(digest(event))`.
- `Set` → `entries.insert(key,val)`, `last_digest = Some(digest(event))`.
- No verification in `apply` (replay of already-trusted log). Verification is `Handle`'s job
  on the write side and the `Projector`'s job on the untrusted read side.

## Projector — untrusted read-side re-verification

`RegisterView { registers: HashMap<RegisterId, HashMap<String,String>> }`.
`Projector::apply(view, &event)` **re-verifies** the signature + chain against a tracked
per-register `(owner, last_digest)` before folding. A forged/tampered event → `Err`
(fallible projector, per project design). Proves a consumer reading `$all` off the store
does not trust the store's bytes — it re-checks crypto itself. Exercises
"Projector folds external signed events into a view."

## Write / persist path

Typed `Store::repository::<SignedRegister>()` → `EventStore` facade, JSON codec, fjall
adapter. `save` returns the `$all` position (#330). Metadata `None` (documented strain).

## Tests — 4 categories first (project rule 7)

1. **Sequence/Protocol** — incept → set → set on one root; `replay` rebuilds identical
   `entries` + `last_digest`; chain digests link (each `Set.prior_digest == digest(prev)`).
   `SubmitSet` before `Incept` → `NotIncepted`. Second `Incept` → `AlreadyIncepted`.
2. **Lifecycle** — write incept+sets through fjall, drop store, reopen from same path,
   load register, assert state + chain head resume; append another `Set` continues the chain.
3. **Defensive boundary** — tampered `sig` → `BadSignature`; wrong `prior_digest` →
   `BrokenChain`; event signed by a different key → `BadSignature`/`Unauthorized`; a forged
   event injected into the read stream → `Projector` rejects with `Err`.
4. **Linearizability/Isolation** — two concurrent writers `SubmitSet` on the same register
   at the same expected version (`tokio::spawn` + `Barrier`); exactly one commits, the other
   gets an optimistic `Conflict`; final chain is single-threaded/unbroken.

Use `AggregateFixture` (given/when/then, `testing` feature) for the pure decide/replay tests;
real fjall (`tempfile::TempDir`) for lifecycle + linearizability.

## README

`examples/signed-events/README.md`: which nexus traits are exercised and how (`Id`,
`Aggregate`/`Handle`/`AggregateState`, `DomainEvent`, `Projector`, `EventStore` facade,
`RawEventStore` via fjall), the ed25519 + blake3 chain, and a **strain-point note** linking
the filed metadata-on-facade issue.

## Acceptance (from #185)
- compiles, runs, integration tests pass under `nix flake check`
- README present
- strain point (metadata-on-facade) filed as separate issue and linked

## Deliberately out of scope (YAGNI)
- key rotation, multi-sig, witnesses, CESR, SAID, KERI vocabulary
- metadata-carrying facade change (that's the *filed* follow-up, not this example)
- any kernel/store source change (the whole point is to need none)
