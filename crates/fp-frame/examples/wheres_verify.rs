use fp_columnar::Column;
use fp_frame::Series;
use fp_index::Index;
use fp_types::{NullKind, Scalar};
fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^ (h >> 31)
}
fn mkf(off: i64, n: usize, seed: u64) -> Series {
    let idx = Index::from_range(off, off + n as i64, 1);
    let v: Vec<Scalar> = (0..n)
        .map(|i| {
            if sm(i, seed).is_multiple_of(5) {
                Scalar::Null(NullKind::Null)
            } else {
                Scalar::Float64((sm(i, seed) % 100) as f64)
            }
        })
        .collect();
    Series::new("s", idx, Column::from_values(v).unwrap()).unwrap()
}
fn mkbool(off: i64, n: usize) -> Series {
    let idx = Index::from_range(off, off + n as i64, 1);
    let v: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Bool(sm(i, 3).is_multiple_of(2)))
        .collect();
    Series::new("c", idx, Column::from_values(v).unwrap()).unwrap()
}
fn dump(tag: &str, s: &Series) {
    let idx = s.index().labels();
    print!("{tag} {} ", idx.len());
    for (l, v) in idx.iter().zip(s.column().values().iter()) {
        let key = match l {
            fp_index::IndexLabel::Int64(x) => *x,
            _ => i64::MIN,
        };
        let val = match v {
            Scalar::Float64(f) => {
                if f.is_nan() {
                    "NaN".into()
                } else {
                    format!("{f:.4}")
                }
            }
            Scalar::Null(_) => "NaN".into(),
            o => format!("{o:?}"),
        };
        print!("{key}:{val} ");
    }
    println!();
}
fn main() {
    let n = 60usize;
    let s = mkf(-5, n, 1);
    let cond = mkbool(20, n);
    let other = mkf(20, n, 2);
    dump("where", &s.where_cond_series(&cond, &other).unwrap());
    dump("mask", &s.mask_series(&cond, &other).unwrap());
}
