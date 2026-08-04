//! No-mock conformance guard for routing Series.str.replace / repeat through the
//! contiguous-buffer `apply_str_utf8` (instead of Vec<Scalar::Utf8> + from_values).
//! Asserts the resulting strings against hand-computed expected, including the
//! all-valid fast path AND a missing-bearing series (which must hit the fallback
//! and preserve nulls).

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

#[test]
fn replace_all_valid() {
    let s = series(vec![u("item_1"), u("xitemy"), u("no"), u("itemitem")]);
    let r = s.str().replace("item", "ROW").unwrap();
    assert_eq!(
        strs(&r),
        vec![
            Some("ROW_1".to_string()),
            Some("xROWy".to_string()),
            Some("no".to_string()),
            Some("ROWROW".to_string()),
        ]
    );
}

#[test]
fn repeat_all_valid() {
    let s = series(vec![u("ab"), u("c"), u("")]);
    let r = s.str().repeat(3).unwrap();
    assert_eq!(
        strs(&r),
        vec![
            Some("ababab".to_string()),
            Some("ccc".to_string()),
            Some("".to_string()),
        ]
    );
}

// missing-bearing series must hit the apply_str fallback and preserve nulls
#[test]
fn replace_with_missing_falls_back() {
    let s = series(vec![u("item_a"), Scalar::Null(NullKind::NaN), u("item_b")]);
    let r = s.str().replace("item", "X").unwrap();
    assert_eq!(
        strs(&r),
        vec![Some("X_a".to_string()), None, Some("X_b".to_string())]
    );
}

#[test]
fn repeat_with_missing_falls_back() {
    let s = series(vec![u("z"), Scalar::Null(NullKind::NaN)]);
    let r = s.str().repeat(2).unwrap();
    assert_eq!(strs(&r), vec![Some("zz".to_string()), None]);
}
