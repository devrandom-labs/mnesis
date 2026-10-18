//! Signing-format regressions use the real handlers and re-verifying projector.
#![allow(clippy::unwrap_used, reason = "exact signing regression assertions")]

use ed25519_dalek::{Signature, Signer, SigningKey};
use futures::StreamExt;
use mnesis::{Version, events};
use mnesis_example_signed_events::domain::{
    Incept, RegisterEvent, RegisterId, SignatureVersion, SignatureVersionError, SignedRegister,
    SubmitSet, event_digest, set_preimage,
};
use mnesis_example_signed_events::projection::{RegisterProjector, ViewError};
use mnesis_fjall::FjallStore;
use mnesis_store::{
    PendingBatch, Projector, RawEventStore, Repository, StoreError, StreamKey, pending_envelope,
};
use proptest::prelude::{ProptestConfig, any, proptest};
use proptest::test_runner::RngSeed;
use serde::Deserialize;

#[derive(Deserialize)]
struct FrozenHistory {
    events: Vec<RegisterEvent>,
    last_digest: [u8; 32],
}

#[test]
fn handlers_and_projector_match_the_frozen_v2_history() {
    let frozen: FrozenHistory =
        serde_json::from_str(include_str!("fixtures/signing-v2.json")).unwrap();
    let signing = SigningKey::from_bytes(&[7; 32]);
    let id = RegisterId::from_pubkey(&signing.verifying_key().to_bytes());
    let mut root = SignedRegister::new(id);
    let genesis = root
        .handle(Incept {
            signing_key: signing.clone(),
        })
        .unwrap()
        .unwrap();
    assert_eq!(genesis.first(), &frozen.events[0]);
    root.commit_persisted(&genesis).unwrap();
    for ((key, val), expected) in [("a\0b", "c"), ("", "雪\0é")]
        .into_iter()
        .zip(&frozen.events[1..])
    {
        let decided = root
            .handle(SubmitSet {
                key: key.to_owned(),
                val: val.to_owned(),
                signing_key: signing.clone(),
            })
            .unwrap()
            .unwrap();
        assert_eq!(decided.first(), expected);
        root.commit_persisted(&decided).unwrap();
    }
    assert_eq!(frozen.events.len(), 3);
    assert_eq!(root.state().unwrap().last_digest, Some(frozen.last_digest));
    assert_eq!(
        event_digest(frozen.events.last().unwrap()),
        frozen.last_digest
    );
    let stream = StreamKey::from_slice(id.as_ref());
    let projector = RegisterProjector;
    let verified = frozen
        .events
        .iter()
        .try_fold(projector.initial(), |view, event| {
            projector.apply_attributed(view, Some(&stream), event)
        })
        .unwrap();
    assert_eq!(
        verified.entries_of(&id).unwrap(),
        &root.state().unwrap().entries
    );
    assert_eq!(root.state().unwrap().entries.len(), 2);
    assert_eq!(
        root.state().unwrap().entries.get(""),
        Some(&"雪\0é".to_owned())
    );
    assert_eq!(
        root.state().unwrap().entries.get("a\0b"),
        Some(&"c".to_owned())
    );
    let encoded = serde_json::to_vec(&frozen.events).unwrap();
    assert_eq!(
        serde_json::from_slice::<Vec<RegisterEvent>>(&encoded).unwrap(),
        frozen.events
    );
}

