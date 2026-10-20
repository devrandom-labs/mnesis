# Postgres delivery

Required PG0–PG9 are delivered in published0.5.3. The [acceptance map](acceptance.md)
links every requirement to its owned tests or measurements. PG10 remains conditional.

| Evidence | Exact delivery |
| --- | --- |
| Completion | [PR407](https://github.com/devrandom-labs/mnesis/pull/407), protected merge `e8bbb1093e5dacbfe82d0cc43a900bc03a739146` |
| Reviewed completion head | `189804de4a3eb3e9dfabc4ab065073a69455e15f`;42successful checks,1scheduled skip,0review threads |
| Completion expected/actual tree | `d8e66f8e21619e8b2bd0bdb1aaaad2844c04958c` |
| Release | [PR404](https://github.com/devrandom-labs/mnesis/pull/404), protected merge `9c46ec7535f935154e4a70bd0b9fa72bcc54a21d` |
| Reviewed release head | `79aefa351044497aaa5a816161c3a11e5eff256d`;42successful checks,1scheduled skip,0review threads;12normal host checks pass |
| Release expected/actual tree | `8c1690dddece3fac42eb8d9b088e157285506e3d` |
| Publication | [Run39473891856](https://github.com/devrandom-labs/mnesis/actions/runs/39473891856) succeeds and publishes all9crates0.5.3; [Postgres release](https://github.com/devrandom-labs/mnesis/releases/tag/mnesis-postgres-v0.5.3) published2026-10-20T18:22:53Z |
| Registry | All9versions present/not-yanked; [identity receipt](evidence/pg9-registry-053.json) |
| Postgres archive SHA256 | `64a900d42938d753ad332faf351dd2a0d5c75deb9b887798fc9ca2c791e233a4` |
| Source equality | All4runtime source trees and Postgres migrations equal the protected publication commit; each archive's VCS record names that commit |
| Product source fingerprint | `dc45f0ac1d4420a4eca4cfcb17f5e2e2bd7000e3857dca8b6ebacf2a40954ae2` |
| Isolated registry compile | All/no-default/export/import/projection/snapshot/tracing7/7PASS; all4Mnesis dependencies registry0.5.3; [actual compiler log](evidence/pg9-registry-compile-053.log) |
| Registry native consumer | Fresh private PostgreSQL18.6,fsync/full_page_writes/synchronous_commit=on;1PASS.076s/0skip, backend0 after exit, waited server shutdown succeeds; [actual run](evidence/pg9-registry-native-053.log) and [external consumer source](evidence/pg9-registry-consumer-053.txt) |

The final source received two fresh contract/source audits. Every actual test
invocation received three reviews: owning invariant/falsifier, actual execution/
count/result, unchanged source/assertions/selection/deadlines. Compiler-only,
empty, guarded, cached and echoed output receives no database runtime credit.
The ordinary signed Nix hook and protected workflow remain unchanged.

The [selected release CI transcript](evidence/pg9-release-053-checks.log) includes
complete all/default/export/import/projection/snapshot/tracing native suites
264/149/168/176/173/174/150, each zero skips and preceded by required laws.
Both Nix runs pass1594workspace tests and replay2/1/1. Workspace database guards
are not native evidence. The [publisher receipt](evidence/pg9-release-053-publish.log)
retains actual publication output; registry/archive identity independently confirms it.

The clean consumer uses exact published versions, no workspace/path/patch/dev
unification, and fails if its real database is absent. It checks binary-key events,
exact checkpoint revision/position/state/history after close/reopen, inclusive
stream reads and exclusive global resume. Unknown writes fail without retry.
Capacity and recovery evidence retains its stated limits in [capacity](capacity.md)
and [recovery](recovery.md). Original failures, rejected experiments, budgets and
per-run reviews remain in [research](research.md).
