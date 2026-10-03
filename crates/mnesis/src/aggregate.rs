use crate::error::{DecisionError, KernelError};
use crate::event::DomainEvent;
use crate::events::Events;
use crate::id::Id;
use crate::version::Version;
use core::error::Error;
use core::fmt;
use core::fmt::Debug;
use core::num::NonZeroUsize;

/// State of an event-sourced aggregate. Mutated by applying domain events.
///
/// This is the **evolve** function: given a state and an event, produce
/// the next state. It is infallible — events are facts that have already
/// been accepted.
///
/// No `Clone` bound: [`apply`](Self::apply) takes `self` by value and
/// returns the next state, so rehydration folds the owned state through
/// `apply` with no per-event copy.
///
/// # Example
///
/// ```
/// use mnesis::AggregateState;
/// use mnesis::DomainEvent;
/// use mnesis::Message;
///
/// #[derive(Debug, Clone)]
/// enum CounterEvent { Incremented, Decremented }
/// impl Message for CounterEvent {}
/// impl DomainEvent for CounterEvent {
///     fn name(&self) -> &'static str {
///         match self {
///             CounterEvent::Incremented => "Incremented",
///             CounterEvent::Decremented => "Decremented",
///         }
///     }
/// }
///
/// #[derive(Default, Debug, Clone)]
/// struct CounterState { value: i64 }
///
/// impl AggregateState for CounterState {
///     fn initial() -> Self { Self::default() }
///     type Event = CounterEvent;
///     fn apply(mut self, event: &CounterEvent) -> Self {
///         match event {
///             CounterEvent::Incremented => self.value += 1,
///             CounterEvent::Decremented => self.value -= 1,
///         }
///         self
///     }
/// }
/// ```
pub trait AggregateState: Send + Sync + Debug + 'static {
    type Event: DomainEvent;

    /// The initial state of a new aggregate.
    ///
    /// This replaces `Default` — use this when the zero-valued state
    /// is invalid for your domain. For simple cases, just return
    /// `Self { field: 0, ... }` or derive `Default` and call
    /// `Self::default()`.
    fn initial() -> Self;

    /// Apply a domain event, returning the new state.
    ///
    /// Takes `self` by value to guarantee atomic state transitions —
    /// either the entire function completes and returns a valid new
    /// state, or it panics and the old state is consumed. No partial
    /// mutation is possible.
    ///
    /// This method is infallible by design: events represent facts
    /// that have already been accepted. Validation happens in command
    /// handlers ([`Handle`]) before events are produced. If this method
    /// panics, it indicates a bug in the state machine.
    #[must_use]
    fn apply(self, event: &Self::Event) -> Self;
}

/// Type-level specification binding state, error, and ID types.
///
/// This is the marker trait — just associated types and configurable
/// constants, no methods. [`AggregateRoot<A>`] uses these types internally.
/// For command handling, see [`Handle`].
///
/// # Example
///
/// ```
/// use mnesis::*;
/// use std::num::NonZeroUsize;
///
/// # #[derive(Debug, Clone)] enum Ev { A }
/// # impl Message for Ev {}
/// # impl DomainEvent for Ev { fn name(&self) -> &'static str { "A" } }
/// # #[derive(Default, Debug, Clone)] struct St;
/// # impl AggregateState for St { type Event = Ev; fn initial() -> Self { Self::default() } fn apply(self, _: &Ev) -> Self { self } }
/// # #[derive(Debug, Clone, Hash, PartialEq, Eq)] struct MyId(String);
/// # impl std::fmt::Display for MyId { fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { write!(f, "{}", self.0) } }
/// # impl AsRef<[u8]> for MyId { fn as_ref(&self) -> &[u8] { self.0.as_bytes() } }
/// # #[derive(Debug, thiserror::Error)] #[error("e")] struct MyError;
///
/// struct MyAggregate;
///
/// impl Aggregate for MyAggregate {
///     type State = St;
///     type Error = MyError;
///     type Id = MyId;
/// }
/// ```
pub trait Aggregate: Sized {
    type State: AggregateState;
    type Error: Error + Send + Sync + Debug + 'static;
    type Id: Id;

