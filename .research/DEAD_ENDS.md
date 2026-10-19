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

2026-10-19: moving header-size choice from decode_frame into the only body decoder
(+6/-10Rustlines, no interface/workload/gate changes) rejected. Existing native
three-workload candidate2.705–2.727ns; saved-original actual --bench A/B/A returns
roughly2.5–2.64ns. Small/medium regress beyond their baseline interval variation;
whole binary size unchanged6704128. Exact logs /tmp/mnesis-decoder-{parent-first,
parent-second,candidate-first,parent-return-measured}.log, source -body-candidate.rs.
Reverted only source, gitdiff0. Do not retry to chase cold simulation green. Earlier
standalone invocation without --bench was smoke only and gets NO timing credit.

2026-10-19: shared #[cold] OffsetOverflow factory plus lazy construction rejected
(+16/-33Rustlines). Actual same3native workloads report +2.88..5.55/+3.29..6.46/
+1.48..5.56% vs current saved-parent control, no improvement. LLVM same function
address/file size but changed branches, no emitted independent factory; do not
claim code-equivalence from failed symbol-only empty dumps. Exact address dumps
and candidate source/binary retained /tmp/mnesis-decoder-*. Source reverted only
own file, gitdiff0. A shorter source or potential x86 layout shift is not evidence.
