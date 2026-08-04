//! SeriesGroupBy first/last/max/min on a Utf8 value Series @1M.
//! Run: bench_sgb_str <n> <gcard> <op>
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
    let gc: u64 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(100);
    let op = a.get(3).map(String::as_str).unwrap_or("first");
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let by = Series::new(
        "k",
        Index::new(labels.clone()),
        contig(n, |i| format!("g{:04}", sm(i, 0) % gc)),
    )
    .unwrap();
    let v = Series::new(
        "v",
        Index::new(labels),
        contig(n, |i| format!("v{:08}", sm(i, 1))),
    )
    .unwrap();
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        let r = match op {
            "first" => v.groupby(&by).unwrap().first(),
            "last" => v.groupby(&by).unwrap().last(),
            "max" => v.groupby(&by).unwrap().max(),
            "min" => v.groupby(&by).unwrap().min(),
            "nunique" => v.groupby(&by).unwrap().nunique(),
            "vc" => v.groupby(&by).unwrap().value_counts(),
            "tfirst" => v.groupby(&by).unwrap().transform("first"),
            "tmax" => v.groupby(&by).unwrap().transform("max"),
            "tcount" => v.groupby(&by).unwrap().transform("count"),
            _ => panic!("op"),
        };
        std::hint::black_box(r.unwrap());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("sgb_str_{op} n={n} gc={gc}: best={best}ns");
}
