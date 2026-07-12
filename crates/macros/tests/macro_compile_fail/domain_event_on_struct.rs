/// DomainEvent derive must reject structs.

#[derive(Debug, mnesis::DomainEvent)]
struct NotAnEnum {
    name: String,
}

fn main() {}
