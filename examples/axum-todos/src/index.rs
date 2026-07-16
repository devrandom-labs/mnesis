//! The read model behind `GET /todos`.

use std::fmt;
use std::num::NonZeroU32;

use futures::StreamExt;
use mnesis_fjall::{FjallStore, GlobalSeq};
use mnesis_store::state::{CodecSnapshotStore, Hydrated, SnapshotStore};
use mnesis_store::store::Store;
use mnesis_store::{DecodedStreamExt, JsonCodec, Projector, StepStreamExt, Subscription};
use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use uuid::Uuid;

use crate::domain::TodoEvent;

/// The JSON shape upstream serves from `GET /todos` — `{ id, text, completed }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoView {
    pub id: Uuid,
    pub text: String,
    pub completed: bool,
}

/// Creation-ordered todos.
///
/// Vec order **is** `$all` fold order of `Created` events, so
/// `offset`/`limit` pagination is stable across requests. Upstream
/// paginated `HashMap::values()`, which is unordered — a projection must
/// choose an ordering, and upstream never did (finding #326-5).
///
/// Updates do an `O(n)` linear find per event — fine at example scale; a
/// production projection would pair the Vec with an id→index map.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodosIndex {
    todos: Vec<TodoView>,
}

impl TodosIndex {
    /// One page of todos — the upstream `skip(offset).take(limit)` contract.
    #[must_use]
    pub fn page(&self, offset: usize, limit: usize) -> Vec<TodoView> {
        self.todos
            .iter()
            .skip(offset)
            .take(limit)
            .cloned()
            .collect()
    }

    /// Whether a todo with this id is currently in the index.
    #[must_use]
    pub fn contains(&self, id: Uuid) -> bool {
        self.todos.iter().any(|todo| todo.id == id)
    }

    /// Number of todos currently in the index.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.todos.len()
    }

    /// Whether the index currently holds no todos.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.todos.is_empty()
    }
}

/// Fold-time failure of the read-model projector.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum IndexError {
    /// An update or delete addressed an id the index never saw created.
    ///
    /// This means a corrupt or reordered `$all` feed — recovery policy is
    /// the consumer's (mnesis rule); this projector only surfaces it.
    #[error("event for unknown todo {id}")]
    UnknownTodo { id: Uuid },
}

/// Pure fold of `TodoEvent`s into the creation-ordered [`TodosIndex`].
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
                state.todos.push(TodoView {
                    id: *id,
                    text: text.clone(),
                    completed: false,
                });
            }
            TodoEvent::TextChanged { id, text } => {
                state
                    .todos
                    .iter_mut()
                    .find(|todo| todo.id == *id)
                    .ok_or(IndexError::UnknownTodo { id: *id })?
                    .text
                    .clone_from(text);
            }
            TodoEvent::CompletionChanged { id, completed } => {
                state
                    .todos
                    .iter_mut()
                    .find(|todo| todo.id == *id)
                    .ok_or(IndexError::UnknownTodo { id: *id })?
                    .completed = *completed;
            }
            TodoEvent::Deleted { id } => {
                state.todos.retain(|todo| todo.id != *id);
            }
        }
        Ok(state)
    }
}

/// The one name of this projection — diagnostic label and storage key alike,
/// so the two can never drift.
const INDEX_NAME: &str = "todos-index";

/// The projection's own id in fjall's `projections` partition.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct IndexId;

impl fmt::Display for IndexId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(INDEX_NAME)
    }
}

impl AsRef<[u8]> for IndexId {
    fn as_ref(&self) -> &[u8] {
        INDEX_NAME.as_bytes()
    }
}

/// Schema version of the folded state — bump to force a rebuild on deploy.
pub const INDEX_SCHEMA: NonZeroU32 = NonZeroU32::MIN;

/// One boxed error domain for the loop.
///
/// Subscription register, read, decode, fold, and snapshot commit all differ
/// in type; the loop only surfaces the error and dies — handling and logging
/// are the spawner's job.
pub type BoxErr = Box<dyn std::error::Error + Send + Sync>;

