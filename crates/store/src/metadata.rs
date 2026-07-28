//! Write-path metadata production for the typed facade (#344).
//!
//! Metadata is attached to events as they are persisted through
//! [`Repository::save`](crate::Repository::save), [`EventStore::save_with`](crate::EventStore::save_with),
//! and all higher-level callers built on top of the facade (saga reactions,
//! command execution, snapshot decorator saves). The read side already exposes
//! `Option<bytes::Bytes>` on every envelope; the provider closes the write-side
//! gap without forcing callers to drop to [`RawEventStore`](crate::store::RawEventStore).

use crate::value::{Metadata, Payload};
use mnesis::Version;

/// Write-path metadata producer for events of type `E`.
///
/// A provider is called once per event, post-encode, with the version the event
/// will be assigned, a reference to the event, and the validated payload bytes
/// that will be persisted. It returns `Some(Metadata)` to attach metadata to the
/// envelope, or `None` to leave metadata absent.
///
/// The provider is infallible by design. Cap errors (e.g. `ValueError::MetadataTooLong`)
/// are the provider author's concern at `Metadata` construction; the facade
/// never re-validates an already-validated [`Metadata`] value.
///
/// # No-op default
///
/// The `()` impl always returns `None`. It is the inert default slot for the
/// `M = ()` type parameter on [`EventStore`](crate::EventStore) and
/// [`RepositoryBuilder`](crate::RepositoryBuilder), mirroring the role
/// [`NoSnapshot`](crate::NoSnapshot) plays on the snapshot axis.
///
/// # Closure blanket impl
///
/// A plain `Fn(Version, &E, &Payload) -> Option<Metadata>` closure implements
/// the trait, so callers can write `.metadata(|v, e, p| ...)` without naming
/// the trait explicitly.
///
/// # Stateful providers
///
/// The provider is called through `&self`; statefulness (for example an HLC
/// clock or a monotonic counter) uses interior mutability. This is the same
/// contract [`WakeSource`](crate::wake::WakeSource) carries: shared behind an
/// `Arc`, mutated via atomics or a mutex if needed.
///
/// # Documented tension: upcasting vs. byte-level signatures
///
/// Metadata is never upcasted. A signature over payload bytes couples signature
/// validity to the frozen payload encoding: if an upcaster rewrites the payload
/// on the read path, the signature stops verifying. Raw subscription paths see
/// pre-upcast bytes (verification works); the typed facade `load` path replays
/// typed events and the consumer never sees bytes. KERI-style bridges handle
/// schema evolution via digest chains, not byte stability.
pub trait MetadataProvider<E: ?Sized>: Send + Sync + 'static {
    /// Produce metadata for `event` at `version` with the validated `payload`
    /// bytes that will be persisted.
    fn metadata(&self, version: Version, event: &E, payload: &Payload) -> Option<Metadata>;
}

/// Inert metadata provider — always returns `None`.
///
/// This is the default `M = ()` slot on [`EventStore`](crate::EventStore) and
/// [`RepositoryBuilder`](crate::RepositoryBuilder), so existing callers that do
/// not call [`.metadata()`](RepositoryBuilder::metadata) behave exactly as
/// before: every event is persisted with metadata absent.
impl<E: ?Sized> MetadataProvider<E> for () {
    fn metadata(&self, _version: Version, _event: &E, _payload: &Payload) -> Option<Metadata> {
        None
    }
}

/// Closure blanket impl for `MetadataProvider`.
///
/// Lets callers pass a plain closure to `.metadata(|version, event, payload| ...)`
/// without naming the trait. The `Send + Sync + 'static` bounds are required so
/// the closure can be held by the facade alongside the codec.
impl<E: ?Sized, F> MetadataProvider<E> for F
where
    F: Fn(Version, &E, &Payload) -> Option<Metadata> + Send + Sync + 'static,
{
    fn metadata(&self, version: Version, event: &E, payload: &Payload) -> Option<Metadata> {
        self(version, event, payload)
    }
}
