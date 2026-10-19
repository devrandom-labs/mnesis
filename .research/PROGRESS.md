# Current Postgres delivery — 2026-10-19

Persistent goal ACTIVE/no budget: all required PG0–PG9, protected merges,
publication and clean registry real-DB consumer. PG10 conditional. Labcast owns
command IDs/dedup/fencing. No delegation. User narrowed security to explicit PRD;
finish workload/verification/delivery. Preserve every worktree/stash/raw record.
Root docs/postgres-prd8357c0c2 USER research.md change remains untouched.

24 focused changes protected-merged through diagnostics402:
5b8680280854c397c957140784ba7795bcb488a0. All40SUCCESS/one scheduledSKIP/0threads;
merge tree exact standard e1fd9b70203e80892480cfa38ee552fb27024a38, preserves
user release4010525d517 and4009f7116c7.0.5.1 publication independently verified
run39312731245; sparse registry not yanked, checksum
 aa483fa702ad0dd67a0ba941704bc7b196cfbc5925ac8088cc6bda40ea61e7cb.
Release403 is OPEN v0.5.2. Published subsets are not whole-goal completion.

CURRENT DELIVERY SLICE /tmp/mnesis-postgres-soak test/postgres-soak, base5b86.
Full1800s soak71259 TERMINAL0:1812.797935834s final count/1814.44915625s through
close,591768rounds/2662995events/31recovery faults/0known rejections. Peak1879613,
baseline1071253/retained103974bytes,1294346952265allocated bytes (traffic, notRSS).
Observed periodic backend high46/configured max52, final pool0/backend0 checks.
Every ordering/ordinal/state/position/revision/native-count assertion reached.
THREEreviews and retained failures recorded. Source61d3f493... remains exact.
475/500benchlines,14/15functions; deadline339/380lines. No production/API/dependency
or migration change. Native phase3PASS5.194s/36unselected verifies55P03 known
beforeCOMMIT but Unknown after; every original code/assertion preserved. Known
pre-COMMIT55P03 only separately counted, never Conflict/automatic retry; other
errors fatal. Full suite264PASS73.287s/0skip/23binaries, compile21.25s excluded,
THREEreviewed. Both prior full reds retained (finite-scan assumption and lock
rejection at1269s); rescan/ordinal native controls retained. No more soak rerun
unless a material source change/concern justifies it.

PG7 observer-only backup_tests+9lines adds logical timing, no assertion changes.
Current strictall/all PASS17.03s. Three native two-case operator runs PASS
6.213/6.504/23.422s, each18unselected and THREEreviewed. Small TWO-event/TWO-stream
fixture, state4→5; not four events. Logical backup/readiness ms
2.680417/48.200084;2.092292/45.876;2.133125/41.92975. Physical verified backup/ready
376.025125/141.374167;403.584208/140.4845;413.460084/140.917959. RPO0 applies only
to quiesced fixture. Defined readiness omits other setup/source close and final
cleanup/external provisioning/routing. Full third logical case21.768s is recorded,
not hidden or diagnosed without evidence. docs/prds/postgres/recovery.md records
all scopes/limits. Raw/tmp/mnesis-pg7-recovery-repeat-{1,2,3}.log. Native suite
264 above predates observer9lines; changed owner's six runs and finalLinux cover
new observation. README quickstart0.5/publicindex2lines carried here identically
from PG9 WIP to avoid duplicate docs changes. Current README20added/4removed
budget; final whitespace/minimization next then ordinary SIGNED commit/full Nix
gate (max-jobs1), push/open focusedPR, actual Linux/artifacts/threads review and
exact-head protected squash. Update /tmp/mnesis-pg8-soak-pr.md first. No gate
bypass/admin/lint/threshold/source-filter change. No local native while gate runs.

