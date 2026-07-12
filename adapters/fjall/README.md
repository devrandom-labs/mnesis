# mnesis-fjall

Embedded **[fjall](https://crates.io/crates/fjall) LSM-tree** event-store adapter for [Mnesis](https://github.com/devrandom-labs/mnesis) — the default on-device store for the IoT/mobile-first target. Implements `RawEventStore`, `WakeSource`, `AtomicAppend`, `StreamLister`, and the snapshot/projection `SnapshotStore` seams.

Reads are zero-copy: with fjall's `bytes_1` feature the `bytes::Bytes` inside a `PersistedEnvelope` *is* the same Arc-counted buffer fjall handed back — no alloc + memcpy on the hot path.

## Quickstart

```toml
[dependencies]
mnesis-fjall = "0.1"
```

```rust
let store = FjallStore::builder(path).open()?.into_store();
```

## Features

| Feature | Adds |
|---------|------|
| `snapshot` | Aggregate snapshot persistence |
| `projection` | Projection-state persistence (all-levels LZ4 partition) |
| `export` / `import` | Backup/restore (`StreamLister` + `AtomicAppend`) |

## MSRV & stability

MSRV **1.95**. Ships in the **0.x tier** — storage internals iterate without forcing kernel major bumps. The on-disk frame format is major-bounded (see STABILITY.md). See [STABILITY.md](../../STABILITY.md).

## License

Licensed under your choice of [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE).
