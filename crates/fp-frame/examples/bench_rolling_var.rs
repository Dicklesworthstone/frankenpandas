//! Bench + golden digest for Series::rolling().{sum,mean,var,std} on all-valid
//! Float64 input.
//!
//! Run: cargo run -p fp-frame --example bench_rolling_var --release
//!
//! `apply_rolling` previously rebuilt a fresh `Vec<f64>` for every window via
//! `window_values` (a `filter_map` over `Scalar::to_f64`). For an all-valid
//! Float64 column the window IS a contiguous slice of the typed buffer, so the
//! per-window allocation and per-element Scalar dispatch are pure overhead. The
//! typed fast path feeds `&data[start..end]` straight to the aggregation
//! closure. var/std are two-pass per window (mean, then sum of squares) so the
//! win compounds with window size.
//!
//! The golden battery pins the exact f64 bits across windows, min_periods, and
//! centered/trailing variants; the randomized cross-check compares the API
//! against an independent verbatim per-window reference (bit-for-bit).

use std::time::Instant;

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::Scalar;

fn s_from(vals: &[f64]) -> Series {
    let idx: Vec<IndexLabel> = (0..vals.len() as i64).map(IndexLabel::Int64).collect();
    let sc: Vec<Scalar> = vals.iter().map(|&v| Scalar::Float64(v)).collect();
    Series::from_values("s", idx, sc).unwrap()
}

// Only the aggregations whose back end is `apply_rolling` — the function this
// lever touches. (Rolling sum/mean route to slt1p's O(n) online sweep and are
// deliberately NOT bit-identical to the naive fold, so they are excluded from
// this golden.)
#[derive(Clone, Copy)]
enum Agg {
    Var,
    Std,
    Skew,
    Kurt,
    Sem,
    Prod,
}

fn agg_name(a: Agg) -> &'static str {
    match a {
        Agg::Var => "var",
        Agg::Std => "std",
        Agg::Skew => "skew",
        Agg::Kurt => "kurt",
        Agg::Sem => "sem",
        Agg::Prod => "prod",
    }
}

const ALL_AGGS: [Agg; 6] = [
    Agg::Var,
    Agg::Std,
    Agg::Skew,
    Agg::Kurt,
    Agg::Sem,
    Agg::Prod,
];

fn run_api(s: &Series, window: usize, min_periods: Option<usize>, center: bool, a: Agg) -> Series {
    let r = if center {
        s.rolling_center(window, min_periods)
    } else {
        s.rolling(window, min_periods)
    };
    match a {
        Agg::Var => r.var(),
        Agg::Std => r.std(),
        Agg::Skew => r.skew(),
        Agg::Kurt => r.kurt(),
        Agg::Sem => r.sem(),
        Agg::Prod => r.prod(),
    }
    .unwrap()
}

/// Independent verbatim reference for the all-valid window aggregation. Mirrors
/// the original generic `apply_rolling` semantics exactly (collect window, gate
/// on min_periods by length, apply the same closures in the same fold order).
fn ref_values(vals: &[f64], window: usize, min_periods: usize, center: bool, a: Agg) -> Vec<f64> {
    let len = vals.len();
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        let (start, end) = if center {
            let half = window / 2;
            let start = i.saturating_sub(half);
            let end = (i + half + window % 2).min(len);
            (start, end)
        } else {
            ((i + 1).saturating_sub(window), i + 1)
        };
        let nums = &vals[start..end];
        if nums.len() < min_periods {
            out.push(f64::NAN); // sentinel "null"; compared as null below
            continue;
        }
        // Verbatim reference only for var/std (formulas the bench owns); other
        // aggregations are proven by the before==after FNV comparison instead.
        let v = match a {
            Agg::Var => {
                if nums.len() < 2 {
                    f64::NAN
                } else {
                    let mean = nums.iter().sum::<f64>() / nums.len() as f64;
                    nums.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (nums.len() - 1) as f64
                }
            }
            Agg::Std => {
                if nums.len() < 2 {
                    f64::NAN
                } else {
                    let mean = nums.iter().sum::<f64>() / nums.len() as f64;
                    (nums.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (nums.len() - 1) as f64)
                        .sqrt()
                }
            }
            _ => unreachable!("ref_values only covers var/std"),
        };
        out.push(v);
    }
    out
}

