use mnesis_macros::transforms;

mod ambiguous_rename {
    use super::*;
#[transforms(aggregate = (), error = std::convert::Infallible)]
impl Marker { #[transform(event = "E", from = 1, to = 2, rename = "F")] fn step(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> { Ok(payload.to_vec()) }
 #[transform(event = "E", from = 1, to = 2, rename = "G")] fn other(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> { Ok(payload.to_vec()) } }
}

mod schema_cycle {
    use super::*;
#[transforms(aggregate = (), error = std::convert::Infallible)]
impl Marker { #[transform(event = "E", from = 1, to = 1)] fn step(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> { Ok(payload.to_vec()) } }
}

mod orphan_rename {
    use super::*;
#[transforms(aggregate = (), error = std::convert::Infallible)]
impl Marker { #[transform(event = "Old", from = 1, to = 2, rename = "New")] fn step(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> { Ok(payload.to_vec()) }
 #[transform(event = "New", from = 3, to = 4)] fn other(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> { Ok(payload.to_vec()) } }
}

mod incompatible_terminal_schemas {
    use super::*;
#[transforms(aggregate = (), error = std::convert::Infallible)]
impl Marker {
#[transform(event = "Old", from = 1, to = 2, rename = "New")] fn old(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> { Ok(payload.to_vec()) }
#[transform(event = "Alias", from = 1, to = 2, rename = "Bridge")] fn alias(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> { Ok(payload.to_vec()) }
#[transform(event = "Bridge", from = 2, to = 3)] fn bridge(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> { Ok(payload.to_vec()) }
#[transform(event = "Bridge", from = 3, to = 4, rename = "New")] fn last(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> { Ok(payload.to_vec()) }
}
}

fn main() {}
