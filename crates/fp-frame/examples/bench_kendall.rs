//! Bench + golden digest for Series::corr_kendall (Kendall tau-b) with ties.
//!
//! Run: cargo run -p fp-frame --example bench_kendall --release
//!
//! With ties the old path was an O(n²) all-pairs loop. Knight's algorithm
//! computes the identical tau-b in O(n log n). The golden battery pins the
//! exact f64 output on tied data so the fast path proves bit-identical.

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::Scalar;
use std::time::Instant;

fn s_from(vals: &[f64]) -> Series {
    let idx: Vec<IndexLabel> = (0..vals.len() as i64).map(IndexLabel::Int64).collect();
    let sc: Vec<Scalar> = vals.iter().map(|&v| Scalar::Float64(v)).collect();
    Series::from_values("s", idx, sc).unwrap()
}

fn golden() -> String {
    let mut out = String::new();
    let cases: &[(&[f64], &[f64])] = &[
        // ties in both x and y
        (&[1.0, 2.0, 2.0, 3.0, 3.0, 3.0], &[1.0, 1.0, 2.0, 2.0, 3.0, 3.0]),
        // joint ties (identical (x,y) pairs)
        (&[1.0, 1.0, 2.0, 2.0], &[5.0, 5.0, 9.0, 9.0]),
        // discordant-heavy with ties
        (&[1.0, 2.0, 3.0, 4.0, 4.0], &[5.0, 4.0, 3.0, 2.0, 2.0]),
        // all x tied -> denom NaN
        (&[7.0, 7.0, 7.0], &[1.0, 2.0, 3.0]),
        // mixed integers as floats, larger
        (
            &[1.0, 2.0, 2.0, 3.0, 1.0, 2.0, 3.0, 3.0, 1.0, 2.0],
            &[2.0, 2.0, 1.0, 3.0, 1.0, 2.0, 3.0, 1.0, 2.0, 3.0],
        ),
    ];
    for (i, (x, y)) in cases.iter().enumerate() {
        let a = s_from(x);
        let b = s_from(y);
        let tau = a.corr_kendall(&b).unwrap();
        out.push_str(&format!("case{i}: {:.17e}\n", tau));
    }
    out
}

/// Verbatim copy of the original O(n²) pairwise tau-b loop, as an independent
/// reference for the randomized cross-check.
fn kendall_ref(x: &[f64], y: &[f64]) -> f64 {
    let n = x.len();
    if n < 2 {
        return f64::NAN;
    }
    let (mut con, mut dis, mut tx, mut ty) = (0_i64, 0_i64, 0_i64, 0_i64);
    for i in 0..n {
        for j in (i + 1)..n {
            let dx = x[i] - x[j];
            let dy = y[i] - y[j];
            if dx.abs() < f64::EPSILON && dy.abs() < f64::EPSILON {
                tx += 1;
                ty += 1;
            } else if dx.abs() < f64::EPSILON {
                tx += 1;
            } else if dy.abs() < f64::EPSILON {
                ty += 1;
            } else if (dx > 0.0 && dy > 0.0) || (dx < 0.0 && dy < 0.0) {
                con += 1;
            } else {
                dis += 1;
            }
        }
    }
    let n_pairs = (n * (n - 1)) as f64 / 2.0;
    let denom = ((n_pairs - tx as f64) * (n_pairs - ty as f64)).sqrt();
    if denom < f64::EPSILON {
        f64::NAN
    } else {
        (con - dis) as f64 / denom
    }
}

fn cross_check() -> (usize, usize) {
    // LCG for deterministic pseudo-random integer-tied datasets.
    let mut state: u64 = 0x2545F4914F6CDD1D;
    let mut next = || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        state
    };
    let (mut ok, mut bad) = (0usize, 0usize);
    for _ in 0..4000 {
        let n = (next() % 30) as usize + 2;
        let range = (next() % 6) as u64 + 1; // small range -> many ties
        let xs: Vec<f64> = (0..n).map(|_| (next() % range) as f64).collect();
        let ys: Vec<f64> = (0..n).map(|_| (next() % range) as f64).collect();
        let got = s_from(&xs).corr_kendall(&s_from(&ys)).unwrap();
        let want = kendall_ref(&xs, &ys);
        // bit-for-bit (NaN compares equal by bits here since both produce the
        // canonical NaN from the same sqrt path).
        if got.to_bits() == want.to_bits() || (got.is_nan() && want.is_nan()) {
            ok += 1;
        } else {
            bad += 1;
            if bad <= 3 {
                eprintln!("MISMATCH n={n} got={got:?} want={want:?}");
            }
        }
    }
    (ok, bad)
}

fn main() {
    let g = golden();
    print!("GOLDEN_BEGIN\n{g}GOLDEN_END\n");

    let (ok, bad) = cross_check();
    println!("CROSSCHECK ok={ok} bad={bad}");

    // Large tied dataset: values in a small range force many ties, so the old
    // path takes the O(n²) loop (no-tie fast path bails on ties).
    let n: usize = 40_000;
    let xs: Vec<f64> = (0..n).map(|i| ((i * 7) % 50) as f64).collect();
    let ys: Vec<f64> = (0..n).map(|i| ((i * 13) % 40) as f64).collect();
    let a = s_from(&xs);
    let b = s_from(&ys);

    let t = Instant::now();
    let tau = a.corr_kendall(&b).unwrap();
    let d = t.elapsed();

    println!(
        "TIMING n={n} corr_kendall={:.3}ms tau={:.6}",
        d.as_secs_f64() * 1e3,
        tau
    );
}
