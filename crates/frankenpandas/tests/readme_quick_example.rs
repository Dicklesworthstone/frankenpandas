//! Integration test: README Quick Example + Quick Start
//!
//! Locks in that the documented prelude is sufficient to compile and run the
//! examples shown in the top-level README. If the README evolves, this test
//! must evolve too — or the README is lying.
//!
//! Tracks fd90.146 (br-frankenpandas-we6ql). Regression guard for fd90.121
//! through fd90.144 prelude expansion.

use frankenpandas::prelude::*;

/// README Quick Example (lines 41-63).
///
/// Imports prelude only. Verifies:
/// - read_csv_str
/// - DataFrame.query (DataFrameExprExt trait method)
/// - DataFrame.groupby + DataFrameGroupBy.agg_named
/// - write_json_string + JsonOrient
/// - write_feather_bytes
#[test]
fn readme_quick_example_compiles_and_runs() -> Result<(), Box<dyn std::error::Error>> {
    let df = read_csv_str("name,age,city\nAlice,30,NYC\nBob,25,LA\nCarol,35,NYC")?;

    let adults = df.query("age > 28")?;

    let summary = adults.groupby(&["city"])?.agg_named(&[
        ("avg_age", "age", "mean"),
        ("count", "age", "count"),
    ])?;

    let _json = write_json_string(&summary, JsonOrient::Records)?;
    let _feather = write_feather_bytes(&summary)?;

    // Sanity-check: only NYC group survives the query filter
    // (Alice 30 + Carol 35; LA's Bob is filtered out at age > 28).
    assert_eq!(summary.index().len(), 1);
    Ok(())
}

/// README Quick Start (lines 209-234).
///
/// Imports prelude only. Verifies broader API surface including:
/// - Series::from_values + IndexLabel/Scalar via prelude
/// - to_datetime
/// - write_csv_string
/// - frankenpandas::rusqlite::Connection (sql-sqlite feature, on by default)
/// - write_sql + SqlIfExists
/// - read_sql_table
#[cfg(feature = "sql-sqlite")]
#[test]
fn readme_quick_start_round_trip_through_sqlite() -> Result<(), Box<dyn std::error::Error>> {
    let df = read_csv_str(
        "ticker,price,volume\nAAPL,185.50,1000\nGOOG,140.25,500\nAAPL,186.00,1200",
    )?;

    let expensive = df.query("price > 150")?;
    let by_ticker = expensive.groupby(&["ticker"])?.sum()?;

    // Series construction via prelude — exercises Series + IndexLabel + Scalar.
    let dates = Series::from_values(
        "d",
        vec![0i64.into()],
        vec![Scalar::Utf8("2024-01-15".into())],
    )?;
    let _parsed = to_datetime(&dates)?;

    // Format exports.
    let _csv = write_csv_string(&by_ticker)?;
    let _json = write_json_string(&by_ticker, JsonOrient::Records)?;
    let _feather = write_feather_bytes(&by_ticker)?;

    // SQL round-trip via the re-exported rusqlite (fd90.130).
    let conn = frankenpandas::rusqlite::Connection::open_in_memory()?;
    write_sql(&by_ticker, &conn, "results", SqlIfExists::Fail)?;
    let back = read_sql_table(&conn, "results")?;

    // Both AAPL trades survive price > 150 filter; GOOG (140.25) is dropped.
    // After groupby(ticker).sum(), only the AAPL group row remains.
    assert_eq!(by_ticker.index().len(), 1);
    assert_eq!(back.index().len(), 1);
    Ok(())
}

/// README Merge: Advanced Options (lines 873-902, fixed in fd90.113).
///
/// Imports prelude only. Verifies:
/// - DataFrameMergeExt trait + merge_with_options method
/// - MergeExecutionOptions struct + Default impl
/// - MergeValidateMode::OneToOne variant
/// - MergedDataFrame return type with public `index` + `columns` fields
/// - DataFrame::new(index, columns) reconstruction from MergedDataFrame
#[test]
fn readme_merge_with_options_compiles_and_runs() -> Result<(), Box<dyn std::error::Error>> {
    let df1 = read_csv_str("key,a\n1,10\n2,20\n3,30")?;
    let df2 = read_csv_str("key,b\n1,100\n2,200\n3,300")?;

    let merged = df1.merge_with_options(
        &df2,
        &["key"],
        &["key"],
        JoinType::Inner,
        MergeExecutionOptions {
            validate_mode: Some(MergeValidateMode::OneToOne),
            ..Default::default()
        },
    )?;

    // Reconstruct a usable DataFrame from MergedDataFrame's public fields.
    let result = DataFrame::new(merged.index, merged.columns)?;

    // Inner join on key — all 3 rows match.
    assert_eq!(result.index().len(), 3);
    Ok(())
}

