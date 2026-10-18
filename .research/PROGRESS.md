# Current work — 2026-10-18

Persistent goal ACTIVE, no token budget. All PG0–PG9 required through protected
merge, established release publication and a clean registry real-Postgres consumer.
PG10 conditional. Root docs/postgres-prd preserved; never recreate deleted branches.

Current worktree /private/tmp/mnesis-postgres-views, work/postgres-views based on
merged PR384 88a4575b3a3d0c5db8aaac2b92241c76b19f9a60. PG7b changes
UNCOMMITTED. Skill/full protocol reread after recovery. User requires THREE test
case reviews after EVERY run: contract, defect/falsifier, assertions/skips/oracle.
Keep genuine reds, never weaken tests/bounds/lints. No extra agent authorized.

Merged: PR376 reads/recovery,377 outcomes/races,379 migrations,380 binary/unsigned,
381 atomic/import,382 snapshots090504ae,383 checkpoints c482fb6a3762c74d7b10416029de22a642de2b71.
PG6 feature complete. Cross-capability acceptance and PG8/PG9 still required.
PR383 head7599588e reviewed/protected squash2026-10-18T15:55:19Z. All40green1scheduled
skip; actual mutation artifact12142160368 SHAffcd8a971a0cb86ce9422e3261e7cbe63635d28fd30dfae76a48c5082b21ac67
16=8caught8unviable0survivors; per-case reviews preserved. Deleted remote verified.

PR384 bounded listing MERGED2026-10-18T16:52:53Z,88a4575b, all40green1scheduled
skip/CLEAN/exact signed f1380837 before normal protected squash. Remote absent.
CI39145116648, mutations39145116667, pushCI39145081397. Actual mutation artifact
12143007654 SHA39ab9563a2313d030929765dad2e0f83230f6cd3a97fee988b2600785ecdf154
downloaded /tmp/mnesis-pg7a-ci-mutants-39145116667; all6firstfailures reviewed3ways,
1caught5unviable0missed/timeout. Native CI4distinct192/28.621,24.836,28.320,28.447s.
Native source already verified192(all),126(export),118(default), Linux final192/
21.137s plus4smokes; preserved original xmin/search_path failures and3fresh SQL
controls. No branch push after merge. PG7b safely empty-range rebased onto88a4575b,
named stash f4fc89d0b83d5411f13eebca269b0cf76c1ef0a5 retained/applied--index.

PG7b initial five native laws: shared view captured BEFORE first read including
concurrent atomic three-stream commit; buffered/unpolled idle expiry; canceled
opening AFTER actual native view; invalid lifetimes before closed pool; explicit
close ACK/drop/reopen native xmin cleanup. API absence8E0599 after fixture fixes,
zero runtime. Initial candidate uses existing bounded Tokio task/channel, one
pool lease, native read-only RR transaction + real snapshot before ready, typed
sources. Close waits rollback ACK; expiry/drop actively interrupt owner.
Budget450added/80removed production,1session type/3errors,600initialnative lines,
zero deps/runtime/framework/tool. First lint6/5arguments fixed private immutable
Window bounds; second lint83/80testlines fixed repeated PlannedAppend fixtures
using array.map WITHOUT removing checks. Current strict Clippy then FIVE native
session laws launched in session46407, log/tmp/mnesis-pg7b-first-native.log.
Initial5native PASS1.210s after fixing fixture listener stole sole idle lease
(native readiness query then priming, same3s timeout). Added2adversaries ACTUALLY
FAIL: blocked SQL retainedxmin>5s despite1s expiry; poolclose idle60s waits>3s.
Fixed LOCAL client_connection_check_interval1s preserving shorter caller setting,
plus supported SQLxclose_event in owner. Both sameoriginalredsPASS; additionally
actual60s blockedqueryDrop release, poolmax2=>exact2nativeconns+originalPoolTimedOut,
nativebackend57P01 and fusederror, inclusive version/37byteedge/74+72limit errors.
11native laws572lines,318modulelines; production371added24removed,1publicsession/
3typederrors,zero deps/framework/runtime/tool, existingNixgate+5lines smoke.
All203/24.551s prior equivalentSQLbinding; default118/15.435s current; final
export-only136/19.915s warning-free; strict Clippyall/default/exporteachpasses.
CURRENT Linux203/27.703s plus5separate actualsmokes1each.313/.256/.204/.271/.106s,
artifact/nix/store/0vv8gv9db3m8ikrml0pqrzmlh2s7335f-mnesis-postgres-integration.
SourceRustdiffSHAd5e7e35be8b81624f892832f7979b9075405987ffd829b919e145c68841941c3.
Freshfinal-source controls: /tmp/mnesis-pg7b-final-isolation.qxqkA0 RR=>READCOMMITTED
actual2vs1 .057sFAIL; /tmp/mnesis-pg7b-disconnect-disabled.LqroYB 1000=>0 native
blockedxmin5.062sFAIL. Each proofadapterclean/recompileexplicitpath/current317+
parameterized source, originalboundsunchanged,3reviews. Earliercontrolretained.
Missingexport sessionselector exits4/0tests, notruntime. Eachrun3reviewsrecorded.
First22mutants baseline REJECTED: wronguserjoel28000,11unitpass1fail191unrun,
0mutants. Actual18.6mutationclusteruser=mnesis; corrected sweep session55094
/tmp/mnesis-pg7b-corrected-mutants.log,output/tmp/mnesis-pg7b-corrected-mutants.
Cluster /tmp/mnesis-pg7a-mutants.Pqoy5x currentlyRUNNING (dedicatedseparate18.6).

