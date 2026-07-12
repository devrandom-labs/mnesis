//! Issue #281 acceptance: a toy adapter written against ONLY the published
//! writing-a-store-adapter guide (the `nexus-store-testing` crate docs) and
//! the `nexus-store` / `nexus-wake` public APIs — no shipped adapter source
//! was consulted. It must pass the full conformance matrix.

#![allow(clippy::unwrap_used, reason = "tests")]
#![allow(clippy::expect_used, reason = "tests")]
#![allow(clippy::missing_panics_doc, reason = "tests")]

use std::collections::HashMap;
use std::sync::Arc;

use bytes::Bytes;
use futures::stream;
use nexus::{ErrorId, Version};
use nexus_store::StreamKey;
use nexus_store::envelope::{EnvelopeError, PendingEnvelope, PersistedEnvelope};
use nexus_store::error::AppendError;
use nexus_store::import::{AtomicAppend, AtomicAppendError, PlannedAppend};
use nexus_store::store::{AllPosition, RawEventStore};
use nexus_store::value::SchemaVersion;
use nexus_store::wake::WakeSource;
use nexus_store::wire::{FrameOffsets, WireError, encode_frame};
use nexus_wake::{NotifyError, StreamNotifiers, WakeReg};
use thiserror::Error;
use tokio::sync::Mutex;

// ═══════════════════════════════════════════════════════════════════════════
// ToyStore — HashMap-backed adapter, one tokio Mutex = the atomic step
// ═══════════════════════════════════════════════════════════════════════════

/// Adapter error, distinct from the facade's error types per the guide.
#[derive(Debug, Error)]
enum ToyError {
    #[error("wire encode failed")]
    Wire(#[from] WireError),
    #[error("envelope rebuild failed")]
    Envelope(#[from] EnvelopeError),
    #[error("$all position overflow")]
    PositionOverflow,
    #[error("stream version overflow")]
    VersionOverflow,
}

/// Why a batch failed sequential validation. Overflow is deliberately its
/// own arm: it maps to the `Store` domain, never `Conflict` — a `Conflict`
/// with `expected == actual` would be retry-eligible yet un-retryable by
/// construction (rule 3; fjall documents the same mapping).
enum SeqError {
    /// Gap, duplicate, or out-of-order — the `Conflict` domain.
    Malformed,
    /// The version sequence would pass `u64::MAX`.
    Overflow,
}

/// Store-local `$all` resume position: a scalar sequence, strictly monotonic
/// across all streams in commit order (gaps permitted by contract).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct ToyPos(u64);

impl AllPosition for ToyPos {}

/// One persisted row: the guide's "Storing an event" recipe — the `Version`
/// plus `encode_frame`'s value/offsets and the `SchemaVersion`.
#[derive(Debug, Clone)]
struct StoredEvent {
    version: Version,
    schema_version: SchemaVersion,
    value: Bytes,
    offsets: FrameOffsets,
    pos: ToyPos,
}

impl StoredEvent {
    fn rebuild(&self) -> Result<PersistedEnvelope, ToyError> {
        Ok(PersistedEnvelope::try_new(
            self.version,
            self.value.clone(),
            self.schema_version,
            self.offsets.event_type.clone(),
            self.offsets.payload.clone(),
            self.offsets.metadata.clone(),
        )?)
    }
}

#[derive(Debug, Default)]
struct Inner {
    streams: HashMap<Vec<u8>, Vec<StoredEvent>>,
    next_pos: u64,
}

impl Inner {
    fn head(&self, key: &[u8]) -> Option<Version> {
        self.streams
            .get(key)
            .and_then(|events| events.last())
            .map(|event| event.version)
    }
}

#[derive(Debug)]
struct ToyStore {
    inner: Mutex<Inner>,
    notifiers: Arc<StreamNotifiers>,
}

impl ToyStore {
    fn new() -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            notifiers: StreamNotifiers::new(),
        }
    }
}

