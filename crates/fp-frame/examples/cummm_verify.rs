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
    let outs: Vec<String> = col
        .iter()
        .map(|x| match x {
            Scalar::Null(_) => "NA".into(),
            Scalar::Float64(v) => format!("{v:.4}"),
            Scalar::Int64(v) => format!("{v}.0000"),
            o => format!("{o:?}"),
        })
        .collect();
    outs.join(",")
}

fn main() {
    // correctness (small)
    let n = 5000usize;
    let iv: Vec<Scalar> = (0..n)
        .map(|i| {
            if i < 3 || sm(i, 7).is_multiple_of(4) {
                Scalar::Null(NullKind::Null)
            } else {
                Scalar::Int64((sm(i, 9) % 200) as i64 - 100)
            }
        })
        .collect();
    let idx = Index::from_range(0, n as i64, 1);
    let mut map = BTreeMap::new();
    map.insert("iv".into(), Column::from_values(iv.clone()).unwrap());
    let df = DataFrame::new_with_column_order(idx.clone(), map, vec!["iv".into()]).unwrap();
    let s = Series::new("iv", idx.clone(), df.column("iv").unwrap().clone()).unwrap();
    let mx = s.cummax().unwrap();
    let mn = s.cummin().unwrap();
    let mut f = std::fs::File::create("/tmp/fp_cummm.txt").unwrap();
    let vs: Vec<String> = iv
        .iter()
        .map(|x| match x {
            Scalar::Int64(v) => v.to_string(),
            _ => "NA".into(),
        })
        .collect();
    writeln!(f, "V\t{}", vs.join(",")).unwrap();
    writeln!(
        f,
        "MAX\t{}\t{:?}",
        dumpvals(mx.column().values()),
        mx.column().dtype()
    )
    .unwrap();
    writeln!(
        f,
        "MIN\t{}\t{:?}",
        dumpvals(mn.column().values()),
        mn.column().dtype()
    )
    .unwrap();
    println!(
        "wrote /tmp/fp_cummm.txt max_dtype={:?} min_dtype={:?}",
        mx.column().dtype(),
        mn.column().dtype()
    );

    // perf (2M)
    let n2 = 2_000_000usize;
    let bv: Vec<Scalar> = (0..n2)
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
    m2.insert("b".into(), Column::from_values(bv).unwrap());
    let df2 = DataFrame::new_with_column_order(idx2.clone(), m2, vec!["b".into()]).unwrap();
    let sb = Series::new("b", idx2.clone(), df2.column("b").unwrap().clone()).unwrap();
    timeit("cummax nullable-i64", || {
        std::hint::black_box(sb.cummax().unwrap());
    });
    timeit("cummin nullable-i64", || {
        std::hint::black_box(sb.cummin().unwrap());
    });
}
