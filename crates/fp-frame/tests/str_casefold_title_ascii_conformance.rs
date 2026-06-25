//! No-mock conformance guard for Series.str.casefold (ASCII make_ascii_lowercase
//! fast path + Unicode case_fold fallback) and the title ASCII byte-ops path.
//! Values cross-checked against pandas 2.2.3.

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

// pandas: ['hello','mixed','strasse','éà','abc_123']
#[test]
fn casefold_ascii_and_unicode() {
    let s = series(vec![
        u("HELLO"),
        u("MiXeD"),
        u("Straße"),
        u("ÉÀ"),
        u("abc_123"),
    ]);
    let r = s.str().casefold().unwrap();
    assert_eq!(
        strs(&r),
        vec![
            Some("hello".into()),
            Some("mixed".into()),
            Some("strasse".into()),
            Some("éà".into()),
            Some("abc_123".into()),
        ]
    );
}

#[test]
fn casefold_missing_fallback() {
    let s = series(vec![u("AB"), Scalar::Null(NullKind::NaN), u("Cd")]);
    let r = s.str().casefold().unwrap();
    assert_eq!(strs(&r), vec![Some("ab".into()), None, Some("cd".into())]);
}

// title ASCII path: re-verify (now byte-ops) — pandas semantics
#[test]
fn title_ascii_byteops() {
    let s = series(vec![u("hello world"), u("foo-bar baz"), u("aB3cD"), u("")]);
    let r = s.str().title().unwrap();
    assert_eq!(
        strs(&r),
        vec![
            Some("Hello World".into()),
            Some("Foo-Bar Baz".into()),
            Some("Ab3Cd".into()),
            Some("".into()),
        ]
    );
}

#[test]
fn title_unicode_keeps_path() {
    // non-ASCII keeps the Unicode path
    let s = series(vec![u("éàb cd")]);
    let r = s.str().title().unwrap();
    assert_eq!(strs(&r), vec![Some("Éàb Cd".into())]);
}
