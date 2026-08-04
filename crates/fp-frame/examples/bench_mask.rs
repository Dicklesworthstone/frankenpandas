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
    for _ in 0..6 {
        let t = std::time::Instant::now();
        f();
        b = b.min(t.elapsed().as_nanos());
    }
    println!("{l}: {:.2}ms", b as f64 / 1e6);
}
fn main() {
    let n = 1_000_000usize;
    let idx = Index::from_range(0, n as i64, 1);
    let mkdf = |seed: u64, bools: bool| {
        let mut m = BTreeMap::new();
        let mut o = vec![];
        for c in 0..5 {
            let nm = format!("c{c}");
            let col: Vec<Scalar> = if bools {
                (0..n)
                    .map(|i| Scalar::Bool(sm(i, c as u64 + seed).is_multiple_of(2)))
                    .collect()
            } else {
                (0..n)
                    .map(|i| Scalar::Float64((sm(i, c as u64 + seed) % 1000) as f64))
                    .collect()
            };
            m.insert(nm.clone(), Column::from_values(col).unwrap());
            o.push(nm);
        }
        DataFrame::new_with_column_order(idx.clone(), m, o).unwrap()
    };
    let df = mkdf(1, false);
    let cond = mkdf(2, true);
    let other = mkdf(3, false);
    timeit("df.mask(cond_df, other_df) aligned", || {
        std::hint::black_box(df.mask_df_other(&cond, &other).unwrap());
    });
}
