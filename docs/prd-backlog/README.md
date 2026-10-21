# Mnesis completion requirements

The [Postgres adapter PRD](../prds/postgres.md) records completed adapter work.
The [command processor PRD](../prds/command.md) defines the proposed extraction
of `controller-archive` into `mnesis-command::CommandProcessor`.
The [status index](status.md) records dependencies and delivery for both features.
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

CP0–CP9 comprise command processor extraction, contract verification, publication
and server registry consumption. CP0 is locally qualified; protected delivery
and CP1–CP9 remain outstanding. PG9 is their completed adapter prerequisite. The new
feature moves generic command identity/receipt mechanics into Mnesis; Labcast
keeps business rules, ID namespaces, authorization, worker fencing and effects.
The source archive is separate from `labcast-server`, whose workspace is empty.
Server consumption does not authorize rebuilding an earlier application design.
The `labcast` mobile repository is outside this feature.
Tower Service is required over the processor-owned Tokio worker. CP5 includes
tracing/metrics and CP7 covers application-owned collection/export and features.
