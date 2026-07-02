# Envelope builder: one fallible terminal + derive `event_type`

**Issue:** #254 — `pending_envelope` builder returns a `Result` mid-chain and makes you restate `event_type`/version.
**Status:** design — research settled (approach A).
**Date:** 2026-07-02

---

## The three frictions

```rust
pending_envelope(ver)              // 3. version threaded by hand
    .event_type(event.name())      // 2. restate the name the event already knows
    .payload(payload).expect(...)  // 1. Result in the MIDDLE of the chain
    .build()
```

Confirmed in the code: `WithEventType::payload() -> Result<WithPayload, EnvelopeError>` (validates the size cap at the setter), and `with_metadata()` / `event_type_bytes()` are fallible too — so the fluent chain breaks with a `?`/`.expect()` before `.build()`.

## Prior art (rule 0)

The idiomatic Rust convention is **validation at the terminal `build()`**, not mid-chain. `bon`: "validations are deferred until you invoke the finishing `build()`" — mid-chain setter validation exists but is the documented exception. `derive_builder` runs its `validate` fn inside `build()`. ([bon fallible builders](https://bon-rs.com/guide/patterns/fallible-builders), [typed-builder](https://github.com/idanarye/rust-typed-builder), [derive_builder](https://docs.rs/derive_builder)). ES frameworks derive the event *type* from the event (EventStoreDB `EventData` type, Axon/Marten from the class) and let the append **position own the version** — you never restate either.

## Design (approach A)

Three targeted changes to the `pending_envelope` typestate builder in `crates/nexus-store/src/envelope.rs`. Keep the typestate ordering (`version → event_type → payload → build`); make every intermediate step **infallible** and move validation to a single fallible `build()`.

### 1. Single fallible terminal — `build() -> Result`

Setters stash raw bytes; `build()` validates payload (and metadata) once.

```rust
impl WithEventType {
    /// Stash the payload bytes. Infallible — validation happens in `build`.
    #[must_use]
    pub fn payload(self, payload: impl Into<Bytes>) -> WithPayload { /* store Bytes, no check */ }
}

impl WithPayload {
    #[must_use]
    pub const fn schema_version(mut self, v: SchemaVersion) -> Self { /* unchanged */ }

    /// Stash metadata bytes. Infallible — validation happens in `build`.
    #[must_use]
    pub fn metadata(mut self, metadata: impl Into<Bytes>) -> Self { /* store Some(Bytes) */ }

    /// Validate and finalize. The **one** fallible step.
    ///
    /// # Errors
    /// [`EnvelopeError::Value`] if the payload exceeds `MAX_PAYLOAD_LEN`, or
    /// metadata is empty / exceeds `MAX_METADATA_LEN`.
    pub fn build(self) -> Result<PendingEnvelope, EnvelopeError> {
        let payload = Payload::from_bytes(self.payload)?;
        let metadata = self.metadata.map(Metadata::from_bytes).transpose()?;
        Ok(PendingEnvelope { /* … */ })
    }
}
```

`WithPayload` now holds `payload: Bytes` and `metadata: Option<Bytes>` (raw) instead of the pre-validated newtypes. `with_metadata` is **deleted** — folded into the `.metadata()` setter + fallible `build()`. The old infallible `build()` becomes fallible; that is the accepted single terminal.

### 2. Derive `event_type` from the event — `.event(&e)`

```rust
impl WithVersion {
    /// Set the event type from a `&'static str` literal. Infallible. (unchanged)
    pub fn event_type(self, event_type: &'static str) -> WithEventType { /* … */ }

    /// Derive the event type from a `DomainEvent` — no restating `name()`.
    #[must_use]
    pub fn event<E: DomainEvent + ?Sized>(self, event: &E) -> WithEventType {
        self.event_type(event.name())   // name() is &'static str → EventType::from_static_str, infallible
    }
}
```

`DomainEvent` is already a dependency of `nexus-store` (via `nexus`). `event_type_bytes(Bytes) -> Result<..>` (the raw-bytes escape hatch) **stays fallible** — it's the rare "I have arbitrary bytes, not a `&'static str` or a `DomainEvent`" path, and it is not part of the fluent common chain; keeping its check at the setter is acceptable (it is itself a near-terminal branch). *Alternatively fold it into `build()` too — decide during implementation from whether any caller uses it mid-chain.*

### 3. Version — lean on #253, don't re-solve

The card's "no hand-computed version" is already delivered by #253's `Version::run` (yields the contiguous versions to zip with events). The builder simply **takes** a `Version` at `pending_envelope(ver)` — stating it, not computing it. No new version machinery here.

### Result

```rust
pending_envelope(ver)
    .event(&event)        // derives type from name()
    .payload(bytes)       // infallible
    .build()?             // the single fallible terminal
```

One `?`, at the end. Nothing restated. The internal `save_events` (repository.rs) and the raw-path examples collapse to this shape.

## Migration surface (this is the cost)

The API change touches **every** caller (`.payload(x)?.build()` → `.payload(x).build()?`, `.with_metadata(m)` → `.metadata(m).build()?`):
- **Internal:** `save_events` in `repository.rs` (uses `.payload(payload)?`).
- **Tests (~8 files):** `inmemory_conformance`, `property_tests`, `bounded_batch_proptest`, `subscription_tests`, `bug_hunt_tests`, `inmemory_store_tests`, `adversarial_property_tests`, plus the envelope.rs unit tests. Note `bug_hunt_tests` has builder-specific probes (`bug_probe_cannot_construct_pending_envelope_without_builder`, `attack_pending_envelope_builder_preserves_all_fields`) — update, don't delete.
- **Examples:** the `store-inmemory`/`store-and-kernel` raw-path seeds, and any `encode_decided`-style helper (also adopt `.event(&e)` to drop the restated `event.name()`).

A wide but mechanical sed-like change. The typestate still forbids illegal orderings, so most misconversions won't compile.

## Testing (rule 7 first)

1. **Sequence/protocol:** the full chain `pending_envelope(v).event(&e).payload(p).metadata(m).build()?` yields an envelope with the right version/type/payload/metadata; `.event(&e)` produces the same `event_type` as `.event_type(e.name())`.
2. **Boundary/defensive:** `build()` returns `Err(EnvelopeError::Value)` for an oversize payload and for empty / oversize metadata — the check moved to the terminal, so prove it still fires there. Payload at exactly `MAX_PAYLOAD_LEN` builds; `+1` errors.
3. **Typestate (compile-fail or existing probe):** you still cannot `build()` without version/event_type/payload; `metadata` is optional. Keep `bug_probe_cannot_construct_pending_envelope_without_builder`.
4. **Equivalence:** an envelope built the new way is byte-identical (same wire frame) to the old way for the same inputs — the refactor is behavior-preserving except for *where* the `Result` surfaces.

## What this does not do (approach A scope)

- No `VersionedEvent<&E>`-rooted builder (that was option B — rejected as over-machinery).
- No change to `PersistedEnvelope` (read path) or the wire frame.
- No new version API — #253 owns that.
- `event_type_bytes` may stay fallible (rare escape hatch); the *common* chain is Result-free until `build()`.
