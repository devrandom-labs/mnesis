use core::fmt;
use core::iter::FusedIterator;
use core::num::NonZeroU64;

/// A compile-checked [`Version`] literal.
///
/// `version!(3)` evaluates to a `Version` (not `Option<Version>`) at compile
/// time. `version!(0)` is a **compile error**, not a runtime panic — the check
/// runs in an inline `const {}` block, so there is zero runtime cost even in
/// debug builds.
///
/// ```
/// use mnesis::{Version, version};
/// assert_eq!(version!(3), Version::new(3).unwrap());
/// ```
#[macro_export]
macro_rules! version {
    ($n:expr) => {
        const {
            match $crate::Version::new($n) {
                ::core::option::Option::Some(v) => v,
                ::core::option::Option::None => {
                    ::core::panic!("version literal must be non-zero")
                }
            }
        }
    };
}

/// A monotonically increasing event version number.
///
/// Wraps `NonZeroU64` — event versions are always >= 1.
/// A fresh aggregate with no events has `Option<Version>` = `None`.
///
/// The only public entry points are:
/// - `Version::INITIAL` — the first event version (1)
/// - `Version::new()` — construct from a `u64` (returns `None` for 0)
/// - `version.next()` — derives the next version, returns `None` on overflow
///
/// # Example
///
/// ```
/// use mnesis::Version;
///
/// let v = Version::INITIAL;
/// assert_eq!(v.as_u64(), 1);
/// assert_eq!(v.next().unwrap().as_u64(), 2);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version(NonZeroU64);

impl Version {
    /// The first event version (1).
    pub const INITIAL: Self = Self(NonZeroU64::MIN);

    /// The next version in sequence.
    ///
    /// Returns `None` if the version is `u64::MAX` (overflow).
    #[must_use]
    pub const fn next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(v) => Some(Self(v)),
            None => None,
        }
    }

    /// The underlying integer value. Always >= 1.
    #[must_use]
    pub const fn as_u64(self) -> u64 {
        self.0.get()
    }

    /// Construct a Version from a `u64`.
    ///
    /// Returns `None` if `v` is 0 (invalid — versions are always >= 1).
    /// Mirrors [`NonZeroU64::new`].
    #[must_use]
    pub const fn new(v: u64) -> Option<Self> {
        match NonZeroU64::new(v) {
            Some(nz) => Some(Self(nz)),
            None => None,
        }
    }

    /// `len` consecutive versions `start, start+1, …, start+len-1`.
    ///
    /// The whole run is validated up front, so iteration is infallible and
    /// never silently truncates on overflow — a lazy infinite iterator
    /// zipped against a batch could drop items; this cannot. Returns `None`
    /// if the run would overflow past `u64::MAX`, consistent with
    /// [`Version::new`]/[`Version::next`] returning `Option`. `len == 0`
    /// yields an empty run.
    #[must_use]
    pub fn run(start: Self, len: usize) -> Option<VersionRun> {
        let Some(last_index) = len.checked_sub(1) else {
            return Some(VersionRun {
                next: start,
                remaining: 0,
            });
        };
        let last_offset = u64::try_from(last_index).ok()?;
        let last_raw = start.as_u64().checked_add(last_offset)?;
        // last_raw >= start.as_u64() >= 1, so it is a valid Version; this
        // only confirms it does not exceed u64::MAX bounds (it always does
        // not, by construction), keeping the overflow check symmetric with
        // `new`/`next`.
        Self::new(last_raw)?;
        Some(VersionRun {
            next: start,
            remaining: len,
        })
    }
}

/// Iterator over a checked run of consecutive [`Version`]s.
///
/// Constructed by [`Version::run`], which validates up front that the whole
/// run fits — so iteration is infallible and never silently truncates on
/// overflow (a lazy infinite iterator zipped against a batch could drop
/// items; this cannot).
#[derive(Debug, Clone)]
pub struct VersionRun {
    next: Version,
    remaining: usize,
}

impl Iterator for VersionRun {
    type Item = Version;

    fn next(&mut self) -> Option<Version> {
        let remaining = self.remaining.checked_sub(1)?;
        let current = self.next;
        self.remaining = remaining;
        // Advance only while more remain. On the final item the successor
        // could overflow past MAX, but `run` length-checked the range so we
        // never need it — this guard prevents ever computing it.
        if remaining > 0
            && let Some(n) = self.next.next()
        {
            self.next = n;
        }
        Some(current)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl ExactSizeIterator for VersionRun {}

impl FusedIterator for VersionRun {}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// An event paired with its version in the aggregate's event sequence.
#[derive(Debug)]
pub struct VersionedEvent<E> {
    version: Version,
    event: E,
}

impl<E: Clone> Clone for VersionedEvent<E> {
    fn clone(&self) -> Self {
        Self {
            version: self.version,
            event: self.event.clone(),
        }
    }
}

impl<E: PartialEq> PartialEq for VersionedEvent<E> {
    fn eq(&self, other: &Self) -> bool {
        self.version == other.version && self.event == other.event
    }
}

impl<E: Eq> Eq for VersionedEvent<E> {}

impl<E> VersionedEvent<E> {
    /// The version (sequence number) of this event in the aggregate's history.
    #[must_use]
    pub const fn version(&self) -> Version {
        self.version
    }

    /// The domain event payload.
    #[must_use]
    pub const fn event(&self) -> &E {
        &self.event
    }

    /// Consume the versioned event, returning the version and event separately.
    #[must_use]
    pub fn into_parts(self) -> (Version, E) {
        (self.version, self.event)
    }

    /// Create a new versioned event.
    ///
    /// Public because store adapters in separate crates need to
    /// reconstruct `VersionedEvent` from deserialized parts.
    #[must_use]
    pub const fn new(version: Version, event: E) -> Self {
        Self { version, event }
    }
}
