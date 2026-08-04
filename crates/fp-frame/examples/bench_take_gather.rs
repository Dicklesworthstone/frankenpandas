//! Bench + golden for Series::take and SeriesGroupBy gather — typed gather lever.
//!
//! Run: cargo run -p fp-frame --example bench_take_gather --release
//!
//! Series::take and the SeriesGroupBy take_positions (behind groupby
//! nlargest/nsmallest/head/tail/first/last) cloned a 32 B Scalar per row and
//! rebuilt via Column::from_values. Routing through the typed
//! Column::take_positions keeps the contiguous Int64/Float64 buffer. Output is
//! bit-identical (values, dtype, negative/dup indices, group order).

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

    // Series::take across dtypes, negative + duplicate indices.
    let s = s_i64(vec![10, 20, 30, 40, 50]);
    let r = s.take(&[4, 0, -1, 2, -5, 2]).unwrap();
    out.push_str(&format!("take_lbls={:?}\n", r.index().labels()));
    out.push_str(&format!("take_vals={:?}\n", r.values()));
    out.push_str(&format!("take_oob_err={}\n", s.take(&[99]).is_err()));

    let f = s_scalars(vec![
        Scalar::Float64(1.5),
        Scalar::Float64(f64::NAN),
        Scalar::Float64(-3.0),
    ]);
    out.push_str(&format!(
        "take_f64={:?}\n",
        f.take(&[2, 1, 0]).unwrap().values()
    ));
    let ni = s_scalars(vec![
        Scalar::Int64(7),
        Scalar::Null(NullKind::NaN),
        Scalar::Int64(9),
    ]);
    out.push_str(&format!(
        "take_ni={:?}\n",
        ni.take(&[1, 2, 0]).unwrap().values()
    ));
    let u = s_scalars(
        vec!["a", "b", "c"]
            .into_iter()
            .map(|x| Scalar::Utf8(x.into()))
            .collect(),
    );
    out.push_str(&format!(
        "take_utf8={:?}\n",
        u.take(&[2, -3]).unwrap().values()
    ));

    // SeriesGroupBy gather paths (nlargest / head) route through take_positions.
    let data = s_i64(vec![5, 1, 9, 3, 7, 2, 8]);
    let keys = s_scalars(
        vec!["a", "b", "a", "b", "a", "b", "a"]
            .into_iter()
            .map(|x| Scalar::Utf8(x.into()))
            .collect(),
    );
    let gb = data.groupby(&keys).unwrap();
    let nl = gb.nlargest(2).unwrap();
    out.push_str(&format!("gb_nlargest_lbls={:?}\n", nl.index().labels()));
    out.push_str(&format!("gb_nlargest_vals={:?}\n", nl.values()));
    let hd = data.groupby(&keys).unwrap().head(1).unwrap();
    out.push_str(&format!("gb_head_vals={:?}\n", hd.values()));
    out
}

fn main() {
    let g = golden();
    print!("GOLDEN_BEGIN\n{g}GOLDEN_END\n");

    let n: usize = 1_000_000;
    let s = s_i64((0..n as i64).map(|v| v * 2).collect());
    let mut x: u64 = 0xfeed_face;
    let idxs: Vec<i64> = (0..n)
        .map(|_| {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (x >> 16) as i64 % (n as i64)
        })
        .collect();

    let _ = s.take(&idxs).unwrap(); // warmup

    let t = Instant::now();
    let r = s.take(&idxs).unwrap();
    let d = t.elapsed();
    assert_eq!(r.len(), n);

    println!("TIMING n={n} take={:.3}ms", d.as_secs_f64() * 1e3);
}
