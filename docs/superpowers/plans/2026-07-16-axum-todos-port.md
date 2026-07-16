# Axum Todos Port Implementation Plan (#326)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Port axum's `examples/todos` onto mnesis + mnesis-fjall with the HTTP contract unchanged, recording every API friction as a finding (spec: `docs/superpowers/specs/2026-07-16-axum-todos-port-design.md`).

**Architecture:** One stream per todo (`TodoId(Uuid)`), a hand-rolled `$all` projection loop (the `Projection` stepper and `PersistTrigger` are `Version`-typed — finding) publishing a creation-ordered `TodosIndex` into `tokio::sync::watch`, and the four upstream handlers over `Store<FjallStore>` + `watch::Receiver`. Tests drive a real bound port with the `hyper_util` legacy client (axum's own `the_real_deal` pattern).

**Tech Stack:** axum, tower, tower-http, hyper-util (dev), tokio, mnesis (`derive`, `testing` dev), mnesis-store (`json`, `projection`, `subscription`), mnesis-fjall (`projection`), uuid, tempfile.

**Verified API facts this plan relies on (do not re-derive):**
- `PersistedEnvelope` carries **no stream id** (only `global_seq`); the `$all` item is `(GlobalSeq, PersistedEnvelope)`. A multi-stream projection must read the aggregate id out of the **event payload** — every `TodoEvent` variant carries `id: Uuid`.
- `Handle::handle` returns `Result<Events<E, N>, Error>` — **≥ 1 event, no zero-event decide** (`crates/mnesis/src/aggregate.rs:178`). The no-op `PATCH {}` is answered in the handler from loaded state.
- `Repository::save`/`execute` return no position. `CommandRepository::execute(&mut root, cmd)` (`crates/store/src/execute.rs`) is the sanctioned decide+save; `ExecuteError::is_conflict()` detects the optimistic conflict.
- `Projector::apply(&self, state, &event)` (`crates/store/src/projection.rs:49`); `Projection` stepper + `PersistTrigger` are `Version`-typed → unusable for `$all`; loop commits every event via fjall's `SnapshotStore<Vec<u8>, GlobalSeq>` (feature `projection`) bridged by `CodecSnapshotStore` (`mnesis_store::state`, **ungated**).
- `GlobalSeq` lives in `mnesis_fjall` (`adapters/fjall/src/global_seq.rs`), not mnesis-store.
- `Id` has a **blanket impl** — a newtype needs only `Clone+Send+Sync+Debug+Hash+Eq+Display+AsRef<[u8]>+'static`.
- `#[derive(DomainEvent)]` supports struct variants. `events![a, b]` builds multi-event `Events`. `AggregateFixture::with_id(id).given([..]).when(cmd).then_expect_events([..])` / `.then_expect_error(err)`.
- `Subscription::new(&store).subscribe_all(from)?` → `Step<(GlobalSeq, PersistedEnvelope)>` stream; `.events()` drops phase; `.decoded(codec)` → `(GlobalSeq, Decoded<TodoEvent>)`. Stream is `!Unpin` → `tokio::pin!`.
- `AggregateRoot`: `state()`, `version() -> Option<Version>`; macro generates `Todo::new(id) -> AggregateRoot<Todo>`.

**Conventions:** `git add` new files before the commit (the pre-commit hook's `nix flake check` ignores untracked files). Run `nix develop -c cargo fmt --all` after substantial edits, before staging. Never pre-run `nix flake check` by hand — the hook runs it. Dev loop: `nix develop -c cargo nextest run -p mnesis-example-axum-todos`.

**Deviation log:** record any divergence from this plan at the bottom of this file (what, why, impact).

---

### Task 1: Manifest, workspace membership, dependency scaffolding

**Files:**
- Modify: `Cargo.toml` (workspace members + `[workspace.dependencies]`)
- Create: `examples/axum-todos/Cargo.toml`
- Create: `examples/axum-todos/src/lib.rs` (skeleton)
- Create: `examples/axum-todos/src/main.rs` (placeholder)
- Delete: `examples/axum-todos/src/main.rs` upstream content is **moved**, not lost — it stays visible in `Cargo.toml.upstream`/git history; PROVENANCE.md already records the baseline commit `2e72922`.

- [ ] **Step 1: Add the workspace member**

In root `Cargo.toml`, add to `members` (alphabetical):

```toml
    "examples/axum-todos",
```

- [ ] **Step 2: Create the example manifest with placeholder deps via cargo add**

Write `examples/axum-todos/Cargo.toml`:

```toml
[package]
name = "mnesis-example-axum-todos"
version = "0.0.0"
edition.workspace = true
publish = false

[dependencies]
futures = { workspace = true, features = ["std", "async-await"] }
mnesis = { path = "../../crates/mnesis", features = ["derive"] }
mnesis-fjall = { path = "../../adapters/fjall", features = ["projection"] }
mnesis-store = { path = "../../crates/store", features = [
    "json",
    "projection",
    "subscription",
] }
serde = { workspace = true }
serde_json = { workspace = true }
tempfile = { workspace = true }
thiserror = { workspace = true }
tokio = { workspace = true }
tracing = { workspace = true }
workspace-hack = { version = "0.1", path = "../../crates/workspace-hack" }

[dev-dependencies]
mnesis = { path = "../../crates/mnesis", features = ["derive", "testing"] }

[lints]
workspace = true
```

- [ ] **Step 3: Resolve the NEW dependencies with cargo add (never hand-write versions)**

```bash
cd examples/axum-todos
nix develop -c cargo add axum
nix develop -c cargo add tower --features util,timeout
nix develop -c cargo add tower-http --features trace
nix develop -c cargo add tracing-subscriber --features env-filter
nix develop -c cargo add uuid --features serde,v4
nix develop -c cargo add --dev hyper
nix develop -c cargo add --dev hyper-util --features client,http1,client-legacy
nix develop -c cargo add --dev http-body-util
cd ../..
```

- [ ] **Step 4: Hoist the resolved versions to `[workspace.dependencies]`**

Move each version cargo-add wrote into root `Cargo.toml` `[workspace.dependencies]` (alphabetical), keeping the exact resolved version strings, e.g. (versions from Step 3's output, NOT these placeholders):

```toml
axum = "<resolved>"
http-body-util = "<resolved>"
hyper = "<resolved>"
hyper-util = "<resolved>"
tower = { version = "<resolved>", default-features = false }
tower-http = "<resolved>"
tracing = "<resolved>"
tracing-subscriber = "<resolved>"
uuid = "<resolved>"
```

Then rewrite the example's entries to workspace form:

```toml
axum = { workspace = true }
tower = { workspace = true, features = ["util", "timeout"] }
tower-http = { workspace = true, features = ["trace"] }
tracing-subscriber = { workspace = true, features = ["env-filter"] }
uuid = { workspace = true, features = ["serde", "v4"] }
```

and in `[dev-dependencies]`:

```toml
http-body-util = { workspace = true }
hyper = { workspace = true }
hyper-util = { workspace = true, features = ["client", "http1", "client-legacy"] }
```

Note: `tracing` may already resolve transitively; it still needs an explicit workspace entry since the example names it.

- [ ] **Step 5: Skeleton lib.rs and main.rs so the crate compiles**

`examples/axum-todos/src/lib.rs`:

```rust
//! Port of axum's `examples/todos` onto mnesis + mnesis-fjall (#326).
//!
//! (Narrative docs land in Task 8; this is the compile skeleton.)

// Example code relaxes strict lints locally (production crates do NOT) —
// same posture as `examples/fjall-end-to-end`. `unwrap_used` is allowed
// because upstream handler/main code kept verbatim uses `unwrap()` and the
// diff against the upstream baseline is the point (see PROVENANCE.md).
#![allow(
    clippy::unwrap_used,
    reason = "upstream axum example code is kept verbatim; the port's diff is the deliverable"
)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    reason = "example: error/panic conditions are obvious from the narrative"
)]
#![allow(
    clippy::expect_used,
    reason = "example: expect documents an assumption at startup/teardown"
)]

pub mod domain;
pub mod http;
pub mod index;
```

Create empty `examples/axum-todos/src/domain.rs`, `src/http.rs`, `src/index.rs` each containing only a `//! placeholder` doc line for now (filled by Tasks 2–4).

Replace `examples/axum-todos/src/main.rs` with a placeholder binary (the real one lands in Task 5):

```rust
fn main() {
    println!("port in progress — see lib.rs");
}
```

- [ ] **Step 6: Regenerate hakari, format, verify the workspace builds**

```bash
nix develop -c cargo hakari generate
nix develop -c cargo fmt --all
nix develop -c cargo check -p mnesis-example-axum-todos
```

Expected: clean check (empty modules).

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "chore(examples): scaffold axum-todos port crate (#326)"
```

---

### Task 2: Domain — `TodoId`, events, state, commands, `Handle` impls (TDD via AggregateFixture)

**Files:**
- Modify: `examples/axum-todos/src/domain.rs`

- [ ] **Step 1: Write the failing fixture tests first**

`examples/axum-todos/src/domain.rs` — start with the test module (the types it names don't exist yet):

```rust
//! The todo aggregate: one event stream per todo.

#[cfg(test)]
mod tests {
    use super::*;
    use mnesis::testing::AggregateFixture;
    use uuid::Uuid;

    fn uid() -> Uuid {
        Uuid::new_v4()
    }

    fn fixture(id: Uuid) -> AggregateFixture<Todo> {
        AggregateFixture::with_id(TodoId(id))
    }

    #[test]
    fn create_decides_created() {
        let id = uid();
        let _ = fixture(id)
            .given([])
            .when(Create { id, text: "buy milk".to_owned() })
            .then_expect_events([TodoEvent::Created { id, text: "buy milk".to_owned() }]);
    }

    #[test]
    fn create_on_existing_todo_is_rejected() {
        let id = uid();
        let _ = fixture(id)
            .given([TodoEvent::Created { id, text: "a".to_owned() }])
            .when(Create { id, text: "b".to_owned() })
            .then_expect_error(TodoError::AlreadyExists);
    }

    #[test]
    fn update_both_fields_decides_two_events() {
        let id = uid();
        let _ = fixture(id)
            .given([TodoEvent::Created { id, text: "a".to_owned() }])
            .when(Update { id, text: Some("b".to_owned()), completed: Some(true) })
            .then_expect_events([
                TodoEvent::TextChanged { id, text: "b".to_owned() },
                TodoEvent::CompletionChanged { id, completed: true },
            ]);
    }

    #[test]
    fn update_single_field_decides_one_event() {
        let id = uid();
        let _ = fixture(id)
            .given([TodoEvent::Created { id, text: "a".to_owned() }])
            .when(Update { id, text: None, completed: Some(true) })
            .then_expect_events([TodoEvent::CompletionChanged { id, completed: true }]);
    }

    #[test]
    fn update_with_no_fields_is_rejected() {
        // `Events<E, N>` guarantees ≥ 1 event, so "decide nothing" has no
        // representation in `Handle` (finding #326-3): the all-None command
        // must be an error here; the HTTP handler answers the no-op PATCH
        // from state without entering the domain.
        let id = uid();
        let _ = fixture(id)
            .given([TodoEvent::Created { id, text: "a".to_owned() }])
            .when(Update { id, text: None, completed: None })
            .then_expect_error(TodoError::NothingToUpdate);
    }

    #[test]
    fn update_missing_todo_is_rejected() {
        let id = uid();
        let _ = fixture(id)
            .given([])
            .when(Update { id, text: Some("b".to_owned()), completed: None })
            .then_expect_error(TodoError::NotFound);
    }

    #[test]
    fn update_deleted_todo_is_rejected() {
        let id = uid();
        let _ = fixture(id)
            .given([
                TodoEvent::Created { id, text: "a".to_owned() },
                TodoEvent::Deleted { id },
            ])
            .when(Update { id, text: Some("b".to_owned()), completed: None })
            .then_expect_error(TodoError::NotFound);
    }

    #[test]
    fn delete_decides_deleted() {
        let id = uid();
        let _ = fixture(id)
            .given([TodoEvent::Created { id, text: "a".to_owned() }])
            .when(Delete { id })
            .then_expect_events([TodoEvent::Deleted { id }]);
    }

    #[test]
    fn delete_twice_is_rejected() {
        let id = uid();
        let _ = fixture(id)
            .given([
                TodoEvent::Created { id, text: "a".to_owned() },
                TodoEvent::Deleted { id },
            ])
            .when(Delete { id })
            .then_expect_error(TodoError::NotFound);
    }

    #[test]
    fn state_folds_full_history() {
        let id = uid();
        let _ = fixture(id)
            .given([
                TodoEvent::Created { id, text: "a".to_owned() },
                TodoEvent::TextChanged { id, text: "b".to_owned() },
                TodoEvent::CompletionChanged { id, completed: true },
            ])
            .then_expect_state(&TodoState {
                created: true,
                deleted: false,
                text: "b".to_owned(),
                completed: true,
            });
    }
}
```

- [ ] **Step 2: Run to verify failure**

```bash
nix develop -c cargo nextest run -p mnesis-example-axum-todos
```

Expected: compile FAILURE (`Todo`, `TodoEvent`, … not found).

- [ ] **Step 3: Implement the domain above the test module**

```rust
use std::fmt;

use mnesis::{AggregateState, DomainEvent, Events, Handle, events};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Aggregate id: a `Uuid` newtype. `mnesis::Id` is blanket-implemented, so
/// only `Display` + `AsRef<[u8]>` need supplying (finding: the newtype is
/// mandatory — `Uuid` itself satisfies the bounds but the id type appears in
/// handler signatures, so the app owns a name for it).
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct TodoId(pub Uuid);

impl fmt::Display for TodoId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl AsRef<[u8]> for TodoId {
    fn as_ref(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

/// Every variant carries `id` because the `$all` stream does not: a
/// `PersistedEnvelope` has no stream id, so a multi-stream projection can
/// only learn which todo an event belongs to from the payload itself
/// (finding #326-7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, DomainEvent)]
pub enum TodoEvent {
    Created { id: Uuid, text: String },
    TextChanged { id: Uuid, text: String },
    CompletionChanged { id: Uuid, completed: bool },
    Deleted { id: Uuid },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TodoState {
    pub created: bool,
    pub deleted: bool,
    pub text: String,
    pub completed: bool,
}

impl AggregateState for TodoState {
    type Event = TodoEvent;

    fn initial() -> Self {
        Self::default()
    }

    fn apply(mut self, event: &TodoEvent) -> Self {
        match event {
            TodoEvent::Created { text, .. } => {
                self.created = true;
                self.text = text.clone();
            }
            TodoEvent::TextChanged { text, .. } => self.text = text.clone(),
            TodoEvent::CompletionChanged { completed, .. } => self.completed = *completed,
            TodoEvent::Deleted { .. } => self.deleted = true,
        }
        self
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum TodoError {
    #[error("todo already exists")]
    AlreadyExists,
    #[error("todo does not exist")]
    NotFound,
    #[error("nothing to update")]
    NothingToUpdate,
}

#[mnesis::aggregate(state = TodoState, error = TodoError, id = TodoId)]
pub struct Todo;

/// Commands carry the id because `Handle::handle` is a pure function of
/// `(state, command)` with no identity access — the only route from the URL
/// path to the event payload is command → event, by hand (finding #326-7).
pub struct Create {
    pub id: Uuid,
    pub text: String,
}

pub struct Update {
    pub id: Uuid,
    pub text: Option<String>,
    pub completed: Option<bool>,
}

pub struct Delete {
    pub id: Uuid,
}

impl Handle<Create> for Todo {
    fn handle(state: &TodoState, cmd: Create) -> Result<Events<TodoEvent>, TodoError> {
        if state.created {
            return Err(TodoError::AlreadyExists);
        }
        Ok(events![TodoEvent::Created { id: cmd.id, text: cmd.text }])
    }
}

impl Handle<Update, 1> for Todo {
    fn handle(state: &TodoState, cmd: Update) -> Result<Events<TodoEvent, 1>, TodoError> {
        if !state.created || state.deleted {
            return Err(TodoError::NotFound);
        }
        match (cmd.text, cmd.completed) {
            (Some(text), Some(completed)) => Ok(events![
                TodoEvent::TextChanged { id: cmd.id, text },
                TodoEvent::CompletionChanged { id: cmd.id, completed },
            ]),
            (Some(text), None) => Ok(events![TodoEvent::TextChanged { id: cmd.id, text }]),
            (None, Some(completed)) => {
                Ok(events![TodoEvent::CompletionChanged { id: cmd.id, completed }])
            }
            (None, None) => Err(TodoError::NothingToUpdate),
        }
    }
}

impl Handle<Delete> for Todo {
    fn handle(state: &TodoState, cmd: Delete) -> Result<Events<TodoEvent>, TodoError> {
        if !state.created || state.deleted {
            return Err(TodoError::NotFound);
        }
        Ok(events![TodoEvent::Deleted { id: cmd.id }])
    }
}
```

Note: `then_expect_state` may require the fixture's `Given` state assertion — if the exact method name differs, check `crates/mnesis/src/testing.rs` (`then_expect_state` exists on both `Given` and `Acted` per CLAUDE.md).

- [ ] **Step 4: Run to verify pass**

```bash
nix develop -c cargo nextest run -p mnesis-example-axum-todos
```

Expected: all 10 domain tests PASS.

- [ ] **Step 5: Format + commit**

```bash
nix develop -c cargo fmt --all
git add -A
git commit -m "feat(examples): axum-todos domain — todo aggregate with granular events (#326)"
```

---

### Task 3: Read model — `TodosIndex`, `TodosProjector` (TDD)

**Files:**
- Modify: `examples/axum-todos/src/index.rs`

- [ ] **Step 1: Write the failing fold tests**

Start `examples/axum-todos/src/index.rs` with the test module:

```rust
//! The read model behind `GET /todos`.

#[cfg(test)]
mod tests {
    use super::*;
    use mnesis_store::Projector as _;
    use uuid::Uuid;

    fn created(id: Uuid, text: &str) -> TodoEvent {
        TodoEvent::Created { id, text: text.to_owned() }
    }

    fn fold(events: &[TodoEvent]) -> TodosIndex {
        events.iter().try_fold(TodosProjector.initial(), |state, event| {
            TodosProjector.apply(state, event)
        })
        .expect("fold succeeds")
    }

    #[test]
    fn created_todos_appear_in_creation_order() {
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let index = fold(&[created(a, "first"), created(b, "second")]);
        let page = index.page(0, usize::MAX);
        assert_eq!(page.len(), 2);
        assert_eq!((page[0].id, page[0].text.as_str()), (a, "first"));
        assert_eq!((page[1].id, page[1].text.as_str()), (b, "second"));
        assert!(!page[0].completed);
    }

    #[test]
    fn updates_mutate_the_right_todo() {
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let index = fold(&[
            created(a, "first"),
            created(b, "second"),
            TodoEvent::TextChanged { id: b, text: "renamed".to_owned() },
            TodoEvent::CompletionChanged { id: a, completed: true },
        ]);
        let page = index.page(0, usize::MAX);
        assert_eq!((page[0].text.as_str(), page[0].completed), ("first", true));
        assert_eq!((page[1].text.as_str(), page[1].completed), ("renamed", false));
    }

    #[test]
    fn deleted_todo_leaves_the_index() {
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let index = fold(&[
            created(a, "first"),
            created(b, "second"),
            TodoEvent::Deleted { id: a },
        ]);
        let page = index.page(0, usize::MAX);
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].id, b);
    }

    #[test]
    fn event_for_unknown_todo_is_a_projection_error() {
        // Defensive boundary: the $all fold sees every event; one addressing
        // a todo the index never saw means a corrupted or reordered feed.
        let id = Uuid::new_v4();
        let result = TodosProjector.apply(
            TodosProjector.initial(),
            &TodoEvent::TextChanged { id, text: "ghost".to_owned() },
        );
        assert_eq!(result.unwrap_err(), IndexError::UnknownTodo { id });
    }

    #[test]
    fn pagination_clamps_past_the_end() {
        let index = fold(&[created(Uuid::new_v4(), "only")]);
        assert!(index.page(5, usize::MAX).is_empty());
        assert_eq!(index.page(0, 0).len(), 0);
    }
}
```

- [ ] **Step 2: Run to verify failure**

```bash
nix develop -c cargo nextest run -p mnesis-example-axum-todos
```

Expected: compile FAILURE (`TodosIndex` etc. not found).

- [ ] **Step 3: Implement above the tests**

```rust
use mnesis_store::Projector;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::TodoEvent;

/// The JSON shape upstream serves from `GET /todos` — `{ id, text, completed }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoView {
    pub id: Uuid,
    pub text: String,
    pub completed: bool,
}

/// Creation-ordered todos. Vec order **is** `$all` fold order of `Created`
/// events, so `offset`/`limit` pagination is stable across requests —
/// upstream paginated `HashMap::values()`, which is unordered (finding
/// #326-5: a projection must choose an ordering; upstream never did).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodosIndex {
    todos: Vec<TodoView>,
}

impl TodosIndex {
    /// One page of todos — the upstream `skip(offset).take(limit)` contract.
    #[must_use]
    pub fn page(&self, offset: usize, limit: usize) -> Vec<TodoView> {
        self.todos.iter().skip(offset).take(limit).cloned().collect()
    }

    #[must_use]
    pub fn contains(&self, id: Uuid) -> bool {
        self.todos.iter().any(|t| t.id == id)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.todos.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.todos.is_empty()
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum IndexError {
    /// An update/delete for an id the index never saw created — a corrupt or
    /// reordered feed. Recovery policy is the consumer's (mnesis rule); this
    /// loop surfaces it and stops.
    #[error("event for unknown todo {id}")]
    UnknownTodo { id: Uuid },
}

/// Pure fold of `TodoEvent`s into the index — the `Projector` primitive is
/// reusable even though the `Projection` stepper is not (finding #326-1).
#[derive(Debug, Clone, Copy)]
pub struct TodosProjector;

impl Projector for TodosProjector {
    type Event = TodoEvent;
    type State = TodosIndex;
    type Error = IndexError;

    fn initial(&self) -> TodosIndex {
        TodosIndex::default()
    }

    fn apply(&self, mut state: TodosIndex, event: &TodoEvent) -> Result<TodosIndex, IndexError> {
        match event {
            TodoEvent::Created { id, text } => {
                state.todos.push(TodoView { id: *id, text: text.clone(), completed: false });
            }
            TodoEvent::TextChanged { id, text } => {
                let todo = state
                    .todos
                    .iter_mut()
                    .find(|t| t.id == *id)
                    .ok_or(IndexError::UnknownTodo { id: *id })?;
                todo.text = text.clone();
            }
            TodoEvent::CompletionChanged { id, completed } => {
                let todo = state
                    .todos
                    .iter_mut()
                    .find(|t| t.id == *id)
                    .ok_or(IndexError::UnknownTodo { id: *id })?;
                todo.completed = *completed;
            }
            TodoEvent::Deleted { id } => {
                state.todos.retain(|t| t.id != *id);
            }
        }
        Ok(state)
    }
}
```

Check where `Projector` is re-exported: `mnesis_store::Projector` (root re-export, feature `projection`) or `mnesis_store::projection::Projector` — use whichever compiles (root re-export exists per CLAUDE.md).

- [ ] **Step 4: Run to verify pass**

```bash
nix develop -c cargo nextest run -p mnesis-example-axum-todos
```

Expected: 15 tests PASS (10 domain + 5 index).

- [ ] **Step 5: Format + commit**

```bash
nix develop -c cargo fmt --all
git add -A
git commit -m "feat(examples): axum-todos creation-ordered read model (#326)"
```

---

### Task 4: The hand-rolled `$all` projection loop — `hydrate` + `run` (TDD against a real FjallStore)

**Files:**
- Modify: `examples/axum-todos/src/index.rs` (append)

- [ ] **Step 1: Write the failing loop test (append to the test module in index.rs)**

```rust
    // ── $all loop against a real FjallStore ─────────────────────────────────

    use mnesis_fjall::FjallStore;
    use mnesis_store::store::RawEventStore as _;
    use mnesis_store::{CommandRepository as _, Repository as _};
    use tokio::sync::watch;

    use crate::domain::{Create, Todo, TodoId};

    #[tokio::test]
    async fn loop_folds_writes_and_reopen_resumes_from_checkpoint() {
        let dir = tempfile::tempdir().expect("tempdir");

        // Fresh store: hydrate finds nothing.
        {
            let store = FjallStore::builder(dir.path()).open().expect("open").into_store();
            let (seed, checkpoint) = hydrate(&store).await.expect("hydrate");
            assert_eq!(seed, TodosIndex::default());
            assert!(checkpoint.is_none());

            // Write one todo through the repository, then drive the loop and
            // watch it publish the folded index.
            let id = Uuid::new_v4();
            let repo = store.repository::<Todo>().json().build();
            let mut root = Todo::new(TodoId(id));
            repo.execute(&mut root, Create { id, text: "persisted".to_owned() })
                .await
                .expect("create");

            let (tx, mut rx) = watch::channel(seed.clone());
            let loop_store = store.clone();
            let task = tokio::spawn(async move {
                let _ = run(loop_store, seed, checkpoint, tx).await;
            });
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    if rx.borrow_and_update().contains(id) {
                        break;
                    }
                    rx.changed().await.expect("loop alive");
                }
            })
            .await
            .expect("loop folds the write");
            task.abort();
            let _ = task.await;
        }
        // All store handles dropped: the keyspace closes and the same path
        // reopens. hydrate must find the committed (state, position) pair —
        // the projection resumes, it does not re-fold (lifecycle category).
        {
            let store = FjallStore::builder(dir.path()).open().expect("reopen").into_store();
            let (seed, checkpoint) = hydrate(&store).await.expect("rehydrate");
            assert_eq!(seed.len(), 1, "state came back from the snapshot");
            assert!(checkpoint.is_some(), "checkpoint came back with it");
        }
    }
```

- [ ] **Step 2: Run to verify failure**

Expected: compile FAILURE (`hydrate`, `run` not found).

- [ ] **Step 3: Implement `hydrate` and `run` in index.rs**

Add imports at the top of the file (top-of-file imports only — no inline `use`):

```rust
use std::num::NonZeroU32;
use std::fmt;

use futures::StreamExt;
use mnesis_fjall::{FjallStore, GlobalSeq};
use mnesis_store::state::{CodecSnapshotStore, Hydrated, SnapshotStore};
use mnesis_store::store::Store;
use mnesis_store::{DecodedStreamExt, JsonCodec, StepStreamExt, Subscription};
use tokio::sync::watch;
```

Then:

```rust
/// The projection's own id in fjall's `projections` partition.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct IndexId;

impl fmt::Display for IndexId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("todos-index")
    }
}

impl AsRef<[u8]> for IndexId {
    fn as_ref(&self) -> &[u8] {
        b"todos-index"
    }
}

/// Schema version of the folded state — bump to force a rebuild on deploy.
pub const INDEX_SCHEMA: NonZeroU32 = NonZeroU32::MIN;

/// One boxed error domain for the loop (subscription register, read, decode,
/// fold, and snapshot commit all differ in type; the loop only logs and dies).
pub type BoxErr = Box<dyn std::error::Error + Send + Sync>;

/// Load the persisted `(state, checkpoint)` pair, if any. `Stale` (schema
/// bump) folds from scratch, exactly like `Absent` — for an aggregate-shaped
/// consumer the two collapse; a host that must anticipate a costly rebuild
/// would branch here.
pub async fn hydrate(store: &Store<FjallStore>) -> Result<(TodosIndex, Option<GlobalSeq>), BoxErr> {
    let snapshots = CodecSnapshotStore::new(store.raw(), JsonCodec::default());
    Ok(match snapshots.hydrate(&IndexId, INDEX_SCHEMA).await? {
        Hydrated::Found { position, state } => (state, Some(position)),
        Hydrated::Absent | Hydrated::Stale { .. } => (TodosIndex::default(), None),
    })
}

/// Fold the `$all` stream into the index forever, committing
/// `(state, position)` atomically per event and publishing each new state.
///
/// This is `Projection::advance`/`flush` reimplemented for `$all`, because
/// the stepper and `PersistTrigger` are `Version`-typed (findings #326-1/-2):
/// the position rides *beside* the envelope as fjall's `GlobalSeq`, `Decoded`
/// has no slot for it, and no shipped trigger can accept it. The loop commits
/// every event (so there is no pending tail and no flush); `send_replace`
/// then pays one clone per event — the price of the deliberately no-`Clone`
/// fold, at the seam where another task must see the state (finding: the
/// read-serving cost is real but contained here, not in the fold).
pub async fn run(
    store: Store<FjallStore>,
    seed: TodosIndex,
    checkpoint: Option<GlobalSeq>,
    tx: watch::Sender<TodosIndex>,
) -> Result<(), BoxErr> {
    let snapshots = CodecSnapshotStore::new(store.raw(), JsonCodec::default());
    let stream = Subscription::new(&store)
        .subscribe_all(checkpoint)?
        .events()
        .decoded(JsonCodec::default());
    tokio::pin!(stream);

    let mut state = seed;
    while let Some(item) = stream.next().await {
        let (position, decoded) = item?;
        state = TodosProjector.apply(state, &decoded.event)?;
        snapshots
            .commit(&IndexId, INDEX_SCHEMA, position, &state)
            .await?;
        tx.send_replace(state.clone());
    }
    Ok(())
}
```

Type-flow note for the implementer: `subscribe_all(checkpoint)` yields `Step<(GlobalSeq, PersistedEnvelope)>`; `.events()` unwraps to `(GlobalSeq, PersistedEnvelope)`; `.decoded(JsonCodec::default())` (the `DecodedStreamExt` method, `RawItem` impl for `(P, PersistedEnvelope)`) yields `Result<(GlobalSeq, Decoded<TodoEvent>), _>` — the event type is inferred from `TodosProjector.apply`. If inference needs help, annotate the closure or use a turbofish on `.decoded`.

- [ ] **Step 4: Run to verify pass**

```bash
nix develop -c cargo nextest run -p mnesis-example-axum-todos
```

Expected: all tests PASS including `loop_folds_writes_and_reopen_resumes_from_checkpoint`.

- [ ] **Step 5: Format + commit**

```bash
nix develop -c cargo fmt --all
git add -A
git commit -m "feat(examples): axum-todos hand-rolled \$all projection loop (#326)"
```

---

### Task 5: HTTP layer + app assembly (`http.rs`, `lib.rs::spawn_app`, `main.rs`)

The integration tests (Tasks 6–7) are the TDD for this layer; this task makes the app runnable and keeps the upstream handler shape byte-recognizable.

**Files:**
- Modify: `examples/axum-todos/src/http.rs`
- Modify: `examples/axum-todos/src/lib.rs`
- Modify: `examples/axum-todos/src/main.rs`

- [ ] **Step 1: Implement http.rs**

```rust
//! The four upstream handlers, ported. Routes, methods, status codes, and
//! request/response JSON shapes are unchanged from axum's `examples/todos`
//! (see PROVENANCE.md). The one addition is `409 Conflict`: upstream's
//! `todos_update` has a lost-update race (read-lock → clone → drop → write-
//! lock) that silently drops one of two overlapping writers; under mnesis
//! the optimistic version check makes that unrepresentable, so the second
//! writer surfaces here instead of losing its write (finding #326-6).

use std::time::Duration;

use axum::error_handling::HandleErrorLayer;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, patch};
use axum::{Json, Router};
use mnesis_fjall::FjallStore;
use mnesis_store::store::Store;
use mnesis_store::{CommandRepository, ExecuteError, Repository};
use serde::Deserialize;
use tokio::sync::watch;
use tower::{BoxError, ServiceBuilder};
use tower_http::trace::TraceLayer;
use uuid::Uuid;

