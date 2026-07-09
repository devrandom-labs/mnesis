# no_std nexus-store (#301) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Port `nexus-store` to `no_std` + `alloc` behind an additive default `std` feature, switching all public error bounds to `core::error::Error` (the freeze-critical, post-1.0-irreversible piece), mirroring the merged #279 kernel port.

**Architecture:** `#![cfg_attr(not(feature = "std"), no_std)]` + unconditional `extern crate alloc` (the store always needs an allocator — `Vec`, `Arc`, `Bytes`). All production `std::` paths move to `core::`/`alloc::`; `#[cfg(test)]` code keeps `std::` freely (tests always build with the default `std` feature). Deps flip to `default-features = false` at the workspace root (Cargo cannot override `default-features` through workspace inheritance — verified against the Cargo reference, stated in #301); std consumers re-add the features they lose. `nexus-store` joins the hakari `[final-excludes]` because `workspace-hack` force-enables std-implying `futures-util` features (`io`, `channel`, `sink`), which feature-unification would silently leak into the no_std build.

**Tech Stack:** Rust stable (pinned via `rust-toolchain.toml`), `core::error::Error` (stable since 1.81), cargo-hakari, Nix flake CI gate (wasm32-unknown-unknown + host `--no-default-features`).

**Non-breaking note:** `std::error::Error` has been a re-export of `core::error::Error` since Rust 1.81 — the bounds swap is semantically a no-op for std consumers. The PR still carries `!` (mirrors #303): the dependency default-feature flips can affect `--no-default-features` consumers of the workspace deps.

**Scope decisions (flag at review):**
1. The `std` feature forwards `thiserror/std`, `nexus/std`, `futures/std`, `bytes/std`, `aligned-vec/std` — strict additivity: the default build produces byte-identical dep configurations to today, so "no regression on the std path" holds by construction.
2. The wasm32 gate also builds the dep-free features (`subscription,export,import,snapshot,projection`) under `--no-default-features`, because this plan ports those modules' imports too and without a gate they'd rot. **Empirical checkpoint:** if `futures`' `alloc`-only combinator set (`unfold`, `try_fold`, …) turns out to need `std`, drop the features-variant from the gate and record the deviation here.
3. Optional codec features (`serde`, `json`, `rkyv`, `bytemuck`, `cbor`) stay orthogonal to `std` — they are not in the gate and not claimed no_std-compatible. `crc32c` (behind `cbor`) exposes **no** feature toggles at all, so `cbor` cannot be no_std today; out of scope.

**Deviation log:** record divergences here as they happen (per project convention for multi-step plans).

---

### Task 1: Branch setup

**Files:** none (git only)

- [ ] **Step 1: Fetch and branch off origin/main** (never stale local main — project rule)

```bash
git fetch origin
git checkout -b feat/301-no-std-store origin/main
```

- [ ] **Step 2: Verify clean state**

Run: `git status`
Expected: `nothing to commit, working tree clean`, branch `feat/301-no-std-store`

---

### Task 2: Workspace dependency flips + std-consumer restores

**Files:**
- Modify: `Cargo.toml` (workspace root, `[workspace.dependencies]` lines 24–32)
- Modify: `crates/nexus-fjall/Cargo.toml` (line 22)
- Modify: `crates/nexus-inmemory/Cargo.toml` (lines 19–20)
- Modify: `crates/nexus-postgres/Cargo.toml` (lines 15–16)
- Modify: `crates/nexus-store/Cargo.toml` (dev-dependency `futures`, line 61)

- [ ] **Step 1: Flip workspace declarations to `default-features = false`**

In root `Cargo.toml` `[workspace.dependencies]`:

```toml
aligned-vec = { version = "=0.6.4", default-features = false }
bytes = { version = "1", default-features = false }
futures = { version = "0.3", default-features = false, features = ["alloc"] }
```

(`thiserror` is already `default-features = false` from #279; `futures-core` already is; `arrayvec` already is; `minicbor` already uses `features = ["alloc"]` with no default pull.)

- [ ] **Step 2: Restore lost default features in std consumers**

`futures`' default set is `["std", "async-await", "executor"]`; `bytes`' and `aligned-vec`'s is `["std"]`. Restore exactly that set so consumer builds are unchanged:

`crates/nexus-fjall/Cargo.toml`:
```toml
bytes = { workspace = true, features = ["std"] }
```

`crates/nexus-inmemory/Cargo.toml`:
```toml
bytes = { workspace = true, features = ["std"] }
futures = { workspace = true, features = ["std", "async-await", "executor"] }
```

`crates/nexus-postgres/Cargo.toml`:
```toml
bytes = { workspace = true, features = ["std"] }
futures = { workspace = true, features = ["std", "async-await", "executor"] }
```

`crates/nexus-store/Cargo.toml` `[dev-dependencies]` (tests always link std):
```toml
futures = { workspace = true, features = ["std", "async-await", "executor"] }
```

- [ ] **Step 3: Check the workspace still builds**

Run: `cargo check --workspace`
Expected: clean. If a crate fails on a missing futures/bytes item, that consumer needs the corresponding feature added — fix it there (deviation-log it), never by re-enabling workspace defaults.

---

### Task 3: nexus-store feature surface + hakari exclusion

**Files:**
- Modify: `crates/nexus-store/Cargo.toml` (`[features]` + `[dependencies]`)
- Modify: `.config/hakari.toml` (`[final-excludes] workspace-members`)
- Modify (generated): `crates/workspace-hack/Cargo.toml`, `Cargo.lock`

- [ ] **Step 1: Add the additive `std` default feature**

In `crates/nexus-store/Cargo.toml` `[features]`:

```toml
default = ["std"]
# Additive: enabling `std` only ADDS (the std::error::Error bridge via
# thiserror + the deps' std impls); disabling it (`--no-default-features`)
# yields a no_std + alloc + core::error::Error store. (#301, mirrors #279)
std = [
  "thiserror/std",
  "nexus/std",
  "futures/std",
  "bytes/std",
  "aligned-vec/std",
]
```

- [ ] **Step 2: Re-point the store's own deps**

In `[dependencies]`:

```toml
nexus = { version = "0.1.0", path = "../nexus", default-features = false }
thiserror = { workspace = true }
```

(`thiserror` loses its hardcoded `features = ["std"]`; `futures`/`bytes`/`aligned-vec` lines stay plain `{ workspace = true }` — the workspace decl is now no-default, and `std` forwards through the feature above.)

- [ ] **Step 3: Exclude nexus-store from workspace-hack**

In `.config/hakari.toml`, extend the existing entry:

```toml
workspace-members = ["nexus", "nexus-nostd-smoketest", "nexus-store"]
```

Add a comment line above mirroring the existing ones:
```toml
# `nexus-store` is no_std (core+alloc) — workspace-hack force-enables
# std-implying futures-util features (io/channel/sink) that feature
# unification would leak into its --no-default-features build. (#301)
```

- [ ] **Step 4: Regenerate hakari and remove the dep edge**

```bash
cargo hakari generate
cargo hakari manage-deps
```

Expected: `workspace-hack = { … }` line removed from `crates/nexus-store/Cargo.toml`; `crates/workspace-hack/Cargo.toml` regenerated. Verify with `cargo hakari verify` → exit 0.

- [ ] **Step 5: Workspace still builds**

Run: `cargo check --workspace`
Expected: clean.

- [ ] **Step 6: Commit (workspace prep, green standalone)**

```bash
git add Cargo.toml Cargo.lock .config/hakari.toml crates/workspace-hack crates/nexus-fjall/Cargo.toml crates/nexus-inmemory/Cargo.toml crates/nexus-postgres/Cargo.toml crates/nexus-store/Cargo.toml
git commit -m "build(store): flip futures/bytes/aligned-vec to no-default workspace deps, hakari-exclude nexus-store (#301)"
```

(The pre-commit hook runs `nix flake check` itself — do not pre-run it.)

---

### Task 4: no_std attribute — make the build fail

**Files:**
- Modify: `crates/nexus-store/src/lib.rs` (top of file)

- [ ] **Step 1: Add the attribute + alloc**

At the very top of `crates/nexus-store/src/lib.rs`, before the doc comment/attrs that exist today:

```rust
#![cfg_attr(not(feature = "std"), no_std)]

// The store is alloc-dependent by design (Bytes, Vec, Arc are its working
// vocabulary) — unlike the pure-core kernel, `alloc` is unconditional.
extern crate alloc;
```

- [ ] **Step 2: Run the failing build (this is the port's "failing test")**

Run: `cargo build -p nexus-store --no-default-features`
Expected: FAIL with many `error[E0433]: failed to resolve: use of unresolved module or unlinked crate 'std'` and missing-prelude errors (`Vec`, `String`, `format!`, `Box` not found). Save the error list — it is the authoritative worklist for Task 5.

---

### Task 5: Mechanical sweep — `std::` → `core::`/`alloc::`

**Files (production code only — leave `#[cfg(test)]` modules untouched):**
- Modify: `crates/nexus-store/src/builder.rs`
- Modify: `crates/nexus-store/src/codec.rs`
- Modify: `crates/nexus-store/src/envelope.rs`
- Modify: `crates/nexus-store/src/value.rs`
- Modify: `crates/nexus-store/src/store.rs`
- Modify: `crates/nexus-store/src/state.rs`
- Modify: `crates/nexus-store/src/catchup.rs`
- Modify: `crates/nexus-store/src/subscription.rs`
- Modify: `crates/nexus-store/src/subscription_cursor.rs`
- Modify: `crates/nexus-store/src/repository.rs`
- Modify: `crates/nexus-store/src/export.rs`
- Modify: `crates/nexus-store/src/import.rs`
- Modify: `crates/nexus-store/src/snapshot.rs`
- Modify: `crates/nexus-store/src/stream.rs`
- Modify: `crates/nexus-store/src/upcasting.rs`
- Modify: `crates/nexus-store/src/wire.rs`
- Modify: `crates/nexus-store/src/cbor.rs`
- Modify (alloc imports only, as compiler demands): `error.rs`, `batch.rs`, `conflict.rs`, `step.rs`, `stream_id.rs`, `decoded.rs`, `projection.rs`, `saga.rs`, `execute.rs`

- [ ] **Step 1: Apply the path mapping (exact substitutions)**

| From | To | Known sites |
|---|---|---|
| `std::error::Error` (bounds — **the freeze gate**) | `core::error::Error` | `codec.rs:56,114,173`, `store.rs:117`, `repository.rs:86,126,363`, `stream.rs:45,51` |
| `std::sync::Arc` | `alloc::sync::Arc` | `store.rs:1`, `catchup.rs:12`, `subscription.rs:29`, `repository.rs:14` |
| `std::future::Future` | `core::future::Future` | `store.rs:194,222,255`, `state.rs:1`, `export.rs:62,96,112`, `import.rs:268,311`, `repository.rs:11` |
| `std::num::{NonZeroU32, NonZeroU64}` | `core::num::*` | `builder.rs:47,155,163,295`, `value.rs:13`, `state.rs:2`, `snapshot.rs:1`, `repository.rs:13` |
| `std::fmt` / `std::any::type_name` | `core::fmt` / `core::any::type_name` | `codec.rs:263–266` |
| `std::str::{from_utf8, from_utf8_unchecked, Utf8Error}` | `core::str::*` | `envelope.rs:40,400,446`, `value.rs:50,111,138,154` |
| `std::ops::Range` | `core::ops::Range` | `envelope.rs:1` |
| `std::ptr::eq` | `core::ptr::eq` | `envelope.rs:912` (verify cfg; test-only sites stay) |
| `std::marker::PhantomData` | `core::marker::PhantomData` | `builder.rs:1`, `repository.rs:12` |
| `std::borrow::{Borrow, Cow}` | `core::borrow::Borrow` / `alloc::borrow::Cow` | `repository.rs:10`, `upcasting.rs:1` |
| prelude `Vec`, `String`, `Box`, `format!`, `vec!`, `ToString`, `ToOwned` | explicit `use alloc::vec::Vec;`, `use alloc::string::{String, ToString};`, `use alloc::boxed::Box;`, `use alloc::borrow::ToOwned;`, `use alloc::{format, vec};` | wherever the Task 4 error list points (heaviest: `cbor.rs`, `wire.rs`, `import.rs`, `catchup.rs`, `upcasting.rs`) |

Rules while sweeping (project conventions):
- All `use` imports at the top of the file — no inline paths introduced by the sweep.
- `#[cfg(test)]` modules keep `std::` (they always build with the `std` default feature; do NOT churn them).
- Doc comments that *teach* the bound (`codec.rs:20`, `subscription.rs:50` `std::pin::pin`) get updated to the `core::` spelling for accuracy.
- Do not touch `#[non_exhaustive]` on error enums, lint levels, or anything outside the path sweep.

- [ ] **Step 2: Iterate until the no_std build is green**

Run (repeat until clean): `cargo build -p nexus-store --no-default-features`
Expected: PASS.

- [ ] **Step 3: Verify no production `std::` remains**

Run: `rg -n 'std::' crates/nexus-store/src --no-heading | rg -v 'cfg\(test\)' | rg -v '^\S+:\d+:\s*//'`
Expected: hits only inside `#[cfg(test)]` modules (spot-check each survivor's enclosing cfg).

---

### Task 6: Verify all four build surfaces

**Files:** none (verification only)

- [ ] **Step 1: Host no_std**

Run: `cargo build -p nexus-store --no-default-features`
Expected: PASS. (With `#![no_std]` active, any in-crate `std::` path fails to resolve even on a host with std — this is the leak detector.)

- [ ] **Step 2: Host no_std + dep-free features** (scope decision 2's empirical checkpoint)

Run: `cargo build -p nexus-store --no-default-features --features subscription,export,import,snapshot,projection`
Expected: PASS. If `futures`' alloc-only set is missing a combinator the subscription loop needs (`unfold`, `try_fold`): record the deviation, drop this variant from the flake gate in Task 7, and open a follow-up card instead of forcing `futures/std` in.

- [ ] **Step 3: wasm32 no_std**

Run: `cargo build -p nexus-store --target wasm32-unknown-unknown --no-default-features`
Expected: PASS (target is already installed — the kernel gate builds it).

- [ ] **Step 4: std path regression check**

Run: `cargo nextest run -p nexus-store`
Expected: all tests pass, zero failures — the default `std` build is behaviorally unchanged.

- [ ] **Step 5: Clippy under the full matrix** (project rule: clean under `--all-features --all-targets`)

Run: `cargo clippy --workspace --all-features --all-targets`
Expected: zero warnings.

---

### Task 7: Flake CI gate

**Files:**
- Modify: `flake.nix` (checks section, next to the existing `nexus-wasm`/`nexus-nostd` derivations, ~line 140)

- [ ] **Step 1: Add the store gate as its own check**

```nix
# nexus-store no_std gate (#301). The store needs an allocator, so there
# is no bare-metal thumbv7em build (that would require a
# #[global_allocator]); wasm32 + host --no-default-features is the
# pragmatic gate. The host build is the std-leak detector: with
# #![no_std] active, an in-crate `std::` path fails to resolve even
# though the host ships std. The features variant keeps the dep-free
# feature set (subscription/export/import/snapshot/projection) honest.
nexus-store-nostd = craneLib.mkCargoDerivation (commonArgs // {
  inherit cargoArtifacts;
  pname = "nexus-store-nostd";
  buildPhaseCargoCommand = ''
    cargo build -p nexus-store --no-default-features
    cargo build -p nexus-store --target wasm32-unknown-unknown --no-default-features
    cargo build -p nexus-store --target wasm32-unknown-unknown --no-default-features --features subscription,export,import,snapshot,projection
  '';
});
```

(Adjust the features line per Task 6 Step 2's outcome; if dropped, log the deviation.)

- [ ] **Step 2: Verify the flake evaluates**

Run: `nix flake show 2>/dev/null | head -40`
Expected: `nexus-store-nostd` listed under checks, no eval errors.

---

### Task 8: Format, commit, PR

**Files:** none new

- [ ] **Step 1: Format**

```bash
nix develop -c cargo fmt --all
```

- [ ] **Step 2: Stage everything (including any new files) and commit**

```bash
git add -A
git commit -m "feat(store)!: port nexus-store to no_std (core+alloc) (#301)"
```

The pre-commit hook runs `nix flake check` (never bypass it; never pre-run it). Expect several minutes.

- [ ] **Step 3: Push and open the PR** (gh account `joeldsouzax`)

```bash
git push -u origin feat/301-no-std-store
gh pr create \
  --title "feat(store)!: port nexus-store to no_std (core+alloc) (#301)" \
  --body "$(cat <<'EOF'
## Summary
- `#![cfg_attr(not(feature = "std"), no_std)]` + unconditional `extern crate alloc` behind an additive default `std` feature (mirrors #279 / #303)
- **Freeze gate:** all public error bounds `std::error::Error` → `core::error::Error` (stable since 1.81; a re-export, so no semantic change for std consumers — but irreversible post-1.0 in the other direction)
- `futures`/`bytes`/`aligned-vec` flip to `default-features = false` at the workspace root (workspace inheritance cannot override `default-features`); std consumers restore their exact former feature sets
- `nexus-store` joins hakari `[final-excludes]` — workspace-hack force-enables std-implying `futures-util` features that unification would leak into the no_std build
- New flake check `nexus-store-nostd`: host + wasm32 `--no-default-features`, plus the dep-free feature set

Closes #301

## Test plan
- [ ] `nix flake check` green (std path + new no_std gate)
- [ ] `cargo nextest run -p nexus-store` unchanged
- [ ] `cargo clippy --workspace --all-features --all-targets` clean

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 4: Verify CI, then squash-merge**

Run: `gh pr checks --watch`
Expected: Nix Flake Check green. Merge with `gh pr merge --squash --delete-branch` (squash-only ruleset) — after user confirmation.

---

## Self-Review

- **Spec coverage:** #301's five "What" bullets → Task 4 (attribute), Task 5 (bounds + core/alloc sweep), test-only HashMap bullet is obsolete (`test_support.rs` is `#[cfg(all(test, feature = "subscription"))]` — always std; verified), thiserror bullet → Tasks 2–3 (workspace already no-default from #279; the store's hardcoded `["std"]` becomes feature-forwarded), CI gate → Task 7, hakari → Task 3 (verified needed: workspace-hack enables std-implying futures-util features). Acceptance boxes → Task 5 Step 1 (bounds) + Tasks 6–7 (gates, no regression).
- **Placeholder scan:** the Task 5 sweep intentionally uses a mapping table + compiler-driven worklist instead of per-line diffs — the exact sites are enumerated where known; prelude-import sites are discovered by the Task 4 failing build (deterministic, complete).
- **Type consistency:** feature name `std`, check name `nexus-store-nostd`, branch `feat/301-no-std-store` used consistently throughout.
