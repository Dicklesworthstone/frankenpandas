//! Profiling-only harness (measurement, not optimization).
//!
//! Drives a single hot DataFrame operation in a tight loop so a sampling
//! profiler (samply / perf / cargo-flamegraph) can attribute CPU cost to the
//! responsible functions. Data shapes are kept identical to the `vs_pandas`
//! criterion bench so the flamegraph corresponds to the recorded baseline.
//!
//! Build (profilable) and run:
//!   RUSTFLAGS="-C force-frame-pointers=yes" \
//!     cargo build -p fp-conformance --profile release-perf --example perf_profile
//!   samply record ./target/release-perf/examples/perf_profile drop_duplicates 100000 200
//!
//! Args: <scenario> <n_rows> <iterations>
//!   scenario ∈ { drop_duplicates, sort_single, str_sort, str_groupby_sum,
//!   str_groupby_mean, str_groupby_count, str_groupby_min, str_groupby_max,
//!   str_groupby_var, str_groupby_std, str_groupby_first, str_groupby_last,
//!   str_groupby_prod, str_groupby_median, str_series_sort, str_sort_chain,
//!   filter_bool, inner_join, series_add, series_add_same, series_add_align,
//!   csv_read, csv_read_options, csv_read_no_na_filter }

use std::{collections::BTreeMap, fmt::Write as _, time::Instant};

use fp_columnar::Column;
use fp_frame::{DataFrame, Series};
use fp_index::{DuplicateKeep, Index, IndexLabel};
use fp_io::{CsvReadOptions, read_csv_str, read_csv_with_options};
use fp_join::{AsofDirection, JoinType, merge_asof, merge_dataframes};
use fp_runtime::{EvidenceLedger, RuntimePolicy};
use fp_types::Scalar;

fn lower_hex_digit(nibble: usize) -> u8 {
    match nibble {
        0..=9 => b'0' + nibble as u8,
        10..=15 => b'a' + (nibble as u8 - 10),
        _ => b'?',
    }
}

fn push_id_lower_hex_8(bytes: &mut Vec<u8>, value: usize) {
    bytes.extend_from_slice(b"id_");

    let digits = ((usize::BITS - value.leading_zeros()).div_ceil(4)).max(1) as usize;
    for _ in digits..8 {
        bytes.push(b'0');
    }
    for shift in (0..digits).rev() {
        bytes.push(lower_hex_digit((value >> (shift * 4)) & 0x0f));
    }
}

/// Two Float64 Series whose Int64 indexes overlap but are shifted by one
/// (left: 0..n, right: 1..=n), so `a + b` exercises the AACE outer-union
/// alignment path with n-1 matched labels and two unmatched endpoints —
/// matches br-frankenpandas-b75cc's series_add outer-alignment scenario.
fn build_series_pair(n: usize) -> (Series, Series) {
    let left_labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let right_labels: Vec<IndexLabel> = (1..=n as i64).map(IndexLabel::Int64).collect();
    let left_vals: Vec<Scalar> = (0..n).map(|i| Scalar::Float64(i as f64)).collect();
    let right_vals: Vec<Scalar> = (0..n).map(|i| Scalar::Float64(i as f64 * 2.0)).collect();
    let left = Series::from_values("l", left_labels, left_vals).expect("left series");
    let right = Series::from_values("r", right_labels, right_vals).expect("right series");
    (left, right)
}

