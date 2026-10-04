//! Foreign stock Arrow/Parquet constructions, separate from genuine old-writer files.

use std::{collections::HashMap, sync::Arc};

use arrow::{
    array::{ArrayRef, Int64Array},
    datatypes::{DataType, Field, Schema},
    ipc::writer::{FileWriter, StreamWriter},
    record_batch::RecordBatch,
};
use fp_frame::DataFrame;
use fp_index::IndexLabel;
use fp_types::Scalar;
use parquet::{
    arrow::ArrowWriter,
    file::{metadata::KeyValue, properties::WriterProperties},
};

const LEGACY: &str = "frankenpandas.row_multiindex_names";
const FIELDS: [&str; 4] = [
    "__index_level_0__",
    "__index_level_1__",
    "__index_level_2__",
    "value",
];

#[derive(Clone, Copy, Debug)]
enum Format {
    Parquet,
    Feather,
    Ipc,
}

const FORMATS: [Format; 3] = [Format::Parquet, Format::Feather, Format::Ipc];

fn batch(fields: &[&str], metadata: HashMap<String, String>) -> RecordBatch {
    let schema = Arc::new(Schema::new_with_metadata(
        fields
            .iter()
            .map(|name| Field::new(*name, DataType::Int64, false))
            .collect::<Vec<_>>(),
        metadata,
    ));
    let arrays = (0..fields.len())
        .map(|position| {
            let base = 10 * i64::try_from(position).unwrap();
            Arc::new(Int64Array::from(vec![base + 1, base + 2])) as ArrayRef
        })
        .collect();
    RecordBatch::try_new(schema, arrays).unwrap()
}

fn encoded(batch: &RecordBatch, format: Format, file_entries: Vec<KeyValue>) -> Vec<u8> {
    let mut bytes = Vec::new();
    match format {
        Format::Parquet => {
            let properties = WriterProperties::builder()
                .set_key_value_metadata(Some(file_entries))
                .build();
            let mut writer =
                ArrowWriter::try_new(&mut bytes, batch.schema(), Some(properties)).unwrap();
            writer.write(batch).unwrap();
            writer.close().unwrap();
        }
        Format::Feather => {
            assert!(file_entries.is_empty());
            let mut writer = FileWriter::try_new(&mut bytes, &batch.schema()).unwrap();
            writer.write(batch).unwrap();
            writer.finish().unwrap();
        }
        Format::Ipc => {
            assert!(file_entries.is_empty());
            let mut writer = StreamWriter::try_new(&mut bytes, &batch.schema()).unwrap();
            writer.write(batch).unwrap();
            writer.finish().unwrap();
        }
    }
    bytes
}

fn read(bytes: &[u8], format: Format) -> Result<DataFrame, fp_io::IoError> {
    match format {
        Format::Parquet => fp_io::read_parquet_bytes(bytes),
        Format::Feather => fp_io::read_feather_bytes(bytes),
        Format::Ipc => fp_io::read_ipc_stream_bytes(bytes),
    }
}

fn legacy(raw: &str) -> HashMap<String, String> {
    HashMap::from([(LEGACY.to_owned(), raw.to_owned())])
}

fn entry(value: Option<&str>) -> KeyValue {
    KeyValue {
        key: LEGACY.to_owned(),
        value: value.map(str::to_owned),
    }
}

#[test]
fn absent_legacy_key_keeps_every_synthetic_looking_column_as_data() {
    let batch = batch(&FIELDS, HashMap::new());
    for format in FORMATS {
        let frame = read(&encoded(&batch, format, vec![]), format).unwrap();
        assert!(frame.row_multiindex().is_none(), "{format:?}");
        assert_eq!(frame.len(), 2, "{format:?}");
        assert_eq!(frame.num_columns(), FIELDS.len(), "{format:?}");
        assert_eq!(
            frame
                .column_names()
                .iter()
                .map(|name| name.as_str())
                .collect::<Vec<_>>(),
            FIELDS,
            "{format:?}"
        );
        assert_eq!(
            frame.index().labels(),
            &[IndexLabel::Int64(0), IndexLabel::Int64(1)]
        );
        for (position, name) in FIELDS.iter().enumerate() {
            let base = 10 * i64::try_from(position).unwrap();
            assert_eq!(
                frame.column(name).unwrap().values(),
                &[Scalar::Int64(base + 1), Scalar::Int64(base + 2)],
                "{format:?} {name}"
            );
        }
    }
}

