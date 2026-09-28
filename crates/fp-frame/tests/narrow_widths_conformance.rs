//! Narrow numpy dtypes through fp-frame (br-frankenpandas-rc0923-epic-python-
//! honest-dropin-fvsao.23): the width a cast, a `.dt` field, `cat.codes` or
//! `downcast_numeric` sets; the operations that keep it (structure, where,
//! shift, clip, concat, transpose, melt) and the ones that end it (a missing
//! row in an integer width, a value past its range, a 64-bit cast). Every
//! expected value is live pandas 2.2.3.

use fp_columnar::Column;
use fp_frame::{DataFrame, Series, concat_series, downcast_numeric, to_datetime};
use fp_index::{Index, IndexLabel};
use fp_types::{DType, NumericWidth, Scalar};

fn ints(name: &str, values: &[i64]) -> Series {
    Series::new(
        name,
        Index::default_range(values.len()),
        Column::from_i64_values(values.to_vec()),
    )
    .unwrap()
}

fn floats(name: &str, values: &[f64]) -> Series {
    Series::new(
        name,
        Index::default_range(values.len()),
        Column::from_f64_values(values.to_vec()),
    )
    .unwrap()
}

fn i64s(series: &Series) -> Vec<i64> {
    series
        .values()
        .iter()
        .map(|value| match value {
            Scalar::Int64(v) => *v,
            _ => i64::MIN,
        })
        .collect()
}

fn width(series: &Series) -> Option<NumericWidth> {
    series.column().width()
}

#[test]
fn astype_width_wraps_rounds_and_a_64_bit_cast_ends_it() {
    // pd.Series([1, 2, 300, -5]).astype('int8') -> [1, 2, 44, -5]
    let int8 = ints("v", &[1, 2, 300, -5])
        .astype_width(NumericWidth::Int8, false)
        .unwrap();
    assert_eq!(i64s(&int8), [1, 2, 44, -5]);
    assert_eq!(width(&int8), Some(NumericWidth::Int8));
    // .astype('uint8') -> [1, 2, 44, 251]
    let uint8 = ints("v", &[1, 2, 300, -5])
        .astype_width(NumericWidth::UInt8, false)
        .unwrap();
    assert_eq!(i64s(&uint8), [1, 2, 44, 251]);
    // astype('float32') of 0.1 is 0.10000000149011612
    let single = floats("v", &[0.1])
        .astype_width(NumericWidth::Float32, false)
        .unwrap();
    assert_eq!(
        single.values()[0],
        Scalar::Float64(0.100_000_001_490_116_12)
    );
    // NEGATIVE: astype('int64') of int32 is int64 - the same-dtype cast
    // must not keep the width.
    let back = int8.astype(DType::Int64).unwrap();
    assert_eq!(width(&back), None);
    assert_eq!(i64s(&back), [1, 2, 44, -5]);
}

#[test]
fn dt_fields_are_int32_unless_a_nat_makes_them_float64() {
    let stamps = Series::from_values(
        "t",
        vec![IndexLabel::Int64(0), IndexLabel::Int64(1)],
        vec![
            Scalar::Utf8("2024-03-01".into()),
            Scalar::Utf8("2024-12-31".into()),
        ],
    )
    .unwrap();
    let stamps = to_datetime(&stamps).unwrap();
    let dt = stamps.dt();
    for field in [
        dt.year().unwrap(),
        dt.month().unwrap(),
        dt.day().unwrap(),
        dt.dayofweek().unwrap(),
        dt.dayofyear().unwrap(),
        dt.quarter().unwrap(),
    ] {
        assert_eq!(width(&field), Some(NumericWidth::Int32));
    }
    assert_eq!(i64s(&dt.year().unwrap()), [2024, 2024]);
    // NEGATIVE: a NaT row keeps the field float64 (pandas' 2024.0, NaN).
    let gapped = Series::from_values(
        "t",
        vec![IndexLabel::Int64(0), IndexLabel::Int64(1)],
        vec![
            Scalar::Utf8("2024-03-01".into()),
            Scalar::Null(fp_types::NullKind::Null),
        ],
    )
    .unwrap();
    let year = to_datetime(&gapped).unwrap().dt().year().unwrap();
    assert_eq!(width(&year), None);
    assert_eq!(year.column().dtype(), DType::Float64);
}

