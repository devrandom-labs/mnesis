# Command processor delivery — CP3 merged / CP4 qualified / CP5 custody, 2026-10-21

Persistent CP0–CP9 goal ACTIVE/no token budget. Complete only after required
acceptance, protected merges, verified publication, clean native registry and
labcast-server consumption. Source archive/mobile/unrelated main/releasePR410,
all user work and older stashes preserved. No subagents or new tooling/runtime.

## Delivery

CP0 PR411 mergeca62b557; CI repairPR412 mergedefec41a; CP1 PR413 mergefa0b95b1;
CP2 PR414 merge10d1b3b31844bebe2a941ead80343eee592d8584. Detailed receipts/raw
archives in command/research.md/evidence. Publication remains disabled.
CP3 PR415 protected-merged98abf662278c40a730b88fdbe6814481f1db108e at19:43:46Z,
reviewed signedc3a16f3c2179e30cbfe0b11e22893aab95cd44f8. Expected/actual/head tree
680cbebd645ddf7c98894fc4dad847fffe36f217.42successful checks/one scheduled-only
skip,all required green,0threads,normal exact-head squash. Both CI workflows
39659790301/39659801603 have74 actual invocations (75printed,oneecho),1629PASS
105.926s/116.890s and two native adapter services each. Every run three-reviewed.
Eight fuzz legs reviewed; mutation selected0/no runtime credit. Local gate1629
passes449.181s and disposable counted192-inversion witness asserts384 entries.
Final probe removed. Evidence cp3-delivery.tar.gz SHA
914f642352278eeb6e7abe1cde95d8dd07a3440d801528a194a90e3c21cc5fa7.

## CP4 primary

This worktree /Users/joel/orca/workspaces/nexus/command-prd is feat/command-reader
atc3a16f3c, with staged reader/records/evidence. Protected main98abf662 has EXACT
same tree as this parent; integrate it normally before final PR without losing
source or histories. Preserve all named stashes/binary backups, especially
cp4-fixed-parent-stash.sha; all eight CP4 code/fixtures equal saved reader work.
Public Acceptance/AcceptanceCodec +62product lines/two roots;626test lines under
650 ceiling. Original v2 writer bytes/reader/reverse rollback, physical-record
projection checkpoints, command+fact evolution and independent corruption/no-
repair controls qualify locally; final combined delivery remains pending.

Prior full CP4 all12 gate:1633PASS485.577s/143binaries (prose142 corrected), source
3kdzh5mq. It predates corrected CP3 helper. New combined source ykg0amq07qkhsjzb85fb0irjwlnh886b:
11outputs pass, nextest3fb5932a fails1315PASS/11FAIL/1326executed of1633selected,
307notrun/five existing exclusions,250.067s. All39command laws pass. Ten failures
are actual OS28; one unchanged macro overflow audit REPORTS unexpected compile,
exact cause UNKNOWN. trybuild1.0.118 classifier ignores selected process status/
stderr and defaults success on missing stdout JSON; report is not actual proof
of successful compilation. Do not call it another disk error. Full daemon log
cp4-fixed-nextest-failed.log and all three reviews retained. No weakened tests,
workloads, filters, thread counts, assertions, limits or deadlines.

CURRENT exact unchanged nextest retry COMPLETED0 session85887, log
/tmp/mnesis-command-start-20261021/cp4-fixed-nextest-retry.log. Derivation verified
identical5ay07acsfij6bhsd24q0rwgdmys0mihj; ten source/fixture owners cmp match ykg.
Runb938c04e-b0da-4195-996c-674959d70f4e passes1633/1633 in496.681s,15slow,
five exclusions/143binaries. Three reviews recorded; all39command laws pass.
Compiler fixture passes100.495s; original cause remains unknown and raw retained.
Eleven other outputs rooted cp4-fixed-check-*; dependency xagzqq5... genuinely
rebuilt/rooted cp4-fixed-common-deps. After pass normal signed hook/commit,
protected-main integration/current-head focused PR/CI/merge still required.

## CP5 isolated work

/tmp/mnesis-command-lifecycle, feat/command-lifecycle at26c5a334, staged/uncommitted.
First real lifecycle reds/fixes: assigned readiness cancellation reenters stop
under lock and deadlocks; stop during dequeue still accepted queued work. Initial
weak close-only child retracted. Exact red/green first evidence cp5-lifecycle.tar.gz.
Native abort then exposed lost shutdown report despite observed late commit.
Engine now retains one owned pending and moves it only on Drop to private per-
worker handoff; public Worker waits for native task/destructors and returns scoped
Shutdown with original JoinError AND unresolved original. No per-request Arc/
lock/detached monitor. Native panic payload73/fold payload7, canceled borrowed
wait, resumed pre-poll abort and failed reconstruction/custody all asserted.
Controls baseline4PASS/two caught at intended original/cause assertions,0missed/
timeout/unviable. cp5-custody.tar.gz SHA
9e94a8b4a954d1d288ee52c0d96fd5ed524282c1c5f0c10b8e0e8598587b55d8.
Resource addition150nettest lines: UTF8/JSON-escape4095/4096/4097; valid request
oversizes full acceptance -> typed definite failure/zero rows/reusable ID; exact
receipt/state/raw record/handler counts; queue/deadline extrema; timer-disabled
native panic/Uncertain/zero rows observed in bounded child. Child selection now
asserts exactly one passing test. Targeted6PASS; full debug40PASS1ignored1.528s;
release40PASS1ignored0.476s; strict all-target/all-feature lint PASS0.60s after
checked test Duration arithmetic, no waiver. Every invocation AND child receives
three reviews. Source guard borrow compile failure retained, not hidden.
cp5-resources.tar.gz SHA
9ef5dfac00207c9bd7e4a50d550baa5fc7839bcd3fc3778fe0c358773206dfb6.
No CP3+CP4 integration/full source gate/PR yet. Required tracing/metrics/export,
all feature combinations and measured overhead still OPEN. Integrate parent
before final source qualification. Native stores/performance/soak CP6, docs/
packaging CP7, audits/protected release CP8, registry/server CP9 remain required.

## Resources and tools

All logs/roots /tmp/mnesis-command-start-20261021. Nix shell91477 cwd CP5, idle
(after cargo clean2GiB), tests stdin /dev/null. No local compilation active.
Caches cleaned only AFTER retained raw/source evidence. Deletion
94264 finished3old CP4 cross outputs1.4GiB;49930 finished13old goal Clippy/cross
outputs5.2GiB after0referrers/new counterparts PASS/rooted. Exact paths/logs kept;
normal liveness, no global/user/other-project cleanup. Older outlinks retired so
do not assume all historical roots exist. Source/raw evidence is preserved.
Wait actual handles; never cancel/restart solely on observation timeout.
