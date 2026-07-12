/// DomainEvent derive must be used on enums, not structs.
/// This should fail with a clear error message.

#[derive(Debug, mnesis_macros::DomainEvent)]
struct NotAnEnum {
    name: String,
}

fn main() {}