#[test]
fn category_codes_take_the_smallest_width_over_the_count() {
    let few = Series::from_categorical(
        "c",
        vec![Scalar::Utf8("a".into()), Scalar::Utf8("b".into())],
        false,
    )
    .unwrap();
    assert_eq!(
        width(&few.cat().unwrap().codes().unwrap()),
        Some(NumericWidth::Int8)
    );
    // 127 categories are int16 (codes run -1 .. 126, and int8 ends at 127).
    let many = Series::from_categorical(
        "c",
        (0..127).map(|k| Scalar::Utf8(k.to_string())).collect(),
        false,
    )
    .unwrap();
    assert_eq!(
        width(&many.cat().unwrap().codes().unwrap()),
        Some(NumericWidth::Int16)
    );
}

#[test]
fn downcast_numeric_follows_pandas_candidates() {
    let cases = [
        (ints("v", &[1, 2]), "integer", Some(NumericWidth::Int8)),
        (ints("v", &[1, 300]), "signed", Some(NumericWidth::Int16)),
        (ints("v", &[1, 300]), "unsigned", Some(NumericWidth::UInt16)),
        (ints("v", &[-1, 3]), "unsigned", None),
        (
            floats("v", &[1.5, 2.5]),
            "float",
            Some(NumericWidth::Float32),
        ),
        // whole floats downcast to an integer width ...
        (
            floats("v", &[1.0, 2.0]),
            "integer",
            Some(NumericWidth::Int8),
        ),
        // ... a fraction or a NaN does not
        (floats("v", &[1.5]), "integer", None),
        (floats("v", &[1.0, f64::NAN]), "integer", None),
        // float32 keeps a value only within 5e-4
        (floats("v", &[1e10 + 0.5]), "float", None),
    ];
    for (series, downcast, expected) in cases {
        let narrowed = downcast_numeric(&series, downcast).unwrap();
        assert_eq!(
            width(&narrowed),
            expected,
            "{downcast} {:?}",
            series.values()
        );
    }
    assert!(downcast_numeric(&ints("v", &[1]), "bogus").is_err());
}

#[test]
fn structure_and_value_operations_keep_or_end_the_width() {
    let int32 = ints("v", &[3, 1, 2, 1])
        .astype_width(NumericWidth::Int32, false)
        .unwrap();
    for kept in [
        int32.head(2).unwrap(),
        int32.sort_values(true).unwrap(),
        int32.drop_duplicates().unwrap(),
        int32.nlargest(2).unwrap(),
        int32.cummax().unwrap(),
        int32.clip(Some(2.0), Some(4.0)).unwrap(),
        int32.abs().unwrap(),
    ] {
        assert_eq!(
            width(&kept),
            Some(NumericWidth::Int32),
            "{:?}",
            kept.values()
        );
    }
    // An int32 shift gaps into float64; a float32 one stays float32.
    assert_eq!(width(&int32.shift(1).unwrap()), None);
    let single = floats("f", &[1.0, 2.0])
        .astype_width(NumericWidth::Float32, false)
        .unwrap();
    assert_eq!(
        width(&single.shift(1).unwrap()),
        Some(NumericWidth::Float32)
    );
    // NEGATIVE: a clip bound past int8 leaves int8 for int64 ...
    let int8 = ints("v", &[1, 5])
        .astype_width(NumericWidth::Int8, false)
        .unwrap();
    let clipped = int8.clip(Some(200.0), None).unwrap();
    assert_eq!(width(&clipped), None);
    assert_eq!(i64s(&clipped), [200, 200]);
    // ... while abs wraps as numpy's: abs of int8 -128 is -128.
    let low = ints("v", &[-128, 5])
        .astype_width(NumericWidth::Int8, false)
        .unwrap();
    assert_eq!(i64s(&low.abs().unwrap()), [-128, 5]);
}

