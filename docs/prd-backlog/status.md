# Postgres status and dependencies

Updated: 2026-10-19. Requirements and observable acceptance live in the
[Postgres PRD](../prds/postgres.md); detailed checks and three reviews after every
run live in the [research record](../prds/postgres/research.md). A focused merge
or local pass does not establish whole-adapter delivery.

| ID | Requirement | State | Blocked by | Unblocks |
| --- | --- | --- | --- | --- |
| PG0 | Real-database reproduction, baseline and contract decisions | feature-complete | — | PG1, PG2, PG3, PG4 |
| PG1 | Bounded reads and stable positions | active | PG0 | PG5, PG7, PG8 |
| PG2 | Recovery without notifications | active | PG0 | PG5, PG8 |
| PG3 | Transaction outcomes and retry safety | active | PG0 | PG5, PG6, PG8 |
| PG4 | Schema, migrations and representation | active | PG0 | PG5, PG6, PG7, PG8 |
| PG5 | Atomic writes and import | active | PG1, PG2, PG3, PG4 | PG7, PG9 |
| PG6 | Aggregate snapshots and projection state | active | PG3, PG4 | PG7, PG9 |
| PG7 | Export, backup and restore | active | PG1, PG4, PG5, PG6 | PG9 |
| PG8 | Secure operation, lifecycle and capacity | active | PG1, PG2, PG3, PG4 | PG9 |
| PG9 | Complete verification, minimization and delivery | blocked | PG5, PG6, PG7, PG8 | — |
| PG10 | Standalone subscriber checkpoints, if a selected consumer requires them | candidate | — | — |

Protected focused merges:

- PR376 bounded reads and recovery: `7db7cf83f74d9ae53b45ca9aae6c548572c73e91`.
- PR377 transaction outcomes and races: `f87afc12dd6dabe4acd7847e98756858f5fb4c86`.
- PR379 migrations and native CI: `e1ee1dc29c30b0116da14e4cd595a4d8c0c7beb1`.
- PR380 lossless values and binary keys: `49ffe78f166b7d6a787ea898dfba454218a697e9`.
- PR381 atomic writes and import: `be2e52ec3468657d36684506eed53fe060bef080`.
- PR382 aggregate snapshots: `090504ae3b2081c03cfd8960f6edc5c0058fb3b2`.
- PR383 projection checkpoints: `c482fb6a3762c74d7b10416029de22a642de2b71`.
- PR384 bounded listing: `88a4575b3a3d0c5db8aaac2b92241c76b19f9a60`.
- PR385 consistent export sessions: `12fd839c7b08e2eda1a8020bae9f52dd898c1d15`.
- PR386 release lock synchronization: `87e87d6e00b0d7703a93cf577ec4893a9d49d35e`.
- PR387 history-scoped positions and coherent recovery: `2b181b12d1040f1527d6a28375fd5db91a9d1129`.
- PR388 global ordering across caller search paths: `651aabf35e94b7b0e98e9e87eff19f6964df95f3`.
- PR389 completed bounded backups and logical projection recovery: `49b32374041843a87980fc6f74a945f51f0b9ea3`.
- PR390 verified physical backups and restored projection consumer: `2247c739525f9d8765179b7d9f7f67c6b43bd32f`.
- PR391 backup route and failure cases: `49104117b87010206fb4b25be1b4d3d6e6635b22`.
- PR393 decoder performance repair: `f207923a6131ea0270e0e8b22810d8cb3afb7adc`.
- PR394 stream and saved-state identity, committed heads and independent mutation fixtures: `5ef3e742c63a2f4078ec28057559b5439af28234`.
- PR392 verified TLS and restricted runtime permissions: `8f03ae8553ab6410fdfbe7660ab84ec0873efae0`.
- PR395 native durability profile and phase-accurate transaction outcomes: `f7d390d4c50936b2912207c2d417a8c24a899e0c`.

PG1/PG2 still need remaining capacity, lifecycle and recovery profiles. PG3 now
has merged actual native SSI/deadlock and snapshot uncertainty witnesses;
projection propagation is under native verification. PG4's supported upgrades preserve
rows, resume positions and sequence state; later capability migrations remain
subject to the same obligations.

