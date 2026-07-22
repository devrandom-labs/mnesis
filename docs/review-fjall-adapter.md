# Code review — `mnesis-fjall` adapter (`adapters/fjall/src/`)

**Scope.** Fjall **usage** and Rust **idiom**, not the storage design. The
partition layout, per-partition LZ4, one-atomic-`write_tx` model, and
structural-not-fsync durability are settled (rules 1 & 9) and out of scope
— no proposals to change them appear below.

**Verdict up front.** The production code is *genuinely good*, not merely
clippy-clean. Fjall's API is used correctly: exactly one `write_tx` per
atomic batch, no check-then-act inside a tx, scans stay lazy over a single
`fjall::Iter`, and the `bytes_1` zero-copy `Slice`/`Bytes` path is taken on
every hot read and write. It is idiomatic (`?`/`thiserror`, `let … else`,
combinators, borrow-before-own) and rule-compliant (`checked_add` everywhere,
distinct corruption vs input-validation error domains, two cleanly separated
`SnapshotStore` instantiations). The findings below are all LOW/INFO hygiene
items — no correctness bug, no fjall-misuse, no optimizer-leaning debt was
found. Each was checked against the code, not assumed.

---

## Ranked findings

| Severity | file:line | What's wrong | Idiomatic / correct fix |
| ---------- | ----------- | -------------- | ------------------------- |
| LOW | `partition.rs:137,149` (+ `:164,251,268,273,280,288,294,306,321,332,339,346`) | `Partitions` is declared `pub struct` and exposes ~12 `pub` methods, but `partition` is a **private module** (`mod partition;` in `lib.rs:66`). The `pub` items are therefore only crate-visible — no public leak today — yet the `pub` contracts mislead a reader into thinking `Partitions` is part of the adapter surface. | Mark the struct `pub(crate)` and the internal accessors `pub(crate)` (keep `AllIndex`/`KeyspaceConfig` `pub` — they are re-exported in `lib.rs:78`). No behavior change. |
| LOW | `scan.rs:38-41` | `StreamScan { pub id, pub label }`, `pub struct GlobalScan`, `pub trait ScanStrategy` live in the private `scan` module. The `pub` visibility is dead weight outside the crate and invites accidental re-export. | Change to `pub(crate)` (the associated type `ScanCursor<StreamScan>` in `store.rs:103` keeps working). |
| LOW | `store.rs:264,347` | `hydrate` does `state: payload.to_vec()` — a genuine `Slice → Vec` copy of the snapshot/projection blob. Not a hot path (best-effort, read-on-demand), so acceptable, but it is a real copy of bytes fjall already hands us zero-copy. | Acceptable as-is given the `Hydrated<Vec<u8>>` contract requires owned bytes; if it ever becomes hot, consume the `Slice` in place rather than `decode_snapshot_value(&bytes)` + re-copy. No change required now. |
| INFO | `scan.rs:201-208` | `open_empty` uses a reversed byte range `range(vec![1u8]..=vec![0u8])` to yield "nothing". Correct today (fjall returns empty for `start > end`), but it leans on reversed-range semantics as a sentinel. | Document the invariant explicitly, or expose an `AllPosition::ceiling`-style empty case. Cosmetic; leave as-is unless fjall ever rejects reversed ranges. |
| INFO (doc) | `CLAUDE.md:124` | Prose says `read_all`'s `from` is **inclusive** ("both from bounds are inclusive"). The actual `RawEventStore::read_all` contract (`crates/store/src/store.rs:236-239`) is deliberately **exclusive** ("the adapter reads 'strictly greater'"), and `store.rs:202` implements exactly that. The CLAUDE.md line is stale and contradicts the trait doc. | Fix the CLAUDE.md sentence to say `read_all` is exclusive (asymmetric with `read_stream`'s inclusive `Version` `from`, CLAUDE rule 4). Not adapter code. |

No HIGH/MEDIUM findings. The skeptic's three hypotheses (split `write_tx`,
eager `collect`, needless `Slice → Vec`/`clone` on the hot path) were all
checked and **refuted with evidence** below.

---

## Per-file verdict

### `store.rs` — **genuinely good**

