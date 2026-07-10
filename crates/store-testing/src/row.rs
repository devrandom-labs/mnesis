//! Shared test-data row, id type, and drive/drain helpers used by every
//! conformance module.

use core::fmt;

use bytes::Bytes;
use futures::StreamExt;
use futures::pin_mut;
use nexus::Version;
use nexus_store::StreamKey;
use nexus_store::envelope::{PendingEnvelope, PersistedEnvelope, pending_envelope};
use nexus_store::store::RawEventStore;
use nexus_store::value::SchemaVersion;

/// One row of test data fed into an adapter for the conformance suite to
/// observe back out. All fields must round-trip byte-for-byte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConformanceRow {
    pub version: u64,
    pub event_type: String,
    pub schema_version: u32,
    pub payload: Vec<u8>,
    pub metadata: Option<Vec<u8>>,
}

impl ConformanceRow {
    /// Convenience constructor: `schema_version = 1`, no metadata.
    #[must_use]
    pub fn new(version: u64, event_type: &str, payload: Vec<u8>) -> Self {
        Self {
            version,
            event_type: event_type.to_owned(),
            schema_version: 1,
            payload,
            metadata: None,
        }
    }

    /// Set the schema version (defaults to 1).
    #[must_use]
    pub fn with_schema_version(mut self, schema_version: u32) -> Self {
        self.schema_version = schema_version;
        self
    }

    /// Attach metadata (defaults to absent).
    #[must_use]
    pub fn with_metadata(mut self, metadata: Vec<u8>) -> Self {
        self.metadata = Some(metadata);
        self
    }
}

/// Subscription/snapshot id: satisfies the `Id` blanket bounds.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct SubId(String);

impl SubId {
    #[must_use]
    pub fn new(s: &str) -> Self {
        Self(s.to_owned())
    }

    /// The `StreamKey` carrying the same bytes — for driving `append` on the
    /// stream this id subscribes to.
    #[must_use]
    pub fn key(&self) -> StreamKey {
        StreamKey::from_slice(self.0.as_bytes())
    }
}

impl fmt::Display for SubId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<[u8]> for SubId {
    fn as_ref(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

/// Build the `PendingEnvelope` a row describes. Panics on invalid rows — the
/// suite only constructs valid ones.
#[must_use]
pub fn envelope_for(row: &ConformanceRow) -> PendingEnvelope {
    let version = Version::new(row.version).expect("row version must be >= 1");
    let mut staged = pending_envelope(version)
        .event_type_bytes(Bytes::from(row.event_type.clone().into_bytes()))
        .expect("valid event type")
        .payload(row.payload.clone())
        .schema_version(
            SchemaVersion::from_u32(row.schema_version).expect("schema_version must be >= 1"),
        );
    if let Some(m) = &row.metadata {
        staged = staged.metadata(m.clone());
    }
    staged.build().expect("valid envelope")
}

/// Read a `PersistedEnvelope` back into row form.
#[must_use]
pub fn row_of(env: &PersistedEnvelope) -> ConformanceRow {
    ConformanceRow {
        version: env.version().as_u64(),
        event_type: env.event_type().to_owned(),
        schema_version: env.schema_version(),
        payload: env.payload().to_vec(),
        metadata: env.metadata().map(<[u8]>::to_vec),
    }
}

/// Append `rows` to `id` as one batch on a fresh stream (`expected = None`).
pub async fn append_rows<S: RawEventStore>(store: &S, id: &StreamKey, rows: &[ConformanceRow]) {
    if rows.is_empty() {
        return;
    }
    let envs: Vec<PendingEnvelope> = rows.iter().map(envelope_for).collect();
    store
        .append(id, None, &envs)
        .await
        .unwrap_or_else(|e| panic!("append of {} rows failed: {e:?}", rows.len()));
}

/// Append one event at `version` with the matching optimistic expectation
/// (`None` for version 1). Panics on failure — callers drive clean sequences.
pub async fn append_event<S: RawEventStore>(
    store: &S,
    id: &StreamKey,
    version: u64,
    payload: &[u8],
) {
    let expected = Version::new(version.saturating_sub(1));
    let env = envelope_for(&ConformanceRow::new(version, "E", payload.to_vec()));
    store
        .append(id, expected, &[env])
        .await
        .unwrap_or_else(|e| panic!("append v{version} failed: {e:?}"));
}

/// Drain `read_stream(id, from)` fully into rows.
pub async fn drain_stream<S: RawEventStore>(
    store: &S,
    id: &StreamKey,
    from: Version,
) -> Vec<ConformanceRow> {
    let stream = store
        .read_stream(id, from)
        .await
        .unwrap_or_else(|e| panic!("read_stream failed: {e:?}"));
    pin_mut!(stream);
    let mut out = Vec::new();
    while let Some(item) = stream.next().await {
        let env = item.unwrap_or_else(|e| panic!("read_stream item errored: {e:?}"));
        out.push(row_of(&env));
    }
    out
}

/// Drain `read_all(from)` fully into `(position, payload)` pairs.
pub async fn drain_all<S: RawEventStore>(
    store: &S,
    from: Option<S::AllPosition>,
) -> Vec<(S::AllPosition, Vec<u8>)> {
    let stream = store
        .read_all(from)
        .await
        .unwrap_or_else(|e| panic!("read_all failed: {e:?}"));
    pin_mut!(stream);
    let mut out = Vec::new();
    while let Some(item) = stream.next().await {
        let (pos, env) = item.unwrap_or_else(|e| panic!("read_all item errored: {e:?}"));
        out.push((pos, env.payload().to_vec()));
    }
    out
}

/// Assert positions strictly increase (monotonic, no duplicate).
pub fn assert_strictly_increasing<P: Copy + Ord + fmt::Debug>(positions: &[(P, Vec<u8>)]) {
    for w in positions.windows(2) {
        assert!(
            w[1].0 > w[0].0,
            "$all positions must be strictly increasing: {:?} then {:?}",
            w[0].0,
            w[1].0,
        );
    }
}
