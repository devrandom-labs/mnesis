# Current Postgres delivery — 2026-10-19

Goal ACTIVE/no budget, required PG0–PG9. Root docs/postgres-prd8357c0c2 USER
research.md modification untouched. No delegation. Preserve all worktrees,
stashes/proof branches/raw data. User narrowed security work to explicit PRD
requirements; focus on remaining performance/recovery/CI/docs/release.

23 protected fixes through399 e7bb9f45. User release378 merged787d61c7;0.5.0
published by39305353720, native sparse registry confirms not yanked. Preserve
all release versions/requirements/locks/changelogs. Whole acceptance incomplete.

Diagnostics /tmp/mnesis-postgres-diagnostics fix/postgres-diagnostics signed
4e7e636408d7a22d77d0c615dda0d6514cd6593a G: current0.5 native263PASS147.944s,
0skip23binaries; strictall7.39s/no-default4.89s/fmt/Taplo/HakariPASS. Normal
signed gate32709 TERMINAL0/all12PASS, workspace1593PASS672.998s18slow4compiler
exclusions, replay2/1/1PASS1s each, no_std/store/Wasm compile owners pass.
Every actual invocation THREEreviewed; no empty tracing binary/native guard,
cached/echoed logs or compile-only target earns runtime credit. Current Rust
patch625c798bf11891f839fcb1b10f5c67c5968e367476921c2fa8551895ad918d88.
Now record-only normal signed amend, then push/open focusedPR promptly;
actualLinux/native/feature/fuzz/mutants/CodSpeed logs/artifacts/threads review,
exact-head ordinary protected squash. No admin/waiver/threshold changes.
PRbody/tmp/mnesis-pg8-diagnostics-pr.md update with actual gate completion.
Logs/tmp/mnesis-pg8-diagnostics-release-*; old eb2/482 proof branches retained.

Soak /tmp/mnesis-postgres-soak test/postgres-soak basedf92 diagnostics(.5),
436line native benchmark+5manifest+14README, no production/dependency changes.
Current10s smokePASS16,478events/3659rounds/one real missed-hint/reconnect/restart
fault/44conns; original rescan-disabled and shifted-ordinal controls fail intended
runtime assertions. All3reviewed, raw/copies retained. Full1800s START NEXT after
final diagnostics record-amend terminal and source-preserving parent integration.
No other native or full gate concurrently. Exact benchmark bytes must stay.
Record actual source/settings/start/end/counters/resource bounds; zero30min
credit until complete. Use existing Nix shell and shared root/target.

Capacity /tmp/mnesis-postgres-capacity bench/postgres-capacity basedf92, WIP.
Preregistered real10k/100k/1m, short/long,32/4096B noncompressible templates,
1/16/128 consumers,128row/1MiB fixed pages, early/late stream/global page service,
full global drain,1/8batches competing/independent writers,3windows>=100samples,
quantiles/throughput/allocationcalls+bytes/allocator+RSS/connections/plans/noise.
One reporting Cargo benchmark in preparation, no runtime/compile credit yet.
Existing shared memory fixture addsCOUNT2lines; original counters preserved.
OSrandom template bytes recorded for exact replay, no new RNG/library/tool.
Do not start native capacity while soak runs. Current benchmark budget700Rust,
manifest6/README18,4sharedfixturelines; minimize/check before retention.

Stillrequired PG7 operatorRPO/RTO profiles; PG9 actual feature isolation/default/
individual/all/native+downstream, remove project Python using supported facilities,
AFLsavedcrash-zeroexit gate proof, concise public example/docs/capability map,
minimization/two fresh audits, final protected delivery/publication and clean
registry real-DB consumer. PG10 conditional; Labcast IDs/dedup/fencing unchanged.
No framework/runtime/custom devtool/projectPython. Every test THREEreviews.
ONE local fullgate and ONE native suite at once. Nix profile
/tmp/mnesis-postgres-security-shell; root target. NativePG17DBs all sharecluster.
No blocker/whole-goal completion claim. Current free disk9.1GiB; no cleanup done.
