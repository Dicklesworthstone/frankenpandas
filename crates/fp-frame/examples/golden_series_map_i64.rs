//! Differential golden for Series.map / map_with_default on an Int64 series with
//! an Int64->Scalar mapping (the typed fast path) vs an independent reference.
//! Run: cargo run -p fp-frame --example golden_series_map_i64 --release

use std::collections::HashMap;

use fp_columnar::Column;
use fp_frame::Series;
use fp_index::{Index, IndexLabel};
use fp_types::{NullKind, Scalar};

fn mk(vals: &[i64]) -> Series {
    let labels: Vec<IndexLabel> = (0..vals.len() as i64).map(IndexLabel::Int64).collect();
    Series::new(
        "c".to_string(),
        Index::new(labels),
        Column::from_i64_values(vals.to_vec()),
    )
    .unwrap()
}

fn ref_map(vals: &[i64], mapping: &[(i64, Scalar)], default: Option<&Scalar>) -> Vec<Scalar> {
    // First-occurrence-wins map; unmapped -> default or Null(NaN).
    let mut m: HashMap<i64, &Scalar> = HashMap::new();
    for (k, v) in mapping {
        m.entry(*k).or_insert(v);
    }
    vals.iter()
        .map(|v| match m.get(v) {
            Some(s) => (*s).clone(),
            None => default.cloned().unwrap_or(Scalar::Null(NullKind::NaN)),
        })
        .collect()
}

fn check(name: &str, vals: &[i64], mapping: &[(i64, Scalar)]) {
    let pairs: Vec<(Scalar, Scalar)> = mapping
        .iter()
        .map(|(k, v)| (Scalar::Int64(*k), v.clone()))
        .collect();
    let s = mk(vals);

    let act = s.map(&pairs).unwrap();
    let exp = ref_map(vals, mapping, None);
    if act.column().values() != exp.as_slice() {
        println!("FAIL map {name}");
        std::process::exit(1);
    }
    let dflt = Scalar::Int64(-999);
    let act_d = s.map_with_default(&pairs, &dflt).unwrap();
    let exp_d = ref_map(vals, mapping, Some(&dflt));
    if act_d.column().values() != exp_d.as_slice() {
        println!("FAIL map_with_default {name}");
        std::process::exit(1);
    }
    println!("OK   {name}");
}

fn main() {
    let mut z = 0x99u64;
    let mut rnd = |m: i64| {
        z ^= z << 13;
        z ^= z >> 7;
        z ^= z << 17;
        (z % m as u64) as i64
    };
    let n = 20_000usize;

    // Dense low-card, full coverage.
    let dense: Vec<i64> = (0..n).map(|_| rnd(50)).collect();
    let map_full: Vec<(i64, Scalar)> = (0..50).map(|i| (i, Scalar::Int64(i * 10))).collect();
    check("dense_full_coverage", &dense, &map_full);

    // Partial coverage (some values unmapped -> NaN/default).
    let map_partial: Vec<(i64, Scalar)> = (0..25).map(|i| (i, Scalar::Int64(i + 1000))).collect();
    check("partial_coverage", &dense, &map_partial);

    // Mapping to mixed scalar value types.
    let map_mixed: Vec<(i64, Scalar)> = vec![
        (0, Scalar::Utf8("zero".into())),
        (1, Scalar::Float64(1.5)),
        (2, Scalar::Bool(true)),
        (3, Scalar::Int64(33)),
    ];
    check("mixed_value_types", &dense, &map_mixed);

    // Duplicate keys in mapping (first-occurrence wins).
    let map_dup: Vec<(i64, Scalar)> = vec![
        (5, Scalar::Int64(100)),
        (5, Scalar::Int64(200)),
        (6, Scalar::Int64(300)),
    ];
    check("duplicate_keys", &dense, &map_dup);

    // Negative values + empty mapping.
    check(
        "negatives",
        &(0..n).map(|_| rnd(200) - 100).collect::<Vec<_>>(),
        &map_full,
    );
    check("empty_mapping", &dense, &[]);
    check("empty_series", &[], &map_full);
    check(
        "i64_extremes",
        &[i64::MIN, 0, i64::MAX, i64::MIN],
        &[(i64::MIN, Scalar::Int64(1)), (i64::MAX, Scalar::Int64(2))],
    );

    println!("ALL GOLDEN CHECKS PASSED");
}