    /// Maximum events during rehydration via [`AggregateRoot::replay`].
    /// Prevents a corrupted or malicious store from feeding unbounded events.
    /// Default: 1,000,000.
    const MAX_REHYDRATION_EVENTS: NonZeroUsize = DEFAULT_MAX_REHYDRATION_EVENTS;
}

/// Per-command handler trait — the **decide** function.
///
/// Implement this on the aggregate marker for each command it accepts. The
/// handler reads the current `state`, validates invariants, and returns
/// decided events. It never mutates the aggregate — the repository
/// handles persistence and state advancement.
///
/// The const generic `N` controls how many *additional* events beyond
/// the first can be returned. Total capacity is `N + 1`. The default
/// `N = 0` means the handler returns exactly one event — the common
/// case for most commands. For multi-event handlers, specify `N`
/// explicitly (e.g., `Handle<CreateOrder, 2>` for up to 3 events).
///
/// # Example
///
/// Single-event handler (default `N = 0`):
///
/// ```
/// use mnesis::*;
///
/// # #[derive(Debug, Clone)] enum TodoEvent { Created(String), Completed }
/// # impl Message for TodoEvent {}
/// # impl DomainEvent for TodoEvent { fn name(&self) -> &'static str { "e" } }
/// # #[derive(Default, Debug, Clone)] struct TodoState { done: bool }
/// # impl AggregateState for TodoState { type Event = TodoEvent; fn initial() -> Self { Self::default() } fn apply(self, _: &TodoEvent) -> Self { self } }
/// # #[derive(Debug, Clone, Hash, PartialEq, Eq)] struct TodoId(String);
/// # impl std::fmt::Display for TodoId { fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { write!(f, "{}", self.0) } }
/// # impl AsRef<[u8]> for TodoId { fn as_ref(&self) -> &[u8] { self.0.as_bytes() } }
/// # #[derive(Debug, thiserror::Error)] #[error("e")] struct TodoError;
/// # struct Todo;
/// # impl Aggregate for Todo { type State = TodoState; type Error = TodoError; type Id = TodoId; }
///
/// struct CompleteTodo;
///
/// impl Handle<CompleteTodo> for Todo {
///     fn handle(state: &TodoState, _cmd: CompleteTodo) -> Result<Option<Events<TodoEvent>>, TodoError> {
///         if state.done {
///             // Already complete: nothing to record. Not an error.
///             return Ok(None);
///         }
///         Ok(Some(events![TodoEvent::Completed]))
///     }
/// }
/// ```
pub trait Handle<C, const N: usize = 0>: Aggregate {
    /// Decide a command, returning decided events, "nothing to record", or a
    /// domain error.
    ///
    /// A **pure decision**: reads the aggregate's current `state` and the
    /// command, returns the decided events. No access to version or identity —
    /// a decision is a function of domain state and command only, never of
    /// persistence position. Implemented on the aggregate marker type; invoke
    /// via [`AggregateRoot::handle`] on a loaded aggregate.
    ///
    /// - `Ok(None)` — the command is accepted and changes nothing; e.g. an
    ///   idempotent re-issue, or an update whose fields are all absent.
    ///   Nothing is persisted and the version does not advance.
    /// - `Ok(Some(events))` — the decided events (at least one, by
    ///   construction of [`Events`]).
    /// - `Err(_)` — the command violates a domain invariant.
    ///
    /// The `Option` mirrors [`React::react`](crate::React::react), the saga
    /// dual: a routed-but-no-op outcome is a decision, not a failure, so it is
    /// neither an error variant nor a redundant event appended to an immutable
    /// log.
    ///
    /// # Errors
    ///
    /// Returns `Self::Error` when the command violates a domain invariant.
    fn handle(state: &Self::State, cmd: C)
    -> Result<Option<Events<EventOf<Self>, N>>, Self::Error>;
}

