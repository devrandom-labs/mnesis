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
  the digest of the key that controls it — you cannot mint a register id without
  the key. `RegisterId` is a 32-byte `[u8; 32]` newtype and satisfies
  `mnesis::Id` through the blanket impl (`Display` = hex, `AsRef<[u8]>` = the raw
  digest / stream key).
- **Signed events.** Every event carries an ed25519 signature over a
  deterministic blake3 preimage:
  - `Inception` signs `blake3(b"incept" ‖ owner_pubkey)`.
  - `Set` signs `blake3(b"set" ‖ key ‖ 0x00 ‖ val ‖ prior_digest)`.
- **Hash chain.** Each `Set` carries `prior_digest`, the chain digest of the
  event before it, so a stream is tamper-evident. The chain digest is a
  deterministic, **infallible** structured hash of the event's fields
  (`event_digest`), computed identically on the write side and the read side.

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

1. **Envelope metadata is not on the typed facade — [#344].** `EventStore::save`
   / `save_with` build the pending envelope with **no** `.metadata()`, so the
   high-level repository path always persists `metadata = None`. KERI's natural
   home for a signature/attachment is envelope metadata; a consumer that wants it
   there must drop to the raw `RawEventStore::append` seam and hand-build a
   `PendingEnvelope`. **Decision for this example:** the signature lives *inside
   the event payload*, so the example stays on the blessed typed path — and the
   gap is documented here and filed, not hidden. #344 is now resolved by the
   builder-level `MetadataProvider` configured via `RepositoryBuilder::metadata(provider)`,
   so new code can place signatures in envelope metadata on the typed path.

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
