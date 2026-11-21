//! Sparse columns retain their storage and dtype through public frame APIs.
//! The expected sparse index/density follows pandas 2.2.3, including explicit
//! entries that become equal to the fill after arithmetic (o1qso).

use fp_columnar::{ArithmeticOp, Column, SparseColumn};
use fp_frame::{DataFrame, Series};
use fp_index::Index;
use fp_types::{DType, NumericWidth, Scalar, SparseDType};

#[test]
fn sparse_frame_pipeline_preserves_storage_without_materializing_values() {
    let series = Series::from_sparse_dense(
        "values",
        vec!["a".into(), "b".into(), "c".into(), "d".into()],
        vec![
            Scalar::Int64(0),
            Scalar::Int64(9),
            Scalar::Int64(0),
            Scalar::Int64(5),
        ],
        DType::Int64,
        Scalar::Int64(0),
    )
    .unwrap();
    assert!(!series.column().scalar_cache_is_materialized());
    let frame = DataFrame::from_series(vec![series]).unwrap();
    let selected = frame.take(&[3, 0, 1, 3], 0).unwrap();
    let series = selected.to_series_dict().remove("values").unwrap();
    assert_eq!(series.dtype_name(), "Sparse[int64, 0]");
    let sparse = series.sparse().unwrap();
    assert_eq!(sparse.sp_index(), vec![0, 2, 3]);
    assert_eq!(
        sparse.sp_values(),
        vec![Scalar::Int64(5), Scalar::Int64(9), Scalar::Int64(5)]
    );
    assert_eq!(sparse.density(), 0.75);
    assert!(!series.column().scalar_cache_is_materialized());
    let dense = sparse.to_dense().unwrap();
    assert_eq!(dense.index(), series.index());
    assert_eq!(dense.name(), series.name());
    assert_eq!(dense.dtype(), DType::Int64);
    assert_eq!(
        dense.values(),
        &[
            Scalar::Int64(5),
            Scalar::Int64(0),
            Scalar::Int64(9),
            Scalar::Int64(5)
        ]
    );
}

#[test]
fn sparse_arithmetic_and_serialization_keep_explicit_fill_entries() {
    let input = Series::from_sparse_dense(
        "values",
        vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
        vec![Scalar::Int64(0), Scalar::Int64(7), Scalar::Int64(0)],
        DType::Int64,
        Scalar::Int64(0),
    )
    .unwrap();
    // pandas keeps the stored position even though its value becomes zero.
    let result = input.sub(&input).unwrap();
    let json = serde_json::to_string(&result).unwrap();
    let restored: Series = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.dtype_name(), "Sparse[int64, 0]");
    assert_eq!(
        restored.column().sparse().unwrap().sparse_dtype().fill_numpy_dtype.as_deref(),
        Some("int64")
    );
    let sparse = restored.sparse().unwrap();
    assert_eq!(sparse.npoints(), 1);
    assert_eq!(sparse.sp_index(), vec![1]);
    assert_eq!(sparse.sp_values(), vec![Scalar::Int64(0)]);
    assert_eq!(sparse.density(), 1.0 / 3.0);
}

#[test]
fn sparse_scalar_arithmetic_updates_the_fill_and_keeps_numeric_width() {
    let dtype = SparseDType {
        value_dtype: DType::Int64,
        fill_value: Scalar::Int64(0),
        width: Some(NumericWidth::Int16),
        fill_numpy_dtype: None,
    };
    let column = Column::from_sparse(
        SparseColumn::from_parts(
            dtype,
            4,
            vec![1, 3],
            vec![Scalar::Int64(0), Scalar::Int64(23)],
        )
        .unwrap(),
    );
    let column = column
        .binary_scalar(&Scalar::Int64(2), ArithmeticOp::Add, false)
        .unwrap();
    let series = Series::new(
        "values",
        Index::new_known_unique_int64_unit_range(0, 4),
        column,
    )
    .unwrap();
    assert_eq!(series.dtype_name(), "Sparse[int16, 2]");
    assert_eq!(
        series.column().sparse().unwrap().sparse_dtype().fill_numpy_dtype.as_deref(),
        Some("int64")
    );
    let sparse = series.sparse().unwrap();
    assert_eq!(sparse.fill_value(), &Scalar::Int64(2));
    assert_eq!(sparse.sp_index(), vec![1, 3]);
    assert_eq!(
        sparse.sp_values(),
        vec![Scalar::Int64(2), Scalar::Int64(25)]
    );
    let dense = sparse.to_dense().unwrap();
    assert_eq!(dense.column().width(), Some(NumericWidth::Int16));
    assert_eq!(
        dense.values(),
        &[
            Scalar::Int64(2),
            Scalar::Int64(2),
            Scalar::Int64(2),
            Scalar::Int64(25)
        ]
    );
}

#[test]
fn empty_sparse_series_has_a_dtype_and_nan_density() {
    let series =
        Series::from_sparse_dense("values", vec![], vec![], DType::Bool, Scalar::Bool(false))
            .unwrap();
    let frame = DataFrame::from_series(vec![series]).unwrap();
    let extracted = frame.to_series_dict().remove("values").unwrap();
    assert_eq!(extracted.dtype_name(), "Sparse[bool, False]");
    let sparse = extracted.sparse().unwrap();
    assert_eq!(sparse.npoints(), 0);
    // The Rust f64 accessor expresses 0 / 0 as NaN. The Python binding
    // reports pandas' ZeroDivisionError for this empty-input case.
    assert!(sparse.density().is_nan());
    assert_eq!(sparse.to_dense().unwrap().dtype(), DType::Bool);
}

#[test]
fn sparse_dense_conversion_reports_an_incompatible_fill_without_panicking() {
    let column = Column::from_sparse(
        SparseColumn::from_parts(
            SparseDType {
                value_dtype: DType::Int64,
                fill_value: Scalar::Utf8("missing".into()),
                width: None,
                fill_numpy_dtype: None,
            },
            2,
            vec![1],
            vec![Scalar::Int64(5)],
        )
        .unwrap(),
    );
    let series = Series::new(
        "values",
        Index::new_known_unique_int64_unit_range(0, 2),
        column,
    )
    .unwrap();
    assert!(series.sparse().unwrap().to_dense().is_err());
}
