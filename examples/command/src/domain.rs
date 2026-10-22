use std::convert::Infallible;

use mnesis::{Aggregate, AggregateState, DomainEvent, Events, Handle, Message, events};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Value(pub u64);
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Changed(pub u64);
impl Message for Changed {}
impl DomainEvent for Changed {
    fn name(&self) -> &'static str {
        "value.changed"
    }
}
impl AggregateState for Value {
    type Event = Changed;
    fn initial() -> Self {
        Self(0)
    }
    fn apply(self, event: &Changed) -> Self {
        Self(event.0)
    }
}
pub struct Register;
impl Aggregate for Register {
    type State = Value;
    type Error = Infallible;
    type Id = String;
}
impl Handle<u64> for Register {
    fn handle(state: &Value, command: u64) -> Result<Option<Events<Changed>>, Infallible> {
        Ok((state.0 != command).then(|| events![Changed(command)]))
    }
}
