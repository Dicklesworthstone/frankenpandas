//! pandas' `string` extension dtype through fp-frame
//! (br-frankenpandas-rc0923-epic-python-honest-dropin-fvsao.59): made by
//! `convert_dtypes`, kept by structure (concat of `string` Series, melt,
//! where / replace while the result is text), ended by a cast or a number.
//! Expected dtypes are live pandas 2.2.3.

use fp_columnar::Column;
use fp_frame::{DataFrame, Series, concat_series};
use fp_index::{Index, IndexLabel};
use fp_types::{DType, NullKind, Scalar};

fn texts(name: &str, values: &[Option<&str>]) -> Series {
    Series::from_values(
        name,
        (0..values.len() as i64).map(IndexLabel::Int64).collect(),
        values
            .iter()
            .map(|value| {
                value.map_or(Scalar::Null(NullKind::Null), |text| {
                    Scalar::Utf8(text.to_owned())
                })
            })
            .collect(),
    )
    .unwrap()
}

fn strings(name: &str, values: &[Option<&str>]) -> Series {
    let series = texts(name, values);
    Series::new(
        name,
        series.index().clone(),
        series.column().clone().as_pandas_string(),
    )
    .unwrap()
}

#[test]
fn convert_dtypes_makes_text_the_string_dtype() {
    // pd.Series(['a', None]).convert_dtypes().dtype -> string, [a, <NA>]
    let converted = texts("t", &[Some("a"), None]).convert_dtypes().unwrap();
    assert!(converted.column().is_pandas_string());
    assert_eq!(converted.values()[1], Scalar::Null(NullKind::Null));
    // NEGATIVE: numbers are not text (Int64), and text holding a number
    // (an object mix) is not the string dtype.
    let ints = Series::new(
        "n",
        Index::default_range(2),
        Column::from_i64_values(vec![1, 2]),
    )
    .unwrap();
    assert!(!ints.convert_dtypes().unwrap().column().is_pandas_string());
    let mixed = Series::from_values(
        "m",
        vec![IndexLabel::Int64(0), IndexLabel::Int64(1)],
        vec![Scalar::Utf8("a".to_owned()), Scalar::Int64(1)],
    )
    .unwrap();
    assert!(!mixed.convert_dtypes().unwrap().column().is_pandas_string());
}

#[test]
fn concat_melt_and_text_results_keep_it() {
    let left = strings("t", &[Some("ab"), None]);
    let right = strings("t", &[Some("c")]);
    assert!(
        concat_series(&[&left, &right])
            .unwrap()
            .column()
            .is_pandas_string()
    );
    // NEGATIVE: with an object piece the result is object.
    let object = texts("t", &[Some("q")]);
    assert!(
        !concat_series(&[&left, &object])
            .unwrap()
            .column()
            .is_pandas_string()
    );
    // where / replace keep it while the values stay text.
    let cond = Series::from_values(
        "c",
        vec![IndexLabel::Int64(0), IndexLabel::Int64(1)],
        vec![Scalar::Bool(false), Scalar::Bool(true)],
    )
    .unwrap();
    let filled = left
        .where_cond(&cond, Some(&Scalar::Utf8("zz".to_owned())))
        .unwrap();
    assert!(filled.column().is_pandas_string());
    let replaced = left
        .replace(&[(Scalar::Utf8("ab".to_owned()), Scalar::Utf8("b".to_owned()))])
        .unwrap();
    assert!(replaced.column().is_pandas_string());
    // A reindex onto a new label keeps it (the gap is pd.NA).
    assert!(
        left.reindex(vec![IndexLabel::Int64(0), IndexLabel::Int64(9)])
            .unwrap()
            .column()
            .is_pandas_string()
    );

    let frame = DataFrame::from_series(vec![
        Series::new(
            "k",
            Index::default_range(2),
            Column::from_i64_values(vec![1, 2]),
        )
        .unwrap(),
        strings("t", &[Some("ab"), None]),
    ])
    .unwrap();
    let melted = frame.melt(&["k"], &["t"], None, None).unwrap();
    assert!(melted.column("value").unwrap().is_pandas_string());
    assert_eq!(melted.column("value").unwrap().dtype(), DType::Utf8);
}