#[test]
fn malformed_legacy_type_and_count_are_refused_in_every_format() {
    for raw in [
        "not json",
        "null",
        "{}",
        "[]",
        r#"["only"]"#,
        r#"["a",1]"#,
        r#"["a","b","c","d","e"]"#,
    ] {
        let batch = batch(&FIELDS, legacy(raw));
        for format in FORMATS {
            let error = read(&encoded(&batch, format, vec![]), format).unwrap_err();
            assert!(
                error.to_string().contains("legacy MultiIndex"),
                "{format:?} {raw}: {error}"
            );
        }
    }
}

#[test]
fn valid_names_with_missing_reordered_or_duplicate_declared_fields_are_refused() {
    let layouts = [
        vec!["__index_level_0__", "value"],
        vec!["__index_level_1__", "__index_level_0__", "value"],
        vec![
            "__index_level_0__",
            "__index_level_1__",
            "__index_level_0__",
            "value",
        ],
    ];
    for fields in layouts {
        let batch = batch(&fields, legacy(r#"["a","b"]"#));
        // IPC allows duplicate physical field names; do not assume Parquet's
        // writer accepts them or count a writer refusal as a reader refusal.
        for format in [Format::Feather, Format::Ipc] {
            let error = read(&encoded(&batch, format, vec![]), format).unwrap_err();
            assert!(
                error.to_string().contains("leading index fields"),
                "{format:?}: {error}"
            );
        }
    }
}

#[test]
fn pandas_index_metadata_takes_priority_over_invalid_legacy_metadata() {
    let mut metadata = legacy("not json");
    let pandas = serde_json::json!({
        "index_columns": [FIELDS[0]],
        "columns": [{"field_name": FIELDS[0], "name": "authoritative"}],
    })
    .to_string();
    metadata.insert("pandas".to_owned(), pandas.clone());
    let batch = batch(&FIELDS, metadata);
    for format in FORMATS {
        let entries = if matches!(format, Format::Parquet) {
            vec![
                KeyValue {
                    key: "pandas".to_owned(),
                    value: Some(pandas.clone()),
                },
                entry(None),
                entry(Some("also invalid")),
            ]
        } else {
            vec![]
        };
        let frame = read(&encoded(&batch, format, entries), format).unwrap();
        assert!(frame.row_multiindex().is_none(), "{format:?}");
        assert_eq!(
            frame.index().name().unwrap().as_str(),
            "authoritative",
            "{format:?}"
        );
        assert_eq!(
            frame.index().labels(),
            &[IndexLabel::Int64(1), IndexLabel::Int64(2)]
        );
        assert_eq!(frame.num_columns(), 3, "{format:?}");
        assert_eq!(
            frame.column("value").unwrap().values(),
            &[Scalar::Int64(31), Scalar::Int64(32)]
        );
    }
}

#[test]
fn parquet_duplicate_and_null_legacy_file_entries_are_refused() {
    let batch = batch(&FIELDS, HashMap::new());
    for (entries, message) in [
        (
            vec![entry(Some(r#"["a","b"]"#)), entry(Some(r#"["a","b"]"#))],
            "duplicate",
        ),
        (
            vec![entry(Some(r#"["a","b"]"#)), entry(Some(r#"["x","y"]"#))],
            "duplicate",
        ),
        (vec![entry(None)], "no value"),
    ] {
        let error = read(&encoded(&batch, Format::Parquet, entries), Format::Parquet).unwrap_err();
        assert!(error.to_string().contains(message), "{error}");
    }
}

#[test]
fn parquet_disagreeing_raw_embedded_schema_and_file_names_are_refused() {
    // Stock ArrowWriter embeds this ORIGINAL schema in ARROW:schema. Its
    // reconstructed reader schema merges the file key over the same schema
    // key, so that merged map alone cannot establish raw-copy agreement.
    let batch = batch(&FIELDS, legacy(r#"["schema0","schema1"]"#));
    let bytes = encoded(
        &batch,
        Format::Parquet,
        vec![entry(Some(r#"["file0","file1"]"#))],
    );
    let error = read(&bytes, Format::Parquet).unwrap_err();
    assert!(
        error.to_string().contains("conflicting legacy MultiIndex"),
        "{error}"
    );
}
