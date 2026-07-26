fn main() {
    afl::fuzz!(|data: &[u8]| fuzz_common::cbor_decode_header(data));
}
