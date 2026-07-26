//! Fuzz target for the event-type value newtype (UTF-8 + length-cap validator).
//!
//! Body lives in `fuzz-common` (shared with the afl.rs harness). The test name
//! IS the target name `cargo bolero test value_event_type` selects.

#[test]
fn value_event_type() {
    bolero::check!().for_each(|input: &[u8]| fuzz_common::value_event_type(input));
}
