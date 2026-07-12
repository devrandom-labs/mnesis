/// DomainEvent enum must have at least one variant.

#[derive(Debug, Clone, mnesis::DomainEvent)]
enum EmptyEvent {}

fn main() {}
