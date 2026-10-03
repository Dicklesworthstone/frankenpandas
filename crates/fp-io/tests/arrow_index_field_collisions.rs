use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel, MultiIndex};

fn assert_arrow_roundtrips(frame: &DataFrame) {
    let formats = [
        (
            "parquet",
            fp_io::write_parquet_bytes as fn(&DataFrame) -> _,
            fp_io::read_parquet_bytes as fn(&[u8]) -> _,
        ),
        (
            "feather",
            fp_io::write_feather_bytes,
            fp_io::read_feather_bytes,
        ),
        (
            "ipc",
            fp_io::write_ipc_stream_bytes,
            fp_io::read_ipc_stream_bytes,
        ),
    ];
    for (format, write, read) in formats {
        let bytes = write(frame).unwrap_or_else(|error| panic!("{format} write: {error}"));
        let restored = read(&bytes).unwrap_or_else(|error| panic!("{format} read: {error}"));
        assert_eq!(restored.column_names(), frame.column_names(), "{format}");
        for name in frame.column_names() {
            assert_eq!(
                restored.column(name).unwrap().values(),
                frame.column(name).unwrap().values(),
                "{format} data column {name}"
            );
        }
        assert_eq!(
            restored.index().labels(),
            frame.index().labels(),
            "{format}"
        );
        assert_eq!(restored.index().name(), frame.index().name(), "{format}");
        match (restored.row_multiindex(), frame.row_multiindex()) {
            (Some(restored), Some(original)) => {
                assert_eq!(restored.names(), original.names(), "{format} index names");
                for level in 0..original.names().len() {
                    assert_eq!(
                        restored.get_level_values(level).unwrap().labels(),
                        original.get_level_values(level).unwrap().labels(),
                        "{format} level {level}"
                    );
                }
            }
            (None, None) => {}
            _ => panic!("{format} changed the row index structure"),
        }
    }
}

#[test]
fn unnamed_index_preserves_data_with_a_synthetic_index_field_name() {
    let columns = BTreeMap::from([
        (
            "__index_level_0__".to_owned(),
            Column::from_i64_values(vec![10, 20]),
        ),
        ("value".to_owned(), Column::from_i64_values(vec![1, 2])),
    ]);
    let frame = DataFrame::new_with_column_order(
        Index::from_utf8(vec!["r1".to_owned(), "r2".to_owned()]),
        columns,
        vec!["value".to_owned(), "__index_level_0__".to_owned()],
    )
    .unwrap();
    assert_arrow_roundtrips(&frame);
}

#[test]
fn named_index_preserves_data_when_both_preferred_and_fallback_names_collide() {
    let columns = BTreeMap::from([
        ("key".to_owned(), Column::from_i64_values(vec![10, 20])),
        (
            "__index_level_0__".to_owned(),
            Column::from_i64_values(vec![30, 40]),
        ),
        (
            "__index_level_0_1__".to_owned(),
            Column::from_i64_values(vec![50, 60]),
        ),
    ]);
    let frame = DataFrame::new_with_column_order(
        Index::from_utf8(vec!["r1".to_owned(), "r2".to_owned()]).rename_index(Some("key")),
        columns,
        vec![
            "key".to_owned(),
            "__index_level_0__".to_owned(),
            "__index_level_0_1__".to_owned(),
        ],
    )
    .unwrap();
    assert_arrow_roundtrips(&frame);
}

#[test]
fn multiindex_preserves_duplicate_logical_names_with_distinct_physical_fields() {
    let row_index = MultiIndex::from_arrays(vec![
        vec![IndexLabel::Utf8("a".into()), IndexLabel::Utf8("b".into())],
        vec![IndexLabel::Int64(100), IndexLabel::Int64(200)],
    ])
    .unwrap()
    .set_names(vec![Some("key".into()), Some("key".into())]);
    let frame = DataFrame::new_with_column_order(
        row_index.to_flat_index("|"),
        BTreeMap::from([("value".to_owned(), Column::from_i64_values(vec![10, 20]))]),
        vec!["value".to_owned()],
    )
    .unwrap()
    .with_row_multiindex(row_index)
    .unwrap();
    assert_arrow_roundtrips(&frame);
}

