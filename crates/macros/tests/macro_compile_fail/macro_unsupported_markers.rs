use mnesis_macros::transforms;

mod generic {
    use super::*;
#[transforms(aggregate = (), error = std::convert::Infallible)]
impl<T> Marker { #[transform(event = "E", from = 1, to = 2)] fn step(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> { Ok(payload.to_vec()) } }
}

mod lifetime {
    use super::*;
#[transforms(aggregate = (), error = std::convert::Infallible)]
impl<'a> Marker { #[transform(event = "E", from = 1, to = 2)] fn step(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> { Ok(payload.to_vec()) } }
}

mod constant {
    use super::*;
#[transforms(aggregate = (), error = std::convert::Infallible)]
impl<const N: usize> Marker { #[transform(event = "E", from = 1, to = 2)] fn step(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> { Ok(payload.to_vec()) } }
}

mod where_clause {
    use super::*;
#[transforms(aggregate = (), error = std::convert::Infallible)]
impl Marker where (): Sized { #[transform(event = "E", from = 1, to = 2)] fn step(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> { Ok(payload.to_vec()) } }
}

mod qualified {
    use super::*;
#[transforms(aggregate = (), error = std::convert::Infallible)]
impl path::Marker { #[transform(event = "E", from = 1, to = 2)] fn step(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> { Ok(payload.to_vec()) } }
}

mod arguments {
    use super::*;
#[transforms(aggregate = (), error = std::convert::Infallible)]
impl Marker<u8> { #[transform(event = "E", from = 1, to = 2)] fn step(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> { Ok(payload.to_vec()) } }
}

mod trait_impl {
    use super::*;
#[transforms(aggregate = (), error = std::convert::Infallible)]
impl SomeTrait for Marker { #[transform(event = "E", from = 1, to = 2)] fn step(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> { Ok(payload.to_vec()) } }
}

mod unsafe_impl {
    use super::*;
#[transforms(aggregate = (), error = std::convert::Infallible)]
unsafe impl SomeTrait for Marker { #[transform(event = "E", from = 1, to = 2)] fn step(payload: &[u8]) -> Result<Vec<u8>, std::convert::Infallible> { Ok(payload.to_vec()) } }
}

mod associated_type {
    use super::*;
#[transforms(aggregate = (), error = std::convert::Infallible)]
impl Marker { type Payload = Vec<u8>; }
}

mod associated_macro {
    use super::*;
#[transforms(aggregate = (), error = std::convert::Infallible)]
impl Marker { unknown!(); }
}

mod reserved_upcast {
    use super::*;
#[transforms(aggregate = (), error = std::convert::Infallible)]
impl Marker { fn upcast() {} }
}

mod reserved_current_version {
    use super::*;
#[transforms(aggregate = (), error = std::convert::Infallible)]
impl Marker { fn current_version() {} }
}

mod aggregate_type_parameter {
    use super::*;
#[mnesis::aggregate(state = (), error = (), id = ())]
struct Marker<T>;
}

mod aggregate_lifetime_parameter {
    use super::*;
#[mnesis::aggregate(state = (), error = (), id = ())]
struct Marker<'a>;
}

mod aggregate_const_parameter {
    use super::*;
#[mnesis::aggregate(state = (), error = (), id = ())]
struct Marker<const N: usize>;
}

mod aggregate_where_clause {
    use super::*;
#[mnesis::aggregate(state = (), error = (), id = ())]
struct Marker where (): Sized;
}

mod aggregate_empty_fields {
    use super::*;
#[mnesis::aggregate(state = (), error = (), id = ())]
struct Marker {}
}

fn main() {}
