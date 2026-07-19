//! Subscription primitive: the user-facing handle that builds the generic
//! catch-up-then-live-tail loop.
//!
//! Users construct [`Subscription::new`] from a [`Store<S>`] and call
//! [`Subscription::subscribe`] / [`Subscription::subscribe_all`] to obtain a
//! `futures::Stream` cursor that **never terminates** — when caught up, it
//! waits for new events rather than yielding `None`. Users never name or touch
//! [`Arc`].
//!
//! # Shape
//!
//! `subscribe`/`subscribe_all` are **synchronous**: wake-registration can fail,
//! so they return `Result<impl Stream, _>` eagerly; read errors stream in-band
//! as `Err` items. Items are [`Step`]-tagged so a consumer can tell catch-up
//! from live; drop the phase with [`StepStreamExt::events`](crate::StepStreamExt)
//! or decode-and-keep-it with [`StepStreamExt::decoded`](crate::StepStreamExt).
//! The returned stream is `!Unpin` (it is the `futures::stream::unfold` of the
//! live loop), so consumers MUST `pin!` it before polling — the zero-cost
//! (no-`Box`) tradeoff.
//!
//! # Adapter authoring
//!
//! There is no adapter-facing subscription trait. An adapter need only
//! implement [`RawEventStore`] (the bounded scans) and
//! [`WakeSource`](crate::wake::WakeSource) (the live wake); the generic loop is
//! assembled here from [`StreamCatchup`] / [`AllCatchup`] + the internal
//! `live_stepped` loop, one monomorphized state machine per call site.

use alloc::sync::Arc;

use futures::StreamExt;
use mnesis::{Id, Version};

use crate::PersistedEnvelope;
use crate::catchup::{AllCatchup, StreamCatchup};
use crate::step::Step;
use crate::store::{RawEventStore, Store};
use crate::stream_id::StreamKey;
use crate::subscription_cursor::live_stepped;
use crate::wake::WakeSource;

/// User-facing subscription handle.
///
/// Holds a shared reference to a [`Store<S>`] backend (one `Arc` clone) and
/// exposes [`subscribe`](Self::subscribe) / [`subscribe_all`](Self::subscribe_all).
/// Cheap to construct; no `Arc` ever appears in user code.
///
/// # Example
///
/// ```ignore
/// use core::pin::pin;
/// use futures::StreamExt;
/// use mnesis_store::{Step, StepStreamExt, Store, Subscription};
///
/// let store = Store::new(FjallStore::builder("path").open()?);
/// // Items are `Step<PersistedEnvelope>`: tell catch-up from live directly.
/// let cursor = Subscription::new(&store).subscribe(&account_id, None)?;
/// let mut cursor = pin!(cursor);
/// while let Some(item) = cursor.next().await {
///     match item? {
///         Step::CaughtUp => { /* backlog drained — now live */ }
///         Step::Event(env) => { /* handle the raw envelope */ }
///     }
/// }
/// // …or `.events()` to drop the phase, `.decoded(codec)` to decode+keep it.
/// ```
pub struct Subscription<S> {
    store: Arc<S>,
}

impl<S> core::fmt::Debug for Subscription<S> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Subscription").finish_non_exhaustive()
    }
}

impl<S> Subscription<S> {
    /// Construct from a [`Store<S>`] handle. One `Arc::clone` per call.
    #[must_use]
    pub fn new(store: &Store<S>) -> Self {
        Self {
            store: Arc::clone(store.arc()),
        }
    }
}

