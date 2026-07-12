# mnesis-store

The persistence edge layer for [Mnesis](https://github.com/devrandom-labs/mnesis) — codecs, envelopes, the append/read seams adapters implement (`RawEventStore` + `WakeSource`), schema upcasting, snapshots, projections, subscriptions, and backup export/import. Kernel-pure in, bytes-on-disk out.

`no_std + alloc` capable (disable default features); the subscription loop is generic over `WakeSource`, so it pulls in no runtime.

## Quickstart

```toml
[dependencies]
mnesis-store = "0.1"
```

Pick a storage adapter to back it: [`mnesis-fjall`](../../adapters/fjall) (embedded LSM), [`mnesis-postgres`](../../adapters/postgres), or [`mnesis-inmemory`](../../adapters/inmemory). See the [root README](../../README.md) for a full lifecycle example.

## Features

| Feature | Adds |
|---------|------|
| `std` *(default)* | `std::error::Error` bridge; disable for `no_std + alloc + core::error::Error` |
| `serde` / `json` | Serde codec; JSON alias |
| `bytemuck` / `rkyv` | Zero-copy POD / archived codecs |
| `subscription` | Generic catch-up-then-live-tail loop + `WakeSource` traits (dep-free) |
| `snapshot` / `snapshot-json` | Aggregate snapshot persistence |
| `projection` / `projection-json` | Projection stepper primitives (no runner) |
| `export` / `import` | Backup/restore contract |
| `cbor` | Default CBOR backup box (implies `export` + `import`) |

## MSRV & stability

MSRV **1.95** (pinned stable, no nightly). `mnesis-store` is part of the **1.0 tier** — its documented trait semantics are semver surface. See [STABILITY.md](../../STABILITY.md).

## License

Licensed under your choice of [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE).