#[test]
fn unsupported_or_missing_versions_never_fall_back() {
    assert_eq!(SignatureVersion::try_from(2), Ok(SignatureVersion::V2));
    for actual in [0, 1, 3, u8::MAX] {
        assert_eq!(
            SignatureVersion::try_from(actual),
            Err(SignatureVersionError { actual })
        );
    }
    assert_eq!(serde_json::to_string(&SignatureVersion::V2).unwrap(), "2");
    for input in ["0", "1", "3", "255", "256", "-1", "2.5", "\"2\"", "null"] {
        assert_eq!(
            serde_json::from_str::<SignatureVersion>(input)
                .unwrap_err()
                .classify(),
            serde_json::error::Category::Data
        );
    }
    let frozen: FrozenHistory =
        serde_json::from_str(include_str!("fixtures/signing-v2.json")).unwrap();
    for event in &frozen.events {
        let mut value = serde_json::to_value(event).unwrap();
        let fields = value
            .as_object_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap()
            .as_object_mut()
            .unwrap();
        fields.remove("signature_version");
        assert_eq!(
            serde_json::from_value::<RegisterEvent>(value.clone())
                .unwrap_err()
                .classify(),
            serde_json::error::Category::Data
        );
        for version in [1, 3, 255] {
            let mut changed = value.clone();
            changed
                .as_object_mut()
                .unwrap()
                .values_mut()
                .next()
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert("signature_version".to_owned(), version.into());
            assert_eq!(
                serde_json::from_value::<RegisterEvent>(changed)
                    .unwrap_err()
                    .classify(),
                serde_json::error::Category::Data
            );
        }
    }
}

#[test]
fn relabeling_a_legacy_signature_does_not_authorize_v2() {
    let mut legacy: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/legacy-v1.json")).unwrap();
    for record in &legacy {
        assert_eq!(
            serde_json::from_value::<RegisterEvent>(record.clone())
                .unwrap_err()
                .classify(),
            serde_json::error::Category::Data
        );
    }
    legacy[0]["Inception"]
        .as_object_mut()
        .unwrap()
        .insert("signature_version".to_owned(), 2.into());
    let event: RegisterEvent = serde_json::from_value(legacy.remove(0)).unwrap();
    let signing = SigningKey::from_bytes(&[7; 32]);
    let id = RegisterId::from_pubkey(&signing.verifying_key().to_bytes());
    let stream = StreamKey::from_slice(id.as_ref());
    let projector = RegisterProjector;
    assert_eq!(
        projector
            .apply_attributed(projector.initial(), Some(&stream), &event)
            .err()
            .unwrap(),
        ViewError::BadSignature
    );
}

fn assert_fields_are_bound(key: &str, val: &str) {
    let signing = SigningKey::from_bytes(&[7; 32]);
    let prior = [11; 32];
    let original = set_preimage(SignatureVersion::V2, key, val, &prior);
    let signature = signing.sign(&original);
    let verifier = signing.verifying_key();
    verifier.verify_strict(&original, &signature).unwrap();
    let changed_key = format!("{key}\0");
    let changed_val = format!("{val}\0");
    for changed in [
        set_preimage(SignatureVersion::V2, &changed_key, val, &prior),
        set_preimage(SignatureVersion::V2, key, &changed_val, &prior),
        set_preimage(SignatureVersion::V2, key, val, &[12; 32]),
    ] {
        assert_ne!(original, changed);
        assert!(verifier.verify_strict(&changed, &signature).is_err());
    }
}

#[test]
fn empty_unicode_nul_and_hash_block_boundaries_bind_each_field() {
    for (key, val) in [("", ""), ("", "雪"), ("é\0", "雪\0é"), ("a\0b", "c")] {
        assert_fields_are_bound(key, val);
    }
    // These are tested byte lengths, not a claimed storage or string limit.
    for length in [0, 1, 63, 64, 65, 1023, 1024, 1025, 65535, 65536, 65537] {
        let field = "a".repeat(length);
        assert_fields_are_bound(&field, &field);
    }
    let signing = SigningKey::from_bytes(&[7; 32]);
    let first = set_preimage(SignatureVersion::V2, "a\0b", "c", &[11; 32]);
    let second = set_preimage(SignatureVersion::V2, "a", "b\0c", &[11; 32]);
    let signature = Signature::from_bytes(&signing.sign(&first).to_bytes());
    signing
        .verifying_key()
        .verify_strict(&first, &signature)
        .unwrap();
    assert!(
        signing
            .verifying_key()
            .verify_strict(&second, &signature)
            .is_err()
    );
}

proptest! {
    #![proptest_config(ProptestConfig { rng_seed: RngSeed::Fixed(13), ..ProptestConfig::default() })]
    #[test]
    fn arbitrary_unicode_fields_bind_each_component(
        key in proptest::collection::vec(any::<char>(), 0..128),
        val in proptest::collection::vec(any::<char>(), 0..128),
    ) {
        let key_text: String = key.into_iter().collect();
        let val_text: String = val.into_iter().collect();
        assert_fields_are_bound(&key_text, &val_text);
    }
}

