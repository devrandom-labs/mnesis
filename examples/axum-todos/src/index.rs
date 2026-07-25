//! The read model behind `GET /todos`.

use std::fmt;
use std::num::NonZeroU32;

use futures::StreamExt;
use mnesis_fjall::{FjallStore, GlobalSeq};
use mnesis_store::state::{CodecSnapshotStore, Hydrated, PersistTrigger, SnapshotStore};
use mnesis_store::store::Store;
use mnesis_store::{
    DecodedStreamExt, JsonCodec, Projection, Projector, StepStreamExt, Subscription,
};
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

/// What the projection publishes on its watch channel: the folded index paired
/// with the `$all` [`GlobalSeq`] checkpoint it was folded to.
///
/// State and checkpoint travel in **one** payload, never two channels, so a
/// reader that observes `checkpoint >= pos` is guaranteed the paired `index`
/// already reflects every event up to `pos` — the read-your-writes token a
/// `GET` awaits (#330). This is the same "state and position are one value"
/// invariant [`commit_persisted`](mnesis::AggregateRoot::commit_persisted) and
/// [`SnapshotStore`] rely on; two channels would let a reader see a fresh
/// checkpoint against a stale index.
#[derive(Debug, Clone, Default)]
pub struct IndexState {
    /// The folded read model.
    pub index: TodosIndex,
    /// The `$all` position `index` was folded through, or `None` before the
    /// first event. `Ord` on `Option<GlobalSeq>` puts `None` below every
    /// position, so a wait for `checkpoint >= Some(pos)` never matches `None`.
    pub checkpoint: Option<GlobalSeq>,
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

/// Commit `(state, position)` after every event.
///
/// The watch channel publishes per event, so the durable checkpoint never
/// lags the published state — a restart never replays an event readers have
/// already observed. `EveryNEvents` is deliberately `Version`-only (bucket
/// arithmetic has no meaning on a composite `$all` position — #328), so an
/// `$all` per-event pacer is this four-line custom trigger.
struct EveryEvent;

impl PersistTrigger<GlobalSeq> for EveryEvent {
    fn should_persist(
        &self,
        _old_position: Option<GlobalSeq>,
        _new_position: GlobalSeq,
        _event_names: impl Iterator<Item: AsRef<str>>,
    ) -> bool {
        true
    }
}

/// One boxed error domain for the loop.
///
/// Subscription register, read, decode, fold, and snapshot commit all differ
/// in type; the loop only surfaces the error and dies — handling and logging
/// are the spawner's job.
pub type BoxErr = Box<dyn std::error::Error + Send + Sync>;

/// Load the persisted `(state, checkpoint)` pair, if any.
///
/// `spawn_app`'s synchronous peek for the watch-channel seed and the
/// `resumed_from` oracle; [`run`]'s own `Projection::load` re-reads the same
/// snapshot as its authoritative starting point (one extra startup
/// point-read).
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
/// Driven by the position-generic [`Projection`] stepper (#327): `load`
/// hydrates `(state, checkpoint)` from fjall's `projections` partition, and
/// the `(GlobalSeq, StreamKey, Decoded)` tuple the subscription yields feeds
/// [`Projection::advance`] whole — the stepper drops the attribution key
/// (this projection routes by the payload's todo id) — no hand-rolled
/// fold/commit loop. The
/// [`EveryEvent`] trigger commits every fold, so there is no pending tail
/// (a `flush` would be a no-op — if you ever swap [`EveryEvent`] for a
/// bucketed trigger, add `proj.flush(&state)` after the loop) and
/// `send_replace` pays one clone per event — the price of the deliberately
/// no-`Clone` fold, at the seam where another task must see the state.
///
/// Die-on-error contract: any `Err` (hydrate, register, read, decode, fold,
/// or commit) ends the loop and drops `tx`, so receivers observe
/// `changed() -> Err` as the death signal. A deterministic fold error (e.g.
/// [`IndexError::UnknownTodo`]) is a permanent crash-loop across restarts —
/// the committed checkpoint sits just before the poisoned event, so every
/// resume re-reads it; recovery is a rebuild (schema bump), never a retry.
/// The caller must seed the watch channel with the hydrated state (see
/// [`hydrate`]), or readers serve a stale default until the first event
/// arrives.
pub async fn run(store: Store<FjallStore>, tx: watch::Sender<IndexState>) -> Result<(), BoxErr> {
    let snapshots = CodecSnapshotStore::new(store.raw(), JsonCodec::default());
    let (mut proj, mut state) =
        Projection::load(IndexId, TodosProjector, EveryEvent, snapshots, INDEX_SCHEMA).await?;
    let stream = Subscription::new(&store)
        .subscribe_all(proj.checkpoint())?
        .events()
        .decoded(JsonCodec::default());
    tokio::pin!(stream);

    while let Some(item) = stream.next().await {
        state = proj.advance(state, item?).await?;
        // Publish the folded state paired with the checkpoint it reached, so a
        // reader awaiting a returned position sees a consistent snapshot (#330).
        tx.send_replace(IndexState {
            index: state.clone(),
            checkpoint: proj.checkpoint(),
        });
    }
    Ok(())
}

#[cfg(test)]
#[allow(
    clippy::panic,
    reason = "test asserts an unreachable projection outcome via panic in a let-else"
)]
mod tests {
    use std::time::Duration;

