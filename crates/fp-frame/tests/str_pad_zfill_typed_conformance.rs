//! No-mock conformance guard for Series.str.pad / center / ljust / rjust / zfill
//! routed through apply_str_utf8 (contiguous byte-buffer output). Values cross-
//! checked against pandas 2.2.3, including the CPython-center both-split, the
//! zfill sign handling, the already-wide pass-through, and missing-fallback.

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
fn some(v: &[&str]) -> Vec<Option<String>> {
    v.iter().map(|s| Some((*s).to_string())).collect()
}

#[test]
fn pad_both_cpython_center() {
    let s = series(vec![u("ab"), u("xyz"), u("hello")]);
    let r = s.str().pad(5, "both", '*').unwrap();
    assert_eq!(strs(&r), some(&["**ab*", "*xyz*", "hello"]));
}

#[test]
fn center_and_ljust_rjust() {
    assert_eq!(
        strs(
            &series(vec![u("ab"), u("abc")])
                .str()
                .center(6, '-')
                .unwrap()
        ),
        some(&["--ab--", "-abc--"])
    );
    assert_eq!(
        strs(&series(vec![u("ab")]).str().ljust(5, '.').unwrap()),
        some(&["ab..."])
    );
    assert_eq!(
        strs(&series(vec![u("ab")]).str().rjust(5, '.').unwrap()),
        some(&["...ab"])
    );
}

#[test]
fn zfill_sign_and_passthrough() {
    let s = series(vec![u("-12"), u("12"), u("+7"), u("abcdef")]);
    let r = s.str().zfill(6).unwrap();
    assert_eq!(strs(&r), some(&["-00012", "000012", "+00007", "abcdef"]));
}

#[test]
fn pad_missing_fallback() {
    let s = series(vec![u("ab"), Scalar::Null(NullKind::NaN), u("cd")]);
    assert_eq!(
        strs(&s.str().pad(4, "left", '0').unwrap()),
        vec![Some("00ab".into()), None, Some("00cd".into())]
    );
    assert_eq!(
        strs(&s.str().zfill(4).unwrap()),
        vec![Some("00ab".into()), None, Some("00cd".into())]
    );
}
