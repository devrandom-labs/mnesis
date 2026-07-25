//! Category 4 — linearizability/isolation: two concurrent writers submit a
//! `Set` on the same register at the same expected version. Exactly one commits;
//! the other is rejected with an optimistic-concurrency conflict, and the final
//! chain is single-threaded and unbroken.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: unwrap/expect/panic document setup invariants and assertions"
)]

use std::sync::Arc;

use ed25519_dalek::SigningKey;
use mnesis::Version;
use mnesis_example_signed_events::domain::{Incept, RegisterId, SignedRegister, SubmitSet};
use mnesis_fjall::FjallStore;
use mnesis_store::store::RawEventStore;
use mnesis_store::{CommandRepository, Repository};
use rand_core::OsRng;
use tokio::sync::Barrier;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_sets_conflict_and_leave_one_unbroken_chain() {
    let dir = tempfile::tempdir().unwrap();
    let store = FjallStore::builder(dir.path().join("db"))
        .open()
        .unwrap()
        .into_store();
    let repo = Arc::new(store.repository::<SignedRegister>().json().build());

    let signing_key = Arc::new(SigningKey::generate(&mut OsRng));
    let id = RegisterId::from_pubkey(&signing_key.verifying_key().to_bytes());

    // Incept first so both writers race the *same* v1 → v2 transition.
    {
        let mut root = SignedRegister::new(id);
        let _ = repo
            .execute(
                &mut root,
                Incept {
                    signing_key: (*signing_key).clone(),
                },
            )
            .await
            .unwrap();
    }

    // Both writers load the aggregate at version 1.
    let mut root_a = repo.load(id).await.unwrap();
    let mut root_b = repo.load(id).await.unwrap();
    assert_eq!(root_a.version(), Version::new(1));
    assert_eq!(root_b.version(), Version::new(1));

    let barrier = Arc::new(Barrier::new(2));

    let repo_a = Arc::clone(&repo);
    let key_a = Arc::clone(&signing_key);
    let barrier_a = Arc::clone(&barrier);
    let writer_a = tokio::spawn(async move {
        barrier_a.wait().await;
        repo_a
            .execute(
                &mut root_a,
                SubmitSet {
                    key: "k".to_owned(),
                    val: "a".to_owned(),
                    signing_key: (*key_a).clone(),
                },
            )
            .await
    });

    let repo_b = Arc::clone(&repo);
    let key_b = Arc::clone(&signing_key);
    let barrier_b = Arc::clone(&barrier);
    let writer_b = tokio::spawn(async move {
        barrier_b.wait().await;
        repo_b
            .execute(
                &mut root_b,
                SubmitSet {
                    key: "k".to_owned(),
                    val: "b".to_owned(),
                    signing_key: (*key_b).clone(),
                },
            )
            .await
    });

    let results = [writer_a.await.unwrap(), writer_b.await.unwrap()];
    let committed = results.iter().filter(|r| r.is_ok()).count();
    let conflicted = results
        .iter()
        .filter(|r| matches!(r, Err(e) if e.is_conflict()))
        .count();
    assert_eq!(committed, 1, "exactly one writer commits");
    assert_eq!(
        conflicted, 1,
        "exactly one writer sees an optimistic conflict"
    );

    // The stream advanced by exactly one version, holding exactly one entry —
    // the chain is single-threaded and unbroken.
    let final_root = repo.load(id).await.unwrap();
    assert_eq!(final_root.version(), Version::new(2), "one commit landed");
    assert_eq!(final_root.state().entries.len(), 1, "only the winner's set");
    assert!(final_root.state().last_digest.is_some());
}
