# Command processor research and evidence

Updated: 2026-10-21. Contract owner: [command PRD](../command.md).
This records planning and CP0/CP1 local qualification. Protected delivery and
CP2–CP9 remain outstanding; the latest owning sections record actual results.

## Input identity and method

Mnesis destination is branch `docs/command-prd`, based on fetched `origin/main`
at `27de3b4a4aad59909ffaccdf6fa83e8b256ccca6` (release changelogs, PR408).
The source is `/Users/joel/Code/labcast/controller-archive/crates/controller`.
It was reviewed and tested before the move from `labcast/crates/controller`.
The archive has no Git revision. The
[input manifest](evidence/labcast-inputs.sha256) fingerprints every crate file
plus root Cargo manifest/lock, flake lock and toolchain. Its SHA-256 is
`1e4be80cc62b9c814793456cea53baf5fae1d583d060dfb787ef3adac5fdff6a`.
All 30 input hashes were verified from `controller-archive` after the user's
location correction; all match. Recheck from that root before extraction. CP0
must retain a durable source receipt; a checksum alone cannot restore a file.
This agent did not move or modify source. The existing baseline logs retain
their actual execution paths and are not presented as runs from the archive.

The archive README explicitly records removal from the mobile repository.
`labcast-server` is the future server consumer, not the source. Its AGENTS.md,
README and Cargo manifest were inspected: no runnable server, no controller,
empty workspace, and no initial Git commit at inspection. Those instructions
reserve business design for the server owner. CP9 therefore specifies a neutral
registry integration witness, or migration of an actual consumer if one exists
then. `labcast` is mobile and receives no changes under this feature. The archive
retains `flake.lock` but no `flake.nix`; the old execution command below is
historical and cannot be rerun there unchanged. Use the existing Mnesis Nix
environment for future compatibility checks; do not invent archive tooling.

Read the canonical devrandom-research skill and full protocol, both repository
instructions and Labcast's owned skill/protocol. Resumed Mnesis's existing ledger
with its initializer. Used Bombay's backlog/feature PRD model and inspected the
actual Mnesis build/release owners. The current server/archive documents take
precedence over historical controller integration claims. No workflow change
is proposed, so a new cross-project environment redesign is unnecessary.

Review covered all four production files, all test targets and their helpers,
the independent history model, both fuzz targets, corpus/fixture inventory,
both manifests/locks, root public exports, runtime setup and recorded integration
limits. Relevant Mnesis 0.5.3 owners inspected: aggregate/store error and execution
contracts, repository append/commit ordering, Postgres errors, stability tiers,
workspace features, Nix source filtering/native Postgres tests, feature CI,
fuzz workspaces and release-plz publishing/lock synchronization.

## Locked dependency verification

These are observed lockfile versions, not requested version ranges or latest
release claims. The destination lock remains authoritative during extraction.

| Dependency | Labcast source | Mnesis destination | Consequence |
| --- | --- | --- | --- |
| Kernel/store/inmemory | 0.4.0 | 0.5.3 | Run actual compatibility consumers; reject duplicate incompatible type families. |
| Rust | 1.99.0 | 1.99.0 | No toolchain upgrade is needed for the move. |
| Tokio | 1.53.1 | 1.53.1 | Reuse task/channel/timer APIs; inspect isolated features because Mnesis requests `full`. |
| bytes | 1.11.1 | 1.12.1 | Lossless acceptance bytes and dependency cost require comparison. |
| serde / serde_json | 1.0.228 / 1.0.149 | 1.0.229 / 1.0.151 | Golden stored records and exact retry equality must survive the upgrade. |
| thiserror | 2.0.18 | 2.0.19 | Reuse typed error derives; no replacement error dependency. |
| futures | 0.3.32 | 0.3.33 | Existing test/stream dependency. |
| proptest | 1.10.0 | 1.11.0 | Compare generated histories without silently narrowing cases/ranges. |
| SQLx | No current source-workspace dependency | 0.9.0 | Postgres remains a test/application adapter, not a processor normal dependency. |

Mnesis uses MIT OR Apache-2.0 package metadata. Labcast's controller manifest
has `publish = false` through its workspace and no license field; no controller
license file was found. CP0 records source rights and license attribution before
publication. Existing destination dependencies are already selected by Mnesis;
no new framework candidate is approved by this research. Current advisory,
transitive license, artifact-size and compile-cost checks belong to the final
selected graph. The existing host gate below does not qualify the future package.

The sibling `mnesis-bombay` manifest still selects Mnesis 0.3.1 and Bombay/Behavior
integration. It is a distinct execution integration and is not a prerequisite
or suitable package to import for this Tokio-only extraction. Its version is
not evidence that the new command package is compatible with its public types.

## Source audit

Paths in this table are relative to the fingerprinted Labcast crate. They are
source observations, not all reproduced defects. Preserve useful existing laws.

| Owner and location | Observation | Disposition |
| --- | --- | --- |
| `src/lib.rs:9–13` | Private modules expose six responsibility types; no Labcast domain dependency. | Extract this boundary; keep mechanism private. |
| `src/model.rs:31–68` | Request identity includes expected version; encode/decode uses a 4 KiB bound and typed equality round-trip. | Preserve typed identity and test lossy/custom serialization. |
| `src/model.rs:75–130` | Receipt stores ID/version/fact count. Errors box sources; `Capacity` covers unrelated remedies. | CP2 types sources/reasons without inventing a retry-safe catch-all. |
| `src/model.rs:134–234` | Record contains original request/receipt/facts. State retains requests/receipts and folds facts; duplicate IDs/cumulative overflow poison integrity. | Preserve atomic acceptance and cross-record validation. |
| `src/model.rs:236–288` | Wrapper aggregate counts physical records separately from domain facts; no-op becomes an accepted record; 4,096 receipts is a permanent stream ceiling. | CP5 documents/tests both capacities and recovery room. |
| `src/model.rs:291–345` | Private codec validates event type, envelope/payload schema, physical/receipt version, ID and fact count. External consumers cannot access it. | CP4 exposes one validated acceptance read boundary and keeps v2 compatible. |
| `src/model.rs:17–19,192–197,224,246` | Hard limits, saturating arithmetic and guarded fact addition differ from destination arithmetic conventions. | Review actual proofs; use checked size/count operations where required, with negative tests. |
| `src/engine.rs:57–80` | Every load replays; head advancement settles uncertainty only once the old conditional append cannot win. | Preserve monotone-history assumptions; stale/empty reads do not settle. |
| `src/engine.rs:83–149` | Pending request is installed before execution; definite retry failure restores older pending state; unclassified writes remain unknown. | Preserve custody while changing error representation. |
| `src/engine.rs:151–196` | Existing receipt precedes version rejection; conflict reload recovers original receipt. | Deterministic race witnesses must assert these actual branches. |
| `src/engine.rs:206–230` | Known originals remain accessible; a persisted-ID retry performs an original lookup and another load in submit. | Measure before removing the second load; do not weaken external-writer validation. |
| `src/host.rs:29–40,156–168` | Borrowed shutdown report binds raw stream bytes and retains unresolved request. It carries no database history ID and is not persistent storage. | Explicit same-store caller obligation and abrupt-loss recovery test. |
| `src/host.rs:171–185` | Checks queue bounds, nonzero timeout and current runtime, then spawns the host. Timer availability is not checked. | CP5 proves/documents runtime preconditions using supported Tokio APIs. |
| `src/host.rs:117–121,143–168` | `start<S>`/`resume<S>` erase the store type from the returned handle. Typed adapter errors cannot be restored by editing the error enum alone. | CP0/CP2 prototype the error type through the actual public handle, channels and consumer inference. |
| `src/host.rs:224–269` | Validation happens before enqueue; `try_send` bounds waiting calls; dropped admitted reply becomes uncertain. | Test pre/post-admission cancellation and distinguish queued/committed status. |
| `src/host.rs:273–308` | Host serializes calls; stop rejects queued calls, observes current operation and returns unresolved work. | Test interleavings, abort/drop and cleanup; no supervisory framework. |
| `tests/recovery.rs:317–353` | Two Tokio hosts are joined, but no barrier forces shared-base reads. Sequential execution satisfies assertions. | Real test gap; CP3 adds boundary overlap and a negative control. |
| `tests/recovery.rs:140–244,407–650` | Controlled delayed appends, empty reads, competitor advancement, task loss and retry rejection assert persisted results. | Substantive existing evidence; move and preserve it. |
| `tests/support/history.rs`, `tests/properties.rs` | Independent list model, 256 generated cases, fixed non-vacuous prefix; schedules are serial. | Keep model; add controlled concurrent histories and capacity boundaries. |
| `tests/unit/model.rs` | Codec corruption, duplicate ID, fact bounds, 4,096 receipts and round-trip limits have owning tests. | Preserve; add old/new reader interoperability and missing cases from CP4. |
| `tests/adaptation.rs` | Two independent aggregates and namespaced IDs use public API; domain failure, no-ops and handler counts checked. | Keep neutral domains; migrate consumer imports, not product policy. |
| `tests/recovery.rs:671–698` | Ignored optimized measurement covers 128 small records and duplicate submissions only. | Useful baseline; insufficient for maximum history, allocations or Postgres claims. |
| `fuzz/` | Request parser and serial history model use libFuzzer; Mnesis already has stable replay/AFL owners. | Move invariants/corpus into existing destination facilities. |
| `README.md` | Still mentions tests living in a removed backend and commands requiring migrated consumers. | CP7/CP9 replace stale claims with current runnable evidence. |

Mnesis's `crates/store/src/repository.rs` calls raw append and then
`commit_persisted`; a post-append fold error must not be called a pre-write failure.
`StoreError` already preserves adapter/encode/decode/kernel types.
`AppendError` distinguishes conflict from an adapter source; it has no universal
known-rollback variant. Postgres's `CommitUnknown` preserves a SQLx cause.
These are reasons to reuse existing contracts and remain conservative, not to
create a new generic transaction-outcome framework inside the processor.

The private record format also means standard domain-event consumers cannot
simply decode a stored frame as `EventOf<A>`. A multi-fact acceptance is one
physical record. CP4 must preserve that distinction for projection checkpoints.
This finding is based on actual exports/codec source; no production projection
defect was reproduced in a current server application.

## Primary sources and alternatives

Access date: 2026-10-21. Versioned APIs below are technical inputs. Moving web
documentation and repository READMEs establish terminology only; they are not
dependency pins or guarantees inherited by Mnesis.

