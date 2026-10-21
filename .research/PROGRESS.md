# Command processor delivery — CP5 telemetry, 2026-10-21

Persistent CP0–CP9 goal ACTIVE/no token budget. Completion requires every
acceptance criterion, protected merges, publication, clean registry/native and
labcast-server consumers. Preserve archive/mobile/unrelated main/release work,
all older/user work and stashes. No subagents/framework/runtime/custom dev tool
or project Python. Canonical skill and full protocol reread after recovery.

CP0–CP4 protected-merged (PR411/413/414/415/416). Latest PR416 reviewed786dce7130,
merge8fc562836ce5c665838ac3db3d3d26cda960be33 at21:01:03Z. Expected/actual/head/
virtual-merge tree24680399552aca8b52f383bcc3c79862669401b3;42checks pass, one
scheduled skip, zero threads. Both CI workflows1633PASS/143binaries/five existing
exclusions, all actual74invocations/workflow and eight fuzz campaigns three-
reviewed. Mutation baseline39PASS/two correct name-oracle catches/eight compiler-
only unviable/zero survivors. cp4-delivery.tar.gz SHA
34c0b9db19e8bcb3752c37b6700ed9da1fc5de77fc797cb77e6e962cf2dd229f.
Earlier original failures, negative controls and delivery receipts stay in their
owning research/evidence archives; no repeated or native command credit inferred.

CP5 core: clean /tmp/mnesis-command-lifecycle feat/command-lifecycle signed
1539509ae5c60977a6fb77469f5ae894e06e3e98, tree29d5408e7c9eb59eec31cdc0731011c37828c65a.
Protected-main integration signed1539509ae5c60977a6fb77469f5ae894e06e3e98,
tree29d5408e7c9eb59eec31cdc0731011c37828c65a; final diff ONLY10hypothesis lines.
Six conflicts resolved to tested owners after incoming main tree==already-imported
786dce7130 proof; normal hook all12cached PASS, no new runtime credit. Draft PR417
now main, pushed without force; fresh push39676293555/PR39676300194 and mutation
39676299812/fuzz39676299874. Latest41SUCCESS/one scheduled skip/one push Nix
check running, no failures. PR-side Nix and both real Postgres integrations pass;
full logs still need invocation-by-invocation review. Fuzz eight campaigns pass
and three-reviewed in cp5-core-final-fuzz-reviews.md (not yet archived).
Must review exact heads/results/artifacts before merge.
Current mutation39676299812/artifact12382418496 now independently three-reviewed:
45PASS baseline,9actual intended runtime catches/11inspected compiler-only,
0missed/timeout. Archive SHA
b021eae7c76e8e3ccc741f2b2c1be825af17288353d9d7008806a78f4791122a.
Old a8push39672707143 automatically canceled by existing concurrency policy;
65complete actual invocations+one interrupted native profile+one echoed summary
three-reviewed; workspace1639PASS138.877s, interrupted168 reports25PASS only.
Core repairs observed stop/dequeue acceptance and reentrant readiness deadlock,
and abort/panic report custody. Native Worker retains original JoinError AND
unresolved request; one Engine-owned pending handed off on Drop. No per-request
Arc/lock/detached monitor. Controls baseline4PASS/two intended catches. Resource
UTF8/escapes4095/4096/4097, whole-record overhead, queue/deadline extrema and
disabled timer native-panic child retain exact outcomes and bounded cleanup.
All12 local gates pass,1639PASS/143bins/five original exclusions/15slow/458.854s,
45command laws, plus three fresh2/1/1replays, every run/child three-reviewed.
Gate SHA6c06675547b52701561d282971a44c7f04e075ba9f7668792d4a651ed976c60e.
Full filtered source ten-owner cmp; README excluded, packaging remains CP7.

ACTIVE dirty /tmp/mnesis-command-observe feat/command-observe based on exact
signed core1539509a. Complete binary/index patches, status/source hashes/old stash
list saved cp5-stages-before-parent*. One named retained stash ID in
cp5-stages-parent-stash.sha; all ten code/manifest/test hashes match after
integration. Union append-only histories; keep final core inventory/PRD/gate
evidence. Older/user stashes untouched. Primary command-prd remains clean.

