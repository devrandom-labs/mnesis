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

2026-10-19 byte accounting: PostgreSQL18 function resolution
https://www.postgresql.org/docs/18/typeconv-func.html says identical signature
functions select the first visible schema, while qualified names constrain the
schema. https://www.postgresql.org/docs/18/ddl-schemas.html documents explicit
pg_catalog placement in search_path. Native locked17.10 original failures verify
actual applicability here; ordinary trusted application functions suffice, not
an assumed malicious-input or arbitrarycode-execution claim. Preserve caller
settings and qualify the adapter's own required native accounting calls.
# PG9 patched dependency sources — accessed 2026-10-19

- https://rustsec.org/advisories/RUSTSEC-2026-0221.html: affected StackSlot permits
  non-Send tags across threads; patched >=5.4.2. Official release
  https://github.com/smol-rs/event-listener/releases/tag/v5.4.2 names this fix.
  Exact downloaded5.4.2 Cargo.toml: Rust1.60, edition2021, MIT OR Apache-2.0,
  default std optional. Existing Rust1.99 and core+alloc wake ownership fit.
- Exact5.4.2 src/lib.rs listen eagerly inserts the listener; notify uses the
  existing notification fence. StackSlot Send/Sync now require T:Send. The
  existing eventcount generation proof still requires its own actual tests and
  bare-metal build; upstream statements alone do not establish adapter behavior.
- https://rustsec.org/advisories/RUSTSEC-2026-0009.html: patched time>=0.3.47.
  Current root/fuzz/fuzz-afl locks contain neither time nor refinery. Their old
  exception comments describe an absent graph, not an evidenced current blocker.
# PG4 metadata preparation — accessed2026-10-19

Exact locked sqlx-core0.9.0/src/migrate/migration.rs computes embedded checksums
with Sha384::digest and exposes their bytes through each Migration. Derive bounds
from those embedded checksums rather than duplicating the current48-byte size.
PostgreSQL18 official binary string documentation
https://www.postgresql.org/docs/18/functions-binarystring.html documents bytea
substr with an explicit length. Exact17.10 applicability needs native execution.
Official SQLx0.9 Migrator docs page failed to open; use the actual locked source,
not inferred API behavior. Existing native migration laws own checksum/future
version rejection, restoration and runtime privileges. No proof claimed yet for
the new allocation hypothesis; old count/checksum-error laws alone do not bound
transfer or memory.

### PG8 configured deadlines, accessed 2026-10-19

- PostgreSQL18 client connection defaults:
  https://www.postgresql.org/docs/18/runtime-config-client.html .
  statement_timeout bounds a statement at the server; lock_timeout bounds each
  lock acquisition. Set per owning session/role, not a new adapter timer policy.
  Zero disables either. A client watchdog does not establish rollback.
- Locked local SQLx0.9 source, sqlx-core/src/pool/options.rs:249–263:
  acquire_timeout caps permit wait, connection checks/hooks and new handshakes.
  sqlx-postgres/src/options/mod.rs:425–455: options supplies native startup GUCs.
  Online docs.rs pages for these exact APIs returned internal errors; local
  locked source is authoritative. Existing dependencies suffice, no new crate.
- Existing adapters/postgres/tests/lifecycle_tests.rs and support/cluster.rs own
  listener observation, pool-close and native-server lifetimes. Reuse these
  established patterns; correctness timings here are not capacity measurements.

### PG8 diagnostics owners, accessed 2026-10-19

Locked tracing0.1.44 and subscriber0.3.23 are existing workspace dependencies;
crates/store's execute/repository/snapshot owners use skip_all async instrument
spans. Reuse those facilities, not a metrics/tracing framework. Locked
sqlx-postgres0.9/src/listener.rs:226–307 implements recv as repeated try_recv;
try_recv returns None on supported disconnects and eagerly reconnects by default.
Observing None therefore preserves SQLx's own reconnection behavior while making
it visible. Other errors still reach the existing bounded retry loop. Native
query statistics remain Postgres' responsibility, not a new adapter registry.
### PG8 soak facility selection, accessed2026-10-19

