#![forbid(unsafe_code)]

//! End-to-end integration test exercising a realistic pandas-like data
//! pipeline across multiple FrankenPandas subsystems (frankenpandas-enl).
//!
//! Pipeline: CSV read → to_datetime → query filter → groupby agg →
//! merge join → sort → export to CSV/JSON/Feather/SQL.

use fp_expr::DataFrameExprExt;
use fp_frame::{DataFrame, Series};
use fp_join::DataFrameMergeExt;
use fp_types::Scalar;

/// Build a sample "trades" dataset as CSV text.
fn sample_trades_csv() -> &'static str {
    "date,ticker,price,volume\n\
     2024-01-15,AAPL,185.50,1000\n\
     2024-01-15,GOOG,140.25,500\n\
     2024-01-16,AAPL,186.00,1200\n\
     2024-01-16,GOOG,141.00,800\n\
     2024-01-17,AAPL,184.75,900\n\
     2024-01-17,GOOG,142.50,600\n\
     2024-01-18,AAPL,187.25,1100\n\
     2024-01-18,GOOG,143.00,700\n"
}

/// Build a sample "sector" lookup table.
fn sample_sectors() -> DataFrame {
    DataFrame::from_dict(
        &["ticker", "sector"],
        vec![
            (
                "ticker",
                vec![
                    Scalar::Utf8("AAPL".into()),
                    Scalar::Utf8("GOOG".into()),
                    Scalar::Utf8("MSFT".into()),
                ],
            ),
            (
                "sector",
                vec![
                    Scalar::Utf8("Technology".into()),
                    Scalar::Utf8("Technology".into()),
                    Scalar::Utf8("Technology".into()),
                ],
            ),
        ],
    )
    .unwrap()
}

// ── Step 1: CSV Read with options ────────────────────────────────────

#[test]
fn e2e_step1_csv_read() {
    let csv = sample_trades_csv();
    let frame = fp_io::read_csv_str(csv).expect("CSV read failed");

    assert_eq!(frame.index().len(), 8, "expected 8 trade rows");
    assert_eq!(frame.column_names().len(), 4, "expected 4 columns");
    assert!(frame.column("date").is_some());
    assert!(frame.column("ticker").is_some());
    assert!(frame.column("price").is_some());
    assert!(frame.column("volume").is_some());
}

#[test]
fn e2e_step1_csv_read_with_options() {
    let csv = sample_trades_csv();
    let opts = fp_io::CsvReadOptions {
        usecols: Some(vec!["date".into(), "ticker".into(), "price".into()]),
        nrows: Some(4),
        ..Default::default()
    };
    let frame = fp_io::read_csv_with_options(csv, &opts).expect("CSV read with options failed");

    assert_eq!(frame.index().len(), 4, "nrows=4 should limit to 4 rows");
    assert_eq!(
        frame.column_names().len(),
        3,
        "usecols should select 3 columns"
    );
    assert!(frame.column("volume").is_none(), "volume not in usecols");
}

// ── Step 2: DateTime parsing ─────────────────────────────────────────

#[test]
fn e2e_step2_to_datetime() {
    let csv = sample_trades_csv();
    let frame = fp_io::read_csv_str(csv).unwrap();

    let date_col_series = Series::new(
        "date".to_owned(),
        frame.index().clone(),
        frame.column("date").unwrap().clone(),
    )
    .unwrap();

    let parsed = fp_frame::to_datetime(&date_col_series).expect("to_datetime failed");
    assert_eq!(parsed.len(), 8);

    // All dates should be normalized to "YYYY-MM-DD 00:00:00" format.
    if let Scalar::Utf8(s) = &parsed.values()[0] {
        assert!(
            s.starts_with("2024-01-15"),
            "expected 2024-01-15, got: {s}"
        );
        assert!(s.contains("00:00:00"), "expected time component, got: {s}");
    } else {
        panic!("expected Utf8 datetime string");
    }
}

// ── Step 3: Query filter ─────────────────────────────────────────────

#[test]
fn e2e_step3_query_filter() {
    let csv = sample_trades_csv();
    let frame = fp_io::read_csv_str(csv).unwrap();

    // Filter: price > 185
    let filtered = frame.query("price > 185").expect("query failed");
    assert!(filtered.index().len() < 8, "filter should reduce rows");

    // All remaining prices should be > 185
    for val in filtered.column("price").unwrap().values() {
        if let Ok(v) = val.to_f64() {
            assert!(v > 185.0, "filtered value {v} should be > 185");
        }
    }
}

// ── Step 4: GroupBy aggregation ──────────────────────────────────────

#[test]
fn e2e_step4_groupby_agg() {
    let csv = sample_trades_csv();
    let frame = fp_io::read_csv_str(csv).unwrap();

    let grouped = frame
        .groupby(&["ticker"])
        .expect("groupby failed")
        .sum()
        .expect("sum failed");

    // Two tickers: AAPL and GOOG.
    assert_eq!(grouped.index().len(), 2, "expected 2 groups");

    // Volume column should have summed values.
    let volume = grouped.column("volume").expect("volume column missing");
    for val in volume.values() {
        if let Ok(v) = val.to_f64() {
            assert!(v > 0.0, "summed volume should be positive");
        }
    }
}

