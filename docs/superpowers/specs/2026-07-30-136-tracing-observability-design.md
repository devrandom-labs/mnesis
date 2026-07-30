# Design: `tracing` feature on `mnesis-store` — observability for #136

**Issue:** [#136 — Add observability (OpenTelemetry tracing and metrics)](https://github.com/devrandom-labs/mnesis/issues/136)
**Date:** 2026-07-30
**Status:** Approved (Option 1 below)

## Summary

Add a `tracing` cargo feature to `mnesis-store` that compiles inline
`tracing`-crate spans into the facade seams (load / save / execute / saga
react / snapshot hydrate+commit / projection commit) and one INFO event at the
subscription catch-up→live boundary. Off by default; when off, the
instrumentation does not exist in the binary. Telemetry names are declared
diagnostic output — **not** semver surface — via a new STABILITY.md clause.

No new crate. No metrics crate (deferred). No adapter-internal spans (separate
additive cards). No loop/lag telemetry (Agency's, per the no-runner boundary).

## Why inline-behind-a-feature (Eventuous style)

The decisive property: only code *inside* the crate can explain latency, not
just report it. A wrapper can say "load took 800 ms"; inline spans can say
"10 000 events replayed, snapshot missed". That depth is what #136 identifies
as Eventuous's biggest differentiator, and it is unreachable from any
decorator.

The two costs of inline instrumentation both have cheap, precedented fixes:

1. **Frozen vocabulary.** Span names are things people build dashboards on.
   Fix: a STABILITY.md clause declaring telemetry output unstable across
   minors — the same move tokio makes (its runtime instrumentation sits behind
   a `tracing` feature with the telemetry surface explicitly unstable;
   tokio-console requires the `tokio_unstable` cfg for this reason).
2. **Code entanglement.** Fix: instrumentation is confined to a handful of
   facade call sites (listed below), never sprinkled through `wire.rs`,
   codecs, or per-event hot loops.

Verified facts underpinning the design:

- `tracing` supports no_std + alloc (`default-features = false`, per
  docs.rs/tracing) — `mnesis-store`'s no_std tier is no_std + alloc, so the
  feature is **independent of `std`**.
- `tracing` is already in the workspace lockfile, cargo-audit/deny scope, and
  workspace-hack (via `examples/axum-todos`), so the optional dep adds no new
  audited surface.

## Design

### Feature wiring

```toml
# crates/store/Cargo.toml
[features]
tracing = ["dep:tracing"]
std = [ ..., "tracing?/std" ]

[dependencies]
tracing = { workspace = true, optional = true }
```

The workspace root entry becomes `default-features = false` so the no_std
build holds; `examples/axum-todos` re-enables what it needs additively
(members cannot *disable* a workspace dep's default features, only add).

### Span inventory (first cut — facade seams only)

| Name | Kind / level | Site | Fields |
|---|---|---|---|
| `mnesis.aggregate.load` | span, DEBUG | `repository.rs` `Repository::load` | `aggregate` (type name), `stream` (id label); records resulting `version` |
| `mnesis.aggregate.save` | span, DEBUG | `repository.rs` `Repository::save` | `aggregate`, `stream`, `events` (count), `expected` (version); records assigned `$all` `position` |
| `mnesis.aggregate.execute` | span, DEBUG | `execute.rs` `CommandRepository::execute` | `aggregate`, `stream`; parents the load/save spans |
| `mnesis.saga.react` | span, DEBUG | `saga.rs` `react_and_save` | `saga`, `stream`, `intents` (count) |
| `mnesis.snapshot.hydrate` | span, DEBUG | `snapshot.rs` (`snapshot` + `tracing`) | `stream`; records `hit` (Found / Stale / Absent) |
| `mnesis.snapshot.commit` | span, DEBUG | `snapshot.rs` | `stream`, `version` |
| `mnesis.projection.commit` | span, DEBUG | `projection.rs` stepper persist (trigger fired / `flush`) | `id`, `position` |
| `mnesis.subscription.caught_up` | event, INFO | `subscription_cursor.rs` at the `Step::CaughtUp` emission | `position` (last delivered) |

Deliberately **no** span on: per-event `Projection::advance`, codec
encode/decode, `wire.rs` frame build, upcaster transforms, adapter internals.
Per-event sites are hot paths (span-per-event is overhead and noise); the
profiling questions those sites answer are bench territory today. Any of them
can be added later behind the same feature — additive beats frozen-wrong
(rule 9).

### Levels and errors

- Per-operation spans: DEBUG (high volume in production).
- Rare boundary transitions (`caught_up`): INFO.
- No TRACE-level emission in the first cut.
- **Errors are not duplicated as tracing events.** Failures already propagate
  as typed `Result`s (rule 3); the span closing abnormally is visible to
  subscribers, and double-reporting would create two sources of truth.

### STABILITY.md clause (new)

> Telemetry emitted under the `tracing` feature — span/event names, field
> names, and levels — is diagnostic output, not semver surface. It may change
> in minor releases. Build dashboards against it with that understanding.

### Out of scope (explicitly)

- **Subscription lag, projection loop, dispatch telemetry** — properties of
  the runner loop; mnesis ships no runner (the boundary that retired
  `mnesis-framework`). Agency owns these. The `CaughtUp` step and
  `GlobalSeq`/`AllPosition` values are already exposed so a runner can compute
  lag.
- **Metrics** (`metrics` crate or OTel metrics) — deferred. Spans bridge to
  OpenTelemetry via `tracing-opentelemetry` today; a metrics layer can be
  added later without new mnesis surface.
- **Adapter-internal spans** (fjall compaction/flush, postgres pool) —
  separate additive cards per adapter, same feature-gate pattern.
- **Kernel (`mnesis`) instrumentation** — pure functions (`handle`, `apply`,
  `replay`) are deterministic CPU work already parented by the facade spans;
  nothing to await, nothing to attribute.

## Testing

- **Compile-surface:** the flake's `--all-features --all-targets` clippy lints
  the feature on; default-feature builds prove it off. The no_std gates
  (thumbv7em/wasm32) must pass with `tracing` enabled and `std` off.
- **Behavior (rule 8 — exact assertions):** one integration test with a
  capturing `tracing::Subscriber` installed via
  `tracing::subscriber::with_default`, asserting the **exact** span names and
  key fields emitted by a load → execute → save round-trip and the single
  `caught_up` INFO event at the subscription boundary. Wired through the self
  dev-dependency pattern (dev-dep re-enters the crate with
  `features = ["tracing"]`) so the default-feature `nix flake check` nextest
  run covers it — same mechanism the kernel uses for its `testing` feature.
- **Rule 7 categories:** this feature adds no persistence, no protocol state,
  and no concurrency of its own; the sequence-shaped assertion is the span
  nesting (`execute` parents `load`/`save`), covered by the test above.

## Alternatives considered

1. **Inline spans behind a `tracing` feature (CHOSEN).** Pro: can explain
   latency from inside; zero cost when off (code absent); nothing new to
   maintain structurally. Con: cfg-gated call sites in the facade; telemetry
   names ship inside a 1.0 crate — mitigated by the STABILITY.md clause.
2. **Decorator in a new `mnesis-telemetry` 0.x crate (axum/tower style).**
   Pro: frozen crates untouched; vocabulary free to change at 0.x; opt-out is
   doing nothing. Con: blind to internals forever (can time `load`, can never
   say why); a crate to own for ~200 lines; users must remember to wrap.
3. **Decorator inside `mnesis-store` behind a feature.** Worst of both:
   still blind, and its names still freeze with the crate. Rejected.
4. **Ship nothing (status quo of every Rust ES crate).** Every consumer
   rebuilds the same wrapper badly; the differentiator #136 identifies is
   forfeited. Rejected — the issue exists because this is not good enough.

## Issue #136 open questions — resolved

- *On by default or feature-gated?* → Feature-gated, off by default.
- *`metrics` crate too?* → Deferred; spans first, metrics additive later.
- *Fjall adapter spans?* → Separate additive card, same pattern.
- *Log levels?* → DEBUG per-operation spans, INFO rare boundary events, no
  TRACE in the first cut; errors ride `Result`, never duplicated as events.
