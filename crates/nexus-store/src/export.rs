//! Export contract — generic over [`RawEventStore`], raw and box-agnostic.
//!
//! Export is a **generic store capability**, defined against
//! [`RawEventStore`] only — it never touches a wire frame or an adapter
//! partition, so it works for the in-memory store, fjall, and a future
//! postgres store alike.
//!
//! Export does **no data manipulation**. `export_stream(id, from)` is a pass
//! through to [`RawEventStore::read_stream`] — it yields the stored
//! [`PersistedEnvelope`](crate::envelope::PersistedEnvelope)s verbatim. The events are not rewritten, the
//! store-local `global_seq` is not stripped, and the stream id is not stamped
//! onto each event. Two facts make that unnecessary:
//!
//! - **The caller supplies the id.** `export_stream(id, …)` is per-stream, so
//!   the caller already knows which stream the events belong to — exactly like
//!   a read. The stream id never has to ride on each event.
//! - **Import re-appends.** On restore, import writes events through the
//!   normal append path, which stamps a fresh `global_seq` itself. The old
//!   store-local value simply rides along and is ignored — stripping it at
//!   export would be work import redoes for free.
//!
//! Two traits:
//!
//! - [`StreamLister`] — enumerate the stream ids a store holds. The one new
//!   store-layer capability export needs; an all-streams export is
//!   `list_streams` ∘ `export_stream`.
//! - [`EventExporter`] — open a per-stream export (a raw read).
//!
//! The stream id *is* recorded once per stream — but in the **backup box**
//! (the CBOR default, a later card), as a per-stream section heading, never on
//! the events. A restore reads that heading to route the section back to the
//! right stream; import then ignores each event's `global_seq`. See issue
//! #145 §5.

use futures::Stream;
use nexus::Version;

use crate::store::{RawEventStore, Store};
use crate::stream::EventStream;
use crate::stream_id::StreamKey;

/// Enumerate the stream ids present in a store.
///
/// The generic source of "which streams exist" — needed because a backup of
/// an arbitrary store doesn't know its ids up front, and `export_stream`
/// requires one. Yields the raw stream-id bytes (the form the store holds
/// them in); the caller reconstitutes a typed [`Id`] if it needs one.
///
/// Lazy and async, mirroring [`RawEventStore::read_all`]: a store with many
/// streams streams its ids rather than materializing them all.
///
/// Adapters back this with whatever index already tracks streams (fjall: its
/// `streams` partition; in-memory: its map; postgres: `SELECT DISTINCT`).
pub trait StreamLister: RawEventStore {
    /// The stream of stream ids.
    type StreamList: Stream<Item = Result<StreamKey, Self::Error>> + Send + 'static;

    /// Open a one-shot stream over every stream id in the store, in no
    /// guaranteed order, terminating when exhausted.
    fn list_streams(
        &self,
    ) -> impl std::future::Future<Output = Result<Self::StreamList, Self::Error>> + Send;
}

/// Export a single stream's events — a raw pass-through read.
///
/// `export_stream(id, from)` reads stream `id` from `from` **inclusive** (the
/// same semantics as [`RawEventStore::read_stream`]: `from = Version::INITIAL`
/// yields the whole stream from v1) up to its current head, then terminates.
/// Each yielded [`PersistedEnvelope`](crate::envelope::PersistedEnvelope) is the stored event **verbatim** — no
/// rewrite, `global_seq` intact, no per-event stream id.
///
/// `from` is inclusive because the type forbids otherwise: [`Version`] is a
/// `NonZeroU64` (minimum 1), so an exclusive `from` could never include v1 and
/// a full export would be impossible. To **resume** after the last exported
/// version `V`, pass `V.next()` (the caller's responsibility, mirroring how a
/// subscription cursor resumes).
///
/// The stream is **pull-based**: it reads as polled, in bounded memory, so a
/// consumer can write events to a file incrementally over any timespan.
///
/// `export_all` (`list_streams` ∘ `export_stream`) and continuous/live export
/// (compose with the never-ending `subscribe` cursor) are consumer-side
/// combinators, not part of this trait. The blanket impl below makes **every**
/// [`RawEventStore`] an `EventExporter` for free.
pub trait EventExporter: RawEventStore {
    /// The stream of exported events. Identical to the read stream — export
    /// performs no transform.
    type ExportStream: EventStream<Error = Self::Error> + 'static;

    /// Open a per-stream export of stream `id`, starting at `from` (inclusive).
    fn export_stream(
        &self,
        id: &StreamKey,
        from: Version,
    ) -> impl std::future::Future<Output = Result<Self::ExportStream, Self::Error>> + Send;
}

/// Every [`RawEventStore`] is an [`EventExporter`] — export is just a read.
///
/// `export_stream` forwards to [`RawEventStore::read_stream`] unchanged, so the
/// associated [`ExportStream`](EventExporter::ExportStream) is the adapter's
/// own [`Stream`](RawEventStore::Stream) type: a concrete, monomorphized
/// cursor with no boxing, no dynamic dispatch, and no per-event transform.
impl<S: RawEventStore> EventExporter for S {
    type ExportStream = S::Stream;

    fn export_stream(
        &self,
        id: &StreamKey,
        from: Version,
    ) -> impl std::future::Future<Output = Result<Self::ExportStream, Self::Error>> + Send {
        self.read_stream(id, from)
    }
}

/// `Store<S>` forwards [`StreamLister`] to its inner backend (issue #247), so a
/// handle holder can `store.list_streams()` without `.raw()`. `EventExporter`
/// then applies to `Store<S>` via the blanket impl above (`Store<S>` is itself a
/// [`RawEventStore`]).
impl<S: StreamLister> StreamLister for Store<S> {
    type StreamList = S::StreamList;

    async fn list_streams(&self) -> Result<Self::StreamList, Self::Error> {
        self.raw().list_streams().await
    }
}
