use mnesis::{Aggregate, AggregateState, DomainEvent, Events, Handle, Message, events};
use serde::{Deserialize, Serialize};

#[path = "operation.rs"]
mod operation;
pub use operation::Operation;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Increment(pub u8);
impl Message for Increment {}
impl DomainEvent for Increment {
    fn name(&self) -> &'static str {
        "increment"
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sum(pub u64);
impl AggregateState for Sum {
    type Event = Increment;
    fn initial() -> Self {
        Self(0)
    }
    fn apply(self, event: &Increment) -> Self {
        Self(self.0 + u64::from(event.0))
    }
}
#[derive(Debug, thiserror::Error)]
#[error("rejected fixture command")]
pub struct Rejection;
pub struct Counter;
impl Aggregate for Counter {
    type State = Sum;
    type Error = Rejection;
    type Id = String;
}
impl Handle<Operation> for Counter {
    fn handle(_: &Sum, command: Operation) -> Result<Option<Events<Increment>>, Rejection> {
        match command {
            Operation::Add(amount) => Ok(Some(events![Increment(amount)])),
            Operation::Ignore => Ok(None),
            Operation::Reject => Err(Rejection),
        }
    }
}
