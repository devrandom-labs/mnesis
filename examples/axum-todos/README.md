# axum-todos — the application-author seam, validated (#326)

A port of axum's own [`examples/todos`](https://github.com/tokio-rs/axum/tree/main/examples/todos)
onto mnesis + mnesis-fjall. The routes, methods, status codes, and JSON shapes are
upstream's, unchanged; the `Arc<RwLock<HashMap<Uuid, Todo>>>` behind them is replaced by
an event-sourced todo aggregate (one stream per todo) on a real on-disk `FjallStore`,
with `GET /todos` served from a consumer-owned `$all` projection. The upstream file was
vendored verbatim before the port so the diff is honest — see `PROVENANCE.md`.

## What it proves

Every prior example in this repo was written by the API's own designers, so it could
only ever confirm the design it was born fitting. This port takes its requirements from
people with zero mnesis knowledge and refuses to reshape them: every place the port
strains is therefore a genuine finding about the application-author seam, and the
frictions found are the deliverable — each is filed as its own issue (table below).

## Run

```sh
cargo run -p mnesis-example-axum-todos
```

Then, with [xh](https://github.com/ducaale/xh) (or curl):

```sh
xh POST :3000/todos text="buy milk"
xh GET :3000/todos
xh PATCH :3000/todos/{id} completed:=true
xh DELETE :3000/todos/{id}
```

Tests:

```sh
cargo nextest run -p mnesis-example-axum-todos
```

The integration tests are not router unit tests — they drive a real `hyper` client
against a real bound port against a real fjall keyspace (axum's own
`testing::the_real_deal` pattern), covering the four mandatory categories: the full
lifecycle sequence, keyspace reopen + projection resume, boundary inputs, and
barrier-aligned concurrent PATCHes.

## Architecture

- **`domain.rs`** — the `Todo` aggregate, one event stream per todo. The fat upstream
  `PATCH` (both fields optional) decomposes into granular events (`TextChanged`,
  `CompletionChanged`); every event variant carries the todo `id` by hand, because a
  `$all` item has no stream id and `Handle::handle` is a pure `(state, command)`
  function with no identity access (finding 7).
- **`index.rs`** — the creation-ordered read model behind `GET /todos`, plus the
  hand-rolled `$all` loop: `subscribe_all → .events() → .decoded() → fold →
  commit(state, position) → publish to watch`. Hand-rolled because the shipped
  `Projection` stepper and `PersistTrigger` are `Version`-typed (findings 1–2).
- **`http.rs`** — the four upstream handlers, verbatim in shape. Writes go through
  `repo.execute(...)` on a per-request repository facade; reads borrow the projection's
  `watch::Receiver`. Two statuses are added: `409` on a conflicting overlapping write
  and `503` when the projection loop has died (the dropped watch sender is the signal).
- **`lib.rs` / `main.rs`** — `spawn_app` opens (or reopens) the keyspace, hydrates the
  index from its persisted `(state, position)` checkpoint, seeds the watch channel, and
  spawns the projection loop and the server; the binary binds upstream's
  `127.0.0.1:3000` over a tempdir keyspace.

## Deliberate divergences from upstream

- **`409 Conflict`** — upstream's `todos_update` lost-update race (read-lock → clone →
  drop → write-lock) is unrepresentable under the optimistic version check, so the
  second overlapping writer surfaces instead of being silently lost.
- **`503 Service Unavailable`** — reads come from a projection, and a dead projection
  loop would otherwise serve frozen reads with 200s forever.
- **Eventually consistent `GET`** — a `GET` racing its own `POST` may not see it yet;
  there is no position for the handler to await (finding 4), so the port is honest
  about it rather than pretending to read-your-writes.
- **Deterministic pagination order** — upstream paginated `HashMap::values()`, which is
  unordered; the projection had to choose, and creation order (the `$all` fold order of
  `Created` events) makes `offset`/`limit` stable across requests — which upstream's
  own contract never was.

## Findings (each filed as its own issue)

| # | Finding | Issue |
|---|---------|-------|
| 1 | The `Projection` stepper cannot drive an `$all` projection — its `SnapshotStore` bound and checkpoint are `Version`-typed (per-stream); the example hand-rolls the loop in `index.rs` | #TBD |
| 2 | `PersistTrigger` is `Version`-typed and `Decoded<T>` has no position slot — the `$all` position rides in a tuple beside the item; no shipped trigger can accept it, so the loop commits every event | #TBD |
| 3 | `Handle` cannot decide zero events (`Events<E, N>` guarantees ≥ 1; `React` returns `Option`) — the legitimate no-op `PATCH {}` is answered in the handler from loaded state, without entering the domain | #TBD |
| 4 | `Repository::save`/`execute` return no position — read-your-writes is unbuildable at the repository seam; `GET` is eventually consistent and the tests await the watch channel instead | #TBD |
| 5 | `GET /todos` pagination forced an ordering decision upstream never made; creation order was chosen, making pagination deterministic | #TBD |
| 6 | Two statuses upstream never returns are unavoidable: `409` (the lost-update race surfaces instead of losing the write) and `503` (a dead projection loop must not serve frozen 200s) | #TBD |
| 7 | `$all` items carry no stream id and `Handle::handle` has no identity access — the todo id is threaded by hand: URL path → command field → every event variant's payload | #TBD |

### What did NOT strain

The hand-rolled `$all` loop (`subscribe_all → .events() → .decoded() → fold → commit →
publish`) compiled verbatim first try — no turbofish, no borrow gymnastics, `?` unified
five error types into one boxed error; the composition surface is solid, and the gaps
above (findings 1–2) are structural, not ergonomic. Conflict handling at the handler
was trivial: `CommandRepository::execute` + `ExecuteError::is_conflict()` — the
anticipated `StoreError<A, C, U>` three-generic pain never materialized. One real DX
paper-cut: `Repository` must be imported separately for `.load` even with
`CommandRepository` in scope (supertrait methods don't come along).

---

Upstream attribution: this example started as a verbatim copy of axum's
`examples/todos` — see `PROVENANCE.md` for the exact commit and
`LICENSE.axum-upstream` for the upstream MIT license.
