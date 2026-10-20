# Postgres capacity evidence

Measured 2026-10-20 against published0.5.2 product source at9e5ceeb3, before
workload-only PR405 integration. Benchmark source SHA256
582662e92362290754a5cf2f1a604a25a9b56f7cdb35fb4876bbaf05b90ed54a is unchanged
on protected8b0ee977. This is a reproducible local capacity profile, not a service
level or cross-node failover guarantee. No library tuning follows these results.

## Workload and reproduction

Use the repository Nix shell, `cargo bench -p mnesis-postgres --bench capacity`.
For command-level OS resource measurements build with `--no-run`, then run the
reported compiled binary under `/usr/bin/time -l`; exclude compilation and shell
startup. `MNESIS_CAPACITY_EVENTS=10000` runs four smoke fixtures only. Full output:
[ordinary/forced plans](evidence/pg8-capacity-modes-full.log),
[prior generic-only baseline](evidence/pg8-capacity-052-full.log),
[smoke](evidence/pg8-capacity-052-smoke.log),
[hardware](evidence/pg8-capacity-hardware.txt).

Apple M4 Pro,14cores,24GiB, macOS26.5.2, Rust/Cargo1.99.0, PostgreSQL18.6.
Each fixture creates its own Unix-only writable-primary cluster with logged
tables, fsync/full_page_writes/synchronous_commit=on. Pool24 includes one listener;
page128rows/1MiB; statement/acquisition30s and lock5s. It seeds through actual
100-event appends, checks native row count, VACUUM ANALYZE, then awaits the safe
watermark for at most60s before timing. Preparation is excluded.

The12fixtures are10k/100k/1m events ×100-event or full-history streams ×32/4096B
payloads. One OS-random template per fixture is retained in the log, with an
ordinal replacing its first8bytes. Repeated tails do not model independent
production entropy.4096B is the larger measured payload, not the maximum accepted
row. Frame/key overhead counts toward the configured1MiB byte budget.

Reads exercise inclusive stream heads/tails and initial/exclusive-resume global
pages at1/16/128simultaneous consumers. Two whole cohorts warm each cell, then
three windows collect at least100responses (100/112/128 by concurrency), validating
versions, keys where returned, ordinal/tail bytes and increasing positions.
Latency includes model validation. Each full global drain separately validates
all payloads/keys/positions/count, without a separate per-event version assertion.

Append cells use1/8events and1/16/128writers, barrier overlap, independent fresh
streams or one competing fresh stream per cohort. They run one warmup cohort,
then three windows with at least100successful commits. Shared cohorts require
exactly one winner; all other outcomes must be typed conflicts, and every unknown
or other error fails. Independent SQL counts must equal successful event totals.
This measures new-stream appends in a populated database; it does not establish
1/8-event latency when extending an already long aggregate. Seeding exercises
that append path in100-event batches outside timing. Row growth is reported.

## Results and limits

All12fixtures,432read and432append windows,96plans and12pool-zero closes finish
with zero invariant failures. Command361.55s real/159.03user/117.29sys;
maximum resident set57,737,216B, peak memory footprint35,291,544B, no swaps.
Those OS values describe the timed command, not an isolated client allocator
peak or all database memory. Client RSS snapshots, allocator live/peak/retained,
allocation traffic and call counts are separate observations. Allocation counters
add instrumentation overhead and count successful allocations, not operations.

At1m events, three-window ranges below use milliseconds. Read rates count events
validated in bounded pages. Independent append rates count committed events;
shared response quantiles include losing conflicts (committed quantiles are
separately printed). This table selects the long-stream fixture with4096B payload;
all other cells, including short streams, remain in the raw evidence.

