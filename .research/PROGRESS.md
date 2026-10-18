# Current work — 2026-10-18

Persistent goal ACTIVE, no token budget: ALL PG0–PG9 through protected merge,
existing release process, verified publication and a clean registry real-PG
consumer. PG10 conditional. Never complete on local green. User latest asks
status; answered candidly: ten PRs merged, NOT close to publication, release
lock fixed, independent CodSpeed unresolved; broader PG7/8/9 remain. Continue.

Read canonical skill and FULL protocol after recovery:
/Users/joel/nix-config/modules/home/devrandom-research/skill/{SKILL.md,references/protocol.md}.
Already applied/announced. Read repo CLAUDE.md and /Users/joel/.claude/CLAUDE.md.
Strict Clippy: no allow/threshold relaxation, functions80lines/5args.
No framework/runtime/custom dev tool/project Python; existing Rust/SQLx/Tokio/Nix.
No subagents authorized. User requires THREE reviews after EVERY run: contract,
actual defect/falsifier, assertions/skips/timing/source/oracle gaps. Per-run record
in docs/prds/postgres/research.md. Keep genuine reds, no unviable mutant credit.
Immutable .research/CONTRACT; EXPERIMENTS append-only. Historical details and
exact delivery/mutation artifact evidence live in research.md and evidence/.
This progress file replaces stale duplicate states; authoritative current state.

## Protected delivery

TEN merged PRs:376 reads/recovery7db7cf83;377 outcomesf87afc12;379 migrations/native
CIe1ee1dc2;380 binary/unsigned49ffe78f;381 atomicbe2e52ec;382 snapshots090504ae;
383 checkpoints c482fb6a;384 listing88a4575b;385 sessions12fd839c;386 releasefix
87e87d6e00b0d7703a93cf577ec4893a9d49d35e CURRENT known origin/main.
Detailed signed heads/40checks/mutation artifacts/first failures in records.
PR385 merged by joeldsouzax before agent attempted merge. Other merged worktrees
preserved. Root /Users/joel/orca/workspaces/nexus/main remains docs/postgres-prd8357,
with user docs/prds/postgres/research.md UNTOUCHED; root PROGRESS is our mirror.

Release lock failure FIXED actual live workflow39152877808 on87e87, verified App
commit daefe06097f7310ef21a596ca8ee1bb7a5049994 on release PR378. EXACT2fuzz locks,
4local version edits0.4.0=>0.4.1, no registry/source changes. All4 original locked
AFL builds +60s campaigns and4LibFuzzer61s campaigns pass39153035060. Root publisher
skipped all9already-published0.4.0; NO new publication.
PR378 OPEN, head daefe..., ONLY known failed check CodSpeed -10.42%.
Do NOT merge release378 before whole adapter accepted. No threshold/metric/workload
change, baseline reset or manual ack just to green. Three traces identical35ns
instructions; one extra simulated L1/LL miss adds58.333ns to501.667=>560ns.
Layout/cache causality unresolved; CPU differing isn't proof. Rejected inline
experiment25–36% native regression/+512B reverted; A/B/A and0.4.1native~2.6ns don't
waive x86 metric. Original performance branch UNCOMMITTED records preserved/copied.

## Current primary slice: history and restore

Worktree /private/tmp/mnesis-postgres-backup, branch work/postgres-backup based87e87.
UNCOMMITTED staged/unstaged owned changes, NO PR yet. New files tracked for Nix.
Breaking0.xminor permitted STABILITY; use `feat(postgres)!:` normal signed commit;
release-plz handles version. No manual version bump and no hook bypass.
Product: migration5 ONLY (1–4unchanged), persisted16byte History via native UUID,
cluster system_identifier/databaseOID binding, bounded startup max-event-xid
>= native snapshot XMAX rejects unsafe copied transactions. XMAX NOT xmin.
Public History{new/as_bytes}; store.history; PgAllPos::new(history,txid,seq),
position.history;32Bvsold16. No unscoped sentinel. Foreign global resume/checkpoint
write rejects exact HistoryMismatch before pool acquire. Errors DatabaseHistory,
FutureTransaction,HistoryMismatch. Private insert helpers return native tuple to
avoid incomplete public position/extra arguments. Commit uncertainty unchanged.
Checkpoint migration backfills history FK atomically; reads/writes retain scoped
revision/schema/position/state pair; wrong-history/stale/oversized body omitted.
Accounting48not32. Preserve old37byte floor as empty49>37 rejection;53edge/54overflow
explicitly justified before edits. Default limits/all original oracles unchanged.
Runtime SELECT history/events needed even cache-only role; no metadata mutation
rights. Native checkpoint role seven exact42501 denial probes, not originalfive.