Tracing feature/private Scope+Trace candidate: two local app subscribers,
caller context across queue, no persisted context/payloads; worker root separate.
Real red missing instrumentation, then native registry panic on wrong-dispatch
final span cleanup, backtrace-only same red, then corrected Scope-owned final
reference+Drop under captured dispatcher passed one law0.011s. All three-reviewed,
failures retained. Latest cfg-zero-size disabled source and added abort/panic/
closed-span/finite-value assertions pass3/3in0.011s e657edd8. Independent removal
of ONLY Scope Drop reproduces native registry panic/readClosed (3ac6ac37), source
restored and hash verified. Final enabled48PASS1.639s (97a1fd67), disabled45PASS
1.492s (9c484f39), one original ignored/six binaries; each invocation and both
bounded children three-reviewed. Strict enabled0.73s/disabled1.17s lint passes;
all earlier compiler/lint failures retained, no production waiver. Feature-off
const/no-op definitions keep tracing disabled without dummy runtime work.
Original slice237net product/449net tests against250/450ceilings;11public roots unchanged.
Trace evidence SHA5015ed1a37018f268ad5866918f38e5198f27a86ad90fe22f8973c8499389f2f.
Stage extension now covers admission/queue/load/decision/append/reconcile and
actual worker cleanup with finite outcomes. Load includes replay; append includes
decision/encoding/persistence/fold. Cleanup trace is not stop-to-join latency.
Two invalid requests under direct/Tower APIs retain typed byte errors and release
admission before valid persistence. Removed-invalid instrumentation control fails
after data checks; removed Scope Drop control also fails. All failures retained.
Combined trace budget387net product/649net tests; final324net product/1643whole
physical production,596line trace suite plus44line neutral fixture. No extra root.
Final signed-parent default349465d0:45PASS/1.539s, release-all7b9c4315:49PASS/0.536s,
both six binaries/one original ignore. Every run and each bounded child three-
reviewed. Enabled lint1.57s and latest disabled1.80s pass without production waiver.
Source frozen for full12Nix gate. Original stage/control/source-parent evidence
cp5-stages.tar.gz SHAc4b1aec492738e7d75d46a61195933e0806084912691fabdc1b1a4b607705e98.
Final current-source tests/patch/ten hashes/parent archived cp5-trace-final.tar.gz
SHA64cefa88d55737f29699525d749b0e959c8bea1dc7dab4a099762fc740aebcde.
Only this goal-owned observe Cargo target cleaned (26124 files/4.6GiB reported,
7.1GiB filesystem free afterward); all user projects/source/caches remain.
Whole-workspace Nix gate NOW RUNNING handle91717, raw cp5-trace-flake.log,
normal nix flake check -L --max-jobs1. All ten source hashes verified immediately
before launch. Poll same handle; do not start overlapping local compilation.
Review every actual fresh test invocation and bounded child three times.
Whole-workspace Nix gate then signed
focused tracing PR; metrics and all four combinations still required. No full
tracing/telemetry/CP5 delivery claim; candidate uncommitted, no hook bypass.

Metrics still REQUIRED, not implemented. Supported metrics facade0.24.6 qualified;
metrics-util0.20.4 DebuggingRecorder snapshots RESET gauges/counters, unsuitable
for live intermediate gauge assertions. Registry/AtomicStorage source inspected.
See HYPOTHESES for admission/queue counter causality, existing native permit
ownership, pending Option as sole protocol authority, late acceptance versus
duplicate counting, bounded labels and accurate outcome/duration requirements.
Applications own collectors/exporters/OpenTelemetry. No normal SDK/runtime.

Then CP6 real Postgres/Fjall command/recovery/migration/capacity/performance and
15-minute soak, CP7 docs/public consumers/features/fuzz/packaging/CI, CP8 two
audits/protected release, CP9 registry/server. publish=false remains intentional.
No command publication or completed consumer claimed. Existing PG0–PG9 complete;
PG10 conditional. No unrelated security work or broad workflow changes.

