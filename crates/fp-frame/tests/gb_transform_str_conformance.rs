//! No-mock conformance guard for GroupBy.transform over a Utf8 value (the
//! try_transform_dense Utf8 broadcast arm). It fires for a contiguous-Utf8 value;
//! a Scalar-backed value bails to the generic build_groups path. Same data both
//! ways must yield the byte-identical broadcast (dense vs generic) and match
//! pandas 2.2.3 (per-group reduction broadcast to every row, row-aligned).

use fp_columnar::Column;
use fp_frame::Series;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;

const K: [&str; 4] = ["a", "a", "b", "a"];
const V: [&str; 4] = ["p", "q", "r", "s"];

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
fn cells(s: &Series) -> Vec<String> {
    s.column()
        .values()
        .iter()
        .map(|v| match v {
            Scalar::Utf8(x) => x.clone(),
            Scalar::Int64(x) => x.to_string(),
            other => panic!("unexpected {other:?}"),
        })
        .collect()
}

fn run(func: &str, v_contig: bool) -> Vec<String> {
    let by = series("k", contig(&K), 4);
    let v = series("v", if v_contig { contig(&V) } else { scalar(&V) }, 4);
    cells(&v.groupby(&by).unwrap().transform(func).unwrap())
}

#[test]
fn transform_utf8_matches_generic_and_pandas() {
    let expected: &[(&str, Vec<&str>)] = &[
        ("first", vec!["p", "p", "r", "p"]),
        ("last", vec!["s", "s", "r", "s"]),
        ("max", vec!["s", "s", "r", "s"]),
        ("min", vec!["p", "p", "r", "p"]),
        ("count", vec!["3", "3", "1", "3"]),
    ];
    for (func, want) in expected {
        let dense = run(func, true);
        let generic = run(func, false);
        assert_eq!(dense, generic, "{func}: dense vs generic");
        let want_owned: Vec<String> = want.iter().map(|s| s.to_string()).collect();
        assert_eq!(dense, want_owned, "{func}: vs pandas");
    }
}