NEXT PG9 /tmp/mnesis-feature-checks fix/feature-checks base5b86, WIP combined
verification change (original /tmp/mnesis-fuzz-verdict preserved, don't duplicatePR).
Delete44linePython wrapper/inlineAFLPython; Nix cargo-hack0.6.45/jq1.8.2, supported
Hakari disable/dev-free offline checks (fetch locked first) + exact restoration.
47actual compiler invocations PASS: kernel6/macros1/store17/store-testing4/wake1/
wake-nostd1/inmemory4/Fjall6/PG7. ZERO unit/native runtime credit. Both original
false-green hypotheses rejected: Tokio probe valid due explicit rootfull;
JsonCodec rejected by old+new compiler. Wrong initial cwd and wrapper101 vs1
expectation retained/no invented catch. New native archives default/5features
from manifest+all, required owned law/no-tests=fail before full suite. Existing
all7smokes/psql/DATABASE_URL/pipefail unchanged. Nixfmt58+/19- within65/20budget.
Combined actionlint/Nixfmt/devShell PASS (static/tool only). Public docs identical
to parent changes above; preserve through source-identical stash/rebase union.
AFL verdict12lines, explicitpipefail, any regular saved crash except README;
constructed missing/README/default/worker checks +failed-producer boundary pass.
First worker check contamination retained/corrected. Still needs real Linux AFL
saved-crash with zero engine exit; no supported Darwin AFLpackage, don't force
or invent engine/tool. No recent100artifacts supply old realcrash proof. Signed
normal gate/actual Linux feature and fuzz CI/protected delivery remain. Integrate
onto workload parent after it merges, preserve stashes and all append-only records.
PRbody/tmp/mnesis-pg9-verification-pr.md needs actual delivery evidence.

CAPACITY /tmp/mnesis-postgres-capacity bench/postgres-capacity base5b86 WIP.
Preserved stash58c04f27; only research append conflict unioned/protectedprefixexact.
685/700lines +manifest4/6 +README17/18 +existingallocatorCOUNT4churnlines.
Strictall/all PASS13.42s after consthelper red, optimized build11.48s/no-run.
ZERO native measurements so far. Artifact from CargoJSON
root/target/release/deps/capacity-4117bc1a0d839cb6. Matrix10k/100k/1m,short/long,
32/4096B random recorded template,1/16/128clients,1/8events independent/competing,
3windows>=100samples, fixed128rows/1MiB/24pool, fullglobaldrain/native counts,
quantiles/throughput/allocationcalls+bytes/allocator/RSS/pools and actual cached
prepared-query EXPLAIN. Native seed eligibility awaited/setup excluded. Append
native row growth explicit. Model overhead/RSS snapshot vswhole-command resource
scope honest. Hardware/tmp/mnesis-pg8-capacity-hardware.txt M4Pro14cores/24GiB/
macOS26.5.2/Rust1.99/PGtools18.6. Native10k smoke thenfullmatrix only after active
localgate ends; source integration/current full evidence/minimization required.

Remaining final acceptance: full capacity matrix/plans/noise/saturation/resources,
PG9 exact real native feature/default/all/downstream and real AFLfailure proof,
final acceptance map/current PRD/backlog/docs, minimization/two fresh whole audits,
focused protected delivery, final existing release process and clean registry
native consumer. No whole-goal completion until all pass with exact evidence.

EVERY actual test run: three distinct reviews of owning contract; actual
execution/falsifier/counts; unchanged source/assertions/selection/deadlines.
Retain reds. Compilation/guards/empty binaries/cached or echoed logs are zero new
runtime credit. Normal signed commits and unchanged .githooks/pre-commit Nix gate.
One full local gate and one native DB suite at once; avoid sharing CPU/IO during
native measurements. Nix profile/tmp/mnesis-postgres-security-shell, root/target.
All PG17DBs share physicalcluster; fullsoak/privatePG18 has closed. No broadcleanup.
No new framework/runtime/customdevelopmenttool/projectPython. Current free16GiB.
