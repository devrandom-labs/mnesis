# `$all` Stream Attribution (#333) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every `$all` read/subscription item carries the stream's `StreamKey` structurally: `(AllPosition, StreamKey, PersistedEnvelope)`.

**Architecture:** The existing `StreamKey` (`Bytes` newtype, `crates/store/src/stream_id.rs`) threads through the `$all` read path only. The `Catchup` seam generalizes to an associated `Item` + `position_of` so the adapter's item shape flows through the live loop verbatim. fjall carries the id in the `events_global` **key** (layout A2, gated by a Task-1 benchmark); postgres/in-memory already hold the id. Per-stream paths are untouched.

**Tech Stack:** Rust (pinned stable, edition 2024), fjall 3 (`bytes_1`), sqlx-postgres, criterion. Spec: `docs/superpowers/specs/2026-07-18-333-all-stream-attribution-design.md`.

**Branch:** `feat/333-all-stream-attribution` (already exists, spec committed).

---

## Ground rules for the executor

- **Verification loop:** `nix develop -c cargo nextest run -p <crate>` per task; NEVER toggle feature sets between runs — use `--all-features` consistently for local runs (the gate covers default features; `--all-features` is the superset the clippy rule requires anyway).
- **Every commit** runs the full `nix flake check` via the pre-commit hook (slow, several minutes). Do NOT skip it, do NOT pre-run it by hand. New source files must be `git add`ed before the hook runs or the flake build fails on the missing module.
- **Clippy is law:** deny-all + pedantic + nursery + restriction. No `unwrap`/`expect`/`panic`/`as` casts in production code; `#[allow]` only with `reason` on specific items. All imports at top of file. thiserror for all errors. Checked arithmetic for sizes/offsets.
- **Postgres tests** need `DATABASE_URL`; they self-skip locally (`skip_unless`), CI's nixosTest covers them. Do not try to run them locally.
- Run `nix develop -c cargo fmt --all` after every substantial edit, before staging.
- Keep a **deviation log** at the bottom of this file: every divergence from this plan, with reason and impact.

---

### Task 1: Benchmark the fjall layout fork (A2 key vs A1 value) — decision gate

The spec (rule 9) requires the layout decided by measurement, not assertion. This
task measures the two candidate `events_global` layouts **in isolation** (raw
fjall partitions, no store change needed) and records the numbers.

**Files:**
- Create: `adapters/fjall/benches/all_index_layout.rs`
- Modify: `adapters/fjall/Cargo.toml` (add the `[[bench]]` entry beside the existing ones)

- [x] **Step 1: Read the existing disk-size bench for its harness pattern**

