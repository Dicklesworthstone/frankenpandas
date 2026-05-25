//! Differential benchmarks: frankenpandas vs pandas oracle.
//!
//! Implements the 6-category benchmark matrix from BENCH_MATRIX_SPEC.md:
//! 1. IO (0.25 weight)
//! 2. DataFrameOps (0.20 weight)
//! 3. GroupBy (0.20 weight)
//! 4. Joins (0.15 weight)
//! 5. Rolling/Expanding (0.10 weight)
//! 6. Indexing (0.10 weight)
//!
//! Run locally:
//!     cargo bench -p fp-conformance --bench vs_pandas
//!
//! Run specific category:
//!     cargo bench -p fp-conformance --bench vs_pandas -- "io/"
//!     cargo bench -p fp-conformance --bench vs_pandas -- "dataframe_ops/"
//!     cargo bench -p fp-conformance --bench vs_pandas -- "groupby/"
//!     cargo bench -p fp-conformance --bench vs_pandas -- "joins/"
//!     cargo bench -p fp-conformance --bench vs_pandas -- "rolling/"
//!     cargo bench -p fp-conformance --bench vs_pandas -- "indexing/"

use std::collections::BTreeMap;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use fp_columnar::Column;
use fp_frame::{DataFrame, Series, concat_dataframes, concat_dataframes_with_axis};
use fp_index::{DuplicateKeep, Index, IndexLabel};
use fp_io::read_csv_str;
use fp_join::{JoinType, merge_dataframes_on_with};
use fp_types::Scalar;

const SIZES: &[usize] = &[10_000, 100_000];

// ============================================================================
// DATA GENERATION HELPERS
// ============================================================================

fn build_numeric_frame(n: usize, cols: usize) -> DataFrame {
    let labels: Vec<IndexLabel> = (0..n).map(|i| IndexLabel::Int64(i as i64)).collect();
    let index = Index::new(labels);
    let mut columns = BTreeMap::new();
    let mut column_order = Vec::with_capacity(cols);
    for c in 0..cols {
        let col_name = format!("c{}", c);
        let values: Vec<Scalar> = (0..n)
            .map(|i| Scalar::Float64((i * (c + 1)) as f64 * 0.1))
            .collect();
        let column = Column::from_values(values).expect("column");
        columns.insert(col_name.clone(), column);
        column_order.push(col_name);
    }
    DataFrame::new_with_column_order(index, columns, column_order).expect("frame")
}

fn build_groupby_frame(n: usize, num_groups: usize) -> DataFrame {
    let keys: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Int64((i % num_groups) as i64))
        .collect();
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

fn build_join_frames(n: usize) -> (DataFrame, DataFrame) {
    let left_keys: Vec<Scalar> = (0..n).map(|i| Scalar::Int64(i as i64)).collect();
    let left_vals: Vec<Scalar> = (0..n).map(|i| Scalar::Float64(i as f64)).collect();
    let left_labels: Vec<IndexLabel> = (0..n).map(|i| IndexLabel::Int64(i as i64)).collect();

    let right_keys: Vec<Scalar> = (0..n).map(|i| Scalar::Int64((i * 2) as i64)).collect();
    let right_vals: Vec<Scalar> = (0..n).map(|i| Scalar::Float64(i as f64 * 10.0)).collect();
    let right_labels: Vec<IndexLabel> = (0..n).map(|i| IndexLabel::Int64(i as i64)).collect();

    let left_index = Index::new(left_labels);
    let right_index = Index::new(right_labels);

    let mut left_cols = BTreeMap::new();
    left_cols.insert("key".to_string(), Column::from_values(left_keys).unwrap());
    left_cols.insert("left_val".to_string(), Column::from_values(left_vals).unwrap());

    let mut right_cols = BTreeMap::new();
    right_cols.insert("key".to_string(), Column::from_values(right_keys).unwrap());
    right_cols.insert("right_val".to_string(), Column::from_values(right_vals).unwrap());

    let left = DataFrame::new_with_column_order(
        left_index,
        left_cols,
        vec!["key".to_string(), "left_val".to_string()],
    )
    .unwrap();
    let right = DataFrame::new_with_column_order(
        right_index,
        right_cols,
        vec!["key".to_string(), "right_val".to_string()],
    )
    .unwrap();
    (left, right)
}

fn build_series(n: usize) -> Series {
    let labels: Vec<IndexLabel> = (0..n).map(|i| IndexLabel::Int64(i as i64)).collect();
    let values: Vec<Scalar> = (0..n).map(|i| Scalar::Float64(i as f64 * 0.1)).collect();
    Series::from_values("s", labels, values).expect("series")
}

