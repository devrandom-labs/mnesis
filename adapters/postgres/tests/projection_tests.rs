//! Existing projection stepper over real event history and native checkpoint CAS.
#![cfg(feature = "projection")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "native projection assertions"
)]

use futures::TryStreamExt;
use mnesis_postgres::PostgresStore;
use mnesis_store::checkpoint::{
    CheckpointError, CheckpointHydrated, CheckpointRejection, CheckpointStore,
};
use mnesis_store::state::{AfterEventTypes, CodecSnapshotStore};
use mnesis_store::{
    DecodedStreamExt, JsonCodec, Projection, ProjectionError, ProjectionStateError, RawEventStore,
    StreamKey,
};
use mnesis_test_domains::{CounterEvent, CounterState};
use std::num::{NonZeroU32, NonZeroU64};

#[path = "support/projection.rs"]
mod projection;

async fn fixture() -> Option<PostgresStore> {
    let url = std::env::var("DATABASE_URL").ok()?;
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    sqlx::raw_sql("DROP SCHEMA IF EXISTS mnesis CASCADE; DROP VIEW IF EXISTS public.events")
        .execute(&pool)
        .await
        .unwrap();
    PostgresStore::migrate(&pool).await.unwrap();
    Some(PostgresStore::from_pool(pool).await.unwrap())
}

#[tokio::test]
async fn real_projection_reloads_the_exact_pair_and_resumes_after_its_native_position() {
    let Some(raw) = fixture().await else { return };
    let events = projection::events(&raw).await;
    let id = StreamKey::from_slice(b"count");
    let codec = CodecSnapshotStore::new(&raw, JsonCodec::default());
    let mut first = Projection::load(
        id.clone(),
        projection::Counter,
        AfterEventTypes::new(&["Incremented", "Set"]),
        &codec,
        NonZeroU32::MIN,
    )
    .await
    .unwrap();
    assert_eq!(first.checkpoint(), None);
    first.advance(events[0].clone()).await.unwrap();
    assert_eq!(first.checkpoint(), Some(events[0].0));
    assert_eq!(first.state().unwrap(), &CounterState { value: 1 });
    drop(first);
    let mut resumed = Projection::load(
        id.clone(),
        projection::Counter,
        AfterEventTypes::new(&["Incremented", "Set"]),
        &codec,
        NonZeroU32::MIN,
    )
    .await
    .unwrap();
    assert_eq!(resumed.state().unwrap(), &CounterState { value: 1 });
    let remaining = raw
        .read_all(resumed.checkpoint())
        .await
        .unwrap()
        .decoded::<CounterEvent, _>(JsonCodec::default())
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    assert_eq!(remaining, [events[1].clone()]);
    resumed
        .advance(remaining.into_iter().next().unwrap())
        .await
        .unwrap();
    assert_eq!(resumed.state().unwrap(), &CounterState { value: 4 });
    assert_eq!(
        codec
            .hydrate_checkpoint(&id, NonZeroU32::MIN)
            .await
            .unwrap(),
        CheckpointHydrated::Found {
            revision: NonZeroU64::new(2).unwrap(),
            position: events[1].0,
            state: CounterState { value: 4 },
        }
    );
}

#[tokio::test]
async fn real_projection_rebuild_changes_schema_without_resetting_revision() {
    let Some(raw) = fixture().await else { return };
    let events = projection::events(&raw).await;
    let id = StreamKey::from_slice(b"rebuild-count");
    let codec = CodecSnapshotStore::new(&raw, JsonCodec::default());
    let mut initial = Projection::load(
        id.clone(),
        projection::Counter,
        AfterEventTypes::new(&["Incremented", "Set"]),
        &codec,
        NonZeroU32::MIN,
    )
    .await
    .unwrap();
    for item in &events {
        initial.advance(item.clone()).await.unwrap();
    }
    drop(initial);
    let schema = NonZeroU32::new(2).unwrap();
    let rejected = Projection::load(
        id.clone(),
        projection::Counter,
        AfterEventTypes::new(&["Incremented", "Set"]),
        &codec,
        schema,
    )
    .await;
    assert!(
        matches!(rejected, Err(CheckpointError::Rejected(CheckpointRejection::SchemaMismatch {
        stored, requested,
    })) if stored == NonZeroU32::MIN && requested == schema)
    );
    let mut rebuilt = Projection::rebuild(
        id.clone(),
        projection::Counter,
        AfterEventTypes::new(&["Incremented", "Set"]),
        &codec,
        schema,
    )
    .await
    .unwrap();
    assert_eq!(rebuilt.checkpoint(), None);
    assert_eq!(rebuilt.state().unwrap(), &CounterState { value: 0 });
    for item in &events {
        rebuilt.advance(item.clone()).await.unwrap();
    }
    assert_eq!(rebuilt.state().unwrap(), &CounterState { value: 4 });
    assert_eq!(
        codec.hydrate_checkpoint(&id, schema).await.unwrap(),
        CheckpointHydrated::Found {
            revision: NonZeroU64::new(4).unwrap(),
            position: events[1].0,
            state: CounterState { value: 4 },
        }
    );
}

#[tokio::test]
async fn rejected_projection_cannot_continue_until_it_reloads_committed_state() {
    let Some(raw) = fixture().await else { return };
    let events = projection::events(&raw).await;
    let id = StreamKey::from_slice(b"racing-consumers");
    let codec = CodecSnapshotStore::new(&raw, JsonCodec::default());
    let mut winner = Projection::load(
        id.clone(),
        projection::Counter,
        AfterEventTypes::new(&["Incremented", "Set"]),
        &codec,
        NonZeroU32::MIN,
    )
    .await
    .unwrap();
    let mut lagging = Projection::load(
        id.clone(),
        projection::Counter,
        AfterEventTypes::new(&["Incremented", "Set"]),
        &codec,
        NonZeroU32::MIN,
    )
    .await
    .unwrap();
    winner.advance(events[0].clone()).await.unwrap();
    let result = lagging.advance(events[0].clone()).await;
    assert!(
        matches!(result, Err(ProjectionError::Commit(CheckpointError::Rejected(
        CheckpointRejection::Conflict { expected: None, actual: Some(revision) }
    ))) if revision == NonZeroU64::MIN)
    );
    assert!(matches!(
        lagging.state(),
        Err(ProjectionStateError::ReloadRequired)
    ));
    assert!(matches!(
        lagging.advance(events[1].clone()).await,
        Err(ProjectionError::State(ProjectionStateError::ReloadRequired))
    ));
    let mut resumed = Projection::load(
        id.clone(),
        projection::Counter,
        AfterEventTypes::new(&["Incremented", "Set"]),
        &codec,
        NonZeroU32::MIN,
    )
    .await
    .unwrap();
    assert_eq!(resumed.checkpoint(), Some(events[0].0));
    assert_eq!(resumed.state().unwrap(), &CounterState { value: 1 });
    resumed.advance(events[1].clone()).await.unwrap();
    assert_eq!(
        codec
            .hydrate_checkpoint(&id, NonZeroU32::MIN)
            .await
            .unwrap(),
        CheckpointHydrated::Found {
            revision: NonZeroU64::new(2).unwrap(),
            position: events[1].0,
            state: CounterState { value: 4 },
        }
    );
}
