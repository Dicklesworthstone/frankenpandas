//! Bench: nullable-temporal Column::where_cond. A nullable Datetime64 self + temporal
//! scalar other had NO typed arm -> generic Vec<Scalar> select. NEW = typed i64-ns select.
//! Run: cargo run -p fp-columnar --release --example bench_where_temporal -- 5000000 20

use fp_columnar::{Column, ValidityMask};
use fp_types::{NullKind, Scalar};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20);

    let base: i64 = 1_600_000_000_000_000_000;
    let mut state: u64 = 0x03BE_5EED;
    let mut data = vec![0i64; n];
    let mut validity = ValidityMask::all_valid(n);
    let mut cbits = vec![false; n];
    for i in 0..n {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        if (state >> 40) % 4 == 0 {
            validity.set(i, false);
        } else {
            data[i] = base + ((state >> 20) % 10_000_000) as i64;
        }
        cbits[i] = (state >> 33) & 1 == 0;
    }
    let col = Column::from_datetime64_values_with_validity(data.clone(), validity.clone());
    let cond = Column::from_bool_values(cbits.clone());
    let other = Scalar::Datetime64(base);

    let got = col.where_cond(&cond, &other).unwrap();
    assert_eq!(got.dtype(), fp_types::DType::Datetime64);
    println!("where_cond(Datetime64) OK");

    // Reference = the OLD generic path.
    let scalars: Vec<Scalar> = (0..n)
        .map(|i| {
            if validity.get(i) {
                Scalar::Datetime64(data[i])
            } else {
                Scalar::Null(NullKind::NaT)
            }
        })
        .collect();

    let mut best_new = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let o = col.where_cond(&cond, &other).unwrap();
        best_new = best_new.min(t.elapsed().as_nanos());
        std::hint::black_box(&o);
    }
    let mut best_ref = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let out: Vec<Scalar> = scalars
            .iter()
            .zip(&cbits)
            .map(|(v, &c)| if c { v.clone() } else { other.clone() })
            .collect();
        let o = Column::from_values(out).unwrap();
        best_ref = best_ref.min(t.elapsed().as_nanos());
        std::hint::black_box(&o);
    }
    println!(
        "where_cond nullable-Datetime64 n={n} NEW={:.2}ms REF(materialize)={:.2}ms ({:.1}x)",
        best_new as f64 / 1e6,
        best_ref as f64 / 1e6,
        best_ref as f64 / best_new as f64,
    );
}
