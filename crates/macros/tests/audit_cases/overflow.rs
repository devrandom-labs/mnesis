#[mnesis_macros::transforms(aggregate = (), error = std::convert::Infallible)]
impl OverflowTransforms {
    #[transform(event = "E", from = 18446744073709551615, to = 0)]
    fn change(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> {
        Ok(payload.to_vec())
    }
}
fn main() {}
