# Envelope Builder Ergonomics (#254) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development or superpowers:executing-plans. Steps use `- [ ]` checkboxes.

**Goal:** Make `pending_envelope` a clean chain — one fallible terminal `build() -> Result`, an `.event(&e)` that derives `event_type` from `DomainEvent::name()`, and no mid-chain `Result` (#254).

**Architecture:** Refactor the typestate builder in `crates/nexus-store/src/envelope.rs`: infallible setters that stash raw `Bytes`, validation moved into a single fallible `build()`; add `WithVersion::event(&e)`; delete `with_metadata` (fold into a `.metadata()` setter). Because this is an API break, the builder change and **every** call-site migration land in ONE commit.

**Design doc:** `docs/plans/2026-07-02-envelope-builder-ergonomics-design.md`

**Conventions:** run tests `nix develop -c cargo test -p nexus-store -- <name>`. Don't run `nix flake check` by hand — the pre-commit hook runs it (Bash `timeout: 600000` on `git commit`; if it times out with exit 143, check `git log` and re-run, up to 3×). All `use` at top. Strict clippy (pedantic/nursery denied); examples/tests need `--all-targets --all-features` (not covered by the `--lib` flake gate).

---

## Task 1: Atomic builder refactor + all call-site migrations (ONE commit)

**Files:**
- Modify: `crates/nexus-store/src/envelope.rs` (the builder + its unit tests)
- Modify: `crates/nexus-store/src/repository.rs` (`save_events` uses `.payload(payload)?`)
- Modify test callers: `crates/nexus-store/tests/{inmemory_conformance,property_tests,bounded_batch_proptest,subscription_tests,bug_hunt_tests,inmemory_store_tests,adversarial_property_tests}.rs`
- Modify examples: `examples/store-inmemory/src/main.rs`, `examples/store-and-kernel/src/main.rs`, and any other example using `pending_envelope` (grep to confirm).

### Step 1: Refactor the builder in `envelope.rs`

Change the three intermediate states so setters are infallible and `build()` is the single fallible terminal. Target shape:

- `WithVersion` gains `event`:
```rust
impl WithVersion {
    #[must_use]
    pub fn event_type(self, event_type: &'static str) -> WithEventType { /* unchanged: EventType::from_static_str */ }

    /// Derive the event type from a `DomainEvent` — no restating `name()`.
    #[must_use]
    pub fn event<E: DomainEvent + ?Sized>(self, event: &E) -> WithEventType {
        self.event_type(event.name())
    }

    // event_type_bytes(Bytes) -> Result<WithEventType, EnvelopeError> stays as-is
    // (rare raw-bytes escape hatch, not part of the fluent common chain).
}
```
Add `use nexus::DomainEvent;` to the top imports if not already present.

- `WithEventType::payload` becomes infallible and `WithPayload` holds raw `Bytes`:
```rust
#[derive(Debug)]
pub struct WithPayload {
    version: Version,
    event_type: EventType,
    payload: Bytes,                 // raw — validated in build()
    schema_version: SchemaVersion,
    metadata: Option<Bytes>,        // raw — validated in build()
}

impl WithEventType {
    /// Stash the payload bytes. Infallible — validated in `build`.
    #[must_use]
    pub fn payload(self, payload: impl Into<Bytes>) -> WithPayload {
        WithPayload {
            version: self.version,
            event_type: self.event_type,
            payload: payload.into(),
            schema_version: SchemaVersion::INITIAL,
            metadata: None,
        }
    }
}
```

- `WithPayload`: keep `schema_version`, add a `metadata` setter, make `build()` the fallible terminal, delete `with_metadata`:
```rust
impl WithPayload {
    #[must_use]
    pub const fn schema_version(mut self, schema_version: SchemaVersion) -> Self {
        self.schema_version = schema_version;
        self
    }

    /// Stash metadata bytes. Infallible — validated in `build`.
    #[must_use]
    pub fn metadata(mut self, metadata: impl Into<Bytes>) -> Self {
        self.metadata = Some(metadata.into());
        self
    }

    /// Validate and finalize — the one fallible step.
    ///
    /// # Errors
    /// [`EnvelopeError::Value`] if the payload exceeds `MAX_PAYLOAD_LEN`, or the
    /// metadata is empty or exceeds `MAX_METADATA_LEN`.
    pub fn build(self) -> Result<PendingEnvelope, EnvelopeError> {
        let payload = Payload::from_bytes(self.payload)?;
        let metadata = self.metadata.map(Metadata::from_bytes).transpose()?;
        Ok(PendingEnvelope {
            version: self.version,
            event_type: self.event_type,
            schema_version: self.schema_version,
            payload,
            metadata,
        })
    }
}
```
Delete the old `with_metadata` method entirely. Update the `pending_envelope` doc example to `.build()?`.

### Step 2: Update `envelope.rs`'s own unit tests

The existing `pending_envelope_builds_with_metadata` / `_without_metadata` tests use the old API. Convert them: `.payload(p)?` → `.payload(p)`, `.with_metadata(m)` → `.metadata(m)`, terminal `.build()` → `.build()?` (or `.build().unwrap()` in tests). Add:
```rust
#[test]
fn event_derives_type_from_domain_event_name() { /* .event(&e) event_type == e.name() */ }

#[test]
fn build_rejects_oversize_payload() { /* payload > MAX_PAYLOAD_LEN → Err(EnvelopeError::Value) at build() */ }

#[test]
fn build_rejects_empty_metadata() { /* .metadata(empty).build() → Err */ }
```
(Define a tiny local `DomainEvent` impl for the `.event()` test, or reuse an existing test event in the crate.)

### Step 3: Fix `save_events` in `repository.rs`

Current:
```rust
let envelope = pending_envelope(next_version)
    .event_type(event_name)
    .payload(payload)?
    .schema_version(SchemaVersion::new(schema_nz32))
    .build();
```
becomes:
```rust
let envelope = pending_envelope(next_version)
    .event(event)                 // derive type from the DomainEvent — drops `event.name()`
    .payload(payload)
    .schema_version(SchemaVersion::new(schema_nz32))
    .build()?;
```
`event` is the `&EventOf<A>` in scope (it's a `DomainEvent`). The `?` now folds `EnvelopeError` into the function's error — confirm `StoreError` has a `From<EnvelopeError>` or map it (`save_events` returns `StoreError`; check how it currently converts — there may already be an `EnvelopeError` arm/`From`. If not, add a `map_err`). Keep `event_name`/`schema` logic; only the builder call changes. If `event.name()` is still needed for `current_version(event_name)`, keep that line.

### Step 4: Migrate every remaining call site (compiler-driven)

Build the crate: `nix develop -c cargo build -p nexus-store --all-targets` (timeout 600000). The compiler now errors at every old call site. Apply the mechanical transform at each:
- `.payload(x)?` → `.payload(x)`
- terminal `.build()` → `.build()?` (or `.build().unwrap()` in tests where a panic is the intended assertion)
- `.with_metadata(m)` → `.metadata(m)` then ensure the terminal is `.build()?`
- Where a `DomainEvent` is in scope and the code does `.event_type(e.name())`, switch to `.event(&e)` (examples' encode helpers).

Repeat build until clean. Do the same for examples: `nix develop -c cargo build -p nexus-example-store-inmemory -p nexus-example-store-and-kernel --all-targets` (confirm real package names via `grep -h '^name' examples/*/Cargo.toml`), plus any other example the grep in Step 5 flags.

### Step 5: Verify

- `nix develop -c grep -rn "with_metadata\|\.payload([^)]*)?" crates/ examples/` — no surviving `with_metadata` calls; no `.payload(...)?` (the `?` should be gone).
- `nix develop -c cargo test -p nexus-store` (timeout 600000) → all pass.
- Run the affected example tests (per-package) → pass.
- `nix develop -c cargo clippy --all-targets --all-features` (timeout 600000) → clean.

### Step 6: Commit (Bash timeout 600000)
```bash
git add crates/nexus-store/src/envelope.rs crates/nexus-store/src/repository.rs crates/nexus-store/tests/ examples/
git commit -m "feat(store)!: envelope builder — one fallible terminal + .event() derives type (#254)"
```
(The `!` marks the breaking API change. Include any other files the compiler forced you to touch — `git add -u` after building to catch them all, then review `git status` before committing.)

---

## Task 2: Whole-branch review + PR

- [ ] **Step 1: Review** `git diff origin/main..HEAD`: confirm (a) no mid-chain `Result` remains in the builder (only `build()`/`event_type_bytes` are fallible), (b) `.event(&e)` derives the same `event_type` as `.event_type(e.name())`, (c) payload/metadata validation genuinely still fires — at `build()` now (an oversize payload must still error), (d) `save_events` error conversion is correct (`EnvelopeError` → `StoreError`, no domain blurring — rule 3), (e) no call site silently dropped a validation.
- [ ] **Step 2: Open PR** (`joeldsouzax` account), title `feat(store)!: envelope builder ergonomics (#254)`, body summarizing the one-terminal + `.event()` change and noting the breaking API (mid-chain `?` moves to `.build()?`). `Closes #254`. Squash-merge on green (`--delete-branch`).

---

## Self-Review Notes (author checklist — done)
- **Coverage:** single fallible `build()` (T1 S1), `.event(&e)` derive (T1 S1), delete `with_metadata` → `.metadata()` setter (T1 S1), internal + all-caller migration (T1 S3-4), version left to #253 (no-op here) ✓.
- **Consistency:** `WithPayload` now holds raw `Bytes`/`Option<Bytes>`; `build() -> Result<PendingEnvelope, EnvelopeError>`; `.metadata()`/`.event()` infallible; `event_type_bytes` stays fallible — used consistently across steps.
- **Rule adherence:** rule 3 — `EnvelopeError` stays its own domain, `save_events` maps it into `StoreError` without blurring; validation not silently dropped (moved, not removed) — the boundary tests prove it still fires at `build()`.
- **Watch-items:** (1) `save_events` may already have an `EnvelopeError → StoreError` conversion — check before adding one; (2) `bug_hunt_tests` builder probes must be updated not deleted; (3) confirm no example/test relied on `build()` being infallible (it's now `Result`); (4) `event_type_bytes` fallibility — leave as-is unless a caller uses it mid-chain.
```
