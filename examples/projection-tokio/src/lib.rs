//! A consumer-owned Tokio loop over an assembled projection.
//!
//! The projection owns its read model, validates event positions and persists
//! checkpoints conditionally. The caller assembles it with `Projection::load`
//! or explicitly starts a schema rebuild with `Projection::rebuild`. This loop
//! consumes decoded subscription items until shutdown, then flushes the tail.

use std::future::Future;

use futures::StreamExt;
use mnesis::{Id, Version};
use mnesis_store::checkpoint::CheckpointStore;
use mnesis_store::store::RawEventStore;
use mnesis_store::wake::WakeSource;
use mnesis_store::{
    DecodedStreamExt, OwningCodec, PersistTrigger, Projection, Projector, StepStreamExt,
    Subscription,
};

/// Drive an assembled [`Projection`] under tokio until `shutdown` resolves or
/// the stream ends.
///
/// The caller assembles the projection, then supplies the cursor and codec:
///
/// ```ignore
/// let projection =
///     Projection::load(id, projector, trigger, &checkpoints, schema).await?;
/// run_projection(projection, Subscription::new(&store), codec, shutdown).await?;
/// ```
///
/// 1. Subscribe from the stepper's `observed` position (the cursor never returns
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
/// checkpoint-commit failures via the boxed error, preserving each source chain.
pub async fn run_projection<I, P, Trig, SS, S, EC>(
    mut projection: Projection<I, P, Trig, SS>,
    subscription: Subscription<S>,
    codec: EC,
    shutdown: impl Future<Output = ()> + Send,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    I: Id,
    P: Projector,
    Trig: PersistTrigger,
    SS: CheckpointStore<P::State, Version>,
    S: RawEventStore + WakeSource,
    <S as RawEventStore>::Stream: Unpin,
    EC: OwningCodec<P::Event>,
{
    projection.state()?;
    // 1. Continue after the already-folded tail, including unpersisted events.
    //    The live loop's stream is `!Unpin`, so
    //    pin it before polling. `.events()` drops the phase marker; `.decoded()`
    //    reuses the codec and discharges the owning-codec bound here.
    let stream = subscription
        .subscribe(projection.id(), projection.observed())?
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
                projection.advance(item?).await?;
            }
        }
    }

    // 3. Flush the folded-but-unpersisted tail once.
    projection.flush().await?;
    Ok(())
}
