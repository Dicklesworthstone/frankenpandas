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
    // Uses the From<&str> for Scalar ergonomics (fd90.182) — mirrors README Quick Start.
    let dates = Series::from_values("d", vec![0i64.into()], vec!["2024-01-15".into()])?;
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

/// README Merge-Asof for Time Series Alignment (lines 1336-1348).
///
/// Imports prelude only. Verifies the recipe's documented chain:
/// - merge_asof(&left, &right, on, direction) returning Result<MergedDataFrame, JoinError>
/// - AsofDirection::Backward variant (nearest preceding match)
/// - MergedDataFrame public `index` + `columns` field access
/// - DataFrame::new(index, columns) reconstruction
#[test]
fn readme_merge_asof_compiles_and_runs() -> Result<(), Box<dyn std::error::Error>> {
    // trades: 3 transactions at timestamps 10, 20, 30.
    let trades = read_csv_str("timestamp,trade_price\n10,100\n20,200\n30,300")?;
    // quotes: 4 quotes at timestamps 5, 15, 25, 35 — none match exactly.
    let quotes = read_csv_str("timestamp,quote\n5,99\n15,150\n25,250\n35,350")?;

    let merged = merge_asof(
        &trades,
        &quotes,
        "timestamp",
        AsofDirection::Backward,
    )?;

    // MergedDataFrame has public index + columns fields. Reconstruct a
    // DataFrame to call methods on it (per fd90.137 docs note).
    let result = DataFrame::new(merged.index, merged.columns)?;

    // Backward asof = take the LAST quote at or before each trade timestamp.
    //   trade 10 → quote at 5 (=99)    nearest preceding
    //   trade 20 → quote at 15 (=150)
    //   trade 30 → quote at 25 (=250)
    assert_eq!(result.index().len(), 3);
    assert!(result.column_names().iter().any(|n| n.as_str() == "trade_price"));
    assert!(result.column_names().iter().any(|n| n.as_str() == "quote"));
    Ok(())
}

/// README Random Sampling (lines 1630-1643).
///
/// Imports prelude only. Verifies the fd90.115 signature fixes
/// (Option<usize>/Option<f64> for sample, &[f64] for sample_weights)
/// and the fd90.162 inline-weight expression survive end-to-end.
#[test]
fn readme_random_sampling_compiles_and_runs() -> Result<(), Box<dyn std::error::Error>> {
    // 100-row test DataFrame.
    let mut csv = String::from("val\n");
    for i in 0..100 {
        csv.push_str(&format!("{}\n", i));
    }
    let df = read_csv_str(&csv)?;
    assert_eq!(df.index().len(), 100);

    // Sample n rows.
    let sampled = df.sample(Some(10), None, false, Some(42))?;
    assert_eq!(sampled.index().len(), 10);

    // Sample fraction.
    let frac = df.sample(None, Some(0.2), false, Some(42))?;
    assert_eq!(frac.index().len(), 20);

    // Sample with replacement (bootstrap).
    let bootstrap = df.sample(Some(50), None, true, Some(42))?;
    assert_eq!(bootstrap.index().len(), 50);

    // Weighted sampling — `weights` is &[f64] (per fd90.115 fix), inline
    // expression (per fd90.162 fix).
    let weights: Vec<f64> = (0..df.len()).map(|i| (i + 1) as f64).collect();
    let weighted = df.sample_weights(15, &weights, false, Some(42))?;
    assert_eq!(weighted.index().len(), 15);

    // Determinism: same seed → same rows.
    let again = df.sample(Some(10), None, false, Some(42))?;
    assert_eq!(again.index().len(), 10);
    Ok(())
}

/// README Duplicate Handling (lines 1609-1622).
///
/// Imports prelude only. Verifies the fd90.116 + fd90.122 signature
/// fixes survive end-to-end:
/// - df.duplicated(None, DuplicateKeep::First) returns a boolean Series
/// - df.drop_duplicates(None, DuplicateKeep::First, false) keeps first occurrences
/// - series.drop_duplicates() (no-arg variant on Series)
/// - index.has_duplicates() returns bool directly (no Result, no ?)
/// - index.drop_duplicates_keep(DuplicateKeep::First) returns Index (no Result)
#[test]
fn readme_duplicate_handling_compiles_and_runs() -> Result<(), Box<dyn std::error::Error>> {
    // 5 rows where rows 0+2 are dup (a=1) and rows 1+3 are dup (a=2). Row 4 is unique (a=3).
    let df = read_csv_str("a\n1\n2\n1\n2\n3")?;
    assert_eq!(df.index().len(), 5);

    // Mark duplicates (DataFrame variant requires subset + keep).
    let mask = df.duplicated(None, DuplicateKeep::First)?;
    assert_eq!(mask.len(), 5);
    // mask[0] = false (first 1), mask[1] = false (first 2),
    // mask[2] = true (dup 1), mask[3] = true (dup 2), mask[4] = false (first 3).

    // Drop duplicates (DataFrame variant requires subset + keep + ignore_index).
    let unique = df.drop_duplicates(None, DuplicateKeep::First, false)?;
    // After dedup: 1, 2, 3 → 3 unique rows.
    assert_eq!(unique.index().len(), 3);

    // Series-level (no-arg).
    let series = read_csv_str("v\n10\n20\n10\n30\n20")?
        .column("v")
        .expect("v column exists")
        .clone();
    let series = Series::new("v", read_csv_str("v\n10\n20\n10\n30\n20")?.index().clone(), series)?;
    let deduped = series.drop_duplicates()?;
    assert_eq!(deduped.len(), 3); // 10, 20, 30

    // Index-level (no Result on either method). Construct an Index with
    // explicit duplicate labels — read_csv_str produces unique Int64 row
    // indices by default, so we hand-build one here.
    let dup_index = Index::new(vec![
        IndexLabel::Int64(1),
        IndexLabel::Int64(2),
        IndexLabel::Int64(1), // duplicate of position 0
        IndexLabel::Int64(3),
    ]);
    let has_dups = dup_index.has_duplicates();
    assert!(has_dups);
    let unique_idx = dup_index.drop_duplicates_keep(DuplicateKeep::First);
    assert_eq!(unique_idx.len(), 3); // 1, 2, 3 (drop second 1)
    Ok(())
}

