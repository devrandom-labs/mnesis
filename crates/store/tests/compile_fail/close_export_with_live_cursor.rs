use futures::StreamExt;
use mnesis::Version;
use mnesis_inmemory::InMemoryStore;
use mnesis_store::export::{ConsistentExporter, ExportSession};
use mnesis_store::StreamKey;
use std::time::Duration;

async fn close_with_live_cursor(store: &InMemoryStore) {
    let session = store.open_export_session(Duration::from_secs(60)).await.unwrap();
    let id = StreamKey::from_slice(b"a");
    let mut cursor = session.export_stream(&id, Version::INITIAL).await.unwrap();
    session.close().await.unwrap();
    let _ = cursor.next().await;
}

fn main() {}
