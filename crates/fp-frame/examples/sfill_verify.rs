use fp_columnar::Column;
use fp_frame::Series;
use fp_index::Index;
use fp_types::{NullKind, Scalar};
fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^ (h >> 31)
}
fn mks(off: i64, n: usize, seed: u64) -> Series {
    let idx = Index::from_range(off, off + n as i64, 1);
    let v: Vec<Scalar> = (0..n)
        .map(|i| {
            if sm(i, seed).is_multiple_of(4) {
                Scalar::Null(NullKind::Null)
            } else {
                Scalar::Float64((sm(i, seed) % 50) as f64)
            }
        })
        .collect();
    Series::new("s", idx, Column::from_values(v).unwrap()).unwrap()
}
fn dump(s: &Series) -> Vec<(i64, String)> {
    s.index()
        .labels()
        .iter()
        .zip(s.column().values().iter())
        .map(|(l, v)| {
            let key = match l {
                fp_index::IndexLabel::Int64(x) => *x,
                _ => i64::MIN,
            };
            let val = match v {
                Scalar::Float64(f) => {
                    if f.is_nan() {
                        "NaN".into()
                    } else {
                        format!("{:.6}", f)
                    }
                }
                Scalar::Null(_) => "NaN".into(),
                Scalar::Int64(x) => format!("{:.6}", *x as f64),
                other => format!("{other:?}"),
            };
            (key, val)
        })
        .collect()
}
fn main() {
    // shifted affine ranges w/ nulls, unmatched both sides; add + div (div exercises NaN-result path)
    let a = mks(-10, 120, 1);
    let b = mks(50, 120, 2);
    for (opname, s) in [
        ("add", a.add_fill(&b, 0.0).unwrap()),
        ("div", a.div_fill(&b, 1.0).unwrap()),
        ("mul", a.mul_fill(&b, 2.0).unwrap()),
    ] {
        let d = dump(&s);
        print!("{opname} {} ", d.len());
        for (k, v) in &d {
            print!("{k}:{v} ");
        }
        println!();
    }
}
