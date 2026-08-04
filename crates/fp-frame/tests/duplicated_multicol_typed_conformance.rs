//! No-mock conformance guard for the multi-column duplicated_mask typed raw path
//! (Int64 + contiguous-Utf8, not just Float64). The typed path fires only when
//! EVERY selected column has a typed contiguous backing; a single Scalar-backed
//! column forces the generic Scalar-digest path. So the same data built both ways
//! must yield byte-identical masks (dense-vs-generic bit-identity) across all keep
//! variants — and match pandas 2.2.3.

use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{DuplicateKeep, Index, IndexLabel};
use fp_types::Scalar;

const K1: [&str; 5] = ["a", "a", "b", "a", "b"];
const K2: [&str; 5] = ["x", "x", "x", "x", "x"];
const V3: [i64; 5] = [1, 1, 1, 2, 1];

fn contig_utf8(v: &[&str]) -> Column {
    let mut bytes = Vec::new();
    let mut offsets = vec![0usize];
    for s in v {
        bytes.extend_from_slice(s.as_bytes());
        offsets.push(bytes.len());
    }
    Column::from_utf8_contiguous(bytes, offsets)
}
fn scalar_utf8(v: &[&str]) -> Column {
    Column::from_values(v.iter().map(|s| Scalar::Utf8((*s).to_string())).collect()).unwrap()
}

fn frame(typed: bool) -> DataFrame {
    let index = Index::new((0..5i64).map(IndexLabel::Int64).collect());
    let mut cols = BTreeMap::new();
    // typed: contiguous Utf8 + raw Int64. generic: one scalar-backed Utf8 column
    // (forces typed_cols=None) so the whole mask flows through the Scalar path.
    cols.insert(
        "k1".to_string(),
        if typed {
            contig_utf8(&K1)
        } else {
            scalar_utf8(&K1)
        },
    );
    cols.insert("k2".to_string(), contig_utf8(&K2));
    cols.insert("v3".to_string(), Column::from_i64_values(V3.to_vec()));
    DataFrame::new_with_column_order(index, cols, vec!["k1".into(), "k2".into(), "v3".into()])
        .unwrap()
}

fn mask(df: &DataFrame, keep: DuplicateKeep) -> Vec<bool> {
    let subset = ["k1".to_string(), "k2".to_string(), "v3".to_string()];
    df.duplicated(Some(&subset), keep)
        .unwrap()
        .values()
        .iter()
        .map(|v| match v {
            Scalar::Bool(b) => *b,
            other => panic!("unexpected {other:?}"),
        })
        .collect()
}

#[test]
fn typed_matches_generic_and_pandas() {
    let expected: &[(DuplicateKeep, Vec<bool>)] = &[
        (DuplicateKeep::First, vec![false, true, false, false, true]),
        (DuplicateKeep::Last, vec![true, false, true, false, false]),
        (DuplicateKeep::None, vec![true, true, true, false, true]),
    ];
    for (keep, want) in expected {
        let typed = mask(&frame(true), *keep);
        let generic = mask(&frame(false), *keep);
        assert_eq!(typed, generic, "{keep:?}: typed vs generic");
        assert_eq!(&typed, want, "{keep:?}: vs pandas");
    }
}
