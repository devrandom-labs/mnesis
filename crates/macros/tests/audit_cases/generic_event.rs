#[derive(Debug, mnesis::DomainEvent)]
enum GenericEvent<T: Send + Sync + std::fmt::Debug + 'static> {
    Value(T),
}

#[derive(Debug, mnesis::DomainEvent)]
enum WithWhere<T, const N: usize>
where
    T: Send + Sync + std::fmt::Debug + 'static,
{
    Array([T; N]),
    Empty,
}

#[derive(Debug, mnesis::DomainEvent)]
enum WithLifetime<'a: 'static> {
    Text(&'a str),
}

fn assert_event<E: mnesis::DomainEvent>(event: E, expected: &str) {
    assert_eq!(event.name(), expected);
}

fn main() {
    assert_event(GenericEvent::Value(42_u8), "Value");
    assert_event(WithWhere::Array([1_u8, 2]), "Array");
    assert_event(WithWhere::<u8, 0>::Empty, "Empty");
    assert_event(WithLifetime::Text("value"), "Text");
}
