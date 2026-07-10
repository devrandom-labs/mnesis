//! The kit's own red/green loop: every conformance check runs against
//! `InMemoryStore`, the reference adapter. A check that fails here is a kit
//! bug (or a real `InMemoryStore` contract violation — either way, a find).

#![allow(clippy::unwrap_used, reason = "tests")]
#![allow(clippy::expect_used, reason = "tests")]
#![allow(clippy::missing_panics_doc, reason = "tests")]

use nexus_inmemory::InMemoryStore;

#[allow(
    clippy::unused_async,
    reason = "factory shape must stay async fn — other adapters' factories (fjall, postgres) \
              await inside; InMemoryStore::new() alone has nothing to await"
)]
async fn factory() -> (InMemoryStore, ()) {
    (InMemoryStore::new(), ())
}

// Check modules land in Tasks 2-6; the macro invocation replaces these direct
// calls in Task 7. For now this file only proves the crate + factory compile.
#[tokio::test]
async fn factory_compiles() {
    let (_store, ()) = factory().await;
}
