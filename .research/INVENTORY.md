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
depend on it. Four private modules retain model/codec, engine and host ownership;
six root responsibility types expose the public contract. Tower admission uses
Tokio PollSender plus the worker's short parking_lot admission gate. The native
JoinHandle preserves observed task completion. CP1 is unpublished/release-disabled.
[Feature research](../docs/prds/command/research.md#cp1a-mechanical-extraction)
records intentional source changes and actual verification. Erased errors,
resource distinctions, forced races, accepted-fact decoding, instrumentation,
real adapters and publication remain explicit CP2–CP9 work, not completed audits.
