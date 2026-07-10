use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::Index;
use fp_join::{AsofDirection, merge_asof};
use fp_types::Scalar;
fn mkl(keys: &[i64], vals: &[i64], kn: &str, vn: &str) -> DataFrame {
    let idx = Index::from_range(0, keys.len() as i64, 1);
    let mut m = BTreeMap::new();
    m.insert(
        kn.to_string(),
        Column::from_values(keys.iter().map(|&k| Scalar::Int64(k)).collect()).unwrap(),
    );
    m.insert(
        vn.to_string(),
        Column::from_values(vals.iter().map(|&v| Scalar::Int64(v)).collect()).unwrap(),
    );
    DataFrame::new_with_column_order(idx, m, vec![kn.to_string(), vn.to_string()]).unwrap()
}
fn dump(tag: &str, df: &fp_join::MergedDataFrame) {
    let n = df.index.labels().len();
    let g = |c: &str, i: usize| -> String {
        df.columns
            .get(c)
            .map(|col| match &col.values()[i] {
                Scalar::Int64(x) => x.to_string(),
                Scalar::Float64(f) => {
                    if f.is_nan() {
                        "NA".into()
                    } else {
                        format!("{f:.0}")
                    }
                }
                Scalar::Null(_) => "NA".into(),
                o => format!("{o:?}"),
            })
            .unwrap_or("_".into())
    };
    print!("{tag} {n} ");
    for i in 0..n {
        print!("[{},{},{}] ", g("t", i), g("lv", i), g("rv", i));
    }
    println!();
}
fn main() {
    let left = mkl(&[1, 5, 10, 15, 20], &[100, 101, 102, 103, 104], "t", "lv");
    let right = mkl(&[2, 3, 7, 16, 25], &[200, 201, 202, 203, 204], "t", "rv");
    for (tag, d) in [
        ("bwd", AsofDirection::Backward),
        ("fwd", AsofDirection::Forward),
        ("near", AsofDirection::Nearest),
    ] {
        dump(tag, &merge_asof(&left, &right, "t", d).unwrap());
    }
}
