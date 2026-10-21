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
characterization. Paired allocation/RSS scaling and generic-plan diagnostics
are now recorded there; release latency/throughput distributions remain pending.

## Rust surface

At baseline there were no production, dependency or public API changes.
Postgres exported
`PostgresStore`, `PostgresError` and `PgAllPos`; their existing capabilities and
gaps are listed in the PRD. Per-slice and cumulative budgets belong there.

## Command processor — 2026-10-21

The Postgres material above is historical. Current source identity, dependency
inventory, 27-test archived baseline and optimized duplicate measurements are
owned by [command research](../docs/prds/command/research.md#executed-baseline).
CP0's frozen source and executable consumers are recorded in its qualification
section and evidence bundles. CP1's 32-test debug/release results are separate
from the archived baseline; no real-database or new performance claim follows.
The [command PRD](../docs/prds/command.md#cp6-qualify-adapters-recovery-and-performance)
owns required backend/capacity measurements; these remain pending.
