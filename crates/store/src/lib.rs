//! Persistence edge layer for the mnesis event-sourcing kernel.
//!
//! `mnesis-store` sits between the pure-domain kernel (`mnesis`) and the
//! storage adapters (`mnesis-fjall`, future postgres, etc.). It owns the
//! shapes that cross the kernel↔storage boundary — envelopes, codecs,
//! event streams, repositories, and snapshot stores — and the wire-format
//! row builder every adapter is required to use.
//!
//! # Crate layout
//!
//! Flat: one file per concept, no module subdirectories. Each module's
//! own `//!` header documents its rationale.
//!
//! - [`codec`] — one [`Encode<E>`](crate::Encode) trait and one
//!   [`Decode<E>`](crate::Decode) trait with an `Output<'a>` GAT. The GAT
//!   collapses what used to be two traits (`Decode` + `BorrowingDecode`)
//!   into a single shape that covers both owning serde codecs and
//!   borrowing codecs (rkyv, bytemuck). Feature-gated codec impls
//!   (`serde`, `json`, `bytemuck`, `rkyv`) ship with the crate.
//! - [`envelope`] — [`PendingEnvelope`] (write path, typestate-built) and
//!   [`PersistedEnvelope`] (read path, owned [`bytes::Bytes`] + cached
//!   `Range<u32>` offsets). The read envelope is cheap-to-clone (Arc
//!   refcount + range copies) and has no lifetime parameter, so it flows
//!   through `futures::Stream` items without bridging code.
//! - [`store`] — adapter-facing [`RawEventStore`] trait,
//!   [`Store<S>`](crate::store::Store) shared handle, and [`AllPosition`]
//!   (the adapter-defined `$all` resume position — the concrete type lives in
//!   each adapter, only the trait here).
//! - [`subscription`] — user-facing [`Subscription<S>`] struct (built
//!   via `Subscription::new(&store)`). Its `subscribe` / `subscribe_all`
//!   methods assemble the generic catch-up-then-live-tail loop from
//!   [`RawEventStore`] + [`WakeSource`]; there is
//!   no adapter-facing subscription trait. The returned cursor is `!Unpin`
//!   (consumers `pin!` it).
//! - [`stream`] — [`EventStream`] marker trait over
//!   `futures::Stream<Item = Result<PersistedEnvelope, _>>`. The marker
//!   carries no methods of its own — every combinator comes from
//!   [`futures::StreamExt`](https://docs.rs/futures/latest/futures/stream/trait.StreamExt.html)
//!   and [`TryStreamExt`](https://docs.rs/futures/latest/futures/stream/trait.TryStreamExt.html).
//! - [`wire`] — single canonical frame builder
//!   ([`encode_frame`](crate::wire::encode_frame)) that every adapter must use.
//!   Aligns payload offsets to 16 bytes. Adapters must also check the actual
//!   buffer address and realign when needed; borrowing codecs validate their
//!   format and required alignment before returning references.
//! - [`repository`] / [`builder`] — aggregate-facing [`Repository<A>`]
//!   trait plus its facade impl ([`EventStore`], one terminal for both
//!   owning and borrowing codecs), constructed via the
//!   [`RepositoryBuilder`] typestate.
//! - [`state`] — [`SnapshotStore<S, P>`](crate::SnapshotStore) for atomic
//!   snapshot cache persistence and the codec bridge for typed state.
//! - [`checkpoint`] — conditional projection writes with monotonically
//!   increasing revisions and ordered positions. State and position are
//!   replaced together; stale writers cannot overwrite a newer checkpoint.
//! - [`upcasting`] — schema evolution via plain functions passed to [`EventStore::load_with`] and
//!   [`EventMorsel`] zero-copy-when-possible data unit.
//! - [`snapshot`] (feature-gated) — decorator that wraps a repository to
//!   hydrate from a [`SnapshotStore`] on read and commit on write per a
//!   [`PersistTrigger`].
//! - [`projection`] (feature-gated) — [`Projector`] trait (pure fallible
//!   fold). mnesis ships no runner; the loop is consumer-owned (see
//!   `examples/projection-tokio`).
//!
//! # Feature flags
//!
//! | Feature | Effect |
//! |---|---|
//! | `serde` | Generic serde codec (`SerdeCodec<F>`) |
//! | `json` | `Json` format + `JsonCodec` alias (implies `serde`) |
//! | `bytemuck` | `BytemuckCodec` for `#[repr(C)]` POD types (zero-copy `&E`) |
//! | `rkyv` | `RkyvCodec` for rkyv-archived types (zero-copy `&Archived<E>`) |
//! | `snapshot` | `Snapshotting<R, SS, T>` repository decorator |
//! | `snapshot-json` | `snapshot` + `json` |
//! | `projection` | `Projector` trait |
//! | `projection-json` | `projection` + `json` |
//! | `subscription` | [`Subscription`] catch-up-then-live-tail loop + [`wake`] traits (dep-free; in-process wake impl lives in `mnesis-wake`) |
//!
//! # Design notes
//!
//! The earlier `codec/`, `envelope/`, `upcasting/`, `store/`,
//! `repository/`, `state/`, `projection/` directories were collapsed into
//! single files because each held only a handful of small files with no
//! cohesion benefit. The boundary that matters is the crate boundary
//! (kernel-pure → store-persistence → adapters); the boundary that
//! didn't matter was inside `mnesis-store`.

