//! Bench + golden digest for Series::sort_values — typed gather lever.
//!
//! Run: cargo run -p fp-frame --example bench_sort_values_gather --release
//!
//! sort_values' typed radix argsort is O(n), so the permutation GATHER
//! (reorder_by_positions) dominates. It cloned a 32 B Scalar per row and
//! re-validated in Column::new; routing through Column::take_positions keeps
//! the gather on the contiguous typed buffer. Output is bit-identical.

use std::time::Instant;

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::{NullKind, Scalar};

fn s_i64(vals: Vec<i64>) -> Series {
    let idx: Vec<IndexLabel> = (0..vals.len() as i64).map(IndexLabel::Int64).collect();
    Series::from_values("s", idx, vals.into_iter().map(Scalar::Int64).collect()).unwrap()
}

fn s_scalars(vals: Vec<Scalar>) -> Series {
    let idx: Vec<IndexLabel> = (0..vals.len() as i64).map(IndexLabel::Int64).collect();
    Series::from_values("s", idx, vals).unwrap()
}

fn golden() -> String {
    let mut out = String::new();
    // Int64 ascending/descending with ties (stable).
    let s = s_i64(vec![3, 1, 2, 1, 3]);
    out.push_str(&format!(
        "i64_asc={:?}\n",
        s.sort_values(true).unwrap().values()
    ));
    out.push_str(&format!(
        "i64_desc={:?}\n",
        s.sort_values(false).unwrap().values()
    ));

    // Float64 with NaN (NaN sorts last under na_position='last').
    let f = s_scalars(vec![
        Scalar::Float64(2.5),
        Scalar::Float64(f64::NAN),
        Scalar::Float64(-1.0),
        Scalar::Float64(2.5),
    ]);
    out.push_str(&format!(
        "f64_asc={:?}\n",
        f.sort_values(true).unwrap().values()
    ));

    // Nullable Int64 (Null mixed in) via na_position first/last.
    let ni = s_scalars(vec![
        Scalar::Int64(5),
        Scalar::Null(NullKind::NaN),
        Scalar::Int64(-2),
        Scalar::Int64(5),
    ]);
    out.push_str(&format!(
        "ni_last={:?}\n",
        ni.sort_values_na(true, "last").unwrap().values()
    ));
    out.push_str(&format!(
        "ni_first={:?}\n",
        ni.sort_values_na(true, "first").unwrap().values()
    ));

    // Utf8.
    let u = s_scalars(
        vec!["banana", "apple", "cherry", "apple"]
            .into_iter()
            .map(|x| Scalar::Utf8(x.to_string()))
            .collect(),
    );
    out.push_str(&format!(
        "utf8_asc={:?}\n",
        u.sort_values(true).unwrap().values()
    ));

    // Bool.
    let b = s_scalars(vec![
        Scalar::Bool(true),
        Scalar::Bool(false),
        Scalar::Bool(true),
    ]);
    out.push_str(&format!(
        "bool_asc={:?}\n",
        b.sort_values(true).unwrap().values()
    ));
    out
}

fn gen_i64(n: usize, modulo: i64) -> Vec<i64> {
    let mut x: u64 = 0x9e37_79b9;
    (0..n)
        .map(|_| {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (x >> 16) as i64 % modulo
        })
        .collect()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(String::as_str).unwrap_or("bench");

    if mode == "ab" {
        // Same-process A/B for the Int64 sort_values lever (br-frankenpandas-p4zpz):
        // inlined O(n log n) comparison argsort (the pre-lever path) vs the O(n)
        // stable `fp_columnar::radix_argsort_i64`. One process => reliable ratio
        // despite variable rch worker speed; the per-iter equality assert is the
        // isomorphism proof on 2M random values WITH heavy ties (modulo 1e6).
        let n: usize = args
            .get(2)
            .and_then(|s| s.parse().ok())
            .unwrap_or(2_000_000);
        let iters: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(15);
        let vals = gen_i64(n, 1_000_000);
        let (mut cmp_ns, mut rad_ns) = (0u128, 0u128);
        let (mut sink_a, mut sink_b) = (0usize, 0usize);
        for ascending in [true, false] {
            for _ in 0..iters {
                let t = Instant::now();
                let mut keyed: Vec<(usize, i64)> = vals.iter().copied().enumerate().collect();
                keyed.sort_by(|left, right| {
                    let order = left.1.cmp(&right.1);
                    if ascending { order } else { order.reverse() }
                });
                let order_cmp: Vec<usize> = keyed.into_iter().map(|(p, _)| p).collect();
                cmp_ns += t.elapsed().as_nanos();
                sink_a = sink_a.wrapping_add(order_cmp[0]);

                let t = Instant::now();
                let order_rad = fp_columnar::radix_argsort_i64(&vals, ascending);
                rad_ns += t.elapsed().as_nanos();
                sink_b = sink_b.wrapping_add(order_rad[0]);

                assert_eq!(
                    order_cmp, order_rad,
                    "ISOMORPHISM FAIL (ascending={ascending}): radix order != comparison order"
                );
            }
        }
        let cmp_ms = cmp_ns as f64 / 1e6 / (iters * 2) as f64;
        let rad_ms = rad_ns as f64 / 1e6 / (iters * 2) as f64;
        println!(
            "AB n={n} iters={iters}x2dir comparison={cmp_ms:.3}ms radix={rad_ms:.3}ms ratio={:.3}x (sink {sink_a}/{sink_b})",
            cmp_ms / rad_ms
        );
        return;
    }

    let g = golden();
    print!("GOLDEN_BEGIN\n{g}GOLDEN_END\n");

    let n: usize = 1_000_000;
    let vals = gen_i64(n, 1_000_000);
    let s = s_i64(vals);

    let _ = s.sort_values(true).unwrap(); // warmup

    let t = Instant::now();
    let r = s.sort_values(true).unwrap();
    let d = t.elapsed();
    assert_eq!(r.len(), n);

    println!("TIMING n={n} sort_values={:.3}ms", d.as_secs_f64() * 1e3);
}
