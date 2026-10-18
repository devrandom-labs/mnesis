#[mnesis_macros::transforms(aggregate = (), error = std::convert::Infallible)]
impl RenamedChain {
    #[transform(event = "Old", from = 1, to = 2, rename = "New")]
    fn first(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> {
        let mut payload = payload.to_vec();
        payload.push(2);
        Ok(payload)
    }
    #[transform(event = "New", from = 2, to = 3)]
    fn second(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> {
        let mut payload = payload.to_vec();
        payload.push(3);
        Ok(payload)
    }
}
fn main() {
    use mnesis_store::{EventMorsel, SchemaVersion};
    let result = RenamedChain::upcast(EventMorsel::borrowed("Old", SchemaVersion::INITIAL, &[1])).unwrap();
    assert_eq!(result.event_type(), "New");
    assert_eq!(result.schema_version(), SchemaVersion::from_u32(3).unwrap());
    assert_eq!(result.payload(), &[1, 2, 3]);
    assert_eq!(RenamedChain::current_version("Old"), Some(SchemaVersion::INITIAL));
    assert_eq!(RenamedChain::current_version("New"), Some(SchemaVersion::from_u32(3).unwrap()));
    let again = RenamedChain::upcast(result).unwrap();
    assert_eq!(again.payload(), &[1, 2, 3]);
}
