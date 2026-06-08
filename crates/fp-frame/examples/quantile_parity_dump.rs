//! Differential dump: FP Series.quantile across all 5 interpolation modes x
//! many q on small exactly-representable datasets, for comparison vs pandas
//! (Phase-B correctness hunt). Prints "QP d<i> q<q> <mode> <value>".
//!
//! Run: cargo run -p fp-frame --example quantile_parity_dump --release

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::Scalar;

fn s_from(vals: &[f64]) -> Series {
    let idx: Vec<IndexLabel> = (0..vals.len() as i64).map(IndexLabel::Int64).collect();
    let sc: Vec<Scalar> = vals.iter().map(|&v| Scalar::Float64(v)).collect();
    Series::from_values("s", idx, sc).unwrap()
}

fn main() {
    let datasets: &[&[f64]] = &[
        &[1.0, 2.0, 3.0, 4.0],
        &[1.0, 2.0, 3.0, 4.0, 5.0],
        &[10.0, 7.0, 3.0, 1.0, 9.0, 2.0],
        &[-3.0, -1.0, 0.0, 2.5, 4.0],
        &[5.0, 5.0, 5.0],
        &[1.0, 1.0, 2.0, 8.0],
        &[1.0, 2.0],
        &[42.0],
    ];
    // Keep the printed set small: rch tail-truncates stdout to ~40 lines.
    // q=0.3/0.7 never land on an element, maximally stressing interpolation.
    let qs = [0.3, 0.7];
    let modes = ["linear", "lower", "higher", "nearest", "midpoint"];
    for &i in &[0usize, 1, 5] {
        let d = datasets[i];
        let s = s_from(d);
        for &q in &qs {
            for m in modes {
                let v = match s.quantile_with_interpolation(q, m) {
                    Ok(Scalar::Float64(v)) => v,
                    Ok(other) => other.to_f64().unwrap_or(f64::NAN),
                    Err(_) => f64::NAN,
                };
                println!("QP d{i} q{q} {m} {v:.17e}");
            }
        }
    }
}