#[test]
fn float32_cumsum_accumulates_in_float32() {
    let values: Vec<f64> = (1..=40).map(|k| f64::from(k) * 0.1).collect();
    let single = floats("f", &values)
        .astype_width(NumericWidth::Float32, false)
        .unwrap();
    let summed = single.cumsum().unwrap();
    assert_eq!(width(&summed), Some(NumericWidth::Float32));
    let mut expected = 0.0_f32;
    for (value, got) in values.iter().zip(summed.values()) {
        #[allow(clippy::cast_possible_truncation)]
        let term = *value as f32;
        expected += term;
        assert_eq!(*got, Scalar::Float64(f64::from(expected)));
    }
    // An unsigned column sums to uint64, a signed one to int64.
    let uint8 = ints("u", &[200, 100])
        .astype_width(NumericWidth::UInt8, false)
        .unwrap();
    assert_eq!(width(&uint8.cumsum().unwrap()), Some(NumericWidth::UInt64));
    let int8 = ints("i", &[100, 100])
        .astype_width(NumericWidth::Int8, false)
        .unwrap();
    assert_eq!(width(&int8.cumsum().unwrap()), None);
}

#[test]
fn concat_transpose_melt_and_select_dtypes_see_the_widths() {
    let int8 = ints("v", &[1])
        .astype_width(NumericWidth::Int8, false)
        .unwrap();
    let int16 = ints("v", &[2])
        .astype_width(NumericWidth::Int16, false)
        .unwrap();
    // pd.concat of int8 and int16 is int16; with int64 it is int64.
    assert_eq!(
        width(&concat_series(&[&int8, &int16]).unwrap()),
        Some(NumericWidth::Int16)
    );
    assert_eq!(
        width(&concat_series(&[&int8, &ints("v", &[3])]).unwrap()),
        None
    );

    let frame = DataFrame::from_series(vec![
        ints("k", &[1, 2]),
        ints("i", &[3, 4])
            .astype_width(NumericWidth::Int32, false)
            .unwrap(),
        floats("f", &[0.5, 1.5])
            .astype_width(NumericWidth::Float32, false)
            .unwrap(),
    ])
    .unwrap();
    let selected = |name: &str| {
        frame
            .select_dtypes_by_name(&[name], &[])
            .unwrap()
            .column_names()
            .into_iter()
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(selected("int32"), ["i"]);
    // NEGATIVE: 'int64' is the 64-bit column alone; 'integer' both.
    assert_eq!(selected("int64"), ["k"]);
    assert_eq!(selected("integer"), ["k", "i"]);
    assert_eq!(selected("float32"), ["f"]);

    let doubled = fp_frame::concat_dataframes(&[&frame, &frame]).unwrap();
    assert_eq!(
        doubled.column("i").unwrap().width(),
        Some(NumericWidth::Int32)
    );
    assert_eq!(
        doubled.column("f").unwrap().width(),
        Some(NumericWidth::Float32)
    );

    let only_i = frame.select_columns(&["i"]).unwrap();
    let transposed = only_i.transpose().unwrap();
    assert!(
        transposed
            .column_names()
            .iter()
            .all(|name| transposed.column(name).unwrap().width() == Some(NumericWidth::Int32))
    );
    let melted = frame.melt(&["k"], &["i"], None, None).unwrap();
    assert_eq!(
        melted.column("value").unwrap().width(),
        Some(NumericWidth::Int32)
    );
    assert_eq!(melted.column("k").unwrap().width(), None);
}

#[test]
fn memory_usage_counts_itemsize_and_the_mask() {
    // int8 is a byte a value; a masked column adds its mask byte
    // (pd.Series([1, None], dtype='Int64').nbytes is 18, 'Int32' 10).
    let int8 = ints("v", &[1, 2, 3])
        .astype_width(NumericWidth::Int8, false)
        .unwrap();
    assert_eq!(int8.nbytes(), 3);
    let masked = Series::new(
        "v",
        Index::default_range(2),
        Column::new(
            DType::Int64Nullable,
            vec![Scalar::Int64(1), Scalar::Null(fp_types::NullKind::Null)],
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(masked.nbytes(), 18);
    assert_eq!(
        masked
            .astype_width(NumericWidth::Int32, true)
            .unwrap()
            .nbytes(),
        10
    );
}
