//! The existing projection consumer used by logical and physical recovery laws.

use std::num::NonZeroU32;

use futures::TryStreamExt;
use mnesis_postgres::{PgAllPos, PostgresStore};
use mnesis_store::state::{AfterEventTypes, CodecSnapshotStore};
use mnesis_store::{DecodedStreamExt, JsonCodec, Projection, RawEventStore, StreamKey};
use mnesis_test_domains::CounterEvent;

use super::projection::Counter;

pub async fn resume(store: &PostgresStore) -> (PgAllPos, i64) {
    let codec = CodecSnapshotStore::new(store, JsonCodec::default());
    let mut consumer = Projection::load(
        StreamKey::from_slice(b"restored-counter"),
        Counter,
        AfterEventTypes::new(&["Incremented", "Set"]),
        &codec,
        NonZeroU32::MIN,
    )
    .await
    .unwrap();
    let events = store
        .read_all(consumer.checkpoint())
        .await
        .unwrap()
        .decoded::<CounterEvent, _>(JsonCodec::default());
    futures::pin_mut!(events);
    while let Some(item) = events.try_next().await.unwrap() {
        consumer.advance(item).await.unwrap();
    }
    (
        consumer.checkpoint().unwrap(),
        consumer.state().unwrap().value,
    )
}
