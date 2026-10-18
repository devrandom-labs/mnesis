//! Lower, equal and unrelated terminal versions cannot be supplied to commit.
use mnesis::{Aggregate, AggregateRoot, AggregateState, DomainEvent, Events, Message, Version};

#[derive(Debug)]
struct Event;
impl Message for Event {}
impl DomainEvent for Event {
    fn name(&self) -> &'static str { "E" }
}
#[derive(Debug)]
struct State;
impl AggregateState for State {
    type Event = Event;
    fn initial() -> Self { Self }
    fn apply(self, _: &Event) -> Self { self }
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Id;
impl AsRef<[u8]> for Id {
    fn as_ref(&self) -> &[u8] { b"s" }
}
impl core::fmt::Display for Id {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { f.write_str("s") }
}
struct App;
impl Aggregate for App {
    type State = State;
    type Error = core::convert::Infallible;
    type Id = Id;
}
fn main() {
    let mut root = AggregateRoot::<App>::restore(Id, State, Version::new(7).unwrap());
    let events = Events::<Event>::new(Event);
    for terminal in [1, 7, 42] {
        root.commit_persisted(Version::new(terminal).unwrap(), &events);
    }
}
