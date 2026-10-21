# Command processor delivery — extraction repair, 2026-10-21

Persistent CP0–CP9 goal remains active without a budget. Required delivery still
includes native processor integration, telemetry/performance, publication and
server registry consumption. PRD/backlog and feature research own acceptance.

FoundationPR411 protected-merged ca62b557ed870ec7ceb336ea9d08f21df0e664d5,
actual/expected tree3ee54613c61226c317c63347c6bde81c7c04711e. CI repairPR412
protected-merged efec41a. No unrelated releasePR410/main-user/mobile work changed.

This /tmp/mnesis-command-extract worktree is feat/command-extract/PR413, integrating
protected foundation/main and the targeted mutation repair. Original b711855 passes
32 owner tests and1626 workspace tests, but CI169mutants fails3missed/1timeout.
Actual first-failure review separates88 runtime catches/77 unviable. New negative
controls catch capacity0/1 and stopped-call predicate within test deadlines;
baseline32PASS, all3caught, no timeout. Three reviews after every actual run are
in docs/prds/command/research.md; source/log receipts in cp1-mutation.tar.gz.
Only canonical test assertions/wait bounds and one exact existing-policy private
Debug registration change. No production/API/dependency/format change. Named
stash and raw patch preserve pre-integration repair until signed commit verified.

All12 current host outputs pass (1626PASS479.112s); replay2/1/1PASS, all three-reviewed.
Next: normal signed commit hook, push and
review every current-head CI result; protected merge only after clean checks and
resolved threads. No bypass. Serialize full gates (--max-jobs1); root outputs under
/tmp/mnesis-command-start-20261021/cp1-repair-check*. Preserve raw failure evidence.

Primary /Users/joel/orca/workspaces/nexus/command-prd is feat/command-errors, signed
02c8ac8645da731db6af05aad93ebc9e28a55852 (CP2 typed errors/custody). All12 local
outputs and normal hook pass,1628tests; post-test Nix classifier crash on the sole
LCOV text output is retained as a tooling limitation, not a completed scan.
CP2 protected delivery is pending. Continue CP3 forced races, CP4 public accepted
fact reader/format/projection, CP5 lifecycle/custody/telemetry, CP6 real stores/
performance/soak, CP7 docs/features/fuzz, CP8release, CP9server consumption.
CP5 reentrant-wake and stop/dequeue falsifiers are recorded, not yet resolved.