Native restore tests483lines: independent18.6clusters, actual pg_dump future
transaction red retained; desired unsafe raw restore rejects. Equality at xmax
asserted. Wrong cluster/db bindings reject, sameDB schema recreation changes
scope, valid newer commit behind older held xmin still opens. Actual fsync-on
pg_basebackup, stop old source before cloneboot, same history/event/cache/checkpoint
metadata, append/resume/CASadvance. Not power loss/crossnode failover/actual
Projection restart proof. Populated-v4 migration collision42P07/step5, rollback,
repair/MAXfields/binary state, checksum1–4preservation/reopen passes.

Budget before edits <=300added/100removed productionRust,<=110SQL,1publictype/
3errors,0kernel/deps. Initial500newnative +80v4 +40equality +70metadata=690 new
native lines;<=90old callsite/fixture edits plus separate<=50existing session
fixture lines allowed after actual Linux red. Actual product~194add/84remove,
SQL~54new, restore483+v4~96+floor~23+foreign~75 fits690; final exact count pending.

Current native evidence before final two fixture followups:210/210PASS37.703s,
0skips; prior209/44.268s. Actual17.10+private18.6, seven permission probes execute.
Strict allfeature/alltargetClippyPASS9.10s before latest metadata/session fixture
followups. Need current full gate (running) + native211 final/matrix/Linux.

Fresh manual controls package-clean own target /tmp/mnesis-pg6a-proof-target.z1jmzk,
source /tmp/mnesis-pg7c-guards: >=to>FAILexact equality; xmaxtoxminFAILvalid held
writer; disablefutureFAILunsafe copy; bypassHistory.checkFAILclosedpool foreign
resume; systembindingbypassFAILfirst; DBbindingbypassFAILsecond. Every real runtime
failure3reviewed. FINAL checkpoint body predicate control preserves four bind
parameters via $4=$4, mismatch errors still pass but allocation6289652>262144FAIL
0.131s. Repaired fixture independently leaves FKcount1/history=t EVEN AFTERfail.
Production predicates never changed; control copy currently has body bypass.

Fixture mistakes kept, not hidden:
- Untyped legacy state array i32=>native42804; fixed0_u8, no product change.
- First new corruption fixture left history/FK altered. Following default run
11pass/1fail/110UNRUN exact Schema; native catalog proves missingFK/historyfalse.
Fix collects actual read/write/pair/alloc outcomes, repairs injected changes BEFORE
all unchanged assertions. Positive run1/1PASS25.357s; final body controlFAIL above.
Original failed none log accidentally reused by matrix; preserved incomplete
TOOL TRANSCRIPT excerpt explicitly labeled in pg7c-default-poisoning-original-excerpt.log.
Do not claim complete original raw log. Other raw reds unique, preserved.
- First Linux full211 cohort175pass/1fail/35UNRUN51.934s,0skips. Unchanged session
fault law kills IDLE backend then sends first query; actual original SQLxIo32
BrokenPipe instead of Database57P01. Not error erasure. Keep raw Linux red.
Fix requires pending query, native pg_stat_activity Lock wait BEFOREtermination,
then SAME exact57P01/fusedcursor/failedclose assertions; no broadened error or
changed production. PrimaryPGdocs signal success != completedtermination. Added
same3s observation pattern/deadline and same60s lifetime. Budget<=50fixturelines.

## RUNNING checks — inspect before restarting

1 Native16featurematrix session40118, /tmp/mnesis-pg7c-feature-matrix.log and
/tmp/mnesis-pg7c-features-<comma-list>.log. SERIAL physical17.10cluster.
Done/reviewed3ways: none122/20.334s; export140/246.059s; import148/41.699s,all0skips.
Export binary predates in-flight57P01fixture, so no final Linux-red resolution
credit. Later feature builds pick current source. Next projection, snapshot,
all pairs/triples then all4. Read EVERYactual result and3reviews per run.
2 Current normal `nix flake check -L` session96884,
/tmp/mnesis-pg7c-history-flake-current.log. Complete unchanged gate, corrected
fixtures. Old pre-cleanup gate31065 agent-canceled by exactPID26280 SIGINT because
obsolete source,1557/1567passed/10stillrunning/4compiler unselected; NOwholepass.
Raw /tmp/mnesis-pg7c-history-flake-gate.log retained. Slow Mac Fjall properties
240s+ not waived, no deadlines/sample counts/durability changed. Current gate must
finish. Readonly sampler of nixbld process denied; no stack/no escalation.
3 CURRENT corrected source Linux native build session84300,
/tmp/mnesis-pg7c-history-linux-second.log, remote aarch64-linux builder.
Old first Linux raw /tmp/mnesis-pg7c-history-linux.log copied project evidence red.
Old5smokes (1each) all3reviewed; filteredsmokes NOT feature-isolated builds.
Native Linux independent physical cluster, okay concurrently with Mac matrix.
No local native suite concurrently sharing physical cluster, even anotherDB.

