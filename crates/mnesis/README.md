# mnesis

Event-sourcing kernel — aggregates, events, versioning, command handling. No `Box<dyn>`, no runtime downcasting, no `std` allocation.

See the [root README](../../README.md) for usage and examples.

## Verification

Repository verification tooling includes the following checks. Results apply to
the executed cases and workloads:

| Technique | What it checks |
|-----------|---------------|
| Unit tests + edge cases | Specified behavior and boundaries |
| Property-based testing (proptest) | Sampled invariants and boundaries |
| Compile-failure tests (trybuild) | Invalid code fails to compile |
| Static assertions | Send, Sync, size, trait bounds enforced at compile time |
| Miri | Checks executed paths under strict provenance |
| Mutation testing (cargo-mutants) | Checks the tested viable mutations |
| Benchmarks (criterion) | Measures the specified workloads |
| Doc tests | Active documentation examples compile and run |
| Architecture tests | Kernel imports nothing from outer layers |

## License

Licensed under your choice of [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE).