use crate::domain::{Create, Delete, Todo, TodoId, TodoState, Update};
use crate::index::{TodoView, TodosIndex};

/// Router state — replaces upstream's `Db = Arc<RwLock<HashMap<Uuid, Todo>>>`.
/// Writes go through the store handle; reads come from the projection's
/// watch channel. Both are cheap clones.
#[derive(Clone)]
pub struct AppState {
    pub store: Store<FjallStore>,
    pub index: watch::Receiver<TodosIndex>,
}

/// The upstream router, middleware included, over [`AppState`].
pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/todos", get(todos_index).post(todos_create))
        .route("/todos/{id}", patch(todos_update).delete(todos_delete))
        .layer(
            ServiceBuilder::new()
                .layer(HandleErrorLayer::new(|error: BoxError| async move {
                    if error.is::<tower::timeout::error::Elapsed>() {
                        Ok(StatusCode::REQUEST_TIMEOUT)
                    } else {
                        Err((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("Unhandled internal error: {error}"),
                        ))
                    }
                }))
                .timeout(Duration::from_secs(10))
                .layer(TraceLayer::new_for_http())
                .into_inner(),
        )
        .with_state(state)
}

// The query parameters for todos index (upstream verbatim).
#[derive(Debug, Deserialize, Default)]
pub struct Pagination {
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

async fn todos_index(
    pagination: Query<Pagination>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    // Upstream took a read lock on the HashMap; here the projection's watch
    // channel is borrowed. Eventual consistency is the semantic change: a
    // GET racing its own POST may not see it yet (finding #326-4 — there is
    // no position to await; `save` returns `()`).
    let todos = state.index.borrow().page(
        pagination.offset.unwrap_or(0),
        pagination.limit.unwrap_or(usize::MAX),
    );
    Json(todos)
}

#[derive(Debug, Deserialize)]
struct CreateTodo {
    text: String,
}

async fn todos_create(
    State(state): State<AppState>,
    Json(input): Json<CreateTodo>,
) -> Result<impl IntoResponse, StatusCode> {
    let id = Uuid::new_v4();
    let repo = state.store.repository::<Todo>().json().build();
    let mut todo = Todo::new(TodoId(id));
    repo.execute(&mut todo, Create { id, text: input.text })
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok((StatusCode::CREATED, Json(todo_view(id, todo.state()))))
}

#[derive(Debug, Deserialize)]
struct UpdateTodo {
    text: Option<String>,
    completed: Option<bool>,
}

async fn todos_update(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    Json(input): Json<UpdateTodo>,
) -> Result<impl IntoResponse, StatusCode> {
    let repo = state.store.repository::<Todo>().json().build();
    let mut todo = repo
        .load(TodoId(id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    // "Does it exist?" is a domain question: an id with no events loads as a
    // fresh root at version None, never an error.
    if todo.version().is_none() || todo.state().deleted {
        return Err(StatusCode::NOT_FOUND);
    }
    // `Handle` cannot decide zero events (`Events<E, N>` guarantees ≥ 1,
    // unlike `React`'s `Option`), so the no-op PATCH is answered from loaded
    // state without entering the domain (finding #326-3).
    if input.text.is_none() && input.completed.is_none() {
        return Ok(Json(todo_view(id, todo.state())));
    }
    match repo
        .execute(&mut todo, Update { id, text: input.text, completed: input.completed })
        .await
    {
        Ok(_) => Ok(Json(todo_view(id, todo.state()))),
        Err(e) if e.is_conflict() => Err(StatusCode::CONFLICT),
        Err(ExecuteError::Decide(_)) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn todos_delete(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<StatusCode, StatusCode> {
    let repo = state.store.repository::<Todo>().json().build();
    let mut todo = repo
        .load(TodoId(id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if todo.version().is_none() || todo.state().deleted {
        return Err(StatusCode::NOT_FOUND);
    }
    match repo.execute(&mut todo, Delete { id }).await {
        Ok(_) => Ok(StatusCode::NO_CONTENT),
        Err(e) if e.is_conflict() => Err(StatusCode::CONFLICT),
        Err(ExecuteError::Decide(_)) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

fn todo_view(id: Uuid, state: &TodoState) -> TodoView {
    TodoView { id, text: state.text.clone(), completed: state.completed }
}
```

Import-path check: `ExecuteError` should be at `mnesis_store::ExecuteError` (root re-export, like `CommandRepository`); if not, `mnesis_store::execute::ExecuteError`. Verify with `rg "pub use.*ExecuteError" crates/store/src/lib.rs`.

- [ ] **Step 2: Implement spawn_app + App in lib.rs (replacing the placeholder body, keeping the doc/allow header)**

```rust
pub mod domain;
pub mod http;
pub mod index;

use std::net::SocketAddr;
use std::path::Path;

use mnesis_fjall::{FjallStore, GlobalSeq};
use mnesis_store::store::{RawEventStore, Store};
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::task::JoinHandle;

pub use crate::index::{BoxErr, TodosIndex};

/// A running instance: HTTP server + projection loop over one fjall keyspace.
pub struct App {
    pub addr: SocketAddr,
    pub store: Store<FjallStore>,
    pub index: watch::Receiver<TodosIndex>,
    /// `Some(position)` iff the projection resumed from a persisted
    /// checkpoint rather than folding from scratch — the lifecycle tests'
    /// resume oracle.
    pub resumed_from: Option<GlobalSeq>,
    server: JoinHandle<()>,
    projection: JoinHandle<()>,
}

/// Open (or reopen) the keyspace at `path`, hydrate the index, spawn the
/// projection loop and the server. Bind with port 0 to let the OS pick (tests
/// do); the binary binds upstream's `127.0.0.1:3000`.
pub async fn spawn_app(path: &Path, addr: SocketAddr) -> Result<App, BoxErr> {
    let store = FjallStore::builder(path).open()?.into_store();

    let (seed, checkpoint) = index::hydrate(&store).await?;
    // The watch channel is seeded with the hydrated state, so reads are
    // served immediately after a reopen — no catch-up wait.
    let (tx, rx) = watch::channel(seed.clone());
    let projection = tokio::spawn({
        let store = store.clone();
        async move {
            if let Err(error) = index::run(store, seed, checkpoint, tx).await {
                tracing::error!("projection loop stopped: {error}");
            }
        }
    });

    let router = http::app(http::AppState { store: store.clone(), index: rx.clone() });
    let listener = TcpListener::bind(addr).await?;
    let addr = listener.local_addr()?;
    let server = tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, router).await {
            tracing::error!("server stopped: {error}");
        }
    });

    Ok(App { addr, store, index: rx, resumed_from: checkpoint, server, projection })
}

impl App {
    /// Abort both tasks and drop every store handle so the keyspace lock is
    /// released and the same path can be reopened — the lifecycle teardown.
    pub async fn shutdown(self) {
        let Self { server, projection, store, .. } = self;
        server.abort();
        projection.abort();
        let _ = server.await;
        let _ = projection.await;
        drop(store);
    }

    /// Run until the server task exits (the binary's "serve forever").
    pub async fn wait(self) {
        let _ = self.server.await;
    }
}
```

- [ ] **Step 3: Implement main.rs (upstream's main, storage swapped)**

```rust
//! Provides a RESTful web server managing some Todos, backed by mnesis +
//! mnesis-fjall instead of upstream's `Arc<RwLock<HashMap<Uuid, Todo>>>`.
//!
//! API (unchanged from upstream):
//!
//! - `GET /todos`: return a JSON list of Todos.
//! - `POST /todos`: create a new Todo.
//! - `PATCH /todos/{id}`: update a specific Todo.
//! - `DELETE /todos/{id}`: delete a specific Todo.
//!
//! Run with
//!
//! ```not_rust
//! cargo run -p mnesis-example-axum-todos
//! ```

use std::net::SocketAddr;

use mnesis_example_axum_todos::spawn_app;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                format!("{}=debug,tower_http=debug", env!("CARGO_CRATE_NAME")).into()
            }),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Upstream's Db was in-memory and vanished on exit; a tempdir keyspace
    // preserves that behaviour while exercising the real persistent adapter.
    let data_dir = tempfile::tempdir().expect("create data dir");
    let addr: SocketAddr = "127.0.0.1:3000".parse().expect("static addr parses");
    let app = spawn_app(data_dir.path(), addr).await.expect("app spawns");
    tracing::debug!("listening on {}", app.addr);
    app.wait().await;
}
```

- [ ] **Step 4: Verify it compiles and runs**

```bash
nix develop -c cargo check -p mnesis-example-axum-todos --all-targets
```

Expected: clean. Optionally smoke: `cargo run -p mnesis-example-axum-todos` + `xh POST :3000/todos text=hello` in another shell, Ctrl-C after.

- [ ] **Step 5: Format + commit**

```bash
nix develop -c cargo fmt --all
git add -A
git commit -m "feat(examples): axum-todos HTTP layer over mnesis, contract unchanged (#326)"
```

---

### Task 6: Integration harness + sequence & read-reflects-write tests

**Files:**
- Create: `examples/axum-todos/tests/http.rs`

- [ ] **Step 1: Write the harness and the first two tests**

```rust
//! Integration tests driving the port's HTTP seam: a real `hyper_util`
//! client against a real bound port against a real fjall keyspace — axum's
//! own `examples/testing::the_real_deal` pattern. Per #326, the tests drive
//! the HTTP layer, not the repository: the point is the app's seam, not ours.

use std::net::SocketAddr;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use mnesis_example_axum_todos::{App, TodosIndex, spawn_app};
use serde_json::{Value, json};
use tempfile::TempDir;
use uuid::Uuid;

type HttpClient = Client<HttpConnector, Body>;

fn client() -> HttpClient {
    Client::builder(TokioExecutor::new()).build_http()
}

fn any_addr() -> SocketAddr {
    "127.0.0.1:0".parse().expect("static addr parses")
}

async fn spawn(dir: &TempDir) -> App {
    spawn_app(dir.path(), any_addr()).await.expect("app spawns")
}

fn req(method: &str, url: String, body: Option<Value>) -> Request<Body> {
    let builder = Request::builder().method(method).uri(url);
    match body {
        Some(v) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(&v).expect("json serializes")))
            .expect("request builds"),
        None => builder.body(Body::empty()).expect("request builds"),
    }
}

async fn read_json(response: axum::http::Response<hyper::body::Incoming>) -> Value {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body collects")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("body is JSON")
}

/// Await the projection catching up to a condition — the seam where the port
/// is honestly eventually-consistent (finding #326-4): there is no position
/// for a handler to await, so tests await the watch channel instead.
async fn wait_for_index(app: &App, f: impl Fn(&TodosIndex) -> bool) {
    let mut rx = app.index.clone();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if f(&rx.borrow_and_update()) {
                return;
            }
            rx.changed().await.expect("projection loop alive");
        }
    })
    .await
    .expect("index did not reflect the write within 5s");
}

