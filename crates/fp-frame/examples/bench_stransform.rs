//! Series transforms over a 1M f64 column. Run: bench_stransform <n> <op>
use fp_columnar::Column;
use fp_frame::Series;
use fp_index::{Index, IndexLabel};

fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 31;
    h
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let op = a.get(2).map(String::as_str).unwrap_or("cumsum");
    // Lazy unit-range Int64 index (O(1) clone), matching pandas' default
    // RangeIndex — a materialized Index::new(Vec<IndexLabel>) would tax every
    // transform with a 1M-enum index clone pandas never pays.
    let idx = if a.get(3).map(String::as_str) == Some("mat") {
        Index::new((0..n as i64).map(IndexLabel::Int64).collect())
    } else {
        Index::new_known_unique_int64_unit_range(0, n)
    };
    let s = Series::new(
        "v",
        idx,
        Column::from_f64_values(
            (0..n)
                .map(|i| (sm(i, 0) % 100_000) as f64 - 50_000.0)
                .collect(),
        ),
    )
    .unwrap();
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        // Isolation probes (no Series/Column): measure just the work to attribute
        // cumsum's cost between the prefix-scan and the output construction.
        if op == "rawscan" {
            let data = s.column().as_f64_slice().unwrap();
            let mut acc = 0.0_f64;
            let out: Vec<f64> = data
                .iter()
                .map(|&v| {
                    acc += v;
                    acc
                })
                .collect();
            std::hint::black_box(&out);
            let e = t.elapsed().as_nanos();
            if e < best {
                best = e;
            }
            continue;
        }
        if op == "rawscan_arc" {
            let data = s.column().as_f64_slice().unwrap();
            let mut acc = 0.0_f64;
            let out: Vec<f64> = data
                .iter()
                .map(|&v| {
                    acc += v;
                    acc
                })
                .collect();
            let arc: std::sync::Arc<[f64]> = std::sync::Arc::from(out);
            std::hint::black_box(&arc);
            let e = t.elapsed().as_nanos();
            if e < best {
                best = e;
            }
            continue;
        }
        let r = match op {
            "cumsum" => s.cumsum(),
            "cummax" => s.cummax(),
            "cummin" => s.cummin(),
            "cumprod" => s.cumprod(),
            "diff" => s.diff(1),
            "pct_change" => s.pct_change(1),
            "shift" => s.shift(1),
            "clip" => s.clip(Some(-1000.0), Some(1000.0)),
            "rank" => s.rank("average", true, "keep"),
            "abs" => s.abs(),
            "round" => s.round(2),
            "sign" => s.sign(),
            "floor" => s.floor(),
            "ceil" => s.ceil(),
            "trunc" => s.trunc(),
            "sqrt" => s.sqrt(),
            "exp" => s.exp(),
            "log" => s.log(),
            _ => panic!("op"),
        };
        std::hint::black_box(r.unwrap());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("stransform_{op} n={n}: best={best}ns");
}
