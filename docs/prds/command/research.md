# Command processor research and evidence

Updated: 2026-10-21. Contract owner: [command PRD](../command.md).
This records planning and CP0 qualification. Protected delivery and CP1–CP9
implementation remain outstanding; the latest CP0 section owns its local status.

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
