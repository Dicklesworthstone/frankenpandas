//! No-mock conformance guard for the GroupBy.value_counts contiguous-Utf8 value
//! span tally. The span path fires only when the value column is contiguous Utf8;
//! a Scalar-backed value column keeps the generic ScalarKey path. Same data built
//! both ways must yield the byte-identical (label, count) sequence — and match
//! pandas 2.2.3 (per-group count desc, first-seen tiebreak).

use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;

const K: [&str; 5] = ["a", "a", "a", "b", "b"];
const V: [&str; 5] = ["x", "y", "x", "x", "z"];

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

fn frame(v_contig: bool) -> DataFrame {
    let index = Index::new((0..5i64).map(IndexLabel::Int64).collect());
    let mut cols = BTreeMap::new();
    cols.insert("k".to_string(), contig(&K));
    cols.insert(
        "v".to_string(),
        if v_contig { contig(&V) } else { scalar(&V) },
    );
    DataFrame::new_with_column_order(index, cols, vec!["k".into(), "v".into()]).unwrap()
}

fn pairs(df: &DataFrame) -> Vec<(String, i64)> {
    let labels: Vec<String> = df.index().labels().iter().map(|l| l.to_string()).collect();
    let counts: Vec<i64> = df
        .column("count")
        .unwrap()
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
fn span_matches_scalar_and_pandas() {
    let span = frame(true).groupby(&["k"]).unwrap().value_counts().unwrap();
    let scalar_path = frame(false)
        .groupby(&["k"])
        .unwrap()
        .value_counts()
        .unwrap();
    assert_eq!(pairs(&span), pairs(&scalar_path), "span vs scalar path");
    assert_eq!(
        pairs(&span),
        vec![
            ("a, x".to_string(), 2),
            ("a, y".to_string(), 1),
            ("b, x".to_string(), 1),
            ("b, z".to_string(), 1),
        ],
        "vs pandas"
    );
}
