# mutants-gate

The verdict tool for the mutation-testing gate. Turns a `cargo-mutants` run into
a pass/fail that **cannot be vacuously green**: it fails on a survivor, a
timeout, an interrupted run (fewer outcomes than candidates), a per-function
viability collapse against the committed `mutants-baseline.json`, or an
unaccounted new function. Ported from the sibling `bombay` repo's gate.

The mutation sweep is **not** part of `nix flake check` — `cargo-mutants`
rebuilds and re-tests once per mutant (minutes to hours), the same quarantine as
fuzzing. It lives in the flake *package* `nix build .#mutants` and the
`.github/workflows/mutants.yml` workflow (nightly + PR, single blocking
`mutants-gate` check). This crate's own unit tests (the verdict logic) DO run in
the normal `nix flake check` nextest.

## Scope

The sweep is scoped to the kernel crate `crates/mnesis/src/**` (pure, fast,
deterministic tests — the highest-value mutation target). Widen the `--file`
globs in `flake.nix` (the `mutants` package) as coverage grows to the store.

## Commands

```
mutants-gate check <mutants.out-dir> <baseline.json>   # exit 0 pass / 1 fail / 2 usage
mutants-gate emit-baseline <mutants.out-dir>           # print a baseline skeleton
```

## Seeding the baseline (one-time, per scope change)

`mutants-baseline.json` records the per-`file::function` floor of viable mutants.
It must be generated from a real sweep on a machine where the test suite runs
(not the sandbox). To seed or re-seed after widening scope:

```
nix build .#mutants -L                 # runs the sweep, writes ./result/mutants.out
cargo run -p mutants-gate -- \
  emit-baseline result/mutants.out > mutants-baseline.json
$EDITOR mutants-baseline.json          # REVIEW: a should-be-tested 0-viable
                                       # function belongs in floors, not known_zero
git add mutants-baseline.json
```

Review matters: `emit-baseline` floors every currently-viable function and lists
every structurally-0-viable one as `known_zero_viable`. A function that *should*
have viable mutants but shows zero (because a test is missing) lands in
`known_zero_viable` — catch it here, by eye, not by the machine.
