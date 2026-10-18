# Research sources

Exact selected versions, inspected local contracts, upstream URLs, access date,
applicability and limitations are recorded once in the feature PRD:

- [Locked dependency verification](../docs/prds/postgres.md#locked-dependency-verification).
- [Primary sources](../docs/prds/postgres.md#primary-sources).

The installed research skill directory had no readable `SKILL.md`; preparation
used the canonical source at
`~/nix-config/modules/home/devrandom-research/skill/SKILL.md` and read its full
`references/protocol.md`. Resume with the installed skill if available.

- 2026-10-18, PG1: PostgreSQL 17 window/conditional evaluation docs and locked
  SQLx 0.8.6 FromRow/QueryBuilder; native byte/query-plan evidence in feature
  research. PG3: same-version transaction source, PostgreSQL protocol frames and
  SQLSTATE appendix. `40003` cannot share the definite-rollback classification of
  `40001`/`40P01`. Candidate test transport uses existing Tokio/tempfile only.

- 2026-10-18 audit follow-up: RustSec RUSTSEC-2026-0221
  https://rustsec.org/advisories/RUSTSEC-2026-0221.html identifies unconditional
  Send/Sync on `StackSlot<T>` with !Send tags, patched in event-listener >=5.4.2.
  Locked SQLx 0.8.6 pool `on_closed` is `Event<()>` and uses heap EventListener;
  wake-nostd also uses untagged Event::new/heap listen. No current !Send tag or
  StackSlot call was found in these paths. This limits applicability but does not
  replace the PG9 dependency remediation: evaluate the compatible patched update
  with no_std/platform gates rather than add an ignore. Current lock is 5.4.1.
