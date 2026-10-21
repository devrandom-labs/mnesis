# Tower compatibility experiment

2026-10-21. Experiment decision: do not adopt
`tower-actor` 0.2.0 unchanged as the processor host. This is an executable fit
check, not implementation or release acceptance for CP0–CP9.
After this experiment, the user selected Tower Service over the owned worker as
a PRD requirement. That interface still needs its own acceptance evidence.

## Inputs and method

Used the existing Mnesis Nix shell and Rust 1.99.0 on Darwin arm64. Disposable
consumer: `/tmp/mnesis-tower-fit.Pr50LQ`. No product files or dependencies changed.
Published `tower-actor` 0.2.0 records upstream commit
`6416fa4b1165356cf06ffb4d75851725734b2a21`, also the repository's latest commit
observed through GitHub's API (2025-04-10). Manifest license: MIT OR Apache-2.0.
Tower resolved to 0.5.3, Tokio 1.53.1 and tokio-util 0.7.20. The package introduces
thiserror 1.0.69 alongside the consumer's thiserror 2 and uses pin-project.
Exact dependency trees, checksums, source, commands and logs are in the
[experiment bundle](evidence/tower-fit.tar.gz). No advisory or performance claim.

The prototype copies the archive's engine, model and six model tests unchanged.
Only a neutral counter domain and Tower worker fixtures are authored (237 Rust
lines). It executes actual Mnesis commands and inspects actual raw acceptance
records. No store or duplicate-detection implementation was substituted.
Initial fixtures required two compile corrections: missing archived unit-test
file, then shared `Store<S>` ownership rather than cloning `InMemoryStore`.
Those failed compile logs are retained; neither executed tests.

## Actual results

| Execution | Result |
| --- | --- |
| Archive Mnesis 0.4.0, debug compatible selection | 11 passed; one worker-error requirement explicitly filtered for separate execution. |
| Archive Mnesis 0.4.0, debug failure probes | Two failed; 11 filtered. |
| Archive Mnesis 0.4.0, release failure probes | Same two failed; 11 filtered. |
| Destination Mnesis 0.5.3, release full matrix | 11 passed, two failed; zero ignored or filtered. |

The eleven passing cases comprise six original codec/model tests and five new
composition cases: canceled active/queued callers with exact stored order and
duplicate recovery; capacity-one backpressure across cloned handles; actual
commit followed by lost reply and recovery in a fresh engine; typed domain
rejection without append; and an explicit user-supplied shutdown report channel.
Gates force overlap; the worker cannot append before cancellation is exercised.
Every execution received three reviews in the bundle: contract/counterexample,
actual selection/results, and source identity/false-positive analysis.

Two required properties failed and remain red:

1. A worker returning typed `Worker(73)` after receiving a request produces only
   `ActorHungUp` at the caller. Its original terminal cause is lost. Ordinary
   per-request errors remain typed; these are separate paths.
2. After successful readiness reservation, the worker closes and drops its
   receiver. A subsequent valid `call` returns a response future which does not
   resolve within five seconds while the service handle stays alive. Release
   diagnostics show all preceding steps completed. This is a reproducible
   bounded non-completion witness, not an assertion of an infinite wait.

Source review explains the lifecycle gaps: `Actor::new` discards its spawned
task's `JoinHandle`, and the response future only waits on a oneshot. The actor
uses a reserved `PollSender` permit; its closed-state check does not necessarily
reflect receiver closure. The exact deeper channel behavior is not rewritten
or patched by this experiment.

## Consequence for the PRD

Retain a processor-owned Tokio worker and observable completion. Evaluate a
small `tower::Service` implementation over its handle in CP0; prove admission
semantics, cancellation, typed errors and shutdown before releasing it. Tower
readiness applies backpressure, whereas the current handle rejects a full queue
immediately. Do not silently change that contract. Sequential execution remains
the worker's responsibility; the actor crate does not enforce it for arbitrary
user-provided workers.

Reusing tower-actor would still require shutdown signaling, reporting, retained
terminal causes, request-context error mapping and a resolution of the reserved-
readiness race. This experiment does not establish a net reduction in code or
maintenance, so it does not justify that dependency now. No fork, replacement
actor framework, upstream issue or implementation change was created.

Limits: in-memory backend; fresh-engine recovery rather than OS-process failure;
no Postgres composition, production shutdown implementation, throughput or memory
benchmark. Those remain owning PRD acceptance work.

Primary sources: [package](https://docs.rs/tower-actor/0.2.0/tower_actor/),
[worker and service implementation](https://github.com/penumbra-zone/tower-actor/blob/6416fa4b1165356cf06ffb4d75851725734b2a21/src/actor.rs),
[reply future](https://github.com/penumbra-zone/tower-actor/blob/6416fa4b1165356cf06ffb4d75851725734b2a21/src/future.rs).
