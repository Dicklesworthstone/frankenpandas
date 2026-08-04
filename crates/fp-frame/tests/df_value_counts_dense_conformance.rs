//! No-mock conformance guard for the DataFrame.value_counts dense fast path
//! (value_counts_dense_contiguous). The dense path fires only when every column
//! has a typed contiguous backing; one Scalar-backed column forces the generic
//! per-row Vec<ScalarKey> path. So the same data built both ways must yield the
//! byte-identical (label, count) sequence — and match pandas 2.2.3 (count desc,
//! composite-key asc tie-break).

use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::{DataFrame, Series};
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;

const K1: [&str; 5] = ["b", "a", "b", "a", "c"];
const K2: [&str; 5] = ["y", "x", "x", "x", "x"];

fn contig(v: &[&str]) -> Column {
    let mut bytes = Vec::new();
    let mut offsets = vec![0usize];
    for s in v {
        bytes.extend_from_slice(s.as_bytes());
        offsets.push(bytes.len());
    }
    Column::from_utf8_contiguous(bytes, offsets)
}
fn scalar(v: &[&str]) -> Column {
    Column::from_values(v.iter().map(|s| Scalar::Utf8((*s).to_string())).collect()).unwrap()
}

fn frame(typed: bool) -> DataFrame {
    let index = Index::new((0..5i64).map(IndexLabel::Int64).collect());
    let mut cols = BTreeMap::new();
    cols.insert(
        "k1".to_string(),
        if typed { contig(&K1) } else { scalar(&K1) },
    );
    cols.insert("k2".to_string(), contig(&K2));
    DataFrame::new_with_column_order(index, cols, vec!["k1".into(), "k2".into()]).unwrap()
}

fn pairs(s: &Series) -> Vec<(String, i64)> {
    let labels: Vec<String> = s
        .index()
        .labels()
        .iter()
        .map(|l| match l {
            IndexLabel::Utf8(x) => x.clone(),
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    let counts: Vec<i64> = s
        .values()
        .iter()
        .map(|v| match v {
            Scalar::Int64(x) => *x,
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    labels.into_iter().zip(counts).collect()
}

#[test]
fn dense_matches_generic_and_pandas() {
    let dense = frame(true).value_counts().unwrap();
    let generic = frame(false).value_counts().unwrap();
    assert_eq!(pairs(&dense), pairs(&generic), "dense vs generic");
    assert_eq!(
        pairs(&dense),
        vec![
            ("a, x".to_string(), 2),
            ("b, x".to_string(), 1),
            ("b, y".to_string(), 1),
            ("c, x".to_string(), 1),
        ],
        "vs pandas"
    );
}
