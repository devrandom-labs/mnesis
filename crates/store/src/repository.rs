// `try_fold` closure bodies clone Arc-wrapped codec/upcast captures from outer
// scope and re-bind them by the same name — clippy flags as `shadow_reuse`,
// but the rebinding is idiomatic for per-iteration Arc clones in async
// combinator chains and renaming everywhere would just add noise.
#![allow(
    clippy::shadow_reuse,
    reason = "per-iteration Arc clones in try_fold closures intentionally re-bind"
)]

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::borrow::Borrow;
use core::future::Future;
use core::marker::PhantomData;

use mnesis::{Aggregate, AggregateRoot, DomainEvent, EventOf, Events, Version};

use futures::TryStreamExt;

use crate::codec::{Decode, Encode};
use crate::envelope::{EnvelopeError, PendingBatch, PersistedEnvelope, pending_envelope};
use crate::error::{AppendError, LoadWithError, StoreError};
use crate::metadata::MetadataProvider;
use crate::store::{AllPosition, RawEventStore, Store};
use crate::stream_id::StreamKey;
use crate::upcasting::EventMorsel;
use crate::value::{Payload, SchemaVersion};

// ═══════════════════════════════════════════════════════════════════════════
// Repository<A> — high-level aggregate facade (load + save)
// ═══════════════════════════════════════════════════════════════════════════

/// Port for loading and saving aggregates via event streams.
///
/// Implementations handle codec encode/decode, streaming rehydration
/// via [`AggregateRoot::replay()`], and version tracking internally.
/// Users interact with aggregates, not envelopes.
///
/// # Stream identity
///
/// The aggregate's `Id` (via `Aggregate::Id`) is used directly as the
/// stream identifier. Adapters are responsible for mapping the `Id` to
/// their internal key format (e.g. string-based key, numeric ID, etc.).
///
/// # Streaming Rehydration
///
/// `load()` streams events from the store one-by-one through `replay()`,
/// enabling zero-allocation rehydration with zero-copy codecs (rkyv,
/// flatbuffers). No intermediate `Vec` allocation is needed.
///
/// # Save contract
///
/// `save()` takes a mutable reference to the aggregate and the
/// non-empty [`Events<E, N>`](mnesis::Events) decided by
/// [`Handle::handle()`](mnesis::Handle::handle). It encodes the events,
/// appends them atomically using `aggregate.version()` as the expected
/// version, and on success calls `commit_persisted` to advance the version
/// and fold the events into in-memory state atomically.
///
/// Taking `&Events<E, N>` (not `&[EventOf<A>]`) carries the kernel's
/// `>= 1` guarantee through to persistence: an empty save is
/// unrepresentable, so there is no runtime no-op case to guard.
///
/// # Schema evolution
///
/// The trait surface does not carry an upcaster — `load()` reads events
/// at their stored schema version and decodes them directly, while `save()`
/// stamps `Version::INITIAL` as the schema version on each new event. For
/// schema evolution, drop to the concrete facade and call its inherent
/// [`load_with`](EventStore::load_with) /
/// [`save_with`](EventStore::save_with) methods (or compose the
/// substrate via [`Store::raw`](crate::Store::raw)).
///
/// # Error handling
///
/// Implementations must bridge errors from four sources:
/// - [`RawEventStore`] errors (I/O, conflicts)
/// - [`Encode`] errors (serialization failures on write)
/// - [`Decode`] errors (deserialization failures on read)
/// - [`KernelError`](mnesis::KernelError) (version mismatch during replay)
///
/// [`StoreError`] can represent all four via its
/// `Adapter`, `Encode`, `Decode`, and `Kernel` variants. Use `StoreError`
/// as `Self::Error` or define a custom error with `From` impls.
pub trait Repository<A: Aggregate>: Send + Sync {
    /// The error type for repository operations.
    type Error: core::error::Error + Send + Sync + 'static;

    /// The `$all` position [`save`](Self::save) returns — the adapter's
    /// [`AllPosition`], surfaced up from
    /// [`RawEventStore::append`] (#330).
    ///
    /// This is the read-your-writes token: a projection whose checkpoint has
    /// reached a returned position has necessarily observed the write. On a
    /// distributed adapter (postgres) the position may be withheld from `$all`
    /// until a commit watermark clears (#213), so any wait needs a timeout.
    type Position: AllPosition;

