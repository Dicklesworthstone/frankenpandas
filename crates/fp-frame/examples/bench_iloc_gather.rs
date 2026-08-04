//! Bench + golden digest for Series::iloc — typed gather lever.
//!
//! Run: cargo run -p fp-frame --example bench_iloc_gather --release
//!
//! iloc is a pure positional gather (no sort). It cloned a 32 B Scalar per row
//! and rebuilt the column via Column::from_values; routing through the typed
//! Column::take_positions keeps an all-valid Int64/Float64 buffer contiguous.
//! Output (values, dtype, negative-index handling) is bit-identical.

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
    let s = s_i64(vec![10, 20, 30, 40, 50]);
    // reorder + negative indices + duplicates
    let r = s.iloc(&[4, 0, -1, 2, -5, 2]).unwrap();
    out.push_str(&format!("i64_lbls={:?}\n", r.index().labels()));
    out.push_str(&format!("i64_vals={:?}\n", r.values()));

    // Float64 incl NaN
    let f = s_scalars(vec![
        Scalar::Float64(1.5),
        Scalar::Float64(f64::NAN),
        Scalar::Float64(-3.0),
    ]);
    out.push_str(&format!("f64={:?}\n", f.iloc(&[2, 1, 0]).unwrap().values()));

    // Nullable Int64 (Null present => not all-valid path)
    let ni = s_scalars(vec![
        Scalar::Int64(7),
        Scalar::Null(NullKind::NaN),
        Scalar::Int64(9),
    ]);
    out.push_str(&format!("ni={:?}\n", ni.iloc(&[1, 2, 0]).unwrap().values()));

    // Utf8 + Bool
    let u = s_scalars(
        vec!["a", "b", "c"]
            .into_iter()
            .map(|x| Scalar::Utf8(x.into()))
            .collect(),
    );
    out.push_str(&format!("utf8={:?}\n", u.iloc(&[2, -3]).unwrap().values()));
    let b = s_scalars(vec![Scalar::Bool(true), Scalar::Bool(false)]);
    out.push_str(&format!(
        "bool={:?}\n",
        b.iloc(&[1, 0, 1]).unwrap().values()
    ));

    // Out-of-bounds errors
    out.push_str(&format!("oob_err={}\n", s.iloc(&[99]).is_err()));
    out
}

fn main() {
    let g = golden();
    print!("GOLDEN_BEGIN\n{g}GOLDEN_END\n");

    let n: usize = 1_000_000;
    let s = s_i64((0..n as i64).map(|v| v * 2).collect());
    // Shuffled full permutation (LCG).
    let mut x: u64 = 0xdead_beef;
    let mut pos: Vec<i64> = (0..n as i64).collect();
    for i in (1..n).rev() {
        x = x
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let j = (x >> 16) as usize % (i + 1);
        pos.swap(i, j);
    }

    let _ = s.iloc(&pos).unwrap(); // warmup

    let t = Instant::now();
    let r = s.iloc(&pos).unwrap();
    let d = t.elapsed();
    assert_eq!(r.len(), n);

    println!("TIMING n={n} iloc={:.3}ms", d.as_secs_f64() * 1e3);
}