- `append` (`store.rs:111-170`): one `write_tx` (`let mut tx = self.db.write_tx();`, :122) spanning the version check, the plan, every `stage_event`, both counter advances, and `tx.commit()` (:161). A stale `expected_version` is caught *inside* the tx before any staging (:124-128) and even on an empty batch (:119-133) — so conflict detection does not require a write. No second tx, no per-stream commit loop. ✓ rule 1.
- No check-then-act-on-the-same-key: `read_version`/`read_global` read; `set_version`/`set_global`/`stage_event` write *different* keys. The optimistic-concurrency read and the staged write are distinct rows, so the rule-1 "don't re-read a key you staged" smell is absent. ✓
- `read_stream`/`read_all` return a `ScanCursor` (lazy `Stream`) — never an eager `Vec`. ✓
- `atomic_append_many` (`store.rs:523-550`): the whole chunk lands in **one** `write_tx`; any early `Err` drops `tx` uncommitted so nothing lands across any partition. `validate_atomic_writes` (`:425`) tracks a running projected head in a `HashMap` so a non-injective route (two writes → one stream) conflicts instead of concatenating. ✓ rule 1.
- Two `SnapshotStore` impls are **distinct trait instantiations** (`snapshot_impl` `P = Version`, `projection_impl` `P = GlobalSeq`), backed by **separate partitions** (`snapshots` vs `projections`) so ids can never collide (`store.rs:302-319`). Cleanly separated, as documented. ✓
- `WakeSource` delegates to `StreamNotifiers`; `wake` is called **after** the durable commit (`:167`, `:546`). ✓

### `partition.rs` — **genuinely good** (encapsulation nit only)

- `stage_event` (`partition.rs:241-247`) is the **single** denormalization site: `Slice::from(row.frame.clone())` (Arc bump, zero data copy) is inserted into `events` and, under `AllIndex::Denormalized`, into `events_global`. The `bytes_1` `Slice`/frame is the same Arc as the `Bytes` in `StagedRow.frame` — no memcpy. `slice.clone()` for the first insert + move for the second is the minimal possible refcount churn. ✓ zero-copy hot path.
- `read_version`/`read_global` (`partition.rs:195-228`) consume the `Option<Slice>` via `.as_ref()` / `try_into::<[u8;8]>()` — no `to_vec`. Wrong-sized values map to `FjallError::CorruptMeta`, never `unwrap`. ✓
- `stream_ids()` (`:280`) returns a raw `fjall::Iter` for the export lister — lazy, not a `collect`. ✓
- Only nit: `pub` on a private-module type (see table).

### `scan.rs` — **genuinely good**

- `ScanCursor` (`scan.rs:164-243`) wraps **one** lazy `fjall::Iter` opened with `range(lower..=upper)` (`open`, :189); `poll_one` pulls one row at a time (`:210-228`) — no eager materialization, snapshot pinned at `open`. ✓
- Zero-copy decode: `let bytes_value: Bytes = value.into();` (`:107,143`) converts the `Slice` to `Bytes` with `bytes_1` (no copy); `wire::decode_frame(bytes_value.as_ref())` borrows; `build_envelope(bytes_value, …)` moves the buffer into the `PersistedEnvelope`. No re-decode of the same bytes. ✓
- `StreamIdCursor` (`stream_lister_impl`, `store.rs:573-605`): `Bytes::from(key)` from the key-only `iter.next()?.key()` read — zero-copy and key-only (never reads the version-counter value). ✓
- Poison-on-error (`:211-227`) stops the cursor rather than silently skipping a corrupt row. ✓
- Only nit: `pub` on private-module items (see table).

### `plan.rs` — **genuinely good**

- `plan_run` (`plan.rs:82-140`) is the single IO-free encode core shared by both `append` and `atomic_append_many` (`:494`) — one implementation, no drift. ✓ DRY.
- Strict-sequential validation via a **running `checked_add`** counter (`:96,104-106`): `expected.checked_add(1).ok_or(PlanError::VersionOverflow)` — no `unwrap_or(u64::MAX)`, no index→u64 cast. ✓ rule 2.
- `PlanError` is a neutral failure mapped into each caller's domain (`append_plan_err` `store.rs:82`, `atomic_plan_err` `store.rs:392`); overflow maps to `Store(VersionOverflow/GlobalSeqOverflow)`, **never** `Conflict`. ✓ rule 3 (one variant = one domain).

### `global_seq.rs` — **genuinely good**

- `GlobalSeq(NonZeroU64)`; `next()` uses `self.0.checked_add(1)` (`:30-35`); `new()` rejects `0` via `NonZeroU64::new` (`:45-50`). No sentinel, no `unwrap_or`. ✓ rule 2.

### `wire_key.rs` — **genuinely good**

