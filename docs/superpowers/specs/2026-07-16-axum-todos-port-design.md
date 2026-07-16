# Design: port axum's `todos` example onto mnesis (#326)

**Date:** 2026-07-16
**Issue:** #326 — application-author seam validation
**Branch:** `feat/326-axum-todos-example` (verbatim upstream baseline already committed as `2e72922`)

## Purpose

Every existing example was written by the people who designed the API, against
requirements they chose. This port takes its requirements from outside: axum's
own `examples/todos` (four handlers, HTTP contract, status codes — written with
zero knowledge of mnesis), with the `Arc<RwLock<HashMap>>` swapped for
`mnesis` + `mnesis-fjall` and everything else left as-is.

**The frictions are the deliverable.** Each place the port strains is either a
docs gap or a design problem, and both are findings to file as issues — not
things to smooth over by reshaping the example.

## Decisions taken (with rationale)

1. **One stream per todo** (aggregate id = todo id). The natural mnesis
   modelling; makes `GET /todos` an `$all` fold.
2. **Hand-roll the `$all` projection loop in the example.** The `Projection`
   stepper cannot be used: its impl is bounded
   `SS: SnapshotStore<P::State, Version>` and checkpoints on
   `decoded.version` — per-stream only. `subscribe_all` yields the position
   *beside* the envelope as an adapter-defined `AllPosition` (fjall:
   `GlobalSeq`), and `Decoded<T>` has no slot for it. Nothing gets pulled back
   into mnesis; the loop lives in the example and the gap is filed (finding 1).
3. **Read-serving via `tokio::sync::watch`.** The loop folds by value
   (`Projector::apply` consumes state — deliberate, keeps `Clone` off the
   fold), then `send_replace(state.clone())` publishes after each advance.
   Handlers `borrow()`. Honest about the cost: the no-`Clone` fold win is
   given straight back at the publish step, once per event. Ask-the-loop
   (oneshot mailbox) was rejected as runtime shape (Agency's job);
   `Arc<RwLock>` was rejected because by-value `apply` inside a lock forces
   the take-then-put-back poison edge the stepper's design exists to avoid.
4. **PATCH keeps the fat command, decides granular events.** One
   `Handle<UpdateTodo, 1>` at the HTTP seam (the contract is fixed), deciding
   into `TextChanged` / `CompletionChanged` — zero, one, or two events. The
   zero case does not typecheck: `Handle::handle` returns
   `Result<Events<E, N>, Error>` (≥ 1 event guaranteed), while `React::react`
   returns `Result<Option<Events<..>>, _>`. A no-op `PATCH {}` has no
   representation (finding 3).
5. **Accept eventual consistency on `GET`; file it.** A `GET` immediately
   after `POST` may return `[]` — upstream was synchronous and never could.
   Status codes and JSON shapes stay identical; the semantics moved. A
   read-your-writes barrier is impossible anyway: `Repository::save` returns
   `Result<(), _>` — no `Version`, no `GlobalSeq` — so there is no position
   to wait on without dropping to the raw store (finding 4).
6. **Conflict maps to `409 CONFLICT`** — a status upstream never returns,
   because upstream's `todos_update` has a lost-update race (read lock →
   clone → drop → write lock) that silently drops one writer. Under mnesis
   the optimistic version check makes the race unrepresentable, so `409` is
   the minimum honest divergence. No internal retry (rule 5: conflict is
   surfaced, never retried).
7. **Tests drive a bound port, not `oneshot`.** `ServiceExt::oneshot` takes
   the `Router` by value and drives it on the test task — no sockets, no real
   concurrency. axum's own `examples/testing::the_real_deal` pattern instead:
   bind `127.0.0.1:0`, `tokio::spawn(axum::serve(...))`, drive with the
   `hyper_util` legacy client (what axum itself uses; adds nothing to the dep
   tree axum didn't already pull — matters because the workspace currently
   has **zero** HTTP deps and cargo-deny/audit are gates).

## Architecture

`examples/axum-todos`: lib + thin bin (the `projection-tokio` split, so tests
drive the lib). Fjall on a tempdir keyspace, JSON codec. Workspace member.

