//! format_datetime_ns over 1M datetime64[ns] values. bench_fmt_datetime <n>
use fp_index::format_datetime_ns;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let sub: bool = a.get(2).map(|s| s == "sub").unwrap_or(false);
    let base = 946_684_800_000_000_000i64; // 2000-01-01
    let step = if sub {
        37_000_000_123i64
    } else {
        37_000_000_000i64
    };
    let nanos: Vec<i64> = (0..n as i64).map(|i| base + i * step).collect();
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        let mut acc = 0usize;
        for &ns in &nanos {
            acc += format_datetime_ns(ns).len();
        }
        std::hint::black_box(acc);
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!(
        "format_datetime_ns n={n} sub={sub}: best={best}ns ({:.2}ms)",
        best as f64 / 1e6
    );
}
