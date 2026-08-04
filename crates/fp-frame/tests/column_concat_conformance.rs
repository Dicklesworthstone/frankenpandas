//! No-mock conformance guard for the typed Column::concat lever (br-frankenpandas-3j3ku):
//! the typed Int64/Float64 buffer concat must be BIT-TRANSPARENT vs the Scalar path —
//! same dtype, same values. Covers all-valid Int64 + Float64 (typed paths) and an
//! Int64+nullable-Int64 mix (Scalar fallback). Compiled via `cargo check --tests`; full
//! run batch-pending.

use fp_columnar::{Column, ValidityMask};
use fp_types::{DType, Scalar};

fn ints(c: &Column) -> Vec<i64> {
    c.values()
        .iter()
        .map(|v| match v {
            Scalar::Int64(x) => *x,
            _ => i64::MIN,
        })
        .collect()
}

#[test]
fn column_concat_int64_typed_exact() {
    let a = Column::from_i64_values(vec![1, 2, 3]);
    let b = Column::from_i64_values(vec![4, 5]);
    let out = a.concat(&b).unwrap();
    assert_eq!(out.dtype(), DType::Int64);
    assert_eq!(ints(&out), vec![1, 2, 3, 4, 5]);
}

#[test]
fn column_concat_float64_typed_exact() {
    let a = Column::from_f64_values(vec![1.5, 2.5]);
    let b = Column::from_f64_values(vec![3.5]);
    let out = a.concat(&b).unwrap();
    assert_eq!(out.dtype(), DType::Float64);
    let got: Vec<f64> = out
        .values()
        .iter()
        .map(|v| match v {
            Scalar::Float64(x) => *x,
            _ => f64::NAN,
        })
        .collect();
    assert_eq!(got, vec![1.5, 2.5, 3.5]);
}

#[test]
fn column_concat_nullable_int64_scalar_fallback_exact() {
    // b has a missing slot, so as_i64_slice declines -> Scalar concat path. The
    // typed-path guard (above) and this fallback must agree on the present values.
    let a = Column::from_i64_values(vec![10, 20]);
    let mut validity = ValidityMask::all_valid(2);
    validity.set(1, false);
    let b = Column::from_i64_values_with_validity(vec![30, 0], validity);
    let out = a.concat(&b).unwrap();
    assert_eq!(out.dtype(), DType::Int64);
    assert_eq!(out.values().len(), 4);
    assert!(matches!(out.values()[0], Scalar::Int64(10)));
    assert!(matches!(out.values()[2], Scalar::Int64(30)));
    assert!(out.values()[3].is_missing(), "nullable slot stays missing");
}
