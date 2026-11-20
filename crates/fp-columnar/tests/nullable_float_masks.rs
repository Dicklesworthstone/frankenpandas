//! Masked float filling and selection must preserve a computed NaN as a
//! present value, independently of NA (br-frankenpandas-6f379).

use fp_columnar::{Column, ValidityMask};
use fp_types::{DType, NullKind, NumericWidth, Scalar};

fn input() -> Column {
    Column::new(
        DType::Float64Nullable,
        vec![
            Scalar::Float64(4.0),
            Scalar::Null(NullKind::Null),
            Scalar::Float64(-4.0),
            Scalar::Float64(0.0),
        ],
    )
    .unwrap()
}

fn assert_validity(column: &Column, expected: &[bool]) {
    assert_eq!(column.len(), expected.len());
    assert_eq!(
        (0..column.len())
            .map(|row| column.validity().get(row))
            .collect::<Vec<_>>(),
        expected
    );
}

fn assert_nan(value: &Scalar) {
    assert!(matches!(value, Scalar::Float64(number) if number.is_nan()));
}

#[test]
fn column_fill_preserves_present_nan_in_both_operands() {
    let source = input().sqrt().unwrap();
    let replacement = Column::from_nullable_f64_values_with_validity(
        vec![9.0, f64::NAN, 7.0, 8.0],
        ValidityMask::all_valid(4),
    );
    for source in [
        source.clone(),
        source.cast_to_width(NumericWidth::Float32, true).unwrap(),
    ] {
        let filled = source.fillna_with_column(&replacement).unwrap();
        assert_eq!(filled.dtype(), DType::Float64Nullable);
        assert_eq!(filled.width(), source.width());
        assert_validity(&filled, &[true; 4]);
        assert_eq!(filled.values()[0], Scalar::Float64(2.0));
        assert_nan(&filled.values()[1]);
        assert_nan(&filled.values()[2]);
        assert_eq!(filled.values()[3], Scalar::Float64(0.0));

        // The same NaN supplied by a numpy float column is missing.
        let plain = Column::from_f64_values(vec![9.0, f64::NAN, 7.0, 8.0]);
        let filled = source.fillna_with_column(&plain).unwrap();
        assert_validity(&filled, &[true, false, true, true]);
        assert_nan(&filled.values()[2]);

        let ints = Column::from_i64_values(vec![10, 11, 12, 13]);
        let filled = source.fillna_with_column(&ints).unwrap();
        assert_validity(&filled, &[true; 4]);
        assert_eq!(filled.values()[1], Scalar::Float64(11.0));
        assert_nan(&filled.values()[2]);
    }
    assert!(
        input()
            .fillna_with_column(&Column::from_f64_values(vec![]))
            .is_err()
    );
    let empty = Column::new(DType::Float64Nullable, Vec::new()).unwrap();
    assert_eq!(
        empty.fillna_with_column(&empty).unwrap().dtype(),
        DType::Float64Nullable
    );
}

#[test]
fn directional_fill_uses_present_nan_as_a_donor_and_resets_limits() {
    let mut validity = ValidityMask::all_valid(8);
    for row in [0, 2, 3, 5, 6] {
        validity.set(row, false);
    }
    let source = Column::from_nullable_f64_values_with_validity(
        vec![99.0, 2.0, 99.0, 99.0, f64::NAN, 99.0, 99.0, -0.0],
        validity,
    );
    for source in [
        source.clone(),
        source.cast_to_width(NumericWidth::Float32, true).unwrap(),
    ] {
        let forward = source.ffill(Some(1)).unwrap();
        assert_eq!(forward.width(), source.width());
        assert_validity(
            &forward,
            &[false, true, true, false, true, true, false, true],
        );
        assert_eq!(forward.values()[2], Scalar::Float64(2.0));
        assert_nan(&forward.values()[4]);
        assert_nan(&forward.values()[5]);
        let backward = source.bfill(Some(1)).unwrap();
        assert_validity(
            &backward,
            &[true, true, false, true, true, false, true, true],
        );
        assert_eq!(backward.values()[0], Scalar::Float64(2.0));
        assert_nan(&backward.values()[3]);
        assert_nan(&backward.values()[4]);
        assert_eq!(
            backward.as_masked_f64_slice().unwrap().0[6].to_bits(),
            (-0.0_f64).to_bits()
        );
        let unbounded = source.ffill(None).unwrap();
        assert_validity(
            &unbounded,
            &[false, true, true, true, true, true, true, true],
        );
        assert_nan(&unbounded.values()[6]);
        let unchanged = source.bfill(Some(0)).unwrap();
        assert_eq!(unchanged.validity(), source.validity());
        assert_nan(&unchanged.values()[4]);
    }
    let plain = Column::from_f64_values(vec![1.0, f64::NAN, 3.0]);
    assert_eq!(
        plain.ffill(None).unwrap().values(),
        &[
            Scalar::Float64(1.0),
            Scalar::Float64(1.0),
            Scalar::Float64(3.0),
        ]
    );
    let empty = Column::new(DType::Float64Nullable, Vec::new()).unwrap();
    assert_eq!(empty.ffill(None).unwrap().dtype(), DType::Float64Nullable);
    assert_eq!(empty.bfill(None).unwrap().dtype(), DType::Float64Nullable);
}

#[test]
fn conditional_selection_carries_the_selected_payload_and_mask() {
    let source = input().sqrt().unwrap();
    let replacement = Column::from_nullable_f64_values_with_validity(
        vec![f64::NAN, 7.0, 8.0, f64::NAN],
        ValidityMask::all_valid(4),
    );
    let cond = Column::new(
        DType::BoolNullable,
        vec![
            Scalar::Bool(false),
            Scalar::Bool(true),
            Scalar::Bool(true),
            Scalar::Null(NullKind::Null),
        ],
    )
    .unwrap();
    for source in [
        source.clone(),
        source.cast_to_width(NumericWidth::Float32, true).unwrap(),
    ] {
        let selected = source.where_cond_series(&cond, &replacement).unwrap();
        assert_eq!(selected.dtype(), DType::Float64Nullable);
        assert_eq!(selected.width(), source.width());
        assert_validity(&selected, &[true, false, true, true]);
        for row in [0, 2, 3] {
            assert_nan(&selected.values()[row]);
        }
        let masked = source.mask_series(&cond, &replacement).unwrap();
        assert_validity(&masked, &[true; 4]);
        assert_eq!(
            &masked.values()[..3],
            &[
                Scalar::Float64(2.0),
                Scalar::Float64(7.0),
                Scalar::Float64(8.0),
            ]
        );
        assert_nan(&masked.values()[3]);
        let scalar = source.where_cond(&cond, &Scalar::Float64(1.25)).unwrap();
        assert_validity(&scalar, &[true, false, true, true]);
        assert_nan(&scalar.values()[2]);
        assert_eq!(scalar.values()[3], Scalar::Float64(1.25));
        let missing = source.mask(&cond, &Scalar::Float64(f64::NAN)).unwrap();
        assert_validity(&missing, &[true, false, false, false]);
    }
    // Unselected invalid replacements must not be cast.
    let keep = Column::from_bool_values(vec![true; 4]);
    let unchanged = source
        .where_cond(&keep, &Scalar::Utf8("invalid".to_owned()))
        .unwrap();
    assert_nan(&unchanged.values()[2]);
    assert!(
        source
            .where_cond(
                &Column::from_bool_values(vec![false; 4]),
                &Scalar::Utf8("invalid".to_owned()),
            )
            .is_err()
    );
}
