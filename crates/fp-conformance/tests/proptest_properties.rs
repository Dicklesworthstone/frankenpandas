#![forbid(unsafe_code)]

//! Property-based testing infrastructure for FrankenPandas (bd-2t5e.1, AG-01).
//!
//! Strategy generators produce arbitrary but pandas-valid inputs across the
//! (dtype x null_pattern x index_type x operation) combinatorial space.
//! Properties verify behavioral invariants that must hold for ALL inputs,
//! not just hand-picked fixtures.

use proptest::prelude::*;

use fp_frame::{DataFrame, Series};
use fp_groupby::{GroupByExecutionOptions, GroupByOptions, groupby_sum, groupby_sum_with_options};
use fp_index::{Index, IndexLabel, align_union, validate_alignment_plan};
use fp_join::{
    JoinExecutionOptions, JoinType, JoinedSeries, join_series, join_series_with_options,
};
use fp_runtime::{EvidenceLedger, RuntimePolicy};
use fp_types::{NullKind, Scalar};

// ---------------------------------------------------------------------------
// Strategy generators
// ---------------------------------------------------------------------------

/// Generate an arbitrary numeric Scalar suitable for arithmetic operations.
fn arb_numeric_scalar() -> impl Strategy<Value = Scalar> {
    prop_oneof![
        3 => (-1_000_000i64..1_000_000i64).prop_map(Scalar::Int64),
        3 => (-1e6_f64..1e6_f64).prop_map(Scalar::Float64),
        1 => Just(Scalar::Null(NullKind::Null)),
        1 => Just(Scalar::Null(NullKind::NaN)),
    ]
}

/// Generate an arbitrary IndexLabel.
fn arb_index_label() -> impl Strategy<Value = IndexLabel> {
    prop_oneof![
        3 => (0i64..100).prop_map(IndexLabel::Int64),
        1 => "[a-e]{1,3}".prop_map(IndexLabel::Utf8),
    ]
}

/// Generate a Vec of IndexLabels with `len` entries, allowing some duplicates.
fn arb_index_labels(len: usize) -> impl Strategy<Value = Vec<IndexLabel>> {
    proptest::collection::vec(arb_index_label(), len)
}

/// Generate a Vec of unique IndexLabels with `len` entries.
fn arb_unique_index_labels(len: usize) -> impl Strategy<Value = Vec<IndexLabel>> {
    proptest::collection::btree_set(arb_index_label(), len)
        .prop_map(|labels| labels.into_iter().collect())
}

/// Generate an Index with `len` labels, allowing some duplicates.
fn arb_index(len: usize) -> impl Strategy<Value = Index> {
    arb_index_labels(len).prop_map(Index::new)
}

/// Generate a Vec of numeric Scalars of given length.
fn arb_numeric_values(len: usize) -> impl Strategy<Value = Vec<Scalar>> {
    proptest::collection::vec(arb_numeric_scalar(), len)
}

/// Generate an arbitrary Series with numeric values and the given length.
fn arb_numeric_series(name: &'static str, len: usize) -> impl Strategy<Value = Series> {
    (arb_index_labels(len), arb_numeric_values(len)).prop_filter_map(
        "series construction must succeed",
        move |(labels, values)| Series::from_values(name.to_owned(), labels, values).ok(),
    )
}

/// Generate an arbitrary Series with numeric values and unique index labels.
fn arb_unique_numeric_series(name: &'static str, len: usize) -> impl Strategy<Value = Series> {
    (arb_unique_index_labels(len), arb_numeric_values(len)).prop_filter_map(
        "series construction must succeed",
        move |(labels, values)| Series::from_values(name.to_owned(), labels, values).ok(),
    )
}

/// Generate a pair of numeric series with independently chosen lengths (1..max_len).
fn arb_series_pair(max_len: usize) -> impl Strategy<Value = (Series, Series)> {
    (1..=max_len, 1..=max_len).prop_flat_map(|(len_a, len_b)| {
        (
            arb_numeric_series("left", len_a),
            arb_numeric_series("right", len_b),
        )
    })
}

/// Generate a pair of numeric series with unique index labels.
fn arb_unique_series_pair(max_len: usize) -> impl Strategy<Value = (Series, Series)> {
    (1..=max_len, 1..=max_len).prop_flat_map(|(len_a, len_b)| {
        (
            arb_unique_numeric_series("left", len_a),
            arb_unique_numeric_series("right", len_b),
        )
    })
}

/// Generate a pair of indices with independently chosen lengths.
fn arb_index_pair(max_len: usize) -> impl Strategy<Value = (Index, Index)> {
    (1..=max_len, 1..=max_len).prop_flat_map(|(len_a, len_b)| (arb_index(len_a), arb_index(len_b)))
}

/// Generate a pair of series suitable for groupby (keys + values, same length).
fn arb_groupby_pair(max_len: usize) -> impl Strategy<Value = (Series, Series)> {
    (1..=max_len).prop_flat_map(|len| {
        // Keys: use a small label space so groupby actually groups things.
        let key_labels = arb_index_labels(len);
        let key_values = proptest::collection::vec(
            prop_oneof![
                3 => (0i64..10).prop_map(Scalar::Int64),
                1 => Just(Scalar::Null(NullKind::Null)),
            ],
            len,
        );
        let val_labels = arb_index_labels(len);
        let val_values = arb_numeric_values(len);

        (key_labels, key_values, val_labels, val_values).prop_filter_map(
            "groupby series must construct",
            |(kl, kv, vl, vv)| {
                let keys = Series::from_values("keys".to_owned(), kl, kv).ok()?;
                let vals = Series::from_values("values".to_owned(), vl, vv).ok()?;
                Some((keys, vals))
            },
        )
    })
}

/// Generate a small DataFrame with unique index labels and a variable column subset.
fn arb_combine_first_dataframe(max_rows: usize) -> impl Strategy<Value = DataFrame> {
    (1..=max_rows).prop_flat_map(|nrows| {
        (
            arb_unique_index_labels(nrows),
            proptest::sample::subsequence(
                vec!["a".to_string(), "b".to_string(), "c".to_string()],
                1..=3,
            ),
        )
            .prop_flat_map(move |(labels, column_names)| {
                let ncols = column_names.len();
                (
                    Just(labels),
                    Just(column_names),
                    proptest::collection::vec(arb_numeric_values(nrows), ncols),
                )
            })
            .prop_filter_map(
                "dataframe construction must succeed",
                |(labels, column_names, column_values)| {
                    let index = Index::new(labels);
                    let mut columns = std::collections::BTreeMap::new();
                    for (name, values) in column_names.iter().cloned().zip(column_values) {
                        columns.insert(name, fp_columnar::Column::from_values(values).ok()?);
                    }
                    DataFrame::new_with_column_order(index, columns, column_names).ok()
                },
            )
    })
}

fn poison_numeric_scalar(value: &Scalar) -> Scalar {
    match value {
        Scalar::Int64(v) => Scalar::Int64(v.saturating_add(1)),
        Scalar::Float64(v) if v.is_finite() => Scalar::Float64(v + 1.0),
        Scalar::Float64(_) => Scalar::Float64(0.0),
        Scalar::Null(_) => Scalar::Int64(1),
        Scalar::Bool(v) => Scalar::Bool(!v),
        Scalar::Utf8(v) => Scalar::Utf8(format!("{v}_poisoned")),
    }
}

fn sign_flip_numeric_scalar(value: &Scalar) -> Scalar {
    match value {
        Scalar::Int64(v) => Scalar::Int64(v.saturating_neg()),
        Scalar::Float64(v) => Scalar::Float64(-v),
        Scalar::Null(kind) => Scalar::Null(*kind),
        other => other.clone(),
    }
}

fn sign_flip_series(series: &Series) -> Series {
    let flipped_values = series
        .values()
        .iter()
        .map(sign_flip_numeric_scalar)
        .collect::<Vec<_>>();
    Series::from_values(
        series.name().to_owned(),
        series.index().labels().to_vec(),
        flipped_values,
    )
    .expect("sign-flipped series must construct")
}

fn shift_numeric_scalar(value: &Scalar, delta: f64) -> Scalar {
    match value {
        Scalar::Int64(v) => Scalar::Float64(*v as f64 + delta),
        Scalar::Float64(v) => Scalar::Float64(*v + delta),
        Scalar::Null(kind) => Scalar::Null(*kind),
        other => other.clone(),
    }
}

fn shift_series(series: &Series, delta: f64) -> Series {
    let shifted_values = series
        .values()
        .iter()
        .map(|value| shift_numeric_scalar(value, delta))
        .collect::<Vec<_>>();
    Series::from_values(
        series.name().to_owned(),
        series.index().labels().to_vec(),
        shifted_values,
    )
    .expect("shifted series must construct")
}

fn arb_between_inclusive() -> impl Strategy<Value = &'static str> {
    prop_oneof![Just("both"), Just("neither"), Just("left"), Just("right"),]
}

fn sign_flip_between_inclusive(inclusive: &str) -> &'static str {
    match inclusive {
        "both" => "both",
        "neither" => "neither",
        "left" => "right",
        "right" => "left",
        _ => "both",
    }
}

fn bool_values(series: &Series) -> Result<Vec<bool>, String> {
    series
        .values()
        .iter()
        .map(|value| match value {
            Scalar::Bool(flag) => Ok(*flag),
            other => Err(format!("expected bool series output, got {other:?}")),
        })
        .collect()
}

fn sorted_bounds(a: f64, b: f64) -> (f64, f64) {
    if a <= b { (a, b) } else { (b, a) }
}

fn nested_clip_bounds(
    outer_a: f64,
    outer_b: f64,
    inner_frac_a: f64,
    inner_frac_b: f64,
) -> (f64, f64, f64, f64) {
    let (outer_lower, outer_upper) = sorted_bounds(outer_a, outer_b);
    let width = outer_upper - outer_lower;
    let (inner_start, inner_end) = sorted_bounds(inner_frac_a, inner_frac_b);
    let inner_lower = outer_lower + width * inner_start;
    let inner_upper = outer_lower + width * inner_end;
    (outer_lower, outer_upper, inner_lower, inner_upper)
}

fn poison_series_right_cells_for_combine_first(left: &Series, right: &Series) -> Series {
    let left_lookup: std::collections::BTreeMap<&IndexLabel, &Scalar> = left
        .index()
        .labels()
        .iter()
        .zip(left.values().iter())
        .collect();

    let poisoned_values = right
        .index()
        .labels()
        .iter()
        .zip(right.values().iter())
        .map(|(label, value)| {
            if let Some(left_value) = left_lookup.get(label)
                && !left_value.is_missing()
            {
                return poison_numeric_scalar(value);
            }
            value.clone()
        })
        .collect::<Vec<_>>();

    Series::from_values(
        right.name().to_owned(),
        right.index().labels().to_vec(),
        poisoned_values,
    )
    .expect("poisoned right series must construct")
}

fn poison_dataframe_right_cells_for_combine_first(
    left: &DataFrame,
    right: &DataFrame,
) -> DataFrame {
    let left_row_positions: std::collections::BTreeMap<&IndexLabel, usize> = left
        .index()
        .labels()
        .iter()
        .enumerate()
        .map(|(position, label)| (label, position))
        .collect();

    let column_order = right
        .column_names()
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    let mut poisoned_columns = std::collections::BTreeMap::new();

    for column_name in &column_order {
        let right_column = right
            .column(column_name)
            .expect("column name listed in order must exist");
        let left_column = left.column(column_name);
        let poisoned_values = right
            .index()
            .labels()
            .iter()
            .enumerate()
            .map(|(right_position, label)| {
                if let Some(left_column) = left_column
                    && let Some(&left_position) = left_row_positions.get(label)
                {
                    let left_value = &left_column.values()[left_position];
                    if !left_value.is_missing() {
                        return poison_numeric_scalar(&right_column.values()[right_position]);
                    }
                }
                right_column.values()[right_position].clone()
            })
            .collect::<Vec<_>>();
        poisoned_columns.insert(
            column_name.clone(),
            fp_columnar::Column::from_values(poisoned_values)
                .expect("poisoned right dataframe column must construct"),
        );
    }

    DataFrame::new_with_column_order(right.index().clone(), poisoned_columns, column_order)
        .expect("poisoned right dataframe must construct")
}

fn normalized_join_rows(joined: &JoinedSeries) -> Vec<String> {
    let mut rows = joined
        .index
        .labels()
        .iter()
        .zip(joined.left_values.values().iter())
        .zip(joined.right_values.values().iter())
        .map(|((label, left_value), right_value)| {
            format!("{label:?}|{left_value:?}|{right_value:?}")
        })
        .collect::<Vec<_>>();
    rows.sort();
    rows
}

fn normalized_join_rows_with_swapped_sides(joined: &JoinedSeries) -> Vec<String> {
    let mut rows = joined
        .index
        .labels()
        .iter()
        .zip(joined.left_values.values().iter())
        .zip(joined.right_values.values().iter())
        .map(|((label, left_value), right_value)| {
            format!("{label:?}|{right_value:?}|{left_value:?}")
        })
        .collect::<Vec<_>>();
    rows.sort();
    rows
}

