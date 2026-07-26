fn main() {
    afl::fuzz!(|data: &[u8]| fuzz_common::value_event_type(data));
}
