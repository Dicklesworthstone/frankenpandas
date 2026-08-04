//! Differential golden for DataFrame.replace vs an independent per-value
//! first-semantic-match reference, over Int64 + mixed-dtype columns.
//! Run: cargo run -p fp-frame --example golden_df_replace_i64 --release

use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;

fn ref_replace(col: &[Scalar], repl: &[(Scalar, Scalar)]) -> Vec<Scalar> {
    col.iter()
        .map(|v| {
            for (from, to) in repl {
                if from.semantic_eq(v) {
                    return to.clone();
                }
            }
            v.clone()
        })
        .collect()
}

fn run(name: &str, cols: Vec<(&str, Vec<Scalar>)>, repl: &[(Scalar, Scalar)]) {
    let n = cols[0].1.len();
    let index = Index::new((0..n as i64).map(IndexLabel::Int64).collect());
    let mut map = BTreeMap::new();
    let mut order = Vec::new();
    for (cn, cv) in &cols {
        map.insert((*cn).to_string(), Column::from_values(cv.clone()).unwrap());
        order.push((*cn).to_string());
    }
    let df = DataFrame::new_with_column_order(index, map, order).unwrap();
    let out = df.replace(repl).unwrap();
    for (cn, cv) in &cols {
        let got = out.column(cn).unwrap().values().to_vec();
        let exp = ref_replace(cv, repl);
        if got != exp {
            println!("FAIL {name} col={cn}");
            std::process::exit(1);
        }
    }
    println!("OK   {name}");
}

fn i64s(v: &[i64]) -> Vec<Scalar> {
    v.iter().map(|&x| Scalar::Int64(x)).collect()
}

fn main() {
    let mut z = 0x55u64;
    let mut rnd = |m: i64| {
        z ^= z << 13;
        z ^= z >> 7;
        z ^= z << 17;
        (z % m as u64) as i64
    };
    let n = 5_000usize;
    let a: Vec<i64> = (0..n).map(|_| rnd(40)).collect();
    let b: Vec<i64> = (0..n).map(|_| rnd(40)).collect();

    let r_i64: Vec<(Scalar, Scalar)> = (0..20)
        .map(|i| (Scalar::Int64(i), Scalar::Int64(i + 500)))
        .collect();
    run(
        "two_int64_cols",
        vec![("a", i64s(&a)), ("b", i64s(&b))],
        &r_i64,
    );

    // Mixed value types in the replacement (Scalar-output path).
    let r_mixed: Vec<(Scalar, Scalar)> = vec![
        (Scalar::Int64(0), Scalar::Utf8("zero".into())),
        (Scalar::Int64(1), Scalar::Float64(1.5)),
        (Scalar::Int64(2), Scalar::Int64(99)),
    ];
    run("int64_mixed_repl", vec![("a", i64s(&a))], &r_mixed);

    // Float64 key (forces the linear semantic_eq fallback, cross-type matching).
    let fcol: Vec<Scalar> = (0..n).map(|i| Scalar::Float64((i % 10) as f64)).collect();
    let r_float: Vec<(Scalar, Scalar)> = vec![
        (Scalar::Float64(3.0), Scalar::Float64(-1.0)),
        (Scalar::Float64(7.0), Scalar::Float64(-2.0)),
    ];
    run("float_col_float_repl", vec![("f", fcol)], &r_float);

    // Utf8 column.
    let scol: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Utf8(format!("k{}", i % 8)))
        .collect();
    let r_str: Vec<(Scalar, Scalar)> = vec![
        (Scalar::Utf8("k0".into()), Scalar::Utf8("ZERO".into())),
        (Scalar::Utf8("k3".into()), Scalar::Utf8("THREE".into())),
    ];
    run("utf8_col", vec![("s", scol)], &r_str);

    run("empty_repl", vec![("a", i64s(&a))], &[]);
    run(
        "negatives",
        vec![(
            "a",
            i64s(&(0..n).map(|_| rnd(100) - 50).collect::<Vec<_>>()),
        )],
        &r_i64,
    );

    println!("ALL GOLDEN CHECKS PASSED");
}
