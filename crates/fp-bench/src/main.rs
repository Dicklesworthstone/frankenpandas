//! Head-to-head vs-pandas timing harness — FrankenPandas side.
//!
//! Driven by `benches/vs_pandas_harness.py`, which invokes:
//!   fp-bench --category C --workload W --size S --dtype D --json
//! and parses `{"times_us": [..]}` from stdout. The Python side runs the
//! identical workload on pandas 2.2.3; the harness computes the head-to-head
//! ratio. This restores the FrankenPandas half of the no-gaps measurement loop
//! (the `fp-bench` binary had never existed, so the harness skipped every FP
//! workload). Setup/population is OUTSIDE the timed window, matching the spec.
//!
//! Coverage (v1): dataframe_ops, groupby, rolling. io/indexing/joins/expanding
//! /ewm map to more setup-heavy harnesses and are filed as follow-up; the Python
//! side simply reports those FP workloads as INCOMPLETE until added here.

use std::{collections::BTreeMap, hint::black_box, time::Instant};

use fp_columnar::Column;
use fp_frame::{DataFrame, Series, to_datetime};
use fp_index::{DuplicateKeep, Index, IndexLabel, RangeIndex};
use fp_join::{JoinType, merge_dataframes_on_with};
use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

const WARMUP: usize = 3;
const ITERS: usize = 25;
const TAKE_BATCH: usize = 256;

/// splitmix64 — deterministic, seed-stable uniform stream. We only need a
/// fair-distribution data set for TIMING (not bit-identity with numpy's PCG64),
/// so a cheap reproducible generator suffices.
struct SplitMix64(u64);
impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Uniform f64 in [0, 1).
    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn arg<'a>(args: &'a [String], key: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == key)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

fn size_rows_cols(size: &str) -> (usize, usize) {
    match size {
        "10k" => (10_000, 10),
        "100k" => (100_000, 10),
        "1M" => (1_000_000, 10),
        _ => (100_000, 10),
    }
}

fn arithmetic_take_positions(rows: usize) -> Vec<usize> {
    let start = rows / 8;
    let stop = rows - start;
    (start..stop).step_by(2).collect()
}

/// Build one Float64 column of `rows` values per the requested dtype, advancing
/// the shared RNG so columns differ (mirrors numpy's column-by-column fill).
fn gen_f64_column(rng: &mut SplitMix64, rows: usize, dtype: &str) -> Vec<f64> {
    let nan_frac = match dtype {
        "float64_nan10" => 0.10,
        "float64_nan50" => 0.50,
        _ => 0.0,
    };
    (0..rows)
        .map(|_| {
            let value = rng.unit() * 1_000_000.0;
            if nan_frac > 0.0 && rng.unit() < nan_frac {
                f64::NAN
            } else {
                value
            }
        })
        .collect()
}

fn build_frame(rows: usize, cols: usize, dtype: &str) -> (DataFrame, Vec<Vec<f64>>) {
    let mut rng = SplitMix64(0x5151_5151_5151_5151);
    let index = Index::new_known_unique_int64_unit_range(0, rows);
    let mut columns = BTreeMap::new();
    let mut column_order = Vec::with_capacity(cols);
    let mut raw: Vec<Vec<f64>> = Vec::with_capacity(cols);
    for c in 0..cols {
        let name = format!("col_{c}");
        let data = gen_f64_column(&mut rng, rows, dtype);
        raw.push(data.clone());
        columns.insert(name.clone(), Column::from_f64_values(data));
        column_order.push(name);
    }
    let df = DataFrame::new_with_column_order(index, columns, column_order)
        .expect("fp-bench frame construction");
    (df, raw)
}

/// Build the two merge inputs for the joins category — mirrors the criterion
/// `build_join_frames` and the pandas `_build_join_frames` (left key 0..n,
/// right key 0,2,..,2(n-1); a unique-key Int64 join whose inner result keeps
/// ~n/2 matched rows).
fn build_join_frames(n: usize) -> (DataFrame, DataFrame) {
    let left_index = Index::new_known_unique_int64_unit_range(0, n);
    let mut left_cols = BTreeMap::new();
    left_cols.insert(
        "key".to_string(),
        Column::from_i64_values((0..n as i64).collect()),
    );
    left_cols.insert(
        "left_val".to_string(),
        Column::from_f64_values((0..n).map(|i| i as f64).collect()),
    );
    let left = DataFrame::new_with_column_order(
        left_index,
        left_cols,
        vec!["key".to_string(), "left_val".to_string()],
    )
    .expect("fp-bench left join frame");

    let right_index = Index::new_known_unique_int64_unit_range(0, n);
    let mut right_cols = BTreeMap::new();
    right_cols.insert(
        "key".to_string(),
        Column::from_i64_values((0..n as i64).map(|i| i * 2).collect()),
    );
    right_cols.insert(
        "right_val".to_string(),
        Column::from_f64_values((0..n).map(|i| i as f64 * 10.0).collect()),
    );
    let right = DataFrame::new_with_column_order(
        right_index,
        right_cols,
        vec!["key".to_string(), "right_val".to_string()],
    )
    .expect("fp-bench right join frame");
    (left, right)
}

/// Build a string-column frame for the strings category — mirrors the pandas
/// `_build_str_frame`: `key` is a ~1000-distinct group label (g0000..g0999),
/// `name` is a unique ~15-byte id (for the sort key), `val` is a Float64.
fn build_str_frame(n: usize) -> DataFrame {
    let mut key_bytes = Vec::new();
    let mut key_off = Vec::with_capacity(n + 1);
    key_off.push(0usize);
    let mut name_bytes = Vec::new();
    let mut name_off = Vec::with_capacity(n + 1);
    name_off.push(0usize);
    for i in 0..n {
        let key = format!("g{:04}", i % 1000);
        key_bytes.extend_from_slice(key.as_bytes());
        key_off.push(key_bytes.len());
        let name = format!("item_{i:010}");
        name_bytes.extend_from_slice(name.as_bytes());
        name_off.push(name_bytes.len());
    }
    let vals: Vec<f64> = (0..n).map(|i| i as f64).collect();
    let index = Index::new_known_unique_int64_unit_range(0, n);
    let mut cols = BTreeMap::new();
    cols.insert(
        "key".to_string(),
        Column::from_utf8_contiguous(key_bytes, key_off),
    );
    cols.insert(
        "name".to_string(),
        Column::from_utf8_contiguous(name_bytes, name_off),
    );
    cols.insert("val".to_string(), Column::from_f64_values(vals));
    DataFrame::new_with_column_order(
        index,
        cols,
        vec!["key".to_string(), "name".to_string(), "val".to_string()],
    )
    .expect("fp-bench string frame")
}

/// Build a square `dim x dim` all-finite Float64 frame for the df.dot GEMM
/// workload (col_0..col_{dim-1}), mirroring the pandas `_build_square_frame`.
fn build_square_f64_frame(dim: usize) -> DataFrame {
    let mut rng = SplitMix64(0x1234_5678_9abc_def0);
    let index = Index::new_known_unique_int64_unit_range(0, dim);
    let mut columns = BTreeMap::new();
    let mut column_order = Vec::with_capacity(dim);
    for c in 0..dim {
        let name = format!("col_{c}");
        let data: Vec<f64> = (0..dim).map(|_| rng.unit()).collect();
        columns.insert(name.clone(), Column::from_f64_values(data));
        column_order.push(name);
    }
    DataFrame::new_with_column_order(index, columns, column_order).expect("fp-bench square frame")
}

/// Time a closure `ITERS` times after `WARMUP` warmups; return per-iter µs.
fn time_us<F: FnMut()>(mut op: F) -> Vec<f64> {
    for _ in 0..WARMUP {
        op();
    }
    let mut out = Vec::with_capacity(ITERS);
    for _ in 0..ITERS {
        let t = Instant::now();
        op();
        out.push(t.elapsed().as_secs_f64() * 1e6);
    }
    out
}

