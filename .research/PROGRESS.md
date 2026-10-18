# Progress

Persistent goal active, no token budget. Repository
`/Users/joel/orca/workspaces/nexus/main`, branch `docs/postgres-prd`.
Canonical scope/dependencies: `docs/prds/postgres.md`, `docs/prd-backlog/status.md`.
Exact evidence, contracts and pre-edit budgets: `docs/prds/postgres/research.md`.

Signed commits: 87a9d054 (PRD/contract/evidence), af6a43b4 (wake deadlines),
755ec8eb (bounded keyset rows), d4624e58 (listener lifecycle). All passed the
required full Darwin gate before retention. Latest committed suite: 74 native
Postgres tests, zero skips; 1,369 workspace tests, four skips. No PR/merge/release.

Current PG1 byte-budget experiment ready to retain: 16MiB default page/row budget,
Limits replaces unpublished batch-only constructor, SQL header-window prefix
selection before fetching bodies. Original large-row witness failed (24–26MB).
Current 79 native tests pass, zero skips, 12.601s; large-row retained bytes ~15MB,
oversized legacy body is not fetched, atomic rejection and cancellation pass.
Targeted Clippy/fmt pass; actual production net +124, cumulative net +190; no
new dependencies. Native plans retain header index seeks but add primary-key
lookups; full capacity evidence remains PG8.

COMPLETED matched gate: unified exec session **46146**, log
`/tmp/mnesis-postgres-byte-flake.log`; source/test diff SHA-256
`0644165e71d4a033df307670ae3027ddf38f8a5b0422f9e99e17d277a6222c5f`.
All 12 checks passed, 1,374 workspace tests passed and four skipped (301.257s
nextest). Retain through the normal signed pre-commit hook, then open the
focused initial read/wake PR. PG3 fault experiment pre-edit record is ready in feature research.

Postgres 17.10 cluster `/tmp/mnesis-postgres-pg0` is running, Unix socket `/tmp`,
port 55432, DB `mnesis_pg0`. Native tests must be serial (fixtures truncate events).
Use explicit git+file Nix dev shell, filter optional rust-analyzer and blank
shellHook; exact command in research. Rooted shell `/tmp/mnesis-postgres-shell`.

Continue PG3 with actual server COMMIT acknowledgement suppression and independent
row proof (not arbitrary backend termination). PG4 migration facility constraint:
SQLx per-migration transactions and session advisory lock; close scoped migration
connections on drop, never wrap migrator in an outer transaction that outlives
its unlock. Full unsigned values, binary event types and read-only open pending.
All PG5–PG9 capabilities and delivery remain required; PG10 conditional. Main is
protected by ruleset7146669: squash PRs, resolved threads, Nix Flake Check,
Compile-fail tests and Feature isolation, no bypass. Native Postgres CI also
required by PRD. No publication or clean registry-consumer proof yet.
