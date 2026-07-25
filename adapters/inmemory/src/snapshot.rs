//! In-memory [`SnapshotStore`] — the snapshot/projection-state counterpart of
//! [`InMemoryStore`](crate::InMemoryStore).

use std::collections::HashMap;
use std::convert::Infallible;
use std::num::NonZeroU32;

use mnesis::Id;
use mnesis_store::state::{Hydrated, SnapshotStore};
use tokio::sync::RwLock;

/// In-memory snapshot store for tests.
///
/// Keyed by the id's **stable byte identity** (`Id::as_ref`), the contract's
/// designated storage key — never its human-facing `Display`, which is lossy
/// (distinct binary ids can share one `Display` string and would collide).
#[derive(Debug, Default)]
pub struct InMemorySnapshotStore<S, P> {
    snapshots: RwLock<HashMap<Vec<u8>, (NonZeroU32, P, S)>>,
}

impl<S, P> InMemorySnapshotStore<S, P> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            snapshots: RwLock::new(HashMap::new()),
        }
    }
}

impl<S, P> SnapshotStore<S, P> for InMemorySnapshotStore<S, P>
where
    S: Clone + Send + Sync + 'static,
    P: Clone + Send + Sync + 'static,
{
    type Error = Infallible;

    async fn hydrate(
        &self,
        id: &impl Id,
        schema_version: NonZeroU32,
    ) -> Result<Hydrated<S, P>, Infallible> {
        let snapshots = self.snapshots.read().await;
        Ok(match snapshots.get(id.as_ref()) {
            None => Hydrated::Absent,
            Some((stored_schema, _, _)) if *stored_schema != schema_version => Hydrated::Stale {
                stored_schema: *stored_schema,
            },
            Some((_, position, state)) => Hydrated::Found {
                position: position.clone(),
                state: state.clone(),
            },
        })
    }

    async fn commit(
        &self,
        id: &impl Id,
        schema_version: NonZeroU32,
        position: P,
        state: &S,
    ) -> Result<(), Infallible> {
        self.snapshots.write().await.insert(
            id.as_ref().to_vec(),
            (schema_version, position, state.clone()),
        );
        Ok(())
    }
}
