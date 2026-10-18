use mnesis_store::{EventMorsel, SchemaVersion};

#[mnesis_macros::transforms(aggregate = (), error = std::convert::Infallible)]
#[cfg(all())]
impl RevisitedName {
    const FINAL: u8 = 3;
    fn identity<T>(value: T) -> T { value }
    #[transform(event = "Left", from = 1, to = 2, rename = "Right")]
    fn left(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> {
        let mut data = Self::identity(payload.to_vec());
        data.push(2);
        Ok(data)
    }
    #[transform(event = "Right", from = 2, to = 3, rename = "Left")]
    fn right(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> {
        let mut data = payload.to_vec();
        data.push(Self::FINAL);
        Ok(data)
    }
}

#[mnesis_macros::transforms(aggregate = (), error = std::convert::Infallible)]
impl Converging {
    #[transform(event = "A", from = 1, to = 2, rename = "C")]
    fn a(_: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> { Ok(vec![1]) }
    #[transform(event = "B", from = 1, to = 2, rename = "C")]
    fn b(_: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> { Ok(vec![2]) }
    #[transform(event = "C", from = 2, to = 3)]
    fn c(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> {
        let mut data = payload.to_vec(); data.push(3); Ok(data)
    }
}

#[mnesis_macros::transforms(aggregate = (), error = std::convert::Infallible)]
#[cfg(any())]
impl Disabled {
    #[transform(event = "E", from = 1, to = 2)]
    fn unavailable(_: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> { missing_function() }
}

fn main() {
    let revisited = RevisitedName::upcast(EventMorsel::borrowed("Left", SchemaVersion::INITIAL, &[1])).unwrap();
    assert_eq!(revisited.event_type(), "Left");
    assert_eq!(revisited.payload(), &[1, 2, 3]);
    assert_eq!(revisited.schema_version(), SchemaVersion::from_u32(3).unwrap());
    assert_eq!(RevisitedName::current_version("Right"), Some(SchemaVersion::from_u32(2).unwrap()));
    for (name, expected) in [("A", vec![1, 3]), ("B", vec![2, 3])] {
        let result = Converging::upcast(EventMorsel::borrowed(name, SchemaVersion::INITIAL, &[])).unwrap();
        assert_eq!(result.event_type(), "C");
        assert_eq!(result.payload(), expected);
        assert_eq!(result.schema_version(), Converging::current_version("C").unwrap());
    }
    let unknown = Converging::upcast(EventMorsel::borrowed("Unknown", SchemaVersion::from_u32(u32::MAX).unwrap(), &[7])).unwrap();
    assert!(unknown.is_borrowed());
    assert_eq!(unknown.payload(), &[7]);
    assert_eq!(unknown.schema_version(), SchemaVersion::from_u32(u32::MAX).unwrap());
}
