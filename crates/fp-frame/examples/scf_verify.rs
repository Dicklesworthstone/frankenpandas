use std::io::Write;

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
    // affine shifted (a=[-20,80), b=[30,130)) contiguous union [-20,130); nulls in a
    let mk = |start: i64, n: usize, seed: u64, wn: bool| {
        let idx = Index::from_range(start, start + n as i64, 1);
        let v: Vec<Scalar> = (0..n)
            .map(|i| {
                if wn && sm(i, 3).is_multiple_of(3) {
                    Scalar::Null(NullKind::Null)
                } else {
                    Scalar::Float64((sm(i, seed) % 100) as f64)
                }
            })
            .collect();
        Series::new("s", idx, Column::from_values(v).unwrap()).unwrap()
    };
    let a = mk(-20, 100, 1, true);
    let b = mk(30, 100, 2, false);
    let r = a.combine_first(&b).unwrap();
    let idx = r.index().labels();
    let c0 = r.column().values();
    let mut f = std::fs::File::create("/tmp/fp_scf.txt").unwrap();
    let rows: Vec<String> = (0..idx.len())
        .map(|i| {
            let l = match &idx[i] {
                fp_index::IndexLabel::Int64(v) => v.to_string(),
                o => format!("{o:?}"),
            };
            let v = match &c0[i] {
                Scalar::Float64(x) => {
                    if x.is_nan() {
                        "NA".into()
                    } else {
                        format!("{}", *x as i64)
                    }
                }
                Scalar::Null(_) => "NA".into(),
                o => format!("{o:?}"),
            };
            format!("{l}={v}")
        })
        .collect();
    writeln!(f, "ROWS\t{}", rows.join("|")).unwrap();
    let g = |start: i64, n: usize, seed: u64, wn: bool| {
        (0..n)
            .map(|i| {
                format!(
                    "{}:{}",
                    start + i as i64,
                    if wn && sm(i, 3).is_multiple_of(3) {
                        "NA".into()
                    } else {
                        format!("{}", sm(i, seed) % 100)
                    }
                )
            })
            .collect::<Vec<_>>()
            .join(",")
    };
    writeln!(f, "A\t{}", g(-20, 100, 1, true)).unwrap();
    writeln!(f, "B\t{}", g(30, 100, 2, false)).unwrap();
    println!("out={}", idx.len());
}
