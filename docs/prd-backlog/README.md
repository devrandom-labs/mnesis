# Mnesis completion requirements

The [Postgres adapter PRD](../prds/postgres.md) defines the selected adapter
completion work. Its [status index](status.md) records dependencies and delivery.
The older `docs/kanban.org` is retained; it does not define this feature's scope.

Follow the existing devrandom research protocol and Bombay's feature-local PRD
model: verify locked contracts, reproduce the defect, record owners and a small
change budget, implement, prove observable acceptance and negative cases, then
minimize and deliver through a reviewed PR. Keep detailed evidence in the PRD.

States: `candidate` needs contract/baseline verification; `blocked` has unresolved
prerequisites; `active` has verified contracts and a recorded implementation
decision; `feature-complete` has passing acceptance; `distilled` also has the
final minimization audit; `merged` has a reviewed merge with required CI passing.
Record publication and downstream verification separately. Neither local success
nor a planning-document merge marks implementation delivered.

Every `Blocked by` edge must have a reciprocal `Unblocks` edge. PG0–PG9 comprise
the required Postgres release. PG10 is conditional consumer work and cannot be
silently added to or used to shrink that release scope.