- `decode_event_key`/`decode_global_key`/`decode_stream_version` reject wrong sizes via typed `DecodeError` (`ValueTooShort`/`InvalidSize`), never `unwrap`. `encode_event_key` bounds id length with `u16::try_from` → `EncodeError::IdTooLong`. ✓ rule 2 / no panic on decode.
- Length-prefixed event key (`[u16 BE id_len][id][u64 BE ver]`, `:45-97`) prevents prefix collisions in range scans — the reason `read_stream`'s `upper_key = encode_event_key(id, u64::MAX)` correctly bounds one stream. ✓

### `snapshot.rs` — **genuinely good**

- Blob codec `[u32 LE schema_version][u64 BE position][payload]`; `decode_snapshot_value` returns `&[u8]` into the `Slice` (zero-copy borrow), bounds-checked against `SNAPSHOT_VALUE_HEADER_SIZE`. The `u64` position field is read by both impls via the same shape (`store.rs:240,320`). ✓

### `error.rs` / `builder.rs` / `subscription_id.rs` / `lib.rs` — **genuinely good**

- `FjallError` (`error.rs:18-79`) keeps corruption (`CorruptValue`/`EnvelopeCorrupt`/`CorruptMeta`) distinct from `InvalidInput` (write-path validation *before* any byte is written) — the rule-3 "corruption ≠ input-validation" split, applied consistently. `#[non_exhaustive]` on the public error enum is correct per the 1.0 freeze carve-out (rule 3 addendum). Diagnostic fields are `ErrorId`/`ErrorId<128>` — no heap on error paths. All fjall `Result`s are mapped via `.map_err(FjallError::Io)?`, never `unwrap`/`expect` (confirmed by grep: every `unwrap`/`expect` in the crate is `#[cfg(test)]`). ✓
- `builder.rs:109-151` opens all partitions in one `open()`, applies the config closures by monomorphization (no `Box<dyn>`), and pairs `events`/`events_global` off one `events_config` so the `$all` twin can't silently diverge. ✓
- `subscription_id.rs` keeps `OwnedStreamId` crate-local by module containment (`pub struct` in a private mod) — satisfies `Id`'s `'static` bound across subscription refills without leaking. ✓
- `lib.rs` exports exactly the intended surface (`FjallStoreBuilder`, `FjallError`, `GlobalSeq`, `AllIndex`, `KeyspaceConfig`, `FjallStore`); `Partitions`/`ScanCursor`/`StagedRow` are private. ✓

---

## Claims verified (the skeptic's checklist)

- **Split `write_tx`?** No. `grep` for `write_tx` shows exactly one `self.db.write_tx()` in `append` (`store.rs:122`) and one in `atomic_append_many` (`store.rs:535`); both span the full batch and commit once. ✓
- **Eager `collect()` of a scan?** No. `grep` for `collect()` in non-test code returns nothing. Both `ScanCursor` and `StreamIdCursor` are lazy `Stream`s pulling one `fjall::Iter` row per `poll`. ✓
- **Needless `Slice → Vec`/`clone` on hot path?** `grep` for `to_vec()` in production code finds only the two snapshot/projection `hydrate` copies (`store.rs:264,347`) — non-hot. Every event-path read/write goes through `Slice`/`Bytes` with `bytes_1` (`stage_event` `partition.rs:242-245`, `decode` `scan.rs:107,143`, `StreamIdCursor` `store.rs:589`). ✓
- **`unwrap`/`expect` on a fjall `Result`?** None in production code. The only `unwrap`/`expect` are inside `#[cfg(test)]` mods. Fjall `Result`/`FjallError` is always `.map_err(FjallError::Io)?`. ✓
- **Bare arithmetic on `Version`/`GlobalSeq`?** None. Every counter advance is `checked_add`; overflow is a typed `VersionOverflow`/`GlobalSeqOverflow`/`PlanError::*Overflow`, never a sentinel. ✓
- **`read_all` exclusive vs `read_stream` inclusive — bug?** Verified **not** a bug: the `RawEventStore::read_all` trait contract (`crates/store/src/store.rs:236-239`) is deliberately exclusive ("the adapter reads 'strictly greater'"), an intentional asymmetry with `read_stream`'s inclusive `Version` `from`. `store.rs:202` matches the contract exactly. The only defect is a stale sentence in `CLAUDE.md:124` (listed in the table). ✓

---

## Bottom line

The adapter does **not** lean on the optimizer and does **not** carry
fjall-misuse or non-idiomatic debt. The only actionable items are visibility
hygiene (`pub` → `pub(crate)` on private-module items) and one stale doc
sentence — all LOW/INFO, none correctness-affecting. No code change is
required to ship; the optional second pass (low-risk idiom fixes) would be
limited to the `pub(crate)` visibility cleanups above.
