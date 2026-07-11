//! `InMemoryStore` conformance against the executable store contract —
//! every check delegated to the `nexus-store-testing` kit (#281).

#![allow(clippy::unwrap_used, reason = "tests")]
#![allow(clippy::expect_used, reason = "tests")]
#![allow(clippy::missing_panics_doc, reason = "tests")]

use nexus::Version;
use nexus_inmemory::{InMemorySnapshotStore, InMemoryStore};

nexus_store_testing::conformance! {
    factory: || async { (InMemoryStore::new(), ()) },
}

nexus_store_testing::conformance_atomic_append! {
    factory: || async { (InMemoryStore::new(), ()) },
}

nexus_store_testing::conformance_snapshot! {
    factory: || async { (InMemorySnapshotStore::<Vec<u8>, Version>::new(), ()) },
    positions: (Version::new(5).unwrap(), Version::new(9).unwrap()),
    extremes: (Version::new(1).unwrap(), Version::new(u64::MAX).unwrap()),
}
