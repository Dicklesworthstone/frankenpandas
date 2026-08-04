//! Series.nunique on a single Float64 column, measurable vs pandas
//! `s.nunique()`. Run: cargo run -p fp-frame --example nunique_bench --release -- 1000000 100000 40

use std::time::Instant;

use fp_columnar::Column;
use fp_frame::Series;
use fp_index::Index;

fn best<F: FnMut()>(iters: usize, mut f: F) -> u128 {
    for _ in 0..3 {
        f();
    }
    let mut b = u128::MAX;
    for _ in 0..iters {
        let t = Instant::now();
        f();
        b = b.min(t.elapsed().as_nanos());
    }
    b
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let rows: usize = args
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);
    let distinct: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(100_000);
    let iters: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(40);

    let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    let dtype = args.get(4).map(String::as_str).unwrap_or("f64");
    let index = Index::new_known_unique_int64_unit_range(0, rows);
    let s = if dtype == "i64" {
        // Wide/sparse i64 ids: spread across the full i64 range so the dense
        // histogram path declines and the FxHashSet fast path is exercised.
        let data: Vec<i64> = (0..rows)
            .map(|_| (next() % distinct) as i64 * 0x1_0000_0001_i64)
            .collect();
        Series::new("c".to_string(), index, Column::from_i64_values(data)).expect("series")
    } else {
        // Float values drawn from `distinct` buckets (scaled) so distinct count is controlled.
        let data: Vec<f64> = (0..rows)
            .map(|_| (next() % distinct) as f64 * 1.5)
            .collect();
        Series::new("c".to_string(), index, Column::from_f64_values(data)).expect("series")
    };

    let op = args.get(5).map(String::as_str).unwrap_or("nunique");
    let ns = if op == "unique" {
        best(iters, || {
            std::hint::black_box(s.unique());
        })
    } else {
        best(iters, || {
            std::hint::black_box(s.nunique());
        })
    };
    eprintln!(
        "{op}_{dtype}: rows={rows} distinct={distinct} got={} best={:.1}us",
        s.nunique(),
        ns as f64 / 1000.0
    );
    println!("{:.1}", ns as f64 / 1000.0);
}
