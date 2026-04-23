//! Differential benchmarks: frankenpandas vs pandas oracle.
//!
//! Per br-frankenpandas-xha7. Today this file ships the fp-frame
//! baseline half; the pandas half (subprocess timing of
//! `pandas_oracle.py`) lands behind a `bench_pandas` feature in a
//! follow-up slice once we've validated the baseline numbers are
//! stable across the CI matrix.
//!
//! Run locally:
//!     cargo bench -p fp-conformance --bench vs_pandas
//!
//! Compare against a prior run (via criterion's built-in history):
//!     CRITERION_HOME=.target-bench cargo bench -p fp-conformance \
//!         --bench vs_pandas -- --save-baseline before
//!     # ... make changes ...
//!     CRITERION_HOME=.target-bench cargo bench -p fp-conformance \
//!         --bench vs_pandas -- --baseline before

use std::collections::BTreeMap;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use fp_columnar::Column;
use fp_frame::{DataFrame, Series};
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;

// Keep the per-bench input size up here so the pandas companion
// benches can share the exact same projection.
const SIZES: &[usize] = &[100, 1_000, 10_000];

fn build_groupby_frame(n: usize) -> DataFrame {
    let keys: Vec<Scalar> = (0..n).map(|i| Scalar::Int64((i % 10) as i64)).collect();
    let values: Vec<Scalar> = (0..n).map(|i| Scalar::Float64(i as f64 * 0.1)).collect();
    let labels: Vec<IndexLabel> = (0..n).map(|i| IndexLabel::Int64(i as i64)).collect();
    let index = Index::new(labels);
    let key_column = Column::from_values(keys).expect("key column");
    let value_column = Column::from_values(values).expect("value column");
    let mut columns = BTreeMap::new();
    columns.insert("k".to_string(), key_column);
    columns.insert("v".to_string(), value_column);
    let column_order = vec!["k".to_string(), "v".to_string()];
    DataFrame::new_with_column_order(index, columns, column_order).expect("frame")
}

fn build_arith_series(n: usize) -> (Series, Series) {
    let labels: Vec<IndexLabel> = (0..n).map(|i| IndexLabel::Int64(i as i64)).collect();
    let values_a: Vec<Scalar> = (0..n).map(|i| Scalar::Float64(i as f64)).collect();
    let values_b: Vec<Scalar> = (0..n).map(|i| Scalar::Float64((i as f64) * 1.5)).collect();
    let a = Series::from_values("a", labels.clone(), values_a).expect("a");
    let b = Series::from_values("b", labels, values_b).expect("b");
    (a, b)
}

fn bench_groupby_sum(c: &mut Criterion) {
    let mut group = c.benchmark_group("fp_frame::groupby_sum");
    for &n in SIZES {
        let frame = build_groupby_frame(n);
        group.bench_with_input(BenchmarkId::new("rows", n), &frame, |b, f| {
            b.iter(|| f.groupby(&["k"]).expect("groupby").sum().expect("sum"))
        });
    }
    group.finish();
}

fn bench_series_add(c: &mut Criterion) {
    let mut group = c.benchmark_group("fp_frame::series_add");
    for &n in SIZES {
        let (a, b_series) = build_arith_series(n);
        group.bench_with_input(
            BenchmarkId::new("rows", n),
            &(a.clone(), b_series.clone()),
            |b, inputs| b.iter(|| inputs.0.add(&inputs.1).expect("add")),
        );
    }
    group.finish();
}

criterion_group!(benches, bench_groupby_sum, bench_series_add);
criterion_main!(benches);
