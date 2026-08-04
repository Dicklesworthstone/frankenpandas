use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::Index;
use fp_types::{NullKind, Scalar};
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
    let n = 2_000_000usize;
    let idx = Index::from_range(0, n as i64, 1);
    let base = 1_600_000_000_000_000_000i64;
    let mut m = BTreeMap::new();
    m.insert(
        "t".to_string(),
        Column::from_datetime64_values((0..n).map(|i| base + i as i64 * 1_000_000_000).collect()),
    );
    // f column ~5% NaN -> dropna removes those rows, rest gathered as runs
    m.insert(
        "f".to_string(),
        Column::from_values(
            (0..n)
                .map(|i| {
                    if sm(i, 3).is_multiple_of(20) {
                        Scalar::Null(NullKind::NaN)
                    } else {
                        Scalar::Float64((sm(i, 2) % 1000) as f64)
                    }
                })
                .collect(),
        )
        .unwrap(),
    );
    let df = DataFrame::new_with_column_order(idx, m, vec!["t".into(), "f".into()]).unwrap();
    timeit("df.dropna 2M (datetime+f, 5% NaN)", || {
        std::hint::black_box(df.dropna().unwrap());
    });
}
