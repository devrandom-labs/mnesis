//! Fuzz target for the canonical on-disk frame decoder.
//!
//! The body lives in `fuzz-common` (shared with the afl.rs harness). The test
//! name IS the target name `cargo bolero test wire_decode_frame` selects.

#[test]
fn wire_decode_frame() {
    bolero::check!().for_each(|input: &[u8]| fuzz_common::wire_decode_frame(input));
}
