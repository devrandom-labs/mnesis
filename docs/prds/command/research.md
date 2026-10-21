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
