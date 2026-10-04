//! Public-API regressions for positional and shared-key width restoration.
//! NumericWidth is a post-v0.3.0 API; these tests exercise the candidate.

use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::{ColumnStore, DataFrame};
use fp_index::Index;
use fp_join::{
    JoinType, MergeExecutionOptions, merge_dataframes_on, merge_dataframes_on_with_options,
};
use fp_types::{NumericWidth, Scalar};

fn frame_with_key(column: Column) -> DataFrame {
    DataFrame::new_with_column_order(
        Index::new_known_unique_int64_unit_range(0, column.len()),
        BTreeMap::from([("key".to_owned(), column)]),
        vec!["key".to_owned()],
    )
    .unwrap()
}

fn merge_with_indicator(
    left: &DataFrame,
    right: &DataFrame,
    how: JoinType,
    indicator: bool,
) -> fp_join::MergedDataFrame {
    merge_dataframes_on_with_options(
        left,
        right,
        &["key"],
        &["key"],
        how,
        MergeExecutionOptions {
            indicator_name: indicator.then(|| "origin".to_owned()),
            ..Default::default()
        },
    )
    .unwrap()
}

#[test]
fn all_matched_right_integer_keys_keep_left_width_in_both_execution_paths() {
    for (left_width, right_width) in [
        (NumericWidth::Int16, NumericWidth::Int32),
        (NumericWidth::Int32, NumericWidth::Int16),
    ] {
        let left = frame_with_key(
            Column::from_i64_values(vec![1, 2])
                .cast_to_width(left_width, false)
                .unwrap(),
        );
        let right = frame_with_key(
            Column::from_i64_values(vec![1, 2])
                .cast_to_width(right_width, false)
                .unwrap(),
        );
        for indicator in [false, true] {
            let merged = merge_with_indicator(&left, &right, JoinType::Right, indicator);
            let key = merged.columns.get("key").unwrap();
            assert_eq!(key.width(), Some(left_width));
            assert_eq!(key.values(), &[Scalar::Int64(1), Scalar::Int64(2)]);
        }
    }
}

#[test]
fn right_only_keys_keep_right_width_in_both_execution_paths() {
    for left_narrow in [false, true] {
        let left = frame_with_key(if left_narrow {
            float32(vec![1.0])
        } else {
            Column::from_f64_values(vec![1.0])
        });
        let right = frame_with_key(if left_narrow {
            Column::from_f64_values(vec![0.1])
        } else {
            float32(vec![2.0])
        });
        for indicator in [false, true] {
            let merged = merge_with_indicator(&left, &right, JoinType::Right, indicator);
            let key = merged.columns.get("key").unwrap();
            assert_eq!(key.width(), right.column("key").unwrap().width());
            assert_eq!(key.values(), right.column("key").unwrap().values());
        }
    }
}

#[test]
fn empty_right_and_empty_outer_keys_keep_left_width_in_both_execution_paths() {
    for left_narrow in [false, true] {
        for (how, left_values) in [
            (JoinType::Right, vec![1.0, 2.0]),
            (JoinType::Right, vec![]),
            (JoinType::Outer, vec![]),
        ] {
            let left = frame_with_key(if left_narrow {
                float32(left_values)
            } else {
                Column::from_f64_values(left_values)
            });
            let right = frame_with_key(if left_narrow {
                Column::from_f64_values(vec![])
            } else {
                float32(vec![])
            });
            for indicator in [false, true] {
                let merged = merge_with_indicator(&left, &right, how, indicator);
                let key = merged.columns.get("key").unwrap();
                assert_eq!(key.width(), left.column("key").unwrap().width());
                assert_eq!(key.len(), 0);
                assert!(merged.index.is_empty());
            }
        }
    }
}

fn float32(values: Vec<f64>) -> Column {
    Column::from_f64_values(values)
        .cast_to_width(NumericWidth::Float32, false)
        .unwrap()
}

