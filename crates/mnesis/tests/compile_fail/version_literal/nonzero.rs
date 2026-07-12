fn main() {
    let v = mnesis::version!(3);
    assert_eq!(v.as_u64(), 3);
}