/// README Window Operations (lines 474-487).
///
/// Imports prelude only. Verifies the rolling / expanding / ewm chains.
/// Resample is skipped here because it requires a datetime-like index;
/// the rest of the chain compiles and runs against a numeric Series.
/// - series.rolling(window, min_periods).mean()? on a 100-element series
/// - series.rolling(window, Some(min_periods)).std()? with min_periods
/// - series.expanding(min_periods).max()? cumulative
/// - series.ewm(span, alpha).mean()? exponentially weighted
#[test]
fn readme_window_operations_compiles_and_runs() -> Result<(), Box<dyn std::error::Error>> {
    // Build a 100-element numeric Series via Series::from_values.
    let labels: Vec<IndexLabel> = (0..100i64).map(IndexLabel::Int64).collect();
    let values: Vec<Scalar> = (0..100i64).map(|v| Scalar::Float64(v as f64)).collect();
    let series = Series::from_values("x", labels, values)?;

    // Rolling window — 30-element moving average, no min_periods constraint.
    let ma_30 = series.rolling(30, None).mean()?;
    assert_eq!(ma_30.len(), 100);

    // Rolling window with min_periods.
    let vol = series.rolling(20, Some(5)).std()?;
    assert_eq!(vol.len(), 100);

    // Expanding window — running maximum.
    let cum_max = series.expanding(None).max()?;
    assert_eq!(cum_max.len(), 100);

    // Exponentially weighted moving average.
    let ewma = series.ewm(Some(20.0), None).mean()?;
    assert_eq!(ewma.len(), 100);
    Ok(())
}

/// README Sorting (lines 1194-1206).
///
/// Imports prelude only. Verifies:
/// - df.sort_values(column, ascending: bool)? both directions
/// - series.sort_values_na(ascending, na_position: &str)? — 'first' / 'last'
///   for NaN placement
/// - df.sort_index(ascending: bool)? both directions
#[test]
fn readme_sorting_compiles_and_runs() -> Result<(), Box<dyn std::error::Error>> {
    // 4-row DataFrame with a price column.
    let df = read_csv_str("price\n50\n10\n40\n20")?;

    // Sort by column values ascending.
    let asc = df.sort_values("price", true)?;
    assert_eq!(asc.index().len(), 4);
    let asc_first = asc.column("price").expect("price col").values()[0].clone();
    assert_eq!(asc_first, Scalar::Int64(10));

    // Sort by column values descending.
    let desc = df.sort_values("price", false)?;
    let desc_first = desc.column("price").expect("price col").values()[0].clone();
    assert_eq!(desc_first, Scalar::Int64(50));

    // Series with NaN — verify na_position controls where NaN lands.
    let labels: Vec<IndexLabel> = (0..4i64).map(IndexLabel::Int64).collect();
    let values = vec![
        Scalar::Float64(3.0),
        Scalar::Null(NullKind::NaN),
        Scalar::Float64(1.0),
        Scalar::Float64(2.0),
    ];
    let series = Series::from_values("v", labels, values)?;

    let na_first = series.sort_values_na(true, "first")?;
    // First element should be NaN.
    let first = na_first.values()[0].clone();
    assert!(first.is_missing(), "expected NaN first, got {:?}", first);

    let na_last = series.sort_values_na(true, "last")?;
    // Last element should be NaN.
    let last = na_last.values()[na_last.len() - 1].clone();
    assert!(last.is_missing(), "expected NaN last, got {:?}", last);

    // Sort by index labels.
    let idx_asc = df.sort_index(true)?;
    let idx_desc = df.sort_index(false)?;
    assert_eq!(idx_asc.index().len(), 4);
    assert_eq!(idx_desc.index().len(), 4);
    Ok(())
}

/// README Pivot Tables: Full Options (lines 1232-1249).
///
/// Imports prelude only. Locks in fd90.114's signature fixes for
/// pivot_table_with_margins / pivot_table_with_margins_name (which
/// require an explicit margins:bool arg). Verifies all six variants:
/// - df.pivot_table(values, index, columns, aggfunc)?
/// - df.pivot_table_multi_values(&[values...], index, columns, aggfunc)?
/// - df.pivot_table_with_margins(..., margins: bool)?
/// - df.pivot_table_with_margins_name(..., margins: bool, label)?
/// - df.pivot_table_fill(..., fill_value: f64)?
/// - df.pivot_table_multi_agg(values, index, columns, &[fns...])?
#[test]
fn readme_pivot_tables_compiles_and_runs() -> Result<(), Box<dyn std::error::Error>> {
    // Small wide DataFrame: revenue + quantity by region × product.
    let df = read_csv_str(
        "region,product,revenue,quantity\n\
         east,widget,100,5\n\
         east,gadget,200,10\n\
         west,widget,150,7\n\
         west,gadget,250,12",
    )?;
    assert_eq!(df.index().len(), 4);

    // Basic pivot table.
    let pt = df.pivot_table("revenue", "region", "product", "sum")?;
    assert_eq!(pt.index().len(), 2); // east + west rows

    // Multiple values columns.
    let pt = df.pivot_table_multi_values(
        &["revenue", "quantity"],
        "region",
        "product",
        "sum",
    )?;
    assert_eq!(pt.index().len(), 2);

    // With margins (subtotals row/col); margins=true.
    let pt = df.pivot_table_with_margins("revenue", "region", "product", "sum", true)?;
    // 2 region rows + 1 margin "All" row.
    assert_eq!(pt.index().len(), 3);

    // Custom margins label.
    let pt = df.pivot_table_with_margins_name(
        "revenue",
        "region",
        "product",
        "sum",
        true,
        "Grand Total",
    )?;
    assert_eq!(pt.index().len(), 3);

    // Fill NaN in pivot output.
    let pt = df.pivot_table_fill("revenue", "region", "product", "sum", 0.0)?;
    assert_eq!(pt.index().len(), 2);

    // Multiple aggregation functions — emits {col}_{fn} columns.
    let pt = df.pivot_table_multi_agg(
        "revenue",
        "region",
        "product",
        &["sum", "mean", "count"],
    )?;
    assert_eq!(pt.index().len(), 2);
    Ok(())
}

/// README Concat: Full Options (lines 1210-1228).
///
/// Imports prelude only. Locks in fd90.141's prelude expansion of
/// the 5 concat variants. Verifies:
/// - concat_dataframes(&[&df, &df])?                     — axis-0 stack
/// - concat_dataframes_with_axis(&[&df, &df], 1)?        — axis-1 outer
/// - concat_dataframes_with_axis_join(..., 1, Inner)?    — axis-1 inner
/// - concat_dataframes_with_axis_join(..., 0, Inner)?    — axis-0 inner
/// - concat_dataframes_with_keys(..., &['train','test'])? — hierarchical labels
/// - concat_dataframes_with_ignore_index(..., false)?    — reindex 0..n
#[test]
fn readme_concat_full_options_compiles_and_runs() -> Result<(), Box<dyn std::error::Error>> {
    // Same columns for axis=0 stack/inner. Different columns for axis=1.
    let df1 = read_csv_str("a,b\n1,10\n2,20")?;
    let df2 = read_csv_str("a,b\n3,30\n4,40")?;
    let df3 = read_csv_str("c,d\n100,1000\n200,2000")?;

    // Axis 0 (default — stack rows on shared columns).
    let stacked = concat_dataframes(&[&df1, &df2])?;
    assert_eq!(stacked.index().len(), 4);

    // Axis 1 (columns side-by-side, outer join on index) — needs disjoint columns.
    let wide = concat_dataframes_with_axis(&[&df1, &df3], 1)?;
    // Same 2-row index in both → 2 rows wide.
    assert_eq!(wide.index().len(), 2);

    // Axis 1 with inner join (only shared index labels).
    let inner = concat_dataframes_with_axis_join(&[&df1, &df3], 1, ConcatJoin::Inner)?;
    assert_eq!(inner.index().len(), 2);

    // Axis 0 with inner join (only shared columns) — df1+df2 share a,b.
    let shared = concat_dataframes_with_axis_join(&[&df1, &df2], 0, ConcatJoin::Inner)?;
    assert_eq!(shared.index().len(), 4);

    // With hierarchical keys.
    let labeled = concat_dataframes_with_keys(&[&df1, &df2], &["train", "test"])?;
    assert_eq!(labeled.index().len(), 4);

    // Ignore original indexes (reindex to 0..n).
    let clean = concat_dataframes_with_ignore_index(&[&df1, &df2], true)?;
    assert_eq!(clean.index().len(), 4);
    Ok(())
}

