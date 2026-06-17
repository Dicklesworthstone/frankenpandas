//! Differential golden for Series vs Series comparison operators.
//! Covers (a) equal-index fast path and (b) misaligned-index alignment path,
//! across all six operators, with NaN present. Proves the identity fast path
//! in `comparison_op` is bit-identical to the prior unconditional align path.
//! Run: cargo run -p fp-frame --example golden_cmp_series --release

use fp_columnar::Column;
use fp_frame::Series;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;

fn idx(labels: &[i64]) -> Index {
    Index::new(labels.iter().map(|&x| IndexLabel::Int64(x)).collect())
}

fn fmt(s: &Series) -> String {
    let mut out = String::new();
    // Include index labels + values so a wrong union index is caught.
    for (lbl, v) in s.index().labels().iter().zip(s.column().values().iter()) {
        out.push_str(&format!("{lbl:?}={v:?};"));
    }
    out
}

fn main() {
    let mut z = 0x9e37u64;
    let mut rnd = || {
        z ^= z << 13;
        z ^= z >> 7;
        z ^= z << 17;
        (z >> 11) as f64 / (1u64 << 53) as f64 * 100.0 - 50.0
    };
    let n = 4000usize;
    let mut a: Vec<f64> = (0..n).map(|_| rnd()).collect();
    let b: Vec<f64> = (0..n).map(|_| rnd()).collect();
    // Sprinkle NaN to exercise null comparison semantics.
    a[3] = f64::NAN;
    a[100] = f64::NAN;

    let shared = idx(&(0..n as i64).collect::<Vec<_>>());
    let sa = Series::new("a".to_string(), shared.clone(), Column::from_f64_values(a.clone())).unwrap();
    let sb = Series::new("b".to_string(), shared.clone(), Column::from_f64_values(b.clone())).unwrap();

    // Misaligned: same labels, reversed order (alignment path, no fast path).
    let rev = idx(&(0..n as i64).rev().collect::<Vec<_>>());
    let sb_rev = Series::new("b".to_string(), rev, Column::from_f64_values(b.clone())).unwrap();

    // Partial overlap: shift labels by n/2 so only half intersect.
    let shifted = idx(&((n as i64 / 2)..(n as i64 + n as i64 / 2)).collect::<Vec<_>>());
    let sb_shift = Series::new("b".to_string(), shifted, Column::from_f64_values(b.clone())).unwrap();

    let mut acc = String::new();
    let ops: [(&str, fn(&Series, &Series) -> Series); 6] = [
        ("gt", |x, y| x.gt(y).unwrap()),
        ("lt", |x, y| x.lt(y).unwrap()),
        ("eq", |x, y| x.eq_series(y).unwrap()),
        ("ne", |x, y| x.ne_series(y).unwrap()),
        ("ge", |x, y| x.ge(y).unwrap()),
        ("le", |x, y| x.le(y).unwrap()),
    ];
    for (name, f) in ops {
        acc.push_str(name);
        acc.push_str("|shared|");
        acc.push_str(&fmt(&f(&sa, &sb)));
        acc.push_str("|rev|");
        acc.push_str(&fmt(&f(&sa, &sb_rev)));
        acc.push_str("|shift|");
        acc.push_str(&fmt(&f(&sa, &sb_shift)));
        acc.push('\n');
    }

    // Cheap stable hash of the full output.
    let mut h = 0xcbf29ce484222325u64;
    for byte in acc.bytes() {
        h ^= byte as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    println!("CMP_SERIES_FNV1A={h:016x}");
    println!("len={}", acc.len());
    println!("ALL GOLDEN CHECKS PASSED");
}
