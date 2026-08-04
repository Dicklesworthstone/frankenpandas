//! groupby cumsum on a Utf8 key — DataFrameGroupBy + SeriesGroupBy. Run: -- 1000000 1000 12
use std::{collections::BTreeMap, time::Instant};

use fp_columnar::Column;
use fp_frame::{DataFrame, Series};
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;
fn best<F: FnMut()>(it: usize, mut f: F) -> u128 {
    let mut b = u128::MAX;
    for _ in 0..it {
        let t = Instant::now();
        f();
        let e = t.elapsed().as_nanos();
        if e < b {
            b = e;
        }
    }
    b
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let card: i64 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(1000);
    let it: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(12);
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let keys: Vec<Scalar> = (0..n)
        .map(|i| {
            Scalar::Utf8(format!(
                "k{:04}",
                ((i as i64).wrapping_mul(2654435761) >> 13) % card
            ))
        })
        .collect();
    let vals: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Float64((i % 997) as f64 * 1.5))
        .collect();
    // DataFrameGroupBy
    let mut cols = BTreeMap::new();
    cols.insert("k".to_string(), Column::from_values(keys.clone()).unwrap());
    cols.insert("v".to_string(), Column::from_values(vals.clone()).unwrap());
    let df = DataFrame::new_with_column_order(
        Index::new(labels.clone()),
        cols,
        vec!["k".into(), "v".into()],
    )
    .unwrap();
    let dfc = best(it, || {
        std::hint::black_box(df.groupby(&["k"]).unwrap().cumsum().unwrap());
    });
    // SeriesGroupBy
    let by = Series::from_values("k", labels.clone(), keys).unwrap();
    let v = Series::from_values("v", labels, vals).unwrap();
    let sgc = best(it, || {
        std::hint::black_box(v.groupby(&by).unwrap().cumsum().unwrap());
    });
    println!("cumsum_utf8 n={n} card={card}: df_gb={dfc}ns series_gb={sgc}ns");
}
