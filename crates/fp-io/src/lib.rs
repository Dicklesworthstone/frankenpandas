#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use csv::{ReaderBuilder, WriterBuilder};
use fp_columnar::{Column, ColumnError};
use fp_frame::{DataFrame, FrameError};
use fp_index::Index;
use fp_types::{NullKind, Scalar};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum IoError {
    #[error("csv input has no headers")]
    MissingHeaders,
    #[error(transparent)]
    Csv(#[from] csv::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Utf8(#[from] std::string::FromUtf8Error),
    #[error(transparent)]
    Column(#[from] ColumnError),
    #[error(transparent)]
    Frame(#[from] FrameError),
}

pub fn read_csv_str(input: &str) -> Result<DataFrame, IoError> {
    let mut reader = ReaderBuilder::new()
        .has_headers(true)
        .from_reader(input.as_bytes());

    let headers = reader.headers().cloned().map_err(IoError::from)?;

    if headers.is_empty() {
        return Err(IoError::MissingHeaders);
    }

    // AG-07: Vec-based column accumulation (O(1) per cell vs O(log c) BTreeMap).
    // Capacity hint from byte length avoids reallocation for typical CSVs.
    let header_count = headers.len();
    let row_hint = input.len() / (header_count * 8).max(1);
    let mut columns: Vec<Vec<Scalar>> = (0..header_count)
        .map(|_| Vec::with_capacity(row_hint))
        .collect();

    let mut row_count: i64 = 0;
    for row in reader.records() {
        let record = row?;
        for (idx, col) in columns.iter_mut().enumerate() {
            let field = record.get(idx).unwrap_or_default();
            col.push(parse_scalar(field));
        }
        row_count += 1;
    }

    let mut out_columns = BTreeMap::new();
    for (idx, values) in columns.into_iter().enumerate() {
        let name = headers.get(idx).unwrap_or_default().to_owned();
        out_columns.insert(name, Column::from_values(values)?);
    }

    let index = Index::from_i64((0..row_count).collect());
    Ok(DataFrame::new(index, out_columns)?)
}

pub fn write_csv_string(frame: &DataFrame) -> Result<String, IoError> {
    let mut writer = WriterBuilder::new().from_writer(Vec::new());

    let headers = frame.columns().keys().cloned().collect::<Vec<_>>();
    writer.write_record(&headers)?;

    for row_idx in 0..frame.index().len() {
        let row = headers
            .iter()
            .map(|name| {
                frame
                    .column(name)
                    .and_then(|column| column.value(row_idx))
                    .map_or_else(String::new, scalar_to_csv)
            })
            .collect::<Vec<_>>();
        writer.write_record(&row)?;
    }

    let bytes = writer.into_inner().map_err(|err| err.into_error())?;
    Ok(String::from_utf8(bytes)?)
}

fn parse_scalar(field: &str) -> Scalar {
    let trimmed = field.trim();
    if trimmed.is_empty() {
        return Scalar::Null(NullKind::Null);
    }

    if let Ok(value) = trimmed.parse::<i64>() {
        return Scalar::Int64(value);
    }
    if let Ok(value) = trimmed.parse::<f64>() {
        return Scalar::Float64(value);
    }
    if let Ok(value) = trimmed.parse::<bool>() {
        return Scalar::Bool(value);
    }

    Scalar::Utf8(trimmed.to_owned())
}

fn scalar_to_csv(scalar: &Scalar) -> String {
    match scalar {
        Scalar::Null(_) => String::new(),
        Scalar::Bool(v) => v.to_string(),
        Scalar::Int64(v) => v.to_string(),
        Scalar::Float64(v) => {
            if v.is_nan() {
                String::new()
            } else {
                v.to_string()
            }
        }
        Scalar::Utf8(v) => v.clone(),
    }
}

#[cfg(test)]
mod tests {
    use fp_types::{NullKind, Scalar};

    use super::{read_csv_str, write_csv_string};

    #[test]
    fn csv_round_trip_preserves_null_and_numeric_shape() {
        let input = "id,value\n1,10\n2,\n3,3.5\n";
        let frame = read_csv_str(input).expect("read");
        let value_col = frame.column("value").expect("value");

        assert_eq!(value_col.values()[1], Scalar::Null(NullKind::NaN));

        let out = write_csv_string(&frame).expect("write");
        assert!(out.contains("id,value"));
        assert!(out.contains("3,3.5"));
    }

    // === AG-07-T: CSV Parser Optimization Tests ===

    #[test]
    fn test_csv_vec_based_column_order() {
        // Verify Vec-based parser preserves header-to-data mapping exactly.
        // BTreeMap sorts alphabetically, so we use alpha-ordered headers.
        let input = "alpha,bravo,charlie\n1,2,3\n4,5,6\n";
        let frame = read_csv_str(input).expect("parse");
        let keys: Vec<&String> = frame.columns().keys().collect();
        assert_eq!(keys, &["alpha", "bravo", "charlie"]);
        assert_eq!(frame.column("alpha").unwrap().values()[0], Scalar::Int64(1));
        assert_eq!(frame.column("bravo").unwrap().values()[0], Scalar::Int64(2));
        assert_eq!(
            frame.column("charlie").unwrap().values()[1],
            Scalar::Int64(6)
        );
        eprintln!("[TEST] test_csv_vec_based_column_order | rows=2 cols=3 parse_ok=true | PASS");
    }

    #[test]
    fn test_csv_capacity_hint_reasonable() {
        // Generate a ~1MB CSV and verify it parses correctly.
        // The capacity hint (input.len / (cols*8)) should avoid excessive reallocs.
        let mut csv = String::with_capacity(1_100_000);
        csv.push_str("a,b,c,d,e\n");
        let target_rows = 50_000; // ~20 bytes/row * 50k ≈ 1MB
        for i in 0..target_rows {
            csv.push_str(&format!("{},{},{},{},{}\n", i, i * 2, i * 3, i * 4, i * 5));
        }
        assert!(csv.len() > 500_000, "CSV should be large");

        let frame = read_csv_str(&csv).expect("parse large CSV");
        assert_eq!(frame.index().len(), target_rows);
        assert_eq!(frame.columns().len(), 5);
        // Spot-check last row
        assert_eq!(
            frame.column("a").unwrap().values()[target_rows - 1],
            Scalar::Int64((target_rows - 1) as i64)
        );
        eprintln!(
            "[TEST] test_csv_capacity_hint_reasonable | rows={target_rows} cols=5 parse_ok=true | PASS"
        );
    }

    #[test]
    fn test_csv_empty_columns() {
        // CSV with headers but no data rows -> empty DataFrame with correct column names.
        let input = "x,y,z\n";
        let frame = read_csv_str(input).expect("parse");
        assert_eq!(frame.index().len(), 0);
        let keys: Vec<&String> = frame.columns().keys().collect();
        assert_eq!(keys, &["x", "y", "z"]);
        for col in frame.columns().values() {
            assert!(col.is_empty());
        }
        eprintln!("[TEST] test_csv_empty_columns | rows=0 cols=3 parse_ok=true | PASS");
    }

    #[test]
    fn test_csv_single_column() {
        // CSV with one column, many rows -> correct parsing.
        let mut csv = String::from("value\n");
        for i in 0..500 {
            csv.push_str(&format!("{}\n", i));
        }
        let frame = read_csv_str(&csv).expect("parse");
        assert_eq!(frame.index().len(), 500);
        assert_eq!(frame.columns().len(), 1);
        assert_eq!(
            frame.column("value").unwrap().values()[499],
            Scalar::Int64(499)
        );
        eprintln!("[TEST] test_csv_single_column | rows=500 cols=1 parse_ok=true | PASS");
    }

    #[test]
    fn test_csv_many_columns() {
        // CSV with 100+ columns -> all columns present, correct values.
        let col_count = 120;
        let headers: Vec<String> = (0..col_count).map(|i| format!("c{i:03}")).collect();
        let mut csv = headers.join(",");
        csv.push('\n');
        // 3 data rows
        for row in 0..3 {
            let vals: Vec<String> = (0..col_count)
                .map(|c| format!("{}", row * 1000 + c))
                .collect();
            csv.push_str(&vals.join(","));
            csv.push('\n');
        }
        let frame = read_csv_str(&csv).expect("parse");
        assert_eq!(frame.columns().len(), col_count);
        assert_eq!(frame.index().len(), 3);
        // Spot-check: c000 row 0 = 0, c119 row 2 = 2119
        assert_eq!(frame.column("c000").unwrap().values()[0], Scalar::Int64(0));
        assert_eq!(
            frame.column("c119").unwrap().values()[2],
            Scalar::Int64(2119)
        );
        eprintln!("[TEST] test_csv_many_columns | rows=3 cols={col_count} parse_ok=true | PASS");
    }

    #[test]
    fn test_csv_mixed_dtypes() {
        // Columns with uniform int/float/string/bool/null -> correct type inference.
        let input = "ints,floats,strings,bools,nulls\n\
                     1,1.5,hello,true,\n\
                     2,2.7,world,false,\n\
                     3,3.14,foo,true,\n";
        let frame = read_csv_str(input).expect("parse");

        let ints = frame.column("ints").unwrap();
        assert_eq!(ints.values()[0], Scalar::Int64(1));

        let floats = frame.column("floats").unwrap();
        assert_eq!(floats.values()[1], Scalar::Float64(2.7));

        let strings = frame.column("strings").unwrap();
        assert_eq!(strings.values()[2], Scalar::Utf8("foo".to_owned()));

        let bools = frame.column("bools").unwrap();
        assert_eq!(bools.values()[0], Scalar::Bool(true));
        assert_eq!(bools.values()[1], Scalar::Bool(false));

        // "nulls" column is all empty -> all null/NaN
        let nulls = frame.column("nulls").unwrap();
        for v in nulls.values() {
            assert!(v.is_missing(), "null column values should be missing");
        }
        eprintln!(
            "[TEST] test_csv_mixed_dtypes | rows=3 cols=5 parse_ok=true | dtype_per_col=[int64,float64,utf8,bool,null] | PASS"
        );
    }

    #[test]
    fn test_csv_unicode_headers() {
        // CSV with unicode header names -> correct column names.
        let input = "名前,Größe,café\nAlice,170,latte\nBob,180,espresso\n";
        let frame = read_csv_str(input).expect("parse");
        assert!(frame.column("名前").is_some());
        assert!(frame.column("Größe").is_some());
        assert!(frame.column("café").is_some());
        assert_eq!(
            frame.column("名前").unwrap().values()[0],
            Scalar::Utf8("Alice".to_owned())
        );
        eprintln!("[TEST] test_csv_unicode_headers | rows=2 cols=3 parse_ok=true | PASS");
    }

    #[test]
    fn test_csv_quoted_fields() {
        // CSV with quoted fields containing commas and newlines -> correct parsing.
        let input =
            "name,address\n\"Smith, John\",\"123 Main St\nApt 4\"\nJane,\"456 Oak, Suite 1\"\n";
        let frame = read_csv_str(input).expect("parse");
        assert_eq!(frame.index().len(), 2);
        assert_eq!(
            frame.column("name").unwrap().values()[0],
            Scalar::Utf8("Smith, John".to_owned())
        );
        // Quoted field with embedded newline
        let addr0 = &frame.column("address").unwrap().values()[0];
        match addr0 {
            Scalar::Utf8(s) => assert!(s.contains('\n'), "should contain embedded newline"),
            other => panic!("expected Utf8, got {:?}", other),
        }
        eprintln!("[TEST] test_csv_quoted_fields | rows=2 cols=2 parse_ok=true | PASS");
    }

    #[test]
    fn test_csv_trailing_newline() {
        // CSV with/without trailing newline -> identical DataFrame.
        let with = "a,b\n1,2\n3,4\n";
        let without = "a,b\n1,2\n3,4";
        let f1 = read_csv_str(with).expect("with newline");
        let f2 = read_csv_str(without).expect("without newline");

        assert_eq!(f1.index().len(), f2.index().len());
        assert_eq!(f1.columns().len(), f2.columns().len());
        for key in f1.columns().keys() {
            let c1 = f1.column(key).unwrap();
            let c2 = f2.column(key).unwrap();
            assert_eq!(c1.values(), c2.values(), "column {key} mismatch");
        }
        eprintln!("[TEST] test_csv_trailing_newline | rows=2 cols=2 parse_ok=true | PASS");
    }

    #[test]
    fn test_csv_round_trip_unchanged() {
        // read_csv_str then write_csv_string produces semantically equivalent output.
        let input = "id,name,score\n1,Alice,95.5\n2,Bob,87\n3,,100\n";
        let frame = read_csv_str(input).expect("read");
        let output = write_csv_string(&frame).expect("write");
        // Re-parse the output and compare
        let frame2 = read_csv_str(&output).expect("re-read");
        assert_eq!(frame.index().len(), frame2.index().len());
        for key in frame.columns().keys() {
            let c1 = frame.column(key).unwrap();
            let c2 = frame2.column(key).unwrap();
            assert!(
                c1.semantic_eq(c2),
                "column {key} not semantically equal after round-trip"
            );
        }
        eprintln!("[TEST] test_csv_round_trip_unchanged | rows=3 cols=3 parse_ok=true | PASS");
    }

    #[test]
    fn test_csv_large_file_perf() {
        // 100K-row, 10-column CSV -> parse completes, correct row/column counts.
        let col_count = 10;
        let row_count = 100_000;
        let headers: Vec<String> = (0..col_count).map(|i| format!("col{i}")).collect();
        let mut csv = String::with_capacity(row_count * 50);
        csv.push_str(&headers.join(","));
        csv.push('\n');
        for r in 0..row_count {
            for c in 0..col_count {
                if c > 0 {
                    csv.push(',');
                }
                csv.push_str(&(r * col_count + c).to_string());
            }
            csv.push('\n');
        }

        let frame = read_csv_str(&csv).expect("parse 100K rows");
        assert_eq!(frame.index().len(), row_count);
        assert_eq!(frame.columns().len(), col_count);
        // Spot-check first and last rows
        assert_eq!(frame.column("col0").unwrap().values()[0], Scalar::Int64(0));
        assert_eq!(
            frame.column("col9").unwrap().values()[row_count - 1],
            Scalar::Int64(((row_count - 1) * col_count + 9) as i64)
        );
        eprintln!(
            "[TEST] test_csv_large_file_perf | rows={row_count} cols={col_count} parse_ok=true | PASS"
        );
    }

    #[test]
    fn test_csv_golden_output() {
        // Fixed CSV input -> write_csv_string output matches golden reference exactly.
        let input = "a,b,c\n1,hello,3.14\n2,,true\n3,world,\n";
        let frame = read_csv_str(input).expect("parse");
        let output = write_csv_string(&frame).expect("write");

        // Golden reference: columns in BTreeMap order; Bool(true) coerced to Float64
        // in column c (which has Float64 + Bool → Float64), so true → 1.
        let expected = "a,b,c\n1,hello,3.14\n2,,1\n3,world,\n";
        assert_eq!(
            output, expected,
            "output does not match golden reference.\nGot:\n{output}\nExpected:\n{expected}"
        );
        eprintln!("[TEST] test_csv_golden_output | golden_match=true | PASS");
    }
}
