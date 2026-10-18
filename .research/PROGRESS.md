# Progress

Persistent goal active with no token budget. Working on `docs/postgres-prd`:
[descriptive PRD](../docs/prds/postgres.md) and
[implementation status](../docs/prd-backlog/status.md).
The PRD includes source findings, exact selected dependencies, ownership,
requirements, failure cases, falsifiers, budgets and delivery gates.

PG0 is feature-complete: native regressions reproduce unbounded reads and three
independent wake failures against original production source, with original 56
adapter tests passing and contract decisions recorded. PG1/PG2 are active.
Commands, exact measurements, environment, original logs and contract decisions:
[characterization](../docs/prds/postgres/research.md). First read/recovery changes
now pass all 63 real-database adapter tests and targeted Clippy. Bounded-read
memory stays ~1.09 MB for a 10× backlog increase; original memory grew 10×.
No schema/dependency changes, commit, PR, merge or release yet.

Local Postgres is running from locked Nixpkgs at `/tmp/mnesis-postgres-pg0`, Unix
socket `/tmp`, port 55432, database `mnesis_pg0`. Native dev shell works with its
optional rust-analyzer filtered through a temporary override, documented in the
characterization. Logs: `/tmp/mnesis-pg0-baseline.log` and
`/tmp/mnesis-pg0-wakes.log`. Use serial tests: setup truncates the shared table.

Raw evidence is checked into `docs/prds/postgres/evidence/`. Temporary logs retain
the full command output. Changes include a timer-backed Postgres registration,
finite keyset page streams using existing futures facilities, a batch-size pool
constructor, native tests and clarified owner documentation. All new files have
intent-to-add entries so Nix sees source/tests; none is committed.

Next: finish formatting and run the required full Nix gate. Capture query plans,
complete practical read byte bounds and remaining PG2 lifecycle/recovery cases.
Then execute PG3/PG4 and later requirements through the status index. The selected
row/numeric/event-type representation decisions still require PG4 migrations.
Full scope, final audits, protected merge, release and registry consumer remain.

Latest refinement: query plans proved nullable resume filters and the `txid`
output alias still caused backlog work/sorting. `store.rs` now uses SQLx
`QueryBuilder` for strict native-key resume and `ORDER BY events.txid`.
All 63 current real-DB tests pass, zero skips, after removing the redundant
per-page watermark (fixed safe cutoff remains). Targeted Clippy passes. Its
80-line limit was handled by removing redundant bindings/conditions, with no
lint changes or new abstraction.

First full Nix gate passed (session 30447): 12 configured Darwin checks,
1,358 passing workspace tests, four skips, fuzz replay; known rustdoc warnings.
It precedes the latest query refinement. Formatting and targeted Clippy now
pass; current native database test duration is 6.545 seconds.

Final source-matched gate passed: unified exec session 38340, log
`/tmp/mnesis-postgres-flake-final.log`. Rust diff SHA-256 at invocation:
`18dda4e3770c6f357ec1c4043e22654c444f1af5c3aff3f58ae5cc2cc5d4f762`.
All 12 configured Darwin checks pass. Subsequent source changes require a fresh
matched gate. First experiment is ready to commit; no PR/merge/release yet. Nix dev shell is
rooted at `/tmp/mnesis-postgres-shell`; use explicit Git flake references for
new shell invocations. Local Postgres remains running on the recorded socket.

After the matched gate: commit the retained first experiment with its tests and
records, then continue remaining PG1 byte-budget/cancellation and PG2
listener/reconnection/lifecycle campaigns. PG3 needs a real COMMIT-acknowledgement
loss fault with independently proven durable rows; arbitrary backend termination
cannot substitute for that proof. The goal remains active through PG9 delivery.