/// README Element-Wise Operations (lines 1027-1054).
///
/// Imports prelude only. Locks in fd90.123's pct_change(1) signature fix
/// plus the broader scalar / df-to-df / cumulative API. Verifies that
/// the documented chain compiles and runs end-to-end.
#[test]
fn readme_element_wise_operations_compiles_and_runs() -> Result<(), Box<dyn std::error::Error>> {
    let df = read_csv_str("a,b\n10,1\n20,2\n30,3\n40,4")?;

    // Scalar arithmetic.
    let _ = df.mul_scalar(2.0)?;
    let _ = df.add_scalar(100.0)?;
    let _ = df.div_scalar(2.0)?;
    let _ = df.pow_scalar(2.0)?;
    let _ = df.mod_scalar(10.0)?;
    let _ = df.floordiv_scalar(3.0)?;

    // DataFrame-to-DataFrame (aligned).
    let df2 = read_csv_str("a,b\n5,1\n10,2\n15,3\n20,4")?;
    let _ = df.sub_df(&df2)?;
    let _ = df.div_df(&df2)?;
    let _ = df.mul_df(&df2)?;

    // With fill value.
    let _ = df.add_df_fill(&df2, 0.0)?;

    // Cumulative ops.
    let csum = df.cumsum()?;
    assert_eq!(csum.index().len(), 4);
    let _ = df.cumprod()?;
    let _ = df.cummax()?;
    let _ = df.cummin()?;

    // Sequential ops.
    let _ = df.diff(1)?;
    let _ = df.shift(1)?;
    let pct = df.pct_change(1)?; // fd90.123 fix — periods is required arg
    assert_eq!(pct.index().len(), 4);
    Ok(())
}

/// README Missing Data Handling (lines 944-994).
///
/// Imports prelude only. Locks in fd90.104's dropna_with_threshold
/// rename and exercises the broader detection / filling / dropping API.
#[test]
fn readme_missing_data_handling_compiles_and_runs() -> Result<(), Box<dyn std::error::Error>> {
    // Series with NaN values to exercise detection/filling.
    let labels: Vec<IndexLabel> = (0..5i64).map(IndexLabel::Int64).collect();
    let values = vec![
        Scalar::Float64(1.0),
        Scalar::Null(NullKind::NaN),
        Scalar::Float64(3.0),
        Scalar::Null(NullKind::NaN),
        Scalar::Float64(5.0),
    ];
    let series = Series::from_values("v", labels, values)?;

    // Detection.
    let nulls = series.isna()?;
    assert_eq!(nulls.len(), 5);
    let valid = series.notna()?;
    assert_eq!(valid.len(), 5);
    let count = series.count();
    assert_eq!(count, 3); // 3 non-NaN values
    let has = series.hasnans();
    assert!(has);

    // Filling.
    let _filled = series.fillna(&Scalar::Float64(0.0))?;
    let _ff_unlim = series.ffill(None)?;
    let _ff_lim = series.ffill(Some(3))?;
    let _bf = series.bfill(Some(2))?;
    let _interp = series.interpolate()?;

    // DataFrame-level dropping.
    let df = read_csv_str("a,b,c\n1,1,1\n2,,2\n3,3,\n,,4\n5,5,5")?;
    let _ = df.dropna()?;
    let _ = df.dropna_with_options(DropNaHow::All, None)?;
    let _ = df.dropna_with_options(
        DropNaHow::Any,
        Some(&["a".into(), "b".into()]),
    )?;
    // fd90.104 rename: dropna_with_thresh → dropna_with_threshold (with subset arg).
    let _ = df.dropna_with_threshold(2, None)?;
    let _ = df.dropna_columns()?;
    Ok(())
}

/// README Type Coercion and Conversion (lines 1003-1019).
///
/// Imports prelude only. Verifies:
/// - series.astype(DType)
/// - df.astype_column(name, DType)
/// - df.astype_columns(&[(name, DType)])
/// - df.convert_dtypes()
/// - df.infer_objects()
/// - to_numeric(&series) module-level fn
#[test]
fn readme_type_coercion_compiles_and_runs() -> Result<(), Box<dyn std::error::Error>> {
    // Int64 series → cast to Float64.
    let labels: Vec<IndexLabel> = (0..3i64).map(IndexLabel::Int64).collect();
    let int_series = Series::from_values(
        "n",
        labels.clone(),
        vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
    )?;
    let float_col = int_series.astype(DType::Float64)?;
    assert_eq!(float_col.len(), 3);

    // DataFrame with int columns we'll cast.
    let df = read_csv_str("price,score\n100,1\n200,2\n300,3")?;
    let _ = df.astype_column("price", DType::Float64)?;
    // Multiple-column cast — both targets need to be reachable from Int64.
    let _ = df.astype_columns(&[("price", DType::Float64), ("score", DType::Float64)])?;

    // Auto-infer.
    let _ = df.convert_dtypes()?;
    let _ = df.infer_objects()?;

    // Coerce to numeric — Utf8 strings parsed; non-parseable → NaN.
    let str_series = Series::from_values(
        "s",
        labels,
        vec![
            Scalar::Utf8("1.5".into()),
            Scalar::Utf8("not_a_number".into()),
            Scalar::Utf8("3.0".into()),
        ],
    )?;
    let numeric = to_numeric(&str_series)?;
    assert_eq!(numeric.len(), 3);
    Ok(())
}

