#![allow(clippy::unwrap_used, reason = "tests")]
#![allow(clippy::panic, reason = "tests")]

use mnesis::Version;
use mnesis_inmemory::InMemoryStore;
use mnesis_store::AppendError;
use mnesis_store::StreamKey;
use mnesis_store::pending_envelope;
use mnesis_store::store::RawEventStore;

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
        .append(&long, Version::new(1), &[env])
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
