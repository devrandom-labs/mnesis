//! The todo aggregate: one event stream per todo.

use std::fmt;

use mnesis::{AggregateState, DomainEvent, Events, Handle, events};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Aggregate id: a `Uuid` newtype.
///
/// `mnesis::Id` is blanket-implemented, so only `Display` + `AsRef<[u8]>`
/// need supplying (finding: the newtype is mandatory — `Uuid` itself
/// satisfies the bounds but the id type appears in handler signatures, so
/// the app owns a name for it).
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct TodoId(pub Uuid);

impl fmt::Display for TodoId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl AsRef<[u8]> for TodoId {
    fn as_ref(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

/// Every variant carries `id`.
///
/// The `$all` stream does not: a `PersistedEnvelope` has no stream id, so a
/// multi-stream projection can only learn which todo an event belongs to
/// from the payload itself (finding #326-7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, DomainEvent)]
pub enum TodoEvent {
    Created { id: Uuid, text: String },
    TextChanged { id: Uuid, text: String },
    CompletionChanged { id: Uuid, completed: bool },
    Deleted { id: Uuid },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TodoState {
    pub created: bool,
    pub deleted: bool,
    pub text: String,
    pub completed: bool,
}

impl AggregateState for TodoState {
    type Event = TodoEvent;

    fn initial() -> Self {
        Self::default()
    }

    fn apply(mut self, event: &TodoEvent) -> Self {
        match event {
            TodoEvent::Created { text, .. } => {
                self.created = true;
                self.text.clone_from(text);
            }
            TodoEvent::TextChanged { text, .. } => self.text.clone_from(text),
            TodoEvent::CompletionChanged { completed, .. } => self.completed = *completed,
            TodoEvent::Deleted { .. } => self.deleted = true,
        }
        self
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum TodoError {
    #[error("todo already exists")]
    AlreadyExists,
    #[error("todo does not exist")]
    NotFound,
    #[error("nothing to update")]
    NothingToUpdate,
}

#[mnesis::aggregate(state = TodoState, error = TodoError, id = TodoId)]
pub struct Todo;

/// Commands carry the id.
///
/// `Handle::handle` is a pure function of `(state, command)` with no
/// identity access — the only route from the URL path to the event payload
/// is command -> event, by hand (finding #326-7).
pub struct Create {
    pub id: Uuid,
    pub text: String,
}

pub struct Update {
    pub id: Uuid,
    pub text: Option<String>,
    pub completed: Option<bool>,
}

pub struct Delete {
    pub id: Uuid,
}

impl Handle<Create> for Todo {
    fn handle(state: &TodoState, cmd: Create) -> Result<Events<TodoEvent>, TodoError> {
        if state.created {
            return Err(TodoError::AlreadyExists);
        }
        Ok(events![TodoEvent::Created {
            id: cmd.id,
            text: cmd.text
        }])
    }
}

impl Handle<Update, 1> for Todo {
    fn handle(state: &TodoState, cmd: Update) -> Result<Events<TodoEvent, 1>, TodoError> {
        if !state.created || state.deleted {
            return Err(TodoError::NotFound);
        }
        match (cmd.text, cmd.completed) {
            (Some(text), Some(completed)) => Ok(events![
                TodoEvent::TextChanged { id: cmd.id, text },
                TodoEvent::CompletionChanged {
                    id: cmd.id,
                    completed
                },
            ]),
            (Some(text), None) => Ok(events![TodoEvent::TextChanged { id: cmd.id, text }]),
            (None, Some(completed)) => Ok(events![TodoEvent::CompletionChanged {
                id: cmd.id,
                completed
            }]),
            (None, None) => Err(TodoError::NothingToUpdate),
        }
    }
}

impl Handle<Delete> for Todo {
    fn handle(state: &TodoState, cmd: Delete) -> Result<Events<TodoEvent>, TodoError> {
        if !state.created || state.deleted {
            return Err(TodoError::NotFound);
        }
        Ok(events![TodoEvent::Deleted { id: cmd.id }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mnesis::testing::AggregateFixture;
    use uuid::Uuid;

    fn uid() -> Uuid {
        Uuid::new_v4()
    }

    fn fixture(id: Uuid) -> AggregateFixture<Todo> {
        AggregateFixture::with_id(TodoId(id))
    }

    #[test]
    fn create_decides_created() {
        let id = uid();
        let _ = fixture(id)
            .given([])
            .when(Create {
                id,
                text: "buy milk".to_owned(),
            })
            .then_expect_events([TodoEvent::Created {
                id,
                text: "buy milk".to_owned(),
            }]);
    }

    #[test]
    fn create_on_existing_todo_is_rejected() {
        let id = uid();
        let _ = fixture(id)
            .given([TodoEvent::Created {
                id,
                text: "a".to_owned(),
            }])
            .when(Create {
                id,
                text: "b".to_owned(),
            })
            .then_expect_error(TodoError::AlreadyExists);
    }

    #[test]
    fn update_both_fields_decides_two_events() {
        let id = uid();
        let _ = fixture(id)
            .given([TodoEvent::Created {
                id,
                text: "a".to_owned(),
            }])
            .when(Update {
                id,
                text: Some("b".to_owned()),
                completed: Some(true),
            })
            .then_expect_events([
                TodoEvent::TextChanged {
                    id,
                    text: "b".to_owned(),
                },
                TodoEvent::CompletionChanged {
                    id,
                    completed: true,
                },
            ]);
    }

    #[test]
    fn update_single_field_decides_one_event() {
        let id = uid();
        let _ = fixture(id)
            .given([TodoEvent::Created {
                id,
                text: "a".to_owned(),
            }])
            .when(Update {
                id,
                text: None,
                completed: Some(true),
            })
            .then_expect_events([TodoEvent::CompletionChanged {
                id,
                completed: true,
            }]);
    }

    #[test]
    fn update_with_no_fields_is_rejected() {
        // `Events<E, N>` guarantees >= 1 event, so "decide nothing" has no
        // representation in `Handle` (finding #326-3): the all-None command
        // must be an error here; the HTTP handler answers the no-op PATCH
        // from state without entering the domain.
        let id = uid();
        let _ = fixture(id)
            .given([TodoEvent::Created {
                id,
                text: "a".to_owned(),
            }])
            .when(Update {
                id,
                text: None,
                completed: None,
            })
            .then_expect_error(TodoError::NothingToUpdate);
    }

    #[test]
    fn update_missing_todo_is_rejected() {
        let id = uid();
        let _ = fixture(id)
            .given([])
            .when(Update {
                id,
                text: Some("b".to_owned()),
                completed: None,
            })
            .then_expect_error(TodoError::NotFound);
    }

    #[test]
    fn update_deleted_todo_is_rejected() {
        let id = uid();
        let _ = fixture(id)
            .given([
                TodoEvent::Created {
                    id,
                    text: "a".to_owned(),
                },
                TodoEvent::Deleted { id },
            ])
            .when(Update {
                id,
                text: Some("b".to_owned()),
                completed: None,
            })
            .then_expect_error(TodoError::NotFound);
    }

    #[test]
    fn delete_decides_deleted() {
        let id = uid();
        let _ = fixture(id)
            .given([TodoEvent::Created {
                id,
                text: "a".to_owned(),
            }])
            .when(Delete { id })
            .then_expect_events([TodoEvent::Deleted { id }]);
    }

    #[test]
    fn delete_twice_is_rejected() {
        let id = uid();
        let _ = fixture(id)
            .given([
                TodoEvent::Created {
                    id,
                    text: "a".to_owned(),
                },
                TodoEvent::Deleted { id },
            ])
            .when(Delete { id })
            .then_expect_error(TodoError::NotFound);
    }

    #[test]
    fn state_folds_full_history() {
        let id = uid();
        let _ = fixture(id)
            .given([
                TodoEvent::Created {
                    id,
                    text: "a".to_owned(),
                },
                TodoEvent::TextChanged {
                    id,
                    text: "b".to_owned(),
                },
                TodoEvent::CompletionChanged {
                    id,
                    completed: true,
                },
            ])
            .then_expect_state(|state| {
                assert_eq!(
                    state,
                    &TodoState {
                        created: true,
                        deleted: false,
                        text: "b".to_owned(),
                        completed: true,
                    }
                );
            });
    }
}
