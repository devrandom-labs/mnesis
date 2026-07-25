//! Category 1 — sequence/protocol: a multi-step incept → set → set interaction
//! persisted through the real store, then reloaded, with the on-disk chain
//! links verified.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: unwrap/expect/panic document setup invariants and assertions"
)]

use ed25519_dalek::SigningKey;
use futures::StreamExt;
use mnesis::Version;
use mnesis_example_signed_events::domain::{
    Incept, RegisterEvent, RegisterId, SignedRegister, SubmitSet, event_digest,
};
use mnesis_fjall::FjallStore;
use mnesis_store::store::{RawEventStore, Store};
use mnesis_store::{CommandRepository, Repository, StreamKey};
use rand_core::OsRng;
use tempfile::TempDir;

fn open() -> (TempDir, Store<FjallStore>) {
    let dir = tempfile::tempdir().unwrap();
    let store = FjallStore::builder(dir.path().join("db"))
        .open()
        .unwrap()
        .into_store();
    (dir, store)
}

fn keypair() -> (SigningKey, RegisterId) {
    let signing_key = SigningKey::generate(&mut OsRng);
    let id = RegisterId::from_pubkey(&signing_key.verifying_key().to_bytes());
    (signing_key, id)
}

#[tokio::test]
async fn incept_then_two_sets_round_trips_and_the_chain_links() {
    let (_dir, store) = open();
    let repo = store.repository::<SignedRegister>().json().build();
    let (signing_key, id) = keypair();

    let mut root = SignedRegister::new(id);
    let _ = repo
        .execute(
            &mut root,
            Incept {
                signing_key: signing_key.clone(),
            },
        )
        .await
        .unwrap();
    let _ = repo
        .execute(
            &mut root,
            SubmitSet {
                key: "a".to_owned(),
                val: "1".to_owned(),
                signing_key: signing_key.clone(),
            },
        )
        .await
        .unwrap();
    let _ = repo
        .execute(
            &mut root,
            SubmitSet {
                key: "b".to_owned(),
                val: "2".to_owned(),
                signing_key,
            },
        )
        .await
        .unwrap();

    // Reload a fresh aggregate and assert folded state + version.
    let loaded = repo.load(id).await.unwrap();
    assert_eq!(loaded.version(), Version::new(3));
    assert_eq!(loaded.state().entries.get("a"), Some(&"1".to_owned()));
    assert_eq!(loaded.state().entries.get("b"), Some(&"2".to_owned()));

    // Read the raw persisted stream and assert every Set points at the digest
    // of the event immediately before it — the hash chain is intact on disk.
    let mut stream = store
        .read_stream(&StreamKey::from_slice(id.as_ref()), Version::INITIAL)
        .await
        .unwrap();
    let mut events = Vec::new();
    while let Some(item) = stream.next().await {
        let env = item.unwrap();
        events.push(serde_json::from_slice::<RegisterEvent>(env.payload()).unwrap());
    }
    assert_eq!(events.len(), 3, "three events persisted");
    assert!(matches!(events[0], RegisterEvent::Inception { .. }));

    let digest_of_0 = event_digest(&events[0]);
    let digest_of_1 = event_digest(&events[1]);
    match &events[1] {
        RegisterEvent::Set { prior_digest, .. } => {
            assert_eq!(*prior_digest, digest_of_0, "set#1 chains to inception");
        }
        incept @ RegisterEvent::Inception { .. } => panic!("expected Set, got {incept:?}"),
    }
    match &events[2] {
        RegisterEvent::Set { prior_digest, .. } => {
            assert_eq!(*prior_digest, digest_of_1, "set#2 chains to set#1");
        }
        incept @ RegisterEvent::Inception { .. } => panic!("expected Set, got {incept:?}"),
    }
}
