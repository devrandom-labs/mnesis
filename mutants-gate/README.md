# mutants-gate

The verdict tool for the mutation-testing gate. Turns a `cargo-mutants` run into
a pass/fail that **cannot be vacuously green**: it fails on a survivor, a
timeout, an incomplete or inconsistent report, a per-function
viability collapse against the committed `mutants-baseline.json`, or an
unaccounted new function. Ported from the sibling `bombay` repo's gate.

The mutation sweep is **not** part of `nix flake check` — `cargo-mutants`
rebuilds and re-tests once per mutant (minutes to hours), the same quarantine as
fuzzing. It lives in per-crate flake *packages* and the
`.github/workflows/mutants.yml` workflow, which collapses to one blocking
`mutants-gate` check. This crate's own unit tests (the verdict logic) DO run in
the normal `nix flake check` nextest.

## Scope — rolled out one crate at a time

A whole-workspace sweep is ~1542 mutants / many hours — over GitHub's 6h job cap
and unwieldy to close. So the gate is rolled out **per crate**: each crate has

- `nix build .#mutants-<crate>` — the verdict (sweep + `check` against its baseline),
- `nix build .#mutants-sweep-<crate>` — the seeder (sweep + `emit-baseline`, rejects unclean runs),
- `.mutants-baselines/<crate>.json` — its committed ratchet.

Rolled-out crates are listed in the nightly matrix in `mutants.yml` (start:
`mnesis`); PRs run `cargo-mutants --in-diff` (changed lines only) instead of a
full sweep. `mnesis-postgres` is excluded — its tests need a live DB the sandbox
lacks (tracked in #351). Crate set lives in `flake.nix`'s `mutantCrates`.

## Commands

```
mutants-gate check <mutants.out-dir> <baseline.json>   # exit 0 pass / 1 fail / 2 usage
mutants-gate emit-baseline <mutants.out-dir>           # print a clean baseline skeleton
```

## Rolling out a new crate (seed its baseline)

Each crate's baseline records the per-`file::function` floor of viable mutants.
It must be generated from a real sweep on a machine where the test suite runs
(not the CI-less bash sandbox — its test binaries hang). To roll out `<crate>`:

```
nix build .#mutants-sweep-<crate> -L          # sweep, writes ./result/mutants-baseline.json
cp result/mutants-baseline.json .mutants-baselines/<crate>.json
$EDITOR .mutants-baselines/<crate>.json       # REVIEW: a should-be-tested 0-viable
                                              # function belongs in floors, not known_zero
git add .mutants-baselines/<crate>.json
# then add "<crate>" to the mutants-sweep matrix in .github/workflows/mutants.yml
```

Any survivor the sweep prints (`MISSED`/`TIMEOUT` in the build log) is a real
test gap: kill it with a test, or — only for a mutant no test *can* kill
(equivalent, non-terminating, unreachable, diagnostic) — add a documented
`exclude_re` entry to `.cargo/mutants.toml`. Review matters: `emit-baseline`
floors every viable function and lists every 0-viable one as `known_zero_viable`;
a function that *should* have viable mutants but shows zero (a missing test) lands
in `known_zero_viable` — catch it here, by eye, not by the machine.

## Report integrity

Both commands read `mutants.json` and `outcomes.json`. Each candidate must have
exactly one matching outcome. Identity includes package, file, source span,
replacement, genre and the enclosing function descriptor; missing identity
fields, duplicates, unknown substitutions and missing outcomes fail. Display
names and diffs are derived and excluded from identity. The fields were checked
against [cargo-mutants 27.1.0's serializer](https://github.com/sourcefrog/cargo-mutants/blob/v27.1.0/src/mutant.rs).
The retained real candidate fixture was captured with that version's
`mutants --list --json --package mutants-gate`, before this gate change.
Original audit fixture bytes remain in `tests/audit_cases/*/original-*.json`;
the active fixtures add the required identity fields to test the original
failure domains, with exact rejection diagnostics.

A normal run must contain exactly one successful unmutated baseline. When
cargo-mutants intentionally used `--baseline skip`, append `--skip-baseline`
to the gate command; that policy requires zero baseline entries. A mutant
`Success` or `Failure` cannot prove a caught mutation and is rejected.
`Unviable` remains distinct from a failed test run and contributes no viable
coverage. Check-only mutation runs cannot satisfy this test gate.

Seeding also requires the candidate/outcome match and rejects survivors,
timeouts and invalid runs before printing any JSON. Valid unviable candidates
remain explicit known-zero entries for review. No complete mutation campaign
is implied by the gate's own unit/process tests.
