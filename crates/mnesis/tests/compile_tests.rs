#[test]
fn compile_fail_tests() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail/*.rs");
}

/// `version!(0)` fails via a `panic!` evaluated inside an inline `const {}`
/// block. `cargo check` — which is what trybuild runs above, since that
/// `TestCases` has no `pass` case — type-checks without fully evaluating an
/// anonymous inline-const expression, so it does not observe the panic and
/// the case would wrongly report "compiled successfully". Registering a
/// `pass` case flips trybuild to `cargo build` (trybuild only picks `build`
/// over `check` when at least one `pass` case is present), which does run
/// codegen-time const evaluation and correctly fails. Kept in its own
/// subdirectory (not matched by the `*.rs` glob above) and its own
/// `TestCases` so the cheaper `check` mode above is unaffected.
#[test]
fn compile_fail_version_zero() {
    let t = trybuild::TestCases::new();
    t.pass("tests/compile_fail/version_literal/nonzero.rs");
    t.compile_fail("tests/compile_fail/version_literal/zero.rs");
}
