use mnesis_macros::transforms;

mod zero_from {
    use super::*;
#[transforms(aggregate = (), error = std::convert::Infallible)]
impl Marker { #[transform(event = "E", from = 0, to = 1)] fn step(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> { Ok(payload.to_vec()) } }
}

mod zero_to {
    use super::*;
#[transforms(aggregate = (), error = std::convert::Infallible)]
impl Marker { #[transform(event = "E", from = 1, to = 0)] fn step(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> { Ok(payload.to_vec()) } }
}

mod max_from {
    use super::*;
#[transforms(aggregate = (), error = std::convert::Infallible)]
impl Marker { #[transform(event = "E", from = 4294967295, to = 1)] fn step(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> { Ok(payload.to_vec()) } }
}

mod oversized_from {
    use super::*;
#[transforms(aggregate = (), error = std::convert::Infallible)]
impl Marker { #[transform(event = "E", from = 18446744073709551615, to = 2)] fn step(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> { Ok(payload.to_vec()) } }
}

mod oversized_to {
    use super::*;
#[transforms(aggregate = (), error = std::convert::Infallible)]
impl Marker { #[transform(event = "E", from = 1, to = 4294967296)] fn step(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> { Ok(payload.to_vec()) } }
}

mod max_destination_without_history {
    use super::*;
#[transforms(aggregate = (), error = std::convert::Infallible)]
impl Marker { #[transform(event = "E", from = 4294967294, to = 4294967295)] fn step(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> { Ok(payload.to_vec()) } }
}

fn main() {}
