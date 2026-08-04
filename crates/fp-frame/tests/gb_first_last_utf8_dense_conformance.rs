//! No-mock conformance guard for the GroupBy.first/last Utf8-key dense path
//! (try_first_last_dense factorize branch). The dense path fires only for a
//! contiguous-Utf8 key; a Scalar-backed key bails to the generic aggregate path.
//! So the same data built both ways must yield the byte-identical (label, value)
//! sequence — and match pandas 2.2.3 (sorted-key index, first/last row per group).

use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;

const K: [&str; 5] = ["b", "a", "b", "a", "c"];
const V: [&str; 5] = ["x1", "x2", "x3", "x4", "x5"];

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

fn frame(k_contig: bool) -> DataFrame {
    let index = Index::new((0..5i64).map(IndexLabel::Int64).collect());
    let mut cols = BTreeMap::new();
    cols.insert(
        "k".to_string(),
        if k_contig { contig(&K) } else { scalar(&K) },
    );
    cols.insert("v".to_string(), contig(&V));
    DataFrame::new_with_column_order(index, cols, vec!["k".into(), "v".into()]).unwrap()
}

fn pairs(df: &DataFrame) -> Vec<(String, String)> {
    let labels: Vec<String> = df.index().labels().iter().map(|l| l.to_string()).collect();
    let vals: Vec<String> = df
        .column("v")
        .unwrap()
        .values()
        .iter()
        .map(|v| match v {
            Scalar::Utf8(s) => s.clone(),
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    labels.into_iter().zip(vals).collect()
}

#[test]
fn first_last_dense_matches_generic_and_pandas() {
    let expected: &[(&str, Vec<(&str, &str)>)] = &[
        ("first", vec![("a", "x2"), ("b", "x1"), ("c", "x5")]),
        ("last", vec![("a", "x4"), ("b", "x3"), ("c", "x5")]),
        ("max", vec![("a", "x4"), ("b", "x3"), ("c", "x5")]),
        ("min", vec![("a", "x2"), ("b", "x1"), ("c", "x5")]),
    ];
    for (op, want) in expected {
        let run = |df: &DataFrame| -> DataFrame {
            let gb = df.groupby(&["k"]).unwrap();
            match *op {
                "first" => gb.first().unwrap(),
                "last" => gb.last().unwrap(),
                "max" => gb.max().unwrap(),
                "min" => gb.min().unwrap(),
                _ => unreachable!(),
            }
        };
        let dense = run(&frame(true)); // contiguous key -> factorize dense path
        let generic = run(&frame(false)); // scalar-backed key -> generic aggregate
        assert_eq!(pairs(&dense), pairs(&generic), "{op}: dense vs generic");
        let want_owned: Vec<(String, String)> = want
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();
        assert_eq!(pairs(&dense), want_owned, "{op}: vs pandas");
    }
}
