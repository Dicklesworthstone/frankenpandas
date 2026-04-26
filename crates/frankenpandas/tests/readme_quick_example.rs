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