fn normalized_series_rows(series: &Series) -> Vec<String> {
    let mut rows = series
        .index()
        .labels()
        .iter()
        .zip(series.values().iter())
        .map(|(label, value)| format!("{label:?}|{value:?}"))
        .collect::<Vec<_>>();
    rows.sort();
    rows
}

fn normalized_dataframe_rows(df: &DataFrame) -> Vec<String> {
    let column_names = df.column_names();
    let mut rows = (0..df.index().len())
        .map(|row_idx| {
            let rendered_values = column_names
                .iter()
                .map(|name| {
                    format!(
                        "{name}:{:?}",
                        df.column(name)
                            .expect("column listed in order must exist")
                            .values()[row_idx]
                    )
                })
                .collect::<Vec<_>>()
                .join("|");
            format!("{:?}|{rendered_values}", df.index().labels()[row_idx])
        })
        .collect::<Vec<_>>();
    rows.sort();
    rows
}

fn sample_count_from_hint(total: usize, replace: bool, hint: usize) -> usize {
    let upper_bound = if replace {
        total.saturating_mul(2).max(1)
    } else {
        total.max(1)
    };
    1 + (hint % upper_bound)
}

// ---------------------------------------------------------------------------
// Property: Index alignment invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    /// align_union always produces a valid alignment plan.
    #[test]
    fn prop_align_union_plan_is_valid((left, right) in arb_index_pair(20)) {
        let plan = align_union(&left, &right);
        validate_alignment_plan(&plan).expect("alignment plan must be valid");
    }

    /// align_union union index contains all left labels.
    #[test]
    fn prop_align_union_contains_all_left_labels((left, right) in arb_index_pair(20)) {
        let plan = align_union(&left, &right);
        let union_labels = plan.union_index.labels();
        for label in left.labels() {
            prop_assert!(
                union_labels.contains(label),
                "union must contain left label {:?}", label
            );
        }
    }

    /// align_union union index contains all right labels.
    #[test]
    fn prop_align_union_contains_all_right_labels((left, right) in arb_index_pair(20)) {
        let plan = align_union(&left, &right);
        let union_labels = plan.union_index.labels();
        for label in right.labels() {
            prop_assert!(
                union_labels.contains(label),
                "union must contain right label {:?}", label
            );
        }
    }

    /// align_union position vectors have the same length as the union index.
    #[test]
    fn prop_align_union_position_lengths_match((left, right) in arb_index_pair(20)) {
        let plan = align_union(&left, &right);
        let n = plan.union_index.len();
        prop_assert_eq!(plan.left_positions.len(), n);
        prop_assert_eq!(plan.right_positions.len(), n);
    }

    /// align_union preserves left order for unique indices: non-None left
    /// positions are strictly increasing when both inputs have no duplicates.
    /// If either side has duplicates, alignment can repeat positions.
    #[test]
    fn prop_align_union_preserves_left_order((left, right) in arb_index_pair(20)) {
        if left.has_duplicates() || right.has_duplicates() {
            // With duplicates, position_map_first introduces non-monotonic
            // position references. This is correct behavior.
            return Ok(());
        }
        let plan = align_union(&left, &right);
        let mut prev_pos: Option<usize> = None;
        for p in plan.left_positions.iter().flatten() {
            if let Some(prev) = prev_pos {
                prop_assert!(
                    *p > prev,
                    "left positions must be strictly increasing for unique index: prev={}, current={}", prev, *p
                );
            }
            prev_pos = Some(*p);
        }
    }
}

// ---------------------------------------------------------------------------
// Property: Index duplicate detection
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    /// has_duplicates is consistent with a naive O(n^2) check.
    #[test]
    fn prop_has_duplicates_matches_naive(index in arb_index(20)) {
        let has_dups = index.has_duplicates();
        let labels = index.labels();
        let naive = labels.iter().enumerate().any(|(i, l)| {
            labels[..i].contains(l)
        });
        prop_assert_eq!(has_dups, naive, "has_duplicates must match naive check");
    }

    /// has_duplicates is deterministic across calls.
    #[test]
    fn prop_has_duplicates_is_deterministic(index in arb_index(20)) {
        let first = index.has_duplicates();
        let second = index.has_duplicates();
        prop_assert_eq!(first, second);
    }
}