NEXT inspect all22corrected mutant outcomes/firstfailures3ways, treat survivorsas
gaps/notnewexclusions. Need cancellation of close while blocked rollback and
caller-stricter connection interval (sourcepreserved; no runtimewitnessyet).
Finish minimization/fullnormal signed gate/focusedPRsession/allCI/download actual
mutationartifact/protectedmerge. No PG7bcommit/PRyet. Currentdocs/evidenceedits
partlystaged; all source already staged. No currentnative17.10run. Linuxdone.
Keep normal commit hook nixflakecheck unmodified. Then continue consistent backup
format2/Fjall/Postgres roundtrip/corrupt/incomplete/salvage and restore (nextslice),
PG8/PG9 entiregoal; do NOT stopafter sessionPR. New tests beyond initial600 budget
require pre-edit explicitbudgetchange, not arbitrarylineallow. No source changes
while mutationcopy/gatefinalproofpending unless actualfailurejustifies; recordhash.

PG8 pending TLSVerifyFull/auth/CA/hostname/redaction/immutable grants; caller pool,
query/acquire/lock bounds, startup/reconnect/shutdown; durability future sessions;
primaryrestart/physical+logicalrestore/foreignwatermark;10k/100k/1m x1/16/128
consumers,p50/95/99/resources/plans and30minute mixed soak. Foreign history native
witness BEFORE origin design. Cold fixture2–27s cause UNPROVEN, no loosened bounds.
PG9 remove existing scripts/check-features.py AND fuzz.yml embedded Python through
supported Cargo/Nix/JQ tools (no replacement custom tooling); native AFL failure
proof before gate claims; advisories/audit registry issues; complete feature/
downstream/absence gates, docs/example/changelog, two fresh full audits,
ALL protected merges then release PR378/publication/VCS/checksum/consumer evidence.
Do not merge release378 before all acceptance. Labcast IDs/dedup/fencing stay there.

ENV local17.10 /tmp/mnesis-postgres-pg0,/tmp port55432 userjoel. PG7b database
postgres:///mnesis_pg7b?host=/tmp&port=55432&user=joel. Never run independent native
suites concurrently in SAME physical cluster (xmin spans databases). Nix profiles
/tmp/mnesis-postgres-shell-199 and shell-ci reuse via nix develop PROFILE -c.
Shared target /Users/joel/orca/workspaces/nexus/main/target. Dedicated control target
/tmp/mnesis-pg6a-proof-target.z1jmzk force cargo clean -p mnesis-postgres before
EACH source switch + explicit manifest/compilepath. ARM linux-builder works.
No broad GC/raw cache deletion; prior cleanup autoreview rejected. Disk~12GiB.
Evidence JSON use .log suffix to avoid global Nix source filter churn.
Preserve stash OIDs b51e18ce53f0a3e9326676e014d3bdab26778392,
fbcc7d919c5df03ee3216065de84a63ddb825992,
65981423d8263e5a00d6df11e62668b3ed4cd2b6. Never pop by old index.

