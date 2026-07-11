//! `nexus-fjall::FjallStore` conformance against the executable store
//! contract — every check delegated to the `nexus-store-testing` kit (#281).
//!
//! `open_fresh` is shared by the base matrix and the lifecycle checks: the
//! kit's context slot `C` carries the `TempDir` so the on-disk directory
//! stays alive for exactly as long as the `FjallStore` that reads it, and
//! the lifecycle `reopen` closure drops the store, then reopens the SAME
//! `dir.path()` — proving real persistence (fjall is the kit's first
//! persistent adapter; `InMemoryStore`'s "reopen" only exercises plumbing).

#![allow(clippy::unwrap_used, reason = "tests")]
#![allow(clippy::expect_used, reason = "tests")]
#![allow(clippy::missing_panics_doc, reason = "tests")]

use nexus::Version;
use nexus_fjall::FjallStore;
use tempfile::TempDir;

async fn open_fresh() -> (FjallStore, TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = FjallStore::builder(dir.path().join("db"))
        .open()
        .expect("open fjall store");
    (store, dir)
}

nexus_store_testing::conformance! {
    factory: open_fresh,
}

nexus_store_testing::conformance_atomic_append! {
    factory: open_fresh,
}

nexus_store_testing::conformance_snapshot! {
    factory: open_fresh,
    positions: (Version::new(5).unwrap(), Version::new(9).unwrap()),
    extremes: (Version::new(1).unwrap(), Version::new(u64::MAX).unwrap()),
}

nexus_store_testing::conformance_lifecycle! {
    open: open_fresh,
    reopen: |store: FjallStore, dir: TempDir| async move {
        drop(store);
        let reopened = FjallStore::builder(dir.path().join("db"))
            .open()
            .expect("reopen fjall store");
        (reopened, dir)
    },
}