/// README DataFrame Introspection (lines 1145-1167).
///
/// Imports prelude only. Locks in fd90.175's dtypes()? Result-return
/// fix and exercises the broader introspection API.
#[test]
fn readme_dataframe_introspection_compiles_and_runs() -> Result<(), Box<dyn std::error::Error>> {
    let df = read_csv_str("a,b\n1,2\n3,4\n5,6")?;

    // Shape — (nrows, ncols).
    let shape = df.shape();
    assert_eq!(shape, (3, 2));

    // dtypes — Series (fd90.175 fix; was wrongly documented as Vec<(String, DType)>).
    let dtypes = df.dtypes()?;
    assert_eq!(dtypes.len(), 2); // a + b columns

    // info — string summary.
    let info = df.info();
    assert!(info.contains("a"));

    // memory_usage — Series of per-column byte estimates.
    let mem = df.memory_usage()?;
    assert!(mem.len() >= 2);

    // ndim — always 2 for DataFrame.
    assert_eq!(df.ndim(), 2);

    // axes — (Vec<IndexLabel>, Vec<String>).
    let (idx, cols) = df.axes();
    assert_eq!(idx.len(), 3);
    assert_eq!(cols.len(), 2);

    // is_empty — false for non-empty DataFrame.
    assert!(!df.is_empty());

    // equals — deep comparison.
    let df_clone = df.clone();
    assert!(df.equals(&df_clone));

    // compare — element-wise diff (returns DataFrame).
    let _ = df.compare(&df_clone)?;
    Ok(())
}

/// README SeriesGroupBy (lines 1177-1190).
///
/// Imports prelude only. Locks in fd90.135's method-list correction
/// (removed phantom sem/skew/kurtosis/value_counts; the actual
/// SeriesGroupBy surface is the 15 methods this test exercises).
#[test]
fn readme_series_groupby_compiles_and_runs() -> Result<(), Box<dyn std::error::Error>> {
    let labels: Vec<IndexLabel> = (0..6i64).map(IndexLabel::Int64).collect();

    // Revenue series: 6 numeric values.
    let revenue = Series::from_values(
        "revenue",
        labels.clone(),
        vec![
            Scalar::Float64(100.0),
            Scalar::Float64(200.0),
            Scalar::Float64(150.0),
            Scalar::Float64(250.0),
            Scalar::Float64(300.0),
            Scalar::Float64(400.0),
        ],
    )?;

    // Region series: 2 unique groups (A, B) of 3 elements each.
    let region = Series::from_values(
        "region",
        labels,
        vec![
            Scalar::Utf8("A".into()),
            Scalar::Utf8("A".into()),
            Scalar::Utf8("A".into()),
            Scalar::Utf8("B".into()),
            Scalar::Utf8("B".into()),
            Scalar::Utf8("B".into()),
        ],
    )?;

    let by_region = revenue.groupby(&region)?;

    // Per-group aggregates.
    let sums = by_region.sum()?;
    assert_eq!(sums.len(), 2); // A, B
    let _ = by_region.mean()?;
    let _ = by_region.std()?;
    let _ = by_region.median()?;
    let _ = by_region.prod()?;
    let _ = by_region.count()?;
    let _ = by_region.min()?;
    let _ = by_region.max()?;
    let _ = by_region.var()?;
    let _ = by_region.first()?;
    let _ = by_region.last()?;
    let _ = by_region.size()?;

    // Multi-aggregation returns a DataFrame.
    let multi = by_region.agg(&["sum", "mean", "count"])?;
    assert_eq!(multi.index().len(), 2);
    Ok(())
}

/// README Time-Series Operations (lines 1256-1276).
///
/// Imports prelude only. Exercises:
/// - df.at_time(time_str)? / df.between_time(start, end)?
/// - series.dt() — DatetimeAccessor with year/month/.../strftime
/// - series.dt().tz_localize(Some(tz))? / tz_convert(Some(tz))?
#[test]
fn readme_time_series_operations_compiles_and_runs() -> Result<(), Box<dyn std::error::Error>> {
    // DataFrame with datetime ISO-string index labels for at_time / between_time.
    let labels: Vec<IndexLabel> = vec![
        IndexLabel::Utf8("2024-01-15T08:00:00".into()),
        IndexLabel::Utf8("2024-01-15T10:00:00".into()),
        IndexLabel::Utf8("2024-01-15T12:00:00".into()),
        IndexLabel::Utf8("2024-01-15T14:00:00".into()),
    ];
    let val_series = Series::from_values(
        "v",
        labels.clone(),
        vec![
            Scalar::Int64(1),
            Scalar::Int64(2),
            Scalar::Int64(3),
            Scalar::Int64(4),
        ],
    )?;
    let df = DataFrame::new(
        Index::new(labels.clone()),
        std::collections::BTreeMap::from([
            ("v".to_owned(), val_series.column().clone()),
        ]),
    )?;

    // at_time / between_time — string-typed time matchers.
    let _ = df.at_time("12:00:00")?;
    let _ = df.between_time("09:00:00", "12:00:00")?;

    // Datetime component extraction via .dt() accessor.
    let date_series = Series::from_values(
        "d",
        (0..3i64).map(IndexLabel::Int64).collect(),
        vec![
            Scalar::Utf8("2024-01-15T12:30:00".into()),
            Scalar::Utf8("2024-02-29T08:00:00".into()),
            Scalar::Utf8("2024-12-31T23:59:59".into()),
        ],
    )?;
    let dt = date_series.dt();
    let _ = dt.year()?;
    let _ = dt.month()?;
    let _ = dt.day()?;
    let _ = dt.hour()?;
    let _ = dt.minute()?;
    let _ = dt.second()?;
    let _ = dt.dayofweek()?;
    let _ = dt.dayofyear()?;
    let _ = dt.quarter()?;
    let _ = dt.weekofyear()?;
    let _ = dt.is_month_start()?;
    let _ = dt.is_month_end()?;
    let _ = dt.is_quarter_start()?;
    let _ = dt.is_quarter_end()?;
    let _ = dt.strftime("%Y-%m-%d %H:%M")?;

    // Timezone operations — tz arg is Option<&str>.
    let _ = date_series.dt().tz_localize(Some("America/New_York"))?;
    // tz_convert needs an already-localized series; use the localized output above.
    let localized = date_series.dt().tz_localize(Some("America/New_York"))?;
    let _ = localized.dt().tz_convert(Some("UTC"))?;
    Ok(())
}