/// Deterministic ~24-char Utf8 Series with ~10% rows containing "needle"
/// Contiguous-Utf8 string Series with `cardinality` distinct ~12-byte values
/// (moderately repeated), for the value_counts byte-span tally benchmark
/// (br-frankenpandas-vcstr). Backed by one byte buffer + offsets so
/// `as_utf8_contiguous` returns Some (the dense fast path).
fn build_str_vc_series(n: usize, cardinality: usize) -> Series {
    let cardinality = cardinality.max(1);
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let mut bytes = Vec::with_capacity(n * 12);
    let mut offsets = Vec::with_capacity(n + 1);
    offsets.push(0);
    let mut key = String::with_capacity(16);
    for row in 0..n {
        let mixed = (row as u64)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .rotate_left(17)
            ^ (row as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        let id = mixed % cardinality as u64;
        key.clear();
        write!(&mut key, "val_{id:06x}").expect("writing to a String cannot fail");
        bytes.extend_from_slice(key.as_bytes());
        offsets.push(bytes.len());
    }
    let index = Index::new(labels);
    let column = Column::from_utf8_contiguous(bytes, offsets);
    Series::new("s", index, column).expect("str vc series")
}

/// mid-string — the canonical str.contains workload for the SIMD string-scan
/// campaign (every row is a real heap String, exercising the AoS wall).
fn build_str_series(n: usize) -> Series {
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let values: Vec<Scalar> = (0..n)
        .map(|i| {
            let h = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 33;
            if i % 10 == 3 {
                Scalar::Utf8(format!("prefix_{h:08x}_needle_{:04}", i % 7919))
            } else {
                Scalar::Utf8(format!("prefix_{h:08x}_filler_{:04}", i % 7919))
            }
        })
        .collect();
    Series::from_values("s", labels, values).expect("str series")
}

/// Series of Float64 values with moderate cardinality (so rank tie-groups
/// actually fire), for the radix-rank benchmark (br-frankenpandas-ruvoo).
fn build_rank_series(n: usize) -> Series {
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let values: Vec<Scalar> = (0..n)
        .map(|i| {
            let mixed = (i as u64)
                .wrapping_mul(0x9E37_79B9_7F4A_7C15)
                .rotate_left(29)
                ^ (i as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            Scalar::Float64((mixed % 50_000) as f64 * 0.5)
        })
        .collect();
    Series::from_values("s", labels, values).expect("rank series")
}

/// Series with a deterministically-shuffled all-Int64 index (with duplicate
/// labels) — for the radix Series.sort_index benchmark (br-frankenpandas-d9joc).
fn build_series_sortindex(n: usize) -> Series {
    let labels: Vec<IndexLabel> = (0..n)
        .map(|i| {
            let mixed = (i as u64)
                .wrapping_mul(0x9E37_79B9_7F4A_7C15)
                .rotate_left(23)
                ^ (i as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            IndexLabel::Int64((mixed % (n as u64 * 4)) as i64)
        })
        .collect();
    let values: Vec<Scalar> = (0..n).map(|i| Scalar::Float64(i as f64 * 0.1)).collect();
    Series::from_values("s", labels, values).expect("series sortindex")
}

fn build_series_pair_same(n: usize) -> (Series, Series) {
    // Identical indexes => alignment is gap-free and all-valid, exercising the
    // typed-output fast path in Column::aligned_binary_f64 (the common
    // `df['a'] + df['b']` shape where both columns share the frame index).
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let left_vals: Vec<Scalar> = (0..n).map(|i| Scalar::Float64(i as f64)).collect();
    let right_vals: Vec<Scalar> = (0..n).map(|i| Scalar::Float64(i as f64 * 2.0)).collect();
    let left = Series::from_values("l", labels.clone(), left_vals).expect("left series");
    let right = Series::from_values("r", labels, right_vals).expect("right series");
    (left, right)
}

fn build_groupby_frame(n: usize, num_groups: usize) -> DataFrame {
    let keys: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Int64((i % num_groups) as i64))
        .collect();
    let values: Vec<Scalar> = (0..n).map(|i| Scalar::Float64(i as f64 * 0.1)).collect();
    let labels: Vec<IndexLabel> = (0..n).map(|i| IndexLabel::Int64(i as i64)).collect();
    let index = Index::new(labels);
    let key_column = fp_columnar::Column::from_values(keys).expect("key column");
    let value_column = fp_columnar::Column::from_values(values).expect("value column");
    let mut columns = BTreeMap::new();
    columns.insert("k".to_string(), key_column);
    columns.insert("v".to_string(), value_column);
    let column_order = vec!["k".to_string(), "v".to_string()];
    DataFrame::new_with_column_order(index, columns, column_order).expect("frame")
}

/// Typed-backed groupby frame for the dense transform benchmark
/// (br-frankenpandas-8kags): Int64 key (from_i64_values, so as_i64_slice fires)
/// of bounded cardinality + Float64 value columns (from_f64_values). Mirrors the
/// shape numeric data takes after a typed read/construction (where the dense
/// groupby paths apply), unlike build_groupby_frame's Scalar-backed columns.
/// (value Series, key Series) for a SeriesGroupBy cum* benchmark
/// (br-frankenpandas-gbcum): all-valid Float64 values + bounded Int64 keys, so
/// the dense-gid typed cum path applies.
fn build_groupby_cum_pair(n: usize, num_groups: usize) -> (Series, Series) {
    let labels: Vec<IndexLabel> = (0..n).map(|i| IndexLabel::Int64(i as i64)).collect();
    let keys: Vec<i64> = (0..n).map(|i| (i % num_groups) as i64).collect();
    let vals: Vec<f64> = (0..n).map(|i| (i.wrapping_mul(37) % 9973) as f64 * 0.25).collect();
    let value = Series::new(
        "v".to_string(),
        Index::new(labels.clone()),
        Column::from_f64_values(vals),
    )
    .expect("value series");
    let key = Series::new(
        "k".to_string(),
        Index::new(labels),
        Column::from_i64_values(keys),
    )
    .expect("key series");
    (value, key)
}

/// Like `build_transform_frame` but with all-valid bounded-Int64 value columns
/// (so groupby rank hits the per-group counting-sort histogram, pd7ie). Values
/// are `row % 9973` (bounded, with ties) to exercise tie handling.
fn build_transform_frame_i64(n: usize, num_groups: usize, ncols: usize) -> DataFrame {
    let labels: Vec<IndexLabel> = (0..n).map(|i| IndexLabel::Int64(i as i64)).collect();
    let index = Index::new(labels);
    let keys: Vec<i64> = (0..n).map(|i| (i % num_groups) as i64).collect();
    let mut columns = BTreeMap::new();
    let mut column_order = vec!["k".to_string()];
    columns.insert("k".to_string(), Column::from_i64_values(keys));
    for c in 0..ncols {
        let name = format!("v{c}");
        let vals: Vec<i64> = (0..n)
            .map(|i| (i.wrapping_mul(c + 1) % 9973) as i64)
            .collect();
        columns.insert(name.clone(), Column::from_i64_values(vals));
        column_order.push(name);
    }
    DataFrame::new_with_column_order(index, columns, column_order).expect("transform frame i64")
}

fn build_transform_frame(n: usize, num_groups: usize, ncols: usize) -> DataFrame {
    let labels: Vec<IndexLabel> = (0..n).map(|i| IndexLabel::Int64(i as i64)).collect();
    let index = Index::new(labels);
    let keys: Vec<i64> = (0..n).map(|i| (i % num_groups) as i64).collect();
    let mut columns = BTreeMap::new();
    let mut column_order = vec!["k".to_string()];
    columns.insert("k".to_string(), Column::from_i64_values(keys));
    for c in 0..ncols {
        let name = format!("v{c}");
        let vals: Vec<f64> = (0..n)
            .map(|i| (i.wrapping_mul(c + 1) % 9973) as f64 * 0.5)
            .collect();
        columns.insert(name.clone(), Column::from_f64_values(vals));
        column_order.push(name);
    }
    DataFrame::new_with_column_order(index, columns, column_order).expect("transform frame")
}

/// String-keyed frame whose grouping/sort key is stored as one contiguous
/// Utf8 byte buffer plus offsets. Keys are ~26 bytes, moderately repeated,
/// and row order is deterministic but not sorted.
/// Frame with a contiguous-Utf8 `id` key (cardinality `card`) + one Float64
/// value column, for string-key inner-join benchmarks. With card == n the keys
/// are ~unique so the join is ~1:1 (output ~= n), keeping the cost on the
/// build+probe (br-frankenpandas-i388q) rather than a fanout output.
fn build_str_join_frame(value_name: &str, n: usize, card: usize, key_start: usize) -> DataFrame {
    let card = card.max(1);
    let labels: Vec<IndexLabel> = (0..n).map(|i| IndexLabel::Int64(i as i64)).collect();
    let index = Index::new(labels);
    let mut bytes = Vec::with_capacity(n * 16);
    let mut offsets = Vec::with_capacity(n + 1);
    offsets.push(0);
    for row in 0..n {
        push_id_lower_hex_8(&mut bytes, (row % card) + key_start);
        offsets.push(bytes.len());
    }
    let values: Vec<f64> = (0..n)
        .map(|row| ((row as u64).wrapping_mul(37) % 10_003) as f64 * 0.25)
        .collect();
    let mut columns = BTreeMap::new();
    columns.insert(
        "id".to_string(),
        Column::from_utf8_contiguous(bytes, offsets),
    );
    columns.insert(value_name.to_string(), Column::from_f64_values(values));
    let column_order = vec!["id".to_string(), value_name.to_string()];
    DataFrame::new_with_column_order(index, columns, column_order).expect("str join frame")
}

fn build_str_key_frame(n: usize, key_cardinality: usize) -> DataFrame {
    let cardinality = key_cardinality.max(1);
    let labels: Vec<IndexLabel> = (0..n).map(|i| IndexLabel::Int64(i as i64)).collect();
    let index = Index::new(labels);

    let mut bytes = Vec::with_capacity(n * 26);
    let mut offsets = Vec::with_capacity(n + 1);
    offsets.push(0);
    let mut key = String::with_capacity(32);
    for row in 0..n {
        let mixed = (row as u64)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .rotate_left(17)
            ^ (row as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        let key_id = mixed % cardinality as u64;
        key.clear();
        write!(&mut key, "key_{mixed:016x}_{key_id:04x}").expect("writing to a String cannot fail");
        bytes.extend_from_slice(key.as_bytes());
        offsets.push(bytes.len());
    }

    let values: Vec<f64> = (0..n)
        .map(|row| ((row as u64).wrapping_mul(37) % 10_003) as f64 * 0.25)
        .collect();
    let mut columns = BTreeMap::new();
    columns.insert(
        "k".to_string(),
        Column::from_utf8_contiguous(bytes, offsets),
    );
    columns.insert("v".to_string(), Column::from_f64_values(values));
    let column_order = vec!["k".to_string(), "v".to_string()];
    DataFrame::new_with_column_order(index, columns, column_order).expect("str key frame")
}

/// Frame of `ncols` independent all-valid contiguous-Utf8 columns (~24-byte
/// values, deterministic) and an Int64 index — a text-heavy DataFrame whose
/// `iloc_bool`/`sort`/`take` cost is dominated by Utf8 column gather, isolating
/// `Column::take_positions`' contiguous-Utf8 path (br-frankenpandas-nl1tw).
fn build_str_multi_frame(n: usize, ncols: usize) -> DataFrame {
    let labels: Vec<IndexLabel> = (0..n).map(|i| IndexLabel::Int64(i as i64)).collect();
    let index = Index::new(labels);
    let mut columns = BTreeMap::new();
    let mut column_order = Vec::with_capacity(ncols);
    for c in 0..ncols {
        let mut bytes = Vec::with_capacity(n * 24);
        let mut offsets = Vec::with_capacity(n + 1);
        offsets.push(0);
        let mut key = String::with_capacity(32);
        for row in 0..n {
            let mixed = (row as u64)
                .wrapping_mul(0x9E37_79B9_7F4A_7C15)
                .rotate_left(17 + c as u32)
                ^ (row as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            key.clear();
            write!(&mut key, "col{c}_val_{mixed:016x}").expect("writing to a String cannot fail");
            bytes.extend_from_slice(key.as_bytes());
            offsets.push(bytes.len());
        }
        let name = format!("s{c}");
        columns.insert(name.clone(), Column::from_utf8_contiguous(bytes, offsets));
        column_order.push(name);
    }
    DataFrame::new_with_column_order(index, columns, column_order).expect("str multi frame")
}

/// Like `build_str_key_frame` but keys genuinely repeat (cardinality bounds the
/// distinct keys), so each group holds many rows — exercising multi-element
/// mean/min/max/count accumulation for the bit-identicality goldens. Row order
/// is deterministic but not sorted; values vary within a group.
fn build_str_key_frame_repeated(n: usize, cardinality: usize) -> DataFrame {
    let cardinality = cardinality.max(1);
    let labels: Vec<IndexLabel> = (0..n).map(|i| IndexLabel::Int64(i as i64)).collect();
    let index = Index::new(labels);

    let mut bytes = Vec::with_capacity(n * 12);
    let mut offsets = Vec::with_capacity(n + 1);
    offsets.push(0);
    let mut key = String::with_capacity(16);
    for row in 0..n {
        let mixed = (row as u64)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .rotate_left(17)
            ^ (row as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        let key_id = mixed % cardinality as u64;
        key.clear();
        write!(&mut key, "key_{key_id:04x}").expect("writing to a String cannot fail");
        bytes.extend_from_slice(key.as_bytes());
        offsets.push(bytes.len());
    }

    let values: Vec<f64> = (0..n)
        .map(|row| ((row as u64).wrapping_mul(37) % 10_003) as f64 * 0.25)
        .collect();
    let mut columns = BTreeMap::new();
    columns.insert(
        "k".to_string(),
        Column::from_utf8_contiguous(bytes, offsets),
    );
    columns.insert("v".to_string(), Column::from_f64_values(values));
    let column_order = vec!["k".to_string(), "v".to_string()];
    DataFrame::new_with_column_order(index, columns, column_order).expect("str key repeated frame")
}

/// Frame with a deterministically-shuffled all-Int64 index (so sort_index must
/// actually reorder) + a couple Float64 value columns — for the radix
/// sort_index benchmark (br-frankenpandas-y5s15).
fn build_sortindex_frame(n: usize) -> DataFrame {
    let labels: Vec<IndexLabel> = (0..n)
        .map(|i| {
            let mixed = (i as u64)
                .wrapping_mul(0x9E37_79B9_7F4A_7C15)
                .rotate_left(23)
                ^ (i as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            IndexLabel::Int64((mixed % (n as u64 * 4)) as i64)
        })
        .collect();
    let index = Index::new(labels);
    let mut columns = BTreeMap::new();
    let v0: Vec<Scalar> = (0..n).map(|i| Scalar::Float64(i as f64 * 0.1)).collect();
    let v1: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Int64((i as i64).wrapping_mul(7)))
        .collect();
    columns.insert(
        "v0".to_string(),
        fp_columnar::Column::from_values(v0).expect("v0"),
    );
    columns.insert(
        "v1".to_string(),
        fp_columnar::Column::from_values(v1).expect("v1"),
    );
    let column_order = vec!["v0".to_string(), "v1".to_string()];
    DataFrame::new_with_column_order(index, columns, column_order).expect("sortindex frame")
}

fn build_numeric_frame(n: usize, cols: usize) -> DataFrame {
    let labels: Vec<IndexLabel> = (0..n).map(|i| IndexLabel::Int64(i as i64)).collect();
    let index = Index::new(labels);
    let mut columns = BTreeMap::new();
    let mut column_order = Vec::with_capacity(cols);
    for c in 0..cols {
        let col_name = format!("c{c}");
        let values: Vec<Scalar> = (0..n)
            .map(|i| Scalar::Float64((i * (c + 1)) as f64 * 0.1))
            .collect();
        let column = fp_columnar::Column::from_values(values).expect("column");
        columns.insert(col_name.clone(), column);
        column_order.push(col_name);
    }
    DataFrame::new_with_column_order(index, columns, column_order).expect("frame")
}

fn build_every_other_bool_mask(index: &Index, n: usize) -> Series {
    let mask: Vec<bool> = (0..n).map(|i| i % 2 == 0).collect();
    let column = Column::from_bool_values(mask);
    Series::new("__mask", index.clone(), column).expect("bool mask")
}

/// Frame for the multi-column sort benchmark (br-frankenpandas-1tuf5): an Int64
/// key with many ties (low cardinality) so the second sort key actually breaks
/// ties, plus a Float64 second key + a Float64 payload. Sorting by `[k0, k1]`
/// exercises both the Int64 and no-NaN-Float64 typed-comparison paths.
fn build_multisort_frame(n: usize) -> DataFrame {
    let labels: Vec<IndexLabel> = (0..n).map(|i| IndexLabel::Int64(i as i64)).collect();
    let index = Index::new(labels);
    let k0: Vec<Scalar> = (0..n)
        .map(|i| {
            let h = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 40;
            Scalar::Int64((h % 1000) as i64)
        })
        .collect();
    let k1: Vec<Scalar> = (0..n)
        .map(|i| {
            let h = (i as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9) >> 33;
            Scalar::Float64((h % 100_000) as f64 * 0.5)
        })
        .collect();
    let v: Vec<Scalar> = (0..n).map(|i| Scalar::Float64(i as f64 * 0.25)).collect();
    let mut columns = BTreeMap::new();
    columns.insert(
        "k0".to_string(),
        fp_columnar::Column::from_values(k0).expect("k0"),
    );
    columns.insert(
        "k1".to_string(),
        fp_columnar::Column::from_values(k1).expect("k1"),
    );
    columns.insert(
        "v".to_string(),
        fp_columnar::Column::from_values(v).expect("v"),
    );
    let column_order = vec!["k0".to_string(), "k1".to_string(), "v".to_string()];
    DataFrame::new_with_column_order(index, columns, column_order).expect("multisort frame")
}

/// Build a many-column, all-finite, non-collinear numeric frame for the
/// pairwise corr/cov kernel benchmark. Values come from a cheap deterministic
/// hash so columns are linearly independent (corr != 1 off-diagonal) and
/// contain no NaN (exercises the all-finite Gram-matrix fast path).
fn build_corr_frame(n: usize, cols: usize) -> DataFrame {
    let labels: Vec<IndexLabel> = (0..n).map(|i| IndexLabel::Int64(i as i64)).collect();
    let index = Index::new(labels);
    let mut columns = BTreeMap::new();
    let mut column_order = Vec::with_capacity(cols);
    for c in 0..cols {
        let col_name = format!("c{c}");
        let values: Vec<Scalar> = (0..n)
            .map(|i| {
                // Deterministic splitmix-style hash -> finite f64 in ~[-1, 1).
                let mut z = (i as u64)
                    .wrapping_mul(0x9e37_79b9_7f4a_7c15)
                    .wrapping_add((c as u64).wrapping_mul(0xbf58_476d_1ce4_e5b9));
                z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
                z ^= z >> 31;
                let unit = (z >> 11) as f64 / (1u64 << 53) as f64; // [0, 1)
                Scalar::Float64(unit.mul_add(2.0, -1.0))
            })
            .collect();
        let column = fp_columnar::Column::from_values(values).expect("column");
        columns.insert(col_name.clone(), column);
        column_order.push(col_name);
    }
    DataFrame::new_with_column_order(index, columns, column_order).expect("frame")
}

fn build_csv_string(n: usize, cols: usize) -> String {
    let mut csv = String::with_capacity(n * cols * 15);
    let mut float_buffer = ryu::Buffer::new();
    for c in 0..cols {
        if c > 0 {
            csv.push(',');
        }
        write!(&mut csv, "c{c}").expect("writing to a String cannot fail");
    }
    csv.push('\n');
    for i in 0..n {
        for c in 0..cols {
            if c > 0 {
                csv.push(',');
            }
            let value = (i * (c + 1)) as f64 * 0.1;
            if value.fract() == 0.0 {
                write!(&mut csv, "{}", value as i64).expect("writing to a String cannot fail");
            } else {
                csv.push_str(float_buffer.format(value));
            }
        }
        csv.push('\n');
    }
    csv
}

fn read_csv_no_na_filter(csv: &str) -> DataFrame {
    let options = CsvReadOptions {
        na_filter: false,
        ..CsvReadOptions::default()
    };
    read_csv_with_options(csv, &options).expect("csv read no NA filter")
}

fn read_csv_options_default(csv: &str) -> DataFrame {
    read_csv_with_options(csv, &CsvReadOptions::default()).expect("csv read options")
}

/// Join workload frame: `id` key column at fixed cardinality + one value
/// column. Matches `high_ram_perf_baseline::build_join_frame` so the flamegraph
/// corresponds to the recorded `dataframe_inner_join` baseline (~36x vs pandas).
fn build_join_frame(
    value_name: &str,
    n: usize,
    key_cardinality: usize,
    multiplier: i64,
) -> DataFrame {
    build_join_frame_offset(value_name, n, key_cardinality, multiplier, 0)
}

/// `build_join_frame` with the key domain shifted by `key_offset`, so two
/// frames with the same cardinality and a half-cardinality offset share ~50%
/// of their keys — the partial-overlap shape left/right/outer joins need to
/// exercise their unmatched-row (null-introducing) paths.
fn build_join_frame_offset(
    value_name: &str,
    n: usize,
    key_cardinality: usize,
    multiplier: i64,
    key_offset: i64,
) -> DataFrame {
    let cardinality = key_cardinality.max(1);
    let keys: Vec<Scalar> = (0..n)
        .map(|row| Scalar::Int64((row % cardinality) as i64 + key_offset))
        .collect();
    let values: Vec<Scalar> = (0..n)
        .map(|row| Scalar::Int64(((row as i64 * multiplier + 11) % 10_007).abs()))
        .collect();
    DataFrame::from_dict(
        &["id", value_name],
        vec![("id", keys), (value_name, values)],
    )
    .expect("join frame")
}

/// Join frame with an Int64 `id` key (bounded cardinality) plus `ncols`
/// Float64 value columns. The Float64 values force `merge_dataframes` off the
/// fused dense-i64 builder onto the position-based output path
/// (`build_single_key_inner_merge_output`), and the multiple wide columns make
/// per-column gather dominate — the shape br-frankenpandas-j3jnd parallelizes.
fn build_join_frame_f64_wide(
    value_prefix: &str,
    n: usize,
    key_cardinality: usize,
    ncols: usize,
) -> DataFrame {
    let cardinality = key_cardinality.max(1);
    let mut names: Vec<String> = Vec::with_capacity(ncols + 1);
    names.push("id".to_string());
    let mut cols: Vec<(String, Vec<Scalar>)> = Vec::with_capacity(ncols + 1);
    let keys: Vec<Scalar> = (0..n)
        .map(|row| Scalar::Int64((row % cardinality) as i64))
        .collect();
    cols.push(("id".to_string(), keys));
    for c in 0..ncols {
        let name = format!("{value_prefix}{c}");
        let values: Vec<Scalar> = (0..n)
            .map(|row| Scalar::Float64((row.wrapping_mul(c + 1) % 10_007) as f64 * 0.5))
            .collect();
        names.push(name.clone());
        cols.push((name, values));
    }
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let col_refs: Vec<(&str, Vec<Scalar>)> = cols
        .into_iter()
        .map(|(n, v)| (Box::leak(n.into_boxed_str()) as &str, v))
        .collect();
    DataFrame::from_dict(&name_refs, col_refs).expect("f64 wide join frame")
}

/// Join frame with a unique Int64 `id` key (1:1 join, no fanout) shifted by
/// `key_offset`, plus `ncols` all-valid Utf8 value columns. A left join of two
/// of these with a half-`n` offset leaves ~50% of right rows unmatched, so the
/// right Utf8 columns gather through `Column::reindex_by_positions`' null-
/// introducing path (br-frankenpandas-cmxjz). Wide (many Utf8 cols) so the
/// per-column gather dominates the one-time key/position work.
fn build_join_frame_utf8_wide(
    value_prefix: &str,
    n: usize,
    ncols: usize,
    key_offset: i64,
) -> DataFrame {
    let mut names: Vec<String> = Vec::with_capacity(ncols + 1);
    names.push("id".to_string());
    let mut cols: Vec<(String, Vec<Scalar>)> = Vec::with_capacity(ncols + 1);
    let keys: Vec<Scalar> = (0..n as i64).map(|row| Scalar::Int64(row + key_offset)).collect();
    cols.push(("id".to_string(), keys));
    for c in 0..ncols {
        let name = format!("{value_prefix}{c}");
        let values: Vec<Scalar> = (0..n)
            .map(|row| {
                let h = (row as u64)
                    .wrapping_mul(0x9E37_79B9_7F4A_7C15)
                    .wrapping_add((c as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9))
                    >> 33;
                Scalar::Utf8(format!("val_{h:08x}_{:04}", row % 7919))
            })
            .collect();
        names.push(name.clone());
        cols.push((name, values));
    }
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let col_refs: Vec<(&str, Vec<Scalar>)> = cols
        .into_iter()
        .map(|(n, v)| (Box::leak(n.into_boxed_str()) as &str, v))
        .collect();
    DataFrame::from_dict(&name_refs, col_refs).expect("utf8 wide join frame")
}

/// Two sorted frames for `merge_asof` on an Int64 `on` key. Left has `n` rows
/// (key 0..n) + `lcols` Float64 cols; right has ~n/2 rows (even keys) + `rcols`
/// Float64 value cols. The wide Float64 right side makes the per-column output
/// build dominate — the shape br-frankenpandas-fu8f5 parallelizes.
fn build_asof_frames(n: usize, lcols: usize, rcols: usize) -> (DataFrame, DataFrame) {
    let l_keys: Vec<Scalar> = (0..n as i64).map(Scalar::Int64).collect();
    let mut l_names: Vec<&str> = vec!["on"];
    let mut l_cols: Vec<(&str, Vec<Scalar>)> = vec![("on", l_keys)];
    for c in 0..lcols {
        let name: &str = Box::leak(format!("lv{c}").into_boxed_str());
        let v: Vec<Scalar> = (0..n)
            .map(|r| Scalar::Float64((r + c) as f64 * 0.25))
            .collect();
        l_names.push(name);
        l_cols.push((name, v));
    }
    let left = DataFrame::from_dict(&l_names, l_cols).expect("asof left");

    let rn = n / 2;
    let r_keys: Vec<Scalar> = (0..rn as i64).map(|i| Scalar::Int64(i * 2)).collect();
    let mut r_names: Vec<&str> = vec!["on"];
    let mut r_cols: Vec<(&str, Vec<Scalar>)> = vec![("on", r_keys)];
    for c in 0..rcols {
        let name: &str = Box::leak(format!("rv{c}").into_boxed_str());
        let v: Vec<Scalar> = (0..rn)
            .map(|r| Scalar::Float64((r * (c + 1) % 9973) as f64 * 0.5))
            .collect();
        r_names.push(name);
        r_cols.push((name, v));
    }
    let right = DataFrame::from_dict(&r_names, r_cols).expect("asof right");
    (left, right)
}

/// Deterministic serialization of a frame's observable state (index labels +
/// per-column dtype and values in column order). Used for the isomorphism
/// golden-output sha256 proof; it must be stable across the optimization.
fn golden_dump(df: &DataFrame) -> String {
    let mut s = String::new();
    s.push_str(&format!("nrows={}\n", df.len()));
    for label in df.index().labels() {
        s.push_str(&format!("{label:?}|"));
    }
    s.push('\n');
    for name in df.column_names() {
        let col = df.columns().get(name).expect("column");
        s.push_str(&format!("col {name} dtype={:?}\n", col.dtype()));
        for v in col.values() {
            s.push_str(&format!("{v:?};"));
        }
        s.push('\n');
    }
    s
}

fn golden_dump_series(s: &Series) -> String {
    let mut out = String::new();
    out.push_str(&format!("len={} dtype={:?}\n", s.len(), s.column().dtype()));
    for label in s.index().labels() {
        out.push_str(&format!("{label:?}|"));
    }
    out.push('\n');
    for v in s.column().values() {
        out.push_str(&format!("{v:?};"));
    }
    out.push('\n');
    out
}

fn run_golden(scenario: &str, n: usize) {
    let out = match scenario {
        "drop_duplicates" => build_groupby_frame(n, 100)
            .drop_duplicates(None, DuplicateKeep::First, false)
            .expect("dedup"),
        "groupby_cumsum" => {
            let (value, key) = build_groupby_cum_pair(n, 100);
            let out = value
                .groupby(&key)
                .expect("groupby")
                .cumsum()
                .expect("cumsum");
            return print!("{}", golden_dump_series(&out));
        }
        "df_groupby_cumsum" => build_transform_frame(n, 100, 4)
            .groupby(&["k"])
            .expect("groupby")
            .cumsum()
            .expect("cumsum"),
        "groupby_diff" => {
            let (value, key) = build_groupby_cum_pair(n, 100);
            let out = value
                .groupby(&key)
                .expect("groupby")
                .diff(1)
                .expect("diff");
            return print!("{}", golden_dump_series(&out));
        }
        "df_groupby_diff" => build_transform_frame(n, 100, 4)
            .groupby(&["k"])
            .expect("groupby")
            .diff(1)
            .expect("diff"),
        "groupby_transform_mean" => build_transform_frame(n, 100, 4)
            .groupby(&["k"])
            .expect("groupby")
            .transform("mean")
            .expect("transform"),
        "groupby_rank" => build_transform_frame_i64(n, 100, 4)
            .groupby(&["k"])
            .expect("groupby")
            .rank("average", true, "keep")
            .expect("rank"),
        "groupby_rank_f64" => build_transform_frame(n, 100, 4)
            .groupby(&["k"])
            .expect("groupby")
            .rank("average", true, "keep")
            .expect("rank"),
        "groupby_quantile" => build_transform_frame(n, 100, 4)
            .groupby(&["k"])
            .expect("groupby")
            .quantile(0.5)
            .expect("quantile"),
        "str_transform_mean" => build_str_key_frame_repeated(n, 64)
            .groupby(&["k"])
            .expect("groupby")
            .transform("mean")
            .expect("transform"),
        "groupby_transform_median" => build_transform_frame(n, 100, 4)
            .groupby(&["k"])
            .expect("groupby")
            .transform("median")
            .expect("transform"),
        "groupby_transform_first" => build_transform_frame(n, 100, 4)
            .groupby(&["k"])
            .expect("groupby")
            .transform("first")
            .expect("transform"),
        "groupby_transform_last" => build_transform_frame(n, 100, 4)
            .groupby(&["k"])
            .expect("groupby")
            .transform("last")
            .expect("transform"),
        "groupby_transform_prod" => build_transform_frame(n, 100, 4)
            .groupby(&["k"])
            .expect("groupby")
            .transform("prod")
            .expect("transform"),
        "groupby_transform_var" => build_transform_frame(n, 100, 4)
            .groupby(&["k"])
            .expect("groupby")
            .transform("var")
            .expect("transform"),
        "groupby_transform_std" => build_transform_frame(n, 100, 4)
            .groupby(&["k"])
            .expect("groupby")
            .transform("std")
            .expect("transform"),
        "groupby_transform_sum" => build_transform_frame(n, 100, 4)
            .groupby(&["k"])
            .expect("groupby")
            .transform("sum")
            .expect("transform"),
        // card ~3n/4 ⇒ a MIX of size-1 groups (var = Null(NaN)) and size-2 groups
        // (finite var): exercises the validity-mask path
        // (from_f64_values_with_validity) — size-1 rows masked to Null within an
        // otherwise-Float64 column, alongside valid var rows.
        "groupby_transform_var_mixed" => build_transform_frame(n, n * 3 / 4, 1)
            .groupby(&["k"])
            .expect("groupby")
            .transform("var")
            .expect("transform"),
        "sort_single" => build_numeric_frame(n, 4)
            .sort_values("c0", true)
            .expect("sort"),
        "sort_multi" => build_multisort_frame(n)
            .sort_values_multi(&["k0", "k1"], &[true, true], "last")
            .expect("sort multi"),
        "sort_index" => build_sortindex_frame(n)
            .sort_index(true)
            .expect("sort index"),
        "str_sort" => build_str_key_frame(n, 4096)
            .sort_values("k", true)
            .expect("str sort"),
        "str_groupby_sum" => build_str_key_frame(n, 4096)
            .groupby(&["k"])
            .expect("str groupby")
            .sum()
            .expect("str groupby sum"),
        "str_groupby_mean" => build_str_key_frame_repeated(n, 64)
            .groupby(&["k"])
            .expect("str groupby")
            .mean()
            .expect("str groupby mean"),
        "str_groupby_count" => build_str_key_frame_repeated(n, 64)
            .groupby(&["k"])
            .expect("str groupby")
            .count()
            .expect("str groupby count"),
        "str_groupby_min" => build_str_key_frame_repeated(n, 64)
            .groupby(&["k"])
            .expect("str groupby")
            .min()
            .expect("str groupby min"),
        "str_groupby_max" => build_str_key_frame_repeated(n, 64)
            .groupby(&["k"])
            .expect("str groupby")
            .max()
            .expect("str groupby max"),
        "str_groupby_var" => build_str_key_frame_repeated(n, 64)
            .groupby(&["k"])
            .expect("str groupby")
            .var()
            .expect("str groupby var"),
        "str_groupby_std" => build_str_key_frame_repeated(n, 64)
            .groupby(&["k"])
            .expect("str groupby")
            .std()
            .expect("str groupby std"),
        "str_groupby_first" => build_str_key_frame_repeated(n, 64)
            .groupby(&["k"])
            .expect("str groupby")
            .first()
            .expect("str groupby first"),
        "str_groupby_last" => build_str_key_frame_repeated(n, 64)
            .groupby(&["k"])
            .expect("str groupby")
            .last()
            .expect("str groupby last"),
        "str_groupby_prod" => build_str_key_frame_repeated(n, 64)
            .groupby(&["k"])
            .expect("str groupby")
            .prod()
            .expect("str groupby prod"),
        "str_groupby_median" => build_str_key_frame_repeated(n, 64)
            .groupby(&["k"])
            .expect("str groupby")
            .median()
            .expect("str groupby median"),
        "filter_bool" => {
            let frame = build_numeric_frame(n, 10);
            let mask = build_every_other_bool_mask(frame.index(), n);
            frame.filter_rows(&mask).expect("filter")
        }
        "str_filter" => {
            // Filter a text-heavy frame (4 contiguous-Utf8 columns): exercises
            // Column::take_positions' Utf8 gather (br-frankenpandas-nl1tw).
            let frame = build_str_multi_frame(n, 4);
            let mask: Vec<bool> = (0..n).map(|i| i % 2 == 0).collect();
            frame.iloc_bool(&mask).expect("str filter")
        }
        "df_corr" => build_corr_frame(n, 64).corr().expect("corr"),
        "df_cov" => build_corr_frame(n, 64).cov().expect("cov"),
        "df_spearman" => build_corr_frame(n, 64)
            .corr_method("spearman")
            .expect("spearman"),
        "df_kendall" => build_corr_frame(n, 32)
            .corr_method("kendall")
            .expect("kendall"),
        "inner_join" => {
            let left = build_join_frame("left_value", n, 512, 7);
            let right = build_join_frame("right_value", n, 512, 13);
            let out = merge_dataframes(&left, &right, "id", JoinType::Inner).expect("join");
            DataFrame::new_with_column_order(out.index, out.columns, out.column_order)
                .expect("join golden frame")
        }
        "str_left_join" => {
            let left = build_join_frame_utf8_wide("lv", n, 6, 0);
            let right = build_join_frame_utf8_wide("rv", n, 6, (n / 2) as i64);
            let out = merge_dataframes(&left, &right, "id", JoinType::Left).expect("join");
            DataFrame::new_with_column_order(out.index, out.columns, out.column_order)
                .expect("join golden frame")
        }
        "str_outer_join" => {
            let left = build_join_frame_utf8_wide("lv", n, 6, 0);
            let right = build_join_frame_utf8_wide("rv", n, 6, (n / 2) as i64);
            let out = merge_dataframes(&left, &right, "id", JoinType::Outer).expect("join");
            DataFrame::new_with_column_order(out.index, out.columns, out.column_order)
                .expect("join golden frame")
        }
        "f64_inner_join" => {
            let left = build_join_frame_f64_wide("lv", n, 512, 6);
            let right = build_join_frame_f64_wide("rv", n, 512, 6);
            let out = merge_dataframes(&left, &right, "id", JoinType::Inner).expect("join");
            DataFrame::new_with_column_order(out.index, out.columns, out.column_order)
                .expect("join golden frame")
        }
        "str_inner_join" => {
            // ~1:1 keys, ~10% overlap (right keys offset): build+probe over all n
            // rows dominates the small matched output (br-frankenpandas-i388q).
            let left = build_str_join_frame("lv", n, n, 0);
            let right = build_str_join_frame("rv", n, n, n - n / 10);
            let out = merge_dataframes(&left, &right, "id", JoinType::Inner).expect("join");
            DataFrame::new_with_column_order(out.index, out.columns, out.column_order)
                .expect("join golden frame")
        }
        "asof_join" => {
            let (left, right) = build_asof_frames(n, 1, 8);
            let out = merge_asof(&left, &right, "on", AsofDirection::Backward).expect("asof");
            DataFrame::new_with_column_order(out.index, out.columns, out.column_order)
                .expect("asof golden frame")
        }
        "join_1to1" => {
            let left = build_join_frame("left_value", n, n, 7);
            let right = build_join_frame("right_value", n, n, 13);
            let out = merge_dataframes(&left, &right, "id", JoinType::Inner).expect("join");
            DataFrame::new_with_column_order(out.index, out.columns, out.column_order)
                .expect("join golden frame")
        }
        "left_join" => {
            let left = build_join_frame("left_value", n, 512, 7);
            let right = build_join_frame_offset("right_value", n, 512, 13, 256);
            let out = merge_dataframes(&left, &right, "id", JoinType::Left).expect("join");
            DataFrame::new_with_column_order(out.index, out.columns, out.column_order)
                .expect("join golden frame")
        }
        "outer_join" => {
            let left = build_join_frame("left_value", n, 512, 7);
            let right = build_join_frame_offset("right_value", n, 512, 13, 256);
            let out = merge_dataframes(&left, &right, "id", JoinType::Outer).expect("join");
            DataFrame::new_with_column_order(out.index, out.columns, out.column_order)
                .expect("join golden frame")
        }
        "right_join" => {
            let left = build_join_frame("left_value", n, 512, 7);
            let right = build_join_frame_offset("right_value", n, 512, 13, 256);
            let out = merge_dataframes(&left, &right, "id", JoinType::Right).expect("join");
            DataFrame::new_with_column_order(out.index, out.columns, out.column_order)
                .expect("join golden frame")
        }
        "str_contains" => {
            let s = build_str_series(n);
            let out = s.str().contains("needle").expect("str contains");
            return print!("{}", golden_dump_series(&out));
        }
        "str_len" => {
            let s = build_str_series(n);
            let out = s.str().len().expect("str len");
            return print!("{}", golden_dump_series(&out));
        }
        "str_lower" => {
            let s = build_str_series(n);
            let out = s.str().lower().expect("str lower");
            return print!("{}", golden_dump_series(&out));
        }
        "str_chain" => {
            // lower -> strip -> contains: a 3-op pipeline whose intermediate
            // columns can stay contiguous (zero Scalar materialization).
            let s = build_str_series(n);
            let lowered = s.str().lower().expect("lower");
            let stripped = lowered.str().strip().expect("strip");
            let out = stripped.str().contains("needle").expect("contains");
            return print!("{}", golden_dump_series(&out));
        }
        "str_starts_chain" => {
            // lower -> startswith: the second op anchors at row offsets.
            let s = build_str_series(n);
            let lowered = s.str().lower().expect("lower");
            let out = lowered.str().startswith("prefix_0").expect("startswith");
            return print!("{}", golden_dump_series(&out));
        }
        "str_series_sort" => {
            // Plain sort over an Eager Utf8 column (control: as_utf8_contiguous
            // returns None, so this exercises the unchanged Scalar path).
            let s = build_str_series(n);
            let out = s.sort_values(true).expect("sort");
            return print!("{}", golden_dump_series(&out));
        }
        "str_series_take" => {
            // Reversed-index take over an Eager (Scalar-backed) Utf8 Series —
            // isolates Column::take_positions' Scalar-backed-Utf8 gather.
            let s = build_str_series(n);
            // Deterministic pseudo-random permutation (Fisher-Yates with an
            // LCG): a scattered gather order is what stresses the per-row
            // String-clone allocator, isolating take_positions' Utf8 gather.
            let mut idx: Vec<i64> = (0..n as i64).collect();
            let mut state: u64 = 0x243F_6A88_85A3_08D3;
            for i in (1..n).rev() {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                let j = (state >> 33) as usize % (i + 1);
                idx.swap(i, j);
            }
            let out = s.take(&idx).expect("take");
            return print!("{}", golden_dump_series(&out));
        }
        "str_value_counts" => {
            // value_counts over a contiguous-Utf8 Series — exercises the byte-span
            // FxHash tally (vcstr) vs the ScalarKey/SipHash path.
            let s = build_str_vc_series(n, 1000);
            let out = s.value_counts().expect("value_counts");
            return print!("{}", golden_dump_series(&out));
        }
        "str_unique" => {
            // Series.unique over contiguous-Utf8 — byte-span FxHash dedup (vcstr).
            let s = build_str_vc_series(n, 1000);
            let u = s.unique();
            let labels: Vec<IndexLabel> = (0..u.len() as i64).map(IndexLabel::Int64).collect();
            let out = Series::from_values("u", labels, u).expect("unique series");
            return print!("{}", golden_dump_series(&out));
        }
        "str_factorize" => {
            // Series.factorize over contiguous-Utf8 — byte-span FxHash codes (vcstr).
            let s = build_str_vc_series(n, 1000);
            let (codes, uniques) = s.factorize().expect("factorize");
            return print!(
                "{}{}",
                golden_dump_series(&codes),
                golden_dump_series(&uniques)
            );
        }
        "str_drop_duplicates" => {
            // Series.drop_duplicates over contiguous-Utf8 — byte-span FxHash (vcstr).
            let s = build_str_vc_series(n, 1000);
            let out = s.drop_duplicates().expect("drop_duplicates");
            return print!("{}", golden_dump_series(&out));
        }
        "str_mode" => {
            // Series.mode over contiguous-Utf8 — byte-span FxHash tally (vcstr).
            let s = build_str_vc_series(n, 1000);
            let out = s.mode().expect("mode");
            return print!("{}", golden_dump_series(&out));
        }
        "str_isin" => {
            // Series.isin over contiguous-Utf8 vs ~500 string needles — byte-span
            // FxHash membership + typed Bool output (vcstr).
            let s = build_str_vc_series(n, 1000);
            let needles: Vec<Scalar> = (0..500u64)
                .map(|id| Scalar::Utf8(format!("val_{id:06x}")))
                .collect();
            let out = s.isin(&needles).expect("isin");
            return print!("{}", golden_dump_series(&out));
        }
        "str_duplicated" => {
            // Series.duplicated over contiguous-Utf8 — byte-span FxHash dup-flags
            // + typed Bool mask (vcstr).
            let s = build_str_vc_series(n, 1000);
            let out = s.duplicated().expect("duplicated");
            return print!("{}", golden_dump_series(&out));
        }
        "reindex_str" => {
            // Reindex an all-valid Utf8 Series to ~50% missing labels — exercises
            // Column::reindex_by_positions' null-introducing Utf8 gather (cmxjz).
            let s = build_str_series(n);
            let new_labels: Vec<IndexLabel> = (0..n as i64)
                .map(|i| {
                    if i % 2 == 0 {
                        IndexLabel::Int64(i / 2)
                    } else {
                        IndexLabel::Int64(n as i64 + i)
                    }
                })
                .collect();
            let out = s.reindex(new_labels).expect("reindex");
            return print!("{}", golden_dump_series(&out));
        }
        "series_sort_index" => {
            let s = build_series_sortindex(n);
            let out = s.sort_index(true).expect("series sort index");
            return print!("{}", golden_dump_series(&out));
        }
        "rank_avg" => {
            let s = build_rank_series(n);
            let out = s.rank("average", true, "keep").expect("rank");
            return print!("{}", golden_dump_series(&out));
        }
        "rank_first" => {
            let s = build_rank_series(n);
            let out = s.rank("first", true, "keep").expect("rank");
            return print!("{}", golden_dump_series(&out));
        }
        "str_sort_chain" => {
            // lower -> sort_values: the sort reads the contiguous buffer
            // (br-frankenpandas-vecff) instead of materializing 1M Scalars.
            let s = build_str_series(n);
            let lowered = s.str().lower().expect("lower");
            let out = lowered.sort_values(true).expect("sort");
            return print!("{}", golden_dump_series(&out));
        }
        "series_add" => {
            let (left, right) = build_series_pair(n);
            let out = left.add(&right).expect("series add");
            return print!("{}", golden_dump_series(&out));
        }
        "series_add_same" => {
            let (left, right) = build_series_pair_same(n);
            let out = left.add(&right).expect("series add same");
            return print!("{}", golden_dump_series(&out));
        }
        "series_add_align" => {
            let (left, right) = build_series_pair(n);
            let policy = RuntimePolicy::strict();
            let mut ledger = EvidenceLedger::new();
            let out = match left.add_with_policy(&right, &policy, &mut ledger) {
                Ok(out) => out,
                Err(err) => {
                    eprintln!("series add align golden failed: {err}");
                    std::process::exit(1);
                }
            };
            return print!("{}", golden_dump_series(&out));
        }
        "csv_read" => {
            let csv = build_csv_string(n, 10);
            read_csv_str(&csv).expect("csv read")
        }
        "csv_read_options" => {
            let csv = build_csv_string(n, 10);
            read_csv_options_default(&csv)
        }
        "csv_read_no_na_filter" => {
            let csv = build_csv_string(n, 10);
            read_csv_no_na_filter(&csv)
        }
        other => {
            eprintln!("unknown golden scenario: {other}");
            std::process::exit(2);
        }
    };
    print!("{}", golden_dump(&out));
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let scenario = args.get(1).map(String::as_str).unwrap_or("drop_duplicates");
    let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(100_000);
    let iters: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(200);

    // Golden mode: `perf_profile golden <scenario> <n>` prints a deterministic
    // dump of the operation's output for sha256 isomorphism proofs.
    if scenario == "golden" {
        let gscenario = args.get(2).map(String::as_str).unwrap_or("drop_duplicates");
        let gn: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(5_000);
        run_golden(gscenario, gn);
        return;
    }

    eprintln!("perf_profile: scenario={scenario} n={n} iters={iters}");
    let start = Instant::now();
    let mut sink: usize = 0;

    match scenario {
        "drop_duplicates" => {
            let frame = build_groupby_frame(n, 100);
            for _ in 0..iters {
                let out = frame
                    .drop_duplicates(None, DuplicateKeep::First, false)
                    .expect("dedup");
                sink = sink.wrapping_add(out.len());
            }
        }
        "groupby_transform_median" => {
            let frame = build_transform_frame(n, 100, 4);
            for _ in 0..iters {
                let out = frame
                    .groupby(&["k"])
                    .expect("groupby")
                    .transform("median")
                    .expect("transform");
                sink = sink.wrapping_add(out.len());
            }
        }
        "groupby_transform_std" => {
            let frame = build_transform_frame(n, 100, 4);
            for _ in 0..iters {
                let out = frame
                    .groupby(&["k"])
                    .expect("groupby")
                    .transform("std")
                    .expect("transform");
                sink = sink.wrapping_add(out.len());
            }
        }
        "groupby_cumsum" => {
            let (value, key) = build_groupby_cum_pair(n, 100);
            for _ in 0..iters {
                let out = value
                    .groupby(&key)
                    .expect("groupby")
                    .cumsum()
                    .expect("cumsum");
                sink = sink.wrapping_add(out.len());
            }
        }
        "df_groupby_cumsum" => {
            let frame = build_transform_frame(n, 100, 4);
            for _ in 0..iters {
                let out = frame
                    .groupby(&["k"])
                    .expect("groupby")
                    .cumsum()
                    .expect("cumsum");
                sink = sink.wrapping_add(out.len());
            }
        }
        "groupby_diff" => {
            let (value, key) = build_groupby_cum_pair(n, 100);
            for _ in 0..iters {
                let out = value
                    .groupby(&key)
                    .expect("groupby")
                    .diff(1)
                    .expect("diff");
                sink = sink.wrapping_add(out.len());
            }
        }
        "df_groupby_diff" => {
            let frame = build_transform_frame(n, 100, 4);
            for _ in 0..iters {
                let out = frame
                    .groupby(&["k"])
                    .expect("groupby")
                    .diff(1)
                    .expect("diff");
                sink = sink.wrapping_add(out.len());
            }
        }
        "groupby_transform_mean" => {
            let frame = build_transform_frame(n, 100, 4);
            for _ in 0..iters {
                let out = frame
                    .groupby(&["k"])
                    .expect("groupby")
                    .transform("mean")
                    .expect("transform");
                sink = sink.wrapping_add(out.len());
            }
        }
        "groupby_rank" => {
            let frame = build_transform_frame_i64(n, 100, 4);
            for _ in 0..iters {
                let out = frame
                    .groupby(&["k"])
                    .expect("groupby")
                    .rank("average", true, "keep")
                    .expect("rank");
                sink = sink.wrapping_add(out.len());
            }
        }
        "groupby_rank_f64" => {
            let frame = build_transform_frame(n, 100, 4);
            for _ in 0..iters {
                let out = frame
                    .groupby(&["k"])
                    .expect("groupby")
                    .rank("average", true, "keep")
                    .expect("rank");
                sink = sink.wrapping_add(out.len());
            }
        }
        "groupby_quantile" => {
            let frame = build_transform_frame(n, 100, 4);
            for _ in 0..iters {
                let out = frame
                    .groupby(&["k"])
                    .expect("groupby")
                    .quantile(0.5)
                    .expect("quantile");
                sink = sink.wrapping_add(out.len());
            }
        }
        "str_transform_mean" => {
            let frame = build_str_key_frame_repeated(n, 64);
            for _ in 0..iters {
                let out = frame
                    .groupby(&["k"])
                    .expect("groupby")
                    .transform("mean")
                    .expect("transform");
                sink = sink.wrapping_add(out.len());
            }
        }
        "sort_single" => {
            let frame = build_numeric_frame(n, 4);
            for _ in 0..iters {
                let out = frame.sort_values("c0", true).expect("sort");
                sink = sink.wrapping_add(out.len());
            }
        }
        "sort_multi" => {
            let frame = build_multisort_frame(n);
            for _ in 0..iters {
                let out = frame
                    .sort_values_multi(&["k0", "k1"], &[true, true], "last")
                    .expect("sort multi");
                sink = sink.wrapping_add(out.len());
            }
        }
        "sort_index" => {
            let frame = build_sortindex_frame(n);
            for _ in 0..iters {
                let out = frame.sort_index(true).expect("sort index");
                sink = sink.wrapping_add(out.len());
            }
        }
        "str_sort" => {
            let frame = build_str_key_frame(n, 4096);
            for _ in 0..iters {
                let out = frame.sort_values("k", true).expect("str sort");
                sink = sink.wrapping_add(out.len());
            }
        }
        "str_groupby_sum" => {
            let frame = build_str_key_frame(n, 4096);
            for _ in 0..iters {
                let out = frame
                    .groupby(&["k"])
                    .expect("str groupby")
                    .sum()
                    .expect("str groupby sum");
                sink = sink.wrapping_add(out.len());
            }
        }
        "str_groupby_mean" => {
            let frame = build_str_key_frame(n, 4096);
            for _ in 0..iters {
                let out = frame
                    .groupby(&["k"])
                    .expect("str groupby")
                    .mean()
                    .expect("str groupby mean");
                sink = sink.wrapping_add(out.len());
            }
        }
        "str_groupby_count" => {
            let frame = build_str_key_frame(n, 4096);
            for _ in 0..iters {
                let out = frame
                    .groupby(&["k"])
                    .expect("str groupby")
                    .count()
                    .expect("str groupby count");
                sink = sink.wrapping_add(out.len());
            }
        }
        "str_groupby_sum_lowcard" => {
            // Common-case groupby: many rows, FEW distinct string keys
            // (br-frankenpandas-90yoh). 64 groups -> hash-group + sort-distinct.
            let frame = build_str_key_frame_repeated(n, 64);
            for _ in 0..iters {
                let out = frame
                    .groupby(&["k"])
                    .expect("str groupby")
                    .sum()
                    .expect("str groupby sum");
                sink = sink.wrapping_add(out.len());
            }
        }
        "str_groupby_count_lowcard" => {
            let frame = build_str_key_frame_repeated(n, 64);
            for _ in 0..iters {
                let out = frame
                    .groupby(&["k"])
                    .expect("str groupby")
                    .count()
                    .expect("str groupby count");
                sink = sink.wrapping_add(out.len());
            }
        }
        "str_groupby_min" => {
            let frame = build_str_key_frame(n, 4096);
            for _ in 0..iters {
                let out = frame
                    .groupby(&["k"])
                    .expect("str groupby")
                    .min()
                    .expect("str groupby min");
                sink = sink.wrapping_add(out.len());
            }
        }
        "str_groupby_max" => {
            let frame = build_str_key_frame(n, 4096);
            for _ in 0..iters {
                let out = frame
                    .groupby(&["k"])
                    .expect("str groupby")
                    .max()
                    .expect("str groupby max");
                sink = sink.wrapping_add(out.len());
            }
        }
        "str_groupby_var" => {
            let frame = build_str_key_frame(n, 4096);
            for _ in 0..iters {
                let out = frame
                    .groupby(&["k"])
                    .expect("str groupby")
                    .var()
                    .expect("str groupby var");
                sink = sink.wrapping_add(out.len());
            }
        }
        "str_groupby_std" => {
            let frame = build_str_key_frame(n, 4096);
            for _ in 0..iters {
                let out = frame
                    .groupby(&["k"])
                    .expect("str groupby")
                    .std()
                    .expect("str groupby std");
                sink = sink.wrapping_add(out.len());
            }
        }
        "str_groupby_first" => {
            let frame = build_str_key_frame(n, 4096);
            for _ in 0..iters {
                let out = frame
                    .groupby(&["k"])
                    .expect("str groupby")
                    .first()
                    .expect("str groupby first");
                sink = sink.wrapping_add(out.len());
            }
        }
        "str_groupby_last" => {
            let frame = build_str_key_frame(n, 4096);
            for _ in 0..iters {
                let out = frame
                    .groupby(&["k"])
                    .expect("str groupby")
                    .last()
                    .expect("str groupby last");
                sink = sink.wrapping_add(out.len());
            }
        }
        "str_groupby_prod" => {
            let frame = build_str_key_frame(n, 4096);
            for _ in 0..iters {
                let out = frame
                    .groupby(&["k"])
                    .expect("str groupby")
                    .prod()
                    .expect("str groupby prod");
                sink = sink.wrapping_add(out.len());
            }
        }
        "str_groupby_median" => {
            let frame = build_str_key_frame(n, 4096);
            for _ in 0..iters {
                let out = frame
                    .groupby(&["k"])
                    .expect("str groupby")
                    .median()
                    .expect("str groupby median");
                sink = sink.wrapping_add(out.len());
            }
        }
        "filter_bool" => {
            let frame = build_numeric_frame(n, 10);
            let mask = build_every_other_bool_mask(frame.index(), n);
            for _ in 0..iters {
                let out = frame.filter_rows(&mask).expect("filter");
                sink = sink.wrapping_add(out.len());
            }
        }
        "str_filter" => {
            let frame = build_str_multi_frame(n, 4);
            let mask: Vec<bool> = (0..n).map(|i| i % 2 == 0).collect();
            for _ in 0..iters {
                let out = frame.iloc_bool(&mask).expect("str filter");
                sink = sink.wrapping_add(out.len());
            }
        }
        "df_corr" => {
            let frame = build_corr_frame(n, 64);
            for _ in 0..iters {
                let out = frame.corr().expect("corr");
                sink = sink.wrapping_add(out.len());
            }
        }
        "df_cov" => {
            let frame = build_corr_frame(n, 64);
            for _ in 0..iters {
                let out = frame.cov().expect("cov");
                sink = sink.wrapping_add(out.len());
            }
        }
        "df_spearman" => {
            let frame = build_corr_frame(n, 64);
            for _ in 0..iters {
                let out = frame.corr_method("spearman").expect("spearman");
                sink = sink.wrapping_add(out.len());
            }
        }
        "df_kendall" => {
            // kendall is O(M^2) per pair; keep n small in the bench invocation.
            let frame = build_corr_frame(n, 32);
            for _ in 0..iters {
                let out = frame.corr_method("kendall").expect("kendall");
                sink = sink.wrapping_add(out.len());
            }
        }
        "inner_join" => {
            // cardinality 512 matches the high_ram baseline; output fans out to
            // ~n^2/cardinality rows, which is where the ~36x cost lives.
            let left = build_join_frame("left_value", n, 512, 7);
            let right = build_join_frame("right_value", n, 512, 13);
            for _ in 0..iters {
                let out = merge_dataframes(&left, &right, "id", JoinType::Inner).expect("join");
                sink = sink.wrapping_add(out.index.len());
            }
        }
        "str_left_join" => {
            // i64 key (1:1, no fanout) + 6 Utf8 value cols per side, 50% key
            // overlap: left join leaves ~50% right rows unmatched, so the 6 right
            // Utf8 cols gather through reindex_by_positions' null path (cmxjz).
            let left = build_join_frame_utf8_wide("lv", n, 6, 0);
            let right = build_join_frame_utf8_wide("rv", n, 6, (n / 2) as i64);
            for _ in 0..iters {
                let out = merge_dataframes(&left, &right, "id", JoinType::Left).expect("join");
                sink = sink.wrapping_add(out.index.len());
            }
        }
        "str_outer_join" => {
            // Outer join of 50%-overlapping frames: BOTH sides' 6 Utf8 cols
            // null-introduce, so all 12 gather through reindex_by_positions'
            // null path (cmxjz).
            let left = build_join_frame_utf8_wide("lv", n, 6, 0);
            let right = build_join_frame_utf8_wide("rv", n, 6, (n / 2) as i64);
            for _ in 0..iters {
                let out = merge_dataframes(&left, &right, "id", JoinType::Outer).expect("join");
                sink = sink.wrapping_add(out.index.len());
            }
        }
        "f64_inner_join" => {
            // i64 key + 6 Float64 value columns per side: forces the position-
            // based output builder (fused dense-i64 path bails on Float64), where
            // per-column gather over the ~n^2/card output dominates (j3jnd).
            let left = build_join_frame_f64_wide("lv", n, 512, 6);
            let right = build_join_frame_f64_wide("rv", n, 512, 6);
            for _ in 0..iters {
                let out = merge_dataframes(&left, &right, "id", JoinType::Inner).expect("join");
                sink = sink.wrapping_add(out.index.len());
            }
        }
        "str_inner_join" => {
            // ~1:1 keys with ~10% overlap: build+probe over all n rows dominates
            // the small matched output (br-frankenpandas-i388q).
            let left = build_str_join_frame("lv", n, n, 0);
            let right = build_str_join_frame("rv", n, n, n - n / 10);
            for _ in 0..iters {
                let out = merge_dataframes(&left, &right, "id", JoinType::Inner).expect("join");
                sink = sink.wrapping_add(out.index.len());
            }
        }
        "asof_join" => {
            // merge_asof on a sorted i64 key, wide Float64 right side: output is
            // left.len() rows x all cols, dominated by the per-column build (fu8f5).
            let (left, right) = build_asof_frames(n, 1, 8);
            for _ in 0..iters {
                let out = merge_asof(&left, &right, "on", AsofDirection::Backward).expect("asof");
                sink = sink.wrapping_add(out.index.len());
            }
        }
        "left_join" => {
            // 50% key overlap (right keys shifted by half the cardinality):
            // half the left rows match (fanout output), half are unmatched
            // (null-introduced right values -> Float64 promotion path).
            let left = build_join_frame("left_value", n, 512, 7);
            let right = build_join_frame_offset("right_value", n, 512, 13, 256);
            for _ in 0..iters {
                let out = merge_dataframes(&left, &right, "id", JoinType::Left).expect("join");
                sink = sink.wrapping_add(out.index.len());
            }
        }
        "outer_join" => {
            // 50% key overlap: matched fanout rows plus unmatched rows from
            // BOTH sides (null-introduced on each side).
            let left = build_join_frame("left_value", n, 512, 7);
            let right = build_join_frame_offset("right_value", n, 512, 13, 256);
            for _ in 0..iters {
                let out = merge_dataframes(&left, &right, "id", JoinType::Outer).expect("join");
                sink = sink.wrapping_add(out.index.len());
            }
        }
        "right_join" => {
            // 50% key overlap: half the right rows match (fanout), half are
            // unmatched (null-introduced left values, dtype preserved).
            let left = build_join_frame("left_value", n, 512, 7);
            let right = build_join_frame_offset("right_value", n, 512, 13, 256);
            for _ in 0..iters {
                let out = merge_dataframes(&left, &right, "id", JoinType::Right).expect("join");
                sink = sink.wrapping_add(out.index.len());
            }
        }
        "str_contains" => {
            let s = build_str_series(n);
            for _ in 0..iters {
                let out = s.str().contains("needle").expect("str contains");
                sink = sink.wrapping_add(out.len());
            }
        }
        "str_len" => {
            let s = build_str_series(n);
            for _ in 0..iters {
                let out = s.str().len().expect("str len");
                sink = sink.wrapping_add(out.len());
            }
        }
        "str_lower" => {
            let s = build_str_series(n);
            for _ in 0..iters {
                let out = s.str().lower().expect("str lower");
                sink = sink.wrapping_add(out.len());
            }
        }
        "str_chain" => {
            let s = build_str_series(n);
            for _ in 0..iters {
                let lowered = s.str().lower().expect("lower");
                let stripped = lowered.str().strip().expect("strip");
                let out = stripped.str().contains("needle").expect("contains");
                sink = sink.wrapping_add(out.len());
            }
        }
        "str_starts_chain" => {
            let s = build_str_series(n);
            for _ in 0..iters {
                let lowered = s.str().lower().expect("lower");
                let out = lowered.str().startswith("prefix_0").expect("startswith");
                sink = sink.wrapping_add(out.len());
            }
        }
        "str_series_sort" => {
            let s = build_str_series(n);
            for _ in 0..iters {
                let out = s.sort_values(true).expect("sort");
                sink = sink.wrapping_add(out.len());
            }
        }
        "str_series_take" => {
            let s = build_str_series(n);
            // Deterministic pseudo-random permutation (Fisher-Yates with an
            // LCG): a scattered gather order is what stresses the per-row
            // String-clone allocator, isolating take_positions' Utf8 gather.
            let mut idx: Vec<i64> = (0..n as i64).collect();
            let mut state: u64 = 0x243F_6A88_85A3_08D3;
            for i in (1..n).rev() {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                let j = (state >> 33) as usize % (i + 1);
                idx.swap(i, j);
            }
            for _ in 0..iters {
                let out = s.take(&idx).expect("take");
                sink = sink.wrapping_add(out.len());
            }
        }
        "str_value_counts" => {
            let s = build_str_vc_series(n, 1000);
            for _ in 0..iters {
                let out = s.value_counts().expect("value_counts");
                sink = sink.wrapping_add(out.len());
            }
        }
        "str_unique" => {
            let s = build_str_vc_series(n, 1000);
            for _ in 0..iters {
                let u = s.unique();
                sink = sink.wrapping_add(u.len());
            }
        }
        "str_factorize" => {
            let s = build_str_vc_series(n, 1000);
            for _ in 0..iters {
                let (codes, uniques) = s.factorize().expect("factorize");
                sink = sink.wrapping_add(codes.len() ^ uniques.len());
            }
        }
        "str_drop_duplicates" => {
            let s = build_str_vc_series(n, 1000);
            for _ in 0..iters {
                let out = s.drop_duplicates().expect("drop_duplicates");
                sink = sink.wrapping_add(out.len());
            }
        }
        "str_mode" => {
            let s = build_str_vc_series(n, 1000);
            for _ in 0..iters {
                let out = s.mode().expect("mode");
                sink = sink.wrapping_add(out.len());
            }
        }
        "str_isin" => {
            let s = build_str_vc_series(n, 1000);
            let needles: Vec<Scalar> = (0..500u64)
                .map(|id| Scalar::Utf8(format!("val_{id:06x}")))
                .collect();
            for _ in 0..iters {
                let out = s.isin(&needles).expect("isin");
                sink = sink.wrapping_add(out.len());
            }
        }
        "str_duplicated" => {
            let s = build_str_vc_series(n, 1000);
            for _ in 0..iters {
                let out = s.duplicated().expect("duplicated");
                sink = sink.wrapping_add(out.len());
            }
        }
        "reindex_str" => {
            let s = build_str_series(n);
            let new_labels: Vec<IndexLabel> = (0..n as i64)
                .map(|i| {
                    if i % 2 == 0 {
                        IndexLabel::Int64(i / 2)
                    } else {
                        IndexLabel::Int64(n as i64 + i)
                    }
                })
                .collect();
            for _ in 0..iters {
                let out = s.reindex(new_labels.clone()).expect("reindex");
                sink = sink.wrapping_add(out.len());
            }
        }
        "series_sort_index" => {
            let s = build_series_sortindex(n);
            for _ in 0..iters {
                let out = s.sort_index(true).expect("series sort index");
                sink = sink.wrapping_add(out.len());
            }
        }
        "rank_avg" => {
            let s = build_rank_series(n);
            for _ in 0..iters {
                let out = s.rank("average", true, "keep").expect("rank");
                sink = sink.wrapping_add(out.len());
            }
        }
        "str_sort_chain" => {
            let s = build_str_series(n);
            for _ in 0..iters {
                let lowered = s.str().lower().expect("lower");
                let out = lowered.sort_values(true).expect("sort");
                sink = sink.wrapping_add(out.len());
            }
        }
        "inner_join_read" => {
            // Same fanout join as inner_join, but every iteration also READS
            // every output column (as_i64_slice sum), forcing any lazy output
            // representation to fully materialize — the downstream-consumer
            // gate for lazy join outputs (br-frankenpandas-3ad4n).
            let left = build_join_frame("left_value", n, 512, 7);
            let right = build_join_frame("right_value", n, 512, 13);
            for _ in 0..iters {
                let out = merge_dataframes(&left, &right, "id", JoinType::Inner).expect("join");
                let mut acc: i64 = 0;
                for name in &out.column_order {
                    let column = out.columns.get(name).expect("output column must exist");
                    let slice = column.as_i64_slice().expect("dense join output is Int64");
                    acc = acc.wrapping_add(slice.iter().sum::<i64>());
                }
                sink = sink
                    .wrapping_add(out.index.len())
                    .wrapping_add(acc as usize);
            }
        }
        "join_1to1" => {
            // Unique keys on both sides (cardinality = n) -> 1:1 join, output n
            // rows. Output gather is O(n); per-row composite-key extraction
            // (2n allocations) is the dominant non-gather cost here.
            let left = build_join_frame("left_value", n, n, 7);
            let right = build_join_frame("right_value", n, n, 13);
            for _ in 0..iters {
                let out = merge_dataframes(&left, &right, "id", JoinType::Inner).expect("join");
                sink = sink.wrapping_add(out.index.len());
            }
        }
        "series_add" => {
            let (left, right) = build_series_pair(n);
            for _ in 0..iters {
                let out = left.add(&right).expect("series add");
                sink = sink.wrapping_add(out.len());
            }
        }
        "series_add_same" => {
            let (left, right) = build_series_pair_same(n);
            for _ in 0..iters {
                let out = left.add(&right).expect("series add same");
                sink = sink.wrapping_add(out.len());
            }
        }
        "series_add_align" => {
            let (left, right) = build_series_pair(n);
            let policy = RuntimePolicy::strict();
            for _ in 0..iters {
                let mut ledger = EvidenceLedger::new();
                let out = match left.add_with_policy(&right, &policy, &mut ledger) {
                    Ok(out) => out,
                    Err(err) => {
                        eprintln!("series add align failed: {err}");
                        std::process::exit(1);
                    }
                };
                sink = sink.wrapping_add(out.len());
            }
        }
        "csv_read" => {
            // Matches bench_runner::build_csv_string + io/csv_read shape:
            // 10 dense numeric columns, default pandas-style CSV parsing.
            let csv = build_csv_string(n, 10);
            for _ in 0..iters {
                let out = read_csv_str(&csv).expect("csv read");
                sink = sink.wrapping_add(out.len());
            }
        }
        "csv_read_options" => {
            let csv = build_csv_string(n, 10);
            for _ in 0..iters {
                let out = read_csv_options_default(&csv);
                sink = sink.wrapping_add(out.len());
            }
        }
        "csv_read_no_na_filter" => {
            let csv = build_csv_string(n, 10);
            for _ in 0..iters {
                let out = read_csv_no_na_filter(&csv);
                sink = sink.wrapping_add(out.len());
            }
        }
        other => {
            eprintln!("unknown scenario: {other}");
            std::process::exit(2);
        }
    }

    let elapsed = start.elapsed();
    let per_iter_ms = elapsed.as_secs_f64() * 1e3 / iters as f64;
    eprintln!(
        "perf_profile: done {iters} iters in {:.3}s ({per_iter_ms:.3} ms/iter), sink={sink}",
        elapsed.as_secs_f64()
    );
}