fn build_csv_string(n: usize, cols: usize) -> String {
    let mut csv = String::with_capacity(n * cols * 15);
    let header: Vec<String> = (0..cols).map(|c| format!("c{}", c)).collect();
    csv.push_str(&header.join(","));
    csv.push('\n');
    for i in 0..n {
        let row: Vec<String> = (0..cols).map(|c| format!("{}", (i * (c + 1)) as f64 * 0.1)).collect();
        csv.push_str(&row.join(","));
        csv.push('\n');
    }
    csv
}

// ============================================================================
// CATEGORY 1: IO (weight: 0.25)
// ============================================================================

fn bench_io_csv_read(c: &mut Criterion) {
    let mut group = c.benchmark_group("io/csv_read");
    for &n in SIZES {
        let csv_str = build_csv_string(n, 10);
        group.bench_with_input(BenchmarkId::new("rows", n), &csv_str, |b, csv| {
            b.iter(|| read_csv_str(csv).expect("csv read"))
        });
    }
    group.finish();
}

fn bench_io_csv_write(c: &mut Criterion) {
    let mut group = c.benchmark_group("io/csv_write");
    for &n in SIZES {
        let frame = build_numeric_frame(n, 10);
        group.bench_with_input(BenchmarkId::new("rows", n), &frame, |b, f| {
            b.iter(|| f.to_csv(',', false))
        });
    }
    group.finish();
}

fn bench_io_json_write(c: &mut Criterion) {
    let mut group = c.benchmark_group("io/json_write");
    for &n in &[10_000usize, 50_000] {
        let frame = build_numeric_frame(n, 10);
        group.bench_with_input(BenchmarkId::new("rows", n), &frame, |b, f| {
            b.iter(|| f.to_json("records").expect("json"))
        });
    }
    group.finish();
}

// ============================================================================
// CATEGORY 2: DataFrameOps (weight: 0.20)
// ============================================================================

fn bench_df_sort_single(c: &mut Criterion) {
    let mut group = c.benchmark_group("dataframe_ops/sort_single");
    for &n in SIZES {
        let frame = build_numeric_frame(n, 10);
        group.bench_with_input(BenchmarkId::new("rows", n), &frame, |b, f| {
            b.iter(|| f.sort_values("c0", true).expect("sort"))
        });
    }
    group.finish();
}

fn bench_df_sort_multi(c: &mut Criterion) {
    let mut group = c.benchmark_group("dataframe_ops/sort_multi");
    for &n in SIZES {
        let frame = build_numeric_frame(n, 10);
        group.bench_with_input(BenchmarkId::new("rows", n), &frame, |b, f| {
            b.iter(|| f.sort_values_multi(&["c0", "c1"], &[true, false], "last").expect("sort_multi"))
        });
    }
    group.finish();
}

fn bench_df_filter_bool(c: &mut Criterion) {
    let mut group = c.benchmark_group("dataframe_ops/filter_bool");
    for &n in SIZES {
        let frame = build_numeric_frame(n, 10);
        let mask: Vec<bool> = (0..n).map(|i| i % 2 == 0).collect();
        group.bench_with_input(BenchmarkId::new("rows", n), &(frame, mask), |b, (f, m)| {
            b.iter(|| f.iloc_bool(m).expect("filter"))
        });
    }
    group.finish();
}

fn bench_df_drop_duplicates(c: &mut Criterion) {
    let mut group = c.benchmark_group("dataframe_ops/drop_duplicates");
    for &n in SIZES {
        let frame = build_groupby_frame(n, 100);
        group.bench_with_input(BenchmarkId::new("rows", n), &frame, |b, f| {
            b.iter(|| f.drop_duplicates(None, DuplicateKeep::First, false).expect("dedup"))
        });
    }
    group.finish();
}

fn bench_df_value_counts(c: &mut Criterion) {
    let mut group = c.benchmark_group("dataframe_ops/value_counts");
    for &n in SIZES {
        let series = build_series(n);
        group.bench_with_input(BenchmarkId::new("rows", n), &series, |b, s| {
            b.iter(|| s.value_counts().expect("value_counts"))
        });
    }
    group.finish();
}

fn bench_df_cumsum(c: &mut Criterion) {
    let mut group = c.benchmark_group("dataframe_ops/cumsum");
    for &n in SIZES {
        let series = build_series(n);
        group.bench_with_input(BenchmarkId::new("rows", n), &series, |b, s| {
            b.iter(|| s.cumsum().expect("cumsum"))
        });
    }
    group.finish();
}

// ============================================================================
// CATEGORY 3: GroupBy (weight: 0.20)
// ============================================================================