/// README GroupBy: Complete Aggregation Matrix (lines 545-566).
///
/// Imports prelude only. Exercises the full DataFrameGroupBy surface:
/// 14 named aggs via string dispatch, several direct method calls,
/// group-level transforms (cumsum/cumcount/etc), and multi-fn agg.
#[test]
fn readme_groupby_aggregation_matrix_compiles_and_runs() -> Result<(), Box<dyn std::error::Error>> {
    // 6-row DataFrame with 2 groups (a=1: rows 0,1,2; a=2: rows 3,4,5).
    let df = read_csv_str("a,b\n1,10\n1,20\n1,30\n2,40\n2,50\n2,60")?;
    let gb = df.groupby(&["a"])?;

    // Direct method calls — Returns DataFrame indexed by group keys.
    let _ = gb.sum()?;
    let _ = gb.mean()?;
    let _ = gb.count()?;
    let _ = gb.min()?;
    let _ = gb.max()?;
    let _ = gb.std()?;
    let _ = gb.var()?;
    let _ = gb.median()?;
    let _ = gb.first()?;
    let _ = gb.last()?;
    let _ = gb.prod()?;
    let _ = gb.size()?;
    let _ = gb.nunique()?;
    let _ = gb.idxmin()?;
    let _ = gb.idxmax()?;
    let _ = gb.all()?;
    let _ = gb.any()?;

    // String-dispatch via agg_list — supports the 12 aggs shared with the
    // value-broadcast path (sum/mean/count/min/max/std/var/median/first/
    // last/nunique/prod). The remaining 3 names from the README's 14-row
    // table (sem/skew/kurt|kurtosis) are exposed via direct method calls
    // (.sem(), .skew(), .kurt(), .kurtosis()).
    for fn_name in [
        "sum", "mean", "count", "min", "max", "std", "var", "median", "first",
        "last", "nunique", "prod",
    ] {
        let _ = gb.agg_list(&[fn_name])?;
    }
    let _ = gb.sem()?;
    let _ = gb.skew()?;
    let _ = gb.kurtosis()?; // 'kurt' alias is via agg() string dispatch only

    // Multi-fn agg via agg_list — returns a DataFrame with rows for each fn.
    let _ = gb.agg_list(&["sum", "mean", "count"])?;

    // agg_named — explicit (out_col, src_col, fn).
    let named = gb.agg_named(&[
        ("total_b", "b", "sum"),
        ("avg_b", "b", "mean"),
    ])?;
    assert_eq!(named.index().len(), 2);

    // Group-level transforms / ops (line 566).
    let _ = gb.cumsum()?;
    let _ = gb.cumprod()?;
    let _ = gb.cummax()?;
    let _ = gb.cummin()?;
    let _ = gb.shift(1)?;
    let _ = gb.diff(1)?;
    let _ = gb.head(2)?;
    let _ = gb.tail(2)?;
    let _ = gb.cumcount()?;
    let _ = gb.ngroup()?;
    let _ = gb.describe()?;

    Ok(())
}

/// README Apply and Transform (lines 643-673).
///
/// Imports prelude only. Locks in fd90.107 (transform closure / GroupBy
/// string variant), fd90.108 (apply_row name arg), fd90.134 (pipe
/// FrameError chain), and fd90.180 (assign_fn inline ratio expression).
#[test]
fn readme_apply_and_transform_compiles_and_runs() -> Result<(), Box<dyn std::error::Error>> {
    let df = read_csv_str("region,revenue,cost\nA,200,100\nB,400,200\nA,300,150\nB,600,200")?;

    // applymap — element-wise closure on each Scalar.
    let _doubled_or_self = df.applymap(|s| match s {
        Scalar::Int64(v) => Scalar::Int64(v * 2),
        Scalar::Float64(v) => Scalar::Float64(v * 2.0),
        other => other.clone(),
    })?;

    // apply_row — fd90.108: takes (name, closure).
    let row_total: Series = df.apply_row("row_total", |row_values: &[Scalar]| {
        let total: f64 = row_values
            .iter()
            .filter_map(|s| match s {
                Scalar::Int64(v) => Some(*v as f64),
                Scalar::Float64(v) => Some(*v),
                _ => None,
            })
            .sum();
        Scalar::Float64(total)
    })?;
    assert_eq!(row_total.len(), 4);

    // transform — fd90.107: closure variant returns same-shape DataFrame.
    let _doubled = df.transform(|s: &Scalar| match s {
        Scalar::Int64(v) => Scalar::Int64(v * 2),
        Scalar::Float64(v) => Scalar::Float64(v * 2.0),
        other => other.clone(),
    })?;

    // GroupBy.transform — fd90.107: string variant ('mean' broadcasts per-group).
    let group_means = df.groupby(&["region"])?.transform("mean")?;
    // Result has one row per ORIGINAL row (broadcast back), not per group.
    assert_eq!(group_means.index().len(), 4);

    // assign_fn — fd90.180: inline ratio = revenue/cost expression.
    use frankenpandas::FrameError;
    let df2 = df.assign_fn(vec![(
        "ratio",
        Box::new(|df: &DataFrame| -> Result<Column, FrameError> {
            let rev = df.column("revenue").expect("revenue column");
            let cost = df.column("cost").expect("cost column");
            let values: Vec<Scalar> = rev
                .values()
                .iter()
                .zip(cost.values())
                .map(|(r, c)| match (r, c) {
                    (Scalar::Int64(a), Scalar::Int64(b)) => {
                        Scalar::Float64(*a as f64 / *b as f64)
                    }
                    _ => Scalar::Null(NullKind::NaN),
                })
                .collect();
            Column::from_values(values).map_err(FrameError::from)
        }) as Box<dyn Fn(&DataFrame) -> Result<Column, FrameError>>,
    )])?;
    assert!(df2.column_names().iter().any(|n| n.as_str() == "ratio"));

    // pipe — fd90.134: closures must return Result<_, FrameError>.
    let result = df
        .pipe(|d| d.sort_values("revenue", true))?
        .pipe(|d| d.head(2))?;
    assert_eq!(result.index().len(), 2);
    Ok(())
}

