# Command processor delivery — CP3 CI fixture correction, 2026-10-21

Persistent CP0–CP9 goal active/no budget. Full acceptance/protected merge,
publication and registry/server consumers still required. No local completion.
Source archive/mobile/unrelated user-main/releasePR410 preserved.

CP0 PR411 mergedca62b557; CI repairPR412 mergedefec41a; CP1 PR413 mergedfa0b95b1.
CP2 PR414 merged10d1b3b31844bebe2a941ead80343eee592d8584 at17:45:38Z, reviewed26c5a334;
expected/actual/head tree9291a8825e94dae927052f49f3a0c9d798b912ab.42checks pass/one
scheduled-only skip,0threads. Both74-invocation current workflows three-reviewed,
mutation40caught38unviable0missed/timeout. Exact final raw receipts are in primary
CP4 evidence branch. Package remains release-disabled/unpublished.

This worktree /tmp/mnesis-command-races is feat/command-races at signed7534d590,
PR415/main draft. Parent integration6998d7fa and protected-main integration7534d590
have all12 local checks/normal hooks passing1629, but current push Nix CI fails:
369executed/368PASS1FAIL/1260notrun/five exclusions. PR CI passes by chance. First
failure is canonical winning receipt Uncertain after2s, not a green qualification.

Fixture audit/probe proves Gate::wait notifies entered BEFORE release waiter
registration. Force the first waiter to wait until the second actually polls its
acquire: original shared gate releases the supposedly held loser. A stronger
bounded select directly exposes actual early Ok(Receipt1), no deadline increase.
Candidate uses independent gates per contender over TWO FaultStore wrappers that
share the SAME actual backend Arc, preserving real CAS authority. The initial
probe forced only the first pair. Final counted pairwise probe forces all192
inversions, asserts384 entries and single-use signals:1PASS0.100s. Disposable
probe removed; exact faults/recovery bytes match full-gate source. New actual
production negative controls still both caught at
receipt/mismatch oracles,0missed/unviable/timeout. No product/API/dependency/Nix/
format change. Current diff two fixture owners; final PR316nettest lines in five
owners under350 budget. All192fixed/256concurrent/256serial workloads retained.
Debug35PASS1ignored1.341s; release35PASS1ignored0.503s; strict all-target/all-feature
Clippy passes1.19s after const-only constructor correction. Every actual run has
three reviews. Evidence cp3-gates-fixed.tar.gz SHA:
a8559a645c8d06bdfa4059e029916b338091b67091bbc28a3047d1ddee39a10b.

All12 corrected host outputs now PASS/rooted cp3-fixed-check-*. Full nextest
42955380-7383-437d-93ac-dc49650724a2:1629PASS/14slow/5existing exclusions/142bins,
449.181s. Actual source7zzzj1idnh6cql2qwjnb43aqn5ph34kd cmp matches four product
modules and repaired owners. Every fresh run has three reviews. Next normal
signed hook, update PR415 body278->316 and actual validation, push; review CURRENT
new-head CI before protected merge. Old push/pr logs fully reviewed:54/74 actual
invocations, no fabricated credit for echoed summaries or zero-test libs. Old
mutation job selected no mutants/no runtime; old fuzz eight legs three-reviewed.

Primary /Users/joel/orca/workspaces/nexus/command-prd is feat/command-reader at
7534d590 with preserved staged CP4. Original binary patch/named stash retained,
older/user stash preserved. All code/fixtures unchanged except stronger inherited
capacity assertion. Full C4 gate now passes1633/485.577s/all12outputs, source3kdzh5mq,
39command laws plus golden/read/projection/evolution/corruption/no-repair controls.
It predates this parent repair, so integrate corrected C3 and requalify. No CP4
commit/PR yet. Existing public reader changes+62product lines/two roots;626test
lines within amended650. No native/release claim.

/tmp/mnesis-command-lifecycle at26c5a334 has staged CP5 first repair: actual
reentrant ready cancellation deadlock and stop/dequeue command acceptance both
reproduced/fixed. Host+9net lines, tests+134; debug/release36 and strict Clippy pass.
No full gate/merge or CP3+CP4 integration. Terminal cause+pending custody still
needed; source comparison and per-worker Drop handoff candidate in SOURCES.
Timers/resources/cancellation/tracing/metrics/export and overhead remain open.
Then CP6 real stores/recovery/performance/15min soak; CP7 docs/features/fuzz;
CP8 audits/protected delivery/publication; CP9 released server consumer.

Only goal-generated caches cleaned AFTER retained evidence. Specific deletion
of three old CP3 cross outputs completed:1.4GiB; current CP4 replacements pass/
rooted and zero referrers were verified. Exact paths/log retained. Common cache
cp3-common-deps is rooted. CP4 common cache was already collected before its
root attempt; that attempt failed/no root created. No global/other-project cleanup.
Raw logs/roots /tmp/mnesis-command-start-20261021. Shell91477 currently cwd this
worktree, idle after final counted probe. Tests stdin /dev/null. Full gate96429
completed0. No overlapping local builds;7GiB free after full gate. No subagents.
