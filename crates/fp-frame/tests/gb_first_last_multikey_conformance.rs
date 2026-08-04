//! No-mock conformance guard for the MULTI-key first/last dense path
//! (try_first_last_dense extended to >=2 keys via multi_mixed_dense_grouping). The
//! dense path needs contiguous-Utf8 keys; a Scalar-backed key bails to the generic
//! aggregate. So the same data built both ways must yield the byte-identical value
//! sequence (dense vs generic) — and match pandas 2.2.3 (first/last row per group,
//! sorted-key MultiIndex).

use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;

const K1: [&str; 5] = ["b", "a", "b", "a", "c"];
const K2: [&str; 5] = ["y", "x", "x", "x", "x"];
const V: [&str; 5] = ["v1", "v2", "v3", "v4", "v5"];

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
        "k1".to_string(),
        if k_contig { contig(&K1) } else { scalar(&K1) },
    );
    cols.insert("k2".to_string(), contig(&K2));
    cols.insert("v".to_string(), contig(&V));
    DataFrame::new_with_column_order(index, cols, vec!["k1".into(), "k2".into(), "v".into()])
        .unwrap()
}

fn vals(df: &DataFrame) -> Vec<String> {
    df.column("v")
        .unwrap()
        .values()
        .iter()
        .map(|v| match v {
            Scalar::Utf8(s) => s.clone(),
            other => panic!("unexpected {other:?}"),
        })
        .collect()
}

#[test]
fn multikey_dense_matches_generic_and_pandas() {
    for (op, want) in [
        ("first", vec!["v2", "v3", "v1", "v5"]),
        ("last", vec!["v4", "v3", "v1", "v5"]),
    ] {
        let run = |df: &DataFrame| -> DataFrame {
            let gb = df.groupby(&["k1", "k2"]).unwrap();
            if op == "first" {
                gb.first().unwrap()
            } else {
                gb.last().unwrap()
            }
        };
        let dense = run(&frame(true));
        let generic = run(&frame(false));
        assert_eq!(vals(&dense), vals(&generic), "{op}: dense vs generic");
        assert_eq!(vals(&dense), want, "{op}: vs pandas");
        // sorted-key flat index label
        let idx: Vec<String> = dense
            .index()
            .labels()
            .iter()
            .map(|l| l.to_string())
            .collect();
        assert_eq!(
            idx,
            vec!["a, x", "b, x", "b, y", "c, x"],
            "{op}: sorted MultiIndex"
        );
    }
}