Next finish matrix/current Linux/current normal gate; exact budget/source audit;
copy useful raw .log evidence (avoid .json/.sql evidence fileset); current docs
README/PRD/backlog draft migration5/releasefix updated. Append actual experiments,
normal signed breaking commit hook nixflakecheck, focusedPR. All CI+reviewthreads,
actual downloaded mutant artifact everyfirstfailure3reviews, protectednormal
squash with --match-head-commit. Never admin/auto merge. Not delivered until merge.
No local generated mutation sweep yet current slice; six manual controls above.

## Whole goal remaining

PG7 bounded completedCBORchunks/Postgres roundtrip/Fjallinterchange, malformed/
truncated/routing/conflict/partial/salvage/sourceerror/cancel, logicaloperator drill
and actualProjection restart after physical/logical recovery. Existing decoder
materializes ONEchunk. No fs::read wholefile bounded claim. Session&self permits
lazy listing and streams without fullIDcollect. Reuse ChunkWriter/supported
minicbor sinks/fixedchunks; no backup format/framework/tool needed.
PG8 TLSVerifyFull/auth/wrongCA/hostname/redaction; roles; startup/reconnect/close/
Drop/callerpool/query/acquire/lock bounds; primaryrestart/durability/sessionoptions;
10k/100k/1m x1/16/128, payloads/competingwriters/percentiles/RSS/alloc/conns/plans,
30-minmixedsoak0violations. Pending session canceled-close-blockedquery/shorter
caller socketinterval/networkpartition scope. Credible source findings NOTyet
native witnessed: read_all unqualified pg_snapshot_xmin/pg_current_snapshot;
constraint deparser caller searchpath; migration metadata fetch_all before count.
PG9 remove EXISTING scripts/check-features.py and embedded fuzz.yml Python with
Cargo-hack/JQ/Nix/just, no replacementtool suite. AFL zero-exitcrash gate fault
proof; event-listener5.4.1 advisory/ignored staleentries; registry advisory blind
spots/yanks; features/downstream/absence gates; docs/example/changelog; two fresh
wholeaudits; all merges then publication/VCS/checksum/cleanregistryPGconsumer.
Do NOT mark goal complete early. Labcast retains command IDs/dedup/workerfencing.

## Environment and preserved work

LocalPG17.10 /tmp/mnesis-postgres-pg0,Unix/tmpport55432,userjoel.
NativeURL postgres:///mnesis_pg7b?host=/tmp&port=55432&user=joel.
Admin control DB mnesis_pg7c_controls created; same physicalcluster, never run
concurrent native suites there. Private restore18.6clusters own socketsport55436.
Nix shell nix develop /tmp/mnesis-postgres-shell-199 -c ... Rust1.99/SQLx0.9/Tokio.
Shared target /Users/joel/orca/workspaces/nexus/main/target. Dedicated18.6mutants
cluster /tmp/mnesis-pg7a-mutants.Pqoy5x STOPPED, socket subdir, usermnesis.
Builder ssh-ng://builder@linux-builder aarch64-linux works. x86nixos.localDNSfails.
Disk last~15GiBfree. No broad GC/cache deletion; earlier raw cache rm auto-review
rejected/user told. Preserve all named worktrees and stashes, no pop by index:
b51e18ce53f0a3e9326676e014d3bdab26778392,fbcc7d919c5df03ee3216065de84a63ddb825992,
65981423d8263e5a00d6df11e62668b3ed4cd2b6,f4fc89d0b83d5411f13eebca269b0cf76c1ef0a5,
219c6aaf1dfc8ea83f6ff6d3a0389b8a8ba0449f,c5bc39b383f1da5bfdf1d3ece2e1dc97075315a2.
Old candidate notes /tmp/mnesis-pg7c-history-candidates.md OUTDATED, do not redesign
already implemented history API from them. Timing PGUSER control .05 reverse
unset .04 did not reproduce; no causal attribution/no performance-parity claim.

