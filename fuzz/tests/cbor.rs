//! Fuzz targets for the backup-box CBOR decoders (header + chunk body).
//!
//! Bodies live in `fuzz-common` (shared with the afl.rs harness). Each test name
//! IS the target name `cargo bolero test <name>` selects.

#[test]
fn cbor_decode_header() {
    bolero::check!().for_each(|input: &[u8]| fuzz_common::cbor_decode_header(input));
}

#[test]
fn cbor_decode_chunk() {
    bolero::check!().for_each(|input: &[u8]| fuzz_common::cbor_decode_chunk(input));
}
