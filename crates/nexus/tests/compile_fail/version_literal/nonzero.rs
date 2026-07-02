fn main() {
    let v = nexus::version!(3);
    assert_eq!(v.as_u64(), 3);
}
