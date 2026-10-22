# Mnesis Command

Durable command processing over an application's Mnesis aggregate and store.
Each accepted request is stored atomically with its original receipt and ordered
facts. Repeating the exact request returns its original receipt. An uncertain
write must be reconciled before an unrelated new decision proceeds.

The application supplies its aggregate rules, identity, store and Tokio runtime.
It owns authorization and external effects. Admission is a bounded in-memory queue;
it is not durable until the acceptance record commits. Retain each original
request until its outcome is known and await the processor's shutdown task.

`start` and `resume` return the processor and an owned `Worker`. Stop admission,
then await the worker for a `Shutdown` report. Aborting the worker also requires
awaiting it. The report retains both the exact unresolved request and the original
Tokio task cancellation or panic; `take_cause` lets the application inspect a panic
payload without losing the request. Pass the report by reference to `resume`.
An absent unresolved request is not proof that an earlier write rolled back.
Dropping the worker detaches the task and loses its report. A canceled wait through
`&mut Worker` keeps ownership available for another wait.

The supplied Tokio runtime must enable time and outlive its workers. Startup
checks for a runtime handle; it does not probe the timer driver, stored history
or database health. Without timers, the first operation fails the worker with
Tokio's native panic, retained in its shutdown report. An uncertain caller keeps
the exact original request. Operation deadlines exclude queue waiting and cannot
preempt a handler or adapter that does not yield. Use `read` or an exact retry
to observe history; uncertainty does not schedule a background retry.

Stored acceptance records use `controller.accepted.v2`; the package rename does
not rename event types or rewrite history. Current bounds are 4,096 accepted
records per stream and 4 KiB encoded request/record limits. Receipts are retained
for the entire supported history, including accepted no-ops.

This crate evolves in Mnesis's 0.x consumer tier. The
[command processor PRD](../../docs/prds/command.md) defines its required public
contract and delivery criteria.

## Tracing and metrics

The default feature set is empty. Enable `tracing`, `metrics`, or both. The
application installs subscribers and metrics recorders and owns exporters,
endpoints, credentials, sampling and flushing. The processor installs no global
collector, starts no exporter and sends no telemetry over the network. Both
features also work without collectors. Arbitrary blocking collector callbacks
can stall the calling task; applications must configure bounded, nonblocking
export and an appropriate overflow policy.

Tracing captures each caller's span and dispatcher before sending its request.
The worker retains that context across queueing, cancellation and awaits; its
own lifetime is a separate root. Spans are `mnesis.command.request`,
`mnesis.command.worker` and `mnesis.command.stage`. Request fields are `operation`
and `outcome`; stage fields are `stage` and `outcome`. Events are
`mnesis.command.accepted`, `mnesis.command.duplicate` and
`mnesis.command.reply_dropped`. No request IDs, payloads or error strings are
recorded. Context never becomes part of the persisted request identity.

Every metric below has the prefix `mnesis_command_`. Counters count process-local
observations, not authoritative durable totals. Replaying history alone does not
increase acceptance counts; observing a locally unresolved original's exact
committed receipt does. A repeated receipt increases the duplicate counter.
Process loss can leave a committed acceptance uncounted. Counts across hosts
must not be used as a global receipt ledger.

| Name | Kind and meaning | Labels |
| --- | --- | --- |
| `submitted_total` | Calls reaching request instrumentation, including invalid submissions; excludes startup and readiness polls | `operation` |
| `admitted_total` | Calls actually transferred to the worker mailbox | `operation` |
| `rejected_total` | Calls with a recorded outer error other than `uncertain` | `operation`, `outcome` |
| `uncertain_total` | Recorded `Uncertain` results, including an awaited submit/retry response lost at worker exit | `operation` |
| `acceptances_total` | Newly observed successful local acceptances or reconciliation of a retained original | None |
| `duplicates_total` | Successful duplicate receipt observations | None |
| `conflicts_total` | Calls whose recorded outer result is `Conflict` | None |
| `reconciliations_total` | Completed or interrupted reconciliation stages | `outcome` |
| `worker_failures_total` | Native task failure observed by awaiting the worker | `cause` |
| `reserved` | Tower readiness permits observed by live processor handles | None |
| `queued` | Prepared queue slots, including the short interval before transfer and failed handoff cleanup | None |
| `inflight` | Worker calls currently executing | None |
| `unresolved` | Live engines retaining a pending original, including during a write | None |
| `queue_seconds` | Prepared queue slot lifetime until dequeue, rejection or destruction | None |
| `execution_seconds` | Actual worker execution until completion or destruction; excludes queue waiting | `operation` |
| `stage_seconds` | Admission, load/replay, decision, append, reconciliation or cleanup duration | `stage`, `outcome` |
| `shutdown_seconds` | First explicit `stop()` until native worker completion is observed | None |

Histograms use monotonic wall-clock **seconds**. Append timing includes decision,
encoding, persistence and folding. The tracing queue stage starts at preparation;
the metric queue timer starts once the send path owns capacity. Reconciliation
duration uses `stage_seconds` with `stage=reconcile`. Shutdown tracing measures
cleanup only; `shutdown_seconds` includes completion-observation delay and is
omitted when there is no known explicit stop origin. Repeated `stop()` calls
preserve the first origin. Cancellation/panic without explicit stop adds no
invented shutdown sample.

Operations are `submit`, `retry`, `read`, `original`; worker failure causes are
`cancelled`, `panic`. Stages are `admission`, `load`, `decision`, `append`,
`reconcile`, `shutdown`. Outcomes are `ok`, `interrupted`, `failed`, `mismatch`,
`conflict`, `queue_full`, `queue_limit`, `deadline_zero`, `closed`, `scope`,
`runtime`, `uncertain`, `request`, `corrupt`, `timeout`. Some categories apply
only to particular stages. Request and metric label vocabularies are bounded;
no aggregate, request or trace ID becomes a metric label.

Gauges aggregate live owners across clones and hosts. An unused readiness permit
can outlive a stopped worker; releasing the handle or using the permit releases
its contribution. A shutdown report or caller can retain an unresolved original
after its engine exits, so `unresolved=0` never proves rollback. Dropping a reply
does not cancel admitted work. An interrupted request does not invent a known
outcome; a dropped caller may never observe or increment an uncertain response.
Metric updates are not atomic snapshots of the command protocol. In particular,
`rejected_total` describes that call's outer error and cannot prove that an older
write did not commit. Always use typed outcomes, the retained original and its
exact conditional retry for recovery; telemetry never makes a timeout safe to retry.