    /// Load an aggregate by replaying its event stream.
    ///
    /// Streams events from the store one-by-one through `replay()`,
    /// enabling zero-allocation rehydration with zero-copy codecs.
    /// Returns a fresh aggregate at initial state if the stream is empty.
    fn load(&self, id: A::Id)
    -> impl Future<Output = Result<AggregateRoot<A>, Self::Error>> + Send;

    /// Persist decided events and advance the aggregate's in-memory state.
    ///
    /// `events` is the non-empty [`Events<E, N>`](mnesis::Events) decided by
    /// [`Handle::handle()`](mnesis::Handle::handle). The aggregate's
    /// current [`version()`](AggregateRoot::version) is used as the
    /// expected version for optimistic concurrency.
    ///
    /// The `&Events<EventOf<A>, N>` parameter guarantees at least one
    /// event at compile time — there is no empty-input case.
    ///
    /// Checks the whole batch version range before encoding or persistence.
    /// On success, calls `commit_persisted` with the persisted events to
    /// advance the version and fold the events into in-memory state atomically,
    /// and returns the [`Position`](Self::Position) the last event landed at —
    /// the read-your-writes token (#330). The advanced version is read off
    /// `aggregate`; only the position, which the aggregate does not carry, is
    /// returned (rule 4 — no redundant `(version, position)` pair).
    fn save<const N: usize>(
        &self,
        aggregate: &mut AggregateRoot<A>,
        events: &Events<EventOf<A>, N>,
    ) -> impl Future<Output = Result<Self::Position, Self::Error>> + Send;
}

// ═══════════════════════════════════════════════════════════════════════════
// ReplayFrom<A> — pub(crate) trait shared with the Snapshotting decorator
// ═══════════════════════════════════════════════════════════════════════════

/// Internal trait for replaying events from a given starting point.
///
/// [`EventStore`] implements this so the
/// [`Snapshotting`](crate::snapshot::Snapshotting) decorator can share
/// replay logic. Not public API.
pub(crate) trait ReplayFrom<A: Aggregate>: Send + Sync {
    /// The error type for replay operations.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Replay events starting from `from` version (inclusive) into `root`.
    ///
    /// Returns the updated aggregate with all events applied.
    fn replay_from(
        &self,
        root: AggregateRoot<A>,
        from: Version,
    ) -> impl Future<Output = Result<AggregateRoot<A>, Self::Error>> + Send;
}

// ═══════════════════════════════════════════════════════════════════════════
// Shared helpers
// ═══════════════════════════════════════════════════════════════════════════

