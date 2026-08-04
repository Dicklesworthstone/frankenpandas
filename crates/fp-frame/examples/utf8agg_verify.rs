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

fn dump(f: &mut std::fs::File, tag: &str, s: &Series) {
    let lbls = s.index().labels();
    let col = s.column().values();
    let mut rows: Vec<(String, String)> = Vec::new();
    for i in 0..s.len() {
        let lbl = match &lbls[i] {
            fp_index::IndexLabel::Utf8(v) => v.clone(),
            o => format!("{o:?}"),
        };
        let v = match &col[i] {
            Scalar::Null(_) => "NA".into(),
            Scalar::Float64(x) if x.is_nan() => "NA".into(),
            Scalar::Float64(x) => format!("{:x}", x.to_bits()),
            Scalar::Int64(x) => format!("{x}i"),
            o => format!("{o:?}"),
        };
        rows.push((lbl, v));
    }
    rows.sort();
    writeln!(
        f,
        "{tag}\t{}",
        rows.iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(",")
    )
    .unwrap();
}

fn main() {
    let n = 6000usize;
    let card = 11usize;
    let keys: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Utf8(format!("c{}", sm(i, 1) % card as u64)))
        .collect();
    // category "c4" fully missing in value
    let iv: Vec<Scalar> = (0..n)
        .map(|i| {
            let g = sm(i, 1) % card as u64;
            if g == 4 || sm(i, 7).is_multiple_of(4) {
                Scalar::Null(NullKind::Null)
            } else {
                Scalar::Int64((sm(i, 9) % 100) as i64 - 50)
            }
        })
        .collect();
    let idx = Index::from_range(0, n as i64, 1);
    let mut map = BTreeMap::new();
    map.insert("k".into(), Column::from_values(keys.clone()).unwrap());
    map.insert("iv".into(), Column::from_values(iv.clone()).unwrap());
    let df =
        DataFrame::new_with_column_order(idx.clone(), map, vec!["k".into(), "iv".into()]).unwrap();
    let k = Series::new("k", idx.clone(), df.column("k").unwrap().clone()).unwrap();
    let s = Series::new("iv", idx.clone(), df.column("iv").unwrap().clone()).unwrap();
    let mut f = std::fs::File::create("/tmp/fp_utf8agg.txt").unwrap();
    let ks: Vec<String> = keys
        .iter()
        .map(|x| {
            if let Scalar::Utf8(v) = x {
                v.clone()
            } else {
                "?".into()
            }
        })
        .collect();
    let vs: Vec<String> = iv
        .iter()
        .map(|x| match x {
            Scalar::Int64(v) => v.to_string(),
            _ => "NA".into(),
        })
        .collect();
    writeln!(f, "K\t{}", ks.join(",")).unwrap();
    writeln!(f, "V\t{}", vs.join(",")).unwrap();
    dump(&mut f, "sum", &s.groupby(&k).unwrap().sum().unwrap());
    dump(&mut f, "mean", &s.groupby(&k).unwrap().mean().unwrap());
    dump(&mut f, "max", &s.groupby(&k).unwrap().max().unwrap());
    dump(&mut f, "min", &s.groupby(&k).unwrap().min().unwrap());
    println!("wrote /tmp/fp_utf8agg.txt");

    // perf
    let n2 = 2_000_000usize;
    let c2 = 1000usize;
    let k2: Vec<Scalar> = (0..n2)
        .map(|i| Scalar::Utf8(format!("cat{}", sm(i, 1) % c2 as u64)))
        .collect();
    let v2: Vec<Scalar> = (0..n2)
        .map(|i| {
            if sm(i, 7).is_multiple_of(5) {
                Scalar::Null(NullKind::Null)
            } else {
                Scalar::Int64((sm(i, 9) % 1000) as i64)
            }
        })
        .collect();
    let idx2 = Index::from_range(0, n2 as i64, 1);
    let mut m2 = BTreeMap::new();
    m2.insert("k".into(), Column::from_values(k2).unwrap());
    m2.insert("v".into(), Column::from_values(v2).unwrap());
    let df2 =
        DataFrame::new_with_column_order(idx2.clone(), m2, vec!["k".into(), "v".into()]).unwrap();
    println!(
        "key contiguous? {} (Eager expected false)",
        df2.column("k").unwrap().as_utf8_contiguous().is_some()
    );
    let kk = Series::new("k", idx2.clone(), df2.column("k").unwrap().clone()).unwrap();
    let sv = Series::new("v", idx2.clone(), df2.column("v").unwrap().clone()).unwrap();
    timeit("sgb.sum  Utf8-key nullable-i64", || {
        std::hint::black_box(sv.groupby(&kk).unwrap().sum().unwrap());
    });
    timeit("sgb.mean Utf8-key nullable-i64", || {
        std::hint::black_box(sv.groupby(&kk).unwrap().mean().unwrap());
    });
    timeit("sgb.max  Utf8-key nullable-i64", || {
        std::hint::black_box(sv.groupby(&kk).unwrap().max().unwrap());
    });
}
