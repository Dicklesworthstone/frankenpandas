//! No-mock conformance guard for DataFrame.groupby(Datetime64/Timedelta64 key).
//! Differential against the SAME grouping done with an Int64 key over the same
//! ns (the already-trusted dense path): result value columns + group order must
//! match (modulo the index label dtype). Covers the dense aggregations (part 2,
//! aggregate_temporal_sparse) AND build_groups (part 1, via groupby.nunique).
//! A NaT (i64::MIN) key bails to the generic path.

use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_types::{DType, Scalar};

fn df_with_key(key_labels: Vec<IndexLabel>, vals: &[f64]) -> DataFrame {
    let n = vals.len();
    let key_col = match &key_labels[0] {
        IndexLabel::Int64(_) => Column::from_i64_values(
            key_labels
                .iter()
                .map(|l| {
                    if let IndexLabel::Int64(v) = l {
                        *v
                    } else {
                        unreachable!()
                    }
                })
                .collect(),
        ),
        IndexLabel::Datetime64(_) => Column::from_datetime64_values(
            key_labels
                .iter()
                .map(|l| {
                    if let IndexLabel::Datetime64(v) = l {
                        *v
                    } else {
                        unreachable!()
                    }
                })
                .collect(),
        ),
        IndexLabel::Timedelta64(_) => Column::new(
            DType::Timedelta64,
            key_labels
                .iter()
                .map(|l| {
                    if let IndexLabel::Timedelta64(v) = l {
                        Scalar::Timedelta64(*v)
                    } else {
                        unreachable!()
                    }
                })
                .collect(),
        )
        .unwrap(),
        _ => unreachable!(),
    };
    let mut cols = BTreeMap::new();
    cols.insert("k".to_string(), key_col);
    cols.insert("v".to_string(), Column::from_f64_values(vals.to_vec()));
    let idx = Index::new((0..n as i64).map(IndexLabel::Int64).collect());
    DataFrame::new_with_column_order(idx, cols, vec!["k".into(), "v".into()]).unwrap()
}

fn ns_cases() -> Vec<Vec<i64>> {
    let base = 1_577_836_800_000_000_000i64;
    let day = 86_400_000_000_000i64;
    vec![
        (0..5000)
            .map(|i| base + ((i * 2654435761_i64) % 50) * day)
            .collect(),
        (0..3000).map(|i| base + (i % 7) * day).collect(),
        vec![
            base + 3 * day,
            base,
            base + day,
            base,
            base + 3 * day,
            base + day,
        ],
    ]
}

#[test]
fn groupby_datetime_key_matches_int64_key() {
    for ns in ns_cases() {
        let n = ns.len();
        let vals: Vec<f64> = (0..n).map(|i| ((i * 131 + 7) % 89) as f64 + 0.5).collect();
        let dt = df_with_key(
            ns.iter().map(|&v| IndexLabel::Datetime64(v)).collect(),
            &vals,
        );
        let i64df = df_with_key(ns.iter().map(|&v| IndexLabel::Int64(v)).collect(), &vals);

        for func in ["sum", "mean", "count", "max", "min", "std", "nunique"] {
            let rdt = run(&dt, func);
            let ri = run(&i64df, func);
            assert_eq!(rdt.0, ri.0, "{func}: group order (ns) mismatch");
            assert_eq!(rdt.1, ri.1, "{func}: value mismatch");
        }
    }
}

// Returns (group ns in result order, value column f64 bits) for comparison.
fn run(df: &DataFrame, func: &str) -> (Vec<i64>, Vec<u64>) {
    let g = df.groupby(&["k"]).unwrap();
    let r = match func {
        "sum" => g.sum(),
        "mean" => g.mean(),
        "count" => g.count(),
        "max" => g.max(),
        "min" => g.min(),
        "std" => g.std(),
        "nunique" => g.nunique(),
        _ => unreachable!(),
    }
    .unwrap();
    let ns: Vec<i64> = r
        .index()
        .labels()
        .iter()
        .map(|l| match l {
            IndexLabel::Datetime64(v) | IndexLabel::Int64(v) | IndexLabel::Timedelta64(v) => *v,
            other => panic!("idx dtype {other:?}"),
        })
        .collect();
    let vbits: Vec<u64> = r
        .columns()
        .get("v")
        .unwrap()
        .values()
        .iter()
        .map(|s| match s {
            Scalar::Float64(f) => f.to_bits(),
            Scalar::Int64(i) => *i as u64,
            Scalar::Null(_) => u64::MAX,
            other => panic!("val {other:?}"),
        })
        .collect();
    (ns, vbits)
}

#[test]
fn groupby_timedelta_key_matches_int64_key() {
    let ns: Vec<i64> = (0..3000).map(|i| (i % 11) * 1_000_000_000).collect();
    let n = ns.len();
    let vals: Vec<f64> = (0..n).map(|i| (i % 13) as f64).collect();
    let td = df_with_key(
        ns.iter().map(|&v| IndexLabel::Timedelta64(v)).collect(),
        &vals,
    );
    let i64df = df_with_key(ns.iter().map(|&v| IndexLabel::Int64(v)).collect(), &vals);
    for func in ["sum", "mean", "count", "max"] {
        assert_eq!(
            run(&td, func).1,
            run(&i64df, func).1,
            "{func} td value mismatch"
        );
    }
}

#[test]
fn groupby_datetime_key_with_nat_bails_and_is_correct() {
    let base = 1_577_836_800_000_000_000i64;
    let day = 86_400_000_000_000i64;
    let ns = [base, i64::MIN, base + day, base, i64::MIN, base + day];
    let vals = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
    let dt = df_with_key(
        ns.iter().map(|&v| IndexLabel::Datetime64(v)).collect(),
        &vals,
    );
    // NaT key bails the temporal fast path -> generic path drops the NaT group
    // (groupby dropna=True default), exactly like pandas. Present rows: base
    // (1+4=5), base+day (3+6=9) -> 14; the two NaT rows (2,5) are excluded.
    let r = dt.groupby(&["k"]).unwrap().sum().unwrap();
    let total: f64 = r
        .columns()
        .get("v")
        .unwrap()
        .values()
        .iter()
        .map(|s| if let Scalar::Float64(f) = s { *f } else { 0.0 })
        .sum();
    assert_eq!(total, 14.0, "NaT group dropped (dropna default); 5+9=14");
}
