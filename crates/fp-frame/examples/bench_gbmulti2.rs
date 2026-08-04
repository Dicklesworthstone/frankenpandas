//! df.groupby([a,b]).agg([sum,mean,std,count]) two-Utf8-key multi-agg. Run: -- 1000000 100 50 10
use std::{collections::BTreeMap, time::Instant};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let c1: i64 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(100);
    let c2: i64 = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(50);
    let it: usize = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(10);
    let index = Index::new((0..n as i64).map(IndexLabel::Int64).collect());
    let mut cols = BTreeMap::new();
    cols.insert(
        "a".to_string(),
        Column::from_values(
            (0..n)
                .map(|i| {
                    Scalar::Utf8(format!(
                        "a{:03}",
                        ((i as i64).wrapping_mul(2654435761) >> 13) % c1
                    ))
                })
                .collect(),
        )
        .unwrap(),
    );
    cols.insert(
        "b".to_string(),
        Column::from_values(
            (0..n)
                .map(|i| {
                    Scalar::Utf8(format!(
                        "b{:03}",
                        ((i as i64).wrapping_mul(40503) >> 7) % c2
                    ))
                })
                .collect(),
        )
        .unwrap(),
    );
    cols.insert(
        "v".to_string(),
        Column::from_values(
            (0..n)
                .map(|i| Scalar::Float64((i % 997) as f64 * 1.5))
                .collect(),
        )
        .unwrap(),
    );
    let df =
        DataFrame::new_with_column_order(index, cols, vec!["a".into(), "b".into(), "v".into()])
            .unwrap();
    let mut best = u128::MAX;
    for _ in 0..it {
        let t = Instant::now();
        std::hint::black_box(
            df.groupby(&["a", "b"])
                .unwrap()
                .agg_list(&["sum", "mean", "std", "count"])
                .unwrap(),
        );
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("gb_multiagg2_utf8 n={n} c1={c1} c2={c2}: best={best}ns");
}
