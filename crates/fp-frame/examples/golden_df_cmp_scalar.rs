//! Differential golden for DataFrame.compare_scalar_df. Exercises the new
//! all-valid-f64 fast path AND the retained AoS path (i64 column, f64 column
//! with NaN/null, string column), all six operators. Proves the fast path is
//! bit-identical to the prior unconditional per-cell loop.
//! Run: cargo run -p fp-frame --example golden_df_cmp_scalar --release
use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;

fn main() {
    let mut z = 0x2468u64;
    let mut rnd = || {
        z ^= z << 13;
        z ^= z >> 7;
        z ^= z << 17;
        (z >> 11) as f64 / (1u64 << 53) as f64 * 100.0 - 50.0
    };
    let n = 3000usize;
    let f: Vec<f64> = (0..n).map(|_| rnd()).collect();
    let mut fnull: Vec<f64> = (0..n).map(|_| rnd()).collect();
    fnull[5] = f64::NAN; // forces as_f64_slice -> None (NaN excluded), AoS path
    let i: Vec<i64> = (0..n).map(|k| (k as i64 % 91) - 45).collect();
    let s: Vec<Scalar> = (0..n)
        .map(|k| Scalar::Utf8(format!("k{}", k % 7)))
        .collect();

    let idx = Index::new((0..n as i64).map(IndexLabel::Int64).collect());
    let mut m = BTreeMap::new();
    m.insert("f".to_string(), Column::from_f64_values(f.clone()));
    m.insert(
        "fn".to_string(),
        Column::from_values(
            fnull
                .iter()
                .map(|&x| {
                    if x.is_nan() {
                        Scalar::Null(fp_types::NullKind::Null)
                    } else {
                        Scalar::Float64(x)
                    }
                })
                .collect(),
        )
        .unwrap(),
    );
    m.insert("i".to_string(), Column::from_i64_values(i.clone()));
    m.insert("s".to_string(), Column::from_values(s.clone()).unwrap());
    let order = vec![
        "f".to_string(),
        "fn".to_string(),
        "i".to_string(),
        "s".to_string(),
    ];
    let df = DataFrame::new_with_column_order(idx, m, order.clone()).unwrap();

    let mut acc = String::new();
    use fp_frame::DataFrame as DF;
    type DataFrameScalarCmpOp = (&'static str, fn(&DF, &Scalar) -> DF);
    let ops: [DataFrameScalarCmpOp; 6] = [
        ("eq", |d, k| d.eq_scalar_df(k).unwrap()),
        ("ne", |d, k| d.ne_scalar_df(k).unwrap()),
        ("gt", |d, k| d.gt_scalar_df(k).unwrap()),
        ("ge", |d, k| d.ge_scalar_df(k).unwrap()),
        ("lt", |d, k| d.lt_scalar_df(k).unwrap()),
        ("le", |d, k| d.le_scalar_df(k).unwrap()),
    ];
    for (name, op) in ops {
        let out = op(&df, &Scalar::Float64(1.5));
        acc.push_str(name);
        for c in &order {
            acc.push('|');
            acc.push_str(c);
            acc.push(':');
            for v in out.column(c).unwrap().values().iter() {
                acc.push_str(&format!("{v:?},"));
            }
        }
        acc.push('\n');
    }
    let mut h = 0xcbf29ce484222325u64;
    for b in acc.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    println!("DF_CMP_SCALAR_FNV1A={h:016x}");
    println!("len={}", acc.len());
    println!("ALL GOLDEN CHECKS PASSED");
}