/// The first [`Version`] an append will assign, given the stream's current
/// version (`None` = empty stream). Returns `None` on overflow past `u64::MAX`.
///
/// Single source of truth for the "next version to write" computation shared by
/// the aggregate save paths and the saga repository's intent-version pinning —
/// keeps the arithmetic checked in exactly one place (CLAUDE.md rule 2).
pub(crate) const fn first_persisted_version(current: Option<Version>) -> Option<Version> {
    match current {
        None => Some(Version::INITIAL),
        Some(v) => v.next(),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// EventStore — one facade for any codec (owning or borrowing)
// ═══════════════════════════════════════════════════════════════════════════

/// Event store over a single [`Encode`] + [`Decode`] codec — one terminal for
/// both owning and borrowing codecs.
///
/// The owning-vs-borrowing distinction is inferred from the codec's
/// [`Decode::Output`] GAT, not restated at the call
/// site: an owning codec (`Output<'a> = E`, e.g. serde — one allocation per
/// decoded event) and a borrowing codec (`Output<'a> = &'a E`, e.g. a
/// `#[repr(C)]` POD reinterpret — zero allocation) are unified on the load
/// path by the bound `Output<'a>: Borrow<E>` (`std`'s `Borrow<T> for T` and
/// `Borrow<T> for &T` cover both), and the decoded value is fed to
/// [`replay`](mnesis::AggregateRoot::replay) via `out.borrow()` in either case.
///
/// # Construction
///
/// Created via [`Store::repository::<A>()`](crate::store::Store::repository),
/// which names the aggregate `A` once:
///
/// ```ignore
/// let store = Store::new(backend);
/// let orders = store.repository::<Order>().codec(OrderCodec).build();
/// let order = orders.load(id).await?;        // AggregateRoot<Order> — inferred
/// orders.save(&mut order, &events).await?;   // inferred
/// ```
///
/// # Aggregate binding
///
/// The aggregate `A` is a phantom type parameter (carried as
/// `PhantomData<fn() -> A>`, so the facade is `Send + Sync + 'static`
/// regardless of `A` and stays covariant in it). It exists solely so the
/// facade implements [`Repository<A>`] for **exactly one** `A`: with `A`
/// fixed on the type, `load(id)` / `save(..)` infer the aggregate from the
/// receiver, with no per-call annotation (the blanket-over-`A` impl that
/// previously defeated inference is gone). `A` is named once, at
/// `repository::<A>()`. The substrate [`Store<S>`] remains multi-aggregate;
/// mint one cheap per-aggregate facade per aggregate type.
///
/// # Schema evolution
///
/// The plain [`load`](Repository::load) / [`save`](Repository::save) path
/// performs no upcasting. For schema evolution, call
/// [`load_with`](Self::load_with) with the macro-generated function
/// (e.g. `OrderTransforms::upcast`) on the read path, and
/// [`save_with`](Self::save_with) with `OrderTransforms::current_version`
/// on the write path:
///
/// ```ignore
/// // Read path:
/// let root = es.load_with(id, |_| Ok::<_, core::convert::Infallible>(()), OrderTransforms::upcast).await?;
///
/// // Write path:
/// es.save_with(&mut root, &events, OrderTransforms::current_version).await?;
/// ```
///
/// # Internal ownership
///
/// Owns the codec as `Arc<C>` and the metadata provider as `Arc<M>` so async
/// load paths can clone both handles into combinator closures and capture them
/// by value. Per Rust 2024's stricter capture rules (RFC 3498, rustc issue
/// 133529), a closure that borrows from `&self` and is then handed to a
/// `try_fold`-style combinator whose returned future is `+ Send` cannot satisfy
/// the bound — the future-Send check effectively requires the borrow to be
/// `'static`. Owning the components via `Arc` and cloning per call sidesteps the
/// borrow entirely. Cost: one heap allocation at facade construction, one
/// pointer bump per `load`.
///
/// The `M = ()` default keeps every existing call site compiling unchanged; the
/// inert provider always returns `None` metadata.
pub struct EventStore<S, C, A, M = ()> {
    store: Store<S>,
    codec: Arc<C>,
    meta: Arc<M>,
    _aggregate: PhantomData<fn() -> A>,
}

impl<S, C, A, M> EventStore<S, C, A, M> {
    /// Create an event store bound to a shared store, codec, and metadata
    /// provider for aggregate `A`.
    pub(crate) fn new(store: Store<S>, codec: C, meta: M) -> Self {
        Self {
            store,
            codec: Arc::new(codec),
            meta: Arc::new(meta),
            _aggregate: PhantomData,
        }
    }
}

impl<S, C, A, M> ReplayFrom<A> for EventStore<S, C, A, M>
where
    A: Aggregate,
    S: RawEventStore + 'static,
    for<'a> C: Encode<EventOf<A>> + Decode<EventOf<A>, Output<'a>: Borrow<EventOf<A>>> + 'static,
    EventOf<A>: DomainEvent,
    S::Stream: Send,
    M: Send + Sync + 'static,
{
    type Error =
        StoreError<S::Error, <C as Encode<EventOf<A>>>::Error, <C as Decode<EventOf<A>>>::Error>;

    #[cfg_attr(
        feature = "tracing",
        tracing::instrument(
            name = "mnesis.aggregate.load",
            level = "debug",
            skip_all,
            fields(
                aggregate = core::any::type_name::<A>(),
                stream = %root.id(),
                from = %from,
                version = tracing::field::Empty
            )
        )
    )]
    async fn replay_from(
        &self,
        root: AggregateRoot<A>,
        from: Version,
    ) -> Result<AggregateRoot<A>, Self::Error> {
        // Clone everything into function-local owned values. The
        // combinator closure captures the locals (Arc clones), with no
        // borrow of `&self`. See the doc comment on `EventStore` for the
        // full Rust 2024 capture-rules rationale.
        let store = self.store.clone();
        let codec = Arc::<C>::clone(&self.codec);

        let raw_stream = store
            .raw()
            .read_stream(&StreamKey::from_slice(root.id().as_ref()), from)
            .await
            .map_err(StoreError::Adapter)?;

        let loaded = raw_stream
            .map_err(StoreError::Adapter)
            .try_fold(root, move |mut r, env| {
                let codec = Arc::<C>::clone(&codec);
                async move {
                    let version = env.version();
                    // `out` is the codec's Output<'a>: either an owned
                    // `EventOf<A>` or a `&EventOf<A>`. `.borrow()` yields
                    // `&EventOf<A>` in both arms (std Borrow blanket impls),
                    // and is consumed in-place by `replay` so it never
                    // escapes (avoids the GAT `'static` implication).
                    let out = <C as Decode<EventOf<A>>>::decode(&codec, &env)
                        .map_err(StoreError::Decode)?;
                    r.replay(version, out.borrow())?;
                    Ok(r)
                }
            })
            .await?;
        #[cfg(feature = "tracing")]
        tracing::Span::current().record("version", tracing::field::debug(&loaded.version()));
        Ok(loaded)
    }
}

