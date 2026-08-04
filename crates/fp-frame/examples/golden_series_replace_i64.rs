//! Differential golden for Series.replace on an Int64 series with an
//! Int64->Scalar replacement table (typed fast path) vs an independent
//! reference (unmapped values keep their original).
//! Run: cargo run -p fp-frame --example golden_series_replace_i64 --release

use std::collections::HashMap;

use fp_columnar::Column;
use fp_frame::Series;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;

fn mk(vals: &[i64]) -> Series {
    let labels: Vec<IndexLabel> = (0..vals.len() as i64).map(IndexLabel::Int64).collect();
    Series::new(
        "c".to_string(),
        Index::new(labels),
        Column::from_i64_values(vals.to_vec()),
    )
    .unwrap()
}

fn reference(vals: &[i64], repl: &[(i64, Scalar)]) -> Vec<Scalar> {
    // First-occurrence wins; unmapped keeps original Int64.
    let mut m: HashMap<i64, &Scalar> = HashMap::new();
    for (k, v) in repl {
        m.entry(*k).or_insert(v);
    }
    vals.iter()
        .map(|v| match m.get(v) {
            Some(s) => (*s).clone(),
            None => Scalar::Int64(*v),
        })
        .collect()
}

fn check(name: &str, vals: &[i64], repl: &[(i64, Scalar)]) {
    let pairs: Vec<(Scalar, Scalar)> = repl
        .iter()
        .map(|(k, v)| (Scalar::Int64(*k), v.clone()))
        .collect();
    let act = mk(vals).replace(&pairs).unwrap();
    let exp = reference(vals, repl);
    if act.column().values() != exp.as_slice() {
        println!("FAIL {name}");
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
    let dense: Vec<i64> = (0..n).map(|_| rnd(50)).collect();

    // All-Int64 replacements, partial coverage (typed-output path).
    let r_partial: Vec<(i64, Scalar)> = (0..25).map(|i| (i, Scalar::Int64(i + 9999))).collect();
    check("partial_int64", &dense, &r_partial);

    // Full coverage.
    let r_full: Vec<(i64, Scalar)> = (0..50).map(|i| (i, Scalar::Int64(i * 7))).collect();
    check("full_int64", &dense, &r_full);

    // Mixed value types (forces Scalar-output typed path).
    let r_mixed: Vec<(i64, Scalar)> = vec![
        (0, Scalar::Utf8("x".into())),
        (1, Scalar::Float64(2.5)),
        (2, Scalar::Bool(false)),
        (3, Scalar::Int64(33)),
    ];
    check("mixed_values", &dense, &r_mixed);

    // Duplicate keys (first wins).
    check(
        "dup_keys",
        &dense,
        &[(5, Scalar::Int64(1)), (5, Scalar::Int64(2))],
    );
    check(
        "negatives",
        &(0..n).map(|_| rnd(200) - 100).collect::<Vec<_>>(),
        &r_full,
    );
    check("empty_repl", &dense, &[]);
    check("empty_series", &[], &r_full);
    check(
        "i64_extremes",
        &[i64::MIN, 0, i64::MAX, i64::MIN],
        &[(i64::MIN, Scalar::Int64(1)), (i64::MAX, Scalar::Int64(2))],
    );

    println!("ALL GOLDEN CHECKS PASSED");
}