/// README Expression-Driven Analysis (lines 1403-1417).
///
/// Imports prelude only (plus std::collections::BTreeMap from std). Verifies:
/// - df.eval(expr) — DataFrameExprExt trait method returning Series
/// - df.query(expr) — compound boolean expressions
/// - df.query_with_locals(expr, &locals) — @local variable references
/// - BTreeMap<String, Scalar> locals binding contract
#[test]
fn readme_expression_driven_analysis_compiles_and_runs() -> Result<(), Box<dyn std::error::Error>> {
    use std::collections::BTreeMap;

    let df = read_csv_str("revenue,cost,price,rating,value\n200,150,40,4.5,150\n100,80,60,3.5,80\n300,250,30,4.7,200")?;

    // Compute new columns with eval — returns Series.
    let profit = df.eval("revenue - cost")?;
    assert_eq!(profit.len(), 3);

    // Filter with compound conditions — returns DataFrame.
    let hot_deals = df.query("price < 50 and rating > 4.0")?;
    // price<50 ∧ rating>4.0 → row 0 (40, 4.5) and row 2 (30, 4.7) match. Row 1 (60, 3.5) drops.
    assert_eq!(hot_deals.index().len(), 2);

    // Use local variables in expressions.
    let locals = BTreeMap::from([
        ("threshold".to_owned(), Scalar::Float64(100.0)),
    ]);
    let above = df.query_with_locals("value > @threshold", &locals)?;
    // value>100 → row 0 (150) + row 2 (200). Row 1 (80) drops.
    assert_eq!(above.index().len(), 2);
    Ok(())
}

/// README MultiIndex Operations (lines 1383-1395).
///
/// Imports prelude only. Verifies the standalone MultiIndex API chain:
/// - MultiIndex::from_product(Vec<Vec<IndexLabel>>) via .into() blanket
/// - .set_names(Vec<Option<String>>) consumes-self chain
/// - .get_level_values(usize) returning Result<Index>
/// - .to_flat_index(&str) returning a flat Index with composite labels
#[test]
fn readme_multiindex_operations_compiles_and_runs() -> Result<(), Box<dyn std::error::Error>> {
    // Cartesian product: 2 regions × 2 years = 4 entries.
    let mi = MultiIndex::from_product(vec![
        vec!["east".into(), "west".into()],
        vec![2023i64.into(), 2024i64.into()],
    ])?
    .set_names(vec![Some("region".into()), Some("year".into())]);

    assert_eq!(mi.nlevels(), 2);
    assert_eq!(mi.len(), 4);

    // Extract level 0 (the regions).
    let regions = mi.get_level_values(0)?;
    assert_eq!(regions.len(), 4);

    // Flatten with separator → single Index.
    let flat = mi.to_flat_index("_");
    assert_eq!(flat.len(), 4);
    Ok(())
}

