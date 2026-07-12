use mnesis::Version;
use mnesis_store::pending_envelope;

fn main() {
    let _ = pending_envelope(Version::new(1).unwrap()).payload(vec![1]);
}
