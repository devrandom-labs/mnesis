# mnesis — codebase map

Folder-level overview (depth ≤ 2). Directory names drop the `mnesis-` prefix; published
package names keep it. Core boundary: `crates/` = everything an adapter builds on,
`adapters/` = infrastructure implementations of the store seams.

| Path | Package | What lives there |
|---|---|---|
| `crates/mnesis/` | `mnesis` | Kernel — aggregates, events, versioning, command handling, saga dual, testing fixtures. `no_std`, pure `core` (no alloc) |
| `crates/macros/` | `mnesis-macros` | Proc macros — `#[derive(DomainEvent)]`, `#[mnesis::aggregate]`, `#[mnesis::transforms]`; `tests/cross_crate_test` verifies macro output from a foreign crate |
| `crates/store/` | `mnesis-store` | Persistence edge — codecs (`serde`/`bytemuck`/`rkyv`), envelopes, wire frame, catch-up + live-tail subscription loop, snapshots, projections, repository facade, backup/export/import (CBOR). `no_std` + `alloc` |
| `crates/store-testing/` | `mnesis-store-testing` | Executable store-contract conformance kit (`conformance!` macros) — every adapter's dev-dep |
| `crates/wake/` | `mnesis-wake` | In-process wake registry (`StreamNotifiers`, tokio watch generations) — the `std` `WakeSource` |
| `crates/wake-nostd/` | `mnesis-wake-nostd` | `no_std` + `alloc` global-eventcount `WakeSource` (embassy-oriented, on-device live tail) |
| `crates/nostd-smoketest/` | `mnesis-nostd-smoketest` | Smoke consumer proving macro *output* compiles for thumbv7em/wasm32 |
| `crates/test-domains/` | `mnesis-test-domains` | Shared test-domain fixtures |
| `crates/workspace-hack/` | `workspace-hack` | cargo-hakari build-time unification crate (never published) |
| `adapters/fjall/` | `mnesis-fjall` | Embedded LSM-tree event store adapter (fjall 3) — the on-disk/IoT path, incl. `$all` index |
| `adapters/inmemory/` | `mnesis-inmemory` | In-memory adapter — `InMemoryStore`, `InMemorySnapshotStore`; `mnesis-store`'s test fixture via a path-only dev-dep |
| `adapters/postgres/` | `mnesis-postgres` | PostgreSQL adapter (sqlx, `LISTEN`/`NOTIFY` wake, `xid8` $all watermark) — tests self-skip without `DATABASE_URL` |
| `examples/axum-todos/` | `mnesis-example-axum-todos` | Runnable HTTP API (127.0.0.1:3000) — CRUD over mnesis + fjall with an `$all` projection and read-your-writes |
| `examples/inmemory/` | `mnesis-example-inmemory` | Kernel-only bank account loop (docs/getting-started.mdx in full) |
| `examples/store-inmemory/` | — | All store traits with `InMemoryStore`: codec, upcasting, substrate path |
| `examples/store-and-kernel/` | — | Full lifecycle: create → decide → encode → persist → read → decode → rehydrate |
| `examples/fjall-end-to-end/` | — | End-to-end over the persistent fjall adapter |
| `examples/projection-tokio/` | — | Reference tokio loop driving the projection primitives |
| `examples/signed-events/` | — | Ed25519-signed event envelopes |
| `examples/closing-the-books/` | — | Bounded-stream alternative to snapshots |
| `fuzz/` | — | Separate cargo workspace — bolero coverage-guided fuzz targets + committed corpus |
| `fuzz-afl/` | — | Separate workspace — AFL++/CMPLOG engine |
| `fuzz-common/` | — | Shared fuzz harness code |
| `mutants-gate/` | `mutants-gate` | Mutation-testing ratchet verdict tool (reads `.mutants-baselines/`) |
| `docs/` | — | User docs (mdx, mintlify-style), concepts, reference, superpowers plans/specs |
| `.github/workflows/` | — | CI: `nix flake check`, Postgres nixosTest, CodSpeed benches, deep-fuzz, mutants, release-plz, dep updates |
| `.githooks/` | — | Pre-commit hook (activate: `git config core.hooksPath .githooks`) |
| `.mutants-baselines/` | — | Per-crate cargo-mutants ratchet baselines |
| `.cargo/` | — | cargo config, audit + mutants config |
| root | — | `Cargo.toml` (workspace + lints), `flake.nix` (CI/dev shell), `rust-toolchain.toml` (pin 1.99.0), `clippy.toml`, `rustfmt.toml`, `taplo.toml`, `deny.toml`, `release-plz.toml` |
