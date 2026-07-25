//! Port of axum's own `examples/todos` onto mnesis + mnesis-fjall (#326).
//!
//! The HTTP contract — routes, methods, status codes, request/response
//! shapes — is unchanged from upstream (see `PROVENANCE.md`); the
//! `Arc<RwLock<HashMap<Uuid, Todo>>>` behind it is replaced by an
//! event-sourced todo aggregate (one stream per todo) on a real on-disk
//! `FjallStore`, with `GET /todos` served from a consumer-owned `$all`
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
//! - `503 Service Unavailable` — reads come from a projection; a dead
//!   projection loop must not serve frozen reads as 200s.
//!
//! `GET` is **read-your-writes** consistent (#330, closing finding #326-4): a
//! write echoes the `$all` position it landed at in an `X-Mnesis-Position`
//! header, and a `GET` that sends the same header back blocks until the
//! projection's checkpoint reaches it — so a client always observes its own
//! write. Without the header, `GET` stays eventually consistent (a client that
//! doesn't care pays nothing).
//!
//! Surfaces exercised: `#[mnesis::aggregate]` / `Handle` / `events!`,
//! `Store::repository::<A>().json().build()`,
//! `CommandRepository::execute` returning `Execution { position, .. }` with
//! `ExecuteError::is_conflict`,
//! `Subscription::subscribe_all` with `.events().decoded()`, `Projector`,
//! fjall's `SnapshotStore<Vec<u8>, GlobalSeq>` via `CodecSnapshotStore`,
//! and `AggregateFixture` (unit tests).

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

use std::net::SocketAddr;
use std::path::Path;

use mnesis_fjall::{FjallStore, GlobalSeq};
use mnesis_store::store::{RawEventStore, Store};
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::task::JoinHandle;

pub use crate::index::{BoxErr, IndexState, TodosIndex};

/// A running instance: HTTP server + projection loop over one fjall keyspace.
pub struct App {
    pub addr: SocketAddr,
    pub store: Store<FjallStore>,
    pub index: watch::Receiver<IndexState>,
    /// `Some(position)` iff the projection resumed from a persisted
    /// checkpoint rather than folding from scratch — the lifecycle tests'
    /// resume oracle.
    pub resumed_from: Option<GlobalSeq>,
    server: JoinHandle<()>,
    projection: JoinHandle<()>,
}

/// Open (or reopen) the keyspace at `path`, hydrate the index, spawn the
/// projection loop and the server.
///
/// Bind with port 0 to let the OS pick (tests do); the binary binds
/// upstream's `127.0.0.1:3000`.
pub async fn spawn_app(path: &Path, addr: SocketAddr) -> Result<App, BoxErr> {
    let store = FjallStore::builder(path).open()?.into_store();

    let (seed, checkpoint) = index::hydrate(&store).await?;
    // The watch channel is seeded with the hydrated state *and its checkpoint*,
    // so reads are served immediately after a reopen and a read-your-writes wait
    // for any position at or below the checkpoint is satisfied at once — no
    // catch-up wait. The loop's own `Projection::load` re-reads the same
    // snapshot as its starting point.
    let (tx, rx) = watch::channel(IndexState {
        index: seed,
        checkpoint,
    });
    let projection_store = store.clone();
    let projection = tokio::spawn(async move {
        if let Err(error) = index::run(projection_store, tx).await {
            tracing::error!("projection loop stopped: {error}");
        }
    });

    let router = http::app(http::AppState {
        store: store.clone(),
        index: rx.clone(),
    });
    let listener = TcpListener::bind(addr).await?;
    let bound = listener.local_addr()?;
    let server = tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, router).await {
            tracing::error!("server stopped: {error}");
        }
    });

    Ok(App {
        addr: bound,
        store,
        index: rx,
        resumed_from: checkpoint,
        server,
        projection,
    })
}

impl App {
    /// Abort both tasks and drop every store handle so the keyspace lock is
    /// released and the same path can be reopened — the lifecycle teardown.
    pub async fn shutdown(self) {
        let Self {
            server,
            projection,
            store,
            ..
        } = self;
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