impl<S: RawEventStore + WakeSource> Subscription<S> {
    /// Open a per-stream catch-up + live-tail subscription.
    ///
    /// `from: None` starts from version 1; `from: Some(v)` starts from the event
    /// *strictly after* version `v`. Items are
    /// [`Step<PersistedEnvelope>`](Step): the replay events, then exactly one
    /// [`Step::CaughtUp`] at the backlog→live boundary, then live events — the
    /// phase marker is intrinsic to a subscription (a finite read has none).
    /// Checkpoint by [`version()`](PersistedEnvelope::version) on each event.
    ///
    /// Compose from here: [`.events()`](crate::StepStreamExt::events) drops the
    /// phase for a consumer that only wants events (then the full
    /// [`DecodedStreamExt`](crate::DecodedStreamExt) applies);
    /// [`.decoded(codec)`](crate::StepStreamExt::decoded) keeps the phase and
    /// hands you typed `Step<Decoded<E>>`. The returned stream **never returns
    /// `None`** — it waits for new events when caught up — and is `!Unpin`, so
    /// `pin!` it before polling.
    ///
    /// # Errors
    ///
    /// `<S as WakeSource>::Error` if wake-registration fails. Read errors are
    /// surfaced as `Err` items in the stream.
    #[allow(
        clippy::type_complexity,
        reason = "the Step-tagged item is intrinsic to the contract; an alias would \
                  hide the `impl Stream`/`use<>` capture the API depends on"
    )]
    pub fn subscribe<I: Id>(
        &self,
        id: &I,
        from: Option<Version>,
    ) -> Result<
        impl futures_core::Stream<Item = Result<Step<PersistedEnvelope>, <S as RawEventStore>::Error>>
        + Send
        + use<S, I>,
        <S as WakeSource>::Error,
    >
    where
        <S as RawEventStore>::Stream: Unpin,
    {
        let catchup = StreamCatchup::new(Arc::clone(&self.store), id.as_ref())?;
        // The generic loop yields `Step<(Version, env)>`; a per-stream event's
        // position rides inside the envelope, so drop the tag from each `Event`.
        // `Step::map` carries the `CaughtUp` phase marker through untouched.
        Ok(live_stepped(catchup, from).map(|item| item.map(|step| step.map(|(_, env)| env))))
    }

    /// Open an all-streams (`$all`) catch-up + live-tail subscription in
    /// [`AllPosition`](crate::AllPosition) order.
    ///
    /// `from: None` starts from the first event ever appended; `from: Some(p)`
    /// starts from the event *strictly after* position `p`. Items are
    /// [`Step<(AllPosition, StreamKey, PersistedEnvelope)>`](Step): the replay
    /// events, then exactly one [`Step::CaughtUp`], then live events. Each event
    /// carries three parts beside the box: the **position** to checkpoint (the
    /// consumer hands it back here or to [`read_all`](RawEventStore::read_all)
    /// to resume; the checkpoint type is adapter-defined and must be
    /// serializable), the **stream key** to route on without decoding the
    /// payload, and the **envelope** for content.
    ///
    /// Compose with [`.events()`](crate::StepStreamExt::events) /
    /// [`.decoded(codec)`](crate::StepStreamExt::decoded) exactly as
    /// [`subscribe`](Self::subscribe). The returned stream **never returns
    /// `None`** and is `!Unpin`, so `pin!` it before polling.
    ///
    /// # Errors
    ///
    /// `<S as WakeSource>::Error` if wake-registration fails. Read errors are
    /// surfaced as `Err` items in the stream.
    #[allow(
        clippy::type_complexity,
        reason = "the position-tagged `$all` Step item is intrinsic to the contract; an \
                  alias would hide the `impl Stream`/`use<>` capture the API depends on"
    )]
    pub fn subscribe_all(
        &self,
        from: Option<<S as RawEventStore>::AllPosition>,
    ) -> Result<
        impl futures_core::Stream<
            Item = Result<
                Step<(
                    <S as RawEventStore>::AllPosition,
                    StreamKey,
                    PersistedEnvelope,
                )>,
                <S as RawEventStore>::Error,
            >,
        > + Send
        + use<S>,
        <S as WakeSource>::Error,
    >
    where
        <S as RawEventStore>::AllStream: Unpin,
    {
        let catchup = AllCatchup::new(Arc::clone(&self.store))?;
        Ok(live_stepped(catchup, from))
    }
}