Read `adapters/fjall/benches/projection_storage.rs` fully. Reuse its
keyspace-setup, persist, and on-disk-size measurement approach (it is the
recorded #270-era pattern). Reuse `fjall_benchmarks.rs`'s `payload()` helper
shape for synthetic values.

- [x] **Step 2: Write the layout bench**

Core shape (adapt setup/teardown to the harness read in Step 1 — same
`TempDir` + keyspace pattern, same disk-size measurement):

```rust
//! Measures the two candidate `$all`-index layouts for #333 (rule 9):
//!   A2: key = [u64 BE gs][u16 BE id_len][id][u64 BE ver], value = shared frame bytes
//!   A1: key = [u64 BE gs][u64 BE ver],                    value = [u16 id_len][id][frame]
//! 20_000 events, 120-byte payloads, 36-byte ids (uuid-string-sized), across 100 streams.

const EVENTS: u64 = 20_000;
const STREAMS: u64 = 100;
const PAYLOAD: usize = 120;
const ID_LEN: usize = 36;

fn a2_key(gs: u64, id: &[u8], ver: u64) -> Vec<u8> {
    let mut k = Vec::with_capacity(8 + 2 + id.len() + 8);
    k.extend_from_slice(&gs.to_be_bytes());
    k.extend_from_slice(&u16::try_from(id.len()).expect("bench id fits u16").to_be_bytes());
    k.extend_from_slice(id);
    k.extend_from_slice(&ver.to_be_bytes());
    k
}

fn a1_value(id: &[u8], frame: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(2 + id.len() + frame.len());
    v.extend_from_slice(&u16::try_from(id.len()).expect("bench id fits u16").to_be_bytes());
    v.extend_from_slice(id);
    v.extend_from_slice(frame);
    v
}
```

For each layout: open a fresh keyspace + one partition (same config as the
production `events_global`), insert all `EVENTS` rows (ids cycle through
`STREAMS` synthetic 36-byte ids, frame built once via
`mnesis_store::wire::encode_frame` as `fjall_benchmarks.rs::build_frame_value`
does), persist/flush, then record (a) wall-clock insert time via criterion and
(b) on-disk directory size the way `projection_storage.rs` measures it.

- [x] **Step 3: Run it and record the numbers**

Run: `nix develop -c cargo bench -p mnesis-fjall --bench all_index_layout`

Record in the **Decision record** section at the bottom of this plan: insert
time and on-disk bytes for A2 vs A1.

- [x] **Step 4: Decide**

Default is **A2** (id in key; value stays the shared `Slice` — zero extra
allocation on append). Choose A1 only if the numbers show A2 worse by >5% on
disk size **or** append time. If A1 wins, Task 7 swaps its key/value edits
accordingly (the store-level contract in Tasks 2–6 is identical either way) —
log the swap in the deviation log.

- [x] **Step 5: Commit**

```bash
git add adapters/fjall/benches/all_index_layout.rs adapters/fjall/Cargo.toml docs/superpowers/plans/2026-07-18-333-all-stream-attribution.md
git commit -m "bench(fjall): measure \$all-index layout fork for #333 (rule 9)"
```

---

### Task 2: Store contract — `AllStream` item becomes the 3-tuple

**Files:**
- Modify: `crates/store/src/store.rs:141-157` (the `AllStream` doc + bound), `:179-188` (append doc), `:224-251` (read_all doc)
- Modify: `crates/store/src/test_support.rs` (the in-crate `TestStore` double)

This task intentionally breaks compilation across the workspace; Tasks 3–8
restore it crate by crate. Work `-p mnesis-store` only until Task 5 is done.

- [ ] **Step 1: Change the `AllStream` bound**

In `crates/store/src/store.rs` change the associated type bound:

```rust
    type AllStream: futures::Stream<
            Item = Result<(Self::AllPosition, StreamKey, PersistedEnvelope), Self::Error>,
        > + Send
        + 'static;
```

`StreamKey` is already imported at the top of the file (`use crate::stream_id::StreamKey;`).

- [ ] **Step 2: Update the trait docs**

Rewrite the `AllStream` doc comment (`:141-154`) to describe the item as
`(position, stream key, envelope)`: position for checkpointing, **stream key
for routing** (the store knows the origin stream at append time; a `$all`
consumer routes on raw bytes without decoding the payload), envelope for
content. In the `read_all` doc (`:224-251`) add one paragraph:

```text
    /// # Stream attribution
    ///
    /// Each item carries the [`StreamKey`] of the stream the event was appended
    /// to — a **store guarantee**, not a payload convention. The per-stream
    /// read ([`read_stream`](Self::read_stream)) deliberately does NOT stamp
    /// it: there the id is the query argument and every returned envelope
    /// belongs to it by construction (intentional read-path asymmetry).
```

The delegating `impl RawEventStore for Store<S>` (`:298-299`, `:318-320`)
re-states only `type AllStream = S::AllStream;` — no change needed there.

- [ ] **Step 3: Update `TestStore` (the lib-test double)**

In `crates/store/src/test_support.rs`:
- `all: Vec<(TestAllPos, PersistedEnvelope)>` → `all: Vec<(TestAllPos, StreamKey, PersistedEnvelope)>`
- `type AllStream` item accordingly:

```rust
    type AllStream = futures::stream::Iter<
        std::vec::IntoIter<Result<(TestAllPos, StreamKey, PersistedEnvelope), TestStoreError>>,
    >;
```

- In `append`, push the key: where the current code pushes `(pos, env)` push
  `(pos, id.clone(), env)` (`id: &StreamKey` is the append argument; one Arc bump).
- In `read_all`, the filter/clone maps through the 3-tuple unchanged in logic.

- [ ] **Step 4: Verify the crate does NOT yet build (expected)**

Run: `nix develop -c cargo check -p mnesis-store --all-features`
Expected: FAIL in `catchup.rs` / `subscription.rs` / `decoded.rs` /
`projection.rs` — the ripple Tasks 3–5 fix. Do not commit yet.

---

### Task 3: Generalize the `Catchup` seam and the live loop

**Files:**
- Modify: `crates/store/src/catchup.rs`
- Modify: `crates/store/src/subscription_cursor.rs`
- Modify: `crates/store/src/subscription.rs`

- [ ] **Step 1: Give `Catchup` an associated `Item` + `position_of`**

Replace the trait's `Scan` item shape (`catchup.rs:31-56`) with:

```rust
pub trait Catchup: Send {
    /// The position key the scan resumes from (`Version` for a stream, the
    /// adapter's [`AllPosition`](crate::AllPosition) for `$all`).
    type Position: Copy + Send;
    /// One delivered scan item. Per-stream: `(Version, PersistedEnvelope)`
    /// (the loop-internal tag). `$all`: the adapter's
    /// `(AllPosition, StreamKey, PersistedEnvelope)` item, passed through
    /// verbatim — the loop never re-shapes what the adapter yields.
    type Item: Send;
    /// The bounded scan this target opens.
    type Scan: futures::Stream<Item = Result<Self::Item, Self::Error>> + Send;
    /// The scan/wait error type.
    type Error: core::error::Error + Send + Sync + 'static;

    fn read_after(
        &self,
        from: Option<Self::Position>,
    ) -> impl Future<Output = Result<Self::Scan, Self::Error>> + Send;

    /// Extract the resume position an item was delivered at — the single
    /// place the loop learns where it is.
    fn position_of(item: &Self::Item) -> Self::Position;

    fn arm(&self) -> impl Future<Output = ()> + Send + 'static;
}
```

Keep the existing doc prose about strict-after resume; move the
"position-tagged" sentence onto `Item`.

- [ ] **Step 2: Update the two impls**

`StreamCatchup` (unchanged machinery, explicit `Item`):

```rust
    type Position = Version;
    type Item = (Version, PersistedEnvelope);
    // Scan/Error/read_after/arm unchanged.
    fn position_of(item: &Self::Item) -> Version {
        item.0
    }
```

`AllCatchup` — the adapter's item passes through whole:

```rust
    type Position = <S as RawEventStore>::AllPosition;
    type Item = (
        <S as RawEventStore>::AllPosition,
        StreamKey,
        PersistedEnvelope,
    );
    type Scan = <S as RawEventStore>::AllStream;
    type Error = <S as RawEventStore>::Error;
    // read_after/arm unchanged.
    fn position_of(item: &Self::Item) -> Self::Position {
        item.0
    }
```

- [ ] **Step 3: Rewrite `live_stepped` over `C::Item`**

In `subscription_cursor.rs`, the return type becomes:

```rust
) -> impl futures::Stream<Item = Result<Step<C::Item>, C::Error>> + Send
```

and the two delivery sites (`:104-110` and `:123-127`) become:

```rust
                Some(Ok(item)) => {
                    s.read_from = Some(C::position_of(&item));
                    s.drained_in_chunk += 1;
                    if s.drained_in_chunk >= CATCHUP_CHUNK {
                        s.scan = None; // reopen next iteration from the advanced read_from
                    }
                    return Some((Ok(Step::Event(item)), s));
                }
```

(and identically in the arm-probe branch: `s.read_from = Some(C::position_of(&item)); … Step::Event(item)`).
The `use crate::PersistedEnvelope;` import becomes unused — remove it.

- [ ] **Step 4: Update `subscription.rs`**

`subscribe` (`:130`) — unchanged in behavior; the tag-drop map still applies
because `StreamCatchup::Item` is still the pair:

```rust
        Ok(live_stepped(catchup, from).map(|item| item.map(|step| step.map(|(_, env)| env))))
```

`subscribe_all` (`:159-177`) — only the signature's item type changes:

```rust
    pub fn subscribe_all(
        &self,
        from: Option<<S as RawEventStore>::AllPosition>,
    ) -> Result<
        impl futures_core::Stream<
            Item = Result<
                Step<(
                    <S as RawEventStore>::AllPosition,
                    StreamKey,
                    PersistedEnvelope,
                )>,
                <S as RawEventStore>::Error,
            >,
        > + Send
        + use<S>,
        <S as WakeSource>::Error,
    >
```

Add `use crate::stream_id::StreamKey;` to the imports. Update the
`subscribe_all` doc: the item is `Step<(AllPosition, StreamKey, PersistedEnvelope)>`;
checkpoint the position tag; **route by the stream key** without decoding.

- [ ] **Step 5: Fix the inline tests in both files**

Mechanical destructuring updates in `catchup.rs` tests and
`subscription_cursor.rs` tests — every `$all` binding of the form
`(pos, env)` from an `AllCatchup`/`read_all` scan becomes `(pos, key, env)`
(bind `_key` where unused), e.g. `catchup.rs:317`:

```rust
        let all: Vec<(crate::test_support::TestAllPos, StreamKey, PersistedEnvelope)> = …
        let positions: Vec<u64> = all.iter().map(|(p, _, _)| p.as_u64()).collect();
```

and `subscription_cursor.rs`'s `FailingCatchup` mock gains
`type Item = (Version, PersistedEnvelope);` and
`fn position_of(item: &Self::Item) -> Version { item.0 }` (its scan items are
per-stream pairs; no other change). In `scan_item_error_is_surfaced_in_order`
the `read_all` destructure `let (_pos, ok_env)` becomes `let (_pos, _key, ok_env)`.

While here, extend `all_catchup_reads_after_none_then_exclusive`
(`catchup.rs:301`) to also assert attribution — after the positions assert:

```rust
        let keys: Vec<&[u8]> = all.iter().map(|(_, k, _)| k.as_bytes()).collect();
        assert_eq!(
            keys,
            vec![b"a".as_slice(), b"b".as_slice(), b"a".as_slice()],
            "$all items must carry the stream key they were appended to"
        );
```

- [ ] **Step 6: Verify catchup/cursor lib tests pass**

Run: `nix develop -c cargo nextest run -p mnesis-store --all-features --lib`
Expected: PASS (integration `tests/` may still fail — Task 5 covers them).

---

### Task 4: `decoded.rs` — `RawItem` follows the 3-tuple

**Files:**
- Modify: `crates/store/src/decoded.rs:106-115` (the tuple impl), `:75-93` (docs), `:133` + `:179-183` (doc references)

- [ ] **Step 1: Replace the 2-tuple impl**

```rust
impl<P: Copy> sealed::Sealed for (P, StreamKey, PersistedEnvelope) {}
impl<P: Copy> RawItem for (P, StreamKey, PersistedEnvelope) {
    type Typed<T> = (P, StreamKey, Decoded<T>);
    fn envelope(&self) -> &PersistedEnvelope {
        &self.2
    }
    fn retag<T>(&self, decoded: Decoded<T>) -> (P, StreamKey, Decoded<T>) {
        (self.0, self.1.clone(), decoded)
    }
}
```

Add `use crate::stream_id::StreamKey;` to the imports. Delete the old
`(P, PersistedEnvelope)` impl — that shape no longer exists on any public path
(the trait is sealed, so this is not a downstream break beyond the intended one).

- [ ] **Step 2: Update docs**

In the `RawItem` doc (`:75-85`): the `$all` shape is
`(P, StreamKey, PersistedEnvelope)` → typed item `(P, StreamKey, Decoded<T>)`;
both tags (position bookmark, routing key) stay **beside** the box. Update the
`decoded` method doc (`:133`) and the `for_each_decoded` note (`:179-183`):
over an `$all` stream neither the position **nor the stream key** is surfaced
to `f`; positioned/routed `$all` folds use `.decoded()` or a raw loop.

- [ ] **Step 3: Verify**

Run: `nix develop -c cargo check -p mnesis-store --all-features --lib`
Expected: compiles (integration tests still pending Task 5).

---

### Task 5: `projection.rs` `Positioned` + store integration tests

**Files:**
- Modify: `crates/store/src/projection.rs:95-102` (+ docs `:61-76`, `:149-165`, `:265-271`)
- Modify: `crates/store/tests/*.rs` (mechanical destructuring)

- [ ] **Step 1: Replace the `$all` `Positioned` impl**

```rust
impl<E, P: AllPosition> sealed::Sealed for (P, StreamKey, Decoded<E>) {}
impl<E, P: AllPosition> Positioned for (P, StreamKey, Decoded<E>) {
    type Event = E;
    type Pos = P;
    fn into_parts(self) -> (P, Decoded<E>) {
        (self.0, self.2)
    }
}
```

Add `use crate::stream_id::StreamKey;` to the imports. Document on the impl (and
in the `Positioned` trait doc's `$all` bullet) that the stepper **drops the
key**: `Projector::apply(state, &event)` is a fold over events, and a key-aware
fold is a `Projector` signature question deliberately out of #333's scope — a
consumer that routes by key hand-rolls its loop over the `.decoded()` stream
(the axum-todos `index.rs` loop shows the stepper path still compiling whole).

- [ ] **Step 2: Sweep the integration tests**

Every `$all` destructure in `crates/store/tests/` (notably
`subscription_tests.rs`, `phase_subscription_tests.rs`, `decoded_view_tests.rs`,
`decoded_inline_tests.rs`, `step_stream_ext_tests.rs`, `export_inline_tests.rs`,
`bug_hunt_tests.rs`, `property_tests.rs`, `adversarial_property_tests.rs`)
updates from `(pos, env)` / `(pos, decoded)` to `(pos, key, env)` /
`(pos, key, decoded)`. Find them with:

```bash
rg -n "read_all|subscribe_all" crates/store/tests/ -l
```

then in each hit follow the values: bindings, type ascriptions
(`Vec<(P, PersistedEnvelope)>` → `Vec<(P, StreamKey, PersistedEnvelope)>`), and
tuple-index accesses (`.1` on an `$all` item is now `.2`). Bind `_key` where the
test doesn't assert it. In at least ONE existing `$all` ordering test per file
add a positive attribution assertion (key equals the appended stream), same
shape as Task 3 Step 5's `keys` assert.

- [ ] **Step 3: Run the full store suite**

Run: `nix develop -c cargo nextest run -p mnesis-store --all-features`
Expected: PASS.

- [ ] **Step 4: Commit (Tasks 2–5 together — the store crate is one coherent change)**

```bash
git add -A crates/store
git commit -m "feat(store)!: \$all items carry the StreamKey — (AllPosition, StreamKey, PersistedEnvelope) (#333)"
```

Note: the workspace still fails to build (adapters pending); the pre-commit
hook runs `nix flake check` on the whole tree, so this commit CANNOT land yet.
**Do Tasks 6–9 first, then commit everything in one commit** — amend this step:
stage `crates/store` but defer the `git commit` until Task 9 Step 5. (The hook
is all-or-nothing per commit; a mid-ripple commit is impossible by
construction. Log this in the deviation log if you split differently.)

---

### Task 6: in-memory adapter

**Files:**
- Modify: `adapters/inmemory/src/lib.rs` (`global_index`, append, `read_all`, `InMemoryAllStream` item; inline docs)
- Modify: `adapters/inmemory/tests/inmemory_store_tests.rs` (destructures)

- [ ] **Step 1: Carry the key in the `$all` index**

- `global_index: Arc<Mutex<BTreeMap<InMemoryAllPos, StoredFrame>>>` →
  `Arc<Mutex<BTreeMap<InMemoryAllPos, (StreamKey, StoredFrame)>>>` (both
  declaration `:164` and construction `:182`, and the `AllReadState` field `:237`).
- In `append` (`:445` region): insert `(id.clone(), frame.clone())` where the
  frame alone was inserted.
- In the `$all` batch/collect path (`:251` region) and `InMemoryAllStream`
  (`:286-317`): items become
  `Result<(InMemoryAllPos, StreamKey, PersistedEnvelope), InMemoryStoreError>`,
  building `(pos, key.clone(), frame_to_envelope(&frame)?)`.

- [ ] **Step 2: Update the adapter's own tests**

Same mechanical sweep as Task 5 Step 2 over
`adapters/inmemory/tests/inmemory_store_tests.rs` + any `$all` inline tests.

- [ ] **Step 3: Verify**

Run: `nix develop -c cargo nextest run -p mnesis-inmemory --all-features`
Expected: PASS (its conformance suite may fail until Task 9 updates the kit —
if so, note it and proceed; the kit update lands before the commit).

---

### Task 7: fjall adapter — id in the `events_global` key (A2)

*(If Task 1 chose A1, swap Steps 1–3 for the value-wrap equivalent and log it.)*

**Files:**
- Modify: `adapters/fjall/src/wire_key.rs:99-135` (global key codec) + its tests
- Modify: `adapters/fjall/src/plan.rs` (`StagedRow.global_key`, `plan_run`) + its tests
- Modify: `adapters/fjall/src/scan.rs` (`ScanStrategy::upper_key`, `GlobalScan`) + its tests
- Modify: `adapters/fjall/src/store.rs` (white-box tests `:678-830` region)
- Modify: `adapters/fjall/tests/*.rs` (destructures)

- [ ] **Step 1: TDD the key codec — write the failing tests first**

In `wire_key.rs` tests, replace/extend the global-key tests:

```rust
    #[test]
    fn global_key_round_trips_with_id() {
        let key = encode_global_key(42, b"stream-7", 7).unwrap();
        let (gs, id, ver) = decode_global_key(&key).unwrap();
        assert_eq!(gs, 42);
        assert_eq!(id, b"stream-7");
        assert_eq!(ver, 7);
    }

    #[test]
    fn global_key_sorts_by_global_seq_regardless_of_id() {
        let k1 = encode_global_key(1, b"zzzzzzzz", 9).unwrap();
        let k2 = encode_global_key(2, b"a", 1).unwrap();
        assert!(k1 < k2, "8-byte BE global_seq prefix must dominate ordering");
    }

    #[test]
    fn global_key_rejects_old_16_byte_layout() {
        // Pre-#333 layout: [u64 gs][u64 ver] — must be a typed decode error,
        // never a misparse (the documented clean-break defense).
        let mut old = [0u8; 16];
        old[0..8].copy_from_slice(&42u64.to_be_bytes());
        old[8..16].copy_from_slice(&7u64.to_be_bytes());
        assert!(decode_global_key(&old).is_err());
    }

    #[test]
    fn global_key_rejects_id_len_mismatch() {
        let mut key = encode_global_key(1, b"abc", 1).unwrap();
        // Corrupt the id_len field (offset 8..10) to overclaim.
        key[8..10].copy_from_slice(&u16::MAX.to_be_bytes());
        assert!(decode_global_key(&key).is_err());
    }
```

Also extend the existing proptest round-trip (if present for global keys) to
ids of length 1, 2, `u16::MAX` (boundary values, rule 8).

- [ ] **Step 2: Run to verify they fail**

Run: `nix develop -c cargo test -p mnesis-fjall --lib wire_key`
Expected: FAIL (wrong arity / old layout still accepted).

- [ ] **Step 3: Implement the codec**

Replace `wire_key.rs:99-135`:

```rust
/// Fixed part of a `$all` index key: `[u64 BE global_seq][u16 BE id_len]`
/// before the id, `[u64 BE version]` after it.
const GLOBAL_KEY_PREFIX_SIZE: usize = 8 + 2;
const GLOBAL_KEY_VERSION_SIZE: usize = 8;

/// Encode an `events_global` key as
/// `[u64 BE global_seq][u16 BE id_len][id_bytes][u64 BE version]`.
///
/// `global_seq` alone is unique per event, so the 8-byte BE prefix fully
/// determines sort order — the id and version behind it are payload carried in
/// the key (never order-determining), which keeps the row *value* the same
/// shared frame bytes as the `events` partition (#333, layout A2). `version`
/// is carried so the read path can reconstruct a `PersistedEnvelope`.
///
/// # Errors
///
/// Returns [`EncodeError::IdTooLong`] if `id` exceeds `u16::MAX` bytes.
pub fn encode_global_key(global_seq: u64, id: &[u8], version: u64) -> Result<Vec<u8>, EncodeError> {
    let id_len =
        u16::try_from(id.len()).map_err(|_| EncodeError::IdTooLong { len: id.len() })?;
    let mut buf =
        Vec::with_capacity(GLOBAL_KEY_PREFIX_SIZE + id.len() + GLOBAL_KEY_VERSION_SIZE);
    buf.extend_from_slice(&global_seq.to_be_bytes());
    buf.extend_from_slice(&id_len.to_be_bytes());
    buf.extend_from_slice(id);
    buf.extend_from_slice(&version.to_be_bytes());
    Ok(buf)
}

/// Decode an `events_global` key into `(global_seq, id_bytes, version)`.
///
/// # Errors
///
/// Returns [`DecodeError::ValueTooShort`] if `key` is shorter than the fixed
/// parts, or [`DecodeError::InvalidSize`] if the claimed `id_len` does not
/// match the remaining bytes exactly — which structurally rejects the
/// pre-#333 16-byte `[gs][version]` layout (documented clean break).
pub fn decode_global_key(key: &[u8]) -> Result<(u64, &[u8], u64), DecodeError> {
    let min = GLOBAL_KEY_PREFIX_SIZE + GLOBAL_KEY_VERSION_SIZE;
    if key.len() < min {
        return Err(DecodeError::ValueTooShort {
            min,
            actual: key.len(),
        });
    }
    let global_seq = u64::from_be_bytes([
        key[0], key[1], key[2], key[3], key[4], key[5], key[6], key[7],
    ]);
    let id_len = usize::from(u16::from_be_bytes([key[8], key[9]]));
    let expected_total = GLOBAL_KEY_PREFIX_SIZE
        .checked_add(id_len)
        .and_then(|n| n.checked_add(GLOBAL_KEY_VERSION_SIZE))
        .ok_or(DecodeError::InvalidSize {
            expected: usize::MAX,
            actual: key.len(),
        })?;
    if key.len() != expected_total {
        return Err(DecodeError::InvalidSize {
            expected: expected_total,
            actual: key.len(),
        });
    }
    let id_bytes = &key[GLOBAL_KEY_PREFIX_SIZE..GLOBAL_KEY_PREFIX_SIZE + id_len];
    let version_start = GLOBAL_KEY_PREFIX_SIZE + id_len;
    let version = u64::from_be_bytes([
        key[version_start],
        key[version_start + 1],
        key[version_start + 2],
        key[version_start + 3],
        key[version_start + 4],
        key[version_start + 5],
        key[version_start + 6],
        key[version_start + 7],
    ]);
    Ok((global_seq, id_bytes, version))
}
```

(Note: the old 16-byte key decodes as `id_len` from bytes 8–9 of the version
field; `expected_total` then ≠ 16 for every possible value — 16 would require
`id_len` = −2 — so rejection is structural, matching the spec.)

Run: `nix develop -c cargo test -p mnesis-fjall --lib wire_key`
Expected: PASS.

- [ ] **Step 4: `plan.rs` — stage the id into the key**

- `StagedRow.global_key: [u8; 16]` → `pub global_key: Vec<u8>`.
- At `plan.rs:123`:

```rust
        let global_key =
            encode_global_key(global_seq, id_bytes, version).map_err(PlanError::from)?;
```

(match the error-mapping style `plan_run` already uses for
`encode_event_key` at the same site — the id was already length-validated
there, so this arm is defensively unreachable but typed).
- Update `plan.rs` inline tests (`:258-261`) to the new arity:
  `encode_global_key(1, b"s", 1).unwrap()`.

- [ ] **Step 5: `scan.rs` — decode the key into the 3-tuple; unbounded upper**

- `ScanStrategy::upper_key` → `fn upper_key(&self) -> Result<Option<Vec<u8>>, FjallError>;`
  — `StreamScan` wraps its existing result in `Some`, `GlobalScan` returns
  `Ok(None)` (the `events_global` partition holds only `$all` keys, so the
  scan is unbounded above; a fixed-width max key no longer exists with a
  variable-length id in the key).
- `ScanCursor::open` builds the range accordingly:

```rust
        let lower = strategy.lower_key(from)?;
        let iter = match strategy.upper_key()? {
            Some(upper) => keyspace.inner().range(lower..=upper),
            None => keyspace.inner().range(lower..),
        };
```

- `GlobalScan::lower_key` — the empty-id lower bound sorts before every real
  key with the same `global_seq` (real `id_len` ≥ 1 because fjall rejects
  empty stream keys at append):

```rust
    fn lower_key(&self, from: Self::Position) -> Result<Vec<u8>, FjallError> {
        encode_global_key(from.as_u64(), b"", 0).map_err(|e| FjallError::InvalidInput {
            stream_id: ErrorId::default(),
            version: Some(from.as_u64()),
            reason: reason_label(&e),
        })
    }
```

(adjust the `InvalidInput` field shape to the actual variant — check
`error.rs`; the encode cannot actually fail for an empty id, but the arm stays
typed, never `unwrap`.)
- `GlobalScan::Item = (GlobalSeq, StreamKey, PersistedEnvelope)`; in `decode`
  extract the id **zero-copy** from the key's `Slice`:

```rust
    fn decode(&self, key: &Slice, value: Slice) -> Result<Self::Item, FjallError> {
        let (key_global_seq, id_bytes, version_raw) =
            decode_global_key(key).map_err(|_| FjallError::CorruptValue {
                stream_id: ErrorId::default(),
                version: None,
            })?;
        let id_len = id_bytes.len();
        let position = GlobalSeq::new(key_global_seq).ok_or_else(|| FjallError::CorruptValue {
            stream_id: ErrorId::default(),
            version: Some(version_raw),
        })?;

        // Zero-copy: the key Slice is Arc-backed Bytes (bytes_1); subslice the
        // id out of it rather than copying.
        let key_bytes: Bytes = key.clone().into();
        let stream = StreamKey::from_bytes(
            key_bytes.slice(GLOBAL_KEY_PREFIX_SIZE..GLOBAL_KEY_PREFIX_SIZE + id_len),
        );

        let bytes_value: Bytes = value.into();
        let decoded =
            wire::decode_frame(bytes_value.as_ref()).map_err(|_| FjallError::CorruptValue {
                stream_id: ErrorId::default(),
                version: Some(version_raw),
            })?;
        let env = build_envelope(bytes_value, decoded, version_raw, ErrorId::default())?;
        Ok((position, stream, env))
    }
```

`GLOBAL_KEY_PREFIX_SIZE` must be exported `pub(crate)` from `wire_key.rs` for
this. Import `mnesis_store::StreamKey` in `scan.rs`.
- Update `scan.rs` inline tests: `global_row` helper gains an id parameter
  (`encode_global_key(global_seq, id, version).unwrap()`),
  `global_decode_yields_position_tagged_envelope` asserts the key:

```rust
        let (k, v) = global_row(b"user-1", 42, 7, "Created", b"data");
        let (pos, stream, env) = GlobalScan.decode(&k, v).unwrap();
        assert_eq!(stream.as_bytes(), b"user-1");
```

and `scan_cursor_global_yields_ascending_global_seq` additionally asserts the
per-item keys are `[a, b, a, b]`.
- Add the **clean-break defense test** (white-box, this file or `store.rs`'s
  test mod): insert an old-layout 16-byte key + valid frame directly into
  `events_global` (the existing white-box insertion helper at
  `store.rs:703` region), then `read_all` → the item is
  `Err(FjallError::CorruptValue { .. })` — never a misparse, never a skip.

- [ ] **Step 6: Sweep fjall integration tests + white-box store tests**

`store.rs:822` (`append_writes_events_global_index`): the expected key becomes
`encode_global_key(1, b"<the test's id bytes>", 1).unwrap()`. Then the
mechanical `(pos, env)` → `(pos, key, env)` sweep over
`adapters/fjall/tests/` (`subscription_tests.rs`, `projection_tests.rs`,
`export_import_tests.rs`, `property_tests.rs`, `resilience_tests.rs`,
`state_machine_tests.rs` — find with `rg -n "read_all|subscribe_all"`), adding
one positive attribution assert in the main `$all` ordering test.

- [ ] **Step 7: Verify**

Run: `nix develop -c cargo nextest run -p mnesis-fjall --all-features`
Expected: PASS (kit-driven conformance tests pending Task 9, as in Task 6).

---

### Task 8: postgres adapter

**Files:**
- Modify: `adapters/postgres/src/store.rs:499-572` (`read_all`) + the `AllEventRow` struct (near `:545`'s `query_as` target)
- Modify: `adapters/postgres/tests/all_noskip_tests.rs` (destructures)

- [ ] **Step 1: Surface the column it already stores**

Add `stream_id` to the SELECT and the row struct:

```rust
        let rows: Vec<AllEventRow> = sqlx::query_as(
            "SELECT stream_id, txid::text::bigint AS txid, global_seq, \
                    version, event_type, schema_version, payload, metadata \
             FROM events \
             WHERE ($1::bigint IS NULL OR (txid::text::bigint, global_seq) > ($1, $2)) \
               AND txid < pg_snapshot_xmin(pg_current_snapshot()) \
             ORDER BY txid, global_seq",
        )
```

`AllEventRow` gains `stream_id: Vec<u8>` (the column is `BYTEA`,
`schema.rs:17`). The mapping closure (`:559-569`) becomes:

```rust
            .map(move |r| {
                let txid =
                    u64::try_from(r.txid).map_err(|_| corrupt(label, "txid out of range"))?;
                let seq = u64::try_from(r.global_seq)
                    .map_err(|_| corrupt(label, "global_seq <= 0 or out of range"))?;
                let stream = StreamKey::from_bytes(r.stream_id);
                let env = row_to_envelope(r.event, label)?;
                Ok((PgAllPos::new(txid, seq), stream, env))
            })
```

with the vec type `Vec<Result<(PgAllPos, StreamKey, PersistedEnvelope), PostgresError>>`
and the `type AllStream` declaration updated to match. `StreamKey` is already
imported in this file.

- [ ] **Step 2: Sweep `all_noskip_tests.rs`**

Same `(pos, env)` → `(pos, key, env)` sweep + one positive attribution assert
in the interleaved-streams ordering test.

- [ ] **Step 3: Verify it compiles (DB tests skip locally)**

Run: `nix develop -c cargo nextest run -p mnesis-postgres --all-features`
Expected: PASS/SKIP (no `DATABASE_URL` locally; CI's nixosTest runs them).

---

### Task 9: Conformance kit — the freeze-proof

**Files:**
- Modify: `crates/store-testing/src/sequence.rs` (existing `$all` checks + ONE new check)
- Modify: `crates/store-testing/src/linearizability.rs`, `boundary.rs`, `row.rs`, `lifecycle.rs` (any `$all` destructures)
- Modify: `crates/store-testing/src/lib.rs` (macro registration `:437-445` + the adapter-guide docs `:76-184`)
- Modify: `crates/store-testing/tests/toy_adapter.rs` (the guide-only toy adapter)

- [ ] **Step 1: Write the new attribution check (TDD — it is the contract)**

In `sequence.rs`, after `check_all_global_order_across_streams` (`:258`),
following that check's exact helper/factory/bound pattern (read it first;
reuse its append + collect helpers verbatim):

```rust
/// #333: every `$all` item carries the [`StreamKey`] of the stream it was
/// appended to — attribution is a store guarantee, not a payload convention.
/// Interleaves two streams and asserts each item's key matches its append
/// target, in position order.
pub async fn check_all_items_carry_their_stream_key<S, C, F, Fut>(factory: &F)
// … same generic bounds as check_all_global_order_across_streams …
{
    // append order: alpha@1, beta@1, alpha@2  (same seeding pattern as the
    // neighbouring check, two distinct stream keys)
    // read_all(None), collect items
    // assert, in order:
    //   [(b"alpha", 1), (b"beta", 1), (b"alpha", 2)]
    // where each element is (item.1.as_bytes(), item.2.version().as_u64())
}
```

(The body is a copy of `check_all_global_order_across_streams` with the assert
swapped to the `(key, version)` pairs above — keep the exact-value `assert_eq!`
discipline, no `contains`.)

Also extend `check_subscription_all_backlog_then_caught_up_then_live`
(`:558`): assert the key on at least the live-delivered item (the wake path
must stamp attribution too, not just catch-up).

- [ ] **Step 2: Register it in the core matrix**

`lib.rs:441` region, beside its siblings:

```rust
            $crate::__conformance_case!(sequence, check_all_items_carry_their_stream_key, $factory, $skip);
```

- [ ] **Step 3: Sweep the kit's existing `$all` destructures**

All `(pos, env)` bindings in `sequence.rs` / `linearizability.rs` /
`boundary.rs` / `row.rs` / `lifecycle.rs` → `(pos, key, env)` (bind `_key`
where not asserted). `rg -n "read_all|subscribe_all" crates/store-testing/src/`.

- [ ] **Step 4: Update the toy adapter + the guide**

`tests/toy_adapter.rs`: its `RawEventStore` impl stamps the key on its `$all`
items (it already receives `id: &StreamKey` at append — store `id.clone()`
beside each `$all` row, mirroring Task 6's in-memory change). The guide docs in
`lib.rs` (`:76-184`): document the `$all` item as
`(AllPosition, StreamKey, PersistedEnvelope)`, the attribution guarantee, and
the per-stream asymmetry (id-is-the-query-argument). The toy adapter passing
34+1 checks from the guide alone remains the acceptance criterion.

- [ ] **Step 5: Full workspace verify, then the single big commit**

```bash
nix develop -c cargo nextest run --workspace --all-features
nix develop -c cargo clippy --workspace --all-features --all-targets
nix develop -c cargo fmt --all
git add -A
git commit -m "feat(store)!: carry StreamKey on every \$all item (#333)

AllStream item becomes (AllPosition, StreamKey, PersistedEnvelope); Catchup
generalizes over an Item + position_of so adapter items flow through the live
loop verbatim; fjall stores the id in the events_global key (A2, benchmarked),
preserving the shared value bytes; old 16-byte keys reject as CorruptValue
(documented pre-1.0 clean break); postgres surfaces its existing stream_id
column; conformance kit pins attribution as check_all_items_carry_their_stream_key."
```

Expected: nextest PASS, clippy clean, hook's `nix flake check` PASS.

---

### Task 10: axum-todos evidence loop

**Files:**
- Modify: `examples/axum-todos/src/index.rs` (the `$all` stepper loop `:203-231` region)
- Modify: `examples/axum-todos/src/domain.rs` (the #326-7 finding doc comments)

- [ ] **Step 1: Confirm the stepper path compiles whole**

`index.rs`'s loop feeds `.decoded()` items to `Projection::advance`; with
Task 5's `Positioned` impl the 3-tuple feeds `advance` unchanged — verify, and
update any explicit item-type annotations in the file to the 3-tuple.

- [ ] **Step 2: Update the finding commentary**

In `domain.rs`, amend the `finding #326-7` doc comments: the `$all` path now
carries the stream key structurally (#333), so embedding `id: Uuid` in every
event variant is a **domain modeling choice** (self-describing events), no
longer a framework obligation for routability. Do not restructure the domain
events themselves — the example's payload ids remain a legitimate style.

- [ ] **Step 3: Verify + commit**

```bash
nix develop -c cargo nextest run -p axum-todos
nix develop -c cargo fmt --all
git add examples/axum-todos
git commit -m "docs(examples): axum-todos — #326-7 finding resolved by #333 stream attribution"
```

*(Examples are outside the flake's `--lib` clippy gate; run
`nix develop -c cargo clippy -p axum-todos --all-targets` by hand.)*

---

### Task 11: Docs sweep + PR

**Files:**
- Modify: `CLAUDE.md` (architecture notes: `AllStream` item shape, fjall `events_global` key layout, subscription/decoded/projection composition mentions)
- Modify: this plan (decision record + deviation log finalized)

- [ ] **Step 1: Update CLAUDE.md**

Every mention of the `$all` item `(AllPosition, PersistedEnvelope)` / the
`events_global` key `[global_seq][version]` updates to the new shapes
(`rg -n "AllPosition, PersistedEnvelope|global_seq\]\[version" CLAUDE.md`).
Add one sentence to the `store.rs` bullet recording the #333 decision
(attribution = store guarantee; per-stream asymmetry intentional). Do NOT fix
unrelated stale notes (tracked separately by the wire-V2/read-all-bounds
memory) — scope discipline.

- [ ] **Step 2: Commit + PR**

```bash
git add CLAUDE.md docs/superpowers/plans/2026-07-18-333-all-stream-attribution.md
git commit -m "docs: record #333 \$all attribution decision in CLAUDE.md"
git push -u origin feat/333-all-stream-attribution
gh pr create --title "feat(store)!: \$all items carry their StreamKey (#333)" --body-file - <<'EOF'
Closes #333. Supersedes #217.

## What
`RawEventStore::AllStream` items become `(AllPosition, StreamKey, PersistedEnvelope)` — stream attribution on the `$all` path is a store guarantee, not a payload convention.

## Why
See `docs/superpowers/specs/2026-07-18-333-all-stream-attribution-design.md` (freeze asymmetry; #326-7 field evidence; routing-before-decoding; wire-V2 "identity beside the envelope" precedent).

## Layout decision (rule 9)
<paste Task 1 bench numbers: A2 vs A1 insert time + on-disk size>

## Breaking
- `AllStream` item shape (read_all / subscribe_all / .decoded() / Positioned)
- fjall `events_global` key layout — pre-1.0 clean break; old 16-byte keys reject as typed `CorruptValue` (index is derived data, rebuildable)

## Conformance
New kit check `check_all_items_carry_their_stream_key` pins attribution; toy adapter passes from the guide alone.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
```

Use the `joeldsouzax` gh account. Merge later via
`gh pr merge --squash --delete-branch` after CI (Nix Flake Check + nixosTest
postgres) is green and the user approves.

---

## Decision record (filled by Task 1)

Benchmark: `nix develop -c cargo bench -p mnesis-fjall --bench all_index_layout`
(20 000 events, 120-byte payloads, 100 streams cycling through 36-byte
uuid-string-sized ids, `events_global`'s production partition config —
32 KiB blocks + all-levels LZ4). Logical size for both layouts: 3.93 MiB.

| Layout | Insert time (criterion, 20k events) | On-disk size (`disk_space()`) | Chosen |
|---|---|---|---|
| A2 (id in key, shared value) | 28.461 ms [27.607, 29.315] | 0.36 MiB (×0.092 of logical) | **✓** |
| A1 (id in value wrap) | 28.669 ms [28.304, 29.065] | 0.37 MiB (×0.094 of logical) | — |

Delta (A1 vs A2): **+2.0%** on-disk, **+0.7%** insert time — both well under
the 5% threshold, and both point the same direction (A1 worse, not better).
(Numbers re-measured after a review fix: the criterion routine now returns the
`(TempDir, keyspace)` tuple so database shutdown + directory deletion drop
outside the timed window; the first run had timed that teardown in both
layouts, inflating both by ~7 ms equally.)
Per the decision rule (default A2; switch to A1 only if A2 is worse by >5% on
either axis), **A2 is confirmed**: id in the key, value stays the shared frame
`Slice` clone with zero extra per-append allocation. Task 7 proceeds with A2
as planned (no swap needed).

## Deviation log

| Date | Deviation | Reason | Impact |
|---|---|---|---|
| 2026-07-18 | Bench mirrors `mnesis_fjall::partition::scan_defaults()` inline (`events_global_config()` in the bench file) rather than importing it | `partition` is a private module (`mod partition`, not `pub mod`) in `mnesis-fjall`'s `lib.rs` — only `AllIndex`/`KeyspaceConfig` are re-exported, so a `benches/` file (a separate crate) cannot reach `scan_defaults` directly. Same workaround `projection_storage.rs` already uses for `projections`/`snapshots` configs. | None on the measurement — the mirrored options are copied verbatim (32 KiB blocks, all-levels LZ4, no bloom); if `scan_defaults()`'s tuning ever changes, this bench's copy must be updated in lockstep (same maintenance burden `projection_storage.rs` already carries). |
| 2026-07-18 | Tasks 2–6 + kit mechanical sweep executed as one unit; kit's new check deferred to Task 9 | dev-dep chain: store tests need inmemory; adapter tests need store-testing to compile | none on final state; Task 9 still owns the new check, macro registration, guide docs, and the commit |
