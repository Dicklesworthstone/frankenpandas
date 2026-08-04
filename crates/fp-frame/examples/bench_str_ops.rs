//! Series.str.{contains,replace,contains_regex,replace_regex_all,count,split_count} @1M.
//! Run: bench_str_ops <n> <op>
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
    let op = a.get(2).map(String::as_str).unwrap_or("contains");
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    // "user_000123-name_foo.bar" style — words, digits, separators
    let s = Series::new(
        "v",
        Index::new(labels),
        contig(n, |i| {
            format!(
                "user_{:06}-name_{}.bar",
                sm(i, 0) % 1_000_000,
                sm(i, 1) % 9999
            )
        }),
    )
    .unwrap();
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        let st = s.str();
        let r = match op {
            "contains" => st.contains("name").map(|_| ()),
            "replace" => st.replace("name", "X").map(|_| ()),
            "contains_regex" => st.contains_regex(r"\d{3}").map(|_| ()),
            "replace_regex" => st.replace_regex_all(r"\d+", "#").map(|_| ()),
            "count" => st.count("a").map(|_| ()),
            "split_count" => st.split_count("_").map(|_| ()),
            _ => panic!("op"),
        };
        let _: () = r.unwrap();
        std::hint::black_box(());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("str_{op} n={n}: best={best}ns");
}
