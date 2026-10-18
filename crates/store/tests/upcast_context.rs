//! Decoder context and fail-closed ordering on the real repository read path.
#![allow(clippy::unwrap_used, reason = "exact regression assertions")]

use std::borrow::Cow;
use std::convert::Infallible;
use std::sync::Arc;

use bytes::Bytes;
use mnesis::Version;
use mnesis_inmemory::InMemoryStore;
use mnesis_store::{
    Decode, Encode, EventMorsel, LoadWithError, PendingBatch, PersistedEnvelope, RawEventStore,
    SchemaVersion, Store, StoreError, StreamKey, pending_envelope,
};
use mnesis_test_domains::{Counter, CounterEvent, TestId};
use parking_lot::Mutex;

#[derive(Debug, PartialEq, Eq)]
struct Context {
    version: u64,
    schema: u32,
    name: String,
    payload: Vec<u8>,
    metadata: Option<Vec<u8>>,
    pointer: usize,
}

fn context(env: &PersistedEnvelope) -> Context {
    Context {
        version: env.version().as_u64(),
        schema: env.schema_version(),
        name: env.event_type().to_owned(),
        payload: env.payload().to_vec(),
        metadata: env.metadata().map(<[u8]>::to_vec),
        pointer: env.payload().as_ptr().addr(),
    }
}

struct Observe(Arc<Mutex<Vec<Context>>>);
impl Encode<CounterEvent> for Observe {
    type Error = Infallible;
    fn encode(&self, _: &CounterEvent) -> Result<Bytes, Infallible> {
        Ok(Bytes::from_static(b"original"))
    }
}
impl Decode<CounterEvent> for Observe {
    type Output<'a> = CounterEvent;
    type Error = Infallible;
    fn decode<'a>(&'a self, env: &'a PersistedEnvelope) -> Result<CounterEvent, Infallible> {
        self.0.lock().push(context(env));
        Ok(CounterEvent::Incremented)
    }
}

async fn fixture() -> Store<InMemoryStore> {
    let store = Store::new(InMemoryStore::new());
    let rows: Vec<_> = (1..=2)
        .map(|v| {
            pending_envelope(Version::new(v).unwrap())
                .event_type("Original")
                .payload(b"original".as_slice())
                .schema_version(SchemaVersion::from_u32(7).unwrap())
                .metadata(b"original metadata".as_slice())
                .build()
                .unwrap()
        })
        .collect();
    store
        .append(
            &StreamKey::from_slice(b"s"),
            None,
            PendingBatch::new(&rows).unwrap(),
        )
        .await
        .unwrap();
    store
}

#[tokio::test]
async fn identity_borrowed_and_owned_morsels_reuse_the_original_envelope() {
    for owned in [false, true] {
        let store = fixture().await;
        let verified = Arc::new(Mutex::new(Vec::new()));
        let decoded = Arc::new(Mutex::new(Vec::new()));
        let record = Arc::clone(&verified);
        let repo = store
            .repository::<Counter>()
            .codec(Observe(Arc::clone(&decoded)))
            .build();
        let root = repo
            .load_with(
                TestId::new("s"),
                move |env| {
                    record.lock().push(context(env));
                    Ok::<_, Infallible>(())
                },
                move |m| {
                    Ok::<_, Infallible>(if owned {
                        EventMorsel::new(m.event_type(), m.schema_version(), m.payload().to_vec())
                    } else {
                        m
                    })
                },
            )
            .await
            .unwrap();
        assert_eq!(root.version(), Version::new(2));
        assert_eq!(root.state().unwrap().value, 2);
        assert_eq!(*decoded.lock(), *verified.lock());
        assert_eq!(decoded.lock().len(), 2);
    }
}

