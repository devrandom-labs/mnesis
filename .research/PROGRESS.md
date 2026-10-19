# Decoder performance experiment

Whole Postgres goal ACTIVE, no token budget. This is one release/performance
subtask, not adapter completion. Read root .research/PROGRESS.md for other work;
canonical devrandom-research SKILL.md plus full protocol on context recovery.
Preserve root user docs/prds/postgres/research.md edit, all worktrees/stashes.
No subagents. Every actual run gets THREE reviews: contract, execution/fault,
assertions/source/skips/limits. No framework/runtime/custom tooling/Python.
Normal signed commit unchanged full nix flake check hook, protected PR/CI merge.

Current /tmp/mnesis-decoder branch fix/decoder, based protectedmain
49104117b87010206fb4b25be1b4d3d6e6635b22. UNCOMMITTED candidate source ONLY
crates/store/src/wire.rs26+/34-=60churn within100. Existing FrameHeader reader
uses core::slice::first_chunk and exact array pattern for fixed fields after
original minimum/version guard. Keeps exact V1minimum19/V2minimum11/error
priority/endian/metadata semantics. No unsafe/extra indexed arithmetic/generic
helper/public API/dependency/hint/benchmark/gate change. Two unusedV1offsets
removed; V2offsets cfg(test) after verifying all consumers are wire unit fixtures.
Uses standard core method stable1.77, actual pinned1.99 compilation confirms.

Original performance failure applies BOTH release PR378(fe0fbee) and TLS
PR392(03052633): all3decoder sizes501.7->560ns,10.42%, no fix/waiver/reset/ack.
PR392 all CI terminal39SUCCESS/1CodSpeedFAIL/1scheduledskip, zero reviewthreads.
Native235fouractual CI runs all pass; detailed THREEreviews in its own modified
UNCOMMITTED research record, source/signedhead unchanged. DO NOT merge PR392
while performance fails. Need this independent fix first, then rebase/rerun TLS.

## Native evidence

Same original owning Fjall benchmark,64/1024/65536byte inputs,30samples each,
1swarmup/3smeasurement, Rust1.99/LLVM23.1.1/M4Pro. No concurrent own native tests
during measurement. Existing /tmp/mnesis-release-performance-target reused,
Criterion named baselines avoid old rejected-inline saved results.
Parent491 baseline first CIs2.5093–2.5722/2.5021–2.5101/2.5008–2.5076ns;
second2.5089–2.5157/2.5423–2.6182/2.6419–2.7294ns (material drift retained).
Saved original /tmp/mnesis-decoder-parent-benchmark6704128bytes. Actual --bench
A/B/A returns2.5003–2.5498/2.4992–2.5748/2.5580–2.6428ns. Initial direct invocation
without --bench did smoke only, explicitly NO timing credit; -return-smoke.log.

REJECTED and source reverted before current candidate:
1 header-size choice moved into body (+6/-10), actualnative2.705..2.727ns, real
small/medium regression vs A/B/A. Saved -body-candidate.rs and -candidate-benchmark.
2 shared #[cold] offset error factory/lazyok_or (+16/-33), native2.618–2.711/
2.606–2.692/2.598–2.666ns, no speed improvement/slower. Saved -cold-candidate.rs,
-cold-benchmark. Actual bounded LLVM disassembly190lines shows branch changes,
same filebytes NOT equalcode. Symbol-only attempts had missing-symbol warnings/
2-line headers and get NO instruction proof. Address dumps are real. Both
experiments append-only recorded in EXPERIMENTS/deadends, no source retained.
Older preserved /tmp/mnesis-release-performance records reject outer #[inline]
25–36%slower/+512bytes; do not repeat to chasegreen.

CURRENT safe fixed-array candidate actual native benchmark all3PASS, CIs
1.5489–1.5590/1.5691–1.6777/1.5553–1.6242ns,35.68–40.30%faster vs actual parent
return baseline. Exceeds measured drift, no x86/CI/release claim yet. Whole binary
6704128unchanged; decoder owner span736->488bytes. Actual LLVM new owner dump
/tmp/mnesis-decoder-array-addresses.log128lines, zero panic_bounds_check calls
vs8 in original190line owner. Earlier hypothesis guessed6; actual complete
bounded dump is8. No proof for uninspected architectures. No fake code equivalence.
FormatPASS, strict storeall/all Clippy9.57s,40selectedwirelawsPASS0.256s with
177FILTERED, not fullproof. Current full store651/651PASS17.808s0skips includes
compiler fixture owner, no databasecredit. THREEreviews recorded in research.md; complete selected set executes, zero skips, no PostgreSQL credit.

