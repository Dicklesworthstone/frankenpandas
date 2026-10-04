//! Fuzz crash regression tests.
//!
//! Each test in this file replays a minimized crash artifact from
//! `fuzz/regressions/<target>/` against the fuzz harness pub fn that
//! originally crashed. Tests must PASS (= no panic / no unexpected Err)
//! on the post-fix code.
//!
//! See `fuzz/regressions/README.md` for the add-a-regression workflow.
//!
//! Tracked under br-frankenpandas-lvl6.

// Example template (uncomment + fill in when the first crash arrives):
//
// #[test]
// fn regression_fuzz_csv_parse_unterminated_quote_2026_05_01() {
//     let input = include_bytes!(
//         "../../../fuzz/regressions/fuzz_csv_parse/unterminated_quote_2026_05_01.csv"
//     );
//     // Must not panic. May return Err — that's the expected fix semantics.
//     let _ = fp_conformance::fuzz_csv_parse_bytes(input);
// }

/// Fuzz Nightly 36992343034: 1905 nested '(' overflowed the parser's stack
/// (ASan stack-overflow, an abort). pandas raises TokenError past 200
/// nested brackets; fp refuses them as a parse error (br-frankenpandas-bhbuz).
#[test]
fn regression_fuzz_parse_expr_deep_parentheses_2026_10_02() {
    let input =
        include_bytes!("../../../fuzz/regressions/fuzz_parse_expr/deep_parentheses_2026_10_02.txt");
    assert!(fp_conformance::fuzz_parse_expr_bytes(input).is_ok());
    let text = String::from_utf8_lossy(input);
    assert!(matches!(
        fp_expr::parse_expr(&text),
        Err(fp_expr::ExprError::ParseError(message)) if message == "too many nested parentheses"
    ));
}

// Sentinel test: proves this file compiles + gets picked up by
// `cargo test -p fp-conformance --test fuzz_regressions` even when the
// regression corpus is empty. Replace or keep as a canary once real
// regressions land.
#[test]
fn fuzz_regressions_module_compiles() {
    // No-op. Existence of this test is the signal.
}
