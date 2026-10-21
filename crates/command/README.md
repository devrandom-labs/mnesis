# Mnesis Command

Durable command processing over an application's Mnesis aggregate and store.
Each accepted request is stored atomically with its original receipt and ordered
facts. Repeating the exact request returns its original receipt. An uncertain
write must be reconciled before an unrelated new decision proceeds.

The application supplies its aggregate rules, identity, store and Tokio runtime.
It owns authorization and external effects. Admission is a bounded in-memory queue;
it is not durable until the acceptance record commits. Retain each original
request until its outcome is known and await the processor's shutdown task.

Stored acceptance records use `controller.accepted.v2`; the package rename does
not rename event types or rewrite history. Current bounds are 4,096 accepted
records per stream and 4 KiB encoded request/record limits. Receipts are retained
for the entire supported history, including accepted no-ops.

This crate evolves in Mnesis's 0.x consumer tier. The
[command processor PRD](../../docs/prds/command.md) defines its required public
contract and delivery criteria.
