//! No-mock conformance guard for the SeriesGroupBy max/min Utf8 dense span path
//! (try_utf8_extreme_dense). It fires only for a contiguous-Utf8 value; a Scalar-
//! backed value bails to agg_values_scalar. The same data built both ways must yield
//! the byte-identical (label, value) sequence (dense vs generic). For a fixture
//! whose first-seen group order equals sorted order, it also matches pandas 2.2.3.

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

fn pairs(s: &Series) -> Vec<(String, String)> {
    let labels: Vec<String> = s.index().labels().iter().map(|l| l.to_string()).collect();
    let vals: Vec<String> = s
        .column()
        .values()
        .iter()
        .map(|v| match v {
            Scalar::Utf8(x) => x.clone(),
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    labels.into_iter().zip(vals).collect()
}

fn run(by: &[&str], v: &[&str], v_contig: bool, want_max: bool) -> Vec<(String, String)> {
    let n = by.len();
    let byc = series("k", contig(by), n);
    let vc = series("v", if v_contig { contig(v) } else { scalar(v) }, n);
    let gb = vc.groupby(&byc).unwrap();
    let r = if want_max {
        gb.max().unwrap()
    } else {
        gb.min().unwrap()
    };
    pairs(&r)
}

// Sorted-order fixture (first-seen a,b == sorted): dense == generic == pandas.
#[test]
fn dense_matches_generic_and_pandas_sorted() {
    let by = ["a", "a", "b", "a"];
    let v = ["x2", "x1", "y1", "x3"];
    for (want_max, want) in [
        (true, vec![("a", "x3"), ("b", "y1")]),
        (false, vec![("a", "x1"), ("b", "y1")]),
    ] {
        let dense = run(&by, &v, true, want_max);
        let generic = run(&by, &v, false, want_max);
        assert_eq!(dense, generic, "dense vs generic (max={want_max})");
        let want_owned: Vec<(String, String)> = want
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();
        assert_eq!(dense, want_owned, "vs pandas (max={want_max})");
    }
}

// First-seen != sorted: dense must still equal generic (bit-identity of the change),
// independent of the group-order policy.
#[test]
fn dense_matches_generic_first_seen() {
    let by = ["b", "a", "b", "a"];
    let v = ["p", "q", "r", "s"];
    for want_max in [true, false] {
        let dense = run(&by, &v, true, want_max);
        let generic = run(&by, &v, false, want_max);
        assert_eq!(dense, generic, "dense vs generic (max={want_max})");
    }
}