/// Encode one pending envelope into a staged row at position `pos`.
fn stage(env: &PendingEnvelope, pos: ToyPos) -> Result<StoredEvent, ToyError> {
    let frame = encode_frame(
        env.schema_version_value(),
        &env.event_type_value(),
        &env.payload_value(),
        env.metadata_value().as_ref(),
    )?;
    Ok(StoredEvent {
        version: env.version(),
        schema_version: env.schema_version_value(),
        value: frame.value,
        offsets: frame.offsets,
        pos,
    })
}

/// Validate that `envelopes` runs strictly sequentially from
/// `expected_version + 1` (from 1 when `None`).
fn versions_sequential(
    expected_version: Option<Version>,
    envelopes: &[PendingEnvelope],
) -> Result<(), SeqError> {
    let mut next = match expected_version {
        None => Version::INITIAL,
        Some(v) => v.next().ok_or(SeqError::Overflow)?,
    };
    let mut envs = envelopes.iter().peekable();
    while let Some(env) = envs.next() {
        if env.version() != next {
            return Err(SeqError::Malformed);
        }
        if envs.peek().is_some() {
            next = next.next().ok_or(SeqError::Overflow)?;
        }
    }
    Ok(())
}

impl RawEventStore for ToyStore {
    type Error = ToyError;
    type Stream = stream::Iter<std::vec::IntoIter<Result<PersistedEnvelope, ToyError>>>;
    type AllPosition = ToyPos;
    type AllStream =
        stream::Iter<std::vec::IntoIter<Result<(ToyPos, PersistedEnvelope), ToyError>>>;

    async fn append(
        &self,
        id: &StreamKey,
        expected_version: Option<Version>,
        envelopes: &[PendingEnvelope],
    ) -> Result<(), AppendError<ToyError>> {
        {
            // One critical section covers head check + validation + insertion:
            // the guide's "one atomic step".
            let mut inner = self.inner.lock().await;
            let actual = inner.head(id.as_bytes());
            if actual != expected_version {
                return Err(AppendError::Conflict {
                    stream_id: ErrorId::from_display(id),
                    expected: expected_version,
                    actual,
                });
            }
            // Empty batch: head check done, nothing written, nobody woken.
            if envelopes.is_empty() {
                return Ok(());
            }
            // A gap/duplicate/out-of-order batch is rejected in the Conflict
            // domain, and nothing lands. Version overflow is a Store error —
            // it is not a retry-eligible concurrency conflict.
            match versions_sequential(expected_version, envelopes) {
                Err(SeqError::Malformed) => {
                    return Err(AppendError::Conflict {
                        stream_id: ErrorId::from_display(id),
                        expected: expected_version,
                        actual,
                    });
                }
                Err(SeqError::Overflow) => {
                    return Err(AppendError::Store(ToyError::VersionOverflow));
                }
                Ok(()) => {}
            }
            // Stage everything before touching the map so a failed encode
            // leaves the store byte-identical.
            let mut staged = Vec::with_capacity(envelopes.len());
            let mut pos = inner.next_pos;
            for env in envelopes {
                pos = pos
                    .checked_add(1)
                    .ok_or(AppendError::Store(ToyError::PositionOverflow))?;
                staged.push(stage(env, ToyPos(pos)).map_err(AppendError::Store)?);
            }
            inner.next_pos = pos;
            inner
                .streams
                .entry(id.as_bytes().to_vec())
                .or_default()
                .extend(staged);
        }
        // After the commit is durable — never before — fire the wake path.
        // One call wakes the stream's subscribers and bumps the `$all`
        // generation.
        self.notifiers.wake(id.as_bytes());
        Ok(())
    }

    async fn read_stream(&self, id: &StreamKey, from: Version) -> Result<Self::Stream, ToyError> {
        // Inclusive `from`: every event with version >= from, ascending. The
        // lock guard is a single-statement temporary (dropped before return).
        let items: Vec<Result<PersistedEnvelope, ToyError>> = self
            .inner
            .lock()
            .await
            .streams
            .get(id.as_bytes())
            .map(|events| {
                events
                    .iter()
                    .filter(|event| event.version >= from)
                    .map(StoredEvent::rebuild)
                    .collect()
            })
            .unwrap_or_default();
        Ok(stream::iter(items))
    }