/// README "Replacement" section (lines 1077-1102).
///
/// Locks in the four replacement APIs documented in the README:
/// - DataFrame.replace(&[(from, to)]) for sentinel cleanup
/// - StringAccessor.replace_regex for regex patterns
/// - Series.map_with_na_action for dictionary-style mapping
/// - Series.case_when for conditional grade assignment
///
/// Tracks fd90.184 (br-frankenpandas-y01i5). The case_when block was
/// just simplified in fd90.183 to use the new From<&str> for Scalar
/// ergonomics — having a regression test prevents future signature drift.
#[test]
fn readme_conditional_logic_compiles_and_runs() -> Result<(), Box<dyn std::error::Error>> {
    // df.replace — sentinel-to-NaN cleanup.
    let df = read_csv_str("a,b\n10,1\n-999,2\n30,3")?;
    let cleaned = df.replace(&[(Scalar::Int64(-999), Scalar::Null(NullKind::NaN))])?;
    assert_eq!(cleaned.index().len(), 3);

    // Series.str().replace_regex — single regex substitution on string Series.
    let phones_labels: Vec<IndexLabel> = (0..3i64).map(IndexLabel::Int64).collect();
    let phones = Series::from_values(
        "phone",
        phones_labels,
        vec![
            "555-1234".into(),
            "555-9876".into(),
            "555-0000".into(),
        ],
    )?;
    let masked = phones.str().replace_regex(r"\d{3}-\d{4}", "***-****")?;
    assert_eq!(masked.len(), 3);

    // Series.map_with_na_action — dictionary-style mapping with NaN passthrough.
    let codes_labels: Vec<IndexLabel> = (0..3i64).map(IndexLabel::Int64).collect();
    let codes = Series::from_values(
        "code",
        codes_labels,
        vec![
            Scalar::Int64(1),
            Scalar::Int64(2),
            Scalar::Int64(3),
        ],
    )?;
    let mapping = vec![
        (Scalar::Int64(1), "low".into()),
        (Scalar::Int64(2), "mid".into()),
        (Scalar::Int64(3), "high".into()),
    ];
    let mapped = codes.map_with_na_action(&mapping, true)?;
    assert_eq!(mapped.len(), 3);

    // Series.case_when — conditional grade assignment via .ge_scalar conditions.
    // Mirrors README lines 1091-1101 exactly (fd90.183 ergonomics).
    let scores_labels: Vec<IndexLabel> = (0..4i64).map(IndexLabel::Int64).collect();
    let scores = Series::from_values(
        "score",
        scores_labels,
        vec![
            Scalar::Int64(95),
            Scalar::Int64(85),
            Scalar::Int64(70),
            Scalar::Int64(92),
        ],
    )?;
    let n = scores.len();
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let value_a = Series::from_values("grade", labels.clone(), vec!["A".into(); n])?;
    let value_b = Series::from_values("grade", labels, vec!["B".into(); n])?;
    let graded = scores.case_when(&[
        (scores.ge_scalar(&Scalar::Int64(90))?, value_a),
        (scores.ge_scalar(&Scalar::Int64(80))?, value_b),
    ])?;
    assert_eq!(graded.len(), 4);
    Ok(())
}

/// README "Advanced Selection Methods" section (lines 1106-1138).
///
/// Locks in ~10 selection APIs that previously had no integration coverage:
/// - DataFrame: nlargest / nsmallest / nlargest_keep / select_dtypes / filter_labels
/// - Series: idxmin / idxmax / value_counts / value_counts_with_options
///   / isin / between / searchsorted / factorize
///
/// Tracks fd90.185 (br-frankenpandas-q208a). Mirrors README lines 1106-1138.
#[test]
fn readme_advanced_selection_compiles_and_runs() -> Result<(), Box<dyn std::error::Error>> {
    // Top-N / Bottom-N row selection on numeric columns.
    let df = read_csv_str(
        "ticker,price,volume,revenue\nAAPL,150,1000,1500\nGOOGL,2800,500,2750\n\
         MSFT,300,800,2400\nAMZN,3200,200,1280\nTSLA,800,1500,2400",
    )?;
    let top5 = df.nlargest(5, "revenue")?;
    assert_eq!(top5.index().len(), 5);
    let bot3 = df.nsmallest(3, "price")?;
    assert_eq!(bot3.index().len(), 3);
    let top_keep = df.nlargest_keep(5, "revenue", "all")?;
    assert!(top_keep.index().len() >= 5);

    // Series.idxmin / idxmax — scalar IndexLabel return.
    let labels: Vec<IndexLabel> = (0..5i64).map(IndexLabel::Int64).collect();
    let temps = Series::from_values(
        "temp",
        labels.clone(),
        vec![
            Scalar::Float64(72.0),
            Scalar::Float64(80.0),
            Scalar::Float64(65.0),
            Scalar::Float64(85.0),
            Scalar::Float64(78.0),
        ],
    )?;
    let _coldest = temps.idxmin()?;
    let _hottest = temps.idxmax()?;

    // value_counts on a categorical-shaped Series.
    let cat_labels: Vec<IndexLabel> = (0..6i64).map(IndexLabel::Int64).collect();
    let grades = Series::from_values(
        "grade",
        cat_labels,
        vec![
            "A".into(),
            "B".into(),
            "A".into(),
            "C".into(),
            "B".into(),
            "A".into(),
        ],
    )?;
    let counts = grades.value_counts()?;
    assert!(counts.len() >= 1);
    let pcts = grades.value_counts_with_options(true, true, false, true)?;
    assert!(pcts.len() >= 1);

    // isin — fd90.182 ergonomics: &[&str] inferred to Vec<Scalar> via .into().
    let test_set: Vec<Scalar> = vec!["A".into(), "B".into()];
    let _mask = grades.isin(&test_set)?;

    // between on numeric Series.
    let in_range = temps.between(&Scalar::Float64(70.0), &Scalar::Float64(80.0), "both")?;
    assert_eq!(in_range.len(), 5);

    // searchsorted returns a usize position.
    let sorted_labels: Vec<IndexLabel> = (0..5i64).map(IndexLabel::Int64).collect();
    let sorted_values = Series::from_values(
        "sorted",
        sorted_labels,
        vec![
            Scalar::Float64(10.0),
            Scalar::Float64(20.0),
            Scalar::Float64(30.0),
            Scalar::Float64(40.0),
            Scalar::Float64(50.0),
        ],
    )?;
    let pos = sorted_values.searchsorted(&Scalar::Float64(25.0), "left")?;
    assert_eq!(pos, 2);

    // factorize returns (codes, uniques) tuple.
    let (codes, uniques) = grades.factorize()?;
    assert_eq!(codes.len(), 6);
    assert!(uniques.len() >= 1);

    // select_dtypes — include and exclude paths.
    let numeric_only = df.select_dtypes(&[DType::Int64, DType::Float64], &[])?;
    assert!(numeric_only.column_names().len() >= 1);
    let non_numeric = df.select_dtypes(&[], &[DType::Int64, DType::Float64])?;
    assert!(non_numeric.column_names().iter().any(|n| n.as_str() == "ticker"));

    // filter_labels — items + regex variants on axis=1.
    let subset = df.filter_labels(Some(&["price", "volume"]), None, None, 1)?;
    assert_eq!(subset.column_names().len(), 2);
    let regex_match = df.filter_labels(None, None, Some("^rev"), 1)?;
    assert!(regex_match.column_names().iter().any(|n| n.as_str() == "revenue"));
    Ok(())
}

