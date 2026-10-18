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