/// Shorthand for accessing the event type of an aggregate.
///
/// Instead of writing `<<A as Aggregate>::State as AggregateState>::Event`,
/// write `EventOf<A>`.
pub type EventOf<A> = <<A as Aggregate>::State as AggregateState>::Event;

/// Default maximum events during rehydration via `replay`.
/// Override per-aggregate via `Aggregate::MAX_REHYDRATION_EVENTS`.
#[allow(clippy::unwrap_used, reason = "1_000_000 is non-zero by inspection")]
pub const DEFAULT_MAX_REHYDRATION_EVENTS: NonZeroUsize = NonZeroUsize::new(1_000_000).unwrap();

/// The core event-sourced aggregate container.
///
/// Holds state and version. The aggregate is a **read-only state container**
/// after loading — command handlers ([`Handle`]) read state to make decisions,
/// and the repository handles persistence and version advancement.
///
/// # Loading (rehydration)
///
/// The repository creates a new `AggregateRoot` and replays persisted events:
/// ```ignore
/// let mut root = AggregateRoot::<MyAggregate>::new(id);
/// for event in stored_events {
///     root.replay(event.version(), &event)?;
/// }
/// ```
///
/// # Command handling
///
/// After loading, the application layer calls [`Handle::handle`] to decide,
/// then persists the returned events via the repository. The aggregate
/// itself never buffers or persists events. After a durable persist the
/// driver calls [`commit_persisted`](Self::commit_persisted) once to advance
/// the version and fold the events into state atomically.
///
/// # Panic safety
///
/// Folding owns state without cloning it. State is absent while application
/// code runs. If application code panics, the root permanently rejects state
/// access, decisions, replay and commit with a typed integrity error. Reload
/// committed history into a new root. Already-persisted events are not rolled
/// back; the version remains available as diagnostic metadata.
pub struct AggregateRoot<A: Aggregate> {
    id: A::Id,
    state: Option<A::State>,
    version: Option<Version>,
    replayed_events: usize,
}

impl<A: Aggregate> fmt::Debug for AggregateRoot<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AggregateRoot")
            .field("id", &self.id)
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

impl<A: Aggregate> AggregateRoot<A> {
    /// Create a new aggregate with initial state, no history and a fresh replay budget.
    pub fn new(id: A::Id) -> Self {
        Self {
            id,
            state: Some(A::State::initial()),
            version: None,
            replayed_events: 0,
        }
    }

    /// Create an aggregate root restored from a snapshot.
    ///
    /// The root is initialized with the given state and version,
    /// as if those events had already been replayed. Subsequent
    /// calls to [`replay`](Self::replay) will expect versions
    /// starting at `version + 1`. The replay work budget starts at zero;
    /// events already represented by the snapshot do not consume it.
    ///
    /// Used by snapshot-aware repositories to skip full event replay.
    #[must_use]
    pub const fn restore(id: A::Id, state: A::State, version: Version) -> Self {
        Self {
            id,
            state: Some(state),
            version: Some(version),
            replayed_events: 0,
        }
    }

    /// The aggregate's identity.
    #[must_use]
    pub const fn id(&self) -> &A::Id {
        &self.id
    }

    /// Borrow the current state.
    ///
    /// # Errors
    /// Returns [`KernelError::PoisonedAggregate`] after an application fold
    /// panics. No replacement or partially folded state is exposed.
    pub fn state(&self) -> Result<&A::State, KernelError> {
        self.state.as_ref().ok_or(KernelError::PoisonedAggregate)
    }

    /// The last persisted version, or `None` for a fresh aggregate with no history.
    #[must_use]
    pub const fn version(&self) -> Option<Version> {
        self.version
    }

