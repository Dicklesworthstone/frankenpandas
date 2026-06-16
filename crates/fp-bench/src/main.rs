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

use std::{collections::BTreeMap, time::Instant};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{DuplicateKeep, Index, IndexLabel};
use fp_join::{JoinType, merge_dataframes_on_with};

const WARMUP: usize = 3;
const ITERS: usize = 25;

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
        ("dataframe_ops", "value_counts") => {
            let series = df.get_column("col_0");
            time_us(|| {
                let _ = series.value_counts().expect("value_counts");
            })
        }
        ("dataframe_ops", "cumsum") => time_us(|| {
            let _ = df.cumsum().expect("cumsum");
        }),
        ("groupby", "groupby_sum_int64" | "groupby_mean_float64" | "groupby_agg_multi") => {
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
        ("rolling", "rolling_mean_w10") => {
            let series = df.get_column("col_0");
            time_us(|| {
                let _ = series.rolling(10, Some(10)).mean().expect("rolling mean");
            })
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
        // String-column ops (the rest of the matrix is numeric-only). pandas:
        // f.sort_values("name") / f["key"].value_counts() / f.groupby("key")
        // ["val"].sum(). The numeric `df` built above is unused here.
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
