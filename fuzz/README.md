# Fuzzing the mnesis-store parse surface

Two-layer fuzz gate, ported from the sibling `cesr` repo.

## What is fuzzed

The untrusted-byte decoders in `mnesis-store` — the paths that read bytes from
disk or the network and must **never panic** on truncated, tampered, or
bit-rotted input (a panic on untrusted input is a DoS bug). Bodies are defined
once in `fuzz-common/src/lib.rs` and shared by both engines:

| target | function | surface |
|--------|----------|---------|
| `wire_decode_frame` | `wire::decode_frame` | the canonical on-disk event frame |
| `cbor_decode_header` | `cbor::decode_header` | backup-box chunk header |
| `cbor_decode_chunk` | `cbor::decode_chunk` | backup-box chunk body (crc-checked blocks) |
| `value_event_type` | `EventType::from_bytes` | UTF-8 + length-cap validator |

## Layout

- `fuzz-common/` — shared target bodies. Depends only on `mnesis-store` (no
  fuzzing-engine deps), so it can sit in the stable replay graph.
- `fuzz/` — the bolero (libFuzzer) crate. `tests/*.rs` hold one `bolero::check!`
  per target; the test name **is** the target name.
- `fuzz-afl/` — the afl.rs (AFL++/CMPLOG) crate. `src/bin/*.rs` call the same
  `fuzz-common` bodies via `afl::fuzz!`.

Each is its **own** isolated workspace (empty `[workspace]`) so the fuzzing
dependency tree never enters the main workspace's audit/deny/hakari/dev surface.

## Two enforcement layers

1. **Blocking, every PR — stable corpus replay via `nix flake check`.** The
   `mnesis-fuzz-replay` check runs `cd fuzz && cargo test` on the pinned stable
   toolchain: bolero's `DefaultEngine` replays every committed corpus seed plus
   bounded-random inputs per target, no nightly. This is part of the normal gate.

2. **Deep coverage-guided fuzzing — `.github/workflows/fuzz.yml`.** Nightly + PR
   smoke + dispatch. Two engines (bolero/libFuzzer under ASan on pinned nightly,
   afl.rs/AFL++/CMPLOG on stable), time-boxed (60s/target on PRs, 120s nightly).
   The `fuzz-gate` job collapses the matrix into one blocking required check.
   Corpus compounds night over night via per-engine artifacts (90-day retention).

## Running locally

```
cd fuzz && cargo test                       # deterministic replay (what the gate runs)
cargo bolero test wire_decode_frame          # drive one target (needs cargo-bolero)
```

## Crash regressions

When a run finds a crash, save the reproducer under
`fuzz/tests/__fuzz__/<target>/corpus/` and commit it — bolero's `DefaultEngine`
replays it on every stable `cargo test`, so a fixed bug stays fixed. The corpus
otherwise grows and persists in nightly CI; there are no committed seeds yet.

## Adding a target

1. Add a `pub fn` body to `fuzz-common/src/lib.rs`.
2. Add a `bolero::check!` test in `fuzz/tests/` (test name = target name).
3. Add a `[[bin]]` + `src/bin/<name>.rs` to `fuzz-afl/`.
4. Add the name to both matrix lists in `.github/workflows/fuzz.yml`.
