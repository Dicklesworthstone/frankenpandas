//! Probe a batch of Series.str.* ops on a 1M contiguous Utf8 Series.
//! Run: bench_str_probe <n> <op>
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
    let op = a.get(2).map(String::as_str).unwrap_or("len");
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    // Mixed-case words with separators, e.g. "Word_abcd_00042".
    let s = Series::new(
        "v",
        Index::new(labels),
        contig(n, |i| {
            let w = sm(i, 0) % 26;
            format!(
                "Word_{:04x}_{:05}",
                sm(i, 1) % 0xffff,
                w * 1000 + (i as u64 % 1000)
            )
        }),
    )
    .unwrap();
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        let r = match op {
            "len" => s.str().len(),
            "upper" => s.str().upper(),
            "lower" => s.str().lower(),
            "contains" => s.str().contains("abc"),
            "startswith" => s.str().startswith("Word"),
            "endswith" => s.str().endswith("042"),
            "slice" => s.str().slice(Some(0), Some(5), None),
            "pad" => s.str().pad(24, "left", '*'),
            "zfill" => s.str().zfill(24),
            "splitget" => s.str().split_get("_", 1),
            "count" => s.str().count("o"),
            "capitalize" => s.str().capitalize(),
            "title" => s.str().title(),
            "replace" => s.str().replace("Word", "Term"),
            _ => panic!("op"),
        };
        std::hint::black_box(r.unwrap());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("str_{op} n={n}: best={best}ns");
}
