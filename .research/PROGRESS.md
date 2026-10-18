# Progress

Persistent PG0–PG9 delivery goal active, no token budget. Repository
`/Users/joel/orca/workspaces/nexus/main`. Current branch **docs/postgres-prd**
is in a resolved, **uncommitted merge** of origin/main `6faddb7f` (0.4.0 release).
Do not discard the index: it preserves upstream audit/release work.

Authoritative scope/backlog: docs/prds/postgres.md, docs/prd-backlog/status.md.
Evidence/decisions/budgets: docs/prds/postgres/research.md and evidence/.
Read full canonical devrandom-research protocol after compaction.

Committed/signed initial experiments: 87a9d054 (PRD/contracts), af6a43b4 (rescan
wake deadline), 755ec8eb (bounded keyset rows), d4624e58 (listener recovery),
674ed905 (byte-limited pages before body fetch). Each passed all 12 Darwin Nix
checks. Latest pre-integration run: 1,374 workspace tests/four skips; native 79
passed/zero skips. Current PR: https://github.com/devrandom-labs/mnesis/pull/376
(initial read/wake slice). It was opened before incorporating newer main, so
GitHub currently reports conflicts. Required checks started on its old head.

MERGE IN PROGRESS: main advanced through non-Postgres audit #373/#374 and release
#375. Preserve Rust 1.99.0 and all 0.4.0 dependencies/contracts. Only conflict was
AtomicAppend documentation in crates/store/src/import.rs; resolved with BOTH
new distinct-target/run validation and faithful remote commit uncertainty.
PRD PG5 follows distinct raw targets; PG6 now uses projection CheckpointStore CAS
(not unconditional snapshot state); PG7 includes finite consistent ExportSession
and completed format-2 backups. These are required current owners, PG10 remains
conditional standalone subscriber checkpoints. Incoming Python feature script
and existing NixOS Python harness must be replaced with maintained Cargo/Nix
facilities in PG9, preserving checks. Do not add custom replacement scripts.

Integrated native source already passes fmt/strict Clippy and all79 Postgres
tests/zero skips in12.568s. LIVE full gate **unified exec session48147**, log
/tmp/mnesis-postgres-core04-flake.log. Source diff SHA256 relative origin/main
(under postgres src/tests + core import/store docs) at invocation:
1bdafa8dde90f40343ba98e283404906edf55dc4f1a0296ca113b4c8ac186153.
Collect this existing handle, keep Rust unchanged until retained. Then update
records, stage docs/evidence and commit the merge through normal signed hook;
push docs/postgres-prd and resolve all protected CI/review failures before squash
merge. No bypass. Main ruleset7146669 requires Nix Flake Check, Compile-fail tests,
Feature isolation and resolved threads; native PG CI also required by PRD.

IMPORTANT: uncommitted PG3 experiment safely saved in **stash@{0}** named
`postgres PG3 outcome experiment before PR376 base integration`, originally on
branch fix/postgres-outcomes at674ed905. It adds tempfile dev-dep, one private
Unix transport test file and error/store changes. Original six phase tests: five
pass, unrelated unique wrongly Conflict fails; actual COMMIT ack suppressed while
independent connection proves3 durable rows returned ordinary Sqlx Io. Current
experiment passes9 native outcome tests incl explicit CommitUnknown, cancelled
ack, stalled hint bound, deferred SQLSTATE mapping40003 vs rollbackcodes and
sequence-PK collision. Formatting and strict Clippy passed on old base. Logs
/tmp/mnesis-outcome-original.log and /tmp/mnesis-outcome-tests-premerge.log.
After merge commit, move fix/postgres-outcomes forward to reconciled base and
apply (do not blindly pop/drop) stash; preserve both research records during
expected docs conflicts and keep 0.4.0 lock/dependency versions. Reverify all PG3
against newbase. Import/snapshot outcome reuse and wrapper propagation remain
required; no full PG3 gate yet. No PG3 source is in current PR376.

