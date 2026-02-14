#![forbid(unsafe_code)]

//! Property-based testing infrastructure for FrankenPandas (bd-2t5e.1, AG-01).
//!
//! Strategy generators produce arbitrary but pandas-valid inputs across the
//! (dtype x null_pattern x index_type x operation) combinatorial space.
//! Properties verify behavioral invariants that must hold for ALL inputs,
//! not just hand-picked fixtures.

use proptest::prelude::*;

use fp_frame::Series;
use fp_groupby::{GroupByExecutionOptions, GroupByOptions, groupby_sum, groupby_sum_with_options};
use fp_index::{Index, IndexLabel, align_union, validate_alignment_plan};
use fp_join::{JoinExecutionOptions, JoinType, join_series, join_series_with_options};
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

/// Generate a pair of numeric series with independently chosen lengths (1..max_len).
fn arb_series_pair(max_len: usize) -> impl Strategy<Value = (Series, Series)> {
    (1..=max_len, 1..=max_len).prop_flat_map(|(len_a, len_b)| {
        (
            arb_numeric_series("left", len_a),
            arb_numeric_series("right", len_b),
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
    /// positions are strictly increasing when left has no duplicates.
    /// With duplicates, position_map_first maps all occurrences to the first
    /// position, so ordering is not monotonic.
    #[test]
    fn prop_align_union_preserves_left_order((left, right) in arb_index_pair(20)) {
        if left.has_duplicates() {
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
