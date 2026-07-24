//! Tests the marker aggregate + dispatch pattern — what `#[mnesis::aggregate]`
//! generates. The aggregate is a bare marker; command handlers are pure
//! associated functions on the marker, dispatched via `AggregateRoot::handle`.

use mnesis::testing::AggregateFixture;
use mnesis::*;
use std::fmt;

// --- Domain types ---

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct UserId(String);
impl UserId {
    fn new(v: u64) -> Self {
        Self(format!("user-{v}"))
    }
}
impl fmt::Display for UserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl AsRef<[u8]> for UserId {
    fn as_ref(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

#[derive(Debug, Clone)]
struct UserCreated {
    name: String,
}
#[derive(Debug, Clone)]
struct UserActivated;

#[derive(Debug, Clone)]
enum UserEvent {
    Created(UserCreated),
    Activated(UserActivated),
}
impl Message for UserEvent {}
impl DomainEvent for UserEvent {
    fn name(&self) -> &'static str {
        match self {
            Self::Created(_) => "UserCreated",
            Self::Activated(_) => "UserActivated",
        }
    }
}

#[derive(Default, Debug, Clone)]
struct UserState {
    name: String,
    active: bool,
}
impl AggregateState for UserState {
    type Event = UserEvent;
    fn initial() -> Self {
        Self::default()
    }
    fn apply(mut self, event: &UserEvent) -> Self {
        match event {
            UserEvent::Created(e) => self.name.clone_from(&e.name),
            UserEvent::Activated(_) => self.active = true,
        }
        self
    }
}

#[derive(Debug, thiserror::Error)]
enum UserError {
    #[error("user already exists")]
    AlreadyExists,
    #[error("user already active")]
    AlreadyActive,
}

// --- This is what #[mnesis::aggregate] would generate: a bare marker ---

struct UserAggregate;

impl Aggregate for UserAggregate {
    type State = UserState;
    type Error = UserError;
    type Id = UserId;
}

// --- Commands ---
struct CreateUser {
    name: String,
}
struct ActivateUser;

// --- Command handlers (decide pattern) — the user writes this ---
impl Handle<CreateUser> for UserAggregate {
    fn handle(state: &UserState, cmd: CreateUser) -> Result<Option<Events<UserEvent>>, UserError> {
        if !state.name.is_empty() {
            return Err(UserError::AlreadyExists);
        }
        Ok(Some(events![UserEvent::Created(UserCreated {
            name: cmd.name
        })]))
    }
}

impl Handle<ActivateUser> for UserAggregate {
    fn handle(
        state: &UserState,
        _cmd: ActivateUser,
    ) -> Result<Option<Events<UserEvent>>, UserError> {
        if state.active {
            return Err(UserError::AlreadyActive);
        }
        Ok(Some(events![UserEvent::Activated(UserActivated)]))
    }
}

// --- Tests ---

#[test]
fn marker_aggregate_lifecycle() {
    let mut user = AggregateRoot::<UserAggregate>::new(UserId::new(1));

    let events = user
        .handle(CreateUser {
            name: "Alice".into(),
        })
        .unwrap()
        .unwrap();
    let v1 = Version::new(1).unwrap();
    user.commit_persisted(v1, &events);

    let events = user.handle(ActivateUser).unwrap().unwrap();
    let v2 = Version::new(2).unwrap();
    user.commit_persisted(v2, &events);

    assert_eq!(user.state().name, "Alice");
    assert!(user.state().active);
    assert_eq!(user.version(), Some(v2));
}

// Invariant rejections are pure decide logic: the history that makes each
// command illegal is supplied via `given` (replacing the hand-rolled
// create/commit setup), then `when` decides on top of it. `UserError` has no
// `PartialEq`, so rejection is asserted with `then_expect_error_matching`.
#[test]
fn marker_aggregate_rejects_duplicate_create() {
    AggregateFixture::<UserAggregate>::with_id(UserId::new(2))
        .given([UserEvent::Created(UserCreated { name: "Bob".into() })])
        .when(CreateUser {
            name: "Charlie".into(),
        })
        .then_expect_error_matching(|e| matches!(e, UserError::AlreadyExists));
}

#[test]
fn marker_aggregate_rejects_duplicate_activate() {
    AggregateFixture::<UserAggregate>::with_id(UserId::new(2))
        .given([
            UserEvent::Created(UserCreated { name: "Bob".into() }),
            UserEvent::Activated(UserActivated),
        ])
        .when(ActivateUser)
        .then_expect_error_matching(|e| matches!(e, UserError::AlreadyActive));
}

// Rehydration parity through the fixture: `given` replays the aggregate's own
// history (the real replay path) and `then_expect_state` proves the folded state. Version
// progression is covered by the kept `marker_aggregate_lifecycle` test.
#[test]
fn marker_aggregate_rehydrate() {
    AggregateFixture::<UserAggregate>::with_id(UserId::new(3))
        .given([
            UserEvent::Created(UserCreated {
                name: "Dave".into(),
            }),
            UserEvent::Activated(UserActivated),
        ])
        .then_expect_state(|s| {
            assert_eq!(s.name, "Dave");
            assert!(s.active);
        });
}

#[test]
fn marker_aggregate_id_accessible() {
    let user = AggregateRoot::<UserAggregate>::new(UserId::new(42));
    assert_eq!(user.id(), &UserId::new(42));
}

#[test]
fn marker_aggregate_initial_state() {
    let user = AggregateRoot::<UserAggregate>::new(UserId::new(1));
    assert_eq!(user.state().name, "");
    assert!(!user.state().active);
}

#[test]
fn marker_aggregate_fresh_version_is_none() {
    let user = AggregateRoot::<UserAggregate>::new(UserId::new(1));
    assert_eq!(user.version(), None);
}
