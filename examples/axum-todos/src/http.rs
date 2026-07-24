//! The four upstream handlers, ported.
//!
//! Routes, methods, status codes, and request/response JSON shapes are
//! unchanged from axum's `examples/todos` (see PROVENANCE.md). The one
//! addition is `409 Conflict`: upstream's `todos_update` has a lost-update
//! race (read-lock → clone → drop → write-lock) that silently drops one of
//! two overlapping writers; under mnesis the optimistic version check makes
//! that unrepresentable, so the second writer surfaces here instead of
//! losing its write (finding #326-6).

use std::error::Error;
use std::time::Duration;

use axum::error_handling::HandleErrorLayer;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, patch};
use axum::{Json, Router};
use mnesis_fjall::FjallStore;
use mnesis_store::repository::Repository;
use mnesis_store::store::Store;
use mnesis_store::{CommandRepository, ExecuteError};
use serde::Deserialize;
use tokio::sync::watch;
use tower::{BoxError, ServiceBuilder};
use tower_http::trace::TraceLayer;
use uuid::Uuid;

use crate::domain::{Create, Delete, Todo, TodoError, TodoId, TodoState, Update};
use crate::index::{TodoView, TodosIndex};

/// Router state — replaces upstream's `Db = Arc<RwLock<HashMap<Uuid, Todo>>>`.
///
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

/// Log the underlying failure before collapsing it to a 500 — upstream's
/// handlers were infallible, so every error path here is new port surface,
/// and a silent 500 is undiagnosable.
fn internal<E: Error>(error: E) -> StatusCode {
    tracing::error!(%error, "request failed");
    StatusCode::INTERNAL_SERVER_ERROR
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
) -> Result<impl IntoResponse, StatusCode> {
    // The projection loop's death signal is the dropped sender (see
    // `index::run`'s docs): without this guard, a dead loop would serve
    // frozen reads with 200s forever. 503, like 409, is a status upstream
    // never returns — the minimum honest divergence.
    if state.index.has_changed().is_err() {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    // Upstream took a read lock on the HashMap; here the projection's watch
    // channel is borrowed. Eventual consistency is the semantic change: a
    // GET racing its own POST may not see it yet (finding #326-4 — there is
    // no position to await; `save` returns `()`).
    let todos = state.index.borrow().page(
        pagination.offset.unwrap_or(0),
        pagination.limit.unwrap_or(usize::MAX),
    );
    Ok(Json(todos))
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
    // Minting the facade per request is Arc-clone cheap; one mint per
    // aggregate type per handler is the intended pattern.
    let repo = state.store.repository::<Todo>().json().build();
    let mut todo = Todo::new(TodoId(id));
    repo.execute(
        &mut todo,
        Create {
            id,
            text: input.text,
        },
    )
    .await
    .map_err(internal)?;
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
    let mut todo = repo.load(TodoId(id)).await.map_err(internal)?;
    // "Does it exist?" is a domain question: an id with no events loads as a
    // fresh root at version None, never an error.
    if todo.version().is_none() || todo.state().deleted {
        return Err(StatusCode::NOT_FOUND);
    }
    // The all-absent PATCH body needs no special case here: `Handle` decides
    // `Ok(None)` for it, `execute` skips the append, and `todo` is left at the
    // version and state it loaded with (#329, closing finding #326-3).
    match repo
        .execute(
            &mut todo,
            Update {
                id,
                text: input.text,
                completed: input.completed,
            },
        )
        .await
    {
        Ok(_) => Ok(Json(todo_view(id, todo.state()))),
        Err(e) if e.is_conflict() => Err(StatusCode::CONFLICT),
        Err(ExecuteError::Decide(TodoError::NotFound)) => Err(StatusCode::NOT_FOUND),
        Err(error) => Err(internal(error)),
    }
}

async fn todos_delete(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<StatusCode, StatusCode> {
    let repo = state.store.repository::<Todo>().json().build();
    let mut todo = repo.load(TodoId(id)).await.map_err(internal)?;
    if todo.version().is_none() || todo.state().deleted {
        return Err(StatusCode::NOT_FOUND);
    }
    match repo.execute(&mut todo, Delete { id }).await {
        Ok(_) => Ok(StatusCode::NO_CONTENT),
        Err(e) if e.is_conflict() => Err(StatusCode::CONFLICT),
        Err(ExecuteError::Decide(TodoError::NotFound)) => Err(StatusCode::NOT_FOUND),
        Err(error) => Err(internal(error)),
    }
}

fn todo_view(id: Uuid, state: &TodoState) -> TodoView {
    TodoView {
        id,
        text: state.text.clone(),
        completed: state.completed,
    }
}