PG5 merged after139 native cases,113 default-feature cases,26 mutation candidates
(12 caught/14 unviable/no survivors), all40 CI checks and no review threads.
PG6 aggregate snapshots merged after163 native cases including codec/aggregate
composition, client SIGKILL, restricted login, corruption/byte bounds, migration
rollback/repair and coherent schema-view cancellation. All current feature
combinations, corrected-source Linux, full gate and40 required checks passed in
[PR382](https://github.com/devrandom-labs/mnesis/pull/382). Final CI mutation
artifact independently reviewed:21 caught,2 unviable,0 survivors/timeouts.
The first mutation
result was rejected for fixture poisoning; a later native suppression regression
found and fixed a falsely acknowledged cache write. Conditional
projection checkpoints merged in PR383. Native checks cover CAS races,
bounds, full unsigned values, stale/corrupt metadata, real projection consumers,
commit uncertainty/cancellation, restricted LOGIN and migration collision/repair.
All eight isolated feature combinations pass (118–184 native tests, zero skips),
Linux passes184 plus three feature smokes, and16 mutations yield8 caught/8
unviable/zero survivors. Three fresh controls reject actual lock omission,
unbounded body transfer and mixed checkpoint reads. A native inheritance witness
found and fixed unsafe schema acceptance. Full flake gate, all40 CI checks and
protected delivery passed in [PR383](https://github.com/devrandom-labs/mnesis/pull/383).
The downloaded CI mutation artifact confirms8 caught/8 unviable with every first
failure reviewed. PG7 bounded listing merged in PR384. Its native
laws found and corrected committed-stream omission behind an unrelated held
transaction, and caller search paths redirecting visibility functions. Eight
listing laws pass, with three fresh SQL controls and the downloaded CI mutation
artifact reviewed before protected merge. Consistent sessions merged in PR385
after203 native cases, all40 checks and22 reviewed mutations (9 caught,
13 unviable, zero survivors/timeouts). Their first five passing laws missed blocked-query and caller-pool
cleanup defects; both original reds are retained, with supported native socket
checks and pool-close ownership now passing eleven focused laws. Fresh isolation
and native-disconnect controls reject the original defects. Backup composition
and restore remain. The active history slice rejects unsafe copied transaction
IDs, scopes positions and checkpoint state, and tests physical recovery and
populated migration rollback/backfill. Protected-merged in PR387 after all40checks
and reviewed native mutation evidence; not released. Global-read search-path
regressions are protected-merged in PR388 after215native/Linux laws, all40checks
and original-source/runtime-control falsification. Seven bounded completed CBOR
backup laws now pass222native/Linux cases, including Postgres/Fjall interchange,
source failure/cancellation and actual logical projection rebuild/restart. They
are protected-merged in PR389 after all40checks and zero unresolved threads.
The physical operator drill passes224native/Linux laws, verifies manifest/WAL,
checks old-primary shutdown and resumes the actual saved projection before a
new append/advance; it is protected-merged in PR390 after all40checks and no
unresolved review threads.
Six new completed-chunk laws now pass230 combined native cases, including binary
caller routing, late duplicate/final-conflict policy reports, snapshot consistency
across an overlapping atomic commit, missing/oversized final inventory files and
explicit salvage with retained provenance. Two fresh native controls reject
ordinary-view substitution and omitted inventory preflight. Protected-merged in
PR391 after230/230Linux laws, the unchanged signed full Nix gate, all40CIchecks
and zero unresolved threads. PG7 is still active: logical operator timing/profile
evidence remains to record. PG8 verified TLS and restricted runtime role laws
are protected-merged in PR392, with supported SQLx TLS explicitly selected and
separate certificate/name verification controls rejecting both weakened policies.
Durability, remaining lifecycle/recovery/diagnostics and capacity/soak evidence
remain required; no full operations or release claim follows these focused laws.
Release lock synchronization is protected-merged in PR386. Its live workflow
created a verified App commit on release PR378; all four original locked AFL
builds and campaigns pass. The original10.42% decoder regression is retained in
the research record. PR393 repairs it without changing benchmarks or thresholds;
release PR378 at c5ec958221a3f488af3258532e8c8ef2c4480d74 now has all40 required
checks passing, including actual CodSpeed. This is not final adapter acceptance.
No publication is claimed. All prerequisite edges stay visible.

PG10 requires a selected consumer and explicit acknowledgement contract before
activation. Labcast owns command IDs, deduplication and stale-worker rejection.
Published adapter version and clean registry-consumer verification: pending.

PG4–PG6 acceptance reopened: native caller-name witnesses against protected main
49104117 returned first-body when asked for second's stream under a caller
search path with application bytea equality. Schema-qualified tables alone do
not preserve stream identity. Original red and three reviews are retained in
the research record. Follow-up native failures prove a forged committed head and
misrouted snapshot writes and snapshot/checkpoint reads. Minimal query repairs
pass the original oracles and are protected-merged in PR394 after actual mutation
artifact review rejected fixture-poisoned catches. Atomic/import share the repaired head owner; no blanket claim
that all remaining query-name hypotheses are settled.

The query repair is protected-merged in PR394 from signed4ac45d46, with235 native
cases and all12 local Nix checks passing, all40 required CI checks and no unresolved
threads. Independent artifact review accepts14 runtime catches and2 compiler
unviable cases; the first eight contaminated catches were rejected and retained.
TLS PR392 is protected-merged after240 combined native cases and all12 local checks,
all40 final required CI checks passing and zero unresolved threads. Neither adds
durability or capacity proof; byte-accounting/integer/catalog hypotheses stay open.

The dependent durability candidate now passes251 combined native cases and140
complete no-default cases, including profile/restart laws and separate phase
oracles. A conservative250-green candidate was rejected: deferred40003 now fails
before COMMIT, where the owned transaction is known uncommitted. A new actual
client-COMMIT-boundary test preserves40003 uncertainty; both wrong mappers have
native failing controls. Actual post-server-COMMIT lost-ack/timeout laws remain.
The first signed gate failed before nextest execution from storage exhaustion;
original failure retained, no bypass. The corrected signed gate passes all12
checks with1585 instrumented workspace cases passing. PR395 is protected-merged
after all40requiredCIchecks, four actual251-case Linux native runs, verified
mutation artifact6runtimecatches/3compilerunviable/0survivors and zero threads.
No capacity or release acceptance is inferred.

The next accounting slice reproduces four native byte-limit bypasses when a
caller application supplies octet_length(bytea). Normal larger-limit writes stay
valid; bounded readers must still return exact RowLimit errors. Minimal native
function qualification passes all9names laws, including the original identity
and head laws. Complete current-source checks and protected delivery are pending;
SUM/window/integer/catalog and migration-metadata hypotheses remain open.
