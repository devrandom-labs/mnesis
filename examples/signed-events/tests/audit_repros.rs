//! Known signing ambiguity; see A13 in todo.md.
use mnesis_example_signed_events::domain::{SignatureVersion, set_preimage};

#[test]
fn distinct_key_value_pairs_have_distinct_signing_preimages() {
    assert_ne!(
        set_preimage(SignatureVersion::V2, "a\0b", "c", &[7; 32]),
        set_preimage(SignatureVersion::V2, "a", "b\0c", &[7; 32])
    );
}
