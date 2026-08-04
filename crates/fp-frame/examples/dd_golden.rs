//! Deterministic drop_duplicates golden — proves the all-valid-unique Float64
//! subset shortcut (subset_has_all_valid_unique_column) is bit-identical across
//! the exact-bits FxHashSet uniqueness lever (br-frankenpandas-edi9i).
//! Exercises four regimes that all flow through that check:
//!   A) all-unique floats          -> shortcut hit, full clone
//!   B) exact bit duplicates       -> not unique, dedup path drops rows
//!   C) fuzzy-near-but-distinct    -> not unique (fuzzy adjacency), but the
//!      digest dedup path keeps both (distinct bits) — the case where the
//!      sorted-adjacency predicate, not the exact-bits set, decides uniqueness
//!   D) +0.0 / -0.0 mix            -> distinct bits, fuzzy-equal values
//!
//! Run: cargo run --profile release-perf -p fp-frame --example dd_golden

use std::time::Instant;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{DuplicateKeep, Index};

fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn fnv1a(bytes: &[u8], state: &mut u64) {
    for &b in bytes {
        *state ^= u64::from(b);
        *state = state.wrapping_mul(0x0000_0100_0000_01b3);
    }
}

fn build(col0: Vec<f64>) -> DataFrame {
    let n = col0.len();
    let mut s = 0xABCD_1234_5678_9999u64;
    let col1: Vec<f64> = (0..n).map(|_| splitmix(&mut s) as f64 / 1e6).collect();
    let index = Index::new_known_unique_int64_unit_range(0, n);
    let mut columns = std::collections::BTreeMap::new();
    columns.insert("col_0".to_string(), Column::from_f64_values(col0));
    columns.insert("col_1".to_string(), Column::from_f64_values(col1));
    DataFrame::new_with_column_order(
        index,
        columns,
        vec!["col_0".to_string(), "col_1".to_string()],
    )
    .expect("frame")
}

fn digest_frame(df: &DataFrame, state: &mut u64) {
    fnv1a(&(df.len() as u64).to_le_bytes(), state);
    for label in df.index().labels() {
        fnv1a(format!("{label}").as_bytes(), state);
        fnv1a(b";", state);
    }
    let col = df.get_column("col_0");
    for v in col.column().values() {
        fnv1a(format!("{v:?}").as_bytes(), state);
        fnv1a(b",", state);
    }
}

fn run_case(col0: Vec<f64>, digest: &mut u64) {
    let df = build(col0);
    let out = df
        .drop_duplicates(Some(&["col_0".to_string()]), DuplicateKeep::First, false)
        .expect("drop_duplicates");
    digest_frame(&out, digest);
    fnv1a(b"|CASE|", digest);
}

fn main() {
    let mut digest = 0xcbf2_9ce4_8422_2325u64;

    // A) all-unique large frame (the benchmarked shortcut path).
    let mut s = 0x1357_9bdf_0246_8aceu64;
    let big: Vec<f64> = (0..100_000)
        .map(|_| (splitmix(&mut s) >> 11) as f64 / (1u64 << 21) as f64 * 1_000_000.0)
        .collect();
    run_case(big, &mut digest);

    // B) exact bit duplicates scattered in.
    run_case(vec![1.0, 2.0, 3.0, 2.0, 4.0, 1.0, 5.0], &mut digest);

    // C) fuzzy-near-but-distinct-bits: 1.0 and 1.0 + 1e-15 are within the
    //    relative-1e-14 tolerance (so NOT unique) yet have different bits.
    let near = 1.0_f64 + 1e-15;
    run_case(vec![1.0, near, 2.0, 3.0], &mut digest);

    // D) +0.0 / -0.0 (distinct bits, fuzzy-equal value) + a unique tail.
    run_case(vec![0.0, -0.0, 7.0, 8.0, 9.0], &mut digest);

    // E) timing of the shortcut path (all-unique 100k).
    let mut s2 = 0x9999_8888_7777_6666u64;
    let big2: Vec<f64> = (0..100_000)
        .map(|_| (splitmix(&mut s2) >> 11) as f64 / (1u64 << 21) as f64 * 1_000_000.0)
        .collect();
    let df = build(big2);
    let subset = ["col_0".to_string()];
    for _ in 0..3 {
        let _ = df
            .drop_duplicates(Some(&subset), DuplicateKeep::First, false)
            .expect("warm");
    }
    let mut best = f64::MAX;
    for _ in 0..25 {
        let t = Instant::now();
        let _ = df
            .drop_duplicates(Some(&subset), DuplicateKeep::First, false)
            .expect("timed");
        best = best.min(t.elapsed().as_secs_f64() * 1e3);
    }

    println!("dd_golden digest={digest:016x}  shortcut_100k_min={best:.3}ms");
}
