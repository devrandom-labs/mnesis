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
## Current native query isolation work — recovery pointer

Decoderworktree /tmp/mnesis-decoder normal SIGNED fullNixcommit session65997
still running (log /tmp/mnesis-decoder-array-hook.log); do not mutate itsindex.
After terminal fix eight evidencecopies' blank EOF whitespace then normal signed
evidenceamend; sourceequivalence/no bypass. TLS PR392 open allchecks except
CodSpeed; no merge/waiver. Durabilityworktree preserves full240native pass+controls.
New /tmp/mnesis-postgres-queries branchfix/postgres-queries base49104117,
production UNCHANGED. Native namesoriginal actual1FAIL0.088s at first-body vs
second-body; healthy+operatorcontrols pass, schema dropped before oracle. Three
reviews recorded there; PG4 reopened. Follow-up3laws original run1 COMPILEFAIL
(payloadarray notIntoBytes),0tests/no nativecredit;3reviews recorded, corrected
input supportedstr only. Follow-up2 session running, logfile
/tmp/mnesis-pg8-names-followup2-original.log. No concurrent localnative suites.
Read queryworktree records for next step; headforgery and snapshotrouting proof
BEFORE productionfixbudget<=35existinglines. Entire PG0-PG9 remains active,
release/publication/registryconsumer unverified; root USERresearch edit untouched.

LATEST queryworktree native5original laws all FAIL0.258s (stream disclosure,
forgedMAX persists1000, snapshotread/write misroute, checkpoint pairdisclosure).
Budgeted minimal13line SQLrepair8+/5- passes same5in0.281s and full235in57.439s
0skips, all3reviewed. StrictClippy4.70sPASS. Feature-scopednative names matrix
nowrunning default/snapshot/projection/import/export sequential, logs
/tmp/mnesis-pg8-names-{default,snapshot,projection,import,export}.log.
Initial Linux47383 started BEFORE newteststaged: excludes5laws, DO NOTcredit
fullnewsource; staged current Linux74113 uses kd12f4...tests/9qshfm...integration.
Decoderhook65997 stillrunning its unaltered fullgate, preserveindex/source.
LiveReleasePR378 head9d5ccbb3 actual0.5.0 allninecrates, APIbreakingPgAllPosarity
reported: prior0.4.1patchconcern disproved, nofixneeded. Stillunmerged/unpublished.

Decoder signed evidenceamend84281DONE0 final9f81cbddd15b86cdb797b247ee9303d252595591 G;
all12samepreviouslybuiltNixderivations0freshtests,3reviews: unchangedcontracts,
sourceequivalence61368a25verifiedforcrates/adapters/manifests/locks/Nix/workflows,
cachedoutputsnotfreshproof. Fullbase whitespacePASS, cleanstatus, normalpush
75485DONE0. FocusedPRcreatecommandrunning; /tmp/mnesis-decoder-pr-create.log.
Actualx86CI/CodSpeed/fuzz/mutations review+protectedmerge mandatory beforeTLSrebase.

PR393 OPEN https://github.com/devrandom-labs/mnesis/pull/393 exactsignedhead
9f81cbddd15b86cdb797b247ee9303d252595591. Protectedbase49104117; fullgatefresh
1571pass568.764s13slow/4compilerexcluded; evidenceamendcachedsame12outputs,
noextraexecutions. Sourceequivalence61368a25 andfullbasewhitespacepassed.
Noactualx86performance/CIresultyet; noautorelease/mergewaiver. Querybranch
five original native reds now passfull235in57.439s andfeature-names2/4/3/2/2
zero skips, all3reviews. CorrectstagednativeLinux74113 stillcompiling; initial
unstagedrun47383ONLY230pass59.022s0skips(no5newlawcredit),5smokes reviewed.

Final querynative235PASS48.365s0skips includes unchangedcallersearch_pathassertion
via sharedtestcleanup, strictClippy2.99sPASS, all3reviewed. Finaltest314/360lines.
Linux78554 drv9fmwd4...tests/3ppfg9...integration srcjfiyyl... names_tests cmp
FAILED atline70: earlier stage-vs-rustfmt timing DID capture different formatting.
Inspectdiff and record limits; final formatted-source Linux still required.
PG4/5/6 reopened pendingdelivery.
Durability new credible deferredconstrainttrigger hypothesis recorded beforeedit:
existingnative immediate-triggerlaw now includes seconddeferredschedule with
settlednativecountsassert BEFORE error. Productionguard UNCHANGED vs saved
/tmp/mnesis-durability-predeferred.rs. Firstcommandexit2unquotedfilter shellsyntax,
ZEROtests3reviewed; secondcorrectquotedactualtestrunning
/tmp/mnesis-pg8-durability-deferred2-original.log. Querylocalnative19912DONE0,
no overlappinglocalPGsuites. Next fix ONLY after actualred+3reviews.