LATEST UPDATE: corrected CURRENT Linux COMPLETE211/211PASS51.024s,0skips,
artifact/nix/store/045sg6v2lhkvj0bd94pbyfxh9fynmic4-mnesis-postgres-integration.
Exact in-flight57P01/fusedcursor/failedclose, new metadata cleanup and physical
coherence/v4backfill allpass. Second5native smokes1each all3reviewed. Rawcurrent
Linux and originalred copiedprojectevidence. EXPeriment JSONL appended native
verification, not delivery. Current whole Clippy49.95sPASSall/all/all unchanged.
Native matrix now10/16complete,ALL0skips,3reviews each inresearch:
none122/20.334;export140/246.059;import148/41.699;projection143/24.137;
snapshot143/23.670;export,import167/32.612;export,projection161/27.712;
export,snapshot161/42.642;import,projection169/188.271;import,snapshot169/141.587.
Remaining projection,snapshot +4triples +all4 currentlyrunning40118. No source
changes. Gate96884 current1565/1567done,2FjallpropertiesSLOW360s+ stillrunning,
NOwholepassyet. Earlieroldgatecorrectlycanceled agent(no userpause),rawkept.
ExactsourcebudgetRust186added84removed/SQL51added3removed,47oldctorcalls; new
native~660+permissionfew fits690;22existing sessionfixture lines within50followup.
Nextoperatorproof mustassert oldprimaryshutdownstatus (currentDropignoresexit)
andactualProjectionrestart; no failoverfencingclaim in currentphysicalcoherence.
PG9futuretoolingread-only: currentNixshell lacksCargoHack; lockednixpkgs provides
cargo-hack0.6.45. EphemeralNixpackagehelp session36471 downloading771KBslow,
/tmp/mnesis-pg9-cargo-hack-selected-help.log; NO source/tool/dependency selection
or newbranch yet. ExistingPythoninonefileandfuzzheredoc; supportedCargo/JQ/Nix
replacementfuture. Lockedminicbor2.2.3 nativeWrite &mut[u8] canboundCBORchunks
withoutcustomsink/newdependency. Do not start competingframework/tool.
LATEST ALL16native combinations COMPLETE2652caseexecutions0skips, everycohort3
reviewed research.md. Last projection,snapshot166/37.941; eip188/34.539;
eis188/35.997;eps184/28.930;ips192/28.550;all211/32.791. Matrix40118done.
Current Nixgate96884 COMPLETED EXIT0,1567pass624.962s14slow4compilerunselected.
Raw postprocessaudit-tmpdir BashpipeSIGSEGV emitted despiteexit0; keepdiagnostic
PG9triage/no suppressedcheck/no claimcleanpackagingaudit. Linuxfixupcleanseparate.
DedicatedadapterpublicrustdocfirstFAILtwo preexistingprivate builder/Innerlinks;
FIXEDONLYDOCprose/publicconstructorlinks(no publicexposure/allow) nowPASS2.36s.
ThislastDOCsourcechange requiresnormalhooknewfullgate; allimplementationunchanged.
SourcebudgetfinalRust190add/90removeSQL51add/3remove (verifyagainbeforefinalclaim).
ALLnative/DOC/Linux/fullfirstcompleted logs copiedevidence; EXPERIMENTS JSONLnative
entry appended. PRbodytemporary/tmp/mnesis-pg7c-history-pr.md PENDINGvalidation
update. Nextstageownedfiles andSIGNEDbreakingcommit normalhook, thenfocusedPR
allCIactualmutationfirstfailures3reviewprotectedmerge. NOcommit/PRyet.
Cargohack0.6.45ephemeralNixhelpfinished confirms --each-feature default/none/all,
--no-dev-deps TEMPORARILY MUTATES realmanifestandrestores:NEVERruninprimarywhile
matrix/otherCargoactive. FuturePG9separateworktree; no secondarybranchcreated.
LATEST commit attempt73618ENDED REJECTED OS28diskfull duringnormalhook noStd
synbuild; HEADstill87e87/noPR. Fullraw/tmp/mnesis-pg7c-history-commit.log copied
pg7c-commit-disk-first-red.log. No commits/hookbypass/gradewaiver. AfterALL16finished,
supportedcargo clean -p mnesis-postgres removedONLYgeneratedpackageartifacts
268342files/36.2GiB, frees32GiB. Sources/Git/DB/evidence preserved; deps kept.
Rootsharedtarget artifacts forcurrentnativebinaries/docsarecleaned, rawlogsremain.
OtherNixprocess63786cwd/Users/joel/Code/labcast/labcast independent,leavealone.
Retry SAME normalSIGNEDbreakingcommit, unchangedfullgate noflagsrelaxed. Product
Rust190added90removed/SQL51/3 unchanged. All16native2652/Linux211/publicdocs2.36
sourcechecksremain actual records, finalCIexact signedheadstillrequired.
