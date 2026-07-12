//! `Id` requires `Display` (and `AsRef<[u8]>`). A type missing `Display` is
//! therefore *not* covered by the blanket impl, so requiring `I: Id` fails.

use mnesis::Id;

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct BadId(u64);
impl AsRef<[u8]> for BadId {
    fn as_ref(&self) -> &[u8] {
        &[]
    }
}
// Missing: impl std::fmt::Display for BadId

fn requires_id<I: Id>() {}

fn main() {
    requires_id::<BadId>();
}
