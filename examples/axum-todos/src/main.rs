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

// The binary is a crate root of its own, so it restates the example's
// posture: `expect` documents an assumption at startup (same as lib.rs).
#![allow(
    clippy::expect_used,
    reason = "example binary: expect documents a startup assumption"
)]
#![allow(
    clippy::doc_markdown,
    reason = "the doc header keeps upstream's wording verbatim (`RESTful`)"
)]

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