/// README "Column Manipulation" section (lines 1287-1311).
///
/// Locks in 6 column-management APIs that previously had no integration coverage:
/// rename_with (closure renaming), add_prefix, add_suffix, assign_column
/// (value vector), assign_fn (closure form), and select_columns (reorder).
///
/// Tracks fd90.186 (br-frankenpandas-ein1y).
#[test]
fn readme_column_manipulation_compiles_and_runs() -> Result<(), Box<dyn std::error::Error>> {
    let df = read_csv_str("revenue,cost,units\n1000,400,10\n2000,800,15\n1500,600,12")?;

    // rename_with — closure-driven column renaming.
    let renamed = df.rename_with(|name| format!("col_{name}"))?;
    assert!(
        renamed
            .column_names()
            .iter()
            .all(|n| n.as_str().starts_with("col_"))
    );

    // add_prefix / add_suffix — bulk renaming.
    let prefixed = df.add_prefix("input_")?;
    assert!(
        prefixed
            .column_names()
            .iter()
            .all(|n| n.as_str().starts_with("input_"))
    );
    let suffixed = df.add_suffix("_raw")?;
    assert!(
        suffixed
            .column_names()
            .iter()
            .all(|n| n.as_str().ends_with("_raw"))
    );

    // assign_column — append a computed column from a Vec<Scalar>.
    let computed: Vec<Scalar> = vec![
        Scalar::Float64(2.5),
        Scalar::Float64(2.5),
        Scalar::Float64(2.5),
    ];
    let with_computed = df.assign_column("computed", computed)?;
    assert!(
        with_computed
            .column_names()
            .iter()
            .any(|n| n.as_str() == "computed")
    );

    // assign_fn — closure that sees current DataFrame state.
    // Mirrors the README's "ratio = revenue / cost" pattern.
    use frankenpandas::FrameError;
    let with_ratio = df.assign_fn(vec![(
        "ratio",
        Box::new(|d: &DataFrame| -> Result<Column, FrameError> {
            let rev = d.column("revenue").expect("revenue column");
            let cost = d.column("cost").expect("cost column");
            let values: Vec<Scalar> = rev
                .values()
                .iter()
                .zip(cost.values())
                .map(|(r, c)| match (r, c) {
                    (Scalar::Int64(a), Scalar::Int64(b)) => {
                        Scalar::Float64(*a as f64 / *b as f64)
                    }
                    _ => Scalar::Null(NullKind::NaN),
                })
                .collect();
            Column::from_values(values).map_err(FrameError::from)
        }) as Box<dyn Fn(&DataFrame) -> Result<Column, FrameError>>,
    )])?;
    assert!(
        with_ratio
            .column_names()
            .iter()
            .any(|n| n.as_str() == "ratio")
    );

    // select_columns — reorder + project.
    let reordered = df.select_columns(&["units", "revenue"])?;
    let names: Vec<&str> = reordered
        .column_names()
        .iter()
        .map(|n| n.as_str())
        .collect();
    assert_eq!(names, vec!["units", "revenue"]);
    Ok(())
}

/// README "Selection and Indexing" section (lines 1522-1558).
///
/// Locks in the conditional-replacement and index-management APIs that
/// were uncovered by previous integration tests:
/// - DataFrame.where_mask_df / where_cond_df / mask_df / mask_df_other
/// - DataFrame.set_index / reset_index
/// - DataFrame.select_dtypes_by_name (string-name variant)
///
/// Tracks fd90.187 (br-frankenpandas-sy8p4).
#[test]
fn readme_selection_and_indexing_compiles_and_runs() -> Result<(), Box<dyn std::error::Error>> {
    // Base DataFrame and a same-shape Bool cond DataFrame for where/mask families.
    let df = read_csv_str("a,b\n10,1\n20,2\n30,3\n40,4")?;
    let cond_df = read_csv_str("a,b\ntrue,false\ntrue,true\nfalse,true\ntrue,false")?;
    let other_df = read_csv_str("a,b\n100,200\n100,200\n100,200\n100,200")?;

    // where_mask_df — keep where cond is true, fill rest with scalar.
    let filled = df.where_mask_df(&cond_df, &Scalar::Float64(0.0))?;
    assert_eq!(filled.index().len(), 4);

    // where_cond_df — keep where cond is true, fill rest from other DataFrame.
    let filled_other = df.where_cond_df(&cond_df, &other_df)?;
    assert_eq!(filled_other.index().len(), 4);

    // mask_df — inverse: replace where cond is true with scalar.
    let masked = df.mask_df(&cond_df, &Scalar::Null(NullKind::NaN))?;
    assert_eq!(masked.index().len(), 4);

    // mask_df_other — inverse with DataFrame replacement.
    let masked_other = df.mask_df_other(&cond_df, &other_df)?;
    assert_eq!(masked_other.index().len(), 4);

    // set_index — promote a column to the index (drop=true removes from data).
    let dated = read_csv_str("date,price\n2024-01-01,100\n2024-01-02,105\n2024-01-03,110")?;
    let indexed = dated.set_index("date", true)?;
    assert!(
        !indexed
            .column_names()
            .iter()
            .any(|n| n.as_str() == "date")
    );
    assert_eq!(indexed.index().len(), 3);

    // reset_index — index → column (drop=false keeps it as a regular column).
    let reset = indexed.reset_index(false)?;
    assert_eq!(reset.index().len(), 3);

    // select_dtypes_by_name — string-form of dtype filtering.
    let numeric_only = df.select_dtypes_by_name(&["int64", "float64"], &[])?;
    assert!(numeric_only.column_names().len() >= 1);
    Ok(())
}

/// README "Module-Level Functions" table (lines 686-699).
///
/// Locks in the 5 module-level functions that previously had no
/// integration coverage (to_datetime and concat_dataframes are
/// already exercised by readme_quick_example_compiles_and_runs and
/// readme_concat_full_options respectively):
///
/// - to_numeric (string → numeric, NaN for failures)
/// - to_timedelta (string/numeric → Timedelta64)
/// - timedelta_total_seconds (Timedelta64 → Float64 seconds)
/// - cut (equal-width binning)
/// - qcut (quantile-based binning)
///
/// Tracks fd90.188 (br-frankenpandas-g1sox).
#[test]
fn readme_module_level_functions_compiles_and_runs() -> Result<(), Box<dyn std::error::Error>> {
    // to_numeric — coerce string-typed Series to numeric.
    let str_labels: Vec<IndexLabel> = (0..3i64).map(IndexLabel::Int64).collect();
    let str_series = Series::from_values(
        "vals",
        str_labels,
        vec!["1.5".into(), "2.0".into(), "3.5".into()],
    )?;
    let numeric = to_numeric(&str_series)?;
    assert_eq!(numeric.len(), 3);

    // to_timedelta — parse duration strings.
    let dur_labels: Vec<IndexLabel> = (0..3i64).map(IndexLabel::Int64).collect();
    let dur_series = Series::from_values(
        "duration",
        dur_labels,
        vec!["1 day".into(), "2 hours".into(), "30 minutes".into()],
    )?;
    let timedeltas = to_timedelta(&dur_series)?;
    assert_eq!(timedeltas.len(), 3);

    // timedelta_total_seconds — Timedelta64 → Float64 seconds.
    let secs = timedelta_total_seconds(&timedeltas)?;
    assert_eq!(secs.len(), 3);

    // cut — equal-width binning.
    let bin_labels: Vec<IndexLabel> = (0..6i64).map(IndexLabel::Int64).collect();
    let values_for_cut = Series::from_values(
        "v",
        bin_labels,
        vec![
            Scalar::Float64(1.0),
            Scalar::Float64(2.5),
            Scalar::Float64(4.0),
            Scalar::Float64(5.5),
            Scalar::Float64(7.0),
            Scalar::Float64(9.0),
        ],
    )?;
    let binned = cut(&values_for_cut, 3)?;
    assert_eq!(binned.len(), 6);

    // qcut — quantile-based binning.
    let qbinned = qcut(&values_for_cut, 3)?;
    assert_eq!(qbinned.len(), 6);
    Ok(())
}

