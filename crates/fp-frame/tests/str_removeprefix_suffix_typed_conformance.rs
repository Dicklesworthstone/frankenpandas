//! No-mock conformance guard for Series.str.removeprefix / removesuffix routed
//! through apply_str_utf8 (contiguous output, borrowed-&str write). Values cross-
//! checked against pandas 2.2.3, including non-matching rows (passthrough), a
//! single-occurrence strip, and missing-fallback.

use fp_columnar::Column;
use fp_frame::Series;
use fp_index::Index;
use fp_types::{NullKind, Scalar};

fn series(vals: Vec<Scalar>) -> Series {
    let n = vals.len();
    Series::new(
        "v",
        Index::new_known_unique_int64_unit_range(0, n),
        Column::from_values(vals).unwrap(),
    )
    .unwrap()
}
fn u(s: &str) -> Scalar {
    Scalar::Utf8(s.to_string())
}
fn strs(s: &Series) -> Vec<Option<String>> {
    s.column()
        .values()
        .iter()
        .map(|v| match v {
            Scalar::Utf8(s) => Some(s.clone()),
            Scalar::Null(_) => None,
            other => panic!("unexpected {other:?}"),
        })
        .collect()
}

// pandas removeprefix('item_'): ['abc','no_prefix','item_x','']
#[test]
fn removeprefix_matches_pandas() {
    let s = series(vec![
        u("item_abc"),
        u("no_prefix"),
        u("item_item_x"),
        u("item_"),
    ]);
    let r = s.str().removeprefix("item_").unwrap();
    assert_eq!(
        strs(&r),
        vec![
            Some("abc".into()),
            Some("no_prefix".into()),
            Some("item_x".into()),
            Some("".into())
        ]
    );
}

// pandas removesuffix('_xyz'): ['a','b','_xyz','']
#[test]
fn removesuffix_matches_pandas() {
    let s = series(vec![u("a_xyz"), u("b"), u("_xyz_xyz"), u("_xyz")]);
    let r = s.str().removesuffix("_xyz").unwrap();
    assert_eq!(
        strs(&r),
        vec![
            Some("a".into()),
            Some("b".into()),
            Some("_xyz".into()),
            Some("".into())
        ]
    );
}

#[test]
fn removeprefix_suffix_missing_fallback() {
    let s = series(vec![u("item_a"), Scalar::Null(NullKind::NaN), u("item_b")]);
    assert_eq!(
        strs(&s.str().removeprefix("item_").unwrap()),
        vec![Some("a".into()), None, Some("b".into())]
    );
    let s2 = series(vec![u("a_xyz"), Scalar::Null(NullKind::NaN)]);
    assert_eq!(
        strs(&s2.str().removesuffix("_xyz").unwrap()),
        vec![Some("a".into()), None]
    );
}
