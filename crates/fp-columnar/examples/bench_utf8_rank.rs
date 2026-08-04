//! Bench for `Column::rank` on an all-valid contiguous-Utf8 column after adding the typed
//! Utf8 arm — ranks over raw `&[u8]` row spans via the stable MSD byte radix, instead of the
//! generic loop that materializes `Vec<Scalar::Utf8>` (a heap `String` per row) then runs an
//! O(n log n) `compare_scalars_na_last` Scalar sort.
//!
//! NEW = col.rank("average", true). CONTROL = a replica of the generic path over the (cached)
//! values() ⇒ conservative lower bound (control skips the String materialization).
//!
//! Run: cargo run -p fp-columnar --release --example bench_utf8_rank -- 2000000 20

use fp_columnar::Column;
use fp_types::{DType, Scalar};

fn ref_rank_average(vals: &[Scalar]) -> Column {
    let n = vals.len();
    let mut idx: Vec<usize> = (0..n).collect(); // all-valid ⇒ every row present
    idx.sort_by(|&a, &b| match (&vals[a], &vals[b]) {
        (Scalar::Utf8(x), Scalar::Utf8(y)) => x.cmp(y),
        _ => std::cmp::Ordering::Equal,
    });
    let mut ranks = vec![Scalar::Float64(0.0); n];
    let mut cursor = 0usize;
    while cursor < n {
        let mut end = cursor + 1;
        while end < n && vals[idx[end]] == vals[idx[cursor]] {
            end += 1;
        }
        let value = (cursor as f64 + 1.0 + end as f64) / 2.0;
        for &gi in &idx[cursor..end] {
            ranks[gi] = Scalar::Float64(value);
        }
        cursor = end;
    }
    Column::new(DType::Float64, ranks).unwrap()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(2_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20);

    // ~2000 distinct labels ⇒ meaningful tie groups; varied for a real sort.
    let mut bytes: Vec<u8> = Vec::with_capacity(n * 10);
    let mut offsets: Vec<usize> = Vec::with_capacity(n + 1);
    offsets.push(0);
    let mut state: u64 = 0x1234_9E37;
    for _ in 0..n {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let k = (state >> 33) % 2000;
        bytes.extend_from_slice(format!("label_{:05}", k).as_bytes());
        offsets.push(bytes.len());
    }
    let col = Column::from_utf8_contiguous(bytes, offsets);
    let vals = col.values().to_vec(); // warm the lazy Scalar-Vec cache for CONTROL

    let mut best_t = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = col.rank("average", true).unwrap();
        best_t = best_t.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let mut best_c = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let r = ref_rank_average(&vals);
        best_c = best_c.min(t.elapsed().as_nanos());
        std::hint::black_box(&r);
    }
    let got = col.rank("average", true).unwrap();
    let want = ref_rank_average(&vals);
    let gv = got.values();
    let wv = want.values();
    assert_eq!(gv.len(), wv.len());
    for k in 0..n {
        assert_eq!(
            format!("{:?}", gv.get(k)),
            format!("{:?}", wv.get(k)),
            "slot {k} mismatch"
        );
    }
    println!(
        "rank utf8_average n={n} NEW={:>7.2}ms CONTROL={:>7.2}ms speedup={:.3}x",
        best_t as f64 / 1e6,
        best_c as f64 / 1e6,
        best_c as f64 / best_t as f64,
    );
}
