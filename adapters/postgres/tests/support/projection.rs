//! Real counter consumer and two-event history, shared by projection and wire laws.

use futures::TryStreamExt;
use mnesis_postgres::{PgAllPos, PostgresStore};
use mnesis_store::{
    Decoded, DecodedStreamExt, JsonCodec, Projector, RawEventStore, Repository, Store, StreamKey,
};
use mnesis_test_domains::{
    Counter as AggregateCounter, CounterError, CounterEvent, CounterState, TestId,
};

pub struct Counter;

impl Projector for Counter {
    type Event = CounterEvent;
    type State = CounterState;
    type Error = CounterError;

    fn initial(&self) -> CounterState {
        CounterState::default()
    }

    fn apply(
        &self,
        state: CounterState,
        event: &CounterEvent,
    ) -> Result<CounterState, CounterError> {
        let value = match event {
            CounterEvent::Incremented => state.value.checked_add(1).ok_or(CounterError)?,
            CounterEvent::Decremented => state.value.checked_sub(1).ok_or(CounterError)?,
            CounterEvent::Set(value) => *value,
        };
        Ok(CounterState { value })
    }
}

pub type Item = (PgAllPos, StreamKey, Decoded<CounterEvent>);

pub async fn events(raw: &PostgresStore) -> Vec<Item> {
    let store = Store::new(raw.clone());
    let repository = store.repository::<AggregateCounter>().json().build();
    for (id, event) in [
        ("first", CounterEvent::Incremented),
        ("second", CounterEvent::Set(4)),
    ] {
        let mut root = repository.load(TestId::new(id)).await.unwrap();
        repository
            .save::<0>(&mut root, &mnesis::events![event])
            .await
            .unwrap();
    }
    let history = raw
        .read_all(None)
        .await
        .unwrap()
        .decoded::<CounterEvent, _>(JsonCodec::default())
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    assert_eq!(history.len(), 2);
    history
}
