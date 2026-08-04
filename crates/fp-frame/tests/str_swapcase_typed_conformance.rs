//! No-mock conformance guard for Series.str.swapcase routed through apply_str_utf8
//! with the ASCII XOR-0x20 fast path. Values cross-checked against pandas 2.2.3.

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

// pandas: ['hELLO world','Ab3Cd','ÉàB','mIxEd','']
#[test]
fn swapcase_ascii_and_unicode() {
    let s = series(vec![
        u("Hello WORLD"),
        u("aB3cD"),
        u("éÀb"),
        u("MiXeD"),
        u(""),
    ]);
    let r = s.str().swapcase().unwrap();
    assert_eq!(
        strs(&r),
        vec![
            Some("hELLO world".into()),
            Some("Ab3Cd".into()),
            Some("ÉàB".into()),
            Some("mIxEd".into()),
            Some("".into()),
        ]
    );
}

#[test]
fn swapcase_missing_fallback() {
    let s = series(vec![u("Ab"), Scalar::Null(NullKind::NaN), u("cD")]);
    let r = s.str().swapcase().unwrap();
    assert_eq!(strs(&r), vec![Some("aB".into()), None, Some("Cd".into())]);
}