#[test]
fn outer_shared_key_preserves_right_only_float64_bits() {
    let left = frame_with_key(float32(vec![1.0]));
    let right = frame_with_key(Column::from_f64_values(vec![0.1]));
    let merged = merge_dataframes_on(&left, &right, &["key"], JoinType::Outer).unwrap();
    assert_eq!(merged.index.len(), 2);
    let key = merged.columns.get("key").unwrap();
    let mut bits = key
        .values()
        .iter()
        .map(|value| match value {
            Scalar::Float64(value) => value.to_bits(),
            other => panic!("shared float key changed scalar kind: {other:?}"),
        })
        .collect::<Vec<_>>();
    bits.sort_unstable();
    let mut expected = vec![0.1_f64.to_bits(), 1.0_f64.to_bits()];
    expected.sort_unstable();
    assert_eq!(
        bits, expected,
        "a coalesced right-only key must not be narrowed to the left width"
    );
    assert_eq!(
        key.width(),
        None,
        "float32 + float64 outer key needs float64"
    );
}

#[test]
fn repeated_payload_names_keep_their_own_width_and_float64_bits() {
    let left = DataFrame::new_with_column_order(
        Index::from_i64(vec![0]),
        ColumnStore::from_pairs([
            ("key".to_owned(), Column::from_i64_values(vec![1])),
            ("a".to_owned(), float32(vec![1.0])),
            ("a".to_owned(), Column::from_f64_values(vec![0.1])),
        ]),
        vec!["key".to_owned(), "a".to_owned(), "a".to_owned()],
    )
    .unwrap();
    let right = frame_with_key(Column::from_i64_values(vec![1]));
    let merged = merge_dataframes_on(&left, &right, &["key"], JoinType::Inner).unwrap();
    assert_eq!(merged.index.len(), 1);
    assert_eq!(merged.column_order, ["key", "a", "a"]);
    assert_eq!(merged.columns.len(), 3);
    let first = merged.columns.column_at(1).unwrap();
    let second = merged.columns.column_at(2).unwrap();
    assert_eq!(first.width(), Some(NumericWidth::Float32));
    assert_eq!(first.values(), &[Scalar::Float64(1.0)]);
    assert_eq!(
        second.width(),
        None,
        "the second a is sourced from the second a, not the first a"
    );
    let Scalar::Float64(value) = second.values()[0] else {
        panic!("second payload changed scalar kind");
    };
    assert_eq!(value.to_bits(), 0.1_f64.to_bits());
}

#[test]
fn cross_side_suffix_collision_does_not_narrow_the_other_side_payload() {
    // Every input name is unique. The right a becomes a_y beside the
    // left's original a_y; pandas permits this cross-side collision.
    let left = DataFrame::new_with_column_order(
        Index::from_i64(vec![0]),
        BTreeMap::from([
            ("key".to_owned(), Column::from_i64_values(vec![1])),
            ("a".to_owned(), Column::from_f64_values(vec![0.2])),
            ("a_y".to_owned(), float32(vec![1.0])),
        ]),
        vec!["key".to_owned(), "a".to_owned(), "a_y".to_owned()],
    )
    .unwrap();
    let right = DataFrame::new_with_column_order(
        Index::from_i64(vec![0]),
        BTreeMap::from([
            ("key".to_owned(), Column::from_i64_values(vec![1])),
            ("a".to_owned(), Column::from_f64_values(vec![0.1])),
        ]),
        vec!["key".to_owned(), "a".to_owned()],
    )
    .unwrap();
    let merged = merge_dataframes_on(&left, &right, &["key"], JoinType::Inner).unwrap();
    assert_eq!(merged.index.len(), 1);
    assert_eq!(merged.column_order, ["key", "a_x", "a_y", "a_y"]);
    assert_eq!(merged.columns.len(), 4);
    assert_eq!(
        merged.columns.column_at(2).unwrap().width(),
        Some(NumericWidth::Float32)
    );
    let right_payload = merged.columns.column_at(3).unwrap();
    assert_eq!(right_payload.width(), None);
    let Scalar::Float64(value) = right_payload.values()[0] else {
        panic!("right payload changed scalar kind");
    };
    assert_eq!(value.to_bits(), 0.1_f64.to_bits());
}
