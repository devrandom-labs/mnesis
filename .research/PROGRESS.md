# Current work — 2026-10-18

Persistent goal ACTIVE, no token budget. ALL PG0–PG9 remain in scope through
protected merges, publication and clean registry real-Postgres consumer. PG10 is
conditional. Labcast owns command identity/deduplication/controller fencing.
Latest continuation changes implementation, not just status: PR387 merged and
four new native search-path defects reproduced and fixed with unchanged oracles.
Prior turn was a verified wait, not implementation progress. The goal had been
marked blocked without a returned reason; it is now active again.

## Instructions

Use canonical /Users/joel/nix-config/modules/home/devrandom-research/skill/SKILL.md
and full references/protocol.md after recovery. Read repo CLAUDE.md and global
/Users/joel/.claude/CLAUDE.md. Strict Clippy, functions80lines/5args, no weakening.
No framework/runtime/custom dev tool/project Python. Existing Rust1.99/SQLx0.9,
current root Cargo.lock actually Tokio1.53.1 (older summary1.52.1 is stale).
No subagents authorized. After EVERY run make THREE reviews: contract; actual
fault/falsifier; assertions/skips/timing/source/oracle gaps. Research.md contains
per-run records. Unviable mutants are not runtime catches. Keep original reds.
CONTRACT immutable; EXPERIMENTS.jsonl append-only. Normal signed commits invoke
unchanged .githooks/pre-commit nix flake check. Protected normal squash requires
all checks, actual mutant artifact first-failure reviews and no unresolved threads.
No admin/auto merge, no release before whole-adapter acceptance.

## Delivery

ELEVEN protected focused merges:376/377/379/380/381/382/383/384/385/386/387.
Latest origin/main2b181b12d1040f1527d6a28375fd5db91a9d1129 is PR387 history,
merged20:23:02Z from signedb2410ad4480ec1382fb50cc1c82b688e2b51aca3.
40checks success/1scheduled skip; no review threads. Downloaded final mutant
artifact12149346087/run39161571503: baseline211/21136.111s;27candidates,
11runtimecaught/16compileunviable/0survivors/timeouts. Every first failure
reviewed3ways in research.md. Actual CI native211/211 in40.890/35.104/42.118s;
workspace1567 in80.641/121.218s; smokes/feature jobs/fuzz each3reviewed.
Full logs /tmp/mnesis-pr387-ci.log, -ci-call.log, -fuzz.log; artifact directory
/tmp/mnesis-pr387-mutants. History local16feature combinations2652executions,
0skips and211Linux verified before merge. Physical coherence is not yet explicit
operator-stop verification or actual restored Projection consumer restart.

Release lock issue fixed/merged PR386 at87e87d6e. Live Release39152877808 and
App commitdaefe06097f7310ef21a596ca8ee1bb7a5049994 verified; original four locked
AFL campaigns39153035060 pass. No new publication. PR378 stays open/unmerged.
Independent CodSpeed -10.42% remains; no threshold/reset/manual acknowledgment.
Rejected inline experiment regressed25–36%; native A/B/A does not waive x86 cost.
Release/performance worktrees preserved. Upstream Darwin audit classifier crash
was emitted despite Nix0; current inspected nextest output has only text+zstd,
no ELF/shebang files. Not an upstream fix or compressed-content audit.

## Current primary slice

/private/tmp/mnesis-postgres-path, branch fix/postgres-path, based currentmain
2b181b12. Parent squash rebased using saved owned stash85459c87b8f3e82586b3f59bf1dfe376cb845c76,
applied by exact ID and retained. Root /Users/joel/orca/workspaces/nexus/main
remains docs/postgres-prd at8357c0c2; preserve user's docs/prds/postgres/research.md
edit there. Only mirror our .research/PROGRESS.md into root.

Pre-edit budget<=20production lines/200new native-test lines,0API/migrations/deps.
Actual product9added/8removed, test156added/4removed. Four new all_noskip_tests
laws inject test-owned ordering schema beforepg_catalog. Actual older held native
transaction + Barrier/spawn SUT append prove withheld latercommit, exact final
ordered events, ordinary rowresume and u64::MAXseqresume. Remove injection before
assertions; native default-pool positive control. Originalsource4/4FAIL0.208s:
current snapshot hides all; xmin/cutoff op release B early; row op native0A000.
Minimal query-only fix qualifies two functions and three operators.
Same targeted6/6PASS0.270s; strict adapter all/all Clippy currentPASS after fixture
compile/lint repairs (no lint relaxation). Full native215/215PASS42.586s,0skips.
Fresh isolated scalar-only reversion fails exact final u64MAX oracle0.051s with
ordinary rowresume alreadypassing. All runs3reviewed. Controlworktree detached
/tmp/mnesis-path-controls.FsopcM intentionally contains scalar mutant, not product.
Proof target /tmp/mnesis-pg6a-proof-target.z1jmzk package-clean before control.