LATEST: corrected22mutants DONE9caught13unviable0survivor/timeout; actual baseline
203/26.294s18.6, all22firstfailures/three reviews in research/outcomes/firstfailure
.log. No new exclusions. Dedicated mutationcluster stoppedFAST/data kept. Full
nixflakecheck -L currentlysession58305, /tmp/mnesis-pg7b-full-gate.log; Rustsource
unchangedSHAd5e7. Replays2/1/1each1s pass3reviewsaftereach, fullgate stillrunning.
NEXT inspect remaining workspace1563/defaultDBfree vs actualnative evidence,
record3reviews, then signednormalhookcommit (no bypass), focusedPRsessions,
allCI/download actualmutationartifact/percase3reviews/protectedmerge. NoPG7bPRyet.
Sourcecontrols/featureClippy/Linux allfinal; no need gratuitousrepeat unlesschange.
Evidence/docsgateeditsunstaged; stagebeforecommit. Rootprogressmirrorednow.
LATEST2: fullgate session58305 completedexit0,12namedchecks10fresh+audit/tomlcached,
workspace1563/519.852s13slow4compilerfunctionsexcluded(existingCIowner).3reviews
recorded; summaryevidence added, raw/tmp/mnesis-pg7b-full-gate.log. No test/source/
lint bounds relaxed. NEXTstageallownedchanges/signednormalhookcommit (cached
checks0newruntimecredit), pushfocusedPRsession/fullCI/artifactreview/protectedmerge.
ThenPG7c note/tmp/mnesis-pg7c-pre-edit.md records actual decoder materialization
contract limit; native boundedbackup/freshclusterrestore witnesses BEFORE API
choices. No PG7bPRyet. ProductsourceunchangedSHAd5e7,572nativelines. Rootmirrored.
LATEST3: signed commitcd3ec0355299b574c5fefe3dfa50830b20bcf987(G), normalfullhook
12cached/0fresh-runtimecredit. Docs-only signedamendment now records actualcommit
and samehook withoutsourcechanges. Push/openfocusedPR next; do notstopgoal.

