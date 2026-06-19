//! No-mock conformance guard for the typed Bool any/all lever (br-frankenpandas-xa8di):
//! the as_bool_slice fast path (all-valid Bool) and the Scalar fallback (Bool with
//! missing) must both match pandas any/all semantics (skipna=True: missing skipped;
//! empty -> any=false, all=true). Compiled via `cargo check --tests`; full run batch-pending.

use fp_columnar::Column;
use fp_frame::Series;
use fp_index::{Index, IndexLabel};
use fp_types::{NullKind, Scalar};

fn bool_series(vals: &[bool]) -> Series {
    Series::from_values(
        "b",
        (0..vals.len() as i64).map(IndexLabel::Int64).collect(),
        vals.iter().map(|&x| Scalar::Bool(x)).collect(),
    )
    .unwrap()
}

#[test]
fn any_all_typed_bool_all_valid() {
    // all-valid Bool -> as_bool_slice typed fast path
    assert_eq!(bool_series(&[true, false, true]).any().unwrap(), true);
    assert_eq!(bool_series(&[true, false, true]).all().unwrap(), false);
    assert_eq!(bool_series(&[true, true]).any().unwrap(), true);
    assert_eq!(bool_series(&[true, true]).all().unwrap(), true);
    assert_eq!(bool_series(&[false, false]).any().unwrap(), false);
    assert_eq!(bool_series(&[false, false]).all().unwrap(), false);
}

#[test]
fn any_all_bool_with_missing_scalar_fallback() {
    // a missing slot makes as_bool_slice decline -> Scalar fallback; skipna=True skips it.
    let with_missing = |vals: Vec<Scalar>| {
        Series::from_values(
            "b",
            (0..vals.len() as i64).map(IndexLabel::Int64).collect(),
            vals,
        )
        .unwrap()
    };
    let s = with_missing(vec![
        Scalar::Bool(false),
        Scalar::Null(NullKind::Null),
        Scalar::Bool(true),
    ]);
    assert_eq!(s.any().unwrap(), true); // a true is present
    assert_eq!(s.all().unwrap(), false); // a false is present
    let s2 = with_missing(vec![Scalar::Bool(true), Scalar::Null(NullKind::Null)]);
    assert_eq!(s2.any().unwrap(), true);
    assert_eq!(s2.all().unwrap(), true); // missing skipped, remaining all true
}

#[test]
fn any_all_empty_bool() {
    let empty = Series::new("b", Index::new(vec![]), Column::from_bool_values(vec![])).unwrap();
    assert_eq!(empty.any().unwrap(), false);
    assert_eq!(empty.all().unwrap(), true);
}
