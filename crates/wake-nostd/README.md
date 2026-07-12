# mnesis-wake-nostd

A **`no_std` + alloc** `WakeSource` for [Mnesis](https://github.com/devrandom-labs/mnesis) — on-device live-tail subscriptions under a bare-metal async executor (e.g. embassy). `GlobalWake` is one global eventcount (`AtomicU32` generation + `event_listener::Event`) shared by every registration: every commit wakes every parked subscription, and each false wake costs one empty re-scan (spurious wakes are contract-permitted).

Optional and executor-dependent. The primary device model is **append-and-sync** (subscriptions run server-side under `mnesis-wake`); this crate exists for genuine on-device live-tail. `AtomicU32` (not `U64`) so it works on `thumbv7em` (Cortex-M4), which has no 64-bit atomics.

## Quickstart

```toml
[dependencies]
mnesis-wake-nostd = "0.1"
```

Builds for `thumbv7em-none-eabihf` and `wasm32-unknown-unknown`.

## MSRV & stability

MSRV **1.95** (pinned stable — no `#![feature]` gates). Part of the **1.0 tier**. See [STABILITY.md](../../STABILITY.md).

## License

Licensed under your choice of [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE).
