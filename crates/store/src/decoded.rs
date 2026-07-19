//! Consumer-side typed view over the raw envelope streams (#249).
//!
//! [`read_stream`](crate::RawEventStore::read_stream) and
//! [`read_all`](crate::RawEventStore::read_all) yield **raw**
//! [`PersistedEnvelope`]s, and [`Subscription`](crate::Subscription) yields
//! them wrapped in a [`Step`] phase marker — by design (the distributed
//! multi-consumer contract: each consumer holds its own codec). This module
//! adds an ergonomic layer on top — it does **not** make the core subscription
//! typed.
//!
//! - [`DecodedStreamExt::decoded`] — for **owning** codecs (JSON, bincode): a
//!   stream of carry-away [`Decoded<E>`] items.
//! - [`DecodedStreamExt::for_each_decoded`] — for **owning and zero-copy**
//!   codecs (rkyv, bytemuck): an internal-iteration fold that hands the borrowed
//!   window to a closure, so no lending stream is needed.
//! - [`StepStreamExt`] — the phase-aware surface over a `Step`-tagged
//!   subscription stream: [`.events()`](StepStreamExt::events) drops the phase
//!   (then the two `DecodedStreamExt` methods apply), or
//!   [`.decoded()`](StepStreamExt::decoded) decodes while **keeping** the phase.

use core::future::Future;

use bytes::Bytes;
use futures::{Stream, StreamExt};

use crate::codec::{Decode, OwningCodec};
use crate::envelope::PersistedEnvelope;
use crate::step::Step;
use crate::stream_id::StreamKey;
use mnesis::Version;

/// A raw envelope, un-packed: the decoded event plus its resume bookmark and
/// metadata. `T` is the owned event (`E`) on the stream path, or the borrowed
/// window (`Decode::Output`) inside a fold closure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded<T> {
    /// The decoded event (owned `E`, or a borrowed window).
    pub event: T,
    /// The per-stream version — the per-stream resume bookmark.
    pub version: Version,
    /// The event's metadata, if any (cheap Arc-shared bytes).
    pub metadata: Option<Bytes>,
}

