//! No-mock conformance guard for GroupBy.agg_list over a Utf8 value: the per-func
//! reductions now dispatch through the public gb.<func>() entry points (so the
//! Utf8 dense paths fire) instead of aggregate_named_func. Output must equal the
//! direct gb.<func>() result relabelled {col}_{func}, and match pandas 2.2.3.

use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;

fn contig(v: &[&str]) -> Column {
    let mut bytes = Vec::new();
    let mut offsets = vec![0usize];
    for s in v {
        bytes.extend_from_slice(s.as_bytes());
        offsets.push(bytes.len());
    }
    Column::from_utf8_contiguous(bytes, offsets)
}
fn frame() -> DataFrame {
    let index = Index::new((0..5i64).map(IndexLabel::Int64).collect());
    let mut cols = BTreeMap::new();
    cols.insert("k".to_string(), contig(&["b", "a", "b", "a", "c"]));
    cols.insert("v".to_string(), contig(&["x1", "x2", "x3", "x4", "x5"]));
    DataFrame::new_with_column_order(index, cols, vec!["k".into(), "v".into()]).unwrap()
}
fn col_str(df: &DataFrame, name: &str) -> Vec<String> {
    df.column(name)
        .unwrap_or_else(|| panic!("missing {name}"))
        .values()
        .iter()
        .map(|v| match v {
            Scalar::Utf8(s) => s.clone(),
            Scalar::Int64(x) => x.to_string(),
            other => panic!("unexpected {other:?}"),
        })
        .collect()
}

#[test]
fn agglist_utf8_matches_direct_and_pandas() {
    let df = frame();
    let r = df
        .groupby(&["k"])
        .unwrap()
        .agg_list(&["count", "first", "max"])
        .unwrap();
    assert_eq!(col_str(&r, "v_count"), vec!["2", "2", "1"], "count");
    assert_eq!(col_str(&r, "v_first"), vec!["x2", "x1", "x5"], "first");
    assert_eq!(col_str(&r, "v_max"), vec!["x4", "x3", "x5"], "max");
    // each agg_list column equals the direct gb.<func>() column (just relabelled)
    let gb = df.groupby(&["k"]).unwrap();
    assert_eq!(col_str(&r, "v_count"), col_str(&gb.count().unwrap(), "v"));
    assert_eq!(col_str(&r, "v_first"), col_str(&gb.first().unwrap(), "v"));
    assert_eq!(col_str(&r, "v_max"), col_str(&gb.max().unwrap(), "v"));
}
