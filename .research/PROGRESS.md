# Progress

Persistent PG0–PG9 delivery goal active, no token budget. Repository:
/Users/joel/orca/workspaces/nexus/main. Read canonical devrandom-research SKILL.md
and full references/protocol.md after compaction. Authoritative requirements:
docs/prds/postgres.md and docs/prd-backlog/status.md. Evidence and budgets:
docs/prds/postgres/research.md and evidence/. Preserve all worktrees/stash.

User requires THREE case reviews AFTER EVERY test run: acceptance/contracts,
original-defect/realistic-fault falsifiers, then assertions/skips/timing/oracle
loopholes. Fix meaningful findings and rerun. Never weaken tests or hide outcomes.

Root branch docs/postgres-prd, signed headc5a5ac340712970178fec00fcca4297505ab5c64,
incorporates upstream0.4.0/Rust1.99 release6faddb7f. PR376:
https://github.com/devrandom-labs/mnesis/pull/376. Initial bounded reads/recovery
only.79 native tests pass,zero skips; full12 Darwin checks,1524 workspace passes,
four compiler exclusions. Calibrated peak allocation and exact own-listener
cancellation witnesses added after three reviews found weaker earlier oracles.
Source hash relative6faddb7f (adapter src/tests + core docs):
3f6f3b4697bbaa5d4788b4eb1304f0449c64764f93f88b899bdc844df405087c.

Current root UNCOMMITTED slice: .github/workflows/mutants.yml, flake.nix,
research and evidence. CI run39105034067 had14 missed mutations because no DB
was supplied. Do not exclude Postgres to fix it. Workflow now starts fresh native
Postgres with initdb/pg_ctl/createdb, Unix-only socket, trap cleanup, serialized
nextest and direct Nix cargo-mutants (personal ~/.cargo/bin plugin is broken).
Nix devShell adds existing postgresql package. Locked default is18.6; earlier
native baseline uses17.10. First real17.10 diff sweep27 mutants:14 caught,
13 compile-unviable,zero missed/timeouts; baseline79/79. Three reviews recorded.
Fresh workflow-extracted sweep is LIVE unified exec31585, log
/tmp/mnesis-fresh-mutation.log, output/tmp/mnesis-fresh-mutation/mutants.out.
Collect results, review three times, verify fresh server stopped/directory gone,
record evidence. Full root gate85712 completed with all12 outputs reused; no
new test execution credited. Then signed normal-hook commit/push CI correction
and resolve all PR376 checks/reviews before protected squash merge. All other
current PR376 jobs (Nix, compiler, feature, native CI, fuzz, deny, CodeQL) passed
atc5. Ruleset7146669 has no bypass. Never force main or bypass protected workflow.

PG3 DELIVERABLE checkout /private/tmp/mnesis-postgres-pg3-delivery, branch
fix/postgres-outcomes. Base reconciledc5; phase-only changes staged. Signed
commit currently LIVE unified exec (see /tmp/mnesis-pg3-commit.log).90 native
cases/zero skips,fmt/strict Clippy and all12 Darwin checks pass (1535 workspace
passes/four compiler exclusions); phase diff hash:
3f681b11f5b4c68b9986d70adf952e66c998a774bff3ba0de1c40a581bbcbc1c.
Rechecked same hash and cached full gate on delivery checkout; three reviews
recorded. Commit adds CommitUnknown preserving sqlx source; only verified commit
rollback codes are definite,40003 unknown; unrelated unique failures not Conflict;
confirmed hints bounded250ms. Scoped Unix backend forwarder suppresses actual
COMMIT ack, independent control proves durable rows, exact EOF preserved. Typed
Counter execute/reload reconciles without resubmission. Actual deadlock/SSI and
PG5/PG6 reuse still required. Push branch, open focused PR (can stack temporarily
on docs/postgres-prd). After376 squash merge, rebase ONLY phase commit onto new
main using c5 as old boundary; do not reinclude first PR history. NoPG3 merged yet.

PG3 RESEARCH checkout /private/tmp/mnesis-postgres-outcomes, branch
work/postgres-outcomes, HEAD6faddb7f; root read/wake baseline staged, tree stored
/tmp/mnesis-postgres-pg3-base-tree. Preserve original named stash@{0}. Extract
phase-only diff against that TREE, not HEAD. Delivery patch safely committed
before dropping any original; no need to drop it at all.

PG4 checkout /private/tmp/mnesis-postgres-schema, branchwork/postgres-schema,
HEAD6faddb7f, root+phase baseline staged, tree
/tmp/mnesis-postgres-schema-base-tree. New representation_tests.rs203lines,
all6 original cases fail against realPG: open readonly performsDDL, unsigned
version/resume beyondi64, high version append, NUL EventType,5120B raw key.
Log/tmp/mnesis-pg4-original.log + evidence,three reviews recorded there.
NoPG4 production changes yet. Add reasoned test panic lint exception before
Clippy. Budget+700/-200/net500 Rust,+350 SQL,no new engine/framework/dependency
except supported SQLx migrate. Native hash EXCLUDE USING hash(key WITH =) with
ON CONFLICT ON CONSTRAINT DO NOTHING supports5120B keys,23P01 duplicate,
real colliding hashes remain distinct via equality recheck. Candidate native
streams identity table + compact numeric FK avoids raw-key Btree tuple limit.
Need actual adapter concurrency/plans/migration evidence before retention.
NUMERIC(20,0) for fullu64 version,eventtypeBYTEA,xid8text parse preservesu64.
Native xid index ordering remains uncast. Scoped SQLx migrator connection must
close_on_drop (failure lock/search_path), no outer transaction beyond unlock.
Explicit migration API; readonly runtime verifies version/checksum/critical shape;
unknown newer fails; verified legacy adoption preserves positions. Bootstrap
owned schema serialized; no second migration engine. Include.sql in Nix source.

PG5–PG9 still REQUIRED: distinct-target/complete-run validation before storage,
AtomicAppend, snapshots, projection CheckpointStore revision CAS, StreamLister,
finite ConsistentExporter session and complete format2 CBOR restore, secure TLS,
lifecycle/operations, actual capacity/30min soak/recovery, migration upgrades,
fail-closed native CI, docs/minimization, protected merge, release-plz publication,
clean registry consumer. PG10 conditional. Labcast owns command IDs/dedup/fencing.
Incoming scripts/check-features.py and NixOS Python testScript must be removed
through supported Cargo/Nix facilities byPG9, preserving checks. cargo-hack0.6.45
candidate researched. Assess event-listener5.4.1 advisory patched>=5.4.2; no ignore.

Nix dev profiles: /tmp/mnesis-postgres-shell-199 (Rust1.99, noPG binary),
/tmp/mnesis-postgres-shell-ci (same +PG18.6). Use nix develop <profile> --command;
no host installs. Native DB17.10 running cluster/tmp/mnesis-postgres-pg0,
socket/tmp,port55432. DBs mnesis_pg0, mnesis_pg4, mnesis_pg_mutants isolate tests.
DATABASE_URL='postgres:///mnesis_pg4?host=/tmp&port=55432'. Tests sharing tables
must run serially. CARGO_TARGET_DIR root/target can reuse artifacts. /tmp is a
symlink: Git Nix refs need /private/tmp worktree paths. No merged/publication/
consumer proof exists; goal must stay active until complete delivery.
