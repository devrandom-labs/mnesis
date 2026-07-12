# no_std Kernel Port (#279) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `crates/mnesis` (the kernel) build for `no_std` targets (bare-metal ARM + WASM) behind an additive `std` default feature, and move its public error bounds from `std::error::Error` to `core::error::Error` before the 1.0 freeze.

**Architecture:** The kernel is already heap-free in production and depends only on `arrayvec` + `thiserror` (both no_std-capable). The port is: add `#![cfg_attr(not(feature = "std"), no_std)]`, swap `std::` → `core::` in 4 files, make `thiserror` no_std at the workspace, drop the `workspace-hack` edge from `mnesis` (it drags std in), and add two `nix flake check` gates that build the kernel for `thumbv7em-none-eabihf` + `wasm32-unknown-unknown`. Nothing outside `crates/mnesis` changes behavior; store/adapter no_std is out of scope (#300/#301/#302).

**Tech Stack:** Rust 2024 (pinned stable 1.95), `fenix` toolchain via `rust-toolchain.toml`, `crane` flake checks, `cargo-hakari` (workspace-hack), `thiserror` 2.0.

**Spec:** `docs/superpowers/specs/2026-07-06-no-std-kernel-design.md`

**Working branch:** `feat/279-no-std-kernel` (already created off `origin/main`).

> **Conventions for every commit below:**
> - The pre-commit hook runs `nix flake check` automatically — **do not** run the full gate by hand first. Just `git commit`; a red gate blocks the commit.
> - Run all cargo/tooling through the dev shell: `nix develop -c <cmd>`.
> - Commit trailer: `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`.
> - The current Nix system attribute is `aarch64-darwin` (from `nix flake check` output). Use it in `nix build .#checks.aarch64-darwin.<name>` commands.

---

## Task 1: Add cross-compile targets and establish the failing no_std baseline

**Files:**
- Modify: `rust-toolchain.toml`

- [ ] **Step 1: Add the two targets to the pinned toolchain**

Replace the `[toolchain]` block in `rust-toolchain.toml` so it reads exactly:

```toml
[toolchain]
channel = "1.95.0"
components = ["cargo", "rustc", "rust-std", "rustfmt", "clippy", "llvm-tools-preview"]
targets = ["wasm32-unknown-unknown", "thumbv7em-none-eabihf"]
```

(Only the `targets = [...]` line is new. `fenix`'s `fromToolchainFile` honors this field; the `sha256` in `flake.nix` is a channel-manifest hash and does **not** change — proven by the sibling `cesr` crate, which pins the identical channel + sha256 and adds `wasm32-unknown-unknown` the same way.)

- [ ] **Step 2: Confirm the target resolves and the no_std build is currently RED**

Run: `nix develop -c cargo build -p mnesis --no-default-features --target thumbv7em-none-eabihf`

Expected: **FAIL** with errors like `` error[E0463]: can't find crate for `std` `` (the kernel still implicitly links std — `lib.rs` is not yet `#![no_std]`). This is the red baseline the port turns green.

> If instead you see `error: "thumbv7em-none-eabihf" may not be installed` or `can't find crate for `core``, the target's `rust-std` was not fetched — re-check the `targets` line in `rust-toolchain.toml` and re-enter the shell (`nix develop`).

- [ ] **Step 3: Commit**

```bash
git add rust-toolchain.toml
git commit -m "build(mnesis): add wasm32 + thumbv7em targets to pinned toolchain (#279)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

Expected: pre-commit `nix flake check` passes (the std build is unaffected by adding targets), commit succeeds.

---

## Task 2: Make `thiserror` no_std at the workspace and add the `std` feature to `mnesis`

**Files:**
- Modify: `Cargo.toml` (root, line 42)
- Modify: `crates/mnesis/Cargo.toml` (`[features]`)
- Modify: `crates/mnesis-store/Cargo.toml` (line 62)
- Modify: `crates/mnesis-fjall/Cargo.toml` (line 27)
- Modify: `crates/mnesis-postgres/Cargo.toml` (line 20)

**Why the ripple:** Cargo workspace inheritance cannot override `default-features` (only `optional`/`features` are inheritable). To let the kernel take `thiserror` without std, the *workspace* declaration must be `default-features = false`; the three std crates then opt back into std explicitly so their behavior is unchanged.

- [ ] **Step 1: Turn off `thiserror` default features at the workspace**

In root `Cargo.toml`, change line 42 from:

```toml
thiserror = "2.0.18"
```

to:

```toml
thiserror = { version = "2.0.18", default-features = false }
```

- [ ] **Step 2: Give `mnesis` an additive `std` feature that re-enables `thiserror/std`**

In `crates/mnesis/Cargo.toml`, replace the `[features]` block:

```toml
[features]
default = []
derive = ["dep:mnesis-macros"]
testing = []
```

with:

```toml
[features]
default = ["std"]
# Additive: enabling `std` only ADDS (the std::error::Error bridge via thiserror);
# disabling it (`--no-default-features`) yields a no_std + core::error::Error kernel.
std = ["thiserror/std"]
derive = ["dep:mnesis-macros"]
testing = []
```

- [ ] **Step 3: Keep the three std crates on std (behavior-preserving)**

In each of `crates/mnesis-store/Cargo.toml` (line 62), `crates/mnesis-fjall/Cargo.toml` (line 27), `crates/mnesis-postgres/Cargo.toml` (line 20), change:

```toml
thiserror = { workspace = true }
```

to:

```toml
thiserror = { workspace = true, features = ["std"] }
```

- [ ] **Step 4: Verify the whole workspace still builds on std**

Run: `nix develop -c cargo build --workspace`

Expected: **PASS** (no behavior change; every std crate still has `thiserror/std`).

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml crates/mnesis/Cargo.toml crates/mnesis-store/Cargo.toml crates/mnesis-fjall/Cargo.toml crates/mnesis-postgres/Cargo.toml
git commit -m "build: make thiserror no_std at workspace, add mnesis std feature (#279)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

Expected: pre-commit `nix flake check` passes, commit succeeds.

---

## Task 3: Drop the `workspace-hack` edge from `mnesis`

**Files:**
- Modify: `.config/hakari.toml`
- Modify (by tooling): `crates/mnesis/Cargo.toml`, `crates/workspace-hack/Cargo.toml`

**Why:** `workspace-hack` unifies std deps (tokio, sqlx) and every member depends on it — that would drag std into the no_std kernel build. hakari's `final-excludes` removes the edge and the `hakari verify` gate then enforces its *absence*.

- [ ] **Step 1: Exclude `mnesis` from workspace-hack**

In `.config/hakari.toml`, change the existing `[final-excludes]` table from:

```toml
[final-excludes]
third-party = [
  { name = "futures-core" },
]
```

to:

```toml
[final-excludes]
# `mnesis` is the no_std kernel — it must not depend on the (std) workspace-hack.
# hakari removes the edge; `hakari verify` enforces its absence. (#279)
workspace-members = ["mnesis"]
third-party = [
  { name = "futures-core" },
]
```

- [ ] **Step 2: Regenerate and apply**

Run: `nix develop -c cargo hakari generate`
Then: `nix develop -c cargo hakari manage-deps`

(`generate` rewrites `crates/workspace-hack/Cargo.toml` without `mnesis`'s contributions; `manage-deps` removes the `workspace-hack` line from `crates/mnesis/Cargo.toml`.)

- [ ] **Step 3: Verify hakari is consistent and `mnesis` no longer depends on workspace-hack**

Run: `nix develop -c cargo hakari verify`
Expected: **PASS** (exit 0).

Run: `grep -n "workspace-hack" crates/mnesis/Cargo.toml`
Expected: **no output** (the dependency line is gone).

- [ ] **Step 4: Verify the std build still works**

Run: `nix develop -c cargo build -p mnesis`
Expected: **PASS**.

- [ ] **Step 5: Commit**

```bash
git add .config/hakari.toml crates/mnesis/Cargo.toml crates/workspace-hack/Cargo.toml
git commit -m "build(mnesis): exclude kernel from workspace-hack for no_std (#279)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

Expected: pre-commit `nix flake check` passes (`mnesis-hakari` check green), commit succeeds.

---

## Task 4: Port the kernel to no_std (the red → green step)

**Files:**
- Modify: `crates/mnesis/src/lib.rs`
- Modify: `crates/mnesis/src/aggregate.rs:6-10`
- Modify: `crates/mnesis/src/message.rs:1`
- Modify: `crates/mnesis/src/id.rs:1-2`
- Modify: `crates/mnesis/src/version.rs:1-3`
- Modify: `crates/mnesis/src/testing.rs` (add one import)

- [ ] **Step 1: Make `lib.rs` no_std with allocator-free production code**

At the very top of `crates/mnesis/src/lib.rs` (before the `mod` declarations), add:

```rust
#![cfg_attr(not(feature = "std"), no_std)]

// The production kernel is pure `core` — no allocator required on-device.
// `alloc` is pulled in only for the test fixture and unit tests.
#[cfg(any(test, feature = "testing"))]
extern crate alloc;
```

- [ ] **Step 2: Swap `std::` → `core::` in `aggregate.rs`**

In `crates/mnesis/src/aggregate.rs`, change lines 6–10 from:

```rust
use std::error::Error;
use std::fmt;
use std::fmt::Debug;
use std::mem;
use std::num::NonZeroUsize;
```

to:

```rust
use core::error::Error;
use core::fmt;
use core::fmt::Debug;
use core::mem;
use core::num::NonZeroUsize;
```

(The `std::` occurrences remaining at lines ~92/100/149 are doctests and ~449/606 are `#[cfg(test)]` code — leave them; they compile as std test binaries and never enter the no_std build.)

- [ ] **Step 3: Swap `std::` → `core::` in `message.rs`**

In `crates/mnesis/src/message.rs`, change line 1 from:

```rust
use std::fmt::Debug;
```

to:

```rust
use core::fmt::Debug;
```

- [ ] **Step 4: Swap `std::` → `core::` in `id.rs`**

In `crates/mnesis/src/id.rs`, change lines 1–2 from:

```rust
use std::fmt::{Debug, Display};
use std::hash::Hash;
```

to:

```rust
use core::fmt::{Debug, Display};
use core::hash::Hash;
```

- [ ] **Step 5: Swap `std::` → `core::` in `version.rs`**

In `crates/mnesis/src/version.rs`, change lines 1–3 from:

```rust
use std::fmt;
use std::iter::FusedIterator;
use std::num::NonZeroU64;
```

to:

```rust
use core::fmt;
use core::iter::FusedIterator;
use core::num::NonZeroU64;
```

- [ ] **Step 6: Point `testing.rs` at `alloc::vec::Vec`**

In `crates/mnesis/src/testing.rs`, add this import at the top of the file with the other `use` statements (the fixture uses `Vec` / `Vec::new()` in ~10 places; a single import resolves them all):

```rust
use alloc::vec::Vec;
```

- [ ] **Step 7: Verify no stray production `std::` remains in the ported files**

Run:

```bash
nix develop -c bash -c "grep -n 'std::' crates/mnesis/src/{aggregate,message,id,version}.rs | grep -vE '///|impl std::fmt::Display for Ctr|use std::panic'"
```

Expected: only doctest lines (prefixed `///`) — no bare production `use std::` or inline `std::` outside `#[cfg(test)]`. If a production line appears, swap its `std::` → `core::`.

- [ ] **Step 8: Verify the std build + tests are unchanged**

Run: `nix develop -c cargo build -p mnesis --all-features`
Expected: **PASS**.

Run: `nix develop -c cargo test -p mnesis --features testing,derive`
Expected: **PASS** (all existing kernel tests green — proves the `core::` swap is semantics-preserving).

- [ ] **Step 9: Verify the no_std builds are now GREEN (red → green)**

Run: `nix develop -c cargo build -p mnesis --no-default-features --target thumbv7em-none-eabihf`
Expected: **PASS** (was FAIL in Task 1 Step 2).

Run: `nix develop -c cargo build -p mnesis --no-default-features --target wasm32-unknown-unknown`
Expected: **PASS**.

- [ ] **Step 10: Commit**

```bash
git add crates/mnesis/src/lib.rs crates/mnesis/src/aggregate.rs crates/mnesis/src/message.rs crates/mnesis/src/id.rs crates/mnesis/src/version.rs crates/mnesis/src/testing.rs
git commit -m "feat(mnesis)!: port kernel to no_std, error bounds to core::error::Error (#279)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

Expected: pre-commit `nix flake check` passes, commit succeeds.

---

## Task 5: Wire the two no_std builds into `nix flake check` as permanent gates

**Files:**
- Modify: `flake.nix` (add two entries to the `checks` attrset)

- [ ] **Step 1: Add `mnesis-wasm` and `mnesis-nostd` checks**

In `flake.nix`, inside the `checks = { ... }` attrset (e.g. right after the `mnesis-hakari` check and before the closing `};` of `checks`), add:

```nix
          # no_std gates — CI is just `nix flake check`, so these ride along.
          # `mnesis-nostd` (thumbv7em-none-eabihf) is the STRONG gate: a fully
          # std-free bare-metal target. `wasm32-unknown-unknown` still ships std,
          # so it alone would not catch a std leak. Both build --no-default-features.
          mnesis-wasm = craneLib.mkCargoDerivation (commonArgs // {
            inherit cargoArtifacts;
            pname = "mnesis-wasm";
            buildPhaseCargoCommand = ''
              cargo build -p mnesis --target wasm32-unknown-unknown --no-default-features
            '';
          });

          mnesis-nostd = craneLib.mkCargoDerivation (commonArgs // {
            inherit cargoArtifacts;
            pname = "mnesis-nostd";
            buildPhaseCargoCommand = ''
              cargo build -p mnesis --target thumbv7em-none-eabihf --no-default-features
            '';
          });