/// Load the persisted `(state, checkpoint)` pair, if any.
///
/// `Stale` (schema bump) folds from scratch, exactly like `Absent` — for this
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
/// the `$all` position rides *beside* the envelope as fjall's [`GlobalSeq`],
/// `Decoded` has no slot for it, and no shipped trigger can accept it. The
/// loop commits every event (so there is no pending tail and no flush);
/// `send_replace` then pays one clone per event — the price of the
/// deliberately no-`Clone` fold, at the seam where another task must see the
/// state.
///
/// Die-on-error contract: any `Err` (register, read, decode, fold, or commit)
/// ends the loop and drops `tx`, so receivers observe `changed() -> Err` as
/// the death signal. A deterministic fold error (e.g. [`IndexError::UnknownTodo`])
/// is a permanent crash-loop across restarts — the committed checkpoint sits
/// just before the poisoned event, so every resume re-reads it; recovery is a
/// rebuild (schema bump), never a retry. The caller must seed the watch
/// channel with the hydrated state, or readers serve a stale default until
/// the first event arrives.
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use mnesis_fjall::FjallStore;
    use mnesis_store::CommandRepository as _;
    use mnesis_store::store::RawEventStore as _;
    use tokio::sync::watch;
    use uuid::Uuid;

    use super::*;
    use crate::domain::{Create, Todo, TodoId};

    fn created(id: Uuid, text: &str) -> TodoEvent {
        TodoEvent::Created {
            id,
            text: text.to_owned(),
        }
    }

    fn fold(events: &[TodoEvent]) -> TodosIndex {
        events
            .iter()
            .try_fold(TodosProjector.initial(), |state, event| {
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
            TodoEvent::TextChanged {
                id: b,
                text: "renamed".to_owned(),
            },
            TodoEvent::CompletionChanged {
                id: a,
                completed: true,
            },
        ]);
        let page = index.page(0, usize::MAX);
        assert_eq!((page[0].text.as_str(), page[0].completed), ("first", true));
        assert_eq!(
            (page[1].text.as_str(), page[1].completed),
            ("renamed", false)
        );
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
            &TodoEvent::TextChanged {
                id,
                text: "ghost".to_owned(),
            },
        );
        assert_eq!(result.unwrap_err(), IndexError::UnknownTodo { id });
    }

    #[test]
    fn completion_change_for_unknown_todo_is_a_projection_error() {
        let id = Uuid::new_v4();
        let result = TodosProjector.apply(
            TodosProjector.initial(),
            &TodoEvent::CompletionChanged {
                id,
                completed: true,
            },
        );
        assert_eq!(result.unwrap_err(), IndexError::UnknownTodo { id });
    }

    #[test]
    fn pagination_clamps_past_the_end() {
        let index = fold(&[created(Uuid::new_v4(), "only")]);
        assert!(index.page(5, usize::MAX).is_empty());
        assert_eq!(index.page(0, 0).len(), 0);
    }

    #[test]
    fn pagination_offset_and_limit_compose_in_order() {
        let (a, b, c) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let index = fold(&[
            created(a, "first"),
            created(b, "second"),
            created(c, "third"),
        ]);
        let page = index.page(1, 1);
        assert_eq!(page.len(), 1);
        assert_eq!((page[0].id, page[0].text.as_str()), (b, "second"));
    }

    /// Persist one `Created` event through the real repository path.
    async fn create_todo(store: &Store<FjallStore>, id: Uuid, text: &str) {
        let repo = store.repository::<Todo>().json().build();
        let mut root = Todo::new(TodoId(id));
        repo.execute(
            &mut root,
            Create {
                id,
                text: text.to_owned(),
            },
        )
        .await
        .expect("create");
    }

    /// Spawn [`run`] from `(seed, checkpoint)` and await until the published
    /// index contains `id`; returns the receiver holding the folded state.
    async fn drive_until_contains(
        store: &Store<FjallStore>,
        seed: TodosIndex,
        checkpoint: Option<GlobalSeq>,
        id: Uuid,
    ) -> watch::Receiver<TodosIndex> {
        let (tx, mut rx) = watch::channel(seed.clone());
        let loop_store = store.clone();
        let task = tokio::spawn(async move {
            let _ = run(loop_store, seed, checkpoint, tx).await;
        });
        tokio::time::timeout(Duration::from_secs(5), async {
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
        rx
    }

    #[tokio::test]
    async fn loop_folds_writes_and_reopen_resumes_from_checkpoint() {
        let dir = tempfile::tempdir().expect("tempdir");
        let first_id = Uuid::new_v4();

        // Fresh store: hydrate finds nothing.
        {
            let store = FjallStore::builder(dir.path())
                .open()
                .expect("open")
                .into_store();
            let (seed, checkpoint) = hydrate(&store).await.expect("hydrate");
            assert_eq!(seed, TodosIndex::default());
            assert!(checkpoint.is_none());

            // Write one todo through the repository, then drive the loop and
            // watch it publish the folded index.
            create_todo(&store, first_id, "persisted").await;
            drive_until_contains(&store, seed, checkpoint, first_id).await;
        }
        // All store handles dropped: the keyspace closes and the same path
        // reopens. hydrate must find the committed (state, position) pair —
        // the projection resumes, it does not re-fold (lifecycle category).
        {
            let store = FjallStore::builder(dir.path())
                .open()
                .expect("reopen")
                .into_store();
            let (seed, checkpoint) = hydrate(&store).await.expect("rehydrate");
            assert!(seed.contains(first_id), "state came back from the snapshot");
            assert_eq!(
                checkpoint,
                GlobalSeq::new(1),
                "checkpoint is the first event's position"
            );

            // Restart the loop from the recovered pair and append a SECOND
            // todo. Resume is strictly after the checkpoint, and Created is a
            // non-idempotent push — if resume ever re-delivered event 1, the
            // duplicate would land before event 2 and fail the exact-page
            // assertion below.
            let second_id = Uuid::new_v4();
            create_todo(&store, second_id, "resumed").await;
            let rx = drive_until_contains(&store, seed, checkpoint, second_id).await;

            let ids: Vec<Uuid> = rx
                .borrow()
                .page(0, usize::MAX)
                .iter()
                .map(|todo| todo.id)
                .collect();
            assert_eq!(
                ids,
                vec![first_id, second_id],
                "exactly the two todos in creation order — resume did not re-fold event 1"
            );
        }
    }
}
