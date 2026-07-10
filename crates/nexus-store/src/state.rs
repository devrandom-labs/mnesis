use alloc::vec::Vec;
use core::future::Future;
use core::num::{NonZeroU32, NonZeroU64};

use nexus::{Id, Version};

use crate::codec::{Decode, Encode, OwningCodec};

// ═══════════════════════════════════════════════════════════════════════════
// SnapshotStore<S, P> — atomic state + position persistence
// ═══════════════════════════════════════════════════════════════════════════

/// Outcome of [`SnapshotStore::hydrate`] — a three-state answer that keeps
/// "nothing saved" distinct from "saved, but stale".
///
/// The distinction is invisible to an aggregate snapshot (both mean "replay the
/// stream"), but load-bearing for a projection: [`Absent`](Self::Absent) is a
/// brand-new projection expected to start empty, whereas [`Stale`](Self::Stale)
/// means an existing projection was invalidated by a schema bump and the very
/// next thing that happens is a **full re-fold of the whole `$all` stream**. On
/// a mobile/`IoT` host that re-fold can be a long, battery-heavy operation, so
/// the host must be able to see it coming (warn, defer to Wi-Fi/charging,
/// throttle) — collapsing both into `None` would hide it.
///
/// There is deliberately no `stored_state` on `Stale`: derived state has no
/// upcasting path (a schema change forces a rebuild, it cannot be migrated), so
/// carrying the old bytes would only invite a migration that does not exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hydrated<S, P> {
    /// Nothing has ever been saved for this id — start from scratch.
    Absent,
    /// A snapshot exists but under a *different* schema version; it cannot be
    /// decoded into the requested shape, so the caller must rebuild from the
    /// log. Carries the schema version that was found, for observability.
    Stale {
        /// The schema version the stored snapshot was written under.
        stored_schema: NonZeroU32,
    },
    /// A snapshot at the requested schema version.
    Found {
        /// The position the state was folded up to.
        position: P,
        /// The restored state.
        state: S,
    },
}

impl<S, P> Hydrated<S, P> {
    /// The restored `(position, state)` when a snapshot at the requested schema
    /// version was found; `None` for `Absent` or `Stale` (both mean "rebuild").
    ///
    /// A convenience for callers that treat absent and stale identically (e.g.
    /// the aggregate-snapshot decorator, which replays the stream either way).
    /// Callers that must tell the two apart — projection hosts — match the enum.
    #[must_use]
    pub fn into_found(self) -> Option<(P, S)> {
        match self {
            Self::Found { position, state } => Some((position, state)),
            Self::Absent | Self::Stale { .. } => None,
        }
    }
}

/// Atomic persistence of a snapshot — derived state plus the position it
/// was folded up to.
///
/// One trait, two callers:
/// - aggregate snapshots — the aggregate's state, at its `Version`.
/// - projections — the projection's state, at its position.
///
/// State and position are saved and loaded *together*. A half-write
/// (state without position, or position without state) is impossible:
/// the trait exposes only the two *combined* operations, never "save
/// state alone". Atomicity itself is the adapter's responsibility — it
/// owns both the state and position storage and commits them in one
/// transaction.
///
/// Generic over the position type `P` so one trait serves a single
/// stream (`P = Version`) and a multi-stream, single-producer projection
/// (`P =` the adapter's [`AllPosition`](crate::AllPosition)).
pub trait SnapshotStore<S, P>: Send + Sync {
    /// Adapter-specific error type.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Load the saved state and position from a single consistent snapshot.
    ///
    /// Returns [`Hydrated::Found`] with the state and position when a snapshot
    /// at `schema_version` exists; [`Hydrated::Stale`] when one exists under a
    /// different schema version (caller must rebuild); [`Hydrated::Absent`] when
    /// nothing has been saved. `Stale` and `Absent` are kept distinct so a
    /// projection host can tell a fresh start from a schema-bump rebuild.
    ///
    /// # Errors
    ///
    /// Returns `Self::Error` if the underlying store fails to read.
    fn hydrate(
        &self,
        id: &impl Id,
        schema_version: NonZeroU32,
    ) -> impl Future<Output = Result<Hydrated<S, P>, Self::Error>> + Send;