Existing adapters/fjall/benches/projection_storage.rs and its [[bench]] demonstrate
standalone Cargo reporting benchmarks. .github/workflows/codspeed.yml explicitly
selects four existing Criterion targets; a native reporting benchmark is not
silently substituted into CPU simulation. Reuse standard Cargo/Tokio/SQLx; no new
benchmark framework or custom development tool. Existing Postgres cluster.rs
owns initdb/pg_ctl lifecycle; memory.rs already exposes LIVE/PEAK/TOTAL allocated
bytes. Existing native projection consumers verify the checkpoint API, and
crates/store/src/checkpoint.rs owns revision CAS/state-position pairing.

Host baseline: Mac16,7;24GiB RAM;14 logical CPUs, observed via sysctl. This private
native soak is not an isolated throughput or latency measurement. Actual DB
settings/toolchain/source/elapsed will be recorded at execution; short smoke
runs and concurrent-host timings cannot satisfy the full capacity matrix.

PG9 feature facilities checked2026-10-19: locked Nixpkgs
c59305bab2065cfecc4944690d9eedbb56f3a9fa cargo-hack0.6.45 MIT/Apache-2.0,
https://github.com/taiki-e/cargo-hack documents --each-feature includes no/default/
all cases and --no-dev-deps prevents Cargo dev-dependency feature leakage by
modifying then restoring real manifests. Use disposable checkouts only.
https://doc.rust-lang.org/cargo/commands/cargo-metadata.html documents explicit
format-version1, resolved package versions, --locked; use Cargo's parser rather
than another TOML parser for the AFL lock version. Existing workspace archive,
Postgres test hook and checks workflow own execution and fail-closed verdicts.

PG9 AFL verdict sources verified2026-10-19:
https://github.com/AFLplusplus/AFLplusplus/blob/stable/docs/fuzzing_in_depth.md
(native output/crash triage and bounded CI campaigns); existing pinned afl.rs
harness/CI owns target versions and CMPLOG. GitHub workflow shell reference
https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax#jobsjob_idstepsshell
states unspecified Linux Bash uses bash-e, explicit Bash adds pipefail. Existing
AFL run does not specify either shell or pipefail; preserve producer failure
explicitly in the existing step. Bash arrays/glob syntax is already used in CI.
Cargo metadata format-version1 supports locked resolved package versions and
can later remove the inline Python parser; do not add a second TOML parser.
https://doc.rust-lang.org/cargo/commands/cargo-metadata.html

Locked nixpkgs c59305bab2065cfecc4944690d9eedbb56f3a9fa exposes actionlint1.7.12
MIT and cargo-hack0.6.45 MIT/Apache-2.0. Actionlint used as a one-off supported
workflow inspection tool via locked --inputs-from; no project wrapper/dependency
or permanent check introduced. Its packaged ShellCheck/pyflakes dependencies are
Nix-owned tooling, not project Python. Current fuzz.yml passes actionlint with
zero diagnostics; this is workflow static analysis, zero engine execution.

Native AFL control cannot run on this Darwin host through the selected supported
Nix package: locked aflplusplus5.00c declares x86_64-linux/i686-linux only (not
Darwin). Do not force unsupported builds or invent a local engine. Real bounded
saved-crash control belongs on the existing Linux AFL workflow after the normal
signed gate. No platform limitation is a reason to mark the goal blocked.

2026-10-20 actual upload-artifact@v4 resolved commit
ea165f8d65b6e75b540449e92b4886f43607fa02, actual controlled job rejects native
AFL colon filenames. Selected action README recommends tar-before-upload to
preserve paths/metadata:
https://github.com/actions/upload-artifact/blob/ea165f8d65b6e75b540449e92b4886f43607fa02/README.md#permission-loss
Applicability: standard tar/gzip packages original worker directories and exact
crash bytes under portable artifact filename. No action version, framework,
runner, harness dependency, compression threshold or crash verdict change.
Current upstream default README is v7; selectedv4 pinned README, actual log and
supported ordinary archive upload are the relevant evidence, not v7-only inputs.

