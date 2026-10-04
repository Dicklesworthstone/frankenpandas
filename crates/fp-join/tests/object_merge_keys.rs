use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::Index;
use fp_join::{
    AsofDirection, JoinType, MergeAsofOptions, merge_asof_with_options, merge_dataframes,
    merge_dataframes_on,
};
use fp_types::{HostObject, HostValue, NullKind, ObjectValue, Scalar};

fn bytes(value: &[u8]) -> Scalar {
    Scalar::Object(ObjectValue::bytes(value.to_vec()))
}

fn frame(keys: Vec<Scalar>, values: Vec<i64>, value_name: &str) -> DataFrame {
    let len = keys.len();
    DataFrame::new_with_column_order(
        Index::from_i64((0..len as i64).collect()),
        BTreeMap::from([
            ("key".to_owned(), Column::from_object_values(keys)),
            (value_name.to_owned(), Column::from_i64_values(values)),
        ]),
        vec!["key".to_owned(), value_name.to_owned()],
    )
    .unwrap()
}

#[test]
fn disjoint_byte_keys_do_not_match_as_missing() {
    let left = frame(vec![bytes(b"a"), bytes(b"b")], vec![10, 20], "left");
    let right = frame(vec![bytes(b"c"), bytes(b"d")], vec![30, 40], "right");
    for result in [
        merge_dataframes(&left, &right, "key", JoinType::Inner),
        merge_dataframes_on(&left, &right, &["key"], JoinType::Inner),
    ] {
        assert_eq!(result.unwrap().index.len(), 0);
    }
}

#[test]
fn equal_byte_keys_match_by_value_without_matching_other_objects() {
    let left = frame(vec![bytes(b"a"), bytes(b"b")], vec![10, 20], "left");
    let right = frame(vec![bytes(b"b"), bytes(b"c")], vec![30, 40], "right");
    for result in [
        merge_dataframes(&left, &right, "key", JoinType::Inner),
        merge_dataframes_on(&left, &right, &["key"], JoinType::Inner),
    ] {
        let merged = result.unwrap();
        assert_eq!(merged.index.len(), 1);
        assert_eq!(
            merged.columns.get("left").unwrap().values(),
            &[Scalar::Int64(20)]
        );
        assert_eq!(
            merged.columns.get("right").unwrap().values(),
            &[Scalar::Int64(30)]
        );
    }
}

#[test]
fn list_keys_are_refused_instead_of_matching_as_missing() {
    let left = frame(
        vec![Scalar::Object(ObjectValue::list(vec![Scalar::Int64(1)]))],
        vec![10],
        "left",
    );
    let right = frame(
        vec![Scalar::Object(ObjectValue::list(vec![Scalar::Int64(2)]))],
        vec![20],
        "right",
    );
    for result in [
        merge_dataframes(&left, &right, "key", JoinType::Inner),
        merge_dataframes_on(&left, &right, &["key"], JoinType::Inner),
    ] {
        assert!(
            result.is_err(),
            "unsupported keys must fail before producing matches"
        );
    }
}

#[test]
fn bytes_stay_distinct_from_strings_and_match_nulls_only_as_nulls() {
    let left = frame(
        vec![bytes(b"a"), Scalar::Null(NullKind::NaN)],
        vec![10, 20],
        "left",
    );
    let right = frame(
        vec![Scalar::Utf8("a".to_owned()), Scalar::Null(NullKind::NaN)],
        vec![30, 40],
        "right",
    );
    for result in [
        merge_dataframes(&left, &right, "key", JoinType::Inner),
        merge_dataframes_on(&left, &right, &["key"], JoinType::Inner),
    ] {
        let merged = result.unwrap();
        assert_eq!(merged.index.len(), 1);
        assert_eq!(
            merged.columns.get("left").unwrap().values(),
            &[Scalar::Int64(20)]
        );
        assert_eq!(
            merged.columns.get("right").unwrap().values(),
            &[Scalar::Int64(40)]
        );
    }
}

struct OpaqueKey;

impl HostObject for OpaqueKey {
    fn host_eq(&self, _: &dyn HostObject) -> bool {
        panic!("refused host keys must not invoke equality")
    }

    fn host_repr(&self) -> String {
        panic!("refused host keys must not invoke repr")
    }

    fn host_str(&self) -> String {
        panic!("refused host keys must not invoke str")
    }

    fn host_hash(&self) -> Option<u64> {
        panic!("refused host keys must not invoke hashing")
    }

    fn host_cmp(&self, _: &dyn HostObject) -> Option<std::cmp::Ordering> {
        panic!("refused host keys must not invoke ordering")
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[test]
fn host_keys_are_refused_without_invoking_host_callbacks() {
    let host = || Scalar::Object(ObjectValue::Host(HostValue::new(OpaqueKey)));
    let left = frame(vec![host()], vec![10], "left");
    let right = frame(vec![host()], vec![20], "right");
    for result in [
        merge_dataframes(&left, &right, "key", JoinType::Inner),
        merge_dataframes_on(&left, &right, &["key"], JoinType::Inner),
    ] {
        assert!(result.is_err());
    }
    let left = left
        .with_column("time", Column::from_i64_values(vec![1]))
        .unwrap();
    let right = right
        .with_column("time", Column::from_i64_values(vec![1]))
        .unwrap();
    let result = merge_asof_with_options(
        &left,
        &right,
        "time",
        AsofDirection::Backward,
        MergeAsofOptions::new().by(vec!["key".to_owned()]),
    );
    assert!(result.is_err());
}

#[test]
fn typed_string_merge_keys_keep_their_scalar_cache_unmaterialized() {
    let make = |keys: &[u8], name: &str, values: Vec<i64>| {
        DataFrame::new_with_column_order(
            Index::from_i64(vec![0, 1]),
            BTreeMap::from([
                (
                    "key".to_owned(),
                    Column::from_utf8_contiguous(keys.to_vec(), vec![0, 1, 2]),
                ),
                (name.to_owned(), Column::from_i64_values(values)),
            ]),
            vec!["key".to_owned(), name.to_owned()],
        )
        .unwrap()
    };
    for direct in [true, false] {
        let left = make(b"ab", "left", vec![10, 20]);
        let right = make(b"bc", "right", vec![30, 40]);
        assert!(!left.column("key").unwrap().scalar_cache_is_materialized());
        assert!(!right.column("key").unwrap().scalar_cache_is_materialized());
        let result = if direct {
            merge_dataframes(&left, &right, "key", JoinType::Inner)
        } else {
            merge_dataframes_on(&left, &right, &["key"], JoinType::Inner)
        }
        .unwrap();
        assert_eq!(result.index.len(), 1);
        assert_eq!(
            result.columns.get("left").unwrap().values(),
            &[Scalar::Int64(20)]
        );
        assert_eq!(
            result.columns.get("right").unwrap().values(),
            &[Scalar::Int64(30)]
        );
        assert!(!left.column("key").unwrap().scalar_cache_is_materialized());
        assert!(!right.column("key").unwrap().scalar_cache_is_materialized());
    }
}