// ── Step 5: Merge/join ───────────────────────────────────────────────

#[test]
fn e2e_step5_merge() {
    let csv = sample_trades_csv();
    let trades = fp_io::read_csv_str(csv).unwrap();
    let sectors = sample_sectors();

    // Merge on shared "ticker" column.
    let merged = trades
        .merge(&sectors, &["ticker"], fp_join::JoinType::Inner)
        .expect("merge failed");

    // All trade rows should be present (inner merge on ticker).
    assert!(!merged.index.is_empty(), "merged should have rows");
    assert!(
        merged.columns.contains_key("sector"),
        "sector column should be in merged result"
    );
    assert!(
        merged.columns.contains_key("price"),
        "price column should be in merged result"
    );
}

// ── Step 6: Sort ─────────────────────────────────────────────────────

#[test]
fn e2e_step6_sort() {
    let csv = sample_trades_csv();
    let frame = fp_io::read_csv_str(csv).unwrap();

    let sorted = frame
        .sort_values("price", false)
        .expect("sort_values failed");

    // Prices should be in descending order.
    let prices = sorted.column("price").unwrap();
    for i in 1..prices.len() {
        let prev = prices.values()[i - 1].to_f64().unwrap_or(f64::NAN);
        let curr = prices.values()[i].to_f64().unwrap_or(f64::NAN);
        assert!(
            prev >= curr,
            "descending sort: {prev} should >= {curr} at index {i}"
        );
    }
}

// ── Step 7: Export to multiple formats ───────────────────────────────

#[test]
fn e2e_step7_export_csv_json() {
    let csv = sample_trades_csv();
    let frame = fp_io::read_csv_str(csv).unwrap();

    // CSV round-trip.
    let csv_out = fp_io::write_csv_string(&frame).expect("CSV write failed");
    let csv_back = fp_io::read_csv_str(&csv_out).expect("CSV re-read failed");
    assert_eq!(csv_back.index().len(), frame.index().len());

    // JSON Records round-trip.
    let json_out =
        fp_io::write_json_string(&frame, fp_io::JsonOrient::Records).expect("JSON write failed");
    let json_back =
        fp_io::read_json_str(&json_out, fp_io::JsonOrient::Records).expect("JSON re-read failed");
    assert_eq!(json_back.index().len(), frame.index().len());
}

#[test]
fn e2e_step7_export_feather() {
    let csv = sample_trades_csv();
    let frame = fp_io::read_csv_str(csv).unwrap();

    // Feather round-trip.
    let feather_bytes = fp_io::write_feather_bytes(&frame).expect("Feather write failed");
    let feather_back =
        fp_io::read_feather_bytes(&feather_bytes).expect("Feather re-read failed");
    assert_eq!(feather_back.index().len(), frame.index().len());
    assert_eq!(feather_back.column_names().len(), frame.column_names().len());
}

#[test]
fn e2e_step7_export_sql() {
    let csv = sample_trades_csv();
    let frame = fp_io::read_csv_str(csv).unwrap();

    // SQL round-trip.
    let conn = rusqlite::Connection::open_in_memory().expect("sqlite open failed");
    fp_io::write_sql(&frame, &conn, "trades", fp_io::SqlIfExists::Fail).expect("SQL write failed");
    let sql_back = fp_io::read_sql_table(&conn, "trades").expect("SQL re-read failed");
    assert_eq!(sql_back.index().len(), frame.index().len());
    assert_eq!(sql_back.column_names().len(), frame.column_names().len());
}

// ── Full pipeline ────────────────────────────────────────────────────

#[test]
fn e2e_full_pipeline() {
    // 1. Read CSV.
    let trades = fp_io::read_csv_str(sample_trades_csv()).unwrap();
    assert_eq!(trades.index().len(), 8);

    // 2. Filter: only rows where price > 141.
    let filtered = trades.query("price > 141").unwrap();
    assert!(!filtered.is_empty());

    // 3. GroupBy ticker, sum volume.
    let grouped = filtered
        .groupby(&["ticker"])
        .unwrap()
        .sum()
        .unwrap();
    assert!(grouped.index().len() <= 2);

    // 4. Sort by volume descending.
    let sorted = grouped.sort_values("volume", false).unwrap();
    assert_eq!(sorted.index().len(), grouped.index().len());

    // 5. Export to Feather.
    let feather = fp_io::write_feather_bytes(&sorted).unwrap();
    assert!(!feather.is_empty());

    // 6. Read back from Feather and verify.
    let recovered = fp_io::read_feather_bytes(&feather).unwrap();
    assert_eq!(recovered.index().len(), sorted.index().len());

    // 7. Export to SQL.
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    fp_io::write_sql(&recovered, &conn, "results", fp_io::SqlIfExists::Fail).unwrap();
    let sql_result = fp_io::read_sql_table(&conn, "results").unwrap();
    assert_eq!(sql_result.index().len(), recovered.index().len());
}