### PG8 capacity measurement owners, accessed2026-10-19

Reuse existing Cargo standalone reporting benchmark pattern, Rust std Vec sorting
and Duration/Instant, Tokio Barrier/JoinSet, SQLx0.9 and native cluster/memory
fixtures. No new package or generic harness. Read actual store/page SQL and
position source for exact bounds; inspect server-owned prepared statements for
EXPLAIN instead of maintaining a copied page-query implementation. Supported
Postgres EXPLAIN EXECUTE and force_generic_plan expose actual bound plans.
Capture native version/durability settings, sample denominators and raw output.

Use the Unix OS entropy device once per fixture to create32/4096-byte templates,
recording exact template bytes in the raw evidence. Reuse each template unchanged
within all repeated windows; per-row ordinals replace only the first8bytes.
Random-looking bodies avoid a zero-filled, trivially TOAST-compressible large
payload profile. No new RNG dependency or handwritten generator. This is synthetic
local capacity, not a production payload-distribution or cross-node guarantee.
Successful allocation calls extend the existing counter owner by2lines; byte
traffic and live/peak/retained bytes remain separately reported. OS ps RSS reports
process residency, not Rust allocator bytes; external native time can record
whole-run peak RSS. Report these boundaries and counter/validation overhead.

Primary facilities verified2026-10-19:
- https://www.postgresql.org/docs/18/sql-explain.html: ANALYZE executes the
  prepared statement; BUFFERS reports actual buffer work. Instrumentation and
  discarded result transfer mean EXPLAIN is diagnostic, not client throughput.
- https://www.postgresql.org/docs/18/view-pg-prepared-statements.html: prepared
  statements are visible only in their creating session. Inspect each leased
  pool connection, quote names with native quote_ident and reuse actual SQLx SQL.
- https://www.postgresql.org/docs/18/sql-prepare.html: force_generic_plan selects
  a parameter-independent plan; EXPLAIN EXECUTE displays it. Reset the setting
  after diagnostic replay so measured application operations retain defaults.

## Command processor planning — 2026-10-21

