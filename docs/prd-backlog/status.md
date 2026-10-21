# Mnesis status and dependencies

Updated: 2026-10-21. Command extraction requirements live in the
[command processor PRD](../prds/command.md). Postgres requirements and acceptance live in the
[Postgres PRD](../prds/postgres.md); detailed checks and three reviews after every
run live in the [research record](../prds/postgres/research.md). A focused merge
or local pass does not establish whole-adapter delivery.

## Postgres adapter

| ID | Requirement | State | Blocked by | Unblocks |
| --- | --- | --- | --- | --- |
| PG0 | Real-database reproduction, baseline and contract decisions | complete | — | PG1, PG2, PG3, PG4 |
| PG1 | Bounded reads and stable positions | complete | PG0 | PG5, PG7, PG8 |
| PG2 | Recovery without notifications | complete | PG0 | PG5, PG8 |
| PG3 | Transaction outcomes and retry safety | complete | PG0 | PG5, PG6, PG8 |
| PG4 | Schema, migrations and representation | complete | PG0 | PG5, PG6, PG7, PG8 |
| PG5 | Atomic writes and import | complete | PG1, PG2, PG3, PG4 | PG7, PG9 |
| PG6 | Aggregate snapshots and projection state | complete | PG3, PG4 | PG7, PG9 |
| PG7 | Export, backup and restore | complete | PG1, PG4, PG5, PG6 | PG9 |
| PG8 | Secure operation, lifecycle and capacity | complete | PG1, PG2, PG3, PG4 | PG9 |
| PG9 | Complete verification, minimization and delivery | complete | PG5, PG6, PG7, PG8 | CP0 |
| PG10 | Standalone subscriber checkpoints, if a selected consumer requires them | candidate | — | — |

## Command processor

CP0–CP9 are required for the new feature. The planning branch is
`docs/command-prd`, based on `27de3b4a4aad59909ffaccdf6fa83e8b256ccca6`.
The user authorized full CP0–CP9 implementation and delivery on 2026-10-21.
CP0–CP2 are protected-merged (PR411/PR413/PR414).
CP3's updated integration passes its full gate; current-head CI and protected delivery remain. Later requirements
retain their unverified status until their own acceptance evidence exists.
[Source audit and actual baseline](../prds/command/research.md) record 27 passing
source tests and one separately executed in-memory measurement. No current
processor/Postgres, migration or new-package registry consumer is certified.

| ID | Requirement | State | Blocked by | Unblocks |
| --- | --- | --- | --- | --- |
| CP0 | Source provenance, contracts, compatibility prototypes and baseline | feature-complete | PG9 | CP1 |
| CP1 | Extract `mnesis-command`, expose Tower Service and align dependencies | feature-complete | CP0 | CP2, CP3, CP4, CP5 |
| CP2 | Typed outcomes and uncertain-write custody | feature-complete | CP1 | CP6, CP7 |
| CP3 | Identity, ordering and deterministic concurrent acceptance | feature-complete | CP1 | CP6 |
| CP4 | Stored format compatibility and public accepted-fact consumption | candidate | CP1 | CP6, CP7 |
| CP5 | Tower admission, observed shutdown, tracing and telemetry | candidate | CP1 | CP6, CP7 |
| CP6 | Real adapter recovery, capacity and instrumentation overhead | candidate | CP2, CP3, CP4, CP5 | CP8 |
| CP7 | Public consumers, documentation, packaging and existing CI | candidate | CP2, CP4, CP5 | CP8 |
| CP8 | Final audits, protected merges and verified publication | candidate | CP6, CP7 | CP9 |
| CP9 | Server registry consumption, archive retirement and final delivery | candidate | CP8 | — |

Source is `controller-archive`; `labcast-server` is the future consumer and
`labcast` is mobile, outside this feature.
Publication is a prerequisite for final registry adoption; the server change
can be prepared earlier. Generic command identity/deduplication move under this
new feature. Labcast retains its domain rules, authorization and effect fencing.
The Postgres feature is not reopened and PG10 is not activated.

## Postgres delivery history

The final delivery statement below is authoritative for PG0–PG9. Earlier
pending/ownership statements are historical records of that feature's scope;
the command processor table above owns the new extraction proposal.

Final delivery2026-10-20: required PG0–PG9 are complete in published0.5.3.
Completion PR407 and release PR404 are protected-merged,42checks pass at each
reviewed head, and all12host checks pass. The clean registry consumer confirms
real Postgres persistence/reopen/resume with zero skips and full cleanup.
[Exact delivery evidence](../prds/postgres/delivery.md) records merge trees,
publication, checksums, source equality and registry-only dependency graphs.
PG10 remains conditional; Labcast retains command IDs, deduplication and fencing.

Historical delivery update2026-10-19: 24 focused changes are protected-merged through PR402
at5b868028. Release39312731245 published mnesis-postgres0.5.1; the primary sparse
registry confirms it is not yanked. This publishes a subset. Diagnostics are
merged; full soak/capacity, operator recovery measurements, remaining feature
and fuzz gates, final audits and registry-consumer acceptance remain open.

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
- PR396 native byte accounting across caller search paths: `766394cd508046dbce3ae6893a1b9c9da805c941`.
- PR397 supported wake dependency without an advisory exception: `3313567f8714f655c7ed65f0fb9be62caa463b0a`.
- PR398 bounded startup migration metadata: `8df11b218d959622a26c743c31ca68e0b8782388`.
- PR399 operational deadlines and unavailable-server shutdown: `e7bb9f4560ddb05a3148a8d5841312ec2ef3afb0`.
- PR402 bounded operational diagnostics: `5b8680280854c397c957140784ba7795bcb488a0`.

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

