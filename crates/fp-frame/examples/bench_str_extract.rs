//! Series.str.extract(regex) over a 1M contiguous Utf8 series.
//! Run: bench_str_extract <n>
use fp_columnar::Column;
use fp_frame::Series;
use fp_index::{Index, IndexLabel};

fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 31;
    h
}
fn contig(n: usize, f: impl Fn(usize) -> String) -> Column {
    let mut bytes = Vec::new();
    let mut offsets = Vec::with_capacity(n + 1);
    offsets.push(0usize);
    for i in 0..n {
        bytes.extend_from_slice(f(i).as_bytes());
        offsets.push(bytes.len());
    }
    Column::from_utf8_contiguous(bytes, offsets)
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    // "item000123_tail" — extract the digit run.
    let s = Series::new(
        "v",
        Index::new(labels),
        contig(n, |i| format!("item{:06}_tail", sm(i, 0) % 1_000_000)),
    )
    .unwrap();
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        std::hint::black_box(s.str().extract(r"item(\d+)").unwrap());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("str_extract n={n}: best={best}ns");
}