    /// Decide a command against the current state.
    ///
    /// Dispatches to the aggregate's [`Handle`] impl, passing the current
    /// [`state`](Self::state). Pure — reads state, returns the decided events
    /// (or `None` when the command changes nothing), mutates nothing. The dual
    /// of [`react`](Self::react).
    ///
    /// # Errors
    ///
    /// Returns [`DecisionError::Domain`] for a domain rejection or
    /// [`DecisionError::Kernel`] when the root is unusable.
    pub fn handle<C, const N: usize>(
        &self,
        cmd: C,
    ) -> Result<Option<Events<EventOf<A>, N>>, DecisionError<A::Error>>
    where
        A: Handle<C, N>,
    {
        A::handle(self.state()?, cmd).map_err(DecisionError::Domain)
    }

    /// Replay a single persisted event during rehydration.
    ///
    /// Folds owned state without cloning. Validation errors preserve the root;
    /// application panics make its state unavailable and require a reload.
    ///
    /// Takes a borrowed event reference so zero-copy codecs (rkyv, flatbuffers)
    /// can pass views directly from database buffers without cloning.
    ///
    /// The rehydration entry point: replays persisted events in strict version
    /// order (starting at [`Version::INITIAL`], strictly sequential) to
    /// reconstruct state. A repository drives this during load, but it is also
    /// the supported path for manual / no-store event sourcing — replay a
    /// known-good history `1..=n` to rebuild an aggregate with no persistence
    /// layer at all.
    ///
    /// # Errors
    ///
    /// Returns [`KernelError::PoisonedAggregate`] if an earlier application fold panicked.
    ///
    /// Returns [`KernelError::VersionMismatch`] if `version` is not the
    /// next expected version (gap, duplicate, or out-of-order).
    ///
    /// Returns [`KernelError::RehydrationLimitExceeded`] after this root has
    /// successfully replayed [`Aggregate::MAX_REHYDRATION_EVENTS`] events.
    /// Snapshots and committed command batches do not consume replay work.
    /// Validation failures consume no budget.
    ///
    /// Returns [`KernelError::VersionOverflow`] if the version sequence
    /// is exhausted (aggregate already at `u64::MAX`).
    ///
    /// # Panics
    ///
    /// Propagates application `apply` panics, permanently making state unavailable.
    pub fn replay(&mut self, version: Version, event: &EventOf<A>) -> Result<(), KernelError> {
        self.state()?;
        let expected = match self.version {
            None => Version::INITIAL,
            Some(v) => v.next().ok_or(KernelError::VersionOverflow)?,
        };
        if version != expected {
            return Err(KernelError::VersionMismatch {
                expected,
                actual: version,
            });
        }
        let limit = A::MAX_REHYDRATION_EVENTS.get();
        if self.replayed_events >= limit {
            return Err(KernelError::RehydrationLimitExceeded { max: limit });
        }
        let completed = self
            .replayed_events
            .checked_add(1)
            .ok_or(KernelError::RehydrationLimitExceeded { max: limit })?;
        self.fold_events(core::iter::once(event))?;
        self.replayed_events = completed;
        self.version = Some(version);
        Ok(())
    }

    /// Check root integrity and compute the last version of a prospective commit.
    ///
    /// Call before persistence so overflow cannot occur after writing events.
    /// The nonempty batch supplies the count; callers cannot assign its version.
    ///
    /// # Errors
    /// Returns [`KernelError::PoisonedAggregate`] for an unusable root or
    /// [`KernelError::VersionOverflow`] if the whole batch cannot fit.
    pub fn commit_version<const N: usize>(
        &self,
        events: &Events<EventOf<A>, N>,
    ) -> Result<Version, KernelError> {
        self.state()?;
        let tail = u64::try_from(events.rest().len()).map_err(|_| KernelError::VersionOverflow)?;
        let count = tail.checked_add(1).ok_or(KernelError::VersionOverflow)?;
        let current = self.version.map_or(0, Version::as_u64);
        let ending = current
            .checked_add(count)
            .ok_or(KernelError::VersionOverflow)?;
        Version::new(ending).ok_or(KernelError::VersionOverflow)
    }

