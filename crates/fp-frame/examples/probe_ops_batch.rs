use std::{hint::black_box, time::Instant};

use fp_columnar::Column;
use fp_frame::Series;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;
fn bench(name: &str, it: usize, mut f: impl FnMut() -> usize) {
    for _ in 0..2 {
        black_box(f());
    }
    let st = Instant::now();
    let mut k = 0usize;
    for _ in 0..it {
        k ^= black_box(f());
    }
    println!(
        "{name}: {:.2} ms/call (k={k})",
        st.elapsed().as_secs_f64() * 1000.0 / it as f64
    );
}
fn main() {
    let n = 1_000_000usize;
    let mut z = 0x1u64;
    let v: Vec<f64> = (0..n)
        .map(|_| {
            z ^= z << 13;
            z ^= z >> 7;
            z ^= z << 17;
            (z >> 11) as f64 / (1u64 << 53) as f64 * 1e6
        })
        .collect();
    let s = Series::new(
        "c".to_string(),
        Index::new((0..n as i64).map(IndexLabel::Int64).collect()),
        Column::from_f64_values(v),
    )
    .unwrap();
    bench("rank_avg", 6, || {
        s.rank("average", true, "keep").unwrap().len()
    });
    bench("nlargest_100", 8, || s.nlargest(100).unwrap().len());
    bench("diff", 8, || s.diff(1).unwrap().len());
    bench("between", 8, || {
        s.between(
            &Scalar::Float64(250000.0),
            &Scalar::Float64(750000.0),
            "both",
        )
        .unwrap()
        .len()
    });
    bench("clip", 8, || {
        s.clip(Some(100000.0), Some(900000.0)).unwrap().len()
    });
}
