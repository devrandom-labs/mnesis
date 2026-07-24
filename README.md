# Mnesis

Event sourcing for Rust — no `Box<dyn>`, no runtime downcasting, no hidden allocations.

[![CI](https://github.com/devrandom-labs/mnesis/actions/workflows/checks.yml/badge.svg)](https://github.com/devrandom-labs/mnesis/actions/workflows/checks.yml)
[![CodSpeed](https://img.shields.io/endpoint?url=https://codspeed.io/badge.json)](https://app.codspeed.io/devrandom-labs/mnesis?utm_source=badge)
[![license](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue)](LICENSE-MIT)

```rust
use mnesis::*;

#[derive(Debug, Clone, DomainEvent)]
enum Event {
    Deposited(Deposited),
    Withdrawn(Withdrawn),
}

#[derive(Debug, Clone)]
struct Deposited { amount: u64 }
#[derive(Debug, Clone)]
struct Withdrawn { amount: u64 }

#[derive(Default, Debug, Clone)]
struct Account { balance: u64 }

impl AggregateState for Account {
    type Event = Event;
    fn initial() -> Self { Self::default() }
    fn apply(mut self, event: &Event) -> Self {
        match event {
            Event::Deposited(e) => self.balance += e.amount,
            Event::Withdrawn(e) => self.balance -= e.amount,
        }
        self
    }
}

#[mnesis::aggregate(state = Account, error = BankError, id = AccountId)]
struct BankAccount;

struct Deposit { amount: u64 }

impl Handle<Deposit> for BankAccount {
    fn handle(&self, cmd: Deposit) -> Result<Events<Event>, BankError> {
        Ok(events![Event::Deposited(Deposited { amount: cmd.amount })])
    }
}
```

## Why Mnesis?

**Concrete event enums, not trait objects.** Events are plain Rust enums. `match` is exhaustive — the compiler catches every missing handler at build time. No `Box<dyn Any>`, no runtime downcasting, no message bus plumbing.

**`no_std` down to the persistence layer.** Not just the domain types — the whole core stack builds for bare-metal ARM (`thumbv7em-none-eabihf`) and WASM. The **kernel** is `no_std` and **pure `core`** — no allocator at all: `Events<E, N>` is `ArrayVec`-backed (stack, any `N`), so it never touches the heap (`N = 0`, the default, is simply a single event). **`mnesis-store`** — codecs, envelopes, the catch-up + live-tail subscription loop, backup/restore, snapshots, projections — is `no_std` + `alloc`. And `mnesis-wake-nostd` gives on-device live subscriptions under an [embassy](https://embassy.dev) executor. CI builds every one of these on `thumbv7em` on each push, so a `std` leak fails the gate.

**Zero-copy read path.** One `Decode<E>` trait with an `Output<'a>` GAT covers both owning codecs (serde JSON/bincode/postcard return `E`) and borrowing codecs (rkyv returns `&'a Archived<E>`, bytemuck returns `&'a E`). Event streams are `futures::Stream<Item = Result<PersistedEnvelope, _>>`, so the entire `futures::StreamExt` / `TryStreamExt` combinator surface is free. The on-disk row format aligns every event payload to a 16-byte boundary, so zero-copy decoders (rkyv, flatbuffers, `#[repr(C)]` POD) get sound `&T` references without a copy or a realignment step.

## Crates

| Crate | Description | `no_std` |
|-------|-------------|----------|
| [`mnesis`](crates/mnesis) | Kernel — aggregates, events, versioning, command handling | ✅ pure `core`, no-alloc |
| [`mnesis-macros`](crates/macros) | Derive macros — `DomainEvent`, `#[aggregate]`, `#[transforms]` | emits `no_std` code |
| [`mnesis-store`](crates/store) | Persistence edge — codecs, event streams, upcasters, repositories | ✅ + `alloc` |
| [`mnesis-wake-nostd`](crates/wake-nostd) | On-device wake source for live subscriptions (embassy) | ✅ + `alloc` |
| [`mnesis-fjall`](adapters/fjall) | Embedded LSM-tree event store adapter (fjall) | `std` |

Projection is provided as primitives (`Projector`, `PersistTrigger`, `Subscription`, `SnapshotStore`); mnesis ships no event-loop runner — the loop is the consumer's. See `examples/projection-tokio`.

## Features

- **`no_std` + `alloc` all the way to persistence** — the kernel, `mnesis-store` (subscriptions, backup/restore, snapshots, projections), and `mnesis-wake-nostd` build for bare-metal ARM (`thumbv7em`) and WASM; CI gates it on every push
- Schema evolution via `#[mnesis::transforms]` upcasters
- Optimistic concurrency with version-checked appends
- Allocation-free errors (`ArrayString`-based, no heap on error paths)
- Aggregate snapshots with `AggregateRoot::restore`
- Pluggable codecs via cargo features on `mnesis-store`: `serde` + `json`, `bytemuck` (`#[repr(C)]` POD), `rkyv` (archived zero-copy)
- 16-byte payload alignment as a wire-format invariant — sound zero-copy decode for rkyv, flatbuffers, and `#[repr(C)]` types out of the box
- Owned `bytes::Bytes` envelopes — event streams are plain `futures::Stream`, so every `futures::StreamExt` / `TryStreamExt` combinator works
- Verified with proptest, miri, mutation testing, trybuild, and criterion

## Getting Started

Kernel only (pure domain logic, no persistence):

```toml
[dependencies]
mnesis = { git = "https://github.com/devrandom-labs/mnesis", features = ["derive"] }
```

With persistence (fjall embedded store):

```toml
[dependencies]
mnesis = { git = "https://github.com/devrandom-labs/mnesis", features = ["derive"] }
mnesis-store = { git = "https://github.com/devrandom-labs/mnesis" }
mnesis-fjall = { git = "https://github.com/devrandom-labs/mnesis" }
```

See the [examples](examples/) for complete working code:
- [`inmemory`](examples/inmemory) — pure in-memory event sourcing (bank account domain)
- [`store-inmemory`](examples/store-inmemory) — all store traits with `InMemoryStore`, including codec and upcasting
- [`store-and-kernel`](examples/store-and-kernel) — full lifecycle: create, decide, encode, persist, read, decode, rehydrate

## Status

Mnesis is **experimental** with an unstable API. The kernel is well-tested (proptest, miri, mutation testing, trybuild). The store and fjall adapter are under active development.

The 1.0 promise — semver surface, crate tiers, on-disk format, MSRV, and deprecation policy — is written down in [STABILITY.md](STABILITY.md).

## License

Licensed under your choice of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE).