```

- [ ] **Step 2: Build both new checks directly**

Run: `nix build .#checks.aarch64-darwin.mnesis-nostd .#checks.aarch64-darwin.mnesis-wasm -L`

Expected: **both build successfully** (no `std` errors).

> If a check errors that `cargoArtifacts` lacks the target, that is fine to debug by confirming Task 4 Step 9 still passes in the dev shell — the crane derivation runs the identical `cargo build` command.

- [ ] **Step 3: Commit**

```bash
git add flake.nix
git commit -m "ci(mnesis): gate no_std kernel build on thumbv7em + wasm32 (#279)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

Expected: pre-commit `nix flake check` now includes `mnesis-wasm` + `mnesis-nostd` and passes.

---

## Task 6: Confirm the README claim and the macro output; final acceptance

**Files:**
- Verify (adjust only if inaccurate): `README.md`
- Verify only: `crates/mnesis-macros/src/lib.rs`

- [ ] **Step 1: Confirm the embedded/WASM README claim is present and accurate**

Run: `grep -in "embedded\|wasm\|no_std\|no-std\|zero-std\|allocation" README.md`

Expected: the paragraph claiming the kernel compiles for embedded/WASM exists. It is now CI-backed, so **keep it**. Only if the wording overstates scope (e.g. implies the *store* is no_std) trim it to the kernel — for example ensure it reads as "the **kernel** compiles for embedded and WASM targets," not the whole framework. If you edit, `git add README.md`.

- [ ] **Step 2: Confirm the `#[mnesis::aggregate]` macro emits no `std::` paths**

