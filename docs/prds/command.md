# Command processor extraction and delivery

Status: CP0/CP1 protected-merged; CP2 locally qualified. Protected delivery pending.
Updated: 2026-10-21. Owners: Mnesis command maintainers and the Labcast consumer owner.
Planning branch: `docs/command-prd`.
Mnesis baseline: `27de3b4a4aad59909ffaccdf6fa83e8b256ccca6`, version 0.5.3.
Source: `/Users/joel/Code/labcast/controller-archive/crates/controller`.
Consumer: `labcast-server`; the `labcast` mobile repository is outside this work.
Exact inputs, primary sources, findings and executed checks: [research](command/research.md).
Dependencies and delivery states: [backlog](../prd-backlog/status.md#command-processor).

The user authorized implementation and delivery of CP0–CP9 on 2026-10-21 under a
persistent goal without a token budget. Recorded acceptance and delivery evidence
determine progress; authorization does not claim extraction or publication.

## Problem and intended result

Labcast's archived controller contains reusable command-processing infrastructure
that depends only on Mnesis, Tokio and serialization libraries. It has been removed
from the mobile repository but has no maintained upstream package. Keeping this
general event-sourcing capability in a local archive prevents ordinary registry
reuse. The name `Controller` hides its job.

Move the supported implementation to `crates/command` in Mnesis, publish it as
`mnesis-command`, and name the main public type `CommandProcessor`. A consumer
supplies an aggregate and its command handler. The processor admits requests,
loads committed history, checks retry identity and expected version, invokes
the handler, atomically saves acceptance, and returns the original receipt.
It preserves uncertain writes through reconciliation and observed shutdown.
Expose Tower's service interface over that owned Tokio worker, with supported
tracing and metrics features and application-owned telemetry export.

The result is one maintained implementation with a documented persistence
contract, useful typed errors, meaningful concurrency tests, real adapter
evidence and a clean registry consumer. Moving files or passing an in-memory
test alone does not meet that result. CP0–CP9 are required for full delivery.

## Domain and naming

The domain is **command processing for an event-sourced aggregate**. Here an
aggregate is the consistency boundary whose state and decisions share one
event stream. Its business meaning remains with the application.

`CommandProcessor` describes the infrastructure around that business handler.
The name follows established usage, not a claim of one universal standard.
Brighter distinguishes its processor from the application handler; Axon calls
handler routing a command bus; Rust `cqrs-es` describes the load, replay,
handle and persist sequence. [Primary-source comparison](command/research.md#primary-sources-and-alternatives).

| Term | Meaning in this feature |
| --- | --- |
| Command | Application intent supplied to the existing `Handle<C, N>` contract. |
| Request | Command, stable caller-supplied ID and original expected physical stream version. |
| Acceptance | One atomic stored record containing the original request, receipt and ordered domain facts. |
| Receipt | Evidence of accepted persistence; contains original ID, physical version and fact count. |
| Retry | Resubmission of the exact original request, including its expected version. |
| Conflict | The original conditional append cannot win at its expected version. |
| Uncertain outcome | An append may have committed or may still commit; a missing receipt alone does not resolve it. |
| Shutdown report | Observed task completion and any unresolved original request, bound to its stream. |

Durability covers accepted records and receipts. Queue admission is volatile.
A queued request can be lost when the process dies; callers must retain the
original request until its outcome is settled. Domain rejections are not saved
by the current contract. This is not a durable inbox or a cache of every response.

## Ownership and scope

| Owner | Responsibility |
| --- | --- |
| `mnesis` | Pure aggregate rules, version arithmetic, replay and kernel integrity. |
| `mnesis-store` | Repository execution, codecs, atomic conditional append, streams, positions, snapshots and projections. |
| `mnesis-command` | Admission, request/receipt identity, acceptance records, conflict reconciliation, uncertain-write barriers and task lifecycle. |
| Store adapter | Actual persistence, isolation, durability, migration and adapter-specific errors. |
| Labcast domain | Business commands, decisions, identities, ownership generations and valid transitions. |
| Labcast application | Current authorization, selecting/routing to an aggregate, retaining pending requests, coordinating effects and recovery. |
| Worker/effect boundary | Rejecting stale generations and reconciling external execution outcomes. |

The earlier Postgres PRD kept command identity and deduplication in Labcast.
This new feature deliberately moves their **generic mechanism** to Mnesis.
Labcast still defines ID namespaces and business meaning. Worker fencing and
external effects remain downstream. PG0–PG9 stay complete; PG10 stays conditional.
No Postgres table or adapter-owned command ledger is added for this extraction.

`mnesis-command` is an optional Tokio consumer of the kernel/store contracts.
Neither the kernel, store nor an adapter gains a normal dependency on it.
Record that consumer layer explicitly in the workspace architecture: the older
description of all `crates/` as adapter foundations is insufficient for this crate.
Keep the kernel/store's existing `no_std` and target gates intact.

Excluded from this release: a command bus, handler registry, actor framework,
supervision tree, scheduler, distributed placement, leases, a durable queue,
outbox, multi-aggregate command transaction, saga engine, exactly-once external
effects, blanket CQRS adoption, new database schema and automatic history
compaction. Existing Mnesis capabilities remain available to applications.
No new framework, runtime, custom development tool or project Python is permitted.

## Source audit and starting evidence

The reviewed crate has 937 production lines in four files, six root-exported
types, 1,891 lines of tests/helpers, and two fuzz targets. These are physical
line counts, not complexity scores. All source and dependency inputs are
fingerprinted because the source archive has no Git revision. All 30 recorded
input hashes match the previously inspected and executed source after its move.

Fresh Nix execution passed 27 tests; one performance test was intentionally
ignored in the ordinary run. That test then ran explicitly in release mode:
five samples of 100 duplicate requests over 128 records took 5,040–5,112 µs
per sample. These are in-memory observations, not Postgres capacity evidence.
[Commands, logs and three reviews](command/research.md#executed-baseline).

| Finding | Current behavior and consequence | Required owner |
| --- | --- | --- |
| Existing correctness core | One acceptance record includes zero or more facts; retries and delayed commits have substantive tests. Preserve these laws. | CP1–CP3 |
| Dependency mismatch | Labcast uses kernel/store 0.4.0; destination is 0.5.3 with different locked serialization/test dependencies. Compatibility needs execution. | CP0, CP1 |
| Error erasure | `Store` and `Uncertain.source` use `Box<dyn Error>`; kernel failures can become undifferentiated `Corrupt`; capacity reasons are merged. | CP2 |
| Fixed limits | 4,096 accepted records and 4 KiB request/record limits are separate constraints. New commands stop at the receipt limit; no-ops count too. | CP5, CP6 |
| Full replay | Every load rebuilds all retained history; `retry` of a persisted ID can load twice. Existing timing covers only 128 small records. | CP6 |
| Private record format | Domain facts live inside private `Record`/`Codec` types. External consumers cannot use their validation through the current public API. | CP4 |
| Format evolution | Event type is `controller.accepted.v2`, payload schema is 2, envelope schema is 1. Renaming the crate must not silently change these bytes. | CP4 |
| Race coverage | The two-host test uses `join!` but does not force both hosts to read the same head before appending. | CP3 |
| Runtime precondition | Startup checks a Tokio handle, but does not establish an enabled timer. Operation timeouts cannot preempt a non-yielding handler. | CP5 |
| Recovery custody | Shutdown evidence is in memory and checks raw stream bytes. Abrupt process loss requires a caller-retained original request. | CP2, CP5 |
| Repository split | Source is in `controller-archive`; `labcast` is mobile. `labcast-server` has an empty Rust workspace and no runnable server. Historical backend checks are not current evidence. | CP0, CP7, CP9 |
| Destination conventions | Saturating arithmetic, erased causes and private-module `pub` items need review against current Mnesis rules. Fuzz harnesses differ. | CP1, CP2, CP7 |

These are source findings and evidence gaps. The planning run did not establish
new runtime defects in the processor or execute its Postgres composition.

## Required contracts

### Tower service and execution ownership

`CommandProcessor` must expose Tower's `Service<Request<C, I>>` contract, returning
the original typed receipt or error. Reuse the workspace's Tower 0.5.3 contract
(`tower-service` 0.3.3); select the narrow dependency rather than enabling unrelated
middleware. Tower is the supported service interface. One owned Tokio worker
per processor instance executes calls sequentially and retains unresolved writes.
The [tower-actor experiment](command/tower.md) remains evidence against adopting
that package unchanged. It is not evidence that Tower itself cannot be used.

- `poll_ready` reserves bounded admission capacity, not database readiness or a
  successful command outcome. Count reservations with queued work; one active
  call is additional. A reservation belongs to its service handle until used or
  the handle is dropped/released. Tower cannot observe cancellation of a borrowed
  `ready()` future; do not promise automatic reclamation in that case. Canceling
  an owning `oneshot` operation drops its service handle and releases its reservation.
- For a valid admitted request, `call` transfers it to the owned worker before
  returning its response future. Dropping that future does not cancel admitted
  work, including when it was never polled. Validation failure does not enqueue.
- Clones share one worker/queue, but do not duplicate a readiness reservation.
  Use readiness and `call` on the same handle. No extra buffer or worker per layer.
- Worker termination between readiness and `call` returns a typed failure with
  the request's known admission/outcome status. It must not panic or leave the
  reply pending. Preserve original worker causes through an owned completion path.
- Retain explicit stop and observed task completion with unresolved-request
  handoff. Ready permits and retained clones must not prevent shutdown forever.
  A stopped worker cannot admit new work through an earlier reservation.
- Preserve the existing immediate overload-rejection convenience operation if
  retained; document its difference from waiting for Tower readiness. It delegates
  to the same admission/execution owner. Reads/retries use existing typed methods;
  do not invent a public heterogeneous command bus to fit every operation into one type.
- Document supported middleware composition and ordering. An outer timeout stops
  waiting; it cannot prove rollback. Retry policies preserve original ID/content/
  expected version and reconcile uncertainty. Do not enable automatic retries by
  default. Extra buffering or concurrency layers must not bypass per-worker order
  or the declared admission bound; an outer readiness reservation is not admission.

CP0 proves the interface/error shape; CP1 exposes it; CP2/CP3 preserve outcomes
and order; CP5 proves cancellation, resource accounting and completion. Include
real Tower `ServiceExt` consumers and timeout/cancellation compositions, not only
direct calls to a convenience method. Readiness alone is never a health check.

### Identity and acceptance

1. Scope an ID to the actual raw aggregate stream and selected store history.
   The caller owns namespace and uniqueness. Keep the generic validated ID type;
   test both the default nonzero integer and an application namespace/UUID type.
2. An accepted ID binds to the entire original typed request, including expected
   version. An equal retry returns the original receipt without rerunning the
   business handler or appending. Changed content under that ID returns mismatch.
   Equality and lossless serialization are explicit requirements on custom types.
3. A fresh domain rejection, invalid request, admission failure or definite
   pre-append failure writes no acceptance. Such a failure does not reserve the
   ID forever. Never describe this as replaying every previous failure response.
4. Every accepted command, including a no-op, consumes exactly one physical
   stream version. Its receipt and all facts commit together. Fact order and
   count survive replay; `N` means capacity for up to `N + 1` facts in the
   existing Mnesis contract. No invented fact versions replace physical versions.
5. Check known acceptance before rejecting a stale expected version. Concurrent
   identical requests converge on one receipt; concurrent different requests at
   the same base yield one winner. Conditional append is the authority across
   hosts. A local queue establishes no distributed ownership.
6. Never automatically rebase a request onto a newer version or allocate a new
   ID during retry. A new business decision requires a new caller request.

### Outcomes and reconciliation

| Observation | Required result |
| --- | --- |
| Failure before this attempt reaches append, no older uncertainty | Definite typed rejection/failure; no record for this attempt. |
| Confirmed append conflict | Reload and return the matching original receipt, mismatch, or conflict. Preserve a failed reload as a typed read failure. |
| Append success and valid committed state | Original durable receipt; no second execution on later retries. |
| Timeout, dropped response, or unclassified adapter write failure | Uncertain with original ID and typed cause where available. |
| Receipt absent and observed head at/below original expected version | Older write remains uncertain. |
| Valid committed head advanced past original expected version | Original conditional append has committed or can no longer win; inspect the accepted ID to distinguish the result. |
| Exact original retried while its old append is unresolved | Only the same conditional append is allowed. One winner; the other attempt cannot append again. |
| Retry fails before append while an older attempt is unresolved | Preserve the older uncertainty and original request; retain the newer cause. |
| Known earlier receipt while an unrelated write is unresolved | Return that receipt without clearing the unrelated barrier. |
| Read timeout/corruption/unavailability | Report the read failure without claiming an older write rolled back. |

Keep unclassified adapter errors conservative. The shared append contract does
not expose a universal retry-safety classifier. Do not depend on SQLx in the
processor or parse error strings. Postgres `CommitUnknown` must remain reachable
as a typed source and can never become permission for a new decision. A more
precise generic failure contract requires a separate demonstrated need and
owner review; it is not required to make conservative reconciliation work.

Kernel failures after successful storage must preserve committed/uncertain
custody. In particular, `Repository::save` appends before `commit_persisted`.
Do not classify every kernel error as proof that nothing was persisted.

### Lifecycle, resource and application obligations

- Retain the existing bounded waiting queue plus one active call. Queue overflow
  and closed admission reject before enqueue. Admission has no durable receipt.
- Cancellation of an admitted caller's wait does not cancel the host's accepted
  work. Distinguish cancellation before enqueue, while queued, during a read,
  during append and after commit. Preserve the exact original when outcome is unknown.
- Explicit stop closes admission, rejects queued work, and lets active work
  settle or become uncertain. Await the task and retain its shutdown report.
  Dropping a task handle, aborting a task and stopping normally have different
  consequences; document and test them with Tokio's supported primitives.
- Resume validates stream scope and preserves the borrowed report on failure.
  Its scope does not prove database identity. Selecting the same authoritative
  store/history is a caller obligation unless a concrete supported capability
  can validate it. Do not invent a generic database fingerprint.
- Document timer-enabled Tokio startup, runtime lifetime and cooperative
  deadlines. A per-operation deadline excludes queue waiting and is not a hard
  CPU deadline for user code. Business handlers/folds must terminate, remain
  deterministic and perform no external effects.
- `start`/`resume` construct a host; their success alone does not validate stored
  history or database readiness. Document the first-load behavior and use the
  existing read/retry operations for observation. Recovery is request-driven;
  returning uncertainty does not schedule an automatic background retry.
- Distinguish queue fullness, record bytes, request bytes, history exhaustion,
  domain-fact replay limit, version overflow and invalid configuration in typed
  diagnostics. A single `Capacity` label is insufficient for operator action.
- Preserve finite limits and existing-history readability. The first release
  can retain the 4,096/4 KiB profile if CP6 qualifies it. No automatic receipt
  eviction, stream reset or ID reuse may create duplicate acceptance.
- Published limits apply per processor/stream. Applications bound host count,
  caller allocations, aggregate state size and external resources separately.
  A serialized-byte limit is not a bound on arbitrary user type heap allocations.
- Current authorization must precede requests, state reads and receipt disclosure.
  Stale-worker fencing and effect completion remain application/effect contracts.

## Requirements and acceptance

### CP0 Freeze the source and resolve contract decisions

Freeze the existing `controller-archive` crate, fixtures, fuzz corpus, manifests,
lockfiles and relevant evidence before modifying or retiring it. Record per-file
hashes, archive hash and source provenance in durable project records; the local
directory alone is not a durable source receipt. Preserve unrelated work. Confirm
the authority to publish this source under Mnesis's MIT OR Apache-2.0 license;
the current Labcast package has no license declaration. Check package-name
availability/ownership before publication; this planning task reserves no name.

Use the baseline in this PRD, re-run only changed/needed checks, and verify the
0.5.3 kernel/store interfaces and adapter outcomes. Complete the following
engineering decisions with small executable consumers before production edits:

| Decision | Selected direction; required evidence |
| --- | --- |
| Public naming | `mnesis-command::CommandProcessor`; migrate imports directly. No permanent forwarding `Controller` crate. |
| Layering | Optional Tokio consumer in `crates/command`; no reverse core/store/adapter dependency. |
| Tower interface | Required `Service<Request>` over the owned worker; prototype reservations, typed outcomes and observed completion. No unchanged tower-actor adoption. |
| Observability | Required optional tracing and metrics support, with application-owned collection/export. Qualify dependency/features and the instruments below. |
| Error shape | Typed source-preserving result; prototype adapter/domain type propagation without growing a new error framework. |
| Format | Preserve v2 bytes during extraction; identify actual persisted source data and compatibility obligations. |
| Fact consumption | One supported validated acceptance reader/codec with ordered facts; no consumer copies of private JSON. |
| Limits | Preserve baseline bounds initially; choose/document supported workload before tuning or adding configuration. |
| Recovery | Accepted-ID semantics, exact retry and caller-held crash recovery as specified above; no durable inbox claim. |
| Labcast adoption | A neutral registry consumer in `labcast-server`, whose workspace is currently empty; server domain design remains separate. No changes to mobile. |

Acceptance: immutable source receipt, dependency/feature inventory, public API
inventory, measured baseline, all decisions recorded, and per-slice budgets.
Unresolved source licensing blocks publication; absent Labcast business scope
blocks business integration only. Neither prevents independent crate verification.

### CP1 Extract and align the crate

Move production modules, canonical tests, neutral fixtures and relevant fuzz
inputs into their Mnesis owners. Keep the initial copy/rename review separate
from behavior fixes. Use workspace package metadata/dependencies and supported
path-plus-version dependencies for publishable siblings. Select the existing
Mnesis lock and pinned Rust; do not bring a parallel 0.4 dependency graph.

Keep the six existing public responsibilities: processor, request, receipt,
limits, shutdown and errors. Remove accidental private-module exports. Keep
tests under `tests/`; expose no internals solely for tests. Review arithmetic
and type bounds without weakening established limits or lossless encoding.

Acceptance: debug/release tests exercise the moved implementation, two unrelated
aggregate consumers compile, the isolated package graph contains no Labcast
policy/adapter normal dependency, and original behavioral laws still pass.
Core/store `no_std` builds remain unchanged and pass. Record all intentional
source/API differences from the archived input.

### CP2 Preserve typed outcomes and uncertain custody

Implement the outcome table above with errors callers can pattern-match without
downcasting or string parsing. Preserve domain, kernel, codec and adapter causes.
Keep absence of a cause meaningful for timeout/dropped reply. Make resource
limits and invalid configuration distinguishable. Avoid a recursive error shape
that merely nests previous processor errors or retains an unbounded retry chain.
The source's generic `start<S>` returns a handle that erases `S`; typed adapter
errors therefore need an intentional handle/channel type change. Prototype that
propagation in CP0, including inference at consumer call sites. Do not merely
replace the error box while leaving another untyped boundary in the reply path.

Acceptance: direct typed matching in an external consumer; definite encode and
domain failures leave no rows; commit-then-lost-reply yields one acceptance after
retry; late commit survives repeated empty reads, read timeout, shutdown/resume
and a failed retry. Test application fold failure after append and subsequent
reconstruction failure. An absent lookup on a fresh host is never a rollback
certificate. No implicit conflict-rebase/backoff loop is introduced.

### CP3 Prove identity, ordering and concurrent acceptance

Keep the existing canonical identity/no-op/replay laws. Replace reliance on
scheduling luck with explicit barriers at the relevant read/append boundary.
Both contenders must observe the same base before either append is released.
Use existing stores with narrow test-only fault wrappers where the actual
failure boundary requires them; do not write a competing store implementation.

Acceptance: exact duplicate race, different-ID race, same-ID/different-content
race, changed expected version, separate stream namespaces, advancing competitor
against a delayed append, and an exact retry racing its old append. Assert raw
record count/content, original receipt, domain state and handler-call counts.
An admitted handler may run more than once across losing attempts; only accepted
records are deduplicated. Extend generated histories with controlled concurrent
schedules and boundary values; the existing serial property model alone is insufficient.

### CP4 Preserve the format and expose accepted facts

Publish the minimum supported read boundary for acceptance records. A consumer
must decode/validate them and fold their ordered facts through existing stream
and projection facilities without copying the private schema. This is required
library interoperability; Labcast need not adopt a separate read model.
Keep validation at one owner. Select the concrete type/method names in CP0.

Preserve `controller.accepted.v2`, payload schema 2, envelope schema 1, request
identity and original receipts through the code move. Renaming a Rust type or
crate does not authorize a data rewrite. Separate generic record evolution from
application command/fact evolution. Show how an application keeps old variants
readable or uses existing verified transformations without changing retry identity.
All old requests must still decode because replay retains requests as well as
facts. Changing only event decoding does not establish command-schema compatibility.
Continue explicit rejection of unsupported v1/unknown/corrupt records unless a
real retained history requires a separately specified migration.

If CP0 finds a retained database on an older adapter schema, qualify that
adapter's supported migration and backup/restore path separately. Payload
compatibility does not establish SQL-schema compatibility. Invoke the adapter's
public migration API; do not copy DDL or rename stored event types with ad hoc SQL.

Acceptance: source v2 golden records load in the extracted crate and reproduce
state/receipts; unchanged-format writes remain readable by the source consumer
for rollback. Validate duplicate JSON fields, unknown schema, ID/receipt/count/
version mismatch, duplicate accepted IDs, order gaps, truncated/oversized data
and cumulative limits. No corruption path writes repair data implicitly.

An actual downstream projection consumes a multi-fact record and a no-op,
persists state and its original physical position together, restarts and resumes
without skipped or repeated facts. Use existing store checkpoint semantics.
Checkpoint after the entire record; if partial-record progress is ever offered,
it needs an explicit ordinal contract and separate justification. A successful
command receipt is not a projection-caught-up token. Reuse returned store
positions only if a demonstrated consumer needs that additional public capability.

### CP5 Complete lifecycle and resource contracts

Qualify the lifecycle/resource obligations above with bounded observations and
cleanup. Test unsupported runtime setup, closed admission, saturation, stop
racing enqueue, dropping all handles, caller cancellation, task abort/panic and
resumption with the wrong stream. Observe joins and delayed server operations;
test completion must not leave detached test tasks or live database work.

Test limits at zero where invalid, one, max-minus-one, max and max-plus-one;
include multi-byte UTF-8/JSON escapes and request-plus-fact overhead. A request
that fits 4 KiB can still produce an oversized acceptance record; this must
fail definitively before append without spending its ID. At history capacity,
old receipts and state stay readable and new requests reject predictably.
An application can calculate/reserve room for its recovery commands without
silently extending history or dropping old deduplication data.

Acceptance: each resource failure has the documented typed outcome; pending
uncertainty survives every relevant limit/shutdown failure; no unchecked size
overflow or falsely hard deadline promise; bounded teardown on supported
cooperative handlers/adapters. Timer-disabled runtime behavior is demonstrated
and documented, not assumed to be covered by `Handle::try_current`.

Required negative tests cover reserved readiness followed by worker exit, cloned
handles during shutdown, abandoned readiness reservations, and dropped/unpolled
response futures. Preserve both failing tower-actor witnesses as acceptance
requirements for the selected service. Exact queue bounds and no stranded replies
must be asserted under forced interleavings. Task completion retains terminal
causes and the original unresolved request; a synthetic `Closed` is insufficient.

#### Tracing and telemetry (CP5)

Deliver tracing and operational metrics as supported, optional features; they
are required capabilities even though applications may disable them. Use the
existing `tracing` 0.1.44 convention. Prefer the maintained `metrics` facade for
counters, gauges and histograms; CP0 verifies its selected version, compatibility,
license and dependency cost. No custom observer framework or parallel metrics API.

The application owns subscriber/recorder installation, sampling, exporters,
credentials, endpoints and flushing. The library installs no global collector,
starts no exporter task and makes no network calls for telemetry. Demonstrate
OpenTelemetry trace export through `tracing-opentelemetry` in an external example;
export SDKs stay out of the processor's normal dependencies. Disabled features or
absent collectors must preserve the command contract without initialization errors.

Capture each caller's span at admission and carry it through the mailbox. Use
async-safe instrumentation across awaits. Separate request spans from the worker's
lifetime, preserve the parent/link across caller cancellation, and prevent one
caller's context leaking into another request. Replayed events do not emit new
command-acceptance events. Never add trace context to persisted request identity.

| Instrument | Required meaning |
| --- | --- |
| Trace stages | Admission, queue wait, load/replay, decision, append, reconciliation and shutdown; record typed outcome category. |
| Counters | Submitted/admitted requests, rejection reason, newly observed acceptances, duplicate receipts, conflicts, unknown outcomes, reconciliations and worker failures. |
| Gauges | Queued/reserved/in-flight calls and hosts with unresolved writes; shared across clones with cleanup on every exit path. |
| Histograms | Queue wait, execution/store-operation duration, reconciliation duration and shutdown duration, with documented units. |

Publish stable names, units, outcome definitions and a finite label vocabulary.
No request/aggregate IDs, payloads, arbitrary error strings or trace IDs as metric
labels. No command/fact payloads in default traces. In-process metrics count
observations, not an authoritative global ledger: a commit with a lost response
can remain uncounted until observed. A duplicate response is not another acceptance.
Processor timings describe its stages; adapter timings remain with the adapter.
Metric reads are observational, not an atomic replacement for state/receipt queries.

Acceptance: a recording subscriber/recorder checks parentage across concurrent
callers, exact outcomes for controlled workloads, no duplicate acceptance counts,
and balanced shared gauges after cancellation, worker failure and shutdown.
Exercise all four tracing/metrics feature combinations and no-collector operation.
An application-owned exporter example demonstrates trace export and metric
collection. Verify exporter unavailability with bounded/nonblocking application
configuration does not change receipts or admission; document that arbitrary
blocking subscriber callbacks can stall the task. Measure disabled, enabled but
uncollected, and collected overhead in CP6; establish limits before optimization.

### CP6 Qualify adapters, recovery and performance

Use the real in-memory, Fjall and Postgres adapters. The package's normal
dependencies remain adapter-independent. Nix owns disposable service lifetime;
Rust integration tests own assertions. Extend existing Postgres CI facilities
to execute processor-owned tests, rather than treating adapter conformance as
processor acceptance or adding another development harness.

Required backend matrix:

| Profile | Observable acceptance |
| --- | --- |
| In-memory | Complete canonical sequence, boundary and deterministic concurrency laws. |
| Fjall | Write/close/reopen, process loss around acceptance, exact retry, full-history and corrupt-record behavior under its documented durability policy. |
| Postgres | Explicit adapter migration/open; independent SQL observations; duplicate/content/CAS races across independent hosts; loss before append, during transaction and after commit; process/database restart; pool/resource cleanup. |
| Accepted-fact consumer | Public decoder plus actual subscription/projection, multi-fact/no-op records, restart/checkpoint continuity and lost-final-notification recovery with no later write. |

Use supported existing fault facilities. A wrapper that loses a reply after
an actual committed append proves the processor branch; distinguish that from
a wire/transaction fault. Retain adapter-native commit uncertainty evidence and
add at least one actual database failure composition. Every required database
test must fail if its service is unavailable; no environment-based green skip.

Before tuning, measure empty/1/128/1,024/4,095/4,096-record histories; small and
near-limit valid records; new/no-op/duplicate/mismatch/conflict/retry/read calls;
queue sizes 1/8/128 and 1/16/128 concurrent callers. Include multiple hosts on
one stream and independent streams. Use selected profiles rather than blindly
running the full Cartesian product. At minimum every listed boundary is covered.

Record p50/p95/p99 operation latency, throughput, peak RSS, allocations where
supported, queue rejection, database reads/bytes, task/connection cleanup,
compile time, artifact size and normal dependency count. Use at least 10 warmup
iterations and 30 timed samples for steady-state comparisons; record dispersion,
machine, database settings and raw results. Compare the mechanical extraction
against the source baseline under the same dependencies and workload. Separately
measure the dependency upgrade so its cost is not hidden by the rename.

Run a 15-minute mixed acceptance/retry/failure profile with multiple hosts and
finite streams. Assert no duplicate acceptance, lost receipt or order break,
and complete cleanup. Near-capacity read and duplicate recovery must remain
usable under the declared limits. Set numeric latency/memory envelopes from CP0
measurements before optimizing; investigate reproducible regressions above 10%
and the measured noise floor. The threshold is an investigation gate, not a
Labcast product SLA. No performance improvement is claimed without evidence.

A cache/snapshot optimization must retain receipt identity, invalidate across
external writers and preserve replay validation. Compare the current full
replay and double-load costs first. Existing store snapshot support alone does
not snapshot the receipt ledger. Do not add compaction or a cache by default.

### CP7 Complete public documentation, packaging and CI

Document the domain, supported profile, source migration, persistence format,
typed outcomes, exact retry, crash custody, runtime setup and application
obligations. Provide a minimal neutral consumer that submits, repeats, reads,
stops and resumes. Add a second consumer proving the public accepted-fact path.
Examples must use only exported APIs and must execute in the normal checks.
Include a Tower consumer and application-owned tracing/OpenTelemetry and metrics
setup. Document default features, all tracing/metrics combinations, instrument
names/units, bounded labels and supported middleware ordering. Keep exporters in
the example/application owner. Explain startup versus readiness versus store health.

Add `mnesis-command` explicitly to the evolving 0.x stability tier. Its public
Tower/Tokio/Serde/error types and dependency compatibility are part of its documented
surface; do not apply core's narrower public-dependency list accidentally.
Use existing release-plz, Cargo packaging and Nix checks. Verify crate metadata,
README/license inclusion, docs.rs build and isolated package features without
workspace feature unification or published workspace-hack edges.

Port the two fuzz bodies and useful corpus into the existing Mnesis stable
Bolero/AFL arrangement where applicable. Do not import a new cargo-fuzz/nightly
workflow or add another isolated workspace by habit. Preserve request decoding
and stateful history invariants, canonical test ownership and release lock
synchronization. Add the new package to explicitly enumerated feature/mutation/
release test lists; normal workspace membership does not cover every CI job.

Acceptance: rustdoc examples, debug/release owner tests, Clippy/format, isolated
feature checks, current unwaived dependency/license checks, fuzz replay and
bounded campaigns, reviewed mutation evidence and `nix flake check` pass at the
delivery source. Keep existing compile-fail and core `no_std` gates. Linux
native Postgres results are mandatory. macOS native package tests remain
required; mobile/browser runtime support is not inferred from kernel builds.

### CP8 Minimize, review, merge and publish

Perform two fresh audits of the affected processor, shared owners and consumer
contracts. Start from source and invariants. Resolve credible untested findings;
do not expand into unrelated project audits. Remove redundant copies, private
API leaks, unused capabilities and unsupported claims. Re-run affected checks
after changes; repeat broader tests only for a changed dependency or remaining risk.

Deliver focused PRs in dependency order through the established protected
workflow. Resolve review and CI failures without bypassing gates. Let the
existing release process choose the actual compatible package version; 0.5.3
is the inspected dependency baseline, not a promised new-crate version. Verify
the new package's registry ownership, release permissions and first-publication
behavior. No placeholder release or manual token handling is required by this PRD.

Acceptance: exact reviewed heads, passing required jobs, protected merge
commits/trees, successful publisher run, tag, package checksum and matching
registry source recorded. A clean external registry-only consumer resolves one
compatible kernel/store family and executes persistence/reopen/exact retry
against real Postgres. Compile-only or path-patched consumers do not count.

### CP9 Verify server consumption and close delivery

Prepare consumption in `/Users/joel/Code/labcast/labcast-server` after the public
API is stable; complete it only against the verified published release. Inspect
its current state again before edits. At planning time it has an empty Rust
workspace, no runnable server and no local controller dependency to replace.
The source is the separate archive, not the server or mobile repository.

Use a small unpublished neutral integration consumer following the server's
existing Nix/Cargo conventions. Exercise the released package's public contract
and a real Postgres lifecycle. This proves server-side dependency compatibility,
not business integration or a production-ready server. Do not rebuild an old
backend or invent business rules to supply this witness. Generic canonical laws
remain in Mnesis; later server design owns access, worker fencing and effects.
If an actual server consumer exists by implementation time, migrate that owner
and preserve its validated IDs and behavior instead of adding redundant probes.

Keep `controller-archive` as immutable provenance until delivery and rollback
evidence are verified. Record its retirement as an active implementation; do
not delete the only archival copy. Check actual consumers before removing any
obsolete patches, fixtures or build entries. No permanent forwarding crate and
no changes to the `labcast` mobile repository are required by this feature.

Acceptance: the server consumer's reviewed change and required checks pass;
registry-only resolution is verified; Mnesis is the sole maintained implementation;
preserved provenance and history migration/rollback instructions are accessible. Record
both repositories' exact delivery evidence and update this PRD/backlog. Full
feature completion requires CP0–CP9, including publication and downstream
verification. If either repository exposes a product defect, reopen its owning
requirement and the affected release/consumer checks.

## Change budgets and hypotheses

Budgets are pre-edit review limits, not reasons to discard necessary tests.
Record actual added/deleted lines, files, public types and dependency changes
per slice, including untracked files. Explain a required extension before edits.

| Slice | Expected owners | Initial budget |
| --- | --- | --- |
| CP0 | Feature records, frozen inputs and small disposable API consumers | 8 record owners; no product edits or permanent probe tool. |
| CP1 | `crates/command`, root manifest/lock/Hakari, necessary build registration | Copy existing production/tests with provenance; at most 150 net new production lines beyond the 937-line baseline; six public responsibility types. |
| CP2–CP5 | Processor error/model/host/engine and owning tests | Each focused change: at most 8 files, 250 net new production lines, 500 test lines; at most two added public types per slice with consumer justification. |
| CP6 | Owning integration tests/benches and existing Nix Postgres setup | At most 10 files and 900 test/benchmark/Nix lines; no new normal dependency or framework. |
| CP7 | README/rustdoc/examples, existing CI/fuzz/package/stability owners | At most 14 files and 500 authored lines, plus moved corpus/fixtures. Generated lock changes measured separately. |
| CP8–CP9 | Delivery records, release-generated files, server consumer and archive status | Focused PRs; server at most 10 files and 350 new consumer/config lines. No mobile changes or destruction of the archive. |

Cumulative checkpoint: target at most 1,600 production lines and ten root-exported
types in the new crate. Approved dependency scope adds the existing Tower service
contract and optional workspace tracing, plus one qualified optional metrics
facade and the existing Tokio `PollSender` helper (`tokio-util`) to the source's
set. The helper owns readiness reservations rather than custom future machinery.
No normal exporter SDK, tower-actor dependency or second
implementation. Telemetry uses existing ecosystem interfaces, not new public
observer types. These are estimates to
challenge. A larger necessary design needs a recorded counterexample, alternatives
and an amended budget; line-count compliance cannot replace correct contracts.

| Hypothesis | Falsifier | Rollback boundary |
| --- | --- | --- |
| H1: This is a portable infrastructure owner. | Neutral consumers need Labcast policy, a handler registry or a replacement aggregate model. | Reassess only the extraction boundary; retain original source. |
| H2: Current shared owners support safe retry. | A timeout/late commit yields two acceptances or a read falsely proves rollback. | Restore last proven outcome path; retain failing regression. |
| H3: Typed errors can preserve causes economically. | Callers still downcast, causes disappear, or generics force unrelated policy into the API. | Revise error design before public release. |
| H4: v2 records can survive a code-only move. | Old records change identity/receipt/facts or rollback readers fail. | Keep old writer format; no deployment/data rewrite. |
| H5: A public acceptance reader avoids copied schema. | A projection must duplicate JSON validation or invent event positions. | Rework CP4 boundary before publication. |
| H6: The bounded profile is operationally useful. | Near-capacity retry exceeds its declared envelope, memory grows beyond measured bounds, or recovery cannot fit. | Reassess limits/algorithm with preserved comparison; no silent pruning. |
| H7: Existing environment and release facilities suffice. | New custom tooling or workspace-only dependencies are required to consume the archive. | Repair existing package/build owners before releasing. |
| H8: Tower and telemetry preserve the processor contract. | Readiness leaks permits, middleware changes retry identity, a reply hangs on shutdown, trace context crosses requests, or metric collection changes acceptance. | Keep the owning worker and typed outcomes; repair the service/instrumentation boundary before release. |

## Verification and completion record

The four required test categories are sequence/protocol, lifecycle, defensive
boundary and linearizability/isolation. Apply them before extra test methods.
Property, fuzz, mutation and performance work supplement their owning laws.
Use Loom/Miri only for relevant custom concurrency/unsafe behavior; the initial
design delegates synchronization to Tokio and contains no unsafe implementation.

After every actual test run, record three distinct reviews: (1) contract and
counterexample, (2) execution/selection/counts and real service boundaries,
(3) assertions, source identity and whether the test could pass with the defect.
Do not weaken assertions, filters, deadlines, workloads or gates to get green.
Mutation results require review of the actual first failure; fixture failure is
not a caught product mutation. Preserve failures and skipped/unavailable coverage.

Each CP row needs evidence paths, source/lock identity, command, environment,
observed counts, negative controls and remaining limitations. CP8 additionally
records merges/publication; CP9 records the downstream registry consumer and
retirement of the archive as an active implementation. Keep facts in the feature research/delivery
record and link them from the backlog. A planning-document branch, passing local
build or completed Postgres release does not mark this new feature delivered.
