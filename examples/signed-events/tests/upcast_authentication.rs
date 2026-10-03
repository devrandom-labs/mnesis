//! Authenticate the stored payload before rewriting it for a new schema.
#![allow(
    clippy::unwrap_used,
    reason = "exact cryptographic regression assertions"
)]

use std::convert::Infallible;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use mnesis::{Aggregate, AggregateState, DomainEvent, Message, Version};
use mnesis_example_signed_events::domain::RegisterId;
use mnesis_fjall::FjallStore;
use mnesis_store::{
    Decode, Encode, EventMorsel, LoadWithError, PendingBatch, PersistedEnvelope, RawEventStore,
    SchemaVersion, StreamKey, pending_envelope,
};

#[derive(Debug)]
struct Event;
impl Message for Event {}
impl DomainEvent for Event {
    fn name(&self) -> &'static str {
        "Renamed"
    }
}
#[derive(Debug)]
struct State(u32);
impl AggregateState for State {
    type Event = Event;
    fn initial() -> Self {
        Self(0)
    }
    fn apply(self, _: &Event) -> Self {
        Self(self.0.checked_add(1).unwrap())
    }
}
struct Model;
impl Aggregate for Model {
    type Id = RegisterId;
    type State = State;
    type Error = Infallible;
}

struct Codec {
    called: Arc<AtomicBool>,
    key: VerifyingKey,
}
impl Encode<Event> for Codec {
    type Error = Infallible;
    fn encode(&self, _: &Event) -> Result<bytes::Bytes, Infallible> {
        Ok(bytes::Bytes::from_static(b"original"))
    }
}
impl Decode<Event> for Codec {
    type Output<'a> = Event;
    type Error = Infallible;
    fn decode<'a>(&'a self, env: &'a PersistedEnvelope) -> Result<Event, Infallible> {
        assert_eq!(env.version(), Version::INITIAL);
        assert_eq!(env.event_type(), "Renamed");
        assert_eq!(env.schema_version(), 8);
        assert_eq!(env.payload(), b"transformed");
        let signature = Signature::from_slice(env.metadata().unwrap()).unwrap();
        // Retained original metadata explicitly does not authenticate rewritten bytes.
        assert!(self.key.verify_strict(env.payload(), &signature).is_err());
        self.called.store(true, Ordering::SeqCst);
        Ok(Event)
    }
}

async fn append_case(
    store: &mnesis_store::Store<FjallStore>,
    case: u8,
) -> (VerifyingKey, RegisterId) {
    let signing = SigningKey::from_bytes(&[7; 32]);
    let original_key = signing.verifying_key();
    let id = RegisterId::from_pubkey(&original_key.to_bytes());
    let mut signature = signing.sign(b"original").to_bytes();
    if case == 2 {
        signature[0] ^= 1;
    }
    let payload = if case == 1 {
        b"tampered".as_slice()
    } else {
        b"original".as_slice()
    };
    let row = pending_envelope(Version::INITIAL)
        .event_type("Original")
        .payload(payload)
        .schema_version(SchemaVersion::from_u32(7).unwrap())
        .metadata(signature.to_vec())
        .build()
        .unwrap();
    store
        .append(
            &StreamKey::from_slice(id.as_ref()),
            None,
            PendingBatch::new(&[row]).unwrap(),
        )
        .await
        .unwrap();
    (original_key, id)
}

async fn run_case(case: u8) {
    let dir = tempfile::tempdir().unwrap();
    let store = FjallStore::builder(dir.path()).open().unwrap().into_store();
    let (original_key, id) = append_case(&store, case).await;
    let verifier_key = if case == 3 {
        SigningKey::from_bytes(&[8; 32]).verifying_key()
    } else {
        original_key
    };
    let transformed = Arc::new(AtomicBool::new(false));
    let decoded = Arc::new(AtomicBool::new(false));
    let record = Arc::clone(&transformed);
    let repo = store
        .repository::<Model>()
        .codec(Codec {
            called: Arc::clone(&decoded),
            key: original_key,
        })
        .build();
    let result = repo
        .load_with(
            id,
            move |env| {
                assert_eq!(env.event_type(), "Original");
                assert_eq!(env.schema_version(), 7);
                assert_eq!(env.version(), Version::INITIAL);
                let parsed = Signature::from_slice(env.metadata().unwrap())?;
                verifier_key.verify_strict(env.payload(), &parsed)
            },
            move |_| {
                record.store(true, Ordering::SeqCst);
                Ok::<_, Infallible>(EventMorsel::new(
                    "Renamed",
                    SchemaVersion::from_u32(8).unwrap(),
                    b"transformed".to_vec(),
                ))
            },
        )
        .await;
    if case == 0 {
        let root = result.unwrap();
        assert_eq!(root.state().unwrap().0, 1);
        assert_eq!(root.version(), Some(Version::INITIAL));
    } else {
        assert!(matches!(result, Err(LoadWithError::Verification(_))));
    }
    assert_eq!(transformed.load(Ordering::SeqCst), case == 0);
    assert_eq!(decoded.load(Ordering::SeqCst), case == 0);
    let shutdown = store.raw().clone();
    drop(repo);
    drop(store);
    shutdown.close().await.unwrap();
}

#[tokio::test]
async fn ed25519_verification_precedes_upcast_and_rejects_original_tampering() {
    for case in 0..=3 {
        run_case(case).await;
    }
}
