# Hypotheses

The first H1/H2 read and wake experiments pass native regressions and the full
Darwin gate; their remaining feature acceptance is open. H3–H5 are proposed.
Their falsifiers and rollback
boundaries are in the [PRD hypothesis table](../docs/prds/postgres.md#hypotheses-and-falsifiers).
Associated workloads and observable laws are in PG0–PG8. Start with real-database
reproduction, not production refactoring. Record each experiment before editing.

Fresh audit follow-ups: PG4 must measure incompressible long binary keys against
Postgres's B-tree tuple limit; the 5,000-byte notification witness is compressible
and does not establish arbitrary-key capacity. Record practical accepted limits
or a native representation that preserves equality without probabilistic identity.
Do not silently substitute a hash for the durable stream identity. PG9 must
assess compatible patched event-listener >=5.4.2 rather than ignore the advisory.

Native PG4 probe now rejects a5,120-byte incompressible key in the legacy B-tree,
while EXCLUDE USING hash(key WITH =) accepts it and rejects duplicates. Actual
hash-colliding distinct keys survive equality recheck. This supported native
constraint supersedes a handcrafted advisory-lock identity protocol as the
preferred candidate; full adapter/migration/concurrency fitness still untested.

PG8: a transaction in another database on the same Postgres cluster can pin the
safe $all watermark. Native22861→22862 probe demonstrates this. Require an actual
adapter subscription recovery witness after a foreign-DB transaction ends with
no later writes, and document cluster-level long-transaction limits/diagnostics.
Never run independent native suites concurrently on one cluster; database-only
fixture separation does not isolate snapshot xmin. No unsafe shortcut authorized.