CURRENT live Linux build session36336, /tmp/mnesis-pg8-path-linux.log:
Nix build .#packages.aarch64-linux.postgres-integration with remote builder.
Only20new derivations, dependencies then adapter archive now compiling. Poll the
same handle; do not restart on observation timeout. Independent of local cluster.
Local full native76056DONE; control5566DONE expected100. No other own Cargo jobs.
Next normal SIGNED commit with full unchanged hook; then focusedPR, allCI/artifact
review/normalprotectedmerge. Freeze source; do not repeat complete16matrix for
this cfg-independent query unless new evidence warrants it. Linux and normal
hook still required; source/doc changes force owning checks, not endless reruns.

## Whole goal remaining

PG7 completed bounded format2CBOR Postgres roundtrip/Fjall interchange; empty,
many/binary/schema/malformed/truncated/route/conflict/partial/salvage/sourceerror/
cancel; real logical operator drill and actual Projection restart after physical
and logical restore; explicitly assert old-primary stop (current Drop ignores
pg_ctl exit). Existing decoder materializes one chunk. Use fixed-size supported
minicbor2.2.3 &mut[u8] sink/ChunkWriter::finish; no custom sink/framework/format.
Session&self permits lazy listing/events without collecting wholecatalog.
PG8 TLSverifyfull/auth/wrongCA/hostname/redaction; roles; startup/listener/close/
Drop/reconnect/serverrestart/unavailable/callerpool/acquire/query/lock deadlines;
durability/futureconnection settings; 10k/100k/1m x1/16/128 consumers, payloads/
writes/percentiles/RSS/alloc/connections/plans/saturation;30minute mixedsoak with
zero invariants violated and recovery bound. No invented performance targets.
Other UNPROVEN findings: startup migration metadata fetch_all beforecount;
constraint deparser searchpath; caller pool settings; session canceled-close with
blockedquery, shorter socketinterval/networkpartition scopes. Witness beforefix.
PG9 existing scripts/check-features.py and fuzz.yml embeddedPython removal via
supported Nix/Cargo/JQ (no replacementscript suite). Locked nixpkgs cargo-hack
0.6.45 available but not installed in current shell. --each-feature includes
none/default/all; --no-dev-deps TEMPORARILY MUTATES manifest: use isolated worktree,
never primary concurrent Cargo. Existing feature CI omitsPostgres. AFL zero-exit
crash gate negative proof, advisory event-listener5.4.1/ignoredstale entries,
registry/yank blindspots, isolateddownstream/features/absence gates, docs/example/
changelog/twofreshwholeaudits, protecteddelivery and verifiedpublication/consumer.

## Environment/preservation

PG17.10 /tmp/mnesis-postgres-pg0 socket/tmpport55432 userjoel.
NativeURL postgres:///mnesis_pg7b?host=/tmp&port=55432&user=joel.
ControlDBmnesis_pg7c_controls SAMEphysicalcluster; never concurrent local native
suites even acrossDBs (xmin spansDBs). Restore18.6privateclusters separate.
Nix shell /tmp/mnesis-postgres-shell-199. Sharedtarget rootworkspace/target.
Remote aarch64-linux builder ssh-ng://builder@linux-builder works; x86hostDNSfails.
Free disk last30GiB after supported package-only cargo clean reclaimed32GiB;
no raw rm/GC or unrelated job killing. Labcast NixPID63786 was independent.
Preserve all worktrees/stashes, including new85459c87 above and older:
b51e18ce53f0a3e9326676e014d3bdab26778392 fbcc7d919c5df03ee3216065de84a63ddb825992
65981423d8263e5a00d6df11e62668b3ed4cd2b6 f4fc89d0b83d5411f13eebca269b0cf76c1ef0a5
219c6aaf1dfc8ea83f6ff6d3a0389b8a8ba0449f c5bc39b383f1da5bfdf1d3ece2e1dc97075315a2.
Old history candidate notes are outdated. Do not redo finished history design.
