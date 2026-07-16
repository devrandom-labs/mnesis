# Provenance

`src/main.rs` in this directory started as a **verbatim copy** of the `todos` example
from the [axum](https://github.com/tokio-rs/axum) repository.

| | |
|---|---|
| Upstream | https://github.com/tokio-rs/axum |
| Path | `examples/todos/src/main.rs` |
| Commit | `b7e37889932edcf521ca54e5ed30245f01180994` (2026-07-16) |
| License | MIT — see `LICENSE.axum-upstream` (Copyright (c) 2019–2025 axum Contributors) |

`Cargo.toml.upstream` is the upstream manifest, kept for reference only; this crate's
own `Cargo.toml` uses mnesis workspace dependencies.

## Why it was vendored verbatim first

Tracked by #326. The point of this example is that **the requirements come from outside
mnesis** — the routes, handlers, HTTP contract, and status codes were written by people
with no knowledge of our API, so porting it cannot be quietly reshaped to flatter the
design we already built.

Committing the upstream file untouched *before* porting keeps that honest: the port's
diff shows exactly what event sourcing changed and nothing else. If a handler had to be
rewritten to fit mnesis, it is visible in the diff rather than lost in a from-scratch
example that was born fitting.

## Deliberate upstream behaviour worth preserving

Two things in the upstream file are load-bearing for #326 and must **not** be silently
"fixed" during the port:

1. **`todos_update` has a lost-update race.** It takes a read lock, clones, drops the
   lock, mutates the clone, then takes a write lock and inserts. Two concurrent `PATCH`
   requests to the same id interleave and one update is silently lost. Under mnesis this
   race is unrepresentable — the optimistic version check turns the second writer into a
   conflict — so the port must *surface* it at the HTTP boundary, not paper over it.

2. **`todos_index` paginates over `HashMap::values()`**, which is unordered, so
   `offset`/`limit` are not stable across requests. A projection must choose an ordering.
   That choice is a finding, not an implementation detail — record it rather than assume it.

Per #326, every strain point found during the port is filed as its own issue.
