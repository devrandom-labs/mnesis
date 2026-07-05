//! Consumer-owned projection loop over `nexus_store` primitives.
//!
//! nexus deliberately ships **no** event-loop runner — that is runtime,
//! and the runtime is the consumer's. nexus ships the pure primitives: a
//! [`Projector`] (how to fold), a [`PersistTrigger`] (when to persist), a
//! [`Subscription`] (the cursor), a [`SnapshotStore`] (atomic
//! `(state, position)` commit), and — assembling the last three around the
//! first — the inert [`Projection`] stepper. This function is one concrete
//! loop under tokio; the Agency/Bombay actor framework writes its own loop —
//! a Zenoh-native actor whose mailbox *is* the loop — calling the same
//! `Projection::advance`. Two loops, no shared loop code, nothing to drift.
//!
//! ## What the stepper removed (issue #255)
//!
//! Wiring the four primitives by hand used to be an 8-argument, 6-generic
//! function that made the consumer restate the owning-codec
//! `for<'a> Decode<…, Output<'a> = E>` bound — enough friction that the old
//! version needed a `too_many_arguments` allow. Now:
//!
//! - The **assembly** is [`Projection::load`] — five named inputs, no codec,
//!   no `for<'a>`, no lint. It hands back the stepper and the starting state.
//! - The **codec** is discharged by [`.decoded()`](StepStreamExt) *before* an
//!   event reaches the stepper, so it never enters the fold-side generics.
//! - The one remaining codec bound rides the [`OwningCodec`] alias
//!   (serde's `DeserializeOwned` trick), so even this generic loop never
//!   spells `for<'a>`.

use std::future::Future;

use futures::StreamExt;
use nexus::{Id, Version};
use nexus_store::state::SnapshotStore;
use nexus_store::store::RawEventStore;
use nexus_store::wake::WakeSource;
use nexus_store::{
    DecodedStreamExt, OwningCodec, PersistTrigger, Projection, Projector, StepStreamExt,
    Subscription,
};

/// Drive an assembled [`Projection`] under tokio until `shutdown` resolves or
/// the stream ends.
///
/// The caller assembles the projection and its starting state with
/// [`Projection::load`], then hands both here along with the cursor and codec:
///
/// ```ignore
/// let (proj, state) =
///     Projection::load(id, projector, trigger, &snapshots, schema).await?;
/// run_projection(proj, state, Subscription::new(&store), codec, shutdown).await?;
/// ```
///
/// 1. Subscribe from the stepper's `checkpoint` (the cursor never returns
///    `None`); drop the catch-up→live phase marker with `.events()` (a
///    projection consumes events, it does not branch on the phase) and decode
///    each with `.decoded(codec)`.
/// 2. For each decoded event, `advance` folds it and commits if the trigger
///    fires.
/// 3. On shutdown, `flush` commits any folded-but-unpersisted tail once.
///
/// # Errors
///
/// Propagates subscription-register, stream-read/decode, projector-apply, and
/// snapshot-commit failures via the boxed error; each preserves its `#[source]`
/// chain in `to_string()`.
pub async fn run_projection<I, P, Trig, SS, S, EC>(
    mut projection: Projection<I, P, Trig, SS>,
    mut state: P::State,
    subscription: Subscription<S>,
    codec: EC,
    shutdown: impl Future<Output = ()> + Send,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    I: Id,
    P: Projector,
    Trig: PersistTrigger,
    SS: SnapshotStore<P::State, Version>,
    S: RawEventStore + WakeSource,
    <S as RawEventStore>::Stream: Unpin,
    EC: OwningCodec<P::Event>,
{
    // 1. Subscribe from the checkpoint. The live loop's stream is `!Unpin`, so
    //    pin it before polling. `.events()` drops the phase marker; `.decoded()`
    //    reuses the codec and discharges the owning-codec bound here.
    let stream = subscription
        .subscribe(projection.id(), projection.checkpoint())?
        .events()
        .decoded(codec);
    tokio::pin!(stream);
    tokio::pin!(shutdown);

    // 2. Drive until shutdown or stream end.
    loop {
        tokio::select! {
            () = &mut shutdown => break,
            next = stream.next() => {
                let Some(item) = next else { break };
                state = projection.advance(state, item?).await?;
            }
        }
    }

    // 3. Flush the folded-but-unpersisted tail once.
    projection.flush(&state).await?;
    Ok(())
}
