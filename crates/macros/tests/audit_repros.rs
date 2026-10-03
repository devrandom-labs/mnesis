//! A12 in todo.md. Valid generic events and rename chains must compile.
#[test]
fn generic_event_compiles() {
    trybuild::TestCases::new().pass("tests/audit_cases/generic_event.rs");
}

#[test]
fn renamed_transform_chain_compiles() {
    trybuild::TestCases::new().pass("tests/audit_cases/renamed_chain.rs");
}

#[test]
fn transform_graphs_have_deterministic_terminating_dispatch() {
    trybuild::TestCases::new().pass("tests/audit_cases/graph_contract.rs");
}

#[test]
fn reproduce_transform_overflow_diagnostic() {
    trybuild::TestCases::new().compile_fail("tests/audit_cases/overflow.rs");
}