LATEST4 USERSTEERING: fix failingreleaseASGO, preservewholePG0–9goal. PG7b
signedamendedheadf94333c7649a1a69e875728bc4a5553355f11fa7 pushed, PR385OPEN
https://github.com/devrandom-labs/mnesis/pull/385,38green2Nixpending0fail atlastpoll.
ParentworktreeviewsCLEAN, doNOTedit/pushwhileCIexacthead. NeedallCI/download actual
mutationartifact/firstfailures3reviews/normalprotectedmerge (notyetdelivered).
CURRENTworktree /private/tmp/mnesis-release-checks work/release-checks based88main,
45line releaseworkflow fixUNCOMMITTED. Full Nixgate session79394running, log
/tmp/mnesis-release-fix-full-gate.log (5fresh otherscached).45<=80pre-editbudget,
0Rust/deps/scripts/runtime/framework. Main-only release job; separate PRcheckout;
selectivecargo update2paths per2fuzz locks; GitHubApp signed conditionalCAScommit;
noPRguard prs_created true (upstreamnonepr={}!). Actionlintpasses. Exact YAMLbody
controls: realSTaleData nativeAPI rejects stalehead/no branchwrite, currentmain
0lockupdates/noAPI/payload, originalreleaseCargo--locked101=>corrected metadata0.
All3reviews/detailsdocs/prds/postgres/research.md. Never remove --locked/maskjobs.
ReleasePR378 head8836a37c4ffc046adde6a2f425da801ba01ab28d OPENunmerged;4AFLbuilds
failed39147192934 stale fuzz-afl.lock beforefuzz. Coremnesis/storeversion0.4.1
not reflectedisolatedlocks. Manualproofworktree /private/tmp/mnesis-release-locks
work/release-locks,4versionedits2locksUNCOMMITTED; probe/detachedworktree
/private/tmp/mnesis-release-workflow-probe at8836 hasSAMEupdates, preserved.
Planmergefocusedworkflowrepair throughnormalgate/CI/protectedpath, then dispatch
ReleaseonMAIN torefreshPR378locksautomatically (rootpublisherstillmain0.4.0).
DO NOTMERGEPR378or publishpendingadapter beforewholeacceptance. No liveworkflow
executionyet. Userauthorizationfixreleaseexplicit, normalprotectedworkflowremains.
CodSpeed117734118722 releasePR3decodecases501.7ns=>560ns-10.42%; actualpublicSSR
showsAMD EPYC7763vsbaseline9V74 equalflags, sameRust1.99.0/LLVM23.1.1/cargo-codspeed
5.0.3/Ubuntuimage. CPUdifference NOTproofcause. No CodSpeedtool/registeredrepo
selfhostedrunner. Don'traise thresholds/acknowledge/removebench/switchmetrics to
forcegreen. Needactualtrace orcontrolledsame-environment proof. Publicdata/tmp/
mnesis-release-codspeed-data.log,detail.log,page.html,selectedenvironments.log;
rootCIlogs/tmp/mnesis-release-378-codspeed.log and-main-88a4575-codspeed.log. Official
variance docs/tmp/mnesis-codspeed-regression-causes.md and-reducing-variance.md.
Fullgoalincomplete: afterPG7bdelivery/repair resumePG7c (preeditnotes/tmp/mnesis-pg7c-pre-edit.md),
PG8security/lifecycle/restore/capacity30minsoak,PG9remainingPython/advisories/etc,
publication+cleanregistryconsumer. Rootdocsbranchpreserved. Allworktrees/stasheskeep.

LATEST5: PG7bPR385 ALREADYMERGED byjoeldsouzax17:41:10Z actualevent,merge
12fd839c7b08e2eda1a8020bae9f52dd898c1d15,branchdeleted17:41:12Z. Agent didNOTissue
merge (discoveredstateMERGED whilemergeableUNKNOWN).40green1scheduledskip, all
native4x203,workspace2x1563,compiler2/1/1x2,fuzz8,22mutation firstfailures reviewed
3ways+evidence in currentrepairbranch. Actualartifact12144836614 SHA74c563874ac99...
9caught13unviable0missed/timeout. NEXT fullgatefixbranch afterrebase to12fdmain,
normalhook/sign/focusedPR/protectedmerge thenReleaseMAINdispatch. No livefixyet.
ReleasePR378now9aab7b894f21637ffdf7aad924183a6a58b54b20,oldAFLfailure repeats.
Repairstash preserved OID in/tmp/mnesis-release-repair-stash-oid; all5append
conflicts resolvedpreserving mainrecords+onlynewrepair/CIrecords, no production
sourceedit. Prior gate79394 on88source stillrunning; no current12fdgate credit.
Preserved repair stash219c6aaf1dfc8ea83f6ff6d3a0389b8a8ba0449f. Actualartifact
12144836614 digest74c563874ac9918daa08b868a7e56cb6b91afcb26102ae0ea480d22d1ac12648.
LATEST6: releasefix signed111d0b8a normalhookPASS12checks9cached3freshcompileONLY,
0fresh runtime. Pregate88alsoPASS1563/508.004s, not rebasedsourceproof. Records
3reviews inresearch. Amenddocs/backlog now thenpushfocusedPR. CodSpeedACTUALtrace
retrieved /tmp/mnesis-codspeed-{base,head}-callgraph.log andevidence: largepayload
bothir35ns, all58.333nsdelta fromONE extra L1+LLmisspair in decode_frame self,
10nodes9edgesboth. ActualcostchangeNOTinstructions; causehardwarevslayoutunseparated,
noack/threshold/workloadchange. Nextall3traces andmeasured causalminimalimprovement.