fn bench_groupby_sum(c: &mut Criterion) {
    let mut group = c.benchmark_group("groupby/sum_int64");
    for &n in SIZES {
        let frame = build_groupby_frame(n, 100);
        group.bench_with_input(BenchmarkId::new("rows", n), &frame, |b, f| {
            b.iter(|| f.groupby(&["k"]).expect("groupby").sum().expect("sum"))
        });
    }
    group.finish();
}

fn bench_groupby_mean(c: &mut Criterion) {
    let mut group = c.benchmark_group("groupby/mean_float64");
    for &n in SIZES {
        let frame = build_groupby_frame(n, 100);
        group.bench_with_input(BenchmarkId::new("rows", n), &frame, |b, f| {
            b.iter(|| f.groupby(&["k"]).expect("groupby").mean().expect("mean"))
        });
    }
    group.finish();
}

fn bench_groupby_agg_multi(c: &mut Criterion) {
    let mut group = c.benchmark_group("groupby/agg_multi");
    for &n in SIZES {
        let frame = build_groupby_frame(n, 100);
        group.bench_with_input(BenchmarkId::new("rows", n), &frame, |b, f| {
            b.iter(|| {
                let g = f.groupby(&["k"]).expect("groupby");
                let _ = g.sum().expect("sum");
                let _ = g.mean().expect("mean");
                g.std().expect("std")
            })
        });
    }
    group.finish();
}

fn bench_groupby_ngroup(c: &mut Criterion) {
    let mut group = c.benchmark_group("groupby/ngroup");
    for &n in SIZES {
        let frame = build_groupby_frame(n, 100);
        group.bench_with_input(BenchmarkId::new("rows", n), &frame, |b, f| {
            b.iter(|| f.groupby(&["k"]).expect("groupby").ngroup().expect("ngroup"))
        });
    }
    group.finish();
}

// ============================================================================
// CATEGORY 4: Joins (weight: 0.15)
// ============================================================================

fn bench_join_inner(c: &mut Criterion) {
    let mut group = c.benchmark_group("joins/merge_inner");
    for &n in &[10_000usize, 50_000] {
        let (left, right) = build_join_frames(n);
        group.bench_with_input(BenchmarkId::new("rows", n), &(left, right), |b, (l, r)| {
            b.iter(|| {
                merge_dataframes_on_with(l, r, &["key"], &["key"], JoinType::Inner)
                    .expect("inner")
            })
        });
    }
    group.finish();
}

fn bench_join_left(c: &mut Criterion) {
    let mut group = c.benchmark_group("joins/merge_left");
    for &n in &[10_000usize, 50_000] {
        let (left, right) = build_join_frames(n);
        group.bench_with_input(BenchmarkId::new("rows", n), &(left, right), |b, (l, r)| {
            b.iter(|| {
                merge_dataframes_on_with(l, r, &["key"], &["key"], JoinType::Left)
                    .expect("left")
            })
        });
    }
    group.finish();
}

fn bench_join_outer(c: &mut Criterion) {
    let mut group = c.benchmark_group("joins/merge_outer");
    for &n in &[10_000usize, 50_000] {
        let (left, right) = build_join_frames(n);
        group.bench_with_input(BenchmarkId::new("rows", n), &(left, right), |b, (l, r)| {
            b.iter(|| {
                merge_dataframes_on_with(l, r, &["key"], &["key"], JoinType::Outer)
                    .expect("outer")
            })
        });
    }
    group.finish();
}

fn bench_concat_axis0(c: &mut Criterion) {
    let mut group = c.benchmark_group("joins/concat_axis0");
    for &n in SIZES {
        let f1 = build_numeric_frame(n / 2, 10);
        let f2 = build_numeric_frame(n / 2, 10);
        group.bench_with_input(BenchmarkId::new("rows", n), &(f1, f2), |b, (a, bb)| {
            b.iter(|| concat_dataframes(&[a, bb]).expect("concat"))
        });
    }
    group.finish();
}

fn bench_concat_axis1(c: &mut Criterion) {
    let mut group = c.benchmark_group("joins/concat_axis1");
    for &n in &[10_000usize, 50_000] {
        let f1 = build_numeric_frame(n, 5);
        let f2 = build_numeric_frame(n, 5);
        group.bench_with_input(BenchmarkId::new("rows", n), &(f1, f2), |b, (a, bb)| {
            b.iter(|| concat_dataframes_with_axis(&[a, bb], 1).expect("concat axis1"))
        });
    }
    group.finish();
}

// ============================================================================
// CATEGORY 5: Rolling/Expanding (weight: 0.10)
// ============================================================================

fn bench_rolling_mean(c: &mut Criterion) {
    let mut group = c.benchmark_group("rolling/mean_w10");
    for &n in SIZES {
        let series = build_series(n);
        group.bench_with_input(BenchmarkId::new("rows", n), &series, |b, s| {
            b.iter(|| s.rolling(10, None).mean().expect("rolling mean"))
        });
    }
    group.finish();
}

