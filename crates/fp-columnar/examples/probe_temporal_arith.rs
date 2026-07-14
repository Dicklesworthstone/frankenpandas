//! Probe: does temporal col-vs-col arithmetic (dt-dt, td-td, td+td) error or work,
//! and is dt-dt exact + correctly typed Timedelta64? How slow?
//! Run: cargo run -p fp-columnar --release --example probe_temporal_arith -- 5000000 20

use fp_columnar::Column;
use fp_types::{DType, Scalar};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20);

    let base: i64 = 1_600_000_000_000_000_000;
    let mut state: u64 = 0x7EAA_1AA5;
    let end: Vec<i64> = (0..n)
        .map(|_| {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            base + ((state >> 20) % 10_000_000) as i64
        })
        .collect();
    let start: Vec<i64> = (0..n)
        .map(|_| {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            base + ((state >> 20) % 10_000_000) as i64
        })
        .collect();
    let dt_end = Column::from_datetime64_values(end.clone());
    let dt_start = Column::from_datetime64_values(start.clone());

    // dt - dt -> Timedelta64
    match dt_end.sub(&dt_start) {
        Ok(out) => println!(
            "dt - dt -> dtype={:?} (want Timedelta64), len={}",
            out.dtype(),
            out.len()
        ),
        Err(e) => println!("dt - dt ERRORED: {e:?}"),
    }

    // Exactness near 1.6e18: (base+5) - base = 5 ns.
    let a = Column::from_datetime64_values(vec![base + 5, base + 1]);
    let b = Column::from_datetime64_values(vec![base, base + 3]);
    match a.sub(&b) {
        Ok(out) => println!("(base+5,base+1) - (base,base+3) = {:?} (want [Td 5, Td -2])", out.values()),
        Err(e) => println!("exact dt-dt ERRORED: {e:?}"),
    }

    // td +/- td
    let td_a = Column::from_timedelta64_values_with_validity(
        vec![10, 20, 30],
        fp_columnar::ValidityMask::all_valid(3),
    );
    let td_b = Column::from_timedelta64_values_with_validity(
        vec![1, 2, 3],
        fp_columnar::ValidityMask::all_valid(3),
    );
    match (td_a.add(&td_b), td_a.sub(&td_b)) {
        (Ok(s), Ok(d)) => println!("td+td={:?} td-td={:?}", s.values(), d.values()),
        (a, b) => println!("td arith err: add={a:?} sub={b:?}"),
    }

    // Mixed dt ± td -> Datetime64.
    let dtm = Column::from_datetime64_values(vec![base, base + 100]);
    let tdm = Column::from_timedelta64_values_with_validity(
        vec![10, 20],
        fp_columnar::ValidityMask::all_valid(2),
    );
    match dtm.add(&tdm) {
        Ok(o) => println!("dt + td -> dtype={:?} vals={:?} (want Datetime64 [base+10, base+120])", o.dtype(), o.values()),
        Err(e) => println!("dt + td ERRORED: {e:?}"),
    }
    match dtm.sub(&tdm) {
        Ok(o) => println!("dt - td -> dtype={:?} (want Datetime64)", o.dtype()),
        Err(e) => println!("dt - td ERRORED: {e:?}"),
    }
    match tdm.add(&dtm) {
        Ok(o) => println!("td + dt -> dtype={:?} (want Datetime64)", o.dtype()),
        Err(e) => println!("td + dt ERRORED: {e:?}"),
    }

    // These must STILL error (pandas raises).
    println!("dt + dt is_err = {} (want true)", dt_end.add(&dt_start).is_err());
    println!("td - dt is_err = {} (want true)", tdm.sub(&dtm).is_err());

    // Timing.
    if dt_end.sub(&dt_start).map(|c| c.dtype() == DType::Timedelta64).unwrap_or(false) {
        let mut best = u128::MAX;
        for _ in 0..iters {
            let t = std::time::Instant::now();
            let out = dt_end.sub(&dt_start).unwrap();
            best = best.min(t.elapsed().as_nanos());
            std::hint::black_box(&out);
        }
        // hand-rolled ideal (raw saturating_sub into a Timedelta64 col)
        let mut best_ideal = u128::MAX;
        for _ in 0..iters {
            let t = std::time::Instant::now();
            let d: Vec<i64> = end.iter().zip(&start).map(|(&a, &b)| a.saturating_sub(b)).collect();
            let out = Column::from_timedelta64_values_with_validity(d, fp_columnar::ValidityMask::all_valid(n));
            best_ideal = best_ideal.min(t.elapsed().as_nanos());
            std::hint::black_box(&out);
        }
        let _ = Scalar::Int64(0);
        println!(
            "timing dt-dt: NEW={:.2}ms IDEAL={:.2}ms ({:.1}x) ({n} rows)",
            best as f64 / 1e6,
            best_ideal as f64 / 1e6,
            best as f64 / best_ideal as f64,
        );
    }
}
