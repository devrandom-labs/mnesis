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
| PG5 | Atomic writes and import | feature-complete | PG1, PG2, PG3, PG4 | PG7, PG9 |
| PG6 | Aggregate snapshots and projection state | active | PG3, PG4 | PG7, PG9 |
| PG7 | Export, backup and restore | blocked | PG1, PG4, PG5, PG6 | PG9 |
| PG8 | Secure operation, lifecycle and capacity | blocked | PG1, PG2, PG3, PG4 | PG9 |
| PG9 | Complete verification, minimization and delivery | blocked | PG5, PG6, PG7, PG8 | — |
| PG10 | Standalone subscriber checkpoints, if a selected consumer requires them | candidate | — | — |

Protected focused merges:

- PR376 bounded reads and recovery: `7db7cf83f74d9ae53b45ca9aae6c548572c73e91`.
- PR377 transaction outcomes and races: `f87afc12dd6dabe4acd7847e98756858f5fb4c86`.
- PR379 migrations and native CI: `e1ee1dc29c30b0116da14e4cd595a4d8c0c7beb1`.
- PR380 lossless values and binary keys: `49ffe78f166b7d6a787ea898dfba454218a697e9`.
- PR381 atomic writes and import: `be2e52ec3468657d36684506eed53fe060bef080`.

PG1/PG2 still need remaining capacity, lifecycle and recovery profiles. PG3 now
has merged actual native SSI/deadlock witnesses; snapshot uncertainty has candidate
evidence and projection propagation remains required. PG4's supported upgrades preserve
rows, resume positions and sequence state; later capability migrations remain
subject to the same obligations.

PG5 merged after139 native cases,113 default-feature cases,26 mutation candidates
(12 caught/14 unviable/no survivors), all40 CI checks and no review threads.
PG6 aggregate snapshot candidate passes163 native cases including codec/aggregate
composition, client SIGKILL, restricted login, corruption/byte bounds, migration
rollback/repair and coherent schema-view cancellation. All current feature
combinations, corrected-source Linux and clean mutation sweep pass; final full
gate, CI and focused delivery remain pending in
[PR382](https://github.com/devrandom-labs/mnesis/pull/382). The first mutation
result was rejected for fixture poisoning; a later native suppression regression
found and fixed a falsely acknowledged cache write. Conditional
projection checkpoint implementation remains the next PG6 slice.
All prerequisite edges stay visible after prerequisite acceptance.

PG10 requires a selected consumer and explicit acknowledgement contract before
activation. Labcast owns command IDs, deduplication and stale-worker rejection.
Published adapter version and clean registry-consumer verification: pending.
