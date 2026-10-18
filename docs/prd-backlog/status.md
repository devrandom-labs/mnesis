# Postgres status and dependencies

Updated: 2026-10-18. Requirements and acceptance live in the
[Postgres PRD](../prds/postgres.md). All rows are implementation states; writing
this PRD does not satisfy their acceptance gates.

| ID | Requirement | State | Blocked by | Unblocks |
| --- | --- | --- | --- | --- |
| PG0 | Real-database reproduction, baseline and contract decisions | feature-complete | — | PG1, PG2, PG3, PG4 |
| PG1 | Bounded reads and stable positions | active | PG0 | PG5, PG7, PG8 |
| PG2 | Recovery without notifications | active | PG0 | PG5, PG8 |
| PG3 | Transaction outcomes and retry safety | active | PG0 | PG5, PG6, PG8 |
| PG4 | Schema, migrations and representation | active | PG0 | PG5, PG6, PG7, PG8 |
| PG5 | Atomic writes and import | blocked | PG1, PG2, PG3, PG4 | PG7, PG9 |
| PG6 | Aggregate snapshots and projection state | blocked | PG3, PG4 | PG7, PG9 |
| PG7 | Export, backup and restore | blocked | PG1, PG4, PG5, PG6 | PG9 |
| PG8 | Secure operation, lifecycle and capacity | blocked | PG1, PG2, PG3, PG4 | PG9 |
| PG9 | Complete verification, minimization and delivery | blocked | PG5, PG6, PG7, PG8 | — |
| PG10 | Standalone subscriber checkpoints, if a selected consumer requires them | candidate | — | — |

PG0 characterization and contract decisions are recorded in
[the feature research](../prds/postgres/research.md). PG1/PG2 first experiments
pass the 79-test real-database suite, with zero skips. Row and byte bounds,
notification deadlines and listener lifecycle have native regression evidence;
remaining acceptance and final delivery checks are open. Prerequisite edges
remain visible after prerequisite acceptance.
PG10 requires its own consumer contract,
acceptance criteria and dependency edges before activation. Labcast command IDs,
deduplication and stale-worker rejection remain downstream responsibilities.

Read/wake slice merged as PR376; outcome slice merged as PR377. Migration PR379
is in CI. Representation candidate passes113 native cases and mutation checks;
its full gate and focused PR remain pending. Required whole-adapter acceptance
and release delivery remain open.
Published adapter version and registry-consumer verification: pending.

2026-10-18 delivery update: PG3 first phase slice merged as protected PR377,
f87afc12dd6dabe4acd7847e98756858f5fb4c86 (40 green checks/1 scheduled skip).
Actual SSI/deadlock and propagation through pending capabilities keep PG3 active.
PG4 migration slice passes104 native laws on fresh PostgreSQL18.6 and1549 workspace
cases; supported Nix hook proves real failures reject builds. Representation
upgrade is separate pending work; PG4 remains active. No release/consumer proof.