#[tokio::test]
async fn legacy_rows_survive_reopen_and_rejected_typed_load_unchanged() {
    let legacy: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/legacy-v1.json")).unwrap();
    let payloads: Vec<Vec<u8>> = legacy
        .iter()
        .map(|record| serde_json::to_vec(record).unwrap())
        .collect();
    let signing = SigningKey::from_bytes(&[7; 32]);
    let id = RegisterId::from_pubkey(&signing.verifying_key().to_bytes());
    let stream_key = StreamKey::from_slice(id.as_ref());
    let rows = [
        pending_envelope(Version::INITIAL)
            .event_type("Inception")
            .payload(payloads[0].clone())
            .build()
            .unwrap(),
        pending_envelope(Version::new(2).unwrap())
            .event_type("Set")
            .payload(payloads[1].clone())
            .build()
            .unwrap(),
    ];
    let directory = tempfile::tempdir().unwrap();
    let original = FjallStore::builder(directory.path())
        .open()
        .unwrap()
        .into_store();
    original
        .append(&stream_key, None, PendingBatch::new(&rows).unwrap())
        .await
        .unwrap();
    let first_shutdown = original.raw().clone();
    drop(original);
    first_shutdown.close().await.unwrap();
    let reopened = FjallStore::builder(directory.path())
        .open()
        .unwrap()
        .into_store();
    let repository = reopened.repository::<SignedRegister>().json().build();
    let error = repository.load(id).await.err().unwrap();
    let StoreError::Decode(decode_error) = error else {
        unreachable!("legacy data must fail decoding");
    };
    assert_eq!(decode_error.classify(), serde_json::error::Category::Data);
    let mut raw_rows = reopened
        .read_stream(&stream_key, Version::INITIAL)
        .await
        .unwrap();
    for (index, expected) in payloads.iter().enumerate() {
        let actual = raw_rows.next().await.unwrap().unwrap();
        assert_eq!(actual.payload(), expected.as_slice());
        assert_eq!(
            actual.version(),
            Version::new(u64::try_from(index).unwrap().checked_add(1).unwrap()).unwrap()
        );
    }
    assert!(raw_rows.next().await.is_none());
    drop(raw_rows);
    let last_shutdown = reopened.raw().clone();
    drop(repository);
    drop(reopened);
    last_shutdown.close().await.unwrap();
}

#[test]
fn a_signature_cannot_be_reused_for_the_colliding_key_value_pair() {
    let signing = SigningKey::from_bytes(&[7; 32]);
    let id = RegisterId::from_pubkey(&signing.verifying_key().to_bytes());
    let stream = StreamKey::from_slice(id.as_ref());
    let mut root = SignedRegister::new(id);
    let genesis = root
        .handle(Incept {
            signing_key: signing.clone(),
        })
        .unwrap()
        .unwrap()
        .first()
        .clone();
    root.commit_persisted::<0>(&events![genesis.clone()])
        .unwrap();
    let genuine = root
        .handle(SubmitSet {
            key: "a\0b".to_owned(),
            val: "c".to_owned(),
            signing_key: signing,
        })
        .unwrap()
        .unwrap()
        .first()
        .clone();
    let mut forged = genuine.clone();
    if let RegisterEvent::Set { key, val, .. } = &mut forged {
        *key = "a".to_owned();
        *val = "b\0c".to_owned();
    }
    let projector = RegisterProjector;
    let baseline = projector
        .apply_attributed(projector.initial(), Some(&stream), &genesis)
        .unwrap();
    let error = projector
        .apply_attributed(baseline, Some(&stream), &forged)
        .err()
        .unwrap();
    assert_eq!(error, ViewError::BadSignature);
    let view = projector
        .apply_attributed(projector.initial(), Some(&stream), &genesis)
        .unwrap();
    let verified = projector
        .apply_attributed(view, Some(&stream), &genuine)
        .unwrap();
    assert_eq!(verified.entries_of(&id).unwrap().get("a\0b").unwrap(), "c");
}
