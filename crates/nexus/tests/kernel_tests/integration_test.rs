//! Integration test: the complete kernel user experience.
//! Uses #[derive(DomainEvent)] macro + full aggregate lifecycle
//! with the Handle/decide pattern.

use nexus::testing::AggregateFixture;
use nexus::*;
use std::fmt;

// --- ID ---
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

// --- Events (using derive macro!) ---
// `PartialEq` lets the fixture-driven tests assert produced events exactly via
// `then_expect_events` (which requires `EventOf<A>: PartialEq + Debug`).
#[derive(Debug, Clone, PartialEq)]
struct UserCreated {
    name: String,
}

#[derive(Debug, Clone, PartialEq)]
struct UserActivated;

#[derive(Debug, Clone, PartialEq, nexus_macros::DomainEvent)]
enum UserEvent {
    Created(UserCreated),
    Activated(UserActivated),
}

// --- State ---
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

// --- Aggregate marker ---
struct User;

#[derive(Debug, thiserror::Error)]
enum UserError {
    #[error("user already exists")]
    AlreadyExists,
    #[error("user already active")]
    AlreadyActive,
}

impl Aggregate for User {
    type State = UserState;
    type Error = UserError;
    type Id = UserId;
}

// --- Commands ---
struct CreateUser {
    name: String,
}
struct ActivateUser;

// --- Command handlers (decide pattern) ---
impl Handle<CreateUser> for User {
    fn handle(state: &UserState, cmd: CreateUser) -> Result<Events<UserEvent>, UserError> {
        if !state.name.is_empty() {
            return Err(UserError::AlreadyExists);
        }
        Ok(events![UserEvent::Created(UserCreated { name: cmd.name })])
    }
}

impl Handle<ActivateUser> for User {
    fn handle(state: &UserState, _cmd: ActivateUser) -> Result<Events<UserEvent>, UserError> {
        if state.active {
            return Err(UserError::AlreadyActive);
        }
        Ok(events![UserEvent::Activated(UserActivated)])
    }
}

// --- Tests ---

#[test]
fn full_aggregate_lifecycle_with_handle() {
    let mut user = AggregateRoot::<User>::new(UserId::new(1));

    // Decide: command produces events
    let create_events = user
        .handle(CreateUser {
            name: "Alice".into(),
        })
        .unwrap();
    assert_eq!(create_events.len(), 1);
    assert_eq!(create_events.iter().next().unwrap().name(), "Created");

    // Simulate persistence + state advancement
    let v1 = Version::new(1).unwrap();
    user.commit_persisted(v1, &create_events);

    assert_eq!(user.state().name, "Alice");
    assert_eq!(user.version(), Some(v1));

    // Second command
    let activate_events = user.handle(ActivateUser).unwrap();
    let v2 = Version::new(2).unwrap();
    user.commit_persisted(v2, &activate_events);

    assert!(user.state().active);
    assert_eq!(user.version(), Some(v2));
}

// Decide-after-rehydration, driven through `AggregateFixture`: `given` replays
// the `Created` event (the real rehydration path), the chained `then_expect_state`
// proves the rehydrated state, then `when(ActivateUser)` decides on top of it.
// Version progression through `commit_persisted` is covered by the kept
// `full_aggregate_lifecycle_with_handle` test and `aggregate_root_tests`.
#[test]
fn rehydrate_then_decide() {
    AggregateFixture::<User>::with_id(UserId::new(2))
        .given([UserEvent::Created(UserCreated { name: "Bob".into() })])
        .then_expect_state(|s| assert_eq!(s.name, "Bob"))
        .when(ActivateUser)
        .then_expect_events([UserEvent::Activated(UserActivated)])
        .then_expect_state(|s| assert!(s.active));
}

// Invariant rejections are pure decide logic: the prior history that makes a
// command illegal is supplied via `given`, replacing the hand-rolled
// create/commit setup. `UserError` has no `PartialEq`, so rejection is asserted
// with `then_expect_error_matching`.
#[test]
fn duplicate_create_is_rejected() {
    AggregateFixture::<User>::with_id(UserId::new(3))
        .given([UserEvent::Created(UserCreated {
            name: "Charlie".into(),
        })])
        .when(CreateUser {
            name: "David".into(),
        })
        .then_expect_error_matching(|e| matches!(e, UserError::AlreadyExists));
}

#[test]
fn duplicate_activate_is_rejected() {
    AggregateFixture::<User>::with_id(UserId::new(3))
        .given([
            UserEvent::Created(UserCreated {
                name: "Charlie".into(),
            }),
            UserEvent::Activated(UserActivated),
        ])
        .when(ActivateUser)
        .then_expect_error_matching(|e| matches!(e, UserError::AlreadyActive));
}

#[test]
fn derive_macro_generates_correct_event_names() {
    let created = UserEvent::Created(UserCreated {
        name: "test".into(),
    });
    let activated = UserEvent::Activated(UserActivated);

    assert_eq!(created.name(), "Created");
    assert_eq!(activated.name(), "Activated");
}
