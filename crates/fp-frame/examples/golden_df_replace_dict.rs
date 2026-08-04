//! Differential golden for DataFrame.replace_dict (per-column replacements) vs
//! an independent reference, incl. unlisted columns left unchanged.
//! Run: cargo run -p fp-frame --example golden_df_replace_dict --release

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
    let c: Vec<i64> = (0..n).map(|_| rnd(40)).collect();

    let index = Index::new((0..n as i64).map(IndexLabel::Int64).collect());
    let mut map = BTreeMap::new();
    map.insert("a".to_string(), Column::from_values(i64s(&a)).unwrap());
    map.insert("b".to_string(), Column::from_values(i64s(&b)).unwrap());
    map.insert("c".to_string(), Column::from_values(i64s(&c)).unwrap());
    let df = DataFrame::new_with_column_order(
        index,
        map,
        vec!["a".to_string(), "b".to_string(), "c".to_string()],
    )
    .unwrap();

    // Replace in a and b only (c left unchanged).
    let mut per: BTreeMap<String, Vec<(Scalar, Scalar)>> = BTreeMap::new();
    per.insert(
        "a".to_string(),
        (0..20)
            .map(|i| (Scalar::Int64(i), Scalar::Int64(i + 500)))
            .collect(),
    );
    per.insert(
        "b".to_string(),
        vec![(Scalar::Int64(5), Scalar::Utf8("five".into()))],
    );

    let out = df.replace_dict(&per).unwrap();
    let exp_a = ref_replace(&i64s(&a), &per["a"]);
    let exp_b = ref_replace(&i64s(&b), &per["b"]);
    let exp_c = i64s(&c); // unchanged
    let ok = out.column("a").unwrap().values().to_vec() == exp_a
        && out.column("b").unwrap().values().to_vec() == exp_b
        && out.column("c").unwrap().values().to_vec() == exp_c;
    if !ok {
        println!("FAIL replace_dict");
        std::process::exit(1);
    }
    println!("OK   replace_dict (a, b replaced; c unchanged)");
    println!("ALL GOLDEN CHECKS PASSED");
}
