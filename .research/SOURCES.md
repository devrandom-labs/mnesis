# Research sources

Exact selected versions, inspected local contracts, upstream URLs, access date,
applicability and limitations are recorded once in the feature PRD:

- [Locked dependency verification](../docs/prds/postgres.md#locked-dependency-verification).
- [Primary sources](../docs/prds/postgres.md#primary-sources).

The installed research skill directory had no readable `SKILL.md`; preparation
used the canonical source at
`~/nix-config/modules/home/devrandom-research/skill/SKILL.md` and read its full
`references/protocol.md`. Resume with the installed skill if available.

- 2026-10-18, PG1: PostgreSQL 17 window/conditional evaluation docs and locked
  SQLx 0.8.6 FromRow/QueryBuilder; native byte/query-plan evidence in feature
  research. PG3: same-version transaction source, PostgreSQL protocol frames and
  SQLSTATE appendix. `40003` cannot share the definite-rollback classification of
  `40001`/`40P01`. Candidate test transport uses existing Tokio/tempfile only.

- 2026-10-18 audit follow-up: RustSec RUSTSEC-2026-0221
  https://rustsec.org/advisories/RUSTSEC-2026-0221.html identifies unconditional
  Send/Sync on `StackSlot<T>` with !Send tags, patched in event-listener >=5.4.2.
  Locked SQLx 0.8.6 pool `on_closed` is `Event<()>` and uses heap EventListener;
  wake-nostd also uses untagged Event::new/heap listen. No current !Send tag or
  StackSlot call was found in these paths. This limits applicability but does not
  replace the PG9 dependency remediation: evaluate the compatible patched update
  with no_std/platform gates rather than add an ignore. Current lock is 5.4.1.

## PG4 SQLx migrations (2026-10-18)

SQLx0.8.6, locked registry source, MIT OR Apache-2.0:
https://docs.rs/sqlx/0.8.6/sqlx/migrate/struct.Migrator.html
and sqlx-core-0.8.6/src/migrate/{migrator,migration}.rs plus
sqlx-postgres-0.8.6/src/migrate.rs. Native migrator supplies checksums,
per-migration transaction+metadata and PostgreSQL session advisory locks.
Errors before final unlock leave the lock held: scoped close_on_drop connections
are required. Bootstrap holds a reentrant acquisition through completion, so
schema creation is serialized before SQLx creates its metadata. No transaction
outlives the migrator unlock. Migration version/checksum authority is the embedded
Migrator::iter(), not a second manually maintained version constant.

Send experiment: SQLx0.8.6 Migrator::run on an already acquired connection fails
Acquire lifetime generalization when used by tokio::spawn/conformance. Its own
source labels run_direct as the workaround for that exact error. Use it only in
the private schema owner, pinned by0.8.6; no migration logic is reimplemented.
RawSql::execute also triggers Executor generalization here; the supported
Executor::execute(&str) returns its Send boxed future directly and compiles.
Strict all-target Clippy, including actual tokio::spawn overlap, now passes.

Feature cost: enabling SQLx migrate adds14 lockfile-only optional packages
(MySQL/SQLite drivers and their dependencies) under Cargo resolution. None is
in the adapter's normal/build tree; retained tree is /tmp/mnesis-pg4a-tree.log.
Targeted cargo-deny advisories passes with no added ignore. Full audit/license
and publication checks remain required. This feature avoids a custom migration
engine and uses the current Rust/SQLx/Tokio owners; no framework/runtime/tool.

Native hash-index equality and exclusion semantics:
https://www.postgresql.org/docs/17/indexes-types.html and
https://www.postgresql.org/docs/17/sql-createtable.html. Hash stores a code but
rechecks actual equality; exclusion can reject equal keys without treating a
hash collision as identity. Existing native5,120-byte/colliding-key probes are
candidate evidence only; adapter concurrency/migration/query-plan acceptance
still required for the later representation slice.

SQLx0.9 correction (2026-10-18): primary release75bc048 and Cargo manifest:
https://github.com/transact-rs/sqlx/releases/tag/v0.9.0
https://github.com/transact-rs/sqlx/blob/v0.9.0/Cargo.toml
Rust1.94 minimum (our1.99 supported), MIT OR Apache-2.0. The MySQL driver disables
RSA by default; using migrate without mysql-rsa removes the vulnerable rsa family
from the lock. Source migration0.9 adds create_schema/dangerous_set_table_name;
we use fixed owned identifiers before installation, never rename live metadata.
It still provides run_direct as its own workaround for generic Acquire Send
failure. QueryBuilder removes its argument lifetime. The supported AssertSqlSafe
marks the existing test-only DDL built exclusively from four literal SQLSTATEs.
Actual normal/build tree and full advisory/license verification are required.

https://github.com/transact-rs/sqlx/issues/3196 and
https://github.com/rust-lang/cargo/issues/13078 document weak optional features
resolving unused driver families in lockfiles. No new security ignore was added.
The0.8 migration-feature alternative fails our actual CargoAudit gate on RSA.
The0.9 source options/mod.rs uses whoami2 then anonymous fallback; parser/parse.rs
explicitly supports and tests the user URL query parameter. Supply the intended
role in native test and CI URLs. Preserve library-owned connection semantics.

2026-10-18 SQLx0.9 source, native object scope and session ownership:
https://github.com/transact-rs/sqlx/blob/v0.9.0/sqlx-postgres/src/error.rs
schema() reads backend field s; supported DatabaseError::try_downcast_ref in
sqlx-core/src/error.rs preserves native fields without parsing display text.
https://github.com/transact-rs/sqlx/blob/v0.9.0/sqlx-postgres/src/options/mod.rs
PGUSER bypasses whoami2 at construction; URL username overrides occur later.
Native traces show sporadic Darwin setup tails; attribution still needs timing.
https://github.com/transact-rs/sqlx/blob/v0.9.0/sqlx-core/src/transaction.rs
and pool/maybe.rs: public doc-hidden Transaction::begin accepts owned
PoolConnection through From. For PG7, preset supported close_on_drop then use
owned transaction rather than actor/manual transaction framework. Verify this
candidate against public APIs and native expiry/cancellation before retention;
doc-hidden constructor is a pinned-version maintenance limitation.

PG6 preparation, accessed2026-10-18: PostgreSQL18 native row-lock/visibility
contracts read at https://www.postgresql.org/docs/18/explicit-locking.html and
https://www.postgresql.org/docs/18/transaction-iso.html. FOR NO KEY UPDATE is
self-conflicting while compatible with KEY SHARE, a candidate for serializing
checkpoint creation on an existing raw-key catalog row without blocking event
foreign-key checks. READ COMMITTED statements obtain fresh statement snapshots;
REPEATABLE READ/SERIALIZABLE can reject changed rows with40001 instead. This is
candidate protocol evidence, not runtime checkpoint acceptance. Use owning
CheckpointWrite::next_revision under the chosen lock and retain source errors;
verify real creation/CAS races and native key-share overlap before retaining.

PG6b,2026-10-18: locked SQLx0.9.0 registry source
sqlx-macros-core/src/derives/row.rs (match attributes.flatten aroundline100)
implements supported #[sqlx(flatten)] via nested FromRow. Use it for shared raw
checkpoint metadata on read/lock paths instead of dummy body/size sentinel fields.
Primary source https://github.com/transact-rs/sqlx/blob/v0.9.0/sqlx-macros-core/src/derives/row.rs.
Static INSERT/UPDATE SQL literals satisfy0.9 SqlSafeStr; no dynamic SQL escape.
Existing CheckpointWrite::next_revision remains sole validation policy, invoked
again only after a zero-row write observes a different actual record. Native
RETURNING optional distinguishes zero rows; never acknowledge a write that
returned no row, even if owner revision validation permits a fresh attempt.

2026-10-18 PG6b inherited-table falsifier: PostgreSQL17 official inheritance
contract https://www.postgresql.org/docs/17/ddl-inherit.html (also18 current).
Parent queries include descendants; primary/unique/FK constraints do not apply
across descendants. Relevant to checkpoint exact-one-row profile; native test
creates duplicate raw key with different state in child. Supported pg_inherits
catalog is the existing schema-validation mechanism; no new dependency/license
or runtime cost on ordinary reads/writes. PG17.10 native witness below determines
whether the current adapter actually rejects this unsafe shape.

2026-10-18 PG7a native membership correction: official PG17
https://www.postgresql.org/docs/17/functions-info.html#FUNCTIONS-PG-SNAPSHOT,
pg_current_snapshot/pg_visible_in_snapshot operate on top-level64bitxid8 and
frozen active-xid list; pg_current_xact_id returns top-level ID even in subtx.
Existing adapter stores that exact top-level function default. Native witness
xmin-only omitted committed higher-xid ID behind unrelated active writer (red);
full pg_snapshot filter passes exact membership and active/future exclusion.
No extension/crate/license/lock cost. Snapshot string cost depends on concurrent
active transaction list, not event backlog; measure in required PG8 capacity.

2026-10-18 PG7b active-query cleanup primary research:
- PostgreSQL17 client_connection_check_interval official Connections section:
  https://www.postgresql.org/docs/17/runtime-config-connection.html#GUC-CLIENT-CONNECTION-CHECK-INTERVAL
  periodic socket checks during active queries on Linux/macOS/BSD/illumos;
  detects kernel-reported closure, NOT arbitrary network/proxy failure.
- Exact17 server implementation REL_17_STABLE/src/backend/tcop/postgres.c,
  ProcessInterrupts/pq_check_connection: check timer armed during query, interrupts
  lock waits; matching src/backend/libpq/pqcomm.c checks socket readiness.
  https://github.com/postgres/postgres/blob/REL_17_STABLE/src/backend/tcop/postgres.c
  https://github.com/postgres/postgres/blob/REL_17_STABLE/src/backend/libpq/pqcomm.c
  Local exact fetched source /tmp/mnesis-pg7b-server-postgres.c and-pqcomm.c.
- SQLx0.9 pool source Pool::close_event is supported and independently waitable.
  No public cancel-query/token in selected sqlx-postgres0.9 source, no dependency
  or private wire protocol justified. Separate cleanup lease would cost2leases/
  session and require atomic admission absent in existing pool; reject speculative
  redesign before simpler supported socket-check witness.
Candidate: LOCAL1s check interval, preserve existing shorter nonzero setting via
pg_settings normalized milliseconds. It limits disconnect-detection latency, NOT
query execution time. Same one data lease/task; no change to lifetime range or
write timeouts. Original5s native cleanup witness decides fitness. PG8 still
must test network loss/TLS/pool resource behavior; local success not partition proof.
Transaction_timeout researched alternative terminates native session, but17int32
milliseconds restricts lifetime domain and doesn't independently make early Drop
cleanup prompt; not needed for first socket-check experiment.
2026-10-18 release repair primary sources:
https://release-plz.dev/docs/github/output (PR output and supported post-update
commit workflow); https://raw.githubusercontent.com/release-plz/action/v0.5/action.yml
(actual prs_created false +pr{} when none); https://docs.github.com/en/graphql/reference/commits
(server-signed createCommitOnBranch/expectedHeadOid; live schema confirmsbranchName).
Cargo own selected1.99 --locked resolution and targeted -p updates used; no lock
relaxation in CI. Existing Cesr App/signing pattern inspected, release-plz current
release workflow kept owner. Native GitHub STALE_DATA control returnsnull/nochange.
https://codspeed.io/docs/instruments/cpu/regression-causes and /reducing-variance
(official primary explanation, hardware/layout are hypotheses, not proof here).
Actual public report https://app.codspeed.io/devrandom-labs/mnesis/branches/release-plz-2026-10-18T07-25-51Z
confirms EPYC7763vs9V74 with equal reported flags; compiler/tool/image logs match.
No performance gate change based only on that warning. Traces or controlled
same-environment evidence still pending.
CodSpeed shipped primary client /_next/static/chunks/pages/_app-4791e6a5e5fedc20.js
FindBenchmarkCallGraph uses repository(...provider:GITHUB).benchmarkResultById
callGraphPresignedUrl, public readonly https://gql.codspeed.io. Actual v3 node/
edge self times reconcile501.6667=>560ns exactly. Full large-payload traces kept
as evidence; no credentials/expiring download URLs retained in project.
2026-10-18 actual native history capability: PG17.10 pg_catalog.pg_control_system
returns system_identifier, and pg_database current-database OID is readable by a
new NOLOGIN role with NO additional grants. Actual BEGIN/CREATE ROLE/SET LOCAL/
query/ROLLBACK succeeds; no role retained. PostgreSQL18 primary docs
https://www.postgresql.org/docs/18/functions-info.html#FUNCTIONS-PG-CONTROL
identify system_identifier as server-wide unique cluster identifier. This is a
candidate native context source, not yet a chosen PgAllPos/API/migration contract.
Do not assume all control-data functions are privileged or add unnecessary UUID/
random/runtime dependencies. Need actual18.6 role/physical/logical/reopen evidence,
compatibility and size/complexity decision before retaining history-context API.
2026-10-18 PG9 decoder performance primary evidence:
Rust Reference https://doc.rust-lang.org/reference/attributes/codegen.html#the-inline-attribute
states inline is a compiler hint, may be ignored, and poor choices can slow code.
No always hint or blanket attribution justified. Actual pinned Rust1.99.0,
codspeed-criterion-compat5.0.1 benchmark owner unchanged. CodSpeed primary client
FindBenchmarkCallGraph retrieves all3payload profiles: all base/head35ns total
instruction contribution and501.6667/560ns total from one extra L1/LL cache-miss
pair. Source cost, binary size, native runtime and x86 CI evidence required.
2026-10-18 selected history facilities: PostgreSQL18 UUID functions official
https://www.postgresql.org/docs/18/functions-uuid.html documents built-in v4
random generator; pg_control_system/pg_database native17+18 no-grant proofs in
PG7c evidence. SQLx0.9 locked src/types/bytes.rs explicitly implements Decode
and Encode<Postgres> for [u8;N], retaining binary wrong-length decode cause;
no custom UUID parser or crate needed. STABILITY.md0.x tier permits breaking
0.(x+1) documented releases. No shared-kernel type/API change selected.
2026-10-18 PG7c Linux fixture followup primary source:
https://www.postgresql.org/docs/18/functions-admin.html#FUNCTIONS-ADMIN-SIGNAL
pg_terminate_backend default timeout0 true proves signal sent, not completed
termination or delivery of a server SQLSTATE to a client whose next write races
socket close. Actual Linux red retains SQLx0.9 Io(BrokenPipe32). Reuse native
pg_stat_activity Lock observation to establish query in flight before termination;
then require exact original57P01 rather than broaden error acceptance. No adapter
cause mapping or runtime change. Keep raw175/1/35UNRUN Linux evidence.
# Global read search path, accessed 2026-10-18

PostgreSQL 18 official schema documentation:
https://www.postgresql.org/docs/18/ddl-schemas.html#DDL-SCHEMAS-PATH
and https://www.postgresql.org/docs/18/typeconv-oper.html.
An explicitly ordered pg_catalog may follow caller schemas. Functions and
operators follow search-path resolution; OPERATOR(pg_catalog.<) is supported
qualified syntax. Applicability: SQLx caller pools retain their configured
search path. Native tests must establish actual adapter impact before a fix.
# Physical recovery controls, accessed2026-10-18

https://www.postgresql.org/docs/18/app-pgverifybackup.html: native verification
checks backup_manifest, system identity, expected/missing files, checksums and
required WAL. Pin the tool version for WAL parsing; do not skip WAL/checksums.
Verification is additional evidence, not a replacement for an actual test restore.

https://www.postgresql.org/docs/18/app-pg-ctl.html documents waited shutdown,
fast-mode rollback/disconnect and status3 when the server is not running
(status4 instead means an inaccessible/missing data directory).
https://www.postgresql.org/docs/18/app-pgbasebackup.html documents a running
cluster's physical base backup with streamed WAL. Applicability: pinned18.6
native fixture tools; assert outcomes, retain fsync-on profile, do not infer
cross-node fencing or power-loss survival from this local controlled drill.

Pinned PostgreSQL18.6 source for exact native file-integrity oracles:
https://raw.githubusercontent.com/postgres/postgres/REL_18_6/src/bin/pg_verifybackup/pg_verifybackup.c
lines979–981 specify manifest-present-but-not-on-disk,1106–1114 checksum mismatch.
Explicit --format=plain selects the actual pg_basebackup format even when its
PG_VERSION marker is absent. Native current tests check both exact diagnostics
and repaired success; exit1 alone previously accepted the wrong tar/WAL error.

2026-10-19 decoder performance: actual current PR392 check summary
/tmp/mnesis-pr392-performance.json and releasefe0fbee check summary retain same
three10.42%regressions plus different-runtime-environments warning; not a waived
comparison. Existing primary CodSpeed callgraph evidence and rejected inline
experiment live in preserved /tmp/mnesis-release-performance research/evidence.
Actual current crates/store/src/wire.rs: decode_frame_body has only one caller;
header-size selection is already derivable from passed FrameHeader. Moving it
changes no parsing rule, but performance must be measured, not inferred.

2026-10-19 primary CodSpeed docs: https://codspeed.io/docs/instruments/cpu,
https://codspeed.io/docs/instruments/cpu/regression-causes and
https://codspeed.io/docs/instruments/cpu/reducing-variance. Weighted executed
instructions plus simulated L1/LLmiss costs; possible function/code layout and
runner variance. Applicability: explains evidence to inspect, NOT actual cause
or waiver. Keep exact existing benchmarks/gates unchanged in this experiment.
Exact installed codspeed-criterion-compat-walltime5.0.1 src/lib.rs835+ supports
--save-baseline/--baseline; used named existing-tool baselines, no custom tooling.

2026-10-19 Rust Reference https://doc.rust-lang.org/reference/attributes/codegen.html#the-cold-attribute:
cold suggests a function is infrequently called; compiler may use that fact, no
performance guarantee. Applicability limited to actual overflow-error constructor,
never marking the normal decoder cold. Existing std Option::ok_or_else/Result
map_err preserve fallible construction; actual generated/runtime cost still due.

2026-10-19 current parent49104117 source: store.rs read_key SELECT number WHERE
value=$1 and read_head SELECT MAX(version)...WHERE value=$1, unqualified. Prior
PG388 already reproduced operator/function search_path redirection for global
ordering. Query identity risk remains unproven until current native witness.
Use existing SQLx/PgPool and native CREATE OPERATOR facilities; no new crate or
protocol/control framework. Native protected profile/caller pool is the owner.

2026-10-19 PostgreSQL18 official current primary documents inspected:
https://www.postgresql.org/docs/18/typeconv-oper.html operator resolution1/2:
identical operand types use earliest search-path operator; qualified OPERATOR
restricts candidates to that schema. Exact typed bytea inputs avoid unknown-type
ambiguity. https://www.postgresql.org/docs/18/typeconv-func.html function
resolution1 likewise chooses earliest identical argument signature; qualify
numeric aggregate pg_catalog.max. https://www.postgresql.org/docs/18/ddl-schemas.html
explicit pg_catalog placement permits earlier application functions/operators.
Native current17.10 witnesses establish applicability with actual SQLx0.9 pools;
primary docs alone are not runtime evidence. Keep caller settings unchanged.

2026-10-19 PG9 preparation (no implementation): locked Crane47b6b27ed9a3a9181415e4367d0c30ab2a0e0250
at /nix/store/44k6cm6lbjpfq3xnl9wg0jpqa5rpsvmf-source/lib/{cargoClippy,cargoNextest,mkCargoDerivation}.nix
and https://crane.dev/API.html confirm doInstallCargoArtifacts controls output
archiving without selecting tests; actual check consumers still need verification.
Cargo-hack https://github.com/taiki-e/cargo-hack#--no-dev-deps explicitly warns
that no-dev-deps mutates real manifests temporarily. Locked Nixpkgs provides0.6.45,
MIT/Apache-2.0, so evaluate only in an isolated source tree, not an active index.
Cargo metadata format remains owned by https://doc.rust-lang.org/cargo/commands/cargo-metadata.html.
RustSec https://rustsec.org/advisories/RUSTSEC-2026-0221.html and upstream
https://github.com/smol-rs/event-listener/releases/tag/v5.4.2 confirm the existing
5.4.1 advisory is patched by5.4.2. Existing ignore is not production acceptance;
compatibility/no_std/dependency/lockfile evidence is still required before update.

# PG8 TLS pre-read — accessed2026-10-18

https://docs.rs/sqlx/0.9.0/sqlx/#tls-support documents supported native-TLS and
Rustls providers; current exact installed sqlx0.9/Cargo.toml and sqlx-core0.9/
Cargo.toml own feature spellings: tls-rustls-ring-webpki,
tls-rustls-ring-native-roots, tls-rustls-aws-lc-rs, tls-native-tls. Rustls optional
requirement0.23.24 with std+tls12 and provider chosen separately. Current root
lock contains no Rustls/ring/native-TLS; no backend selected yet. Exact installed
sqlx-postgres0.9/src/options/{mod,ssl_mode,parse}.rs documents defaultPrefer and
VerifyFull. Two docs.rs PgConnectOptions/PgSslMode URLs failed to open; local
locked source is authoritative. Need real native VerifyFull witness and complete
fitness/license/advisory/lock/build comparison before a provider selection.

PG8 provider decision primary sources, accessed2026-10-18:
https://github.com/rustls/rustls/security (maintenance/MSRV policy),
https://docs.rs/rustls/0.23.45/rustls/ (provider/platform/feature description),
https://github.com/rustls/rustls/releases/tag/v%2F0.23.45 (Sept handshake fix),
https://rustsec.org/packages/rustls.html and advisories/RUSTSEC-2026-0346.html,
0347.html,0348.html,0349.html (Oct14 patches>=0.23.46).
Cached releases search incorrectly still calls0.23.45 latest. Live official
GitHub API repos/rustls/rustls/releases/latest instead returnsv/0.23.46,
published2026-10-14T09:51:22Z, fixing those four advisories. Do not choose stale45.
https://rustsec.org/advisories/RUSTSEC-2025-0009.html requiresring>=0.17.12;
https://rustsec.org/packages/ring.html marks only pre0.17 unmaintained.
Exact SQLx0.9 src/net/tls/tls_rustls.rs imports built-in roots, appends configured
root certs, and uses normal certificate+name verification for VerifyFull; no
custom verifier selected. Alternatives/native trust and dependency costs in HYP.
Runtime performance/build/license/audit facts still require actual resolved lock
and commands, not extrapolation from upstream documentation.

Resolved selection correction: webpki-roots1.0.9 exact Cargo.toml license is
CDLA-Permissive-2.0 (not initial assumedMPL). Actual cargo-deny rejects it under
existing policy. No policy changes; supported native-roots alternative resolves
rustls-native-certs0.8.4 (Apache2/ISC/MIT), SecurityFramework3.7.0/sys2.17.0 and
core-foundation0.10.1 (MIT/Apache2), plus platform support. Exact installed crate
Cargo.tomls show MSRV<=1.85; Rustls0.23.46 MSRV1.71, ring0.17.14 MSRV1.66.
Ring LICENSE explicitly refers current Apache2/ISC BoringSSL plus once_cell;
existing deny policy passed without clarification changes. Native trust is
OS-provided roots plus callerCA through SQLx; no CA-file-only guarantee. Actual
TLS1.2/1.3/provider choice remains supported SQLx Rustls/ring; no custom verifier.

## PG8 durability profile research — accessed2026-10-19

PostgreSQL18 official https://www.postgresql.org/docs/18/runtime-config-wal.html
synchronous_commit table: all non-off modes wait for local flush; off can report
success before crash safety. Replication meanings differ and must be preserved.
fsync/full_page_writes off can compromise crash recovery independently; turning
fsync back on requires proper buffer synchronization. Native tools/settings and
actual restart evidence still required, not inferred from these specifications.
https://www.postgresql.org/docs/18/wal-reliability.html documents storage honesty;
https://www.postgresql.org/docs/18/sql-set.html defines transaction-local settings.
Locked SQLx0.9 PgConnectOptions::options supports startup GUCs; pool queries share
per-connection state. Actual adapter builder validates schema only; store.rs,
atomic.rs,snapshot.rs,checkpoint.rs separately begin write transactions. Initial
source finding remains unproven until real adapter witness. No maintained crate
needed: existing SQLx transaction and native Postgres GUC controls are the owners.

Accessed2026-10-19 PostgreSQL18 primary reference:
https://www.postgresql.org/docs/18/sql-set-constraints.html retroactive pending
constraint-trigger execution when switching DEFERRED to IMMEDIATE; ALL applies
inside the current transaction, no session-wide setting override.
https://www.postgresql.org/docs/18/sql-createtrigger.html distinguishes ordinary
AFTER and DEFERRABLE INITIALLY DEFERRED constraint triggers at COMMIT. Applicability
confirmed by current native original red and unchanged-oracle fixed10law run.

2026-10-19 durability integration: https://www.postgresql.org/docs/18/sql-set-constraints.html
states switching deferred constraints to IMMEDIATE retroactively executes outstanding
checks; failure rejects the SET command. https://www.postgresql.org/docs/18/errcodes-appendix.html
names40003 statement_completion_unknown distinctly from40001 serialization_failure
and40P01 deadlock_detected. SQLx0.9 transaction API
https://docs.rs/sqlx/0.9.0/sqlx/struct.Transaction.html retains owned commit/rollback
facilities. Moving deferred checks does not justify weakening the established
conservative completion-unknown finalization policy; actual phase and independent
row settlement remain separate evidence.

The docs.rs SQLx0.9 Transaction fetch above returned an internal error. Actual API
proof inspected locally at Cargo's locked sqlx-core-0.9.0/src/transaction.rs:
commit consumes self and delegates to TransactionManager before clearing open;
rollback and Drop facilities retain ownership. Do not label the failed web fetch
as a successful source read. PostgreSQL18 SET CONSTRAINTS/error-code sources did
load successfully; native phase/failure evidence remains authoritative here.

2026-10-19 phase audit: PostgreSQL18 SET CONSTRAINTS
https://www.postgresql.org/docs/18/sql-set-constraints.html explicitly makes
pending deferred checks retroactive inside SET CONSTRAINTS; constraint triggers
fire there too. PostgreSQL18 protocol flow
https://www.postgresql.org/docs/18/protocol-flow.html simple-query cycle keeps
actual frontend Query and backend ErrorResponse boundaries observable in the
existing private Unix relay. SQLx0.9 adapter calls transaction.commit only
AFTER drain+validation; source proves no own COMMIT on preceding error. Native
SQLSTATE alone cannot replace that known phase. Injected DO error at client
COMMIT boundary will not be claimed as actual server COMMIT/SSI/deadlock.
