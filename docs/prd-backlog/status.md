# Postgres status and dependencies

Updated: 2026-10-18. Requirements and acceptance live in the
[Postgres PRD](../prds/postgres.md). All rows are implementation states; writing
this PRD does not satisfy their acceptance gates.

| ID | Requirement | State | Blocked by | Unblocks |
| --- | --- | --- | --- | --- |
| PG0 | Real-database reproduction, baseline and contract decisions | feature-complete | — | PG1, PG2, PG3, PG4 |
| PG1 | Bounded reads and stable positions | active | PG0 | PG5, PG7, PG8 |
| PG2 | Recovery without notifications | active | PG0 | PG5, PG8 |
| PG3 | Transaction outcomes and retry safety | candidate | PG0 | PG5, PG6, PG8 |
| PG4 | Schema, migrations and representation | candidate | PG0 | PG5, PG6, PG7, PG8 |
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

Implementation PR: none. Required acceptance/CI: pending. Merge: none.
Published adapter version and registry-consumer verification: pending.
