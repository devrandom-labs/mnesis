//! Runnable demo: incept two signed registers, apply a few signed sets, persist
//! them through a real on-disk [`FjallStore`], load one back through the typed
//! repository facade, then fold the whole `$all` stream through the untrusted,
//! re-verifying [`RegisterProjector`].
//!
//! Keys are built from fixed seeds via `SigningKey::from_bytes` (no RNG), so the
//! binary needs no `rand_core` — key *generation* is a test-only concern.

// Example binary: it narrates the lifecycle to stdout.
#![allow(clippy::print_stdout, reason = "example binary narrates to stdout")]

use std::error::Error;

use ed25519_dalek::SigningKey;
use futures::StreamExt;
use mnesis_example_signed_events::domain::{
    Incept, RegisterEvent, RegisterId, SignedRegister, SubmitSet,
};
use mnesis_example_signed_events::projection::RegisterProjector;
use mnesis_fjall::FjallStore;
use mnesis_store::store::RawEventStore;
use mnesis_store::{CommandRepository, Execution, Projector, Repository};

type BoxError = Box<dyn Error + Send + Sync>;

#[tokio::main]
#[allow(
    clippy::too_many_lines,
    reason = "linear end-to-end demo script; splitting it hurts readability"
)]
async fn main() -> Result<(), BoxError> {
    let dir = tempfile::tempdir()?;
    let store = FjallStore::builder(dir.path().join("db"))
        .open()?
        .into_store();
    let repo = store.repository::<SignedRegister>().json().build();

    // Two registers, each owned by a distinct key (fixed seeds for the demo).
    let alice = SigningKey::from_bytes(&[1u8; 32]);
    let bob = SigningKey::from_bytes(&[2u8; 32]);
    let alice_id = RegisterId::from_pubkey(&alice.verifying_key().to_bytes());
    let bob_id = RegisterId::from_pubkey(&bob.verifying_key().to_bytes());

    println!("== write path (typed facade, signed + chained) ==");
    let mut alice_root = SignedRegister::new(alice_id);
    let _ = repo
        .execute(
            &mut alice_root,
            Incept {
                signing_key: alice.clone(),
            },
        )
        .await?;
    let _ = repo
        .execute(
            &mut alice_root,
            SubmitSet {
                key: "city".to_owned(),
                val: "lisbon".to_owned(),
                signing_key: alice.clone(),
            },
        )
        .await?;
    let last = repo
        .execute(
            &mut alice_root,
            SubmitSet {
                key: "lang".to_owned(),
                val: "pt".to_owned(),
                signing_key: alice.clone(),
            },
        )
        .await?;
    if let Execution::Executed { position, .. } = last {
        println!("  alice's last write landed at $all position {position:?}");
    }

    let mut bob_root = SignedRegister::new(bob_id);
    let _ = repo
        .execute(
            &mut bob_root,
            Incept {
                signing_key: bob.clone(),
            },
        )
        .await?;
    let _ = repo
        .execute(
            &mut bob_root,
            SubmitSet {
                key: "role".to_owned(),
                val: "maintainer".to_owned(),
                signing_key: bob.clone(),
            },
        )
        .await?;

    println!("\n== load path (rehydrate through the facade) ==");
    let loaded = repo.load(alice_id).await?;
    println!("  register {alice_id}");
    println!("  version {:?}", loaded.version());
    let mut entries: Vec<_> = loaded.state().entries.iter().collect();
    entries.sort();
    for (key, val) in entries {
        println!("    {key} = {val}");
    }

    println!("\n== untrusted read side ($all, re-verified) ==");
    let projector = RegisterProjector;
    let mut view = projector.initial();
    let mut all = store.read_all(None).await?;
    while let Some(item) = all.next().await {
        let (_position, stream_key, envelope) = item?;
        let event: RegisterEvent = serde_json::from_slice(envelope.payload())?;
        // A forged or tampered event would surface here as an Err instead of
        // being trusted — that is the whole point of the read-side re-check.
        view = projector.apply_attributed(view, Some(&stream_key), &event)?;
    }
    println!(
        "  re-verified {} register(s) off $all",
        view.registers.len()
    );
    for (id, kv) in &view.registers {
        println!("    {id} → {} verified entr(y/ies)", kv.len());
    }

    Ok(())
}
