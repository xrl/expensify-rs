//! Every misuse the type design claims to make unrepresentable, as a
//! compile-fail case. If one of these starts compiling, a bound was loosened
//! and the guarantee is gone — that is the point of committing the `.stderr`
//! files alongside them.

#[test]
fn misuses_do_not_compile() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/*.rs");
}
