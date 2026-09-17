# mnesis — agent onboarding

Event sourcing for Rust — no `Box<dyn>`, no runtime downcasting, no hidden allocations.
Experimental project, unstable API; the 1.0 promise lives in [STABILITY.md](../STABILITY.md).

Read before non-trivial work: **[CLAUDE.md](../CLAUDE.md)** (architecture, crate graph,
mandatory rules — the authoritative map), [CONTRIBUTING.md](../CONTRIBUTING.md) (workflow),
[docs/](../docs/) (user docs, mdx).

## Stack

- **Language:** Rust, edition 2024, pinned **stable 1.95.0** via `rust-toolchain.toml`
  (single source of truth; also installs `wasm32-unknown-unknown` +
  `thumbv7em-none-eabihf` targets, clippy, rustfmt). No nightly anywhere.
- **Package manager:** cargo. One root workspace, **22 members**; all dependency versions
  centralized in root `Cargo.toml` `[workspace.dependencies]`; `crates/workspace-hack`
  is cargo-hakari-managed (rerun `cargo hakari generate` after dependency changes).
- **CI gate:** `nix flake check` (clippy `--all-features --all-targets`, fmt, taplo,
  audit, deny, nextest + coverage, hakari, no_std/wasm builds, fuzz corpus replay).
  `nix develop` is the repo's preferred dev shell — **not available in the standard
  sandbox**; rustup + cargo locally cover everything except taplo/audit/deny/hakari.
- **External service:** PostgreSQL — **optional** for local dev. `mnesis-postgres`
  integration tests self-skip when `DATABASE_URL` is unset; CI runs them in a NixOS VM
  (`nix build .#postgres-integration`). No Docker Compose; no other services.
- **Lint posture:** clippy `all` + `pedantic` + `nursery` denied, plus `unwrap_used`,
  `expect_used`, `panic`, `todo`, `as_conversions`, `shadow_*`, `str_to_string` etc.
  (workspace lints — all crates inherit). Conventional commits (`feat:`, `fix:`, `docs:`).
  Dual licensed MIT OR Apache-2.0.

## Commands (verified in this sandbox)

```bash
cargo test --all                                  # full workspace suite (workspace feature unification on)
cargo test -p mnesis --features std               # kernel standalone — plain `-p mnesis` fails (see below)
cargo test -p mnesis-store -- envelope_tests      # single test by name (store builds standalone)
cargo clippy --workspace --all-features --all-targets -- --deny warnings
cargo fmt --all --check                           # drop --check to apply
cargo doc --workspace --no-deps
cargo build -p mnesis --target thumbv7em-none-eabihf   # no_std gate; also wasm32-unknown-unknown
cargo run -p mnesis-example-axum-todos            # HTTP API on 127.0.0.1:3000

# Postgres adapter tests (serial; self-skip when DATABASE_URL is unset)
DATABASE_URL='postgres:///mnesis_test?host=/run/postgresql' \
  cargo test -p mnesis-postgres -- --test-threads=1
```

`cargo test -p mnesis` **without** `--features std` fails to compile the lib test: the
kernel is `no_std` by default (`std` is an additive feature, #279/#364) and its inline
tests use `std`/`Vec` — only workspace-wide runs (or `--all`) unify the feature in. The
store and all other crates build standalone with default features.

Primary user flow to exercise on review: the axum-todos example — `GET/POST /todos`,
`PATCH/DELETE /todos/{id}`, read-your-writes via the `x-mnesis-position` response header.

## Codebase map

See [codebase-map.md](codebase-map.md) (folder-level table).

## Local Verification Summary

Generated 2026-09-17 by the onboarding run — `dev_stack_healthy: true`.

| Check | Result |
|---|---|
| Toolchain | rustc 1.95.0 (59807616e 2026-04-14), auto-installed from `rust-toolchain.toml` |
| `cargo test --all` | 116/117 targets green — **1360 passed, 0 failed, 25 ignored**; 1 target fails (see known failure) |
| `cargo clippy --workspace --all-features --all-targets -- --deny warnings` | exit 0, zero warnings |
| `cargo fmt --all --check` | clean |
| `cargo doc --workspace --no-deps` | ok |
| no_std gates | thumbv7em-none-eabihf: `mnesis`, `nostd-smoketest`, `wake-nostd` ok; wasm32: `mnesis`, `wake-nostd` ok; `mnesis-store` host + thumbv7em `--no-default-features` (incl. feature set) ok |
| PostgreSQL 17.11 (live, unix socket) | **56/56** `mnesis-postgres` tests pass serially (14 unit + 2 all-stream + 40 conformance) |
| axum-todos example CRUD | 201 create (+ `x-mnesis-position: 1`), read-your-writes GET 200, PATCH 200, DELETE 204, PATCH-on-deleted → 404, projection consistent throughout |

**Known pre-existing failure (upstream, not an environment issue):**
`cargo test -p mnesis-macros --test compile_fail_tests` — 3 of 14 trybuild `.stderr`
snapshots (`aggregate_bad_error`, `aggregate_bad_id`, `aggregate_bad_state`) mismatch by a
one-column caret offset (`34:40` vs `34:41` etc.) against the pinned 1.95.0 rustc. CI
deliberately excludes trybuild (`-E 'not test(compile_fail)'` — see the "KNOWN GAP" note
in `flake.nix`), so this drift is invisible there. Fix upstream by re-blessing:
`TRYBUILD=overwrite cargo test -p mnesis-macros --test compile_fail_tests`, then commit
the three updated `crates/macros/tests/macro_compile_fail/*.stderr` files.

## Environment notes

- Sandbox has no `nix`/`docker` binaries. Rust comes from rustup (network reachable);
  `rust-toolchain.toml` drives the exact pin. PostgreSQL 17 was installed via apt.
- Postgres cluster: `sudo pg_ctlcluster 17 main start` — the container's policy-rc.d
  blocks auto-start, so this is needed after any reboot. DB `mnesis_test`, role `user`
  (superuser, peer auth over `/run/postgresql`).
- Git hooks: `.githooks/pre-commit` activates via `git config core.hooksPath .githooks`
  (the nix dev shell does this automatically; set it manually under rustup).
- `fuzz/`, `fuzz-afl/`, `fuzz-common/` are separate cargo workspaces with their own
  `Cargo.lock` — `cd` in before running cargo there.
- Long cargo runs (cold test build) exceed the 5-minute shell limit — run them detached
  under tmux and poll.

## Sandbox snapshot

- **Snapshot/template id:** `ag7ls678mz1tr6op6pfu:default` (live session `i63j1nwd9pkyuws677tmq`)
- **Captured:** 2026-09-17T15:29:21.423Z (ISO-8601)
- **Baked-in state:** rustup + pinned 1.95.0 toolchain (incl. wasm32/thumbv7em targets),
  warm `target/` build cache, PostgreSQL 17 online with the `mnesis_test` DB ready, and
  the raw verification logs at `~/onboarding-evidence/` (see
  [skills/local-dev/SKILL.md](skills/local-dev/SKILL.md) for the full bring-up recipe).