/// A read from the raw stream failed, or the codec failed to decode the payload.
/// The two failure domains never share a variant (CLAUDE rule 3).
#[derive(Debug, thiserror::Error)]
pub enum DecodeStreamError<R, D> {
    /// The underlying raw stream yielded an error item.
    #[error("subscription stream read failed")]
    Read(#[source] R),
    /// The codec rejected the payload.
    #[error("event decode failed")]
    Decode(#[source] D),
}

/// [`DecodedStreamExt::for_each_decoded`] failure: a raw read, a decode, or the
/// consumer's own fold closure. Three distinct domains (CLAUDE rule 3).
#[derive(Debug, thiserror::Error)]
pub enum FoldDecodedError<R, D, H> {
    /// The underlying raw stream yielded an error item.
    #[error("subscription stream read failed")]
    Read(#[source] R),
    /// The codec rejected the payload.
    #[error("event decode failed")]
    Decode(#[source] D),
    /// The consumer's fold closure returned an error.
    #[error("decoded-event handler failed")]
    Handler(#[source] H),
}

mod sealed {
    pub trait Sealed {}
}

/// A raw stream item that carries a [`PersistedEnvelope`] and can be re-tagged
/// with its decoded counterpart.
///
/// Implemented for the two raw shapes the store yields:
/// - `PersistedEnvelope` (per-stream / `read_stream`) → typed item `Decoded<T>`;
///   the bookmark (`version`) lives **inside** the box.
/// - `(P, StreamKey, PersistedEnvelope)` (`$all` / `read_all`) → typed item
///   `(P, StreamKey, Decoded<T>)`; the `$all` position bookmark and the origin
///   stream key both stay **beside** the box, preserved through the decode.
///
/// The asymmetry is intentional (CLAUDE rule 4): each tag rides exactly
/// where it lives in the raw layer. Sealed — not implementable downstream.
pub trait RawItem: sealed::Sealed {
    /// The typed item once the envelope is decoded to `Decoded<T>`.
    type Typed<T>;
    /// Borrow the carried envelope (to decode it).
    fn envelope(&self) -> &PersistedEnvelope;
    /// Re-attach the decoded box into this item's shape (copying any tag).
    fn retag<T>(&self, decoded: Decoded<T>) -> Self::Typed<T>;
}

impl sealed::Sealed for PersistedEnvelope {}
impl RawItem for PersistedEnvelope {
    type Typed<T> = Decoded<T>;
    fn envelope(&self) -> &PersistedEnvelope {
        self
    }
    fn retag<T>(&self, decoded: Decoded<T>) -> Decoded<T> {
        decoded
    }
}

impl<P: Copy> sealed::Sealed for (P, StreamKey, PersistedEnvelope) {}
impl<P: Copy> RawItem for (P, StreamKey, PersistedEnvelope) {
    type Typed<T> = (P, StreamKey, Decoded<T>);
    fn envelope(&self) -> &PersistedEnvelope {
        &self.2
    }
    fn retag<T>(&self, decoded: Decoded<T>) -> (P, StreamKey, Decoded<T>) {
        (self.0, self.1.clone(), decoded)
    }
}

/// Adds a typed, codec-reusing view over any stream of raw envelope items.
///
/// Covers [`Subscription`](crate::Subscription),
/// [`read_stream`](crate::RawEventStore::read_stream), and
/// [`read_all`](crate::RawEventStore::read_all).
pub trait DecodedStreamExt<I, R>: Stream<Item = Result<I, R>> + Sized
where
    I: RawItem,
{
    /// Decode each item with `codec`, reusing the codec configured elsewhere.
    ///
    /// Owning codecs only — the `for<'a> Output<'a> = E` bound is unsatisfiable
    /// for a zero-copy codec (whose `Output` borrows the envelope), so the
    /// compiler steers zero-copy consumers to
    /// [`for_each_decoded`](Self::for_each_decoded).
    /// Per-stream items become `Decoded<E>`; `$all` items become
    /// `(AllPosition, StreamKey, Decoded<E>)` (both tags are preserved beside
    /// the box).
    fn decoded<E, C>(
        self,
        codec: C,
    ) -> impl Stream<Item = Result<I::Typed<E>, DecodeStreamError<R, C::Error>>> + Send
    where
        C: OwningCodec<E>,
        E: Send + 'static,
        I: Send + 'static,
        R: Send + 'static,
        Self: Send,
    {
        self.map(move |res| {
            let item = res.map_err(DecodeStreamError::Read)?;
            let event: E = codec
                .decode(item.envelope())
                .map_err(DecodeStreamError::Decode)?;
            let env = item.envelope();
            let decoded = Decoded {
                event,
                version: env.version(),
                metadata: env.metadata_bytes(),
            };
            Ok(item.retag(decoded))
        })
    }

    /// Fold each decoded event by handing your closure the borrowed window —
    /// works for **owning and zero-copy** codecs, because the window lives only
    /// for the call and never escapes (internal iteration; no lending stream).
    /// This is the path a zero-copy codec (rkyv, bytemuck) must take: its
    /// `Output` borrows the envelope and so cannot be carried away by
    /// [`decoded`](Self::decoded)'s stream.
    ///
    /// `f` receives a [`Decoded<Output<'a>>`] valid only for that call. On a
    /// never-ending [`Subscription`](crate::Subscription) this runs until the
    /// first `Err`; over a finite
    /// [`read_stream`](crate::RawEventStore::read_stream) it runs to completion.
    ///
    /// The closure argument is the concrete [`Decoded<Output<'a>>`] — the event
    /// view plus its per-stream `version` and `metadata`. It is deliberately
    /// **not** the `I::Typed<_>` shape [`decoded`](Self::decoded) yields: a bare
    /// closure cannot be inferred higher-ranked over a lifetime hidden behind
    /// the `I::Typed<_>` associated-type projection (rustc "implementation of
    /// `FnMut` is not general enough"), so a concrete outer constructor is
    /// required for the zero-copy path to type-check. Consequently, over an
    /// `$all` stream neither the `AllPosition` tag nor the [`StreamKey`] is
    /// surfaced to `f` (the per-stream `Decoded::version` still is) — a
    /// positioned or routed `$all` consumer must either use
    /// [`decoded`](Self::decoded) (owning codecs), or fold the raw
    /// `subscribe_all` stream directly, calling `codec.decode(&env)` per item
    /// (zero-copy; both tags ride beside the envelope on the raw tuple).
    fn for_each_decoded<E, C, F, H>(
        self,
        codec: C,
        mut f: F,
    ) -> impl Future<Output = Result<(), FoldDecodedError<R, C::Error, H>>>
    where
        E: ?Sized,
        C: Decode<E>,
        F: for<'a> FnMut(Decoded<<C as Decode<E>>::Output<'a>>) -> Result<(), H>,
    {
        async move {
            let stream = self;
            futures::pin_mut!(stream);
            while let Some(res) = stream.next().await {
                let item = res.map_err(FoldDecodedError::Read)?;
                fold_one(&codec, &mut f, item.envelope())?;
            }
            Ok(())
        }
    }
}

/// One decode-then-fold step: decode `env` with `codec`, box it as [`Decoded`],
/// and hand the (possibly borrowed) window to `f`. Kept a free, synchronous fn
/// so the higher-ranked `f` call over the borrowed window's lifetime resolves
/// in a plain fn body (see [`DecodedStreamExt::for_each_decoded`]).
fn fold_one<R, E, C, F, H>(
    codec: &C,
    f: &mut F,
    env: &PersistedEnvelope,
) -> Result<(), FoldDecodedError<R, C::Error, H>>
where
    E: ?Sized,
    C: Decode<E>,
    F: for<'a> FnMut(Decoded<<C as Decode<E>>::Output<'a>>) -> Result<(), H>,
{
    let window = codec.decode(env).map_err(FoldDecodedError::Decode)?;
    let decoded = Decoded {
        event: window,
        version: env.version(),
        metadata: env.metadata_bytes(),
    };
    f(decoded).map_err(FoldDecodedError::Handler)
}

impl<St, I, R> DecodedStreamExt<I, R> for St
where
    St: Stream<Item = Result<I, R>>,
    I: RawItem,
{
}

/// Adds phase-aware views over a [`Step`]-tagged stream — what
/// [`Subscription::subscribe`](crate::Subscription::subscribe) /
/// [`subscribe_all`](crate::Subscription::subscribe_all) yield.
///
/// The `Step` phase marker (the catch-up→live boundary) is intrinsic to a
/// subscription (a finite read has no such boundary), so it rides on the raw
/// stream. This trait lets a consumer either **keep** the phase and decode
/// (`.decoded()`), or **drop** it (`.events()`) and fall back to the plain
/// [`DecodedStreamExt`] surface a finite read uses.
///
/// `Step<I>` is deliberately **not** a [`RawItem`] (the [`CaughtUp`](Step::CaughtUp)
/// marker carries no envelope), so `.decoded()` here and `.decoded()` on
/// [`DecodedStreamExt`] are two non-overlapping impls sharing one name.
pub trait StepStreamExt<I, R>: Stream<Item = Result<Step<I>, R>> + Sized
where
    I: RawItem,
{
    /// Drop the phase: yield bare `I` items ([`CaughtUp`](Step::CaughtUp)
    /// removed, [`Event`](Step::Event) unwrapped). The result is a plain raw
    /// stream, so the full [`DecodedStreamExt`] surface (`.decoded()`,
    /// `.for_each_decoded()`) applies to it — the path for a consumer that does
    /// not care whether it is replaying or live.
    fn events(self) -> impl Stream<Item = Result<I, R>> + Send
    where
        Self: Send,
        I: Send,
        R: Send,
    {
        self.filter_map(|res| async move {
            match res {
                Ok(Step::Event(item)) => Some(Ok(item)),
                Ok(Step::CaughtUp) => None,
                Err(e) => Some(Err(e)),
            }
        })
    }

    /// Decode each event with `codec`, **preserving** the phase marker: the
    /// result is a stream of `Step<I::Typed<E>>` — replay events, then exactly
    /// one [`CaughtUp`](Step::CaughtUp), then live events. Owning codecs only
    /// (same `for<'a> Output<'a> = E` steer as [`DecodedStreamExt::decoded`]).
    ///
    /// This is the projection consumption path: it tells catch-up from live
    /// *and* hands you typed events, reusing the codec — no magic count, no
    /// hand-rolled timeout, mnesis-owned error.
    #[allow(
        clippy::type_complexity,
        reason = "the Step<Decoded>/DecodeStreamError item is intrinsic to the contract; an \
                  alias would hide the `impl Stream` the API depends on"
    )]
    fn decoded<E, C>(
        self,
        codec: C,
    ) -> impl Stream<Item = Result<Step<I::Typed<E>>, DecodeStreamError<R, C::Error>>> + Send
    where
        C: OwningCodec<E>,
        E: Send + 'static,
        I: Send + 'static,
        R: Send + 'static,
        Self: Send,
    {
        self.map(move |res| {
            let step = res.map_err(DecodeStreamError::Read)?;
            match step {
                Step::CaughtUp => Ok(Step::CaughtUp),
                Step::Event(item) => {
                    let env = item.envelope();
                    let event: E = codec.decode(env).map_err(DecodeStreamError::Decode)?;
                    let decoded = Decoded {
                        event,
                        version: env.version(),
                        metadata: env.metadata_bytes(),
                    };
                    Ok(Step::Event(item.retag(decoded)))
                }
            }
        })
    }
}

impl<St, I, R> StepStreamExt<I, R> for St
where
    St: Stream<Item = Result<Step<I>, R>>,
    I: RawItem,
{
}
