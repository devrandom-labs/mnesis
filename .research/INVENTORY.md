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