    use mnesis_fjall::FjallStore;
    use mnesis_store::store::RawEventStore as _;
    use mnesis_store::{CommandRepository as _, Execution};
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

    /// Persist one `Created` event through the real repository path, returning
    /// the `$all` position it landed at — the read-your-writes token (#330).
    async fn create_todo(store: &Store<FjallStore>, id: Uuid, text: &str) -> GlobalSeq {
        let repo = store.repository::<Todo>().json().build();
        let mut root = Todo::new(TodoId(id));
        let Execution::Executed { position, .. } = repo
            .execute(
                &mut root,
                Create {
                    id,
                    text: text.to_owned(),
                },
            )
            .await
            .expect("create")
        else {
            panic!("Create always decides an event");
        };
        position
    }

    /// Spawn [`run`] and await until the published checkpoint reaches `target`
    /// — the loop-level read-your-writes wait (the exact discipline the HTTP
    /// `GET` uses). When the checkpoint reaches `target` the paired `index` in
    /// the same [`IndexState`] necessarily reflects that write. Returns the
    /// receiver. `run` loads its own checkpoint — the production path.
    async fn drive_until(
        store: &Store<FjallStore>,
        seed: IndexState,
        target: GlobalSeq,
    ) -> watch::Receiver<IndexState> {
        let (tx, mut rx) = watch::channel(seed);
        let loop_store = store.clone();
        let task = tokio::spawn(async move {
            let _ = run(loop_store, tx).await;
        });
        tokio::time::timeout(Duration::from_secs(5), async {
            rx.wait_for(|s| s.checkpoint >= Some(target))
                .await
                .expect("loop alive");
        })
        .await
        .expect("loop reaches the returned position");
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

            // Write one todo through the repository, then drive the loop until
            // its checkpoint reaches the position the write returned.
            let pos = create_todo(&store, first_id, "persisted").await;
            let published = drive_until(
                &store,
                IndexState {
                    index: seed,
                    checkpoint,
                },
                pos,
            )
            .await;
            assert!(
                published.borrow().index.contains(first_id),
                "the write is visible once the checkpoint reaches its position"
            );
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
            let pos = create_todo(&store, second_id, "resumed").await;
            let rx = drive_until(
                &store,
                IndexState {
                    index: seed,
                    checkpoint,
                },
                pos,
            )
            .await;

            let ids: Vec<Uuid> = rx
                .borrow()
                .index
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
