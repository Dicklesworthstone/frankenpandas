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
fn mkdf(n: usize, off: i64, ncols: usize) -> DataFrame {
    let idx = Index::from_range(off, off + n as i64, 1);
    let mut m = BTreeMap::new();
    let mut order = vec![];
    for c in 0..ncols {
        let nm = format!("c{c}");
        let col: Vec<Scalar> = (0..n)
            .map(|i| {
                if sm(i, 3).is_multiple_of(4) {
                    Scalar::Null(fp_types::NullKind::Null)
                } else {
                    Scalar::Float64((sm(i, c as u64 + 1) % 1000) as f64)
                }
            })
            .collect();
        m.insert(nm.clone(), Column::from_values(col).unwrap());
        order.push(nm);
    }
    DataFrame::new_with_column_order(idx, m, order).unwrap()
}
fn main() {
    let n = 1_000_000usize;
    let a = mkdf(n, 0, 5);
    let b_aligned = mkdf(n, 0, 5);
    let b_shift = mkdf(n, 500_000, 5);
    timeit("df.add aligned (5col,1M)", || {
        std::hint::black_box(a.add(&b_aligned).unwrap());
    });
    timeit("df.add unaligned-shift (5col,1M)", || {
        std::hint::black_box(a.add(&b_shift).unwrap());
    });
    timeit("df.add_fill unaligned-shift (5col,1M)", || {
        std::hint::black_box(a.add_df_fill(&b_shift, 0.0).unwrap());
    });
    // concat many small frames
    let frames: Vec<DataFrame> = (0..200).map(|i| mkdf(5000, (i * 5000) as i64, 5)).collect();
    let refs: Vec<&DataFrame> = frames.iter().collect();
    timeit("concat 200x(5000x5col)", || {
        std::hint::black_box(fp_frame::concat_dataframes(&refs).unwrap());
    });
}
