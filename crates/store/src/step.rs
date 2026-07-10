//! The catch-up→live phase marker for typed subscription consumption (#250).

/// One item of a phase-aware subscription cursor: a decoded event, or the
/// **caught-up** boundary marker emitted once when replay finishes and the
/// cursor switches to live tailing.
///
/// Everything yielded before [`CaughtUp`](Step::CaughtUp) is replay (catch-up
/// over the backlog); everything after is live. `T` is the item payload —
/// `Decoded<E>` per-stream, `(AllPosition, Decoded<E>)` for `$all`.
///
/// Exhaustive (no `#[non_exhaustive]`, per project rule): the two variants are
/// frozen at 1.0. A lag signal (`FellBehind`) is intentionally omitted — the
/// live loop does not distinguish a lagging live consumer from a caught-up one,
/// and a consumer can observe lag from positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step<T> {
    /// A delivered event (replay or live — the phase is told by the preceding
    /// [`CaughtUp`](Step::CaughtUp)).
    Event(T),
    /// The backlog is drained; subsequent items are live. Emitted exactly once.
    CaughtUp,
}

impl<T> Step<T> {
    /// The event payload, or `None` for [`CaughtUp`](Step::CaughtUp).
    #[must_use]
    pub fn event(self) -> Option<T> {
        match self {
            Self::Event(t) => Some(t),
            Self::CaughtUp => None,
        }
    }

    /// `true` iff this is the [`CaughtUp`](Step::CaughtUp) marker.
    #[must_use]
    pub const fn is_caught_up(&self) -> bool {
        matches!(self, Self::CaughtUp)
    }

    /// Map the carried payload, leaving [`CaughtUp`](Step::CaughtUp) untouched.
    /// The phase marker flows through every transform (drop-the-tag in
    /// `subscribe`, decode in `.decoded()`) so a boundary is never lost.
    #[must_use]
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Step<U> {
        match self {
            Self::Event(t) => Step::Event(f(t)),
            Self::CaughtUp => Step::CaughtUp,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_and_caught_up_accessors() {
        assert_eq!(Step::Event(7u8).event(), Some(7));
        assert_eq!(Step::<u8>::CaughtUp.event(), None);
        assert!(Step::<u8>::CaughtUp.is_caught_up());
        assert!(!Step::Event(7u8).is_caught_up());
    }

    #[test]
    fn map_transforms_event_and_passes_caught_up_through() {
        assert_eq!(
            Step::Event(3u8).map(|n| u32::from(n) * 2),
            Step::Event(6u32)
        );
        assert_eq!(Step::<u8>::CaughtUp.map(u32::from), Step::<u32>::CaughtUp);
    }
}