#[tokio::test]
async fn transforms_preserve_original_context_and_decode_the_declared_schema() {
    for owned in [false, true] {
        let store = fixture().await;
        let decoded = Arc::new(Mutex::new(Vec::new()));
        let repo = store
            .repository::<Counter>()
            .codec(Observe(Arc::clone(&decoded)))
            .build();
        let root = repo
            .load_with(
                TestId::new("s"),
                |env| {
                    assert_eq!(env.event_type(), "Original");
                    assert_eq!(env.schema_version(), 7);
                    assert_eq!(env.payload(), b"original");
                    Ok::<_, Infallible>(())
                },
                move |m| {
                    let schema = SchemaVersion::from_u32(u32::MAX).unwrap();
                    Ok::<_, Infallible>(if owned {
                        EventMorsel::new("Renamed", schema, b"transformed".to_vec())
                    } else {
                        m.with_event_type(Cow::Borrowed("Renamed"))
                            .with_schema_version(schema)
                            .with_payload(Cow::Borrowed(b"transformed"))
                    })
                },
            )
            .await
            .unwrap();
        assert_eq!(root.version(), Version::new(2));
        assert_eq!(root.state().unwrap().value, 2);
        let actual = decoded.lock();
        assert_eq!(actual.len(), 2);
        for (row, version) in actual.iter().zip(1..=2) {
            assert_eq!(row.version, version);
            assert_eq!(row.schema, u32::MAX);
            assert_eq!(row.name, "Renamed");
            assert_eq!(row.payload, b"transformed");
            assert_eq!(
                row.metadata.as_deref(),
                Some(b"original metadata".as_slice())
            );
            assert_eq!(row.pointer % 16, 0);
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("original authentication failed")]
struct AuthenticationFailure;
#[derive(Debug, thiserror::Error)]
#[error("transformation failed")]
struct TransformationFailure;

#[test]
fn synthetic_state_decode_has_only_documented_placeholder_context() {
    let envelope = PersistedEnvelope::for_decode("State", b"state bytes").unwrap();
    assert_eq!(envelope.version(), Version::INITIAL);
    assert_eq!(envelope.schema_version_value(), SchemaVersion::INITIAL);
    assert_eq!(envelope.event_type(), "State");
    assert_eq!(envelope.payload(), b"state bytes");
    assert_eq!(envelope.metadata(), None);
    assert_eq!(envelope.payload().as_ptr().addr() % 16, 0);
    assert!(SchemaVersion::from_u32(0).is_err());
}

#[tokio::test]
async fn verification_failure_prevents_transform_and_decode() {
    let store = fixture().await;
    let decoded = Arc::new(Mutex::new(Vec::new()));
    let transformed = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&transformed);
    let repo = store
        .repository::<Counter>()
        .codec(Observe(Arc::clone(&decoded)))
        .build();
    let result = repo
        .load_with(
            TestId::new("s"),
            |_| Err(AuthenticationFailure),
            move |m| {
                record.lock().push(m.schema_version());
                Ok::<_, Infallible>(m)
            },
        )
        .await;
    assert!(matches!(
        result,
        Err(LoadWithError::Verification(AuthenticationFailure))
    ));
    assert!(transformed.lock().is_empty());
    assert!(decoded.lock().is_empty());
}

#[tokio::test]
async fn transformation_failure_preserves_its_error_and_prevents_decode() {
    let store = fixture().await;
    let decoded = Arc::new(Mutex::new(Vec::new()));
    let repo = store
        .repository::<Counter>()
        .codec(Observe(Arc::clone(&decoded)))
        .build();
    let result = repo
        .load_with(
            TestId::new("s"),
            |_| Ok::<_, Infallible>(()),
            |_| Err::<EventMorsel<'_>, _>(TransformationFailure),
        )
        .await;
    assert!(matches!(
        result,
        Err(LoadWithError::Upcast(TransformationFailure))
    ));
    assert!(decoded.lock().is_empty());
}

#[tokio::test]
async fn invalid_transformed_type_is_a_synthesis_error_before_decode() {
    let store = fixture().await;
    let decoded = Arc::new(Mutex::new(Vec::new()));
    let repo = store
        .repository::<Counter>()
        .codec(Observe(Arc::clone(&decoded)))
        .build();
    let result = repo
        .load_with(
            TestId::new("s"),
            |_| Ok::<_, Infallible>(()),
            |m| Ok::<_, Infallible>(m.with_event_type(Cow::Owned("x".repeat(65_536)))),
        )
        .await;
    assert!(matches!(
        result,
        Err(LoadWithError::Store(StoreError::EnvelopeSynthesis(_)))
    ));
    assert!(decoded.lock().is_empty());
}