Raw evidence /tmp/mnesis-command-start-20261021. Nix shell91477 cwd observe idle
between commands; tests stdin/dev/null; no overlapping local compilations.
Current cp4-fixed-check-* and cp5-core-check-* outputs/common deps rooted;
specific superseded goal caches only retired after evidence/pass/liveness checks,
never global cleanup. Check disk before next full gate. Wait actual handles;
observation timeout is not process completion or a reason to restart. Every
actual test run gets three distinct reviews; never weaken assertions/workloads/
filters/deadlines/gates.

LATEST protected delivery: PR417 MERGED972d2a696e021603dd9a03471c6c3fa36878933a
at21:58:34Z; head1539509a/expected/virtuald34deff/actual all tree29d5408e.
42SUCCESS/1scheduledskip/zero threads, normal exact-head squash. Both final CI
logs reviewed: PR74actual invocations, push54 (standalone Postgres output was
cached/echoed, not another20fresh runs). Both1639PASS147.192s/152.027s; every
actual run and both bounded children three-reviewed. Eight fuzz reviews archived.
cp5-core-delivery.tar.gz SHA5c1b5717b19f46aa8a280249762fd52162bb2aafe9168b5fe07af62813fd645a.
Core clean; preserve it. Observe source gate91717 remains live; source untouched.

Dependent metrics worktree /tmp/mnesis-command-metrics feat/command-metrics
created from1539509a + exact frozen tracing binary patch; all ten source hashes
verified before metrics edits. Budget<=300product/500tests/10owners (generated
lock/hakari separate),1950whole physical checkpoint. Metrics characterization
written, NOT EXECUTED: actual active+queued+reserved native capacity, dropped
unpolled caller, clone/repeated-ready reservations, exact data/receipts and
late-commit reconciliation acceptance once. Do not instrument before its red.
No concurrent local compilation; wait current gate's terminal state first.
Facade0.24.6 qualified; actual registry dev dependency generated unwanted normal
RNG edges via hakari, so rejected and recorded. Use facade-native atomic handles,
standard HashMap/parking_lot and passive histogram Vec only in recorder fixture.
Minimal metadata/hakari/tree regeneration done; verify generated hack returns
unchanged and only facade+rapidhash(+32bit portable-atomic) lock additions.
Record falsifiers/remaining abort/panic/invalid/finite-label/histogram cases before
product instrumentation. No collector/export task/runtime or public API added.

LATEST TRACE GATE:91717 TERMINAL exit0, all12host PASS. Actual workspace
618cde1d:1639PASS/144bins/5existing exclusions/15slow/602.287s; four trace laws
not selected by default gate, separate enabled49release remains authoritative.
All three fresh2/1/1replays, workspace and each bounded child three-reviewed in
research. Ten actual source owners cmp to gjbwsysi6bhagiycqahyc9kngshpncx2-source.
Cached check roots being created; do not infer repeated runtime. Metrics red
NOW compiling through shell91477 (cwd metrics), raw cp5-metrics-before.log; wait
same shell and three-review actual outcome before any metrics source edits.
Next preserve exact observe work/evidence then zero-commit rebase onto merged
972d2a69, restore retained stash/hash verification, normal signed hook/focused PR.

LATEST before trace retention: all12gate and all12root paths compare exactly,
SHA8675719d5e2689b074c51d8667a4ef25db74090431030544617683b6a9a56134 for cp5-trace-gates.
Observe now972d2a69 after zero-commit rebase/exact retained stash91060fa1;
ten source and seven archive hashes unchanged. Normal signed commit/focused PR
next, not merged/published. Dependent metrics owning law26dc39fd compiled15.63s
and failed intentionally1/1for missing four native busy gauges AFTER all actual
admission/ordered persistence/join checks. Three reviews in metrics records;
later four profiles not reached/no credit. Metrics product instrumentation still
unimplemented; raw cp5-metrics-before.log. Full CP0–CP9 goal remains active.
