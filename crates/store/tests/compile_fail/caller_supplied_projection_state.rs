//! An old or fabricated state cannot be supplied to advance or flush.
use mnesis::{DomainEvent, Message, Version};
use mnesis_store::checkpoint::CheckpointStore;
use mnesis_store::{AfterEventTypes, Decoded, Projection, Projector, StreamKey};

#[derive(Debug)]
struct Event;
impl Message for Event {}
impl DomainEvent for Event {
    fn name(&self) -> &'static str { "event" }
}
struct Count;
impl Projector for Count {
    type Event = Event;
    type State = u64;
    type Error = core::convert::Infallible;
    fn initial(&self) -> u64 { 0 }
    fn apply(&self, state: u64, _: &Event) -> Result<u64, Self::Error> { Ok(state) }
}

async fn supply_old_state<C: CheckpointStore<u64, Version>>(
    projection: &mut Projection<StreamKey, Count, AfterEventTypes, C>,
    old_state: u64,
    item: Decoded<Event>,
) {
    let _ = projection.advance(item, old_state).await;
    let _ = projection.flush(&old_state).await;
}

fn main() {}
