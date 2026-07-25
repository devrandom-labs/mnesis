//! Category 2 — lifecycle: write a signed chain, drop the store, reopen the
//! same on-disk keyspace, and confirm the folded state and chain head resume,
//! then append another `Set` that continues the chain unbroken.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: unwrap/expect/panic document setup invariants and assertions"
)]

use std::path::Path;

use ed25519_dalek::SigningKey;
use mnesis::Version;
use mnesis_example_signed_events::domain::{
    Incept, RegisterEvent, RegisterId, SignedRegister, SubmitSet,
};
use mnesis_fjall::FjallStore;
use mnesis_store::store::{RawEventStore, Store};
use mnesis_store::{CommandRepository, Execution, Repository};
use rand_core::OsRng;

fn reopen(path: &Path) -> Store<FjallStore> {
    FjallStore::builder(path).open().unwrap().into_store()
}

#[tokio::test]
async fn reopen_resumes_the_chain_and_continues_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");

    let signing_key = SigningKey::generate(&mut OsRng);
    let id = RegisterId::from_pubkey(&signing_key.verifying_key().to_bytes());

    // First open: incept + two sets, then drop every handle to release the lock.
    let head_after_first = {
        let store = reopen(&path);
        let repo = store.repository::<SignedRegister>().json().build();
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
                    signing_key: signing_key.clone(),
                },
            )
            .await
            .unwrap();
        root.state()
            .last_digest
            .expect("chain head after inception+sets")
    };

    // Second open: the state and chain head must resume from disk.
    let store = reopen(&path);
    let repo = store.repository::<SignedRegister>().json().build();
    let mut root = repo.load(id).await.unwrap();
    assert_eq!(root.version(), Version::new(3), "version resumes at 3");
    assert_eq!(root.state().entries.get("a"), Some(&"1".to_owned()));
    assert_eq!(root.state().entries.get("b"), Some(&"2".to_owned()));
    assert_eq!(
        root.state().last_digest,
        Some(head_after_first),
        "chain head resumes to the exact pre-reopen digest"
    );

    // Appending after a reopen continues the chain from the resumed head.
    let exec = repo
        .execute(
            &mut root,
            SubmitSet {
                key: "c".to_owned(),
                val: "3".to_owned(),
                signing_key,
            },
        )
        .await
        .unwrap();
    match exec {
        Execution::Executed { events, .. } => match events.first() {
            RegisterEvent::Set { prior_digest, .. } => {
                assert_eq!(
                    Some(*prior_digest),
                    Some(head_after_first),
                    "the post-reopen Set chains onto the resumed head"
                );
            }
            incept @ RegisterEvent::Inception { .. } => panic!("expected Set, got {incept:?}"),
        },
        Execution::Ignored => panic!("SubmitSet must record an event"),
    }
    assert_eq!(root.version(), Version::new(4), "version advanced to 4");
}
