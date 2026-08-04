//! Golden: Series.between all 4 inclusive modes vs direct-comparison reference,
//! incl. NaN (must be false in every mode) and Int64. Run:
//!   cargo run -p fp-frame --example golden_between --release
use fp_columnar::Column;
use fp_frame::Series;
use fp_index::{Index, IndexLabel};
use fp_types::{NullKind, Scalar};
fn mk_f64(v: &[f64]) -> Series {
    Series::new(
        "c".to_string(),
        Index::new((0..v.len() as i64).map(IndexLabel::Int64).collect()),
        Column::from_f64_values(v.to_vec()),
    )
    .unwrap()
}
fn mk_i64(v: &[i64]) -> Series {
    Series::new(
        "c".to_string(),
        Index::new((0..v.len() as i64).map(IndexLabel::Int64).collect()),
        Column::from_i64_values(v.to_vec()),
    )
    .unwrap()
}
fn bools(s: Series) -> Vec<bool> {
    s.column()
        .values()
        .iter()
        .map(|x| match x {
            Scalar::Bool(b) => *b,
            Scalar::Null(NullKind::NaN) => false,
            o => panic!("{o:?}"),
        })
        .collect()
}
fn refm(v: f64, lo: f64, hi: f64, mode: &str) -> bool {
    match mode {
        "both" => v >= lo && v <= hi,
        "neither" => v > lo && v < hi,
        "left" => v >= lo && v < hi,
        "right" => v > lo && v <= hi,
        _ => unreachable!(),
    }
}
fn main() {
    let lo = 2.0;
    let hi = 8.0;
    let fv = vec![
        1.0,
        2.0,
        5.0,
        8.0,
        9.0,
        f64::NAN,
        -1.0,
        f64::INFINITY,
        f64::NEG_INFINITY,
        2.0000001,
        7.9999999,
    ];
    for mode in ["both", "neither", "left", "right"] {
        let got = bools(
            mk_f64(&fv)
                .between(&Scalar::Float64(lo), &Scalar::Float64(hi), mode)
                .unwrap(),
        );
        let exp: Vec<bool> = fv
            .iter()
            .map(|&v| {
                if v.is_nan() {
                    false
                } else {
                    refm(v, lo, hi, mode)
                }
            })
            .collect();
        if got != exp {
            println!("FAIL f64 mode={mode}: got={got:?} exp={exp:?}");
            std::process::exit(1);
        }
    }
    let iv = vec![1i64, 2, 5, 8, 9, -1, 0, i64::MIN, i64::MAX];
    for mode in ["both", "neither", "left", "right"] {
        let got = bools(
            mk_i64(&iv)
                .between(&Scalar::Int64(2), &Scalar::Int64(8), mode)
                .unwrap(),
        );
        let exp: Vec<bool> = iv.iter().map(|&v| refm(v as f64, 2.0, 8.0, mode)).collect();
        if got != exp {
            println!("FAIL i64 mode={mode}: got={got:?} exp={exp:?}");
            std::process::exit(1);
        }
    }
    println!("ALL GOLDEN CHECKS PASSED");
}
