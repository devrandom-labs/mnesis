# Progress

Persistent PG0–PG9 delivery goal ACTIVE, no token budget. Read canonical
/Users/joel/nix-config/modules/home/devrandom-research/skill/SKILL.md and full
references/protocol.md after context recovery. Requirements docs/prds/postgres.md;
dependencies docs/prd-backlog/status.md. Preserve every worktree and named stash.
No subagents authorized. Three case reviews AFTER EVERY test run: acceptance,
original defects/real falsifiers, assertion/skips/timing/oracle loopholes. Record
failures honestly; never lower thresholds, hide survivors or count DB-free returns.

Delivered: PR376 first bounded-read/recovery slice, main7db7cf83f74d9ae53b45ca9aae6c548572c73e91;
PR377 phase/error/forced-collision slice merged2026-10-18T08:42:47Z protected squash
f87afc12dd6dabe4acd7847e98756858f5fb4c86. Headb26f0556 has40 green checks/1 scheduled
skip. Native91/zero skips, full12 Darwin checks1536/four separate compiler cases,
fresh independent-cluster10 mutants:7 caught/3 compile-unviable/zero missed or
timeouts. Three reviews caught serialized false-positive conflict test, wrong
caller expected version and cluster-isolation mistake; all corrected. Actual SSI,
deadlock and PG5/6 propagation remain before PG3 closure. No publication yet.

PG4a /private/tmp/mnesis-postgres-migrations branchfix/postgres-schema, baseb26f,
uncommitted. Explicit migration API, owned schema/qualified queries, read-only
runtime shape/checksum validation, SQLx0.9 native migrator, scoped close_on_drop,
self-validating legacy SQL adoption, public guard, LF SQL/build.rs tracking.
SQLx0.8 migrate pulled vulnerable optional RSA; no ignore added. SQLx0.9 removes
RSA, supports pinned1.99, native104/zero skips/14.576s and strict Clippy pass.
Current fixed source diff /tmp/mnesis-pg4a-current-source.diff SHA256
 d59835191badbc8b17869597e7b6d0339f6604c1a2513b97a0ffe0bb4b0e3613.
Frozen tree341eba45b88d066c183548e4fd2a4f515481533b.
Matched Darwin full gate LIVE91699 /tmp/mnesis-pg4a-matched-flake.log; do not edit
Rust/SQL/manifests in that checkout until collected. Earlier12/1547 gate superseded.
Supported nixpkgs postgresqlTestHook replaces Python NixOS harness, pipefail and
published run.log. ARM candidate102/102 passed16.614s on fresh18.6; old source,
not current acceptance. Matched ARM native build LIVE96823,
/tmp/mnesis-native-linux-current.log. Need controlled native failure proof too.
Remote aarch64 builder works; x86 nixos.local DNS unavailable, no tests credited.
Keep migration1 frozen after delivery; migration2 changes representations.

PG4b /private/tmp/mnesis-postgres-representation branchwork/postgres-representation,
baseb26f plus PG4a frozen tree staged. Five native representation cases red on
baseline: signed version narrowing, high-sequence resume, NUL event type,
incompressible5120-byte key, append across signed boundary. All five failures
and three reviews recorded; implement here only, preserve PG4a frozen gate.
Candidate internal binary keys table, hash exclusion raw equality, compact event
FK, numeric(20,0) full-u64 versions, BYTEA type, native xid8/output text. Native
hash probes prove real equal-hash keys distinct and DO NOTHING supported. No
extension/custom identity/retry protocol. Preserve old latest-v1 validator inside
migration1, update latest validation separately. Remove PreparedInsert allocation
only with native range/batch/schema coverage; no contract deletion.
Budget PG4 +700/-200/net500 production Rust,+350SQL; review small slices. Before
PG5 ALL target heads must validate BEFORE ANY write/XID allocation, or multi-stream
append could reverse per-stream global ordering; add native interleaving falsifier.

Remaining PG5–PG9 all required: AtomicAppend/import, snapshots, projection CAS,
StreamLister, finite consistent export/format2 restore, secure TLS/lifecycle/ops,
actual capacity/30min soak/recovery/migration matrix, fail-closed CI, remove existing
project Python via cargo-hack, resolve patched event-listener advisory, docs/two
fresh whole-repo audits, focused protected merges, release-plz publication and
clean registry consumer. PG10 conditional. Labcast owns command IDs/dedup/fencing.
Do not merge release PR until all required acceptance complete.

ENV: Postgres17.10 /tmp/mnesis-postgres-pg0, Unix/tmp port55432, DBsmnesis_pg0,
mnesis_pg4,mnesis_pg_mutants. NEVER run independent native suites concurrently
in ONE CLUSTER even across DBs: snapshot watermark is cluster-wide, proven with
held22861/cutoff22861 then release22862. Separate clusters for parallel suites.
PG8 needs native foreign-DB lag/release recovery and operator diagnostics.
Explicit SQLx0.9 URL postgres:///mnesis_pg4?host=/tmp&port=55432&user=joel.
Nix profiles /tmp/mnesis-postgres-shell-199 and shell-ci (includes18.6). Target
/Users/joel/orca/workspaces/nexus/main/target. Direct cargo-mutants mutants avoids
broken personal plugin. Signed commits; normal hook nix flake check, no bypass.
Ruleset7146669: no bypass, squash PR, resolved threads and full required checks.
Root docs/postgres-prd remains8357 local only; merged remote deleted, do not push.
Root docs/research uncommitted preserved; active delivery records in PG4a checkout.
Find original stashes BY NAME, never blind stash@{0}; later autostashes preserved.

