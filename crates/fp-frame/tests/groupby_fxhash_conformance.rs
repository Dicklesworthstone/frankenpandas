//! No-mock conformance guard for the build_groups FxHash grouping lever
//! (br-frankenpandas-buguz): the FxHashMap/FxHashSet grouping accumulator must be
//! BIT-TRANSPARENT — same group membership, same group order, same per-group aggregate
//! as the SipHash path. Keys are chosen so first-seen order == sorted order, making the
//! expected output unambiguous. Compiled via `cargo check --tests`; full run batch-pending.

use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;

// keys a,b,a,c,b,a / vals 1,2,3,4,5,6 => a:[1,3,6]=10, b:[2,5]=7, c:[4]=4
fn fixture() -> DataFrame {
    let n = 6;
    let index = Index::new((0..n as i64).map(IndexLabel::Int64).collect());
    let keys = ["a", "b", "a", "c", "b", "a"];
    let vals = [1_i64, 2, 3, 4, 5, 6];
    let mut cols = BTreeMap::new();
    cols.insert(
        "k".to_string(),
        Column::from_values(
            keys.iter()
                .map(|s| Scalar::Utf8((*s).to_string()))
                .collect(),
        )
        .unwrap(),
    );
    cols.insert(
        "v".to_string(),
        Column::from_values(vals.iter().map(|&x| Scalar::Int64(x)).collect()).unwrap(),
    );
    DataFrame::new_with_column_order(index, cols, vec!["k".to_string(), "v".to_string()]).unwrap()
}

fn group_labels(df: &DataFrame) -> Vec<String> {
    df.index()
        .labels()
        .iter()
        .map(|l| match l {
            IndexLabel::Utf8(s) => s.clone(),
            other => format!("{other:?}"),
        })
        .collect()
}

fn col_numeric(df: &DataFrame, name: &str) -> Vec<i64> {
    df.column(name)
        .expect("column present")
        .values()
        .iter()
        .map(|v| match v {
            Scalar::Int64(x) => *x,
            Scalar::Float64(x) => *x as i64,
            _ => i64::MIN,
        })
        .collect()
}

#[test]
fn groupby_sum_fxhash_exact_groups_and_aggregate() {
    let g = fixture().groupby(&["k"]).unwrap().sum().unwrap();
    assert_eq!(group_labels(&g), vec!["a", "b", "c"]);
    assert_eq!(col_numeric(&g, "v"), vec![10, 7, 4]);
}

#[test]
fn groupby_count_fxhash_exact_group_sizes() {
    let g = fixture().groupby(&["k"]).unwrap().count().unwrap();
    assert_eq!(group_labels(&g), vec!["a", "b", "c"]);
    assert_eq!(col_numeric(&g, "v"), vec![3, 2, 1]);
}