#[test]
fn multiindex_preserves_an_independent_textual_flat_index_name() {
    let row_index = MultiIndex::from_arrays(vec![
        vec![IndexLabel::Utf8("a".into()), IndexLabel::Utf8("b".into())],
        vec![IndexLabel::Int64(100), IndexLabel::Int64(200)],
    ])
    .unwrap()
    .set_names(vec![None, Some("key".into())]);
    for flat_name in ["custom axis", ""] {
        let frame = DataFrame::new_with_column_order(
            row_index.to_flat_index("|").rename_index(Some(flat_name)),
            BTreeMap::from([("value".to_owned(), Column::from_i64_values(vec![10, 20]))]),
            vec!["value".to_owned()],
        )
        .unwrap()
        .with_row_multiindex(row_index.clone())
        .unwrap();
        assert_arrow_roundtrips(&frame);
    }
}

#[test]
fn multiindex_preserves_the_set_index_multi_flat_name() {
    let frame = DataFrame::new_with_column_order(
        Index::default_range(2),
        BTreeMap::from([
            ("key".to_owned(), Column::from_i64_values(vec![1, 2])),
            ("other".to_owned(), Column::from_i64_values(vec![3, 4])),
            ("value".to_owned(), Column::from_i64_values(vec![10, 20])),
        ]),
        vec!["key".to_owned(), "other".to_owned(), "value".to_owned()],
    )
    .unwrap()
    .set_index_multi(&["key", "other"], true, "|")
    .unwrap();
    assert_eq!(frame.index().name().unwrap().as_str(), "key|other");
    assert_arrow_roundtrips(&frame);
}

#[test]
fn foreign_pandas_metadata_uses_logical_names_for_the_flat_index() {
    use std::sync::Arc;

    use arrow::{
        datatypes::Schema,
        ipc::{reader::StreamReader, writer::StreamWriter},
        record_batch::RecordBatch,
    };

    let row_index = MultiIndex::from_arrays(vec![
        vec![IndexLabel::Utf8("a".into()), IndexLabel::Utf8("b".into())],
        vec![IndexLabel::Int64(100), IndexLabel::Int64(200)],
    ])
    .unwrap()
    .set_names(vec![Some("key".into()), Some("key".into())]);
    let frame = DataFrame::new_with_column_order(
        row_index.to_flat_index("|"),
        BTreeMap::from([("value".to_owned(), Column::from_i64_values(vec![10, 20]))]),
        vec!["value".to_owned()],
    )
    .unwrap()
    .with_row_multiindex(row_index)
    .unwrap();
    let bytes = fp_io::write_ipc_stream_bytes(&frame).unwrap();
    let batch = StreamReader::try_new(std::io::Cursor::new(bytes), None)
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    let mut metadata = batch.schema().metadata().clone();
    let mut pandas: serde_json::Value = serde_json::from_str(&metadata["pandas"]).unwrap();
    pandas.as_object_mut().unwrap().remove("frankenpandas");
    metadata.insert("pandas".to_owned(), pandas.to_string());
    let schema = Arc::new(Schema::new_with_metadata(
        batch.schema().fields().clone(),
        metadata,
    ));
    let batch = RecordBatch::try_new(schema.clone(), batch.columns().to_vec()).unwrap();
    let mut bytes = Vec::new();
    let mut writer = StreamWriter::try_new(&mut bytes, &schema).unwrap();
    writer.write(&batch).unwrap();
    writer.finish().unwrap();
    drop(writer);

    let restored = fp_io::read_ipc_stream_bytes(&bytes).unwrap();
    assert_eq!(restored.index().name().unwrap().as_str(), "key|key");
    assert_eq!(restored.index().labels(), frame.index().labels());
    assert_eq!(
        restored.row_multiindex().unwrap().names(),
        frame.row_multiindex().unwrap().names()
    );
    assert_eq!(restored.column_names(), frame.column_names());
    assert_eq!(
        restored.column("value").unwrap().values(),
        frame.column("value").unwrap().values()
    );
}