| Operation | Clients | p50 ms | p95 ms | p99 ms | Committed/validated events/s |
| --- | --- | --- | --- | --- | --- |
| Global initial page | 1 | .691–.716 | .861–.988 | 1.032–1.070 | 174344–177061 |
| Global initial page | 16 | 5.641–5.994 | 6.465–6.821 | 6.509–6.912 | 320556–334563 |
| Global initial page | 128 | 20.836–25.021 | 38.017–39.994 | 38.329–40.561 | 400601–422719 |
| Global resumed page | 1 | .647–.789 | 1.075–1.127 | 1.173–1.182 | 156540–176014 |
| Global resumed page | 16 | 5.620–6.001 | 6.351–6.609 | 6.438–6.696 | 319276–333564 |
| Global resumed page | 128 | 21.318–24.884 | 38.036–42.026 | 38.456–42.224 | 383432–421401 |
| Independent1-event append | 1 | .213–.281 | .400–.748 | .706–.932 | 2656–3874 |
| Independent1-event append | 16 | .865–.927 | 1.034–1.408 | 1.079–1.425 | 13995–15637 |
| Independent1-event append | 128 | 6.748–7.538 | 7.399–8.134 | 7.490–8.182 | 14894–16385 |
| Independent8-event append | 1 | .633–.678 | .840–1.054 | 1.000–1.571 | 11059–12314 |
| Independent8-event append | 16 | 2.126–2.563 | 2.700–3.057 | 2.818–3.086 | 45097–52548 |
| Independent8-event append | 128 | 18.155–19.857 | 18.686–20.576 | 18.755–20.655 | 48585–53444 |
| Competing1-event responses | 128 | 1.860–1.999 | 2.771–2.924 | 2.972–3.193 | 305–320 |
| Competing8-event responses | 128 | 2.683–2.732 | 4.017–4.102 | 4.417–4.562 | 1923–1934 |

The last row has100commits and12700conflicts per window. Its successful-commit
p99 is4.728/4.757/5.050ms, separately from all-response p99. More concurrent
clients produce queueing once the23data connections are busy. Independent writes
mostly saturate by16clients;128raises latency without proportional throughput.
Competing writers mostly generate conflicts. Do not size a deployment from its
all-response rate as if every response committed.

Three-window operations/s spread `(max/min−1)` across288cells has median7.35%,
p95 66.21%, maximum624.20%. The maximum is100k/long/4096B/128independent1-event
appends:6.702/9.924/48.538ms window duration and5.722/5.883/47.575ms p50. The
previous same-product full baseline gives6.922/7.506/10.421ms for that cell;
the48.538ms sample is retained, without an unsupported cause or improvement
claim. Short windows expose scheduling/storage variation. These observations do
not justify small percent tuning or extrapolated production tail guarantees.
No before/after product change was made; baseline343.78s versus361.55s includes
additional plan executions, so command time is not a product regression metric.

The largest measured client allocator peak across windows is15,136,379B and
largest retained delta273,980B. At fixed pages, global-drain peaks for4096B
payload are809,354–809,489B at10k,809,611–809,746B at100k, and810,003–813,911B
at1m. Drain allocated traffic grows with history (~91MB/911MB/9.11GB), while
retained memory stays near213–217kB. Million-event drains take8.870s(short) and
7.966s(long). Every fixture closes its pool to0; final retained client allocation
is103,838B. Read snapshots show pool3/4/24 and idle2/3/23, reflecting the shared
listener; no per-consumer connection or permanent cursor lease is introduced.
The30-minute mixed workload and31faults are separate evidence in PR405 and
[recovery](recovery.md), not inferred from this shorter matrix.

## Query plans

EXPLAIN ANALYZE BUFFERS replays the actual SQLx prepared page statements, including
native parameters and each selected session's prior generic/custom counters.
The observer compares ordinary `plan_cache_mode=auto` with forced generic and
resets the session afterwards. Timed read cells use ordinary auto. Early/global
resume and stream first/late shapes are present; continuation-stream SQL is not
claimed here. The96individual plans are diagnostics, not latency distributions.

All four1m initial-global auto executions are .170–.238ms and use the watermark
index for128candidates and primary-key lookups for128bodies. Their selected
sessions show0prior generic and37–148custom plans. Forced generic executions
are68.997–78.303ms: candidate selection remains bounded/indexed but the outer
join chooses a hash join and scans1m event bodies. The long4096B auto example
uses519shared buffer hits, compared with381hits/13912reads under forced generic.
A sequential scan of a one-row keys table in auto plans is not a full event scan.

Record this explicit supported-profile limit: forcing generic plans has a large
measured cost on this query shape. The native counters and ordinary timed cells
do not demonstrate that auto selected that expensive plan. No global SQLx
statement-cache change, session GUC override, index or public tuning knob is
justified by the forced-mode diagnostic alone. Existing normal-auto primary
behavior is the measured profile; different distributions, generic-plan policy,
remote networks, replicas, proxies, deployment SLOs and cross-node failover need
measurements in their selected deployment before a support/performance claim.
