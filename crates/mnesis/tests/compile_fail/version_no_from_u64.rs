/// Version cannot be constructed via From<u64>.
/// Only Version::INITIAL, Version::new(), and Version::next() are available.

use mnesis::Version;

fn main() {
    let _v: Version = Version::from(42u64);
}
