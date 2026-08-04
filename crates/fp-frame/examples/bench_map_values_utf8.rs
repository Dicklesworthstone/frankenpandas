//! Bench for `Series::map_values` on a large Utf8 column after the Utf8 fast path
//! (look up the borrowed `&str` directly instead of cloning each value into a
//! `String` key). NEW = ser.map_values(); CONTROL = the old per-row `s.clone()`
//! then `get(&key)`. Results asserted equal.
//!
//! Run: cargo run -p fp-frame --release --example bench_map_values_utf8 -- 5000000 8

use std::collections::HashMap;

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::{NullKind, Scalar};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(8);

    let keys: Vec<String> = (0..500).map(|k| format!("cat_{k:03}")).collect();
    let mut mapping: HashMap<String, Scalar> = HashMap::new();
    for (k, key) in keys.iter().enumerate() {
        mapping.insert(key.clone(), Scalar::Int64(k as i64));
    }
    let vals: Vec<Scalar> = (0..n)
        .map(|i| {
            let h = (i as u64).wrapping_mul(2_654_435_761).wrapping_add(999);
            Scalar::Utf8(keys[(h % 500) as usize].clone())
        })
        .collect();
    let idx: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let ser = Series::from_values("x", idx, vals).unwrap();

    // Isolate the TALLY (both build only a Vec<Scalar>, no output Series) so the
    // comparison measures exactly the per-row clone the fix removes.
    let new_tally = || -> Vec<Scalar> {
        ser.column()
            .values()
            .iter()
            .map(|v| match v {
                Scalar::Utf8(s) => mapping
                    .get(s.as_str())
                    .cloned()
                    .unwrap_or(Scalar::Null(NullKind::NaN)),
                _ => Scalar::Null(NullKind::NaN),
            })
            .collect()
    };
    let old_tally = || -> Vec<Scalar> {
        ser.column()
            .values()
            .iter()
            .map(|v| match v {
                Scalar::Utf8(s) => {
                    let key = s.clone();
                    mapping
                        .get(&key)
                        .cloned()
                        .unwrap_or(Scalar::Null(NullKind::NaN))
                }
                _ => Scalar::Null(NullKind::NaN),
            })
            .collect()
    };
    assert_eq!(new_tally(), old_tally(), "NEW != OLD tally");
    // Also confirm the shipped method matches.
    assert_eq!(
        ser.map_values(&mapping).unwrap().column().values(),
        new_tally().as_slice(),
        "map_values != tally"
    );

    let bench = |f: &dyn Fn() -> Vec<Scalar>| -> f64 {
        let mut b = u128::MAX;
        for _ in 0..iters {
            let t = std::time::Instant::now();
            let r = f();
            b = b.min(t.elapsed().as_nanos());
            std::hint::black_box(r.len());
        }
        b as f64 / 1e6
    };
    let t_new = bench(&new_tally);
    let t_old = bench(&old_tally);
    let t_full = bench(&|| ser.map_values(&mapping).unwrap().column().values().to_vec());
    println!(
        "map_values_utf8 n={n} keys=500 iters={iters} NEW_tally={:.2}ms OLD_tally(clone)={:.2}ms speedup={:.3}x  full_method={:.2}ms",
        t_new, t_old, t_old / t_new, t_full
    );
}