    /// Fold events that the caller has already persisted, exactly once.
    ///
    /// The last version is derived from the current boundary plus event count,
    /// checked before mutation, and recorded before
    /// application folding, so it remains diagnostic metadata if a panic
    /// consumes state. A whole batch installs state only after every fold
    /// succeeds. Reload committed history after a panic; persistence is not
    /// rolled back.
    ///
    /// # Errors
    /// Returns [`KernelError::PoisonedAggregate`] without changing metadata if
    /// the root was already unusable. Returns [`KernelError::VersionOverflow`]
    /// without changing state or metadata if the batch cannot fit. Validate with
    /// [`Self::commit_version`] before writing to external storage.
    ///
    /// # Panics
    /// Propagates application `apply` panics, permanently making state unavailable.
    pub fn commit_persisted<const N: usize>(
        &mut self,
        events: &Events<EventOf<A>, N>,
    ) -> Result<(), KernelError> {
        let version = self.commit_version(events)?;
        // The caller has already persisted this boundary. Keep that fact even
        // if application folding panics; unavailable state prevents reuse.
        self.version = Some(version);
        self.apply_events(events)
    }

    /// Apply already-persisted events to state without advancing the version.
    ///
    /// In-crate primitive (`pub(crate)`): [`commit_persisted`](Self::commit_persisted)
    /// derives the version before folding, and the
    /// `testing` fixtures fold decided events without persisting (no version to
    /// advance). Not public — folding state without a version is exactly the
    /// desync the public API forbids.
    pub(crate) fn apply_events<const N: usize>(
        &mut self,
        events: &Events<EventOf<A>, N>,
    ) -> Result<(), KernelError> {
        self.fold_events(events)
    }

    fn fold_events<'a>(
        &mut self,
        events: impl IntoIterator<Item = &'a EventOf<A>>,
    ) -> Result<(), KernelError>
    where
        EventOf<A>: 'a,
    {
        // Nothing usable stays in the root while application code owns state.
        // Unwinding drops the local state and leaves None permanently. A whole
        // batch installs its state only after every event folds successfully.
        let mut current = self.state.take().ok_or(KernelError::PoisonedAggregate)?;
        for event in events {
            current = current.apply(event);
        }
        self.state = Some(current);
        Ok(())
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    reason = "test code: panic-safety test deliberately panics in apply()"
)]
mod purist_dispatch_tests {
    use super::{Aggregate, AggregateRoot, AggregateState, Handle};
    use crate::event::DomainEvent;
    use crate::events;
    use crate::events::Events;
    use crate::message::Message;
    use crate::version::Version;
    use alloc::{vec, vec::Vec};

    #[derive(Debug, Clone, Hash, PartialEq, Eq)]
    struct CtrId([u8; 8]);

    impl CtrId {
        fn new(n: u64) -> Self {
            Self(n.to_le_bytes())
        }
    }

