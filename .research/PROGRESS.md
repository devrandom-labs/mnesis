# Command processor delivery — current state, 2026-10-21

Persistent CP0–CP9 goal is active without a token budget. Do not complete it until
required source/CI is merged, publication and registry consumption are verified,
and labcast-server consumption passes. PRD: docs/prds/command.md. Exact research,
source receipt and test reviews: docs/prds/command/research.md and evidence/.
Historical progress is preserved at signed b711855 and in the evidence-root
progress-before-cp2.md snapshot; EXPERIMENTS remains append-only.

## Branches and delivery

- Primary /Users/joel/orca/workspaces/nexus/command-prd: feat/command-errors,
  parent signed b71185568914f76fb06aff195c0d4268c239f186. CP2 is uncommitted.
- CI repairPR412 protected-merged efec41afeab8138801fe034be1a60cbf4055848a;
  reviewed24317917, exact tree3ab79ecda4af32faf7d8f016af4b02392df73729.
- FoundationPR411 protected-merged ca62b557ed870ec7ceb336ea9d08f21df0e664d5,
  reviewed d88c6efa; actual/expected tree3ee54613c61226c317c63347c6bde81c7c04711e.
- ExtractionPR413 at b711855: all12 local checks pass (1626 workspace tests,
  32 debug/release owner tests), but current merge is dirty after main advanced
  and its mutation CI failed. Read cp1-ci-mutants-failed.log, resolve without
  weakening gates, integrate protected foundation/main in an isolated worktree.
- Preserve unrelated managed releasePR410 and the original dirty Postgres/main
  worktree. Source controller-archive and the mobile repository are untouched.

## CP2 qualification

Typed domain/kernel/adapter causes now traverse the real handle, mailbox and Tower
reply. Cause is flat; no dynamic error or per-retry recursive chain. CodecError
preserves JSON/writer sources and distinguishes bounds/lossy/unsupported data.
Queue/config/history/replay/version outcomes are distinct. Original pending
custody survives failed exact retries; success clears it only after valid receipt.
Stored v2 bytes/schema/constants and ordering remain unchanged.

34 owner tests pass in debug/release, plus the final strengthened capacity case
in both profiles (4095→4096 via processor, old receipt, no append at full, typed
4097th-row failure). Real committed-fold panic/reconstruction/exact recovery
witness observes both native joins and one handler decision. All actual runs
three-reviewed. All12 host check outputs now exist: workspace nextest1628PASS,
549.765s,142 binaries,five established exclusions; fresh replay2/1/1PASS. The
Nix build exits0, but nextest's post-test audit-tmpdir fixup subprocess logged a
segmentation fault. That output contains only ASCII LCOV data, no executable or
wrapper to inspect. Preserve this tooling limitation; do not claim the classifier
completed. Normal signed commit hook remains required.
Budget: eight source/test owners, +151 production
lines (1181 total), two new root types Cause/CodecError; root total8. No dependency,
framework, runtime, schema, project Python or development-tool addition.

## Resources and next actions

Raw logs: /tmp/mnesis-command-start-20261021. Twelve baseline Nix check outputs are
rooted there as cp0-check*, and shell via standard --profile .../shell. Unrooted
outputs were reclaimed between checks; a parallel hook hit disk exhaustion.
Only this goal's generated Cargo target caches were removed; all code/logs remain.
Use --max-jobs1 for Nix builds and append max-jobs=1 to NIX_CONFIG for normal hooks.
Retain each fresh check result with standard --out-link; no global GC/config change.
Serialize full gates; no cached output earns fresh runtime credit.

Next: CP2 normal signed commit/focused PR; all output roots and raw evidence retained.
Resolve PR413 mutation CI and merge conflicts. Finish current-head PR411/413
protected delivery, then CP3 forced races/generated schedules and CP4 reader/
format/projection. CP5 must qualify completion report custody, stop/dequeue and
reentrant wake/drop behavior, timers/cancellation and tracing/metrics. HYPOTHESES
records concrete falsifiers. CP6 real backends/performance/15min soak, CP7 package/
CI/fuzz/docs, CP8 audits/protected release, CP9 registry server consumer remain.
No real processor Postgres/Fjall, instrumentation, performance or publication claim.
