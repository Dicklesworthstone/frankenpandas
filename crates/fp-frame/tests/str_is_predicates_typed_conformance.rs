//! No-mock conformance guard for the Series.str is* predicates routed through the
//! typed `apply_str_bool` (Vec<bool> -> from_bool_values, no Vec<Scalar::Bool>).
//! Values cross-checked against pandas 2.2.3.

use fp_columnar::Column;
use fp_frame::Series;
use fp_index::Index;
use fp_types::Scalar;

fn fixture() -> Series {
    let items = ["hello", "ABC", "abc123", "  ", "Title Case", "123", ""];
    let n = items.len();
    Series::new(
        "v",
        Index::new_known_unique_int64_unit_range(0, n),
        Column::from_values(
            items
                .iter()
                .map(|s| Scalar::Utf8((*s).to_string()))
                .collect(),
        )
        .unwrap(),
    )
    .unwrap()
}
fn bools(s: &Series) -> Vec<bool> {
    s.column()
        .values()
        .iter()
        .map(|v| match v {
            Scalar::Bool(b) => *b,
            other => panic!("expected Bool, got {other:?}"),
        })
        .collect()
}

#[test]
fn islower_matches_pandas() {
    assert_eq!(
        bools(&fixture().str().islower().unwrap()),
        vec![true, false, true, false, false, false, false]
    );
}
#[test]
fn isalnum_matches_pandas() {
    assert_eq!(
        bools(&fixture().str().isalnum().unwrap()),
        vec![true, true, true, false, false, true, false]
    );
}
#[test]
fn isdigit_matches_pandas() {
    assert_eq!(
        bools(&fixture().str().isdigit().unwrap()),
        vec![false, false, false, false, false, true, false]
    );
}
#[test]
fn isalpha_matches_pandas() {
    assert_eq!(
        bools(&fixture().str().isalpha().unwrap()),
        vec![true, true, false, false, false, false, false]
    );
}
#[test]
fn isupper_matches_pandas() {
    assert_eq!(
        bools(&fixture().str().isupper().unwrap()),
        vec![false, true, false, false, false, false, false]
    );
}
#[test]
fn istitle_matches_pandas() {
    assert_eq!(
        bools(&fixture().str().istitle().unwrap()),
        vec![false, false, false, false, true, false, false]
    );
}
