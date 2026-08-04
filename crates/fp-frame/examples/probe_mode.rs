use std::{hint::black_box, time::Instant};

use fp_columnar::Column;
use fp_frame::Series;
use fp_index::{Index, IndexLabel};
fn main() {
    let n = 1_000_000usize;
    let cats = 100i64;
    let mut z = 0x1u64;
    let vals: Vec<i64> = (0..n)
        .map(|_| {
            z ^= z << 13;
            z ^= z >> 7;
            z ^= z << 17;
            (z % cats as u64) as i64
        })
        .collect();
    let s = Series::new(
        "c".to_string(),
        Index::new((0..n as i64).map(IndexLabel::Int64).collect()),
        Column::from_i64_values(vals),
    )
    .unwrap();
    for _ in 0..3 {
        let m = s.mode().unwrap();
        black_box(m.len());
    }
    let it = 10;
    let st = Instant::now();
    let mut k = 0usize;
    for _ in 0..it {
        let m = s.mode().unwrap();
        k ^= m.len();
        black_box(&m);
    }
    println!(
        "Series.mode int64: {:.2} ms (k={k})",
        st.elapsed().as_secs_f64() * 1000.0 / it as f64
    );
}