fn bench_rolling_std(c: &mut Criterion) {
    let mut group = c.benchmark_group("rolling/std_w50");
    for &n in SIZES {
        let series = build_series(n);
        group.bench_with_input(BenchmarkId::new("rows", n), &series, |b, s| {
            b.iter(|| s.rolling(50, None).std().expect("rolling std"))
        });
    }
    group.finish();
}

fn bench_expanding_sum(c: &mut Criterion) {
    let mut group = c.benchmark_group("rolling/expanding_sum");
    for &n in SIZES {
        let series = build_series(n);
        group.bench_with_input(BenchmarkId::new("rows", n), &series, |b, s| {
            b.iter(|| s.expanding(None).sum().expect("expanding sum"))
        });
    }
    group.finish();
}

fn bench_ewm_mean(c: &mut Criterion) {
    let mut group = c.benchmark_group("rolling/ewm_mean");
    for &n in SIZES {
        let series = build_series(n);
        group.bench_with_input(BenchmarkId::new("rows", n), &series, |b, s| {
            b.iter(|| s.ewm(Some(10.0), None).mean().expect("ewm mean"))
        });
    }
    group.finish();
}

// ============================================================================
// CATEGORY 6: Indexing (weight: 0.10)
// ============================================================================

fn bench_iloc_slice(c: &mut Criterion) {
    let mut group = c.benchmark_group("indexing/iloc_slice");
    for &n in SIZES {
        let frame = build_numeric_frame(n, 10);
        group.bench_with_input(BenchmarkId::new("rows", n), &frame, |b, f| {
            b.iter(|| f.iloc_slice(Some(100), Some((n - 100) as i64)).expect("iloc_slice"))
        });
    }
    group.finish();
}

fn bench_loc_labels(c: &mut Criterion) {
    let mut group = c.benchmark_group("indexing/loc_labels");
    for &n in SIZES {
        let frame = build_numeric_frame(n, 10);
        let labels: Vec<IndexLabel> = (0..1000).map(|i| IndexLabel::Int64((i * 10) as i64)).collect();
        group.bench_with_input(BenchmarkId::new("rows", n), &(frame, labels), |b, (f, lbl)| {
            b.iter(|| f.loc(lbl).expect("loc"))
        });
    }
    group.finish();
}

fn bench_at_scalar(c: &mut Criterion) {
    let mut group = c.benchmark_group("indexing/at_scalar");
    for &n in SIZES {
        let series = build_series(n);
        let label = IndexLabel::Int64((n / 2) as i64);
        group.bench_with_input(BenchmarkId::new("rows", n), &(series, label.clone()), |b, (s, lbl)| {
            b.iter(|| s.at(lbl).expect("at"))
        });
    }
    group.finish();
}

fn bench_reindex(c: &mut Criterion) {
    let mut group = c.benchmark_group("indexing/reindex");
    for &n in SIZES {
        let series = build_series(n);
        let new_labels: Vec<IndexLabel> = (0..n)
            .map(|i| IndexLabel::Int64(((i * 3) % (n * 2)) as i64))
            .collect();
        group.bench_with_input(BenchmarkId::new("rows", n), &(series, new_labels), |b, (s, lbl)| {
            b.iter(|| s.reindex(lbl.clone()).expect("reindex"))
        });
    }
    group.finish();
}

// ============================================================================
// CRITERION GROUPS
// ============================================================================

criterion_group!(
    io_benches,
    bench_io_csv_read,
    bench_io_csv_write,
    bench_io_json_write
);

criterion_group!(
    dataframe_ops_benches,
    bench_df_sort_single,
    bench_df_sort_multi,
    bench_df_filter_bool,
    bench_df_drop_duplicates,
    bench_df_value_counts,
    bench_df_cumsum
);

criterion_group!(
    groupby_benches,
    bench_groupby_sum,
    bench_groupby_mean,
    bench_groupby_agg_multi,
    bench_groupby_ngroup
);

criterion_group!(
    joins_benches,
    bench_join_inner,
    bench_join_left,
    bench_join_outer,
    bench_concat_axis0,
    bench_concat_axis1
);

criterion_group!(
    rolling_benches,
    bench_rolling_mean,
    bench_rolling_std,
    bench_expanding_sum,
    bench_ewm_mean
);

criterion_group!(
    indexing_benches,
    bench_iloc_slice,
    bench_loc_labels,
    bench_at_scalar,
    bench_reindex
);

criterion_main!(
    io_benches,
    dataframe_ops_benches,
    groupby_benches,
    joins_benches,
    rolling_benches,
    indexing_benches
);