LIVE75506 owning checks in current Nixprofile:
fullstore DONE651PASS, then fullFjall running, then storedoctests. Logs
/tmp/mnesis-decoder-array-{store-native,fjall-native,doc}.log,
-array-owning-checks.log. Poll SAME handle. No other own localnative DB/benchjob.
Next inspect final owning outputs3ways, repeat native candidate once if needed,
unchanged full normal signedgate + independent Linux native, no_std/platform/fuzz,
focusedPR/allCI/actualmutation/fuzz evidence/normalprotectedmerge. Need actual
x86 CodSpeed and exact later releasehead pass, not native ARM waiver.
No source mutation while gate/tests running without justified failure.

## Related goal work

Root /Users/joel/orca/workspaces/nexus/main docs/postgres-prd8357c0c2 preserved.
15protectedPRs merged; latestPR39149104117. TLSworktree
/tmp/mnesis-postgres-security PR392 signed03052633cfbf15553b8392034f4799b5e6836235
remainsOPEN, performancefailure only. Liveaudit/deny TLS versions pass current
policy but pre-existing event-listener5.4.1 unsoundwarning/ignore and Nixyankindex
blindspot remain PG9. Do not publish/claimcomplete. No new retained corefix yet.

Durability /tmp/mnesis-postgres-durability work/postgres-durability uncommitted
based49104117: private32line nativeGUC validation beforestartup/4COMMIT owners,
exacterrorvariant, no interface/dependency changes. Native10laws and full240/240
PASS105.061s0skips. Originaloff-startup and4replacement write witnesses red;
three clean runtime controls fail, including matching safe-looking error AFTER
commit caught by independentsettled2/2vs1/1rows. Hostile text-equality bypass found
in candidate then3native operators qualified. Native server fsyncoff/fullpageoff
reject; immediate server stop/status3/restart preserves events/history/state and
permits append/resume, not hardwarepowerloss/failover. Test629/650 +exact shared
66line cluster owner from TLS as explicitdependency. Final rebase onto protected
TLS +combined source checks/delivery still due. No other own PG17suite running.

PG7 logicalRPO/RTOprofilemeasurements, PG8 lifecycle/diagnostics/capacity10k/100k/
1m x1/16/128 and30minute soak, PG9 feature/Python removal/AFLgate/advisories/
minimization/twofresh audits/releasepublication/cleanregistryconsumer all remain.
Do not stop at this performancefix or passing localbuild. Labcast owns IDs/fencing.

Environment: Nixprofile /tmp/mnesis-postgres-security-shell pinnedRust1.99/
OpenSSL3.5.8/PG18.6, shareddebugtarget root/target. Shared native17.10 cluster
/tmp/mnesis-postgres-pg0 Unix/tmpport55432userjoel rooted exactpackage
/tmp/mnesis-postgres-pg0-package. MainDBmnesis_pg7b/controlDBmnesis_pg7c_controls
SAMEphysicalcluster, never concurrent suites evenacrossDBs. Private18.6 separate.
RemoteLinux ssh-ng://builder@linux-builder. No broadGC/rawrm/unrelatedjobkill.
Preserve all stashes3b4e83fa,5a7ca8a2,fd00ca76,b7a00486,85459c87 and olderrootlist.

LATEST owning75506 DONE0: full store651PASS17.808s, fullFjall232PASS550.736s
12slow/0skips, all12doctests IGNORED0executed (no doctestbehaviorcredit). Allruns
3reviewed and retained. Linux72349 DONE0:230/230PASS43.840s0skips plus5filtered
smokes, current core source+protectedparent only, noTLS/durabilitycountadded.
Stage ownedcandidate/records then normal SIGNED full hook, focusedPR/allCI.
No localDB/benchmark/test job now active. Current source frozen for commit.
Credible separate audit concern: read_key/read_head and byte predicates still
use unqualified operators/MAX with caller search_path; global-order fix covered
only prior functions/operators. Needs native isolation/head/byte-limit witnesses
BEFORE any query fix. Can do independent worktree while current gate runs; shared
PG17cluster currentlyidle. Do not claim source finding as reproduced defect.

Decoder fullgate65997DONE0 normal SIGNED61368a255ee05fed3763674c569dea109042b219 G.
All12Nixchecks, fresh1571PASS568.764s13slow/4compilerexcluded; shortreplay2/1/1.
All3reviews recorded incl DBguard earlyreturn and incomplete audit/yankindex.
Afterterminal corrected8ownedevidencecopies EOFblanklines; raworiginals unchanged.
Normal signed evidenceamendnext, sourceequiv61368a25 mandatory then focusedPR
actualx86CodSpeed/fuzz/mutationsallCIreviews/protectedmerge; releaseNOTfixedyet.
