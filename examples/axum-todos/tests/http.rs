//! Integration tests driving the port's HTTP seam: a real `hyper_util`
//! client against a real bound port against a real fjall keyspace — axum's
//! own `examples/testing::the_real_deal` pattern. Per #326, the tests drive
//! the HTTP layer, not the repository: the point is the app's seam, not ours.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code drives a live server; expect/unwrap document harness assumptions"
)]
#![allow(
    clippy::shadow_unrelated,
    reason = "test code re-`let res = ...` for each request/response pair in sequence; each shadow is a fresh, unrelated response"
)]
#![allow(
    clippy::future_not_send,
    reason = "test-only helper capturing a non-Send `Fn` closure across an await; the harness is single-threaded #[tokio::test], never spawned"
)]

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
        .request(req(
            "POST",
            format!("{base}/todos"),
            Some(json!({"text": "buy milk"})),
        ))
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
        .request(req(
            "PATCH",
            format!("{base}/todos/{id}"),
            Some(json!({"completed": true})),
        ))
        .await
        .expect("request");
    assert_eq!(res.status(), StatusCode::OK);
    let updated = read_json(res).await;
    assert_eq!(updated["text"], "buy milk");
    assert_eq!(updated["completed"], true);

    // GET reflects both writes once the projection catches up.
    wait_for_index(&app, |ix| {
        ix.page(0, usize::MAX)
            .iter()
            .any(|t| t.id == uid && t.completed)
    })
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

    app.shutdown().await;
}

// ── Required criterion: a read served from the projection reflects a write ─

#[tokio::test]
async fn read_from_projection_reflects_http_write() {
    let dir = TempDir::new().expect("tempdir");
    let app = spawn(&dir).await;
    let client = client();
    let base = format!("http://{}", app.addr);

    let res = client
        .request(req(
            "POST",
            format!("{base}/todos"),
            Some(json!({"text": "projected"})),
        ))
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

    app.shutdown().await;
}
