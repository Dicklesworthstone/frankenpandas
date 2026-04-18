#![forbid(unsafe_code)]

//! Property-based testing infrastructure for FrankenPandas (bd-2t5e.1, AG-01).
//!
//! Strategy generators produce arbitrary but pandas-valid inputs across the
//! (dtype x null_pattern x index_type x operation) combinatorial space.
//! Properties verify behavioral invariants that must hold for ALL inputs,
//! not just hand-picked fixtures.

use proptest::prelude::*;

use fp_frame::{DataFrame, DropNaHow, FrameError, Series};
use fp_groupby::{
    GroupByExecutionOptions, GroupByOptions, groupby_count, groupby_mean, groupby_sum,
    groupby_sum_with_options,
};
use fp_index::{DuplicateKeep, Index, IndexLabel, align_union, validate_alignment_plan};
use fp_join::{
    JoinExecutionOptions, JoinType, JoinedSeries, join_series, join_series_with_options,
};
use fp_runtime::{EvidenceLedger, RuntimePolicy};
use fp_types::{NullKind, Scalar, Timedelta};

// ---------------------------------------------------------------------------
// Strategy generators
// ---------------------------------------------------------------------------

/// Generate an arbitrary numeric Scalar suitable for arithmetic operations.
fn arb_numeric_scalar() -> impl Strategy<Value = Scalar> {
    prop_oneof![
        3 => (-1_000_000i64..1_000_000i64).prop_map(Scalar::Int64),
        3 => (-1e6_f64..1e6_f64).prop_map(Scalar::Float64),
        1 => Just(Scalar::Null(NullKind::Null)),
        1 => Just(Scalar::Null(NullKind::NaN)),
    ]
}

fn arb_replace_numeric_scalar() -> impl Strategy<Value = Scalar> {
    prop_oneof![
        2 => (-1_000_000i64..1_000_000i64).prop_map(Scalar::Int64),
        3 => (-1e6_f64..1e6_f64).prop_map(Scalar::Float64),
        1 => Just(Scalar::Float64(f64::NAN)),
        1 => Just(Scalar::Null(NullKind::Null)),
        1 => Just(Scalar::Null(NullKind::NaN)),
    ]
}

/// Generate an arbitrary IndexLabel.
fn arb_index_label() -> impl Strategy<Value = IndexLabel> {
    prop_oneof![
        3 => (0i64..100).prop_map(IndexLabel::Int64),
        1 => "[a-e]{1,3}".prop_map(IndexLabel::Utf8),
    ]
}

/// Generate a Vec of IndexLabels with `len` entries, allowing some duplicates.
fn arb_index_labels(len: usize) -> impl Strategy<Value = Vec<IndexLabel>> {
    proptest::collection::vec(arb_index_label(), len)
}

/// Generate a Vec of unique IndexLabels with `len` entries.
fn arb_unique_index_labels(len: usize) -> impl Strategy<Value = Vec<IndexLabel>> {
    proptest::collection::btree_set(arb_index_label(), len)
        .prop_map(|labels| labels.into_iter().collect())
}

/// Generate an Index with `len` labels, allowing some duplicates.
fn arb_index(len: usize) -> impl Strategy<Value = Index> {
    arb_index_labels(len).prop_map(Index::new)
}

/// Generate a Vec of numeric Scalars of given length.
fn arb_numeric_values(len: usize) -> impl Strategy<Value = Vec<Scalar>> {
    proptest::collection::vec(arb_numeric_scalar(), len)
}

fn arb_replace_numeric_values(len: usize) -> impl Strategy<Value = Vec<Scalar>> {
    proptest::collection::vec(arb_replace_numeric_scalar(), len)
}

fn arb_condition_scalar() -> impl Strategy<Value = Scalar> {
    prop_oneof![
        4 => proptest::bool::ANY.prop_map(Scalar::Bool),
        1 => Just(Scalar::Null(NullKind::Null)),
        1 => Just(Scalar::Null(NullKind::NaN)),
    ]
}

fn arb_condition_values(len: usize) -> impl Strategy<Value = Vec<Scalar>> {
    proptest::collection::vec(arb_condition_scalar(), len)
}

fn arb_boolean_condition_scalar() -> impl Strategy<Value = Scalar> {
    proptest::bool::ANY.prop_map(Scalar::Bool)
}

fn arb_boolean_condition_values(len: usize) -> impl Strategy<Value = Vec<Scalar>> {
    proptest::collection::vec(arb_boolean_condition_scalar(), len)
}

/// Generate an arbitrary Series with numeric values and the given length.
fn arb_numeric_series(name: &'static str, len: usize) -> impl Strategy<Value = Series> {
    (arb_index_labels(len), arb_numeric_values(len)).prop_filter_map(
        "series construction must succeed",
        move |(labels, values)| Series::from_values(name.to_owned(), labels, values).ok(),
    )
}

/// Generate an arbitrary Series with numeric values and unique index labels.
fn arb_unique_numeric_series(name: &'static str, len: usize) -> impl Strategy<Value = Series> {
    (arb_unique_index_labels(len), arb_numeric_values(len)).prop_filter_map(
        "series construction must succeed",
        move |(labels, values)| Series::from_values(name.to_owned(), labels, values).ok(),
    )
}

fn arb_variable_unique_numeric_series(
    name: &'static str,
    max_len: usize,
) -> impl Strategy<Value = Series> {
    (1..=max_len).prop_flat_map(move |len| arb_unique_numeric_series(name, len))
}

/// Generate a pair of numeric series with independently chosen lengths (1..max_len).
fn arb_series_pair(max_len: usize) -> impl Strategy<Value = (Series, Series)> {
    (1..=max_len, 1..=max_len).prop_flat_map(|(len_a, len_b)| {
        (
            arb_numeric_series("left", len_a),
            arb_numeric_series("right", len_b),
        )
    })
}

/// Generate a pair of numeric series with unique index labels.
fn arb_unique_series_pair(max_len: usize) -> impl Strategy<Value = (Series, Series)> {
    (1..=max_len, 1..=max_len).prop_flat_map(|(len_a, len_b)| {
        (
            arb_unique_numeric_series("left", len_a),
            arb_unique_numeric_series("right", len_b),
        )
    })
}

/// Generate a pair of indices with independently chosen lengths.
fn arb_index_pair(max_len: usize) -> impl Strategy<Value = (Index, Index)> {
    (1..=max_len, 1..=max_len).prop_flat_map(|(len_a, len_b)| (arb_index(len_a), arb_index(len_b)))
}

/// Generate a pair of series suitable for groupby (keys + values, same length).
fn arb_groupby_pair(max_len: usize) -> impl Strategy<Value = (Series, Series)> {
    (1..=max_len).prop_flat_map(|len| {
        // Keys: use a small label space so groupby actually groups things.
        let key_labels = arb_index_labels(len);
        let key_values = proptest::collection::vec(
            prop_oneof![
                3 => (0i64..10).prop_map(Scalar::Int64),
                1 => Just(Scalar::Null(NullKind::Null)),
            ],
            len,
        );
        let val_labels = arb_index_labels(len);
        let val_values = arb_numeric_values(len);

        (key_labels, key_values, val_labels, val_values).prop_filter_map(
            "groupby series must construct",
            |(kl, kv, vl, vv)| {
                let keys = Series::from_values("keys".to_owned(), kl, kv).ok()?;
                let vals = Series::from_values("values".to_owned(), vl, vv).ok()?;
                Some((keys, vals))
            },
        )
    })
}

/// Generate a small DataFrame with unique index labels and a variable column subset.
fn arb_combine_first_dataframe(max_rows: usize) -> impl Strategy<Value = DataFrame> {
    (1..=max_rows).prop_flat_map(|nrows| {
        (
            arb_unique_index_labels(nrows),
            proptest::sample::subsequence(
                vec!["a".to_string(), "b".to_string(), "c".to_string()],
                1..=3,
            ),
        )
            .prop_flat_map(move |(labels, column_names)| {
                let ncols = column_names.len();
                (
                    Just(labels),
                    Just(column_names),
                    proptest::collection::vec(arb_numeric_values(nrows), ncols),
                )
            })
            .prop_filter_map(
                "dataframe construction must succeed",
                |(labels, column_names, column_values)| {
                    let index = Index::new(labels);
                    let mut columns = std::collections::BTreeMap::new();
                    for (name, values) in column_names.iter().cloned().zip(column_values) {
                        columns.insert(name, fp_columnar::Column::from_values(values).ok()?);
                    }
                    DataFrame::new_with_column_order(index, columns, column_names).ok()
                },
            )
    })
}

fn poison_numeric_scalar(value: &Scalar) -> Scalar {
    match value {
        Scalar::Int64(v) => Scalar::Int64(v.saturating_add(1)),
        Scalar::Float64(v) if v.is_finite() => Scalar::Float64(v + 1.0),
        Scalar::Float64(_) => Scalar::Float64(0.0),
        Scalar::Null(_) => Scalar::Int64(1),
        Scalar::Bool(v) => Scalar::Bool(!v),
        Scalar::Utf8(v) => Scalar::Utf8(format!("{v}_poisoned")),
        Scalar::Timedelta64(v) if *v != Timedelta::NAT => {
            Scalar::Timedelta64(v.saturating_add(Timedelta::NANOS_PER_SEC))
        }
        Scalar::Timedelta64(_) => Scalar::Timedelta64(Timedelta::NANOS_PER_HOUR),
    }
}

fn sign_flip_numeric_scalar(value: &Scalar) -> Scalar {
    match value {
        Scalar::Int64(v) => Scalar::Int64(v.saturating_neg()),
        Scalar::Float64(v) => Scalar::Float64(-v),
        Scalar::Null(kind) => Scalar::Null(*kind),
        other => other.clone(),
    }
}

fn sign_flip_series(series: &Series) -> Series {
    let flipped_values = series
        .values()
        .iter()
        .map(sign_flip_numeric_scalar)
        .collect::<Vec<_>>();
    Series::from_values(
        series.name().to_owned(),
        series.index().labels().to_vec(),
        flipped_values,
    )
    .expect("sign-flipped series must construct")
}

fn sign_flip_dataframe(df: &DataFrame) -> DataFrame {
    let mut flipped_columns = std::collections::BTreeMap::new();
    let column_order = df
        .column_names()
        .into_iter()
        .map(|name| {
            let values = df
                .column(name)
                .expect("column listed in order must exist")
                .values()
                .iter()
                .map(sign_flip_numeric_scalar)
                .collect::<Vec<_>>();
            flipped_columns.insert(
                name.clone(),
                fp_columnar::Column::from_values(values)
                    .expect("sign-flipped dataframe column must construct"),
            );
            name.clone()
        })
        .collect::<Vec<_>>();
    DataFrame::new_with_column_order(df.index().clone(), flipped_columns, column_order)
        .expect("sign-flipped dataframe must construct")
}

fn reverse_series(series: &Series) -> Series {
    Series::from_values(
        series.name().to_owned(),
        series.index().labels().iter().cloned().rev().collect(),
        series.values().iter().cloned().rev().collect(),
    )
    .expect("reversed series must construct")
}

fn reverse_dataframe_rows(df: &DataFrame) -> DataFrame {
    let column_order = df.column_names().into_iter().cloned().collect::<Vec<_>>();
    let mut columns = std::collections::BTreeMap::new();
    for name in &column_order {
        let values = df
            .column(name)
            .expect("column listed in order must exist")
            .values()
            .iter()
            .cloned()
            .rev()
            .collect::<Vec<_>>();
        columns.insert(
            name.clone(),
            fp_columnar::Column::from_values(values)
                .expect("reversed dataframe column must construct"),
        );
    }
    DataFrame::new_with_column_order(
        Index::new(df.index().labels().iter().cloned().rev().collect()),
        columns,
        column_order,
    )
    .expect("reversed dataframe must construct")
}

fn fresh_missing_index_label(existing: &[IndexLabel], salt: usize) -> IndexLabel {
    let mut nonce = salt;
    loop {
        let candidate = IndexLabel::Utf8(format!("__missing_{nonce}"));
        if !existing.contains(&candidate) {
            return candidate;
        }
        nonce += 1;
    }
}

fn reindex_intermediate_labels(labels: &[IndexLabel]) -> Vec<IndexLabel> {
    let missing_a = fresh_missing_index_label(labels, 0);
    let mut seen = labels.to_vec();
    seen.push(missing_a.clone());
    let missing_b = fresh_missing_index_label(&seen, 1);

    let mut intermediate = labels.to_vec();
    intermediate.push(missing_a);
    intermediate.push(missing_b);
    intermediate
}

fn reindex_target_labels(labels: &[IndexLabel]) -> Vec<IndexLabel> {
    let intermediate = reindex_intermediate_labels(labels);
    let missing_a = intermediate[labels.len()].clone();
    let missing_b = intermediate[labels.len() + 1].clone();

    let mut target = labels.iter().cloned().rev().collect::<Vec<_>>();
    target.push(missing_b);
    target.push(missing_a);
    if let Some(first) = labels.first() {
        target.push(first.clone());
    }
    target
}

fn fresh_missing_column_name(existing: &[String], salt: usize) -> String {
    let mut nonce = salt;
    loop {
        let candidate = format!("__missing_col_{nonce}");
        if !existing.iter().any(|name| name == &candidate) {
            return candidate;
        }
        nonce += 1;
    }
}

fn reindex_intermediate_columns(columns: &[String]) -> Vec<String> {
    let missing_a = fresh_missing_column_name(columns, 0);
    let mut seen = columns.to_vec();
    seen.push(missing_a.clone());
    let missing_b = fresh_missing_column_name(&seen, 1);

    let mut intermediate = columns.to_vec();
    intermediate.push(missing_a);
    intermediate.push(missing_b);
    intermediate
}

fn reindex_target_columns(columns: &[String]) -> Vec<String> {
    let intermediate = reindex_intermediate_columns(columns);
    let missing_a = intermediate[columns.len()].clone();
    let missing_b = intermediate[columns.len() + 1].clone();

    let mut target = columns.iter().cloned().rev().collect::<Vec<_>>();
    target.push(missing_b);
    target.push(missing_a);
    target
}

fn reindex_axis1_from_names(df: &DataFrame, columns: &[String]) -> DataFrame {
    df.reindex_axis(
        columns
            .iter()
            .cloned()
            .map(IndexLabel::Utf8)
            .collect::<Vec<_>>(),
        1,
    )
    .expect("DataFrame::reindex_axis(axis=1) must succeed")
}

fn reindex_columns_from_names(df: &DataFrame, columns: &[String]) -> DataFrame {
    let refs = columns.iter().map(String::as_str).collect::<Vec<_>>();
    df.reindex_columns(&refs)
        .expect("DataFrame::reindex_columns() must succeed")
}

fn fresh_rename_column_names(columns: &[String]) -> Vec<String> {
    let mut seen = columns.to_vec();
    (0..columns.len())
        .map(|salt| {
            let candidate = fresh_missing_column_name(&seen, salt);
            seen.push(candidate.clone());
            candidate
        })
        .collect()
}

fn rename_columns_from_names(df: &DataFrame, target_names: &[String]) -> DataFrame {
    let current_names = df.column_names().into_iter().cloned().collect::<Vec<_>>();
    let pairs = current_names
        .iter()
        .map(String::as_str)
        .zip(target_names.iter().map(String::as_str))
        .collect::<Vec<_>>();
    df.rename_columns(&pairs)
        .expect("DataFrame::rename_columns() must succeed for fresh unique names")
}

fn fresh_rename_index_labels(labels: &[IndexLabel]) -> Vec<IndexLabel> {
    let mut seen = labels.to_vec();
    (0..labels.len())
        .map(|salt| {
            let candidate = fresh_missing_index_label(&seen, salt);
            seen.push(candidate.clone());
            candidate
        })
        .collect()
}

fn rename_index_from_labels(df: &DataFrame, target_labels: &[IndexLabel]) -> DataFrame {
    let pairs = df
        .index()
        .labels()
        .iter()
        .cloned()
        .zip(target_labels.iter().cloned())
        .collect::<Vec<_>>();
    df.rename_index(&pairs)
}

fn fresh_shared_rename_index_labels(
    left: &[IndexLabel],
    right: &[IndexLabel],
) -> (Vec<IndexLabel>, Vec<IndexLabel>) {
    let mut existing = left.iter().chain(right.iter()).cloned().collect::<Vec<_>>();
    let distinct_labels = left
        .iter()
        .chain(right.iter())
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let mut mapping = std::collections::BTreeMap::new();

    for (salt, label) in distinct_labels.into_iter().enumerate() {
        let candidate = fresh_missing_index_label(&existing, salt);
        existing.push(candidate.clone());
        mapping.insert(label, candidate);
    }

    let remap = |labels: &[IndexLabel]| {
        labels
            .iter()
            .map(|label| {
                mapping
                    .get(label)
                    .expect("every original label must have a fresh rename")
                    .clone()
            })
            .collect::<Vec<_>>()
    };

    (remap(left), remap(right))
}

fn relabel_groupby_pair_indices(keys: &Series, values: &Series) -> (Series, Series) {
    let (renamed_key_labels, renamed_value_labels) =
        fresh_shared_rename_index_labels(keys.index().labels(), values.index().labels());
    let renamed_keys = keys
        .set_axis(renamed_key_labels)
        .expect("keys relabel must preserve shape");
    let renamed_values = values
        .set_axis(renamed_value_labels)
        .expect("values relabel must preserve shape");
    (renamed_keys, renamed_values)
}

fn reconstruct_groupby_mean_from_sum_and_count(sum: &Series, count: &Series) -> Series {
    let reconstructed = sum
        .values()
        .iter()
        .zip(count.values())
        .map(|(sum_value, count_value)| match count_value.to_f64() {
            Ok(count) if count > 0.0 => Scalar::Float64(
                sum_value
                    .to_f64()
                    .expect("non-empty group sum must be numeric")
                    / count,
            ),
            _ => Scalar::Null(NullKind::Null),
        })
        .collect::<Vec<_>>();

    Series::from_values(
        "mean".to_owned(),
        sum.index().labels().to_vec(),
        reconstructed,
    )
    .expect("reconstructed groupby mean must construct")
}

fn arb_sorted_int_index_labels(len: usize) -> impl Strategy<Value = Vec<IndexLabel>> {
    proptest::collection::btree_set(-100i64..100, len).prop_map(|labels| {
        labels
            .into_iter()
            .map(IndexLabel::Int64)
            .collect::<Vec<_>>()
    })
}

fn merged_truncate_before(
    lhs: &Option<IndexLabel>,
    rhs: &Option<IndexLabel>,
) -> Option<IndexLabel> {
    match (lhs, rhs) {
        (Some(left), Some(right)) => Some(if left >= right {
            left.clone()
        } else {
            right.clone()
        }),
        (Some(left), None) => Some(left.clone()),
        (None, Some(right)) => Some(right.clone()),
        (None, None) => None,
    }
}

fn merged_truncate_after(lhs: &Option<IndexLabel>, rhs: &Option<IndexLabel>) -> Option<IndexLabel> {
    match (lhs, rhs) {
        (Some(left), Some(right)) => Some(if left <= right {
            left.clone()
        } else {
            right.clone()
        }),
        (Some(left), None) => Some(left.clone()),
        (None, Some(right)) => Some(right.clone()),
        (None, None) => None,
    }
}

fn arb_truncate_bounds(
    min_label: i64,
    max_label: i64,
) -> impl Strategy<Value = (Option<IndexLabel>, Option<IndexLabel>)> {
    let low = min_label.saturating_sub(2);
    let high = max_label.saturating_add(2);
    (
        proptest::option::of(low..=high),
        proptest::option::of(low..=high),
    )
        .prop_map(|(before, after)| (before.map(IndexLabel::Int64), after.map(IndexLabel::Int64)))
}

fn arb_sorted_int_series(name: &'static str, len: usize) -> impl Strategy<Value = Series> {
    (arb_sorted_int_index_labels(len), arb_numeric_values(len)).prop_filter_map(
        "sorted int-index series construction must succeed",
        move |(labels, values)| Series::from_values(name.to_owned(), labels, values).ok(),
    )
}

fn arb_sorted_int_dataframe(max_rows: usize) -> impl Strategy<Value = DataFrame> {
    (1..=max_rows).prop_flat_map(|nrows| {
        let idx_labels = arb_sorted_int_index_labels(nrows);
        let col_a = arb_numeric_values(nrows);
        let col_b = arb_numeric_values(nrows);
        (idx_labels, col_a, col_b).prop_filter_map(
            "sorted int-index dataframe construction must succeed",
            move |(labels, va, vb)| {
                let index = Index::new(labels);
                let col_a = fp_columnar::Column::from_values(va).ok()?;
                let col_b = fp_columnar::Column::from_values(vb).ok()?;
                let mut cols = std::collections::BTreeMap::new();
                cols.insert("a".to_string(), col_a);
                cols.insert("b".to_string(), col_b);
                DataFrame::new_with_column_order(
                    index,
                    cols,
                    vec!["a".to_string(), "b".to_string()],
                )
                .ok()
            },
        )
    })
}

fn arb_series_truncate_case(
    name: &'static str,
    max_len: usize,
) -> impl Strategy<Value = (Series, Option<IndexLabel>, Option<IndexLabel>)> {
    (1..=max_len)
        .prop_flat_map(move |len| {
            arb_sorted_int_series(name, len).prop_flat_map(move |series| {
                let labels = series.index().labels();
                let min_label = match labels.first() {
                    Some(IndexLabel::Int64(value)) => *value,
                    _ => unreachable!("sorted truncate labels must be Int64"),
                };
                let max_label = match labels.last() {
                    Some(IndexLabel::Int64(value)) => *value,
                    _ => unreachable!("sorted truncate labels must be Int64"),
                };
                (Just(series), arb_truncate_bounds(min_label, max_label))
            })
        })
        .prop_map(|(series, (before, after))| (series, before, after))
}

fn arb_series_truncate_composition_case(
    name: &'static str,
    max_len: usize,
) -> impl Strategy<
    Value = (
        Series,
        Option<IndexLabel>,
        Option<IndexLabel>,
        Option<IndexLabel>,
        Option<IndexLabel>,
    ),
> {
    (1..=max_len)
        .prop_flat_map(move |len| {
            arb_sorted_int_series(name, len).prop_flat_map(move |series| {
                let labels = series.index().labels();
                let min_label = match labels.first() {
                    Some(IndexLabel::Int64(value)) => *value,
                    _ => unreachable!("sorted truncate labels must be Int64"),
                };
                let max_label = match labels.last() {
                    Some(IndexLabel::Int64(value)) => *value,
                    _ => unreachable!("sorted truncate labels must be Int64"),
                };
                (
                    Just(series),
                    arb_truncate_bounds(min_label, max_label),
                    arb_truncate_bounds(min_label, max_label),
                )
            })
        })
        .prop_map(|(series, (before_a, after_a), (before_b, after_b))| {
            (series, before_a, after_a, before_b, after_b)
        })
}

fn arb_dataframe_truncate_case(
    max_rows: usize,
) -> impl Strategy<Value = (DataFrame, Option<IndexLabel>, Option<IndexLabel>)> {
    arb_sorted_int_dataframe(max_rows)
        .prop_flat_map(|df| {
            let labels = df.index().labels();
            let min_label = match labels.first() {
                Some(IndexLabel::Int64(value)) => *value,
                _ => unreachable!("sorted truncate labels must be Int64"),
            };
            let max_label = match labels.last() {
                Some(IndexLabel::Int64(value)) => *value,
                _ => unreachable!("sorted truncate labels must be Int64"),
            };
            (Just(df), arb_truncate_bounds(min_label, max_label))
        })
        .prop_map(|(df, (before, after))| (df, before, after))
}

fn arb_dataframe_truncate_composition_case(
    max_rows: usize,
) -> impl Strategy<
    Value = (
        DataFrame,
        Option<IndexLabel>,
        Option<IndexLabel>,
        Option<IndexLabel>,
        Option<IndexLabel>,
    ),
> {
    arb_sorted_int_dataframe(max_rows)
        .prop_flat_map(|df| {
            let labels = df.index().labels();
            let min_label = match labels.first() {
                Some(IndexLabel::Int64(value)) => *value,
                _ => unreachable!("sorted truncate labels must be Int64"),
            };
            let max_label = match labels.last() {
                Some(IndexLabel::Int64(value)) => *value,
                _ => unreachable!("sorted truncate labels must be Int64"),
            };
            (
                Just(df),
                arb_truncate_bounds(min_label, max_label),
                arb_truncate_bounds(min_label, max_label),
            )
        })
        .prop_map(|(df, (before_a, after_a), (before_b, after_b))| {
            (df, before_a, after_a, before_b, after_b)
        })
}

fn merged_drop_items<T: Ord + Clone>(lhs: &[T], rhs: &[T]) -> Vec<T> {
    let mut merged = lhs
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    merged.extend(rhs.iter().cloned());
    merged.into_iter().collect()
}

fn arb_unique_utf8_index_strings(len: usize) -> impl Strategy<Value = Vec<String>> {
    proptest::collection::btree_set("[a-z]{1,6}", len)
        .prop_map(|labels| labels.into_iter().collect())
}

fn utf8_index_strings(labels: &[IndexLabel]) -> Vec<String> {
    labels
        .iter()
        .map(|label| match label {
            IndexLabel::Utf8(value) => value.clone(),
            _ => unreachable!("drop row helpers require Utf8 index labels"),
        })
        .collect()
}

fn arb_unique_utf8_numeric_dataframe(max_rows: usize) -> impl Strategy<Value = DataFrame> {
    (1..=max_rows).prop_flat_map(|nrows| {
        let idx_labels = arb_unique_utf8_index_strings(nrows)
            .prop_map(|labels| labels.into_iter().map(IndexLabel::Utf8).collect::<Vec<_>>());
        let col_a = arb_numeric_values(nrows);
        let col_b = arb_numeric_values(nrows);
        let col_c = arb_numeric_values(nrows);
        (idx_labels, col_a, col_b, col_c).prop_filter_map(
            "Utf8-index dataframe construction must succeed",
            move |(labels, va, vb, vc)| {
                let index = Index::new(labels);
                let col_a = fp_columnar::Column::from_values(va).ok()?;
                let col_b = fp_columnar::Column::from_values(vb).ok()?;
                let col_c = fp_columnar::Column::from_values(vc).ok()?;
                let mut cols = std::collections::BTreeMap::new();
                cols.insert("a".to_string(), col_a);
                cols.insert("b".to_string(), col_b);
                cols.insert("c".to_string(), col_c);
                DataFrame::new_with_column_order(
                    index,
                    cols,
                    vec!["a".to_string(), "b".to_string(), "c".to_string()],
                )
                .ok()
            },
        )
    })
}

fn arb_series_drop_case(
    name: &'static str,
    max_len: usize,
) -> impl Strategy<Value = (Series, Vec<IndexLabel>)> {
    (1..=max_len).prop_flat_map(move |len| {
        arb_unique_numeric_series(name, len).prop_flat_map(move |series| {
            let labels = series.index().labels().to_vec();
            (Just(series), proptest::sample::subsequence(labels, 0..=len))
        })
    })
}

fn arb_series_drop_composition_case(
    name: &'static str,
    max_len: usize,
) -> impl Strategy<Value = (Series, Vec<IndexLabel>, Vec<IndexLabel>)> {
    (1..=max_len).prop_flat_map(move |len| {
        arb_unique_numeric_series(name, len).prop_flat_map(move |series| {
            let labels = series.index().labels().to_vec();
            (
                Just(series),
                proptest::sample::subsequence(labels.clone(), 0..=len),
                proptest::sample::subsequence(labels, 0..=len),
            )
        })
    })
}

fn arb_dataframe_row_drop_case(max_rows: usize) -> impl Strategy<Value = (DataFrame, Vec<String>)> {
    arb_unique_utf8_numeric_dataframe(max_rows).prop_flat_map(|df| {
        let labels = utf8_index_strings(df.index().labels());
        let nrows = labels.len();
        (Just(df), proptest::sample::subsequence(labels, 0..=nrows))
    })
}

fn arb_dataframe_row_drop_composition_case(
    max_rows: usize,
) -> impl Strategy<Value = (DataFrame, Vec<String>, Vec<String>)> {
    arb_unique_utf8_numeric_dataframe(max_rows).prop_flat_map(|df| {
        let labels = utf8_index_strings(df.index().labels());
        let nrows = labels.len();
        (
            Just(df),
            proptest::sample::subsequence(labels.clone(), 0..=nrows),
            proptest::sample::subsequence(labels, 0..=nrows),
        )
    })
}

fn arb_dataframe_column_drop_case(
    max_rows: usize,
) -> impl Strategy<Value = (DataFrame, Vec<String>)> {
    arb_unique_utf8_numeric_dataframe(max_rows).prop_flat_map(|df| {
        let columns = df.column_names().into_iter().cloned().collect::<Vec<_>>();
        let ncols = columns.len();
        (Just(df), proptest::sample::subsequence(columns, 0..=ncols))
    })
}

fn arb_dataframe_column_drop_composition_case(
    max_rows: usize,
) -> impl Strategy<Value = (DataFrame, Vec<String>, Vec<String>)> {
    arb_unique_utf8_numeric_dataframe(max_rows).prop_flat_map(|df| {
        let columns = df.column_names().into_iter().cloned().collect::<Vec<_>>();
        let ncols = columns.len();
        (
            Just(df),
            proptest::sample::subsequence(columns.clone(), 0..=ncols),
        )
            .prop_flat_map(move |(df, first)| {
                let remaining = columns
                    .iter()
                    .filter(|name| !first.contains(*name))
                    .cloned()
                    .collect::<Vec<_>>();
                let remaining_len = remaining.len();
                (
                    Just(df),
                    Just(first),
                    proptest::sample::subsequence(remaining, 0..=remaining_len),
                )
            })
    })
}

fn drop_dataframe_rows(df: &DataFrame, labels: &[String]) -> DataFrame {
    let refs = labels.iter().map(String::as_str).collect::<Vec<_>>();
    df.drop(&refs, 0)
        .expect("DataFrame::drop(axis=0) must succeed for existing Utf8 row labels")
}

fn drop_dataframe_columns(df: &DataFrame, columns: &[String]) -> DataFrame {
    let refs = columns.iter().map(String::as_str).collect::<Vec<_>>();
    df.drop(&refs, 1)
        .expect("DataFrame::drop(axis=1) must succeed for existing column labels")
}

fn arb_head_tail_count(len: usize) -> impl Strategy<Value = i64> {
    let upper = i64::try_from(len.saturating_add(3)).expect("head/tail bound must fit in i64");
    0..=upper
}

fn arb_series_head_tail_case(
    name: &'static str,
    max_len: usize,
) -> impl Strategy<Value = (Series, i64)> {
    (1..=max_len).prop_flat_map(move |len| {
        arb_numeric_series(name, len)
            .prop_flat_map(move |series| (Just(series), arb_head_tail_count(len)))
    })
}

fn arb_series_head_tail_composition_case(
    name: &'static str,
    max_len: usize,
) -> impl Strategy<Value = (Series, i64, i64)> {
    (1..=max_len).prop_flat_map(move |len| {
        arb_numeric_series(name, len).prop_flat_map(move |series| {
            (
                Just(series),
                arb_head_tail_count(len),
                arb_head_tail_count(len),
            )
        })
    })
}

fn arb_dataframe_head_tail_case(max_rows: usize) -> impl Strategy<Value = (DataFrame, i64)> {
    (1..=max_rows).prop_flat_map(|nrows| {
        arb_numeric_dataframe(nrows).prop_flat_map(move |df| (Just(df), arb_head_tail_count(nrows)))
    })
}

fn arb_dataframe_head_tail_composition_case(
    max_rows: usize,
) -> impl Strategy<Value = (DataFrame, i64, i64)> {
    (1..=max_rows).prop_flat_map(|nrows| {
        arb_numeric_dataframe(nrows).prop_flat_map(move |df| {
            (
                Just(df),
                arb_head_tail_count(nrows),
                arb_head_tail_count(nrows),
            )
        })
    })
}

fn normalize_take_position(idx: i64, len: usize) -> usize {
    let len_i64 = i64::try_from(len).expect("take length must fit in i64");
    let resolved = if idx < 0 {
        len_i64
            .checked_add(idx)
            .expect("negative take index resolution must not overflow")
    } else {
        idx
    };
    usize::try_from(resolved).expect("normalized take index must be non-negative")
}

fn normalized_take_positions(indices: &[i64], len: usize) -> Vec<usize> {
    indices
        .iter()
        .map(|&idx| normalize_take_position(idx, len))
        .collect()
}

fn compose_take_indices(outer: &[i64], inner: &[i64], len: usize) -> Vec<i64> {
    let outer_positions = normalized_take_positions(outer, len);
    let inner_positions = normalized_take_positions(inner, outer_positions.len());
    inner_positions
        .into_iter()
        .map(|idx| {
            i64::try_from(outer_positions[idx]).expect("composed take index must fit in i64")
        })
        .collect()
}

fn take_rows_via_normalized_positions(df: &DataFrame, indices: &[i64]) -> DataFrame {
    let normalized = normalized_take_positions(indices, df.len());
    df.take_rows(&normalized)
        .expect("DataFrame::take_rows() must succeed for normalized indices")
}

fn take_columns_via_normalized_positions(df: &DataFrame, indices: &[i64]) -> DataFrame {
    let normalized = normalized_take_positions(indices, df.column_names().len());
    df.take_columns(&normalized)
        .expect("DataFrame::take_columns() must succeed for normalized indices")
}

fn negate_condition_scalar(value: &Scalar) -> Scalar {
    match value {
        Scalar::Bool(v) => Scalar::Bool(!v),
        Scalar::Null(kind) => Scalar::Null(*kind),
        _ => Scalar::Null(NullKind::NaN),
    }
}

fn negate_condition_series(series: &Series) -> Series {
    let negated_values = series
        .values()
        .iter()
        .map(negate_condition_scalar)
        .collect::<Vec<_>>();
    Series::from_values(
        series.name().to_owned(),
        series.index().labels().to_vec(),
        negated_values,
    )
    .expect("negated condition series must construct")
}

fn negate_condition_dataframe(df: &DataFrame) -> DataFrame {
    let mut negated_columns = std::collections::BTreeMap::new();
    let column_order = df
        .column_names()
        .into_iter()
        .map(|name| {
            let values = df
                .column(name)
                .expect("column listed in order must exist")
                .values()
                .iter()
                .map(negate_condition_scalar)
                .collect::<Vec<_>>();
            negated_columns.insert(
                name.clone(),
                fp_columnar::Column::from_values(values)
                    .expect("negated condition dataframe column must construct"),
            );
            name.clone()
        })
        .collect::<Vec<_>>();
    DataFrame::new_with_column_order(df.index().clone(), negated_columns, column_order)
        .expect("negated condition dataframe must construct")
}

fn shift_numeric_scalar(value: &Scalar, delta: f64) -> Scalar {
    match value {
        Scalar::Int64(v) => Scalar::Float64(*v as f64 + delta),
        Scalar::Float64(v) => Scalar::Float64(*v + delta),
        Scalar::Null(kind) => Scalar::Null(*kind),
        other => other.clone(),
    }
}

fn shift_series(series: &Series, delta: f64) -> Series {
    let shifted_values = series
        .values()
        .iter()
        .map(|value| shift_numeric_scalar(value, delta))
        .collect::<Vec<_>>();
    Series::from_values(
        series.name().to_owned(),
        series.index().labels().to_vec(),
        shifted_values,
    )
    .expect("shifted series must construct")
}

fn shift_dataframe(df: &DataFrame, delta: f64) -> DataFrame {
    let mut shifted_columns = std::collections::BTreeMap::new();
    let column_order = df
        .column_names()
        .into_iter()
        .map(|name| {
            let values = df
                .column(name)
                .expect("column listed in order must exist")
                .values()
                .iter()
                .map(|value| shift_numeric_scalar(value, delta))
                .collect::<Vec<_>>();
            shifted_columns.insert(
                name.clone(),
                fp_columnar::Column::from_values(values)
                    .expect("shifted dataframe column must construct"),
            );
            name.clone()
        })
        .collect::<Vec<_>>();
    DataFrame::new_with_column_order(df.index().clone(), shifted_columns, column_order)
        .expect("shifted dataframe must construct")
}

fn shift_dataframe_column(df: &DataFrame, column_name: &str, delta: f64) -> DataFrame {
    let mut shifted_columns = std::collections::BTreeMap::new();
    let column_order = df
        .column_names()
        .into_iter()
        .map(|name| {
            let values = df
                .column(name)
                .expect("column listed in order must exist")
                .values()
                .iter()
                .map(|value| {
                    if name == column_name {
                        shift_numeric_scalar(value, delta)
                    } else {
                        value.clone()
                    }
                })
                .collect::<Vec<_>>();
            shifted_columns.insert(
                name.clone(),
                fp_columnar::Column::from_values(values)
                    .expect("partially shifted dataframe column must construct"),
            );
            name.clone()
        })
        .collect::<Vec<_>>();
    DataFrame::new_with_column_order(df.index().clone(), shifted_columns, column_order)
        .expect("partially shifted dataframe must construct")
}

fn uniform_replace_dict(
    df: &DataFrame,
    replacements: &[(Scalar, Scalar)],
) -> std::collections::BTreeMap<String, Vec<(Scalar, Scalar)>> {
    df.column_names()
        .into_iter()
        .cloned()
        .map(|name| (name, replacements.to_vec()))
        .collect()
}

fn scale_numeric_scalar(value: &Scalar, factor: f64) -> Scalar {
    match value {
        Scalar::Int64(v) => Scalar::Float64(*v as f64 * factor),
        Scalar::Float64(v) => Scalar::Float64(*v * factor),
        Scalar::Null(kind) => Scalar::Null(*kind),
        other => other.clone(),
    }
}

fn scale_series(series: &Series, factor: f64) -> Series {
    let scaled_values = series
        .values()
        .iter()
        .map(|value| scale_numeric_scalar(value, factor))
        .collect::<Vec<_>>();
    Series::from_values(
        series.name().to_owned(),
        series.index().labels().to_vec(),
        scaled_values,
    )
    .expect("scaled series must construct")
}

fn scale_dataframe(df: &DataFrame, factor: f64) -> DataFrame {
    let mut scaled_columns = std::collections::BTreeMap::new();
    let column_order = df
        .column_names()
        .into_iter()
        .map(|name| {
            let values = df
                .column(name)
                .expect("column listed in order must exist")
                .values()
                .iter()
                .map(|value| scale_numeric_scalar(value, factor))
                .collect::<Vec<_>>();
            scaled_columns.insert(
                name.clone(),
                fp_columnar::Column::from_values(values)
                    .expect("scaled dataframe column must construct"),
            );
            name.clone()
        })
        .collect::<Vec<_>>();
    DataFrame::new_with_column_order(df.index().clone(), scaled_columns, column_order)
        .expect("scaled dataframe must construct")
}

fn approx_equal_scalar(lhs: &Scalar, rhs: &Scalar) -> bool {
    if lhs.is_missing() || rhs.is_missing() {
        return lhs.is_missing() && rhs.is_missing();
    }

    match (lhs.to_f64(), rhs.to_f64()) {
        (Ok(a), Ok(b)) if a.is_finite() && b.is_finite() => {
            let tol = a.abs().max(b.abs()).max(1.0) * 1e-9;
            (a - b).abs() <= tol
        }
        _ => lhs.semantic_eq(rhs),
    }
}

fn approx_equal_series(lhs: &Series, rhs: &Series) -> bool {
    lhs.name() == rhs.name()
        && lhs.index().labels() == rhs.index().labels()
        && lhs.len() == rhs.len()
        && lhs
            .values()
            .iter()
            .zip(rhs.values())
            .all(|(left, right)| approx_equal_scalar(left, right))
}

fn same_series_payload(lhs: &Series, rhs: &Series) -> bool {
    lhs.index().labels() == rhs.index().labels()
        && lhs.len() == rhs.len()
        && lhs
            .values()
            .iter()
            .zip(rhs.values())
            .all(|(left, right)| approx_equal_scalar(left, right))
}

fn approx_equal_dataframe(lhs: &DataFrame, rhs: &DataFrame) -> bool {
    let lhs_columns = lhs.column_names();
    let rhs_columns = rhs.column_names();
    lhs.index().labels() == rhs.index().labels()
        && lhs_columns == rhs_columns
        && lhs_columns.into_iter().all(|name| {
            let left = lhs.column(name).expect("column listed in order must exist");
            let right = rhs.column(name).expect("column listed in order must exist");
            left.values().len() == right.values().len()
                && left
                    .values()
                    .iter()
                    .zip(right.values())
                    .all(|(left, right)| approx_equal_scalar(left, right))
        })
}

fn same_index_result(
    lhs: &Result<IndexLabel, FrameError>,
    rhs: &Result<IndexLabel, FrameError>,
) -> bool {
    match (lhs, rhs) {
        (Ok(left), Ok(right)) => left == right,
        (Err(_), Err(_)) => true,
        _ => false,
    }
}

fn reconstruct_pct_change_scalar(delta: &Scalar, previous: &Scalar) -> Scalar {
    if delta.is_missing() || previous.is_missing() {
        return Scalar::Null(NullKind::NaN);
    }

    match (delta.to_f64(), previous.to_f64()) {
        (Ok(diff), Ok(prev)) => Scalar::Float64(diff / prev),
        _ => Scalar::Null(NullKind::NaN),
    }
}

fn reconstruct_pct_change_series(series: &Series, periods: usize) -> Series {
    let diff = series
        .diff(periods as i64)
        .expect("Series::diff() must succeed while reconstructing pct_change");
    let shifted = series
        .shift(periods as i64)
        .expect("Series::shift() must succeed while reconstructing pct_change");
    let values = diff
        .values()
        .iter()
        .zip(shifted.values())
        .map(|(delta, previous)| reconstruct_pct_change_scalar(delta, previous))
        .collect::<Vec<_>>();
    Series::from_values(
        series.name().to_owned(),
        series.index().labels().to_vec(),
        values,
    )
    .expect("reconstructed pct_change series must construct")
}

fn reconstruct_pct_change_dataframe(df: &DataFrame, periods: usize) -> DataFrame {
    let diff = df
        .diff(periods as i64)
        .expect("DataFrame::diff() must succeed while reconstructing pct_change");
    let shifted = df
        .shift(periods as i64)
        .expect("DataFrame::shift() must succeed while reconstructing pct_change");

    let column_order = df.column_names().into_iter().cloned().collect::<Vec<_>>();
    let mut columns = std::collections::BTreeMap::new();
    for name in &column_order {
        let diff_col = diff
            .column(name)
            .expect("diff dataframe column listed in order must exist");
        let shifted_col = shifted
            .column(name)
            .expect("shifted dataframe column listed in order must exist");
        let values = diff_col
            .values()
            .iter()
            .zip(shifted_col.values())
            .map(|(delta, previous)| reconstruct_pct_change_scalar(delta, previous))
            .collect::<Vec<_>>();
        columns.insert(
            name.clone(),
            fp_columnar::Column::from_values(values)
                .expect("reconstructed pct_change dataframe column must construct"),
        );
    }

    DataFrame::new_with_column_order(df.index().clone(), columns, column_order)
        .expect("reconstructed pct_change dataframe must construct")
}

fn reconstruct_pct_change_axis1_dataframe(df: &DataFrame, periods: i64) -> DataFrame {
    let diff = df
        .diff_axis1(periods)
        .expect("DataFrame::diff_axis1() must succeed while reconstructing pct_change");
    let shifted = df
        .shift_axis1(periods)
        .expect("DataFrame::shift_axis1() must succeed while reconstructing pct_change");

    let column_order = df.column_names().into_iter().cloned().collect::<Vec<_>>();
    let mut columns = std::collections::BTreeMap::new();
    for name in &column_order {
        let diff_col = diff
            .column(name)
            .expect("diff_axis1 dataframe column listed in order must exist");
        let shifted_col = shifted
            .column(name)
            .expect("shift_axis1 dataframe column listed in order must exist");
        let values = diff_col
            .values()
            .iter()
            .zip(shifted_col.values())
            .map(|(delta, previous)| reconstruct_pct_change_scalar(delta, previous))
            .collect::<Vec<_>>();
        columns.insert(
            name.clone(),
            fp_columnar::Column::from_values(values)
                .expect("reconstructed pct_change_axis1 dataframe column must construct"),
        );
    }

    DataFrame::new_with_column_order(df.index().clone(), columns, column_order)
        .expect("reconstructed pct_change_axis1 dataframe must construct")
}

fn non_missing_prefix_counts(values: &[Scalar]) -> Vec<usize> {
    let mut seen = 0usize;
    values
        .iter()
        .map(|value| {
            if !value.is_missing() {
                seen += 1;
            }
            seen
        })
        .collect()
}

fn scale_cumprod_output_value(value: &Scalar, factor: f64, exponent: usize) -> Scalar {
    if value.is_missing() {
        Scalar::Null(NullKind::NaN)
    } else {
        Scalar::Float64(
            value
                .to_f64()
                .expect("cumprod output must be numeric for non-missing values")
                * factor.powi(exponent as i32),
        )
    }
}

fn expected_scaled_cumprod_series(input: &Series, factor: f64) -> Series {
    let baseline = input
        .cumprod()
        .expect("Series::cumprod() must succeed while scaling expected output");
    let counts = non_missing_prefix_counts(input.values());
    let values = baseline
        .values()
        .iter()
        .zip(counts)
        .map(|(value, exponent)| scale_cumprod_output_value(value, factor, exponent))
        .collect::<Vec<_>>();
    Series::from_values(
        baseline.name().to_owned(),
        baseline.index().labels().to_vec(),
        values,
    )
    .expect("scaled expected cumprod series must construct")
}

fn expected_scaled_cumprod_dataframe(df: &DataFrame, factor: f64) -> DataFrame {
    let baseline = df
        .cumprod()
        .expect("DataFrame::cumprod() must succeed while scaling expected output");
    let column_order = baseline
        .column_names()
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    let mut columns = std::collections::BTreeMap::new();

    for name in &column_order {
        let input_column = df.column(name).expect("input dataframe column must exist");
        let baseline_column = baseline
            .column(name)
            .expect("baseline cumprod dataframe column must exist");
        let counts = non_missing_prefix_counts(input_column.values());
        let values = baseline_column
            .values()
            .iter()
            .zip(counts)
            .map(|(value, exponent)| scale_cumprod_output_value(value, factor, exponent))
            .collect::<Vec<_>>();
        columns.insert(
            name.clone(),
            fp_columnar::Column::from_values(values)
                .expect("scaled expected cumprod dataframe column must construct"),
        );
    }

    DataFrame::new_with_column_order(df.index().clone(), columns, column_order)
        .expect("scaled expected cumprod dataframe must construct")
}

fn expected_scaled_cumprod_axis1_dataframe(df: &DataFrame, factor: f64) -> DataFrame {
    let baseline = df
        .cumprod_axis1()
        .expect("DataFrame::cumprod_axis1() must succeed while scaling expected output");
    let column_order = baseline
        .column_names()
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    let mut per_column = column_order
        .iter()
        .map(|_| Vec::with_capacity(df.index().len()))
        .collect::<Vec<_>>();

    for row_idx in 0..df.index().len() {
        let mut exponent = 0usize;
        for (col_idx, name) in column_order.iter().enumerate() {
            let input_value = &df
                .column(name)
                .expect("input dataframe column must exist")
                .values()[row_idx];
            if !input_value.is_missing() {
                exponent += 1;
            }
            let baseline_value = &baseline
                .column(name)
                .expect("baseline cumprod_axis1 dataframe column must exist")
                .values()[row_idx];
            per_column[col_idx].push(scale_cumprod_output_value(baseline_value, factor, exponent));
        }
    }

    let mut columns = std::collections::BTreeMap::new();
    for (name, values) in column_order.iter().cloned().zip(per_column) {
        columns.insert(
            name,
            fp_columnar::Column::from_values(values)
                .expect("scaled expected cumprod_axis1 dataframe column must construct"),
        );
    }

    DataFrame::new_with_column_order(df.index().clone(), columns, column_order)
        .expect("scaled expected cumprod_axis1 dataframe must construct")
}

fn arb_between_inclusive() -> impl Strategy<Value = &'static str> {
    prop_oneof![Just("both"), Just("neither"), Just("left"), Just("right"),]
}

fn sign_flip_between_inclusive(inclusive: &str) -> &'static str {
    match inclusive {
        "both" => "both",
        "neither" => "neither",
        "left" => "right",
        "right" => "left",
        _ => "both",
    }
}

fn bool_values(series: &Series) -> Result<Vec<bool>, String> {
    series
        .values()
        .iter()
        .map(|value| match value {
            Scalar::Bool(flag) => Ok(*flag),
            other => Err(format!("expected bool series output, got {other:?}")),
        })
        .collect()
}

fn int64_values(series: &Series) -> Result<Vec<i64>, String> {
    series
        .values()
        .iter()
        .map(|value| match value {
            Scalar::Int64(count) => Ok(*count),
            other => Err(format!("expected Int64 series output, got {other:?}")),
        })
        .collect()
}

fn scalar_for_label<'a>(series: &'a Series, label: &IndexLabel) -> Option<&'a Scalar> {
    series
        .index()
        .labels()
        .iter()
        .zip(series.values())
        .find_map(|(candidate, value)| (candidate == label).then_some(value))
}

fn expected_shifted_sum_series(sum: &Series, count: &Series, delta: f64) -> Series {
    let values = sum
        .values()
        .iter()
        .zip(sum.index().labels())
        .map(|(value, label)| {
            let count = scalar_for_label(count, label)
                .expect("count output must include each reduced sum label");
            Scalar::Float64(
                value
                    .to_f64()
                    .expect("sum output must be numeric for non-missing columns")
                    + count
                        .to_f64()
                        .expect("count output must be numeric for reduced columns")
                        * delta,
            )
        })
        .collect::<Vec<_>>();
    Series::from_values(sum.name().to_owned(), sum.index().labels().to_vec(), values)
        .expect("expected shifted sum series must construct")
}

fn invert_bool_series(series: &Series) -> Result<Series, String> {
    let values = bool_values(series)?
        .into_iter()
        .map(|flag| Scalar::Bool(!flag))
        .collect::<Vec<_>>();
    Series::from_values(
        series.name().to_owned(),
        series.index().labels().to_vec(),
        values,
    )
    .map_err(|err| format!("inverted bool series must construct: {err}"))
}

fn invert_bool_dataframe(df: &DataFrame) -> Result<DataFrame, String> {
    let column_order = df.column_names().into_iter().cloned().collect::<Vec<_>>();
    let mut columns = std::collections::BTreeMap::new();
    for name in &column_order {
        let source = Series::from_values(
            name.clone(),
            df.index().labels().to_vec(),
            df.column(name)
                .expect("dataframe column listed in order must exist")
                .values()
                .to_vec(),
        )
        .map_err(|err| format!("dataframe bool column must project into series: {err}"))?;
        let inverted = invert_bool_series(&source)?;
        columns.insert(
            name.clone(),
            fp_columnar::Column::from_values(inverted.values().to_vec())
                .map_err(|err| format!("inverted bool dataframe column must construct: {err}"))?,
        );
    }
    DataFrame::new_with_column_order(df.index().clone(), columns, column_order)
        .map_err(|err| format!("inverted bool dataframe must construct: {err}"))
}

fn all_series_bool_values(series: &Series, expected: bool) -> Result<bool, String> {
    Ok(bool_values(series)?
        .into_iter()
        .all(|flag| flag == expected))
}

fn all_dataframe_bool_values(df: &DataFrame, expected: bool) -> Result<bool, String> {
    for name in df.column_names() {
        let series = Series::from_values(
            name.clone(),
            df.index().labels().to_vec(),
            df.column(name)
                .expect("dataframe column listed in order must exist")
                .values()
                .to_vec(),
        )
        .map_err(|err| format!("dataframe bool column must project into series: {err}"))?;
        if !all_series_bool_values(&series, expected)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn dataframe_isin_via_series(df: &DataFrame, values: &[Scalar]) -> DataFrame {
    let column_order = df.column_names().into_iter().cloned().collect::<Vec<_>>();
    let mut columns = std::collections::BTreeMap::new();
    for name in &column_order {
        let source = Series::from_values(
            name.clone(),
            df.index().labels().to_vec(),
            df.column(name)
                .expect("dataframe column listed in order must exist")
                .values()
                .to_vec(),
        )
        .expect("columnwise series must construct");
        let isin_result = source
            .isin(values)
            .expect("Series::isin() must succeed for dataframe column");
        columns.insert(
            name.clone(),
            fp_columnar::Column::from_values(isin_result.values().to_vec())
                .expect("columnwise isin result column must construct"),
        );
    }

    DataFrame::new_with_column_order(df.index().clone(), columns, column_order)
        .expect("columnwise isin dataframe must construct")
}

fn dataframe_isin_dict_via_series(
    df: &DataFrame,
    per_column: &std::collections::BTreeMap<String, Vec<Scalar>>,
) -> DataFrame {
    let column_order = df.column_names().into_iter().cloned().collect::<Vec<_>>();
    let mut columns = std::collections::BTreeMap::new();
    for name in &column_order {
        let source = Series::from_values(
            name.clone(),
            df.index().labels().to_vec(),
            df.column(name)
                .expect("dataframe column listed in order must exist")
                .values()
                .to_vec(),
        )
        .expect("columnwise series must construct");
        let allowed = per_column.get(name).cloned().unwrap_or_default();
        let isin_result = source
            .isin(&allowed)
            .expect("Series::isin() must succeed for dataframe column dict values");
        columns.insert(
            name.clone(),
            fp_columnar::Column::from_values(isin_result.values().to_vec())
                .expect("columnwise isin_dict result column must construct"),
        );
    }

    DataFrame::new_with_column_order(df.index().clone(), columns, column_order)
        .expect("columnwise isin_dict dataframe must construct")
}

fn sorted_bounds(a: f64, b: f64) -> (f64, f64) {
    if a <= b { (a, b) } else { (b, a) }
}

fn nested_clip_bounds(
    outer_a: f64,
    outer_b: f64,
    inner_frac_a: f64,
    inner_frac_b: f64,
) -> (f64, f64, f64, f64) {
    let (outer_lower, outer_upper) = sorted_bounds(outer_a, outer_b);
    let width = outer_upper - outer_lower;
    let (inner_start, inner_end) = sorted_bounds(inner_frac_a, inner_frac_b);
    let inner_lower = outer_lower + width * inner_start;
    let inner_upper = outer_lower + width * inner_end;
    (outer_lower, outer_upper, inner_lower, inner_upper)
}

fn poison_series_right_cells_for_combine_first(left: &Series, right: &Series) -> Series {
    let left_lookup: std::collections::BTreeMap<&IndexLabel, &Scalar> = left
        .index()
        .labels()
        .iter()
        .zip(left.values().iter())
        .collect();

    let poisoned_values = right
        .index()
        .labels()
        .iter()
        .zip(right.values().iter())
        .map(|(label, value)| {
            if let Some(left_value) = left_lookup.get(label)
                && !left_value.is_missing()
            {
                return poison_numeric_scalar(value);
            }
            value.clone()
        })
        .collect::<Vec<_>>();

    Series::from_values(
        right.name().to_owned(),
        right.index().labels().to_vec(),
        poisoned_values,
    )
    .expect("poisoned right series must construct")
}

fn poison_dataframe_right_cells_for_combine_first(
    left: &DataFrame,
    right: &DataFrame,
) -> DataFrame {
    let left_row_positions: std::collections::BTreeMap<&IndexLabel, usize> = left
        .index()
        .labels()
        .iter()
        .enumerate()
        .map(|(position, label)| (label, position))
        .collect();

    let column_order = right
        .column_names()
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    let mut poisoned_columns = std::collections::BTreeMap::new();

    for column_name in &column_order {
        let right_column = right
            .column(column_name)
            .expect("column name listed in order must exist");
        let left_column = left.column(column_name);
        let poisoned_values = right
            .index()
            .labels()
            .iter()
            .enumerate()
            .map(|(right_position, label)| {
                if let Some(left_column) = left_column
                    && let Some(&left_position) = left_row_positions.get(label)
                {
                    let left_value = &left_column.values()[left_position];
                    if !left_value.is_missing() {
                        return poison_numeric_scalar(&right_column.values()[right_position]);
                    }
                }
                right_column.values()[right_position].clone()
            })
            .collect::<Vec<_>>();
        poisoned_columns.insert(
            column_name.clone(),
            fp_columnar::Column::from_values(poisoned_values)
                .expect("poisoned right dataframe column must construct"),
        );
    }

    DataFrame::new_with_column_order(right.index().clone(), poisoned_columns, column_order)
        .expect("poisoned right dataframe must construct")
}

fn normalized_join_rows(joined: &JoinedSeries) -> Vec<String> {
    let mut rows = joined
        .index
        .labels()
        .iter()
        .zip(joined.left_values.values().iter())
        .zip(joined.right_values.values().iter())
        .map(|((label, left_value), right_value)| {
            format!("{label:?}|{left_value:?}|{right_value:?}")
        })
        .collect::<Vec<_>>();
    rows.sort();
    rows
}

fn normalized_join_rows_with_swapped_sides(joined: &JoinedSeries) -> Vec<String> {
    let mut rows = joined
        .index
        .labels()
        .iter()
        .zip(joined.left_values.values().iter())
        .zip(joined.right_values.values().iter())
        .map(|((label, left_value), right_value)| {
            format!("{label:?}|{right_value:?}|{left_value:?}")
        })
        .collect::<Vec<_>>();
    rows.sort();
    rows
}

fn normalized_series_rows(series: &Series) -> Vec<String> {
    let mut rows = series
        .index()
        .labels()
        .iter()
        .zip(series.values().iter())
        .map(|(label, value)| format!("{label:?}|{value:?}"))
        .collect::<Vec<_>>();
    rows.sort();
    rows
}

fn normalized_dataframe_rows(df: &DataFrame) -> Vec<String> {
    let column_names = df.column_names();
    let mut rows = (0..df.index().len())
        .map(|row_idx| {
            let rendered_values = column_names
                .iter()
                .map(|name| {
                    format!(
                        "{name}:{:?}",
                        df.column(name)
                            .expect("column listed in order must exist")
                            .values()[row_idx]
                    )
                })
                .collect::<Vec<_>>()
                .join("|");
            format!("{:?}|{rendered_values}", df.index().labels()[row_idx])
        })
        .collect::<Vec<_>>();
    rows.sort();
    rows
}

fn ordered_series_rows(series: &Series) -> Vec<String> {
    series
        .index()
        .labels()
        .iter()
        .zip(series.values().iter())
        .map(|(label, value)| format!("{label:?}|{value:?}"))
        .collect()
}

fn ordered_dataframe_rows(df: &DataFrame) -> Vec<String> {
    let column_names = df.column_names();
    (0..df.index().len())
        .map(|row_idx| {
            let rendered_values = column_names
                .iter()
                .map(|name| {
                    format!(
                        "{name}:{:?}",
                        df.column(name)
                            .expect("column listed in order must exist")
                            .values()[row_idx]
                    )
                })
                .collect::<Vec<_>>()
                .join("|");
            format!("{:?}|{rendered_values}", df.index().labels()[row_idx])
        })
        .collect()
}

fn sample_count_from_hint(total: usize, replace: bool, hint: usize) -> usize {
    let upper_bound = if replace {
        total.saturating_mul(2).max(1)
    } else {
        total.max(1)
    };
    1 + (hint % upper_bound)
}

// ---------------------------------------------------------------------------
// Property: Index alignment invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    /// align_union always produces a valid alignment plan.
    #[test]
    fn prop_align_union_plan_is_valid((left, right) in arb_index_pair(20)) {
        let plan = align_union(&left, &right);
        validate_alignment_plan(&plan).expect("alignment plan must be valid");
    }

    /// align_union union index contains all left labels.
    #[test]
    fn prop_align_union_contains_all_left_labels((left, right) in arb_index_pair(20)) {
        let plan = align_union(&left, &right);
        let union_labels = plan.union_index.labels();
        for label in left.labels() {
            prop_assert!(
                union_labels.contains(label),
                "union must contain left label {:?}", label
            );
        }
    }

    /// align_union union index contains all right labels.
    #[test]
    fn prop_align_union_contains_all_right_labels((left, right) in arb_index_pair(20)) {
        let plan = align_union(&left, &right);
        let union_labels = plan.union_index.labels();
        for label in right.labels() {
            prop_assert!(
                union_labels.contains(label),
                "union must contain right label {:?}", label
            );
        }
    }

    /// align_union position vectors have the same length as the union index.
    #[test]
    fn prop_align_union_position_lengths_match((left, right) in arb_index_pair(20)) {
        let plan = align_union(&left, &right);
        let n = plan.union_index.len();
        prop_assert_eq!(plan.left_positions.len(), n);
        prop_assert_eq!(plan.right_positions.len(), n);
    }

    /// align_union preserves left order for unique indices: non-None left
    /// positions are strictly increasing when both inputs have no duplicates.
    /// If either side has duplicates, alignment can repeat positions.
    #[test]
    fn prop_align_union_preserves_left_order((left, right) in arb_index_pair(20)) {
        if left.has_duplicates() || right.has_duplicates() {
            // With duplicates, position_map_first introduces non-monotonic
            // position references. This is correct behavior.
            return Ok(());
        }
        let plan = align_union(&left, &right);
        let mut prev_pos: Option<usize> = None;
        for p in plan.left_positions.iter().flatten() {
            if let Some(prev) = prev_pos {
                prop_assert!(
                    *p > prev,
                    "left positions must be strictly increasing for unique index: prev={}, current={}", prev, *p
                );
            }
            prev_pos = Some(*p);
        }
    }
}

// ---------------------------------------------------------------------------
// Property: Index duplicate detection
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    /// has_duplicates is consistent with a naive O(n^2) check.
    #[test]
    fn prop_has_duplicates_matches_naive(index in arb_index(20)) {
        let has_dups = index.has_duplicates();
        let labels = index.labels();
        let naive = labels.iter().enumerate().any(|(i, l)| {
            labels[..i].contains(l)
        });
        prop_assert_eq!(has_dups, naive, "has_duplicates must match naive check");
    }

    /// has_duplicates is deterministic across calls.
    #[test]
    fn prop_has_duplicates_is_deterministic(index in arb_index(20)) {
        let first = index.has_duplicates();
        let second = index.has_duplicates();
        prop_assert_eq!(first, second);
    }
}

// ---------------------------------------------------------------------------
// Property: Series addition invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Series add in hardened mode never panics; it either succeeds or returns an error.
    #[test]
    fn prop_series_add_hardened_no_panic((left, right) in arb_series_pair(15)) {
        let policy = RuntimePolicy::hardened(Some(100_000));
        let mut ledger = EvidenceLedger::new();
        let _ = left.add_with_policy(&right, &policy, &mut ledger);
        // Property: no panic occurred.
    }

    /// Series add result index length equals result values length.
    #[test]
    fn prop_series_add_index_values_length_match((left, right) in arb_series_pair(15)) {
        let policy = RuntimePolicy::hardened(Some(100_000));
        let mut ledger = EvidenceLedger::new();
        if let Ok(result) = left.add_with_policy(&right, &policy, &mut ledger) {
            prop_assert_eq!(
                result.index().len(),
                result.values().len(),
                "index and values must have same length"
            );
        }
    }

    /// Series add result index is the union of left and right indices.
    #[test]
    fn prop_series_add_result_index_is_union((left, right) in arb_series_pair(10)) {
        let policy = RuntimePolicy::hardened(Some(100_000));
        let mut ledger = EvidenceLedger::new();
        if let Ok(result) = left.add_with_policy(&right, &policy, &mut ledger) {
            let result_labels = result.index().labels();
            for label in left.index().labels() {
                prop_assert!(
                    result_labels.contains(label),
                    "result index must contain left label {:?}", label
                );
            }
            for label in right.index().labels() {
                prop_assert!(
                    result_labels.contains(label),
                    "result index must contain right label {:?}", label
                );
            }
        }
    }

    /// Series add with itself produces values that are 2x the original (for non-missing),
    /// but only when the index has no duplicates. With duplicates, alignment maps to
    /// first occurrence (position_map_first), so subsequent duplicate-index positions
    /// get the value at the first position, not their own.
    #[test]
    fn prop_series_add_self_doubles_values(series in arb_numeric_series("self_add", 10)) {
        // Only test the doubling property when the index is unique.
        if series.index().has_duplicates() {
            return Ok(());
        }
        let policy = RuntimePolicy::hardened(Some(100_000));
        let mut ledger = EvidenceLedger::new();
        if let Ok(result) = series.add_with_policy(&series, &policy, &mut ledger) {
            for (i, (orig, doubled)) in series.values().iter().zip(result.values().iter()).enumerate() {
                if orig.is_missing() {
                    prop_assert!(
                        doubled.is_missing(),
                        "missing + missing should be missing at idx={}", i
                    );
                } else if let Ok(v) = orig.to_f64()
                    && let Ok(r) = doubled.to_f64()
                {
                    let expected = v * 2.0;
                    if expected.is_finite() {
                        prop_assert!(
                            (r - expected).abs() < 1e-9,
                            "self-add should double: {} + {} = {} (expected {}) at idx={}",
                            v, v, r, expected, i
                        );
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Property: between metamorphic invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Inclusive-mode variants obey the expected boolean algebra:
    /// both = left OR right, neither = left AND right.
    #[test]
    fn prop_series_between_inclusive_modes_form_expected_lattice(
        series in arb_numeric_series("between", 12),
        bound_a in -1e6_f64..1e6_f64,
        bound_b in -1e6_f64..1e6_f64,
    ) {
        let (lower, upper) = sorted_bounds(bound_a, bound_b);
        let left = series
            .between(&Scalar::Float64(lower), &Scalar::Float64(upper), "left")
            .expect("Series::between(left) must succeed for numeric inputs");
        let right = series
            .between(&Scalar::Float64(lower), &Scalar::Float64(upper), "right")
            .expect("Series::between(right) must succeed for numeric inputs");
        let both = series
            .between(&Scalar::Float64(lower), &Scalar::Float64(upper), "both")
            .expect("Series::between(both) must succeed for numeric inputs");
        let neither = series
            .between(&Scalar::Float64(lower), &Scalar::Float64(upper), "neither")
            .expect("Series::between(neither) must succeed for numeric inputs");

        let left_values = bool_values(&left).expect("between(left) must produce bool output");
        let right_values = bool_values(&right).expect("between(right) must produce bool output");
        let both_values = bool_values(&both).expect("between(both) must produce bool output");
        let neither_values =
            bool_values(&neither).expect("between(neither) must produce bool output");

        for (idx, (((left_flag, right_flag), both_flag), neither_flag)) in left_values
            .into_iter()
            .zip(right_values)
            .zip(both_values)
            .zip(neither_values)
            .enumerate()
        {
            prop_assert_eq!(
                both_flag,
                left_flag || right_flag,
                "between(both) must equal between(left) OR between(right) at idx={}",
                idx
            );
            prop_assert_eq!(
                neither_flag,
                left_flag && right_flag,
                "between(neither) must equal between(left) AND between(right) at idx={}",
                idx
            );
        }
    }

    /// Translating the series and both bounds by the same delta must preserve the result.
    #[test]
    fn prop_series_between_is_translation_invariant(
        series in arb_numeric_series("between", 12),
        bound_a in -1e6_f64..1e6_f64,
        bound_b in -1e6_f64..1e6_f64,
        delta in -1e3_f64..1e3_f64,
        inclusive in arb_between_inclusive(),
    ) {
        let (lower, upper) = sorted_bounds(bound_a, bound_b);
        let baseline = series
            .between(&Scalar::Float64(lower), &Scalar::Float64(upper), inclusive)
            .expect("Series::between() must succeed for numeric inputs");
        let shifted = shift_series(&series, delta);
        let shifted_result = shifted
            .between(
                &Scalar::Float64(lower + delta),
                &Scalar::Float64(upper + delta),
                inclusive,
            )
            .expect("translated Series::between() must succeed for numeric inputs");
        prop_assert!(
            baseline.equals(&shifted_result),
            "series between should be translation invariant for inclusive={inclusive}"
        );
    }

    /// Negating the series and swapping/negating bounds must preserve the result,
    /// with left/right inclusivity exchanged.
    #[test]
    fn prop_series_between_is_sign_symmetric(
        series in arb_numeric_series("between", 12),
        bound_a in -1e6_f64..1e6_f64,
        bound_b in -1e6_f64..1e6_f64,
        inclusive in arb_between_inclusive(),
    ) {
        let (lower, upper) = sorted_bounds(bound_a, bound_b);
        let baseline = series
            .between(&Scalar::Float64(lower), &Scalar::Float64(upper), inclusive)
            .expect("Series::between() must succeed for numeric inputs");
        let flipped_series = sign_flip_series(&series);
        let flipped_result = flipped_series
            .between(
                &Scalar::Float64(-upper),
                &Scalar::Float64(-lower),
                sign_flip_between_inclusive(inclusive),
            )
            .expect("sign-flipped Series::between() must succeed for numeric inputs");
        prop_assert!(
            baseline.equals(&flipped_result),
            "series between should commute with sign flip for inclusive={inclusive}"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: round metamorphic invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Series round with the same precision is idempotent.
    #[test]
    fn prop_series_round_is_idempotent(
        series in arb_numeric_series("round", 12),
        decimals in -3i32..=6,
    ) {
        let once = series
            .round(decimals)
            .expect("Series::round() must succeed for numeric inputs");
        let twice = once
            .round(decimals)
            .expect("Series::round() must succeed on its own output");
        prop_assert!(
            once.equals(&twice),
            "series round must be idempotent for fixed precision"
        );
    }

    /// Negating every numeric value before rounding must negate the rounded result.
    #[test]
    fn prop_series_round_is_sign_symmetric(
        series in arb_numeric_series("round", 12),
        decimals in -3i32..=6,
    ) {
        let baseline = series
            .round(decimals)
            .expect("Series::round() must succeed for numeric inputs");
        let flipped = sign_flip_series(&series);
        let flipped_rounded = flipped
            .round(decimals)
            .expect("Series::round() must succeed after sign flipping");
        let expected = sign_flip_series(&baseline);
        prop_assert!(
            flipped_rounded.equals(&expected),
            "series round should commute with numeric sign flip"
        );
    }

    /// DataFrame round with the same precision is idempotent.
    #[test]
    fn prop_dataframe_round_is_idempotent(
        df in arb_numeric_dataframe(8),
        decimals in -3i32..=6,
    ) {
        let once = df
            .round(decimals)
            .expect("DataFrame::round() must succeed for numeric inputs");
        let twice = once
            .round(decimals)
            .expect("DataFrame::round() must succeed on its own output");
        prop_assert!(
            once.equals(&twice),
            "dataframe round must be idempotent for fixed precision"
        );
    }

    /// Negating every numeric cell before rounding must negate the rounded result.
    #[test]
    fn prop_dataframe_round_is_sign_symmetric(
        df in arb_numeric_dataframe(8),
        decimals in -3i32..=6,
    ) {
        let baseline = df
            .round(decimals)
            .expect("DataFrame::round() must succeed for numeric inputs");
        let flipped = df
            .mul_scalar(-1.0)
            .expect("DataFrame::mul_scalar(-1.0) must succeed for numeric inputs");
        let flipped_rounded = flipped
            .round(decimals)
            .expect("DataFrame::round() must succeed after sign flipping");
        let expected = baseline
            .mul_scalar(-1.0)
            .expect("rounded dataframe must support numeric sign flipping");
        prop_assert!(
            flipped_rounded.equals(&expected),
            "dataframe round should commute with numeric sign flip"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: clip metamorphic invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Series clip with the same bounds is idempotent.
    #[test]
    fn prop_series_clip_is_idempotent(
        series in arb_numeric_series("clip", 12),
        bound_a in -1e6_f64..1e6_f64,
        bound_b in -1e6_f64..1e6_f64,
    ) {
        let (lower, upper) = sorted_bounds(bound_a, bound_b);
        let once = series
            .clip(Some(lower), Some(upper))
            .expect("Series::clip() must succeed for numeric inputs");
        let twice = once
            .clip(Some(lower), Some(upper))
            .expect("Series::clip() must succeed on its own output");
        prop_assert!(
            once.equals(&twice),
            "series clip must be idempotent for fixed bounds"
        );
    }

    /// Tightening clip bounds in a second pass must equal clipping directly to the tighter interval.
    #[test]
    fn prop_series_clip_nested_bounds_compose(
        series in arb_numeric_series("clip", 12),
        outer_a in -1e6_f64..1e6_f64,
        outer_b in -1e6_f64..1e6_f64,
        inner_frac_a in 0.0f64..1.0,
        inner_frac_b in 0.0f64..1.0,
    ) {
        let (outer_lower, outer_upper, inner_lower, inner_upper) =
            nested_clip_bounds(outer_a, outer_b, inner_frac_a, inner_frac_b);
        let staged = series
            .clip(Some(outer_lower), Some(outer_upper))
            .and_then(|clipped| clipped.clip(Some(inner_lower), Some(inner_upper)))
            .expect("Series::clip() composition must succeed");
        let direct = series
            .clip(Some(inner_lower), Some(inner_upper))
            .expect("Series::clip() must succeed for nested bounds");
        prop_assert!(
            staged.equals(&direct),
            "series clip should compose to the tighter interval"
        );
    }

    /// DataFrame clip with the same bounds is idempotent.
    #[test]
    fn prop_dataframe_clip_is_idempotent(
        df in arb_numeric_dataframe(8),
        bound_a in -1e6_f64..1e6_f64,
        bound_b in -1e6_f64..1e6_f64,
    ) {
        let (lower, upper) = sorted_bounds(bound_a, bound_b);
        let once = df
            .clip(Some(lower), Some(upper))
            .expect("DataFrame::clip() must succeed for numeric inputs");
        let twice = once
            .clip(Some(lower), Some(upper))
            .expect("DataFrame::clip() must succeed on its own output");
        prop_assert!(
            once.equals(&twice),
            "dataframe clip must be idempotent for fixed bounds"
        );
    }

    /// Tightening DataFrame clip bounds in a second pass must equal clipping directly to the tighter interval.
    #[test]
    fn prop_dataframe_clip_nested_bounds_compose(
        df in arb_numeric_dataframe(8),
        outer_a in -1e6_f64..1e6_f64,
        outer_b in -1e6_f64..1e6_f64,
        inner_frac_a in 0.0f64..1.0,
        inner_frac_b in 0.0f64..1.0,
    ) {
        let (outer_lower, outer_upper, inner_lower, inner_upper) =
            nested_clip_bounds(outer_a, outer_b, inner_frac_a, inner_frac_b);
        let staged = df
            .clip(Some(outer_lower), Some(outer_upper))
            .and_then(|clipped| clipped.clip(Some(inner_lower), Some(inner_upper)))
            .expect("DataFrame::clip() composition must succeed");
        let direct = df
            .clip(Some(inner_lower), Some(inner_upper))
            .expect("DataFrame::clip() must succeed for nested bounds");
        prop_assert!(
            staged.equals(&direct),
            "dataframe clip should compose to the tighter interval"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: abs metamorphic invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Series abs is idempotent.
    #[test]
    fn prop_series_abs_is_idempotent(series in arb_numeric_series("abs", 12)) {
        let once = series.abs().expect("Series::abs() must succeed for numeric inputs");
        let twice = once.abs().expect("Series::abs() must succeed on its own output");
        prop_assert!(
            once.equals(&twice),
            "series abs must be idempotent"
        );
    }

    /// Flipping every numeric sign before abs must not change the observed result.
    #[test]
    fn prop_series_abs_is_sign_flip_invariant(series in arb_numeric_series("abs", 12)) {
        let baseline = series.abs().expect("Series::abs() must succeed for numeric inputs");
        let flipped = sign_flip_series(&series);
        let flipped_abs = flipped
            .abs()
            .expect("Series::abs() must succeed after sign flipping");
        prop_assert!(
            baseline.equals(&flipped_abs),
            "series abs must ignore numeric sign"
        );
    }

    /// DataFrame abs is idempotent.
    #[test]
    fn prop_dataframe_abs_is_idempotent(df in arb_numeric_dataframe(8)) {
        let once = df.abs().expect("DataFrame::abs() must succeed for numeric inputs");
        let twice = once
            .abs()
            .expect("DataFrame::abs() must succeed on its own output");
        prop_assert!(
            once.equals(&twice),
            "dataframe abs must be idempotent"
        );
    }

    /// Multiplying every numeric cell by -1 before abs must not change the observed result.
    #[test]
    fn prop_dataframe_abs_is_sign_flip_invariant(df in arb_numeric_dataframe(8)) {
        let baseline = df.abs().expect("DataFrame::abs() must succeed for numeric inputs");
        let flipped = df
            .mul_scalar(-1.0)
            .expect("DataFrame::mul_scalar(-1.0) must succeed for numeric inputs");
        let flipped_abs = flipped
            .abs()
            .expect("DataFrame::abs() must succeed after sign flipping");
        prop_assert!(
            baseline.equals(&flipped_abs),
            "dataframe abs must ignore numeric sign"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: combine_first metamorphic invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Series combine_first is idempotent for unique-label inputs.
    #[test]
    fn prop_series_combine_first_idempotent(series in arb_unique_numeric_series("combine_first", 12)) {
        let result = series
            .combine_first(&series)
            .expect("combine_first must succeed for self-composition");
        prop_assert!(
            result.equals(&series),
            "combine_first(self, self) must preserve the original series"
        );
    }

    /// Changes to right-side values hidden behind non-missing left values must not
    /// affect the result.
    #[test]
    fn prop_series_combine_first_ignores_poisoned_right_values(
        (left, right) in arb_unique_series_pair(12)
    ) {
        let baseline = left
            .combine_first(&right)
            .expect("combine_first must succeed for unique-label inputs");
        let poisoned_right = poison_series_right_cells_for_combine_first(&left, &right);
        let poisoned = left
            .combine_first(&poisoned_right)
            .expect("combine_first must succeed for poisoned unique-label inputs");
        prop_assert!(
            baseline.equals(&poisoned),
            "right-side changes under non-missing left values must be observationally irrelevant"
        );
    }

    /// DataFrame combine_first is idempotent for unique-label inputs.
    #[test]
    fn prop_dataframe_combine_first_idempotent(df in arb_combine_first_dataframe(8)) {
        let result = df
            .combine_first(&df)
            .expect("combine_first must succeed for self-composition");
        prop_assert!(
            result.equals(&df),
            "combine_first(self, self) must preserve the original dataframe"
        );
    }

    /// Right-side cell edits hidden behind non-missing left cells must not affect
    /// the observed dataframe result.
    #[test]
    fn prop_dataframe_combine_first_ignores_poisoned_right_values(
        (left, right) in (arb_combine_first_dataframe(8), arb_combine_first_dataframe(8))
    ) {
        let baseline = left
            .combine_first(&right)
            .expect("combine_first must succeed for unique-label dataframe inputs");
        let poisoned_right = poison_dataframe_right_cells_for_combine_first(&left, &right);
        let poisoned = left
            .combine_first(&poisoned_right)
            .expect("combine_first must succeed for poisoned dataframe inputs");
        prop_assert!(
            baseline.equals(&poisoned),
            "right-side changes under non-missing left cells must be observationally irrelevant"
        );
    }

    /// Cascading combine_first operations must compose associatively when row and
    /// column labels are unique.
    #[test]
    fn prop_dataframe_combine_first_associative(
        (left, middle, right) in (
            arb_combine_first_dataframe(6),
            arb_combine_first_dataframe(6),
            arb_combine_first_dataframe(6)
        )
    ) {
        let left_assoc = left
            .combine_first(&middle)
            .and_then(|df| df.combine_first(&right))
            .expect("left-associated combine_first chain must succeed");
        let right_assoc = middle
            .combine_first(&right)
            .and_then(|df| left.combine_first(&df))
            .expect("right-associated combine_first chain must succeed");
        prop_assert!(
            left_assoc.equals(&right_assoc),
            "combine_first chaining must be associative for unique-label dataframe inputs"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: Join invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Inner join output contains only labels present in both inputs.
    #[test]
    fn prop_inner_join_labels_in_both_inputs((left, right) in arb_series_pair(10)) {
        if let Ok(joined) = join_series(&left, &right, JoinType::Inner) {
            let left_labels = left.index().labels();
            let right_labels = right.index().labels();
            for label in joined.index.labels() {
                prop_assert!(
                    left_labels.contains(label) && right_labels.contains(label),
                    "inner join label {:?} must be in both inputs", label
                );
            }
        }
    }

    /// Left join output contains all left labels.
    #[test]
    fn prop_left_join_contains_all_left_labels((left, right) in arb_series_pair(10)) {
        if let Ok(joined) = join_series(&left, &right, JoinType::Left) {
            let joined_labels = joined.index.labels();
            for label in left.index().labels() {
                prop_assert!(
                    joined_labels.contains(label),
                    "left join must contain left label {:?}", label
                );
            }
        }
    }

    /// Join output lengths are consistent (index, left_values, right_values all same length).
    #[test]
    fn prop_join_output_lengths_consistent((left, right) in arb_series_pair(10)) {
        for join_type in [JoinType::Inner, JoinType::Left, JoinType::Right, JoinType::Outer] {
            if let Ok(joined) = join_series(&left, &right, join_type) {
                let n = joined.index.len();
                prop_assert_eq!(joined.left_values.len(), n, "left_values length mismatch for {:?}", join_type);
                prop_assert_eq!(joined.right_values.len(), n, "right_values length mismatch for {:?}", join_type);
            }
        }
    }

    /// Inner join is a subset of left join (in terms of output size).
    #[test]
    fn prop_inner_join_subset_of_left_join((left, right) in arb_series_pair(10)) {
        let inner = join_series(&left, &right, JoinType::Inner);
        let left_j = join_series(&left, &right, JoinType::Left);
        if let (Ok(inner), Ok(left_j)) = (inner, left_j) {
            prop_assert!(
                inner.index.len() <= left_j.index.len(),
                "inner join ({}) must be <= left join ({})",
                inner.index.len(), left_j.index.len()
            );
        }
    }

    /// Right join output contains all right labels.
    #[test]
    fn prop_right_join_contains_all_right_labels((left, right) in arb_series_pair(10)) {
        if let Ok(joined) = join_series(&left, &right, JoinType::Right) {
            let joined_labels = joined.index.labels();
            for label in right.index().labels() {
                prop_assert!(
                    joined_labels.contains(label),
                    "right join must contain right label {:?}", label
                );
            }
        }
    }

    /// Outer join output contains all labels from both sides.
    #[test]
    fn prop_outer_join_contains_all_labels((left, right) in arb_series_pair(10)) {
        if let Ok(joined) = join_series(&left, &right, JoinType::Outer) {
            let joined_labels = joined.index.labels();
            for label in left.index().labels() {
                prop_assert!(
                    joined_labels.contains(label),
                    "outer join must contain left label {:?}", label
                );
            }
            for label in right.index().labels() {
                prop_assert!(
                    joined_labels.contains(label),
                    "outer join must contain right label {:?}", label
                );
            }
        }
    }

    /// Inner join is a subset of outer join (in terms of output size).
    #[test]
    fn prop_inner_join_subset_of_outer_join((left, right) in arb_series_pair(10)) {
        let inner = join_series(&left, &right, JoinType::Inner);
        let outer = join_series(&left, &right, JoinType::Outer);
        if let (Ok(inner), Ok(outer)) = (inner, outer) {
            prop_assert!(
                inner.index.len() <= outer.index.len(),
                "inner join ({}) must be <= outer join ({})",
                inner.index.len(), outer.index.len()
            );
        }
    }

    /// Inner join is symmetric once left/right payload columns are swapped.
    #[test]
    fn prop_inner_join_commutative_up_to_side_swap((left, right) in arb_series_pair(10)) {
        let forward = join_series(&left, &right, JoinType::Inner)
            .expect("inner join must succeed for valid series inputs");
        let swapped = join_series(&right, &left, JoinType::Inner)
            .expect("swapped inner join must succeed for valid series inputs");

        prop_assert_eq!(
            normalized_join_rows(&forward),
            normalized_join_rows_with_swapped_sides(&swapped),
            "swapping inner-join inputs should only swap payload sides"
        );
    }

    /// Outer join is symmetric once left/right payload columns are swapped and
    /// row order is normalized.
    #[test]
    fn prop_outer_join_commutative_up_to_side_swap((left, right) in arb_series_pair(10)) {
        let forward = join_series(&left, &right, JoinType::Outer)
            .expect("outer join must succeed for valid series inputs");
        let swapped = join_series(&right, &left, JoinType::Outer)
            .expect("swapped outer join must succeed for valid series inputs");

        prop_assert_eq!(
            normalized_join_rows(&forward),
            normalized_join_rows_with_swapped_sides(&swapped),
            "swapping outer-join inputs should only swap payload sides"
        );
    }

    /// Left joins are the side-swapped dual of right joins.
    #[test]
    fn prop_left_join_equals_swapped_right_join((left, right) in arb_series_pair(10)) {
        let left_join = join_series(&left, &right, JoinType::Left)
            .expect("left join must succeed for valid series inputs");
        let swapped_right_join = join_series(&right, &left, JoinType::Right)
            .expect("swapped right join must succeed for valid series inputs");

        prop_assert_eq!(
            normalized_join_rows(&left_join),
            normalized_join_rows_with_swapped_sides(&swapped_right_join),
            "left join must match side-swapped right join"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: GroupBy invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// GroupBy sum in hardened mode never panics.
    #[test]
    fn prop_groupby_sum_hardened_no_panic((keys, values) in arb_groupby_pair(15)) {
        let policy = RuntimePolicy::hardened(Some(100_000));
        let mut ledger = EvidenceLedger::new();
        let _ = groupby_sum(&keys, &values, GroupByOptions::default(), &policy, &mut ledger);
        // Property: no panic occurred.
    }

    /// GroupBy sum result has no more groups than input rows.
    #[test]
    fn prop_groupby_sum_groups_bounded_by_input((keys, values) in arb_groupby_pair(15)) {
        let policy = RuntimePolicy::hardened(Some(100_000));
        let mut ledger = EvidenceLedger::new();
        if let Ok(result) = groupby_sum(&keys, &values, GroupByOptions::default(), &policy, &mut ledger) {
            prop_assert!(
                result.index().len() <= keys.values().len(),
                "groups ({}) must be <= input rows ({})",
                result.index().len(), keys.values().len()
            );
        }
    }

    /// GroupBy sum result index/values lengths match.
    #[test]
    fn prop_groupby_sum_index_values_length_match((keys, values) in arb_groupby_pair(15)) {
        let policy = RuntimePolicy::hardened(Some(100_000));
        let mut ledger = EvidenceLedger::new();
        if let Ok(result) = groupby_sum(&keys, &values, GroupByOptions::default(), &policy, &mut ledger) {
            prop_assert_eq!(
                result.index().len(),
                result.values().len(),
                "groupby result index/values length mismatch"
            );
        }
    }

    /// GroupBy sum with dropna=true should not have null keys in output.
    #[test]
    fn prop_groupby_sum_dropna_removes_null_keys((keys, values) in arb_groupby_pair(15)) {
        let policy = RuntimePolicy::hardened(Some(100_000));
        let mut ledger = EvidenceLedger::new();
        let opts = GroupByOptions {
            dropna: true,
            ..GroupByOptions::default()
        };
        if let Ok(result) = groupby_sum(&keys, &values, opts, &policy, &mut ledger) {
            for (i, label) in result.index().labels().iter().enumerate() {
                match label {
                    IndexLabel::Int64(_)
                    | IndexLabel::Utf8(_)
                    | IndexLabel::Timedelta64(_)
                    | IndexLabel::Datetime64(_) => {}
                }
                // All labels should be valid (non-null) when dropna=true.
                // IndexLabel doesn't have a null variant, so this is inherently satisfied
                // by the type system. But we verify the result is well-formed.
                prop_assert!(
                    result.values().len() > i,
                    "result should have value at index {}", i
                );
            }
        }
    }

    /// Renaming both indexes with the same fresh bijection must preserve GroupBy mean results.
    #[test]
    fn prop_groupby_mean_alignment_rename_invariant(
        (keys, values) in arb_groupby_pair(15),
        dropna in proptest::bool::ANY,
    ) {
        let policy = RuntimePolicy::hardened(Some(100_000));
        let baseline = {
            let mut ledger = EvidenceLedger::new();
            groupby_mean(
                &keys,
                &values,
                GroupByOptions {
                    dropna,
                    ..GroupByOptions::default()
                },
                &policy,
                &mut ledger,
            )
        };
        let (renamed_keys, renamed_values) = relabel_groupby_pair_indices(&keys, &values);
        let renamed = {
            let mut ledger = EvidenceLedger::new();
            groupby_mean(
                &renamed_keys,
                &renamed_values,
                GroupByOptions {
                    dropna,
                    ..GroupByOptions::default()
                },
                &policy,
                &mut ledger,
            )
        };

        match (baseline, renamed) {
            (Ok(original), Ok(relabeled)) => {
                prop_assert!(
                    same_series_payload(&original, &relabeled),
                    "groupby_mean should be invariant under alignment-preserving index relabeling"
                );
            }
            (Err(_), Err(_)) => {}
            (Ok(_), Err(err)) => {
                prop_assert!(false, "relabeled groupby_mean errored after baseline succeeded: {err}");
            }
            (Err(err), Ok(_)) => {
                prop_assert!(false, "baseline groupby_mean errored but relabeled succeeded: {err}");
            }
        }
    }

    /// Renaming both indexes with the same fresh bijection must preserve GroupBy count results.
    #[test]
    fn prop_groupby_count_alignment_rename_invariant(
        (keys, values) in arb_groupby_pair(15),
        dropna in proptest::bool::ANY,
    ) {
        let policy = RuntimePolicy::hardened(Some(100_000));
        let baseline = {
            let mut ledger = EvidenceLedger::new();
            groupby_count(
                &keys,
                &values,
                GroupByOptions {
                    dropna,
                    ..GroupByOptions::default()
                },
                &policy,
                &mut ledger,
            )
        };
        let (renamed_keys, renamed_values) = relabel_groupby_pair_indices(&keys, &values);
        let renamed = {
            let mut ledger = EvidenceLedger::new();
            groupby_count(
                &renamed_keys,
                &renamed_values,
                GroupByOptions {
                    dropna,
                    ..GroupByOptions::default()
                },
                &policy,
                &mut ledger,
            )
        };

        match (baseline, renamed) {
            (Ok(original), Ok(relabeled)) => {
                prop_assert!(
                    same_series_payload(&original, &relabeled),
                    "groupby_count should be invariant under alignment-preserving index relabeling"
                );
            }
            (Err(_), Err(_)) => {}
            (Ok(_), Err(err)) => {
                prop_assert!(false, "relabeled groupby_count errored after baseline succeeded: {err}");
            }
            (Err(err), Ok(_)) => {
                prop_assert!(false, "baseline groupby_count errored but relabeled succeeded: {err}");
            }
        }
    }

    /// Translating non-missing values must not change GroupBy count.
    #[test]
    fn prop_groupby_count_is_translation_invariant(
        (keys, values) in arb_groupby_pair(15),
        dropna in proptest::bool::ANY,
        delta in -1_000.0f64..1_000.0,
    ) {
        let policy = RuntimePolicy::hardened(Some(100_000));
        let baseline = {
            let mut ledger = EvidenceLedger::new();
            groupby_count(
                &keys,
                &values,
                GroupByOptions {
                    dropna,
                    ..GroupByOptions::default()
                },
                &policy,
                &mut ledger,
            )
        };
        let shifted = {
            let mut ledger = EvidenceLedger::new();
            groupby_count(
                &keys,
                &shift_series(&values, delta),
                GroupByOptions {
                    dropna,
                    ..GroupByOptions::default()
                },
                &policy,
                &mut ledger,
            )
        };

        match (baseline, shifted) {
            (Ok(original), Ok(translated)) => {
                prop_assert!(
                    same_series_payload(&original, &translated),
                    "groupby_count(x + c) must equal groupby_count(x)"
                );
            }
            (Err(_), Err(_)) => {}
            (Ok(_), Err(err)) => {
                prop_assert!(false, "translated groupby_count errored after baseline succeeded: {err}");
            }
            (Err(err), Ok(_)) => {
                prop_assert!(false, "baseline groupby_count errored but translated succeeded: {err}");
            }
        }
    }

    /// GroupBy mean must equal GroupBy sum divided by GroupBy count for non-empty groups.
    #[test]
    fn prop_groupby_mean_matches_sum_over_count(
        (keys, values) in arb_groupby_pair(15),
        dropna in proptest::bool::ANY,
    ) {
        let policy = RuntimePolicy::hardened(Some(100_000));
        let mean = {
            let mut ledger = EvidenceLedger::new();
            groupby_mean(
                &keys,
                &values,
                GroupByOptions {
                    dropna,
                    ..GroupByOptions::default()
                },
                &policy,
                &mut ledger,
            )
        };
        let sum = {
            let mut ledger = EvidenceLedger::new();
            groupby_sum(
                &keys,
                &values,
                GroupByOptions {
                    dropna,
                    ..GroupByOptions::default()
                },
                &policy,
                &mut ledger,
            )
        };
        let count = {
            let mut ledger = EvidenceLedger::new();
            groupby_count(
                &keys,
                &values,
                GroupByOptions {
                    dropna,
                    ..GroupByOptions::default()
                },
                &policy,
                &mut ledger,
            )
        };

        match (mean, sum, count) {
            (Ok(observed_mean), Ok(observed_sum), Ok(observed_count)) => {
                prop_assert_eq!(
                    observed_mean.index().labels(),
                    observed_sum.index().labels(),
                    "groupby mean/sum indexes must align"
                );
                prop_assert_eq!(
                    observed_mean.index().labels(),
                    observed_count.index().labels(),
                    "groupby mean/count indexes must align"
                );
                let reconstructed = reconstruct_groupby_mean_from_sum_and_count(
                    &observed_sum,
                    &observed_count,
                );
                prop_assert!(
                    same_series_payload(&observed_mean, &reconstructed),
                    "groupby_mean must match groupby_sum/groupby_count reconstruction"
                );
            }
            (Err(_), Err(_), Err(_)) => {}
            (mean_result, sum_result, count_result) => {
                prop_assert!(
                    false,
                    "groupby mean/sum/count disagree on success status: mean={mean_result:?} sum={sum_result:?} count={count_result:?}"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Property: Scalar type coercion invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    /// Scalar semantic_eq is reflexive.
    #[test]
    fn prop_scalar_semantic_eq_reflexive(scalar in arb_numeric_scalar()) {
        prop_assert!(scalar.semantic_eq(&scalar), "semantic_eq must be reflexive");
    }

    /// Scalar semantic_eq is symmetric.
    #[test]
    fn prop_scalar_semantic_eq_symmetric(
        a in arb_numeric_scalar(),
        b in arb_numeric_scalar(),
    ) {
        prop_assert_eq!(
            a.semantic_eq(&b),
            b.semantic_eq(&a),
            "semantic_eq must be symmetric: {:?} vs {:?}", a, b
        );
    }

    /// Missing scalars are detected by is_missing().
    #[test]
    fn prop_null_scalars_are_missing(kind in prop_oneof![
        Just(NullKind::Null),
        Just(NullKind::NaN),
        Just(NullKind::NaT),
    ]) {
        let scalar = Scalar::Null(kind);
        prop_assert!(scalar.is_missing(), "Null({:?}) must be missing", kind);
    }

    /// Non-null, non-NaN scalars are not missing.
    #[test]
    fn prop_concrete_scalars_not_missing(scalar in prop_oneof![
        any::<bool>().prop_map(Scalar::Bool),
        (-1_000_000i64..1_000_000i64).prop_map(Scalar::Int64),
        (-1e6_f64..1e6_f64).prop_filter("not NaN", |v| !v.is_nan()).prop_map(Scalar::Float64),
    ]) {
        prop_assert!(!scalar.is_missing(), "{:?} must not be missing", scalar);
    }

    /// Float64(NaN) is always detected as missing.
    #[test]
    fn prop_float64_nan_is_missing(_dummy in 0..1u8) {
        let nan = Scalar::Float64(f64::NAN);
        prop_assert!(nan.is_missing());
        prop_assert!(nan.is_nan());
    }
}

// ---------------------------------------------------------------------------
// Property: Serialization round-trip
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Scalars survive JSON serialization round-trip. For Float64, JSON
    /// text serialization can lose the last bit of precision, so we compare
    /// with a small tolerance instead of exact semantic_eq.
    #[test]
    fn prop_scalar_json_round_trip(scalar in arb_numeric_scalar()) {
        let json = serde_json::to_string(&scalar).expect("serialize");
        let back: Scalar = serde_json::from_str(&json).expect("deserialize");
        match (&scalar, &back) {
            (Scalar::Float64(a), Scalar::Float64(b)) => {
                if a.is_nan() && b.is_nan() {
                    // Both NaN: ok
                } else {
                    let diff = (a - b).abs();
                    let tol = a.abs().max(1.0) * 1e-15;
                    prop_assert!(diff <= tol,
                        "float round-trip drift: {} -> {} (diff={})", a, b, diff);
                }
            }
            _ => {
                prop_assert!(
                    scalar.semantic_eq(&back),
                    "round-trip failed: {:?} -> {} -> {:?}", scalar, json, back
                );
            }
        }
    }

    /// IndexLabel survives JSON serialization round-trip.
    #[test]
    fn prop_index_label_json_round_trip(label in arb_index_label()) {
        let json = serde_json::to_string(&label).expect("serialize");
        let back: IndexLabel = serde_json::from_str(&json).expect("deserialize");
        prop_assert_eq!(label, back);
    }

    // === Packed Bitvec ValidityMask Property Tests (bd-2t5e.4.1) ===

    /// Packing bools into u64 words then unpacking via bits() produces identical values.
    #[test]
    fn prop_bitvec_roundtrip(bools in proptest::collection::vec(proptest::bool::ANY, 0..512)) {
        let values: Vec<Scalar> = bools.iter().map(|&b| {
            if b { Scalar::Int64(1) } else { Scalar::Null(fp_types::NullKind::Null) }
        }).collect();
        let mask = fp_columnar::ValidityMask::from_values(&values);
        let unpacked: Vec<bool> = mask.bits().collect();
        prop_assert_eq!(bools, unpacked);
    }

    /// and_mask is commutative: a AND b == b AND a.
    #[test]
    fn prop_bitvec_and_commutative(
        bools_a in proptest::collection::vec(proptest::bool::ANY, 0..256),
        bools_b in proptest::collection::vec(proptest::bool::ANY, 0..256),
    ) {
        let len = bools_a.len().min(bools_b.len());
        let vals_a: Vec<Scalar> = bools_a[..len].iter().map(|&b| {
            if b { Scalar::Int64(1) } else { Scalar::Null(fp_types::NullKind::Null) }
        }).collect();
        let vals_b: Vec<Scalar> = bools_b[..len].iter().map(|&b| {
            if b { Scalar::Int64(1) } else { Scalar::Null(fp_types::NullKind::Null) }
        }).collect();
        let a = fp_columnar::ValidityMask::from_values(&vals_a);
        let b = fp_columnar::ValidityMask::from_values(&vals_b);
        let ab: Vec<bool> = a.and_mask(&b).bits().collect();
        let ba: Vec<bool> = b.and_mask(&a).bits().collect();
        prop_assert_eq!(ab, ba);
    }

    /// count_valid() matches the count from iterating bits().
    #[test]
    fn prop_bitvec_count_matches_iter(bools in proptest::collection::vec(proptest::bool::ANY, 0..512)) {
        let values: Vec<Scalar> = bools.iter().map(|&b| {
            if b { Scalar::Int64(1) } else { Scalar::Null(fp_types::NullKind::Null) }
        }).collect();
        let mask = fp_columnar::ValidityMask::from_values(&values);
        let iter_count = mask.bits().filter(|b| *b).count();
        prop_assert_eq!(mask.count_valid(), iter_count);
    }
}

// ---------------------------------------------------------------------------
// Property: AG-06 Arena/Region Memory Isomorphism (bd-2t5e.6.1)
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Arena-backed join produces identical output to global-allocator join
    /// for arbitrary series pairs and both join types.
    #[test]
    fn prop_arena_join_isomorphic_to_global(
        (left, right) in arb_series_pair(15),
        join_type in prop_oneof![Just(JoinType::Inner), Just(JoinType::Left), Just(JoinType::Right), Just(JoinType::Outer)],
    ) {
        let global = join_series_with_options(
            &left, &right, join_type,
            JoinExecutionOptions { use_arena: false, arena_budget_bytes: 0 },
        );
        let arena = join_series_with_options(
            &left, &right, join_type,
            JoinExecutionOptions::default(),
        );

        match (global, arena) {
            (Ok(g), Ok(a)) => {
                prop_assert_eq!(
                    g.index.labels(), a.index.labels(),
                    "arena join index must match global"
                );
                prop_assert_eq!(
                    g.left_values.len(), a.left_values.len(),
                    "arena join left_values length must match"
                );
                prop_assert_eq!(
                    g.right_values.len(), a.right_values.len(),
                    "arena join right_values length must match"
                );
            }
            (Err(_), Err(_)) => { /* both error: ok */ }
            (Ok(_), Err(e)) => {
                prop_assert!(false, "arena errored but global succeeded: {e}");
            }
            (Err(e), Ok(_)) => {
                prop_assert!(false, "global errored but arena succeeded: {e}");
            }
        }
    }

    /// Arena-backed groupby_sum produces identical output to global-allocator
    /// groupby_sum for arbitrary key/value pairs.
    #[test]
    fn prop_arena_groupby_isomorphic_to_global(
        (keys, values) in arb_groupby_pair(15),
        dropna in proptest::bool::ANY,
    ) {
        let policy = RuntimePolicy::hardened(Some(100_000));
        let mut ledger = EvidenceLedger::new();
        let opts = GroupByOptions {
            dropna,
            ..GroupByOptions::default()
        };

        let global = groupby_sum_with_options(
            &keys, &values, opts, &policy, &mut ledger,
            GroupByExecutionOptions { use_arena: false, arena_budget_bytes: 0 },
        );
        let arena = groupby_sum_with_options(
            &keys, &values, opts, &policy, &mut ledger,
            GroupByExecutionOptions::default(),
        );

        match (global, arena) {
            (Ok(g), Ok(a)) => {
                prop_assert_eq!(
                    g.index().labels(), a.index().labels(),
                    "arena groupby index must match global"
                );
                prop_assert_eq!(
                    g.values(), a.values(),
                    "arena groupby values must match global"
                );
            }
            (Err(_), Err(_)) => { /* both error: ok */ }
            (Ok(_), Err(e)) => {
                prop_assert!(false, "arena errored but global succeeded: {e}");
            }
            (Err(e), Ok(_)) => {
                prop_assert!(false, "global errored but arena succeeded: {e}");
            }
        }
    }

    /// Join with a tiny arena budget falls back to global allocator and still
    /// produces correct results.
    #[test]
    fn prop_arena_join_fallback_correct(
        (left, right) in arb_series_pair(15),
        join_type in prop_oneof![Just(JoinType::Inner), Just(JoinType::Left), Just(JoinType::Right), Just(JoinType::Outer)],
    ) {
        let fallback = join_series_with_options(
            &left, &right, join_type,
            JoinExecutionOptions { use_arena: true, arena_budget_bytes: 1 },
        );
        let global = join_series_with_options(
            &left, &right, join_type,
            JoinExecutionOptions { use_arena: false, arena_budget_bytes: 0 },
        );

        match (fallback, global) {
            (Ok(f), Ok(g)) => {
                prop_assert_eq!(f.index.labels(), g.index.labels());
                prop_assert_eq!(f.left_values.len(), g.left_values.len());
                prop_assert_eq!(f.right_values.len(), g.right_values.len());
            }
            (Err(_), Err(_)) => {}
            _ => prop_assert!(false, "fallback/global mismatch in error status"),
        }
    }

    /// Groupby with a tiny arena budget falls back and still produces
    /// correct results.
    #[test]
    fn prop_arena_groupby_fallback_correct(
        (keys, values) in arb_groupby_pair(15),
    ) {
        let policy = RuntimePolicy::hardened(Some(100_000));
        let mut ledger = EvidenceLedger::new();
        let opts = GroupByOptions::default();

        let fallback = groupby_sum_with_options(
            &keys, &values, opts, &policy, &mut ledger,
            GroupByExecutionOptions { use_arena: true, arena_budget_bytes: 1 },
        );
        let global = groupby_sum_with_options(
            &keys, &values, opts, &policy, &mut ledger,
            GroupByExecutionOptions { use_arena: false, arena_budget_bytes: 0 },
        );

        match (fallback, global) {
            (Ok(f), Ok(g)) => {
                prop_assert_eq!(f.index().labels(), g.index().labels());
                prop_assert_eq!(f.values(), g.values());
            }
            (Err(_), Err(_)) => {}
            _ => prop_assert!(false, "fallback/global mismatch in error status"),
        }
    }
}

// ---------------------------------------------------------------------------
// Property: DType coercion invariants (frankenpandas-x2n)
// ---------------------------------------------------------------------------

/// Generate an arbitrary DType.
fn arb_dtype() -> impl Strategy<Value = fp_types::DType> {
    prop_oneof![
        Just(fp_types::DType::Null),
        Just(fp_types::DType::Bool),
        Just(fp_types::DType::Int64),
        Just(fp_types::DType::Float64),
        Just(fp_types::DType::Utf8),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    /// common_dtype is symmetric: common_dtype(a, b) == common_dtype(b, a).
    #[test]
    fn prop_common_dtype_symmetric(a in arb_dtype(), b in arb_dtype()) {
        let ab = fp_types::common_dtype(a, b);
        let ba = fp_types::common_dtype(b, a);
        match (ab, ba) {
            (Ok(ab_dt), Ok(ba_dt)) => {
                prop_assert_eq!(ab_dt, ba_dt,
                    "common_dtype must be symmetric: ({:?},{:?}) -> {:?} vs {:?}", a, b, ab_dt, ba_dt);
            }
            (Err(_), Err(_)) => { /* both incompatible: ok */ }
            _ => {
                prop_assert!(false,
                    "common_dtype symmetry broken: ({:?},{:?}) one Ok, one Err", a, b);
            }
        }
    }

    /// common_dtype is reflexive: common_dtype(a, a) == Ok(a).
    #[test]
    fn prop_common_dtype_reflexive(a in arb_dtype()) {
        let result = fp_types::common_dtype(a, a);
        prop_assert_eq!(result, Ok(a), "common_dtype({:?}, {:?}) must be {:?}", a, a, a);
    }

    /// common_dtype with Null is identity: common_dtype(Null, x) == Ok(x).
    #[test]
    fn prop_common_dtype_null_identity(x in arb_dtype()) {
        let result = fp_types::common_dtype(fp_types::DType::Null, x);
        prop_assert_eq!(result, Ok(x), "Null is the identity element for common_dtype");
    }

    /// common_dtype is transitive for compatible triples:
    /// If common_dtype(a, b) = Ok(ab) and common_dtype(ab, c) = Ok(abc),
    /// then common_dtype(a, common_dtype(b, c)) should also be Ok(abc).
    #[test]
    fn prop_common_dtype_transitive(a in arb_dtype(), b in arb_dtype(), c in arb_dtype()) {
        let ab = fp_types::common_dtype(a, b);
        let bc = fp_types::common_dtype(b, c);

        // Only test transitivity when both intermediate steps succeed.
        if let (Ok(ab_dt), Ok(bc_dt)) = (ab, bc) {
            let ab_c = fp_types::common_dtype(ab_dt, c);
            let a_bc = fp_types::common_dtype(a, bc_dt);

            match (ab_c, a_bc) {
                (Ok(left), Ok(right)) => {
                    prop_assert_eq!(left, right,
                        "common_dtype transitivity: ({:?},{:?},{:?}) -> {:?} vs {:?}",
                        a, b, c, left, right);
                }
                (Err(_), Err(_)) => { /* both fail: ok */ }
                _ => {
                    prop_assert!(false,
                        "common_dtype transitivity inconsistency for ({:?},{:?},{:?})", a, b, c);
                }
            }
        }
    }

    /// infer_dtype is consistent with common_dtype: infer_dtype on a
    /// homogeneous slice returns the dtype of the elements.
    #[test]
    fn prop_infer_dtype_homogeneous(scalar in arb_numeric_scalar()) {
        if scalar.is_missing() {
            return Ok(());
        }
        let values = vec![scalar.clone(), scalar.clone()];
        let inferred = fp_types::infer_dtype(&values);
        prop_assert_eq!(inferred, Ok(scalar.dtype()),
            "infer_dtype on homogeneous slice should return element dtype");
    }
}

// ---------------------------------------------------------------------------
// Property: Scalar cast safety (frankenpandas-x2n)
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    /// Casting a scalar to its own dtype always succeeds and returns
    /// a semantically equal value (identity cast, AG-03).
    /// Note: Null scalars normalize to the canonical missing form for DType::Null
    /// (i.e., Null(NaN) -> Null(Null)), which is correct behavior.
    #[test]
    fn prop_cast_identity(scalar in arb_numeric_scalar()) {
        let target = scalar.dtype();
        let result = fp_types::cast_scalar(&scalar, target);
        match result {
            Ok(casted) => {
                if scalar.is_missing() {
                    // Missing values normalize to canonical missing for the dtype.
                    prop_assert!(casted.is_missing(),
                        "identity cast of missing must produce missing: {:?} -> {:?}", scalar, casted);
                } else {
                    prop_assert!(casted.semantic_eq(&scalar),
                        "identity cast must preserve value: {:?} -> {:?}", scalar, casted);
                }
            }
            Err(e) => {
                prop_assert!(false, "identity cast must not fail: {:?} -> {:?}", scalar, e);
            }
        }
    }

    /// Casting a missing scalar to any dtype produces a missing scalar.
    #[test]
    fn prop_cast_missing_stays_missing(
        kind in prop_oneof![Just(NullKind::Null), Just(NullKind::NaN), Just(NullKind::NaT)],
        target in arb_dtype(),
    ) {
        let scalar = Scalar::Null(kind);
        let result = fp_types::cast_scalar(&scalar, target);
        match result {
            Ok(casted) => {
                prop_assert!(casted.is_missing(),
                    "casting Null({:?}) to {:?} must produce missing, got {:?}", kind, target, casted);
            }
            Err(_) => {
                // Some casts may legitimately fail (e.g., NaT to Utf8 in some configs).
                // That's acceptable; the important thing is it doesn't panic.
            }
        }
    }

    /// Casting Int64 to Float64 never loses the integer value (within i64 range
    /// that fits exactly in f64).
    #[test]
    fn prop_cast_int64_to_float64_preserves(v in -1_000_000_000i64..1_000_000_000i64) {
        let scalar = Scalar::Int64(v);
        let result = fp_types::cast_scalar(&scalar, fp_types::DType::Float64);
        match result {
            Ok(Scalar::Float64(fv)) => {
                prop_assert_eq!(fv as i64, v,
                    "Int64->Float64 must preserve: {} -> {}", v, fv);
            }
            other => {
                prop_assert!(false, "unexpected cast result: {:?}", other);
            }
        }
    }

    /// Casting Bool to Int64 produces 0 or 1.
    #[test]
    fn prop_cast_bool_to_int64(b in proptest::bool::ANY) {
        let scalar = Scalar::Bool(b);
        let result = fp_types::cast_scalar(&scalar, fp_types::DType::Int64);
        let expected = if b { 1i64 } else { 0i64 };
        prop_assert_eq!(result, Ok(Scalar::Int64(expected)));
    }

    /// Casting compatible types via common_dtype then cast_scalar round-trips:
    /// if common_dtype(a.dtype(), b.dtype()) = Ok(target), then
    /// cast_scalar(a, target) and cast_scalar(b, target) both succeed.
    #[test]
    fn prop_cast_to_common_dtype_succeeds(
        a in arb_numeric_scalar(),
        b in arb_numeric_scalar(),
    ) {
        let dt_a = a.dtype();
        let dt_b = b.dtype();
        if let Ok(target) = fp_types::common_dtype(dt_a, dt_b) {
            let cast_a = fp_types::cast_scalar(&a, target);
            let cast_b = fp_types::cast_scalar(&b, target);
            prop_assert!(cast_a.is_ok(),
                "cast {:?} ({:?}) to {:?} must succeed", a, dt_a, target);
            prop_assert!(cast_b.is_ok(),
                "cast {:?} ({:?}) to {:?} must succeed", b, dt_b, target);
        }
    }
}

// ---------------------------------------------------------------------------
// Property: CSV round-trip invariants (frankenpandas-x2n)
// ---------------------------------------------------------------------------

/// Generate a DataFrame-safe string that CANNOT be parsed as a number or bool.
/// This ensures CSV round-trip doesn't change Utf8 -> Int64/Float64/Bool.
/// The mandatory underscore after the first two letters prevents generating
/// "true" or "false" (which would parse as Bool).
fn arb_safe_string() -> impl Strategy<Value = String> {
    "[a-z]{2}_[a-z0-9]{0,7}"
}

/// Generate a column name (valid, non-empty, no special characters).
fn arb_column_name() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9_]{0,5}"
}

/// Generate a homogeneous column of CSV-safe scalars (all same type per column).
/// This is required because CSV round-trip does per-cell type inference,
/// so mixed-type columns would break on re-parse.
fn arb_csv_column(nrows: usize) -> impl Strategy<Value = Vec<Scalar>> {
    prop_oneof![
        3 => proptest::collection::vec(
            (-1_000_000i64..1_000_000i64).prop_map(Scalar::Int64), nrows
        ),
        2 => proptest::collection::vec(
            arb_safe_string().prop_map(Scalar::Utf8), nrows
        ),
    ]
}

/// Generate a small DataFrame with N rows and M columns of CSV-safe scalars.
/// Each column is homogeneous (all same type) for CSV round-trip safety.
fn arb_csv_dataframe(
    max_rows: usize,
    max_cols: usize,
) -> impl Strategy<Value = fp_frame::DataFrame> {
    (1..=max_rows, 1..=max_cols).prop_flat_map(|(nrows, ncols)| {
        let col_names = proptest::collection::vec(arb_column_name(), ncols);
        let columns = proptest::collection::vec(arb_csv_column(nrows), ncols);
        (col_names, columns).prop_filter_map(
            "dataframe construction must succeed",
            move |(names, cols)| {
                // Ensure unique column names
                let mut seen = std::collections::HashSet::new();
                let mut unique_names = Vec::new();
                for name in &names {
                    let mut candidate = name.clone();
                    let mut suffix = 0;
                    while seen.contains(&candidate) {
                        suffix += 1;
                        candidate = format!("{name}{suffix}");
                    }
                    seen.insert(candidate.clone());
                    unique_names.push(candidate);
                }

                let col_order: Vec<&str> = unique_names.iter().map(String::as_str).collect();
                let data: Vec<(&str, Vec<Scalar>)> = unique_names
                    .iter()
                    .zip(cols.iter())
                    .map(|(n, v)| (n.as_str(), v.clone()))
                    .collect();

                fp_frame::DataFrame::from_dict(&col_order, data).ok()
            },
        )
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// CSV round-trip preserves DataFrame shape (rows x columns).
    #[test]
    fn prop_csv_round_trip_preserves_shape(df in arb_csv_dataframe(8, 4)) {
        let csv_text = fp_io::write_csv_string(&df);
        prop_assert!(csv_text.is_ok(), "CSV write must succeed");
        let csv_text = csv_text.unwrap();

        let parsed = fp_io::read_csv_str(&csv_text);
        prop_assert!(parsed.is_ok(), "CSV parse must succeed: {:?}", parsed.err());
        let parsed = parsed.unwrap();

        prop_assert_eq!(
            parsed.index().len(), df.index().len(),
            "CSV round-trip must preserve row count"
        );
        prop_assert_eq!(
            parsed.column_names().len(), df.column_names().len(),
            "CSV round-trip must preserve column count"
        );
    }

    /// CSV round-trip preserves column names (order may be preserved by BTreeMap).
    #[test]
    fn prop_csv_round_trip_preserves_column_names(df in arb_csv_dataframe(5, 3)) {
        let csv_text = fp_io::write_csv_string(&df).unwrap();
        let parsed = fp_io::read_csv_str(&csv_text).unwrap();

        let orig_names: Vec<&String> = df.column_names();
        let parsed_names: Vec<&String> = parsed.column_names();
        prop_assert_eq!(orig_names, parsed_names, "column names must survive CSV round-trip");
    }

    /// CSV round-trip for Int64-only DataFrames preserves values exactly.
    #[test]
    fn prop_csv_round_trip_int64_exact(
        values in proptest::collection::vec(-100_000i64..100_000i64, 1..10),
        col_name in arb_column_name(),
    ) {
        let scalars: Vec<Scalar> = values.iter().map(|&v| Scalar::Int64(v)).collect();
        let df = fp_frame::DataFrame::from_dict(
            &[col_name.as_str()],
            vec![(col_name.as_str(), scalars.clone())],
        );
        prop_assert!(df.is_ok());
        let df = df.unwrap();

        let csv_text = fp_io::write_csv_string(&df).unwrap();
        let parsed = fp_io::read_csv_str(&csv_text).unwrap();

        let orig_col = df.column(&col_name).unwrap();
        let parsed_col = parsed.column(&col_name).unwrap();
        prop_assert_eq!(
            orig_col.values(), parsed_col.values(),
            "Int64 values must survive CSV round-trip exactly"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: JSON round-trip invariants (frankenpandas-x2n)
// ---------------------------------------------------------------------------

/// Generate a small DataFrame suitable for JSON round-trip testing.
/// Uses only non-null scalars since JSON null handling varies by orient.
fn arb_json_dataframe(
    max_rows: usize,
    max_cols: usize,
) -> impl Strategy<Value = fp_frame::DataFrame> {
    (1..=max_rows, 1..=max_cols).prop_flat_map(|(nrows, ncols)| {
        let col_names = proptest::collection::vec(arb_column_name(), ncols);
        let columns = proptest::collection::vec(
            proptest::collection::vec(
                prop_oneof![
                    3 => (-100_000i64..100_000i64).prop_map(Scalar::Int64),
                    2 => arb_safe_string().prop_map(Scalar::Utf8),
                ],
                nrows,
            ),
            ncols,
        );
        (col_names, columns).prop_filter_map(
            "json dataframe must construct",
            move |(names, cols)| {
                let mut seen = std::collections::HashSet::new();
                let mut unique_names = Vec::new();
                for name in &names {
                    let mut candidate = name.clone();
                    let mut suffix = 0;
                    while seen.contains(&candidate) {
                        suffix += 1;
                        candidate = format!("{name}{suffix}");
                    }
                    seen.insert(candidate.clone());
                    unique_names.push(candidate);
                }

                // Ensure homogeneous columns (all same type per column)
                // so JSON round-trip doesn't lose type info.
                let mut homo_cols = Vec::new();
                for col in &cols {
                    if col.is_empty() {
                        homo_cols.push(col.clone());
                        continue;
                    }
                    let first_dtype = col[0].dtype();
                    if col.iter().all(|s| s.dtype() == first_dtype) {
                        homo_cols.push(col.clone());
                    } else {
                        // Force all to Int64 for homogeneity
                        homo_cols.push(
                            col.iter()
                                .map(|s| match s {
                                    Scalar::Int64(v) => Scalar::Int64(*v),
                                    _ => Scalar::Int64(0),
                                })
                                .collect(),
                        );
                    }
                }

                let col_order: Vec<&str> = unique_names.iter().map(String::as_str).collect();
                let data: Vec<(&str, Vec<Scalar>)> = unique_names
                    .iter()
                    .zip(homo_cols.iter())
                    .map(|(n, v)| (n.as_str(), v.clone()))
                    .collect();

                fp_frame::DataFrame::from_dict(&col_order, data).ok()
            },
        )
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]

    /// JSON Records orient round-trip preserves shape.
    #[test]
    fn prop_json_records_round_trip_shape(df in arb_json_dataframe(5, 3)) {
        let json = fp_io::write_json_string(&df, fp_io::JsonOrient::Records);
        prop_assert!(json.is_ok(), "JSON Records write must succeed");
        let json = json.unwrap();

        let parsed = fp_io::read_json_str(&json, fp_io::JsonOrient::Records);
        prop_assert!(parsed.is_ok(), "JSON Records parse must succeed: {:?}", parsed.err());
        let parsed = parsed.unwrap();

        prop_assert_eq!(parsed.index().len(), df.index().len(),
            "Records round-trip must preserve row count");
        prop_assert_eq!(parsed.column_names().len(), df.column_names().len(),
            "Records round-trip must preserve column count");
    }

    /// JSON Columns orient round-trip preserves shape.
    #[test]
    fn prop_json_columns_round_trip_shape(df in arb_json_dataframe(5, 3)) {
        let json = fp_io::write_json_string(&df, fp_io::JsonOrient::Columns);
        prop_assert!(json.is_ok(), "JSON Columns write must succeed");
        let json = json.unwrap();

        let parsed = fp_io::read_json_str(&json, fp_io::JsonOrient::Columns);
        prop_assert!(parsed.is_ok(), "JSON Columns parse must succeed: {:?}", parsed.err());
        let parsed = parsed.unwrap();

        prop_assert_eq!(parsed.index().len(), df.index().len(),
            "Columns round-trip must preserve row count");
        prop_assert_eq!(parsed.column_names().len(), df.column_names().len(),
            "Columns round-trip must preserve column count");
    }

    /// JSON Split orient round-trip preserves shape.
    #[test]
    fn prop_json_split_round_trip_shape(df in arb_json_dataframe(5, 3)) {
        let json = fp_io::write_json_string(&df, fp_io::JsonOrient::Split);
        prop_assert!(json.is_ok(), "JSON Split write must succeed");
        let json = json.unwrap();

        let parsed = fp_io::read_json_str(&json, fp_io::JsonOrient::Split);
        prop_assert!(parsed.is_ok(), "JSON Split parse must succeed: {:?}", parsed.err());
        let parsed = parsed.unwrap();

        prop_assert_eq!(parsed.index().len(), df.index().len(),
            "Split round-trip must preserve row count");
        prop_assert_eq!(parsed.column_names().len(), df.column_names().len(),
            "Split round-trip must preserve column count");
    }

    /// JSON Values orient round-trip preserves row count.
    /// (Values orient loses column names, so we only check shape.)
    #[test]
    fn prop_json_values_round_trip_row_count(df in arb_json_dataframe(5, 3)) {
        let json = fp_io::write_json_string(&df, fp_io::JsonOrient::Values);
        prop_assert!(json.is_ok(), "JSON Values write must succeed");
        let json = json.unwrap();

        let parsed = fp_io::read_json_str(&json, fp_io::JsonOrient::Values);
        prop_assert!(parsed.is_ok(), "JSON Values parse must succeed: {:?}", parsed.err());
        let parsed = parsed.unwrap();

        prop_assert_eq!(parsed.index().len(), df.index().len(),
            "Values round-trip must preserve row count");
    }

    /// JSON Records orient round-trip preserves column names.
    #[test]
    fn prop_json_records_round_trip_column_names(df in arb_json_dataframe(3, 4)) {
        let json = fp_io::write_json_string(&df, fp_io::JsonOrient::Records).unwrap();
        let parsed = fp_io::read_json_str(&json, fp_io::JsonOrient::Records).unwrap();

        let mut orig: Vec<String> = df.column_names().into_iter().cloned().collect();
        let mut parsed_names: Vec<String> = parsed.column_names().into_iter().cloned().collect();
        orig.sort();
        parsed_names.sort();
        prop_assert_eq!(orig, parsed_names, "column names must survive JSON Records round-trip");
    }
}

// ---------------------------------------------------------------------------
// Property: ValidityMask algebra invariants (frankenpandas-x2n)
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    /// not_mask is involutory: not(not(mask)) == mask.
    #[test]
    fn prop_bitvec_not_involution(bools in proptest::collection::vec(proptest::bool::ANY, 0..256)) {
        let values: Vec<Scalar> = bools.iter().map(|&b| {
            if b { Scalar::Int64(1) } else { Scalar::Null(fp_types::NullKind::Null) }
        }).collect();
        let mask = fp_columnar::ValidityMask::from_values(&values);
        let double_not: Vec<bool> = mask.not_mask().not_mask().bits().collect();
        let original: Vec<bool> = mask.bits().collect();
        prop_assert_eq!(original, double_not, "not(not(mask)) must equal mask");
    }

    /// or_mask is commutative: a OR b == b OR a.
    #[test]
    fn prop_bitvec_or_commutative(
        bools_a in proptest::collection::vec(proptest::bool::ANY, 0..256),
        bools_b in proptest::collection::vec(proptest::bool::ANY, 0..256),
    ) {
        let len = bools_a.len().min(bools_b.len());
        let vals_a: Vec<Scalar> = bools_a[..len].iter().map(|&b| {
            if b { Scalar::Int64(1) } else { Scalar::Null(fp_types::NullKind::Null) }
        }).collect();
        let vals_b: Vec<Scalar> = bools_b[..len].iter().map(|&b| {
            if b { Scalar::Int64(1) } else { Scalar::Null(fp_types::NullKind::Null) }
        }).collect();
        let a = fp_columnar::ValidityMask::from_values(&vals_a);
        let b = fp_columnar::ValidityMask::from_values(&vals_b);
        let ab: Vec<bool> = a.or_mask(&b).bits().collect();
        let ba: Vec<bool> = b.or_mask(&a).bits().collect();
        prop_assert_eq!(ab, ba, "or_mask must be commutative");
    }

    /// De Morgan's law: not(a AND b) == not(a) OR not(b).
    #[test]
    fn prop_bitvec_de_morgan(
        bools_a in proptest::collection::vec(proptest::bool::ANY, 1..128),
        bools_b in proptest::collection::vec(proptest::bool::ANY, 1..128),
    ) {
        let len = bools_a.len().min(bools_b.len());
        let vals_a: Vec<Scalar> = bools_a[..len].iter().map(|&b| {
            if b { Scalar::Int64(1) } else { Scalar::Null(fp_types::NullKind::Null) }
        }).collect();
        let vals_b: Vec<Scalar> = bools_b[..len].iter().map(|&b| {
            if b { Scalar::Int64(1) } else { Scalar::Null(fp_types::NullKind::Null) }
        }).collect();
        let a = fp_columnar::ValidityMask::from_values(&vals_a);
        let b = fp_columnar::ValidityMask::from_values(&vals_b);

        let not_and: Vec<bool> = a.and_mask(&b).not_mask().bits().collect();
        let or_not: Vec<bool> = a.not_mask().or_mask(&b.not_mask()).bits().collect();
        prop_assert_eq!(not_and, or_not, "De Morgan: not(a AND b) == not(a) OR not(b)");
    }
}

// ---------------------------------------------------------------------------
// Property: DataFrame arithmetic invariants (frankenpandas-s6d)
// ---------------------------------------------------------------------------

/// Generate a small DataFrame with numeric columns for arithmetic testing.
fn arb_numeric_dataframe(max_rows: usize) -> impl Strategy<Value = DataFrame> {
    (1..=max_rows).prop_flat_map(|nrows| {
        let idx_labels = arb_index_labels(nrows);
        let col_a = arb_numeric_values(nrows);
        let col_b = arb_numeric_values(nrows);
        (idx_labels, col_a, col_b).prop_filter_map(
            "dataframe construction must succeed",
            move |(labels, va, vb)| {
                let index = Index::new(labels);
                let col_a = fp_columnar::Column::from_values(va).ok()?;
                let col_b = fp_columnar::Column::from_values(vb).ok()?;
                let mut cols = std::collections::BTreeMap::new();
                cols.insert("a".to_string(), col_a);
                cols.insert("b".to_string(), col_b);
                DataFrame::new_with_column_order(
                    index,
                    cols,
                    vec!["a".to_string(), "b".to_string()],
                )
                .ok()
            },
        )
    })
}

fn arb_aligned_where_series_triplet(
    max_len: usize,
) -> impl Strategy<Value = (Series, Series, Series)> {
    (1..=max_len).prop_flat_map(|len| {
        (
            arb_index_labels(len),
            arb_numeric_values(len),
            arb_condition_values(len),
            arb_numeric_values(len),
        )
            .prop_filter_map(
                "aligned where-series construction must succeed",
                |(labels, data_values, cond_values, other_values)| {
                    let data =
                        Series::from_values("data".to_owned(), labels.clone(), data_values).ok()?;
                    let cond =
                        Series::from_values("cond".to_owned(), labels.clone(), cond_values).ok()?;
                    let other =
                        Series::from_values("other".to_owned(), labels, other_values).ok()?;
                    Some((data, cond, other))
                },
            )
    })
}

fn arb_aligned_where_dataframe_triplet(
    max_rows: usize,
) -> impl Strategy<Value = (DataFrame, DataFrame, DataFrame)> {
    (1..=max_rows).prop_flat_map(|nrows| {
        (
            arb_index_labels(nrows),
            arb_numeric_values(nrows),
            arb_numeric_values(nrows),
            arb_condition_values(nrows),
            arb_condition_values(nrows),
            arb_numeric_values(nrows),
            arb_numeric_values(nrows),
        )
            .prop_filter_map(
                "aligned where-dataframe construction must succeed",
                |(labels, data_a, data_b, cond_a, cond_b, other_a, other_b)| {
                    let index = Index::new(labels);
                    let mut data_cols = std::collections::BTreeMap::new();
                    data_cols.insert(
                        "a".to_string(),
                        fp_columnar::Column::from_values(data_a).ok()?,
                    );
                    data_cols.insert(
                        "b".to_string(),
                        fp_columnar::Column::from_values(data_b).ok()?,
                    );
                    let data = DataFrame::new_with_column_order(
                        index.clone(),
                        data_cols,
                        vec!["a".to_string(), "b".to_string()],
                    )
                    .ok()?;

                    let mut cond_cols = std::collections::BTreeMap::new();
                    cond_cols.insert(
                        "a".to_string(),
                        fp_columnar::Column::from_values(cond_a).ok()?,
                    );
                    cond_cols.insert(
                        "b".to_string(),
                        fp_columnar::Column::from_values(cond_b).ok()?,
                    );
                    let cond = DataFrame::new_with_column_order(
                        index.clone(),
                        cond_cols,
                        vec!["a".to_string(), "b".to_string()],
                    )
                    .ok()?;

                    let mut other_cols = std::collections::BTreeMap::new();
                    other_cols.insert(
                        "a".to_string(),
                        fp_columnar::Column::from_values(other_a).ok()?,
                    );
                    other_cols.insert(
                        "b".to_string(),
                        fp_columnar::Column::from_values(other_b).ok()?,
                    );
                    let other = DataFrame::new_with_column_order(
                        index,
                        other_cols,
                        vec!["a".to_string(), "b".to_string()],
                    )
                    .ok()?;

                    Some((data, cond, other))
                },
            )
    })
}

fn arb_aligned_boolean_where_series_pair(
    max_len: usize,
) -> impl Strategy<Value = (Series, Series)> {
    (1..=max_len).prop_flat_map(|len| {
        (
            arb_unique_index_labels(len),
            arb_numeric_values(len),
            arb_boolean_condition_values(len),
        )
            .prop_filter_map(
                "aligned boolean where-series construction must succeed",
                |(labels, data_values, cond_values)| {
                    let data =
                        Series::from_values("data".to_owned(), labels.clone(), data_values).ok()?;
                    let cond = Series::from_values("cond".to_owned(), labels, cond_values).ok()?;
                    Some((data, cond))
                },
            )
    })
}

fn arb_aligned_boolean_where_dataframe_pair(
    max_rows: usize,
) -> impl Strategy<Value = (DataFrame, DataFrame)> {
    (1..=max_rows).prop_flat_map(|nrows| {
        (
            arb_unique_index_labels(nrows),
            arb_numeric_values(nrows),
            arb_numeric_values(nrows),
            arb_boolean_condition_values(nrows),
            arb_boolean_condition_values(nrows),
        )
            .prop_filter_map(
                "aligned boolean where-dataframe construction must succeed",
                |(labels, data_a, data_b, cond_a, cond_b)| {
                    let index = Index::new(labels);
                    let mut data_cols = std::collections::BTreeMap::new();
                    data_cols.insert(
                        "a".to_string(),
                        fp_columnar::Column::from_values(data_a).ok()?,
                    );
                    data_cols.insert(
                        "b".to_string(),
                        fp_columnar::Column::from_values(data_b).ok()?,
                    );
                    let data = DataFrame::new_with_column_order(
                        index.clone(),
                        data_cols,
                        vec!["a".to_string(), "b".to_string()],
                    )
                    .ok()?;

                    let mut cond_cols = std::collections::BTreeMap::new();
                    cond_cols.insert(
                        "a".to_string(),
                        fp_columnar::Column::from_values(cond_a).ok()?,
                    );
                    cond_cols.insert(
                        "b".to_string(),
                        fp_columnar::Column::from_values(cond_b).ok()?,
                    );
                    let cond = DataFrame::new_with_column_order(
                        index,
                        cond_cols,
                        vec!["a".to_string(), "b".to_string()],
                    )
                    .ok()?;

                    Some((data, cond))
                },
            )
    })
}

fn arb_variable_numeric_series(
    name: &'static str,
    max_len: usize,
) -> impl Strategy<Value = Series> {
    (1..=max_len).prop_flat_map(move |len| arb_numeric_series(name, len))
}

fn arb_replace_numeric_series(name: &'static str, max_len: usize) -> impl Strategy<Value = Series> {
    (1..=max_len).prop_flat_map(move |len| {
        (arb_index_labels(len), arb_replace_numeric_values(len)).prop_filter_map(
            "replace-series construction must succeed",
            move |(labels, values)| Series::from_values(name.to_owned(), labels, values).ok(),
        )
    })
}

fn arb_replace_numeric_dataframe(max_rows: usize) -> impl Strategy<Value = DataFrame> {
    (1..=max_rows).prop_flat_map(|nrows| {
        let idx_labels = arb_index_labels(nrows);
        let col_a = arb_replace_numeric_values(nrows);
        let col_b = arb_replace_numeric_values(nrows);
        (idx_labels, col_a, col_b).prop_filter_map(
            "replace-dataframe construction must succeed",
            move |(labels, va, vb)| {
                let index = Index::new(labels);
                let col_a = fp_columnar::Column::from_values(va).ok()?;
                let col_b = fp_columnar::Column::from_values(vb).ok()?;
                let mut cols = std::collections::BTreeMap::new();
                cols.insert("a".to_string(), col_a);
                cols.insert("b".to_string(), col_b);
                DataFrame::new_with_column_order(
                    index,
                    cols,
                    vec!["a".to_string(), "b".to_string()],
                )
                .ok()
            },
        )
    })
}

fn arb_take_positions_for_len(len: usize, max_take: usize) -> impl Strategy<Value = Vec<i64>> {
    let len_i64 = i64::try_from(len).expect("take length must fit in i64");
    proptest::collection::vec(
        prop_oneof![
            (0..len_i64).prop_map(|idx| idx),
            (1..=len_i64).prop_map(|distance| -distance),
        ],
        1..=max_take,
    )
}

fn arb_unique_take_positions_for_len(
    len: usize,
    max_take: usize,
) -> impl Strategy<Value = Vec<i64>> {
    let len_i64 = i64::try_from(len).expect("take length must fit in i64");
    let population = (0..len_i64).collect::<Vec<_>>();
    proptest::sample::subsequence(population, 1..=max_take.min(len)).prop_flat_map(
        move |positions| {
            let count = positions.len();
            (
                Just(positions),
                proptest::collection::vec(proptest::bool::ANY, count),
            )
                .prop_map(move |(positions, use_negative)| {
                    positions
                        .into_iter()
                        .zip(use_negative)
                        .map(|(position, negative)| {
                            if negative {
                                position - len_i64
                            } else {
                                position
                            }
                        })
                        .collect::<Vec<_>>()
                })
        },
    )
}

fn arb_series_take_case(
    name: &'static str,
    max_len: usize,
    max_take: usize,
) -> impl Strategy<Value = (Series, Vec<i64>)> {
    (1..=max_len).prop_flat_map(move |len| {
        (
            arb_numeric_series(name, len),
            arb_take_positions_for_len(len, max_take),
        )
    })
}

fn arb_series_take_composition_case(
    name: &'static str,
    max_len: usize,
    max_take: usize,
) -> impl Strategy<Value = (Series, Vec<i64>, Vec<i64>)> {
    (1..=max_len).prop_flat_map(move |len| {
        (
            arb_numeric_series(name, len),
            arb_take_positions_for_len(len, max_take),
        )
            .prop_flat_map(move |(series, outer)| {
                let outer_len = outer.len();
                (
                    Just(series),
                    Just(outer),
                    arb_take_positions_for_len(outer_len, max_take),
                )
            })
    })
}

fn arb_dataframe_take_rows_case(
    max_rows: usize,
    max_take: usize,
) -> impl Strategy<Value = (DataFrame, Vec<i64>)> {
    arb_combine_first_dataframe(max_rows).prop_flat_map(move |df| {
        let len = df.len();
        (Just(df), arb_take_positions_for_len(len, max_take))
    })
}

fn arb_dataframe_take_rows_composition_case(
    max_rows: usize,
    max_take: usize,
) -> impl Strategy<Value = (DataFrame, Vec<i64>, Vec<i64>)> {
    arb_combine_first_dataframe(max_rows).prop_flat_map(move |df| {
        let len = df.len();
        (Just(df), arb_take_positions_for_len(len, max_take)).prop_flat_map(move |(df, outer)| {
            let outer_len = outer.len();
            (
                Just(df),
                Just(outer),
                arb_take_positions_for_len(outer_len, max_take),
            )
        })
    })
}

fn arb_dataframe_take_columns_case(
    max_rows: usize,
    max_take: usize,
) -> impl Strategy<Value = (DataFrame, Vec<i64>)> {
    arb_combine_first_dataframe(max_rows).prop_flat_map(move |df| {
        let ncols = df.column_names().len();
        (Just(df), arb_unique_take_positions_for_len(ncols, max_take))
    })
}

fn arb_dataframe_take_columns_composition_case(
    max_rows: usize,
    max_take: usize,
) -> impl Strategy<Value = (DataFrame, Vec<i64>, Vec<i64>)> {
    arb_combine_first_dataframe(max_rows).prop_flat_map(move |df| {
        let ncols = df.column_names().len();
        (Just(df), arb_unique_take_positions_for_len(ncols, max_take)).prop_flat_map(
            move |(df, outer)| {
                let outer_len = outer.len();
                (
                    Just(df),
                    Just(outer),
                    arb_unique_take_positions_for_len(outer_len, max_take),
                )
            },
        )
    })
}

fn arb_series_set_axis_case(
    name: &'static str,
    max_len: usize,
) -> impl Strategy<Value = (Series, Vec<IndexLabel>)> {
    (1..=max_len).prop_flat_map(move |len| (arb_numeric_series(name, len), arb_index_labels(len)))
}

fn arb_series_set_axis_composition_case(
    name: &'static str,
    max_len: usize,
) -> impl Strategy<Value = (Series, Vec<IndexLabel>, Vec<IndexLabel>)> {
    (1..=max_len).prop_flat_map(move |len| {
        (
            arb_numeric_series(name, len),
            arb_index_labels(len),
            arb_index_labels(len),
        )
    })
}

fn arb_isin_test_values(max_len: usize) -> impl Strategy<Value = Vec<Scalar>> {
    proptest::collection::vec(arb_replace_numeric_scalar(), 0..=max_len)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// DataFrame add_scalar never panics.
    #[test]
    fn prop_df_add_scalar_no_panic(df in arb_numeric_dataframe(10), scalar in -1e6_f64..1e6_f64) {
        let _ = df.add_scalar(scalar);
    }

    /// DataFrame add_scalar preserves shape (rows x columns).
    #[test]
    fn prop_df_add_scalar_preserves_shape(df in arb_numeric_dataframe(10)) {
        if let Ok(result) = df.add_scalar(1.0) {
            prop_assert_eq!(result.index().len(), df.index().len(),
                "add_scalar must preserve row count");
            prop_assert_eq!(result.column_names().len(), df.column_names().len(),
                "add_scalar must preserve column count");
        }
    }

    /// DataFrame add_scalar(0) is approximately identity for non-missing values.
    #[test]
    fn prop_df_add_zero_is_identity(df in arb_numeric_dataframe(10)) {
        if let Ok(result) = df.add_scalar(0.0) {
            for name in df.column_names() {
                let orig_col = df.column(name).unwrap();
                let result_col = result.column(name).unwrap();
                for (i, (orig, res)) in orig_col.values().iter().zip(result_col.values()).enumerate() {
                    if orig.is_missing() {
                        prop_assert!(res.is_missing(),
                            "missing + 0 should stay missing at col={}, idx={}", name, i);
                    } else if let (Ok(ov), Ok(rv)) = (orig.to_f64(), res.to_f64())
                        && ov.is_finite()
                    {
                        prop_assert!((ov - rv).abs() < 1e-9,
                            "add(0) should be identity: {} vs {} at col={}, idx={}",
                            ov, rv, name, i);
                    }
                }
            }
        }
    }

    /// DataFrame mul_scalar(1) is approximately identity for non-missing values.
    #[test]
    fn prop_df_mul_one_is_identity(df in arb_numeric_dataframe(10)) {
        if let Ok(result) = df.mul_scalar(1.0) {
            for name in df.column_names() {
                let orig_col = df.column(name).unwrap();
                let result_col = result.column(name).unwrap();
                for (i, (orig, res)) in orig_col.values().iter().zip(result_col.values()).enumerate() {
                    if orig.is_missing() {
                        prop_assert!(res.is_missing(),
                            "missing * 1 should stay missing at col={}, idx={}", name, i);
                    } else if let (Ok(ov), Ok(rv)) = (orig.to_f64(), res.to_f64())
                        && ov.is_finite()
                    {
                        prop_assert!((ov - rv).abs() < 1e-9,
                            "mul(1) should be identity: {} vs {} at col={}, idx={}",
                            ov, rv, name, i);
                    }
                }
            }
        }
    }

    /// DataFrame add_df result index contains all labels from both inputs.
    #[test]
    fn prop_df_add_df_index_is_union(
        df1 in arb_numeric_dataframe(8),
        df2 in arb_numeric_dataframe(8),
    ) {
        if let Ok(result) = df1.add_df(&df2) {
            let result_labels = result.index().labels();
            for label in df1.index().labels() {
                prop_assert!(
                    result_labels.contains(label),
                    "add_df result must contain left label {:?}", label
                );
            }
            for label in df2.index().labels() {
                prop_assert!(
                    result_labels.contains(label),
                    "add_df result must contain right label {:?}", label
                );
            }
        }
    }

    /// DataFrame add_df result has union of columns from both inputs.
    #[test]
    fn prop_df_add_df_columns_are_union(
        df1 in arb_numeric_dataframe(5),
        df2 in arb_numeric_dataframe(5),
    ) {
        if let Ok(result) = df1.add_df(&df2) {
            let result_names: Vec<&String> = result.column_names();
            for name in df1.column_names() {
                prop_assert!(
                    result_names.contains(&name),
                    "add_df result must contain left column {:?}", name
                );
            }
            for name in df2.column_names() {
                prop_assert!(
                    result_names.contains(&name),
                    "add_df result must contain right column {:?}", name
                );
            }
        }
    }

    /// DataFrame eq_scalar_df produces Bool output, allowing missing values
    /// to propagate as Null.
    #[test]
    fn prop_df_eq_scalar_produces_bool(df in arb_numeric_dataframe(10)) {
        let scalar = Scalar::Int64(0);
        if let Ok(result) = df.eq_scalar_df(&scalar) {
            for name in result.column_names() {
                let col = result.column(name).unwrap();
                for (i, val) in col.values().iter().enumerate() {
                    prop_assert!(
                        matches!(val, Scalar::Bool(_)) || val.is_missing(),
                        "eq_scalar_df must produce Bool or missing values, got {:?} at col={}, idx={}",
                        val, name, i
                    );
                }
            }
        }
    }

    /// DataFrame comparison ops preserve shape.
    #[test]
    fn prop_df_comparison_preserves_shape(df in arb_numeric_dataframe(10)) {
        let scalar = Scalar::Int64(5);
        for op_name in ["eq", "ne", "gt", "ge", "lt", "le"] {
            let result = match op_name {
                "eq" => df.eq_scalar_df(&scalar),
                "ne" => df.ne_scalar_df(&scalar),
                "gt" => df.gt_scalar_df(&scalar),
                "ge" => df.ge_scalar_df(&scalar),
                "lt" => df.lt_scalar_df(&scalar),
                "le" => df.le_scalar_df(&scalar),
                _ => unreachable!(),
            };
            if let Ok(result_df) = result {
                prop_assert_eq!(result_df.index().len(), df.index().len(),
                    "{}_scalar_df must preserve row count", op_name);
                prop_assert_eq!(result_df.column_names().len(), df.column_names().len(),
                    "{}_scalar_df must preserve column count", op_name);
            }
        }
    }

    /// DataFrame eq + ne are complementary for non-NaN values.
    /// For any non-NaN value, eq(x) XOR ne(x) must be true.
    #[test]
    fn prop_df_eq_ne_complementary(df in arb_numeric_dataframe(8)) {
        let scalar = Scalar::Int64(0);
        let eq_result = df.eq_scalar_df(&scalar);
        let ne_result = df.ne_scalar_df(&scalar);
        if let (Ok(eq_df), Ok(ne_df)) = (eq_result, ne_result) {
            for name in eq_df.column_names() {
                let eq_col = eq_df.column(name).unwrap();
                let ne_col = ne_df.column(name).unwrap();
                let orig_col = df.column(name).unwrap();
                for (i, ((eq_v, ne_v), orig)) in eq_col.values().iter()
                    .zip(ne_col.values())
                    .zip(orig_col.values())
                    .enumerate()
                {
                    // Skip NaN values (NaN comparisons have special semantics)
                    if orig.is_missing() {
                        continue;
                    }
                    if let (Scalar::Bool(eq_b), Scalar::Bool(ne_b)) = (eq_v, ne_v) {
                        prop_assert_ne!(eq_b, ne_b,
                            "eq XOR ne must be true for non-NaN at col={}, idx={}", name, i);
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Property: nlargest / nsmallest metamorphic invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Series nlargest must be prefix-monotone in n.
    #[test]
    fn prop_series_nlargest_is_prefix_monotone(
        series in arb_variable_numeric_series("nlargest", 12),
        hint_a in 0usize..32,
        hint_b in 0usize..32,
    ) {
        let available = series.values().iter().filter(|value| !value.is_missing()).count();
        if available == 0 {
            return Ok(());
        }
        let n_small = 1 + (hint_a % available);
        let n_large = n_small + (hint_b % (available - n_small + 1));
        let small = series
            .nlargest(n_small)
            .expect("Series::nlargest() must succeed for numeric inputs");
        let large = series
            .nlargest(n_large)
            .expect("Series::nlargest() must succeed for numeric inputs");
        let small_rows = ordered_series_rows(&small);
        let large_rows = ordered_series_rows(&large);
        prop_assert_eq!(
            small_rows,
            large_rows[..n_small].to_vec(),
            "series nlargest({}) must equal the prefix of nlargest({})",
            n_small,
            n_large
        );
    }

    /// Series nsmallest must be prefix-monotone in n.
    #[test]
    fn prop_series_nsmallest_is_prefix_monotone(
        series in arb_variable_numeric_series("nsmallest", 12),
        hint_a in 0usize..32,
        hint_b in 0usize..32,
    ) {
        let available = series.values().iter().filter(|value| !value.is_missing()).count();
        if available == 0 {
            return Ok(());
        }
        let n_small = 1 + (hint_a % available);
        let n_large = n_small + (hint_b % (available - n_small + 1));
        let small = series
            .nsmallest(n_small)
            .expect("Series::nsmallest() must succeed for numeric inputs");
        let large = series
            .nsmallest(n_large)
            .expect("Series::nsmallest() must succeed for numeric inputs");
        let small_rows = ordered_series_rows(&small);
        let large_rows = ordered_series_rows(&large);
        prop_assert_eq!(
            small_rows,
            large_rows[..n_small].to_vec(),
            "series nsmallest({}) must equal the prefix of nsmallest({})",
            n_small,
            n_large
        );
    }

    /// Negating a Series must turn nlargest into nsmallest after re-negating the output.
    #[test]
    fn prop_series_nlargest_nsmallest_are_sign_dual(
        series in arb_variable_numeric_series("nlargest", 12),
        hint in 0usize..32,
    ) {
        let available = series.values().iter().filter(|value| !value.is_missing()).count();
        if available == 0 {
            return Ok(());
        }
        let n = 1 + (hint % available);
        let largest = series
            .nlargest(n)
            .expect("Series::nlargest() must succeed for numeric inputs");
        let flipped = sign_flip_series(&series);
        let smallest_on_flipped = flipped
            .nsmallest(n)
            .expect("Series::nsmallest() must succeed after sign flipping");
        let expected = sign_flip_series(&smallest_on_flipped);
        prop_assert!(
            largest.equals(&expected),
            "series nlargest(x, n) must equal -nsmallest(-x, n)"
        );
    }

    /// Negating a Series must turn nsmallest into nlargest after re-negating the output.
    #[test]
    fn prop_series_nsmallest_nlargest_are_sign_dual(
        series in arb_variable_numeric_series("nsmallest", 12),
        hint in 0usize..32,
    ) {
        let available = series.values().iter().filter(|value| !value.is_missing()).count();
        if available == 0 {
            return Ok(());
        }
        let n = 1 + (hint % available);
        let smallest = series
            .nsmallest(n)
            .expect("Series::nsmallest() must succeed for numeric inputs");
        let flipped = sign_flip_series(&series);
        let largest_on_flipped = flipped
            .nlargest(n)
            .expect("Series::nlargest() must succeed after sign flipping");
        let expected = sign_flip_series(&largest_on_flipped);
        prop_assert!(
            smallest.equals(&expected),
            "series nsmallest(x, n) must equal -nlargest(-x, n)"
        );
    }

    /// DataFrame nlargest must be prefix-monotone in n for a fixed numeric column.
    #[test]
    fn prop_dataframe_nlargest_is_prefix_monotone(
        df in arb_numeric_dataframe(8),
        hint_a in 0usize..32,
        hint_b in 0usize..32,
    ) {
        let column = df.column("a").expect("numeric dataframe must have column a");
        let available = column.values().iter().filter(|value| !value.is_missing()).count();
        if available == 0 {
            return Ok(());
        }
        let n_small = 1 + (hint_a % available);
        let n_large = n_small + (hint_b % (available - n_small + 1));
        let small = df
            .nlargest(n_small, "a")
            .expect("DataFrame::nlargest() must succeed for numeric inputs");
        let large = df
            .nlargest(n_large, "a")
            .expect("DataFrame::nlargest() must succeed for numeric inputs");
        let small_rows = ordered_dataframe_rows(&small);
        let large_rows = ordered_dataframe_rows(&large);
        prop_assert_eq!(
            small_rows,
            large_rows[..n_small].to_vec(),
            "dataframe nlargest({}) must equal the prefix of nlargest({})",
            n_small,
            n_large
        );
    }

    /// DataFrame nsmallest must be prefix-monotone in n for a fixed numeric column.
    #[test]
    fn prop_dataframe_nsmallest_is_prefix_monotone(
        df in arb_numeric_dataframe(8),
        hint_a in 0usize..32,
        hint_b in 0usize..32,
    ) {
        let column = df.column("a").expect("numeric dataframe must have column a");
        let available = column.values().iter().filter(|value| !value.is_missing()).count();
        if available == 0 {
            return Ok(());
        }
        let n_small = 1 + (hint_a % available);
        let n_large = n_small + (hint_b % (available - n_small + 1));
        let small = df
            .nsmallest(n_small, "a")
            .expect("DataFrame::nsmallest() must succeed for numeric inputs");
        let large = df
            .nsmallest(n_large, "a")
            .expect("DataFrame::nsmallest() must succeed for numeric inputs");
        let small_rows = ordered_dataframe_rows(&small);
        let large_rows = ordered_dataframe_rows(&large);
        prop_assert_eq!(
            small_rows,
            large_rows[..n_small].to_vec(),
            "dataframe nsmallest({}) must equal the prefix of nsmallest({})",
            n_small,
            n_large
        );
    }

    /// Negating a DataFrame must turn nlargest into nsmallest on the same column after re-negating.
    #[test]
    fn prop_dataframe_nlargest_nsmallest_are_sign_dual(
        df in arb_numeric_dataframe(8),
        hint in 0usize..32,
    ) {
        let column = df.column("a").expect("numeric dataframe must have column a");
        let available = column.values().iter().filter(|value| !value.is_missing()).count();
        if available == 0 {
            return Ok(());
        }
        let n = 1 + (hint % available);
        let largest = df
            .nlargest(n, "a")
            .expect("DataFrame::nlargest() must succeed for numeric inputs");
        let flipped = sign_flip_dataframe(&df);
        let smallest_on_flipped = flipped
            .nsmallest(n, "a")
            .expect("DataFrame::nsmallest() must succeed after sign flipping");
        let expected = sign_flip_dataframe(&smallest_on_flipped);
        prop_assert!(
            largest.equals(&expected),
            "dataframe nlargest(x, n) must equal -nsmallest(-x, n)"
        );
    }

    /// Negating a DataFrame must turn nsmallest into nlargest on the same column after re-negating.
    #[test]
    fn prop_dataframe_nsmallest_nlargest_are_sign_dual(
        df in arb_numeric_dataframe(8),
        hint in 0usize..32,
    ) {
        let column = df.column("a").expect("numeric dataframe must have column a");
        let available = column.values().iter().filter(|value| !value.is_missing()).count();
        if available == 0 {
            return Ok(());
        }
        let n = 1 + (hint % available);
        let smallest = df
            .nsmallest(n, "a")
            .expect("DataFrame::nsmallest() must succeed for numeric inputs");
        let flipped = sign_flip_dataframe(&df);
        let largest_on_flipped = flipped
            .nlargest(n, "a")
            .expect("DataFrame::nlargest() must succeed after sign flipping");
        let expected = sign_flip_dataframe(&largest_on_flipped);
        prop_assert!(
            smallest.equals(&expected),
            "dataframe nsmallest(x, n) must equal -nlargest(-x, n)"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: sort_values metamorphic invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Series sort_values ascending is idempotent.
    #[test]
    fn prop_series_sort_values_ascending_is_idempotent(
        series in arb_variable_numeric_series("sort_values", 12),
    ) {
        let once = series
            .sort_values(true)
            .expect("Series::sort_values(true) must succeed for numeric inputs");
        let twice = once
            .sort_values(true)
            .expect("sorting an ascending Series again must succeed");
        prop_assert!(
            once.equals(&twice),
            "sorting a Series ascending twice must be idempotent"
        );
    }

    /// Series sort_values descending is idempotent.
    #[test]
    fn prop_series_sort_values_descending_is_idempotent(
        series in arb_variable_numeric_series("sort_values", 12),
    ) {
        let once = series
            .sort_values(false)
            .expect("Series::sort_values(false) must succeed for numeric inputs");
        let twice = once
            .sort_values(false)
            .expect("sorting a descending Series again must succeed");
        prop_assert!(
            once.equals(&twice),
            "sorting a Series descending twice must be idempotent"
        );
    }

    /// Negating a Series must turn ascending sort order into descending order after re-negating.
    #[test]
    fn prop_series_sort_values_ascending_descending_are_sign_dual(
        series in arb_variable_numeric_series("sort_values", 12),
    ) {
        let ascending = series
            .sort_values(true)
            .expect("Series::sort_values(true) must succeed for numeric inputs");
        let flipped = sign_flip_series(&series);
        let descending_on_flipped = flipped
            .sort_values(false)
            .expect("Series::sort_values(false) must succeed after sign flipping");
        let expected = sign_flip_series(&descending_on_flipped);
        prop_assert!(
            ascending.equals(&expected),
            "series sort_values(ascending=true) must equal -sort_values(-x, ascending=false)"
        );
    }

    /// DataFrame sort_values ascending is idempotent for a fixed numeric column.
    #[test]
    fn prop_dataframe_sort_values_ascending_is_idempotent(
        df in arb_numeric_dataframe(8),
    ) {
        let once = df
            .sort_values("a", true)
            .expect("DataFrame::sort_values(\"a\", true) must succeed for numeric inputs");
        let twice = once
            .sort_values("a", true)
            .expect("sorting an ascending DataFrame again must succeed");
        prop_assert!(
            once.equals(&twice),
            "sorting a DataFrame ascending twice must be idempotent"
        );
    }

    /// DataFrame sort_values descending is idempotent for a fixed numeric column.
    #[test]
    fn prop_dataframe_sort_values_descending_is_idempotent(
        df in arb_numeric_dataframe(8),
    ) {
        let once = df
            .sort_values("a", false)
            .expect("DataFrame::sort_values(\"a\", false) must succeed for numeric inputs");
        let twice = once
            .sort_values("a", false)
            .expect("sorting a descending DataFrame again must succeed");
        prop_assert!(
            once.equals(&twice),
            "sorting a DataFrame descending twice must be idempotent"
        );
    }

    /// Negating a DataFrame must turn ascending sort order into descending order after re-negating.
    #[test]
    fn prop_dataframe_sort_values_ascending_descending_are_sign_dual(
        df in arb_numeric_dataframe(8),
    ) {
        let ascending = df
            .sort_values("a", true)
            .expect("DataFrame::sort_values(\"a\", true) must succeed for numeric inputs");
        let flipped = sign_flip_dataframe(&df);
        let descending_on_flipped = flipped
            .sort_values("a", false)
            .expect("DataFrame::sort_values(\"a\", false) must succeed after sign flipping");
        let expected = sign_flip_dataframe(&descending_on_flipped);
        prop_assert!(
            ascending.equals(&expected),
            "dataframe sort_values(\"a\", ascending=true) must equal -sort_values(-x, \"a\", ascending=false)"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: sort_index metamorphic invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Series sort_index ascending is idempotent.
    #[test]
    fn prop_series_sort_index_ascending_is_idempotent(
        series in arb_replace_numeric_series("sort_index", 12),
    ) {
        let once = series
            .sort_index(true)
            .expect("Series::sort_index(true) must succeed");
        let twice = once
            .sort_index(true)
            .expect("sorting an index-ascending Series again must succeed");
        prop_assert!(
            once.equals(&twice),
            "series sort_index(ascending=true) must be idempotent"
        );
    }

    /// Series sort_index descending is idempotent.
    #[test]
    fn prop_series_sort_index_descending_is_idempotent(
        series in arb_replace_numeric_series("sort_index", 12),
    ) {
        let once = series
            .sort_index(false)
            .expect("Series::sort_index(false) must succeed");
        let twice = once
            .sort_index(false)
            .expect("sorting an index-descending Series again must succeed");
        prop_assert!(
            once.equals(&twice),
            "series sort_index(ascending=false) must be idempotent"
        );
    }

    /// Negating values must commute with Series sort_index because the order depends only on the index.
    #[test]
    fn prop_series_sort_index_commutes_with_sign_flip(
        series in arb_replace_numeric_series("sort_index", 12),
    ) {
        let baseline = series
            .sort_index(true)
            .expect("Series::sort_index(true) must succeed");
        let flipped_then_sorted = sign_flip_series(&series)
            .sort_index(true)
            .expect("Series::sort_index(true) must succeed after sign flipping");
        let expected = sign_flip_series(&baseline);
        prop_assert!(
            approx_equal_series(&flipped_then_sorted, &expected),
            "series sort_index must commute with sign flipping"
        );
    }

    /// With unique index labels, reversing the input rows must not change ascending sort_index output.
    #[test]
    fn prop_series_sort_index_unique_reverse_input_is_invariant(
        series in arb_unique_numeric_series("sort_index_unique", 12),
    ) {
        let baseline = series
            .sort_index(true)
            .expect("Series::sort_index(true) must succeed for unique-index inputs");
        let reversed_sorted = reverse_series(&series)
            .sort_index(true)
            .expect("Series::sort_index(true) must succeed after reversing unique-index input");
        prop_assert!(
            baseline.equals(&reversed_sorted),
            "series sort_index(ascending=true) must ignore input row order when index labels are unique"
        );
    }

    /// With unique index labels, descending sort_index must equal reversing the ascending result.
    #[test]
    fn prop_series_sort_index_unique_descending_is_reversed_ascending(
        series in arb_unique_numeric_series("sort_index_unique", 12),
    ) {
        let ascending = series
            .sort_index(true)
            .expect("Series::sort_index(true) must succeed for unique-index inputs");
        let descending = series
            .sort_index(false)
            .expect("Series::sort_index(false) must succeed for unique-index inputs");
        let expected = reverse_series(&ascending);
        prop_assert!(
            descending.equals(&expected),
            "series sort_index(ascending=false) must equal reversing sort_index(ascending=true) for unique-index inputs"
        );
    }

    /// DataFrame sort_index ascending is idempotent.
    #[test]
    fn prop_dataframe_sort_index_ascending_is_idempotent(
        df in arb_replace_numeric_dataframe(8),
    ) {
        let once = df
            .sort_index(true)
            .expect("DataFrame::sort_index(true) must succeed");
        let twice = once
            .sort_index(true)
            .expect("sorting an index-ascending DataFrame again must succeed");
        prop_assert!(
            once.equals(&twice),
            "dataframe sort_index(ascending=true) must be idempotent"
        );
    }

    /// DataFrame sort_index descending is idempotent.
    #[test]
    fn prop_dataframe_sort_index_descending_is_idempotent(
        df in arb_replace_numeric_dataframe(8),
    ) {
        let once = df
            .sort_index(false)
            .expect("DataFrame::sort_index(false) must succeed");
        let twice = once
            .sort_index(false)
            .expect("sorting an index-descending DataFrame again must succeed");
        prop_assert!(
            once.equals(&twice),
            "dataframe sort_index(ascending=false) must be idempotent"
        );
    }

    /// Negating values must commute with DataFrame sort_index because the row order depends only on the index.
    #[test]
    fn prop_dataframe_sort_index_commutes_with_sign_flip(
        df in arb_replace_numeric_dataframe(8),
    ) {
        let baseline = df
            .sort_index(true)
            .expect("DataFrame::sort_index(true) must succeed");
        let flipped_then_sorted = sign_flip_dataframe(&df)
            .sort_index(true)
            .expect("DataFrame::sort_index(true) must succeed after sign flipping");
        let expected = sign_flip_dataframe(&baseline);
        prop_assert!(
            approx_equal_dataframe(&flipped_then_sorted, &expected),
            "dataframe sort_index must commute with sign flipping"
        );
    }

    /// With unique index labels, reversing the input rows must not change ascending sort_index output.
    #[test]
    fn prop_dataframe_sort_index_unique_reverse_input_is_invariant(
        df in arb_combine_first_dataframe(8),
    ) {
        let baseline = df
            .sort_index(true)
            .expect("DataFrame::sort_index(true) must succeed for unique-index inputs");
        let reversed_sorted = reverse_dataframe_rows(&df)
            .sort_index(true)
            .expect("DataFrame::sort_index(true) must succeed after reversing unique-index input");
        prop_assert!(
            approx_equal_dataframe(&baseline, &reversed_sorted),
            "dataframe sort_index(ascending=true) must ignore input row order when index labels are unique"
        );
    }

    /// With unique index labels, descending sort_index must equal reversing the ascending result.
    #[test]
    fn prop_dataframe_sort_index_unique_descending_is_reversed_ascending(
        df in arb_combine_first_dataframe(8),
    ) {
        let ascending = df
            .sort_index(true)
            .expect("DataFrame::sort_index(true) must succeed for unique-index inputs");
        let descending = df
            .sort_index(false)
            .expect("DataFrame::sort_index(false) must succeed for unique-index inputs");
        let expected = reverse_dataframe_rows(&ascending);
        prop_assert!(
            approx_equal_dataframe(&descending, &expected),
            "dataframe sort_index(ascending=false) must equal reversing sort_index(ascending=true) for unique-index inputs"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: reindex metamorphic invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Reindexing a uniquely indexed Series to its own labels must be identity.
    #[test]
    fn prop_series_reindex_self_is_identity(
        series in arb_variable_unique_numeric_series("reindex", 12),
    ) {
        let own_labels = series.index().labels().to_vec();
        let reindexed = series
            .reindex(own_labels)
            .expect("Series::reindex(self.index) must succeed for unique-index inputs");
        prop_assert!(
            approx_equal_series(&series, &reindexed),
            "series reindexing to its own labels must be identity"
        );
    }

    /// Series reindex_like must match direct reindexing to the other index.
    #[test]
    fn prop_series_reindex_like_matches_direct_reindex(
        (source, other) in arb_unique_series_pair(12),
    ) {
        let via_like = source
            .reindex_like(&other)
            .expect("Series::reindex_like() must succeed for unique-index sources");
        let direct = source
            .reindex(other.index().labels().to_vec())
            .expect("Series::reindex() must succeed for unique-index sources");
        prop_assert!(
            approx_equal_series(&via_like, &direct),
            "series reindex_like must equal direct reindex(other.index)"
        );
    }

    /// Reindexing and sign-flipping a uniquely indexed Series must commute.
    #[test]
    fn prop_series_reindex_commutes_with_sign_flip(
        series in arb_variable_unique_numeric_series("reindex", 12),
    ) {
        let target = reindex_target_labels(series.index().labels());
        let baseline = series
            .reindex(target.clone())
            .expect("Series::reindex() must succeed for unique-index inputs");
        let flipped_then_reindexed = sign_flip_series(&series)
            .reindex(target)
            .expect("Series::reindex() must succeed after sign flipping");
        let expected = sign_flip_series(&baseline);
        prop_assert!(
            approx_equal_series(&flipped_then_reindexed, &expected),
            "series reindex must commute with sign flipping"
        );
    }

    /// Reindexing through a unique intermediate superset must collapse to the direct target reindex.
    #[test]
    fn prop_series_reindex_composes_through_unique_intermediate_superset(
        series in arb_variable_unique_numeric_series("reindex", 12),
    ) {
        let intermediate = reindex_intermediate_labels(series.index().labels());
        let target = reindex_target_labels(series.index().labels());
        let direct = series
            .reindex(target.clone())
            .expect("Series::reindex(target) must succeed for unique-index inputs");
        let via_intermediate = series
            .reindex(intermediate)
            .and_then(|reindexed| reindexed.reindex(target))
            .expect("Series::reindex() composition must succeed through a unique intermediate");
        prop_assert!(
            approx_equal_series(&direct, &via_intermediate),
            "series reindex through a unique intermediate superset must equal direct reindex"
        );
    }

    /// pad is an alias for ffill in Series reindex_with_method.
    #[test]
    fn prop_series_reindex_ffill_aliases_pad(
        series in arb_variable_unique_numeric_series("reindex", 12),
    ) {
        let target = reindex_target_labels(series.index().labels());
        let ffill = series
            .reindex_with_method(target.clone(), "ffill")
            .expect("Series::reindex_with_method(ffill) must succeed");
        let pad = series
            .reindex_with_method(target, "pad")
            .expect("Series::reindex_with_method(pad) must succeed");
        prop_assert!(
            approx_equal_series(&ffill, &pad),
            "series reindex_with_method('pad') must equal reindex_with_method('ffill')"
        );
    }

    /// backfill is an alias for bfill in Series reindex_with_method.
    #[test]
    fn prop_series_reindex_bfill_aliases_backfill(
        series in arb_variable_unique_numeric_series("reindex", 12),
    ) {
        let target = reindex_target_labels(series.index().labels());
        let bfill = series
            .reindex_with_method(target.clone(), "bfill")
            .expect("Series::reindex_with_method(bfill) must succeed");
        let backfill = series
            .reindex_with_method(target, "backfill")
            .expect("Series::reindex_with_method(backfill) must succeed");
        prop_assert!(
            approx_equal_series(&bfill, &backfill),
            "series reindex_with_method('backfill') must equal reindex_with_method('bfill')"
        );
    }

    /// Reindexing a uniquely indexed DataFrame to its own labels must be identity.
    #[test]
    fn prop_dataframe_reindex_self_is_identity(
        df in arb_combine_first_dataframe(8),
    ) {
        let own_labels = df.index().labels().to_vec();
        let reindexed = df
            .reindex(own_labels)
            .expect("DataFrame::reindex(self.index) must succeed for unique-index inputs");
        prop_assert!(
            approx_equal_dataframe(&df, &reindexed),
            "dataframe reindexing to its own labels must be identity"
        );
    }

    /// Reindexing and sign-flipping a uniquely indexed DataFrame must commute.
    #[test]
    fn prop_dataframe_reindex_commutes_with_sign_flip(
        df in arb_combine_first_dataframe(8),
    ) {
        let target = reindex_target_labels(df.index().labels());
        let baseline = df
            .reindex(target.clone())
            .expect("DataFrame::reindex() must succeed for unique-index inputs");
        let flipped_then_reindexed = sign_flip_dataframe(&df)
            .reindex(target)
            .expect("DataFrame::reindex() must succeed after sign flipping");
        let expected = sign_flip_dataframe(&baseline);
        prop_assert!(
            approx_equal_dataframe(&flipped_then_reindexed, &expected),
            "dataframe reindex must commute with sign flipping"
        );
    }

    /// Reindexing a DataFrame through a unique intermediate superset must collapse to the direct target reindex.
    #[test]
    fn prop_dataframe_reindex_composes_through_unique_intermediate_superset(
        df in arb_combine_first_dataframe(8),
    ) {
        let intermediate = reindex_intermediate_labels(df.index().labels());
        let target = reindex_target_labels(df.index().labels());
        let direct = df
            .reindex(target.clone())
            .expect("DataFrame::reindex(target) must succeed for unique-index inputs");
        let via_intermediate = df
            .reindex(intermediate)
            .and_then(|reindexed| reindexed.reindex(target))
            .expect("DataFrame::reindex() composition must succeed through a unique intermediate");
        prop_assert!(
            approx_equal_dataframe(&direct, &via_intermediate),
            "dataframe reindex through a unique intermediate superset must equal direct reindex"
        );
    }

    /// pad is an alias for ffill in DataFrame reindex_with_method.
    #[test]
    fn prop_dataframe_reindex_ffill_aliases_pad(
        df in arb_combine_first_dataframe(8),
    ) {
        let target = reindex_target_labels(df.index().labels());
        let ffill = df
            .reindex_with_method(target.clone(), "ffill")
            .expect("DataFrame::reindex_with_method(ffill) must succeed");
        let pad = df
            .reindex_with_method(target, "pad")
            .expect("DataFrame::reindex_with_method(pad) must succeed");
        prop_assert!(
            approx_equal_dataframe(&ffill, &pad),
            "dataframe reindex_with_method('pad') must equal reindex_with_method('ffill')"
        );
    }

    /// backfill is an alias for bfill in DataFrame reindex_with_method.
    #[test]
    fn prop_dataframe_reindex_bfill_aliases_backfill(
        df in arb_combine_first_dataframe(8),
    ) {
        let target = reindex_target_labels(df.index().labels());
        let bfill = df
            .reindex_with_method(target.clone(), "bfill")
            .expect("DataFrame::reindex_with_method(bfill) must succeed");
        let backfill = df
            .reindex_with_method(target, "backfill")
            .expect("DataFrame::reindex_with_method(backfill) must succeed");
        prop_assert!(
            approx_equal_dataframe(&bfill, &backfill),
            "dataframe reindex_with_method('backfill') must equal reindex_with_method('bfill')"
        );
    }

    /// DataFrame reindex_axis(axis=1) must match reindex_columns for the same unique target columns.
    #[test]
    fn prop_dataframe_reindex_axis1_matches_reindex_columns(
        df in arb_combine_first_dataframe(8),
    ) {
        let own_columns = df.column_names().into_iter().cloned().collect::<Vec<_>>();
        let target_columns = reindex_target_columns(&own_columns);
        let via_axis = reindex_axis1_from_names(&df, &target_columns);
        let via_columns = reindex_columns_from_names(&df, &target_columns);
        prop_assert!(
            approx_equal_dataframe(&via_axis, &via_columns),
            "dataframe reindex_axis(axis=1) must match reindex_columns"
        );
    }

    /// Reindexing DataFrame columns and sign-flipping must commute.
    #[test]
    fn prop_dataframe_reindex_axis1_commutes_with_sign_flip(
        df in arb_combine_first_dataframe(8),
    ) {
        let own_columns = df.column_names().into_iter().cloned().collect::<Vec<_>>();
        let target_columns = reindex_target_columns(&own_columns);
        let baseline = reindex_axis1_from_names(&df, &target_columns);
        let flipped_then_reindexed = reindex_axis1_from_names(&sign_flip_dataframe(&df), &target_columns);
        let expected = sign_flip_dataframe(&baseline);
        prop_assert!(
            approx_equal_dataframe(&flipped_then_reindexed, &expected),
            "dataframe column reindex must commute with sign flipping"
        );
    }

    /// Reindexing DataFrame columns through a unique intermediate superset must collapse to the direct target reindex.
    #[test]
    fn prop_dataframe_reindex_axis1_composes_through_unique_intermediate_superset(
        df in arb_combine_first_dataframe(8),
    ) {
        let own_columns = df.column_names().into_iter().cloned().collect::<Vec<_>>();
        let intermediate_columns = reindex_intermediate_columns(&own_columns);
        let target_columns = reindex_target_columns(&own_columns);
        let direct = reindex_axis1_from_names(&df, &target_columns);
        let via_intermediate = reindex_axis1_from_names(
            &reindex_axis1_from_names(&df, &intermediate_columns),
            &target_columns,
        );
        prop_assert!(
            approx_equal_dataframe(&direct, &via_intermediate),
            "dataframe column reindex through a unique intermediate superset must equal direct reindex"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: truncate metamorphic invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Truncating a Series with open bounds must be identity.
    #[test]
    fn prop_series_truncate_open_bounds_is_identity(
        series in (1..=12usize).prop_flat_map(|len| arb_sorted_int_series("truncate", len)),
    ) {
        let truncated = series
            .truncate(None, None)
            .expect("Series::truncate(None, None) must succeed");
        prop_assert!(
            approx_equal_series(&truncated, &series),
            "series truncate(None, None) must be identity"
        );
    }

    /// Series truncate must commute with sign flipping because bounds only inspect the index.
    #[test]
    fn prop_series_truncate_commutes_with_sign_flip(
        (series, before, after) in arb_series_truncate_case("truncate", 12),
    ) {
        let truncated = series
            .truncate(before.as_ref(), after.as_ref())
            .expect("Series::truncate() must succeed for sorted int labels");
        let flipped_then_truncated = sign_flip_series(&series)
            .truncate(before.as_ref(), after.as_ref())
            .expect("Series::truncate() must succeed after sign flipping");
        let expected = sign_flip_series(&truncated);
        prop_assert!(
            approx_equal_series(&flipped_then_truncated, &expected),
            "series truncate must commute with sign flipping"
        );
    }

    /// Nested Series truncates must collapse to the directly intersected bounds.
    #[test]
    fn prop_series_truncate_composes(
        (series, before_a, after_a, before_b, after_b) in arb_series_truncate_composition_case("truncate", 12),
    ) {
        let nested = series
            .truncate(before_a.as_ref(), after_a.as_ref())
            .and_then(|truncated| truncated.truncate(before_b.as_ref(), after_b.as_ref()))
            .expect("nested Series::truncate() must succeed for sorted int labels");
        let direct_before = merged_truncate_before(&before_a, &before_b);
        let direct_after = merged_truncate_after(&after_a, &after_b);
        let direct = series
            .truncate(direct_before.as_ref(), direct_after.as_ref())
            .expect("direct composed Series::truncate() must succeed");
        prop_assert!(
            approx_equal_series(&nested, &direct),
            "nested series truncates must equal directly truncating with intersected bounds"
        );
    }

    /// Truncating a DataFrame with open bounds must be identity.
    #[test]
    fn prop_dataframe_truncate_open_bounds_is_identity(
        df in arb_sorted_int_dataframe(8),
    ) {
        let truncated = df
            .truncate(None, None)
            .expect("DataFrame::truncate(None, None) must succeed");
        prop_assert!(
            approx_equal_dataframe(&truncated, &df),
            "dataframe truncate(None, None) must be identity"
        );
    }

    /// DataFrame truncate must commute with sign flipping because bounds only inspect the index.
    #[test]
    fn prop_dataframe_truncate_commutes_with_sign_flip(
        (df, before, after) in arb_dataframe_truncate_case(8),
    ) {
        let truncated = df
            .truncate(before.as_ref(), after.as_ref())
            .expect("DataFrame::truncate() must succeed for sorted int labels");
        let flipped_then_truncated = sign_flip_dataframe(&df)
            .truncate(before.as_ref(), after.as_ref())
            .expect("DataFrame::truncate() must succeed after sign flipping");
        let expected = sign_flip_dataframe(&truncated);
        prop_assert!(
            approx_equal_dataframe(&flipped_then_truncated, &expected),
            "dataframe truncate must commute with sign flipping"
        );
    }

    /// Nested DataFrame truncates must collapse to the directly intersected bounds.
    #[test]
    fn prop_dataframe_truncate_composes(
        (df, before_a, after_a, before_b, after_b) in arb_dataframe_truncate_composition_case(8),
    ) {
        let nested = df
            .truncate(before_a.as_ref(), after_a.as_ref())
            .and_then(|truncated| truncated.truncate(before_b.as_ref(), after_b.as_ref()))
            .expect("nested DataFrame::truncate() must succeed for sorted int labels");
        let direct_before = merged_truncate_before(&before_a, &before_b);
        let direct_after = merged_truncate_after(&after_a, &after_b);
        let direct = df
            .truncate(direct_before.as_ref(), direct_after.as_ref())
            .expect("direct composed DataFrame::truncate() must succeed");
        prop_assert!(
            approx_equal_dataframe(&nested, &direct),
            "nested dataframe truncates must equal directly truncating with intersected bounds"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: drop metamorphic invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Dropping no Series labels must be identity.
    #[test]
    fn prop_series_drop_empty_is_identity(
        series in (1..=12usize).prop_flat_map(|len| arb_unique_numeric_series("drop", len)),
    ) {
        let labels = Vec::<IndexLabel>::new();
        let dropped = series
            .drop(&labels)
            .expect("Series::drop([]) must succeed");
        prop_assert!(
            approx_equal_series(&dropped, &series),
            "series drop([]) must be identity"
        );
    }

    /// Dropping Series labels in two passes must equal dropping their union directly.
    #[test]
    fn prop_series_drop_composes(
        (series, first, second) in arb_series_drop_composition_case("drop", 12),
    ) {
        let nested = series
            .drop(&first)
            .and_then(|dropped| dropped.drop(&second))
            .expect("nested Series::drop() must succeed");
        let merged = merged_drop_items(&first, &second);
        let direct = series
            .drop(&merged)
            .expect("direct composed Series::drop() must succeed");
        prop_assert!(
            approx_equal_series(&nested, &direct),
            "series drop(labels1).drop(labels2) must equal dropping the union directly"
        );
    }

    /// Dropping Series labels must commute with numeric sign flipping.
    #[test]
    fn prop_series_drop_commutes_with_sign_flip(
        (series, labels) in arb_series_drop_case("drop", 12),
    ) {
        let dropped = series
            .drop(&labels)
            .expect("Series::drop() must succeed for existing labels");
        let flipped_then_dropped = sign_flip_series(&series)
            .drop(&labels)
            .expect("Series::drop() must succeed after sign flipping");
        let expected = sign_flip_series(&dropped);
        prop_assert!(
            approx_equal_series(&flipped_then_dropped, &expected),
            "series drop must commute with sign flipping"
        );
    }

    /// Dropping no DataFrame rows must be identity.
    #[test]
    fn prop_dataframe_drop_rows_empty_is_identity(
        df in arb_unique_utf8_numeric_dataframe(8),
    ) {
        let labels = Vec::<String>::new();
        let dropped = drop_dataframe_rows(&df, &labels);
        prop_assert!(
            approx_equal_dataframe(&dropped, &df),
            "dataframe drop(axis=0, labels=[]) must be identity"
        );
    }

    /// Dropping DataFrame rows in two passes must equal dropping their union directly.
    #[test]
    fn prop_dataframe_drop_rows_composes(
        (df, first, second) in arb_dataframe_row_drop_composition_case(8),
    ) {
        let nested = drop_dataframe_rows(&drop_dataframe_rows(&df, &first), &second);
        let merged = merged_drop_items(&first, &second);
        let direct = drop_dataframe_rows(&df, &merged);
        prop_assert!(
            approx_equal_dataframe(&nested, &direct),
            "dataframe row drop must compose through the union of row labels"
        );
    }

    /// Dropping DataFrame rows must commute with numeric sign flipping.
    #[test]
    fn prop_dataframe_drop_rows_commutes_with_sign_flip(
        (df, labels) in arb_dataframe_row_drop_case(8),
    ) {
        let dropped = drop_dataframe_rows(&df, &labels);
        let flipped_then_dropped = drop_dataframe_rows(&sign_flip_dataframe(&df), &labels);
        let expected = sign_flip_dataframe(&dropped);
        prop_assert!(
            approx_equal_dataframe(&flipped_then_dropped, &expected),
            "dataframe row drop must commute with sign flipping"
        );
    }

    /// Dropping no DataFrame columns must be identity.
    #[test]
    fn prop_dataframe_drop_columns_empty_is_identity(
        df in arb_unique_utf8_numeric_dataframe(8),
    ) {
        let columns = Vec::<String>::new();
        let dropped = drop_dataframe_columns(&df, &columns);
        prop_assert!(
            approx_equal_dataframe(&dropped, &df),
            "dataframe drop(axis=1, labels=[]) must be identity"
        );
    }

    /// Dropping disjoint DataFrame columns in two passes must equal dropping their union directly.
    #[test]
    fn prop_dataframe_drop_columns_composes(
        (df, first, second) in arb_dataframe_column_drop_composition_case(8),
    ) {
        let nested = drop_dataframe_columns(&drop_dataframe_columns(&df, &first), &second);
        let merged = merged_drop_items(&first, &second);
        let direct = drop_dataframe_columns(&df, &merged);
        prop_assert!(
            approx_equal_dataframe(&nested, &direct),
            "dataframe column drop must compose through the union of disjoint column labels"
        );
    }

    /// Dropping DataFrame columns must commute with numeric sign flipping.
    #[test]
    fn prop_dataframe_drop_columns_commutes_with_sign_flip(
        (df, columns) in arb_dataframe_column_drop_case(8),
    ) {
        let dropped = drop_dataframe_columns(&df, &columns);
        let flipped_then_dropped = drop_dataframe_columns(&sign_flip_dataframe(&df), &columns);
        let expected = sign_flip_dataframe(&dropped);
        prop_assert!(
            approx_equal_dataframe(&flipped_then_dropped, &expected),
            "dataframe column drop must commute with sign flipping"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: head / tail metamorphic invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Nested Series head calls must collapse to the smaller requested prefix.
    #[test]
    fn prop_series_head_composes(
        (series, first, second) in arb_series_head_tail_composition_case("head", 12),
    ) {
        let nested = series
            .head(first)
            .and_then(|head| head.head(second))
            .expect("nested Series::head() must succeed");
        let direct = series
            .head(first.min(second))
            .expect("direct Series::head() must succeed");
        prop_assert!(
            approx_equal_series(&nested, &direct),
            "series head(head(x, n1), n2) must equal head(x, min(n1, n2))"
        );
    }

    /// Nested Series tail calls must collapse to the smaller requested suffix.
    #[test]
    fn prop_series_tail_composes(
        (series, first, second) in arb_series_head_tail_composition_case("tail", 12),
    ) {
        let nested = series
            .tail(first)
            .and_then(|tail| tail.tail(second))
            .expect("nested Series::tail() must succeed");
        let direct = series
            .tail(first.min(second))
            .expect("direct Series::tail() must succeed");
        prop_assert!(
            approx_equal_series(&nested, &direct),
            "series tail(tail(x, n1), n2) must equal tail(x, min(n1, n2))"
        );
    }

    /// Reversing a headed Series must match tailing the reversed Series.
    #[test]
    fn prop_series_head_reverse_dual_to_tail(
        (series, n) in arb_series_head_tail_case("head_tail", 12),
    ) {
        let reversed_head = reverse_series(
            &series
                .head(n)
                .expect("Series::head() must succeed before reversing"),
        );
        let tail_on_reversed = reverse_series(&series)
            .tail(n)
            .expect("Series::tail() must succeed on reversed input");
        prop_assert!(
            approx_equal_series(&reversed_head, &tail_on_reversed),
            "reverse(head(x, n)) must equal tail(reverse(x), n)"
        );
    }

    /// Nested DataFrame head calls must collapse to the smaller requested prefix.
    #[test]
    fn prop_dataframe_head_composes(
        (df, first, second) in arb_dataframe_head_tail_composition_case(8),
    ) {
        let nested = df
            .head(first)
            .and_then(|head| head.head(second))
            .expect("nested DataFrame::head() must succeed");
        let direct = df
            .head(first.min(second))
            .expect("direct DataFrame::head() must succeed");
        prop_assert!(
            approx_equal_dataframe(&nested, &direct),
            "dataframe head(head(x, n1), n2) must equal head(x, min(n1, n2))"
        );
    }

    /// Nested DataFrame tail calls must collapse to the smaller requested suffix.
    #[test]
    fn prop_dataframe_tail_composes(
        (df, first, second) in arb_dataframe_head_tail_composition_case(8),
    ) {
        let nested = df
            .tail(first)
            .and_then(|tail| tail.tail(second))
            .expect("nested DataFrame::tail() must succeed");
        let direct = df
            .tail(first.min(second))
            .expect("direct DataFrame::tail() must succeed");
        prop_assert!(
            approx_equal_dataframe(&nested, &direct),
            "dataframe tail(tail(x, n1), n2) must equal tail(x, min(n1, n2))"
        );
    }

    /// Reversing a headed DataFrame must match tailing the reversed DataFrame.
    #[test]
    fn prop_dataframe_head_reverse_dual_to_tail(
        (df, n) in arb_dataframe_head_tail_case(8),
    ) {
        let reversed_head = reverse_dataframe_rows(
            &df.head(n)
                .expect("DataFrame::head() must succeed before reversing"),
        );
        let tail_on_reversed = reverse_dataframe_rows(&df)
            .tail(n)
            .expect("DataFrame::tail() must succeed on reversed input");
        prop_assert!(
            approx_equal_dataframe(&reversed_head, &tail_on_reversed),
            "reverse(head(x, n)) must equal tail(reverse(x), n) for dataframes"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: rename metamorphic invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Renaming a Series twice must collapse to the last name.
    #[test]
    fn prop_series_rename_overwrite_composes(
        (series, first_name, second_name) in (1..=12usize).prop_flat_map(|len| {
            (
                arb_numeric_series("rename", len),
                "[a-z]{1,8}",
                "[a-z]{1,8}",
            )
        }),
    ) {
        let nested = series
            .rename(&first_name)
            .and_then(|renamed| renamed.rename(&second_name))
            .expect("nested Series::rename() must succeed");
        let direct = series
            .rename(&second_name)
            .expect("direct Series::rename() must succeed");
        prop_assert!(
            approx_equal_series(&nested, &direct),
            "series rename(name1).rename(name2) must equal rename(name2)"
        );
    }

    /// Renaming a Series must commute with sign flipping because it only changes metadata.
    #[test]
    fn prop_series_rename_commutes_with_sign_flip(
        (series, new_name) in (1..=12usize)
            .prop_flat_map(|len| (arb_numeric_series("rename", len), "[a-z]{1,8}")),
    ) {
        let renamed = series
            .rename(&new_name)
            .expect("Series::rename() must succeed");
        let flipped_then_renamed = sign_flip_series(&series)
            .rename(&new_name)
            .expect("Series::rename() must succeed after sign flipping");
        let expected = sign_flip_series(&renamed);
        prop_assert!(
            approx_equal_series(&flipped_then_renamed, &expected),
            "series rename(name) must commute with sign flipping"
        );
    }

    /// Renaming a Series must commute with take because positional selection is name-blind.
    #[test]
    fn prop_series_rename_commutes_with_take(
        (series, indices, new_name) in (1..=12usize).prop_flat_map(|len| {
            (
                arb_numeric_series("rename", len),
                arb_take_positions_for_len(len, 12),
                "[a-z]{1,8}",
            )
        }),
    ) {
        let renamed_then_taken = series
            .rename(&new_name)
            .and_then(|renamed| renamed.take(&indices))
            .expect("Series::rename().take() must succeed");
        let taken_then_renamed = series
            .take(&indices)
            .and_then(|taken| taken.rename(&new_name))
            .expect("Series::take().rename() must succeed");
        prop_assert!(
            approx_equal_series(&renamed_then_taken, &taken_then_renamed),
            "series rename(name) must commute with take(indices)"
        );
    }

    /// Injective DataFrame column renames must compose by composing the renaming functions.
    #[test]
    fn prop_dataframe_rename_columns_composes_injective_prefixes(
        df in arb_combine_first_dataframe(8),
    ) {
        let nested = df
            .rename_with(|name| format!("first_{name}"))
            .and_then(|renamed| renamed.rename_with(|name| format!("second_{name}")))
            .expect("nested DataFrame::rename_with() must succeed for injective prefixes");
        let direct = df
            .rename_with(|name| format!("second_first_{name}"))
            .expect("direct DataFrame::rename_with() must succeed for injective prefixes");
        prop_assert!(
            approx_equal_dataframe(&nested, &direct),
            "nested injective column renames must equal the directly composed rename"
        );
    }

    /// DataFrame rename_columns(mapping) must match rename_with() for the same complete fresh renaming.
    #[test]
    fn prop_dataframe_rename_columns_mapping_matches_rename_with(
        df in arb_combine_first_dataframe(8),
    ) {
        let current_names = df.column_names().into_iter().cloned().collect::<Vec<_>>();
        let target_names = fresh_rename_column_names(&current_names);
        let rename_map = current_names
            .iter()
            .cloned()
            .zip(target_names.iter().cloned())
            .collect::<std::collections::HashMap<_, _>>();
        let via_mapping = rename_columns_from_names(&df, &target_names);
        let via_function = df
            .rename_with(|name| {
                rename_map
                    .get(name)
                    .expect("complete column rename map must cover every column")
                    .clone()
            })
            .expect("DataFrame::rename_with() must succeed for complete fresh renaming");
        prop_assert!(
            approx_equal_dataframe(&via_mapping, &via_function),
            "dataframe rename_columns(mapping) must match rename_with() on the same complete renaming"
        );
    }

    /// Renaming DataFrame columns must commute with sign flipping because it only changes metadata.
    #[test]
    fn prop_dataframe_rename_columns_commutes_with_sign_flip(
        df in arb_combine_first_dataframe(8),
    ) {
        let current_names = df.column_names().into_iter().cloned().collect::<Vec<_>>();
        let target_names = fresh_rename_column_names(&current_names);
        let renamed = rename_columns_from_names(&df, &target_names);
        let flipped_then_renamed = rename_columns_from_names(&sign_flip_dataframe(&df), &target_names);
        let expected = sign_flip_dataframe(&renamed);
        prop_assert!(
            approx_equal_dataframe(&flipped_then_renamed, &expected),
            "dataframe column rename must commute with sign flipping"
        );
    }

    /// DataFrame index renames over unique labels must compose through the final target labels.
    #[test]
    fn prop_dataframe_rename_index_composes(
        df in arb_combine_first_dataframe(8),
    ) {
        let original_labels = df.index().labels().to_vec();
        let first_labels = fresh_rename_index_labels(&original_labels);
        let first = rename_index_from_labels(&df, &first_labels);
        let second_labels = fresh_rename_index_labels(&first_labels);
        let nested = rename_index_from_labels(&first, &second_labels);
        let direct = rename_index_from_labels(&df, &second_labels);
        prop_assert!(
            approx_equal_dataframe(&nested, &direct),
            "nested dataframe index renames must equal the directly composed rename"
        );
    }

    /// DataFrame rename_index(mapping) must match rename_index_with() for the same complete fresh renaming.
    #[test]
    fn prop_dataframe_rename_index_mapping_matches_rename_with(
        df in arb_combine_first_dataframe(8),
    ) {
        let current_labels = df.index().labels().to_vec();
        let target_labels = fresh_rename_index_labels(&current_labels);
        let rename_map = current_labels
            .iter()
            .cloned()
            .zip(target_labels.iter().cloned())
            .collect::<std::collections::HashMap<_, _>>();
        let via_mapping = rename_index_from_labels(&df, &target_labels);
        let via_function = df.rename_index_with(|label| {
            rename_map
                .get(label)
                .expect("complete index rename map must cover every label")
                .clone()
        });
        prop_assert!(
            approx_equal_dataframe(&via_mapping, &via_function),
            "dataframe rename_index(mapping) must match rename_index_with() on the same complete renaming"
        );
    }

    /// Renaming DataFrame index labels must commute with sign flipping because it only changes metadata.
    #[test]
    fn prop_dataframe_rename_index_commutes_with_sign_flip(
        df in arb_combine_first_dataframe(8),
    ) {
        let current_labels = df.index().labels().to_vec();
        let target_labels = fresh_rename_index_labels(&current_labels);
        let renamed = rename_index_from_labels(&df, &target_labels);
        let flipped_then_renamed = rename_index_from_labels(&sign_flip_dataframe(&df), &target_labels);
        let expected = sign_flip_dataframe(&renamed);
        prop_assert!(
            approx_equal_dataframe(&flipped_then_renamed, &expected),
            "dataframe index rename must commute with sign flipping"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: set_axis metamorphic invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Replacing the axis twice must collapse to the last label set.
    #[test]
    fn prop_series_set_axis_overwrite_composes(
        (series, first_labels, second_labels) in arb_series_set_axis_composition_case("set_axis", 12),
    ) {
        let nested = series
            .set_axis(first_labels)
            .and_then(|renamed| renamed.set_axis(second_labels.clone()))
            .expect("nested Series::set_axis() must succeed for matching lengths");
        let direct = series
            .set_axis(second_labels)
            .expect("direct Series::set_axis() must succeed for matching lengths");
        prop_assert!(
            approx_equal_series(&nested, &direct),
            "series set_axis(labels1).set_axis(labels2) must equal set_axis(labels2)"
        );
    }

    /// Replacing axis labels must commute with sign flipping because it does not inspect values.
    #[test]
    fn prop_series_set_axis_commutes_with_sign_flip(
        (series, labels) in arb_series_set_axis_case("set_axis", 12),
    ) {
        let relabeled = series
            .set_axis(labels.clone())
            .expect("Series::set_axis() must succeed for matching lengths");
        let flipped_then_relabeled = sign_flip_series(&series)
            .set_axis(labels)
            .expect("Series::set_axis() must succeed after sign flipping");
        let expected = sign_flip_series(&relabeled);
        prop_assert!(
            approx_equal_series(&flipped_then_relabeled, &expected),
            "series set_axis(labels) must commute with sign flipping"
        );
    }

    /// Replacing axis labels must commute with renaming because the two operations touch disjoint metadata.
    #[test]
    fn prop_series_set_axis_commutes_with_rename(
        (series, labels, new_name) in (1..=12usize).prop_flat_map(|len| {
            (
                arb_numeric_series("set_axis", len),
                arb_index_labels(len),
                "[a-z]{1,8}",
            )
        }),
    ) {
        let renamed_then_relabeled = series
            .rename(&new_name)
            .and_then(|renamed| renamed.set_axis(labels.clone()))
            .expect("Series::rename().set_axis() must succeed");
        let relabeled_then_renamed = series
            .set_axis(labels)
            .and_then(|renamed| renamed.rename(&new_name))
            .expect("Series::set_axis().rename() must succeed");
        prop_assert!(
            approx_equal_series(&renamed_then_relabeled, &relabeled_then_renamed),
            "series set_axis(labels) must commute with rename(name)"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: take metamorphic invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Nested Series take operations must collapse to a single take on composed positions.
    #[test]
    fn prop_series_take_composes(
        (series, outer, inner) in arb_series_take_composition_case("take", 12, 12),
    ) {
        let nested = series
            .take(&outer)
            .and_then(|taken| taken.take(&inner))
            .expect("nested Series::take() must succeed for valid positions");
        let composed_indices = compose_take_indices(&outer, &inner, series.len());
        let direct = series
            .take(&composed_indices)
            .expect("direct composed Series::take() must succeed");
        prop_assert!(
            approx_equal_series(&nested, &direct),
            "nested series take operations must equal a single take on composed positions"
        );
    }

    /// Series take must commute with sign flip because positional selection is value-blind.
    #[test]
    fn prop_series_take_commutes_with_sign_flip(
        (series, indices) in arb_series_take_case("take", 12, 12),
    ) {
        let taken = series
            .take(&indices)
            .expect("Series::take() must succeed for valid positions");
        let flipped_then_taken = sign_flip_series(&series)
            .take(&indices)
            .expect("Series::take() must succeed after sign flipping");
        let expected = sign_flip_series(&taken);
        prop_assert!(
            approx_equal_series(&flipped_then_taken, &expected),
            "series take must commute with sign flipping"
        );
    }

    /// DataFrame take(axis=0) must match take_rows on normalized positions.
    #[test]
    fn prop_dataframe_take_axis0_matches_take_rows(
        (df, indices) in arb_dataframe_take_rows_case(8, 12),
    ) {
        let axis_take = df
            .take(&indices, 0)
            .expect("DataFrame::take(axis=0) must succeed for valid positions");
        let helper_take = take_rows_via_normalized_positions(&df, &indices);
        prop_assert!(
            approx_equal_dataframe(&axis_take, &helper_take),
            "dataframe take(axis=0) must equal take_rows(normalized indices)"
        );
    }

    /// Nested DataFrame take(axis=0) operations must collapse to a single take on composed row positions.
    #[test]
    fn prop_dataframe_take_axis0_composes(
        (df, outer, inner) in arb_dataframe_take_rows_composition_case(8, 12),
    ) {
        let nested = df
            .take(&outer, 0)
            .and_then(|taken| taken.take(&inner, 0))
            .expect("nested DataFrame::take(axis=0) must succeed for valid positions");
        let composed_indices = compose_take_indices(&outer, &inner, df.len());
        let direct = df
            .take(&composed_indices, 0)
            .expect("direct composed DataFrame::take(axis=0) must succeed");
        prop_assert!(
            approx_equal_dataframe(&nested, &direct),
            "nested dataframe row takes must equal a single take on composed positions"
        );
    }

    /// DataFrame take(axis=0) must commute with sign flip because row selection is value-blind.
    #[test]
    fn prop_dataframe_take_axis0_commutes_with_sign_flip(
        (df, indices) in arb_dataframe_take_rows_case(8, 12),
    ) {
        let taken = df
            .take(&indices, 0)
            .expect("DataFrame::take(axis=0) must succeed for valid positions");
        let flipped_then_taken = sign_flip_dataframe(&df)
            .take(&indices, 0)
            .expect("DataFrame::take(axis=0) must succeed after sign flipping");
        let expected = sign_flip_dataframe(&taken);
        prop_assert!(
            approx_equal_dataframe(&flipped_then_taken, &expected),
            "dataframe row take must commute with sign flipping"
        );
    }

    /// DataFrame take(axis=1) must match take_columns on normalized positions.
    #[test]
    fn prop_dataframe_take_axis1_matches_take_columns(
        (df, indices) in arb_dataframe_take_columns_case(8, 6),
    ) {
        let axis_take = df
            .take(&indices, 1)
            .expect("DataFrame::take(axis=1) must succeed for valid positions");
        let helper_take = take_columns_via_normalized_positions(&df, &indices);
        prop_assert!(
            approx_equal_dataframe(&axis_take, &helper_take),
            "dataframe take(axis=1) must equal take_columns(normalized indices)"
        );
    }

    /// Nested DataFrame take(axis=1) operations must collapse to a single take on composed column positions.
    #[test]
    fn prop_dataframe_take_axis1_composes(
        (df, outer, inner) in arb_dataframe_take_columns_composition_case(8, 6),
    ) {
        let nested = df
            .take(&outer, 1)
            .and_then(|taken| taken.take(&inner, 1))
            .expect("nested DataFrame::take(axis=1) must succeed for valid positions");
        let composed_indices = compose_take_indices(&outer, &inner, df.column_names().len());
        let direct = df
            .take(&composed_indices, 1)
            .expect("direct composed DataFrame::take(axis=1) must succeed");
        prop_assert!(
            approx_equal_dataframe(&nested, &direct),
            "nested dataframe column takes must equal a single take on composed positions"
        );
    }

    /// DataFrame take(axis=1) must commute with sign flip because column selection is value-blind.
    #[test]
    fn prop_dataframe_take_axis1_commutes_with_sign_flip(
        (df, indices) in arb_dataframe_take_columns_case(8, 6),
    ) {
        let taken = df
            .take(&indices, 1)
            .expect("DataFrame::take(axis=1) must succeed for valid positions");
        let flipped_then_taken = sign_flip_dataframe(&df)
            .take(&indices, 1)
            .expect("DataFrame::take(axis=1) must succeed after sign flipping");
        let expected = sign_flip_dataframe(&taken);
        prop_assert!(
            approx_equal_dataframe(&flipped_then_taken, &expected),
            "dataframe column take must commute with sign flipping"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: diff metamorphic invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Translating a Series must not change its finite differences.
    #[test]
    fn prop_series_diff_is_translation_invariant(
        series in arb_numeric_series("diff", 12),
        periods in -3i64..=3,
        delta in -10.0f64..10.0,
    ) {
        let original = series
            .diff(periods)
            .expect("Series::diff() must succeed for numeric inputs");
        let shifted = shift_series(&series, delta);
        let translated = shifted
            .diff(periods)
            .expect("Series::diff() must succeed after translation");
        prop_assert!(
            approx_equal_series(&original, &translated),
            "series diff(x + c) must equal diff(x)"
        );
    }

    /// Scaling a Series by a positive factor must scale its finite differences.
    #[test]
    fn prop_series_diff_scales_linearly(
        series in arb_numeric_series("diff", 12),
        periods in -3i64..=3,
        factor in 0.25f64..5.0,
    ) {
        let original = series
            .diff(periods)
            .expect("Series::diff() must succeed for numeric inputs");
        let scaled_input = scale_series(&series, factor);
        let scaled_diff = scaled_input
            .diff(periods)
            .expect("Series::diff() must succeed after positive scaling");
        let expected = scale_series(&original, factor);
        prop_assert!(
            approx_equal_series(&scaled_diff, &expected),
            "series diff(kx) must equal k * diff(x) for positive k"
        );
    }

    /// Negating a Series must negate its finite differences.
    #[test]
    fn prop_series_diff_is_sign_symmetric(
        series in arb_numeric_series("diff", 12),
        periods in -3i64..=3,
    ) {
        let original = series
            .diff(periods)
            .expect("Series::diff() must succeed for numeric inputs");
        let flipped_input = sign_flip_series(&series);
        let flipped_diff = flipped_input
            .diff(periods)
            .expect("Series::diff() must succeed after sign flipping");
        let expected = sign_flip_series(&original);
        prop_assert!(
            flipped_diff.equals(&expected),
            "series diff(-x) must equal -diff(x)"
        );
    }

    /// Translating a DataFrame must not change per-column finite differences.
    #[test]
    fn prop_dataframe_diff_is_translation_invariant(
        df in arb_numeric_dataframe(8),
        periods in -3i64..=3,
        delta in -10.0f64..10.0,
    ) {
        let original = df
            .diff(periods)
            .expect("DataFrame::diff() must succeed for numeric inputs");
        let shifted = shift_dataframe(&df, delta);
        let translated = shifted
            .diff(periods)
            .expect("DataFrame::diff() must succeed after translation");
        prop_assert!(
            approx_equal_dataframe(&original, &translated),
            "dataframe diff(x + c) must equal diff(x)"
        );
    }

    /// Scaling a DataFrame by a positive factor must scale per-column finite differences.
    #[test]
    fn prop_dataframe_diff_scales_linearly(
        df in arb_numeric_dataframe(8),
        periods in -3i64..=3,
        factor in 0.25f64..5.0,
    ) {
        let original = df
            .diff(periods)
            .expect("DataFrame::diff() must succeed for numeric inputs");
        let scaled_input = scale_dataframe(&df, factor);
        let scaled_diff = scaled_input
            .diff(periods)
            .expect("DataFrame::diff() must succeed after positive scaling");
        let expected = scale_dataframe(&original, factor);
        prop_assert!(
            approx_equal_dataframe(&scaled_diff, &expected),
            "dataframe diff(kx) must equal k * diff(x) for positive k"
        );
    }

    /// Negating a DataFrame must negate its per-column finite differences.
    #[test]
    fn prop_dataframe_diff_is_sign_symmetric(
        df in arb_numeric_dataframe(8),
        periods in -3i64..=3,
    ) {
        let original = df
            .diff(periods)
            .expect("DataFrame::diff() must succeed for numeric inputs");
        let flipped_input = sign_flip_dataframe(&df);
        let flipped_diff = flipped_input
            .diff(periods)
            .expect("DataFrame::diff() must succeed after sign flipping");
        let expected = sign_flip_dataframe(&original);
        prop_assert!(
            flipped_diff.equals(&expected),
            "dataframe diff(-x) must equal -diff(x)"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: pct_change metamorphic invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Scaling a Series by a positive factor must not change pct_change.
    #[test]
    fn prop_series_pct_change_is_positive_scale_invariant(
        series in arb_numeric_series("pct_change", 12),
        periods in 1usize..=3,
        factor in 0.25f64..5.0,
    ) {
        let baseline = series
            .pct_change(periods)
            .expect("Series::pct_change() must succeed for numeric inputs");
        let scaled = scale_series(&series, factor);
        let scaled_change = scaled
            .pct_change(periods)
            .expect("Series::pct_change() must succeed after positive scaling");
        prop_assert!(
            approx_equal_series(&baseline, &scaled_change),
            "series pct_change(kx) must equal pct_change(x) for positive k"
        );
    }

    /// Negating a Series must not change pct_change.
    #[test]
    fn prop_series_pct_change_is_sign_flip_invariant(
        series in arb_numeric_series("pct_change", 12),
        periods in 1usize..=3,
    ) {
        let baseline = series
            .pct_change(periods)
            .expect("Series::pct_change() must succeed for numeric inputs");
        let flipped = sign_flip_series(&series);
        let flipped_change = flipped
            .pct_change(periods)
            .expect("Series::pct_change() must succeed after sign flipping");
        prop_assert!(
            approx_equal_series(&baseline, &flipped_change),
            "series pct_change(-x) must equal pct_change(x)"
        );
    }

    /// Series pct_change must agree with diff(periods) / shift(periods).
    #[test]
    fn prop_series_pct_change_matches_diff_over_shift(
        series in arb_numeric_series("pct_change", 12),
        periods in 1usize..=3,
    ) {
        let observed = series
            .pct_change(periods)
            .expect("Series::pct_change() must succeed for numeric inputs");
        let reconstructed = reconstruct_pct_change_series(&series, periods);
        prop_assert!(
            approx_equal_series(&observed, &reconstructed),
            "series pct_change must match diff(periods) / shift(periods)"
        );
    }

    /// Scaling a DataFrame by a positive factor must not change pct_change.
    #[test]
    fn prop_dataframe_pct_change_is_positive_scale_invariant(
        df in arb_numeric_dataframe(8),
        periods in 1usize..=3,
        factor in 0.25f64..5.0,
    ) {
        let baseline = df
            .pct_change(periods)
            .expect("DataFrame::pct_change() must succeed for numeric inputs");
        let scaled = scale_dataframe(&df, factor);
        let scaled_change = scaled
            .pct_change(periods)
            .expect("DataFrame::pct_change() must succeed after positive scaling");
        prop_assert!(
            approx_equal_dataframe(&baseline, &scaled_change),
            "dataframe pct_change(kx) must equal pct_change(x) for positive k"
        );
    }

    /// Negating a DataFrame must not change pct_change.
    #[test]
    fn prop_dataframe_pct_change_is_sign_flip_invariant(
        df in arb_numeric_dataframe(8),
        periods in 1usize..=3,
    ) {
        let baseline = df
            .pct_change(periods)
            .expect("DataFrame::pct_change() must succeed for numeric inputs");
        let flipped = sign_flip_dataframe(&df);
        let flipped_change = flipped
            .pct_change(periods)
            .expect("DataFrame::pct_change() must succeed after sign flipping");
        prop_assert!(
            approx_equal_dataframe(&baseline, &flipped_change),
            "dataframe pct_change(-x) must equal pct_change(x)"
        );
    }

    /// DataFrame pct_change must agree with per-column diff(periods) / shift(periods).
    #[test]
    fn prop_dataframe_pct_change_matches_diff_over_shift(
        df in arb_numeric_dataframe(8),
        periods in 1usize..=3,
    ) {
        let observed = df
            .pct_change(periods)
            .expect("DataFrame::pct_change() must succeed for numeric inputs");
        let reconstructed = reconstruct_pct_change_dataframe(&df, periods);
        prop_assert!(
            approx_equal_dataframe(&observed, &reconstructed),
            "dataframe pct_change must match diff(periods) / shift(periods)"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: axis1 diff and pct_change metamorphic invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Translating every numeric cell must not change row-wise finite differences.
    #[test]
    fn prop_dataframe_diff_axis1_is_translation_invariant(
        df in arb_numeric_dataframe(8),
        periods in prop_oneof![Just(-2i64), Just(-1i64), Just(1i64), Just(2i64)],
        delta in -10.0f64..10.0,
    ) {
        let original = df
            .diff_axis1(periods)
            .expect("DataFrame::diff_axis1() must succeed for numeric inputs");
        let shifted = shift_dataframe(&df, delta);
        let translated = shifted
            .diff_axis1(periods)
            .expect("DataFrame::diff_axis1() must succeed after translation");
        prop_assert!(
            approx_equal_dataframe(&original, &translated),
            "dataframe diff_axis1(x + c) must equal diff_axis1(x)"
        );
    }

    /// Scaling every numeric cell by a positive factor must scale row-wise finite differences.
    #[test]
    fn prop_dataframe_diff_axis1_scales_linearly(
        df in arb_numeric_dataframe(8),
        periods in prop_oneof![Just(-2i64), Just(-1i64), Just(1i64), Just(2i64)],
        factor in 0.25f64..5.0,
    ) {
        let original = df
            .diff_axis1(periods)
            .expect("DataFrame::diff_axis1() must succeed for numeric inputs");
        let scaled_input = scale_dataframe(&df, factor);
        let scaled_diff = scaled_input
            .diff_axis1(periods)
            .expect("DataFrame::diff_axis1() must succeed after positive scaling");
        let expected = scale_dataframe(&original, factor);
        prop_assert!(
            approx_equal_dataframe(&scaled_diff, &expected),
            "dataframe diff_axis1(kx) must equal k * diff_axis1(x) for positive k"
        );
    }

    /// Negating every numeric cell must negate row-wise finite differences.
    #[test]
    fn prop_dataframe_diff_axis1_is_sign_symmetric(
        df in arb_numeric_dataframe(8),
        periods in prop_oneof![Just(-2i64), Just(-1i64), Just(1i64), Just(2i64)],
    ) {
        let original = df
            .diff_axis1(periods)
            .expect("DataFrame::diff_axis1() must succeed for numeric inputs");
        let flipped = sign_flip_dataframe(&df);
        let flipped_diff = flipped
            .diff_axis1(periods)
            .expect("DataFrame::diff_axis1() must succeed after sign flipping");
        let expected = sign_flip_dataframe(&original);
        prop_assert!(
            flipped_diff.equals(&expected),
            "dataframe diff_axis1(-x) must equal -diff_axis1(x)"
        );
    }

    /// Scaling every numeric cell by a positive factor must not change row-wise pct_change.
    #[test]
    fn prop_dataframe_pct_change_axis1_is_positive_scale_invariant(
        df in arb_numeric_dataframe(8),
        periods in prop_oneof![Just(-2i64), Just(-1i64), Just(1i64), Just(2i64)],
        factor in 0.25f64..5.0,
    ) {
        let baseline = df
            .pct_change_axis1(periods)
            .expect("DataFrame::pct_change_axis1() must succeed for numeric inputs");
        let scaled = scale_dataframe(&df, factor);
        let scaled_change = scaled
            .pct_change_axis1(periods)
            .expect("DataFrame::pct_change_axis1() must succeed after positive scaling");
        prop_assert!(
            approx_equal_dataframe(&baseline, &scaled_change),
            "dataframe pct_change_axis1(kx) must equal pct_change_axis1(x) for positive k"
        );
    }

    /// Negating every numeric cell must not change row-wise pct_change.
    #[test]
    fn prop_dataframe_pct_change_axis1_is_sign_flip_invariant(
        df in arb_numeric_dataframe(8),
        periods in prop_oneof![Just(-2i64), Just(-1i64), Just(1i64), Just(2i64)],
    ) {
        let baseline = df
            .pct_change_axis1(periods)
            .expect("DataFrame::pct_change_axis1() must succeed for numeric inputs");
        let flipped = sign_flip_dataframe(&df);
        let flipped_change = flipped
            .pct_change_axis1(periods)
            .expect("DataFrame::pct_change_axis1() must succeed after sign flipping");
        prop_assert!(
            approx_equal_dataframe(&baseline, &flipped_change),
            "dataframe pct_change_axis1(-x) must equal pct_change_axis1(x)"
        );
    }

    /// Row-wise pct_change must agree with diff_axis1(periods) / shift_axis1(periods).
    #[test]
    fn prop_dataframe_pct_change_axis1_matches_diff_over_shift(
        df in arb_numeric_dataframe(8),
        periods in prop_oneof![Just(-2i64), Just(-1i64), Just(1i64), Just(2i64)],
    ) {
        let observed = df
            .pct_change_axis1(periods)
            .expect("DataFrame::pct_change_axis1() must succeed for numeric inputs");
        let reconstructed = reconstruct_pct_change_axis1_dataframe(&df, periods);
        prop_assert!(
            approx_equal_dataframe(&observed, &reconstructed),
            "dataframe pct_change_axis1 must match diff_axis1(periods) / shift_axis1(periods)"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: dropna metamorphic invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Series dropna is idempotent.
    #[test]
    fn prop_series_dropna_is_idempotent(
        series in arb_variable_numeric_series("dropna", 12),
    ) {
        let once = series
            .dropna()
            .expect("Series::dropna() must succeed for numeric inputs");
        let twice = once
            .dropna()
            .expect("Series::dropna() must succeed on an already dropped series");
        prop_assert!(
            once.equals(&twice),
            "series dropna() must be idempotent"
        );
    }

    /// Translating a Series commutes with dropna because missingness is unchanged.
    #[test]
    fn prop_series_dropna_is_translation_covariant(
        series in arb_variable_numeric_series("dropna", 12),
        delta in -10.0f64..10.0,
    ) {
        let baseline = series
            .dropna()
            .expect("Series::dropna() must succeed for numeric inputs");
        let shifted_input = shift_series(&series, delta);
        let shifted_result = shifted_input
            .dropna()
            .expect("Series::dropna() must succeed after translation");
        let expected = shift_series(&baseline, delta);
        prop_assert!(
            approx_equal_series(&shifted_result, &expected),
            "series dropna(x + c) must equal dropna(x) + c"
        );
    }

    /// Negating a Series commutes with dropna because missingness is unchanged.
    #[test]
    fn prop_series_dropna_is_sign_symmetric(
        series in arb_variable_numeric_series("dropna", 12),
    ) {
        let baseline = series
            .dropna()
            .expect("Series::dropna() must succeed for numeric inputs");
        let flipped_input = sign_flip_series(&series);
        let flipped_result = flipped_input
            .dropna()
            .expect("Series::dropna() must succeed after sign flip");
        let expected = sign_flip_series(&baseline);
        prop_assert!(
            approx_equal_series(&flipped_result, &expected),
            "series dropna(-x) must equal -dropna(x)"
        );
    }

    /// Default DataFrame row-wise dropna is idempotent.
    #[test]
    fn prop_dataframe_dropna_is_idempotent(
        df in arb_numeric_dataframe(8),
    ) {
        let once = df
            .dropna()
            .expect("DataFrame::dropna() must succeed for numeric inputs");
        let twice = once
            .dropna()
            .expect("DataFrame::dropna() must succeed on an already dropped frame");
        prop_assert!(
            once.equals(&twice),
            "dataframe dropna() must be idempotent"
        );
    }

    /// Translating numeric cells commutes with row-wise dropna.
    #[test]
    fn prop_dataframe_dropna_is_translation_covariant(
        df in arb_numeric_dataframe(8),
        delta in -10.0f64..10.0,
    ) {
        let baseline = df
            .dropna()
            .expect("DataFrame::dropna() must succeed for numeric inputs");
        let shifted_input = shift_dataframe(&df, delta);
        let shifted_result = shifted_input
            .dropna()
            .expect("DataFrame::dropna() must succeed after translation");
        let expected = shift_dataframe(&baseline, delta);
        prop_assert!(
            approx_equal_dataframe(&shifted_result, &expected),
            "dataframe dropna(x + c) must equal dropna(x) + c"
        );
    }

    /// Negating numeric cells commutes with row-wise dropna.
    #[test]
    fn prop_dataframe_dropna_is_sign_symmetric(
        df in arb_numeric_dataframe(8),
    ) {
        let baseline = df
            .dropna()
            .expect("DataFrame::dropna() must succeed for numeric inputs");
        let flipped_input = sign_flip_dataframe(&df);
        let flipped_result = flipped_input
            .dropna()
            .expect("DataFrame::dropna() must succeed after sign flip");
        let expected = sign_flip_dataframe(&baseline);
        prop_assert!(
            approx_equal_dataframe(&flipped_result, &expected),
            "dataframe dropna(-x) must equal -dropna(x)"
        );
    }

    /// Applying `how='all'` before default `how='any'` must not change the final result.
    #[test]
    fn prop_dataframe_dropna_any_after_all_matches_direct_any(
        df in arb_numeric_dataframe(8),
    ) {
        let all_then_any = df
            .dropna_with_options(DropNaHow::All, None)
            .expect("DataFrame::dropna_with_options(All) must succeed")
            .dropna()
            .expect("DataFrame::dropna() must succeed after how='all'");
        let direct_any = df
            .dropna()
            .expect("DataFrame::dropna() must succeed for numeric inputs");
        prop_assert!(
            all_then_any.equals(&direct_any),
            "dataframe dropna(any) must equal dropna(any) after dropna(all)"
        );
    }

    /// Stronger row thresholds compose over weaker ones.
    #[test]
    fn prop_dataframe_dropna_threshold_composes_monotonically(
        df in arb_numeric_dataframe(8),
        (low_thresh, high_thresh) in (1usize..=2).prop_flat_map(|low| (Just(low), low..=2usize)),
    ) {
        let weaker = df
            .dropna_with_threshold(low_thresh, None)
            .expect("DataFrame::dropna_with_threshold() must succeed for weaker threshold");
        let strong_after_weak = weaker
            .dropna_with_threshold(high_thresh, None)
            .expect("DataFrame::dropna_with_threshold() must succeed after weaker threshold");
        let strong_direct = df
            .dropna_with_threshold(high_thresh, None)
            .expect("DataFrame::dropna_with_threshold() must succeed for stronger threshold");
        prop_assert!(
            strong_after_weak.equals(&strong_direct),
            "dataframe stronger thresh must equal stronger thresh after weaker thresh"
        );
    }

    /// Column-wise dropna is idempotent.
    #[test]
    fn prop_dataframe_dropna_columns_is_idempotent(
        df in arb_numeric_dataframe(8),
    ) {
        let once = df
            .dropna_columns()
            .expect("DataFrame::dropna_columns() must succeed for numeric inputs");
        let twice = once
            .dropna_columns()
            .expect("DataFrame::dropna_columns() must succeed on an already dropped frame");
        prop_assert!(
            once.equals(&twice),
            "dataframe dropna_columns() must be idempotent"
        );
    }

    /// Translating numeric cells commutes with column-wise dropna.
    #[test]
    fn prop_dataframe_dropna_columns_is_translation_covariant(
        df in arb_numeric_dataframe(8),
        delta in -10.0f64..10.0,
    ) {
        let baseline = df
            .dropna_columns()
            .expect("DataFrame::dropna_columns() must succeed for numeric inputs");
        let shifted_input = shift_dataframe(&df, delta);
        let shifted_result = shifted_input
            .dropna_columns()
            .expect("DataFrame::dropna_columns() must succeed after translation");
        let expected = shift_dataframe(&baseline, delta);
        prop_assert!(
            approx_equal_dataframe(&shifted_result, &expected),
            "dataframe dropna_columns(x + c) must equal dropna_columns(x) + c"
        );
    }

    /// Applying column-wise `how='all'` before default `how='any'` must not change the final columns kept.
    #[test]
    fn prop_dataframe_dropna_columns_any_after_all_matches_direct_any(
        df in arb_numeric_dataframe(8),
    ) {
        let all_then_any = df
            .dropna_columns_with_options(DropNaHow::All, None)
            .expect("DataFrame::dropna_columns_with_options(All) must succeed")
            .dropna_columns()
            .expect("DataFrame::dropna_columns() must succeed after how='all'");
        let direct_any = df
            .dropna_columns()
            .expect("DataFrame::dropna_columns() must succeed for numeric inputs");
        prop_assert!(
            all_then_any.equals(&direct_any),
            "dataframe dropna_columns(any) must equal dropna_columns(any) after dropna_columns(all)"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: isna / notna metamorphic invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// `Series::notna()` must be the boolean complement of `Series::isna()`.
    #[test]
    fn prop_series_notna_complements_isna(
        series in arb_variable_numeric_series("isna", 12),
    ) {
        let isna = series
            .isna()
            .expect("Series::isna() must succeed for numeric inputs");
        let notna = series
            .notna()
            .expect("Series::notna() must succeed for numeric inputs");
        let expected = invert_bool_series(&isna)
            .expect("complemented Series::isna() mask must construct");
        prop_assert!(
            approx_equal_series(&notna, &expected),
            "series notna() must equal the boolean complement of isna()"
        );
    }

    /// Negating numeric values must not change the `Series::isna()` mask.
    #[test]
    fn prop_series_isna_is_sign_flip_invariant(
        series in arb_variable_numeric_series("isna", 12),
    ) {
        let baseline = series
            .isna()
            .expect("Series::isna() must succeed for numeric inputs");
        let flipped = sign_flip_series(&series)
            .isna()
            .expect("Series::isna() must succeed after sign flip");
        prop_assert!(
            approx_equal_series(&baseline, &flipped),
            "series isna(-x) must equal isna(x)"
        );
    }

    /// Filling missing values in a Series with a concrete scalar must eliminate all missing positions.
    #[test]
    fn prop_series_fillna_eliminates_isna_positions(
        series in arb_variable_numeric_series("isna", 12),
        fill in -1_000i64..1_000i64,
    ) {
        let filled = series
            .fillna(&Scalar::Int64(fill))
            .expect("Series::fillna() must succeed for numeric inputs");
        let isna = filled
            .isna()
            .expect("Series::isna() must succeed after fillna");
        let notna = filled
            .notna()
            .expect("Series::notna() must succeed after fillna");
        prop_assert!(
            all_series_bool_values(&isna, false)
                .expect("Series::isna() output must be boolean-valued"),
            "series fillna(v).isna() must be all false"
        );
        prop_assert!(
            all_series_bool_values(&notna, true)
                .expect("Series::notna() output must be boolean-valued"),
            "series fillna(v).notna() must be all true"
        );
    }

    /// `DataFrame::notna()` must be the cellwise boolean complement of `DataFrame::isna()`.
    #[test]
    fn prop_dataframe_notna_complements_isna(
        df in arb_numeric_dataframe(8),
    ) {
        let isna = df
            .isna()
            .expect("DataFrame::isna() must succeed for numeric inputs");
        let notna = df
            .notna()
            .expect("DataFrame::notna() must succeed for numeric inputs");
        let expected = invert_bool_dataframe(&isna)
            .expect("complemented DataFrame::isna() mask must construct");
        prop_assert!(
            approx_equal_dataframe(&notna, &expected),
            "dataframe notna() must equal the cellwise boolean complement of isna()"
        );
    }

    /// Negating numeric cells must not change the `DataFrame::isna()` mask.
    #[test]
    fn prop_dataframe_isna_is_sign_flip_invariant(
        df in arb_numeric_dataframe(8),
    ) {
        let baseline = df
            .isna()
            .expect("DataFrame::isna() must succeed for numeric inputs");
        let flipped = sign_flip_dataframe(&df)
            .isna()
            .expect("DataFrame::isna() must succeed after sign flip");
        prop_assert!(
            approx_equal_dataframe(&baseline, &flipped),
            "dataframe isna(-x) must equal isna(x)"
        );
    }

    /// Filling missing values in a DataFrame with a concrete scalar must eliminate all missing positions.
    #[test]
    fn prop_dataframe_fillna_eliminates_isna_positions(
        df in arb_numeric_dataframe(8),
        fill in -1_000i64..1_000i64,
    ) {
        let filled = df
            .fillna(&Scalar::Int64(fill))
            .expect("DataFrame::fillna() must succeed for numeric inputs");
        let isna = filled
            .isna()
            .expect("DataFrame::isna() must succeed after fillna");
        let notna = filled
            .notna()
            .expect("DataFrame::notna() must succeed after fillna");
        prop_assert!(
            all_dataframe_bool_values(&isna, false)
                .expect("DataFrame::isna() output must be boolean-valued"),
            "dataframe fillna(v).isna() must be all false"
        );
        prop_assert!(
            all_dataframe_bool_values(&notna, true)
                .expect("DataFrame::notna() output must be boolean-valued"),
            "dataframe fillna(v).notna() must be all true"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: count / count_na metamorphic invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Negating numeric values must not change `Series::count()`.
    #[test]
    fn prop_series_count_is_sign_flip_invariant(
        series in arb_variable_numeric_series("count", 12),
    ) {
        let baseline = series.count();
        let flipped = sign_flip_series(&series).count();
        prop_assert_eq!(
            baseline,
            flipped,
            "series count(-x) must equal count(x)"
        );
    }

    /// Non-missing and missing positions partition a Series.
    #[test]
    fn prop_series_count_plus_isna_matches_len(
        series in arb_variable_numeric_series("count", 12),
    ) {
        let missing = bool_values(
            &series
                .isna()
                .expect("Series::isna() must succeed for numeric inputs"),
        )
        .expect("Series::isna() output must be boolean-valued")
        .into_iter()
        .filter(|flag| *flag)
        .count();
        prop_assert_eq!(
            series.count() + missing,
            series.len(),
            "series count() + missing_count must equal len()"
        );
    }

    /// Filling missing values with a concrete scalar must make every Series position countable.
    #[test]
    fn prop_series_fillna_makes_count_equal_len(
        series in arb_variable_numeric_series("count", 12),
        fill in -1_000i64..1_000i64,
    ) {
        let filled = series
            .fillna(&Scalar::Int64(fill))
            .expect("Series::fillna() must succeed for numeric inputs");
        prop_assert_eq!(
            filled.count(),
            series.len(),
            "series fillna(v).count() must equal len()"
        );
    }

    /// Negating numeric cells must not change per-column `DataFrame::count()`.
    #[test]
    fn prop_dataframe_count_is_sign_flip_invariant(
        df in arb_numeric_dataframe(8),
    ) {
        let baseline = df
            .count()
            .expect("DataFrame::count() must succeed for numeric inputs");
        let flipped = sign_flip_dataframe(&df)
            .count()
            .expect("DataFrame::count() must succeed after sign flip");
        prop_assert!(
            approx_equal_series(&baseline, &flipped),
            "dataframe count(-x) must equal count(x)"
        );
    }

    /// Per-column non-missing and missing counts must partition DataFrame rows.
    #[test]
    fn prop_dataframe_count_plus_count_na_matches_row_count(
        df in arb_numeric_dataframe(8),
    ) {
        let count = df
            .count()
            .expect("DataFrame::count() must succeed for numeric inputs");
        let count_na = df
            .count_na()
            .expect("DataFrame::count_na() must succeed for numeric inputs");
        let row_count = i64::try_from(df.index().len()).expect("row count must fit in i64");
        let counts = int64_values(&count).expect("DataFrame::count() must produce Int64 output");
        let na_counts =
            int64_values(&count_na).expect("DataFrame::count_na() must produce Int64 output");
        for (present, missing) in counts.into_iter().zip(na_counts) {
            prop_assert_eq!(
                present + missing,
                row_count,
                "dataframe count() + count_na() must equal row_count for every column"
            );
        }
    }

    /// Filling missing values with a concrete scalar must make every DataFrame cell countable.
    #[test]
    fn prop_dataframe_fillna_sets_count_to_row_count_and_count_na_to_zero(
        df in arb_numeric_dataframe(8),
        fill in -1_000i64..1_000i64,
    ) {
        let filled = df
            .fillna(&Scalar::Int64(fill))
            .expect("DataFrame::fillna() must succeed for numeric inputs");
        let count = filled
            .count()
            .expect("DataFrame::count() must succeed after fillna");
        let count_na = filled
            .count_na()
            .expect("DataFrame::count_na() must succeed after fillna");
        let expected_count = Series::from_values(
            count.name().to_owned(),
            count.index().labels().to_vec(),
            vec![Scalar::Int64(i64::try_from(df.index().len()).expect("row count must fit in i64")); count.len()],
        )
        .expect("expected count series must construct");
        let expected_count_na = Series::from_values(
            count_na.name().to_owned(),
            count_na.index().labels().to_vec(),
            vec![Scalar::Int64(0); count_na.len()],
        )
        .expect("expected count_na series must construct");
        prop_assert!(
            approx_equal_series(&count, &expected_count),
            "dataframe fillna(v).count() must equal row_count per column"
        );
        prop_assert!(
            approx_equal_series(&count_na, &expected_count_na),
            "dataframe fillna(v).count_na() must be zero per column"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: sum metamorphic invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Negating numeric values must negate `Series::sum()`.
    #[test]
    fn prop_series_sum_is_sign_symmetric(
        series in arb_variable_numeric_series("sum", 12),
    ) {
        let baseline = series
            .sum()
            .expect("Series::sum() must succeed for numeric inputs");
        let flipped = sign_flip_series(&series)
            .sum()
            .expect("Series::sum() must succeed after sign flip");
        let expected = sign_flip_numeric_scalar(&baseline);
        prop_assert!(
            approx_equal_scalar(&flipped, &expected),
            "series sum(-x) must equal -sum(x)"
        );
    }

    /// Translating numeric values by `c` must shift `Series::sum()` by `c * count()`.
    #[test]
    fn prop_series_sum_is_translation_covariant(
        series in arb_variable_numeric_series("sum", 12),
        delta in -1_000.0f64..1_000.0,
    ) {
        let baseline = series
            .sum()
            .expect("Series::sum() must succeed for numeric inputs");
        let shifted = shift_series(&series, delta)
            .sum()
            .expect("Series::sum() must succeed after translation");
        let expected = Scalar::Float64(
            baseline
                .to_f64()
                .expect("Series::sum() output must be numeric")
                + delta * series.count() as f64,
        );
        prop_assert!(
            approx_equal_scalar(&shifted, &expected),
            "series sum(x + c) must equal sum(x) + c * count(x)"
        );
    }

    /// Filling missing values with zero must preserve `Series::sum()`.
    #[test]
    fn prop_series_fillna_zero_preserves_sum(
        series in arb_variable_numeric_series("sum", 12),
    ) {
        let baseline = series
            .sum()
            .expect("Series::sum() must succeed for numeric inputs");
        let filled = series
            .fillna(&Scalar::Int64(0))
            .expect("Series::fillna() must succeed for numeric inputs")
            .sum()
            .expect("Series::sum() must succeed after fillna(0)");
        prop_assert!(
            approx_equal_scalar(&baseline, &filled),
            "series fillna(0).sum() must equal sum()"
        );
    }

    /// Negating numeric cells must negate every component of `DataFrame::sum()`.
    #[test]
    fn prop_dataframe_sum_is_sign_symmetric(
        df in arb_numeric_dataframe(8),
    ) {
        let baseline = df
            .sum()
            .expect("DataFrame::sum() must succeed for numeric inputs");
        let flipped = sign_flip_dataframe(&df)
            .sum()
            .expect("DataFrame::sum() must succeed after sign flip");
        let expected = sign_flip_series(&baseline);
        prop_assert!(
            approx_equal_series(&flipped, &expected),
            "dataframe sum(-x) must equal -sum(x) per column"
        );
    }

    /// Translating numeric cells by `c` must shift each column sum by `c * count()`.
    #[test]
    fn prop_dataframe_sum_is_translation_covariant(
        df in arb_numeric_dataframe(8),
        delta in -1_000.0f64..1_000.0,
    ) {
        let baseline = df
            .sum()
            .expect("DataFrame::sum() must succeed for numeric inputs");
        let counts = df
            .count()
            .expect("DataFrame::count() must succeed for numeric inputs");
        let shifted = shift_dataframe(&df, delta)
            .sum()
            .expect("DataFrame::sum() must succeed after translation");
        let expected = expected_shifted_sum_series(&baseline, &counts, delta);
        prop_assert!(
            approx_equal_series(&shifted, &expected),
            "dataframe sum(x + c) must equal sum(x) + c * count(x) per column"
        );
    }

    /// Filling missing values with zero must preserve every component of `DataFrame::sum()`.
    #[test]
    fn prop_dataframe_fillna_zero_preserves_sum(
        df in arb_numeric_dataframe(8),
    ) {
        let baseline = df
            .sum()
            .expect("DataFrame::sum() must succeed for numeric inputs");
        let filled = df
            .fillna(&Scalar::Int64(0))
            .expect("DataFrame::fillna() must succeed for numeric inputs")
            .sum()
            .expect("DataFrame::sum() must succeed after fillna(0)");
        let preserved = Series::from_values(
            baseline.name().to_owned(),
            baseline.index().labels().to_vec(),
            baseline
                .index()
                .labels()
                .iter()
                .map(|label| {
                    scalar_for_label(&filled, label)
                        .expect("fillna(0).sum() must retain every baseline sum label")
                        .clone()
                })
                .collect::<Vec<_>>(),
        )
        .expect("projected fillna(0) sum must construct");
        prop_assert!(
            approx_equal_series(&baseline, &preserved),
            "dataframe fillna(0).sum() must preserve every existing sum component"
        );

        for label in filled.index().labels() {
            if scalar_for_label(&baseline, label).is_none() {
                let extra = scalar_for_label(&filled, label)
                    .expect("filled sum label must resolve to a value");
                prop_assert!(
                    approx_equal_scalar(extra, &Scalar::Float64(0.0)),
                    "fillna(0) may only introduce zero-sum columns into DataFrame::sum() output"
                );
            }
        }
    }

}

// ---------------------------------------------------------------------------
// Property: mean metamorphic invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Negating numeric values must negate `Series::mean()`.
    #[test]
    fn prop_series_mean_is_sign_symmetric(
        series in arb_variable_numeric_series("mean", 12),
    ) {
        let baseline = series
            .mean()
            .expect("Series::mean() must succeed for numeric inputs");
        let flipped = sign_flip_series(&series)
            .mean()
            .expect("Series::mean() must succeed after sign flip");
        let expected = sign_flip_numeric_scalar(&baseline);
        prop_assert!(
            approx_equal_scalar(&flipped, &expected),
            "series mean(-x) must equal -mean(x)"
        );
    }

    /// Translating numeric values by `c` must shift `Series::mean()` by `c`.
    #[test]
    fn prop_series_mean_is_translation_covariant(
        series in arb_variable_numeric_series("mean", 12),
        delta in -1_000.0f64..1_000.0,
    ) {
        let baseline = series
            .mean()
            .expect("Series::mean() must succeed for numeric inputs");
        let shifted = shift_series(&series, delta)
            .mean()
            .expect("Series::mean() must succeed after translation");
        let expected = shift_numeric_scalar(&baseline, delta);
        prop_assert!(
            approx_equal_scalar(&shifted, &expected),
            "series mean(x + c) must equal mean(x) + c"
        );
    }

    /// Scaling numeric values by a positive factor must scale `Series::mean()` by the same factor.
    #[test]
    fn prop_series_mean_scales_linearly(
        series in arb_variable_numeric_series("mean", 12),
        factor in 0.0f64..1_000.0,
    ) {
        let baseline = series
            .mean()
            .expect("Series::mean() must succeed for numeric inputs");
        let scaled = scale_series(&series, factor)
            .mean()
            .expect("Series::mean() must succeed after scaling");
        let expected = scale_numeric_scalar(&baseline, factor);
        prop_assert!(
            approx_equal_scalar(&scaled, &expected),
            "series mean(kx) must equal k * mean(x) for nonnegative k"
        );
    }

    /// Negating numeric cells must negate every component of `DataFrame::mean()`.
    #[test]
    fn prop_dataframe_mean_is_sign_symmetric(
        df in arb_numeric_dataframe(8),
    ) {
        let baseline = df
            .mean()
            .expect("DataFrame::mean() must succeed for numeric inputs");
        let flipped = sign_flip_dataframe(&df)
            .mean()
            .expect("DataFrame::mean() must succeed after sign flip");
        let expected = sign_flip_series(&baseline);
        prop_assert!(
            approx_equal_series(&flipped, &expected),
            "dataframe mean(-x) must equal -mean(x) per column"
        );
    }

    /// Translating numeric cells by `c` must shift every component of `DataFrame::mean()` by `c`.
    #[test]
    fn prop_dataframe_mean_is_translation_covariant(
        df in arb_numeric_dataframe(8),
        delta in -1_000.0f64..1_000.0,
    ) {
        let baseline = df
            .mean()
            .expect("DataFrame::mean() must succeed for numeric inputs");
        let shifted = shift_dataframe(&df, delta)
            .mean()
            .expect("DataFrame::mean() must succeed after translation");
        let expected = shift_series(&baseline, delta);
        prop_assert!(
            approx_equal_series(&shifted, &expected),
            "dataframe mean(x + c) must equal mean(x) + c per column"
        );
    }

    /// Scaling numeric cells by a positive factor must scale every component of `DataFrame::mean()`.
    #[test]
    fn prop_dataframe_mean_scales_linearly(
        df in arb_numeric_dataframe(8),
        factor in 0.0f64..1_000.0,
    ) {
        let baseline = df
            .mean()
            .expect("DataFrame::mean() must succeed for numeric inputs");
        let scaled = scale_dataframe(&df, factor)
            .mean()
            .expect("DataFrame::mean() must succeed after scaling");
        let expected = scale_series(&baseline, factor);
        prop_assert!(
            approx_equal_series(&scaled, &expected),
            "dataframe mean(kx) must equal k * mean(x) per column for nonnegative k"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: median metamorphic invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Negating numeric values must negate `Series::median()`.
    #[test]
    fn prop_series_median_is_sign_symmetric(
        series in arb_variable_numeric_series("median", 12),
    ) {
        let baseline = series
            .median()
            .expect("Series::median() must succeed for numeric inputs");
        let flipped = sign_flip_series(&series)
            .median()
            .expect("Series::median() must succeed after sign flip");
        let expected = sign_flip_numeric_scalar(&baseline);
        prop_assert!(
            approx_equal_scalar(&flipped, &expected),
            "series median(-x) must equal -median(x)"
        );
    }

    /// Translating numeric values by `c` must shift `Series::median()` by `c`.
    #[test]
    fn prop_series_median_is_translation_covariant(
        series in arb_variable_numeric_series("median", 12),
        delta in -1_000.0f64..1_000.0,
    ) {
        let baseline = series
            .median()
            .expect("Series::median() must succeed for numeric inputs");
        let shifted = shift_series(&series, delta)
            .median()
            .expect("Series::median() must succeed after translation");
        let expected = shift_numeric_scalar(&baseline, delta);
        prop_assert!(
            approx_equal_scalar(&shifted, &expected),
            "series median(x + c) must equal median(x) + c"
        );
    }

    /// Scaling numeric values by a positive factor must scale `Series::median()` by the same factor.
    #[test]
    fn prop_series_median_scales_linearly(
        series in arb_variable_numeric_series("median", 12),
        factor in 0.0f64..1_000.0,
    ) {
        let baseline = series
            .median()
            .expect("Series::median() must succeed for numeric inputs");
        let scaled = scale_series(&series, factor)
            .median()
            .expect("Series::median() must succeed after scaling");
        let expected = scale_numeric_scalar(&baseline, factor);
        prop_assert!(
            approx_equal_scalar(&scaled, &expected),
            "series median(kx) must equal k * median(x) for nonnegative k"
        );
    }

    /// Negating numeric cells must negate every component of `DataFrame::median_agg()`.
    #[test]
    fn prop_dataframe_median_is_sign_symmetric(
        df in arb_numeric_dataframe(8),
    ) {
        let baseline = df
            .median_agg()
            .expect("DataFrame::median_agg() must succeed for numeric inputs");
        let flipped = sign_flip_dataframe(&df)
            .median_agg()
            .expect("DataFrame::median_agg() must succeed after sign flip");
        let expected = sign_flip_series(&baseline);
        prop_assert!(
            approx_equal_series(&flipped, &expected),
            "dataframe median_agg(-x) must equal -median_agg(x) per column"
        );
    }

    /// Translating numeric cells by `c` must shift every component of `DataFrame::median_agg()` by `c`.
    #[test]
    fn prop_dataframe_median_is_translation_covariant(
        df in arb_numeric_dataframe(8),
        delta in -1_000.0f64..1_000.0,
    ) {
        let baseline = df
            .median_agg()
            .expect("DataFrame::median_agg() must succeed for numeric inputs");
        let shifted = shift_dataframe(&df, delta)
            .median_agg()
            .expect("DataFrame::median_agg() must succeed after translation");
        let expected = shift_series(&baseline, delta);
        prop_assert!(
            approx_equal_series(&shifted, &expected),
            "dataframe median_agg(x + c) must equal median_agg(x) + c per column"
        );
    }

    /// Scaling numeric cells by a positive factor must scale every component of `DataFrame::median_agg()`.
    #[test]
    fn prop_dataframe_median_scales_linearly(
        df in arb_numeric_dataframe(8),
        factor in 0.0f64..1_000.0,
    ) {
        let baseline = df
            .median_agg()
            .expect("DataFrame::median_agg() must succeed for numeric inputs");
        let scaled = scale_dataframe(&df, factor)
            .median_agg()
            .expect("DataFrame::median_agg() must succeed after scaling");
        let expected = scale_series(&baseline, factor);
        prop_assert!(
            approx_equal_series(&scaled, &expected),
            "dataframe median_agg(kx) must equal k * median_agg(x) per column for nonnegative k"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: std / var metamorphic invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Negating numeric values must preserve `Series::std()`.
    #[test]
    fn prop_series_std_is_sign_invariant(
        series in arb_variable_numeric_series("std", 12),
    ) {
        let baseline = series
            .std()
            .expect("Series::std() must succeed for numeric inputs");
        let flipped = sign_flip_series(&series)
            .std()
            .expect("Series::std() must succeed after sign flip");
        prop_assert!(
            approx_equal_scalar(&flipped, &baseline),
            "series std(-x) must equal std(x)"
        );
    }

    /// Translating numeric values by `c` must preserve `Series::std()`.
    #[test]
    fn prop_series_std_is_translation_invariant(
        series in arb_variable_numeric_series("std", 12),
        delta in -1_000.0f64..1_000.0,
    ) {
        let baseline = series
            .std()
            .expect("Series::std() must succeed for numeric inputs");
        let shifted = shift_series(&series, delta)
            .std()
            .expect("Series::std() must succeed after translation");
        prop_assert!(
            approx_equal_scalar(&shifted, &baseline),
            "series std(x + c) must equal std(x)"
        );
    }

    /// Scaling numeric values by a nonnegative factor must scale `Series::std()` linearly.
    #[test]
    fn prop_series_std_scales_linearly(
        series in arb_variable_numeric_series("std", 12),
        factor in 0.0f64..1_000.0,
    ) {
        let baseline = series
            .std()
            .expect("Series::std() must succeed for numeric inputs");
        let scaled = scale_series(&series, factor)
            .std()
            .expect("Series::std() must succeed after scaling");
        let expected = scale_numeric_scalar(&baseline, factor);
        prop_assert!(
            approx_equal_scalar(&scaled, &expected),
            "series std(kx) must equal k * std(x) for nonnegative k"
        );
    }

    /// Negating numeric values must preserve `Series::var()`.
    #[test]
    fn prop_series_var_is_sign_invariant(
        series in arb_variable_numeric_series("var", 12),
    ) {
        let baseline = series
            .var()
            .expect("Series::var() must succeed for numeric inputs");
        let flipped = sign_flip_series(&series)
            .var()
            .expect("Series::var() must succeed after sign flip");
        prop_assert!(
            approx_equal_scalar(&flipped, &baseline),
            "series var(-x) must equal var(x)"
        );
    }

    /// Translating numeric values by `c` must preserve `Series::var()`.
    #[test]
    fn prop_series_var_is_translation_invariant(
        series in arb_variable_numeric_series("var", 12),
        delta in -1_000.0f64..1_000.0,
    ) {
        let baseline = series
            .var()
            .expect("Series::var() must succeed for numeric inputs");
        let shifted = shift_series(&series, delta)
            .var()
            .expect("Series::var() must succeed after translation");
        prop_assert!(
            approx_equal_scalar(&shifted, &baseline),
            "series var(x + c) must equal var(x)"
        );
    }

    /// Scaling numeric values by a factor must scale `Series::var()` by `k^2`.
    #[test]
    fn prop_series_var_scales_quadratically(
        series in arb_variable_numeric_series("var", 12),
        factor in -1_000.0f64..1_000.0,
    ) {
        let baseline = series
            .var()
            .expect("Series::var() must succeed for numeric inputs");
        let scaled = scale_series(&series, factor)
            .var()
            .expect("Series::var() must succeed after scaling");
        let expected = scale_numeric_scalar(&baseline, factor * factor);
        prop_assert!(
            approx_equal_scalar(&scaled, &expected),
            "series var(kx) must equal k^2 * var(x)"
        );
    }

    /// Negating numeric cells must preserve every component of `DataFrame::std()`.
    #[test]
    fn prop_dataframe_std_is_sign_invariant(
        df in arb_numeric_dataframe(8),
    ) {
        let baseline = df
            .std_agg()
            .expect("DataFrame::std() must succeed for numeric inputs");
        let flipped = sign_flip_dataframe(&df)
            .std_agg()
            .expect("DataFrame::std() must succeed after sign flip");
        prop_assert!(
            approx_equal_series(&flipped, &baseline),
            "dataframe std(-x) must equal std(x) per column"
        );
    }

    /// Translating numeric cells by `c` must preserve every component of `DataFrame::std()`.
    #[test]
    fn prop_dataframe_std_is_translation_invariant(
        df in arb_numeric_dataframe(8),
        delta in -1_000.0f64..1_000.0,
    ) {
        let baseline = df
            .std_agg()
            .expect("DataFrame::std() must succeed for numeric inputs");
        let shifted = shift_dataframe(&df, delta)
            .std_agg()
            .expect("DataFrame::std() must succeed after translation");
        prop_assert!(
            approx_equal_series(&shifted, &baseline),
            "dataframe std(x + c) must equal std(x) per column"
        );
    }

    /// Scaling numeric cells by a nonnegative factor must scale every component of `DataFrame::std()` linearly.
    #[test]
    fn prop_dataframe_std_scales_linearly(
        df in arb_numeric_dataframe(8),
        factor in 0.0f64..1_000.0,
    ) {
        let baseline = df
            .std_agg()
            .expect("DataFrame::std() must succeed for numeric inputs");
        let scaled = scale_dataframe(&df, factor)
            .std_agg()
            .expect("DataFrame::std() must succeed after scaling");
        let expected = scale_series(&baseline, factor);
        prop_assert!(
            approx_equal_series(&scaled, &expected),
            "dataframe std(kx) must equal k * std(x) per column for nonnegative k"
        );
    }

    /// Negating numeric cells must preserve every component of `DataFrame::var()`.
    #[test]
    fn prop_dataframe_var_is_sign_invariant(
        df in arb_numeric_dataframe(8),
    ) {
        let baseline = df
            .var_agg()
            .expect("DataFrame::var() must succeed for numeric inputs");
        let flipped = sign_flip_dataframe(&df)
            .var_agg()
            .expect("DataFrame::var() must succeed after sign flip");
        prop_assert!(
            approx_equal_series(&flipped, &baseline),
            "dataframe var(-x) must equal var(x) per column"
        );
    }

    /// Translating numeric cells by `c` must preserve every component of `DataFrame::var()`.
    #[test]
    fn prop_dataframe_var_is_translation_invariant(
        df in arb_numeric_dataframe(8),
        delta in -1_000.0f64..1_000.0,
    ) {
        let baseline = df
            .var_agg()
            .expect("DataFrame::var() must succeed for numeric inputs");
        let shifted = shift_dataframe(&df, delta)
            .var_agg()
            .expect("DataFrame::var() must succeed after translation");
        prop_assert!(
            approx_equal_series(&shifted, &baseline),
            "dataframe var(x + c) must equal var(x) per column"
        );
    }

    /// Scaling numeric cells by a factor must scale every component of `DataFrame::var()` by `k^2`.
    #[test]
    fn prop_dataframe_var_scales_quadratically(
        df in arb_numeric_dataframe(8),
        factor in -1_000.0f64..1_000.0,
    ) {
        let baseline = df
            .var_agg()
            .expect("DataFrame::var() must succeed for numeric inputs");
        let scaled = scale_dataframe(&df, factor)
            .var_agg()
            .expect("DataFrame::var() must succeed after scaling");
        let expected = scale_series(&baseline, factor * factor);
        prop_assert!(
            approx_equal_series(&scaled, &expected),
            "dataframe var(kx) must equal k^2 * var(x) per column"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: isin metamorphic invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Reordering membership candidates must not change Series::isin.
    #[test]
    fn prop_series_isin_is_permutation_invariant(
        series in arb_replace_numeric_series("isin", 12),
        mut values in arb_isin_test_values(8),
    ) {
        let baseline = series
            .isin(&values)
            .expect("Series::isin() must succeed for numeric and missing values");
        values.reverse();
        let permuted = series
            .isin(&values)
            .expect("Series::isin() must succeed after candidate permutation");
        prop_assert!(
            baseline.equals(&permuted),
            "series isin(values) must be invariant under value permutation"
        );
    }

    /// Duplicating membership candidates must not change Series::isin.
    #[test]
    fn prop_series_isin_is_duplicate_invariant(
        series in arb_replace_numeric_series("isin", 12),
        values in arb_isin_test_values(8),
    ) {
        let baseline = series
            .isin(&values)
            .expect("Series::isin() must succeed for numeric and missing values");
        let mut duplicated_values = values.clone();
        duplicated_values.extend(values);
        let duplicated = series
            .isin(&duplicated_values)
            .expect("Series::isin() must succeed after duplicating candidates");
        prop_assert!(
            baseline.equals(&duplicated),
            "series isin(values) must be invariant under duplicate candidates"
        );
    }

    /// Enlarging the candidate set can only turn false results into true ones for Series::isin.
    #[test]
    fn prop_series_isin_is_superset_monotone(
        series in arb_replace_numeric_series("isin", 12),
        base_values in arb_isin_test_values(6),
        extra_values in arb_isin_test_values(6),
    ) {
        let baseline = series
            .isin(&base_values)
            .expect("Series::isin() must succeed for baseline candidates");
        let mut superset = base_values.clone();
        superset.extend(extra_values);
        let expanded = series
            .isin(&superset)
            .expect("Series::isin() must succeed for superset candidates");
        let baseline_values = bool_values(&baseline).expect("Series::isin() must produce bool output");
        let expanded_values = bool_values(&expanded).expect("Series::isin() must produce bool output");
        for (baseline_value, expanded_value) in baseline_values.into_iter().zip(expanded_values) {
            prop_assert!(
                !baseline_value || expanded_value,
                "series isin superset expansion must not flip true back to false"
            );
        }
    }

    /// DataFrame::isin must agree with applying Series::isin column-wise.
    #[test]
    fn prop_dataframe_isin_matches_columnwise_series(
        df in arb_replace_numeric_dataframe(8),
        values in arb_isin_test_values(8),
    ) {
        let observed = df
            .isin(&values)
            .expect("DataFrame::isin() must succeed for numeric and missing values");
        let expected = dataframe_isin_via_series(&df, &values);
        prop_assert!(
            approx_equal_dataframe(&observed, &expected),
            "dataframe isin(values) must equal applying Series::isin(values) to each column"
        );
    }

    /// DataFrame::isin_dict must agree with applying Series::isin on each column's own candidate set.
    #[test]
    fn prop_dataframe_isin_dict_matches_columnwise_series(
        df in arb_replace_numeric_dataframe(8),
        include_a in proptest::bool::ANY,
        include_b in proptest::bool::ANY,
        values_a in arb_isin_test_values(8),
        values_b in arb_isin_test_values(8),
    ) {
        let mut per_column = std::collections::BTreeMap::new();
        if include_a {
            per_column.insert("a".to_string(), values_a);
        }
        if include_b {
            per_column.insert("b".to_string(), values_b);
        }

        let observed = df
            .isin_dict(&per_column)
            .expect("DataFrame::isin_dict() must succeed for per-column values");
        let expected = dataframe_isin_dict_via_series(&df, &per_column);
        prop_assert!(
            approx_equal_dataframe(&observed, &expected),
            "dataframe isin_dict(per_column) must equal per-column Series::isin()"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: duplicated / drop_duplicates metamorphic invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Sign-flipping every numeric value preserves Series duplicate equivalence classes.
    #[test]
    fn prop_series_duplicated_is_sign_flip_invariant(
        series in arb_replace_numeric_series("duplicated", 12),
    ) {
        let baseline = series
            .duplicated()
            .expect("Series::duplicated() must succeed for numeric and missing values");
        let flipped = sign_flip_series(&series)
            .duplicated()
            .expect("Series::duplicated() must succeed after sign flip");
        prop_assert!(
            baseline.equals(&flipped),
            "series duplicated(-x) must equal duplicated(x)"
        );
    }

    /// Dropping duplicates twice is stable for Series.
    #[test]
    fn prop_series_drop_duplicates_is_idempotent(
        series in arb_replace_numeric_series("drop_duplicates", 12),
    ) {
        let once = series
            .drop_duplicates()
            .expect("Series::drop_duplicates() must succeed for numeric and missing values");
        let twice = once
            .drop_duplicates()
            .expect("Series::drop_duplicates() must stay stable after duplicates are removed");
        prop_assert!(
            approx_equal_series(&once, &twice),
            "series drop_duplicates() must be idempotent"
        );
    }

    /// Sign-flipping a Series commutes with dropping duplicates.
    #[test]
    fn prop_series_drop_duplicates_is_sign_flip_covariant(
        series in arb_replace_numeric_series("drop_duplicates", 12),
    ) {
        let baseline = series
            .drop_duplicates()
            .expect("Series::drop_duplicates() must succeed for numeric and missing values");
        let flipped_result = sign_flip_series(&series)
            .drop_duplicates()
            .expect("Series::drop_duplicates() must succeed after sign flip");
        let expected = sign_flip_series(&baseline);
        prop_assert!(
            approx_equal_series(&flipped_result, &expected),
            "series drop_duplicates(-x) must equal -drop_duplicates(x)"
        );
    }

    /// A deduplicated Series must contain no further duplicates.
    #[test]
    fn prop_series_drop_duplicates_produces_unique_values(
        series in arb_replace_numeric_series("drop_duplicates", 12),
    ) {
        let deduped = series
            .drop_duplicates()
            .expect("Series::drop_duplicates() must succeed for numeric and missing values");
        let flags = bool_values(
            &deduped
                .duplicated()
                .expect("Series::duplicated() must succeed on deduplicated values"),
        )
        .expect("Series::duplicated() must produce bool output");
        prop_assert!(
            flags.into_iter().all(|flag| !flag),
            "series drop_duplicates() output must have no remaining duplicates"
        );
    }

    /// Sign-flipping every numeric cell preserves full-row duplicate equivalence classes.
    #[test]
    fn prop_dataframe_duplicated_is_sign_flip_invariant(
        df in arb_replace_numeric_dataframe(8),
    ) {
        let baseline = df
            .duplicated(None, DuplicateKeep::First)
            .expect("DataFrame::duplicated() must succeed for numeric and missing values");
        let flipped = sign_flip_dataframe(&df)
            .duplicated(None, DuplicateKeep::First)
            .expect("DataFrame::duplicated() must succeed after sign flip");
        prop_assert!(
            baseline.equals(&flipped),
            "dataframe duplicated(-x) must equal duplicated(x)"
        );
    }

    /// Duplicate detection on a subset must ignore translations to non-selected columns.
    #[test]
    fn prop_dataframe_duplicated_subset_ignores_unselected_column_translation(
        df in arb_replace_numeric_dataframe(8),
        delta in -10.0f64..10.0,
    ) {
        let subset = vec!["a".to_string()];
        let baseline = df
            .duplicated(Some(&subset), DuplicateKeep::First)
            .expect("DataFrame::duplicated(subset) must succeed");
        let shifted = shift_dataframe_column(&df, "b", delta)
            .duplicated(Some(&subset), DuplicateKeep::First)
            .expect("DataFrame::duplicated(subset) must ignore non-selected column changes");
        prop_assert!(
            baseline.equals(&shifted),
            "dataframe duplicated(subset=['a']) must ignore changes to column b"
        );
    }

    /// Dropping duplicated rows twice is stable for DataFrames.
    #[test]
    fn prop_dataframe_drop_duplicates_is_idempotent(
        df in arb_replace_numeric_dataframe(8),
    ) {
        let once = df
            .drop_duplicates(None, DuplicateKeep::First, false)
            .expect("DataFrame::drop_duplicates() must succeed for numeric and missing values");
        let twice = once
            .drop_duplicates(None, DuplicateKeep::First, false)
            .expect("DataFrame::drop_duplicates() must stay stable after duplicates are removed");
        prop_assert!(
            approx_equal_dataframe(&once, &twice),
            "dataframe drop_duplicates() must be idempotent"
        );
    }

    /// Sign-flipping every numeric cell commutes with dropping duplicated rows.
    #[test]
    fn prop_dataframe_drop_duplicates_is_sign_flip_covariant(
        df in arb_replace_numeric_dataframe(8),
    ) {
        let baseline = df
            .drop_duplicates(None, DuplicateKeep::First, false)
            .expect("DataFrame::drop_duplicates() must succeed for numeric and missing values");
        let flipped_result = sign_flip_dataframe(&df)
            .drop_duplicates(None, DuplicateKeep::First, false)
            .expect("DataFrame::drop_duplicates() must succeed after sign flip");
        let expected = sign_flip_dataframe(&baseline);
        prop_assert!(
            approx_equal_dataframe(&flipped_result, &expected),
            "dataframe drop_duplicates(-x) must equal -drop_duplicates(x)"
        );
    }

    /// Subset-based row selection must ignore translations to non-selected columns.
    #[test]
    fn prop_dataframe_drop_duplicates_subset_ignores_unselected_column_translation(
        df in arb_replace_numeric_dataframe(8),
        delta in -10.0f64..10.0,
    ) {
        let subset = vec!["a".to_string()];
        let baseline = df
            .drop_duplicates(Some(&subset), DuplicateKeep::First, false)
            .expect("DataFrame::drop_duplicates(subset) must succeed");
        let shifted_input = shift_dataframe_column(&df, "b", delta);
        let shifted_result = shifted_input
            .drop_duplicates(Some(&subset), DuplicateKeep::First, false)
            .expect("DataFrame::drop_duplicates(subset) must ignore non-selected column changes");
        let expected = shift_dataframe_column(&baseline, "b", delta);
        prop_assert!(
            approx_equal_dataframe(&shifted_result, &expected),
            "dataframe drop_duplicates(subset=['a']) must ignore changes to column b except on kept rows"
        );
    }

    /// A deduplicated DataFrame must contain no further duplicates.
    #[test]
    fn prop_dataframe_drop_duplicates_produces_unique_rows(
        df in arb_replace_numeric_dataframe(8),
    ) {
        let deduped = df
            .drop_duplicates(None, DuplicateKeep::First, false)
            .expect("DataFrame::drop_duplicates() must succeed for numeric and missing values");
        let flags = bool_values(
            &deduped
                .duplicated(None, DuplicateKeep::First)
                .expect("DataFrame::duplicated() must succeed on deduplicated rows"),
        )
        .expect("DataFrame::duplicated() must produce bool output");
        prop_assert!(
            flags.into_iter().all(|flag| !flag),
            "dataframe drop_duplicates() output must have no remaining duplicated rows"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: replace metamorphic invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// A single Series replacement pair is idempotent.
    #[test]
    fn prop_series_replace_single_pair_is_idempotent(
        series in arb_replace_numeric_series("replace", 12),
        old in arb_replace_numeric_scalar(),
        new in arb_replace_numeric_scalar(),
    ) {
        prop_assume!(!old.is_missing());
        let replacements = vec![(old, new)];
        let once = series
            .replace(&replacements)
            .expect("Series::replace() must succeed for scalar pairs");
        let twice = once
            .replace(&replacements)
            .expect("Series::replace() must remain stable on the replaced series");
        prop_assert!(
            approx_equal_series(&once, &twice),
            "series replace(old -> new) must be idempotent"
        );
    }

    /// Translating both a Series and its replacement pair by the same offset must commute with replace.
    #[test]
    fn prop_series_replace_is_translation_covariant(
        series in arb_replace_numeric_series("replace", 12),
        old in arb_replace_numeric_scalar(),
        new in arb_replace_numeric_scalar(),
        delta in -10.0f64..10.0,
    ) {
        prop_assume!(!old.is_missing() && !new.is_missing());
        let replacements = vec![(old.clone(), new.clone())];
        let baseline = series
            .replace(&replacements)
            .expect("Series::replace() must succeed for scalar pairs");
        let shifted_input = shift_series(&series, delta);
        let shifted_replacements = vec![(
            shift_numeric_scalar(&old, delta),
            shift_numeric_scalar(&new, delta),
        )];
        let shifted_result = shifted_input
            .replace(&shifted_replacements)
            .expect("Series::replace() must succeed after translation");
        let expected = shift_series(&baseline, delta);
        prop_assert!(
            approx_equal_series(&shifted_result, &expected),
            "series replace(x + c, old + c -> new + c) must equal replace(x, old -> new) + c"
        );
    }

    /// Negating both a Series and its replacement pair must commute with replace.
    #[test]
    fn prop_series_replace_is_sign_symmetric(
        series in arb_replace_numeric_series("replace", 12),
        old in arb_replace_numeric_scalar(),
        new in arb_replace_numeric_scalar(),
    ) {
        prop_assume!(!old.is_missing() && !new.is_missing());
        let replacements = vec![(old.clone(), new.clone())];
        let baseline = series
            .replace(&replacements)
            .expect("Series::replace() must succeed for scalar pairs");
        let flipped_input = sign_flip_series(&series);
        let flipped_replacements = vec![(
            sign_flip_numeric_scalar(&old),
            sign_flip_numeric_scalar(&new),
        )];
        let flipped_result = flipped_input
            .replace(&flipped_replacements)
            .expect("Series::replace() must succeed after sign flip");
        let expected = sign_flip_series(&baseline);
        prop_assert!(
            approx_equal_series(&flipped_result, &expected),
            "series replace(-x, -old -> -new) must equal -replace(x, old -> new)"
        );
    }

    /// A single DataFrame replacement pair is idempotent.
    #[test]
    fn prop_dataframe_replace_single_pair_is_idempotent(
        df in arb_replace_numeric_dataframe(8),
        old in arb_replace_numeric_scalar(),
        new in arb_replace_numeric_scalar(),
    ) {
        prop_assume!(!old.is_missing());
        let replacements = vec![(old, new)];
        let once = df
            .replace(&replacements)
            .expect("DataFrame::replace() must succeed for scalar pairs");
        let twice = once
            .replace(&replacements)
            .expect("DataFrame::replace() must remain stable on the replaced frame");
        prop_assert!(
            approx_equal_dataframe(&once, &twice),
            "dataframe replace(old -> new) must be idempotent"
        );
    }

    /// Translating both a DataFrame and its replacement pair by the same offset must commute with replace.
    #[test]
    fn prop_dataframe_replace_is_translation_covariant(
        df in arb_replace_numeric_dataframe(8),
        old in arb_replace_numeric_scalar(),
        new in arb_replace_numeric_scalar(),
        delta in -10.0f64..10.0,
    ) {
        prop_assume!(!old.is_missing() && !new.is_missing());
        let replacements = vec![(old.clone(), new.clone())];
        let baseline = df
            .replace(&replacements)
            .expect("DataFrame::replace() must succeed for scalar pairs");
        let shifted_input = shift_dataframe(&df, delta);
        let shifted_replacements = vec![(
            shift_numeric_scalar(&old, delta),
            shift_numeric_scalar(&new, delta),
        )];
        let shifted_result = shifted_input
            .replace(&shifted_replacements)
            .expect("DataFrame::replace() must succeed after translation");
        let expected = shift_dataframe(&baseline, delta);
        prop_assert!(
            approx_equal_dataframe(&shifted_result, &expected),
            "dataframe replace(x + c, old + c -> new + c) must equal replace(x, old -> new) + c"
        );
    }

    /// Negating both a DataFrame and its replacement pair must commute with replace.
    #[test]
    fn prop_dataframe_replace_is_sign_symmetric(
        df in arb_replace_numeric_dataframe(8),
        old in arb_replace_numeric_scalar(),
        new in arb_replace_numeric_scalar(),
    ) {
        prop_assume!(!old.is_missing() && !new.is_missing());
        let replacements = vec![(old.clone(), new.clone())];
        let baseline = df
            .replace(&replacements)
            .expect("DataFrame::replace() must succeed for scalar pairs");
        let flipped_input = sign_flip_dataframe(&df);
        let flipped_replacements = vec![(
            sign_flip_numeric_scalar(&old),
            sign_flip_numeric_scalar(&new),
        )];
        let flipped_result = flipped_input
            .replace(&flipped_replacements)
            .expect("DataFrame::replace() must succeed after sign flip");
        let expected = sign_flip_dataframe(&baseline);
        prop_assert!(
            approx_equal_dataframe(&flipped_result, &expected),
            "dataframe replace(-x, -old -> -new) must equal -replace(x, old -> new)"
        );
    }

    /// Uniform scalar replacement must agree with per-column dict replacement using the same pairs.
    #[test]
    fn prop_dataframe_replace_matches_uniform_replace_dict(
        df in arb_replace_numeric_dataframe(8),
        old in arb_replace_numeric_scalar(),
        new in arb_replace_numeric_scalar(),
    ) {
        let replacements = vec![(old, new)];
        let scalar_replaced = df
            .replace(&replacements)
            .expect("DataFrame::replace() must succeed for scalar pairs");
        let per_column = uniform_replace_dict(&df, &replacements);
        let dict_replaced = df
            .replace_dict(&per_column)
            .expect("DataFrame::replace_dict() must succeed for existing columns");
        prop_assert!(
            approx_equal_dataframe(&scalar_replaced, &dict_replaced),
            "dataframe replace() must equal replace_dict() when every column gets the same replacements"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: fillna metamorphic invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Filling missing values with the same scalar twice must be idempotent for Series.
    #[test]
    fn prop_series_fillna_is_idempotent(
        series in arb_variable_numeric_series("fillna", 12),
        fill in -1_000i64..1_000i64,
    ) {
        let fill_scalar = Scalar::Int64(fill);
        let once = series
            .fillna(&fill_scalar)
            .expect("Series::fillna() must succeed for numeric inputs");
        let twice = once
            .fillna(&fill_scalar)
            .expect("Series::fillna() must succeed on an already filled series");
        prop_assert!(
            once.equals(&twice),
            "series fillna(v) must be idempotent"
        );
    }

    /// Translating a Series and its scalar fill value by the same offset commutes with fillna.
    #[test]
    fn prop_series_fillna_is_translation_covariant(
        series in arb_variable_numeric_series("fillna", 12),
        fill in -1_000i64..1_000i64,
        delta in -1_000i64..1_000i64,
    ) {
        let fill_scalar = Scalar::Int64(fill);
        let baseline = series
            .fillna(&fill_scalar)
            .expect("Series::fillna() must succeed for numeric inputs");
        let delta = delta as f64;
        let shifted_input = shift_series(&series, delta);
        let shifted_fill = shift_numeric_scalar(&fill_scalar, delta);
        let shifted_result = shifted_input
            .fillna(&shifted_fill)
            .expect("Series::fillna() must succeed after translation");
        let expected = shift_series(&baseline, delta);
        prop_assert!(
            approx_equal_series(&shifted_result, &expected),
            "series fillna(x + c, v + c) must equal fillna(x, v) + c"
        );
    }

    /// Negating both a Series and its scalar fill value commutes with fillna.
    #[test]
    fn prop_series_fillna_is_sign_symmetric(
        series in arb_variable_numeric_series("fillna", 12),
        fill in -1_000i64..1_000i64,
    ) {
        let fill_scalar = Scalar::Int64(fill);
        let baseline = series
            .fillna(&fill_scalar)
            .expect("Series::fillna() must succeed for numeric inputs");
        let flipped_input = sign_flip_series(&series);
        let flipped_fill = sign_flip_numeric_scalar(&fill_scalar);
        let flipped_result = flipped_input
            .fillna(&flipped_fill)
            .expect("Series::fillna() must succeed after sign flip");
        let expected = sign_flip_series(&baseline);
        prop_assert!(
            approx_equal_series(&flipped_result, &expected),
            "series fillna(-x, -v) must equal -fillna(x, v)"
        );
    }

    /// A zero fill limit must leave Series unchanged.
    #[test]
    fn prop_series_fillna_limit_zero_is_identity(
        series in arb_variable_numeric_series("fillna", 12),
        fill in -1_000i64..1_000i64,
    ) {
        let fill_scalar = Scalar::Int64(fill);
        let result = series
            .fillna_limit(&fill_scalar, 0)
            .expect("Series::fillna_limit(..., 0) must succeed");
        prop_assert!(
            series.equals(&result),
            "series fillna_limit(v, 0) must be identity"
        );
    }

    /// Any limit at least as large as the Series length must match unbounded fillna.
    #[test]
    fn prop_series_fillna_limit_len_matches_unbounded_fillna(
        series in arb_variable_numeric_series("fillna", 12),
        fill in -1_000i64..1_000i64,
    ) {
        let fill_scalar = Scalar::Int64(fill);
        let limited = series
            .fillna_limit(&fill_scalar, series.len())
            .expect("Series::fillna_limit() must succeed for a saturating limit");
        let unbounded = series
            .fillna(&fill_scalar)
            .expect("Series::fillna() must succeed for numeric inputs");
        prop_assert!(
            limited.equals(&unbounded),
            "series fillna_limit(v, len) must equal fillna(v)"
        );
    }

    /// Filling missing values with the same scalar twice must be idempotent for DataFrames.
    #[test]
    fn prop_dataframe_fillna_is_idempotent(
        df in arb_numeric_dataframe(8),
        fill in -1_000i64..1_000i64,
    ) {
        let fill_scalar = Scalar::Int64(fill);
        let once = df
            .fillna(&fill_scalar)
            .expect("DataFrame::fillna() must succeed for numeric inputs");
        let twice = once
            .fillna(&fill_scalar)
            .expect("DataFrame::fillna() must succeed on an already filled frame");
        prop_assert!(
            once.equals(&twice),
            "dataframe fillna(v) must be idempotent"
        );
    }

    /// Translating a DataFrame and its scalar fill value by the same offset commutes with fillna.
    #[test]
    fn prop_dataframe_fillna_is_translation_covariant(
        df in arb_numeric_dataframe(8),
        fill in -1_000i64..1_000i64,
        delta in -1_000i64..1_000i64,
    ) {
        let fill_scalar = Scalar::Int64(fill);
        let baseline = df
            .fillna(&fill_scalar)
            .expect("DataFrame::fillna() must succeed for numeric inputs");
        let delta = delta as f64;
        let shifted_input = shift_dataframe(&df, delta);
        let shifted_fill = shift_numeric_scalar(&fill_scalar, delta);
        let shifted_result = shifted_input
            .fillna(&shifted_fill)
            .expect("DataFrame::fillna() must succeed after translation");
        let expected = shift_dataframe(&baseline, delta);
        prop_assert!(
            approx_equal_dataframe(&shifted_result, &expected),
            "dataframe fillna(x + c, v + c) must equal fillna(x, v) + c"
        );
    }

    /// Negating both a DataFrame and its scalar fill value commutes with fillna.
    #[test]
    fn prop_dataframe_fillna_is_sign_symmetric(
        df in arb_numeric_dataframe(8),
        fill in -1_000i64..1_000i64,
    ) {
        let fill_scalar = Scalar::Int64(fill);
        let baseline = df
            .fillna(&fill_scalar)
            .expect("DataFrame::fillna() must succeed for numeric inputs");
        let flipped_input = sign_flip_dataframe(&df);
        let flipped_fill = sign_flip_numeric_scalar(&fill_scalar);
        let flipped_result = flipped_input
            .fillna(&flipped_fill)
            .expect("DataFrame::fillna() must succeed after sign flip");
        let expected = sign_flip_dataframe(&baseline);
        prop_assert!(
            approx_equal_dataframe(&flipped_result, &expected),
            "dataframe fillna(-x, -v) must equal -fillna(x, v)"
        );
    }

    /// A zero fill limit must leave DataFrames unchanged.
    #[test]
    fn prop_dataframe_fillna_limit_zero_is_identity(
        df in arb_numeric_dataframe(8),
        fill in -1_000i64..1_000i64,
    ) {
        let fill_scalar = Scalar::Int64(fill);
        let result = df
            .fillna_limit(&fill_scalar, 0)
            .expect("DataFrame::fillna_limit(..., 0) must succeed");
        prop_assert!(
            df.equals(&result),
            "dataframe fillna_limit(v, 0) must be identity"
        );
    }

    /// Any limit at least as large as the row count must match unbounded fillna for DataFrames.
    #[test]
    fn prop_dataframe_fillna_limit_row_count_matches_unbounded_fillna(
        df in arb_numeric_dataframe(8),
        fill in -1_000i64..1_000i64,
    ) {
        let fill_scalar = Scalar::Int64(fill);
        let limited = df
            .fillna_limit(&fill_scalar, df.index().len())
            .expect("DataFrame::fillna_limit() must succeed for a saturating limit");
        let unbounded = df
            .fillna(&fill_scalar)
            .expect("DataFrame::fillna() must succeed for numeric inputs");
        prop_assert!(
            limited.equals(&unbounded),
            "dataframe fillna_limit(v, row_count) must equal fillna(v)"
        );
    }

    /// A uniform per-column fill map must agree with scalar fillna.
    #[test]
    fn prop_dataframe_fillna_dict_constant_map_matches_scalar_fill(
        df in arb_numeric_dataframe(8),
        fill in -1_000i64..1_000i64,
    ) {
        let fill_scalar = Scalar::Int64(fill);
        let mut fill_map = std::collections::BTreeMap::new();
        for name in df.column_names() {
            fill_map.insert(name.clone(), fill_scalar.clone());
        }
        let dict_filled = df
            .fillna_dict(&fill_map)
            .expect("DataFrame::fillna_dict() must succeed for existing columns");
        let scalar_filled = df
            .fillna(&fill_scalar)
            .expect("DataFrame::fillna() must succeed for numeric inputs");
        prop_assert!(
            dict_filled.equals(&scalar_filled),
            "dataframe fillna_dict(constant_map) must equal scalar fillna"
        );
    }

    /// fillna(method=...) aliases must agree with direct directional fill methods.
    #[test]
    fn prop_dataframe_fillna_method_aliases_match_directional_fill(
        df in arb_numeric_dataframe(8),
    ) {
        let ffill_alias = df
            .fillna_method("ffill")
            .expect("DataFrame::fillna_method(\"ffill\") must succeed");
        let ffill_direct = df
            .ffill(None)
            .expect("DataFrame::ffill(None) must succeed for numeric inputs");
        let bfill_alias = df
            .fillna_method("bfill")
            .expect("DataFrame::fillna_method(\"bfill\") must succeed");
        let bfill_direct = df
            .bfill(None)
            .expect("DataFrame::bfill(None) must succeed for numeric inputs");
        prop_assert!(
            ffill_alias.equals(&ffill_direct),
            "dataframe fillna_method(\"ffill\") must equal ffill(None)"
        );
        prop_assert!(
            bfill_alias.equals(&bfill_direct),
            "dataframe fillna_method(\"bfill\") must equal bfill(None)"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: ffill / bfill metamorphic invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Unlimited forward fill is idempotent for Series.
    #[test]
    fn prop_series_ffill_without_limit_is_idempotent(
        series in arb_numeric_series("ffill", 12),
    ) {
        let once = series
            .ffill(None)
            .expect("Series::ffill(None) must succeed for numeric inputs");
        let twice = once
            .ffill(None)
            .expect("Series::ffill(None) must succeed on an already filled series");
        prop_assert!(
            once.equals(&twice),
            "series ffill(None) must be idempotent"
        );
    }

    /// Unlimited backward fill is idempotent for Series.
    #[test]
    fn prop_series_bfill_without_limit_is_idempotent(
        series in arb_numeric_series("bfill", 12),
    ) {
        let once = series
            .bfill(None)
            .expect("Series::bfill(None) must succeed for numeric inputs");
        let twice = once
            .bfill(None)
            .expect("Series::bfill(None) must succeed on an already filled series");
        prop_assert!(
            once.equals(&twice),
            "series bfill(None) must be idempotent"
        );
    }

    /// Translating a Series commutes with unlimited forward fill.
    #[test]
    fn prop_series_ffill_is_translation_covariant(
        series in arb_numeric_series("ffill", 12),
        delta in -10.0f64..10.0,
    ) {
        let baseline = series
            .ffill(None)
            .expect("Series::ffill(None) must succeed for numeric inputs");
        let shifted_input = shift_series(&series, delta);
        let shifted_fill = shifted_input
            .ffill(None)
            .expect("Series::ffill(None) must succeed after translation");
        let expected = shift_series(&baseline, delta);
        prop_assert!(
            approx_equal_series(&shifted_fill, &expected),
            "series ffill(x + c) must equal ffill(x) + c"
        );
    }

    /// Translating a Series commutes with unlimited backward fill.
    #[test]
    fn prop_series_bfill_is_translation_covariant(
        series in arb_numeric_series("bfill", 12),
        delta in -10.0f64..10.0,
    ) {
        let baseline = series
            .bfill(None)
            .expect("Series::bfill(None) must succeed for numeric inputs");
        let shifted_input = shift_series(&series, delta);
        let shifted_fill = shifted_input
            .bfill(None)
            .expect("Series::bfill(None) must succeed after translation");
        let expected = shift_series(&baseline, delta);
        prop_assert!(
            approx_equal_series(&shifted_fill, &expected),
            "series bfill(x + c) must equal bfill(x) + c"
        );
    }

    /// Unlimited forward fill is idempotent for DataFrames.
    #[test]
    fn prop_dataframe_ffill_without_limit_is_idempotent(
        df in arb_numeric_dataframe(8),
    ) {
        let once = df
            .ffill(None)
            .expect("DataFrame::ffill(None) must succeed for numeric inputs");
        let twice = once
            .ffill(None)
            .expect("DataFrame::ffill(None) must succeed on an already filled frame");
        prop_assert!(
            once.equals(&twice),
            "dataframe ffill(None) must be idempotent"
        );
    }

    /// Unlimited backward fill is idempotent for DataFrames.
    #[test]
    fn prop_dataframe_bfill_without_limit_is_idempotent(
        df in arb_numeric_dataframe(8),
    ) {
        let once = df
            .bfill(None)
            .expect("DataFrame::bfill(None) must succeed for numeric inputs");
        let twice = once
            .bfill(None)
            .expect("DataFrame::bfill(None) must succeed on an already filled frame");
        prop_assert!(
            once.equals(&twice),
            "dataframe bfill(None) must be idempotent"
        );
    }

    /// Translating a DataFrame commutes with unlimited forward fill.
    #[test]
    fn prop_dataframe_ffill_is_translation_covariant(
        df in arb_numeric_dataframe(8),
        delta in -10.0f64..10.0,
    ) {
        let baseline = df
            .ffill(None)
            .expect("DataFrame::ffill(None) must succeed for numeric inputs");
        let shifted_input = shift_dataframe(&df, delta);
        let shifted_fill = shifted_input
            .ffill(None)
            .expect("DataFrame::ffill(None) must succeed after translation");
        let expected = shift_dataframe(&baseline, delta);
        prop_assert!(
            approx_equal_dataframe(&shifted_fill, &expected),
            "dataframe ffill(x + c) must equal ffill(x) + c"
        );
    }

    /// Translating a DataFrame commutes with unlimited backward fill.
    #[test]
    fn prop_dataframe_bfill_is_translation_covariant(
        df in arb_numeric_dataframe(8),
        delta in -10.0f64..10.0,
    ) {
        let baseline = df
            .bfill(None)
            .expect("DataFrame::bfill(None) must succeed for numeric inputs");
        let shifted_input = shift_dataframe(&df, delta);
        let shifted_fill = shifted_input
            .bfill(None)
            .expect("DataFrame::bfill(None) must succeed after translation");
        let expected = shift_dataframe(&baseline, delta);
        prop_assert!(
            approx_equal_dataframe(&shifted_fill, &expected),
            "dataframe bfill(x + c) must equal bfill(x) + c"
        );
    }

    /// Unlimited axis-1 forward fill is idempotent for DataFrames.
    #[test]
    fn prop_dataframe_ffill_axis1_without_limit_is_idempotent(
        df in arb_numeric_dataframe(8),
    ) {
        let once = df
            .ffill_axis1(None)
            .expect("DataFrame::ffill_axis1(None) must succeed for numeric inputs");
        let twice = once
            .ffill_axis1(None)
            .expect("DataFrame::ffill_axis1(None) must succeed on an already filled frame");
        prop_assert!(
            once.equals(&twice),
            "dataframe ffill_axis1(None) must be idempotent"
        );
    }

    /// Unlimited axis-1 backward fill is idempotent for DataFrames.
    #[test]
    fn prop_dataframe_bfill_axis1_without_limit_is_idempotent(
        df in arb_numeric_dataframe(8),
    ) {
        let once = df
            .bfill_axis1(None)
            .expect("DataFrame::bfill_axis1(None) must succeed for numeric inputs");
        let twice = once
            .bfill_axis1(None)
            .expect("DataFrame::bfill_axis1(None) must succeed on an already filled frame");
        prop_assert!(
            once.equals(&twice),
            "dataframe bfill_axis1(None) must be idempotent"
        );
    }

    /// Translating a DataFrame commutes with unlimited axis-1 forward fill.
    #[test]
    fn prop_dataframe_ffill_axis1_is_translation_covariant(
        df in arb_numeric_dataframe(8),
        delta in -10.0f64..10.0,
    ) {
        let baseline = df
            .ffill_axis1(None)
            .expect("DataFrame::ffill_axis1(None) must succeed for numeric inputs");
        let shifted_input = shift_dataframe(&df, delta);
        let shifted_fill = shifted_input
            .ffill_axis1(None)
            .expect("DataFrame::ffill_axis1(None) must succeed after translation");
        let expected = shift_dataframe(&baseline, delta);
        prop_assert!(
            approx_equal_dataframe(&shifted_fill, &expected),
            "dataframe ffill_axis1(x + c) must equal ffill_axis1(x) + c"
        );
    }

    /// Translating a DataFrame commutes with unlimited axis-1 backward fill.
    #[test]
    fn prop_dataframe_bfill_axis1_is_translation_covariant(
        df in arb_numeric_dataframe(8),
        delta in -10.0f64..10.0,
    ) {
        let baseline = df
            .bfill_axis1(None)
            .expect("DataFrame::bfill_axis1(None) must succeed for numeric inputs");
        let shifted_input = shift_dataframe(&df, delta);
        let shifted_fill = shifted_input
            .bfill_axis1(None)
            .expect("DataFrame::bfill_axis1(None) must succeed after translation");
        let expected = shift_dataframe(&baseline, delta);
        prop_assert!(
            approx_equal_dataframe(&shifted_fill, &expected),
            "dataframe bfill_axis1(x + c) must equal bfill_axis1(x) + c"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: where / mask metamorphic invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Complementing a boolean-or-null condition swaps scalar where and mask.
    #[test]
    fn prop_series_where_and_mask_scalar_are_condition_duals(
        (series, cond, _other) in arb_aligned_where_series_triplet(12),
        fill in -10.0f64..10.0,
    ) {
        let fill_scalar = Scalar::Float64(fill);
        let where_result = series
            .where_cond(&cond, Some(&fill_scalar))
            .expect("Series::where_cond() must succeed for aligned inputs");
        let negated_cond = negate_condition_series(&cond);
        let mask_result = series
            .mask(&negated_cond, Some(&fill_scalar))
            .expect("Series::mask() must succeed for complemented aligned inputs");
        prop_assert!(
            approx_equal_series(&where_result, &mask_result),
            "series where(cond, fill) must equal mask(!cond, fill)"
        );
    }

    /// Complementing a boolean-or-null condition swaps Series-valued where and mask.
    #[test]
    fn prop_series_where_and_mask_series_are_condition_duals(
        (series, cond, other) in arb_aligned_where_series_triplet(12),
    ) {
        let where_result = series
            .where_cond_series(&cond, &other)
            .expect("Series::where_cond_series() must succeed for aligned inputs");
        let negated_cond = negate_condition_series(&cond);
        let mask_result = series
            .mask_series(&negated_cond, &other)
            .expect("Series::mask_series() must succeed for complemented aligned inputs");
        prop_assert!(
            approx_equal_series(&where_result, &mask_result),
            "series where(cond, other) must equal mask(!cond, other)"
        );
    }

    /// With unique labels and a purely boolean condition, using self as `other` is an identity for Series.
    #[test]
    fn prop_series_where_and_mask_with_self_are_identity_for_boolean_conditions(
        (series, cond) in arb_aligned_boolean_where_series_pair(12),
    ) {
        let where_identity = series
            .where_cond_series(&cond, &series)
            .expect("Series::where_cond_series() must succeed for aligned boolean inputs");
        let mask_identity = series
            .mask_series(&cond, &series)
            .expect("Series::mask_series() must succeed for aligned boolean inputs");
        prop_assert!(
            series.equals(&where_identity),
            "series where(cond, self) must be identity for boolean conditions on uniquely labeled inputs"
        );
        prop_assert!(
            series.equals(&mask_identity),
            "series mask(cond, self) must be identity for boolean conditions on uniquely labeled inputs"
        );
    }

    /// Translating both the data and scalar fill commutes with scalar where for Series.
    #[test]
    fn prop_series_where_is_translation_covariant(
        (series, cond, _other) in arb_aligned_where_series_triplet(12),
        fill in -10.0f64..10.0,
        delta in -10.0f64..10.0,
    ) {
        let fill_scalar = Scalar::Float64(fill);
        let baseline = series
            .where_cond(&cond, Some(&fill_scalar))
            .expect("Series::where_cond() must succeed for aligned inputs");
        let shifted_input = shift_series(&series, delta);
        let shifted_fill = Scalar::Float64(fill + delta);
        let shifted_result = shifted_input
            .where_cond(&cond, Some(&shifted_fill))
            .expect("Series::where_cond() must succeed after translation");
        let expected = shift_series(&baseline, delta);
        prop_assert!(
            approx_equal_series(&shifted_result, &expected),
            "series where(x + c, cond, fill + c) must equal where(x, cond, fill) + c"
        );
    }

    /// Translating both the data and scalar fill commutes with scalar mask for Series.
    #[test]
    fn prop_series_mask_is_translation_covariant(
        (series, cond, _other) in arb_aligned_where_series_triplet(12),
        fill in -10.0f64..10.0,
        delta in -10.0f64..10.0,
    ) {
        let fill_scalar = Scalar::Float64(fill);
        let baseline = series
            .mask(&cond, Some(&fill_scalar))
            .expect("Series::mask() must succeed for aligned inputs");
        let shifted_input = shift_series(&series, delta);
        let shifted_fill = Scalar::Float64(fill + delta);
        let shifted_result = shifted_input
            .mask(&cond, Some(&shifted_fill))
            .expect("Series::mask() must succeed after translation");
        let expected = shift_series(&baseline, delta);
        prop_assert!(
            approx_equal_series(&shifted_result, &expected),
            "series mask(x + c, cond, fill + c) must equal mask(x, cond, fill) + c"
        );
    }

    /// Complementing a boolean-or-null condition swaps scalar where and mask for DataFrames.
    #[test]
    fn prop_dataframe_where_and_mask_scalar_are_condition_duals(
        (df, cond, _other) in arb_aligned_where_dataframe_triplet(8),
        fill in -10.0f64..10.0,
    ) {
        let fill_scalar = Scalar::Float64(fill);
        let where_result = df
            .where_cond(&cond, Some(&fill_scalar))
            .expect("DataFrame::where_cond() must succeed for aligned inputs");
        let negated_cond = negate_condition_dataframe(&cond);
        let mask_result = df
            .mask(&negated_cond, Some(&fill_scalar))
            .expect("DataFrame::mask() must succeed for complemented aligned inputs");
        prop_assert!(
            approx_equal_dataframe(&where_result, &mask_result),
            "dataframe where(cond, fill) must equal mask(!cond, fill)"
        );
    }

    /// Complementing a boolean-or-null condition swaps DataFrame-valued where and mask.
    #[test]
    fn prop_dataframe_where_and_mask_dataframe_are_condition_duals(
        (df, cond, other) in arb_aligned_where_dataframe_triplet(8),
    ) {
        let where_result = df
            .where_cond_df(&cond, &other)
            .expect("DataFrame::where_cond_df() must succeed for aligned inputs");
        let negated_cond = negate_condition_dataframe(&cond);
        let mask_result = df
            .mask_df_other(&negated_cond, &other)
            .expect("DataFrame::mask_df_other() must succeed for complemented aligned inputs");
        prop_assert!(
            approx_equal_dataframe(&where_result, &mask_result),
            "dataframe where(cond, other) must equal mask(!cond, other)"
        );
    }

    /// With unique labels and a purely boolean condition, using self as `other` is an identity for DataFrames.
    #[test]
    fn prop_dataframe_where_and_mask_with_self_are_identity_for_boolean_conditions(
        (df, cond) in arb_aligned_boolean_where_dataframe_pair(8),
    ) {
        let where_identity = df
            .where_cond_df(&cond, &df)
            .expect("DataFrame::where_cond_df() must succeed for aligned boolean inputs");
        let mask_identity = df
            .mask_df_other(&cond, &df)
            .expect("DataFrame::mask_df_other() must succeed for aligned boolean inputs");
        prop_assert!(
            df.equals(&where_identity),
            "dataframe where(cond, self) must be identity for boolean conditions on uniquely labeled inputs"
        );
        prop_assert!(
            df.equals(&mask_identity),
            "dataframe mask(cond, self) must be identity for boolean conditions on uniquely labeled inputs"
        );
    }

    /// Translating both the data and scalar fill commutes with scalar where for DataFrames.
    #[test]
    fn prop_dataframe_where_is_translation_covariant(
        (df, cond, _other) in arb_aligned_where_dataframe_triplet(8),
        fill in -10.0f64..10.0,
        delta in -10.0f64..10.0,
    ) {
        let fill_scalar = Scalar::Float64(fill);
        let baseline = df
            .where_cond(&cond, Some(&fill_scalar))
            .expect("DataFrame::where_cond() must succeed for aligned inputs");
        let shifted_input = shift_dataframe(&df, delta);
        let shifted_fill = Scalar::Float64(fill + delta);
        let shifted_result = shifted_input
            .where_cond(&cond, Some(&shifted_fill))
            .expect("DataFrame::where_cond() must succeed after translation");
        let expected = shift_dataframe(&baseline, delta);
        prop_assert!(
            approx_equal_dataframe(&shifted_result, &expected),
            "dataframe where(x + c, cond, fill + c) must equal where(x, cond, fill) + c"
        );
    }

    /// Translating both the data and scalar fill commutes with scalar mask for DataFrames.
    #[test]
    fn prop_dataframe_mask_is_translation_covariant(
        (df, cond, _other) in arb_aligned_where_dataframe_triplet(8),
        fill in -10.0f64..10.0,
        delta in -10.0f64..10.0,
    ) {
        let fill_scalar = Scalar::Float64(fill);
        let baseline = df
            .mask(&cond, Some(&fill_scalar))
            .expect("DataFrame::mask() must succeed for aligned inputs");
        let shifted_input = shift_dataframe(&df, delta);
        let shifted_fill = Scalar::Float64(fill + delta);
        let shifted_result = shifted_input
            .mask(&cond, Some(&shifted_fill))
            .expect("DataFrame::mask() must succeed after translation");
        let expected = shift_dataframe(&baseline, delta);
        prop_assert!(
            approx_equal_dataframe(&shifted_result, &expected),
            "dataframe mask(x + c, cond, fill + c) must equal mask(x, cond, fill) + c"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: rank metamorphic invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Series rank is invariant under translation.
    #[test]
    fn prop_series_rank_is_translation_invariant(
        series in arb_variable_numeric_series("rank", 12),
        ascending in proptest::bool::ANY,
        delta in -1e6_f64..1e6_f64,
    ) {
        let baseline = series
            .rank("average", ascending, "keep")
            .expect("Series::rank() must succeed for numeric inputs");
        let shifted = shift_series(&series, delta);
        let shifted_rank = shifted
            .rank("average", ascending, "keep")
            .expect("Series::rank() must succeed after translation");
        prop_assert!(
            baseline.equals(&shifted_rank),
            "series rank must be invariant under translation"
        );
    }

    /// Series rank is invariant under positive scaling.
    #[test]
    fn prop_series_rank_is_positive_scale_invariant(
        series in arb_variable_numeric_series("rank", 12),
        ascending in proptest::bool::ANY,
        scale_raw in 1u16..1000u16,
    ) {
        let factor = f64::from(scale_raw) / 10.0;
        let baseline = series
            .rank("average", ascending, "keep")
            .expect("Series::rank() must succeed for numeric inputs");
        let scaled = scale_series(&series, factor);
        let scaled_rank = scaled
            .rank("average", ascending, "keep")
            .expect("Series::rank() must succeed after positive scaling");
        prop_assert!(
            baseline.equals(&scaled_rank),
            "series rank must be invariant under positive scaling"
        );
    }

    /// Negating a Series swaps ascending and descending rank order.
    #[test]
    fn prop_series_rank_sign_flip_swaps_direction(
        series in arb_variable_numeric_series("rank", 12),
    ) {
        let ascending_rank = series
            .rank("average", true, "keep")
            .expect("Series::rank(ascending=true) must succeed for numeric inputs");
        let flipped = sign_flip_series(&series);
        let descending_rank = flipped
            .rank("average", false, "keep")
            .expect("Series::rank(ascending=false) must succeed after sign flipping");
        prop_assert!(
            ascending_rank.equals(&descending_rank),
            "series rank(x, ascending=true) must equal rank(-x, ascending=false)"
        );
    }

    /// DataFrame rank is invariant under translation.
    #[test]
    fn prop_dataframe_rank_is_translation_invariant(
        df in arb_numeric_dataframe(8),
        ascending in proptest::bool::ANY,
        delta in -1e6_f64..1e6_f64,
    ) {
        let baseline = df
            .rank("average", ascending, "keep")
            .expect("DataFrame::rank() must succeed for numeric inputs");
        let shifted = shift_dataframe(&df, delta);
        let shifted_rank = shifted
            .rank("average", ascending, "keep")
            .expect("DataFrame::rank() must succeed after translation");
        prop_assert!(
            baseline.equals(&shifted_rank),
            "dataframe rank must be invariant under translation"
        );
    }

    /// DataFrame rank is invariant under positive scaling.
    #[test]
    fn prop_dataframe_rank_is_positive_scale_invariant(
        df in arb_numeric_dataframe(8),
        ascending in proptest::bool::ANY,
        scale_raw in 1u16..1000u16,
    ) {
        let factor = f64::from(scale_raw) / 10.0;
        let baseline = df
            .rank("average", ascending, "keep")
            .expect("DataFrame::rank() must succeed for numeric inputs");
        let scaled = scale_dataframe(&df, factor);
        let scaled_rank = scaled
            .rank("average", ascending, "keep")
            .expect("DataFrame::rank() must succeed after positive scaling");
        prop_assert!(
            baseline.equals(&scaled_rank),
            "dataframe rank must be invariant under positive scaling"
        );
    }

    /// Negating a DataFrame swaps ascending and descending rank order.
    #[test]
    fn prop_dataframe_rank_sign_flip_swaps_direction(
        df in arb_numeric_dataframe(8),
    ) {
        let ascending_rank = df
            .rank("average", true, "keep")
            .expect("DataFrame::rank(ascending=true) must succeed for numeric inputs");
        let flipped = sign_flip_dataframe(&df);
        let descending_rank = flipped
            .rank("average", false, "keep")
            .expect("DataFrame::rank(ascending=false) must succeed after sign flipping");
        prop_assert!(
            ascending_rank.equals(&descending_rank),
            "dataframe rank(x, ascending=true) must equal rank(-x, ascending=false)"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: idxmax / idxmin metamorphic invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Series idxmax is invariant under translation.
    #[test]
    fn prop_series_idxmax_is_translation_invariant(
        series in arb_numeric_series("idxmax", 12),
        delta in -1e6_f64..1e6_f64,
    ) {
        let baseline = series.idxmax();
        let shifted = shift_series(&series, delta);
        let shifted_idxmax = shifted.idxmax();
        prop_assert!(
            same_index_result(&shifted_idxmax, &baseline),
            "series idxmax must be invariant under translation: baseline={baseline:?} shifted={shifted_idxmax:?}"
        );
    }

    /// Series idxmin is invariant under translation.
    #[test]
    fn prop_series_idxmin_is_translation_invariant(
        series in arb_numeric_series("idxmin", 12),
        delta in -1e6_f64..1e6_f64,
    ) {
        let baseline = series.idxmin();
        let shifted = shift_series(&series, delta);
        let shifted_idxmin = shifted.idxmin();
        prop_assert!(
            same_index_result(&shifted_idxmin, &baseline),
            "series idxmin must be invariant under translation: baseline={baseline:?} shifted={shifted_idxmin:?}"
        );
    }

    /// Series idxmax is invariant under positive scaling.
    #[test]
    fn prop_series_idxmax_is_positive_scale_invariant(
        series in arb_numeric_series("idxmax", 12),
        scale_raw in 1u16..1000u16,
    ) {
        let factor = f64::from(scale_raw) / 10.0;
        let baseline = series.idxmax();
        let scaled = scale_series(&series, factor);
        let scaled_idxmax = scaled.idxmax();
        prop_assert!(
            same_index_result(&scaled_idxmax, &baseline),
            "series idxmax must be invariant under positive scaling: baseline={baseline:?} scaled={scaled_idxmax:?}"
        );
    }

    /// Series idxmin is invariant under positive scaling.
    #[test]
    fn prop_series_idxmin_is_positive_scale_invariant(
        series in arb_numeric_series("idxmin", 12),
        scale_raw in 1u16..1000u16,
    ) {
        let factor = f64::from(scale_raw) / 10.0;
        let baseline = series.idxmin();
        let scaled = scale_series(&series, factor);
        let scaled_idxmin = scaled.idxmin();
        prop_assert!(
            same_index_result(&scaled_idxmin, &baseline),
            "series idxmin must be invariant under positive scaling: baseline={baseline:?} scaled={scaled_idxmin:?}"
        );
    }

    /// Negating a Series swaps idxmax and idxmin.
    #[test]
    fn prop_series_idxmax_idxmin_are_sign_dual(
        series in arb_numeric_series("idxextrema", 12),
    ) {
        let idxmax = series.idxmax();
        let flipped = sign_flip_series(&series);
        let idxmin = flipped.idxmin();
        prop_assert!(
            same_index_result(&idxmin, &idxmax),
            "series idxmax(x) must equal idxmin(-x): idxmax={idxmax:?} idxmin_neg={idxmin:?}"
        );
    }

    /// DataFrame idxmax is invariant under translation.
    #[test]
    fn prop_dataframe_idxmax_is_translation_invariant(
        df in arb_numeric_dataframe(8),
        delta in -1e6_f64..1e6_f64,
    ) {
        let baseline = df
            .idxmax()
            .expect("DataFrame::idxmax() must succeed for numeric inputs");
        let shifted = shift_dataframe(&df, delta);
        let shifted_idxmax = shifted
            .idxmax()
            .expect("DataFrame::idxmax() must succeed after translation");
        prop_assert!(
            baseline.equals(&shifted_idxmax),
            "dataframe idxmax must be invariant under translation"
        );
    }

    /// DataFrame idxmin is invariant under translation.
    #[test]
    fn prop_dataframe_idxmin_is_translation_invariant(
        df in arb_numeric_dataframe(8),
        delta in -1e6_f64..1e6_f64,
    ) {
        let baseline = df
            .idxmin()
            .expect("DataFrame::idxmin() must succeed for numeric inputs");
        let shifted = shift_dataframe(&df, delta);
        let shifted_idxmin = shifted
            .idxmin()
            .expect("DataFrame::idxmin() must succeed after translation");
        prop_assert!(
            baseline.equals(&shifted_idxmin),
            "dataframe idxmin must be invariant under translation"
        );
    }

    /// DataFrame idxmax is invariant under positive scaling.
    #[test]
    fn prop_dataframe_idxmax_is_positive_scale_invariant(
        df in arb_numeric_dataframe(8),
        scale_raw in 1u16..1000u16,
    ) {
        let factor = f64::from(scale_raw) / 10.0;
        let baseline = df
            .idxmax()
            .expect("DataFrame::idxmax() must succeed for numeric inputs");
        let scaled = scale_dataframe(&df, factor);
        let scaled_idxmax = scaled
            .idxmax()
            .expect("DataFrame::idxmax() must succeed after positive scaling");
        prop_assert!(
            baseline.equals(&scaled_idxmax),
            "dataframe idxmax must be invariant under positive scaling"
        );
    }

    /// DataFrame idxmin is invariant under positive scaling.
    #[test]
    fn prop_dataframe_idxmin_is_positive_scale_invariant(
        df in arb_numeric_dataframe(8),
        scale_raw in 1u16..1000u16,
    ) {
        let factor = f64::from(scale_raw) / 10.0;
        let baseline = df
            .idxmin()
            .expect("DataFrame::idxmin() must succeed for numeric inputs");
        let scaled = scale_dataframe(&df, factor);
        let scaled_idxmin = scaled
            .idxmin()
            .expect("DataFrame::idxmin() must succeed after positive scaling");
        prop_assert!(
            baseline.equals(&scaled_idxmin),
            "dataframe idxmin must be invariant under positive scaling"
        );
    }

    /// Negating a DataFrame swaps idxmax and idxmin.
    #[test]
    fn prop_dataframe_idxmax_idxmin_are_sign_dual(
        df in arb_numeric_dataframe(8),
    ) {
        let idxmax = df
            .idxmax()
            .expect("DataFrame::idxmax() must succeed for numeric inputs");
        let flipped = sign_flip_dataframe(&df);
        let idxmin = flipped
            .idxmin()
            .expect("DataFrame::idxmin() must succeed after sign flipping");
        prop_assert!(
            same_series_payload(&idxmax, &idxmin),
            "dataframe idxmax(x) must equal idxmin(-x)"
        );
    }

    /// Axis-1 idxmax is invariant under translation.
    #[test]
    fn prop_dataframe_idxmax_axis1_is_translation_invariant(
        df in arb_numeric_dataframe(8),
        delta in -1e6_f64..1e6_f64,
    ) {
        let baseline = df
            .idxmax_axis1()
            .expect("DataFrame::idxmax_axis1() must succeed for numeric inputs");
        let shifted = shift_dataframe(&df, delta);
        let shifted_idxmax = shifted
            .idxmax_axis1()
            .expect("DataFrame::idxmax_axis1() must succeed after translation");
        prop_assert!(
            baseline.equals(&shifted_idxmax),
            "dataframe idxmax_axis1 must be invariant under translation"
        );
    }

    /// Axis-1 idxmin is invariant under translation.
    #[test]
    fn prop_dataframe_idxmin_axis1_is_translation_invariant(
        df in arb_numeric_dataframe(8),
        delta in -1e6_f64..1e6_f64,
    ) {
        let baseline = df
            .idxmin_axis1()
            .expect("DataFrame::idxmin_axis1() must succeed for numeric inputs");
        let shifted = shift_dataframe(&df, delta);
        let shifted_idxmin = shifted
            .idxmin_axis1()
            .expect("DataFrame::idxmin_axis1() must succeed after translation");
        prop_assert!(
            baseline.equals(&shifted_idxmin),
            "dataframe idxmin_axis1 must be invariant under translation"
        );
    }

    /// Axis-1 idxmax is invariant under positive scaling.
    #[test]
    fn prop_dataframe_idxmax_axis1_is_positive_scale_invariant(
        df in arb_numeric_dataframe(8),
        scale_raw in 1u16..1000u16,
    ) {
        let factor = f64::from(scale_raw) / 10.0;
        let baseline = df
            .idxmax_axis1()
            .expect("DataFrame::idxmax_axis1() must succeed for numeric inputs");
        let scaled = scale_dataframe(&df, factor);
        let scaled_idxmax = scaled
            .idxmax_axis1()
            .expect("DataFrame::idxmax_axis1() must succeed after positive scaling");
        prop_assert!(
            baseline.equals(&scaled_idxmax),
            "dataframe idxmax_axis1 must be invariant under positive scaling"
        );
    }

    /// Axis-1 idxmin is invariant under positive scaling.
    #[test]
    fn prop_dataframe_idxmin_axis1_is_positive_scale_invariant(
        df in arb_numeric_dataframe(8),
        scale_raw in 1u16..1000u16,
    ) {
        let factor = f64::from(scale_raw) / 10.0;
        let baseline = df
            .idxmin_axis1()
            .expect("DataFrame::idxmin_axis1() must succeed for numeric inputs");
        let scaled = scale_dataframe(&df, factor);
        let scaled_idxmin = scaled
            .idxmin_axis1()
            .expect("DataFrame::idxmin_axis1() must succeed after positive scaling");
        prop_assert!(
            baseline.equals(&scaled_idxmin),
            "dataframe idxmin_axis1 must be invariant under positive scaling"
        );
    }

    /// Negating a DataFrame swaps axis-1 idxmax and idxmin.
    #[test]
    fn prop_dataframe_idxmax_axis1_idxmin_axis1_are_sign_dual(
        df in arb_numeric_dataframe(8),
    ) {
        let idxmax = df
            .idxmax_axis1()
            .expect("DataFrame::idxmax_axis1() must succeed for numeric inputs");
        let flipped = sign_flip_dataframe(&df);
        let idxmin = flipped
            .idxmin_axis1()
            .expect("DataFrame::idxmin_axis1() must succeed after sign flipping");
        prop_assert!(
            same_series_payload(&idxmax, &idxmin),
            "dataframe idxmax_axis1(x) must equal idxmin_axis1(-x)"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: cumulative extrema metamorphic invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Series cummax is idempotent.
    #[test]
    fn prop_series_cummax_is_idempotent(series in arb_numeric_series("cummax", 12)) {
        let once = series
            .cummax()
            .expect("Series::cummax() must succeed for numeric inputs");
        let twice = once
            .cummax()
            .expect("Series::cummax() must succeed on its own output");
        prop_assert!(
            once.equals(&twice),
            "series cummax must be idempotent"
        );
    }

    /// Series cummin is idempotent.
    #[test]
    fn prop_series_cummin_is_idempotent(series in arb_numeric_series("cummin", 12)) {
        let once = series
            .cummin()
            .expect("Series::cummin() must succeed for numeric inputs");
        let twice = once
            .cummin()
            .expect("Series::cummin() must succeed on its own output");
        prop_assert!(
            once.equals(&twice),
            "series cummin must be idempotent"
        );
    }

    /// Negating a Series before cummin must negate the cummax result.
    #[test]
    fn prop_series_cummax_cummin_are_sign_dual(
        series in arb_numeric_series("cumextrema", 12),
    ) {
        let cummax = series
            .cummax()
            .expect("Series::cummax() must succeed for numeric inputs");
        let flipped = sign_flip_series(&series);
        let flipped_cummin = flipped
            .cummin()
            .expect("Series::cummin() must succeed after sign flipping");
        let expected = sign_flip_series(&cummax);
        prop_assert!(
            flipped_cummin.equals(&expected),
            "series cummin(-x) must equal -cummax(x)"
        );
    }

    /// Negating a Series before cummax must negate the cummin result.
    #[test]
    fn prop_series_cummin_cummax_are_sign_dual(
        series in arb_numeric_series("cumextrema", 12),
    ) {
        let cummin = series
            .cummin()
            .expect("Series::cummin() must succeed for numeric inputs");
        let flipped = sign_flip_series(&series);
        let flipped_cummax = flipped
            .cummax()
            .expect("Series::cummax() must succeed after sign flipping");
        let expected = sign_flip_series(&cummin);
        prop_assert!(
            flipped_cummax.equals(&expected),
            "series cummax(-x) must equal -cummin(x)"
        );
    }

    /// DataFrame cummax is idempotent.
    #[test]
    fn prop_dataframe_cummax_is_idempotent(df in arb_numeric_dataframe(8)) {
        let once = df
            .cummax()
            .expect("DataFrame::cummax() must succeed for numeric inputs");
        let twice = once
            .cummax()
            .expect("DataFrame::cummax() must succeed on its own output");
        prop_assert!(
            once.equals(&twice),
            "dataframe cummax must be idempotent"
        );
    }

    /// DataFrame cummin is idempotent.
    #[test]
    fn prop_dataframe_cummin_is_idempotent(df in arb_numeric_dataframe(8)) {
        let once = df
            .cummin()
            .expect("DataFrame::cummin() must succeed for numeric inputs");
        let twice = once
            .cummin()
            .expect("DataFrame::cummin() must succeed on its own output");
        prop_assert!(
            once.equals(&twice),
            "dataframe cummin must be idempotent"
        );
    }

    /// Negating a DataFrame before cummin must negate the cummax result.
    #[test]
    fn prop_dataframe_cummax_cummin_are_sign_dual(df in arb_numeric_dataframe(8)) {
        let cummax = df
            .cummax()
            .expect("DataFrame::cummax() must succeed for numeric inputs");
        let flipped = df
            .mul_scalar(-1.0)
            .expect("DataFrame::mul_scalar(-1.0) must succeed for numeric inputs");
        let flipped_cummin = flipped
            .cummin()
            .expect("DataFrame::cummin() must succeed after sign flipping");
        let expected = cummax
            .mul_scalar(-1.0)
            .expect("cummax dataframe must support sign flipping");
        prop_assert!(
            flipped_cummin.equals(&expected),
            "dataframe cummin(-x) must equal -cummax(x)"
        );
    }

    /// Negating a DataFrame before cummax must negate the cummin result.
    #[test]
    fn prop_dataframe_cummin_cummax_are_sign_dual(df in arb_numeric_dataframe(8)) {
        let cummin = df
            .cummin()
            .expect("DataFrame::cummin() must succeed for numeric inputs");
        let flipped = df
            .mul_scalar(-1.0)
            .expect("DataFrame::mul_scalar(-1.0) must succeed for numeric inputs");
        let flipped_cummax = flipped
            .cummax()
            .expect("DataFrame::cummax() must succeed after sign flipping");
        let expected = cummin
            .mul_scalar(-1.0)
            .expect("cummin dataframe must support sign flipping");
        prop_assert!(
            flipped_cummax.equals(&expected),
            "dataframe cummax(-x) must equal -cummin(x)"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: cumsum / cumprod metamorphic invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Scaling a Series by a non-zero factor must scale cumsum linearly.
    #[test]
    fn prop_series_cumsum_scales_linearly(
        series in arb_numeric_series("cumsum", 12),
        factor_raw in prop_oneof![(-3i8..=-1i8), (1i8..=3i8)],
    ) {
        let factor = f64::from(factor_raw);
        let baseline = series
            .cumsum()
            .expect("Series::cumsum() must succeed for numeric inputs");
        let scaled_input = scale_series(&series, factor);
        let scaled_cumsum = scaled_input
            .cumsum()
            .expect("Series::cumsum() must succeed after scaling");
        let expected = scale_series(&baseline, factor);
        prop_assert!(
            approx_equal_series(&scaled_cumsum, &expected),
            "series cumsum(kx) must equal k * cumsum(x)"
        );
    }

    /// Scaling a Series by a non-zero factor must scale cumprod by prefix powers.
    #[test]
    fn prop_series_cumprod_scales_by_prefix_power(
        series in arb_numeric_series("cumprod", 12),
        factor_raw in prop_oneof![(-3i8..=-1i8), (1i8..=3i8)],
    ) {
        let factor = f64::from(factor_raw);
        let scaled_input = scale_series(&series, factor);
        let scaled_cumprod = scaled_input
            .cumprod()
            .expect("Series::cumprod() must succeed after scaling");
        let expected = expected_scaled_cumprod_series(&series, factor);
        prop_assert!(
            approx_equal_series(&scaled_cumprod, &expected),
            "series cumprod(kx) must equal k^n * cumprod(x) at each non-missing prefix"
        );
    }

    /// Scaling a DataFrame by a non-zero factor must scale per-column cumsum linearly.
    #[test]
    fn prop_dataframe_cumsum_scales_linearly(
        df in arb_numeric_dataframe(8),
        factor_raw in prop_oneof![(-3i8..=-1i8), (1i8..=3i8)],
    ) {
        let factor = f64::from(factor_raw);
        let baseline = df
            .cumsum()
            .expect("DataFrame::cumsum() must succeed for numeric inputs");
        let scaled_input = scale_dataframe(&df, factor);
        let scaled_cumsum = scaled_input
            .cumsum()
            .expect("DataFrame::cumsum() must succeed after scaling");
        let expected = scale_dataframe(&baseline, factor);
        prop_assert!(
            approx_equal_dataframe(&scaled_cumsum, &expected),
            "dataframe cumsum(kx) must equal k * cumsum(x)"
        );
    }

    /// Scaling a DataFrame by a non-zero factor must scale per-column cumprod by prefix powers.
    #[test]
    fn prop_dataframe_cumprod_scales_by_prefix_power(
        df in arb_numeric_dataframe(8),
        factor_raw in prop_oneof![(-3i8..=-1i8), (1i8..=3i8)],
    ) {
        let factor = f64::from(factor_raw);
        let scaled_input = scale_dataframe(&df, factor);
        let scaled_cumprod = scaled_input
            .cumprod()
            .expect("DataFrame::cumprod() must succeed after scaling");
        let expected = expected_scaled_cumprod_dataframe(&df, factor);
        prop_assert!(
            approx_equal_dataframe(&scaled_cumprod, &expected),
            "dataframe cumprod(kx) must equal k^n * cumprod(x) for each column prefix"
        );
    }

    /// Scaling a DataFrame by a non-zero factor must scale row-wise cumsum linearly.
    #[test]
    fn prop_dataframe_cumsum_axis1_scales_linearly(
        df in arb_numeric_dataframe(8),
        factor_raw in prop_oneof![(-3i8..=-1i8), (1i8..=3i8)],
    ) {
        let factor = f64::from(factor_raw);
        let baseline = df
            .cumsum_axis1()
            .expect("DataFrame::cumsum_axis1() must succeed for numeric inputs");
        let scaled_input = scale_dataframe(&df, factor);
        let scaled_cumsum = scaled_input
            .cumsum_axis1()
            .expect("DataFrame::cumsum_axis1() must succeed after scaling");
        let expected = scale_dataframe(&baseline, factor);
        prop_assert!(
            approx_equal_dataframe(&scaled_cumsum, &expected),
            "dataframe cumsum_axis1(kx) must equal k * cumsum_axis1(x)"
        );
    }

    /// Scaling a DataFrame by a non-zero factor must scale row-wise cumprod by prefix powers.
    #[test]
    fn prop_dataframe_cumprod_axis1_scales_by_prefix_power(
        df in arb_numeric_dataframe(8),
        factor_raw in prop_oneof![(-3i8..=-1i8), (1i8..=3i8)],
    ) {
        let factor = f64::from(factor_raw);
        let scaled_input = scale_dataframe(&df, factor);
        let scaled_cumprod = scaled_input
            .cumprod_axis1()
            .expect("DataFrame::cumprod_axis1() must succeed after scaling");
        let expected = expected_scaled_cumprod_axis1_dataframe(&df, factor);
        prop_assert!(
            approx_equal_dataframe(&scaled_cumprod, &expected),
            "dataframe cumprod_axis1(kx) must equal k^n * cumprod_axis1(x) within each row"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: Sampling metamorphic invariants (frankenpandas-ags)
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Sampling with the same seed must be deterministic for Series.
    #[test]
    fn prop_series_sample_same_seed_is_deterministic(
        series in arb_variable_numeric_series("sample", 12),
        replace in proptest::bool::ANY,
        seed in any::<u64>(),
        hint in 0usize..24,
    ) {
        let sample_n = sample_count_from_hint(series.len(), replace, hint);
        let first = series
            .sample(Some(sample_n), None, replace, Some(seed))
            .expect("series sampling should succeed for derived parameters");
        let second = series
            .sample(Some(sample_n), None, replace, Some(seed))
            .expect("series sampling should succeed for identical replay");

        prop_assert!(
            first.equals(&second),
            "same seed must reproduce the same series sample"
        );
    }

    /// Sampling the full fraction without replacement must permute, not mutate,
    /// the Series rows.
    #[test]
    fn prop_series_sample_full_fraction_is_permutation(
        series in arb_variable_numeric_series("sample", 12),
        seed in any::<u64>(),
    ) {
        let sampled = series
            .sample(None, Some(1.0), false, Some(seed))
            .expect("full-fraction series sampling should succeed");

        prop_assert_eq!(
            normalized_series_rows(&sampled),
            normalized_series_rows(&series),
            "frac=1.0 series sampling without replacement must be a permutation"
        );
    }

    /// Sampling with the same seed must be deterministic for DataFrames.
    #[test]
    fn prop_dataframe_sample_same_seed_is_deterministic(
        df in arb_numeric_dataframe(8),
        replace in proptest::bool::ANY,
        seed in any::<u64>(),
        hint in 0usize..24,
    ) {
        let sample_n = sample_count_from_hint(df.len(), replace, hint);
        let first = df
            .sample(Some(sample_n), None, replace, Some(seed))
            .expect("dataframe sampling should succeed for derived parameters");
        let second = df
            .sample(Some(sample_n), None, replace, Some(seed))
            .expect("dataframe sampling should succeed for identical replay");

        prop_assert!(
            first.equals(&second),
            "same seed must reproduce the same dataframe sample"
        );
    }

    /// Sampling the full fraction without replacement must permute, not mutate,
    /// the DataFrame rows.
    #[test]
    fn prop_dataframe_sample_full_fraction_is_permutation(
        df in arb_numeric_dataframe(8),
        seed in any::<u64>(),
    ) {
        let sampled = df
            .sample(None, Some(1.0), false, Some(seed))
            .expect("full-fraction dataframe sampling should succeed");

        prop_assert_eq!(
            normalized_dataframe_rows(&sampled),
            normalized_dataframe_rows(&df),
            "frac=1.0 dataframe sampling without replacement must be a permutation"
        );
    }

    /// Weighted sampling should be invariant under uniform positive scaling of
    /// the weights when all other inputs are fixed.
    #[test]
    fn prop_dataframe_weighted_sample_invariant_under_weight_scaling(
        df in arb_numeric_dataframe(8),
        base_weights in proptest::collection::vec(1u16..1000u16, 8),
        replace in proptest::bool::ANY,
        seed in any::<u64>(),
        hint in 0usize..24,
    ) {
        let weights = base_weights[..df.len()]
            .iter()
            .map(|&weight| f64::from(weight))
            .collect::<Vec<_>>();
        let sample_n = sample_count_from_hint(df.len(), replace, hint);
        let scaled_weights = weights.iter().map(|weight| weight * 8.0).collect::<Vec<_>>();

        let baseline = df
            .sample_weights(sample_n, &weights, replace, Some(seed))
            .expect("baseline weighted sample should succeed");
        let scaled = df
            .sample_weights(sample_n, &scaled_weights, replace, Some(seed))
            .expect("scaled weighted sample should succeed");

        prop_assert!(
            baseline.equals(&scaled),
            "uniform weight scaling must not change weighted sampling with a fixed seed"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: Shift metamorphic invariants (frankenpandas-xd9)
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Shifting a Series by zero periods must be the identity transform.
    #[test]
    fn prop_series_shift_zero_is_identity(series in arb_variable_numeric_series("shift", 12)) {
        let shifted = series
            .shift(0)
            .expect("series zero-shift should always succeed");

        prop_assert!(
            shifted.equals(&series),
            "series shift(0) must preserve every row"
        );
    }

    /// Same-direction Series shifts must compose additively.
    #[test]
    fn prop_series_shift_same_direction_composes(
        series in arb_variable_numeric_series("shift", 12),
        first in 0i64..10,
        second in 0i64..10,
        negative in proptest::bool::ANY,
    ) {
        let first = if negative { -first } else { first };
        let second = if negative { -second } else { second };
        let twice = series
            .shift(first)
            .and_then(|shifted| shifted.shift(second))
            .expect("composed series shifts should succeed");
        let direct = series
            .shift(first + second)
            .expect("direct series shift should succeed");

        prop_assert!(
            twice.equals(&direct),
            "same-direction series shifts must compose additively"
        );
    }

    /// Shifting a DataFrame by zero periods must be the identity transform.
    #[test]
    fn prop_dataframe_shift_zero_is_identity(df in arb_numeric_dataframe(8)) {
        let shifted = df
            .shift(0)
            .expect("dataframe zero-shift should always succeed");

        prop_assert!(
            shifted.equals(&df),
            "dataframe shift(0) must preserve every cell"
        );
    }

    /// Same-direction DataFrame row shifts must compose additively.
    #[test]
    fn prop_dataframe_shift_same_direction_composes(
        df in arb_numeric_dataframe(8),
        first in 0i64..10,
        second in 0i64..10,
        negative in proptest::bool::ANY,
    ) {
        let first = if negative { -first } else { first };
        let second = if negative { -second } else { second };
        let twice = df
            .shift(first)
            .and_then(|shifted| shifted.shift(second))
            .expect("composed dataframe row shifts should succeed");
        let direct = df
            .shift(first + second)
            .expect("direct dataframe row shift should succeed");

        prop_assert!(
            twice.equals(&direct),
            "same-direction dataframe row shifts must compose additively"
        );
    }

    /// Horizontally shifting a DataFrame by zero periods must be the identity transform.
    #[test]
    fn prop_dataframe_shift_axis1_zero_is_identity(df in arb_numeric_dataframe(8)) {
        let shifted = df
            .shift_axis1(0)
            .expect("dataframe axis=1 zero-shift should always succeed");

        prop_assert!(
            shifted.equals(&df),
            "dataframe shift_axis1(0) must preserve every cell"
        );
    }

    /// Same-direction DataFrame axis=1 shifts must compose additively.
    #[test]
    fn prop_dataframe_shift_axis1_same_direction_composes(
        df in arb_numeric_dataframe(8),
        first in 0i64..6,
        second in 0i64..6,
        negative in proptest::bool::ANY,
    ) {
        let first = if negative { -first } else { first };
        let second = if negative { -second } else { second };
        let twice = df
            .shift_axis1(first)
            .and_then(|shifted| shifted.shift_axis1(second))
            .expect("composed dataframe axis=1 shifts should succeed");
        let direct = df
            .shift_axis1(first + second)
            .expect("direct dataframe axis=1 shift should succeed");

        prop_assert!(
            twice.equals(&direct),
            "same-direction dataframe axis=1 shifts must compose additively"
        );
    }
}

// ---------------------------------------------------------------------------
// Property: Feather (Arrow IPC) round-trip invariants (frankenpandas-44y)
// ---------------------------------------------------------------------------

/// Generate a DataFrame for Feather/SQL round-trip testing.
/// Uses Int64-only columns to avoid type inference ambiguity during read-back.
fn arb_int64_dataframe(
    max_rows: usize,
    max_cols: usize,
) -> impl Strategy<Value = fp_frame::DataFrame> {
    (1..=max_rows, 1..=max_cols).prop_flat_map(|(nrows, ncols)| {
        let col_names = proptest::collection::vec(arb_column_name(), ncols);
        let columns = proptest::collection::vec(
            proptest::collection::vec((-100_000i64..100_000i64).prop_map(Scalar::Int64), nrows),
            ncols,
        );
        (col_names, columns).prop_filter_map(
            "dataframe construction must succeed",
            move |(names, cols)| {
                let mut seen = std::collections::HashSet::new();
                let mut unique_names = Vec::new();
                for name in &names {
                    let mut candidate = name.clone();
                    let mut suffix = 0;
                    while seen.contains(&candidate) {
                        suffix += 1;
                        candidate = format!("{name}{suffix}");
                    }
                    seen.insert(candidate.clone());
                    unique_names.push(candidate);
                }

                let col_order: Vec<&str> = unique_names.iter().map(String::as_str).collect();
                let data: Vec<(&str, Vec<Scalar>)> = unique_names
                    .iter()
                    .zip(cols.iter())
                    .map(|(n, v)| (n.as_str(), v.clone()))
                    .collect();

                fp_frame::DataFrame::from_dict(&col_order, data).ok()
            },
        )
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]

    /// Feather round-trip preserves DataFrame shape.
    #[test]
    fn prop_feather_round_trip_preserves_shape(df in arb_int64_dataframe(8, 4)) {
        let bytes = fp_io::write_feather_bytes(&df);
        prop_assert!(bytes.is_ok(), "Feather write must succeed");
        let bytes = bytes.unwrap();

        let parsed = fp_io::read_feather_bytes(&bytes);
        prop_assert!(parsed.is_ok(), "Feather parse must succeed: {:?}", parsed.err());
        let parsed = parsed.unwrap();

        prop_assert_eq!(parsed.index().len(), df.index().len(),
            "Feather round-trip must preserve row count");
        prop_assert_eq!(parsed.column_names().len(), df.column_names().len(),
            "Feather round-trip must preserve column count");
    }

    /// Feather round-trip preserves column names.
    #[test]
    fn prop_feather_round_trip_preserves_column_names(df in arb_int64_dataframe(5, 3)) {
        let bytes = fp_io::write_feather_bytes(&df).unwrap();
        let parsed = fp_io::read_feather_bytes(&bytes).unwrap();

        let orig: Vec<&String> = df.column_names();
        let back: Vec<&String> = parsed.column_names();
        prop_assert_eq!(orig, back, "column names must survive Feather round-trip");
    }

    /// Feather round-trip preserves Int64 values exactly.
    #[test]
    fn prop_feather_round_trip_int64_exact(df in arb_int64_dataframe(8, 3)) {
        let bytes = fp_io::write_feather_bytes(&df).unwrap();
        let parsed = fp_io::read_feather_bytes(&bytes).unwrap();

        for name in df.column_names() {
            let orig_col = df.column(name).unwrap();
            let parsed_col = parsed.column(name).unwrap();
            prop_assert_eq!(
                orig_col.values(), parsed_col.values(),
                "Int64 values must survive Feather round-trip for column {}", name
            );
        }
    }

    /// IPC stream round-trip preserves shape.
    #[test]
    fn prop_ipc_stream_round_trip_preserves_shape(df in arb_int64_dataframe(8, 4)) {
        let bytes = fp_io::write_ipc_stream_bytes(&df).unwrap();
        let parsed = fp_io::read_ipc_stream_bytes(&bytes).unwrap();

        prop_assert_eq!(parsed.index().len(), df.index().len(),
            "IPC stream round-trip must preserve row count");
        prop_assert_eq!(parsed.column_names().len(), df.column_names().len(),
            "IPC stream round-trip must preserve column count");
    }
}

// ---------------------------------------------------------------------------
// Property: SQL round-trip invariants (frankenpandas-44y)
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(30))]

    /// SQL round-trip preserves DataFrame shape.
    #[test]
    fn prop_sql_round_trip_preserves_shape(df in arb_int64_dataframe(8, 4)) {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let write_result = fp_io::write_sql(&df, &conn, "test_prop", fp_io::SqlIfExists::Replace);
        prop_assert!(write_result.is_ok(), "SQL write must succeed: {:?}", write_result.err());

        let parsed = fp_io::read_sql_table(&conn, "test_prop");
        prop_assert!(parsed.is_ok(), "SQL read must succeed: {:?}", parsed.err());
        let parsed = parsed.unwrap();

        prop_assert_eq!(parsed.index().len(), df.index().len(),
            "SQL round-trip must preserve row count");
        prop_assert_eq!(parsed.column_names().len(), df.column_names().len(),
            "SQL round-trip must preserve column count");
    }

    /// SQL round-trip preserves Int64 values exactly.
    #[test]
    fn prop_sql_round_trip_int64_exact(df in arb_int64_dataframe(8, 3)) {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        fp_io::write_sql(&df, &conn, "test_vals", fp_io::SqlIfExists::Replace).unwrap();
        let parsed = fp_io::read_sql_table(&conn, "test_vals").unwrap();

        for name in df.column_names() {
            let orig_col = df.column(name).unwrap();
            let parsed_col = parsed.column(name).unwrap();
            prop_assert_eq!(
                orig_col.values(), parsed_col.values(),
                "Int64 values must survive SQL round-trip for column {}", name
            );
        }
    }

    /// SQL round-trip preserves column names (sorted, since SQL has no order guarantee).
    #[test]
    fn prop_sql_round_trip_preserves_column_names(df in arb_int64_dataframe(5, 3)) {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        fp_io::write_sql(&df, &conn, "test_names", fp_io::SqlIfExists::Replace).unwrap();
        let parsed = fp_io::read_sql_table(&conn, "test_names").unwrap();

        let mut orig: Vec<String> = df.column_names().into_iter().cloned().collect();
        let mut back: Vec<String> = parsed.column_names().into_iter().cloned().collect();
        orig.sort();
        back.sort();
        prop_assert_eq!(orig, back, "column names must survive SQL round-trip");
    }
}

// ---------------------------------------------------------------------------
// Property: Excel round-trip invariants (frankenpandas-44y)
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(30))]

    /// Excel round-trip preserves DataFrame shape.
    #[test]
    fn prop_excel_round_trip_preserves_shape(df in arb_int64_dataframe(8, 4)) {
        let bytes = fp_io::write_excel_bytes(&df);
        prop_assert!(bytes.is_ok(), "Excel write must succeed");
        let bytes = bytes.unwrap();

        let parsed = fp_io::read_excel_bytes(&bytes, &fp_io::ExcelReadOptions::default());
        prop_assert!(parsed.is_ok(), "Excel parse must succeed: {:?}", parsed.err());
        let parsed = parsed.unwrap();

        prop_assert_eq!(parsed.index().len(), df.index().len(),
            "Excel round-trip must preserve row count");
        prop_assert_eq!(parsed.column_names().len(), df.column_names().len(),
            "Excel round-trip must preserve column count");
    }

    /// Excel round-trip preserves Int64 values exactly.
    /// (Excel stores integers as f64; our reader recovers Int64 for whole numbers.)
    #[test]
    fn prop_excel_round_trip_int64_exact(df in arb_int64_dataframe(8, 3)) {
        let bytes = fp_io::write_excel_bytes(&df).unwrap();
        let parsed = fp_io::read_excel_bytes(&bytes, &fp_io::ExcelReadOptions::default()).unwrap();

        for name in df.column_names() {
            let orig_col = df.column(name).unwrap();
            let parsed_col = parsed.column(name).unwrap();
            prop_assert_eq!(
                orig_col.values(), parsed_col.values(),
                "Int64 values must survive Excel round-trip for column {}", name
            );
        }
    }
}