Run: `grep -n "std ::\|std::" crates/mnesis-macros/src/lib.rs | grep -v "use std::collections"`

Expected: **no `std::` inside `quote!` output** (the only `use std::collections` is the macro's own host-side code; generated code already uses `::core::` / `::mnesis::`). If any generated path uses `std::`, change it to `::core::`. (The `DomainEvent` and `transforms` macros were already verified `::core::`-clean during design.)

- [ ] **Step 3: Final acceptance — full gate**

The pre-commit hook has run `nix flake check` on every commit, so the gate is already green including the two no_std checks. Confirm the acceptance criteria from #279 are met:

- [ ] Public error trait bounds are `core::error::Error` (Task 4 Step 2 — `aggregate.rs` `use core::error::Error`).
- [ ] `nix flake check` builds the kernel no_std (`thumbv7em-none-eabihf`) + `wasm32-unknown-unknown` (Task 5).
- [ ] README embedded/WASM claim retained and CI-backed (Step 1).

- [ ] **Step 4: Commit any README adjustment (skip if none)**

```bash
git add README.md
git commit -m "docs(mnesis): scope embedded/WASM claim to the kernel, now CI-backed (#279)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 7: Open the pull request

- [ ] **Step 1: Push and open the PR**

```bash
git push -u origin feat/279-no-std-kernel
gh pr create --title "feat(mnesis)!: port kernel to no_std (core+alloc) (#279)" --body "$(cat <<'EOF'
Closes #279.

Ports `crates/mnesis` to `no_std` behind an additive `std` default feature, and moves public error bounds from `std::error::Error` to `core::error::Error` (freeze-relevant — breaking to do after 1.0).

## What
- `#![cfg_attr(not(feature = "std"), no_std)]`; production kernel is pure `core` (no allocator), `alloc` only under `test`/`testing`.
- `std::` → `core::` in `aggregate.rs`, `message.rs`, `id.rs`, `version.rs`.
- `thiserror` no_std at the workspace; std crates opt back in via `features = ["std"]`.
- `mnesis` excluded from `workspace-hack` (hakari `final-excludes`) so no std leaks in.
- Toolchain gains `wasm32-unknown-unknown` + `thumbv7em-none-eabihf`; two `nix flake check` gates build the kernel `--no-default-features` for both.

## Scope
Kernel only. Store no_std is tracked as #300 → #301 → #302.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

Expected: PR opens against `main` under the `joeldsouzax` account.

---

## Notes for the executor

- **Empirical check flagged in the spec:** the three std crates get `thiserror` `features = ["std"]` conservatively (behavior-preserving). Since Rust 1.81 `std::error::Error` *is* `core::error::Error`, they may compile without it — but do **not** remove it unless you verify the build stays green; keeping it is zero-risk.
- **Do not** merge to `main` directly (squash-only via PR; signed commits required). The PR is the handoff point.
- If any `nix flake check` derivation fails on `aarch64-linux`/`x86_64-*`, note it — the local gate only checks the current system (`aarch64-darwin`); CI checks all.
