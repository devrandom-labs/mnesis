use mnesis::Version;
use mnesis_store::{EventMorsel, SchemaVersion};

fn main() {
    let _ = EventMorsel::borrowed("event", Version::INITIAL, b"payload");
    let _ = SchemaVersion::from_u32(u64::MAX);
}