impl<S, C, A, M> Repository<A> for EventStore<S, C, A, M>
where
    A: Aggregate,
    S: RawEventStore + 'static,
    for<'a> C: Encode<EventOf<A>> + Decode<EventOf<A>, Output<'a>: Borrow<EventOf<A>>> + 'static,
    EventOf<A>: DomainEvent,
    S::Stream: Send,
    M: MetadataProvider<EventOf<A>>,
{
    type Error =
        StoreError<S::Error, <C as Encode<EventOf<A>>>::Error, <C as Decode<EventOf<A>>>::Error>;

    type Position = S::AllPosition;

    async fn load(&self, id: A::Id) -> Result<AggregateRoot<A>, Self::Error> {
        let root = AggregateRoot::<A>::new(id);
        self.replay_from(root, Version::INITIAL).await
    }

    async fn save<const N: usize>(
        &self,
        aggregate: &mut AggregateRoot<A>,
        events: &Events<EventOf<A>, N>,
    ) -> Result<Self::Position, Self::Error> {
        // The no-upcaster save stamps SchemaVersion::INITIAL as the schema
        // version on every event — the schema-version-lookup function
        // is only needed when an upcaster is in play. See `save_with`.
        save_events::<A, S, C, _, M, N>(self, aggregate, events, |_| None).await
    }
}