fn fmt_series(s: &Series) -> String {
    let mut out = String::new();
    for v in s.values() {
        match v {
            Scalar::Float64(f) => {
                if f.is_nan() {
                    out.push_str("nan ");
                } else {
                    out.push_str(&format!("{:016x} ", f.to_bits()));
                }
            }
            Scalar::Null(_) => out.push_str("null "),
            other => out.push_str(&format!("?{other:?} ")),
        }
    }
    out
}

fn golden() -> String {
    let mut out = String::new();
    let data: &[&[f64]] = &[
        &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0],
        &[3.5, -1.0, 2.25, 0.0, 9.9, -4.4, 8.1, 2.2, 7.7, -3.3],
        &[1e8, 1.0, 1e-8, -1e8, 42.0],
    ];
    for (di, d) in data.iter().enumerate() {
        let s = s_from(d);
        for &window in &[2usize, 3, 5] {
            for &mp in &[None, Some(1usize), Some(window)] {
                for &center in &[false, true] {
                    for a in ALL_AGGS {
                        let res = run_api(&s, window, mp, center, a);
                        out.push_str(&format!(
                            "d{di} w{window} mp{mp:?} c{center} {}: {}\n",
                            agg_name(a),
                            fmt_series(&res)
                        ));
                    }
                }
            }
        }
    }
    out
}

fn cross_check() -> (usize, usize) {
    let mut state: u64 = 0x9E3779B97F4A7C15;
    let mut next = || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        state
    };
    let (mut ok, mut bad) = (0usize, 0usize);
    for _ in 0..3000 {
        let n = (next() % 60) as usize + 1;
        let xs: Vec<f64> = (0..n)
            .map(|_| (next() % 20000) as f64 / 100.0 - 100.0)
            .collect();
        let window = (next() % 8) as usize + 1;
        let mp_pick = next() % 3;
        let mp = match mp_pick {
            0 => None,
            1 => Some(1usize),
            _ => Some(window),
        };
        let center = next() % 2 == 0;
        let a = if next() % 2 == 0 { Agg::Var } else { Agg::Std };
        let s = s_from(&xs);
        let got = run_api(&s, window, mp, center, a);
        let mp_eff = mp.unwrap_or(window);
        let want = ref_values(&xs, window, mp_eff, center, a);
        let got_vals = got.values();
        let mut row_ok = got_vals.len() == want.len();
        if row_ok {
            for (g, w) in got_vals.iter().zip(want.iter()) {
                let matches = match g {
                    Scalar::Null(_) => w.is_nan() && mp_eff > 0, // min_periods-null sentinel
                    Scalar::Float64(f) => f.to_bits() == w.to_bits() || (f.is_nan() && w.is_nan()),
                    _ => false,
                };
                if !matches {
                    row_ok = false;
                    break;
                }
            }
        }
        if row_ok {
            ok += 1;
        } else {
            bad += 1;
            if bad <= 3 {
                eprintln!(
                    "MISMATCH n={n} w={window} mp={mp:?} c={center} agg={}",
                    agg_name(a)
                );
            }
        }
    }
    (ok, bad)
}

/// FNV-1a 64-bit over the full golden battery — a stable parity fingerprint
/// that survives rch's tail-truncated output (the verbose per-row dump can
/// scroll off; this single line cannot).
fn fnv1a64(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn main() {
    let g = golden();
    println!("GOLDEN_FNV1A64 {:016x} len={}", fnv1a64(&g), g.len());

    let (ok, bad) = cross_check();
    println!("CROSSCHECK ok={ok} bad={bad}");

    // Large all-valid Float64 series; moderate window so the per-window
    // two-pass var/std dominates. Deterministic data.
    let n: usize = 200_000;
    let window: usize = 250;
    let xs: Vec<f64> = (0..n)
        .map(|i| ((i as f64) * 0.31).sin() * 100.0 + ((i % 997) as f64))
        .collect();
    let s = s_from(&xs);

    for a in [Agg::Var, Agg::Std, Agg::Skew, Agg::Sem] {
        let t = Instant::now();
        let res = run_api(&s, window, None, false, a);
        let d = t.elapsed();
        // touch output so it is not optimized away
        let last = res.values().last().cloned();
        println!(
            "TIMING n={n} window={window} {}={:.3}ms last={:?}",
            agg_name(a),
            d.as_secs_f64() * 1e3,
            last
        );
    }
}
