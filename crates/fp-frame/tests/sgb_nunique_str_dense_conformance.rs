//! No-mock conformance guard for the SeriesGroupBy.nunique Utf8 dense span path
//! (try_nunique_str_dense). It fires only for a contiguous-Utf8 value; a Scalar-
//! backed value bails to agg_values_scalar. Same data both ways must yield the
//! byte-identical (label, count) sequence (dense vs generic) and match pandas 2.2.3.

use fp_columnar::Column;
use fp_frame::Series;
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
fn scalar(v: &[&str]) -> Column {
    Column::from_values(v.iter().map(|s| Scalar::Utf8((*s).to_string())).collect()).unwrap()
}
fn series(name: &str, col: Column, n: usize) -> Series {
    Series::new(
        name,
        Index::new((0..n as i64).map(IndexLabel::Int64).collect()),
        col,
    )
    .unwrap()
}

fn run(by: &[&str], v: &[&str], v_contig: bool) -> Vec<(String, i64)> {
    let n = by.len();
    let byc = series("k", contig(by), n);
    let vc = series("v", if v_contig { contig(v) } else { scalar(v) }, n);
    let r = vc.groupby(&byc).unwrap().nunique().unwrap();
    let labels: Vec<String> = r.index().labels().iter().map(|l| l.to_string()).collect();
    let counts: Vec<i64> = r
        .column()
        .values()
        .iter()
        .map(|x| match x {
            Scalar::Int64(c) => *c,
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    labels.into_iter().zip(counts).collect()
}

#[test]
fn dense_matches_generic_and_pandas() {
    let by = ["a", "a", "a", "b"];
    let v = ["x", "y", "x", "z"];
    let dense = run(&by, &v, true);
    let generic = run(&by, &v, false);
    assert_eq!(dense, generic, "dense vs generic");
    assert_eq!(
        dense,
        vec![("a".to_string(), 2), ("b".to_string(), 1)],
        "vs pandas"
    );
}

// first-seen != sorted: dense must still equal generic (bit-identity).
#[test]
fn dense_matches_generic_first_seen() {
    let by = ["b", "a", "b", "a"];
    let v = ["p", "q", "p", "r"];
    assert_eq!(run(&by, &v, true), run(&by, &v, false));
}
