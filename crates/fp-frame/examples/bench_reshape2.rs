use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::Index;
use fp_types::Scalar;
fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^ (h >> 31)
}
fn timeit(l: &str, mut f: impl FnMut()) {
    let mut b = u128::MAX;
    for _ in 0..5 {
        let t = std::time::Instant::now();
        f();
        b = b.min(t.elapsed().as_nanos());
    }
    println!("{l}: {:.2}ms", b as f64 / 1e6);
}
fn main() {
    let n = 500_000usize;
    let idx = Index::from_range(0, n as i64, 1);
    let mut m = BTreeMap::new();
    let mut o = vec![];
    m.insert(
        "id".into(),
        Column::from_values((0..n).map(|i| Scalar::Int64(i as i64)).collect()).unwrap(),
    );
    o.push("id".to_string());
    for c in 0..6 {
        let nm = format!("v{c}");
        m.insert(
            nm.clone(),
            Column::from_values(
                (0..n)
                    .map(|i| Scalar::Float64((sm(i, c as u64 + 1) % 1000) as f64))
                    .collect(),
            )
            .unwrap(),
        );
        o.push(nm);
    }
    let df = DataFrame::new_with_column_order(idx, m, o).unwrap();
    let vcols: Vec<String> = (0..6).map(|c| format!("v{c}")).collect();
    let vrefs: Vec<&str> = vcols.iter().map(|s| s.as_str()).collect();
    timeit("df.melt (500k x 6 valcols)", || {
        std::hint::black_box(df.melt(&["id"], &vrefs, None, None).unwrap());
    });
}
