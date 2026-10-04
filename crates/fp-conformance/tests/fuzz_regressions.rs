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

/// Fuzz Nightly 37112426533: `466667723*"23sezz%z"*"*"` (policy byte 1,
/// hardened) asked for a 3.7 GB string on every row and ran the target
/// past libFuzzer's 2 GB RSS limit (oom-f7cbb753...). Hardened mode now
/// refuses a repeated string past its byte budget before allocating it,
/// and `DataFrame.eval` of a constant computes it once (br-frankenpandas-2blaf).
#[test]
fn regression_fuzz_dataframe_eval_repeated_string_oom_2026_10_03() {
    let input = include_bytes!(
        "../../../fuzz/regressions/fuzz_dataframe_eval/repeated_string_oom_2026_10_03.txt"
    );
    assert!(fp_conformance::fuzz_dataframe_eval_bytes(input).is_ok());
    let expr = String::from_utf8_lossy(&input[1..]);
    let frame =
        fp_frame::DataFrame::from_dict(&["a"], vec![("a", vec![fp_types::Scalar::Int64(1); 8])])
            .unwrap();
    let hardened = fp_runtime::RuntimePolicy::hardened(Some(100_000));
    // 8 bytes x 466667723 a row: the column form counts all 8 rows, the
    // value form its one.
    for (column_form, bytes) in [(true, 29_866_734_272_u64), (false, 3_733_341_784)] {
        let mut ledger = fp_runtime::EvidenceLedger::new();
        let refused = if column_form {
            fp_expr::eval_str_with_locals(
                &expr,
                &frame,
                &std::collections::BTreeMap::new(),
                &hardened,
                &mut ledger,
            )
            .map(|_| ())
        } else {
            fp_expr::eval_value_str_with_locals(
                &expr,
                &frame,
                &std::collections::BTreeMap::new(),
                &hardened,
                &mut ledger,
            )
            .map(|_| ())
        };
        let refusal = format!("hardened mode refuses a repeated string of {bytes} bytes");
        assert!(
            refused.is_err_and(|err| err.to_string().contains(&refusal)),
            "column form {column_form}"
        );
    }
    // NEGATIVE: strict mode builds what pandas builds - a small repetition
    // of a constant is one value, not a row each.
    let mut ledger = fp_runtime::EvidenceLedger::new();
    let value = fp_expr::eval_value_str_with_locals(
        "3*\"ab\"",
        &frame,
        &std::collections::BTreeMap::new(),
        &fp_runtime::RuntimePolicy::strict(),
        &mut ledger,
    )
    .unwrap();
    assert!(matches!(
        value,
        fp_expr::EvalValue::Scalar(fp_types::Scalar::Utf8(ref text)) if text == "ababab"
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