impl<S, C, A, M> EventStore<S, C, A, M> {
    /// Load an aggregate, running `upcast` over each persisted event
    /// before decoding it. `verify_original` runs first against the exact
    /// persisted envelope, including its original schema, payload and metadata.
    /// An authentication failure prevents both transformation and decoding.
    ///
    /// For trusted, unauthenticated input, explicitly pass
    /// `|_| Ok::<_, core::convert::Infallible>(())`. Metadata retained after a
    /// transformation describes the original event; its signature does not
    /// authenticate the transformed bytes. Decoders should deserialize the
    /// transformed view, with authentication handled by `verify_original`.
    ///
    /// `upcast` is the schema-evolution function — typically the
    /// associated function the `#[mnesis::transforms]` macro emits
    /// (e.g. `OrderTransforms::upcast`). Pass it directly as a function
    /// pointer; the `'static` bound on `F` and the `+ Send + Sync` bounds
    /// are required by the `try_fold` combinator chain (see the doc
    /// comment on [`EventStore`] for the full Rust 2024 capture-rules
    /// rationale).
    ///
    /// # Errors
    ///
    /// Returns [`LoadWithError::Store`] for adapter, codec or kernel errors
    /// and [`LoadWithError::Upcast`] for any
    /// error returned by the `upcast` function. [`LoadWithError::Verification`]
    /// preserves the original verifier error separately.
    #[allow(
        clippy::type_complexity,
        reason = "the independent LoadWithError source types are intrinsic to the contract; an alias would \
                  hide which domains the upcasting read path can fail from"
    )]
    #[cfg_attr(
        feature = "tracing",
        tracing::instrument(
            name = "mnesis.aggregate.load",
            level = "debug",
            skip_all,
            fields(
                aggregate = core::any::type_name::<A>(),
                stream = %id,
                from = %Version::INITIAL,
                version = tracing::field::Empty
            )
        )
    )]
    pub async fn load_with<F, E, V, VE>(
        &self,
        id: A::Id,
        verify_original: V,
        upcast: F,
    ) -> Result<
        AggregateRoot<A>,
        LoadWithError<
            S::Error,
            <C as Encode<EventOf<A>>>::Error,
            <C as Decode<EventOf<A>>>::Error,
            E,
            VE,
        >,
    >
    where
        A: Aggregate,
        S: RawEventStore + 'static,
        for<'a> C:
            Encode<EventOf<A>> + Decode<EventOf<A>, Output<'a>: Borrow<EventOf<A>>> + 'static,
        F: for<'a> Fn(EventMorsel<'a>) -> Result<EventMorsel<'a>, E> + Send + Sync + 'static,
        E: core::error::Error + Send + Sync + 'static,
        V: Fn(&PersistedEnvelope) -> Result<(), VE> + Send + Sync + 'static,
        VE: core::error::Error + Send + Sync + 'static,
        EventOf<A>: DomainEvent,
        S::Stream: Send,
        M: Send + Sync + 'static,
    {
        let store = self.store.clone();
        let codec = Arc::<C>::clone(&self.codec);
        let root = AggregateRoot::<A>::new(id);

        let raw_stream = store
            .raw()
            .read_stream(&StreamKey::from_slice(root.id().as_ref()), Version::INITIAL)
            .await
            .map_err(|e| LoadWithError::Store(StoreError::Adapter(e)))?;

        let verify_original = Arc::new(verify_original);
        let upcast = Arc::new(upcast);
        let loaded = raw_stream
            .map_err(|e| LoadWithError::Store(StoreError::Adapter(e)))
            .try_fold(root, move |mut r, env| {
                let codec = Arc::<C>::clone(&codec);
                let upcast = Arc::<F>::clone(&upcast);
                let verify_original = Arc::<V>::clone(&verify_original);
                async move {
                    verify_original(&env).map_err(LoadWithError::Verification)?;
                    let version = env.version();
                    let morsel = EventMorsel::borrowed(
                        env.event_type(),
                        env.schema_version_value(),
                        env.payload(),
                    );
                    let transformed = upcast(morsel).map_err(LoadWithError::Upcast)?;
                    let upcast_env = env
                        .for_transformed_decode(
                            transformed.event_type(),
                            transformed.schema_version(),
                            transformed.payload(),
                        )
                        .map_err(|e| LoadWithError::Store(StoreError::EnvelopeSynthesis(e)))?;
                    let out = <C as Decode<EventOf<A>>>::decode(&codec, &upcast_env)
                        .map_err(|e| LoadWithError::Store(StoreError::Decode(e)))?;
                    r.replay(version, out.borrow())
                        .map_err(|e| LoadWithError::Store(StoreError::Kernel(e)))?;
                    Ok(r)
                }
            })
            .await?;
        #[cfg(feature = "tracing")]
        tracing::Span::current().record("version", tracing::field::debug(&loaded.version()));
        Ok(loaded)
    }

    /// Persist decided events, stamping the schema version on each via
    /// `current_version`.
    ///
    /// `current_version` is typically the associated function the
    /// `#[mnesis::transforms]` macro emits (e.g.
    /// `OrderTransforms::current_version`). For event types it doesn't
    /// know about, it returns `None` and the schema version falls back
    /// to [`SchemaVersion::INITIAL`] (the same default as the no-upcaster
    /// [`save`](Repository::save)).
    ///
    /// Returns the [`Position`](Repository::Position) the last event landed at,
    /// exactly as [`save`](Repository::save) does (#330).
    ///
    /// # Errors
    ///
    /// The same set of errors [`save`](Repository::save) can produce —
    /// the schema-version lookup itself is infallible.
    pub async fn save_with<F, const N: usize>(
        &self,
        aggregate: &mut AggregateRoot<A>,
        events: &Events<EventOf<A>, N>,
        current_version: F,
    ) -> Result<
        S::AllPosition,
        StoreError<S::Error, <C as Encode<EventOf<A>>>::Error, <C as Decode<EventOf<A>>>::Error>,
    >
    where
        A: Aggregate,
        S: RawEventStore + 'static,
        C: Encode<EventOf<A>> + Decode<EventOf<A>> + 'static,
        F: Fn(&str) -> Option<SchemaVersion>,
        EventOf<A>: DomainEvent,
        M: MetadataProvider<EventOf<A>>,
    {
        save_events::<A, S, C, _, M, N>(self, aggregate, events, current_version).await
    }
}

