//! Category 3 — defensive boundary: the untrusted read side must reject bytes
//! that violate the crypto invariants, even when they come straight off the
//! store, and the write side must reject a non-owner command.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: unwrap/expect/panic document setup invariants and assertions"
)]

use ed25519_dalek::SigningKey;
use futures::StreamExt;
use mnesis_example_signed_events::domain::{
    Incept, RegisterError, RegisterEvent, RegisterId, SignedRegister, SubmitSet,
};
use mnesis_example_signed_events::projection::{RegisterProjector, ViewError};
use mnesis_fjall::FjallStore;
use mnesis_store::store::{RawEventStore, Store};
use mnesis_store::{CommandRepository, ExecuteError, Projector};
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

/// Read the whole `$all` stream back as `(RegisterId, event)` pairs, routing on
/// the store's `StreamKey` tag (#333) — decoding, not trusting.
async fn read_all_events(store: &Store<FjallStore>) -> Vec<(RegisterId, RegisterEvent)> {
    let mut all = store.read_all(None).await.unwrap();
    let mut out = Vec::new();
    while let Some(item) = all.next().await {
        let (_position, key, env) = item.unwrap();
        let id = RegisterId::from_key_bytes(key.as_bytes()).unwrap();
        let event: RegisterEvent = serde_json::from_slice(env.payload()).unwrap();
        out.push((id, event));
    }
    out
}

#[tokio::test]
async fn read_side_folds_genuine_events_but_rejects_a_tampered_one() {
    let (_dir, store) = open();
    let repo = store.repository::<SignedRegister>().json().build();
    let signing_key = SigningKey::generate(&mut OsRng);
    let id = RegisterId::from_pubkey(&signing_key.verifying_key().to_bytes());

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
                signing_key,
            },
        )
        .await
        .unwrap();

    let events = read_all_events(&store).await;
    assert_eq!(events.len(), 2);

    // Genuine bytes fold cleanly.
    let projector = RegisterProjector;
    let mut view = projector.initial();
    for (route, event) in &events {
        view.route_to(*route);
        view = projector
            .apply(view, event)
            .expect("genuine event verifies");
    }
    assert_eq!(
        view.entries_of(&id).and_then(|e| e.get("a")),
        Some(&"1".to_owned())
    );

    // Now tamper the persisted Set's signature and re-fold from scratch: the
    // read side must reject it rather than trust the store's bytes.
    let mut tampered_events = events;
    if let (_id, RegisterEvent::Set { sig, .. }) = &mut tampered_events[1] {
        sig[0] ^= 0xff;
    } else {
        panic!("event #1 should be a Set");
    }
    let mut fresh = projector.initial();
    fresh.route_to(tampered_events[0].0);
    fresh = projector
        .apply(fresh, &tampered_events[0].1)
        .expect("inception still folds");
    assert!(fresh.registers.contains_key(&id), "inception folded");
    fresh.route_to(tampered_events[1].0);
    assert_eq!(
        projector.apply(fresh, &tampered_events[1].1).unwrap_err(),
        ViewError::BadSignature,
        "a tampered signature is rejected on the read side"
    );
}

#[tokio::test]
async fn a_non_owner_command_is_rejected_at_decide() {
    let (_dir, store) = open();
    let repo = store.repository::<SignedRegister>().json().build();
    let owner_key = SigningKey::generate(&mut OsRng);
    let id = RegisterId::from_pubkey(&owner_key.verifying_key().to_bytes());

    let mut root = SignedRegister::new(id);
    let _ = repo
        .execute(
            &mut root,
            Incept {
                signing_key: owner_key,
            },
        )
        .await
        .unwrap();

    // A different key cannot author a Set on this register.
    let attacker = SigningKey::generate(&mut OsRng);
    let outcome = repo
        .execute(
            &mut root,
            SubmitSet {
                key: "a".to_owned(),
                val: "x".to_owned(),
                signing_key: attacker,
            },
        )
        .await;
    assert!(
        matches!(
            outcome,
            Err(ExecuteError::Decide(RegisterError::Unauthorized))
        ),
        "non-owner Set must be rejected as Unauthorized at decide, nothing persisted"
    );

    // And nothing was appended — the register is still at version 1.
    let reloaded = mnesis_store::Repository::load(&repo, id).await.unwrap();
    assert_eq!(reloaded.version(), mnesis::Version::new(1));
}
