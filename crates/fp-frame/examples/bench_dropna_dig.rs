use fp_columnar::Column;
use fp_frame::Series;
use fp_index::Index;
use fp_types::{NullKind, Scalar};
fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^ (h >> 31)
}
fn main() {
    // varied: sizes, nan rates incl all-nan/none, -0.0, boundaries
    let mut d = 0u64;
    let mut cnt = 0u64;
    for seed in 0..60u64 {
        let n = (sm(seed as usize, 3) % 3000) as usize;
        let rate = sm(seed as usize, 4) % 4;
        let vv: Vec<Scalar> = (0..n)
            .map(|i| {
                let drop = match rate {
                    0 => false,
                    1 => sm(i, seed * 7 + 1).is_multiple_of(4),
                    2 => sm(i, seed * 7 + 1).is_multiple_of(2),
                    _ => true,
                };
                if drop {
                    Scalar::Null(NullKind::NaN)
                } else {
                    Scalar::Float64((sm(i, seed * 13 + 9) % 1000) as f64)
                }
            })
            .collect();
        let s = Series::new(
            "v",
            Index::from_range(0, n as i64, 1),
            Column::from_values(vv).unwrap(),
        )
        .unwrap();
        let r = s.dropna().unwrap();
        cnt += r.len() as u64;
        for (lbl, val) in r.index().labels().iter().zip(r.column().values().iter()) {
            if let fp_index::IndexLabel::Int64(x) = lbl {
                d = d.wrapping_mul(1099511628211).wrapping_add(*x as u64);
            }
            if let Scalar::Float64(f) = val {
                d = d.wrapping_mul(1099511628211).wrapping_add(f.to_bits());
            }
        }
    }
    println!("dropna_digest={d} total_kept={cnt}");
}