// Single save path shared between Repository::save (no upcaster, always stamps
// SchemaVersion::INITIAL) and EventStore::save_with (uses the user's current_version
// fn). Encode-only — the decode shape is irrelevant on the write path, so this
// serves owning and borrowing codecs alike.
#[allow(
    clippy::type_complexity,
    reason = "the three-source StoreError return is intrinsic to the contract; an alias would hide \
              which domains the save path can fail from"
)]
#[cfg_attr(
    feature = "tracing",
    tracing::instrument(
        name = "mnesis.aggregate.save",
        level = "debug",
        skip_all,
        fields(
            aggregate = core::any::type_name::<A>(),
            stream = %aggregate.id(),
            events = events.len(),
            expected = ?aggregate.version(),
            position = tracing::field::Empty
        )
    )
)]
async fn save_events<A, S, C, F, M, const N: usize>(
    facade: &EventStore<S, C, A, M>,
    aggregate: &mut AggregateRoot<A>,
    events: &Events<EventOf<A>, N>,
    current_version: F,
) -> Result<
    S::AllPosition,
    StoreError<S::Error, <C as Encode<EventOf<A>>>::Error, <C as Decode<EventOf<A>>>::Error>,
>
where
    A: Aggregate,
    S: RawEventStore,
    C: Encode<EventOf<A>> + Decode<EventOf<A>>,
    F: Fn(&str) -> Option<SchemaVersion>,
    M: MetadataProvider<EventOf<A>>,
    EventOf<A>: DomainEvent,
{
    aggregate
        .commit_version(events)
        .map_err(|error| match error {
            mnesis::KernelError::VersionOverflow => StoreError::VersionOverflow,
            cause => StoreError::Kernel(cause),
        })?;
    let expected_version = aggregate.version();

    let mut next_version =
        first_persisted_version(expected_version).ok_or(StoreError::VersionOverflow)?;

    // Encode head and tail separately so the non-emptiness `Events` guarantees
    // survives into `PendingBatch` — `from_parts` needs no runtime check and no
    // unprovable `unwrap` (#330). Scoped in a block so the `current_version`
    // closure (which is not `Send`) is dropped before the append `.await` —
    // otherwise the returned future would capture it across the await point and
    // stop being `Send` (clippy `future_not_send`).
    let (head, tail) = {
        let encode_at = |event: &EventOf<A>, version: Version| {
            let payload_bytes = <C as Encode<EventOf<A>>>::encode(&facade.codec, event)
                .map_err(StoreError::Encode)?;
            let payload = Payload::from_bytes(payload_bytes)
                .map_err(EnvelopeError::from)
                .map_err(StoreError::from)?;
            let schema_version = current_version(event.name()).unwrap_or(SchemaVersion::INITIAL);

            let metadata = facade.meta.metadata(version, event, &payload);

            let builder = pending_envelope(version)
                .event(event)
                .payload(payload.into_bytes())
                .schema_version(schema_version);
            match metadata {
                Some(m) => builder.metadata(m.into_bytes()).build(),
                None => builder.build(),
            }
            .map_err(StoreError::from)
        };

        let head = encode_at(events.first(), next_version)?;
        let mut tail = Vec::with_capacity(events.rest().len());
        for event in events.rest() {
            next_version = next_version.next().ok_or(StoreError::VersionOverflow)?;
            tail.push(encode_at(event, next_version)?);
        }
        (head, tail)
    };

    let position = facade
        .store
        .raw()
        .append(
            &StreamKey::from_slice(aggregate.id().as_ref()),
            expected_version,
            PendingBatch::from_parts(&head, &tail),
        )
        .await
        .map_err(|err| match err {
            AppendError::Conflict {
                stream_id,
                expected,
                actual,
            } => StoreError::Conflict {
                stream_id,
                expected,
                actual,
            },
            AppendError::Store(e) => StoreError::Adapter(e),
        })?;

    #[cfg(feature = "tracing")]
    tracing::Span::current().record("position", tracing::field::debug(&position));

    aggregate.commit_persisted(events)?;
    Ok(position)
}

#[cfg(test)]
mod version_helper_tests {
    use super::first_persisted_version;
    use mnesis::Version;

    #[test]
    fn fresh_stream_starts_at_initial() {
        assert_eq!(first_persisted_version(None), Some(Version::INITIAL));
    }

    #[test]
    fn existing_stream_advances_by_one() {
        let v = Version::INITIAL;
        assert_eq!(first_persisted_version(Some(v)), v.next());
    }

    #[test]
    fn overflow_at_max_returns_none() {
        let max = Version::new(u64::MAX).expect("u64::MAX is non-zero");
        assert_eq!(first_persisted_version(Some(max)), None);
    }
}
