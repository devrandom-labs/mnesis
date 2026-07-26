fn main() {
    afl::fuzz!(|data: &[u8]| fuzz_common::wire_decode_frame(data));
}
