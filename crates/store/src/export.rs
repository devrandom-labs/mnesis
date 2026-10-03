//! Raw per-stream export and stream enumeration.
//!
//! [`EventExporter::export_stream`] forwards to [`RawEventStore::read_stream`]
//! and yields stored [`PersistedEnvelope`](crate::PersistedEnvelope)s without
//! rewriting payloads, metadata, schema versions or stream versions. The caller
//! supplies the stream id; a backup records it once per stream section.
//! Envelopes contain neither a global position nor an origin stream id.
//! [`RawEventStore::read_all`] supplies those separately. Import allocates fresh
//! destination global positions when it re-appends the exported events.
//!
//! [`StreamLister`] enumerates stream ids independently of event reads.
//! Composing listing with ordinary per-stream exports does not establish one
//! database-wide view: writes between those operations can split a transaction
//! across the resulting backup. These traits alone therefore cannot guarantee
//! a consistent multi-stream backup while writers remain active.

use futures::Stream;
use mnesis::Version;

use crate::store::{RawEventStore, Store};
use crate::stream::EventStream;
use crate::stream_id::StreamKey;

/// Failure to open or use a consistent export view.
#[derive(Debug, thiserror::Error)]
pub enum ExportError<E> {
    /// Storage, admission or cleanup failed; preserve the adapter's cause.
    #[error("export storage operation failed: {0}")]
    Store(#[source] E),
    /// Zero, or a duration the adapter's monotonic clock cannot represent.
    #[error("invalid export lifetime: {lifetime:?}")]
    InvalidLifetime {
        /// The requested lifetime, unchanged.
        lifetime: core::time::Duration,
    },
    /// The fixed view's deadline has passed. Open a new session and restart.
    #[error("export session expired")]
    Expired,
}

/// Open one fixed database view for a consistent multi-stream backup.
///
/// The view is established during this operation, before it returns. Listing
/// and every subsequent session read must use that same view, including
/// streams opened after a concurrent cross-stream transaction commits.
pub trait ConsistentExporter: RawEventStore {
    /// Read-only view; it need not support writes, wakes or global reads.
    type Session<'a>: ExportSession<Error = Self::Error> + 'a
    where
        Self: 'a;

    /// Open a view with a nonzero, finite lifetime measured from view creation.
    ///
    /// Adapters reject zero or unrepresentable lifetimes with
    /// [`ExportError::InvalidLifetime`]. Dropping the opening future before
    /// admission creates no view. If canceled after view creation, cleanup must
    /// release that view. Expiration must release retained storage independently
    /// of further caller activity; synchronous work already executing may finish
    /// before cleanup runs. Adapters document their resource limits and costs.
    fn open_export_session(
        &self,
        lifetime: core::time::Duration,
    ) -> impl core::future::Future<Output = Result<Self::Session<'_>, ExportError<Self::Error>>> + Send;
}

/// A read-only, deadline-bound view shared by listing and all stream reads.
///
/// Session cursors borrow the session, so closing or dropping it requires
/// releasing those cursors first. Persisted envelopes already yielded are
/// owned values and remain valid afterward. Opening a cursor or requesting its
/// next item after expiration returns [`ExportError::Expired`]; an error ends
/// that cursor. Exhausted cursors remain exhausted. Consumers must treat any
/// error as an incomplete backup, rather than importing a truncated success.
///
/// One view includes either every event of a committed cross-stream transaction
/// or none of them. Streams created afterward are absent from listing and have
/// empty histories in this view. Existing per-stream [`EventExporter`] calls
/// retain their independent-read semantics.
pub trait ExportSession: Send + Sync {
    /// The adapter's original storage error, wrapped without erasing its type.
    type Error: core::error::Error + Send + Sync + 'static;
    /// A terminating stream of the ids in this view, in unspecified order.
    type StreamList<'a>: Stream<Item = Result<StreamKey, ExportError<Self::Error>>> + Send + 'a
    where
        Self: 'a;
    /// Stored envelopes in strictly increasing version order.
    type ExportStream<'a>: EventStream<Error = ExportError<Self::Error>> + 'a
    where
        Self: 'a;

    /// Enumerate only streams present when this view was established.
    fn list_streams(
        &self,
    ) -> impl core::future::Future<Output = Result<Self::StreamList<'_>, ExportError<Self::Error>>> + Send;

    /// Read `id` from `from` inclusive through its head in this view.
    /// `Version::INITIAL` reads its complete history; an absent stream is empty.
    fn export_stream(
        &self,
        id: &StreamKey,
        from: Version,
    ) -> impl core::future::Future<Output = Result<Self::ExportStream<'_>, ExportError<Self::Error>>>
    + Send;

    /// Release the view and wait for adapter cleanup before returning success.
    /// Expired views can also be closed; this operation requires no live cursor.
    fn close(
        self,
    ) -> impl core::future::Future<Output = Result<(), ExportError<Self::Error>>> + Send;
}

impl<S: ConsistentExporter> ConsistentExporter for Store<S> {
    type Session<'a>
        = S::Session<'a>
    where
        Self: 'a;

    async fn open_export_session(
        &self,
        lifetime: core::time::Duration,
    ) -> Result<Self::Session<'_>, ExportError<Self::Error>> {
        self.raw().open_export_session(lifetime).await
    }
}

/// Enumerate the stream ids present in a store.
///
/// The generic source of "which streams exist" — needed because a backup of
/// an arbitrary store doesn't know its ids up front, and `export_stream`
/// requires one. Yields the raw stream-id bytes (the form the store holds
/// them in); the caller reconstitutes a typed [`mnesis::Id`] if it needs one.
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
    ) -> impl core::future::Future<Output = Result<Self::StreamList, Self::Error>> + Send;
}

/// Export a single stream's events — a raw pass-through read.
///
/// `export_stream(id, from)` reads stream `id` from `from` **inclusive** (the
/// same semantics as [`RawEventStore::read_stream`]: `from = Version::INITIAL`
/// yields the whole stream from v1) up to its current head, then terminates.
/// Each yielded [`PersistedEnvelope`](crate::envelope::PersistedEnvelope) is the stored event **verbatim** — no
/// rewrite and no per-event global position or origin stream id.
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
/// Each call opens its own read; multiple calls do not share a consistent
/// view. A continuous/live export can compose with a subscription cursor.
/// The blanket impl makes every [`RawEventStore`] an `EventExporter`.
pub trait EventExporter: RawEventStore {
    /// The stream of exported events. Identical to the read stream — export
    /// performs no transform.
    type ExportStream: EventStream<Error = Self::Error> + 'static;

    /// Open a per-stream export of stream `id`, starting at `from` (inclusive).
    fn export_stream(
        &self,
        id: &StreamKey,
        from: Version,
    ) -> impl core::future::Future<Output = Result<Self::ExportStream, Self::Error>> + Send;
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
    ) -> impl core::future::Future<Output = Result<Self::ExportStream, Self::Error>> + Send {
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
