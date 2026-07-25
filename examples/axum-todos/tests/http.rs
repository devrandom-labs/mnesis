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
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use mnesis::Version;
use mnesis_example_axum_todos::domain::{Todo, TodoId};
use mnesis_example_axum_todos::{App, spawn_app};
use mnesis_store::repository::Repository as _;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::Barrier;
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

/// One `GET /todos{query}` page as JSON.
async fn page(client: &HttpClient, base: &str, query: &str) -> Value {
    let res = client
        .request(req("GET", format!("{base}/todos{query}"), None))
        .await
        .expect("request");
    assert_eq!(res.status(), StatusCode::OK);
    read_json(res).await
}

/// The `$all` position a write echoed in `X-Mnesis-Position` — the
/// read-your-writes token (#330). Every write that appends an event returns it.
fn write_position(res: &axum::http::Response<hyper::body::Incoming>) -> String {
    res.headers()
        .get("x-mnesis-position")
        .expect("a write echoes its $all position")
        .to_str()
        .expect("position header is ascii")
        .to_owned()
}

/// `GET /todos{query}` sending `position` back in `X-Mnesis-Position`, so the
/// handler blocks until the projection has folded through it — the read-your-
/// writes wait that replaces the old eventual-consistency content predicate
/// (finding #326-4 resolved). Asserts `200` and returns the page JSON.
async fn get_after(client: &HttpClient, base: &str, query: &str, position: &str) -> Value {
    let res = client
        .request(
            Request::builder()
                .method("GET")
                .uri(format!("{base}/todos{query}"))
                .header("x-mnesis-position", position)
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("request");
    assert_eq!(res.status(), StatusCode::OK);
    read_json(res).await
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

    // PATCH → 200 with the updated todo, echoing its $all position.
    let res = client
        .request(req(
            "PATCH",
            format!("{base}/todos/{id}"),
            Some(json!({"completed": true})),
        ))
        .await
        .expect("request");
    assert_eq!(res.status(), StatusCode::OK);
    let patch_pos = write_position(&res);
    let updated = read_json(res).await;
    assert_eq!(updated["text"], "buy milk");
    assert_eq!(updated["completed"], true);

    // GET awaiting the PATCH's position reflects both writes — read-your-writes,
    // no content-predicate poll (#330).
    assert_eq!(
        get_after(&client, &base, "", &patch_pos).await,
        json!([{"id": id, "text": "buy milk", "completed": true}])
    );

    // DELETE → 204 with its position; a second DELETE → 404 (a domain fact).
    let res = client
        .request(req("DELETE", format!("{base}/todos/{id}"), None))
        .await
        .expect("request");
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let delete_pos = write_position(&res);
    let res = client
        .request(req("DELETE", format!("{base}/todos/{id}"), None))
        .await
        .expect("request");
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    // GET awaiting the DELETE's position sees the empty index.
    assert_eq!(get_after(&client, &base, "", &delete_pos).await, json!([]));

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
    let position = write_position(&res);
    let id = read_json(res).await["id"].as_str().expect("id").to_owned();

    // GET awaiting the POST's position must see the write it just made.
    assert_eq!(
        get_after(&client, &base, "", &position).await,
        json!([{"id": id, "text": "projected", "completed": false}])
    );

    app.shutdown().await;
}

// ── Category 3: defensive boundary — inputs violating the happy path ───────

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
        .request(req(
            "POST",
            format!("{base}/todos"),
            Some(json!({"text": "as-is"})),
        ))
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

    app.shutdown().await;
}

#[tokio::test]
async fn patch_and_delete_unknown_id_are_404() {
    let dir = TempDir::new().expect("tempdir");
    let app = spawn(&dir).await;
    let client = client();
    let base = format!("http://{}", app.addr);
    let ghost = Uuid::new_v4();

    let res = client
        .request(req(
            "PATCH",
            format!("{base}/todos/{ghost}"),
            Some(json!({"completed": true})),
        ))
        .await
        .expect("request");
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    let res = client
        .request(req("DELETE", format!("{base}/todos/{ghost}"), None))
        .await
        .expect("request");
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    app.shutdown().await;
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

    app.shutdown().await;
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
    let mut last_pos = String::new();
    for text in ["a", "b", "c"] {
        let res = client
            .request(req(
                "POST",
                format!("{base}/todos"),
                Some(json!({"text": text})),
            ))
            .await
            .expect("request");
        last_pos = write_position(&res);
        ids.push(read_json(res).await["id"].as_str().expect("id").to_owned());
    }
    // Block once until the projection has folded all three creates; the
    // subsequent best-effort pages then read a caught-up index.
    let _ = get_after(&client, &base, "", &last_pos).await;

    assert_eq!(
        page(&client, &base, "?limit=1").await,
        json!([{"id": ids[0], "text": "a", "completed": false}])
    );
    assert_eq!(
        page(&client, &base, "?offset=1&limit=1").await,
        json!([{"id": ids[1], "text": "b", "completed": false}])
    );
    assert_eq!(page(&client, &base, "?offset=3").await, json!([]));
    assert_eq!(page(&client, &base, "?offset=100").await, json!([]));

    app.shutdown().await;
}

