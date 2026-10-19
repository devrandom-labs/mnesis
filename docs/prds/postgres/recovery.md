# Operator recovery measurements

Measured on 2026-10-19: Apple M4 Pro, 14 cores, 24 GiB RAM, macOS26.5.2,
Rust1.99.0 debug test binaries and Postgres tools18.6. These are the existing correctness
drills: two committed events in two streams, projection value4, then one new
event and value5. The source is quiesced. RPO is zero for those two events.
Run the [operator procedure](../../../adapters/postgres/README.md#operator-recovery-drill)
with deployment data and chosen recovery objectives before using its timings.

The logical drill uses Postgres17.10 with fsync/full_page_writes/synchronous_commit
on/on/on. Completed CBOR backup imports into a fresh history, rebuilds the saved
projection, reopens it and advances. Its readiness clock starts before preparing
the fresh target schema and ends after reopening and loading the saved pair.

The physical drill uses a private Postgres18.6 cluster. Native pg_basebackup
includes streamed WAL and pg_verifybackup validates the copy. The old primary
stops before the copy boots. Migration checksums, history and the saved pair
survive; a subsequent append advances the same consumer. Its readiness clock
starts after the old primary has stopped and ends when the projection reloads.

| Run | Logical backup, ms | Logical readiness, ms | Physical verified backup, ms | Physical readiness, ms | Full two-case run, s |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 | 2.680417 | 48.200084 | 376.025125 | 141.374167 | 6.213 |
| 2 | 2.092292 | 45.876 | 403.584208 | 140.4845 | 6.504 |
| 3 | 2.133125 | 41.92975 | 413.460084 | 140.917959 | 23.422 |

Readiness windows exclude initial source setup, source drain/close or shutdown,
backup time, final append/cleanup, external provisioning and traffic routing.
The full runs have substantial work outside those windows. These measurements
establish the small drill's method and correctness; they supply no deployment
SLO, large-data restore or cross-node failover claim.

Each run executed both native laws successfully, with18 other cases explicitly
unselected. Compilation is excluded. The unchanged full adapter suite separately
passed264cases. Exact commands, three reviews after every run, failed controls
and remaining delivery criteria are in the [research record](research.md).
Raw outputs: [run1](evidence/mnesis-pg7-recovery-repeat-1.log),
[run2](evidence/mnesis-pg7-recovery-repeat-2.log),
[run3](evidence/mnesis-pg7-recovery-repeat-3.log).
