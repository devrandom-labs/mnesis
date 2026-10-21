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