Update2026-10-18: PG4a signed commit a0f422178ab45f086512f58205445837defa4505
rebased cleanly onto protected PG3 merge; tree identical to cf7deb6f. PR379 open
https://github.com/devrandom-labs/mnesis/pull/379, CI queued/running, no failures
at last inspection. All local matched gates/controlled failure proof retained.
PG4b candidate in representation checkout: normalized binary keys with native hash
exclusion, compactFK events, NUMERIC(20,0) versions, BYTEA event types, text xid8,
unsigned resume fix, removed PreparedInsert allocation/four mirror unit cases.
Native110 all pass before adding3 genuine version1 upgrade cases. Foreign-view
case exposed candidate's destructive DROP; now ownership checked under catalog
search_path. Upgrade3 all pass; full113 fourth run62pass/51 old-candidate-checksum
failures before migration fixtures installed current2. Never edited checksum
metadata; rerun113 LIVE4879? actual latest handle from tool history,
/tmp/mnesis-pg4b-fifth-native.log. All failed runs and3reviews retained.
Next: collect113/3reviews, clean obsolete comments/README, capacity/head/page query
plans and mutated numeric/collision guards, matched full gates/nativeLinux, focused
PG4b commit/PR. Do not edit published migration1. Then PG5 all-heads-before-writes.

Latest PG4b state:113/113 native17.10+freshARM18.6 pass,zero skips.19 native
mutants:14caught/5compile-unviable,zero survivors/timeouts. Manual SQL txid >=
mutant killed2rows!=1 in archived /tmp/mnesis-pg4b-control.TLSUgG; live source
untouched.40,960x4096B retention1,094,103/1,100,611B below4,194,304B; generic
head/index/page plans bounded at0/40000. Single latency samples not SLO evidence.
Source diff vs frozenPG4a341eba SHA2567ea6c9a1220b94a201ad49269a91de014f517e44aca622b7e3f2207a43f39d79.
LIVE fullDarwin gate41329 /tmp/mnesis-pg4b-full-gate.log, still native Fjall
property child doing real work; do not restart or claim it passed. Linux artifact
/nix/store/iw2bpshrp0slzl0954l5ikwmk6i1qdcp-mnesis-postgres-integration.
PG4a PR379onlytwoNixCIchecks pending, othersgreen lastinspection. No failures.
PG4b not committed or PR yet. Baseline mixed stage includesPG4a; after matched
fullgate complete align branch baseline with a0f422 (preserve working tree and
all records), then signed normal-hook PG4b commit and stacked/main PR. Parent
records/evidence have been reconciled into this checkout; no history discarded.
Exact production-source hashes are unchanged. PG5 plan/budget appendedHYPOTHESES;
read all heads before any XID-allocating key/event write, one nativehint deadline
forwholebatch. All requiredremainingPG5–PG9/release/consumer work stillopen.

PG4b full gate NOW PASS all12,1558 cases/four compiler exclusions,461.040s.
Three reviews recorded, source hash7ea6c9a1 unchanged. Need baseline alignment,
signed commit/normal hook and focused PR; actual pendingCI379stilltwoNixjobs.

PG4a PR379 MERGED as e1ee1dc29c30b0116da14e4cd595a4d8c0c7beb1 at09:16:24Z.
PG4b signed d0442282 normal hook also passes12,1558/four,456.756s. Only new SQL
proof caused full rebuild, no adapter source change. Amend records, rebase onto
main e1ee then focused PR. PG5 research /private/tmp/mnesis-postgres-atomic,
branchwork/postgres-atomic, HEADa0 plusPG4b baseline staged; tree file
/tmp/mnesis-pg5-base-tree. Initial AtomicAppend API missing (compileE0277), then
5 shared+4 owner laws9/9 native pass,zero skips8.938s; strictClippy. Current draft
private store/atomic.rs + parent shared insert/head/hint owners. All heads precede
writes; after unique rejection rollback+observe actual head or retain original
Store source; COMMIT unknown stays separate. One250ms native-hint budget perbatch.
Thirdreview: tighten exact process/query advisory wait, unexplained8.170s case tail
recordedPG8. Need forced order/race, COMMIT loss, import and actualSSI/deadlock.
Do not extract PG5 relativeHEADa0: baseline includesPG4b; align safely after PG4b
commits/rebases, preserving every record. No PG5 full feature CI/delivery proof yet.

PG4b PR380 https://github.com/devrandom-labs/mnesis/pull/380 head a32cd743665d735e400072a244ba98c9de840a4f
opened against main e1ee1dc2. Stronger event-prefix transport test found during PG5
cross-capability review: preseed key before DropInsert so fault is after EVENT
INSERT, not key catalog INSERT. Selected1+full113 native pass,zero skips; changes
pending next signed commit/hook and CI. Do not merge old head without correction.
