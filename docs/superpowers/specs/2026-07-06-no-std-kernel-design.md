# Design: `no_std` kernel port (#279)

**Issue:** [#279](https://github.com/devrandom-labs/nexus/issues/279) — `[freeze][T1] no_std kernel: port `nexus` to core+alloc`
**Milestone:** 1 — Nexus: Pre-Freeze (1.0 blockers)
**Scope:** `crates/nexus` only. Store/adapter no_std is out of scope (tracked as #300 → #301 → #302).
**Decision:** Option A (port the kernel) — chosen over Option B (drop the IoT claim). Aligns with the IoT/mobile-first target and unblocks Bombay-SDK #7.

---

## 1. Background — verified current state

The port is mechanical because the kernel is already heap- and std-light. Verified during design:

- **Production kernel code has no heap use** — every `Vec`/`vec!`/`Box`/`String` is in `#[cfg(test)]` code or the `testing` fixture. Dependencies are `arrayvec` (already `default-features = false`, no_std) and `thiserror` (no_std-capable via `default-features = false`).
- **The std sweep touches only 4 files** — `aggregate.rs`, `message.rs`, `id.rs`, `version.rs`. `error.rs`, `error_id.rs`, `event.rs`, `events.rs`, `saga.rs`, `closing_the_books.rs` have **zero** non-doc `std::` usage.
- **The freeze-relevant symbol is `std::error::Error`** in `aggregate.rs`'s public trait bounds. Switching to `core::error::Error` (stable since Rust 1.81, under our 1.95 MSRV) after 1.0 would be a breaking change — hence pre-freeze.
- **`nexus-macros` generated code is already `::core::`-clean** (`::core::option::Option`, `::core::result::Result`, `::nexus::…`). Its own `use std::collections` is host-side proc-macro code, not emitted output. *(Verify the `#[nexus::aggregate]` output too during implementation.)*
- **Toolchain mechanism is proven in-repo** — the sibling `cesr` crate pins the *identical* `1.95.0` channel with the *identical* fenix `sha256` and adds `targets = ["wasm32-unknown-unknown"]` to its `rust-toolchain.toml`; its `cesr-wasm` flake check builds `--target wasm32-unknown-unknown`. fenix's `fromToolchainFile` honors the toolchain file's `targets` field, and the `sha256` (channel-manifest hash) is unchanged by adding targets.

## 2. Design

### 2.1 Feature architecture — additive `std`, default-on

```toml
# crates/nexus/Cargo.toml
[features]
default = ["std"]
std = ["thiserror/std"]   # additive: enabling only adds; never removes API
derive = ["dep:nexus-macros"]
testing = []
```

`std` is additive and on by default, so **every existing build is byte-for-byte unchanged** — `cargo build`, the `nix flake check` nextest run, examples, and downstream consumers all keep std. The no_std path is reached only via `--no-default-features`.

### 2.2 `lib.rs`

```rust
#![cfg_attr(not(feature = "std"), no_std)]

// Production kernel is pure `core` (no allocator required on-device).
// `alloc` is needed only by the test fixture and unit tests.
#[cfg(any(test, feature = "testing"))]
extern crate alloc;
```

Gating `alloc` behind `test`/`testing` states and enforces a stronger property: **the production kernel needs no allocator at all** — a meaningful IoT guarantee.

### 2.3 `std::` → `core::` sweep (4 files)

| From | To | Files |
|------|----|-------|
| `std::error::Error` | `core::error::Error` | `aggregate.rs` (**freeze-relevant public bound**) |
| `std::fmt`, `Debug`, `Display` | `core::fmt` | `message.rs`, `id.rs`, `version.rs`, `aggregate.rs` |
| `std::hash::Hash` | `core::hash::Hash` | `id.rs` |
| `std::num::NonZero*` | `core::num::NonZero*` | `version.rs`, `aggregate.rs` |
| `std::iter::FusedIterator`, `std::mem` | `core::iter`, `core::mem` | `version.rs`, `aggregate.rs` |

Doctests may keep `use std::…` — they compile as separate std test binaries and never enter the no_std build (the flake gate is `cargo build`, not doctest, on the no_std targets).

### 2.4 `testing.rs`

`Vec` → `alloc::vec::Vec`. The fixture stays no_std+alloc-compatible.

### 2.5 `thiserror` — no_std, with a behavior-preserving ripple

Cargo **workspace dependency inheritance cannot override `default-features`** (verified against the Cargo reference — only `optional` and `features` are inheritable). So to make the kernel's `thiserror` no_std:

- Root `Cargo.toml`: `thiserror = { version = "2.0.18", default-features = false }`.
- `nexus`: `std = ["thiserror/std"]` (already above); its dep stays `thiserror = { workspace = true }`.
- `nexus-store`, `nexus-fjall`, `nexus-postgres`: add `features = ["std"]` to their `thiserror` dep **to preserve today's behavior exactly**.

This is a necessary, behavior-preserving cross-crate change even though the card is "kernel-only." *Verification note:* since Rust 1.81 `std::error::Error` **is** `core::error::Error` (a re-export), the std crates may compile unchanged without `features = ["std"]`; add it only where dropping it actually breaks a build (measure, don't assume).

### 2.6 `rust-toolchain.toml`

```toml
targets = ["wasm32-unknown-unknown", "thumbv7em-none-eabihf"]
```

`sha256` in `flake.nix` is unchanged (channel-manifest hash; cesr proves adding a target doesn't move it). rustup users transparently gain the targets too.

### 2.7 `flake.nix` — two checks (CI = `nix flake check`, nothing else)

Mirror the `cesr-wasm` / `postgresTests` crane pattern. Both are `checks` entries, so a plain `nix flake check` runs them — no CI-only steps.

```nix
nexus-wasm = craneLib.mkCargoDerivation (commonArgs // {
  inherit cargoArtifacts;
  pname = "nexus-wasm";
  buildPhaseCargoCommand = ''
    cargo build -p nexus --target wasm32-unknown-unknown --no-default-features
  '';
});
nexus-nostd = craneLib.mkCargoDerivation (commonArgs // {
  inherit cargoArtifacts;
  pname = "nexus-nostd";
  buildPhaseCargoCommand = ''
    cargo build -p nexus --target thumbv7em-none-eabihf --no-default-features
  '';
});
```

`thumbv7em-none-eabihf` is the **strong** gate: a truly std-free Cortex-M4F target (wasm32-unknown-unknown still ships std, so it alone would not catch a std leak). Since the production kernel is pure `core` (no `alloc` in the default/no-feature build), no `#[global_allocator]` is needed for this build.

### 2.8 `workspace-hack` — hakari `final-excludes`

`workspace-hack` unifies std deps (tokio, sqlx) and every crate depends on it, which would drag std into the no_std build. Fix (authoritative, per hakari docs):

```toml
# .config/hakari.toml
[final-excludes]
workspace-members = ["nexus"]
third-party = [ { name = "futures-core" } ]   # existing
```

`cargo hakari manage-deps` then **removes** the `workspace-hack` edge from `nexus`, and `hakari verify` enforces its *absence* — the gate stays green. The kernel's dependency closure becomes just `arrayvec` + `thiserror` (+ the host-only `nexus-macros`).

### 2.9 README

Keep the embedded/WASM paragraph — now backed by the green `nexus-wasm` + `nexus-nostd` flake checks rather than aspirational.

## 3. Testing strategy

This is a build/toolchain + type-bound change, so the primary test **is the build itself**:

1. **The no_std build gates** (`nexus-wasm`, `nexus-nostd`) — these fail if any std leaks into the kernel. This is the defensive-boundary test for the no_std contract.
2. **Existing kernel test suite** — runs unchanged on the std default path via `nix flake check` nextest (sequence/lifecycle/property tests for aggregate/saga/events already exist). No behavior change is expected; a green suite proves the `core::` swap is semantics-preserving.
3. **`trybuild` / doctest** — unchanged; compile on std.

No new runtime tests are required — there is no new runtime behavior, only a narrowed dependency surface and a bound change (`std::error::Error` → `core::error::Error`, which is the same trait since 1.81).

## 4. Risks & verification (measure, don't assume)

- [ ] `thiserror` no_std actually compiles the kernel error types under `--no-default-features` (thiserror 2.0.18 has exactly one feature, `std`, default-on — verified via docs.rs).
- [ ] `#[nexus::aggregate]` macro output is `::core::`-clean (DomainEvent/transforms already are).
- [ ] The two flake checks pass — proves no accidental std usage remains.
- [ ] `hakari verify` passes with `nexus` in `final-excludes`.
- [ ] Whether std crates need explicit `thiserror` `features = ["std"]` (may be a no-op post-1.81).

## 5. Acceptance (from #279)

- [ ] Public error trait bounds are `core::error::Error`.
- [ ] `nix flake check` builds the kernel no_std (`thumbv7em-none-eabihf`) + `wasm32-unknown-unknown`.
- [ ] README embedded/WASM claim retained and now CI-backed.

## 6. Out of scope

Full store no_std is a separate arc, tracked as pre-freeze follow-ups:

- **#300** — extract `StreamNotifiers` → `nexus-wake` (tokio out of `nexus-store`).
- **#301** — port `nexus-store` to no_std + alloc; store public error bounds → `core::error::Error`.
- **#302** — no_std `WakeSource` bridge for on-device live-tail subscriptions.

Key finding underpinning that split: `nexus-store` production code is already no_std+alloc-clean **except** for tokio, which is confined to `notify.rs` behind the `subscription` feature and abstracted behind the `WakeSource` trait.