// ── Category 1: sequence/protocol — the full todo lifecycle over HTTP ──────

#[tokio::test]
async fn sequence_full_todo_lifecycle_over_http() {
    let dir = TempDir::new().expect("tempdir");
    let app = spawn(&dir).await;
    let client = client();
    let base = format!("http://{}", app.addr);

    // POST → 201 with the created todo.
    let res = client
        .request(req("POST", format!("{base}/todos"), Some(json!({"text": "buy milk"}))))
        .await
        .expect("request");
    assert_eq!(res.status(), StatusCode::CREATED);
    let created = read_json(res).await;
    assert_eq!(created["text"], "buy milk");
    assert_eq!(created["completed"], false);
    let id = created["id"].as_str().expect("id is a string").to_owned();
    let uid = Uuid::parse_str(&id).expect("id is a uuid");

    // PATCH → 200 with the updated todo.
    let res = client
        .request(req("PATCH", format!("{base}/todos/{id}"), Some(json!({"completed": true}))))
        .await
        .expect("request");
    assert_eq!(res.status(), StatusCode::OK);
    let updated = read_json(res).await;
    assert_eq!(updated["text"], "buy milk");
    assert_eq!(updated["completed"], true);

    // GET reflects both writes once the projection catches up.
    wait_for_index(&app, |ix| ix.page(0, usize::MAX).iter().any(|t| t.id == uid && t.completed))
        .await;
    let res = client
        .request(req("GET", format!("{base}/todos"), None))
        .await
        .expect("request");
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        read_json(res).await,
        json!([{"id": id, "text": "buy milk", "completed": true}])
    );

    // DELETE → 204; a second DELETE → 404 (deletion is a domain fact).
    let res = client
        .request(req("DELETE", format!("{base}/todos/{id}"), None))
        .await
        .expect("request");
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let res = client
        .request(req("DELETE", format!("{base}/todos/{id}"), None))
        .await
        .expect("request");
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    // GET converges to empty.
    wait_for_index(&app, TodosIndex::is_empty).await;
    let res = client
        .request(req("GET", format!("{base}/todos"), None))
        .await
        .expect("request");
    assert_eq!(read_json(res).await, json!([]));
}

