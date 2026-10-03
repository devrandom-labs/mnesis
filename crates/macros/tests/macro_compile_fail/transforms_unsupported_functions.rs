use mnesis_macros::transforms;

mod async_fn {
    use super::*;
#[transforms(aggregate = (), error = std::convert::Infallible)]
impl Marker { #[transform(event = "E", from = 1, to = 2)] async fn step(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> { unreachable!() } }
}

mod unsafe_fn {
    use super::*;
#[transforms(aggregate = (), error = std::convert::Infallible)]
impl Marker { #[transform(event = "E", from = 1, to = 2)] unsafe fn step(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> { unreachable!() } }
}

mod const_fn {
    use super::*;
#[transforms(aggregate = (), error = std::convert::Infallible)]
impl Marker { #[transform(event = "E", from = 1, to = 2)] const fn step(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> { unreachable!() } }
}

mod abi {
    use super::*;
#[transforms(aggregate = (), error = std::convert::Infallible)]
impl Marker { #[transform(event = "E", from = 1, to = 2)] extern "C" fn step(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> { unreachable!() } }
}

mod type_parameter {
    use super::*;
#[transforms(aggregate = (), error = std::convert::Infallible)]
impl Marker { #[transform(event = "E", from = 1, to = 2)] fn step<T>(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> { unreachable!() } }
}

mod lifetime_parameter {
    use super::*;
#[transforms(aggregate = (), error = std::convert::Infallible)]
impl Marker { #[transform(event = "E", from = 1, to = 2)] fn step<'a>(payload: &'a [u8]) -> Result<Vec<u8>, std::convert::Infallible> { unreachable!() } }
}

mod const_parameter {
    use super::*;
#[transforms(aggregate = (), error = std::convert::Infallible)]
impl Marker { #[transform(event = "E", from = 1, to = 2)] fn step<const N: usize>(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> { unreachable!() } }
}

mod where_clause {
    use super::*;
#[transforms(aggregate = (), error = std::convert::Infallible)]
impl Marker { #[transform(event = "E", from = 1, to = 2)] fn step(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> where (): Sized { unreachable!() } }
}

mod no_payload {
    use super::*;
#[transforms(aggregate = (), error = std::convert::Infallible)]
impl Marker { #[transform(event = "E", from = 1, to = 2)] fn step() -> Result<Vec<u8>, std::convert::Infallible> { unreachable!() } }
}

mod many_payloads {
    use super::*;
#[transforms(aggregate = (), error = std::convert::Infallible)]
impl Marker { #[transform(event = "E", from = 1, to = 2)] fn step(a: &[u8], b: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> { unreachable!() } }
}

mod receiver {
    use super::*;
#[transforms(aggregate = (), error = std::convert::Infallible)]
impl Marker { #[transform(event = "E", from = 1, to = 2)] fn step(&self) -> Result<Vec<u8>, std::convert::Infallible> { unreachable!() } }
}

mod no_return {
    use super::*;
#[transforms(aggregate = (), error = std::convert::Infallible)]
impl Marker { #[transform(event = "E", from = 1, to = 2)] fn step(payload: &[u8]) { unreachable!() } }
}

mod cfg {
    use super::*;
#[transforms(aggregate = (), error = std::convert::Infallible)]
impl Marker { #[cfg(any())] #[transform(event = "E", from = 1, to = 2)] fn step(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> { Ok(payload.to_vec()) } }
}

mod cfg_attr {
    use super::*;
#[transforms(aggregate = (), error = std::convert::Infallible)]
impl Marker { #[cfg_attr(all(), inline)] #[transform(event = "E", from = 1, to = 2)] fn step(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> { Ok(payload.to_vec()) } }
}

fn main() {}
