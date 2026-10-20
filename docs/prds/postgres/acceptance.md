# Postgres acceptance evidence

Final acceptance map under delivery verification. PG0–PG9 remain required; unmerged evidence and an earlier
published version do not complete the whole adapter. PG10 is conditional on a
selected consumer. The dependency/status owner remains [the backlog](../../prd-backlog/status.md).

| Requirement | Observable owners and retained evidence | Final delivery condition |
| --- | --- | --- |
| PG0 | [research record](research.md) original defect witnesses and contract decisions; [original witness](evidence/original-memory-small.log), [original witness](evidence/original-memory-large.log), [original witness](evidence/original-wakes.log). The original read/wake source fails before repair. | Preserve exact baseline, finite-scan, stable-order, unsigned representation and commit-outcome decisions. |
| PG1 | [read_tests.rs](../../../adapters/postgres/tests/read_tests.rs), [all_noskip_tests.rs](../../../adapters/postgres/tests/all_noskip_tests.rs), [representation_tests.rs](../../../adapters/postgres/tests/representation_tests.rs), page/resume conformance; byte-plans.log and [capacity measurements](capacity.md) auto/forced plans. Tests cover row/byte edges, rollback gaps, later corruption, finite boundaries, cancellation and independent pool release. | Capacity matrix protected delivery plus complete final native suite. |
| PG2 | [subscription_tests.rs](../../../adapters/postgres/tests/subscription_tests.rs) and [lifecycle_tests.rs](../../../adapters/postgres/tests/lifecycle_tests.rs); final lost hints, disconnect/reconnect, held transaction/snapshot watermark, arm-before-poll, overlong/malformed hints, shared listener. PR4051800s soak includes31fault cycles with3s healthy-service recovery bound and no later writer. | Complete selected profile passes, no per-subscriber background task or held data connection. |
| PG3 | [outcome_tests.rs](../../../adapters/postgres/tests/outcome_tests.rs), [deadline_tests.rs](../../../adapters/postgres/tests/deadline_tests.rs), actual transport-held COMMIT response, atomic/import/repository/projection propagation, native server outcomes and exact phase proof. | Timeout/connection loss after COMMIT stays unknown; neither adapter nor wrapper retries it. Labcast owns application reconciliation. |
| PG4 | [migration_tests.rs](../../../adapters/postgres/tests/migration_tests.rs), [upgrade_tests.rs](../../../adapters/postgres/tests/upgrade_tests.rs), [representation_tests.rs](../../../adapters/postgres/tests/representation_tests.rs); checked-in migrations1–5 and schema validation. Legacy/populated upgrades, rollback/repair, concurrency, checksum/newer layouts, restricted runtime open, full unsigned values, binary keys and history. | Complete final real-DB suite and operator migration procedure; no destructive reset. |
| PG5 | [atomic_tests.rs](../../../adapters/postgres/tests/atomic_tests.rs) plus shared AtomicAppend laws; [backup_tests.rs](../../../adapters/postgres/tests/backup_tests.rs) whole-chunk import. Exact rejection/progress/conflict and independent readers around overlapping writes. | All-or-none persistence with unknown outcome preserved; default/import feature suites actually run. |
| PG6 | [snapshot_tests.rs](../../../adapters/postgres/tests/snapshot_tests.rs), [checkpoint_tests.rs](../../../adapters/postgres/tests/checkpoint_tests.rs), [projection_tests.rs](../../../adapters/postgres/tests/projection_tests.rs), [outcome_tests.rs](../../../adapters/postgres/tests/outcome_tests.rs); codec/aggregate/projection composition, old-or-new pair, CAS races/rebuild, stale metadata, restart/process death and uncertainty. | Snapshot and projection feature suites actually run; failed/cancelled checkpoint writes require reload. |
| PG7 | [listing_tests.rs](../../../adapters/postgres/tests/listing_tests.rs), [session_tests.rs](../../../adapters/postgres/tests/session_tests.rs), [backup_tests.rs](../../../adapters/postgres/tests/backup_tests.rs), [restore_tests.rs](../../../adapters/postgres/tests/restore_tests.rs); completed format-2 Postgres/Fjall roundtrip, binary routes, damaged/incomplete files, duplicate routing/final conflicts, expiry/cancellation, logical rebuild and verified physical recovery. [recovery measurements](recovery.md) records three measured two-event operator repeats. | Exact source/destination histories and saved projection semantics; no logical checkpoint portability claim. |
| PG8 | [security_tests.rs](../../../adapters/postgres/tests/security_tests.rs) (required TLS/runtime-role cases), [durability_tests.rs](../../../adapters/postgres/tests/durability_tests.rs) (logged/on/on/on and actual primary restart), lifecycle/deadline/diagnostics tests; PR405soak, [capacity measurements](capacity.md) complete matrix and resource/plan limits. | Protected capacity delivery, measured ordinary-auto writable-primary profile only. Remote failover, proxies/replicas, forced-generic performance and fleet SLOs excluded without selected evidence. |
| PG9 | Normal signed Nix gates, actual Linux postgres-integration default/each/all suites, cargo-hack dev-free profiles, existing fuzz/mutation owners and real AFL saved-crash verdict proof; concise adapter README/operator instructions. | Final checks/review/protected merge, two fresh source/contract audits, publication of exact final version and clean registry-native consumer. Pending. |

Detailed original reds, rejected controls, three reviews after each actual test
run, raw logs, measurements and exact protected commits remain in [research record](research.md).
Compiler-only, empty/guarded/cached/echoed output is not native runtime evidence.

The actual Linux default/export/import/projection/snapshot/tracing/all suites
pass149/168/176/173/174/150/264cases, every complete suite zero skips. Each
individual profile first passes its required owning law. The real AFL control
run39454205176 returns engine0 with3saved crashes, verdict1, successful archive
upload12279449876 and expected aggregate failure; all3original bytes/paths and
ZIP digest are independently verified. Its intentional-crash branch is never
merged. Capacity measurements and deployment limits live in [capacity measurements](capacity.md);
the full30-minute soak and measured logical/physical drills are protected405.
Two fresh owner/source audits and comment-only minimization are recorded in
[research record](research.md). Final current-head protected delivery and publication remain open.