/// README Categorical Analysis (lines 1350-1379).
///
/// Imports prelude only. Verifies the categorical chain:
/// - Series::from_categorical(name, Vec<Scalar>, ordered: bool)
/// - .cat() returning Option<CategoricalAccessor>
/// - cat.categories() / cat.codes()?.values() introspection
/// - cat.rename_categories(Vec<Scalar>) returning Result<Series>
/// - renamed.cat().unwrap().to_values()? — round-trip back to value series
#[test]
fn readme_categorical_analysis_compiles_and_runs() -> Result<(), Box<dyn std::error::Error>> {
    // Create categorical with explicit ordering.
    let ratings = Series::from_categorical(
        "satisfaction",
        vec![
            Scalar::Utf8("good".into()),
            Scalar::Utf8("poor".into()),
            Scalar::Utf8("excellent".into()),
            Scalar::Utf8("good".into()),
        ],
        true, // ordered
    )?;

    // Access category operations.
    let cat = ratings.cat().expect("ratings is categorical");
    let categories = cat.categories();
    // First-seen order: good (idx 0), poor (idx 1), excellent (idx 2).
    assert_eq!(categories.len(), 3);
    assert_eq!(categories[0], Scalar::Utf8("good".into()));
    assert_eq!(categories[1], Scalar::Utf8("poor".into()));
    assert_eq!(categories[2], Scalar::Utf8("excellent".into()));

    // Codes: [0, 1, 2, 0] — last value is "good" again so reuses code 0.
    let codes = cat.codes()?;
    let code_values = codes.values();
    assert_eq!(code_values.len(), 4);
    assert_eq!(code_values[0], Scalar::Int64(0));
    assert_eq!(code_values[1], Scalar::Int64(1));
    assert_eq!(code_values[2], Scalar::Int64(2));
    assert_eq!(code_values[3], Scalar::Int64(0));

    // Rename categories — codes stay the same but labels change.
    let renamed = cat.rename_categories(vec![
        Scalar::Utf8("Good".into()),
        Scalar::Utf8("Poor".into()),
        Scalar::Utf8("Excellent".into()),
    ])?;

    // Materialize back to a flat Series of label strings.
    let values = renamed.cat().expect("renamed is still categorical").to_values()?;
    assert_eq!(values.len(), 4);
    Ok(())
}

/// README Financial Data Pipeline (lines 1305-1333).
///
/// Imports prelude only (plus std::env / std::path). Verifies the
/// multi-stage analysis chain from the recipe:
/// - read_csv_str with multi-line input + line continuation
/// - Series::new(name, Index, Column) constructor with cloned column
/// - df.column(name).unwrap().clone() to extract a Column
/// - to_datetime on a Utf8 series of ISO dates
/// - df.groupby(&[col])?.agg_named(&[(out, src, fn), ...])? multi-aggregation
/// - write_jsonl to a path-based output (uses a tempdir so the test cleans up)
#[test]
fn readme_financial_data_pipeline_compiles_and_runs() -> Result<(), Box<dyn std::error::Error>> {
    use std::env;
    use std::fs;

    let trades = read_csv_str(
        "date,ticker,price,volume\n\
         2024-01-15,AAPL,185.50,1000\n\
         2024-01-15,GOOG,140.25,500\n\
         2024-01-16,AAPL,186.00,1200\n\
         2024-01-16,GOOG,141.00,800",
    )?;
    assert_eq!(trades.index().len(), 4);

    // Parse dates.
    let date_series = Series::new(
        "date",
        trades.index().clone(),
        trades.column("date").expect("date column exists").clone(),
    )?;
    let parsed_dates = to_datetime(&date_series)?;
    assert_eq!(parsed_dates.len(), 4);

    // Daily VWAP per ticker — multi-aggregation.
    let vwap = trades.groupby(&["ticker"])?.agg_named(&[
        ("total_value", "price", "sum"),
        ("total_vol", "volume", "sum"),
        ("trade_count", "volume", "count"),
    ])?;
    // 2 unique tickers (AAPL, GOOG).
    assert_eq!(vwap.index().len(), 2);
    // Three named output columns plus the ticker key.
    assert!(vwap.column_names().iter().any(|n| n.as_str() == "total_value"));
    assert!(vwap.column_names().iter().any(|n| n.as_str() == "total_vol"));
    assert!(vwap.column_names().iter().any(|n| n.as_str() == "trade_count"));

    // Export for downstream consumption — use a tempdir so the test cleans up.
    let mut out_path = env::temp_dir();
    out_path.push(format!(
        "frankenpandas_fd90_160_{}.jsonl",
        std::process::id()
    ));
    write_jsonl(&vwap, &out_path)?;
    let written = fs::read_to_string(&out_path)?;
    // Two ticker groups → two JSONL lines (line-delimited JSON; ticker is
    // the index after groupby, so the JSON body contains the agg columns
    // not the ticker label itself — verify by counting newlines).
    let line_count = written.lines().count();
    assert_eq!(line_count, 2, "expected one JSONL line per ticker group");
    assert!(written.contains("total_value"));
    assert!(written.contains("trade_count"));
    fs::remove_file(&out_path).ok();
    Ok(())
}
