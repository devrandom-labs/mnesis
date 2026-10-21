# Immutable contract

## Objective

Complete the Postgres adapter under [the feature PRD](../docs/prds/postgres.md).
The user authorized full implementation, protected PR delivery and publication
on 2026-10-18. Implementation follows feature-local PG0 verification.

## Preserved behavior and guarantees

The PRD's ownership map and PG0 contract decisions define the preserved laws:
atomic persistence, no skipped events, correct resume bounds, faithful transaction
outcomes, compatible recovery and the existing runtime-neutral store boundary.
Resolve contradictory current claims explicitly; never weaken them by omission.

## Performance contract

PG1 defines bounded read resources; PG2 defines recovery timing; PG8 defines the
capacity workloads. Measure the actual baseline before selecting tuning targets.

## Operator constraints

Use the canonical devrandom-research protocol and existing repository owners.
No new runtime, framework, custom development tool or project Python. Preserve
Labcast's command identity, deduplication and stale-worker ownership. Keep
standalone subscriber checkpoints conditional on a selected consumer.

## Rust and Nix conventions

- Use Nix for development and verification.
- Prefer sum/product modeling over boolean or sentinel protocols.
- Use `thiserror` unless an evidenced constraint prevents it.
- Minimize public API and redundant type specification without erasing domain types.
- Preserve or improve representative performance.

## Command processor delivery contract — 2026-10-21

The Postgres objective above is completed history. The user now authorizes the
entire CP0–CP9 command processor delivery in docs/prds/command.md, including
protected PRs, publication and labcast-server consumption, without a token budget.
The PRD owns current command identity, persistence, error, lifecycle and telemetry
contracts. Generic acceptance/retry machinery moves into mnesis-command; Labcast
retains business policy, authorization, worker fencing and effects. Preserve the
source archive and all existing staged/unstaged/untracked work. Mobile stays out
of scope. Every actual test run receives the three required distinct reviews.
Completion requires merged code, verified registry publication and clean registry
and server consumers, not merely local acceptance. No new framework/runtime,
custom tooling, project Python or bypass of protected delivery is authorized.