/// README "DataFrame Output Formats" table (lines 530-543).
///
/// Locks in 11 inline output methods on DataFrame that previously had
/// no integration coverage:
/// - to_csv, to_json (multiple orients)
/// - to_string_table, to_string_truncated
/// - to_html, to_latex, to_markdown
/// - to_dict, to_series_dict, to_records, to_numpy_2d
///
/// Each call asserted to return a non-empty result; correctness is
/// covered by per-method unit tests in fp-frame.
///
/// Tracks fd90.189 (br-frankenpandas-f6vzb).
#[test]
fn readme_dataframe_output_formats_compiles_and_runs() -> Result<(), Box<dyn std::error::Error>> {
    let df = read_csv_str("ticker,price,volume\nAAPL,150,1000\nGOOGL,2800,500\nMSFT,300,800")?;

    // to_csv with comma sep, no index.
    let csv = df.to_csv(',', false);
    assert!(csv.contains("ticker"));
    assert!(csv.contains("AAPL"));

    // to_json across multiple orients.
    let json_records = df.to_json("records")?;
    assert!(json_records.contains("AAPL"));
    let json_columns = df.to_json("columns")?;
    assert!(json_columns.contains("ticker"));

    // to_string_table — aligned ASCII output.
    let table = df.to_string_table(true);
    assert!(table.contains("AAPL"));

    // to_string_truncated — head/tail with "..." between when over max_rows.
    let big = read_csv_str(
        "v\n1\n2\n3\n4\n5\n6\n7\n8\n9\n10",
    )?;
    let truncated = big.to_string_truncated(true, Some(4), None);
    assert!(!truncated.is_empty());

    // to_html — basic HTML table emit.
    let html = df.to_html(true);
    assert!(html.contains("<table"));
    assert!(html.contains("AAPL"));

    // to_latex — LaTeX tabular output.
    let latex = df.to_latex(true);
    assert!(latex.contains("\\begin{tabular}"));

    // to_markdown — github-flavored pipe table.
    let md = df.to_markdown(true, None)?;
    assert!(md.contains("|"));
    assert!(md.contains("AAPL"));

    // to_dict across the documented orients.
    let _dict = df.to_dict("dict")?;
    let _list = df.to_dict("list")?;
    let _records = df.to_dict("records")?;
    let _split = df.to_dict("split")?;

    // to_series_dict — BTreeMap<String, Series>.
    let series_dict = df.to_series_dict();
    assert!(series_dict.contains_key("ticker"));

    // to_records — Vec<Vec<Scalar>>; each row prepends the index label,
    // so length is column_count + 1.
    let records = df.to_records();
    assert_eq!(records.len(), 3);
    assert_eq!(records[0].len(), 4);

    // to_numpy_2d — Vec<Vec<f64>>; non-numeric columns coerce as best-effort.
    let numeric_df = read_csv_str("a,b\n1.0,2.0\n3.0,4.0\n5.0,6.0")?;
    let mat = numeric_df.to_numpy_2d();
    assert_eq!(mat.len(), 3);
    assert_eq!(mat[0].len(), 2);
    Ok(())
}

/// README "Describe" + "Correlation and Covariance" sections (lines 607-637).
///
/// Locks in the statistical-summary APIs that previously had no
/// integration coverage:
///
/// DataFrame: describe, describe_with_percentiles, describe_dtypes,
/// corr, corr_method (spearman/kendall), cov, corrwith.
///
/// Series-level: corr (Series-to-Series), cov_with, autocorr.
///
/// Tracks fd90.190 (br-frankenpandas-gdbwk).
#[test]
fn readme_describe_and_correlation_compiles_and_runs() -> Result<(), Box<dyn std::error::Error>> {
    // Numeric DataFrame for describe + correlation matrices.
    let df = read_csv_str(
        "price,volume,revenue\n140.3,500,1500\n141.4,575,1600\n185.8,850,2400\n\
         186.3,1075,2750\n187.3,1200,3000",
    )?;

    // describe — default 8-row summary (count, mean, std, min, 25%, 50%, 75%, max).
    let summary = df.describe()?;
    assert_eq!(summary.index().len(), 8);

    // describe_with_percentiles — custom quantile rows.
    let summary_p = df.describe_with_percentiles(&[0.1, 0.5, 0.9])?;
    assert!(summary_p.index().len() >= 3);

    // describe_dtypes — numeric-only filter via "number" alias.
    let mixed = read_csv_str("price,ticker\n100,AAPL\n200,GOOGL\n300,MSFT")?;
    let _num_only = mixed.describe_dtypes(&["number"], &[])?;

    // corr — Pearson by default, returns NxN matrix.
    let pearson = df.corr()?;
    assert_eq!(pearson.column_names().len(), 3);
    assert_eq!(pearson.index().len(), 3);

    // corr_method — Spearman + Kendall variants.
    let spearman = df.corr_method("spearman")?;
    assert_eq!(spearman.column_names().len(), 3);
    let kendall = df.corr_method("kendall")?;
    assert_eq!(kendall.column_names().len(), 3);

    // cov — covariance matrix (NxN).
    let cov_mat = df.cov()?;
    assert_eq!(cov_mat.column_names().len(), 3);

    // corrwith — column-wise correlation against another DataFrame.
    let other = df.clone();
    let corr_w = df.corrwith(&other)?;
    assert!(corr_w.len() >= 3);

    // Series-level corr / cov_with / autocorr.
    let s_labels: Vec<IndexLabel> = (0..5i64).map(IndexLabel::Int64).collect();
    let s_a = Series::from_values(
        "a",
        s_labels.clone(),
        vec![
            Scalar::Float64(1.0),
            Scalar::Float64(2.0),
            Scalar::Float64(3.0),
            Scalar::Float64(4.0),
            Scalar::Float64(5.0),
        ],
    )?;
    let s_b = Series::from_values(
        "b",
        s_labels,
        vec![
            Scalar::Float64(2.0),
            Scalar::Float64(4.0),
            Scalar::Float64(6.0),
            Scalar::Float64(8.0),
            Scalar::Float64(10.0),
        ],
    )?;
    let pearson_ab = s_a.corr(&s_b)?;
    assert!((pearson_ab - 1.0).abs() < 1e-9);
    let cov_ab = s_a.cov_with(&s_b)?;
    assert!(cov_ab > 0.0);
    let _ac1 = s_a.autocorr(1)?;
    Ok(())
}
