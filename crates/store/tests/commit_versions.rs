//! A07: commit counts are derived, and overflow is rejected before side effects.
#![allow(clippy::unwrap_used, reason = "test assertions")]
use futures::{FutureExt, StreamExt};
use mnesis::{AggregateRoot, Events, KernelError, Version, events};
use mnesis_inmemory::InMemoryStore;
use mnesis_store::wake::{WakeRegistration, WakeSource};
use mnesis_store::{Decode, Encode, JsonCodec, PersistedEnvelope, RawEventStore, Repository};
use mnesis_test_domains::{Counter, CounterEvent, CounterState, TestId};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

fn one() -> Events<CounterEvent> {
    events![CounterEvent::Incremented]
}
fn three() -> Events<CounterEvent, 2> {
    events![
        CounterEvent::Incremented,
        CounterEvent::Incremented,
        CounterEvent::Incremented
    ]
}
#[test]
fn commit_counts_fresh_existing_and_repeated_batches() {
    for starting in [None, Version::new(10)] {
        let mut root = starting.map_or_else(
            || AggregateRoot::<Counter>::new(TestId::new("s")),
            |version| AggregateRoot::restore(TestId::new("s"), CounterState { value: 0 }, version),
        );
        let expected_first = starting.map_or(1, |_| 11);
        assert_eq!(
            root.commit_version(&one()).unwrap().as_u64(),
            expected_first
        );
        assert_eq!(root.version(), starting);
        assert_eq!(root.state().unwrap().value, 0);
        root.commit_persisted(&one()).unwrap();
        assert_eq!(root.version().unwrap().as_u64(), expected_first);
        assert_eq!(
            root.commit_version(&three()).unwrap().as_u64(),
            expected_first.checked_add(3).unwrap()
        );
        root.commit_persisted(&three()).unwrap();
        root.commit_persisted(&one()).unwrap();
        assert_eq!(
            root.version().unwrap().as_u64(),
            expected_first.checked_add(4).unwrap()
        );
        assert_eq!(root.state().unwrap().value, 5);
    }
}
#[test]
fn commit_counts_used_events_instead_of_capacity() {
    let mut root = AggregateRoot::<Counter>::new(TestId::new("s"));
    let mut batch: Events<CounterEvent, 8> = Events::new(CounterEvent::Incremented);
    assert_eq!(
        root.commit_version(&batch).unwrap(),
        Version::new(1).unwrap()
    );
    batch.add(CounterEvent::Incremented);
    assert_eq!(
        root.commit_version(&batch).unwrap(),
        Version::new(2).unwrap()
    );
    root.commit_persisted(&batch).unwrap();
    assert_eq!(root.version(), Version::new(2));
    assert_eq!(root.state().unwrap().value, 2);
}

#[test]
fn ceiling_and_whole_batch_overflow_preserve_state_and_version() {
    for version in [u64::MAX, u64::MAX.checked_sub(1).unwrap()] {
        let mut root = AggregateRoot::<Counter>::restore(
            TestId::new("s"),
            CounterState { value: i64::MAX },
            Version::new(version).unwrap(),
        );
        assert!(matches!(
            root.commit_version(&three()),
            Err(KernelError::VersionOverflow)
        ));
        assert!(matches!(
            root.commit_persisted(&three()),
            Err(KernelError::VersionOverflow)
        ));
        assert_eq!(root.version(), Version::new(version));
        assert_eq!(root.state().unwrap().value, i64::MAX);
    }
    let mut root = AggregateRoot::<Counter>::restore(
        TestId::new("s"),
        CounterState { value: 0 },
        Version::new(u64::MAX.checked_sub(1).unwrap()).unwrap(),
    );
    root.commit_persisted(&one()).unwrap();
    assert_eq!(root.version(), Version::new(u64::MAX));
    assert_eq!(root.state().unwrap().value, 1);
    assert!(matches!(
        root.commit_persisted(&one()),
        Err(KernelError::VersionOverflow)
    ));
    assert_eq!(root.state().unwrap().value, 1);
}

struct CountingCodec(Arc<AtomicUsize>);
impl Encode<CounterEvent> for CountingCodec {
    type Error = <JsonCodec as Encode<CounterEvent>>::Error;
    fn encode(&self, event: &CounterEvent) -> Result<bytes::Bytes, Self::Error> {
        self.0.fetch_add(1, Ordering::SeqCst);
        JsonCodec::default().encode(event)
    }
}
impl Decode<CounterEvent> for CountingCodec {
    type Output<'a> = CounterEvent;
    type Error = <JsonCodec as Decode<CounterEvent>>::Error;
    fn decode<'a>(&'a self, envelope: &'a PersistedEnvelope) -> Result<CounterEvent, Self::Error> {
        JsonCodec::default().decode(envelope)
    }
}
#[tokio::test]
async fn overflow_precedes_encoding_metadata_storage_and_wakes() {
    let store = InMemoryStore::new().into_store();
    let encoded = Arc::new(AtomicUsize::new(0));
    let metadata = Arc::new(AtomicUsize::new(0));
    let metadata_calls = Arc::clone(&metadata);
    let repo = store
        .repository::<Counter>()
        .codec(CountingCodec(Arc::clone(&encoded)))
        .metadata(move |_, _: &CounterEvent, _: &mnesis_store::Payload| {
            metadata_calls.fetch_add(1, Ordering::SeqCst);
            None
        })
        .build();
    let registration = store.raw().register(None).unwrap();
    let wake = registration.arm();
    let mut root = AggregateRoot::<Counter>::restore(
        TestId::new("s"),
        CounterState { value: i64::MAX },
        Version::new(u64::MAX.checked_sub(1).unwrap()).unwrap(),
    );
    assert!(matches!(
        repo.save(&mut root, &three()).await,
        Err(mnesis_store::StoreError::VersionOverflow)
    ));
    assert_eq!(encoded.load(Ordering::SeqCst), 0);
    assert_eq!(metadata.load(Ordering::SeqCst), 0);
    assert_eq!(root.state().unwrap().value, i64::MAX);
    assert_eq!(
        root.version(),
        Version::new(u64::MAX.checked_sub(1).unwrap())
    );
    assert!(store.read_all(None).await.unwrap().next().await.is_none());
    assert!(wake.now_or_never().is_none());
    let mut fresh = AggregateRoot::<Counter>::new(TestId::new("s"));
    let position = repo.save(&mut fresh, &one()).await.unwrap();
    assert_eq!(position.as_u64(), 1);
    assert_eq!(encoded.load(Ordering::SeqCst), 1);
    assert_eq!(metadata.load(Ordering::SeqCst), 1);
}