// ── Required criterion: a read served from the projection reflects a write ─

#[tokio::test]
async fn read_from_projection_reflects_http_write() {
    let dir = TempDir::new().expect("tempdir");
    let app = spawn(&dir).await;
    let client = client();
    let base = format!("http://{}", app.addr);

    let res = client
        .request(req("POST", format!("{base}/todos"), Some(json!({"text": "projected"}))))
        .await
        .expect("request");
    assert_eq!(res.status(), StatusCode::CREATED);
    let id = read_json(res).await["id"].as_str().expect("id").to_owned();
    let uid = Uuid::parse_str(&id).expect("uuid");

    wait_for_index(&app, |ix| ix.contains(uid)).await;

    let res = client
        .request(req("GET", format!("{base}/todos"), None))
        .await
        .expect("request");
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        read_json(res).await,
        json!([{"id": id, "text": "projected", "completed": false}])
    );
}
```

- [ ] **Step 2: Run to verify pass (or fix)**

```bash
nix develop -c cargo nextest run -p mnesis-example-axum-todos
```

Expected: both integration tests PASS. If `wait_for_index`'s closure form fights `TodosIndex::is_empty` (method vs fn pointer), use `|ix| ix.is_empty()`.

- [ ] **Step 3: Format + commit**

```bash
nix develop -c cargo fmt --all
git add -A
git commit -m "test(examples): axum-todos sequence + projection-read tests over a bound port (#326)"
```

---

### Task 7: Boundary, lifecycle, and linearizability tests

**Files:**
- Modify: `examples/axum-todos/tests/http.rs` (append)

- [ ] **Step 1: Append the boundary tests**

```rust
// ── Category 3: defensive boundary ──────────────────────────────────────────

