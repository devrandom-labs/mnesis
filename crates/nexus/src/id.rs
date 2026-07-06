use core::fmt::{Debug, Display};
use core::hash::Hash;

use crate::ErrorId;

/// Marker for a type usable as a stream / aggregate id.
///
/// An id is anything that can be cloned, moved across threads, compared,
/// hashed, printed, and — crucially — viewed as bytes for use as a storage
/// key (`AsRef<[u8]>` is the stable byte representation; `Display` is for
/// humans, never for serialization).
///
/// There is nothing to implement: a [blanket impl](#impl-Id-for-T) covers
/// every type that already satisfies the supertraits, so a plain newtype
///
/// ```
/// # use std::fmt;
/// #[derive(Debug, Clone, Hash, PartialEq, Eq)]
/// struct AccountId(String);
/// impl fmt::Display for AccountId {
///     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(&self.0) }
/// }
/// impl AsRef<[u8]> for AccountId {
///     fn as_ref(&self) -> &[u8] { self.0.as_bytes() }
/// }
/// ```
///
/// *is* an `Id` — no `impl Id` block, no associated const.
pub trait Id: Clone + Send + Sync + Debug + Hash + Eq + Display + AsRef<[u8]> + 'static {
    /// Stack-allocated diagnostic label for error messages.
    ///
    /// If the display representation exceeds 64 bytes it is truncated on a
    /// char boundary and the tail is replaced with `…`, so truncation is
    /// always visually signalled (never silent). See [`ErrorId`].
    fn to_label(&self) -> ErrorId {
        ErrorId::from_display(self)
    }
}

/// Every type carrying the supertraits is an [`Id`] — declaring an id is a
/// newtype plus `Display`/`AsRef<[u8]>`, with no hand-written `Id` impl.
impl<T: Clone + Send + Sync + Debug + Hash + Eq + Display + AsRef<[u8]> + 'static> Id for T {}