QUERY focused delivery next: /tmp/mnesis-postgres-queries branchfix/postgres-queries,
production13churn(8+/5-) only3SQLowners; tests314/360lines, original5reds preserved.
Finalpathpreserving native235PASS48.365s0skips, strictClippy2.99sPASS. Intermediate
Linux23554.267s, pathhelperLinux23538.043s0skips all3reviewed, latterhelper has
ONLY4formatlinebreakdiff fromcurrentworkingfile. BytewisecmpFAILED honestly.
Final formatted-source Linux35265 nowrunning /tmp/mnesis-pg8-names-formatted-linux.log.
Normal SIGNED fullNixcommitnext; source/indexfrozenwhilehookrunning. Need actual
finalLinux, allgate/replayreviews, focusedPR/allCI/mutations/threadreview beforemerge.
Durabilitydeferredtrigger production UNCHANGED, originalattempt3running54058,
priorfirstshellsyntaxexit2/secondSqlSafeStrcompile101 bothZEROtests and3reviewed.

CURRENTquery parentf207923a, signedrebasedb8531258. Combined native235PASS53.423s
0skips; all3reviews recorded. Preserved finalformattedpredecoderLinux23592.338s
and normalpredecoderfullgate1573PASS563.191s14slow/4compilerexcluded/replay2/1/1.
Added actual PR393finalCI+protectedmerge evidence fromdecoderownedappend; no
sourcechange. Normal SIGNED amendment now runs complete combinedNix gate, freeze
index/source untilterminal. Then clean/whitespace/sourceequiv, normalpush,
focusedPR/allCI/currentLinux/thread/mutationreview/protectedmerge.
TLSrebasedsigned3fa39a4d0ff8d4b9d6eefac5993d8a6b74b03df0 onf207, sourceequals
03052633 except EXACT protectedwire.rs; saved CI stashae4650 retained+suffix
appliedmanually afterprefixcheck. TLSnative next onlyafterquerynativeDONE.

LATEST query signed serial retry59677DONE0 head95a00bc15d72cd04ee79ab25600ec321e03b019e G,
all12Nixchecks; fresh1573PASS523.520s14slow4compilerexcluded/replay2/1/1each1s.
THREEreviews appended locally, executable sourceeqb853PASS. Push focused queryPR
next; actual combined Linux/CI/CodSpeed/artifact reviews before protected merge.
TLS serial signed retry34027 now running alone, /tmp/mnesis-pg8-tls-serial-hook.log.
No simultaneous full local gates. Disk19GiB after query completes. No publication.

CURRENT PR394 oldsigned95a00bc1 CI stilltwoNixjobsrunning; mutationartifact12161164145
16raw14caught2unviable actuallyEIGHT42P06fixturefailures, REJECTED. Newowned
4testlinecleanup in all_noskip_tests capturesappend/join, DROPbeforeunwrap. Native
originalcontrol FK23503then42P06 red; candidate23503twice+namespace0 aftereach.
Healthyfull235PASS78.426s0skip, strictClippy6.49sPASS, actual3reviewsrecords. No
productchange beyondoriginal13SQLlines, no relaxation. Stageownedevidence then
normalSIGNEDamend/fullgate SERIAL (TLSgatefinished), sourcefrozen. Needpushnewhead
with exactlease95a00bc1 onlyaftergate0/GPG/whitespace, actualfreshmutationCI proof.
TLSnewSIGNED276bbdd2 normalleasedpushDONE0, PR392 newrequiredCIinprogress.
Itsserialfullgateall12fresh1575PASS536.315s15slow4compilerexcluded/replay2/1/1.
Do NOTmergeoldquerysourceon misleading rawcatches or completewholegoal. Afternew
querysignedhead available, stackTLS ontoquery to testcombinedsource once; retain
all docs/suffixes/stashes. No otherown localnative/fullgate jobs nowactive.
