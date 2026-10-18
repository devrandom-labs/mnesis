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

PG4a matched gates NOW PASS:12 Darwin,1549 workspace/four compiler exclusions,
467.299s; fresh ARM18.6 native104/zero skips13.409s. Deliberately missing-role
pipeline run rejects as expected with Nix exit1 and server cleanup; no masked tee
failure. Three post-run reviews recorded per run. Source frozen hashd5983519.
Current source ready for normal signed commit, rebase onto mergedf87afc12, focused
PR and protected CI. Only evidence/records changed since matching source gate.