PG9 dependency slice under verification: unwaived deny reproduces the selected
event-listener5.4.1 advisory.5.4.2 removes the exception and one redundant package;
current unwaived audit of all three locks,11wake integration cases and255actual
native Postgres cases pass. Bare-metal/full gate and protected delivery remain.
This does not close feature-isolation, fuzz crash-gate or broader PG8 acceptance.

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
and head laws. Complete current-source checks and protected delivery passed in
PR396 after all40requiredCIchecks, four255-case native Linux runs, verified
mutation artifact(1actualcatch/6compiler-only/0survivors) and0threads;
SUM/window/integer/catalog and migration-metadata hypotheses remain open.

Current follow-up: dependency PR397 is open at signed205b01c4 with all12 local
checks passing,255 combined native cases and no new packages. PG4 startup-memory
bounds pass257 actual native cases; two unchanged original allocation failures
and two independently failing wrong-bound controls are retained. The metadata
candidate is now based on reviewed PR397 with byte-identical product changes;
combined native/full gate and protected delivery are pending. Operational,
capacity, recovery, feature/fuzz gates, documentation and release evidence remain.
User steering limits security work to explicit required acceptance checks.

Dependency PR397 protected-merged as3313567f8714f655c7ed65f0fb9be62caa463b0a
after all40CIchecks,0threads, four255-case Linux native runs and unchanged full
Nix gates. Runtime migration metadata slice passes257native cases and all12host
Nix checks (1588workspace tests); focused Linux/protected delivery follows.
Publication, remaining operations/capacity and final whole-adapter acceptance
are still pending. Security work remains limited to explicit required checks.
PG8 deadline characterization now passes five actual native cases using supported
caller SQLx/Postgres controls, including post-insert rollback and unavailable-server
shutdown. Three native-setting control failures and one acquisition control
failure reject disabled deadlines. No production API or runtime change. Current
operations worktree is preparing combined-parent verification; this is not yet
protected delivery and does not close diagnostics, capacity or soak acceptance.

Runtime metadata PR398 protected-merged as8df11b218d959622a26c743c31ca68e0b8782388
after40CIchecks/0threads/four257-case native Linux runs. Original two memory
failures and two independent wrong-bound runtime controls remain retained;
sole automated mutant was compiler-unviable, not a runtime catch. Operations
now passes262combined native cases and all12host Nix checks, including1593
workspace tests. Focused Linux/protected delivery follows. Required diagnostics,
capacity/soak, recovery timings, PG9 and publication remain pending.

Operational deadlines are under focused review in PR399 at signed482de3c4:
262native cases/all12Nix checks, with four actual wrong-configuration catches and
no production API change. Required diagnostics now passes256candidate-base
native cases and a meaningful wrong-label control; parent integration is running
before its first signed full gate. Optional tracing adds no package/runtime.
Capacity/soak, recovery measurements and PG9 delivery still remain.

2026-10-20 delivery update before final publication: PR405 protected-merged at
8b0ee9778e13d87dc3d45ff23f26c60d285fe6cc after40successful checks and0threads;
ordinary merge tree verified, concurrent release403/.5.2 preserved. Sustained
1800s soak passes2662995events/31recoveryfaults and exact cleanup; three logical/
physical operator timing repeats and phase-specific55P03 laws are recorded.
Capacity full matrix passes locally on .5.2:12fixtures/432read+432appendwindows/
96auto-and-generic plans/12closes; analysis and focused protected delivery remain.
PG9 current slice removes the Python feature wrapper, uses supported cargo-hack/
Hakari, adds Postgres isolation, native default/each-feature suites and AFL saved-
crash verdict. Normal gate, actual Linux feature execution, real AFL negative
control and protected delivery remain. A clean registry .5.2 seven-profile build
and private native checkpoint/reopen/resume consumer pass; final acceptance,
publication revision and registry evidence must still match the final delivery.

Historical acceptance update before final publication, 2026-10-20: [per-requirement evidence](../prds/postgres/acceptance.md)
maps every PG0–PG9 item to its owner and actual verification. PG0–PG8 are
feature-complete; PG8 capacity has all12native fixtures/864timed windows and
96plan observations, with protected integration pending. PR406 is protected-
merged at c1ec43ed (expected/actual tree c92a4e6399986474e46e6bad7d10e3a46782b09d):
42successful checks, zero unresolved review threads, complete actual native
all/default/export/import/projection/snapshot/tracing suites264/149/168/176/
173/174/150, each zero skips and owning mandatory-law checks. Real AFL saved-
crash control proves a zero engine exit still fails the verdict and retains all
three crash files in uploaded artifact12279449876; intentional-crash source is
never merged. Two fresh complete source/contract audits found no outstanding
high-value hypothesis; final changes correct two inaccurate owner comments.
PG9 remains active until final protected merge, publication and exact clean
registry-native consumer. Earlier published0.5.2 evidence does not complete it.