#![cfg_attr(not(feature = "std"), no_std)]

// The store is alloc-dependent by design (Bytes, Vec, Arc are its working
// vocabulary) — unlike the pure-core kernel, `alloc` is unconditional.
extern crate alloc;

pub mod batch;
pub mod builder;
#[cfg(feature = "subscription")]
pub(crate) mod catchup;
#[cfg(feature = "cbor")]
pub mod cbor;
pub mod checkpoint;
pub mod codec;
pub mod conflict;
pub mod decoded;
pub mod envelope;
pub mod error;
pub mod execute;
#[cfg(feature = "export")]
pub mod export;
#[cfg(feature = "import")]
pub mod import;
pub mod metadata;
#[cfg(feature = "projection")]
pub mod projection;
pub mod repository;
pub mod saga;
#[cfg(feature = "snapshot")]
pub mod snapshot;
pub mod state;
pub mod step;
pub mod store;
pub mod stream;
pub mod stream_id;
#[cfg(feature = "subscription")]
pub mod subscription;
#[cfg(feature = "subscription")]
pub(crate) mod subscription_cursor;
#[cfg(all(test, feature = "subscription"))]
pub(crate) mod test_support;
pub mod upcasting;
pub mod value;
#[cfg(feature = "subscription")]
pub mod wake;
pub mod wire;

pub use batch::{BatchSize, BatchSizeError, DEFAULT_BATCH, MAX_BATCH};
#[cfg(feature = "snapshot")]
pub use builder::WithSnapshot;
pub use builder::{NeedsCodec, NoSnapshot, RepositoryBuilder};
// Re-export `bytes` so downstreams name `mnesis_store::bytes::Bytes` to feed
// `Encode` / the value newtypes, sharing *our* version rather than coupling to
// theirs. Additive (non-breaking).
pub use bytes;
#[cfg(feature = "cbor")]
pub use cbor::{
    BackupEventError, ChunkError, ChunkHeader, ChunkSinkError, ChunkWriter, CompletionError,
    SalvagedChunk, SectionError, SectionWriter, WriteError, decode_chunk, decode_header,
    salvage_chunk,
};
#[cfg(feature = "json")]
pub use codec::serde::json::{Json, JsonCodec};
#[cfg(feature = "serde")]
pub use codec::serde::{SerdeCodec, SerdeFormat};
pub use codec::{Decode, Encode, OwningCodec};
pub use conflict::ConflictPredicate;
pub use decoded::{
    DecodeStreamError, Decoded, DecodedStreamExt, FoldDecodedError, RawItem, StepStreamExt,
};
pub use envelope::{
    EnvelopeError, ForDecodeError, PendingBatch, PendingBatchIter, PendingEnvelope,
    PersistedEnvelope, pending_envelope,
};
pub use error::LoadWithError;
pub use error::{AppendError, StoreError};
pub use execute::{CommandRepository, ExecuteError, Execution};
#[cfg(feature = "export")]
pub use export::{EventExporter, StreamLister};
#[cfg(feature = "import")]
pub use import::{
    AbortReason, Atomicity, EventImporter, ImportBlock, ImportError, ImportReport, StreamOutcome,
    StreamReport, StreamSection,
};
pub use metadata::MetadataProvider;
pub use mnesis::Version;
#[cfg(feature = "projection")]
pub use projection::{Positioned, Projection, ProjectionError, ProjectionStateError, Projector};
pub use repository::{EventStore, Repository};
pub use saga::{
    ProjectedIntent, ProjectedIntents, ProjectedIntentsIntoIter, Reaction, SagaError,
    SagaRepository,
};
#[cfg(feature = "snapshot")]
pub use snapshot::Snapshotting;
pub use state::{
    AfterEventTypes, CodecSnapshotStore, CodecSnapshotStoreError, EveryNEvents, Hydrated,
    PersistTrigger, SnapshotStore,
};
pub use step::Step;
pub use store::{AllPosition, RawEventStore, Store};
pub use stream::EventStream;
pub use stream_id::StreamKey;
// Re-export the `Stream` trait from `futures-core` (the small, near-frozen
// definitional crate) rather than `futures`. `futures::Stream` *is* this trait,
// so our public `EventStream` / `subscribe*` surface is married to
// `futures-core`'s stability, not the churning batteries-included `futures`.
pub use futures_core::Stream;
#[cfg(feature = "subscription")]
pub use subscription::Subscription;
pub use upcasting::{EventMorsel, TransformError};
pub use value::{EventType, Metadata, Payload, SchemaVersion, ValueError};
#[cfg(feature = "subscription")]
pub use wake::{WakeRegistration, WakeSource};
