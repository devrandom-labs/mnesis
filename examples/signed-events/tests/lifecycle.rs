//! Category 2 — lifecycle: write a signed chain, close the store, reopen the
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

    // First open: incept + two sets, then await complete engine shutdown.
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
        for (key, val) in [("a", "1"), ("b", "2")] {
            let _ = repo
                .execute(
                    &mut root,
                    SubmitSet {
                        key: key.to_owned(),
                        val: val.to_owned(),
                        signing_key: signing_key.clone(),
                    },
                )
                .await
                .unwrap();
        }
        let head = root
            .state()
            .unwrap()
            .last_digest
            .expect("chain head after inception+sets");
        let shutdown = store.raw().clone();
        drop(repo);
        drop(store);
        shutdown.close().await.unwrap();
        head
    };

    // Second open: the state and chain head must resume from disk.
    let store = reopen(&path);
    let repo = store.repository::<SignedRegister>().json().build();
    let mut root = repo.load(id).await.unwrap();
    assert_eq!(root.version(), Version::new(3), "version resumes at 3");
    let resumed = root.state().unwrap();
    assert_eq!(resumed.entries.get("a"), Some(&"1".to_owned()));
    assert_eq!(resumed.entries.get("b"), Some(&"2".to_owned()));
    assert_eq!(
        resumed.last_digest,
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
    let Execution::Executed { events, .. } = exec else {
        panic!("SubmitSet must record an event");
    };
    let RegisterEvent::Set { prior_digest, .. } = events.first() else {
        panic!("expected Set, got {:?}", events.first());
    };
    assert_eq!(
        *prior_digest, head_after_first,
        "the post-reopen Set chains onto the resumed head"
    );
    assert_eq!(root.version(), Version::new(4), "version advanced to 4");
    let shutdown = store.raw().clone();
    drop(repo);
    drop(store);
    shutdown.close().await.unwrap();
}