| Source | Relevance and limit |
| --- | --- |
| [Axon command dispatchers](https://docs.axoniq.io/axon-framework-reference/development/commands/command-dispatchers/) | A command bus selects the appropriate handler. This processor is already bound to one aggregate; no routing layer is justified. |
| [Brighter implementation repository](https://github.com/BrighterCommand/Brighter) | Established `CommandProcessor` terminology for infrastructure around application handlers. It is a .NET messaging framework, not a Rust dependency candidate. Its GitBook page failed to open in this run; the primary repository was read instead. |
| [Rust cqrs-es 0.5.0 CqrsFramework](https://docs.rs/cqrs-es/0.5.0/cqrs_es/struct.CqrsFramework.html) | Comparable load/replay/handle/persist sequence, plus query dispatch. Replacing Mnesis contracts with another aggregate/store model would broaden this move and does not establish matching receipt/uncertainty laws. Not selected. |
| [Akka event sourcing](https://doc.akka.io/libraries/akka-core/current/typed/persistence.html) | Separates command decisions, stored events and recovery folds. Useful conceptual comparison; importing another runtime is outside scope. |
| [Stripe idempotent requests](https://docs.stripe.com/api/idempotent_requests) | Illustrates that retry semantics are explicit API contracts: its v1 saved-response behavior includes failures after execution begins. Our accepted-record-only semantics are different and must be stated. No Stripe code/dependency is proposed. |
| [Tokio 1.53.1 timeout](https://docs.rs/tokio/1.53.1/tokio/time/fn.timeout.html) | Cancellation drops the future; a non-yielding future can exceed its deadline, and an absent timer causes panic. Relevant to host startup and bounded-shutdown claims. |
| Tokio 1.53.1 local registry source, `src/time/timeout.rs`, `src/runtime/task/join.rs`, `src/sync/mpsc/` | Locked implementation fallback for task/queue semantics where versioned web pages were unavailable. Reuse channels, watch, oneshot, timeouts and observed tasks. |
| [Mnesis inspected source](https://github.com/devrandom-labs/mnesis/tree/27de3b4a4aad59909ffaccdf6fa83e8b256ccca6) | Existing command repository, errors, adapters, projection and release owners are the implementation authority. No additional repository/codec/runtime framework is justified. |

The smallest supported-library choice is the current Rust standard collections
and checked arithmetic, Tokio synchronization, Serde JSON and thiserror on top
of Mnesis. Do not replace the receipt map with a cache that evicts accepted IDs.
Do not add backoff/retry middleware that changes identity or expected version.
No selected requirement needs another actor framework, DI registry or message
broker. Revisit a replacement only against an actual falsifier and dependency
cost/maintenance/license assessment at the selected revision.

## Executed baseline

Environment: Darwin arm64, Apple M4 Pro, 25,769,803,776 bytes RAM (24 GiB).
Commands ran from the former Labcast root through its locked Nix development shell,
before the controller was moved into the archive.
Source and lock hashes were checked after both runs. The builds reused or built
artifacts normally; the test bodies executed. No Postgres process was started.

```sh
nix develop path:. --no-update-lock-file --command \
  cargo test --locked -p labcast-controller
nix develop path:. --no-update-lock-file --command \
  cargo test --locked --release -p labcast-controller --test recovery \
  measured_reload_and_duplicate_workload -- --ignored --exact --nocapture
```

| Run | Actual result | Raw evidence |
| --- | --- | --- |
| Debug package | 6 model + 1 adaptation + 1 property + 19 recovery tests passed. One measurement ignored. Zero doctests exist. Property target took 1.48 s; recovery target 0.21 s. | [tests.log.gz](evidence/tests.log.gz) |
| Existing release measurement | One selected test passed; 19 other recovery tests filtered. Five batches of 100 duplicates over 128 records: 5,098 / 5,109 / 5,112 / 5,075 / 5,040 µs. | [performance.log.gz](evidence/performance.log.gz) |

Median batch time is 5,098 µs; min/max are 5,040/5,112 µs. Dividing by 100 gives
50.98 µs per duplicate for the median batch, not a per-request latency percentile.
Setup and compilation precede timing. One duplicate precedes the measured loop;
there is no sustained warmup, allocation measurement or service-level deadline
experiment. This small baseline cannot justify tuning, a throughput SLA, a
near-capacity claim or the cost of an actual Postgres round trip.

Three reviews after the debug run:

1. Contract/counterexample: delayed work outlives the client future; the tests
   inspect actual rows, receipts and pending originals. No-op/replay tests check
   handler-call counts. Confirmed the real concurrent-overlap gap remains.
2. Execution: exit zero, 27 actual tests passed; one intentional ignore, zero
   doctests, no database evidence. The property is one test with configured
   256 cases; it is not 256 separate test functions or a parallel schedule test.
3. Independence/source: checked fingerprinted sources/locks after execution;
   no assertions, timeouts, filters or property ranges changed. Source inspection
   rejects treating the unforced race as a linearizability proof.

Three reviews after the optimized run:

1. Contract/counterexample: each duplicate receipt equals the original and final
   state is 128. The measured path invokes the actual controller and replay.
2. Execution: `--ignored --exact` selected the named test; it ran five printed
   samples, exited zero, and did not execute the 19 filtered recovery tests.
3. Independence/source: input hashes still match. Setup is outside timing;
   small in-memory data, limited samples and absent allocation/database data
   are retained limitations. This test alone does not prove no extra append.

No runtime, source, dependency, CI or Labcast documentation was changed. Nix's
normal shell entry may refresh its existing development environment. Required
future package, real-database, migration, fuzz/mutation, capacity, release and
registry-consumer checks are enumerated in the PRD and have no passing claim here.

## Planning review and next action

Planning budget: at most 16 documentation/evidence files and 1,450 handwritten
added lines, amended from 1,200 for the user-required Tower and observability
contracts, plus actual logs/fingerprints and disposable probe evidence. Implementation budgets are separate
and owned by the PRD. No public type or product dependency was added here.

First review starts from the source: verify identity/uncertainty, format and
public-fact access, lifecycle/limits and the archive/server/mobile boundaries. Second
review starts from acceptance: every finding has a CP owner and observable
negative witness, dependency edges are reciprocal, publication precedes final
registry adoption, and no test or document claims delivery of absent code.
Record actual planning validation in the research experiment ledger.

Next implementation action, when requested: CP0 freeze the existing archive,
verify rights/name and prototype only the required typed-error/acceptance-reader
compatibility seams. Preserve the archived implementation until the
published package and downstream consumer are verified. The completed Postgres
persistent goal is not reopened by this planning task.

## Planning validation

The source-location correction was checked against all 30 unchanged baseline
inputs and the archive/server ownership documents. Their separate
[location fingerprints](evidence/locations.sha256) are relative to
`/Users/joel/Code/labcast`. They record observations, not a server Git revision.
Both final planning reviews cover the corrected ownership: source-to-requirement
review preserves identity, pending writes, format and lifecycle; acceptance-to-
source review checks negative witnesses, dependencies, server consumption and
publication. Neither review certifies implementation of CP0–CP9.

The first existing Mnesis host gate failed because the Nix volume ran out of
space (`os error 28`). Its [raw log](evidence/flake-first.log.gz) is retained.
No test or product fix followed. Nix released temporary files; the identical
gate was retried with `--max-jobs 1` to reduce simultaneous build disk use.
The initial run did execute the existing fuzz replay targets (2/1/1 passed).
Each of those three target runs received three reviews: (1) the shared bodies
call actual frame/CBOR/value decoders and exercise malformed-input handling;
(2) the log records the named targets and their counts, not a fuzz campaign or
processor test; (3) unchanged bodies primarily check panic freedom, so they do
not prove acceptance identity, persistence, or semantic round trips. No broader
passing test result is inferred from that failed gate.

The baseline logs are compressed without editing their contents. Decompressed
SHA-256: tests `4dee315798f53ba727d23fa6c929a88a66a171d377195e7452bc7ee9192dce17`;
performance `5793134e8f1f9b96bb60c8b4e01114d47345eb8ec7393e2f80a9ba51b32762ae`.
Local document links and all 21 reciprocal backlog rows pass inspection. The
staged whitespace check passes. Production, tests, manifests, locks, workflows
and both downstream locations are unchanged. The original Postgres worktree's
pre-existing research edits remain untouched.

The serial host gate completed: twelve aarch64-darwin checks passed (six reused
existing results). Workspace nextest executed 1,594 tests, all passed in 517.411 s;
four existing compile-fail cases excluded. [Raw log](evidence/flake-serial.log.gz).
Three reviews: (1) unchanged store/model tests exercise actual product owners;
(2) selection excludes four compiler fixtures and Darwin database guards are not
real PostgreSQL coverage; (3) no product source, filter, assertion or deadline
changed. Fuzz results were reused, not new executions. This qualifies the existing
workspace only. Two planning audits and local link/dependency checks completed.

The later user-requested [Tower fit experiment](tower.md) records actual tests
against the archived engine and destination dependencies. Its two failed lifecycle
requirements remain explicit; the PRD now requires interface qualification in CP0.

## Required Tower interface and observability

User selection after the fit experiment: Tower Service, tracing and telemetry are
required PRD capabilities. Interface qualification remains CP0 work; the failed
package experiment is not a passing result for the proposed implementation.
No product, manifest or lockfile changed in this documentation update.

Observed existing workspace owners: Tower 0.5.3, tower-service 0.3.3, tracing
0.1.44 and tracing-subscriber 0.3.23. The Postgres adapter already uses an optional
tracing feature; reuse that convention. No metrics or OpenTelemetry package is
currently locked. Primary sources checked on 2026-10-21:

| Source | Use and limit |
| --- | --- |
| [Tower Service 0.5.3](https://docs.rs/tower/0.5.3/tower/trait.Service.html) | Readiness can reserve resources; service clones do not inherit a reservation. Required admission/shutdown semantics need separate executable evidence. |
| [tracing Span 0.1.44](https://docs.rs/tracing/0.1.44/tracing/struct.Span.html) | Carry per-request context and use async instrumentation rather than holding an entered-span guard across await. Application owns collection. |
| [metrics 0.24.6](https://docs.rs/metrics/0.24.6/metrics/) | Candidate library facade for counters, gauges and histograms with consumer-owned recording. CP0 must qualify and lock it; not installed by this task. |
| [tracing-opentelemetry 0.34.0](https://docs.rs/tracing-opentelemetry/0.34.0/tracing_opentelemetry/) | Application/example bridge for distributed traces. No exporter SDK in processor normal dependencies; compatibility/version selection remains CP0/CP7 work. |

Two documentation review passes checked contract ownership and acceptance:
Tower readiness is capacity reservation, not admission or database health;
caller timeout is not rollback; default retries are disabled; clones share the
worker; telemetry does not become a receipt ledger or a new persistence schema.
Optional feature support is required even when disabled by a consumer. Metrics
need bounded labels and explicitly defined observations. Tests cover context
propagation, shutdown races, gauge cleanup and disabled/enabled collection.

## CP0 active — immutable source and authorization

The user explicitly authorized CP0–CP9 implementation and publication on
2026-10-21. This authorization supplies permission to extract their archived
Labcast source into Mnesis's MIT OR Apache-2.0 package; preserve source provenance
and notices. The unpublished source itself still has no license declaration.
No separate third-party source or rights are asserted.

All 30 baseline hashes match. The archive now has 2,490 files because its fuzz
corpus contains generated cases beyond the original named seeds. The complete
current [source receipt](evidence/source.tar.gz), SHA-256
`822fbfc9c95db9f6908a66b97fc0d39aa5b89457600cc4a28954d877d722dc25`, and
[per-file hashes](evidence/source.sha256) preserve those additional inputs too.
No source input was modified. Pre-goal planning/index snapshots are retained in
`/tmp/mnesis-command-start-20261021`. Existing actual baseline and 0.5.3 engine
compatibility runs remain applicable; the new service interface still needs proof.
Source receipt excludes disposable target/build directories: its tar entries are
exactly the 2,490 source/corpus files in the per-file manifest. The initial archive
also captured a fuzz build cache; that intermediate is retained only in the local
start snapshot, not shipped as source. Current registry name-index request returned
HTTP404 for mnesis-command: unregistered at observation, no reservation or ownership
claim. Existing release-plz publishes new workspace packages; no placeholder is
required. Main currently remains 27de3b4a; unrelated release PR410 is preserved.

### CP0 decisions and executable seams

Local qualification complete; protected delivery remains pending. Exact disposable
sources/locks/logs and three reviews per actual run: [CP0 evidence](evidence/cp0.tar.gz).
Nix/Rust1.99 prototype passed 12 tests (six source model plus six lifecycle/type
cases) and a focused external reader test. Zero ignored/filtered in those runs.
The reader delegates v2 validation to the original codec and borrows ordered facts;
only a private parts accessor changes in the copied model. This is a public
reader seam, not yet full projection or old/new writer qualification.

Selected production direction:

- `CommandProcessor` implements Tower Service over its owned queue/worker.
  Retain concrete adapter error parameter in the public handle/reply type.
  Flat typed cause variants preserve domain/store/kernel/codec failures without
  recursively retaining earlier processor errors. Pure decision errors cannot
  contain the adapter; promote them once at the engine boundary.
- Observed native task completion retains original terminal causes. Prototype
  intentionally verifies panic73 through its JoinHandle and an unknown reply;
  it does not falsely turn that reply into a definite pre-append rejection.
  Actual pending requests and known-unstarted queued rejections remain product
  acceptance work. The prototype's synthetic adapter cause proves type flow only.
- A synchronized lifecycle check prevents reserved permits from sending into a
  dead worker. Shutdown drains queued messages without awaiting outstanding
  readiness reservations. Caller cancellation does not discard admitted work.
- Tower cannot observe dropping a borrowed ready() future. Capacity remains
  owned by its handle until consumed/released/dropped. The PRD now states this
  actual contract; owning oneshot cancellation drops the handle. Clones do not
  duplicate a reserved permit. Reuse Tokio's maintained PollSender helper instead
  of implementing a custom polled reservation future. Its tokio-util0.7.20 was
  qualified in the disposable lock; CP1's actual workspace diff adds this package.
  The earlier "already locked" statement confused the two graphs and is corrected.
- Public read boundary: validated `Acceptance` with borrowed request/receipt/facts
  and `AcceptanceCodec`, reusing one codec validation owner. Names/shape proved
  by an external integration consumer; no public access to mutable record fields.
- Preserve original v2 format and finite limits; keep full replay until CP6
  measurements justify a change. Existing source performance run is the 0.4
  baseline; earlier 0.5.3 engine composition and this prototype establish compile/
  behavior seams, not a new performance claim. CP6 still owns numeric resource
  envelope qualification before optimization.
- Required optional metrics facade: metrics0.24.6 (MIT; declared Rust1.71.1),
  one new normal dependency rapidhash4.5.1. No exporter/SDK normal dependency.
  Its upstream latest commit observed is043577f9914223e3e2d7165d5361e7e78ad75ed2,
  2026-10-16. Existing tracing0.1.44 convention and application-owned OTel bridge
  remain selected; the bridge's dev graph is qualified with its actual example.

Initial extraction stays unpublished and release-disabled until CP8 acceptance;
otherwise release-plz could publish a partial new package automatically. CP8
removes those gates through the existing reviewed release process. No name-only
placeholder is published. Package-name availability must be rechecked then.

CP0 foundation gate freshly executed the existing workspace: nextest run
`0d713cb2-0e5d-475e-97f3-7a4dbd88d747`, 1,594 passed in504.428s,15slow,
four established compile-fail exclusions. Fuzz replay separately ran2/1/1,1s each.
Each actual invocation received three distinct reviews: actual owners/invariants,
selection/counts/service limits, and unchanged source/assertions/false-positive
limits. This is existing workspace evidence; no processor or real-Postgres claim.
All twelve Darwin flake checks passed. [Raw gate logs and reviews](evidence/cp0-foundation.tar.gz)
also preserve verification of all2,490 frozen files. The normal hook may reuse
these unchanged product outputs; that is not another runtime execution.

## CP1a mechanical extraction

Original four production files copied into crates/command and renamed only at
Rust Controller/import references. All937 production and1,891 test/helper lines
are retained; workspace manifests align sibling0.5.3 types and existing locked
Serde/test dependencies. Publish/release disabled until CP8. No archived nightly
harness or generated corpus is installed into product test ownership.
Owning all-targets run:27 passed (6model+1adaptation+1property+19recovery), one
existing ignored measurement. Three reviews: (1) unchanged acceptance/no-op/retry/
late-write and raw-record assertions preserve source invariants; (2) original
suite ran, property still256 cases, no Postgres or omitted measurement credit;
(3) names/imports/dependencies change only, so original unforced concurrent race
remains CP3 work. No assertion/filter/timeout/property-range weakening. Tower
implementation and owning regression tests follow before CP1 acceptance.
CP1 Tower pre-change attempt initially stopped at Cargo's --locked guard after
adding the existing Tower dev dependency. Zero tests/compilation executed; this
is not a defect witness. Reviewed dependency selection, zero execution and test
sources; corrected new tests to inspect the same physical stream they submit to.
The next offline resolution updates only the new member's lock entry before
checking the missing service interface. No assertion was weakened.
Resolved pre-change Tower check fails with E0599: CommandProcessor does not
implement Service, so ready/call are unavailable. Zero test bodies executed.
Three reviews: the failing boundary is the required public trait, offline lock
resolution adds only the existing Tower dependency to the new member, and all
three new cases target actual processor/task/store objects with no fake interface.
The implementation follows without changing their assertions or deadlines.
CP1b focused Tower cases now pass3/3 (20 recovery cases filtered). Three reviews:
(1) real service reservations/worker exit, clone capacity and abort-before-first-
poll reproduce required lifecycle boundaries; (2) only the three new cases ran,
not the complete original suite or real database; (3) assertions/deadlines stayed
unchanged after E0599, and the same raw stream is inspected. Owned Worker Drop
sets closed admission before receiver destruction, preventing stale-permit enqueue;
queued rejections drain without waiting for unconsumed reservations. Add actual
persisted cancellation/order and terminal-cause cases before broader owner checks.
CP1b full owning suite passes32 tests (6model+1adaptation+1property+24recovery),
one original ignored measurement, no filter. Three reviews: (1) raw acceptance
records retain exact[1,2,3] order after dropped unpolled replies and exact retry
adds no record; original panic73 is observed through the native join, while the
admitted reply retains its unknown command identity; (2) owning suite and new
five Tower cases ran, no native database/performance claim; (3) original assertions,
fault gates, deadlines and256-case serial property unchanged. Explicit delayed-
append entry forces cancellation overlap. The original multi-host race remains
unforced and is still owned by CP3. No suppressed failures or new ignored tests.
CP1 final optimized owning run passes32 tests, one original ignored measurement.
Three reviews: (1) same real-order/no-extra-append, pending-write and original
panic-cause assertions pass after the parking_lot synchronization change; (2)
release binaries actually ran all owner targets, without native database or
benchmark credit; (3) all original deadlines/case counts and newly added assertions
are intact. Clippy passes all command targets with repository lint policy; the
only new narrow allow is deliberate panic in the test-only worker-failure fixture.
The real CI blocker is tracked separately: precise rustls-webpki0.103.16 patch,
no waiver, live cargo-deny now passes in its isolated worktree. Extraction carries
that same patch until the baseline fix merges; ordinary dependency resolution is
otherwise unchanged. No telemetry feature or typed-error completion is claimed.
CP1 full gate was started concurrently with the isolated CI-repair gate, then
agent-interrupted to serialize their disk use (about3.3GiB free). Raw log retained
locally as cp1-gate.log. Fuzz cbor target had started two cases but no completed
result was observed. Three reviews: (1) unchanged decoder bodies, no new processor
invariant exercised; (2) interrupted invocation earns no passing execution credit;
(3) no source/assertion/filter/deadline changed to obtain green. Resume the identical
gate after the baseline patch gate finishes, rather than deleting other work or
weakening checks. CP1 local debug/release owner results remain separately recorded.
CP1 dependency inspection through the repository Nix shell: normal-edge Cargo
tree contains no Labcast or Mnesis adapter package. Workspace-hack still expands
build-only unification dependencies (including SQLx core); this is not an isolated
published-dependency cost measurement. CP7 must disable Hakari and verify the
actual packaged graph. Original model.rs and engine.rs compare byte-for-byte to
the archive; host changes alone provide the required service boundary. Root
baseline/inventory pointers now identify current command evidence instead of
leaving a fresh context with only historical Postgres material.
The [CP1 test evidence bundle](evidence/cp1-tests.tar.gz) retains extraction,
pre-Service compile failures, focused/full debug and release results, Clippy
failures and fixes, the dependency tree and interrupted gate. SHA-256:
4b6a2e209de239b112f9ef0a86e39ad70a7396ae8f887dd11863efb26b28dc96.
A separate complete host gate is required before the signed extraction commit;
the interrupted attempt and cached results do not supply that acceptance.
CP1 serial gate first fails the ordinary TOML formatter on the root and command
manifests. Nextest never ran. Correct with repository taplo format; no manifest
values, test selection or assertions change. Fresh corpus replay2/1/1,1s each,
received three reviews after execution: (1) unchanged wire/CBOR panic-freedom and
existing corpus laws only, no command persistence coverage; (2) four cases ran,
zero filtered/ignored, not real-DB or nextest credit; (3) original bodies/seed
workload and assertions unchanged. The gate remains failed until a complete retry.
Nix cargo-audit reports missing-index yank lookup errors under its existing
sandbox mode despite a successful advisory verdict. This is not verified current
yank status; retain raw output and qualify the actual package graph in CP7.
Formatted-gate replay again executes2/1/1 cases,1s each. Three reviews for each
invocation: (1) unchanged wire/CBOR decoder invariants and existing corpus only;
(2) all selected cases execute, zero ignored/filtered, no processor/native service
credit; (3) formatting reorders two manifest keys only, with identical dependency
values and fuzz bodies. This is fresh execution, not reused replay output. The
full extraction gate remains running; no acceptance follows from replay alone.
Final CP1 source review found inherited unrestricted pub declarations inside
private modules. Tighten Engine/model helpers and Serde hooks to pub(super),
as CP1 requires; the six root responsibility types stay public. No persistence,
branch, assertion or capacity change. The formatted gate was stopped before
nextest execution to qualify this reviewed source instead; prior replay's three
reviews remain recorded, and no interrupted workspace pass is claimed. The
archive comparison is therefore now exact model/engine behavior with intentional
visibility changes, rather than literal file equality. Production stays within
the150-net-line budget. Repeat the entire gate after normal rustfmt.
The visibility attempt failed Clippy's redundant_pub_crate rule at seven private
module type/constant declarations before any tests ran. The private module plus
selective root exports already bounds their external surface; the earlier finding
was too broad. Retain that established pattern without a lint waiver. Narrow
member methods/fields and nested Serde hooks remain valid. This records a rejected
scope change, not a processor defect or passing verification. Repeat full checks.
Private-member gate's three fresh decoder replay invocations pass2/1/1 cases,
each1s. Reviews after each: (1) unchanged decoder/corpus invariants, no command
recovery law; (2) exact four cases, no ignored/filtered/native DB credit; (3)
production changes since the last replay are member visibility only, with
unchanged fuzz bodies/assertions/workload. Clippy now passes the complete workspace
without a new waiver. Nextest is compiling and its result remains pending.
CI repairPR412 at signed24317917801f093f3b3843d98b8fee51bd3d2224:
live dependency job39605915582 passes with rustls-webpki0.103.16. Existing fuzz
run39605915714 completes four libFuzzer campaigns (61s each; wire8,416,063,
header1,760,737, chunk2,276,676, value10,027,803 executions) and four AFL campaigns
(60s each, zero saved crashes/timeouts). Three reviews after each campaign:
(1) actual unchanged decoder bodies/corpus, panic-freedom/format checks rather
than processor acceptance; (2) all eight jobs execute their selected targets,
with bounded campaign durations and no native service claim; (3) source/filter/
workload unchanged, no new processor fuzz law inferred, empty-directory crash
artifacts are not crash evidence. Mutation run39605915628 reports no Rust source
changes and tests zero mutants; its green gate is not mutation coverage. Raw
ci412-fuzz.log/ci412-mutants.log/ci412-deny.log retained in the local evidence root.
PR412 native job119241934337 passes on fresh PostgreSQL18.6/x86_64-linux and
observes server shutdown. Three reviews after each of its20 actual invocations:
(1) unchanged real-adapter assertions cover migration, atomicity, uncertainty,
checkpoint, backup, TLS/durability and cleanup; no processor composition claim.
(2) seven complete profiles264/149/168/176/173/174/150, each zero skips, plus13
one-law selections with their other tests explicitly filtered. Database startup
and successful native operations are visible; required selections use
--no-tests=fail. (3) exact CI repair source changes only webpki patch and unused
nextest output; assertions/filters/deadlines unchanged. The final echoed264-test
receipt is not another execution. Raw ci412-postgres-pr.log retains all20 run IDs.
The canceled bootstrap CI run39605893052 has jobs ending0–1s after start and an
unavailable log119241857970; it earns no test credit. Current PR workflow's Feature
isolation passes; its old cancellation-derived failure is retained in rollup history.

CP1 final host gate passes all12 checks on the reviewed source. Workspace nextest
754ea754-e9a7-42f3-a719-1d5ffcf7f8a2:1626PASS451.591s14slow/142binaries.
Three reviews after this invocation: (1) all32 processor owner laws run alongside
the original1594, including real bounded reservations, unpolled cancellation,
raw[1,2,3] acceptance order and original panic73; deterministic cross-host races,
typed causes, post-commit panic custody and real processor adapters remain CP2–CP6.
(2) five exclusions are four established compile-fail tests plus the original
ignored measurement; no tests newly disabled. Darwin database guards provide no
native processor credit. Fresh fuzz2/1/1 invocations were separately reviewed.
(3) only intended host/visibility/import/dependency changes; original assertions,
256 property cases, timeouts and workloads stay intact. Original unforced race
could still pass sequentially and is explicitly not CP3 evidence. Production1030
lines is +93 over937, within the150-line budget; six root types retained. Full
Nix gate/owner debug+release/Clippy/no_std/Wasm all pass. Protected PR/merge pending.
The [CP1 gate bundle](evidence/cp1-gates.tar.gz) retains all three subsequent
gate attempts and the successful reviewed run, final source hashes, formatter
logs and the audited CI repair results. SHA-256:
731e8a8d1c42eeebe44473fa0eefa08596a408373fb9bbbeaaaed962128654c4.
The earlier test bundle is unchanged. Normal signed hook/current-head protected
checks still determine delivery; cached reuse gives no additional runtime credit.
CP2a pre-change typed-cause consumer fails compilation with E0432 (Cause absent).
Zero test bodies ran. Three reviews: (1) canonical lost-commit test now requires
a directly matched StoreError adapter leaf plus unchanged ID/raw-row/receipt
assertions; (2) exact selected test command reached compilation only, not a runtime
failure; (3) the new public contract is absent and current source boxes dynamic
errors, but this diagnostic alone is not a behavioral regression witness. Implement
real typed reply propagation before executing the unchanged strengthened law.
CP2a typed propagation passes32 owning tests (6model+1adaptation+1property+
24recovery), one original ignored measurement. Three reviews: (1) original
late-append/empty-read/retry-rejection/stream-scope laws persist; strengthened
canonical cases directly match typed adapter and std::io decision leaves without
downcasts or recursive processor errors. Pending success clears only after a
validated receipt. (2) all owner targets execute, no filter, property256 cases;
no new post-append fold or native adapter credit yet. (3) raw records/receipt/ID/
state assertions and deadlines remain unchanged; error expectations are strictly
more specific. Dedicated post-append panic/reconstruction and typed kernel-read
witnesses follow before claiming CP2a qualified.
CI repair complete checks: workflows39605915669(PR) and39605894953(push) pass
at24317917. Three reviews after every logged actual invocation: (1) unchanged
contract-owner assertions cover compile diagnostics, feature boundaries, native
recovery/atomicity and decoder behavior; no command processor qualification.
(2) each workflow has74 actual test invocations:3 compiler,27 lib-feature,
20 dedicated nativePG,20 nativePG inside Nix,1 workspace nextest and3 replay.
Each native group has seven complete zero-skip profiles and13 selected laws;
final echoed native receipt excluded. Lib counts per default/no-default/all:
kernel15/15/15,macros1/1/1,store217/217/217,store-testing0/0/0,wake17/17/17,
wake-nostd0/0/0,inmemory25/25/30,Fjall131/131/131,PG11/11/11. Zero-test lib
harnesses are not conformance evidence. Compiler2/1/1; workspace1594 each with
four established compiler exclusions (PR150.282s,push134.285s); replay2/1/1 each.
Real PG18.6 startup/operations/shutdown visible in all four native groups; guarded
non-native harnesses provide no extra DB credit. (3) source/tests/filters/deadlines
match the reviewed CI patch and original owners, without reduced workloads. This
fix changes the lock patch and unused nextest output only; coverage stays present.
Raw ci412-pr.log and ci412-push.log retain exact run IDs/counts and cached steps.
All six current required push/PR contexts pass; zero unresolved review threads,
main27de unchanged, expected squash tree3ab79ecda4af32faf7d8f016af4b02392df73729.
Historical bootstrap cancellation-derived gate failure is not a current required
check failure; gh pr checks --required passes and merge state is CLEAN.
CP2a extended owning suite passes34 tests (6model+1adaptation+1property+
26recovery), one original ignored measurement. Three reviews: (1) a real committed
row survives a post-append application panic and failed fresh reconstruction;
original panic payloads are observed through both joins. Healthy exact recovery
returns receipt1/state7 with one total handler call and unchanged full raw row.
A real4097-record append yields the exact public StoreError::Kernel rehydration
limit4096 without repair writes. (2) all owner targets execute, no filter; existing
256 histories/deadlines intact. The new full-history read has a2s operation profile
chosen before its first execution. In-memory is the only backend in this run.
(3) assertions compare exact typed causes/IDs/receipts/state/raw history, not logs;
no fake repository or mirrored decision implementation. Failed completion retains
the caller's original request in this witness; report handoff on abort/panic is
still CP5 work. Codec/resource distinctions and final CP2 delivery remain pending.
CP2a optimized suite passes34 owner tests, one original ignored measurement.
Three reviews: (1) exact typed adapter/domain/kernel sources, delayed-commit
custody and both actual fold failures recover one committed row with one handler
call; (2) release binaries run all owner targets, property256 cases, no filter or
native DB; (3) assertions/workloads/deadlines remain identical to the debug run,
with raw history/receipt/state/handler-count oracles. No new release measurement
credit is inferred from these correctness tests. Earlier Clippy rejected repeated
expanded startup/reply types; aliases now derive domain/store errors from their
owning types once, without weakening lint policy. Final lint/full gate pending.
Complete CP2 diagnostics owning run passes34 tests, one original ignored
measurement. Three reviews: (1) queue full/invalid capacity/zero deadline, history
full and domain replay limits now have distinct outcomes; request versus record
byte failures preserve the original WriteZero cause. Existing exact retry,
late-write, commit-lost, panic/reconstruction and typed kernel laws still pass.
(2) all owner targets/256 generated histories execute without filter; in-memory
only. No old deadline, workload or gate changed. (3) canonical byte/lossy/schema/
capacity assertions are more specific, and raw-row/state/receipt/no-redecision
oracles remain intact. Checked fact totals replace saturation/unchecked addition;
accepted JSON fields/order and format constants stay unchanged. One formatter
parse error in the strengthened assertion was corrected before tests executed;
its raw log is retained, not treated as a runtime defect. Clippy/full gate and
final optimized diagnostics run remain required before CP2 qualification.
The strengthened capacity witness passes1 selected test,26 filtered/zero ignored.
Three reviews: (1) actual4096-record history still yields original receipt1 and
state/version4096; a fresh request returns HistoryFull and writes nothing. An
external4097th record then yields exact typed kernel limit4096 with no repair.
(2) one canonical in-memory case executes with its preselected2s operation bound,
not the full suite/native DB. (3) exact ID/version/fact-count/state/raw-row equality
rejects missing duplicate precedence, wrong resource mapping or hidden append;
the earlier kernel-read witness was extended rather than duplicated. Clippy had
reported an unfulfilled lint expectation on constant evaluation; remove the
unnecessary expectation, retaining compile-time checked nonzero construction and
all global lint levels. No assertion or workload was weakened.
Final optimized diagnostics suite passes34 tests, one original ignored measurement.
Three reviews: (1) direct typed sources, distinct resource reasons and all prior
uncertainty/fold/corruption laws pass; (2) all optimized owner targets execute,
property256 cases, no filter/native service/performance claim; (3) original
assertions remain and byte/lossy/schema diagnostics are more specific. This review
finds one remaining counterexample in the capacity witness: constructing4096 rows
externally would not reject an off-by-one admission threshold4095. Strengthen
that same case to submit record4096 through the real processor from4095 before
checking full history and the unsupported4097th row. Product source is unchanged;
run only the strengthened case in both profiles, then the full workspace gate.
The revised capacity case passes1 debug test,26 filtered/zero ignored. Three
reviews: (1) the processor itself accepts physical record4096 from a4095-record
history, then preserves old receipt1 and rejects a fresh request at capacity;
unsupported external4097th data retains the typed kernel failure without repair.
(2) only this strengthened case ran, with unchanged2s operation deadline and
in-memory backend. (3) exact final receipt4096/events1 and before/after raw counts
now reject threshold4095, in addition to duplicate-precedence/resource-mapping
counterexamples. Adjacent error descriptions use processor/acceptance terminology;
the persisted controller.accepted.v2 name and all format fields stay unchanged.
The strengthened boundary case passes1 optimized test,26 filtered/zero ignored.
Three reviews: (1) actual4095→4096 admission, old receipt, full-history rejection
and typed unsupported-history failure hold in release; (2) this selected case
executes on the final source, complementing the prior34-test release suite's
unchanged other33 cases; no native/performance credit; (3) all new max-boundary
assertions and2s deadline are identical to debug, with no source/workload weakening.
The complete workspace gate follows on the same final source.

CP2 host build exits0 with all12 standard outputs retained under cp2-check*.
The actual nextest invocation0981e626-c8b8-4a41-a4bb-f723ed3f6135 passes1628
tests in549.765s (142 binaries,15slow,five existing exclusions). Three reviews:
(1) all34 command-owner cases execute, including actual4095→4096 admission,
committed-fold/reconstruction failures, typed sources and retained uncertainty;
forced multi-worker races and failed-completion report custody remain unqualified.
(2) release/LLVM-instrumented workspace selection and exact counts are visible;
four compiler cases remain delegated to existing checkout CI and the original
measurement stays explicitly ignored. No real processor Postgres/Fjall credit
comes from this host run. (3) cmp confirms every changed command source/test file
matches /nix/store/7fj0dingxxhy1caqlq2azwway4hpfd77-source; no assertions, deadlines,
property256 workload or selection were reduced. PR413's known capacity/lifecycle
mutation misses remain visible and require repair; a green run does not close them.

Each of the three fresh replay invocations also received three reviews:
(1) CBOR header/chunk, value validation and wire decoder each exercise their
existing fuzz-common product oracle; these are foundation laws, not command
acceptance evidence. (2) cbor2PASS,value1PASS,wire1PASS,1.00s per invocation,
no ignored/filtered cases or database service; these are bounded replay/smoke
runs, not new long fuzz campaigns. (3) the harnesses remain unchanged and invoke
the actual shared decoder checks rather than mirrored implementations. Raw log:
/tmp/mnesis-command-start-20261021/cp2-full-gate.log.

Important retained limitation: after the passing nextest checkPhase and coverage
output, the Nix audit-tmpdir fixup shell subprocess logs Segmentation fault11.
The derivation/build still exits0 and all outputs exist; this is not a failed test
or evidence that the post-test scan completed. Investigate before calling the
whole gate clean. The existing sandbox cargo-audit missing-index/yank lookup
limitation is also unchanged. Neither diagnostic is suppressed or waived.

CP0 foundation current head d88c6efa1b7ad04b6979ee35e1418f08f010ccfb is CLEAN
with all checks complete and no failures or review threads. Its diff from protected
main efec41a changes records/evidence only: no Rust/TOML/Nix/lock change. Actual
CI39617097532(PR) and39617089755(push) each have74 runtime invocations (75 summary
lines include one echoed native receipt). Three reviews for each invocation:
(1) compiler diagnostics, library feature contracts, native persistence/recovery,
workspace owners and existing decoder fuzz oracles remain unchanged; these runs
qualify foundation delivery, not command implementation. (2) each executes three
compiler2/1/1,27 feature-library invocations,40 nativePG invocations (two fresh
PG18.6 services, each seven zero-skip profiles264/149/168/176/173/174/150 plus13
selected laws), workspace1594 and three replay2/1/1. Nextest135.582s(PR) and
131.335s(push), four established compiler exclusions. Zero-test library harnesses
remain zero coverage credit. Server startup/shutdown visible in all four groups.
(3) filtered product source matches the previously qualified protected CI repair;
no changed assertions, selection, workload or source substituted by this PR.
Raw cp0-final-ci-{pr,push}.log/counts.txt retain exact invocation receipts.

CP0 deep-fuzz39617097385 also completes. Each of its eight campaigns received
three reviews: (1) unchanged wire/header/chunk/value product oracles, not command
identity/concurrency evidence; (2) four libFuzzer61s runs: wire10979052,header1854316,
chunk2214683,value8458746; four bounded AFL runs retain their own final receipts;
(3) no changed harness/filter/workload, no new decoder guarantee inferred beyond
those campaigns. Logs remain cp0-final-fuzz.log. Mutation CI sees no Rust diff
and runs zero mutants, which is not mutation coverage. Current required contexts
all pass and there are zero unresolved review threads. Expected protected squash
tree3ee54613c61226c317c63347c6bde81c7c04711e; merge must match d88c6efa exactly.
CP0 protected merge is ca62b557ed870ec7ceb336ea9d08f21df0e664d5 at2026-10-21T15:37:01Z.
Actual tree3ee54613c61226c317c63347c6bde81c7c04711e matches the expected tree;
ordinary protected squash matched reviewed d88c6efa, with no bypass.

CP2 raw logs/source receipt are retained in [cp2-gates.tar.gz](evidence/cp2-gates.tar.gz),
SHA256342b85b86ff3b0120dcb315c5196352fbeb8ba659a58e247b074624300f5b61b.
The post-test fixup diagnostic is scoped: ls/file/source inspection establishes
that nextest's entire output is one ASCII LCOV coverage file, with no executable
or wrapper. audit-tmpdir classifies ELF RPATHs and wrapped scripts; neither exists
in this output. Its classifier still crashed and is not claimed complete. No
hook/check is disabled, no assertion/workload changes, and normal signed hook
still runs every declared check. This is a retained Nix-shell tooling limitation,
not a reason to change the command processor or expand product scope.

CP3 first selected run fails compilation before any test body: the new race
fixture Total omitted Clone required by the existing processor read contract.
Three reviews: (1) intended both-at-append/handler/raw-record assertions have not
executed, so no concurrency qualification; (2) two selected owner tests, zero
runtime executions; (3) failure belongs to the fixture trait bound, not a product
counterexample. Add the required Clone derive without changing product bounds.
Pre-execution review also retains the original race's nonempty-head coverage:
the shared law must cover both an empty history and an actual accepted prefix,
with unchanged prefix rows and handler accounting. Do not replace that workload
with only empty-stream races. Same preregistered350-line test budget.
CP3 targeted run d87ffbb1-d9ce-46be-a4a2-a754a424a810 passes2 tests in0.183s.
Three reviews: (1) canonical race now forces48 schedules across three race kinds,
both winner orders,1/u64MAX IDs,0/u8MAX bodies and empty/nonempty history. Both
handlers reach real append before release; no new row lands while held. Exact
record/receipt/state, two contender decisions, changed expected version, losing
retry and observed joins are asserted. Existing serial history workload remains.
(2) selected recovery case plus new256-case generated concurrent property execute,
34 other cases excluded by this targeted selection; in-memory only, not a native
DB or complete owning-suite claim. (3) no new product code/alternate CAS exists;
the gate wraps actual inner append, and independent raw JSON/version comparisons
verify the whole accepted record plus unchanged prefix. Counter isolation is
structural: only this canonical case calls the helper in recovery; properties
has its own integration binary/static and serially executes its generated inputs.
No existing capacity/assertion/deadline/256-serial workload was weakened. The
compile-only failure remains separately visible. Full owning suite, optimized
execution, lint and complete gate remain pending.
CP3 lint rejects fixture restructuring: moved helpers lost their existing test
expect allowance, one gate binding shadows the delayed-fault gate, two Request
struct-update clones are redundant, and the canonical helper exceeds the existing
80-line limit. Preserve lint policy: retain helpers under history with only the
parent visibility needed by the shared race consumer, rename the binding, remove
redundant clones and separate final committed-history verification from the
controlled schedule. No assertion/workload removal or new lint waiver.
CP3 complete debug owner run passes35 cases, one original ignored measurement,
in1.469s. Three reviews: (1) final canonical race expands to192 schedules including
1,2,u64MAX-1,u64MAX identities and0,1,u8MAX-1,u8MAX bodies with empty/nonempty
heads. New256 concurrent histories and unchanged256 serial histories all execute;
raw records, original receipts, state, handler counts and observed worker joins
remain asserted. Existing lost-commit/late-append/fold/capacity cases still pass.
(2) all four package targets execute without selection; in-memory only, no native
DB/performance credit. (3) immutable prefix rows plus full accepted JSON/version
are compared independently of processor decisions; setup and final verification
share one private History owner and all lint levels remain unchanged. Clippy
passes after keeping test helpers below their original private owner, removing
redundant copies and separating seeding, controlled scheduling and final history
verification. No assertion/workload was discarded to satisfy lint limits.
CP3 complete optimized owner run passes35 cases, one original ignored measurement,
in0.513s. Three reviews: (1) the same192 forced schedules,256 concurrent and256
serial generated histories plus all prior uncertainty/fold/capacity laws pass;
(2) all four release package targets execute with no selection/native DB, and
this correctness run is not performance qualification; (3) sources/assertions/
workloads match the final lint-clean debug suite. No profile-specific weaker
assertions or shorter inputs. Next negative controls deliberately remove conflict
receipt reconciliation and accept mismatched original request metadata; the
canonical race must reject both at product outcome assertions, not fixture setup.
CP3 targeted negative controls complete: baseline canonical case1PASS, then both
mutants caught,0missed/unviable/timeout. Three reviews for each of three actual
invocations: (1) all192 baseline schedules pass; disabling confirmed-conflict
classification makes the real duplicate contender return typed Uncertain instead
of the original receipt. Accepting every original==request guard makes changed
expected metadata falsely return success. Both violate the owning contract.
(2) one canonical recovery case selected per invocation,35 other cases excluded;
mutants each0PASS/1FAIL, baseline1PASS. These are controlled in-memory laws, not
full mutation/native qualification. (3) actual first failures are receipt unwrap
with the retained real CAS source and exact mismatch assertion respectively;
no fixture setup, gate-entry timeout, runner kill or compiler error is credited.
The maintained generator offered no conflict-arm-deletion candidate, so no test
ran for that initial selection; the classifier mutation is the recorded reachable
substitute. Actual outcomes/diffs/logs remain cp3-negative-controls/mutants.out.
Historical CI review recovered during this audit: original PR413 head b711855
workflows39609659589(PR) and39609560134(push) each execute74 actual test invocations,
with75 summary lines because one native receipt is echoed. This late record is
not a claim of prior/current-head delivery. Three reviews for each invocation:
(1) unchanged compiler/feature/adapter/decoder laws plus all32 extracted command
laws execute; original mutation misses remain separate and invalidate any broad
mutation-green claim. (2) three compiler2/1/1,27 isolated library invocations,
40 nativePG invocations (four actual PG18.6 services across both workflows, each
startup/shutdown observed), workspace1626 and replay2/1/1 per workflow. Native
full profiles264/149/168/176/173/174/150 have zero skips;13 single-law selections
per group retain explicit filters. Workspace157.585s(PR),106.699s(push), five
established exclusions. Zero-test library harnesses earn no coverage credit.
(3) actual run logs match original b711855 owners/selections without workload or
assertion reduction. These historical successes do not qualify repaired6d0d8adc;
its current-head runs39630929335/39630918875 and mutation39630929359 remain pending.
Raw cp1-original-ci-{pr,push}.log/counts.txt retain every actual invocation.
CP3 full gate's three fresh replay invocations each received three reviews:
(1) unchanged CBOR header/chunk, value and wire foundation oracles execute actual
fuzz-common checks; no command/race coverage is inferred. (2) CBOR2PASS,value1PASS,
wire1PASS,1.00s each, zero ignored/filtered, no native service; replay/smoke only.
(3) harnesses, inputs and filters remain unchanged; the gate includes the staged
new race helper, and no check is weakened. Complete workspace nextest/remaining
outputs are still pending; cp3-full-gate.log is the live raw record.
CP3 first full gate fails in unchanged macro reproduce_transform_overflow_diagnostic:
its nested rustc aborts loading @rpath/librustc_driver from the hard-linked
/nix/store/.links alias. The actual selected driver exists. Three reviews:
(1) all35 command cases pass in the instrumented run; the macro's compiler never
reaches its diagnostic oracle, so this is not a changed product assertion or
passing gate. (2) exact log count654 completed passes,1 failure, no final summary;
974 selected cases unfinished/not run plus five original exclusions. Agent sent
SIGINT to the already-failing Nix build when free space fell to581MiB; no partial
workspace success is claimed. (3) same source/assertions/filters/workloads; actual
first failure is dyld SIGABRT, not a guessed flaky assertion. Logs remain intact.

Direct executable-alias negative probe reproduces the same failure134 without
DYLD fallback, then passes0 with only the selected toolchain/lib fallback. Cargo's
supported environment documentation says it extends this path for child builds:
https://doc.rust-lang.org/cargo/reference/environment-variables.html#dynamic-library-paths.
Use Nix lib.makeLibraryPath from rustToolchain, Darwin-only at the existing
nextest owner. No new tool/derivation/runtime, no global Nix configuration changes,
no renamed/excluded macro test, and all1629 cases remain required. The causal
reason that dyld selected the store alias during that nested invocation is not
proven; the failed path and successful fallback probe are observed facts.
Retry only after preserving evidence and freeing this goal's generated primary
Cargo target cache; source/logs and other worktrees remain untouched.
Current PR413 full mutation39630929359 at6d0d8adc succeeds:168 candidates,
91caught,77unviable,0missed/timeout; artifact12359961832. Three reviews for each
of92 actual runtime invocations (baseline plus91 viable mutants): (1) existing
identity/uncertainty/codec/lifecycle oracles still reject every behavioral mutant.
The only removed candidate versus failed169 is the disclosed exact private Debug;
the only status changes are capacity0/1 missed→caught and call predicate timeout→
caught. (2) baseline32PASS/1originalignored; caught invocations stop at actual first
failure, with exact summaries retained.77 compile-unviable get no runtime credit.
No processor-native DB law runs in this sweep. (3) all actual first failures are
reviewed by owner; new capacity cases fail0/1 versus4096, stopped-call case fails
its bounded response assertion, and88 prior behavioral catches remain catches.
No fixture setup poisoning, weakened deadline/input or excluded behavioral mutant
substitutes for a catch. cp1-final-first-failures.json records exact identities,
phase statuses and first failures; raw current artifacts/logs are retained.
CP3 full retry95724cfa-0ec7-4bd7-8dba-2c73572f71c2 passes1629 cases in439.141s,
14slow,142binaries,five existing exclusions. Three reviews: (1) all35 command
cases retain192 fixed/256 concurrent/256 serial histories, exact raw records,
receipt identity, state, handler counts and observed joins. The formerly failing
unchanged macro diagnostic test also passes58.024s with the supported fallback.
(2) the entire original selection executes without new exclusions or shortened
workloads; no processor-native DB/performance credit. Remaining output builds
still determine whole-gate success. (3) cmp verifies all five changed test owners
against source95y5xm17nwv7izl18s33183f3ifrxcfl-source; code/assertions match the
prior failed run. Derivation id33yn stores the exact fallback used by the direct
negative probe. Initial flake attribute-query failed because env is flattened
into derivation variables; actual derivation JSON, not that failed query, proves
the configuration. No test was skipped, renamed or altered for green.

Current-head CP1 push CI39630918875 and original CP2 push CI39628097424 complete.
Each of their74 actual runtime invocations received three reviews: (1) compiler,
feature, decoder and existing native adapter laws are intact; workspace includes
32command cases at6d0d8adc and34 at02c8ac86, respectively. These are not processor-
native tests. (2) each has three compiler2/1/1,27 library-feature,40 nativePG,
workspace1626(102.110s)/1628(143.758s), and replay2/1/1. Native services are real
PG18.6, four startups/shutdowns across the two workflows; each group has seven
zero-skip profiles264/149/168/176/173/174/150 plus13 explicitly selected laws.
75 summary lines include one echoed native receipt, not another invocation.
Zero-test libraries do not count as conformance coverage. (3) actual head/source,
selection, assertions and full native receipts match their owning reviewed changes;
no fixture-only success replaces runtime product laws. Logs/counts are retained as
cp1-final-ci-push and cp2-original-ci. CP2's forthcoming parent integration needs
fresh checks; these original-head passes are not transplanted to a changed tree.
CP3 all12 host outputs now pass and are rooted under cp3-check*. Final local
failure/probe/configuration/source receipts are in evidence/cp3-gates.tar.gz;
original owner/control bundle remains unchanged. Normal signed hook follows,
with cached outputs earning zero new runtime credit. No production code change;
278 net test lines across five owners and five justified Nix lines. CP3 is locally
qualified; protected delivery, real processor backends, telemetry/performance,
publication and server consumption remain required under their owning CP rows.
CP0 delivery update: CI repairPR412 protected-merged efec41afeab8138801fe034be1a60cbf4055848a,
reviewed head24317917801f093f3b3843d98b8fee51bd3d2224 and exact expected/actual
tree3ab79ecda4af32faf7d8f016af4b02392df73729. Current required push/PR checks pass,
zero unresolved threads. Integrate that protected baseline into PR411; four root
research conflicts preserve both append-only parent records. Product source and
frozen inputs unchanged. Correct the prototype/root-lock distinction for tokio-util;
its selected compatibility proof stands, while actual root addition belongs to CP1.
Normal signed hook/current-head CI still required; cached outputs earn no new
runtime credit. This update does not claim CP1–CP9 merged or published.
CP0 integration signed hook was interrupted at the disk limit before completion;
its rebuilds were required because prior unrooted Nix outputs had been reclaimed.
The selected nextest derivation4ng212... is exactly the already qualified CI-fix
one, and Crane's locked commonCargoSources keeps only Cargo/Rust/TOML sources;
no product source changed in the documentation merge. Fresh hook replay2/1/1
received three reviews: unchanged decoder contracts; actual four selected cases,
zero ignored/filtered and no native credit; identical assertions/corpus/source,
not another processor proof. Nextest had not emitted a completed result; no
workspace/gate pass is claimed for the interruption. Remove only this task's
regenerated Cargo target cache and retry serially. Retain requested check outputs
with supported Nix --out-link and shell --profile roots under the task evidence
root so unchanged checks can be reused; no global GC/configuration or new tool.
CP0 rooted retry nextest passes1594/1594 in441.986s(15slow), run
348a6b31-eac6-468a-a006-08a0417911da/138binaries/four established compiler exclusions.
Three reviews: (1) unchanged baseline store/kernel/lifecycle assertions, no command
processor claim; (2) one actual workspace invocation, no native DB credit from
Darwin guards, no fresh fuzz invocation in this retry because its previous output
was reused; (3) same qualified source/selection/assertions/workloads/deadlines as
CI412. Exact source derivation identity confirms cache reclamation caused the
rebuild, not a documentation-driven product change. Final non-test checks and
normal signed hook remain pending; twelve result links will retain the outputs.
All twelve baseline check outputs now pass and are retained by standard Nix
result links. The successful rooted retry's1594-test execution was separately
three-reviewed above; no extra replay was executed. The final normal signed hook
uses the same source/results, with max-jobs1 supplied only to this command's Nix
configuration. This changes no repository check, workload or global setting.

## CP1 mutation review and bounded negative controls

PR413 mutation run39609659694 at signed b711855 fails honestly:169 mutants,
88 runtime catches,77 compile-unviable,3 missed,1 timeout. Evidence artifact
12351030394 was inspected at its actual outcomes/log/diff files. Three reviews
for each of its93 actual runtime invocations (baseline plus92 viable mutants):
(1) existing model/adaptation/history/recovery assertions reject changed identity,
reconciliation, acceptance, bounds and lifecycle behavior; the two public capacity
returns were genuinely untested. A stopped Service::call predicate strands its
reserved reply, exposing an unbounded wait in the test. Private State's human
Debug output is incidental under the existing diagnostic policy, not a persistence
contract. (2) baseline and three missed candidates run32 cases with one original
ignored measurement; caught candidates stop at their first failing owner. The
hung candidate runs27 passes then receives runner SIGTERM in test28, four not run.
All93 exact summaries are in cp1-actual-invocations.json. No real processor DB
integration runs.77 build failures have no runtime coverage credit: they fail
syntax/type/required generic obligations, including invalid let-chain || syntax.
(3) each of88 actual first failures was inspected from logs; they are product
result/state/boundary/join assertions, not failed fixture setup or poisoned shared
globals. One changed fact-count subtraction panics at the production overflow,
which is a valid killed boundary mutant, not a helper failure. The retained
first-failure table names original mutant identity and failure owner; compiler
rejection is kept separate. No count is promoted to unexecuted laws.

Repair changes only canonical tests and one exact diagnostic registration under
the existing policy. The4096-exhaustion law now checks the advertised public
capacity against its actual boundary. Existing worker-exit calls/response/join
waits are bounded2s; successful response/error assertions remain unchanged.
No production code, interface, dependency, format, behavioral mutation filter or
lint level changes. Selecting only capacity0/1 and stopped-call ==→!= for local
negative controls yields baseline32PASS, then all3 caught,0missed/timeout/unviable.
Three reviews for each of these four invocations: (1) baseline preserves all
identity/lifecycle laws; wrong capacities fail exact0/1 versus4096, wrong predicate
fails the actual stopped reply deadline. (2) baseline32PASS/1ignored; each capacity
mutant2PASS/1FAIL then stops, predicate27PASS/1FAIL in actual test28 (2.010s,
Elapsed), not a killed runner. All use real SUT/Tokio/in-memory stores and original
256 property cases. (3) first failures are exactly the strengthened assertions,
not setup/compiler failures; no source behavior was patched to fit tests.
The full current-head CI sweep remains required after push; this three-mutant
selection is targeted repair evidence, not full mutation qualification.

Protected foundation/main ca62b557 (PR411) is integrated without product changes;
research conflicts retain both parents' facts, and the test repair remains in a
named stash plus raw patch until its signed commit is verified. Existing unrelated
stash entries and user work are untouched.
Mutation evidence bundle cp1-mutation.tar.gz SHA256:
1b0c1b470c35297e1bd4e7ff95979f329f803d4efcf2cee195aae37234c26404.
CP1 repair full gate has executed three fresh replay invocations: CBOR2PASS,
value1PASS,wire1PASS,1.00s each. Three reviews after each invocation: (1) unchanged
fuzz-common foundation decoder/validator oracles, no command-processor coverage;
(2) all selected bodies run, zero ignored/filtered, no database; these are bounded
replay/smoke checks, not sustained campaigns. (3) test sources/workload/selection
are unchanged; only canonical command capacity/lifecycle assertions and the exact
private diagnostic policy entry differ. Raw cp1-repair-full-gate.log retained.
Full nextest and remaining checks are still running and receive no early credit.
CP1 repaired workspace nextest efecacc0-8c3b-4a25-bdab-f319af130f08 passes1626
cases in479.112s(15slow),142binaries,five existing exclusions. Three reviews:
(1) all32 command-owner laws execute with strengthened advertised-capacity and
bounded response/join assertions; original identity/uncertainty/compatibility
laws remain intact. This extraction does not qualify CP2 typed errors or CP5
failed-completion handoff. (2) release/LLVM-instrumented workspace selection is
visible; four compiler cases remain in checkout CI and one original measurement
remains explicitly ignored. No native processor service is claimed. (3) cmp
verifies all four command product modules, both changed test owners and exact
mutation policy file against Nix source2hfz6rsrbbbjcb7ymfngjqljfcqz8b4z-source.
Zero product-code change from b711855; only strictly stronger behavioral tests.
The intentional private-Debug policy classification remains disclosed separately.
Post-test coverage installs without the earlier classifier-crash diagnostic in
this run. Remaining output builds still determine complete gate success.
CP1 repaired all12 host outputs now pass and are rooted as cp1-repair-check*.
Raw full-gate log and exact source hashes: evidence/cp1-repair-gate.tar.gz. Normal
signed hook follows, using the same source; cached outputs earn zero fresh test
credit. Merge includes protected main ca62b557 plus two canonical test owners and
four exact diagnostic policy lines, without production/dependency/API/format changes.
CP2 parent integration preserves all typed product modules byte-for-byte and
imports only the stronger capacity/bounded-response tests plus private diagnostic
policy from6d0d8adc. Its gate also needs the same supported Darwin driver fallback
proven in CP3: identical five Nix lines at the existing nextest owner, preregistered
budget, no changed tests/filters. The full updated source must pass before push;
old02c8ac86 CI success is retained as historical, not current-head qualification.
CP2 parent integration's complete nextest5275a5d4-5f16-4a83-8dfb-925df3009749
passes1628 cases in568.553s,14slow,142binaries,five original exclusions. Three
reviews: (1) all34 typed-error/custody owner laws plus stronger parent capacity/
worker-exit assertions execute; no rollback is inferred from a dropped response
or empty read. (2) original full instrumented selection, no new exclusions or
shorter workloads; guarded Darwin PG cases are not processor-native evidence.
(3) cmp verifies four product modules and two changed parent test owners against
source dq3nzcwxifdmrmnlqgz17iybnar3db7s-source; no typed product source differs
from02c8ac86. Parent stronger assertions and exact Debug policy survive conflict
resolution. All12 outputs now pass and are rooted as cp2-integration-check*;
normal signed hook still required. Same supported Darwin fallback preserves the
unchanged macro law; no existing assertion/filter/lint is weakened.
CP2 protected-main integration is content-preserving: mainfa0b95b1's complete tree
is identical to already imported6d0d8adc (5e5ec37a). Squash ancestry creates
add/add conflicts; select the already tested typed source/stronger tests and union
records from signed149ff89c, after proving that exact tree equality. No incoming
unique content exists to discard. Product source and Nix inputs remain byte-for-
byte equal to the1628-pass integration gate. Normal signed hook still runs; cached
outputs earn zero runtime credit. This merge records protected main ancestry.
CP3 updated-parent integration preserves CP2 product source exactly, the unchanged
forced schedules and every stronger capacity/worker-exit assertion. Four research
histories retain both parents; current-status files resolve historical statements
to CP0/CP1 merged and CP2/CP3 locally qualified. Full gate is running on staged
combined source6qwdzdxf1x3xp0jjv5sfqv9gjlb26p9b-source. Its three fresh replay
invocations each receive three reviews: (1) unchanged CBOR/value/wire foundation
decoder oracles, no command/native qualification; (2) actual2/1/1PASS,1.00s each,
zero ignored/filtered and no database; (3) same source/selection/corpus/assertions,
not shortened campaigns or duplicated cached credit. Raw cp3-integration-gate-
live.log retains actual output. Remaining whole-workspace nextest is still required.
CP3 integration gate fails honestly: nextest792/1629 executed,779PASS/13FAIL,
5existing exclusions,837not run,230.708s. Three reviews: (1) existing Fjall property/
state-machine and nested compiler owners fail actual OS28/StorageFull during
writes/compiler artifact creation; all required laws remain, no green gate claim.
(2) one real full instrumented workspace invocation, failed100, all13 actual first
failures inspected; partial successful cases are not complete qualification.
No native command DB evidence. The three replay invocations are separately
three-reviewed above. (3) source/assertions/selection unchanged; the original
stronger parent assertions and race workloads remain intact. The agent overlapped
local compilation with insufficient disk headroom; preserve failure and retry
the same suite only after generated-cache cleanup, without overlapping builds.
Raw authoritative Nix log cp3-integration-nextest-failed.log and whole outer output
cp3-integration-gate-final.log retained. Retire only this goal's superseded three
CP3 cross-target caches after zero referrers verified; exact paths are recorded
in retired-cp3-output-paths.txt and their replacement integration outputs already
pass/rooted. Specific Nix deletion99953 is live; no force/global collection or
other project cleanup. Current nextest must still pass before signed commit.
CP2 is now protected-merged in PR414 as10d1b3b31844bebe2a941ead80343eee592d8584
at17:45:38Z. Expected/actual/head tree9291a8825e94dae927052f49f3a0c9d798b912ab
matches reviewed26c5a334;42current checks pass/one scheduled-only skip and0threads.
Exact raw final CI/mutation/fuzz/merge receipts are retained in the primary CP4
feature record/evidence and will arrive with that separate slice. This pending
CP3 merge already includes26c5a334's tree and all stronger parent assertions.
Its final normal hook and protected-main ancestry still precede push/delivery.
CP3 unchanged retry9d81d83f-7543-4dfa-9ba1-3ce49856acf7 passes1629/1629 in451.503s,
15slow/142binaries/five existing exclusions. Three reviews: (1) all35 command laws
retain192forced/256concurrent/256serial schedules, original receipts/raw prefixes,
state/handler counts and observed worker completion. Stronger parent capacity/
bounded worker-exit oracles remain; macro compiler law passes124.595s. (2) full
instrumented workspace selection executes with no new exclusions or shortened
workloads; no processor-native database credit. All12 current outputs now pass
and remain rooted. Actual retry uses6.6GiB start headroom and no overlapping build.
(3) cmp verifies four product modules, all changed race/parent test owners and
exact policy against source6qwdzdxf1x3xp0jjv5sfqv9gjlb26p9b-source. Initial cmp used
the wrong history path and looked for flake.nix in filtered Cargo source; corrected
checks use support/history.rs, byte-identical CP2 flake and actual derivation env.
No source discrepancy is concealed. The original OS28 failure remains retained;
current source/assertions/filter and full workload did not change for retry.
Normal signed hook and exact current-head protected delivery remain required.
CP3 parent integration is signed6998d7fa12df40b6e6a36bda631c75f57edb49a4,
normal full hook passes using all12 already qualified outputs, zero fresh runtime
invocations. Protected-main integration10d1b3b3 creates historical-record conflicts
because CP2 was squash-merged. Its complete tree is exactly the imported26c5a334
tree9291a882; all incoming facts are already present in6998d7fa. Keep the tested
combined records/status and stronger source, after proving tree equality and zero
automatic product/test/Nix diff. No unique incoming content is discarded. Normal
signed hook runs again; unchanged cached outputs earn no new execution credit.
CP3 first new-head Nix CI39649396688/job119393433782 fails at7534d590: workspace
369/1629 executed,368PASS1FAIL,1260not run,five original exclusions,in4.688s.
Three reviews: (1) actual canonical concurrent owner returns Uncertain{id:1,
source:None} at the expected winner's receipt assertion after2.060s. This is a
real failed qualification, not a green job or proven product defect. (2) full
instrumented Linux selection stops at that first failure; no native command-
database acceptance. (3) raw failure/selection/source retained in cp3-ci-first-
failure.log. Source audit finds entered notification precedes semaphore waiter
registration, so shared FIFO release may select the other contender. Reproduce
that exact allowed interleaving before repairing the fixture; no timeout increase,
reduced schedules, single-thread substitution or weakened receipt assertion.
CP3 forced registration-order probe executes1FAIL/26skipped (25filtered/one
original ignored) in2.023s. Three reviews: (1) actual old helper under a permitted
preemption now cannot finish the selected winner; no product code changes.
(2) Nix recovery target only, same two-thread canonical192-schedule law; first
schedule fails at bounded expected reply, no native database. Raw cp3-order-probe-
before.log and exact temporary gate patch retained. (3) both handlers reach actual
append gates and prefix assertions first pass; first waiter is held until second
has registered on release. Failure is outer Elapsed, consistent with a stolen
release but not yet direct receipt evidence. Strengthen the existing completion
observation to select both replies: an unreleased loser's completion must fail
immediately with its actual response. Keep every later receipt/state/raw assertion
and2s bound. Re-run this same controlled order before selecting the repair.
CP3 direct observation of the same forced order executes1FAIL/26skipped in0.026s.
Three reviews: (1) while both real handlers are at append, first release completes
the supposedly unreleased loser with actual Ok(Receipt{id:1,version:1,events:1});
the selected winner is still held. This proves the fixture's shared-release
assumption, without blaming controller reconciliation or slow CI. (2) same Nix
canonical recovery target, no workload/deadline change or native DB. Raw cp3-order-
probe-direct-before.log. (3) actual first failure contains the loser's real receipt,
not setup/clock expiry. Keep this stronger two-reply observation after repair;
independent per-contender gates must reject neither valid winner nor loser phases.
CP3 candidate under the SAME forced registration inversion passes1/1 in0.090s,
26skipped (25filtered/one original ignored). Three reviews: (1) all192 schedules
now release the selected contender's own gate even when its waiter registers
second; both handlers must still reach actual append before either release. Both
winner orders, raw prefix/record/receipt/state, decisions and joins remain checked.
(2) actual Nix canonical recovery target, same two-thread runtime and2s bounds,
no database. Raw cp3-order-probe-after.log. (3) two fault wrappers share one actual
InMemoryStore Arc, so the real CAS authority is unchanged; separate gates remove
release theft rather than mask it with a longer deadline. The retained two-reply
observation directly fails an early loser completion. Remove only the disposable
registration hook, preserve its red/green patch/logs, then qualify full owners.
CP3 repaired-source full debug owner suite passes35/35,one original ignored,
in1.341s. Three reviews: (1) all192 two-thread schedules,256 concurrent generated
and256 unchanged serial histories plus baseline outcome/uncertainty/fold/capacity
laws pass with independent per-contender release gates and early-loser detection.
(2) all four package binaries run without filter via Nix; no native DB. Raw
cp3-gates-fixed-debug.log. (3) disposable registration hook is removed exactly
by reversing its saved patch; only FaultStore shared-Arc construction and the
canonical race helper remain changed. No product source, deadline, workload or
previous assertion removed. Total authored test delta316 within350; final strict
lint, optimized owners, production negative controls, full gate and CI remain.
The failed CI job's three actual foundation replay invocations each also receive
three reviews: (1) unchanged CBOR/value/wire decoder oracles, not processor/native
qualification; (2)2/1/1PASS,1.00s each,0ignored/filtered before the failed workspace
run; (3) exact existing corpus/source/selection, no shortened campaign. Raw job
log retains each invocation. Other workflow invocations require their own full
run-log review and do not make the failing required job green.
Clippy rejects only the new shared-Arc test constructor being eligible for const;
make that constructor const, with no waiver or assertion/workload change. Strict
all-target/all-feature Clippy then passes1.19s. The source/probe controls remain.
CP3 corrected optimized owner suite passes35/35,one original ignored,in0.503s.
Three reviews: (1) all192 fixed/256 concurrent/256 serial schedules and every
baseline persistence/uncertainty/typed outcome law execute with independent
per-contender gates and the stronger first-completion check. (2) all four release
package binaries, no filter/native DB; raw cp3-gates-fixed-release.log. (3) final
const constructor changes no runtime behavior; probe hooks are absent, exact
stored record/state/receipt and handler/teardown assertions unchanged. Re-run the
two original production mutation controls against this strengthened fixture before
the complete gate; a previously passing weaker fixture is not sufficient evidence.
CP3 repaired-fixture production controls: baseline1PASS; two mutants caught,
0missed/unviable/timeout. Each of three actual invocations receives three reviews:
(1) baseline completes all192 schedules; disabling confirmed-conflict recognition
returns Uncertain for the actual duplicate loser; accepting any same-ID content
wrongly succeeds on changed expected metadata. (2) actual canonical recovery law
selected via Nix/cargo-mutants, all other targets/cases excluded as recorded in raw
logs; no database. (3) each first failure is the intended receipt/mismatch oracle
after real append, no fixture poisoning, compiler failure or clock expiry. Raw
cp3-gates-fixed-controls/mutants.out retains exact commands/diffs/results. Independent
gates preserve and strengthen the original two falsifiers. Final full gate/CI open.
Correction evidence cp3-gates-fixed.tar.gz SHA256:
a8559a645c8d06bdfa4059e029916b338091b67091bbc28a3047d1ddee39a10b.
Before the full corrected gate, retain logs/source and clean only goal-generated
local Cargo cache. Retire its three superseded integration cross-target caches
after zero referrers verified; CP4's replacement outputs already pass/rooted.
Exact paths/deletion log retained. No shared/global cleanup, thread/deadline or
test change. C3 common dependency cache has a normal Nix root. The attempted C4
root fails because its unrooted dependency output was already reclaimed; no test
or source is lost and no rebuild is credited. Its final check outputs remain
rooted; rebuild dependencies only when the next actual C4 gate needs them.
Complete old-head CI review at7534d590: PR39649402288 has74 actual runtime
invocations; push39649396688 has54. Each receives three reviews: (1) compiler,
isolated library, foundation and existing adapter laws run; command owner35 passes
in PR workspace but the push's canonical race fails, so this source is not qualified.
(2) each has compiler2/1/1,27 library runs,replay2/1/1 and one workspace invocation.
PR workspace1629PASS104.473s; push369executed/368PASS1FAIL4.688s,1260notrun. Both
retain5existing exclusions. PR has40native PG invocations/two actual PG18.6 services;
push has20/one service (its Nix-native branch canceled). Each service runs seven
full profiles264/149/168/176/173/174/150 with0skips and13 explicit one-law selections;
start/stop observed. One echoed264 receipt is not another invocation; the failed
test's inner libtest print is not another workspace invocation. (3) raw exact
commands/current source, real services and unchanged assertions/workloads match
their owners. Zero-test libs earn no coverage; selected laws do not imply all
unselected cases ran. Passing PR CI does not override the independently reproduced
push failure, and these old-head passes cannot qualify the repaired source. Raw
cp3-original-final-ci-{pr,push}.log/counts.txt retained for delivery.
Forced-order scope clarification: the disposable global registration counter
inverts the first two gate waits, so the first schedule is deliberately inverted;
the remaining191 schedules execute unchanged. It proves the original permitted
ordering defect and the candidate's first inverted witness, not192 separately
forced inversions. Before final commit, repeat the disposable hook with pairwise
counter indexing so all192 schedules see that inversion. Keep the normal full-gate
source untouched while its build is active; restore/remove the probe and verify
exact bytes after this additional check. No reduced schedule or longer deadline.
Old-head fuzz39649402020: each of eight engine invocations receives three reviews:
(1) unchanged foundation decode/crash oracles, not new command fuzz qualification;
(2) four61s libFuzzer campaigns: wire10750110,header3042652,chunk1336857,value8361714;
four existing AFL/CMPLOG60s campaigns reach their limits and pass saved-crash
verdict, no native DB. (3) exact current7534d590 source/workloads/crash gates and
uploaded failure handling unchanged; no reduced inputs or hidden engine failures.
These bounded foundation campaigns cannot override the required race failure or
qualify repaired source. Raw cp3-original-final-fuzz.log retained.
Corrected full gate's three fresh replay invocations each receive three reviews:
(1) unchanged CBOR/value/wire foundation parser oracles; (2)2/1/1PASS,1.00s each,
0ignored/filtered, no database; (3) same source/corpus/selector, no new command
campaign or cached runtime credit. Raw cp3-gates-fixed-full.log; full nextest is
still running and receives no premature green credit.
Corrected full nextest42955380-7383-437d-93ac-dc49650724a2 completes1629PASS,
14slow,5existing exclusions,142binaries,in449.181s. Three reviews:
(1) all35 command laws execute, including192 fixed race schedules and256 each
generated concurrent/serial histories; late commits remain uncertain and exact
receipts/order remain checked. (2) actual full workspace invocation, no database
service and no native command-store credit; the three separate fresh replays
were reviewed above. All12 host outputs pass/rooted as cp3-fixed-check-*.
(3) four product modules and both repaired test owners byte-match the actual
7zzzj1idnh6cql2qwjnb43aqn5ph34kd-source. No weakened assertions, filters, workload,
deadlines or exclusions. Current source differs only in the two retained fixture
owners. Pairwise disposable ordering probe and current-head CI still required.
Pairwise disposable probe4b67548d-6490-435e-b041-2253dfcc83cf passes1/1 in0.080s,
26excluded (25filtered/one original ignored). Three reviews: (1) the actual192-
schedule owner still asserts both decisions, selected winner, held loser, complete
rows, receipts/state/retries and observed teardown; registration inversion is now
indexed modulo2. (2) actual recovery binary only in Nix, two-thread runtime, no
database or reduced2s bounds. (3) probe changes only gate registration ordering;
all product source is unchanged. Before retaining full192-inversion credit,
strengthen the disposable probe to signal each registration exactly once (so a
spurious repoll cannot leave a spare permit) and assert384 gate entries. This
additional check changes no retained source or gate/workload.
Counted pairwise probe b1cf41ad-040c-4a56-804d-27678c604441 passes1/1 in0.100s,
26excluded. Three reviews: (1) all192 schedules force first-entered registration
after second-entered registration; single-use registration signal prevents spare
permits, and the actual owner asserts384 entries before passing. Every original
receipt/CAS/whole-row/state/retry/handler/teardown oracle remains. (2) one actual
Nix recovery invocation, two threads, unchanged192 schedules and2s bounds; no DB.
(3) both disposable probe changes removed, repaired faults and unchanged recovery
byte-match the already qualified7zzz source. No probe retained, no product edit.
Raw cp3-pairwise-{probe,counted-probe}.log and final probe patch retained. The
original shared-gate defect's direct red remains in cp3-order-probe-direct-before.log; current
full gate and these forced positive witnesses qualify the selected repair locally,
not protected CI or native processor integration.
Final host/ordering and complete old-head CI raw evidence is preserved in
cp3-final-gates.tar.gz, SHA256
3fb393c55e4bc1359e3fc10512b8db4e166b5dc5fee07fc953677ccbfafd3b7a.
Old CI is retained as historical evidence of the red/green discrepancy; only
fresh repaired-head checks can establish protected delivery.
