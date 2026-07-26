use mnesis::Events;
use mnesis::*;

#[derive(Debug, Clone, PartialEq)]
struct Created;
#[derive(Debug, Clone, PartialEq)]
struct Activated;

#[derive(Debug, Clone, PartialEq)]
enum TestEvent {
    Created(Created),
    Activated(Activated),
}
impl Message for TestEvent {}
impl DomainEvent for TestEvent {
    fn name(&self) -> &'static str {
        match self {
            Self::Created(_) => "Created",
            Self::Activated(_) => "Activated",
        }
    }
}

#[test]
fn versioned_event_holds_version_and_event() {
    let v1 = Version::new(1).unwrap();
    let ve = VersionedEvent::new(v1, TestEvent::Created(Created));
    assert_eq!(ve.version(), v1);
    assert_eq!(ve.event(), &TestEvent::Created(Created));
}

#[test]
fn events_guarantees_non_empty() {
    let events: Events<_, 0> = Events::new(TestEvent::Created(Created));
    assert_eq!(events.len(), 1);
    assert!(!events.is_empty());
}

#[test]
fn events_add_increases_len() {
    let mut events: Events<_, 1> = Events::new(TestEvent::Created(Created));
    events.add(TestEvent::Activated(Activated));
    assert_eq!(events.len(), 2);
}

#[test]
fn events_into_iter() {
    let mut events: Events<_, 1> = Events::new(TestEvent::Created(Created));
    events.add(TestEvent::Activated(Activated));
    let collected: Vec<_> = events.into_iter().collect();
    assert_eq!(collected.len(), 2);
    assert_eq!(collected[0], TestEvent::Created(Created));
    assert_eq!(collected[1], TestEvent::Activated(Activated));
}

#[test]
fn events_from_single() {
    let events: Events<_, 0> = Events::from(TestEvent::Created(Created));
    assert_eq!(events.len(), 1);
}

#[test]
fn events_macro_single() {
    let events: Events<_, 0> = mnesis::events![TestEvent::Created(Created)];
    assert_eq!(events.len(), 1);
}

#[test]
fn events_macro_multiple() {
    let events: Events<_, 1> =
        mnesis::events![TestEvent::Created(Created), TestEvent::Activated(Activated),];
    assert_eq!(events.len(), 2);
}

#[test]
#[should_panic(expected = "Events capacity exceeded")]
fn add_panics_on_capacity_overflow() {
    let mut events: Events<_, 0> = Events::new(TestEvent::Created(Created));
    events.add(TestEvent::Activated(Activated)); // N=0, no room for additional events
}

// `Events` is non-empty by construction; `first`/`rest` expose that split so a
// downstream non-empty collection can be built with no runtime check and no
// unprovable `unwrap` (#330 — the store's `PendingBatch` is built this way).
#[test]
fn first_is_the_head_event_of_a_single_event_collection() {
    let events: Events<TestEvent> = Events::new(TestEvent::Created(Created));

    assert_eq!(events.first(), &TestEvent::Created(Created));
    assert!(events.rest().is_empty());
}

#[test]
fn first_and_rest_partition_the_collection_in_order() {
    let mut events: Events<TestEvent, 2> = Events::new(TestEvent::Created(Created));
    events.add(TestEvent::Activated(Activated));
    events.add(TestEvent::Created(Created));

    assert_eq!(events.first(), &TestEvent::Created(Created));
    assert_eq!(
        events.rest(),
        &[TestEvent::Activated(Activated), TestEvent::Created(Created)]
    );
    assert_eq!(events.len(), 3, "first + rest is the whole collection");
}

// ── PartialEq distinguishes first, rest, AND equal (kills all 5 eq mutants) ──
// `eq` is `self.first == other.first && self.rest == other.rest`. Three
// assertions pin every operand: an identical pair must be equal (kills `-> false`
// and each `==` -> `!=`), a first-only difference must be unequal (kills `-> true`
// and the first `==` -> `!=`), and a rest-only difference must be unequal (kills
// the `&&` -> `||`, which would call two differing collections equal).
#[test]
fn events_eq_distinguishes_first_rest_and_equal() {
    let mk = |a, b| {
        let mut e: Events<TestEvent, 1> = Events::new(a);
        e.add(b);
        e
    };
    let base = mk(TestEvent::Created(Created), TestEvent::Activated(Activated));
    let same = mk(TestEvent::Created(Created), TestEvent::Activated(Activated));
    let diff_first = mk(
        TestEvent::Activated(Activated),
        TestEvent::Activated(Activated),
    );
    let diff_rest = mk(TestEvent::Created(Created), TestEvent::Created(Created));

    assert_eq!(base, same);
    assert_ne!(base, diff_first);
    assert_ne!(base, diff_rest);
}
