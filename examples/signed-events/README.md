# signed-events

A **signed, content-addressed, hash-chained** aggregate on mnesis — with
**zero kernel or store changes** (issue #185).

The point of this example is a proof: mnesis's existing traits are enough to
build a KERI-*shaped* aggregate (signed events, a key-derived identity, a
tamper-evident chain) without touching the kernel or any store adapter. It is
KERI-shaped, **not** KERI — there is no KEL, SAID, key rotation, witness, or
CESR here. It is the cheap validation of "mnesis needs no kernel changes for
signed content-addressed events", and a template for Task/Inventory-style
domains.

## The domain — `SignedRegister`

A register is a small key→value store owned by one ed25519 key.

- **Content-addressed id.** `RegisterId = blake3(owner_pubkey)`. The identity is
  the digest of the public key. Anyone with that public key can compute the id;
  signing commands proves possession of the private key. `RegisterId` is a 32-byte `[u8; 32]` newtype and satisfies
  `mnesis::Id` through the blanket impl (`Display` = hex, `AsRef<[u8]>` = the raw
  digest / stream key).
- **Signed events.** Both variants require numeric `signature_version: 2`,
  independent of the envelope schema and stream version. Signing uses BLAKE3's
  derive-key mode with the exact context
  `devrandom-labs/mnesis/examples/signed-events/signing-preimage`:
  - `Inception` hashes `0x02 ‖ b"incept" ‖ owner_pubkey`.
  - `Set` hashes `0x02 ‖ b"set" ‖ blake3(key_utf8) ‖ blake3(val_utf8) ‖ prior_digest`.
  Ed25519 signs the resulting 32-byte digest. Independently hashed variable
  fields have fixed-width boundaries, including empty, Unicode and embedded
  NUL strings; there is no separator restriction.
- **Hash chain.** Each `Set` carries `prior_digest`, the chain digest of the
  event before it, so a stream is tamper-evident. The chain digest is a
  deterministic, **infallible** structured hash of the event's fields
  (`event_digest`), computed identically on the write side and the read side.
  It uses the separate derive-key context
  `devrandom-labs/mnesis/examples/signed-events/event-digest`, followed by
  `0x02`, `b"Inception\0"` or `b"Set\0"`, then owner/signature or
  key hash/value hash/prior/signature respectively. This binds the protocol
  version and separates chain digests from signing digests.

### Where the crypto lives

| Trait | Responsibility |
| ------- | ---------------- |
| `Handle<Incept>` | Sign the genesis event; reject a second inception (`AlreadyIncepted`). |
| `Handle<SubmitSet>` | Sign the set; **verify the signer is the stored owner** — a state-dependent check that rejects a non-owner (`Unauthorized`). |
| `AggregateState::apply` | Pure fold of an already-accepted event. **No verification** — replay trusts the committed log. |
| `Projector` (read side) | Re-verify **every** signature and chain link from scratch on untrusted bytes; reject forgeries/tampering with `Err`. |

## Which mnesis surfaces are exercised

- **Kernel:** `#[mnesis::aggregate]`, `Handle` / `events!`, `AggregateState`,
  `AggregateRoot` (`new` / `replay` / `commit_persisted` / `handle`),
  `DomainEvent` derive, `mnesis::Id` (blanket), and `AggregateFixture`
  (given/when/then) in the unit tests.
- **Store:** `Store::repository::<A>().json().build()` → the typed `EventStore`
  facade, `Repository::load` / `save`, `CommandRepository::execute` returning
  `Execution { position, .. }` (the #330 read-your-writes `$all` position) with
  `ExecuteError::is_conflict` for optimistic-concurrency, `RawEventStore`
  (`read_stream` / `read_all`, the latter's `StreamKey` attribution tag from
  #333), and the `Projector` trait for the read model.
- **Adapter:** a real on-disk `FjallStore` (`FjallStore::builder(path).open()`),
  used for the lifecycle (reopen) and linearizability (concurrent writers) tests.

## Run it

```bash
nix develop -c cargo run  -p mnesis-example-signed-events    # the demo binary
nix develop -c cargo test -p mnesis-example-signed-events    # unit + the 4 integration categories
```

The demo incepts two registers through the typed facade, persists them to a
temp-dir fjall keyspace, reloads one, then folds the whole `$all` stream through
the re-verifying projector.

## Tests — the 4 categories (project rule 7)

| Category | Where | What it proves |
| ---------- | ------- | ---------------- |
| Sequence / protocol | `tests/sequence.rs` (+ `domain.rs` units) | incept → set → set round-trips; the on-disk chain links; replay rebuilds identical state. |
| Lifecycle | `tests/lifecycle.rs` | write → drop store → reopen: state and chain head resume; a post-reopen `Set` continues the chain. |
| Defensive boundary | `tests/boundary.rs` (+ `projection.rs` units) | tampered signature → `BadSignature`; broken link → `BrokenChain`; forged (non-owner) event → rejected; wrong stream id → `IdMismatch`; non-owner command → `Unauthorized` at decide. |
| Linearizability / isolation | `tests/linearizability.rs` | two `tokio::spawn` writers + a `Barrier` on the same version: exactly one commits, the other gets an `is_conflict()` error; the final chain is single-threaded and unbroken. |

## Strain points found (candidate follow-ups)

1. **Self-contained signatures.** This example stores the signature in the
   payload. The typed facade also supports envelope metadata through
   `RepositoryBuilder::metadata(provider)` ([#344]). Metadata preserved during
   upcasting is original evidence, not authentication of rewritten payloads;
   authenticate original bytes before transforming them.

2. **`Projector::apply` could not see the stream key — resolved by [#345].**
   `Projector` now carries a defaulted second method,
   `apply_attributed(state, Option<&StreamKey>, &event)`: the stepper forwards
   the `$all` `StreamKey` tag (#333) through it, and `RegisterProjector`
   overrides it to decode the register id from the key bytes. The keyless
   `apply` remains the required method and is this projector's error path
   (`ViewError::Unattributed`); a single-stream projector implements only
   `apply` and the default delegates, key ignored. The old
   `RegisterView::route_to` driver shim is gone.

3. **The chain digest cannot be the JSON payload bytes.** The design sketch
   suggested `blake3(canonical JSON of the event)`, reproducible read-side. But
   `AggregateState::apply` is infallible and JSON serialisation is fallible, so
   re-serialising inside the fold would force an `unwrap` or a silent-sentinel
   digest — both banned. The example instead hashes the event's fields in a
   fixed, infallible structure (`event_digest`); the read side recomputes the
   same digest from the decoded event, so "reproducible read-side" still holds.

[#344]: https://github.com/devrandom-labs/mnesis/issues/344
[#345]: https://github.com/devrandom-labs/mnesis/issues/345

## Protocol compatibility and reproducible vectors

Missing, legacy (`1`) and unknown future signing discriminators are rejected
at JSON decoding; there is no implicit default or legacy-verification fallback.
Merely adding `signature_version: 2` to an old payload fails signature verification.
The old NUL-separated signing format permits distinct key/value assignments to
share a preimage, so its signatures alone cannot establish the intended assignment.

Migration requires an independently trusted source of the intended assignments
and authorization by the owner holding the private key. Create a fresh version-2
log in a separate database or namespace, emit a new inception, and sign the
trusted assignments through the handlers. Preserve the original legacy log for
audit. Re-signing untrusted legacy fields or continuing the old chain does not
repair its ambiguous authorization history. This example provides no automatic
migration utility.

[Retained vectors and provenance](tests/fixtures/README.md) document deterministic
keys, exact commands and the legacy source revision. `tests/signing_format.rs`
reproduces the version-2 signatures and digest through actual handlers, verifies
the projector, tests strict Ed25519 verification after field changes, exercises
seeded Unicode properties and byte-length boundaries, and reopens real Fjall
legacy rows to check typed rejection without altering their payloads.

The hashing contexts follow [BLAKE3's derive-key API contract](https://docs.rs/blake3/latest/blake3/struct.Hasher.html#method.new_derive_key).
The read side uses [Ed25519 strict verification](https://docs.rs/ed25519-dalek/2.2.0/ed25519_dalek/struct.VerifyingKey.html#method.verify_strict).
Fixed-width field hashes remove encoding-boundary ambiguity; they still rely on
BLAKE3's cryptographic collision resistance.
