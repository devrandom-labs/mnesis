# Postgres status and dependencies

Updated: 2026-10-18. Requirements and observable acceptance live in the
[Postgres PRD](../prds/postgres.md); detailed checks and three reviews after every
run live in the [research record](../prds/postgres/research.md). A focused merge
or local pass does not establish whole-adapter delivery.

| ID | Requirement | State | Blocked by | Unblocks |
| --- | --- | --- | --- | --- |
| PG0 | Real-database reproduction, baseline and contract decisions | feature-complete | — | PG1, PG2, PG3, PG4 |
| PG1 | Bounded reads and stable positions | active | PG0 | PG5, PG7, PG8 |
| PG2 | Recovery without notifications | active | PG0 | PG5, PG8 |
| PG3 | Transaction outcomes and retry safety | active | PG0 | PG5, PG6, PG8 |
| PG4 | Schema, migrations and representation | feature-complete | PG0 | PG5, PG6, PG7, PG8 |
| PG5 | Atomic writes and import | active | PG1, PG2, PG3, PG4 | PG7, PG9 |
| PG6 | Aggregate snapshots and projection state | blocked | PG3, PG4 | PG7, PG9 |
| PG7 | Export, backup and restore | blocked | PG1, PG4, PG5, PG6 | PG9 |
| PG8 | Secure operation, lifecycle and capacity | blocked | PG1, PG2, PG3, PG4 | PG9 |
| PG9 | Complete verification, minimization and delivery | blocked | PG5, PG6, PG7, PG8 | — |
| PG10 | Standalone subscriber checkpoints, if a selected consumer requires them | candidate | — | — |

Protected focused merges:

- PR376 bounded reads and recovery: `7db7cf83f74d9ae53b45ca9aae6c548572c73e91`.
- PR377 transaction outcomes and races: `f87afc12dd6dabe4acd7847e98756858f5fb4c86`.
- PR379 migrations and native CI: `e1ee1dc29c30b0116da14e4cd595a4d8c0c7beb1`.
- PR380 lossless values and binary keys: `49ffe78f166b7d6a787ea898dfba454218a697e9`.

PG1/PG2 still need remaining capacity, lifecycle and recovery profiles. PG3 now
has actual native SSI/deadlock witnesses in the PG5 candidate; propagation through
snapshot/projection consumers remains required. PG4's supported upgrades preserve
rows, resume positions and sequence state; later capability migrations remain
subject to the same obligations.

PG5 candidate passes 138 cases on local Postgres17.10 and fresh Linux18.6, plus
113 default-feature native cases. A subsequent review added direct local wakes
for every atomic target; its selected case passes, final139 checks are pending.
Final mutation, workspace gate, focused PR/review and protected merge remain open.
All prerequisite edges stay visible after prerequisite acceptance.

PG10 requires a selected consumer and explicit acknowledgement contract before
activation. Labcast owns command IDs, deduplication and stale-worker rejection.
Published adapter version and clean registry-consumer verification: pending.
