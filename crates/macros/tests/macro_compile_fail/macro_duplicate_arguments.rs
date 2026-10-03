use mnesis_macros::transforms;

mod duplicate_event {
    use super::*;
#[transforms(aggregate = (), error = std::convert::Infallible)]
impl Marker { #[transform(event = "E", event = "F", from = 1, to = 2)] fn step(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> { Ok(payload.to_vec()) } }
}

mod duplicate_from {
    use super::*;
#[transforms(aggregate = (), error = std::convert::Infallible)]
impl Marker { #[transform(event = "E", from = 1, from = 1, to = 2)] fn step(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> { Ok(payload.to_vec()) } }
}

mod duplicate_to {
    use super::*;
#[transforms(aggregate = (), error = std::convert::Infallible)]
impl Marker { #[transform(event = "E", from = 1, to = 2, to = 2)] fn step(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> { Ok(payload.to_vec()) } }
}

mod duplicate_rename {
    use super::*;
#[transforms(aggregate = (), error = std::convert::Infallible)]
impl Marker { #[transform(event = "E", from = 1, to = 2, rename = "F", rename = "G")] fn step(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> { Ok(payload.to_vec()) } }
}

mod duplicate_outer_aggregate {
    use super::*;
#[transforms(aggregate = (), aggregate = (), error = std::convert::Infallible)]
impl Marker { #[transform(event = "E", from = 1, to = 2)] fn step(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> { Ok(payload.to_vec()) } }
}

mod duplicate_outer_error {
    use super::*;
#[transforms(aggregate = (), error = (), error = std::convert::Infallible)]
impl Marker { #[transform(event = "E", from = 1, to = 2)] fn step(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> { Ok(payload.to_vec()) } }
}

mod duplicate_attribute {
    use super::*;
#[transforms(aggregate = (), error = std::convert::Infallible)]
impl Marker { #[transform(event = "E", from = 1, to = 2)] #[transform(event = "E", from = 1, to = 2)] fn step(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> { Ok(payload.to_vec()) } }
}

mod duplicate_aggregate_state {
    use super::*;
#[mnesis::aggregate(state = (), error = (), id = (), state = ())]
struct Marker;
}

mod duplicate_aggregate_error {
    use super::*;
#[mnesis::aggregate(state = (), error = (), id = (), error = ())]
struct Marker;
}

mod duplicate_aggregate_id {
    use super::*;
#[mnesis::aggregate(state = (), error = (), id = (), id = ())]
struct Marker;
}

fn main() {}
