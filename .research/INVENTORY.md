# Repository inventory

Repository map inspected: `crates/` owns the kernel, macros, store contracts,
wake implementations, conformance kit and build support; `adapters/` owns
in-memory, Fjall and Postgres; `examples/` owns public consumers; `fuzz*`,
`mutants-gate`, `.github/` and `flake.nix` own existing verification machinery.
No changes to that machinery are included in PRD preparation.

The [PRD capability table and ownership map](../docs/prds/postgres.md#completion-scope)
are authoritative for this feature's source findings. Its dependency table,
PG0 contract decisions and PG9 gates record the remaining inventory dimensions.

Reference projects inspected: Bombay PRD backlog and feature records;
mnesis-bombay instructions, manifest and lock; Trive's Nix environment entry
point and flake; CESR's repository guidance, environment entry point and layout.
Their patterns inform the PRD; they do not change Mnesis's dependency selection.

First retained implementation batches preserve all existing owners and executable
core contracts. Adapter now has one Arc-owned listener, one timer per armed
registration and one boxed owned cursor per raw scan. Finite immutable history
releases leases between pages. Remaining credible audit findings: practical byte
bounds (current experiment), phase-aware commit errors, qualified conflicts,
non-narrowed unsigned scalar storage, versioned migrations and real DB gate.
These prevent any fixed-point or production-completion claim. Full capability
and public surface minimization remain PG9 obligations.

2026-10-18 PG7b uncommitted candidate audit: private store/export module owns one
read-only RR transaction/task/pool lease per public associated Session. Session
is reachable through existing ConsistentExporter, no constructor/public mode knob
or independent runner. Requests are a private Key/List/Events sum, typed bounded
channel plus original one-shot/task causes; Window is immutable deadline+Limits.
Existing core GAT lifetime/ExportError and existing page SQL/row decoding reused;
no new dependency/lockfile/framework/runtime/tool, no unwrap/expect/unsafe/panic
in the318-line module. Pool authoritative bounds, no detach/leak. Driver socket
closure required supported local native client checks for blocked SQL; preserve
shorter caller check interval, one parameter owns its1000ms value. Native203 and
fresh source controls reviewed. All-default/export strict Clippy passes, native
Linux plus5required smokes pass. Full workspace Nix gate running, not final
whole-repository convergence: existing Python feature/fuzz machinery, AFL crash
gate proof, advisories, native TLS/durability/pool/session profiles, backup/
restore/foreign history, capacity and release remain explicit PG7–PG9 work.
Credible native lifecycle follow-ups: canceled pending close while SQL blocked;
caller stricter check interval; session outliving original store; connection-loss
profiles hidden from server socket check. No claims these were proven by initial
native11 or mutation percentages. Two final fresh whole-repository audits still
required before whole-adapter completion.

## Command processor — 2026-10-21

Current consumer layer: crates/command depends on kernel/store; adapters do not
depend on it. Four private modules/five source files retain model/codec, engine,
host and captured trace-context ownership. Eleven roots include source responsibilities, typed causes/
codec failures, validated public acceptance reading and owned Worker completion.
Tower admission uses Tokio PollSender plus the worker's short parking_lot gate.
Engine owns the active pending original; its Drop transfers it to a per-worker
handoff, read only after native completion. Shutdown retains typed JoinError and
the original together. Caller callbacks/failed values run outside admission.
Core lifecycle/resource PR417 is protected-merged with verified exact tree;
the trace candidate is not yet merged. Current candidate has1641 physical product
lines (+322net against signed1539509a) and597net new test/helper lines, within
amended trace387/649 and1750whole-product budgets. Trace adds no root export,
collector, exporter or runtime; disabled Scope remains zero sized. Native span
factories and Call construction are shared across modes, removing duplicated
cfg-disabled functions. Source/type/data/record contracts and all11roots unchanged.
Current core is unpublished/release-disabled; CP5 telemetry,
native/performance qualification, documentation and delivery remain required.
[Feature research](../docs/prds/command/research.md#cp1a-mechanical-extraction)
records extraction provenance; later sections own typed outcome, forced-race,
format/reader and lifecycle/resource evidence. Local qualification is not merged,
native or published delivery. No normal exporter SDK or new runtime is selected.

Metrics dependent candidate against c13ef43f: five private modules/six source
files,1907physical product/+266net; meter.rs105 owns native facade registration,
finite labels and resource RAII. Trace owns one outcome mapping for both optional
outputs. Engine derives pending gauge transitions from its sole original Option;
worker reads first known stop origin from existing Lifecycle only after native
completion. No second protocol ledger, collector/runtime/framework/public root.
11exports unchanged. Actual four modes45/49/46/50 and release50,69mutants37caught/
32compiler-only, all12host checks pass;13source owners cmp. Full workspace default
1639/145binaries does not execute cfg-disabled instrumentation cases. Metrics
app exporter and instrumentation overhead remain unqualified; no full CP5 claim.