// ── Category 2: lifecycle — shutdown, reopen, resume over HTTP ─────────────

#[tokio::test]
async fn lifecycle_reopen_resumes_projection_from_checkpoint() {
    let dir = TempDir::new().expect("tempdir");

    let app = spawn(&dir).await;
    assert!(
        app.resumed_from.is_none(),
        "fresh keyspace has no checkpoint"
    );
    let client = client();
    let base = format!("http://{}", app.addr);
    let mut last_pos = String::new();
    for text in ["first", "second"] {
        let res = client
            .request(req(
                "POST",
                format!("{base}/todos"),
                Some(json!({"text": text})),
            ))
            .await
            .expect("request");
        assert_eq!(res.status(), StatusCode::CREATED);
        last_pos = write_position(&res);
    }
    // Read-your-writes: block until the projection has folded both creates, so
    // the shutdown below persists a checkpoint that covers them.
    let page = get_after(&client, &base, "", &last_pos).await;
    assert_eq!(page.as_array().map(Vec::len), Some(2));
    app.shutdown().await;

    // Reopen the same keyspace: the projection must resume from its
    // committed (state, position) checkpoint, not re-fold from scratch —
    // and reads are served from the hydrated snapshot immediately.
    let app = spawn(&dir).await;
    assert!(
        app.resumed_from.is_some(),
        "reopen must find the checkpoint"
    );
    assert_eq!(
        app.index.borrow().index.len(),
        2,
        "hydrated snapshot serves reads at once"
    );

    // The respawned server binds a fresh port, so the client's pool keys on
    // a new authority and never reuses a connection to the dead server.
    let base = format!("http://{}", app.addr);
    let res = client
        .request(req("GET", format!("{base}/todos"), None))
        .await
        .expect("request");
    let todos = read_json(res).await;
    assert_eq!(todos.as_array().map(Vec::len), Some(2));

    // And the resumed loop keeps folding new writes.
    let res = client
        .request(req(
            "POST",
            format!("{base}/todos"),
            Some(json!({"text": "third"})),
        ))
        .await
        .expect("request");
    assert_eq!(res.status(), StatusCode::CREATED);
    let third_pos = write_position(&res);
    let page = get_after(&client, &base, "", &third_pos).await;
    assert_eq!(page.as_array().map(Vec::len), Some(3));

    app.shutdown().await;
}

// ── Category 4: linearizability — no lost updates under overlap ────────────

/// One barrier-aligned round of two overlapping PATCH requests to a fresh todo:
/// returns `(accepted, conflicted)` after asserting the no-lost-update
/// invariant against the event log (the repository read is the oracle,
/// the HTTP layer the SUT).
async fn conflict_round(
    app: &App,
    setup_client: &HttpClient,
    base: &str,
    round: u32,
) -> (u64, u64) {
    let res = setup_client
        .request(req(
            "POST",
            format!("{base}/todos"),
            Some(json!({"text": "race"})),
        ))
        .await
        .expect("request");
    let id = read_json(res).await["id"].as_str().expect("id").to_owned();
    let uid = Uuid::parse_str(&id).expect("uuid");

    let barrier = Arc::new(Barrier::new(2));
    let tasks: Vec<_> = (0..2)
        .map(|writer| {
            let writer_barrier = Arc::clone(&barrier);
            let url = format!("{base}/todos/{id}");
            tokio::spawn(async move {
                let client = client(); // own connection per writer
                writer_barrier.wait().await;
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
        let status = task.await.expect("writer task ran");
        assert!(
            status == StatusCode::OK || status == StatusCode::CONFLICT,
            "unexpected status {status} in round {round}"
        );
        if status == StatusCode::OK {
            accepted += 1;
        } else {
            conflicted += 1;
        }
    }
    assert_eq!(accepted + conflicted, 2);

    // The invariant upstream violates: nothing is silently lost. The
    // stream's version is exactly 1 (Created) + one event per accepted
    // PATCH.
    let repo = app.store.repository::<Todo>().json().build();
    let root = repo.load(TodoId(uid)).await.expect("load");
    assert_eq!(
        root.version().map(Version::as_u64),
        Some(1 + accepted),
        "round {round}: accepted writes must all be in the log"
    );
    (accepted, conflicted)
}

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
        let (_, conflicted) = conflict_round(&app, &setup_client, &base, round).await;
        total_conflicts += conflicted;
    }

    assert!(
        total_conflicts > 0,
        "32 barrier-aligned rounds never overlapped — no conflict surfaced"
    );

    app.shutdown().await;
}
