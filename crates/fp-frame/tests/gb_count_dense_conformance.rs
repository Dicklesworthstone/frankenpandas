//! No-mock conformance guard for the GroupBy.count dense path (try_count_dense):
//! all-valid value columns => count == group size, no value materialization. Fires
//! for contiguous keys; a Scalar-backed key bails to the generic aggregate. Same
//! data both ways must give the byte-identical counts (dense vs generic) and match
//! pandas 2.2.3, for single and multi key.

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
fn scalar(v: &[&str]) -> Column {
    Column::from_values(v.iter().map(|s| Scalar::Utf8((*s).to_string())).collect()).unwrap()
}
fn counts(df: &DataFrame) -> Vec<i64> {
    df.column("v")
        .unwrap()
        .values()
        .iter()
        .map(|x| match x {
            Scalar::Int64(c) => *c,
            other => panic!("unexpected {other:?}"),
        })
        .collect()
}

#[test]
fn single_key_dense_matches_generic_and_pandas() {
    let mk = |k_contig: bool| -> DataFrame {
        let index = Index::new((0..5i64).map(IndexLabel::Int64).collect());
        let mut cols = BTreeMap::new();
        let k = ["b", "a", "b", "a", "c"];
        cols.insert(
            "k".to_string(),
            if k_contig { contig(&k) } else { scalar(&k) },
        );
        cols.insert("v".to_string(), contig(&["x1", "x2", "x3", "x4", "x5"]));
        DataFrame::new_with_column_order(index, cols, vec!["k".into(), "v".into()]).unwrap()
    };
    let dense = mk(true).groupby(&["k"]).unwrap().count().unwrap();
    let generic = mk(false).groupby(&["k"]).unwrap().count().unwrap();
    assert_eq!(counts(&dense), counts(&generic), "dense vs generic");
    assert_eq!(counts(&dense), vec![2, 2, 1], "vs pandas"); // a,b,c
}

#[test]
fn multi_key_dense_matches_generic_and_pandas() {
    let mk = |k_contig: bool| -> DataFrame {
        let index = Index::new((0..5i64).map(IndexLabel::Int64).collect());
        let mut cols = BTreeMap::new();
        let k1 = ["b", "a", "b", "a", "c"];
        cols.insert(
            "k1".to_string(),
            if k_contig { contig(&k1) } else { scalar(&k1) },
        );
        cols.insert("k2".to_string(), contig(&["y", "x", "x", "x", "x"]));
        cols.insert("v".to_string(), contig(&["a", "b", "c", "d", "e"]));
        DataFrame::new_with_column_order(index, cols, vec!["k1".into(), "k2".into(), "v".into()])
            .unwrap()
    };
    let dense = mk(true).groupby(&["k1", "k2"]).unwrap().count().unwrap();
    let generic = mk(false).groupby(&["k1", "k2"]).unwrap().count().unwrap();
    assert_eq!(counts(&dense), counts(&generic), "dense vs generic");
    assert_eq!(counts(&dense), vec![2, 1, 1, 1], "vs pandas"); // (a,x),(b,x),(b,y),(c,x)
}