    async fn read_all(&self, from: Option<ToyPos>) -> Result<Self::AllStream, ToyError> {
        // Exclusive `from`: strictly after, tolerating gaps by range-scanning.
        // Rows are cloned out (one Arc share each) so the lock guard is a
        // single-statement temporary.
        let mut rows: Vec<StoredEvent> = self
            .inner
            .lock()
            .await
            .streams
            .values()
            .flatten()
            .filter(|event| from.is_none_or(|p| event.pos > p))
            .cloned()
            .collect();
        rows.sort_by_key(|event| event.pos);
        let items: Vec<Result<(ToyPos, PersistedEnvelope), ToyError>> = rows
            .iter()
            .map(|event| event.rebuild().map(|env| (event.pos, env)))
            .collect();
        Ok(stream::iter(items))
    }
}

impl WakeSource for ToyStore {
    type Registration = WakeReg;
    type Error = NotifyError;

    fn register(&self, stream: Option<&[u8]>) -> Result<WakeReg, NotifyError> {
        self.notifiers.register(stream)
    }

    fn wake(&self, stream: &[u8]) {
        self.notifiers.wake(stream);
    }
}

impl AtomicAppend for ToyStore {
    async fn atomic_append_many(
        &self,
        writes: &[PlannedAppend],
    ) -> Result<(), AtomicAppendError<ToyError>> {
        {
            let mut inner = self.inner.lock().await;
            // Phase 1 — validate every write against a RUNNING projected head
            // (counting earlier writes to the same target in this batch) and
            // stage; any failure returns before anything is applied.
            let mut heads: HashMap<Vec<u8>, Option<Version>> = HashMap::new();
            let mut staged: Vec<(Vec<u8>, Vec<StoredEvent>)> = Vec::new();
            let mut pos = inner.next_pos;
            for (index, write) in writes.iter().enumerate() {
                let key = write.target.as_bytes().to_vec();
                let head = *heads.entry(key.clone()).or_insert_with(|| inner.head(&key));
                if head != write.expected_version {
                    return Err(AtomicAppendError::Conflict {
                        index,
                        actual: head,
                    });
                }
                if write.events.is_empty() {
                    continue;
                }
                // Defensive contiguity validation at this boundary; overflow
                // is a Store error, never a Conflict (rule 3).
                match versions_sequential(write.expected_version, &write.events) {
                    Err(SeqError::Malformed) => {
                        return Err(AtomicAppendError::Conflict {
                            index,
                            actual: head,
                        });
                    }
                    Err(SeqError::Overflow) => {
                        return Err(AtomicAppendError::Store(ToyError::VersionOverflow));
                    }
                    Ok(()) => {}
                }
                let mut run = Vec::with_capacity(write.events.len());
                for env in &write.events {
                    pos = pos
                        .checked_add(1)
                        .ok_or(AtomicAppendError::Store(ToyError::PositionOverflow))?;
                    run.push(stage(env, ToyPos(pos)).map_err(AtomicAppendError::Store)?);
                }
                let new_head = write.events.last().map(PendingEnvelope::version);
                heads.insert(key.clone(), new_head);
                staged.push((key, run));
            }
            // Phase 2 — commit: everything lands, or (above) nothing did.
            inner.next_pos = pos;
            for (key, run) in staged {
                inner.streams.entry(key).or_default().extend(run);
            }
        }
        // Wake only when something actually landed — same nobody-woken-on-
        // empty discipline as `append` (spurious wakes are permitted, but the
        // reference shouldn't teach them). Each wake also bumps the `$all`
        // generation.
        for write in writes.iter().filter(|w| !w.events.is_empty()) {
            self.notifiers.wake(write.target.as_bytes());
        }
        Ok(())
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// The kit
// ═══════════════════════════════════════════════════════════════════════════

nexus_store_testing::conformance! {
    factory: || async { (ToyStore::new(), ()) },
}

nexus_store_testing::conformance_atomic_append! {
    factory: || async { (ToyStore::new(), ()) },
}