PG4 and all PG5–PG9 implementation/delivery still required. Migration candidate
must use scoped close-on-drop connection and SQLx locks/transactions, no outer
transaction outliving migrator unlock. Measure long incompressible key index
limit; never replace identity with probabilistic hash. Audit advisory event-listener
5.4.1: existing SQLx/wake uses Event<()> heap listeners; compatible patched>=5.4.2
must be assessed PG9, no new ignore. Locked Nixpkgs cargo-hack is0.6.45; upstream
supported --each-feature --no-dev-deps is candidate replacement for Python feature
script, not implemented. No framework/tooling/runtime/Python addition authorized.

Postgres17.10 running: cluster/tmp/mnesis-postgres-pg0, socket/tmp, port55432,
DBmnesis_pg0; DATABASE_URL postgres:///mnesis_pg0?host=/tmp&port=55432. Serial tests
only, shared TRUNCATE fixtures. Native commands use explicit Git Nix flake path,
blank shellHook/filter optional rust-analyzer, exact command in research. No
merge/publication/clean registry-consumer evidence yet; goal must stay active.

USER STEERING: After every test run, review cases THREE times: acceptance,
original-defect/realistic-fault falsifiers, then loopholes (assertions/skips/timing).
Record all three before accepting evidence. Latest79-native run reviewed in
research: peak-memory witness gap and any-listener cancellation oracle found.
After collecting current gate, strengthen tests with PEAK tracking and a unique
pool application name, run native again and perform all3 reviews, then matched
full gate +3 reviews. Do not skip checks to avoid a rerun. PG3 stash tests also
need exact EOF cause matching and real wrapper propagation, not just a variant.

UPDATE: session48147 completed successfully (12 checks,1524 passes/four compiler
exclusions,453.504s). Three case-review passes recorded. The strengthened native
source passes all79/zero skips (12.654s), with a calibrated freed-allocation peak
oracle and exact listener identity. Real payload-guard mutant fails; guard was
restored unchanged. New LIVE full gate **session98721**, log
/tmp/mnesis-postgres-strong-flake.log, source diff SHA256(relative6faddb7f):
3f6f3b4697bbaa5d4788b4eb1304f0449c64764f93f88b899bdc844df405087c.
After it finishes, perform3 reviews, stage records and commit the resolved merge
through cached normal signed hook. Then pushPR376 and check all required jobs.

To continue useful work while root gate is fixed, isolated checkout created at
**/private/tmp/mnesis-postgres-outcomes**, branch work/postgres-outcomes. Its HEAD
is origin/main6faddb7f; the current root read/wake patch is STAGED as its baseline,
whose Git tree hash is in /tmp/mnesis-postgres-pg3-base-tree. The PG3 stash patch
was applied on top; only research append conflict was resolved keeping BOTH
blocks. No experiment was committed there. Extract future PG3-only patch using
`git diff $(cat /tmp/mnesis-postgres-pg3-base-tree) --binary` from that checkout,
not `git diff HEAD` (HEAD would include the entire read/wake base again).
Named original stash remains preserved. Root Rust source is unchanged by this.

PG3 isolated checkout now asserts exact UnexpectedEof source in all phase cases
and adds a real Counter/JSON repository execute/reload case using existing shared
test-domain and codec dev facilities. Native fmt/Clippy/all-adapter run LIVE in
**session41179**, log/tmp/mnesis-pg3-core04-native.log. Expected89cases but record
actual output. Perform all3 case reviews after completion. Canonical Nix Git
ref MUST use /private/tmp (not symlink/tmp); attempted/tmp ref was rejected before
any compilation/test. Root and isolated native tests must stay serial because
sameDB; root currently runs only DB-free full gate. Native CARGO_TARGET_DIR points
to root target to reuse existing Cargo artifacts. NoPG3 full gate/PR yet.
