//! No-mock conformance guard for Series.str.capitalize / title routed through
//! apply_str_utf8 (+ capitalize's ASCII byte-ops fast path). Pins the output
//! strings (hand-computed pandas semantics) across ASCII, non-ASCII (Unicode case
//! folding), empty, single-char, and missing-bearing (fallback) inputs.

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
fn capitalize_ascii() {
    let s = series(vec![u("hELLo"), u("WORLD"), u("a"), u(""), u("ab_CD")]);
    let r = s.str().capitalize().unwrap();
    assert_eq!(
        strs(&r),
        vec![
            Some("Hello".into()),
            Some("World".into()),
            Some("A".into()),
            Some("".into()),
            Some("Ab_cd".into()),
        ]
    );
}

// non-ASCII must take the Unicode path: "éXY" -> "Éxy"
#[test]
fn capitalize_unicode() {
    let s = series(vec![u("éXY"), u("ÜBER")]);
    let r = s.str().capitalize().unwrap();
    assert_eq!(strs(&r), vec![Some("Éxy".into()), Some("Über".into())]);
}

#[test]
fn capitalize_missing_fallback() {
    let s = series(vec![u("hI"), Scalar::Null(NullKind::NaN), u("Bye")]);
    let r = s.str().capitalize().unwrap();
    assert_eq!(strs(&r), vec![Some("Hi".into()), None, Some("Bye".into())]);
}

#[test]
fn title_ascii() {
    let s = series(vec![u("hello world"), u("foo-bar baz"), u("aB cD")]);
    let r = s.str().title().unwrap();
    assert_eq!(
        strs(&r),
        vec![
            Some("Hello World".into()),
            Some("Foo-Bar Baz".into()),
            Some("Ab Cd".into()),
        ]
    );
}

#[test]
fn title_missing_fallback() {
    let s = series(vec![u("hi there"), Scalar::Null(NullKind::NaN)]);
    let r = s.str().title().unwrap();
    assert_eq!(strs(&r), vec![Some("Hi There".into()), None]);
}
