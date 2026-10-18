# Dead ends

No production experiment has been rejected or reverted. Initial read/wake
experiments passed and are committed; later feature acceptance remains open.

Do not use `PgPool::size() == 0` as proof that listener setup ended. SQLx counts
in-flight connection attempts, and this assertion failed in the lifecycle
fixture despite no established LISTEN session. The retained fixture observes
Postgres LISTEN backends and the configured acquisition deadline instead.

- PG1 byte-edge test initially attempted present-but-empty metadata. The kernel's
  `Metadata` constructor rejects it (`MetadataEmpty`), so this is not an accepted
  envelope state. The retained test covers absent versus one-byte metadata;
  stored invalid metadata remains an error rather than weakening that contract.

PG7a xmin-only ordinary stream listing REJECTED after initial5green native laws.
A real held lower-xid transaction hides an already-committed higher-xid stream,
returning[] instead of one ID. Ordered read_all legitimately withholds such rows;
StreamLister must include committed streams at capture. Existing pg_snapshot+
pg_visible_in_snapshot fixes exact membership without pinning connection; canonical
pg_catalog function/type names also required after separate native shadow-function
and fake-snapshot witnesses each returned[]. All8fixed laws pass. Do not retry
xmin-only or unqualified native functions based on earlierfive-case pass.
2026-10-18 release decoder #[inline] experiment rejected, actual same3original
native benchmarks regress (large+25.845%..29.417%) with one line. Compiler hints
are not automatic improvements. Revert, never retain solely for CodSpeed green.
Original failed profile:1additional L1/LLmiss, identical instruction contribution;
cache/hardware/layout cause still distinct from causal fix. Raw candidate/base
logs retained in docs/prds/postgres/evidence/pg9-release-performance-*.log.
