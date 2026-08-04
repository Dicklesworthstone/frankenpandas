#![forbid(unsafe_code)]
//! Metamorphic value-correctness relation for groupby_sum.
//!
//! The existing groupby MRs in proptest_properties.rs assert structural
//! invariants (no panic, group count bounded by input rows, index/values length
//! match) but NOT the aggregated VALUES. This file adds a direct value check:
//! groupby_sum's per-group total equals a naive accumulation in row order.
//! It guards the values that the typed-columnar gather (fi6zx) and the FxHash
//! group-key map (a14692e0) feed into.

use std::collections::BTreeMap;

use fp_frame::Series;
use fp_groupby::{GroupByOptions, groupby_sum};
use fp_index::IndexLabel;
use fp_runtime::{EvidenceLedger, RuntimePolicy};
use fp_types::Scalar;
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    /// groupby_sum(keys, values) per-group total == naive per-key accumulation
    /// over the same row order. The engine folds `entry += v as f64` in row
    /// order, so a row-ordered naive f64 fold is bit-identical; comparison is by
    /// key (group output order is irrelevant here). Small integer values stay
    /// exactly representable in f64, so == is exact.
    #[test]
    fn prop_groupby_sum_matches_naive_per_group(
        rows in prop::collection::vec((0i64..8, -50i64..50), 1..40),
    ) {
        let n = rows.len();
        let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
        let keys = match Series::from_values(
            "k",
            labels.clone(),
            rows.iter().map(|(k, _)| Scalar::Int64(*k)).collect(),
        ) {
            Ok(s) => s,
            Err(_) => return Ok(()),
        };
        let values = match Series::from_values(
            "v",
            labels,
            rows.iter().map(|(_, v)| Scalar::Int64(*v)).collect(),
        ) {
            Ok(s) => s,
            Err(_) => return Ok(()),
        };

        let out = match groupby_sum(
            &keys,
            &values,
            GroupByOptions::default(),
            &RuntimePolicy::strict(),
            &mut EvidenceLedger::new(),
        ) {
            Ok(s) => s,
            Err(_) => return Ok(()),
        };

        // Naive: accumulate value by key in row order (f64, matching the engine).
        let mut naive: BTreeMap<i64, f64> = BTreeMap::new();
        for (k, v) in &rows {
            *naive.entry(*k).or_insert(0.0) += *v as f64;
        }

        let out_labels = out.index().labels().to_vec();
        let out_vals = out.values();
        prop_assert_eq!(out_labels.len(), naive.len(), "group count mismatch");
        prop_assert_eq!(out_labels.len(), out_vals.len(), "label/value length mismatch");
        for (label, val) in out_labels.iter().zip(out_vals.iter()) {
            let k = match label {
                IndexLabel::Int64(i) => *i,
                _ => return Ok(()),
            };
            let got = match val {
                Scalar::Float64(f) => *f,
                Scalar::Int64(i) => *i as f64,
                _ => return Ok(()),
            };
            let expected = *naive.get(&k).expect("group key must exist in naive map");
            prop_assert_eq!(
                got, expected,
                "groupby_sum value mismatch for key {}: got {} expected {}",
                k, got, expected
            );
        }
    }
}
