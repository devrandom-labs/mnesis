# mnesis-wake

The **in-process** subscription wake registry (`StreamNotifiers`) for [Mnesis](https://github.com/devrandom-labs/mnesis) store adapters. Implements `mnesis-store`'s `WakeSource` trait over per-stream `tokio::sync::watch` generation counters, so a parked subscription is woken the instant a durable commit lands — with no lost-wakeup window.

Store adapters (`mnesis-fjall`, `mnesis-postgres`, `mnesis-inmemory`) delegate their `WakeSource` impl to this crate for single-process deployments; a distributed adapter implements `WakeSource` over `LISTEN`/`NOTIFY` instead.

## Quickstart

```toml
[dependencies]
mnesis-wake = "0.1"
```

You rarely construct it directly — an adapter owns a `StreamNotifiers` and exposes wake through its `WakeSource` impl.

## MSRV & stability

MSRV **1.95**. Part of the **1.0 tier** (tokio stays an implementation detail — see STABILITY.md). See [STABILITY.md](../../STABILITY.md).

## License

Licensed under your choice of [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE).
