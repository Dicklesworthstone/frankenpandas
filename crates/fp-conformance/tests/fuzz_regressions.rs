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

// Sentinel test: proves this file compiles + gets picked up by
// `cargo test -p fp-conformance --test fuzz_regressions` even when the
// regression corpus is empty. Replace or keep as a canary once real
// regressions land.
#[test]
fn fuzz_regressions_module_compiles() {
    // No-op. Existence of this test is the signal.
}