    impl std::fmt::Display for CtrId {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", u64::from_le_bytes(self.0))
        }
    }

    impl AsRef<[u8]> for CtrId {
        fn as_ref(&self) -> &[u8] {
            &self.0
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum CtrEvent {
        Added(u64),
    }

    impl Message for CtrEvent {}

    impl DomainEvent for CtrEvent {
        fn name(&self) -> &'static str {
            match self {
                Self::Added(_) => "Added",
            }
        }
    }

    // Intentionally NOT `Clone` — replay/apply must not require it.
    #[derive(Debug)]
    struct CtrState {
        total: u64,
    }

    impl AggregateState for CtrState {
        type Event = CtrEvent;

        fn initial() -> Self {
            Self { total: 0 }
        }

        fn apply(mut self, event: &CtrEvent) -> Self {
            match event {
                CtrEvent::Added(n) => self.total = self.total.wrapping_add(*n),
            }
            self
        }
    }

    #[derive(Debug, thiserror::Error, PartialEq)]
    #[error("counter error")]
    struct CtrError;

    struct Counter;

    impl Aggregate for Counter {
        type State = CtrState;
        type Error = CtrError;
        type Id = CtrId;
    }

    struct Add(u64);

    impl Handle<Add> for Counter {
        fn handle(state: &CtrState, cmd: Add) -> Result<Option<Events<CtrEvent>>, CtrError> {
            if cmd.0 == 0 {
                return Err(CtrError);
            }
            let _ = state.total;
            Ok(Some(events![CtrEvent::Added(cmd.0)]))
        }
    }

    /// A command whose decision is legitimately "nothing to record" when the
    /// aggregate already sits at the requested total (#329).
    struct SetTotal(u64);

    impl Handle<SetTotal> for Counter {
        fn handle(state: &CtrState, cmd: SetTotal) -> Result<Option<Events<CtrEvent>>, CtrError> {
            match cmd.0.checked_sub(state.total) {
                None => Err(CtrError),
                Some(0) => Ok(None),
                Some(delta) => Ok(Some(events![CtrEvent::Added(delta)])),
            }
        }
    }

    #[test]
    fn dispatches_no_op_decision_as_ignored() {
        // Fresh root: total == 0, so `SetTotal(0)` changes nothing. The
        // decision is neither an error nor a redundant event.
        let root = AggregateRoot::<Counter>::new(CtrId::new(1));
        assert!(matches!(root.handle(SetTotal(0)), Ok(None)));
    }

    #[test]
    fn dispatches_decided_events_as_some() {
        let root = AggregateRoot::<Counter>::new(CtrId::new(1));
        let decided = root.handle(SetTotal(7)).expect("ok").expect("some");
        assert_eq!(
            decided.into_iter().collect::<Vec<_>>(),
            vec![CtrEvent::Added(7)]
        );
    }

    #[test]
    fn dispatches_to_handle_on_the_marker() {
        let root = AggregateRoot::<Counter>::new(CtrId::new(1));
        let decided = root.handle(Add(5)).expect("ok").expect("some");
        assert_eq!(
            decided.into_iter().collect::<Vec<_>>(),
            vec![CtrEvent::Added(5)]
        );
    }

    #[test]
    fn surfaces_domain_error_from_handle() {
        assert!(matches!(
            AggregateRoot::<Counter>::new(CtrId::new(1)).handle(Add(0)),
            Err(crate::DecisionError::Domain(CtrError))
        ));
    }

    #[test]
    fn commit_persisted_advances_version_and_folds_state_atomically() {
        // The "can't desync" guarantee: one call must advance the version AND
        // fold every event into state. `CtrState: !Clone`, so this also proves
        // the owned fold is used without cloning.
        let v2 = Version::new(2).expect("nonzero");
        let persisted: Events<CtrEvent, 1> = events![CtrEvent::Added(10), CtrEvent::Added(5)];
        let mut committed = AggregateRoot::<Counter>::new(CtrId::new(42));
        committed
            .commit_persisted(&persisted)
            .expect("root is usable");
        assert_eq!(committed.version(), Some(v2));
        assert_eq!(committed.state().expect("root is usable").total, 15);

        // The (version, state) reached via `commit_persisted` must equal the
        // (version, state) reached by replaying the same events one-by-one.
        let mut replayed = AggregateRoot::<Counter>::new(CtrId::new(42));
        replayed
            .replay(Version::INITIAL, &CtrEvent::Added(10))
            .expect("replay v1");
        replayed.replay(v2, &CtrEvent::Added(5)).expect("replay v2");
        assert_eq!(committed.version(), replayed.version());
        assert_eq!(
            committed.state().expect("root is usable").total,
            replayed.state().expect("root is usable").total
        );
    }

    // The following three tests cover the now-private primitives in isolation.
    // They were relocated in-crate from `tests/kernel_tests/aggregate_root_tests.rs`
    // (a separate crate, which can no longer reach `pub(crate)`/private methods)
    // when the post-persist pair was folded into the public `commit_persisted`.

    #[test]
    fn commit_version_checks_without_mutating_state_or_metadata() {
        let root = AggregateRoot::<Counter>::new(CtrId::new(1));
        let decided: Events<CtrEvent> = events![CtrEvent::Added(1)];
        for _ in 0..2 {
            assert_eq!(
                root.commit_version(&decided).expect("fits"),
                Version::INITIAL
            );
            assert_eq!(root.version(), None);
            assert_eq!(root.state().expect("usable").total, 0);
        }
    }

    #[test]
    fn apply_events_folds_state_without_advancing_version() {
        let mut agg = AggregateRoot::<Counter>::new(CtrId::new(1));
        let decided: Events<CtrEvent, 1> = events![CtrEvent::Added(2), CtrEvent::Added(3)];
        agg.apply_events(&decided).expect("usable");
        assert_eq!(agg.state().expect("root is usable").total, 5);
        // Fixture-only folding does not advance persistence metadata.
        assert_eq!(agg.version(), None);
    }

    #[test]
    fn apply_event_accumulates_state_without_advancing_version() {
        let mut agg = AggregateRoot::<Counter>::new(CtrId::new(1));
        agg.fold_events(core::iter::once(&CtrEvent::Added(1)))
            .expect("usable");
        assert_eq!(agg.state().expect("root is usable").total, 1);
        agg.fold_events(core::iter::once(&CtrEvent::Added(9)))
            .expect("usable");
        assert_eq!(agg.state().expect("root is usable").total, 10);
        // apply_event does NOT advance version.
        assert_eq!(agg.version(), None);
    }

    #[test]
    fn apply_events_mid_batch_panic_poisoned_root_rejects_reuse() {
        // Relocated in-crate from `tests/kernel_tests/security_tests.rs` (h5):
        // A panic mid-fold must leave no accessible partial state and must not
        // touch diagnostic version metadata (this fixture did not persist).
        use std::panic;

        #[derive(Debug, Clone)]
        enum BoomEvent {
            Inc,
            Boom,
        }
        impl Message for BoomEvent {}
        impl DomainEvent for BoomEvent {
            fn name(&self) -> &'static str {
                match self {
                    Self::Inc => "Inc",
                    Self::Boom => "Boom",
                }
            }
        }

        #[derive(Default, Debug)]
        struct BoomState {
            count: u64,
        }
        impl AggregateState for BoomState {
            type Event = BoomEvent;
            fn initial() -> Self {
                Self::default()
            }
            fn apply(mut self, event: &BoomEvent) -> Self {
                match event {
                    BoomEvent::Inc => self.count += 1,
                    BoomEvent::Boom => panic!("boom in apply_events"),
                }
                self
            }
        }

        struct BoomAgg;
        impl Aggregate for BoomAgg {
            type State = BoomState;
            type Error = CtrError;
            type Id = CtrId;
        }

        let mut agg = AggregateRoot::<BoomAgg>::new(CtrId::new(1));
        let events: Events<BoomEvent, 1> = events![BoomEvent::Inc, BoomEvent::Boom];
        let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
            agg.apply_events(&events).expect("usable before panic");
        }));
        assert!(result.is_err(), "apply_events should have panicked");

        assert!(matches!(
            agg.state(),
            Err(crate::KernelError::PoisonedAggregate)
        ));
        assert!(matches!(
            agg.apply_events(&events),
            Err(crate::KernelError::PoisonedAggregate)
        ));
        assert_eq!(
            agg.version(),
            None,
            "version must remain None — apply_events does not set version"
        );
    }

    #[test]
    fn replay_folds_state_without_clone() {
        // `CtrState: !Clone` — this only compiles because `replay`/`apply`
        // move the state out instead of cloning it. If a
        // `Clone` bound creeps back onto `AggregateState`, this fails to build.
        let mut root = AggregateRoot::<Counter>::new(CtrId::new(7));
        root.replay(Version::INITIAL, &CtrEvent::Added(10))
            .expect("replay v1");
        root.replay(Version::new(2).expect("nonzero"), &CtrEvent::Added(5))
            .expect("replay v2");
        assert_eq!(root.state().expect("root is usable").total, 15);
        assert_eq!(root.version(), Version::new(2));
    }
}