fn run(category: &str, workload: &str, size: &str, dtype: &str) -> Option<Vec<f64>> {
    let (rows, cols) = size_rows_cols(size);
    let (df, raw) = build_frame(rows, cols, dtype);

    let times = match (category, workload) {
        ("dataframe_ops", "sort_values_single") => time_us(|| {
            let _ = df.sort_values("col_0", true).expect("sort_values");
        }),
        ("dataframe_ops", "sort_values_multi") => time_us(|| {
            let _ = df
                .sort_values_multi(&["col_0", "col_1", "col_2"], &[true, true, true], "last")
                .expect("sort_values_multi");
        }),
        ("dataframe_ops", "filter_bool_mask") => {
            // df[df.col_0 > df.col_0.median()]
            let med = df
                .get_column("col_0")
                .median()
                .ok()
                .and_then(|s| s.to_f64().ok())
                .unwrap_or(f64::NAN);
            let mask: Vec<bool> = raw[0].iter().map(|&v| v > med).collect();
            time_us(|| {
                let _ = df.loc_bool(&mask).expect("loc_bool");
            })
        }
        ("dataframe_ops", "drop_duplicates") => {
            let subset = vec!["col_0".to_string()];
            time_us(|| {
                let _ = df
                    .drop_duplicates(Some(&subset), DuplicateKeep::First, false)
                    .expect("drop_duplicates");
            })
        }
        ("dataframe_ops", "value_counts_i64") => {
            // value_counts on a bounded Int64 column (i%1000) — 1000 distinct.
            let col = Column::from_i64_values((0..rows as i64).map(|i| i % 1000).collect());
            let series = Series::new("s", Index::new_known_unique_int64_unit_range(0, rows), col)
                .expect("vc i64 series");
            time_us(|| {
                let _ = series.value_counts().expect("value_counts");
            })
        }
        ("dataframe_ops", "value_counts") => {
            let series = df.get_column("col_0");
            time_us(|| {
                let _ = series.value_counts().expect("value_counts");
            })
        }
        ("dataframe_ops", "cumsum") => time_us(|| {
            let _ = df.cumsum().expect("cumsum");
        }),
        ("dataframe_ops", "describe") => time_us(|| {
            // pandas: df.describe()
            let _ = df.describe().expect("describe");
        }),
        ("dataframe_ops", "rank") => time_us(|| {
            // pandas: df.rank() — method='average', ascending=True, na_option='keep'
            let _ = df.rank("average", true, "keep").expect("rank");
        }),
        ("dataframe_ops", "df_rank_axis1") => time_us(|| {
            let _ = df.rank_axis1("average", true, "keep").expect("rank_axis1");
        }),
        ("dataframe_ops", "df_rank_axis1_min") => time_us(|| {
            let _ = df.rank_axis1("min", true, "keep").expect("rank_axis1");
        }),
        ("dataframe_ops", "df_idxmax_axis1") => time_us(|| {
            let _ = df.idxmax_axis1().expect("idxmax_axis1");
        }),
        ("dataframe_ops", "df_idxmin_axis1") => time_us(|| {
            let _ = df.idxmin_axis1().expect("idxmin_axis1");
        }),
        ("dataframe_ops", "df_mean_axis1") => time_us(|| { let _ = df.mean_axis1().expect("x"); }),
        ("dataframe_ops", "df_max_axis1") => time_us(|| { let _ = df.max_axis1().expect("x"); }),
        ("dataframe_ops", "df_var_axis1") => time_us(|| { let _ = df.var_axis1().expect("x"); }),
        ("dataframe_ops", "df_prod_axis1") => time_us(|| { let _ = df.prod_axis1().expect("x"); }),
        ("dataframe_ops", "df_count_axis1") => time_us(|| { let _ = df.count_axis1().expect("x"); }),
        ("dataframe_ops", "df_argmax_axis1") => time_us(|| { let _ = df.argmax_axis1().expect("x"); }),
        ("dataframe_ops", "df_std_axis1") => time_us(|| {
            let _ = df.std_axis1().expect("std_axis1");
        }),
        ("dataframe_ops", "df_median_axis1") => time_us(|| {
            let _ = df.median_axis1().expect("median_axis1");
        }),
        ("dataframe_ops", "df_abs") => time_us(|| {
            // pandas: df.abs()
            let _ = df.abs().expect("abs");
        }),
        ("dataframe_ops", "df_transpose") => time_us(|| {
            // pandas: df.T
            let _ = df.transpose().expect("transpose");
        }),
        ("dataframe_ops", "df_diff") => time_us(|| {
            // pandas: df.diff()
            let _ = df.diff(1).expect("diff");
        }),
        ("dataframe_ops", "df_notna") => time_us(|| {
            // pandas: df.notna()
            let _ = df.notna().expect("notna");
        }),
        ("dataframe_ops", "df_pivot_table") => {
            // pandas: df.pivot_table(values="v", index="r", columns="c",
            // aggfunc="mean"); r=i%100 (100 rows), c=i%10 (10 cols) -> 100x10.
            let r: Vec<i64> = (0..rows as i64).map(|i| i % 100).collect();
            let c: Vec<i64> = (0..rows as i64).map(|i| i % 10).collect();
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let mut columns = BTreeMap::new();
            columns.insert("r".to_string(), Column::from_i64_values(r));
            columns.insert("c".to_string(), Column::from_i64_values(c));
            columns.insert("v".to_string(), Column::from_f64_values(raw[0].clone()));
            let pframe = DataFrame::new_with_column_order(
                index,
                columns,
                vec!["r".to_string(), "c".to_string(), "v".to_string()],
            )
            .expect("pivot frame");
            time_us(|| {
                let _ = pframe
                    .pivot_table("v", "r", "c", "mean")
                    .expect("pivot_table");
            })
        }
        ("dataframe_ops", "df_pivot_table_std") => {
            let r: Vec<i64> = (0..rows as i64).map(|i| i % 100).collect();
            let c: Vec<i64> = (0..rows as i64).map(|i| i % 10).collect();
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let mut columns = BTreeMap::new();
            columns.insert("r".to_string(), Column::from_i64_values(r));
            columns.insert("c".to_string(), Column::from_i64_values(c));
            columns.insert("v".to_string(), Column::from_f64_values(raw[0].clone()));
            let pframe = DataFrame::new_with_column_order(index, columns, vec!["r".to_string(), "c".to_string(), "v".to_string()]).expect("pivot frame");
            time_us(|| { let _ = pframe.pivot_table("v", "r", "c", "std").expect("pivot_table"); })
        }
        ("dataframe_ops", "df_pivot_table_median") => {
            let r: Vec<i64> = (0..rows as i64).map(|i| i % 100).collect();
            let c: Vec<i64> = (0..rows as i64).map(|i| i % 10).collect();
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let mut columns = BTreeMap::new();
            columns.insert("r".to_string(), Column::from_i64_values(r));
            columns.insert("c".to_string(), Column::from_i64_values(c));
            columns.insert("v".to_string(), Column::from_f64_values(raw[0].clone()));
            let pframe = DataFrame::new_with_column_order(index, columns, vec!["r".to_string(), "c".to_string(), "v".to_string()]).expect("pivot frame");
            time_us(|| { let _ = pframe.pivot_table("v", "r", "c", "median").expect("pivot_table"); })
        }
        ("dataframe_ops", "df_pivot") => {
            // pandas: df.pivot(index="r", columns="c", values="v"); UNIQUE (r,c)
            // pairs (pivot errors on dups): r=i/10 (rows/10 distinct), c=i%10.
            let r: Vec<i64> = (0..rows as i64).map(|i| i / 10).collect();
            let c: Vec<i64> = (0..rows as i64).map(|i| i % 10).collect();
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let mut columns = BTreeMap::new();
            columns.insert("r".to_string(), Column::from_i64_values(r));
            columns.insert("c".to_string(), Column::from_i64_values(c));
            columns.insert("v".to_string(), Column::from_f64_values(raw[0].clone()));
            let pframe = DataFrame::new_with_column_order(
                index,
                columns,
                vec!["r".to_string(), "c".to_string(), "v".to_string()],
            )
            .expect("pivot frame");
            time_us(|| {
                let _ = pframe.pivot("r", "c", "v").expect("pivot");
            })
        }
        ("dataframe_ops", "df_crosstab") => {
            // pandas: pd.crosstab(a, b); a=i%100, b=i%10 -> 100x10 counts.
            let a = Column::from_i64_values((0..rows as i64).map(|i| i % 100).collect());
            let b = Column::from_i64_values((0..rows as i64).map(|i| i % 10).collect());
            let s1 = Series::new("a", Index::new_known_unique_int64_unit_range(0, rows), a)
                .expect("crosstab s1");
            let s2 = Series::new("b", Index::new_known_unique_int64_unit_range(0, rows), b)
                .expect("crosstab s2");
            time_us(|| {
                let _ = DataFrame::crosstab(&s1, &s2).expect("crosstab");
            })
        }
        ("dataframe_ops", "series_map") => {
            // pandas: s.map(mapper); s values in 0..100, mapper maps 0..99.
            let vals = Column::from_i64_values((0..rows as i64).map(|i| i % 100).collect());
            let s = Series::new("s", Index::new_known_unique_int64_unit_range(0, rows), vals)
                .expect("map self");
            let mvals = Column::from_i64_values((0..100).collect());
            let mapper = Series::new("m", Index::new_known_unique_int64_unit_range(0, 100), mvals)
                .expect("mapper");
            time_us(|| {
                let _ = s.map_series(&mapper).expect("map");
            })
        }
        ("dataframe_ops", "df_unstack") => {
            // Series with "r, c" composite labels (r=i/10, c=i%10) -> unstack to
            // (rows/10) x 10. Mirrors df_pivot shape via the composite-key path.
            let labels: Vec<IndexLabel> = (0..rows)
                .map(|i| IndexLabel::Utf8(format!("{}, {}", i / 10, i % 10)))
                .collect();
            let s = Series::new("s", Index::new(labels), Column::from_f64_values(raw[0].clone()))
                .expect("unstack series");
            time_us(|| {
                let _ = s.unstack().expect("unstack");
            })
        }
        ("dataframe_ops", "df_get_dummies") => {
            // pandas: pd.get_dummies(df, columns=["cat"]); cat=i%100 -> 100 dummies.
            let cat = Column::from_i64_values((0..rows as i64).map(|i| i % 100).collect());
            let mut columns = BTreeMap::new();
            columns.insert("cat".to_string(), cat);
            let df = DataFrame::new_with_column_order(
                Index::new_known_unique_int64_unit_range(0, rows),
                columns,
                vec!["cat".to_string()],
            )
            .expect("gd frame");
            time_us(|| {
                let _ = df.get_dummies(&["cat"]).expect("get_dummies");
            })
        }
        ("dataframe_ops", "series_categorical") => {
            // pandas: pd.Series(arr).astype("category"); arr=i%100 (100 cats).
            let values: Vec<fp_types::Scalar> =
                (0..rows as i64).map(|i| fp_types::Scalar::Int64(i % 100)).collect();
            time_us(|| {
                let _ = Series::from_categorical("c", values.clone(), false).expect("categorical");
            })
        }
        ("dataframe_ops", "df_quantile") => time_us(|| {
            // pandas: df.quantile(0.5)
            let _ = df.quantile(0.5).expect("quantile");
        }),
        ("dataframe_ops", "df_skew") => time_us(|| {
            // pandas: df.skew()
            let _ = df.skew().expect("skew");
        }),
        ("dataframe_ops", "df_sem") => time_us(|| {
            // pandas: df.sem()
            let _ = df.sem().expect("sem");
        }),
        ("dataframe_ops", "df_nunique") => time_us(|| {
            // pandas: df.nunique()
            let _ = df.nunique().expect("nunique");
        }),
        ("dataframe_ops", "df_cumprod") => time_us(|| {
            // pandas: df.cumprod()
            let _ = df.cumprod().expect("cumprod");
        }),
        ("dataframe_ops", "df_shift") => time_us(|| {
            // pandas: df.shift(1)
            let _ = df.shift(1).expect("shift");
        }),
        ("dataframe_ops", "df_pct_change") => time_us(|| {
            // pandas: df.pct_change()
            let _ = df.pct_change(1).expect("pct_change");
        }),
        ("dataframe_ops", "df_ffill") => time_us(|| {
            // pandas: df.ffill() — run with --dtype float64_nan10/nan50
            let _ = df.ffill(None).expect("ffill");
        }),
        ("dataframe_ops", "df_interpolate") => time_us(|| {
            // pandas: df.interpolate() — run with --dtype float64_nan10
            let _ = df.interpolate().expect("interpolate");
        }),
        ("dataframe_ops", "df_set_index") => time_us(|| {
            // pandas: df.set_index("col_0")
            let _ = df.set_index("col_0", true).expect("set_index");
        }),
        ("dataframe_ops", "df_reset_index") => time_us(|| {
            // pandas: df.reset_index()
            let _ = df.reset_index(false).expect("reset_index");
        }),
        ("dataframe_ops", "df_sort_index") => time_us(|| {
            // pandas: df.sort_index()
            let _ = df.sort_index(true).expect("sort_index");
        }),
        ("dataframe_ops", "astype_str_i64" | "astype_str_f64" | "astype_str_bool") => {
            let col = match workload {
                "astype_str_i64" => Column::from_i64_values((0..rows as i64).collect()),
                "astype_str_bool" => Column::from_bool_values((0..rows).map(|i| i % 2 == 0).collect()),
                _ => Column::from_f64_values((0..rows).map(|i| i as f64 * 1.5).collect()),
            };
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let series = Series::new("s", index, col).expect("astype series");
            time_us(|| {
                let _ = series.astype(fp_types::DType::Utf8).expect("astype str");
            })
        }
        ("dataframe_ops", "df_melt") => time_us(|| {
            // pandas: df.melt()
            let _ = df.melt(&[], &[], None, None).expect("melt");
        }),
        ("dataframe_ops", "df_explode") => {
            // Series of comma-separated strings "aN,bN,cN" (3 parts each).
            // pandas: s.str.split(",").explode().
            let mut bytes: Vec<u8> = Vec::new();
            let mut offsets: Vec<usize> = vec![0];
            for i in 0..rows {
                bytes.extend_from_slice(format!("a{},b{},c{}", i % 97, i % 89, i % 83).as_bytes());
                offsets.push(bytes.len());
            }
            let col = Column::from_utf8_contiguous(bytes, offsets);
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let series = Series::new("s", index, col).expect("explode series");
            time_us(|| {
                let _ = series.explode(",").expect("explode");
            })
        }
        ("dataframe_ops", "df_nlargest") => time_us(|| {
            // pandas: df.nlargest(100, "col_0")
            let _ = df.nlargest(100, "col_0").expect("nlargest");
        }),
        ("dataframe_ops", "df_stack") => time_us(|| {
            // pandas: df.stack()
            let _ = df.stack().expect("stack");
        }),
        ("dataframe_ops", "df_duplicated") => time_us(|| {
            // pandas: df.duplicated()
            let _ = df
                .duplicated(None, DuplicateKeep::First)
                .expect("duplicated");
        }),
        ("dataframe_ops", "df_idxmax") => time_us(|| {
            // pandas: df.idxmax()
            let _ = df.idxmax().expect("idxmax");
        }),
        ("dataframe_ops", "df_count") => time_us(|| {
            // pandas: df.count()
            let _ = df.count().expect("count");
        }),
        ("dataframe_ops", "df_to_numpy") => time_us(|| {
            // pandas: df.to_numpy()
            let _ = df.to_numpy();
        }),
        ("dataframe_ops", "df_mode") => time_us(|| {
            // pandas: df.mode()
            let _ = df.mode().expect("mode");
        }),
        ("dataframe_ops", "df_fillna") => {
            // pandas: df.fillna(0.0) — run with --dtype float64_nan10/nan50
            let fill = fp_types::Scalar::Float64(0.0);
            time_us(|| {
                let _ = df.fillna(&fill).expect("fillna");
            })
        }
        ("dataframe_ops", "df_add_scalar") => time_us(|| {
            // pandas: df + 5.0
            let _ = df.add_scalar(5.0).expect("add_scalar");
        }),
        ("dataframe_ops", "df_sign") => time_us(|| {
            // pandas: np.sign(df)
            let _ = df.sign().expect("sign");
        }),
        ("dataframe_ops", "df_neg") => time_us(|| {
            // pandas: -df
            let _ = df.neg().expect("neg");
        }),
        ("dataframe_ops", "df_floor") => time_us(|| {
            // pandas: np.floor(df)
            let _ = df.floor().expect("floor");
        }),
        ("dataframe_ops", "df_ceil") => time_us(|| {
            // pandas: np.ceil(df)
            let _ = df.ceil().expect("ceil");
        }),
        ("dataframe_ops", "df_round") => time_us(|| {
            // pandas: df.round(2)
            let _ = df.round(2).expect("round");
        }),
        ("dataframe_ops", "df_clip") => time_us(|| {
            // pandas: df.clip(lower=0, upper=500000)
            let _ = df.clip(Some(0.0), Some(500_000.0)).expect("clip");
        }),
        ("dataframe_ops", "df_isna") => time_us(|| {
            // pandas: df.isna()
            let _ = df.isna().expect("isna");
        }),
        (
            "groupby",
            "groupby_sum_int64"
            | "groupby_mean_float64"
            | "groupby_agg_multi"
            | "groupby_std"
            | "groupby_median"
            | "groupby_nunique"
            | "groupby_first"
            | "groupby_max",
        ) => {
            // pandas: key = (col_0 % 100).astype(int64); groupby(key)[col_1].agg
            let keys: Vec<i64> = raw[0].iter().map(|&v| (v as i64).rem_euclid(100)).collect();
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let mut columns = BTreeMap::new();
            columns.insert("key".to_string(), Column::from_i64_values(keys));
            columns.insert("col_1".to_string(), Column::from_f64_values(raw[1].clone()));
            let gframe = DataFrame::new_with_column_order(
                index,
                columns,
                vec!["key".to_string(), "col_1".to_string()],
            )
            .expect("fp-bench groupby frame");
            match workload {
                "groupby_std" => time_us(|| {
                    let _ = gframe
                        .groupby(&["key"])
                        .expect("groupby")
                        .std()
                        .expect("std");
                }),
                "groupby_median" => time_us(|| {
                    let _ = gframe
                        .groupby(&["key"])
                        .expect("groupby")
                        .median()
                        .expect("median");
                }),
                "groupby_nunique" => time_us(|| {
                    let _ = gframe
                        .groupby(&["key"])
                        .expect("groupby")
                        .nunique()
                        .expect("nunique");
                }),
                "groupby_first" => time_us(|| {
                    let _ = gframe
                        .groupby(&["key"])
                        .expect("groupby")
                        .first()
                        .expect("first");
                }),
                "groupby_max" => time_us(|| {
                    let _ = gframe
                        .groupby(&["key"])
                        .expect("groupby")
                        .max()
                        .expect("max");
                }),
                "groupby_sum_int64" => time_us(|| {
                    let _ = gframe
                        .groupby(&["key"])
                        .expect("groupby")
                        .sum()
                        .expect("sum");
                }),
                "groupby_mean_float64" => time_us(|| {
                    let _ = gframe
                        .groupby(&["key"])
                        .expect("groupby")
                        .mean()
                        .expect("mean");
                }),
                _ => time_us(|| {
                    // pandas: df.groupby("key").agg({"col_1": ["sum","mean","std"]})
                    // — one multi-agg call. Mirror it with the canonical agg API
                    // instead of three separate gb.sum()/mean()/std() calls so the
                    // workload measures the path br-frankenpandas-m0gcq will fuse.
                    let gb = gframe.groupby(&["key"]).expect("groupby");
                    let _ = gb.agg_list(&["sum", "mean", "std"]).expect("agg_list");
                }),
            }
        }
        ("groupby", "groupby_transform_mean") => {
            // pandas: s = df["col_1"]; s.groupby(key).transform("mean")
            // — SeriesGroupBy.transform (broadcast each group's mean back to its
            // rows). key = (col_0 % 100).astype(int64).
            let keys: Vec<i64> = raw[0].iter().map(|&v| (v as i64).rem_euclid(100)).collect();
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new(
                "key".to_string(),
                index.clone(),
                Column::from_i64_values(keys),
            )
            .expect("key series");
            let val_series = Series::new(
                "col_1".to_string(),
                index,
                Column::from_f64_values(raw[1].clone()),
            )
            .expect("val series");
            time_us(|| {
                let _ = val_series
                    .groupby(&key_series)
                    .expect("groupby")
                    .transform("mean")
                    .expect("transform");
            })
        }
        ("groupby", "groupby_mean_str") => {
            // String-key aggregation: s.groupby(str_key).mean(). key =
            // "g{col_0 % 1000:04}" (~1000 distinct categorical labels).
            let mut kb = Vec::with_capacity(rows * 5);
            let mut ko = Vec::with_capacity(rows + 1);
            ko.push(0usize);
            for &v in raw[0].iter() {
                kb.extend_from_slice(format!("g{:04}", (v as i64).rem_euclid(1000)).as_bytes());
                ko.push(kb.len());
            }
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new(
                "key".to_string(),
                index.clone(),
                Column::from_utf8_contiguous(kb, ko),
            )
            .expect("key series");
            let val_series = Series::new(
                "col_1".to_string(),
                index,
                Column::from_f64_values(raw[1].clone()),
            )
            .expect("val series");
            time_us(|| {
                let _ = val_series
                    .groupby(&key_series)
                    .expect("groupby")
                    .mean()
                    .expect("mean");
            })
        }
        ("groupby", "groupby_median_str") => {
            let mut kb = Vec::with_capacity(rows * 5);
            let mut ko = Vec::with_capacity(rows + 1);
            ko.push(0usize);
            for &v in raw[0].iter() {
                kb.extend_from_slice(format!("g{:04}", (v as i64).rem_euclid(1000)).as_bytes());
                ko.push(kb.len());
            }
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new("key".to_string(), index.clone(), Column::from_utf8_contiguous(kb, ko)).expect("key");
            let val_series = Series::new("col_1".to_string(), index, Column::from_f64_values(raw[1].clone())).expect("val");
            time_us(|| { let _ = val_series.groupby(&key_series).expect("groupby").median().expect("median"); })
        }
        ("groupby", "groupby_std_str") => {
            let mut kb = Vec::with_capacity(rows * 5);
            let mut ko = Vec::with_capacity(rows + 1);
            ko.push(0usize);
            for &v in raw[0].iter() {
                kb.extend_from_slice(format!("g{:04}", (v as i64).rem_euclid(1000)).as_bytes());
                ko.push(kb.len());
            }
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new("key".to_string(), index.clone(), Column::from_utf8_contiguous(kb, ko)).expect("key");
            let val_series = Series::new("col_1".to_string(), index, Column::from_f64_values(raw[1].clone())).expect("val");
            time_us(|| { let _ = val_series.groupby(&key_series).expect("groupby").std().expect("std"); })
        }
        ("groupby", "groupby_var_str") => {
            let mut kb = Vec::with_capacity(rows * 5);
            let mut ko = Vec::with_capacity(rows + 1);
            ko.push(0usize);
            for &v in raw[0].iter() {
                kb.extend_from_slice(format!("g{:04}", (v as i64).rem_euclid(1000)).as_bytes());
                ko.push(kb.len());
            }
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new("key".to_string(), index.clone(), Column::from_utf8_contiguous(kb, ko)).expect("key");
            let val_series = Series::new("col_1".to_string(), index, Column::from_f64_values(raw[1].clone())).expect("val");
            time_us(|| { let _ = val_series.groupby(&key_series).expect("groupby").var().expect("var"); })
        }
        ("groupby", "groupby_multi_str") => {
            let mut kb = Vec::with_capacity(rows * 5);
            let mut ko = Vec::with_capacity(rows + 1);
            ko.push(0usize);
            for &v in raw[0].iter() {
                kb.extend_from_slice(format!("g{:04}", (v as i64).rem_euclid(1000)).as_bytes());
                ko.push(kb.len());
            }
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new("key".to_string(), index.clone(), Column::from_utf8_contiguous(kb, ko)).expect("key");
            let val_series = Series::new("col_1".to_string(), index, Column::from_f64_values(raw[1].clone())).expect("val");
            time_us(|| {
                let g = val_series.groupby(&key_series).expect("groupby");
                let _ = g.mean().expect("mean");
                let _ = g.std().expect("std");
                let _ = g.var().expect("var");
            })
        }
        ("groupby", "groupby_min_str") => {
            let mut kb = Vec::with_capacity(rows * 5);
            let mut ko = Vec::with_capacity(rows + 1);
            ko.push(0usize);
            for &v in raw[0].iter() {
                kb.extend_from_slice(format!("g{:04}", (v as i64).rem_euclid(1000)).as_bytes());
                ko.push(kb.len());
            }
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new("key".to_string(), index.clone(), Column::from_utf8_contiguous(kb, ko)).expect("key");
            let val_series = Series::new("col_1".to_string(), index, Column::from_f64_values(raw[1].clone())).expect("val");
            time_us(|| { let _ = val_series.groupby(&key_series).expect("groupby").min().expect("min"); })
        }
        ("groupby", "groupby_max_str") => {
            let mut kb = Vec::with_capacity(rows * 5);
            let mut ko = Vec::with_capacity(rows + 1);
            ko.push(0usize);
            for &v in raw[0].iter() {
                kb.extend_from_slice(format!("g{:04}", (v as i64).rem_euclid(1000)).as_bytes());
                ko.push(kb.len());
            }
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new("key".to_string(), index.clone(), Column::from_utf8_contiguous(kb, ko)).expect("key");
            let val_series = Series::new("col_1".to_string(), index, Column::from_f64_values(raw[1].clone())).expect("val");
            time_us(|| { let _ = val_series.groupby(&key_series).expect("groupby").max().expect("max"); })
        }
        ("groupby", "groupby_prod_str") => {
            let mut kb = Vec::with_capacity(rows * 5);
            let mut ko = Vec::with_capacity(rows + 1);
            ko.push(0usize);
            for &v in raw[0].iter() {
                kb.extend_from_slice(format!("g{:04}", (v as i64).rem_euclid(1000)).as_bytes());
                ko.push(kb.len());
            }
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new("key".to_string(), index.clone(), Column::from_utf8_contiguous(kb, ko)).expect("key");
            let val_series = Series::new("col_1".to_string(), index, Column::from_f64_values(raw[1].clone())).expect("val");
            time_us(|| { let _ = val_series.groupby(&key_series).expect("groupby").prod().expect("prod"); })
        }
        ("groupby", "groupby_rank_str") => {
            let mut kb = Vec::with_capacity(rows * 5);
            let mut ko = Vec::with_capacity(rows + 1);
            ko.push(0usize);
            for &v in raw[0].iter() { kb.extend_from_slice(format!("g{:04}", (v as i64).rem_euclid(1000)).as_bytes()); ko.push(kb.len()); }
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new("key".to_string(), index.clone(), Column::from_utf8_contiguous(kb, ko)).expect("key");
            let val_series = Series::new("col_1".to_string(), index, Column::from_f64_values(raw[1].clone())).expect("val");
            time_us(|| { let _ = val_series.groupby(&key_series).expect("groupby").rank("average", true, "keep").expect("rank"); })
        }
        ("groupby", "groupby_sem_str") => {
            let mut kb = Vec::with_capacity(rows * 5);
            let mut ko = Vec::with_capacity(rows + 1);
            ko.push(0usize);
            for &v in raw[0].iter() { kb.extend_from_slice(format!("g{:04}", (v as i64).rem_euclid(1000)).as_bytes()); ko.push(kb.len()); }
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new("key".to_string(), index.clone(), Column::from_utf8_contiguous(kb, ko)).expect("key");
            let val_series = Series::new("col_1".to_string(), index, Column::from_f64_values(raw[1].clone())).expect("val");
            time_us(|| { let _ = val_series.groupby(&key_series).expect("groupby").sem().expect("sem"); })
        }
        ("groupby", "groupby_skew_str") => {
            let mut kb = Vec::with_capacity(rows * 5);
            let mut ko = Vec::with_capacity(rows + 1);
            ko.push(0usize);
            for &v in raw[0].iter() { kb.extend_from_slice(format!("g{:04}", (v as i64).rem_euclid(1000)).as_bytes()); ko.push(kb.len()); }
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new("key".to_string(), index.clone(), Column::from_utf8_contiguous(kb, ko)).expect("key");
            let val_series = Series::new("col_1".to_string(), index, Column::from_f64_values(raw[1].clone())).expect("val");
            time_us(|| { let _ = val_series.groupby(&key_series).expect("groupby").skew().expect("skew"); })
        }
        ("groupby", "groupby_nunique_str") => {
            let mut kb = Vec::with_capacity(rows * 5);
            let mut ko = Vec::with_capacity(rows + 1);
            ko.push(0usize);
            for &v in raw[0].iter() { kb.extend_from_slice(format!("g{:04}", (v as i64).rem_euclid(1000)).as_bytes()); ko.push(kb.len()); }
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new("key".to_string(), index.clone(), Column::from_utf8_contiguous(kb, ko)).expect("key");
            let val_series = Series::new("col_1".to_string(), index, Column::from_f64_values(raw[1].clone())).expect("val");
            time_us(|| { let _ = val_series.groupby(&key_series).expect("groupby").nunique().expect("nunique"); })
        }
        ("groupby", "groupby_all_str") => {
            let mut kb = Vec::with_capacity(rows * 5);
            let mut ko = Vec::with_capacity(rows + 1);
            ko.push(0usize);
            for &v in raw[0].iter() { kb.extend_from_slice(format!("g{:04}", (v as i64).rem_euclid(1000)).as_bytes()); ko.push(kb.len()); }
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new("key".to_string(), index.clone(), Column::from_utf8_contiguous(kb, ko)).expect("key");
            let val_series = Series::new("col_1".to_string(), index, Column::from_f64_values(raw[1].clone())).expect("val");
            time_us(|| { let _ = val_series.groupby(&key_series).expect("groupby").all().expect("all"); })
        }
        ("groupby", "groupby_unique_str") => {
            let mut kb = Vec::with_capacity(rows * 5);
            let mut ko = Vec::with_capacity(rows + 1);
            ko.push(0usize);
            for &v in raw[0].iter() { kb.extend_from_slice(format!("g{:04}", (v as i64).rem_euclid(1000)).as_bytes()); ko.push(kb.len()); }
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new("key".to_string(), index.clone(), Column::from_utf8_contiguous(kb, ko)).expect("key");
            let val_series = Series::new("col_1".to_string(), index, Column::from_f64_values(raw[1].clone())).expect("val");
            time_us(|| { let _ = val_series.groupby(&key_series).expect("groupby").unique().expect("unique"); })
        }
        ("groupby", "groupby_unique_i64") => {
            // Int64-VALUE variant of groupby_unique_str: str key (1000 groups),
            // i64 value column (col_1 % 50000 -> ~50 distinct per group).
            let mut kb = Vec::with_capacity(rows * 5);
            let mut ko = Vec::with_capacity(rows + 1);
            ko.push(0usize);
            for &v in raw[0].iter() { kb.extend_from_slice(format!("g{:04}", (v as i64).rem_euclid(1000)).as_bytes()); ko.push(kb.len()); }
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new("key".to_string(), index.clone(), Column::from_utf8_contiguous(kb, ko)).expect("key");
            let ivals: Vec<i64> = raw[1].iter().map(|&v| (v as i64).rem_euclid(50_000)).collect();
            let val_series = Series::new("col_1".to_string(), index, Column::from_i64_values(ivals)).expect("val");
            time_us(|| { let _ = val_series.groupby(&key_series).expect("groupby").unique().expect("unique"); })
        }
        ("groupby", "groupby_kurt_str") => {
            let mut kb = Vec::with_capacity(rows * 5);
            let mut ko = Vec::with_capacity(rows + 1);
            ko.push(0usize);
            for &v in raw[0].iter() { kb.extend_from_slice(format!("g{:04}", (v as i64).rem_euclid(1000)).as_bytes()); ko.push(kb.len()); }
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new("key".to_string(), index.clone(), Column::from_utf8_contiguous(kb, ko)).expect("key");
            let val_series = Series::new("col_1".to_string(), index, Column::from_f64_values(raw[1].clone())).expect("val");
            time_us(|| { let _ = val_series.groupby(&key_series).expect("groupby").kurt().expect("kurt"); })
        }
        ("groupby", "groupby_quantile_str") => {
            let mut kb = Vec::with_capacity(rows * 5);
            let mut ko = Vec::with_capacity(rows + 1);
            ko.push(0usize);
            for &v in raw[0].iter() { kb.extend_from_slice(format!("g{:04}", (v as i64).rem_euclid(1000)).as_bytes()); ko.push(kb.len()); }
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new("key".to_string(), index.clone(), Column::from_utf8_contiguous(kb, ko)).expect("key");
            let val_series = Series::new("col_1".to_string(), index, Column::from_f64_values(raw[1].clone())).expect("val");
            time_us(|| { let _ = val_series.groupby(&key_series).expect("groupby").quantile(0.5).expect("q"); })
        }
        ("groupby", "groupby_transform_mean_str") => {
            // String-key variant: s.groupby(str_key).transform("mean"). key =
            // "g{col_0 % 1000:04}" (~1000 distinct categorical labels).
            let mut kb = Vec::with_capacity(rows * 5);
            let mut ko = Vec::with_capacity(rows + 1);
            ko.push(0usize);
            for &v in raw[0].iter() {
                kb.extend_from_slice(format!("g{:04}", (v as i64).rem_euclid(1000)).as_bytes());
                ko.push(kb.len());
            }
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new(
                "key".to_string(),
                index.clone(),
                Column::from_utf8_contiguous(kb, ko),
            )
            .expect("key series");
            let val_series = Series::new(
                "col_1".to_string(),
                index,
                Column::from_f64_values(raw[1].clone()),
            )
            .expect("val series");
            time_us(|| {
                let _ = val_series
                    .groupby(&key_series)
                    .expect("groupby")
                    .transform("mean")
                    .expect("transform");
            })
        }
        ("groupby", "groupby_cumcount") => {
            // pandas: df.groupby("key").cumcount() — within-group 0-based row
            // number. key = (col_0 % 100).astype(int64).
            let keys: Vec<i64> = raw[0].iter().map(|&v| (v as i64).rem_euclid(100)).collect();
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new(
                "key".to_string(),
                index.clone(),
                Column::from_i64_values(keys),
            )
            .expect("key series");
            let val_series = Series::new(
                "col_1".to_string(),
                index,
                Column::from_f64_values(raw[1].clone()),
            )
            .expect("val series");
            time_us(|| {
                let _ = val_series
                    .groupby(&key_series)
                    .expect("groupby")
                    .cumcount()
                    .expect("cumcount");
            })
        }
        ("groupby", "groupby_count") => {
            // pandas: df.groupby("key")["col_1"].count() — non-null count per
            // group. key = (col_0 % 100).astype(int64).
            let keys: Vec<i64> = raw[0].iter().map(|&v| (v as i64).rem_euclid(100)).collect();
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new(
                "key".to_string(),
                index.clone(),
                Column::from_i64_values(keys),
            )
            .expect("key series");
            let val_series = Series::new(
                "col_1".to_string(),
                index,
                Column::from_f64_values(raw[1].clone()),
            )
            .expect("val series");
            time_us(|| {
                let _ = val_series
                    .groupby(&key_series)
                    .expect("groupby")
                    .count()
                    .expect("count");
            })
        }
        ("groupby", "df_groupby_str_sum") => {
            // pandas: df.groupby("key")[["v0","v1","v2"]].sum(); key=g{i%1000}
            // (1000 string groups). Exercises DataFrameGroupBy's GroupMap.
            let mut kb = Vec::new();
            let mut ko = vec![0usize];
            for i in 0..rows {
                let k = format!("g{:04}", i % 1000);
                kb.extend_from_slice(k.as_bytes());
                ko.push(kb.len());
            }
            let mut columns = BTreeMap::new();
            columns.insert("key".to_string(), Column::from_utf8_contiguous(kb, ko));
            let mut order = vec!["key".to_string()];
            for c in 0..3 {
                let n = format!("v{c}");
                columns.insert(n.clone(), Column::from_f64_values(raw[c].clone()));
                order.push(n);
            }
            let gdf = DataFrame::new_with_column_order(
                Index::new_known_unique_int64_unit_range(0, rows),
                columns,
                order,
            )
            .expect("gb frame");
            time_us(|| {
                let _ = gdf.groupby(&["key"]).expect("groupby").sum().expect("sum");
            })
        }
        ("groupby", "df_groupby_2key_sum") => {
            let mut columns = BTreeMap::new();
            let k1: Vec<i64> = (0..rows).map(|i| (i % 100) as i64).collect();
            let k2: Vec<i64> = (0..rows).map(|i| ((i / 100) % 50) as i64).collect();
            columns.insert("k1".to_string(), Column::from_i64_values(k1));
            columns.insert("k2".to_string(), Column::from_i64_values(k2));
            let mut order = vec!["k1".to_string(), "k2".to_string()];
            for c in 0..3 { let n = format!("v{c}"); columns.insert(n.clone(), Column::from_f64_values(raw[c].clone())); order.push(n); }
            let gdf = DataFrame::new_with_column_order(Index::new_known_unique_int64_unit_range(0, rows), columns, order).expect("gb frame");
            time_us(|| { let _ = gdf.groupby(&["k1","k2"]).expect("groupby").sum().expect("sum"); })
        }
        ("groupby", "df_groupby_2strkey_sum") => {
            let mut columns = BTreeMap::new();
            let mut kb1 = Vec::new(); let mut ko1 = vec![0usize];
            for i in 0..rows { let k = format!("a{:03}", i % 100); kb1.extend_from_slice(k.as_bytes()); ko1.push(kb1.len()); }
            columns.insert("k1".to_string(), Column::from_utf8_contiguous(kb1, ko1));
            let mut kb2 = Vec::new(); let mut ko2 = vec![0usize];
            for i in 0..rows { let k = format!("b{:03}", (i / 100) % 50); kb2.extend_from_slice(k.as_bytes()); ko2.push(kb2.len()); }
            columns.insert("k2".to_string(), Column::from_utf8_contiguous(kb2, ko2));
            let mut order = vec!["k1".to_string(), "k2".to_string()];
            for c in 0..3 { let n = format!("v{c}"); columns.insert(n.clone(), Column::from_f64_values(raw[c].clone())); order.push(n); }
            let gdf = DataFrame::new_with_column_order(Index::new_known_unique_int64_unit_range(0, rows), columns, order).expect("gb frame");
            time_us(|| { let _ = gdf.groupby(&["k1","k2"]).expect("groupby").sum().expect("sum"); })
        }
        ("groupby", "groupby_agg3_str") => {
            let mut kb = Vec::with_capacity(rows * 5); let mut ko = Vec::with_capacity(rows + 1); ko.push(0usize);
            for &v in raw[0].iter() { kb.extend_from_slice(format!("g{:04}", (v as i64).rem_euclid(1000)).as_bytes()); ko.push(kb.len()); }
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new("key".to_string(), index.clone(), Column::from_utf8_contiguous(kb, ko)).expect("key");
            let val_series = Series::new("col_1".to_string(), index, Column::from_f64_values(raw[1].clone())).expect("val");
            time_us(|| { let _ = val_series.groupby(&key_series).expect("groupby").agg(&["mean","std","max"]).expect("agg"); })
        }
        ("groupby", "groupby_widekey_sum") => {
            // Single WIDE-range i64 key (sparse, ~rows/2 distinct over a huge
            // span): exercises the non-dense build_groups path (dense gate needs
            // a bounded range). key = i * 0x9E3779B97F4A7C15 (wraps -> spread
            // across full i64), value = col_1. groupby(key).sum().
            let key_vals: Vec<i64> = (0..rows as i64)
                .map(|i| (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) as i64 >> 1)
                .collect();
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new(
                "key".to_string(),
                index.clone(),
                Column::from_i64_values(key_vals),
            )
            .expect("widekey key");
            let val_series = Series::new(
                "col_1".to_string(),
                index,
                Column::from_f64_values(raw[1].clone()),
            )
            .expect("widekey val");
            time_us(|| {
                let _ = val_series.groupby(&key_series).expect("groupby").sum().expect("sum");
            })
        }
        ("groupby", "df_groupby_widekey_sum") => {
            // DataFrameGroupBy sibling of groupby_widekey_sum: single WIDE-range
            // i64 key col (~rows distinct) + 3 f64 value cols, df.groupby(key).sum().
            let mut columns = BTreeMap::new();
            let key_vals: Vec<i64> = (0..rows as i64)
                .map(|i| (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) as i64 >> 1)
                .collect();
            columns.insert("key".to_string(), Column::from_i64_values(key_vals));
            let mut order = vec!["key".to_string()];
            for c in 0..3 {
                let n = format!("v{c}");
                columns.insert(n.clone(), Column::from_f64_values(raw[c].clone()));
                order.push(n);
            }
            let gdf = DataFrame::new_with_column_order(
                Index::new_known_unique_int64_unit_range(0, rows),
                columns,
                order,
            )
            .expect("widekey gb frame");
            time_us(|| {
                let _ = gdf.groupby(&["key"]).expect("groupby").sum().expect("sum");
            })
        }
        ("groupby", "df_groupby_int_var") => {
            let mut columns = BTreeMap::new();
            let key_vals: Vec<i64> = (0..rows).map(|i| (i % 1000) as i64).collect();
            columns.insert("key".to_string(), Column::from_i64_values(key_vals));
            let mut order = vec!["key".to_string()];
            for c in 0..3 { let n = format!("v{c}"); columns.insert(n.clone(), Column::from_f64_values(raw[c].clone())); order.push(n); }
            let gdf = DataFrame::new_with_column_order(Index::new_known_unique_int64_unit_range(0, rows), columns, order).expect("gb frame");
            time_us(|| { let _ = gdf.groupby(&["key"]).expect("groupby").var().expect("var"); })
        }
        ("groupby", "df_groupby_int_mean") => {
            let mut columns = BTreeMap::new();
            let key_vals: Vec<i64> = (0..rows).map(|i| (i % 1000) as i64).collect();
            columns.insert("key".to_string(), Column::from_i64_values(key_vals));
            let mut order = vec!["key".to_string()];
            for c in 0..3 { let n = format!("v{c}"); columns.insert(n.clone(), Column::from_f64_values(raw[c].clone())); order.push(n); }
            let gdf = DataFrame::new_with_column_order(Index::new_known_unique_int64_unit_range(0, rows), columns, order).expect("gb frame");
            time_us(|| { let _ = gdf.groupby(&["key"]).expect("groupby").mean().expect("mean"); })
        }
        ("rolling", "rolling_mean_w10") => {
            let series = df.get_column("col_0");
            time_us(|| {
                let _ = series.rolling(10, Some(10)).mean().expect("rolling mean");
            })
        }
        ("rolling", "expanding_skew") => {
            let series = df.get_column("col_0");
            time_us(|| { let _ = series.expanding(Some(1)).skew().expect("expanding skew"); })
        }
        ("rolling", "rolling_std_w50") => {
            let series = df.get_column("col_0");
            time_us(|| {
                let _ = series.rolling(50, Some(50)).std().expect("rolling std");
            })
        }
        ("rolling", "expanding_sum") => {
            let series = df.get_column("col_0");
            time_us(|| {
                let _ = series.expanding(Some(1)).sum().expect("expanding sum");
            })
        }
        ("rolling", "ewm_mean") => {
            let series = df.get_column("col_0");
            time_us(|| {
                let _ = series.ewm(Some(10.0), None).mean().expect("ewm mean");
            })
        }
        // The fp-bench frame uses a default 0..rows Int64 index (matching the
        // pandas side's set_index(range(n))), so loc/reindex labels line up.
        ("indexing", "iloc_slice") => {
            // pandas: df.iloc[n/4 : 3n/4] — a contiguous SLICE (returns a view).
            // Match the slice semantics with DataFrame::iloc_slice(start, stop)
            // (the O(1) lazy-slice path), NOT df.iloc(&positions): passing an
            // explicit 50k-element position Vec measures pandas' *list* indexer
            // (df.iloc[list(...)] is ~200x slower than the slice) against fp's
            // list API — an apples-to-oranges comparison. iloc_slice is the
            // same contiguous-range operation pandas' slice performs.
            let start = Some((rows / 4) as i64);
            let stop = Some((3 * rows / 4) as i64);
            time_us(|| {
                let _ = df.iloc_slice(start, stop).expect("iloc_slice");
            })
        }
        ("indexing", "loc_labels") => {
            // pandas: df.loc[list(range(n/4, 3n/4))]
            let labels: Vec<IndexLabel> = ((rows / 4) as i64..(3 * rows / 4) as i64)
                .map(IndexLabel::Int64)
                .collect();
            time_us(|| {
                let _ = df.loc(&labels).expect("loc");
            })
        }
        ("io", "csv_read") => {
            // pandas: df.to_csv(file, index=False) [setup]; time pd.read_csv(file).
            // FP: serialize once (setup), time read_csv_str of the same text.
            let csv = fp_io::write_csv_string(&df).expect("csv serialize");
            time_us(|| {
                let _ = fp_io::read_csv_str(&csv).expect("read_csv");
            })
        }
        ("io", "csv_write") => {
            // pandas: time df.to_csv(file, index=False). FP: time write_csv_string.
            time_us(|| {
                let _ = fp_io::write_csv_string(&df).expect("write_csv");
            })
        }
        ("indexing", "reindex") => {
            // pandas: df.reindex(Index(range(0, n*2, 2)))
            let new_labels: Vec<IndexLabel> = (0..(rows * 2) as i64)
                .step_by(2)
                .map(IndexLabel::Int64)
                .collect();
            time_us(|| {
                let _ = df.reindex(new_labels.clone()).expect("reindex");
            })
        }
        ("indexing", "range_index_take_arithmetic") => {
            // pandas: pd.RangeIndex(10, 10 + 3*n, 3).take(arithmetic_positions).
            // Batch repeated takes inside the timed unit so FP's lazy affine
            // output path stays above timer noise without charging setup.
            let range = RangeIndex::new(10, 10 + rows as i64 * 3, 3).expect("range index fixture");
            let positions = arithmetic_take_positions(rows);
            time_us(|| {
                for _ in 0..TAKE_BATCH {
                    black_box(
                        range
                            .take(black_box(positions.as_slice()))
                            .expect("range take"),
                    );
                }
            })
        }
        ("indexing", "affine_index_take_arithmetic") => {
            // pandas: pd.Index(np.arange(10, 10 + 3*n, 3)).take(arithmetic_positions).
            let index =
                Index::new_known_unique_int64_affine_range(10, 3, rows).expect("affine index");
            let positions = arithmetic_take_positions(rows);
            time_us(|| {
                for _ in 0..TAKE_BATCH {
                    black_box(index.take(black_box(positions.as_slice())));
                }
            })
        }
        // pandas: left.merge(right, on="key", how=inner|left|outer). The frame
        // built above is unused for joins; build the two merge inputs sized to
        // `rows` (outside the timed window) instead.
        ("joins", "join_inner" | "join_left" | "join_outer") => {
            let (left, right) = build_join_frames(rows);
            let join_type = match workload {
                "join_inner" => JoinType::Inner,
                "join_left" => JoinType::Left,
                _ => JoinType::Outer,
            };
            time_us(|| {
                let _ = merge_dataframes_on_with(&left, &right, &["key"], &["key"], join_type)
                    .expect("merge");
            })
        }
        ("joins", "join_inner_str") => {
            // String-key inner merge: left key = "k{i:08}" (unique), right key =
            // "k{2i:08}" — mirrors the int64 join shape (~n/2 inner matches) but
            // exercises the Utf8 key path. pandas: left.merge(right, on="key").
            let build = |stride: usize, valname: &str| -> DataFrame {
                let mut bytes = Vec::with_capacity(rows * 9);
                let mut off = Vec::with_capacity(rows + 1);
                off.push(0usize);
                for i in 0..rows {
                    bytes.extend_from_slice(format!("k{:08}", i * stride).as_bytes());
                    off.push(bytes.len());
                }
                let index = Index::new_known_unique_int64_unit_range(0, rows);
                let mut cols = BTreeMap::new();
                cols.insert("key".to_string(), Column::from_utf8_contiguous(bytes, off));
                cols.insert(
                    valname.to_string(),
                    Column::from_f64_values((0..rows).map(|i| i as f64).collect()),
                );
                DataFrame::new_with_column_order(
                    index,
                    cols,
                    vec!["key".to_string(), valname.to_string()],
                )
                .expect("fp-bench str join frame")
            };
            let left = build(1, "left_val");
            let right = build(2, "right_val");
            time_us(|| {
                let _ =
                    merge_dataframes_on_with(&left, &right, &["key"], &["key"], JoinType::Inner)
                        .expect("merge");
            })
        }
        // String-column ops (the rest of the matrix is numeric-only). pandas:
        // f.sort_values("name") / f["key"].value_counts() / f.groupby("key")
        // ["val"].sum(). The numeric `df` built above is unused here.
        ("strings", "str_len" | "str_upper" | "str_contains" | "str_startswith") => {
            let frame = build_str_frame(rows);
            let series = frame.get_column("name");
            match workload {
                "str_len" => time_us(|| {
                    let _ = series.str().len().expect("str len");
                }),
                "str_upper" => time_us(|| {
                    let _ = series.str().upper().expect("str upper");
                }),
                "str_contains" => time_us(|| {
                    let _ = series.str().contains("5").expect("str contains");
                }),
                _ => time_us(|| {
                    let _ = series.str().startswith("item").expect("str startswith");
                }),
            }
        }
        // apply_str-backed transforms (zfill/pad/repeat): output Utf8 columns.
        ("strings", "str_zfill" | "str_pad" | "str_repeat") => {
            let frame = build_str_frame(rows);
            let series = frame.get_column("name");
            match workload {
                "str_zfill" => time_us(|| {
                    let _ = series.str().zfill(20).expect("str zfill");
                }),
                "str_pad" => time_us(|| {
                    let _ = series.str().pad(20, "left", ' ').expect("str pad");
                }),
                _ => time_us(|| {
                    let _ = series.str().repeat(2).expect("str repeat");
                }),
            }
        }
        ("strings", "str_sort" | "str_value_counts" | "str_groupby_sum") => {
            let frame = build_str_frame(rows);
            match workload {
                "str_sort" => time_us(|| {
                    let _ = frame.sort_values("name", true).expect("str sort");
                }),
                "str_value_counts" => {
                    let series = frame.get_column("key");
                    time_us(|| {
                        let _ = series.value_counts().expect("str value_counts");
                    })
                }
                _ => {
                    // pandas: f.groupby("key")["val"].sum() — sums ONLY val.
                    // Drop the unrelated "name" column (a ~1M-unique string)
                    // first so fp's groupby(key).sum() likewise aggregates only
                    // val, instead of also concatenating the string column per
                    // group (which made this workload look ~2x slower).
                    let gframe = frame.drop_columns(&["name"]).expect("drop name");
                    time_us(|| {
                        let _ = gframe
                            .groupby(&["key"])
                            .expect("groupby")
                            .sum()
                            .expect("sum");
                    })
                }
            }
        }
        // df.dot GEMM (br no-gaps flagship): square (dim x dim).(dim x dim) where
        // dim = isqrt(rows). pandas df.dot delegates to numpy/OpenBLAS; fp uses
        // its own safe-Rust kernel.
        ("linalg", "df_dot") => {
            let dim = (rows as f64).sqrt() as usize;
            let frame = build_square_f64_frame(dim);
            time_us(|| {
                let _ = frame.dot(&frame).expect("dot");
            })
        }
        // to_datetime parse throughput: `rows` ISO date strings (2020-01-DD,
        // ~28 distinct), same strings on both engines. pandas: pd.to_datetime(s).
        ("datetime", "to_datetime") => {
            let mut date_bytes: Vec<u8> = Vec::new();
            let mut date_off: Vec<usize> = Vec::with_capacity(rows + 1);
            date_off.push(0);
            for i in 0..rows {
                let s = format!("2020-01-{:02}", i % 28 + 1);
                date_bytes.extend_from_slice(s.as_bytes());
                date_off.push(date_bytes.len());
            }
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let series = Series::new(
                "d".to_string(),
                index,
                Column::from_utf8_contiguous(date_bytes, date_off),
            )
            .expect("date series");
            time_us(|| {
                let _ = to_datetime(&series).expect("to_datetime");
            })
        }
        ("datetime", "resample_mean") => {
            // s.resample("M").mean(): `rows` daily points from 2000-01-01,
            // datetime index -> ~rows/30 month buckets.
            let base: i64 = 946_684_800_000_000_000;
            // hourly so 1M points stay within datetime64[ns] range (<=2262).
            let nanos: Vec<i64> = (0..rows as i64).map(|i| base + i * 3_600_000_000_000).collect();
            let vals = Column::from_f64_values((0..rows).map(|i| i as f64).collect());
            let series = Series::new("s", Index::from_datetime64(nanos), vals)
                .expect("resample series");
            time_us(|| {
                let _ = series.resample("M").mean().expect("resample mean");
            })
        }
        ("datetime", "resample_hourly") => {
            // s.resample("h").mean(): `rows` minutely points -> hourly bins
            // (60 rows/bin), exercises the sub-daily ns-bucketing path.
            let base: i64 = 946_684_800_000_000_000;
            let nanos: Vec<i64> = (0..rows as i64).map(|i| base + i * 60_000_000_000).collect();
            let vals = Column::from_f64_values((0..rows).map(|i| i as f64).collect());
            let series = Series::new("s", Index::from_datetime64(nanos), vals)
                .expect("resample series");
            time_us(|| {
                let _ = series.resample("h").mean().expect("resample hourly");
            })
        }
        ("datetime", "resample_std") => {
            let base: i64 = 946_684_800_000_000_000;
            let nanos: Vec<i64> = (0..rows as i64).map(|i| base + i * 3_600_000_000_000).collect();
            let vals = Column::from_f64_values((0..rows).map(|i| i as f64).collect());
            let series = Series::new("s", Index::from_datetime64(nanos), vals).expect("resample series");
            time_us(|| { let _ = series.resample("M").std().expect("resample std"); })
        }
        ("datetime", "resample_median") => {
            let base: i64 = 946_684_800_000_000_000;
            let nanos: Vec<i64> = (0..rows as i64).map(|i| base + i * 3_600_000_000_000).collect();
            let vals = Column::from_f64_values((0..rows).map(|i| i as f64).collect());
            let series = Series::new("s", Index::from_datetime64(nanos), vals).expect("resample series");
            time_us(|| { let _ = series.resample("M").median().expect("resample median"); })
        }
        ("datetime", "resample_agg3") => {
            let base: i64 = 946_684_800_000_000_000;
            let nanos: Vec<i64> = (0..rows as i64).map(|i| base + i * 3_600_000_000_000).collect();
            let vals = Column::from_f64_values((0..rows).map(|i| i as f64).collect());
            let series = Series::new("s", Index::from_datetime64(nanos), vals).expect("resample series");
            time_us(|| { let _ = series.resample("M").agg(&["mean","std","max"]).expect("resample agg"); })
        }
        ("datetime", "resample_sum") => {
            let base: i64 = 946_684_800_000_000_000;
            let nanos: Vec<i64> = (0..rows as i64).map(|i| base + i * 3_600_000_000_000).collect();
            let vals = Column::from_f64_values((0..rows).map(|i| i as f64).collect());
            let series = Series::new("s", Index::from_datetime64(nanos), vals).expect("resample series");
            time_us(|| { let _ = series.resample("M").sum().expect("resample sum"); })
        }
        ("datetime", "resample_max") => {
            let base: i64 = 946_684_800_000_000_000;
            let nanos: Vec<i64> = (0..rows as i64).map(|i| base + i * 3_600_000_000_000).collect();
            let vals = Column::from_f64_values((0..rows).map(|i| i as f64).collect());
            let series = Series::new("s", Index::from_datetime64(nanos), vals).expect("resample series");
            time_us(|| { let _ = series.resample("M").max().expect("resample max"); })
        }
        ("dataframe_ops", "qcut_bins") => {
            // pandas: pd.qcut(s, 10) — quantile-bin a Float64 series into 10 bins.
            let series = df.get_column("col_0");
            time_us(|| {
                let _ = fp_frame::qcut(&series, 10).expect("qcut");
            })
        }
        ("dataframe_ops", "wide_to_long") => {
            // pandas: pd.wide_to_long(df, ["A","B"], i="id", j="year", sep="_",
            // suffix=r"\\d+"); m=rows/2 rows x 2 suffixes -> ~rows long rows.
            let m = (rows / 2).max(1);
            let mut columns = BTreeMap::new();
            columns.insert("id".to_string(), Column::from_i64_values((0..m as i64).collect()));
            let mut order = vec!["id".to_string()];
            for stub in ["A", "B"] {
                for suf in ["2000", "2001"] {
                    let name = format!("{stub}_{suf}");
                    columns.insert(name.clone(), Column::from_f64_values((0..m).map(|i| i as f64).collect()));
                    order.push(name);
                }
            }
            let wdf = DataFrame::new_with_column_order(
                Index::new_known_unique_int64_unit_range(0, m), columns, order).expect("w2l frame");
            time_us(|| {
                let _ = fp_frame::wide_to_long(&wdf, &["A", "B"], &["id"], "year", "_", r"\d+").expect("wide_to_long");
            })
        }
        ("dataframe_ops", "str_split_expand") => {
            // pandas: s.str.split(",", expand=True); each cell "a{i},b{i},c{i}".
            let mut bytes = Vec::new();
            let mut off = vec![0usize];
            for i in 0..rows {
                let cell = format!("a{},b{},c{}", i % 97, i % 89, i % 83);
                bytes.extend_from_slice(cell.as_bytes());
                off.push(bytes.len());
            }
            let series = Series::new(
                "s",
                Index::new_known_unique_int64_unit_range(0, rows),
                Column::from_utf8_contiguous(bytes, off),
            )
            .expect("split series");
            time_us(|| {
                let _ = series.str().split_df_n(",", None).expect("split");
            })
        }
        ("io", "json_read_records") => {
            // pandas: pd.read_json(json, orient="records"); parse a records JSON.
            let json = df.to_json("records").expect("to_json setup");
            time_us(|| {
                let _ = fp_io::read_json_str(&json, fp_io::JsonOrient::Records).expect("read_json");
            })
        }
        ("io", "json_write_records") => {
            // pandas: df.to_json(orient="records"); 10-col f64 frame.
            time_us(|| {
                let _ = df.to_json("records").expect("to_json");
            })
        }
        ("io", "json_write_columns") => {
            time_us(|| {
                let _ = df.to_json("columns").expect("to_json");
            })
        }
        ("io", "json_write_index") => {
            time_us(|| {
                let _ = df.to_json("index").expect("to_json");
            })
        }
        ("io", "json_write_split") => {
            time_us(|| {
                let _ = df.to_json("split").expect("to_json");
            })
        }
        ("io", "json_write_values") => {
            time_us(|| {
                let _ = df.to_json("values").expect("to_json");
            })
        }
        ("dataframe_ops", "df_to_dict_records") => {
            time_us(|| {
                let _ = df.to_dict("records").expect("to_dict");
            })
        }
        ("dataframe_ops", "df_to_dict_dict") => {
            time_us(|| {
                let _ = df.to_dict("dict").expect("to_dict");
            })
        }
        ("dataframe_ops", "df_to_records") => {
            time_us(|| {
                let _ = df.to_records();
            })
        }
        ("dataframe_ops", "df_apply_row") => {
            time_us(|| {
                let _ = df
                    .apply_fn(
                        |row| {
                            let s: f64 = row
                                .iter()
                                .filter_map(|v| match v {
                                    fp_types::Scalar::Float64(f) => Some(*f),
                                    _ => None,
                                })
                                .sum();
                            fp_types::Scalar::Float64(s)
                        },
                        1,
                    )
                    .expect("apply");
            })
        }
        ("dataframe_ops", "cut_explicit") => {
            // pandas: pd.cut(s, bins=[-1,1e5,...,1.1e6]) — explicit edges spanning
            // the [0,1e6] data (all in-range -> all-valid). Exercises cut_bins.
            let series = df.get_column("col_0");
            let mut edges: Vec<fp_types::Scalar> = vec![fp_types::Scalar::Float64(-1.0)];
            for i in 1..=9 {
                edges.push(fp_types::Scalar::Float64(i as f64 * 1e5));
            }
            edges.push(fp_types::Scalar::Float64(1.1e6));
            time_us(|| {
                let _ = fp_frame::cut_bins(&series, &edges, true, None, false).expect("cut_bins");
            })
        }
        ("dataframe_ops", "cut_bins") => {
            // pandas: pd.cut(s, 10) — bin a Float64 series into 10 bins.
            let series = df.get_column("col_0");
            time_us(|| {
                let _ = fp_frame::cut(&series, 10).expect("cut");
            })
        }
        ("datetime", "resample_daily") => {
            // s.resample("D").mean(): `rows` hourly points -> daily bins
            // (24 rows/bin), exercises the daily-contiguous bucketing path.
            let base: i64 = 946_684_800_000_000_000;
            let nanos: Vec<i64> = (0..rows as i64).map(|i| base + i * 3_600_000_000_000).collect();
            let vals = Column::from_f64_values((0..rows).map(|i| i as f64).collect());
            let series = Series::new("s", Index::from_datetime64(nanos), vals)
                .expect("resample series");
            time_us(|| {
                let _ = series.resample("D").mean().expect("resample daily");
            })
        }
        ("datetime", "resample_2d" | "resample_bday" | "resample_w" | "resample_q" | "resample_y") => {
            // hourly points -> 2D / B / W / Q / Y bins.
            let base: i64 = 946_684_800_000_000_000;
            let nanos: Vec<i64> = (0..rows as i64).map(|i| base + i * 3_600_000_000_000).collect();
            let vals = Column::from_f64_values((0..rows).map(|i| i as f64).collect());
            let series = Series::new("s", Index::from_datetime64(nanos), vals)
                .expect("resample series");
            let freq = match workload {
                "resample_2d" => "2D",
                "resample_bday" => "B",
                "resample_w" => "W",
                "resample_q" => "Q",
                _ => "Y",
            };
            time_us(|| {
                let _ = series.resample(freq).mean().expect("resample");
            })
        }
        // dt.floor("D") over `rows` Datetime64 nanos at 37s intervals from
        // 2000-01-01. pandas: s.dt.floor("D").
        ("datetime", "dt_floor") => {
            let base: i64 = 946_684_800_000_000_000; // 2000-01-01 00:00:00 UTC, ns
            let nanos: Vec<i64> = (0..rows as i64)
                .map(|i| base + i * 37_000_000_000)
                .collect();
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let series = Series::new(
                "d".to_string(),
                index,
                Column::from_datetime64_values(nanos),
            )
            .expect("dt series");
            time_us(|| {
                let _ = series.dt().floor("D").expect("dt floor");
            })
        }
        ("datetime", "dt_dayofyear") => {
            let base: i64 = 946_684_800_000_000_000;
            let nanos: Vec<i64> = (0..rows as i64)
                .map(|i| base + i * 37_000_000_000)
                .collect();
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let series = Series::new(
                "d".to_string(),
                index,
                Column::from_datetime64_values(nanos),
            )
            .expect("dt series");
            time_us(|| {
                let _ = series.dt().dayofyear().expect("dt dayofyear");
            })
        }
        ("datetime", "dt_strftime" | "dt_date" | "dt_time" | "dt_day_name" | "dt_month_name") => {
            let base: i64 = 946_684_800_000_000_000;
            // 1 day + 37 s per row so BOTH the date and the time-of-day vary.
            let nanos: Vec<i64> = (0..rows as i64)
                .map(|i| base + i * 86_437_000_000_000)
                .collect();
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let series = Series::new(
                "d".to_string(),
                index,
                Column::from_datetime64_values(nanos),
            )
            .expect("dt series");
            match workload {
                "dt_strftime" => time_us(|| {
                    let _ = series.dt().strftime("%Y-%m-%d").expect("strftime");
                }),
                "dt_time" => time_us(|| {
                    let _ = series.dt().time().expect("time");
                }),
                "dt_day_name" => time_us(|| {
                    let _ = series.dt().day_name().expect("day_name");
                }),
                "dt_month_name" => time_us(|| {
                    let _ = series.dt().month_name().expect("month_name");
                }),
                _ => time_us(|| {
                    let _ = series.dt().date().expect("date");
                }),
            }
        }
        ("datetime", "dt_hour" | "dt_minute" | "dt_quarter") => {
            let base: i64 = 946_684_800_000_000_000;
            let nanos: Vec<i64> = (0..rows as i64)
                .map(|i| base + i * 37_000_000_000)
                .collect();
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let series = Series::new(
                "d".to_string(),
                index,
                Column::from_datetime64_values(nanos),
            )
            .expect("dt series");
            match workload {
                "dt_hour" => time_us(|| {
                    let _ = series.dt().hour().expect("dt hour");
                }),
                "dt_minute" => time_us(|| {
                    let _ = series.dt().minute().expect("dt minute");
                }),
                _ => time_us(|| {
                    let _ = series.dt().quarter().expect("dt quarter");
                }),
            }
        }
        ("datetime", "dt_year" | "dt_month" | "dt_dayofweek") => {
            let base: i64 = 946_684_800_000_000_000;
            let nanos: Vec<i64> = (0..rows as i64)
                .map(|i| base + i * 37_000_000_000)
                .collect();
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let series = Series::new(
                "d".to_string(),
                index,
                Column::from_datetime64_values(nanos),
            )
            .expect("dt series");
            match workload {
                "dt_year" => time_us(|| {
                    let _ = series.dt().year().expect("dt year");
                }),
                "dt_month" => time_us(|| {
                    let _ = series.dt().month().expect("dt month");
                }),
                _ => time_us(|| {
                    let _ = series.dt().dayofweek().expect("dt dayofweek");
                }),
            }
        }
        _ => return None,
    };
    Some(times)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let category = arg(&args, "--category").unwrap_or("dataframe_ops");
    let workload = arg(&args, "--workload").unwrap_or("sort_single");
    let size = arg(&args, "--size").unwrap_or("100k");
    let dtype = arg(&args, "--dtype").unwrap_or("float64");

    match run(category, workload, size, dtype) {
        Some(times) => {
            let body: Vec<String> = times.iter().map(|t| format!("{t}")).collect();
            println!("{{\"times_us\": [{}]}}", body.join(", "));
        }
        None => {
            eprintln!(
                "fp-bench: unsupported {category}/{workload} (v1 coverage: dataframe_ops, groupby, rolling)"
            );
            std::process::exit(2);
        }
    }
}
