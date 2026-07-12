//! The kit's own red/green loop against `InMemoryStore`, driven through the
//! same macros adapters use — so the macros themselves are under test.

#![allow(clippy::unwrap_used, reason = "tests")]
#![allow(clippy::expect_used, reason = "tests")]
#![allow(clippy::missing_panics_doc, reason = "tests")]

use mnesis::Version;
use mnesis_inmemory::{InMemorySnapshotStore, InMemoryStore};

mnesis_store_testing::conformance! {
    factory: || async { (InMemoryStore::new(), ()) },
}

mnesis_store_testing::conformance_atomic_append! {
    factory: || async { (InMemoryStore::new(), ()) },
}

mnesis_store_testing::conformance_snapshot! {
    factory: || async { (InMemorySnapshotStore::<Vec<u8>, Version>::new(), ()) },
    positions: (Version::new(5).unwrap(), Version::new(9).unwrap()),
    extremes: (Version::new(1).unwrap(), Version::new(u64::MAX).unwrap()),
}

// In-memory "reopen" hands back the same store — validates the kit's closure
// plumbing only; fjall/postgres prove real persistence.
mnesis_store_testing::conformance_lifecycle! {
    open: || async { (InMemoryStore::new(), ()) },
    reopen: |store: InMemoryStore, (): ()| async move { (store, ()) },
}
