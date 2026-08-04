use std::{collections::BTreeMap, io::Write};

use fp_columnar::Column;
use fp_frame::{DataFrame, Series};
use fp_index::Index;
use fp_types::{NullKind, Scalar};

fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^ (h >> 31)
}
fn timeit(l: &str, mut f: impl FnMut()) {
    let mut b = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        f();
        b = b.min(t.elapsed().as_nanos());
    }
    println!("{l}: {:.2}ms", b as f64 / 1e6);
}
fn dumpvals(col: &[Scalar]) -> String {
    col.iter()
        .map(|x| match x {
            Scalar::Null(_) => "NA".into(),
            Scalar::Float64(v) if v.is_nan() => "NA".into(),
            Scalar::Float64(v) => format!("{:x}", v.to_bits()),
            Scalar::Int64(v) => format!("{v}"),
            o => format!("{o:?}"),
        })
        .collect::<Vec<_>>()
        .join(",")
}
fn build(iv: &[Scalar]) -> Series {
    let n = iv.len();
    let idx = Index::from_range(0, n as i64, 1);
    let mut map = BTreeMap::new();
    map.insert("iv".into(), Column::from_values(iv.to_vec()).unwrap());
    let df = DataFrame::new_with_column_order(idx.clone(), map, vec!["iv".into()]).unwrap();
    Series::new("iv", idx.clone(), df.column("iv").unwrap().clone()).unwrap()
}
fn vstr(iv: &[Scalar]) -> String {
    iv.iter()
        .map(|x| match x {
            Scalar::Int64(v) => v.to_string(),
            _ => "NA".into(),
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn main() {
    let mut f = std::fs::File::create("/tmp/fp_cumprod.txt").unwrap();
    // Case A: random small nullable (overflow to inf, some 0 -> 0)
    let na = 5000usize;
    let a: Vec<Scalar> = (0..na)
        .map(|i| {
            if i < 2 || sm(i, 7).is_multiple_of(4) {
                Scalar::Null(NullKind::Null)
            } else {
                Scalar::Int64((sm(i, 9) % 12) as i64)
            }
        })
        .collect();
    // Case B: crafted overflow(inf) then a zero -> inf*0 = NaN, then propagate
    let mut b: Vec<Scalar> = (0..400).map(|_| Scalar::Int64(900)).collect();
    b.push(Scalar::Null(NullKind::Null)); // missing mid
    b.extend((0..50).map(|_| Scalar::Int64(900)));
    b.push(Scalar::Int64(0)); // inf * 0 -> NaN
    b.extend((0..20).map(|_| Scalar::Int64(7)));
    for (tag, iv) in [("A", &a), ("B", &b)] {
        let s = build(iv);
        let r = s.cumprod().unwrap();
        writeln!(f, "V{tag}\t{}", vstr(iv)).unwrap();
        writeln!(
            f,
            "O{tag}\t{}\t{:?}",
            dumpvals(r.column().values()),
            r.column().dtype()
        )
        .unwrap();
    }
    println!("wrote /tmp/fp_cumprod.txt");
    // perf
    let n2 = 2_000_000usize;
    let pv: Vec<Scalar> = (0..n2)
        .map(|i| {
            if sm(i, 7).is_multiple_of(5) {
                Scalar::Null(NullKind::Null)
            } else {
                Scalar::Int64((sm(i, 9) % 1000) as i64)
            }
        })
        .collect();
    let sp = build(&pv);
    timeit("cumprod nullable-i64", || {
        std::hint::black_box(sp.cumprod().unwrap());
    });
}