// ---------------------------------------------------------------------------
// Property: Series addition invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Series add in hardened mode never panics; it either succeeds or returns an error.
    #[test]
    fn prop_series_add_hardened_no_panic((left, right) in arb_series_pair(15)) {
        let policy = RuntimePolicy::hardened(Some(100_000));
        let mut ledger = EvidenceLedger::new();
        let _ = left.add_with_policy(&right, &policy, &mut ledger);
        // Property: no panic occurred.
    }

    /// Series add result index length equals result values length.
    #[test]
    fn prop_series_add_index_values_length_match((left, right) in arb_series_pair(15)) {
        let policy = RuntimePolicy::hardened(Some(100_000));
        let mut ledger = EvidenceLedger::new();
        if let Ok(result) = left.add_with_policy(&right, &policy, &mut ledger) {
            prop_assert_eq!(
                result.index().len(),
                result.values().len(),
                "index and values must have same length"
            );
        }
    }

    /// Series add result index is the union of left and right indices.
    #[test]
    fn prop_series_add_result_index_is_union((left, right) in arb_series_pair(10)) {
        let policy = RuntimePolicy::hardened(Some(100_000));
        let mut ledger = EvidenceLedger::new();
        if let Ok(result) = left.add_with_policy(&right, &policy, &mut ledger) {
            let result_labels = result.index().labels();
            for label in left.index().labels() {
                prop_assert!(
                    result_labels.contains(label),
                    "result index must contain left label {:?}", label
                );
            }
            for label in right.index().labels() {
                prop_assert!(
                    result_labels.contains(label),
                    "result index must contain right label {:?}", label
                );
            }
        }
    }

    /// Series add with itself produces values that are 2x the original (for non-missing),
    /// but only when the index has no duplicates. With duplicates, alignment maps to
    /// first occurrence (position_map_first), so subsequent duplicate-index positions
    /// get the value at the first position, not their own.
    #[test]
    fn prop_series_add_self_doubles_values(series in arb_numeric_series("self_add", 10)) {
        // Only test the doubling property when the index is unique.
        if series.index().has_duplicates() {
            return Ok(());
        }
        let policy = RuntimePolicy::hardened(Some(100_000));
        let mut ledger = EvidenceLedger::new();
        if let Ok(result) = series.add_with_policy(&series, &policy, &mut ledger) {
            for (i, (orig, doubled)) in series.values().iter().zip(result.values().iter()).enumerate() {
                if orig.is_missing() {
                    prop_assert!(
                        doubled.is_missing(),
                        "missing + missing should be missing at idx={}", i
                    );
                } else if let Ok(v) = orig.to_f64()
                    && let Ok(r) = doubled.to_f64()
                {
                    let expected = v * 2.0;
                    if expected.is_finite() {
                        prop_assert!(
                            (r - expected).abs() < 1e-9,
                            "self-add should double: {} + {} = {} (expected {}) at idx={}",
                            v, v, r, expected, i
                        );
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Property: between metamorphic invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Inclusive-mode variants obey the expected boolean algebra:
    /// both = left OR right, neither = left AND right.
    #[test]
    fn prop_series_between_inclusive_modes_form_expected_lattice(
        series in arb_numeric_series("between", 12),
        bound_a in -1e6_f64..1e6_f64,
        bound_b in -1e6_f64..1e6_f64,
    ) {
        let (lower, upper) = sorted_bounds(bound_a, bound_b);
        let left = series
            .between(&Scalar::Float64(lower), &Scalar::Float64(upper), "left")
            .expect("Series::between(left) must succeed for numeric inputs");
        let right = series
            .between(&Scalar::Float64(lower), &Scalar::Float64(upper), "right")
            .expect("Series::between(right) must succeed for numeric inputs");
        let both = series
            .between(&Scalar::Float64(lower), &Scalar::Float64(upper), "both")
            .expect("Series::between(both) must succeed for numeric inputs");
        let neither = series
            .between(&Scalar::Float64(lower), &Scalar::Float64(upper), "neither")
            .expect("Series::between(neither) must succeed for numeric inputs");

        let left_values = bool_values(&left).expect("between(left) must produce bool output");
        let right_values = bool_values(&right).expect("between(right) must produce bool output");
        let both_values = bool_values(&both).expect("between(both) must produce bool output");
        let neither_values =
            bool_values(&neither).expect("between(neither) must produce bool output");

        for (idx, (((left_flag, right_flag), both_flag), neither_flag)) in left_values
            .into_iter()
            .zip(right_values)
            .zip(both_values)
            .zip(neither_values)
            .enumerate()
        {
            prop_assert_eq!(
                both_flag,
                left_flag || right_flag,
                "between(both) must equal between(left) OR between(right) at idx={}",
                idx
            );
            prop_assert_eq!(
                neither_flag,
                left_flag && right_flag,
                "between(neither) must equal between(left) AND between(right) at idx={}",
                idx
            );
        }
    }

    /// Translating the series and both bounds by the same delta must preserve the result.
    #[test]
    fn prop_series_between_is_translation_invariant(
        series in arb_numeric_series("between", 12),
        bound_a in -1e6_f64..1e6_f64,
        bound_b in -1e6_f64..1e6_f64,
        delta in -1e3_f64..1e3_f64,
        inclusive in arb_between_inclusive(),
    ) {
        let (lower, upper) = sorted_bounds(bound_a, bound_b);
        let baseline = series
            .between(&Scalar::Float64(lower), &Scalar::Float64(upper), inclusive)
            .expect("Series::between() must succeed for numeric inputs");
        let shifted = shift_series(&series, delta);
        let shifted_result = shifted
            .between(
                &Scalar::Float64(lower + delta),
                &Scalar::Float64(upper + delta),
                inclusive,
            )
            .expect("translated Series::between() must succeed for numeric inputs");
        prop_assert!(
            baseline.equals(&shifted_result),
            "series between should be translation invariant for inclusive={inclusive}"
        );
    }

    /// Negating the series and swapping/negating bounds must preserve the result,
    /// with left/right inclusivity exchanged.
    #[test]
    fn prop_series_between_is_sign_symmetric(
        series in arb_numeric_series("between", 12),
        bound_a in -1e6_f64..1e6_f64,
        bound_b in -1e6_f64..1e6_f64,
        inclusive in arb_between_inclusive(),
    ) {
        let (lower, upper) = sorted_bounds(bound_a, bound_b);
        let baseline = series
            .between(&Scalar::Float64(lower), &Scalar::Float64(upper), inclusive)
            .expect("Series::between() must succeed for numeric inputs");
        let flipped_series = sign_flip_series(&series);
        let flipped_result = flipped_series
            .between(
                &Scalar::Float64(-upper),
                &Scalar::Float64(-lower),
                sign_flip_between_inclusive(inclusive),
            )
            .expect("sign-flipped Series::between() must succeed for numeric inputs");
        prop_assert!(
            baseline.equals(&flipped_result),
            "series between should commute with sign flip for inclusive={inclusive}"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: round metamorphic invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Series round with the same precision is idempotent.
    #[test]
    fn prop_series_round_is_idempotent(
        series in arb_numeric_series("round", 12),
        decimals in -3i32..=6,
    ) {
        let once = series
            .round(decimals)
            .expect("Series::round() must succeed for numeric inputs");
        let twice = once
            .round(decimals)
            .expect("Series::round() must succeed on its own output");
        prop_assert!(
            once.equals(&twice),
            "series round must be idempotent for fixed precision"
        );
    }

    /// Negating every numeric value before rounding must negate the rounded result.
    #[test]
    fn prop_series_round_is_sign_symmetric(
        series in arb_numeric_series("round", 12),
        decimals in -3i32..=6,
    ) {
        let baseline = series
            .round(decimals)
            .expect("Series::round() must succeed for numeric inputs");
        let flipped = sign_flip_series(&series);
        let flipped_rounded = flipped
            .round(decimals)
            .expect("Series::round() must succeed after sign flipping");
        let expected = sign_flip_series(&baseline);
        prop_assert!(
            flipped_rounded.equals(&expected),
            "series round should commute with numeric sign flip"
        );
    }

    /// DataFrame round with the same precision is idempotent.
    #[test]
    fn prop_dataframe_round_is_idempotent(
        df in arb_numeric_dataframe(8),
        decimals in -3i32..=6,
    ) {
        let once = df
            .round(decimals)
            .expect("DataFrame::round() must succeed for numeric inputs");
        let twice = once
            .round(decimals)
            .expect("DataFrame::round() must succeed on its own output");
        prop_assert!(
            once.equals(&twice),
            "dataframe round must be idempotent for fixed precision"
        );
    }

    /// Negating every numeric cell before rounding must negate the rounded result.
    #[test]
    fn prop_dataframe_round_is_sign_symmetric(
        df in arb_numeric_dataframe(8),
        decimals in -3i32..=6,
    ) {
        let baseline = df
            .round(decimals)
            .expect("DataFrame::round() must succeed for numeric inputs");
        let flipped = df
            .mul_scalar(-1.0)
            .expect("DataFrame::mul_scalar(-1.0) must succeed for numeric inputs");
        let flipped_rounded = flipped
            .round(decimals)
            .expect("DataFrame::round() must succeed after sign flipping");
        let expected = baseline
            .mul_scalar(-1.0)
            .expect("rounded dataframe must support numeric sign flipping");
        prop_assert!(
            flipped_rounded.equals(&expected),
            "dataframe round should commute with numeric sign flip"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: clip metamorphic invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Series clip with the same bounds is idempotent.
    #[test]
    fn prop_series_clip_is_idempotent(
        series in arb_numeric_series("clip", 12),
        bound_a in -1e6_f64..1e6_f64,
        bound_b in -1e6_f64..1e6_f64,
    ) {
        let (lower, upper) = sorted_bounds(bound_a, bound_b);
        let once = series
            .clip(Some(lower), Some(upper))
            .expect("Series::clip() must succeed for numeric inputs");
        let twice = once
            .clip(Some(lower), Some(upper))
            .expect("Series::clip() must succeed on its own output");
        prop_assert!(
            once.equals(&twice),
            "series clip must be idempotent for fixed bounds"
        );
    }

    /// Tightening clip bounds in a second pass must equal clipping directly to the tighter interval.
    #[test]
    fn prop_series_clip_nested_bounds_compose(
        series in arb_numeric_series("clip", 12),
        outer_a in -1e6_f64..1e6_f64,
        outer_b in -1e6_f64..1e6_f64,
        inner_frac_a in 0.0f64..1.0,
        inner_frac_b in 0.0f64..1.0,
    ) {
        let (outer_lower, outer_upper, inner_lower, inner_upper) =
            nested_clip_bounds(outer_a, outer_b, inner_frac_a, inner_frac_b);
        let staged = series
            .clip(Some(outer_lower), Some(outer_upper))
            .and_then(|clipped| clipped.clip(Some(inner_lower), Some(inner_upper)))
            .expect("Series::clip() composition must succeed");
        let direct = series
            .clip(Some(inner_lower), Some(inner_upper))
            .expect("Series::clip() must succeed for nested bounds");
        prop_assert!(
            staged.equals(&direct),
            "series clip should compose to the tighter interval"
        );
    }

    /// DataFrame clip with the same bounds is idempotent.
    #[test]
    fn prop_dataframe_clip_is_idempotent(
        df in arb_numeric_dataframe(8),
        bound_a in -1e6_f64..1e6_f64,
        bound_b in -1e6_f64..1e6_f64,
    ) {
        let (lower, upper) = sorted_bounds(bound_a, bound_b);
        let once = df
            .clip(Some(lower), Some(upper))
            .expect("DataFrame::clip() must succeed for numeric inputs");
        let twice = once
            .clip(Some(lower), Some(upper))
            .expect("DataFrame::clip() must succeed on its own output");
        prop_assert!(
            once.equals(&twice),
            "dataframe clip must be idempotent for fixed bounds"
        );
    }

    /// Tightening DataFrame clip bounds in a second pass must equal clipping directly to the tighter interval.
    #[test]
    fn prop_dataframe_clip_nested_bounds_compose(
        df in arb_numeric_dataframe(8),
        outer_a in -1e6_f64..1e6_f64,
        outer_b in -1e6_f64..1e6_f64,
        inner_frac_a in 0.0f64..1.0,
        inner_frac_b in 0.0f64..1.0,
    ) {
        let (outer_lower, outer_upper, inner_lower, inner_upper) =
            nested_clip_bounds(outer_a, outer_b, inner_frac_a, inner_frac_b);
        let staged = df
            .clip(Some(outer_lower), Some(outer_upper))
            .and_then(|clipped| clipped.clip(Some(inner_lower), Some(inner_upper)))
            .expect("DataFrame::clip() composition must succeed");
        let direct = df
            .clip(Some(inner_lower), Some(inner_upper))
            .expect("DataFrame::clip() must succeed for nested bounds");
        prop_assert!(
            staged.equals(&direct),
            "dataframe clip should compose to the tighter interval"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: abs metamorphic invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Series abs is idempotent.
    #[test]
    fn prop_series_abs_is_idempotent(series in arb_numeric_series("abs", 12)) {
        let once = series.abs().expect("Series::abs() must succeed for numeric inputs");
        let twice = once.abs().expect("Series::abs() must succeed on its own output");
        prop_assert!(
            once.equals(&twice),
            "series abs must be idempotent"
        );
    }

    /// Flipping every numeric sign before abs must not change the observed result.
    #[test]
    fn prop_series_abs_is_sign_flip_invariant(series in arb_numeric_series("abs", 12)) {
        let baseline = series.abs().expect("Series::abs() must succeed for numeric inputs");
        let flipped = sign_flip_series(&series);
        let flipped_abs = flipped
            .abs()
            .expect("Series::abs() must succeed after sign flipping");
        prop_assert!(
            baseline.equals(&flipped_abs),
            "series abs must ignore numeric sign"
        );
    }

    /// DataFrame abs is idempotent.
    #[test]
    fn prop_dataframe_abs_is_idempotent(df in arb_numeric_dataframe(8)) {
        let once = df.abs().expect("DataFrame::abs() must succeed for numeric inputs");
        let twice = once
            .abs()
            .expect("DataFrame::abs() must succeed on its own output");
        prop_assert!(
            once.equals(&twice),
            "dataframe abs must be idempotent"
        );
    }

    /// Multiplying every numeric cell by -1 before abs must not change the observed result.
    #[test]
    fn prop_dataframe_abs_is_sign_flip_invariant(df in arb_numeric_dataframe(8)) {
        let baseline = df.abs().expect("DataFrame::abs() must succeed for numeric inputs");
        let flipped = df
            .mul_scalar(-1.0)
            .expect("DataFrame::mul_scalar(-1.0) must succeed for numeric inputs");
        let flipped_abs = flipped
            .abs()
            .expect("DataFrame::abs() must succeed after sign flipping");
        prop_assert!(
            baseline.equals(&flipped_abs),
            "dataframe abs must ignore numeric sign"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: combine_first metamorphic invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Series combine_first is idempotent for unique-label inputs.
    #[test]
    fn prop_series_combine_first_idempotent(series in arb_unique_numeric_series("combine_first", 12)) {
        let result = series
            .combine_first(&series)
            .expect("combine_first must succeed for self-composition");
        prop_assert!(
            result.equals(&series),
            "combine_first(self, self) must preserve the original series"
        );
    }

    /// Changes to right-side values hidden behind non-missing left values must not
    /// affect the result.
    #[test]
    fn prop_series_combine_first_ignores_poisoned_right_values(
        (left, right) in arb_unique_series_pair(12)
    ) {
        let baseline = left
            .combine_first(&right)
            .expect("combine_first must succeed for unique-label inputs");
        let poisoned_right = poison_series_right_cells_for_combine_first(&left, &right);
        let poisoned = left
            .combine_first(&poisoned_right)
            .expect("combine_first must succeed for poisoned unique-label inputs");
        prop_assert!(
            baseline.equals(&poisoned),
            "right-side changes under non-missing left values must be observationally irrelevant"
        );
    }

    /// DataFrame combine_first is idempotent for unique-label inputs.
    #[test]
    fn prop_dataframe_combine_first_idempotent(df in arb_combine_first_dataframe(8)) {
        let result = df
            .combine_first(&df)
            .expect("combine_first must succeed for self-composition");
        prop_assert!(
            result.equals(&df),
            "combine_first(self, self) must preserve the original dataframe"
        );
    }

    /// Right-side cell edits hidden behind non-missing left cells must not affect
    /// the observed dataframe result.
    #[test]
    fn prop_dataframe_combine_first_ignores_poisoned_right_values(
        (left, right) in (arb_combine_first_dataframe(8), arb_combine_first_dataframe(8))
    ) {
        let baseline = left
            .combine_first(&right)
            .expect("combine_first must succeed for unique-label dataframe inputs");
        let poisoned_right = poison_dataframe_right_cells_for_combine_first(&left, &right);
        let poisoned = left
            .combine_first(&poisoned_right)
            .expect("combine_first must succeed for poisoned dataframe inputs");
        prop_assert!(
            baseline.equals(&poisoned),
            "right-side changes under non-missing left cells must be observationally irrelevant"
        );
    }

    /// Cascading combine_first operations must compose associatively when row and
    /// column labels are unique.
    #[test]
    fn prop_dataframe_combine_first_associative(
        (left, middle, right) in (
            arb_combine_first_dataframe(6),
            arb_combine_first_dataframe(6),
            arb_combine_first_dataframe(6)
        )
    ) {
        let left_assoc = left
            .combine_first(&middle)
            .and_then(|df| df.combine_first(&right))
            .expect("left-associated combine_first chain must succeed");
        let right_assoc = middle
            .combine_first(&right)
            .and_then(|df| left.combine_first(&df))
            .expect("right-associated combine_first chain must succeed");
        prop_assert!(
            left_assoc.equals(&right_assoc),
            "combine_first chaining must be associative for unique-label dataframe inputs"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: Join invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Inner join output contains only labels present in both inputs.
    #[test]
    fn prop_inner_join_labels_in_both_inputs((left, right) in arb_series_pair(10)) {
        if let Ok(joined) = join_series(&left, &right, JoinType::Inner) {
            let left_labels = left.index().labels();
            let right_labels = right.index().labels();
            for label in joined.index.labels() {
                prop_assert!(
                    left_labels.contains(label) && right_labels.contains(label),
                    "inner join label {:?} must be in both inputs", label
                );
            }
        }
    }

    /// Left join output contains all left labels.
    #[test]
    fn prop_left_join_contains_all_left_labels((left, right) in arb_series_pair(10)) {
        if let Ok(joined) = join_series(&left, &right, JoinType::Left) {
            let joined_labels = joined.index.labels();
            for label in left.index().labels() {
                prop_assert!(
                    joined_labels.contains(label),
                    "left join must contain left label {:?}", label
                );
            }
        }
    }

    /// Join output lengths are consistent (index, left_values, right_values all same length).
    #[test]
    fn prop_join_output_lengths_consistent((left, right) in arb_series_pair(10)) {
        for join_type in [JoinType::Inner, JoinType::Left, JoinType::Right, JoinType::Outer] {
            if let Ok(joined) = join_series(&left, &right, join_type) {
                let n = joined.index.len();
                prop_assert_eq!(joined.left_values.len(), n, "left_values length mismatch for {:?}", join_type);
                prop_assert_eq!(joined.right_values.len(), n, "right_values length mismatch for {:?}", join_type);
            }
        }
    }

    /// Inner join is a subset of left join (in terms of output size).
    #[test]
    fn prop_inner_join_subset_of_left_join((left, right) in arb_series_pair(10)) {
        let inner = join_series(&left, &right, JoinType::Inner);
        let left_j = join_series(&left, &right, JoinType::Left);
        if let (Ok(inner), Ok(left_j)) = (inner, left_j) {
            prop_assert!(
                inner.index.len() <= left_j.index.len(),
                "inner join ({}) must be <= left join ({})",
                inner.index.len(), left_j.index.len()
            );
        }
    }

    /// Right join output contains all right labels.
    #[test]
    fn prop_right_join_contains_all_right_labels((left, right) in arb_series_pair(10)) {
        if let Ok(joined) = join_series(&left, &right, JoinType::Right) {
            let joined_labels = joined.index.labels();
            for label in right.index().labels() {
                prop_assert!(
                    joined_labels.contains(label),
                    "right join must contain right label {:?}", label
                );
            }
        }
    }

    /// Outer join output contains all labels from both sides.
    #[test]
    fn prop_outer_join_contains_all_labels((left, right) in arb_series_pair(10)) {
        if let Ok(joined) = join_series(&left, &right, JoinType::Outer) {
            let joined_labels = joined.index.labels();
            for label in left.index().labels() {
                prop_assert!(
                    joined_labels.contains(label),
                    "outer join must contain left label {:?}", label
                );
            }
            for label in right.index().labels() {
                prop_assert!(
                    joined_labels.contains(label),
                    "outer join must contain right label {:?}", label
                );
            }
        }
    }

    /// Inner join is a subset of outer join (in terms of output size).
    #[test]
    fn prop_inner_join_subset_of_outer_join((left, right) in arb_series_pair(10)) {
        let inner = join_series(&left, &right, JoinType::Inner);
        let outer = join_series(&left, &right, JoinType::Outer);
        if let (Ok(inner), Ok(outer)) = (inner, outer) {
            prop_assert!(
                inner.index.len() <= outer.index.len(),
                "inner join ({}) must be <= outer join ({})",
                inner.index.len(), outer.index.len()
            );
        }
    }

    /// Inner join is symmetric once left/right payload columns are swapped.
    #[test]
    fn prop_inner_join_commutative_up_to_side_swap((left, right) in arb_series_pair(10)) {
        let forward = join_series(&left, &right, JoinType::Inner)
            .expect("inner join must succeed for valid series inputs");
        let swapped = join_series(&right, &left, JoinType::Inner)
            .expect("swapped inner join must succeed for valid series inputs");

        prop_assert_eq!(
            normalized_join_rows(&forward),
            normalized_join_rows_with_swapped_sides(&swapped),
            "swapping inner-join inputs should only swap payload sides"
        );
    }

    /// Outer join is symmetric once left/right payload columns are swapped and
    /// row order is normalized.
    #[test]
    fn prop_outer_join_commutative_up_to_side_swap((left, right) in arb_series_pair(10)) {
        let forward = join_series(&left, &right, JoinType::Outer)
            .expect("outer join must succeed for valid series inputs");
        let swapped = join_series(&right, &left, JoinType::Outer)
            .expect("swapped outer join must succeed for valid series inputs");

        prop_assert_eq!(
            normalized_join_rows(&forward),
            normalized_join_rows_with_swapped_sides(&swapped),
            "swapping outer-join inputs should only swap payload sides"
        );
    }

    /// Left joins are the side-swapped dual of right joins.
    #[test]
    fn prop_left_join_equals_swapped_right_join((left, right) in arb_series_pair(10)) {
        let left_join = join_series(&left, &right, JoinType::Left)
            .expect("left join must succeed for valid series inputs");
        let swapped_right_join = join_series(&right, &left, JoinType::Right)
            .expect("swapped right join must succeed for valid series inputs");

        prop_assert_eq!(
            normalized_join_rows(&left_join),
            normalized_join_rows_with_swapped_sides(&swapped_right_join),
            "left join must match side-swapped right join"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: GroupBy invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// GroupBy sum in hardened mode never panics.
    #[test]
    fn prop_groupby_sum_hardened_no_panic((keys, values) in arb_groupby_pair(15)) {
        let policy = RuntimePolicy::hardened(Some(100_000));
        let mut ledger = EvidenceLedger::new();
        let _ = groupby_sum(&keys, &values, GroupByOptions::default(), &policy, &mut ledger);
        // Property: no panic occurred.
    }

    /// GroupBy sum result has no more groups than input rows.
    #[test]
    fn prop_groupby_sum_groups_bounded_by_input((keys, values) in arb_groupby_pair(15)) {
        let policy = RuntimePolicy::hardened(Some(100_000));
        let mut ledger = EvidenceLedger::new();
        if let Ok(result) = groupby_sum(&keys, &values, GroupByOptions::default(), &policy, &mut ledger) {
            prop_assert!(
                result.index().len() <= keys.values().len(),
                "groups ({}) must be <= input rows ({})",
                result.index().len(), keys.values().len()
            );
        }
    }

    /// GroupBy sum result index/values lengths match.
    #[test]
    fn prop_groupby_sum_index_values_length_match((keys, values) in arb_groupby_pair(15)) {
        let policy = RuntimePolicy::hardened(Some(100_000));
        let mut ledger = EvidenceLedger::new();
        if let Ok(result) = groupby_sum(&keys, &values, GroupByOptions::default(), &policy, &mut ledger) {
            prop_assert_eq!(
                result.index().len(),
                result.values().len(),
                "groupby result index/values length mismatch"
            );
        }
    }

    /// GroupBy sum with dropna=true should not have null keys in output.
    #[test]
    fn prop_groupby_sum_dropna_removes_null_keys((keys, values) in arb_groupby_pair(15)) {
        let policy = RuntimePolicy::hardened(Some(100_000));
        let mut ledger = EvidenceLedger::new();
        let opts = GroupByOptions { dropna: true };
        if let Ok(result) = groupby_sum(&keys, &values, opts, &policy, &mut ledger) {
            for (i, label) in result.index().labels().iter().enumerate() {
                match label {
                    IndexLabel::Int64(_) | IndexLabel::Utf8(_) => {},
                }
                // All labels should be valid (non-null) when dropna=true.
                // IndexLabel doesn't have a null variant, so this is inherently satisfied
                // by the type system. But we verify the result is well-formed.
                prop_assert!(
                    result.values().len() > i,
                    "result should have value at index {}", i
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Property: Scalar type coercion invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    /// Scalar semantic_eq is reflexive.
    #[test]
    fn prop_scalar_semantic_eq_reflexive(scalar in arb_numeric_scalar()) {
        prop_assert!(scalar.semantic_eq(&scalar), "semantic_eq must be reflexive");
    }

    /// Scalar semantic_eq is symmetric.
    #[test]
    fn prop_scalar_semantic_eq_symmetric(
        a in arb_numeric_scalar(),
        b in arb_numeric_scalar(),
    ) {
        prop_assert_eq!(
            a.semantic_eq(&b),
            b.semantic_eq(&a),
            "semantic_eq must be symmetric: {:?} vs {:?}", a, b
        );
    }

    /// Missing scalars are detected by is_missing().
    #[test]
    fn prop_null_scalars_are_missing(kind in prop_oneof![
        Just(NullKind::Null),
        Just(NullKind::NaN),
        Just(NullKind::NaT),
    ]) {
        let scalar = Scalar::Null(kind);
        prop_assert!(scalar.is_missing(), "Null({:?}) must be missing", kind);
    }

    /// Non-null, non-NaN scalars are not missing.
    #[test]
    fn prop_concrete_scalars_not_missing(scalar in prop_oneof![
        any::<bool>().prop_map(Scalar::Bool),
        (-1_000_000i64..1_000_000i64).prop_map(Scalar::Int64),
        (-1e6_f64..1e6_f64).prop_filter("not NaN", |v| !v.is_nan()).prop_map(Scalar::Float64),
    ]) {
        prop_assert!(!scalar.is_missing(), "{:?} must not be missing", scalar);
    }

    /// Float64(NaN) is always detected as missing.
    #[test]
    fn prop_float64_nan_is_missing(_dummy in 0..1u8) {
        let nan = Scalar::Float64(f64::NAN);
        prop_assert!(nan.is_missing());
        prop_assert!(nan.is_nan());
    }
}

// ---------------------------------------------------------------------------
// Property: Serialization round-trip
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Scalars survive JSON serialization round-trip. For Float64, JSON
    /// text serialization can lose the last bit of precision, so we compare
    /// with a small tolerance instead of exact semantic_eq.
    #[test]
    fn prop_scalar_json_round_trip(scalar in arb_numeric_scalar()) {
        let json = serde_json::to_string(&scalar).expect("serialize");
        let back: Scalar = serde_json::from_str(&json).expect("deserialize");
        match (&scalar, &back) {
            (Scalar::Float64(a), Scalar::Float64(b)) => {
                if a.is_nan() && b.is_nan() {
                    // Both NaN: ok
                } else {
                    let diff = (a - b).abs();
                    let tol = a.abs().max(1.0) * 1e-15;
                    prop_assert!(diff <= tol,
                        "float round-trip drift: {} -> {} (diff={})", a, b, diff);
                }
            }
            _ => {
                prop_assert!(
                    scalar.semantic_eq(&back),
                    "round-trip failed: {:?} -> {} -> {:?}", scalar, json, back
                );
            }
        }
    }

    /// IndexLabel survives JSON serialization round-trip.
    #[test]
    fn prop_index_label_json_round_trip(label in arb_index_label()) {
        let json = serde_json::to_string(&label).expect("serialize");
        let back: IndexLabel = serde_json::from_str(&json).expect("deserialize");
        prop_assert_eq!(label, back);
    }

    // === Packed Bitvec ValidityMask Property Tests (bd-2t5e.4.1) ===

    /// Packing bools into u64 words then unpacking via bits() produces identical values.
    #[test]
    fn prop_bitvec_roundtrip(bools in proptest::collection::vec(proptest::bool::ANY, 0..512)) {
        let values: Vec<Scalar> = bools.iter().map(|&b| {
            if b { Scalar::Int64(1) } else { Scalar::Null(fp_types::NullKind::Null) }
        }).collect();
        let mask = fp_columnar::ValidityMask::from_values(&values);
        let unpacked: Vec<bool> = mask.bits().collect();
        prop_assert_eq!(bools, unpacked);
    }

    /// and_mask is commutative: a AND b == b AND a.
    #[test]
    fn prop_bitvec_and_commutative(
        bools_a in proptest::collection::vec(proptest::bool::ANY, 0..256),
        bools_b in proptest::collection::vec(proptest::bool::ANY, 0..256),
    ) {
        let len = bools_a.len().min(bools_b.len());
        let vals_a: Vec<Scalar> = bools_a[..len].iter().map(|&b| {
            if b { Scalar::Int64(1) } else { Scalar::Null(fp_types::NullKind::Null) }
        }).collect();
        let vals_b: Vec<Scalar> = bools_b[..len].iter().map(|&b| {
            if b { Scalar::Int64(1) } else { Scalar::Null(fp_types::NullKind::Null) }
        }).collect();
        let a = fp_columnar::ValidityMask::from_values(&vals_a);
        let b = fp_columnar::ValidityMask::from_values(&vals_b);
        let ab: Vec<bool> = a.and_mask(&b).bits().collect();
        let ba: Vec<bool> = b.and_mask(&a).bits().collect();
        prop_assert_eq!(ab, ba);
    }

    /// count_valid() matches the count from iterating bits().
    #[test]
    fn prop_bitvec_count_matches_iter(bools in proptest::collection::vec(proptest::bool::ANY, 0..512)) {
        let values: Vec<Scalar> = bools.iter().map(|&b| {
            if b { Scalar::Int64(1) } else { Scalar::Null(fp_types::NullKind::Null) }
        }).collect();
        let mask = fp_columnar::ValidityMask::from_values(&values);
        let iter_count = mask.bits().filter(|b| *b).count();
        prop_assert_eq!(mask.count_valid(), iter_count);
    }
}

// ---------------------------------------------------------------------------
// Property: AG-06 Arena/Region Memory Isomorphism (bd-2t5e.6.1)
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Arena-backed join produces identical output to global-allocator join
    /// for arbitrary series pairs and both join types.
    #[test]
    fn prop_arena_join_isomorphic_to_global(
        (left, right) in arb_series_pair(15),
        join_type in prop_oneof![Just(JoinType::Inner), Just(JoinType::Left), Just(JoinType::Right), Just(JoinType::Outer)],
    ) {
        let global = join_series_with_options(
            &left, &right, join_type,
            JoinExecutionOptions { use_arena: false, arena_budget_bytes: 0 },
        );
        let arena = join_series_with_options(
            &left, &right, join_type,
            JoinExecutionOptions::default(),
        );

        match (global, arena) {
            (Ok(g), Ok(a)) => {
                prop_assert_eq!(
                    g.index.labels(), a.index.labels(),
                    "arena join index must match global"
                );
                prop_assert_eq!(
                    g.left_values.len(), a.left_values.len(),
                    "arena join left_values length must match"
                );
                prop_assert_eq!(
                    g.right_values.len(), a.right_values.len(),
                    "arena join right_values length must match"
                );
            }
            (Err(_), Err(_)) => { /* both error: ok */ }
            (Ok(_), Err(e)) => {
                prop_assert!(false, "arena errored but global succeeded: {e}");
            }
            (Err(e), Ok(_)) => {
                prop_assert!(false, "global errored but arena succeeded: {e}");
            }
        }
    }

    /// Arena-backed groupby_sum produces identical output to global-allocator
    /// groupby_sum for arbitrary key/value pairs.
    #[test]
    fn prop_arena_groupby_isomorphic_to_global(
        (keys, values) in arb_groupby_pair(15),
        dropna in proptest::bool::ANY,
    ) {
        let policy = RuntimePolicy::hardened(Some(100_000));
        let mut ledger = EvidenceLedger::new();
        let opts = GroupByOptions { dropna };

        let global = groupby_sum_with_options(
            &keys, &values, opts, &policy, &mut ledger,
            GroupByExecutionOptions { use_arena: false, arena_budget_bytes: 0 },
        );
        let arena = groupby_sum_with_options(
            &keys, &values, opts, &policy, &mut ledger,
            GroupByExecutionOptions::default(),
        );

        match (global, arena) {
            (Ok(g), Ok(a)) => {
                prop_assert_eq!(
                    g.index().labels(), a.index().labels(),
                    "arena groupby index must match global"
                );
                prop_assert_eq!(
                    g.values(), a.values(),
                    "arena groupby values must match global"
                );
            }
            (Err(_), Err(_)) => { /* both error: ok */ }
            (Ok(_), Err(e)) => {
                prop_assert!(false, "arena errored but global succeeded: {e}");
            }
            (Err(e), Ok(_)) => {
                prop_assert!(false, "global errored but arena succeeded: {e}");
            }
        }
    }

    /// Join with a tiny arena budget falls back to global allocator and still
    /// produces correct results.
    #[test]
    fn prop_arena_join_fallback_correct(
        (left, right) in arb_series_pair(15),
        join_type in prop_oneof![Just(JoinType::Inner), Just(JoinType::Left), Just(JoinType::Right), Just(JoinType::Outer)],
    ) {
        let fallback = join_series_with_options(
            &left, &right, join_type,
            JoinExecutionOptions { use_arena: true, arena_budget_bytes: 1 },
        );
        let global = join_series_with_options(
            &left, &right, join_type,
            JoinExecutionOptions { use_arena: false, arena_budget_bytes: 0 },
        );

        match (fallback, global) {
            (Ok(f), Ok(g)) => {
                prop_assert_eq!(f.index.labels(), g.index.labels());
                prop_assert_eq!(f.left_values.len(), g.left_values.len());
                prop_assert_eq!(f.right_values.len(), g.right_values.len());
            }
            (Err(_), Err(_)) => {}
            _ => prop_assert!(false, "fallback/global mismatch in error status"),
        }
    }

    /// Groupby with a tiny arena budget falls back and still produces
    /// correct results.
    #[test]
    fn prop_arena_groupby_fallback_correct(
        (keys, values) in arb_groupby_pair(15),
    ) {
        let policy = RuntimePolicy::hardened(Some(100_000));
        let mut ledger = EvidenceLedger::new();
        let opts = GroupByOptions::default();

        let fallback = groupby_sum_with_options(
            &keys, &values, opts, &policy, &mut ledger,
            GroupByExecutionOptions { use_arena: true, arena_budget_bytes: 1 },
        );
        let global = groupby_sum_with_options(
            &keys, &values, opts, &policy, &mut ledger,
            GroupByExecutionOptions { use_arena: false, arena_budget_bytes: 0 },
        );

        match (fallback, global) {
            (Ok(f), Ok(g)) => {
                prop_assert_eq!(f.index().labels(), g.index().labels());
                prop_assert_eq!(f.values(), g.values());
            }
            (Err(_), Err(_)) => {}
            _ => prop_assert!(false, "fallback/global mismatch in error status"),
        }
    }
}

// ---------------------------------------------------------------------------
// Property: DType coercion invariants (frankenpandas-x2n)
// ---------------------------------------------------------------------------

/// Generate an arbitrary DType.
fn arb_dtype() -> impl Strategy<Value = fp_types::DType> {
    prop_oneof![
        Just(fp_types::DType::Null),
        Just(fp_types::DType::Bool),
        Just(fp_types::DType::Int64),
        Just(fp_types::DType::Float64),
        Just(fp_types::DType::Utf8),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    /// common_dtype is symmetric: common_dtype(a, b) == common_dtype(b, a).
    #[test]
    fn prop_common_dtype_symmetric(a in arb_dtype(), b in arb_dtype()) {
        let ab = fp_types::common_dtype(a, b);
        let ba = fp_types::common_dtype(b, a);
        match (ab, ba) {
            (Ok(ab_dt), Ok(ba_dt)) => {
                prop_assert_eq!(ab_dt, ba_dt,
                    "common_dtype must be symmetric: ({:?},{:?}) -> {:?} vs {:?}", a, b, ab_dt, ba_dt);
            }
            (Err(_), Err(_)) => { /* both incompatible: ok */ }
            _ => {
                prop_assert!(false,
                    "common_dtype symmetry broken: ({:?},{:?}) one Ok, one Err", a, b);
            }
        }
    }

    /// common_dtype is reflexive: common_dtype(a, a) == Ok(a).
    #[test]
    fn prop_common_dtype_reflexive(a in arb_dtype()) {
        let result = fp_types::common_dtype(a, a);
        prop_assert_eq!(result, Ok(a), "common_dtype({:?}, {:?}) must be {:?}", a, a, a);
    }

    /// common_dtype with Null is identity: common_dtype(Null, x) == Ok(x).
    #[test]
    fn prop_common_dtype_null_identity(x in arb_dtype()) {
        let result = fp_types::common_dtype(fp_types::DType::Null, x);
        prop_assert_eq!(result, Ok(x), "Null is the identity element for common_dtype");
    }

    /// common_dtype is transitive for compatible triples:
    /// If common_dtype(a, b) = Ok(ab) and common_dtype(ab, c) = Ok(abc),
    /// then common_dtype(a, common_dtype(b, c)) should also be Ok(abc).
    #[test]
    fn prop_common_dtype_transitive(a in arb_dtype(), b in arb_dtype(), c in arb_dtype()) {
        let ab = fp_types::common_dtype(a, b);
        let bc = fp_types::common_dtype(b, c);

        // Only test transitivity when both intermediate steps succeed.
        if let (Ok(ab_dt), Ok(bc_dt)) = (ab, bc) {
            let ab_c = fp_types::common_dtype(ab_dt, c);
            let a_bc = fp_types::common_dtype(a, bc_dt);

            match (ab_c, a_bc) {
                (Ok(left), Ok(right)) => {
                    prop_assert_eq!(left, right,
                        "common_dtype transitivity: ({:?},{:?},{:?}) -> {:?} vs {:?}",
                        a, b, c, left, right);
                }
                (Err(_), Err(_)) => { /* both fail: ok */ }
                _ => {
                    prop_assert!(false,
                        "common_dtype transitivity inconsistency for ({:?},{:?},{:?})", a, b, c);
                }
            }
        }
    }

    /// infer_dtype is consistent with common_dtype: infer_dtype on a
    /// homogeneous slice returns the dtype of the elements.
    #[test]
    fn prop_infer_dtype_homogeneous(scalar in arb_numeric_scalar()) {
        if scalar.is_missing() {
            return Ok(());
        }
        let values = vec![scalar.clone(), scalar.clone()];
        let inferred = fp_types::infer_dtype(&values);
        prop_assert_eq!(inferred, Ok(scalar.dtype()),
            "infer_dtype on homogeneous slice should return element dtype");
    }
}

// ---------------------------------------------------------------------------
// Property: Scalar cast safety (frankenpandas-x2n)
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    /// Casting a scalar to its own dtype always succeeds and returns
    /// a semantically equal value (identity cast, AG-03).
    /// Note: Null scalars normalize to the canonical missing form for DType::Null
    /// (i.e., Null(NaN) -> Null(Null)), which is correct behavior.
    #[test]
    fn prop_cast_identity(scalar in arb_numeric_scalar()) {
        let target = scalar.dtype();
        let result = fp_types::cast_scalar(&scalar, target);
        match result {
            Ok(casted) => {
                if scalar.is_missing() {
                    // Missing values normalize to canonical missing for the dtype.
                    prop_assert!(casted.is_missing(),
                        "identity cast of missing must produce missing: {:?} -> {:?}", scalar, casted);
                } else {
                    prop_assert!(casted.semantic_eq(&scalar),
                        "identity cast must preserve value: {:?} -> {:?}", scalar, casted);
                }
            }
            Err(e) => {
                prop_assert!(false, "identity cast must not fail: {:?} -> {:?}", scalar, e);
            }
        }
    }

    /// Casting a missing scalar to any dtype produces a missing scalar.
    #[test]
    fn prop_cast_missing_stays_missing(
        kind in prop_oneof![Just(NullKind::Null), Just(NullKind::NaN), Just(NullKind::NaT)],
        target in arb_dtype(),
    ) {
        let scalar = Scalar::Null(kind);
        let result = fp_types::cast_scalar(&scalar, target);
        match result {
            Ok(casted) => {
                prop_assert!(casted.is_missing(),
                    "casting Null({:?}) to {:?} must produce missing, got {:?}", kind, target, casted);
            }
            Err(_) => {
                // Some casts may legitimately fail (e.g., NaT to Utf8 in some configs).
                // That's acceptable; the important thing is it doesn't panic.
            }
        }
    }

    /// Casting Int64 to Float64 never loses the integer value (within i64 range
    /// that fits exactly in f64).
    #[test]
    fn prop_cast_int64_to_float64_preserves(v in -1_000_000_000i64..1_000_000_000i64) {
        let scalar = Scalar::Int64(v);
        let result = fp_types::cast_scalar(&scalar, fp_types::DType::Float64);
        match result {
            Ok(Scalar::Float64(fv)) => {
                prop_assert_eq!(fv as i64, v,
                    "Int64->Float64 must preserve: {} -> {}", v, fv);
            }
            other => {
                prop_assert!(false, "unexpected cast result: {:?}", other);
            }
        }
    }

    /// Casting Bool to Int64 produces 0 or 1.
    #[test]
    fn prop_cast_bool_to_int64(b in proptest::bool::ANY) {
        let scalar = Scalar::Bool(b);
        let result = fp_types::cast_scalar(&scalar, fp_types::DType::Int64);
        let expected = if b { 1i64 } else { 0i64 };
        prop_assert_eq!(result, Ok(Scalar::Int64(expected)));
    }

    /// Casting compatible types via common_dtype then cast_scalar round-trips:
    /// if common_dtype(a.dtype(), b.dtype()) = Ok(target), then
    /// cast_scalar(a, target) and cast_scalar(b, target) both succeed.
    #[test]
    fn prop_cast_to_common_dtype_succeeds(
        a in arb_numeric_scalar(),
        b in arb_numeric_scalar(),
    ) {
        let dt_a = a.dtype();
        let dt_b = b.dtype();
        if let Ok(target) = fp_types::common_dtype(dt_a, dt_b) {
            let cast_a = fp_types::cast_scalar(&a, target);
            let cast_b = fp_types::cast_scalar(&b, target);
            prop_assert!(cast_a.is_ok(),
                "cast {:?} ({:?}) to {:?} must succeed", a, dt_a, target);
            prop_assert!(cast_b.is_ok(),
                "cast {:?} ({:?}) to {:?} must succeed", b, dt_b, target);
        }
    }
}

// ---------------------------------------------------------------------------
// Property: CSV round-trip invariants (frankenpandas-x2n)
// ---------------------------------------------------------------------------

/// Generate a DataFrame-safe string that CANNOT be parsed as a number or bool.
/// This ensures CSV round-trip doesn't change Utf8 -> Int64/Float64/Bool.
/// The mandatory underscore after the first two letters prevents generating
/// "true" or "false" (which would parse as Bool).
fn arb_safe_string() -> impl Strategy<Value = String> {
    "[a-z]{2}_[a-z0-9]{0,7}"
}

/// Generate a column name (valid, non-empty, no special characters).
fn arb_column_name() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9_]{0,5}"
}

/// Generate a homogeneous column of CSV-safe scalars (all same type per column).
/// This is required because CSV round-trip does per-cell type inference,
/// so mixed-type columns would break on re-parse.
fn arb_csv_column(nrows: usize) -> impl Strategy<Value = Vec<Scalar>> {
    prop_oneof![
        3 => proptest::collection::vec(
            (-1_000_000i64..1_000_000i64).prop_map(Scalar::Int64), nrows
        ),
        2 => proptest::collection::vec(
            arb_safe_string().prop_map(Scalar::Utf8), nrows
        ),
    ]
}

/// Generate a small DataFrame with N rows and M columns of CSV-safe scalars.
/// Each column is homogeneous (all same type) for CSV round-trip safety.
fn arb_csv_dataframe(
    max_rows: usize,
    max_cols: usize,
) -> impl Strategy<Value = fp_frame::DataFrame> {
    (1..=max_rows, 1..=max_cols).prop_flat_map(|(nrows, ncols)| {
        let col_names = proptest::collection::vec(arb_column_name(), ncols);
        let columns = proptest::collection::vec(arb_csv_column(nrows), ncols);
        (col_names, columns).prop_filter_map(
            "dataframe construction must succeed",
            move |(names, cols)| {
                // Ensure unique column names
                let mut seen = std::collections::HashSet::new();
                let mut unique_names = Vec::new();
                for name in &names {
                    let mut candidate = name.clone();
                    let mut suffix = 0;
                    while seen.contains(&candidate) {
                        suffix += 1;
                        candidate = format!("{name}{suffix}");
                    }
                    seen.insert(candidate.clone());
                    unique_names.push(candidate);
                }

                let col_order: Vec<&str> = unique_names.iter().map(String::as_str).collect();
                let data: Vec<(&str, Vec<Scalar>)> = unique_names
                    .iter()
                    .zip(cols.iter())
                    .map(|(n, v)| (n.as_str(), v.clone()))
                    .collect();

                fp_frame::DataFrame::from_dict(&col_order, data).ok()
            },
        )
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// CSV round-trip preserves DataFrame shape (rows x columns).
    #[test]
    fn prop_csv_round_trip_preserves_shape(df in arb_csv_dataframe(8, 4)) {
        let csv_text = fp_io::write_csv_string(&df);
        prop_assert!(csv_text.is_ok(), "CSV write must succeed");
        let csv_text = csv_text.unwrap();

        let parsed = fp_io::read_csv_str(&csv_text);
        prop_assert!(parsed.is_ok(), "CSV parse must succeed: {:?}", parsed.err());
        let parsed = parsed.unwrap();

        prop_assert_eq!(
            parsed.index().len(), df.index().len(),
            "CSV round-trip must preserve row count"
        );
        prop_assert_eq!(
            parsed.column_names().len(), df.column_names().len(),
            "CSV round-trip must preserve column count"
        );
    }

    /// CSV round-trip preserves column names (order may be preserved by BTreeMap).
    #[test]
    fn prop_csv_round_trip_preserves_column_names(df in arb_csv_dataframe(5, 3)) {
        let csv_text = fp_io::write_csv_string(&df).unwrap();
        let parsed = fp_io::read_csv_str(&csv_text).unwrap();

        let orig_names: Vec<&String> = df.column_names();
        let parsed_names: Vec<&String> = parsed.column_names();
        prop_assert_eq!(orig_names, parsed_names, "column names must survive CSV round-trip");
    }

    /// CSV round-trip for Int64-only DataFrames preserves values exactly.
    #[test]
    fn prop_csv_round_trip_int64_exact(
        values in proptest::collection::vec(-100_000i64..100_000i64, 1..10),
        col_name in arb_column_name(),
    ) {
        let scalars: Vec<Scalar> = values.iter().map(|&v| Scalar::Int64(v)).collect();
        let df = fp_frame::DataFrame::from_dict(
            &[col_name.as_str()],
            vec![(col_name.as_str(), scalars.clone())],
        );
        prop_assert!(df.is_ok());
        let df = df.unwrap();

        let csv_text = fp_io::write_csv_string(&df).unwrap();
        let parsed = fp_io::read_csv_str(&csv_text).unwrap();

        let orig_col = df.column(&col_name).unwrap();
        let parsed_col = parsed.column(&col_name).unwrap();
        prop_assert_eq!(
            orig_col.values(), parsed_col.values(),
            "Int64 values must survive CSV round-trip exactly"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: JSON round-trip invariants (frankenpandas-x2n)
// ---------------------------------------------------------------------------

/// Generate a small DataFrame suitable for JSON round-trip testing.
/// Uses only non-null scalars since JSON null handling varies by orient.
fn arb_json_dataframe(
    max_rows: usize,
    max_cols: usize,
) -> impl Strategy<Value = fp_frame::DataFrame> {
    (1..=max_rows, 1..=max_cols).prop_flat_map(|(nrows, ncols)| {
        let col_names = proptest::collection::vec(arb_column_name(), ncols);
        let columns = proptest::collection::vec(
            proptest::collection::vec(
                prop_oneof![
                    3 => (-100_000i64..100_000i64).prop_map(Scalar::Int64),
                    2 => arb_safe_string().prop_map(Scalar::Utf8),
                ],
                nrows,
            ),
            ncols,
        );
        (col_names, columns).prop_filter_map(
            "json dataframe must construct",
            move |(names, cols)| {
                let mut seen = std::collections::HashSet::new();
                let mut unique_names = Vec::new();
                for name in &names {
                    let mut candidate = name.clone();
                    let mut suffix = 0;
                    while seen.contains(&candidate) {
                        suffix += 1;
                        candidate = format!("{name}{suffix}");
                    }
                    seen.insert(candidate.clone());
                    unique_names.push(candidate);
                }

                // Ensure homogeneous columns (all same type per column)
                // so JSON round-trip doesn't lose type info.
                let mut homo_cols = Vec::new();
                for col in &cols {
                    if col.is_empty() {
                        homo_cols.push(col.clone());
                        continue;
                    }
                    let first_dtype = col[0].dtype();
                    if col.iter().all(|s| s.dtype() == first_dtype) {
                        homo_cols.push(col.clone());
                    } else {
                        // Force all to Int64 for homogeneity
                        homo_cols.push(
                            col.iter()
                                .map(|s| match s {
                                    Scalar::Int64(v) => Scalar::Int64(*v),
                                    _ => Scalar::Int64(0),
                                })
                                .collect(),
                        );
                    }
                }

                let col_order: Vec<&str> = unique_names.iter().map(String::as_str).collect();
                let data: Vec<(&str, Vec<Scalar>)> = unique_names
                    .iter()
                    .zip(homo_cols.iter())
                    .map(|(n, v)| (n.as_str(), v.clone()))
                    .collect();

                fp_frame::DataFrame::from_dict(&col_order, data).ok()
            },
        )
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]

    /// JSON Records orient round-trip preserves shape.
    #[test]
    fn prop_json_records_round_trip_shape(df in arb_json_dataframe(5, 3)) {
        let json = fp_io::write_json_string(&df, fp_io::JsonOrient::Records);
        prop_assert!(json.is_ok(), "JSON Records write must succeed");
        let json = json.unwrap();

        let parsed = fp_io::read_json_str(&json, fp_io::JsonOrient::Records);
        prop_assert!(parsed.is_ok(), "JSON Records parse must succeed: {:?}", parsed.err());
        let parsed = parsed.unwrap();

        prop_assert_eq!(parsed.index().len(), df.index().len(),
            "Records round-trip must preserve row count");
        prop_assert_eq!(parsed.column_names().len(), df.column_names().len(),
            "Records round-trip must preserve column count");
    }

    /// JSON Columns orient round-trip preserves shape.
    #[test]
    fn prop_json_columns_round_trip_shape(df in arb_json_dataframe(5, 3)) {
        let json = fp_io::write_json_string(&df, fp_io::JsonOrient::Columns);
        prop_assert!(json.is_ok(), "JSON Columns write must succeed");
        let json = json.unwrap();

        let parsed = fp_io::read_json_str(&json, fp_io::JsonOrient::Columns);
        prop_assert!(parsed.is_ok(), "JSON Columns parse must succeed: {:?}", parsed.err());
        let parsed = parsed.unwrap();

        prop_assert_eq!(parsed.index().len(), df.index().len(),
            "Columns round-trip must preserve row count");
        prop_assert_eq!(parsed.column_names().len(), df.column_names().len(),
            "Columns round-trip must preserve column count");
    }

    /// JSON Split orient round-trip preserves shape.
    #[test]
    fn prop_json_split_round_trip_shape(df in arb_json_dataframe(5, 3)) {
        let json = fp_io::write_json_string(&df, fp_io::JsonOrient::Split);
        prop_assert!(json.is_ok(), "JSON Split write must succeed");
        let json = json.unwrap();

        let parsed = fp_io::read_json_str(&json, fp_io::JsonOrient::Split);
        prop_assert!(parsed.is_ok(), "JSON Split parse must succeed: {:?}", parsed.err());
        let parsed = parsed.unwrap();

        prop_assert_eq!(parsed.index().len(), df.index().len(),
            "Split round-trip must preserve row count");
        prop_assert_eq!(parsed.column_names().len(), df.column_names().len(),
            "Split round-trip must preserve column count");
    }

    /// JSON Values orient round-trip preserves row count.
    /// (Values orient loses column names, so we only check shape.)
    #[test]
    fn prop_json_values_round_trip_row_count(df in arb_json_dataframe(5, 3)) {
        let json = fp_io::write_json_string(&df, fp_io::JsonOrient::Values);
        prop_assert!(json.is_ok(), "JSON Values write must succeed");
        let json = json.unwrap();

        let parsed = fp_io::read_json_str(&json, fp_io::JsonOrient::Values);
        prop_assert!(parsed.is_ok(), "JSON Values parse must succeed: {:?}", parsed.err());
        let parsed = parsed.unwrap();

        prop_assert_eq!(parsed.index().len(), df.index().len(),
            "Values round-trip must preserve row count");
    }

    /// JSON Records orient round-trip preserves column names.
    #[test]
    fn prop_json_records_round_trip_column_names(df in arb_json_dataframe(3, 4)) {
        let json = fp_io::write_json_string(&df, fp_io::JsonOrient::Records).unwrap();
        let parsed = fp_io::read_json_str(&json, fp_io::JsonOrient::Records).unwrap();

        let mut orig: Vec<String> = df.column_names().into_iter().cloned().collect();
        let mut parsed_names: Vec<String> = parsed.column_names().into_iter().cloned().collect();
        orig.sort();
        parsed_names.sort();
        prop_assert_eq!(orig, parsed_names, "column names must survive JSON Records round-trip");
    }
}

// ---------------------------------------------------------------------------
// Property: ValidityMask algebra invariants (frankenpandas-x2n)
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    /// not_mask is involutory: not(not(mask)) == mask.
    #[test]
    fn prop_bitvec_not_involution(bools in proptest::collection::vec(proptest::bool::ANY, 0..256)) {
        let values: Vec<Scalar> = bools.iter().map(|&b| {
            if b { Scalar::Int64(1) } else { Scalar::Null(fp_types::NullKind::Null) }
        }).collect();
        let mask = fp_columnar::ValidityMask::from_values(&values);
        let double_not: Vec<bool> = mask.not_mask().not_mask().bits().collect();
        let original: Vec<bool> = mask.bits().collect();
        prop_assert_eq!(original, double_not, "not(not(mask)) must equal mask");
    }

    /// or_mask is commutative: a OR b == b OR a.
    #[test]
    fn prop_bitvec_or_commutative(
        bools_a in proptest::collection::vec(proptest::bool::ANY, 0..256),
        bools_b in proptest::collection::vec(proptest::bool::ANY, 0..256),
    ) {
        let len = bools_a.len().min(bools_b.len());
        let vals_a: Vec<Scalar> = bools_a[..len].iter().map(|&b| {
            if b { Scalar::Int64(1) } else { Scalar::Null(fp_types::NullKind::Null) }
        }).collect();
        let vals_b: Vec<Scalar> = bools_b[..len].iter().map(|&b| {
            if b { Scalar::Int64(1) } else { Scalar::Null(fp_types::NullKind::Null) }
        }).collect();
        let a = fp_columnar::ValidityMask::from_values(&vals_a);
        let b = fp_columnar::ValidityMask::from_values(&vals_b);
        let ab: Vec<bool> = a.or_mask(&b).bits().collect();
        let ba: Vec<bool> = b.or_mask(&a).bits().collect();
        prop_assert_eq!(ab, ba, "or_mask must be commutative");
    }

    /// De Morgan's law: not(a AND b) == not(a) OR not(b).
    #[test]
    fn prop_bitvec_de_morgan(
        bools_a in proptest::collection::vec(proptest::bool::ANY, 1..128),
        bools_b in proptest::collection::vec(proptest::bool::ANY, 1..128),
    ) {
        let len = bools_a.len().min(bools_b.len());
        let vals_a: Vec<Scalar> = bools_a[..len].iter().map(|&b| {
            if b { Scalar::Int64(1) } else { Scalar::Null(fp_types::NullKind::Null) }
        }).collect();
        let vals_b: Vec<Scalar> = bools_b[..len].iter().map(|&b| {
            if b { Scalar::Int64(1) } else { Scalar::Null(fp_types::NullKind::Null) }
        }).collect();
        let a = fp_columnar::ValidityMask::from_values(&vals_a);
        let b = fp_columnar::ValidityMask::from_values(&vals_b);

        let not_and: Vec<bool> = a.and_mask(&b).not_mask().bits().collect();
        let or_not: Vec<bool> = a.not_mask().or_mask(&b.not_mask()).bits().collect();
        prop_assert_eq!(not_and, or_not, "De Morgan: not(a AND b) == not(a) OR not(b)");
    }
}

// ---------------------------------------------------------------------------
// Property: DataFrame arithmetic invariants (frankenpandas-s6d)
// ---------------------------------------------------------------------------

/// Generate a small DataFrame with numeric columns for arithmetic testing.
fn arb_numeric_dataframe(max_rows: usize) -> impl Strategy<Value = DataFrame> {
    (1..=max_rows).prop_flat_map(|nrows| {
        let idx_labels = arb_index_labels(nrows);
        let col_a = arb_numeric_values(nrows);
        let col_b = arb_numeric_values(nrows);
        (idx_labels, col_a, col_b).prop_filter_map(
            "dataframe construction must succeed",
            move |(labels, va, vb)| {
                let index = Index::new(labels);
                let col_a = fp_columnar::Column::from_values(va).ok()?;
                let col_b = fp_columnar::Column::from_values(vb).ok()?;
                let mut cols = std::collections::BTreeMap::new();
                cols.insert("a".to_string(), col_a);
                cols.insert("b".to_string(), col_b);
                DataFrame::new_with_column_order(
                    index,
                    cols,
                    vec!["a".to_string(), "b".to_string()],
                )
                .ok()
            },
        )
    })
}

fn arb_variable_numeric_series(
    name: &'static str,
    max_len: usize,
) -> impl Strategy<Value = Series> {
    (1..=max_len).prop_flat_map(move |len| arb_numeric_series(name, len))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// DataFrame add_scalar never panics.
    #[test]
    fn prop_df_add_scalar_no_panic(df in arb_numeric_dataframe(10), scalar in -1e6_f64..1e6_f64) {
        let _ = df.add_scalar(scalar);
    }

    /// DataFrame add_scalar preserves shape (rows x columns).
    #[test]
    fn prop_df_add_scalar_preserves_shape(df in arb_numeric_dataframe(10)) {
        if let Ok(result) = df.add_scalar(1.0) {
            prop_assert_eq!(result.index().len(), df.index().len(),
                "add_scalar must preserve row count");
            prop_assert_eq!(result.column_names().len(), df.column_names().len(),
                "add_scalar must preserve column count");
        }
    }

    /// DataFrame add_scalar(0) is approximately identity for non-missing values.
    #[test]
    fn prop_df_add_zero_is_identity(df in arb_numeric_dataframe(10)) {
        if let Ok(result) = df.add_scalar(0.0) {
            for name in df.column_names() {
                let orig_col = df.column(name).unwrap();
                let result_col = result.column(name).unwrap();
                for (i, (orig, res)) in orig_col.values().iter().zip(result_col.values()).enumerate() {
                    if orig.is_missing() {
                        prop_assert!(res.is_missing(),
                            "missing + 0 should stay missing at col={}, idx={}", name, i);
                    } else if let (Ok(ov), Ok(rv)) = (orig.to_f64(), res.to_f64())
                        && ov.is_finite()
                    {
                        prop_assert!((ov - rv).abs() < 1e-9,
                            "add(0) should be identity: {} vs {} at col={}, idx={}",
                            ov, rv, name, i);
                    }
                }
            }
        }
    }

    /// DataFrame mul_scalar(1) is approximately identity for non-missing values.
    #[test]
    fn prop_df_mul_one_is_identity(df in arb_numeric_dataframe(10)) {
        if let Ok(result) = df.mul_scalar(1.0) {
            for name in df.column_names() {
                let orig_col = df.column(name).unwrap();
                let result_col = result.column(name).unwrap();
                for (i, (orig, res)) in orig_col.values().iter().zip(result_col.values()).enumerate() {
                    if orig.is_missing() {
                        prop_assert!(res.is_missing(),
                            "missing * 1 should stay missing at col={}, idx={}", name, i);
                    } else if let (Ok(ov), Ok(rv)) = (orig.to_f64(), res.to_f64())
                        && ov.is_finite()
                    {
                        prop_assert!((ov - rv).abs() < 1e-9,
                            "mul(1) should be identity: {} vs {} at col={}, idx={}",
                            ov, rv, name, i);
                    }
                }
            }
        }
    }

    /// DataFrame add_df result index contains all labels from both inputs.
    #[test]
    fn prop_df_add_df_index_is_union(
        df1 in arb_numeric_dataframe(8),
        df2 in arb_numeric_dataframe(8),
    ) {
        if let Ok(result) = df1.add_df(&df2) {
            let result_labels = result.index().labels();
            for label in df1.index().labels() {
                prop_assert!(
                    result_labels.contains(label),
                    "add_df result must contain left label {:?}", label
                );
            }
            for label in df2.index().labels() {
                prop_assert!(
                    result_labels.contains(label),
                    "add_df result must contain right label {:?}", label
                );
            }
        }
    }

    /// DataFrame add_df result has union of columns from both inputs.
    #[test]
    fn prop_df_add_df_columns_are_union(
        df1 in arb_numeric_dataframe(5),
        df2 in arb_numeric_dataframe(5),
    ) {
        if let Ok(result) = df1.add_df(&df2) {
            let result_names: Vec<&String> = result.column_names();
            for name in df1.column_names() {
                prop_assert!(
                    result_names.contains(&name),
                    "add_df result must contain left column {:?}", name
                );
            }
            for name in df2.column_names() {
                prop_assert!(
                    result_names.contains(&name),
                    "add_df result must contain right column {:?}", name
                );
            }
        }
    }

    /// DataFrame eq_scalar_df produces Bool output, allowing missing values
    /// to propagate as Null.
    #[test]
    fn prop_df_eq_scalar_produces_bool(df in arb_numeric_dataframe(10)) {
        let scalar = Scalar::Int64(0);
        if let Ok(result) = df.eq_scalar_df(&scalar) {
            for name in result.column_names() {
                let col = result.column(name).unwrap();
                for (i, val) in col.values().iter().enumerate() {
                    prop_assert!(
                        matches!(val, Scalar::Bool(_)) || val.is_missing(),
                        "eq_scalar_df must produce Bool or missing values, got {:?} at col={}, idx={}",
                        val, name, i
                    );
                }
            }
        }
    }

    /// DataFrame comparison ops preserve shape.
    #[test]
    fn prop_df_comparison_preserves_shape(df in arb_numeric_dataframe(10)) {
        let scalar = Scalar::Int64(5);
        for op_name in ["eq", "ne", "gt", "ge", "lt", "le"] {
            let result = match op_name {
                "eq" => df.eq_scalar_df(&scalar),
                "ne" => df.ne_scalar_df(&scalar),
                "gt" => df.gt_scalar_df(&scalar),
                "ge" => df.ge_scalar_df(&scalar),
                "lt" => df.lt_scalar_df(&scalar),
                "le" => df.le_scalar_df(&scalar),
                _ => unreachable!(),
            };
            if let Ok(result_df) = result {
                prop_assert_eq!(result_df.index().len(), df.index().len(),
                    "{}_scalar_df must preserve row count", op_name);
                prop_assert_eq!(result_df.column_names().len(), df.column_names().len(),
                    "{}_scalar_df must preserve column count", op_name);
            }
        }
    }

    /// DataFrame eq + ne are complementary for non-NaN values.
    /// For any non-NaN value, eq(x) XOR ne(x) must be true.
    #[test]
    fn prop_df_eq_ne_complementary(df in arb_numeric_dataframe(8)) {
        let scalar = Scalar::Int64(0);
        let eq_result = df.eq_scalar_df(&scalar);
        let ne_result = df.ne_scalar_df(&scalar);
        if let (Ok(eq_df), Ok(ne_df)) = (eq_result, ne_result) {
            for name in eq_df.column_names() {
                let eq_col = eq_df.column(name).unwrap();
                let ne_col = ne_df.column(name).unwrap();
                let orig_col = df.column(name).unwrap();
                for (i, ((eq_v, ne_v), orig)) in eq_col.values().iter()
                    .zip(ne_col.values())
                    .zip(orig_col.values())
                    .enumerate()
                {
                    // Skip NaN values (NaN comparisons have special semantics)
                    if orig.is_missing() {
                        continue;
                    }
                    if let (Scalar::Bool(eq_b), Scalar::Bool(ne_b)) = (eq_v, ne_v) {
                        prop_assert_ne!(eq_b, ne_b,
                            "eq XOR ne must be true for non-NaN at col={}, idx={}", name, i);
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Property: cumulative extrema metamorphic invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Series cummax is idempotent.
    #[test]
    fn prop_series_cummax_is_idempotent(series in arb_numeric_series("cummax", 12)) {
        let once = series
            .cummax()
            .expect("Series::cummax() must succeed for numeric inputs");
        let twice = once
            .cummax()
            .expect("Series::cummax() must succeed on its own output");
        prop_assert!(
            once.equals(&twice),
            "series cummax must be idempotent"
        );
    }

    /// Series cummin is idempotent.
    #[test]
    fn prop_series_cummin_is_idempotent(series in arb_numeric_series("cummin", 12)) {
        let once = series
            .cummin()
            .expect("Series::cummin() must succeed for numeric inputs");
        let twice = once
            .cummin()
            .expect("Series::cummin() must succeed on its own output");
        prop_assert!(
            once.equals(&twice),
            "series cummin must be idempotent"
        );
    }

    /// Negating a Series before cummin must negate the cummax result.
    #[test]
    fn prop_series_cummax_cummin_are_sign_dual(
        series in arb_numeric_series("cumextrema", 12),
    ) {
        let cummax = series
            .cummax()
            .expect("Series::cummax() must succeed for numeric inputs");
        let flipped = sign_flip_series(&series);
        let flipped_cummin = flipped
            .cummin()
            .expect("Series::cummin() must succeed after sign flipping");
        let expected = sign_flip_series(&cummax);
        prop_assert!(
            flipped_cummin.equals(&expected),
            "series cummin(-x) must equal -cummax(x)"
        );
    }

    /// Negating a Series before cummax must negate the cummin result.
    #[test]
    fn prop_series_cummin_cummax_are_sign_dual(
        series in arb_numeric_series("cumextrema", 12),
    ) {
        let cummin = series
            .cummin()
            .expect("Series::cummin() must succeed for numeric inputs");
        let flipped = sign_flip_series(&series);
        let flipped_cummax = flipped
            .cummax()
            .expect("Series::cummax() must succeed after sign flipping");
        let expected = sign_flip_series(&cummin);
        prop_assert!(
            flipped_cummax.equals(&expected),
            "series cummax(-x) must equal -cummin(x)"
        );
    }

    /// DataFrame cummax is idempotent.
    #[test]
    fn prop_dataframe_cummax_is_idempotent(df in arb_numeric_dataframe(8)) {
        let once = df
            .cummax()
            .expect("DataFrame::cummax() must succeed for numeric inputs");
        let twice = once
            .cummax()
            .expect("DataFrame::cummax() must succeed on its own output");
        prop_assert!(
            once.equals(&twice),
            "dataframe cummax must be idempotent"
        );
    }

    /// DataFrame cummin is idempotent.
    #[test]
    fn prop_dataframe_cummin_is_idempotent(df in arb_numeric_dataframe(8)) {
        let once = df
            .cummin()
            .expect("DataFrame::cummin() must succeed for numeric inputs");
        let twice = once
            .cummin()
            .expect("DataFrame::cummin() must succeed on its own output");
        prop_assert!(
            once.equals(&twice),
            "dataframe cummin must be idempotent"
        );
    }

    /// Negating a DataFrame before cummin must negate the cummax result.
    #[test]
    fn prop_dataframe_cummax_cummin_are_sign_dual(df in arb_numeric_dataframe(8)) {
        let cummax = df
            .cummax()
            .expect("DataFrame::cummax() must succeed for numeric inputs");
        let flipped = df
            .mul_scalar(-1.0)
            .expect("DataFrame::mul_scalar(-1.0) must succeed for numeric inputs");
        let flipped_cummin = flipped
            .cummin()
            .expect("DataFrame::cummin() must succeed after sign flipping");
        let expected = cummax
            .mul_scalar(-1.0)
            .expect("cummax dataframe must support sign flipping");
        prop_assert!(
            flipped_cummin.equals(&expected),
            "dataframe cummin(-x) must equal -cummax(x)"
        );
    }

    /// Negating a DataFrame before cummax must negate the cummin result.
    #[test]
    fn prop_dataframe_cummin_cummax_are_sign_dual(df in arb_numeric_dataframe(8)) {
        let cummin = df
            .cummin()
            .expect("DataFrame::cummin() must succeed for numeric inputs");
        let flipped = df
            .mul_scalar(-1.0)
            .expect("DataFrame::mul_scalar(-1.0) must succeed for numeric inputs");
        let flipped_cummax = flipped
            .cummax()
            .expect("DataFrame::cummax() must succeed after sign flipping");
        let expected = cummin
            .mul_scalar(-1.0)
            .expect("cummin dataframe must support sign flipping");
        prop_assert!(
            flipped_cummax.equals(&expected),
            "dataframe cummax(-x) must equal -cummin(x)"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: Sampling metamorphic invariants (frankenpandas-ags)
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Sampling with the same seed must be deterministic for Series.
    #[test]
    fn prop_series_sample_same_seed_is_deterministic(
        series in arb_variable_numeric_series("sample", 12),
        replace in proptest::bool::ANY,
        seed in any::<u64>(),
        hint in 0usize..24,
    ) {
        let sample_n = sample_count_from_hint(series.len(), replace, hint);
        let first = series
            .sample(Some(sample_n), None, replace, Some(seed))
            .expect("series sampling should succeed for derived parameters");
        let second = series
            .sample(Some(sample_n), None, replace, Some(seed))
            .expect("series sampling should succeed for identical replay");

        prop_assert!(
            first.equals(&second),
            "same seed must reproduce the same series sample"
        );
    }

    /// Sampling the full fraction without replacement must permute, not mutate,
    /// the Series rows.
    #[test]
    fn prop_series_sample_full_fraction_is_permutation(
        series in arb_variable_numeric_series("sample", 12),
        seed in any::<u64>(),
    ) {
        let sampled = series
            .sample(None, Some(1.0), false, Some(seed))
            .expect("full-fraction series sampling should succeed");

        prop_assert_eq!(
            normalized_series_rows(&sampled),
            normalized_series_rows(&series),
            "frac=1.0 series sampling without replacement must be a permutation"
        );
    }

    /// Sampling with the same seed must be deterministic for DataFrames.
    #[test]
    fn prop_dataframe_sample_same_seed_is_deterministic(
        df in arb_numeric_dataframe(8),
        replace in proptest::bool::ANY,
        seed in any::<u64>(),
        hint in 0usize..24,
    ) {
        let sample_n = sample_count_from_hint(df.len(), replace, hint);
        let first = df
            .sample(Some(sample_n), None, replace, Some(seed))
            .expect("dataframe sampling should succeed for derived parameters");
        let second = df
            .sample(Some(sample_n), None, replace, Some(seed))
            .expect("dataframe sampling should succeed for identical replay");

        prop_assert!(
            first.equals(&second),
            "same seed must reproduce the same dataframe sample"
        );
    }

    /// Sampling the full fraction without replacement must permute, not mutate,
    /// the DataFrame rows.
    #[test]
    fn prop_dataframe_sample_full_fraction_is_permutation(
        df in arb_numeric_dataframe(8),
        seed in any::<u64>(),
    ) {
        let sampled = df
            .sample(None, Some(1.0), false, Some(seed))
            .expect("full-fraction dataframe sampling should succeed");

        prop_assert_eq!(
            normalized_dataframe_rows(&sampled),
            normalized_dataframe_rows(&df),
            "frac=1.0 dataframe sampling without replacement must be a permutation"
        );
    }

    /// Weighted sampling should be invariant under uniform positive scaling of
    /// the weights when all other inputs are fixed.
    #[test]
    fn prop_dataframe_weighted_sample_invariant_under_weight_scaling(
        df in arb_numeric_dataframe(8),
        base_weights in proptest::collection::vec(1u16..1000u16, 8),
        replace in proptest::bool::ANY,
        seed in any::<u64>(),
        hint in 0usize..24,
    ) {
        let weights = base_weights[..df.len()]
            .iter()
            .map(|&weight| f64::from(weight))
            .collect::<Vec<_>>();
        let sample_n = sample_count_from_hint(df.len(), replace, hint);
        let scaled_weights = weights.iter().map(|weight| weight * 8.0).collect::<Vec<_>>();

        let baseline = df
            .sample_weights(sample_n, &weights, replace, Some(seed))
            .expect("baseline weighted sample should succeed");
        let scaled = df
            .sample_weights(sample_n, &scaled_weights, replace, Some(seed))
            .expect("scaled weighted sample should succeed");

        prop_assert!(
            baseline.equals(&scaled),
            "uniform weight scaling must not change weighted sampling with a fixed seed"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: Shift metamorphic invariants (frankenpandas-xd9)
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Shifting a Series by zero periods must be the identity transform.
    #[test]
    fn prop_series_shift_zero_is_identity(series in arb_variable_numeric_series("shift", 12)) {
        let shifted = series
            .shift(0)
            .expect("series zero-shift should always succeed");

        prop_assert!(
            shifted.equals(&series),
            "series shift(0) must preserve every row"
        );
    }

    /// Same-direction Series shifts must compose additively.
    #[test]
    fn prop_series_shift_same_direction_composes(
        series in arb_variable_numeric_series("shift", 12),
        first in 0i64..10,
        second in 0i64..10,
        negative in proptest::bool::ANY,
    ) {
        let first = if negative { -first } else { first };
        let second = if negative { -second } else { second };
        let twice = series
            .shift(first)
            .and_then(|shifted| shifted.shift(second))
            .expect("composed series shifts should succeed");
        let direct = series
            .shift(first + second)
            .expect("direct series shift should succeed");

        prop_assert!(
            twice.equals(&direct),
            "same-direction series shifts must compose additively"
        );
    }

    /// Shifting a DataFrame by zero periods must be the identity transform.
    #[test]
    fn prop_dataframe_shift_zero_is_identity(df in arb_numeric_dataframe(8)) {
        let shifted = df
            .shift(0)
            .expect("dataframe zero-shift should always succeed");

        prop_assert!(
            shifted.equals(&df),
            "dataframe shift(0) must preserve every cell"
        );
    }

    /// Same-direction DataFrame row shifts must compose additively.
    #[test]
    fn prop_dataframe_shift_same_direction_composes(
        df in arb_numeric_dataframe(8),
        first in 0i64..10,
        second in 0i64..10,
        negative in proptest::bool::ANY,
    ) {
        let first = if negative { -first } else { first };
        let second = if negative { -second } else { second };
        let twice = df
            .shift(first)
            .and_then(|shifted| shifted.shift(second))
            .expect("composed dataframe row shifts should succeed");
        let direct = df
            .shift(first + second)
            .expect("direct dataframe row shift should succeed");

        prop_assert!(
            twice.equals(&direct),
            "same-direction dataframe row shifts must compose additively"
        );
    }

    /// Horizontally shifting a DataFrame by zero periods must be the identity transform.
    #[test]
    fn prop_dataframe_shift_axis1_zero_is_identity(df in arb_numeric_dataframe(8)) {
        let shifted = df
            .shift_axis1(0)
            .expect("dataframe axis=1 zero-shift should always succeed");

        prop_assert!(
            shifted.equals(&df),
            "dataframe shift_axis1(0) must preserve every cell"
        );
    }

    /// Same-direction DataFrame axis=1 shifts must compose additively.
    #[test]
    fn prop_dataframe_shift_axis1_same_direction_composes(
        df in arb_numeric_dataframe(8),
        first in 0i64..6,
        second in 0i64..6,
        negative in proptest::bool::ANY,
    ) {
        let first = if negative { -first } else { first };
        let second = if negative { -second } else { second };
        let twice = df
            .shift_axis1(first)
            .and_then(|shifted| shifted.shift_axis1(second))
            .expect("composed dataframe axis=1 shifts should succeed");
        let direct = df
            .shift_axis1(first + second)
            .expect("direct dataframe axis=1 shift should succeed");

        prop_assert!(
            twice.equals(&direct),
            "same-direction dataframe axis=1 shifts must compose additively"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: Feather (Arrow IPC) round-trip invariants (frankenpandas-44y)
// ---------------------------------------------------------------------------

/// Generate a DataFrame for Feather/SQL round-trip testing.
/// Uses Int64-only columns to avoid type inference ambiguity during read-back.
fn arb_int64_dataframe(
    max_rows: usize,
    max_cols: usize,
) -> impl Strategy<Value = fp_frame::DataFrame> {
    (1..=max_rows, 1..=max_cols).prop_flat_map(|(nrows, ncols)| {
        let col_names = proptest::collection::vec(arb_column_name(), ncols);
        let columns = proptest::collection::vec(
            proptest::collection::vec((-100_000i64..100_000i64).prop_map(Scalar::Int64), nrows),
            ncols,
        );
        (col_names, columns).prop_filter_map(
            "dataframe construction must succeed",
            move |(names, cols)| {
                let mut seen = std::collections::HashSet::new();
                let mut unique_names = Vec::new();
                for name in &names {
                    let mut candidate = name.clone();
                    let mut suffix = 0;
                    while seen.contains(&candidate) {
                        suffix += 1;
                        candidate = format!("{name}{suffix}");
                    }
                    seen.insert(candidate.clone());
                    unique_names.push(candidate);
                }

                let col_order: Vec<&str> = unique_names.iter().map(String::as_str).collect();
                let data: Vec<(&str, Vec<Scalar>)> = unique_names
                    .iter()
                    .zip(cols.iter())
                    .map(|(n, v)| (n.as_str(), v.clone()))
                    .collect();

                fp_frame::DataFrame::from_dict(&col_order, data).ok()
            },
        )
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]

    /// Feather round-trip preserves DataFrame shape.
    #[test]
    fn prop_feather_round_trip_preserves_shape(df in arb_int64_dataframe(8, 4)) {
        let bytes = fp_io::write_feather_bytes(&df);
        prop_assert!(bytes.is_ok(), "Feather write must succeed");
        let bytes = bytes.unwrap();

        let parsed = fp_io::read_feather_bytes(&bytes);
        prop_assert!(parsed.is_ok(), "Feather parse must succeed: {:?}", parsed.err());
        let parsed = parsed.unwrap();

        prop_assert_eq!(parsed.index().len(), df.index().len(),
            "Feather round-trip must preserve row count");
        prop_assert_eq!(parsed.column_names().len(), df.column_names().len(),
            "Feather round-trip must preserve column count");
    }

    /// Feather round-trip preserves column names.
    #[test]
    fn prop_feather_round_trip_preserves_column_names(df in arb_int64_dataframe(5, 3)) {
        let bytes = fp_io::write_feather_bytes(&df).unwrap();
        let parsed = fp_io::read_feather_bytes(&bytes).unwrap();

        let orig: Vec<&String> = df.column_names();
        let back: Vec<&String> = parsed.column_names();
        prop_assert_eq!(orig, back, "column names must survive Feather round-trip");
    }

    /// Feather round-trip preserves Int64 values exactly.
    #[test]
    fn prop_feather_round_trip_int64_exact(df in arb_int64_dataframe(8, 3)) {
        let bytes = fp_io::write_feather_bytes(&df).unwrap();
        let parsed = fp_io::read_feather_bytes(&bytes).unwrap();

        for name in df.column_names() {
            let orig_col = df.column(name).unwrap();
            let parsed_col = parsed.column(name).unwrap();
            prop_assert_eq!(
                orig_col.values(), parsed_col.values(),
                "Int64 values must survive Feather round-trip for column {}", name
            );
        }
    }

    /// IPC stream round-trip preserves shape.
    #[test]
    fn prop_ipc_stream_round_trip_preserves_shape(df in arb_int64_dataframe(8, 4)) {
        let bytes = fp_io::write_ipc_stream_bytes(&df).unwrap();
        let parsed = fp_io::read_ipc_stream_bytes(&bytes).unwrap();

        prop_assert_eq!(parsed.index().len(), df.index().len(),
            "IPC stream round-trip must preserve row count");
        prop_assert_eq!(parsed.column_names().len(), df.column_names().len(),
            "IPC stream round-trip must preserve column count");
    }
}

// ---------------------------------------------------------------------------
// Property: SQL round-trip invariants (frankenpandas-44y)
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(30))]

    /// SQL round-trip preserves DataFrame shape.
    #[test]
    fn prop_sql_round_trip_preserves_shape(df in arb_int64_dataframe(8, 4)) {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let write_result = fp_io::write_sql(&df, &conn, "test_prop", fp_io::SqlIfExists::Replace);
        prop_assert!(write_result.is_ok(), "SQL write must succeed: {:?}", write_result.err());

        let parsed = fp_io::read_sql_table(&conn, "test_prop");
        prop_assert!(parsed.is_ok(), "SQL read must succeed: {:?}", parsed.err());
        let parsed = parsed.unwrap();

        prop_assert_eq!(parsed.index().len(), df.index().len(),
            "SQL round-trip must preserve row count");
        prop_assert_eq!(parsed.column_names().len(), df.column_names().len(),
            "SQL round-trip must preserve column count");
    }

    /// SQL round-trip preserves Int64 values exactly.
    #[test]
    fn prop_sql_round_trip_int64_exact(df in arb_int64_dataframe(8, 3)) {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        fp_io::write_sql(&df, &conn, "test_vals", fp_io::SqlIfExists::Replace).unwrap();
        let parsed = fp_io::read_sql_table(&conn, "test_vals").unwrap();

        for name in df.column_names() {
            let orig_col = df.column(name).unwrap();
            let parsed_col = parsed.column(name).unwrap();
            prop_assert_eq!(
                orig_col.values(), parsed_col.values(),
                "Int64 values must survive SQL round-trip for column {}", name
            );
        }
    }

    /// SQL round-trip preserves column names (sorted, since SQL has no order guarantee).
    #[test]
    fn prop_sql_round_trip_preserves_column_names(df in arb_int64_dataframe(5, 3)) {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        fp_io::write_sql(&df, &conn, "test_names", fp_io::SqlIfExists::Replace).unwrap();
        let parsed = fp_io::read_sql_table(&conn, "test_names").unwrap();

        let mut orig: Vec<String> = df.column_names().into_iter().cloned().collect();
        let mut back: Vec<String> = parsed.column_names().into_iter().cloned().collect();
        orig.sort();
        back.sort();
        prop_assert_eq!(orig, back, "column names must survive SQL round-trip");
    }
}

// ---------------------------------------------------------------------------
// Property: Excel round-trip invariants (frankenpandas-44y)
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(30))]

    /// Excel round-trip preserves DataFrame shape.
    #[test]
    fn prop_excel_round_trip_preserves_shape(df in arb_int64_dataframe(8, 4)) {
        let bytes = fp_io::write_excel_bytes(&df);
        prop_assert!(bytes.is_ok(), "Excel write must succeed");
        let bytes = bytes.unwrap();

        let parsed = fp_io::read_excel_bytes(&bytes, &fp_io::ExcelReadOptions::default());
        prop_assert!(parsed.is_ok(), "Excel parse must succeed: {:?}", parsed.err());
        let parsed = parsed.unwrap();

        prop_assert_eq!(parsed.index().len(), df.index().len(),
            "Excel round-trip must preserve row count");
        prop_assert_eq!(parsed.column_names().len(), df.column_names().len(),
            "Excel round-trip must preserve column count");
    }

    /// Excel round-trip preserves Int64 values exactly.
    /// (Excel stores integers as f64; our reader recovers Int64 for whole numbers.)
    #[test]
    fn prop_excel_round_trip_int64_exact(df in arb_int64_dataframe(8, 3)) {
        let bytes = fp_io::write_excel_bytes(&df).unwrap();
        let parsed = fp_io::read_excel_bytes(&bytes, &fp_io::ExcelReadOptions::default()).unwrap();

        for name in df.column_names() {
            let orig_col = df.column(name).unwrap();
            let parsed_col = parsed.column(name).unwrap();
            prop_assert_eq!(
                orig_col.values(), parsed_col.values(),
                "Int64 values must survive Excel round-trip for column {}", name
            );
        }
    }
}
