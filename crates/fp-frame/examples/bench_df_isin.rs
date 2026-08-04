//! DataFrame.isin(list) over Utf8 columns. Run: -- 1000000 4 50 1000
use std::{collections::BTreeMap, time::Instant};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let k: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(4);
    let card: i64 = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(50);
    let setsz: i64 = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(1000);
    let it: usize = a.get(5).and_then(|s| s.parse().ok()).unwrap_or(10);
    let index = Index::new((0..n as i64).map(IndexLabel::Int64).collect());
    let mut cols = BTreeMap::new();
    let mut order = Vec::new();
    for c in 0..k {
        let nm = format!("c{c}");
        cols.insert(
            nm.clone(),
            Column::from_values(
                (0..n)
                    .map(|i| {
                        Scalar::Utf8(format!(
                            "v{:06}",
                            ((i as i64).wrapping_mul(2654435761 + c as i64) >> 13) % card
                        ))
                    })
                    .collect(),
            )
            .unwrap(),
        );
        order.push(nm);
    }
    let df = DataFrame::new_with_column_order(index, cols, order).unwrap();
    let needles: Vec<Scalar> = (0..setsz)
        .map(|x| Scalar::Utf8(format!("v{:06}", x)))
        .collect();
    let mut best = u128::MAX;
    for _ in 0..it {
        let t = Instant::now();
        std::hint::black_box(df.isin(&needles).unwrap());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("df_isin_utf8 n={n} k={k} card={card} setsz={setsz}: best={best}ns");
}
