# Command processor delivery — CP3 qualification, 2026-10-21

Persistent CP0–CP9 goal remains active, no token budget. Full delivery still needs
required merged changes, publication, clean registry and labcast-server consumer.
PRD/backlog/feature research own acceptance. Do not mark complete from local checks.

## Delivery

- CP0 PR411 protected-merged ca62b557ed870ec7ceb336ea9d08f21df0e664d5;
  expected/actual tree3ee54613c61226c317c63347c6bde81c7c04711e.
- CI repairPR412 protected-merged efec41a. No unrelated PR410/user-main/mobile work.
- CP1 PR413 repair signed/pushed6d0d8adc300c9aaa542a5e94b3b53bc95e2199be,
  two parents b711855 and protected ca62b557. All12 host outputs/normal hook pass,
 1626PASS479.112s; original mutation169 failed3missed/1timeout. Three targeted
  behavioral controls now caught; exact private Debug follows existing policy.
  Current-head mutation39630929359 passes168:91caught77unviable0missed/timeout,
 three-reviewed from artifact12359961832; only one required Nix CI remains live
 on last inspection. No premature protected merge claim.
  Worktree /tmp/mnesis-command-extract is clean. Own repair stash retained alongside
  unrelated pre-existing stash; do not drop unrelated entries.
- CP2 signed02c8ac8645da731db6af05aad93ebc9e28a55852, draftPR414 stacked on413.
  Typed errors/custody locally qualified, all12 outputs/normal hook pass1628tests.
  Sole LCOV output's post-test Nix classifier crash is retained, not suppressed.
  Current push CI has no failure; Nix check was still pending on last inspection.
  Integrate protected extraction and retarget main before protected delivery.

## Current source

Primary /Users/joel/orca/workspaces/nexus/command-prd is feat/command-races,
parent02c8ac86. CP3 changes are staged for the normal signed commit. Existing FaultStore gains one optional
append gate. Canonical shared race helper forces both real handlers to append
before any release;192 fixed schedules and256 generated concurrent histories.
Original256 serial histories remain. Identity/metadata mismatch, exact receipt,
whole raw record/prefix, state and exactly two contender decisions are asserted.
Debug/release35PASS, one original ignored measurement; Clippy passes with no new
lint waiver. Both meaningful negative controls fail product response assertions,
not setup/compiler/runner timeouts. Source product modules remain unchanged.
Five test owners,278 net test lines, under350 budget; one justified Nix owner
adds the Darwin nextest fallback, without product code changes. All actual runs have
three reviews, with raw failure/control logs retained in the feature evidence.

## Next actions and resources

CP3 initial gate failed an unchanged macro nested-compiler lookup;654PASS1FAIL
with no final summary, agent-interrupted at581MiBfree. All35command cases passed,
but that partial run is not a green gate. A direct tool-alias probe fails134
before and passes0 after the selected toolchain/lib fallback. Darwin-only
nextest environment in flake.nix uses supported lib.makeLibraryPath;5lines,
no changed test/filter/assertion/workload. The full retry passes all12 host outputs,1629PASS439.141s;
raw cp3-full-gate-driver.log; actual derivation id33yn confirms fallback at
/nix/store/85qgkjfkq3q4kcxzcf2sy2d3wn8d7z37-rust-1.99.0/lib. The prior failing
macro now passes. Source was staged first so races.rs participates in Nix. Its test/control bundle
cp3-tests.tar.gz SHA25618fa5d2d4ba47b255e8dbfa60e3004446594d48ff9558137291e608c4b6c3679.
Final raw receipt cp3-gates.tar.gz retained. Finish normal signed hook;
open focused stacked PR. No cached check earns new test credit. CI413 needs actual
full mutation review, source/current-head checks and protected exact-head merge.
CP2 parent integration is prepared in /tmp/mnesis-command-errors (uncommitted
merge of6d0d8adc into02c8ac86). Product source unchanged; two test owners gain
stronger parent assertions. It needs its serialized gate/hook before push.
Then integrate protected main/retarget CP2 and CP3, preserving all evidence.
Continue CP4 public validated acceptance/format/projection; CP5 completion custody,
stop/dequeue/reentrant wake/drop, timers and telemetry; CP6 real Postgres/Fjall,
performance/instrumentation/15min soak; CP7 docs/features/fuzz; CP8release; CP9server.
CP5 concrete falsifiers/ownership distinction are in HYPOTHESES, not yet qualified.

Raw logs/output roots /tmp/mnesis-command-start-20261021. All full local gates must
serialize with --max-jobs1 and normal hooks append max-jobs=1 to NIX_CONFIG. Retain
standard --out-link results to prevent background reclamation. Only goal-owned
generated caches may be cleaned; no global GC/config or other projects touched.
Persistent Nix shell session91477 avoids repeated environment startup; commands
using nextest/mutants redirect stdin from /dev/null to preserve shell input.
