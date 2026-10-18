# Progress

Persistent goal active, no token budget. Worktree:
`/Users/joel/orca/workspaces/nexus/main`, branch `docs/postgres-prd`.
Requirements and dependency states: `docs/prds/postgres.md` and
`docs/prd-backlog/status.md`. Full evidence and contract decisions:
`docs/prds/postgres/research.md`; raw evidence is checked in beside it.

Previous goal turn made progress: original real-Postgres failures were captured,
read/wake fixes passed native regressions and the matched Darwin gate. This
turn committed them and added the remaining listener lifecycle experiment.

## Committed

- `87a9d054`: PRD, contracts and characterization evidence.
- `af6a43b4`: independent fallback rescan deadline.
- `755ec8eb`: finite bounded keyset reads and native regression tests.

All three commits are signed. The first matched Darwin gate passed all 12
configured checks, including 1,358 workspace tests, four skips and fuzz replay.
Its log is `/tmp/mnesis-postgres-flake-final.log`; source hash and native
read-memory comparisons are in the feature research. No PR, merge or release.

## Current experiment

The listener now retries setup and terminal receive errors, exits on pool close,
and rejects capacity below two before DDL. Two original lifecycle regressions
failed against `755ec8eb`; all 74 native adapter tests now pass, zero skips,
12.273 seconds. Targeted Clippy and formatting pass. Eleven lifecycle campaigns
cover setup/receive failure, disconnect, routing noise, held snapshot, pool close,
first-poll races and eight cleanup cycles with 128 clones/registrations.

Stage Rust source +37/−23/net +14, tests +539 lines, no dependency or new public
type. Cumulative Rust source +250/−184/net +66. Details/budgets live in the PRD.
The matched full Darwin gate also passes all 12 checks, 1,369 workspace tests
and four skips. These changes and their records are ready to retain.

Completed full matched gate: unified exec session **45611**, log
`/tmp/mnesis-postgres-lifecycle-flake.log`. Source/test diff SHA-256 at invocation:
`5eda6d45949078d950302945d31ce21aa6bdac689aba6b4919882752337a2707`.
Gate output is retained in the feature evidence. Commit this experiment and
open a focused PR once the reviewable batch is ready.

## Environment and next work

Postgres 17.10 from locked Nixpkgs remains running: cluster
`/tmp/mnesis-postgres-pg0`, Unix socket `/tmp`, port 55432, DB `mnesis_pg0`.
Use serial tests; setup truncates events. Native commands use the locked Nix
shell with optional rust-analyzer filtered, using an explicit Git flake reference
so ignored `target/` data is not copied. Exact invocation is in the research.
`/tmp/mnesis-postgres-shell` roots the realized environment.

After the gate/commit, continue PG1 practical byte bounds/cancellation and PG3
commit uncertainty. PG3 must suppress an actual server COMMIT acknowledgement
and independently prove durable rows; arbitrary backend termination is not a
substitute. PG4 still needs migrations, full unsigned versions/positions, binary
event types, structural validation and least-privilege startup. All later
PG5–PG9 capabilities and delivery gates remain required. PG10 stays conditional.