- **`domain.rs`** — `TodoId(Uuid)` newtype implementing `Id`
  (`AsRef<[u8]>` via the uuid's bytes, `Display` via hyphenated form);
  `Todo` marker via `#[mnesis::aggregate]`; `TodoState`
  (`text`, `completed`, `deleted` flag — delete is an event, per
  `closing_the_books` there is no store-side delete); `TodoEvent`
  (`Created` / `TextChanged` / `CompletionChanged` / `Deleted`) via
  `#[derive(DomainEvent)]`; `Handle<CreateTodo>`, `Handle<UpdateTodo, 1>`,
  `Handle<DeleteTodo>` impls. 404 is a **domain question**: a load of an id
  with no events yields a fresh root at version `None`, not an error;
  "exists" = version present and not deleted.
- **`index.rs`** — `TodosIndex` read model: an **ordered** map — a
  `BTreeMap<GlobalSeq, TodoView>` keyed by the `GlobalSeq` of each todo's
  `Created` event (creation order), with a `HashMap<Uuid, GlobalSeq>` id
  index beside it for point updates. Upstream paginated
  `HashMap::values()`, which is unordered; the projection must choose an
  ordering and that choice is finding 5. A
  `Projector` impl folding `TodoEvent` into it, plus the hand-rolled `$all`
  loop:

  ```text
  subscribe_all(checkpoint) -> Step<(GlobalSeq, PersistedEnvelope)>
    .events()               -> (GlobalSeq, PersistedEnvelope)
    .decoded(codec)         -> (GlobalSeq, Decoded<TodoEvent>)
  ```

  per item: `Projector::apply` fold → checkpoint the `GlobalSeq` tag →
  commit `(state, GlobalSeq)` via fjall's `SnapshotStore<Vec<u8>, GlobalSeq>`
  when the `PersistTrigger` fires → `send_replace(state.clone())`. Flush the
  pending tail on shutdown. This *is* `Projection::advance`/`flush`
  reimplemented for `$all` — kept in one file, honest, not tidied into
  something that looks shipped.
- **`http.rs`** — the four upstream handlers with routes, methods, status
  codes, and request/response JSON shapes **unchanged** (plus `409`, decision
  6). Router state: repository handle + `watch::Receiver<TodosIndex>`.
  `todos_index` serves `offset`/`limit` pagination from the borrowed index.
  The conflict arm names `StoreError<A, C, U>` (three generics) — how much
  that hurts at the handler is an open question answered by writing it.
- **`main.rs`** — upstream's `main` with `Db::default()` replaced by the
  fjall store + spawned projection loop; tracing/middleware kept as-is.

## Testing (rule 7 categories, all via real client → bound port)

Harness: `spawn_app(path) -> (addr, watch::Receiver, handles)` — takes the
keyspace path from the caller (so lifecycle tests can reopen the same
keyspace), builds store/router/loop, binds port 0. Mirrors
`mnesis-store-testing`'s factory-with-guard shape deliberately.

1. **Sequence** — POST → PATCH → GET → DELETE → GET on one todo; exact
   status codes and bodies at each step.
2. **Lifecycle** — POST, tear down app + store, `spawn_app` on the same
   path; assert GET returns the todo **and** the projection resumed from its
   checkpoint (not a full re-fold).
3. **Defensive boundary** — `PATCH {}` (zero-events case), PATCH/DELETE on a
   never-existed id (404 from domain state), malformed JSON, pagination past
   the end.
4. **Linearizability** — two `tokio::spawn`ed tasks + `Barrier`, two real
   connections, both PATCH the same id: exactly one `200`, one `409`, and
   exactly **one** event appended to that stream (read back through the
   repository — the assertion that catches the lost-update bug upstream has).
5. **Projection reflects a write** (required criterion) — POST, await
   `watch::Receiver::changed()`, GET, assert present.

## Findings to file as issues (verified against source, not recollection)

1. `Projection` stepper is `Version`-only; no `$all` projection can use it,
   though fjall's `projections` partition
   (`SnapshotStore<Vec<u8>, GlobalSeq>`) exists for exactly that
   (`crates/store/src/projection.rs:115-121`).
2. `Decoded<T>` carries no position slot; `$all` folds carry the
   `AllPosition` tag in a tuple beside the box.
3. `Handle::handle` cannot decide zero events (`Events<E, N>` ≥ 1);
   `React::react` can (`Option`). A legitimate no-op command has no
   representation (`crates/mnesis/src/aggregate.rs:178` vs
   `crates/mnesis/src/saga.rs:88-91`).
4. `Repository::save` returns `Result<(), _>` — no position — so
   read-your-writes is unbuildable at the repository seam
   (`crates/store/src/repository.rs:109-113`).
5. `GET /todos` pagination forces the projection to choose an ordering;
   upstream's `HashMap::values()` had none (already noted in PROVENANCE.md).
6. `409` is an unavoidable divergence from the upstream contract, which has
   no conflict status because it silently loses updates.

Open questions answered by writing the code (filed only if they strain):
`Uuid`-as-`Id` newtype ergonomics; `StoreError<A, C, U>` spelling at the
handler.

## Constraints

- HTTP contract otherwise unchanged; real `FjallStore` (tempdir), not
  in-memory; no production-crate changes; example may relax lints locally
  (as `closing-the-books` does); upstream attribution retained per
  PROVENANCE.md; builds and tests run under `nix flake check`.
- New deps (axum, tower, tower-http, hyper-util, http-body-util, uuid,
  tracing-subscriber) enter `[workspace.dependencies]` via `cargo add`,
  must clear cargo-deny/audit, and `cargo hakari generate` runs after.
