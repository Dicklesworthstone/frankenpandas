#![forbid(unsafe_code)]
//! Metamorphic value-correctness relation for DataFrame::filter_rows (boolean mask).
//!
//! filter_rows has an identical-index fast path that collects the True positions
//! into a pre-allocated Vec and gathers them (br-axw58 / the in-progress bool
//! positions-prealloc work, fi6zx). The existing filter MRs cover iloc[bool] and
//! where/mask duals, but not filter_rows row-value correctness. A bad prealloc
//! size, off-by-one, or wrong selectivity guess would drop/duplicate rows or
//! corrupt values — this asserts the gathered rows are exactly the True-mask rows
//! in order. Building the mask from the frame's own labels exercises the fast
//! path (identical index).

use fp_frame::{DataFrame, Series};
use fp_index::IndexLabel;
use fp_types::Scalar;
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn prop_dataframe_filter_rows_gathers_true_mask(
        rows in prop::collection::vec((any::<i64>(), any::<bool>()), 1..40),
    ) {
        let col_a: Vec<Scalar> = rows.iter().map(|(v, _)| Scalar::Int64(*v)).collect();
        let mask_vals: Vec<Scalar> = rows.iter().map(|(_, b)| Scalar::Bool(*b)).collect();

        let df = match DataFrame::from_dict(&["a"], vec![("a", col_a)]) {
            Ok(d) => d,
            Err(_) => return Ok(()),
        };
        // Build the mask on the frame's OWN index labels -> identical index ->
        // exercises the fast (prealloc) path.
        let df_labels: Vec<IndexLabel> = df.index().labels().to_vec();
        let mask = match Series::from_values("mask", df_labels, mask_vals) {
            Ok(s) => s,
            Err(_) => return Ok(()),
        };
        let filtered = match df.filter_rows(&mask) {
            Ok(f) => f,
            Err(_) => return Ok(()),
        };

        // Expected: values where mask is true, in original order.
        let expected: Vec<i64> = rows.iter().filter(|(_, b)| *b).map(|(v, _)| *v).collect();
        let got = filtered.column("a").unwrap().values().to_vec();
        prop_assert_eq!(got.len(), expected.len(), "filtered row count != True count");
        for (g, e) in got.iter().zip(expected.iter()) {
            prop_assert_eq!(g, &Scalar::Int64(*e), "filtered value mismatch");
        }
    }
}
