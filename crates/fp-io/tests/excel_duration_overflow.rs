//! Public-API integration fixtures for elapsed-time XLSX conversion.
//! Real XLSX archives are produced by the existing locked rust_xlsxwriter and
//! classified by the existing locked calamine before the public fp-io reads.

use std::{
    io::{Cursor, Write},
    path::PathBuf,
};

use calamine::{Data, Reader, open_workbook_auto_from_rs};
use fp_io::{ExcelReadOptions, IoError, read_excel, read_excel_bytes};
use fp_types::{DType, NullKind, Scalar};
use rust_xlsxwriter::{Format, Workbook};

const MILLIS_PER_DAY: f64 = 86_400_000.0;
const MAX_WHOLE_MILLIS: f64 = 9_223_372_036_854.0;

fn duration_workbook(values: &[Option<f64>], duration_header: Option<f64>) -> Vec<u8> {
    let mut workbook = Workbook::new();
    let format = Format::new().set_num_format("[h]:mm:ss.000");
    let sheet = workbook.add_worksheet();
    sheet.set_name("Durations").expect("valid sheet name");
    sheet.write_string(0, 0, "row").expect("row header");
    if let Some(days) = duration_header {
        sheet
            .write_number_with_format(0, 1, days, &format)
            .expect("duration header");
    } else {
        sheet.write_string(0, 1, "elapsed").expect("elapsed header");
    }
    for (i, days) in values.iter().enumerate() {
        let row = u32::try_from(i + 1).expect("small fixture row");
        sheet
            .write_string(row, 0, format!("r{i}"))
            .expect("row label keeps even a blank duration row present");
        if let Some(days) = days {
            sheet
                .write_number_with_format(row, 1, *days, &format)
                .expect("finite duration value");
        }
    }
    let bytes = workbook.save_to_buffer().expect("real XLSX archive");
    assert!(
        bytes.starts_with(b"PK"),
        "fixture must be a ZIP/XLSX archive"
    );

    // Prevent a false green through an ordinary Float cell or a text fixture.
    let mut decoded = open_workbook_auto_from_rs(Cursor::new(bytes.as_slice()))
        .expect("real calamine reader opens the generated workbook");
    let range = decoded
        .worksheet_range("Durations")
        .expect("duration sheet");
    for (row, days) in duration_header
        .map(|days| (0, Some(days)))
        .into_iter()
        .chain(values.iter().enumerate().map(|(i, days)| (i + 1, *days)))
    {
        match (range.get((row, 1)), days) {
            (Some(Data::DateTime(value)), Some(days)) => {
                assert!(
                    value.is_duration(),
                    "row {row}: actual elapsed-time cell required"
                );
                assert_eq!(
                    value.as_f64().to_bits(),
                    days.to_bits(),
                    "raw serial preserved"
                );
            }
            (Some(Data::Empty), None) => {}
            (actual, expected) => {
                panic!("row {row}: real duration fixture mismatch: {actual:?} / {expected:?}")
            }
        }
    }
    bytes
}

fn retained_workbook(bytes: &[u8]) -> PathBuf {
    let id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "fp-io-excel-duration-regression-{}-{id}",
        std::process::id()
    ));
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(&directory)
        .expect("new exclusive own fixture directory");
    let path = directory.join("duration.xlsx");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(&path)
        .expect("new exclusive own workbook")
        .write_all(bytes)
        .expect("complete fixture bytes");
    println!("retained Excel duration fixture: {}", path.display());
    path
}

fn assert_duration_error(error: IoError) {
    let IoError::Excel(message) = error else {
        panic!("unrepresentable duration must report Excel error, got {error:?}");
    };
    assert!(message.contains("duration"), "{message}");
    assert!(message.contains("timedelta64[ns]"), "{message}");
    assert!(message.contains("millisecond"), "{message}");
}

#[test]
fn excel_duration_valid_values_and_exact_millisecond_bounds_are_preserved() {
    let days = [
        Some(0.0),
        Some(1.5),
        Some(-1.5),
        Some(100_000.0),
        Some(-100_000.0),
        Some(MAX_WHOLE_MILLIS / MILLIS_PER_DAY),
        Some(-MAX_WHOLE_MILLIS / MILLIS_PER_DAY),
        Some(1.0 / MILLIS_PER_DAY),
        Some(-1.0 / MILLIS_PER_DAY),
        None,
    ];
    let expected = vec![
        Scalar::Timedelta64(0),
        Scalar::Timedelta64(129_600_000_000_000),
        Scalar::Timedelta64(-129_600_000_000_000),
        Scalar::Timedelta64(8_640_000_000_000_000_000),
        Scalar::Timedelta64(-8_640_000_000_000_000_000),
        Scalar::Timedelta64(9_223_372_036_854_000_000),
        Scalar::Timedelta64(-9_223_372_036_854_000_000),
        Scalar::Timedelta64(1_000_000),
        Scalar::Timedelta64(-1_000_000),
        Scalar::Null(NullKind::NaT),
    ];
    let bytes = duration_workbook(&days, None);
    let original = bytes.clone();
    let path = retained_workbook(&bytes);
    for frame in [
        read_excel_bytes(&bytes, &ExcelReadOptions::default()).expect("valid duration bytes"),
        read_excel(&path, &ExcelReadOptions::default()).expect("valid duration path"),
    ] {
        assert_eq!(frame.index().len(), expected.len());
        let elapsed = frame.column("elapsed").expect("elapsed column");
        assert_eq!(elapsed.dtype(), DType::Timedelta64);
        assert_eq!(elapsed.values(), expected.as_slice());
        assert_eq!(
            frame.column("row").expect("row labels").values()[0],
            Scalar::Utf8("r0".into())
        );
    }
    assert_eq!(bytes, original);
    assert_eq!(std::fs::read(path).expect("retained input bytes"), original);
}

#[test]
fn excel_duration_overflow_is_a_controlled_error_for_data_and_index_cells() {
    for days in [
        200_000.0,
        -200_000.0,
        (MAX_WHOLE_MILLIS + 1.0) / MILLIS_PER_DAY,
        -(MAX_WHOLE_MILLIS + 1.0) / MILLIS_PER_DAY,
    ] {
        // A good row first makes silent null/coercion or a partial frame a failure.
        let bytes = duration_workbook(&[Some(1.5), Some(days)], None);
        let original = bytes.clone();
        let path = retained_workbook(&bytes);
        for options in [
            ExcelReadOptions::default(),
            ExcelReadOptions {
                index_col: Some("elapsed".into()),
                ..Default::default()
            },
        ] {
            assert_duration_error(
                read_excel_bytes(&bytes, &options).expect_err("reject overflow bytes"),
            );
            assert_duration_error(read_excel(&path, &options).expect_err("reject overflow path"));
        }
        assert_eq!(bytes, original);
        assert_eq!(
            std::fs::read(path).expect("retained rejected input"),
            original
        );
    }
}

#[test]
fn excel_duration_overflow_in_a_header_is_a_controlled_error() {
    let bytes = duration_workbook(&[Some(1.5)], Some(200_000.0));
    let original = bytes.clone();
    let path = retained_workbook(&bytes);
    assert_duration_error(
        read_excel_bytes(&bytes, &ExcelReadOptions::default())
            .expect_err("reject duration header bytes"),
    );
    assert_duration_error(
        read_excel(&path, &ExcelReadOptions::default()).expect_err("reject duration header path"),
    );
    assert_eq!(bytes, original);
    assert_eq!(
        std::fs::read(path).expect("retained rejected input"),
        original
    );
}
