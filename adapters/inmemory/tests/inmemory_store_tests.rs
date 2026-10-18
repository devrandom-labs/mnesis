#![allow(clippy::unwrap_used, reason = "tests")]
#![allow(clippy::panic, reason = "tests")]

use futures::FutureExt;
use mnesis::Version;
use mnesis_inmemory::{InMemoryAllPos, InMemoryStore};
use mnesis_store::AppendError;
use mnesis_store::StreamKey;
use mnesis_store::import::{AtomicAppend, AtomicAppendError, InvalidRun, PlannedAppend};
use mnesis_store::pending_envelope;
use mnesis_store::store::RawEventStore;
use mnesis_store::wake::{WakeRegistration, WakeSource};
use mnesis_store::{PendingBatch, PendingEnvelope};

#[tokio::test]
async fn append_conflict_truncates_overlong_stream_id_with_ellipsis() {
    let store = InMemoryStore::new();
    // An overlong id so the conflict label exceeds the 64-byte `ErrorId` cap.
    let long = StreamKey::from_slice("y".repeat(200).as_bytes());
    let env = pending_envelope(Version::new(1).unwrap())
        .event_type("E")
        .payload(b"p".to_vec())
        .build()
        .unwrap();
    // New stream + Some(expected) → conflict carrying the truncated id label.
    let err = store
        .append(&long, Version::new(1), PendingBatch::of(&env))
        .await
        .unwrap_err();
    match err {
        AppendError::Conflict { stream_id, .. } => {
            assert!(stream_id.as_str().len() <= 64);
            assert!(
                stream_id.as_str().ends_with('…'),
                "overlong stream id must be truncated with an ellipsis, got {stream_id:?}"
            );
        }
        // AppendError is #[non_exhaustive] (#209): Store and any future variant
        // collapse into the catch-all — only Conflict is expected here.
        other => panic!("expected Conflict, got: {other}"),
    }
}

fn envelope(version: Version, payload: &'static [u8]) -> PendingEnvelope {
    pending_envelope(version)
        .event_type("E")
        .payload(payload)
        .build()
        .unwrap()
}

#[tokio::test]
async fn duplicate_targets_preserve_counter_and_emit_no_wake() {
    let store = InMemoryStore::new();
    let target = StreamKey::from_slice(b"target");
    let original = pending_envelope(Version::INITIAL)
        .event_type("E")
        .payload(b"old".as_slice())
        .build()
        .unwrap();
    store
        .append(&target, None, PendingBatch::of(&original))
        .await
        .unwrap();
    for version in [1u64, 3, 5] {
        let target_registration = store.register(Some(target.as_ref())).unwrap();
        let global_registration = store.register(None).unwrap();
        let target_wait = target_registration.arm();
        let global_wait = global_registration.arm();
        let writes = [
            PlannedAppend {
                target: target.clone(),
                expected_version: Version::new(1),
                head: pending_envelope(Version::new(2).unwrap())
                    .event_type("E")
                    .payload(b"new".as_slice())
                    .build()
                    .unwrap(),
                tail: vec![],
            },
            PlannedAppend {
                target: target.clone(),
                expected_version: Version::new(version.checked_sub(1).unwrap()),
                head: pending_envelope(Version::new(version).unwrap())
                    .event_type("E")
                    .payload(b"bad".as_slice())
                    .build()
                    .unwrap(),
                tail: vec![],
            },
        ];
        let failure = store.atomic_append_many(&writes).await.unwrap_err();
        assert!(
            matches!(failure, AtomicAppendError::InvalidRoute(error) if error.target == target && error.first_index == 0 && error.index == 1)
        );
        assert!(target_wait.now_or_never().is_none());
        assert!(global_wait.now_or_never().is_none());
    }
    let target_registration = store.register(Some(target.as_ref())).unwrap();
    let global_registration = store.register(None).unwrap();
    let target_wait = target_registration.arm();
    let global_wait = global_registration.arm();
    let malformed = PlannedAppend {
        target: target.clone(),
        expected_version: Version::new(1),
        head: envelope(Version::new(2).unwrap(), b"two"),
        tail: vec![envelope(Version::new(4).unwrap(), b"four")],
    };
    let error = store.atomic_append_many(&[malformed]).await.unwrap_err();
    assert!(
        matches!(error, AtomicAppendError::InvalidRun(InvalidRun::NonSequential { index: 0, expected, actual })
        if expected == Version::new(3).unwrap() && actual == Version::new(4).unwrap())
    );
    assert!(target_wait.now_or_never().is_none());
    assert!(global_wait.now_or_never().is_none());
    let next = pending_envelope(Version::new(2).unwrap())
        .event_type("E")
        .payload(b"next".as_slice())
        .build()
        .unwrap();
    assert_eq!(
        store
            .append(&target, Version::new(1), PendingBatch::of(&next))
            .await
            .unwrap(),
        InMemoryAllPos::new(2).unwrap()
    );
}