    /// Save state and position together, in a single transaction.
    ///
    /// Either both are durably stored, or neither is.
    ///
    /// # Errors
    ///
    /// Returns `Self::Error` if the underlying store fails to commit.
    fn commit(
        &self,
        id: &impl Id,
        schema_version: NonZeroU32,
        position: P,
        state: &S,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

// ═══════════════════════════════════════════════════════════════════════════
// Delegation implementation — share via reference
// ═══════════════════════════════════════════════════════════════════════════

impl<S, P, T> SnapshotStore<S, P> for &T
where
    S: Send + Sync,
    P: Send,
    T: SnapshotStore<S, P>,
{
    type Error = T::Error;

    fn hydrate(
        &self,
        id: &impl Id,
        schema_version: NonZeroU32,
    ) -> impl Future<Output = Result<Hydrated<S, P>, Self::Error>> + Send {
        (**self).hydrate(id, schema_version)
    }

    fn commit(
        &self,
        id: &impl Id,
        schema_version: NonZeroU32,
        position: P,
        state: &S,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        (**self).commit(id, schema_version, position, state)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// PersistTrigger — when-to-persist policy
// ═══════════════════════════════════════════════════════════════════════════

/// Strategy for deciding when to persist state.
///
/// Used by both projection runners (when to checkpoint projection state)
/// and snapshot decorators (when to snapshot aggregate state).
pub trait PersistTrigger: Send + Sync {
    /// Whether state should be persisted now.
    ///
    /// - `old_version`: version before the operation (`None` for first run)
    /// - `new_version`: version after the operation
    /// - `event_names`: names of events just processed
    fn should_persist(
        &self,
        old_version: Option<Version>,
        new_version: Version,
        event_names: impl Iterator<Item: AsRef<str>>,
    ) -> bool;
}

/// Persist every N events (bucket-crossing algorithm).
#[derive(Debug, Clone, Copy)]
pub struct EveryNEvents(pub NonZeroU64);

impl PersistTrigger for EveryNEvents {
    fn should_persist(
        &self,
        old_version: Option<Version>,
        new_version: Version,
        _event_names: impl Iterator<Item: AsRef<str>>,
    ) -> bool {
        let n = self.0.get();
        let old_bucket = old_version.map_or(0, |v| v.as_u64() / n);
        let new_bucket = new_version.as_u64() / n;
        new_bucket > old_bucket
    }
}

/// Persist after specific event types.
#[derive(Debug, Clone)]
pub struct AfterEventTypes {
    types: Vec<&'static str>,
}

impl AfterEventTypes {
    /// Create a trigger that fires when any of the given event types is persisted.
    #[must_use]
    pub fn new(types: &[&'static str]) -> Self {
        Self {
            types: types.to_vec(),
        }
    }
}

impl PersistTrigger for AfterEventTypes {
    fn should_persist(
        &self,
        _old_version: Option<Version>,
        _new_version: Version,
        mut event_names: impl Iterator<Item: AsRef<str>>,
    ) -> bool {
        event_names.any(|name| self.types.iter().any(|t| *t == name.as_ref()))
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// CodecSnapshotStore<SS, C> — byte-level <-> typed bridge via Encode + Decode
// ═══════════════════════════════════════════════════════════════════════════

/// Adapter that bridges a byte-level [`SnapshotStore<Vec<u8>, P>`] to a typed
/// [`SnapshotStore<S, P>`] by encoding/decoding through an [`Encode<S>`] +
/// [`Decode<S>`] pair.
///
/// Use this when your storage backend works with raw bytes (e.g., fjall)
/// but consumers need typed state. The position `P` is opaque to the
/// bridge — it passes through untouched.
pub struct CodecSnapshotStore<SS, C> {
    store: SS,
    codec: C,
}

impl<SS, C> CodecSnapshotStore<SS, C> {
    /// Create a new codec-bridged snapshot store.
    #[must_use]
    pub const fn new(store: SS, codec: C) -> Self {
        Self { store, codec }
    }
}

impl<S, P, SS, C> SnapshotStore<S, P> for CodecSnapshotStore<SS, C>
where
    S: Send + Sync + 'static,
    P: Send,
    SS: SnapshotStore<Vec<u8>, P>,
    C: Encode<S> + OwningCodec<S>,
{
    type Error =
        CodecSnapshotStoreError<SS::Error, <C as Encode<S>>::Error, <C as Decode<S>>::Error>;

    async fn hydrate(
        &self,
        id: &impl Id,
        schema_version: NonZeroU32,
    ) -> Result<Hydrated<S, P>, Self::Error> {
        // Absent/Stale pass through untouched — only `Found` carries bytes to
        // decode. The position `P` and the schema-version signal are opaque to
        // the bridge.
        let (position, bytes) = match self
            .store
            .hydrate(id, schema_version)
            .await
            .map_err(CodecSnapshotStoreError::Store)?
        {
            Hydrated::Absent => return Ok(Hydrated::Absent),
            Hydrated::Stale { stored_schema } => return Ok(Hydrated::Stale { stored_schema }),
            Hydrated::Found { position, state } => (position, state),
        };

        let label = id.to_label();
        // Wrap the snapshot's raw bytes in a synthetic envelope so the
        // codec's envelope-based decode trait can be called. The
        // snapshot wire format is *not* the event wire format; this
        // synthesis only carries the bytes through to `decode()`.
        let env = crate::envelope::PersistedEnvelope::for_decode(label.as_str(), &bytes)
            .map_err(CodecSnapshotStoreError::EnvelopeSynthesis)?;
        let state =
            <C as Decode<S>>::decode(&self.codec, &env).map_err(CodecSnapshotStoreError::Decode)?;

        Ok(Hydrated::Found { position, state })
    }

    async fn commit(
        &self,
        id: &impl Id,
        schema_version: NonZeroU32,
        position: P,
        state: &S,
    ) -> Result<(), Self::Error> {
        let bytes = <C as Encode<S>>::encode(&self.codec, state)
            .map_err(CodecSnapshotStoreError::Encode)?;

        // SnapshotStore<Vec<u8>, P> requires &Vec<u8>; adapt by copying.
        // Snapshot writes are rare relative to the read path, so the
        // extra allocation here is acceptable.
        let bytes_vec = bytes.to_vec();
        self.store
            .commit(id, schema_version, position, &bytes_vec)
            .await
            .map_err(CodecSnapshotStoreError::Store)
    }
}

/// Error from [`CodecSnapshotStore`] — the underlying store, the encoder, the decoder,
/// or the wire-format synthesis used to call the envelope-based decode trait.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CodecSnapshotStoreError<S, EncErr, DecErr> {
    /// The underlying byte-level store failed.
    #[error(transparent)]
    Store(S),
    /// Encoding failed.
    #[error(transparent)]
    Encode(EncErr),
    /// Decoding failed.
    #[error(transparent)]
    Decode(DecErr),
    /// Wire-format synthesis failed while wrapping the snapshot bytes in
    /// an envelope for the codec. Practically unreachable for in-budget
    /// labels (≤64 bytes via `Id::to_label`) and snapshot bytes ≤ 4 GiB.
    #[error("envelope synthesis error: {0}")]
    EnvelopeSynthesis(#[source] crate::envelope::ForDecodeError),
}