#[tokio::test]
async fn patch_with_no_fields_returns_todo_unchanged() {
    // Upstream: both fields optional, absent fields not applied → 200 with
    // the unchanged todo. Under mnesis "decide nothing" has no representation
    // in `Handle` (finding #326-3), so the handler answers this from loaded
    // state; the contract must not show the difference.
    let dir = TempDir::new().expect("tempdir");
    let app = spawn(&dir).await;
    let client = client();
    let base = format!("http://{}", app.addr);

    let res = client
        .request(req("POST", format!("{base}/todos"), Some(json!({"text": "as-is"}))))
        .await
        .expect("request");
    let created = read_json(res).await;
    let id = created["id"].as_str().expect("id").to_owned();

    let res = client
        .request(req("PATCH", format!("{base}/todos/{id}"), Some(json!({}))))
        .await
        .expect("request");
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(read_json(res).await, created);
}

#[tokio::test]
async fn patch_and_delete_unknown_id_are_404() {
    let dir = TempDir::new().expect("tempdir");
    let app = spawn(&dir).await;
    let client = client();
    let base = format!("http://{}", app.addr);
    let ghost = Uuid::new_v4();

    let res = client
        .request(req("PATCH", format!("{base}/todos/{ghost}"), Some(json!({"completed": true}))))
        .await
        .expect("request");
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    let res = client
        .request(req("DELETE", format!("{base}/todos/{ghost}"), None))
        .await
        .expect("request");
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn malformed_json_is_rejected() {
    let dir = TempDir::new().expect("tempdir");
    let app = spawn(&dir).await;
    let client = client();

    let request = Request::builder()
        .method("POST")
        .uri(format!("http://{}/todos", app.addr))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{not json"))
        .expect("request builds");
    let res = client.request(request).await.expect("request");
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn pagination_is_stable_creation_order() {
    // Upstream paginated HashMap::values() — unordered, unstable across
    // requests. The projection had to choose an ordering (finding #326-5);
    // creation order makes offset/limit deterministic, so this test can
    // assert exact pages, which upstream never could.
    let dir = TempDir::new().expect("tempdir");
    let app = spawn(&dir).await;
    let client = client();
    let base = format!("http://{}", app.addr);

    let mut ids = Vec::new();
    for text in ["a", "b", "c"] {
        let res = client
            .request(req("POST", format!("{base}/todos"), Some(json!({"text": text}))))
            .await
            .expect("request");
        ids.push(read_json(res).await["id"].as_str().expect("id").to_owned());
    }
    wait_for_index(&app, |ix| ix.len() == 3).await;

    let page = |query: &'static str| {
        let client = client.clone();
        let base = base.clone();
        async move {
            let res = client
                .request(req("GET", format!("{base}/todos{query}"), None))
                .await
                .expect("request");
            read_json(res).await
        }
    };

    assert_eq!(page("?limit=1").await, json!([{"id": ids[0], "text": "a", "completed": false}]));
    assert_eq!(
        page("?offset=1&limit=1").await,
        json!([{"id": ids[1], "text": "b", "completed": false}])
    );
    assert_eq!(page("?offset=3").await, json!([]));
    assert_eq!(page("?offset=100").await, json!([]));
}
```

- [ ] **Step 2: Append the lifecycle test**

```rust
// ── Category 2: lifecycle — write, close, reopen, resume ────────────────────

#[tokio::test]
async fn lifecycle_reopen_resumes_projection_from_checkpoint() {
    let dir = TempDir::new().expect("tempdir");

    let app = spawn(&dir).await;
    assert!(app.resumed_from.is_none(), "fresh keyspace has no checkpoint");
    let client = client();
    let base = format!("http://{}", app.addr);
    for text in ["first", "second"] {
        let res = client
            .request(req("POST", format!("{base}/todos"), Some(json!({"text": text}))))
            .await
            .expect("request");
        assert_eq!(res.status(), StatusCode::CREATED);
    }
    wait_for_index(&app, |ix| ix.len() == 2).await;
    app.shutdown().await;

    // Reopen the same keyspace: the projection must resume from its
    // committed (state, position) checkpoint, not re-fold from scratch —
    // and reads are served from the hydrated snapshot immediately.
    let app = spawn(&dir).await;
    assert!(app.resumed_from.is_some(), "reopen must find the checkpoint");
    assert_eq!(app.index.borrow().len(), 2, "hydrated snapshot serves reads at once");

    let base = format!("http://{}", app.addr);
    let res = client
        .request(req("GET", format!("{base}/todos"), None))
        .await
        .expect("request");
    let todos = read_json(res).await;
    assert_eq!(todos.as_array().map(Vec::len), Some(2));

    // And the resumed loop keeps folding new writes.
    let res = client
        .request(req("POST", format!("{base}/todos"), Some(json!({"text": "third"}))))
        .await
        .expect("request");
    assert_eq!(res.status(), StatusCode::CREATED);
    wait_for_index(&app, |ix| ix.len() == 3).await;
}
```

- [ ] **Step 3: Append the linearizability test**

First add to the import block at the **top** of `tests/http.rs` (all imports at top of file, never mid-file):

```rust
use mnesis_example_axum_todos::domain::{Todo, TodoId};
use mnesis_store::{CommandRepository as _, Repository as _};
```

Then append:

```rust
// ── Category 4: linearizability — overlapping writers, no lost update ───────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_patches_conflict_instead_of_losing_updates() {
    // Upstream's todos_update silently loses one of two overlapping writes
    // (read-lock → clone → drop → write-lock). Under mnesis that race is
    // unrepresentable: every 200 appended exactly one event, and a stale
    // overlapping writer gets 409 (finding #326-6 — a status upstream never
    // returns, because it lost the update instead).
    //
    // Overlap is genuine (two spawned tasks, two connections, one Barrier)
    // but the interleaving is the scheduler's: any single round may
    // serialize. So every round asserts the no-lost-update invariant, and
    // across 32 rounds at least one genuine conflict must surface.
    let dir = TempDir::new().expect("tempdir");
    let app = spawn(&dir).await;
    let base = format!("http://{}", app.addr);
    let setup_client = client();

    let mut total_conflicts = 0_u64;
    for round in 0_u32..32 {
        let res = setup_client
            .request(req("POST", format!("{base}/todos"), Some(json!({"text": "race"}))))
            .await
            .expect("request");
        let id = read_json(res).await["id"].as_str().expect("id").to_owned();
        let uid = Uuid::parse_str(&id).expect("uuid");

        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let tasks: Vec<_> = (0..2)
            .map(|writer| {
                let barrier = std::sync::Arc::clone(&barrier);
                let url = format!("{base}/todos/{id}");
                tokio::spawn(async move {
                    let client = client(); // own connection per writer
                    barrier.wait().await;
                    let res = client
                        .request(req(
                            "PATCH",
                            url,
                            Some(json!({"text": format!("writer-{writer}")})),
                        ))
                        .await
                        .expect("request");
                    res.status()
                })
            })
            .collect();

        let mut accepted = 0_u64;
        let mut conflicted = 0_u64;
        for task in tasks {
            match task.await.expect("writer task ran") {
                StatusCode::OK => accepted += 1,
                StatusCode::CONFLICT => conflicted += 1,
                other => panic!("unexpected status {other} in round {round}"),
            }
        }
        assert_eq!(accepted + conflicted, 2);

        // The invariant upstream violates: nothing is silently lost. The
        // stream's version is exactly 1 (Created) + one event per accepted
        // PATCH — the repository read is the oracle, the HTTP layer the SUT.
        let repo = app.store.repository::<Todo>().json().build();
        let root = repo.load(TodoId(uid)).await.expect("load");
        assert_eq!(
            root.version().map(mnesis::Version::as_u64),
            Some(1 + accepted),
            "round {round}: accepted writes must all be in the log"
        );
        total_conflicts += conflicted;
    }

    assert!(
        total_conflicts > 0,
        "32 barrier-aligned rounds never overlapped — no conflict surfaced"
    );
}
```

Add `mnesis` to `[dev-dependencies]` usage — already there (fixture). `Version::as_u64` — verify the method exists (`mnesis::Version`; the fjall example calls `d.version.as_u64()`).

- [ ] **Step 4: Run the full suite**

```bash
nix develop -c cargo nextest run -p mnesis-example-axum-todos
```

Expected: all PASS. If `concurrent_patches...` proves flaky in repetition (`--run-ignored` loops), record it in the deviation log — a flaky conflict test through a real HTTP stack is itself a finding about testing guidance.

- [ ] **Step 5: Format + commit**

```bash
nix develop -c cargo fmt --all
git add -A
git commit -m "test(examples): axum-todos boundary, lifecycle, and no-lost-update tests (#326)"
```

---

### Task 8: Narrative docs — lib.rs `//!`, README, PROVENANCE update

**Files:**
- Modify: `examples/axum-todos/src/lib.rs` (doc comment)
- Create: `examples/axum-todos/README.md`
- Modify: `examples/axum-todos/PROVENANCE.md` (port-complete note)

- [ ] **Step 1: Write the lib.rs narrative doc**

Replace the placeholder `//!` block with (keep the `#![allow]`s):

```rust
//! Port of axum's own `examples/todos` onto mnesis + mnesis-fjall (#326).
//!
//! The HTTP contract — routes, methods, status codes, request/response
//! shapes — is unchanged from upstream (see `PROVENANCE.md`); the
//! `Arc<RwLock<HashMap<Uuid, Todo>>>` behind it is replaced by an
//! event-sourced todo aggregate (one stream per todo) on a real on-disk
//! [`FjallStore`], with `GET /todos` served from a consumer-owned `$all`
//! projection.
//!
//! **This example is evidence, not a demo**: the requirements were written
//! by people with no knowledge of mnesis, so every place the port strains
//! is a genuine finding about the application-author seam — each is filed
//! as its own issue and cross-referenced in `README.md`. The two additions
//! the contract could not absorb silently:
//!
//! - `409 Conflict` — upstream's `todos_update` lost-update race is
//!   unrepresentable here, so the second overlapping writer surfaces.
//! - Eventual consistency on `GET` — reads come from the projection;
//!   there is no read-your-writes barrier to build (`save` returns no
//!   position).
//!
//! Surfaces exercised: `#[mnesis::aggregate]` / `Handle` / `events!`,
//! `Store::repository::<A>().json().build()`, `CommandRepository::execute`
//! + `ExecuteError::is_conflict`, `Subscription::subscribe_all` +
//! `.events().decoded()`, `Projector`, fjall's
//! `SnapshotStore<Vec<u8>, GlobalSeq>` via `CodecSnapshotStore`, and
//! `AggregateFixture` (tests).
```

- [ ] **Step 2: Write README.md**

`examples/axum-todos/README.md` — sections:

```markdown
# axum-todos — the application-author seam, validated (#326)

A port of axum's own [`examples/todos`](https://github.com/tokio-rs/axum/tree/main/examples/todos)
with the `Arc<RwLock<HashMap>>` replaced by `mnesis` + `mnesis-fjall`, and the
HTTP contract left as upstream wrote it. Provenance and licence: `PROVENANCE.md`.

## What it proves

Every earlier example was written by the people who designed the API. This one
takes its requirements from outside — so the frictions it hits are findings
about the seam an ordinary application author uses, not staged demos.

## Run

    cargo run -p mnesis-example-axum-todos
    xh POST :3000/todos text="buy milk"
    xh GET :3000/todos
    xh PATCH :3000/todos/<id> completed:=true
    xh DELETE :3000/todos/<id>

Tests (they drive a real bound port): `cargo nextest run -p mnesis-example-axum-todos`.

## Architecture

- `domain.rs` — one event stream per todo; granular events behind the fat
  upstream PATCH; every event carries its todo's `id` (see finding 7).
- `index.rs` — creation-ordered read model + the hand-rolled `$all` loop
  (subscribe_all → decode → fold → commit(state, position) → publish to
  `tokio::sync::watch`).
- `http.rs` — the four upstream handlers, verbatim in shape; `409` added.
- Writes: handler → `repo.execute`. Reads: handler → `watch::Receiver`.

## Findings (each filed as its own issue)

| # | Finding | Issue |
|---|---------|-------|
| 1 | `Projection` stepper is `Version`-only; an `$all` projection hand-rolls the loop it was built to remove | #TBD |
| 2 | `PersistTrigger`/`Decoded` are `Version`-typed; the `$all` position rides beside the item with no policy or slot for it | #TBD |
| 3 | `Handle` cannot decide zero events; the legitimate no-op `PATCH {}` is answered outside the domain | #TBD |
| 4 | `save`/`execute` return no position → read-your-writes is unbuildable at the repository seam | #TBD |
| 5 | `GET /todos` forced an ordering decision upstream never made (creation order chosen) | #TBD |
| 6 | `409 Conflict` is an unavoidable contract divergence — upstream silently lost the update instead | #TBD |
| 7 | `$all` items carry no stream id → the aggregate id is threaded path → command → every event payload by hand | #TBD |

(Fill the issue numbers in Task 9.)
```

- [ ] **Step 3: Append a port-complete note to PROVENANCE.md**

```markdown
## Port status

The port is complete: `src/main.rs`/`src/http.rs` preserve the upstream
routes, methods, status codes, and JSON shapes. Deliberate divergences —
`409 Conflict` (upstream loses the update silently) and eventual consistency
on `GET` (reads come from a projection) — are recorded as findings in
`README.md` and filed as issues.
```

- [ ] **Step 4: Commit**

```bash
nix develop -c cargo fmt --all
git add -A
git commit -m "docs(examples): axum-todos narrative, README findings table, provenance status (#326)"
```

---

### Task 9: File the findings as GitHub issues, fill the README table

**⚠️ Outward-facing: confirm with the user before creating issues.**

- [ ] **Step 1: Draft and (after user confirmation) file seven issues via `gh`** (account `joeldsouzax`; repo labels: `area: examples` where fitting, milestone judgment to the user). Titles:

1. `Projection stepper cannot drive an $all projection (Version-only bounds)` — body cites `projection.rs:115-121`, fjall's `SnapshotStore<Vec<u8>, GlobalSeq>` existing unused by the stepper, and the axum-todos hand-rolled loop as the evidence; proposes generalizing `Projection` over position `P`.
2. `PersistTrigger and Decoded have no $all position slot` — `should_persist(Option<Version>, Version, ..)`; `Decoded { version: Version }`; the `(P, PersistedEnvelope)` tuple is the only carrier.
3. `Handle cannot decide zero events — no-op commands have no representation` — `aggregate.rs:178` vs `saga.rs:88-91` (`React` returns `Option`); axum-todos answers `PATCH {}` outside the domain.
4. `Repository::save/execute return no position — read-your-writes unbuildable` — `repository.rs:109-113`; the port accepted eventual consistency because there is nothing to await.
5. `Docs: multi-stream read models must choose an ordering` — projection guide gap; axum-todos chose creation order.
6. `Docs: optimistic conflict surfaces a status CRUD contracts don't have` — 409 divergence pattern worth documenting for app authors.
7. `$all subscription items carry no stream id` — envelope carries only `global_seq`; the id must ride in every event payload by hand (command → event, since `Handle` has no identity access); ask whether the `$all` read surface should carry the stream key beside the position.

- [ ] **Step 2: Fill the `#TBD`s in README.md with the real issue numbers, commit**

```bash
git add examples/axum-todos/README.md
git commit -m "docs(examples): axum-todos findings issue numbers (#326)"
```

---

### Task 10: Gate, clippy sweep, PR

- [ ] **Step 1: The full-surface clippy sweep the gate skips** (flake clippy is `--lib` only; the workspace must be clean under all-targets/all-features):

```bash
nix develop -c cargo clippy --workspace --all-targets --all-features
```

Expected: zero warnings. Fix anything found (never relax lints).

- [ ] **Step 2: Push and open the PR** (the pre-commit hook already ran `nix flake check` on every commit)

```bash
git push -u origin feat/326-axum-todos-example
gh pr create --title "feat(examples): port axum todos onto mnesis (#326)" --body "..."
```

PR body: what it proves (application-author seam), acceptance-criteria checklist from #326 each ticked with evidence, findings table with issue links, note that no production crate changed. End with the Claude Code attribution footer.

- [ ] **Step 3: Verify CI (Nix Flake Check) is green; hand to user for squash-merge.**

---

## Deviation log

(Record divergences here as they happen: what, why, impact.)

- 2026-07-16, Task 2: `then_expect_state` takes `impl FnOnce(&State)`, not `&State` — plan example adapted to closure form (kernel API was right, plan was wrong; no impact).
- 2026-07-16, Task 2 review: added `delete_missing_todo_is_rejected` (11th test) — quality review demonstrated the `!state.created` guard branch of `Handle<Delete>` was unkilled by the planned 10 tests.
- 2026-07-16, Task 3 review: added `pagination_offset_and_limit_compose_in_order` + `completion_change_for_unknown_todo_is_a_projection_error` (17th/18th tests) — skip/take order swap and the second `ok_or` branch survived the planned 5 tests.
- 2026-07-16, environment: the pre-commit `nix flake check` flaked once on `mnesis-nostd` ("can't find crate for `core`", thumbv7em) during parallel hook builds; identical derivation rebuilt clean. Transient, machine-local; retry the commit if seen.
