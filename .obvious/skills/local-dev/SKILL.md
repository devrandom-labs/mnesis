---
name: local-dev
description: Verified bring-up and health-check recipe for the mnesis Rust workspace (rustup toolchain, cargo test/lint/doc, optional PostgreSQL, axum-todos example CRUD)
---

# local-dev — mnesis onboarding record (2026-09-17)

Durable record of the LOCAL-DEV onboarding run. Every command below was executed and
verified in this sandbox (thread th_ebG2G77D); raw logs live at `~/onboarding-evidence/`.

## What this environment is

- Debian 13 x86_64 sandbox, 8 vCPU, ~8 GB RAM, ~40 GB free disk. **No nix, no docker.**
- Rust via rustup in `~/.cargo`; `rust-toolchain.toml` auto-selects pinned 1.95.0.
- PostgreSQL 17 via apt (cluster `17/main`, unix socket `/run/postgresql`).
- `/tmp` is tmpfs — keep durable artifacts on the rootfs (e.g. `~/`), not `/tmp`.

## Bring-up from a cold sandbox

1. **Rust** (if `cargo` is missing):
   ```bash
   curl -sSf https://sh.rustup.rs -o /tmp/rustup-init.sh
   sh /tmp/rustup-init.sh -y --profile minimal --default-toolchain 1.95.0
   . "$HOME/.cargo/env" && rustup toolchain install   # pinned toolchain + wasm32/thumbv7em targets
   ```
2. **PostgreSQL** (if missing or stopped):
   ```bash
   sudo apt-get update && sudo DEBIAN_FRONTEND=noninteractive apt-get install -y postgresql
   sudo pg_ctlcluster 17 main start        # policy-rc.d blocks auto-start in this container
   sudo -u postgres psql -c 'CREATE ROLE "user" LOGIN SUPERUSER;'
   sudo -u postgres psql -c 'CREATE DATABASE mnesis_test OWNER "user";'
   ```
3. **Git hooks:** `git config core.hooksPath .githooks` — the hook runs `nix flake check`,
   so it only works where nix exists (leave it unset in this sandbox)

## Fast health check

```bash
pg_lsclusters                                  # 17 main 5432 online
psql 'postgres:///mnesis_test?host=/run/postgresql' -tAc 'select 1;'
cargo --version                                # cargo 1.95.0
git status --short --branch                    # clean on main
```

## Verified commands and results (2026-09-17)

| Command | Result |
|---|---|
| `cargo test --all` | 116/117 targets green — 1360 passed, 0 failed, 25 ignored; the one failing target is the pre-existing trybuild drift (below) |
| `cargo clippy --workspace --all-features --all-targets -- --deny warnings` | exit 0, zero warnings |
| `cargo fmt --all --check` | clean |
| `cargo doc --workspace --no-deps` | ok |
| `cargo build -p mnesis --target thumbv7em-none-eabihf` (+ wasm32; + nostd-smoketest, wake-nostd, store `--no-default-features` host/thumbv7em incl. features) | all ok |
| `DATABASE_URL='postgres:///mnesis_test?host=/run/postgresql' cargo test -p mnesis-postgres -- --test-threads=1` | 56/56 pass against live Postgres 17.11 |
| `cargo test -p mnesis --features std` | all green standalone (214 tests across targets) |
| `cargo test -p mnesis-store` | all green standalone (204 lib tests + suites, 0 failed) |
| `cargo run -p mnesis-example-axum-todos` | serves 127.0.0.1:3000 |

Run the postgres suite **serially** (`--test-threads=1`): the DB-backed tests share one
`events` table and isolate via `TRUNCATE` in setup, so parallel runs clobber each other
(same reason CI's nixosTest passes `--test-threads=1`).

## Primary user flow (axum-todos — exercised end-to-end)

```bash
cargo run -p mnesis-example-axum-todos &        # listen 127.0.0.1:3000
curl http://127.0.0.1:3000/todos                                    # 200 []
curl -X POST http://127.0.0.1:3000/todos -H 'content-type: application/json' \
     -d '{"text":"wire the obvious contract"}'                      # 201 + x-mnesis-position: 1
curl -H 'x-mnesis-position: 1' http://127.0.0.1:3000/todos          # 200 — read-your-writes
curl -X PATCH .../todos/<id> -d '{"text":"...","completed":true}'   # 200
curl -X DELETE .../todos/<id>                                      # 204; PATCH after → 404
```

Transcript: `~/onboarding-evidence/axum-crud.log`; server tracing: `axum-server.log`.

## Known quirks

1. **trybuild snapshot drift (pre-existing, upstream-owned):** `cargo test -p mnesis-macros
   --test compile_fail_tests` fails 3/14 cases — `aggregate_bad_error`, `aggregate_bad_id`,
   `aggregate_bad_state` `.stderr` snapshots are one caret-column off (`34:40` vs `34:41`)
   vs the pinned rustc 1.95.0. CI excludes trybuild on purpose (`-E 'not test(compile_fail)'`;
   see the "KNOWN GAP" note in `flake.nix`), so the drift is invisible there. Fix upstream:
   `TRYBUILD=overwrite cargo test -p mnesis-macros --test compile_fail_tests`, commit the
   three refreshed `crates/macros/tests/macro_compile_fail/*.stderr`.
2. **nix unavailable:** `nix flake check` is the CI gate but cannot run in this sandbox.
   The cargo equivalents above cover clippy/fmt/test/doc/no_std; taplo, cargo-audit,
   cargo-deny, hakari, mutants, and fuzz-replay are nix-gated only — don't chase them locally.
3. **Postgres auto-start is blocked** (policy-rc.d 101): start with `sudo pg_ctlcluster 17
   main start` after any reboot. It survives snapshot pause/resume.
4. **5-minute shell limit:** cold full-workspace builds exceed it — run detached
   (`tmux new-session -d -s build '<cmd> > /tmp/x.log 2>&1'`) and poll the log.
5. **Separate workspaces:** `fuzz/`, `fuzz-afl/` have their own `Cargo.lock` — `cd` first.
6. **Kernel single-crate tests need `std`:** plain `cargo test -p mnesis` fails to
   compile the lib test (12 errors: unresolved `std`/`Vec`/`vec!`) because the kernel is
   `no_std` by default and `std` is an additive feature. Use `cargo test -p mnesis
   --features std`, or `cargo test --all` (workspace unification enables it). Every other
   crate (e.g. `mnesis-store`) tests standalone with default features.
7. **MSRV policy:** `rust-version` == the pinned toolchain; bump `rust-toolchain.toml` and
   `rust-version` together, never separately (issue #204).

## Snapshot

Final snapshot captured **2026-09-17T15:29:21.423Z** — template
`ag7ls678mz1tr6op6pfu:default` (live session `i63j1nwd9pkyuws677tmq`). Post-resume state
verified: Postgres online, `select 1` ok, cargo 1.95.0, clean git tree.
