//! The `no_std` aggregate whose macro output the flake gates compile.
//!
//! Mirrors `mnesis-cross-crate-test` (the std cross-crate probe) but stays in
//! `core`: `core::fmt` for `Display`, a fixed-size `[u8; 8]` id (no `String`),
//! and `Events<TaskEvent>` at the default `N = 0` (single event, no allocator).

use core::fmt;
use mnesis::{AggregateRoot, AggregateState, DomainEvent, Events, Handle, Version, events};

// --- Id (core-only: fixed array, `core::fmt` Display) ---
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct TaskId([u8; 8]);

impl TaskId {
    pub fn new(id: u64) -> Self {
        Self(id.to_be_bytes())
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "task-{}", u64::from_be_bytes(self.0))
    }
}

impl AsRef<[u8]> for TaskId {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

// --- Events (derive macro output compiled for the target) ---
#[derive(Debug, Clone, DomainEvent)]
pub enum TaskEvent {
    Started,
    Finished,
}

// --- State ---
#[derive(Default, Debug, Clone)]
pub struct TaskState {
    started: bool,
    done: bool,
}

impl AggregateState for TaskState {
    type Event = TaskEvent;
    fn initial() -> Self {
        Self::default()
    }
    fn apply(mut self, event: &TaskEvent) -> Self {
        match event {
            TaskEvent::Started => self.started = true,
            TaskEvent::Finished => self.done = true,
        }
        self
    }
}

// --- Error (thiserror, no_std — no `std::error::Error` bridge on target) ---
#[derive(Debug, thiserror::Error)]
pub enum TaskError {
    #[error("already started")]
    AlreadyStarted,
    #[error("not started")]
    NotStarted,
    #[error("already done")]
    AlreadyDone,
}

// --- Aggregate (attribute macro output compiled for the target) ---
#[mnesis::aggregate(state = TaskState, error = TaskError, id = TaskId)]
pub struct TaskAggregate;

// --- Commands + decide (single-event `Events<_, 0>`, no allocator) ---
pub struct StartTask;
pub struct FinishTask;

impl Handle<StartTask> for TaskAggregate {
    fn handle(state: &TaskState, _cmd: StartTask) -> Result<Events<TaskEvent>, TaskError> {
        if state.started {
            return Err(TaskError::AlreadyStarted);
        }
        Ok(events![TaskEvent::Started])
    }
}

impl Handle<FinishTask> for TaskAggregate {
    fn handle(state: &TaskState, _cmd: FinishTask) -> Result<Events<TaskEvent>, TaskError> {
        if !state.started {
            return Err(TaskError::NotStarted);
        }
        if state.done {
            return Err(TaskError::AlreadyDone);
        }
        Ok(events![TaskEvent::Finished])
    }
}

// Exercise the generic `AggregateRoot` surface so the monomorphised driver
// methods (`new`/`handle`/`commit_persisted`/`version`) are instantiated for a
// `no_std` aggregate — a std leak in any of them would surface here at compile
// time on the target.
pub fn drive(id: u64) -> Option<Version> {
    let mut root = AggregateRoot::<TaskAggregate>::new(TaskId::new(id));
    if let Ok(events) = root.handle(StartTask) {
        root.commit_persisted(Version::INITIAL, &events);
    }
    root.version()
}

// Prove the `DomainEvent::name` codegen is reachable in `core`.
pub fn event_name(event: &TaskEvent) -> &'static str {
    event.name()
}
