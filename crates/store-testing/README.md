# mnesis-store-testing

The **executable conformance kit** for [Mnesis](https://github.com/devrandom-labs/mnesis) store adapters — the store contract as a runnable test suite. Writing a `RawEventStore + WakeSource` adapter? This crate pins what "correct" means: inclusive vs. exclusive read bounds, conflict rejection with nothing landing, catch-up→live ordering, lost-wakeup defense.

## Quickstart

Add as a **dev-dependency** and invoke the macros over a factory that yields a fresh store per test:

```toml
[dev-dependencies]
mnesis-store-testing = "0.1"
```

```rust
mnesis_store_testing::conformance!(|| async { (MyStore::new(), ()) });
```

Four macros: `conformance!` (core matrix), `conformance_atomic_append!`, `conformance_snapshot!`, `conformance_lifecycle!` (persistent adapters). Each generates one named `#[tokio::test]` per check. The crate docs carry the full **writing-a-store-adapter guide**.

## Features

| Feature | Adds |
|---------|------|
| `snapshot` | `SnapshotStore` conformance checks |
| `atomic-append` | `AtomicAppend` conformance checks |

## MSRV & stability

MSRV **1.95**. Ships in the **0.x tier** (testing surface iterates faster than the frozen kernel). See [STABILITY.md](../../STABILITY.md).

## License

Licensed under your choice of [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE).
