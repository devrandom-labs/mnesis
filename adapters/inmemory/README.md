# mnesis-inmemory

In-memory event-store adapter for [Mnesis](https://github.com/devrandom-labs/mnesis) — a full `RawEventStore + WakeSource + SnapshotStore` (plus `StreamLister`/`AtomicAppend` behind `export`/`import`) backed by ordinary maps. Ideal for tests, examples, and prototyping where persistence isn't needed.

It is also `mnesis-store`'s own test fixture (via a path-only dev-dependency), so it tracks the store contract exactly.

## Quickstart

```toml
[dependencies]
mnesis-inmemory = "0.1"
```

```rust
let store = InMemoryStore::default().into_store();
```

## Features

| Feature | Adds |
|---------|------|
| `export` / `import` | Backup/restore (`StreamLister` + `AtomicAppend`) |

## MSRV & stability

MSRV **1.95**. Ships in the **0.x tier**. See [STABILITY.md](../../STABILITY.md).

## License

Licensed under your choice of [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE).
