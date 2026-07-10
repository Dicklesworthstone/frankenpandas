use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::Index;
use fp_join::{JoinType, merge_dataframes_on};
use fp_types::Scalar;
fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^ (h >> 31)
}
// small frames, 3 keys with NEGATIVE values + moderate overlap so inner/left/outer all exercise matched+unmatched
fn mk(n: usize, seed: u64, payload: &str) -> DataFrame {
    let idx = Index::from_range(0, n as i64, 1);
    let mut m = BTreeMap::new();
    let mut order = vec![];
    // WIDE values (× large primes) so dense_packed_int64 overflows its bounded span
    // and the merge routes through typed_wide_multi_i64_key_positions; low cardinality
    // (7/5/4) keeps cross-side overlap. Includes NEGATIVES.
    let k0: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Int64(((sm(i, seed) % 7) as i64 - 3) * 1_000_003))
        .collect();
    let k1: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Int64(((sm(i, seed + 1) % 5) as i64 - 2) * 1_000_033))
        .collect();
    let k2: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Int64(((sm(i, seed + 2) % 4) as i64 - 1) * 1_000_037))
        .collect();
    let v: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Int64((sm(i, seed + 3) % 1000) as i64))
        .collect();
    for (nm, col) in [("k0", k0), ("k1", k1), ("k2", k2), (payload, v)] {
        m.insert(nm.to_string(), Column::from_values(col).unwrap());
        order.push(nm.to_string());
    }
    DataFrame::new_with_column_order(idx, m, order).unwrap()
}
fn dump(tag: &str, df: &fp_join::MergedDataFrame) {
    // dump rows as sorted multiset of (k0,k1,k2,lv,rv) tuples so order differences in
    // one-to-many groups don't cause false diffs; pandas & fp agree on the multiset.
    let n = df.index.labels().len();
    let get =
        |c: &str| -> Option<Vec<Scalar>> { df.columns.get(c).map(|col| col.values().to_vec()) };
    let k0 = get("k0").unwrap();
    let k1 = get("k1").unwrap();
    let k2 = get("k2").unwrap();
    let lv = get("lv");
    let rv = get("rv");
    let f = |v: &Scalar| -> String {
        match v {
            Scalar::Int64(x) => x.to_string(),
            Scalar::Float64(x) => {
                if x.is_nan() {
                    "NaN".into()
                } else {
                    format!("{x:.1}")
                }
            }
            Scalar::Null(_) => "NaN".into(),
            o => format!("{o:?}"),
        }
    };
    let mut rows: Vec<String> = (0..n)
        .map(|i| {
            let l = lv.as_ref().map(|c| f(&c[i])).unwrap_or("_".into());
            let r = rv.as_ref().map(|c| f(&c[i])).unwrap_or("_".into());
            format!("{}|{}|{}|{}|{}", f(&k0[i]), f(&k1[i]), f(&k2[i]), l, r)
        })
        .collect();
    rows.sort();
    println!("{tag} rows={n}");
    for r in &rows {
        println!("{r}");
    }
}
fn main() {
    let left = mk(40, 1, "lv");
    let right = mk(40, 2, "rv"); // different seed -> partial overlap, unmatched on both sides
    for (tag, jt) in [
        ("inner", JoinType::Inner),
        ("left", JoinType::Left),
        ("outer", JoinType::Outer),
    ] {
        dump(
            tag,
            &merge_dataframes_on(&left, &right, &["k0", "k1", "k2"], jt).unwrap(),
        );
    }
}
