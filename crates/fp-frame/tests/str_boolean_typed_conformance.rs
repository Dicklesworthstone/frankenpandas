//! No-mock conformance guard for the typed-Bool-output fast path in
//! `str_boolean_with_na` (behind Series.str.contains/match/startswith/endswith
//! with options — pandas' default regex path). An all-valid Utf8 input now emits a
//! typed `Vec<bool>` -> `from_bool_values` instead of `Vec<Scalar::Bool>` ->
//! `from_values`. Asserts the resulting Bool values against hand-computed expected
//! (pandas semantics), including a literal, a regex, and a startswith anchor.

use fp_columnar::Column;
use fp_frame::Series;
use fp_index::Index;
use fp_types::Scalar;

fn series(items: &[&str]) -> Series {
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
fn contains_literal_typed() {
    let s = series(&["apple", "banana", "cherry", "apricot"]);
    let r = s
        .str()
        .contains_with_options("ap", true, None, false)
        .unwrap();
    assert_eq!(bools(&r), vec![true, false, false, true]);
}

#[test]
fn contains_regex_typed() {
    let s = series(&["apple", "banana", "cherry", "apricot"]);
    // regex "an" matches "banana" only
    let r = s
        .str()
        .contains_with_options("an", true, None, true)
        .unwrap();
    assert_eq!(bools(&r), vec![false, true, false, false]);
}

#[test]
fn contains_regex_anchored_typed() {
    let s = series(&["user_a", "admin", "user_b", "guest"]);
    let r = s
        .str()
        .contains_with_options("^user", true, None, true)
        .unwrap();
    assert_eq!(bools(&r), vec![true, false, true, false]);
}

#[test]
fn contains_case_insensitive_typed() {
    let s = series(&["Apple", "BANANA", "cherry"]);
    let r = s
        .str()
        .contains_with_options("a", false, None, true)
        .unwrap();
    // case-insensitive 'a' present in Apple, BANANA; not in cherry
    assert_eq!(bools(&r), vec![true, true, false]);
}

#[test]
fn empty_series_typed() {
    let s = series(&[]);
    let r = s
        .str()
        .contains_with_options("x", true, None, true)
        .unwrap();
    assert_eq!(bools(&r), Vec::<bool>::new());
}
