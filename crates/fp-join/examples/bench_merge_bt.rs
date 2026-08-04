use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::Index;
use fp_join::{JoinType, merge_dataframes};
use fp_types::Scalar;
fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^ (h >> 31)
}
fn t(l: &str, mut f: impl FnMut()) {
    let mut b = u128::MAX;
    for _ in 0..4 {
        let x = std::time::Instant::now();
        f();
        b = b.min(x.elapsed().as_nanos());
    }
    println!("{l}: {:.2}ms", b as f64 / 1e6);
}
fn frame(n: usize, cols: Vec<(&str, Column)>) -> DataFrame {
    let order: Vec<String> = cols.iter().map(|(nm, _)| nm.to_string()).collect();
    let mut m = BTreeMap::new();
    for (nm, c) in cols {
        m.insert(nm.to_string(), c);
    }
    DataFrame::new_with_column_order(Index::from_range(0, n as i64, 1), m, order).unwrap()
}
fn main() {
    let n = 1_000_000usize;
    let rn = 200_000usize;
    let li = Column::from_values(
        (0..n)
            .map(|i| Scalar::Int64((sm(i, 1) % rn as u64) as i64))
            .collect(),
    )
    .unwrap();
    let lv = Column::from_values(
        (0..n)
            .map(|i| Scalar::Float64((sm(i, 7) % 1000) as f64))
            .collect(),
    )
    .unwrap();
    let lks = Column::from_values(
        (0..n)
            .map(|i| Scalar::Utf8(format!("k{}", sm(i, 1) % rn as u64)))
            .collect(),
    )
    .unwrap();
    let ri = Column::from_values((0..rn).map(|j| Scalar::Int64(j as i64)).collect()).unwrap();
    let rv = Column::from_values(
        (0..rn)
            .map(|j| Scalar::Float64((sm(j, 9) % 1000) as f64))
            .collect(),
    )
    .unwrap();
    let rks =
        Column::from_values((0..rn).map(|j| Scalar::Utf8(format!("k{j}"))).collect()).unwrap();
    let l_i = frame(n, vec![("k", li.clone()), ("lv", lv.clone())]);
    let r_i = frame(rn, vec![("k", ri), ("rv", rv.clone())]);
    let l_s = frame(n, vec![("k", lks), ("lv", lv)]);
    let r_s = frame(rn, vec![("k", rks), ("rv", rv)]);
    t("merge_i64_inner", || {
        std::hint::black_box(merge_dataframes(&l_i, &r_i, "k", JoinType::Inner).unwrap());
    });
    t("merge_i64_left", || {
        std::hint::black_box(merge_dataframes(&l_i, &r_i, "k", JoinType::Left).unwrap());
    });
    t("merge_utf8_inner", || {
        std::hint::black_box(merge_dataframes(&l_s, &r_s, "k", JoinType::Inner).unwrap());
    });
}
