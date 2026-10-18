# Baseline

## Environment

The [PRD dependency table](../docs/prds/postgres.md#locked-dependency-verification)
records the source revision, selected Rust/dependencies and Nixpkgs revision.
Hardware, build profile, database settings and actual original results are in
[the characterization](../docs/prds/postgres/research.md).

## Verification

Source/contract inspection and locked Postgres package evaluation are recorded
in the [PRD evidence table](../docs/prds/postgres.md#evidence-and-remaining-work).
The existing 56 real-database adapter tests pass. Four new native regression
cases fail against original production source. The characterization records
commands and separates measured retained memory from pending RSS/capacity work.
The full implementation gate commands remain in PG9.

## Measurements

Initial failure traces and single read-memory/timing samples are recorded in the
characterization. Broader allocation/RSS scaling, latency/throughput distributions
and query plans remain pending. No comparative performance claim is established.

## Rust surface

Two new native test files; no production, dependency or public API changes yet.
Baseline Postgres exports
`PostgresStore`, `PostgresError` and `PgAllPos`; their existing capabilities and
gaps are listed in the PRD. Per-slice and cumulative budgets belong there.
