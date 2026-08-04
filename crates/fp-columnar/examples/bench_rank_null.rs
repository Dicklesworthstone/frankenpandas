//! Bench for `Column::rank` on a nullable Int64/Float64 column — the new
//! typed_radix_perm + typed tie-walk path. Source is a LAZY nullable column;
//! without the fast path, rank filters non-missing, sorts by the O(n log n)
//! na-last `compare_scalars_na_last` Scalar comparator, then boxes a Scalar
//! per row.
//!
//! Same-binary A/B: TREATMENT = `col.rank(method, asc)`; CONTROL = the exact
//! pre-change generic algorithm (non-missing stable Scalar sort + assign). A
//! folded checksum of both results is asserted equal so the win is honest.
//!
//! Run: cargo run -p fp-columnar --release --example bench_rank_null -- 5000000 12

use std::cmp::Ordering;

use fp_columnar::{Column, ValidityMask};
use fp_types::Scalar;

fn fold(vals: &[Scalar]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for v in vals {
        let bits = match v {
            Scalar::Float64(x) => x.to_bits(),
            Scalar::Null(_) => 0xFFFF_FFFF_FFFF_FFFF,
            _ => 0xDEAD_BEEF,
        };
        h ^= bits;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn na_last(a: &Scalar, b: &Scalar, ascending: bool) -> Ordering {
    let am = matches!(a, Scalar::Null(_)) || matches!(a, Scalar::Float64(x) if x.is_nan());
    let bm = matches!(b, Scalar::Null(_)) || matches!(b, Scalar::Float64(x) if x.is_nan());
    match (am, bm) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => {
            let ord = match (a, b) {
                (Scalar::Int64(x), Scalar::Int64(y)) => x.cmp(y),
                (Scalar::Float64(x), Scalar::Float64(y)) => {
                    x.partial_cmp(y).unwrap_or(Ordering::Equal)
                }
                _ => Ordering::Equal,
            };
            if ascending { ord } else { ord.reverse() }
        }
    }
}

fn is_missing(v: &Scalar) -> bool {
    matches!(v, Scalar::Null(_)) || matches!(v, Scalar::Float64(x) if x.is_nan())
}

// Replica of the pre-change generic rank path.
fn control_rank(col: &Column, method: &str, ascending: bool) -> Vec<Scalar> {
    let vals = col.values();
    let len = vals.len();
    let mut nm: Vec<usize> = (0..len).filter(|&i| !is_missing(&vals[i])).collect();
    nm.sort_by(|&a, &b| na_last(&vals[a], &vals[b], ascending));
    let mut ranks = vec![Scalar::Null(fp_types::NullKind::NaN); len];
    let n = nm.len();
    let mut cursor = 0usize;
    let mut dense = 0.0_f64;
    while cursor < n {
        let mut end = cursor + 1;
        while end < n && na_last(&vals[nm[cursor]], &vals[nm[end]], ascending) == Ordering::Equal {
            end += 1;
        }
        let (sr, er) = (cursor as f64 + 1.0, end as f64);
        dense += 1.0;
        for gi in cursor..end {
            ranks[nm[gi]] = Scalar::Float64(match method {
                "average" => (sr + er) / 2.0,
                "min" => sr,
                "max" => er,
                "first" => gi as f64 + 1.0,
                "dense" => dense,
                _ => unreachable!(),
            });
        }
        cursor = end;
    }
    ranks
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let len: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(12);

    let idata: Vec<i64> = (0..len)
        .map(|i| {
            let h = (i as u64).wrapping_mul(2_654_435_761).wrapping_add(12_345);
            (h % 50_003) as i64 - 25_001
        })
        .collect();
    let fdata: Vec<f64> = idata.iter().map(|&x| x as f64 * 0.5).collect();
    let mut validity = ValidityMask::all_valid(len);
    for i in (0..len).step_by(7) {
        validity.set(i, false);
    }
    let icol = Column::from_i64_values_with_validity(idata, validity.clone());
    let fcol = Column::from_f64_values_with_validity(fdata, validity);

    for (label, col) in [("i64", &icol), ("f64", &fcol)] {
        for method in ["average", "first"] {
            let mut best_t = u128::MAX;
            let mut ck_t: u64 = 0;
            for _ in 0..iters {
                let t = std::time::Instant::now();
                let out = col.rank(method, true).expect("rank");
                best_t = best_t.min(t.elapsed().as_nanos());
                ck_t = ck_t.wrapping_add(fold(out.values()));
            }
            let mut best_c = u128::MAX;
            let mut ck_c: u64 = 0;
            for _ in 0..iters {
                let t = std::time::Instant::now();
                let out = control_rank(col, method, true);
                best_c = best_c.min(t.elapsed().as_nanos());
                ck_c = ck_c.wrapping_add(fold(&out));
            }
            assert_eq!(ck_t, ck_c, "{label} {method}: treatment != control");
            println!(
                "rank_null {label} method={method} len={len} iters={iters} \
                 treatment={:.2}ms control={:.2}ms speedup={:.3}x",
                best_t as f64 / 1e6,
                best_c as f64 / 1e6,
                best_c as f64 / best_t as f64,
            );
        }
    }
}