Canonical source assessment, exact locks, alternatives and source limitations:
[command research](../docs/prds/command/research.md#primary-sources-and-alternatives).
Inputs: untracked Labcast controller fingerprint manifest SHA-256
1e4be80cc62b9c814793456cea53baf5fae1d583d060dfb787ef3adac5fdff6a;
Mnesis27de3b4a4aad59909ffaccdf6fa83e8b256ccca6. Primary references include Axon
command dispatching, Brighter's implementation repository, Rust cqrs-es0.5.0,
Tokio1.53.1 docs/locked source, and Stripe's explicit idempotency contract.
They establish terminology/alternative semantics, not imported dependencies.
The existing Mnesis contracts remain the implementation authority. Detailed
findings, source paths and fresh baseline logs live once in the feature record.

Tower follow-up, accessed 2026-10-21: published tower-actor 0.2.0 and its recorded
commit 6416fa4b1165356cf06ffb4d75851725734b2a21; source, actual dependency graph,
Mnesis 0.4/0.5.3 executions and limitations in docs/prds/command/tower.md.
Tower Service/tracing/metrics/OpenTelemetry selection: exact existing pins,
primary documentation and remaining qualification in the command research record,
section "Required Tower interface and observability". Accessed 2026-10-21.
CP0 primary source checks 2026-10-21: registry mnesis-command sparse-index404;
metrics0.24.6 downloaded manifest MIT/Rust1.71.1, actual optional graph includes
rapidhash4.5.1; upstream commit043577f9914223e3e2d7165d5361e7e78ad75ed2 dated
2026-10-16. Tokio-util0.7.20 PollSender's locked source and Tower0.5.3 contract
establish handle-owned readiness reservations. Detailed decisions/evidence in
command/research.md, CP0 seams. No ownership reservation or advisory claim.
2026-10-21 CP3 gate prerequisite: official Cargo environment reference
https://doc.rust-lang.org/cargo/reference/environment-variables.html#dynamic-library-paths
says macOS uses DYLD_FALLBACK_LIBRARY_PATH and extends its existing value in child
compiler/binary processes. Actual selected Rust1.99.0 compiler/driver are same
Nix toolchain; executing its hard-linked .links alias reproduces dyld abort134,
then selected toolchain/lib fallback yields0. Use existing lib.makeLibraryPath;
no dependency or global setup change. Broader store-alias cause is not proven.
CI blocker checked2026-10-21: https://rustsec.org/advisories/RUSTSEC-2026-0382.html
issued2026-10-20; live PR411 cargo-deny identifies locked rustls-webpki0.103.15.
Upstream fixed0.103.16 selected, existing requirement accepts it. Only the root
lock contains this package; isolated fuzz locks do not. No waiver/check reduction.
CI patch gate fuzz replay completed three targets2/1/1,1s each. Each invocation
reviewed for decoder contract/counterexamples, exact selection/counts and source
independence: unchanged shared parser bodies check panic freedom, not processor
persistence or sustained fuzz coverage. No ignore/filter/deadline/source change;
these are fresh executions, distinct from earlier planning records.
Crane locked source44k6cm6lbjpfq3xnl9wg0jpqa5rpsvmf-source/lib/cargoNextest.nix:
partitions1 delegates doInstallCargoArtifacts to mkCargoDerivation; its default
true exports compiled target cache. withLlvmCov writes $out/coverage separately.
Project consumers inspected: no check consumes nextest's target.tar.zst; native
Postgres has a separate required executable archive, unchanged. Use supported
false flag for nextest only. Test commands/filters and coverage output are intact.
Failed-install gate workspace review1: all1594 source-owner tests passed, including
actual model/version-conflict/corruption and lifecycle assertions; only dependency
patch changed, so command processor acceptance is not inferred. Review2:601.259s,
16slow,4 established compiler exclusions, no native database credit from guarded
Darwin cases. Review3: raw first failure is zstd install archive ENOSPC after all
tests and coverage, not a product assertion; no source/filter/workload/deadline
changed. Preserve raw failure; retry full checks using supported storage flag.
CI output-only retry failed before tests while unpacking the common Cargo cache
(ENOSPC; raw ci-fix-gate-noarchive.log). Review1: no contract assertion ran;
prior1594-pass result does not make this gate green. Review2: zero new tests,
not a filtered/guarded pass; native database unavailable on this host gate.
Review3: unchanged source/test command; failure is dependency extraction, not an
assertion. No new runtime credit. Next retry serializes Nix derivations using
--max-jobs 1 and removes only this goal's generated CP1 target cache (2.3GiB),
whose debug/release logs are retained. No global GC or other project cleanup.
Serial gate's three fresh fuzz invocations (replay2/1/1, each1s) reviewed after
execution: (1) existing wire/CBOR panic-freedom invariants and corpus replay are
unchanged; processor persistence/identity remain outside these bodies. (2) all
four selected cases executed, zero ignored/filtered, native DB unavailable here;
nextest is a separate still-running invocation. (3) source/filter/seed/workload
unchanged from prior failed-install run; no sustained campaign or new processor
fuzz coverage is inferred. Raw ci-fix-gate-serial.log preserves exact results.
Serial gate nextest review1: unchanged actual workspace assertions exercise store
ordering, conflicts, corruption and lifecycle; dependency patch introduces no
processor acceptance claim. Review2: run da13a0b2-c674-495d-982c-5cde7e3902bb,
1594/1594PASS489.678s14slow/138binaries, four existing compile-fail exclusions.
Darwin guarded PG cases are not live-DB evidence. Review3: no executable source,
assertion, workload, filter or deadline changed; exact lock patch and output-only
flag are independently visible in diff. Raw serial log retained. Coverage output
installed successfully with no compiled target archive. Remaining non-test Nix
checks still determine the full-gate result.
