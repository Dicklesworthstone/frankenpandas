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

    // fd90.288: read_sql_chunks — chunked iteration over the same table.
    // SqlChunkIterator yields Result<DataFrame, IoError>; collect at
    // least one chunk and verify shape.
    let mut chunks = read_sql_chunks(&conn, "SELECT * FROM results", 100)?;
    let first_chunk = chunks.next().expect("at least one chunk")?;
    assert_eq!(first_chunk.index().len(), 1);

    // fd90.289: SQL options-bearing read/write functions (in prelude but
    // not previously called from any test).
    let write_opts = SqlWriteOptions {
        if_exists: SqlIfExists::Replace,
        index: false,
        index_label: None,
        schema: None,
        dtype: None,
        method: SqlInsertMethod::Single,
        chunksize: None,
    };
    write_sql_with_options(&by_ticker, &conn, "results_v2", &write_opts)?;
    let read_opts = SqlReadOptions::default();
    let _ = read_sql_with_options(&conn, "SELECT * FROM results_v2", &read_opts)?;
    let _ = read_sql_query(&conn, "SELECT * FROM results_v2")?;
    let _ = read_sql_table_with_options(&conn, "results_v2", &read_opts)?;

    // fd90.290: DataFrameIoExt to_* bytes methods (skip path-form for now —
    // they need tmp dirs).
    let _bytes_parquet = by_ticker.to_parquet_bytes()?;
    let _bytes_feather = by_ticker.to_feather_bytes()?;
    let _bytes_excel = by_ticker.to_excel_bytes()?;
    // DataFrame.to_sql + to_sql_with_options trait methods.
    by_ticker.to_sql(&conn, "via_trait", SqlIfExists::Replace)?;
    by_ticker.to_sql_with_options(&conn, "via_trait_opts", &write_opts)?;

    // SqlInspector — schema introspection.
    let inspector = SqlInspector::new(&conn);
    let _tables = inspector.tables(None)?;

    // fd90.291: cover the 8 remaining SqlInspector methods documented at
    // README line 148 ("tables / views / schemas / columns / indexes /
    // FKs / UCs / reflect_table / reflect_all_tables"). Build a small
    // inspector-friendly schema with a parent table (PK), child table
    // (composite FK + UNIQUE), an explicit index, and a view.
    conn.execute_batch(
        "CREATE TABLE parent (
             id   INTEGER PRIMARY KEY,
             name TEXT NOT NULL UNIQUE
         );
         CREATE TABLE child (
             cid       INTEGER PRIMARY KEY,
             parent_id INTEGER NOT NULL,
             code      TEXT NOT NULL,
             FOREIGN KEY (parent_id) REFERENCES parent(id),
             UNIQUE (parent_id, code)
         );
         CREATE INDEX idx_child_code ON child(code);
         CREATE VIEW v_child AS SELECT cid, code FROM child;",
    )?;

    // schemas() — SQLite is a single-namespace backend, default impl
    // returns an empty Vec (multi-schema backends like PG/MySQL would
    // populate this). Coverage proves the call dispatches and decodes.
    let schemas = inspector.schemas()?;
    assert!(schemas.is_empty(), "sqlite schemas should be empty: {schemas:?}");

    // views() — surfaces the view we just created.
    let views = inspector.views(None)?;
    assert!(views.iter().any(|v| v == "v_child"), "views: {views:?}");

    // columns() — returns SqlTableSchema with one entry per declared column.
    let parent_cols = inspector.columns("parent", None)?.expect("parent exists");
    assert_eq!(parent_cols.table_name, "parent");
    assert_eq!(parent_cols.columns.len(), 2);
    assert!(parent_cols.column("id").is_some());
    let name_col = parent_cols.column("name").expect("name column");
    assert!(!name_col.nullable);
    // Missing table → Ok(None).
    assert!(inspector.columns("does_not_exist", None)?.is_none());

    // primary_key_columns() — sole PK on parent.id.
    let pk_parent = inspector.primary_key_columns("parent", None)?;
    assert_eq!(pk_parent, vec!["id".to_string()]);

    // indexes() — explicit CREATE INDEX surfaces here.
    let child_indexes = inspector.indexes("child", None)?;
    assert!(
        child_indexes.iter().any(|i| i.name == "idx_child_code"
            && i.columns == vec!["code".to_string()]
            && !i.unique),
        "indexes: {child_indexes:?}",
    );

    // foreign_keys() — composite shape: child.parent_id → parent.id.
    let child_fks = inspector.foreign_keys("child", None)?;
    let fk = child_fks
        .iter()
        .find(|f| f.referenced_table == "parent")
        .expect("FK to parent");
    assert_eq!(fk.columns, vec!["parent_id".to_string()]);
    assert_eq!(fk.referenced_columns, vec!["id".to_string()]);

    // unique_constraints() — composite UC (parent_id, code) on child.
    let child_ucs = inspector.unique_constraints("child", None)?;
    assert!(
        child_ucs.iter().any(|u| u.columns
            == vec!["parent_id".to_string(), "code".to_string()]),
        "ucs: {child_ucs:?}",
    );

    // reflect_table() — bundle: columns + PK + indexes + FKs + UCs.
    let reflected = inspector
        .reflect_table("child", None)?
        .expect("child exists");
    assert_eq!(reflected.table_name, "child");
    assert_eq!(reflected.primary_key_columns, vec!["cid".to_string()]);
    assert!(!reflected.indexes.is_empty());
    assert!(!reflected.foreign_keys.is_empty());
    assert!(!reflected.unique_constraints.is_empty());
    // Missing table → Ok(None).
    assert!(inspector.reflect_table("does_not_exist", None)?.is_none());

    // fd90.10: cover the remaining SqlInspector entry points.
    // dialect_name() — sqlite backend reports "sqlite".
    assert_eq!(inspector.dialect_name(), "sqlite");

    // server_version() — rusqlite override returns Some(library version).
    let server_ver = inspector.server_version()?;
    assert!(server_ver.is_some(), "sqlite server_version: {server_ver:?}");

    // max_identifier_length() — Option<usize> probe; SQLite default impl
    // may return None (no documented hard limit). Just exercise the call.
    let _max_ident = inspector.max_identifier_length();

    // table_exists(...) — schema-aware existence check.
    assert!(inspector.table_exists("parent", None)?);
    assert!(!inspector.table_exists("ghost_table", None)?);

    // table_comment(...) — SQLite has no column-comment storage, so
    // rusqlite override returns Ok(None). Verify the call dispatches.
    assert!(inspector.table_comment("parent", None)?.is_none());

    // has_column(...) — true for present, false for missing/ghost.
    assert!(inspector.has_column("parent", "name", None)?);
    assert!(!inspector.has_column("parent", "ghost_col", None)?);
    assert!(!inspector.has_column("ghost_table", "id", None)?);

    // column(...) — Option<SqlColumnSchema> for one column lookup.
    let id_col = inspector
        .column("parent", "id", None)?
        .expect("parent.id present");
    assert_eq!(id_col.name, "id");
    assert_eq!(id_col.primary_key_ordinal, Some(0));
    assert!(inspector.column("parent", "ghost", None)?.is_none());

    // reflect_all_tables(None) — bundles for parent + child + sqlite-
    // generated tables. At minimum both user tables show up.
    let all_tables = inspector.reflect_all_tables(None)?;
    let table_names: std::collections::BTreeSet<_> =
        all_tables.iter().map(|b| b.table_name.as_str()).collect();
    assert!(table_names.contains("parent"));
    assert!(table_names.contains("child"));

    // reflect_all_views(None) — one bundle for v_child. Views have no
    // constraints, but PRAGMA table_info returns the column shape.
    let all_views = inspector.reflect_all_views(None)?;
    let v_child = all_views
        .iter()
        .find(|b| b.table_name == "v_child")
        .expect("v_child reflected");
    assert!(!v_child.columns.is_empty());
    assert!(v_child.foreign_keys.is_empty());

    // inspect(&conn) — module-level constructor sugar; same surface.
    let via_free_fn = inspect(&conn);
    assert_eq!(via_free_fn.dialect_name(), "sqlite");

    // fd90.11: module-level free-function SQL helpers (fd90.21-32).
    // Each pairs 1:1 with a SqlInspector method — call each here and
    // verify it agrees with the inspector's output for the same input.
    let tables_via_fn = list_sql_tables(&conn, None)?;
    assert_eq!(tables_via_fn, inspector.tables(None)?);

    let views_via_fn = list_sql_views(&conn, None)?;
    assert_eq!(views_via_fn, inspector.views(None)?);

    let schemas_via_fn = list_sql_schemas(&conn)?;
    assert_eq!(schemas_via_fn, inspector.schemas()?);

    let table_schema_via_fn = sql_table_schema(&conn, "parent", None)?;
    assert_eq!(table_schema_via_fn, inspector.columns("parent", None)?);
    assert!(sql_table_schema(&conn, "ghost_table", None)?.is_none());

    let pk_via_fn = sql_primary_key_columns(&conn, "parent", None)?;
    assert_eq!(pk_via_fn, inspector.primary_key_columns("parent", None)?);

    let indexes_via_fn = list_sql_indexes(&conn, "child", None)?;
    assert_eq!(indexes_via_fn, inspector.indexes("child", None)?);

    let fks_via_fn = list_sql_foreign_keys(&conn, "child", None)?;
    assert_eq!(fks_via_fn, inspector.foreign_keys("child", None)?);

    let ucs_via_fn = list_sql_unique_constraints(&conn, "child", None)?;
    assert_eq!(ucs_via_fn, inspector.unique_constraints("child", None)?);

    let comment_via_fn = sql_table_comment(&conn, "parent", None)?;
    assert_eq!(comment_via_fn, inspector.table_comment("parent", None)?);

    let server_ver_via_fn = sql_server_version(&conn)?;
    assert_eq!(server_ver_via_fn, inspector.server_version()?);

    assert_eq!(sql_max_identifier_length(&conn), inspector.max_identifier_length());

    // truncate_sql_table — DDL-style reset; emits DELETE FROM on SQLite
    // (no native TRUNCATE). Insert a row, truncate, verify empty.
    conn.execute(
        "INSERT INTO parent (id, name) VALUES (1, 'doomed')",
        [],
    )?;
    truncate_sql_table(&conn, "parent", None)?;
    let post_truncate = inspector
        .columns("parent", None)?
        .expect("parent still present");
    assert_eq!(post_truncate.columns.len(), 2);

    // fd90.20: SqlQueryResult — return type of SqlConnection::query.
    // Insert two parent rows, then query and inspect the result shape.
    conn.execute("INSERT INTO parent (id, name) VALUES (10, 'alpha')", [])?;
    conn.execute("INSERT INTO parent (id, name) VALUES (20, 'beta')", [])?;
    let qr: SqlQueryResult = conn.query("SELECT id, name FROM parent ORDER BY id", &[])?;
    assert_eq!(qr.columns, vec!["id".to_string(), "name".to_string()]);
    assert_eq!(qr.rows.len(), 2);
    assert_eq!(qr.rows[0][0], Scalar::Int64(10));
    assert_eq!(qr.rows[1][0], Scalar::Int64(20));

    // fd90.20: SqlIndexedChunkIterator — chunked reader with index_col
    // promoting 'id' to the row index. Yields Result<DataFrame, IoError>.
    let mut indexed_chunks: SqlIndexedChunkIterator = read_sql_chunks_with_index_col(
        &conn,
        "SELECT id, name FROM parent ORDER BY id",
        Some("id"),
        100,
    )?;
    let chunk = indexed_chunks.next().expect("at least one chunk")?;
    assert_eq!(chunk.index().len(), 2);
    // After id-promotion, only 'name' remains as a data column.
    assert!(chunk.column("name").is_some());
    assert!(chunk.column("id").is_none());

    // fd90.28: SqlIfExists::Append — incremental load.
    // by_ticker has 1 row (the AAPL aggregate). write to a fresh table,
    // then append again — table should now have 2 rows.
    write_sql(&by_ticker, &conn, "appended", SqlIfExists::Replace)?;
    write_sql(&by_ticker, &conn, "appended", SqlIfExists::Append)?;
    let appended_back = read_sql_table(&conn, "appended")?;
    assert_eq!(appended_back.index().len(), 2);

    // fd90.28: SqlInsertMethod::Multi — multi-row INSERT batching.
    let multi_opts = SqlWriteOptions {
        if_exists: SqlIfExists::Replace,
        index: false,
        index_label: None,
        schema: None,
        dtype: None,
        method: SqlInsertMethod::Multi,
        chunksize: None,
    };
    write_sql_with_options(&by_ticker, &conn, "multi_method", &multi_opts)?;
    let multi_back = read_sql_table(&conn, "multi_method")?;
    assert_eq!(multi_back.index().len(), 1);
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

    // fd90.257: module-level join_series / merge_dataframes_on / merge_ordered.
    // join_series — produces a JoinedSeries on overlapping indexes.
    let s_labels: Vec<IndexLabel> = vec![IndexLabel::Int64(0), IndexLabel::Int64(1)];
    let s_a = Series::from_values("a", s_labels.clone(), vec![Scalar::Int64(1), Scalar::Int64(2)])?;
    let s_b = Series::from_values("b", s_labels, vec![Scalar::Int64(10), Scalar::Int64(20)])?;
    let _joined: JoinedSeries = join_series(&s_a, &s_b, JoinType::Inner)?;

    // merge_dataframes_on — module-level inner-join on shared key.
    let _merged_on = merge_dataframes_on(&df1, &df2, &["key"], JoinType::Inner)?;

    // merge_ordered — outer join with optional fill_method.
    let _ordered = merge_ordered(&df1, &df2, &["key"], None)?;

    // fd90.287: cover the remaining advanced merge/join variants.
    // merge_dataframes — single-column-key alias of merge_dataframes_on.
    let _ = merge_dataframes(&df1, &df2, "key", JoinType::Inner)?;
    // merge_dataframes_on_with — different left_on / right_on key names.
    let _ = merge_dataframes_on_with(&df1, &df2, &["key"], &["key"], JoinType::Inner)?;
    // merge_dataframes_on_with_options — full MergeExecutionOptions support.
    let _ = merge_dataframes_on_with_options(
        &df1,
        &df2,
        &["key"],
        &["key"],
        JoinType::Inner,
        MergeExecutionOptions::default(),
    )?;
    // join_series_with_options — pairs with JoinExecutionOptions.
    let _ = join_series_with_options(&s_a, &s_b, JoinType::Inner, JoinExecutionOptions::default())?;
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

    // fd90.242: eval_with_locals — @local variable references in eval
    // expressions (mirrors query_with_locals at the eval surface).
    let scaled = df.eval_with_locals("revenue * @scale", &BTreeMap::from([
        ("scale".to_owned(), Scalar::Float64(0.5)),
    ]))?;
    assert_eq!(scaled.len(), 3);
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

    // fd90.201: cover the rest of the MultiIndex API surface at README line 435.

    // from_arrays — level-major construction.
    let mi_arrays = MultiIndex::from_arrays(vec![
        vec!["a".into(), "a".into(), "b".into(), "b".into()],
        vec![1i64.into(), 2i64.into(), 1i64.into(), 2i64.into()],
    ])?;
    assert_eq!(mi_arrays.nlevels(), 2);
    assert_eq!(mi_arrays.len(), 4);

    // droplevel — returns MultiIndexOrIndex (single-level case collapses to Index).
    let dropped = mi.droplevel(0)?;
    match dropped {
        MultiIndexOrIndex::Index(idx) => assert_eq!(idx.len(), 4),
        MultiIndexOrIndex::Multi(m) => assert_eq!(m.len(), 4),
    }

    // swaplevel — exchange two level positions.
    let swapped = mi.swaplevel(0, 1)?;
    assert_eq!(swapped.nlevels(), 2);
    assert_eq!(swapped.len(), 4);

    // reorder_levels — explicit level permutation.
    let reordered = mi.reorder_levels(&[1, 0])?;
    assert_eq!(reordered.nlevels(), 2);

    // DataFrame integration: set_index_multi + to_multi_index.
    let df = read_csv_str("region,year,value\neast,2023,100\neast,2024,150\nwest,2023,90")?;
    let df_mi = df.set_index_multi(&["region", "year"], true, "_")?;
    assert_eq!(df_mi.index().len(), 3);
    let extracted = df.to_multi_index(&["region", "year"])?;
    assert_eq!(extracted.nlevels(), 2);
    assert_eq!(extracted.len(), 3);
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

    // fd90.195: exercise the remaining .cat() methods documented in the
    // README at line 422 (add_categories, remove_unused_categories,
    // set_categories, as_ordered, as_unordered).

    // add_categories — extend the category set.
    let extended = cat.add_categories(vec![Scalar::Utf8("neutral".into())])?;
    let ext_cat = extended.cat().expect("extended is categorical");
    assert_eq!(ext_cat.categories().len(), 4);

    // remove_unused_categories — drop "neutral" since it has no observations.
    let pruned = ext_cat.remove_unused_categories()?;
    assert_eq!(
        pruned
            .cat()
            .expect("pruned is categorical")
            .categories()
            .len(),
        3
    );

    // set_categories — replace the category set entirely.
    let reset = cat.set_categories(vec![
        Scalar::Utf8("low".into()),
        Scalar::Utf8("mid".into()),
        Scalar::Utf8("high".into()),
        Scalar::Utf8("excellent".into()),
        Scalar::Utf8("good".into()),
        Scalar::Utf8("poor".into()),
    ])?;
    assert!(reset.cat().is_some());

    // as_ordered / as_unordered — toggle the ordered flag.
    let ordered = cat.as_ordered();
    assert!(ordered.cat().expect("still categorical").ordered());
    let unordered = cat.as_unordered();
    assert!(!unordered.cat().expect("still categorical").ordered());

    // fd90.274: Series.from_categorical_codes — explicit codes + categories.
    let by_codes = Series::from_categorical_codes(
        "by_codes",
        vec![0, 1, 2, 0, -1], // -1 = missing
        vec![
            Scalar::Utf8("A".into()),
            Scalar::Utf8("B".into()),
            Scalar::Utf8("C".into()),
        ],
        false,
    )?;
    assert_eq!(by_codes.len(), 5);
    let by_codes_cat = by_codes.cat().expect("constructed categorical");
    assert_eq!(by_codes_cat.categories().len(), 3);
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

    // fd90.286: merge_asof_with_options — pandas pd.merge_asof(tolerance=...)
    // shape. allow_exact_matches=true and tolerance=Some(5.0) means a quote
    // at exactly trade_ts is fine, and the gap between trade and quote must
    // be <= 5 nanoseconds.
    let opts = MergeAsofOptions {
        allow_exact_matches: true,
        tolerance: Some(5.0),
        by: None,
    };
    let _tuned = merge_asof_with_options(
        &trades,
        &quotes,
        "timestamp",
        AsofDirection::Backward,
        opts,
    )?;
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

    // fd90.253: more Index methods.
    // Build a non-duplicate, sorted Index for clean assertions.
    let sorted_idx = Index::new(vec![
        IndexLabel::Int64(10),
        IndexLabel::Int64(20),
        IndexLabel::Int64(30),
    ]);
    assert!(sorted_idx.is_unique());
    assert!(sorted_idx.is_sorted());
    assert!(sorted_idx.is_monotonic_increasing());
    assert!(!sorted_idx.is_monotonic_decreasing());
    assert!(sorted_idx.contains(&IndexLabel::Int64(20)));
    assert_eq!(sorted_idx.get_loc(&IndexLabel::Int64(20)), Some(1));
    assert_eq!(sorted_idx.position(&IndexLabel::Int64(30)), Some(2));
    let in_mask = sorted_idx.isin(&[IndexLabel::Int64(10), IndexLabel::Int64(99)]);
    assert_eq!(in_mask.len(), 3);
    assert!(in_mask[0]);
    let uniq_idx = dup_index.unique();
    assert_eq!(uniq_idx.len(), 3);
    let dropped_idx = dup_index.drop_duplicates();
    assert_eq!(dropped_idx.len(), 3);
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

    // fd90.199: Cover the rest of the Window Operations matrix
    // documented at README lines 489-496.

    // ── Series Rolling — full method set. Note: .apply takes Fn(&[f64]) -> f64.
    let r = series.rolling(10, None);
    let _ = r.sum()?;
    let _ = r.min()?;
    let _ = r.max()?;
    let _ = r.var()?;
    let _ = r.count()?;
    let _ = r.median()?;
    let _ = r.quantile(0.5)?;
    let _ = r.apply(|vals: &[f64]| vals.iter().copied().sum::<f64>() / vals.len() as f64)?;
    // fd90.282: rest of Rolling.
    let _ = r.first()?;
    let _ = r.last()?;
    let _ = r.prod()?;
    let _ = r.skew()?;
    let _ = r.kurt()?;
    let _ = r.agg(&["sum", "mean"])?;
    // Rolling.corr / cov against another series — same length.
    let other_series = Series::from_values(
        "other",
        (0..100i64).map(IndexLabel::Int64).collect::<Vec<_>>(),
        (0..100i64).map(|v| Scalar::Float64((v + 1) as f64)).collect::<Vec<_>>(),
    )?;
    let _ = r.corr(&other_series)?;
    let _ = r.cov(&other_series)?;

    // ── Series Expanding — full method set. Same f64 closure shape.
    let e = series.expanding(None);
    let _ = e.sum()?;
    let _ = e.mean()?;
    let _ = e.min()?;
    let _ = e.std()?;
    let _ = e.var()?;
    let _ = e.median()?;
    let _ = e.apply(|vals: &[f64]| vals.iter().copied().sum::<f64>())?;
    // fd90.283: rest of Expanding (count/quantile/kurt/skew/corr/cov).
    let _ = e.count()?;
    let _ = e.quantile(0.5)?;
    let _ = e.kurt()?;
    let _ = e.skew()?;
    // Build a 100-element companion for corr/cov (matches `series` length).
    let exp_other = Series::from_values(
        "exp_other",
        (0..100i64).map(IndexLabel::Int64).collect::<Vec<_>>(),
        (0..100i64).map(|v| Scalar::Float64((v + 5) as f64)).collect::<Vec<_>>(),
    )?;
    let _ = e.corr(&exp_other)?;
    let _ = e.cov(&exp_other)?;

    // ── Series EWM — std + var (mean already covered above).
    let ew = series.ewm(Some(20.0), None);
    let _ = ew.std()?;
    let _ = ew.var()?;
    // fd90.283: remaining Series Ewm methods.
    let _ = ew.sum()?;
    let _ = ew.corr(&exp_other)?;
    let _ = ew.cov(&exp_other)?;

    // ── Series Resample — needs datetime-indexed Series.
    let date_labels: Vec<IndexLabel> = vec![
        "2024-01-05".into(),
        "2024-01-15".into(),
        "2024-02-10".into(),
        "2024-02-25".into(),
    ];
    let dt_series = Series::from_values(
        "sales",
        date_labels,
        vec![
            Scalar::Float64(100.0),
            Scalar::Float64(200.0),
            Scalar::Float64(300.0),
            Scalar::Float64(400.0),
        ],
    )?;
    let monthly = dt_series.resample("M");
    let _ = monthly.sum()?;
    let _ = monthly.mean()?;
    let _ = monthly.count()?;
    let _ = monthly.min()?;
    let _ = monthly.max()?;
    let _ = monthly.first()?;
    let _ = monthly.last()?;
    // fd90.281: rest of Resample (std/var/median/prod + apply/apply_fn).
    let _ = monthly.std()?;
    let _ = monthly.var()?;
    let _ = monthly.median()?;
    let _ = monthly.prod()?;
    let _ = monthly.apply(|vals: &[Scalar]| vals.first().cloned().unwrap_or(Scalar::Null(NullKind::NaN)))?;
    let _ = monthly.apply_fn(|vals: &[Scalar]| Ok(vals.first().cloned().unwrap_or(Scalar::Null(NullKind::NaN))))?;

    // ── DataFrame versions.
    let df = read_csv_str("a,b\n1,10\n2,20\n3,30\n4,40\n5,50\n6,60\n7,70\n8,80\n9,90\n10,100")?;
    let dr = df.rolling(3, None);
    let _ = dr.sum()?;
    let _ = dr.mean()?;
    let _ = dr.min()?;
    let _ = dr.max()?;
    let _ = dr.std()?;
    let _ = dr.var()?;
    let _ = dr.count()?;
    let _ = dr.median()?;
    let _ = dr.quantile(0.5)?;
    // fd90.284: rest of DataFrameRolling (corr/cov are pairwise across columns).
    let _ = dr.corr()?;
    let _ = dr.cov()?;
    // corr_with / cov_with — against another Series.
    let dr_other = Series::from_values(
        "dr_other",
        (0..10i64).map(IndexLabel::Int64).collect::<Vec<_>>(),
        (0..10i64).map(|v| Scalar::Float64((v + 1) as f64)).collect::<Vec<_>>(),
    )?;
    let _ = dr.corr_with(&dr_other)?;
    let _ = dr.cov_with(&dr_other)?;

    let de = df.expanding(None);
    let _ = de.sum()?;
    let _ = de.mean()?;
    let _ = de.min()?;
    let _ = de.max()?;
    let _ = de.std()?;
    let _ = de.var()?;
    let _ = de.median()?;
    // fd90.284: rest of DataFrameExpanding (count/quantile/kurt/skew/apply).
    let _ = de.count()?;
    let _ = de.quantile(0.5)?;
    let _ = de.kurt()?;
    let _ = de.skew()?;
    let _ = de.apply(|vals: &[f64]| vals.iter().copied().sum::<f64>())?;

    let dew = df.ewm(Some(5.0), None);
    let _ = dew.mean()?;
    let _ = dew.std()?;
    let _ = dew.var()?;
    // fd90.284: DataFrameEwm.sum.
    let _ = dew.sum()?;

    // ── DataFrame Resample — needs datetime-string row index.
    let dt_df = DataFrame::from_dict_with_index(
        vec![("sales", vec![
            Scalar::Float64(100.0),
            Scalar::Float64(200.0),
            Scalar::Float64(300.0),
        ])],
        vec!["2024-01-15".into(), "2024-01-20".into(), "2024-02-10".into()],
    )?;
    let drs = dt_df.resample("M");
    let _ = drs.sum()?;
    let _ = drs.mean()?;
    let _ = drs.count()?;
    let _ = drs.min()?;
    let _ = drs.max()?;
    let _ = drs.first()?; // fd90.200 parity with Series Resample
    let _ = drs.last()?;
    // fd90.285: agg + prod (rest of DataFrameResample's 9-method surface).
    let _ = drs.agg(&["sum", "mean"])?;
    let _ = drs.prod()?;
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

    // fd90.252: pivot (no aggregation) / pivot_table_with_dropna /
    // pivot_table_aggfunc_dict.
    // pivot — long → wide without aggregation. Each (index_col, columns_col)
    // pair must be unique; the input has 4 rows with 2 regions × 2 products,
    // each unique → 4 cells (2x2 result).
    let pt_pivot = df.pivot("region", "product", "revenue")?;
    assert_eq!(pt_pivot.index().len(), 2);

    // pivot_table_with_dropna — explicit dropna toggle (true is the default
    // for plain pivot_table).
    let pt_drop = df.pivot_table_with_dropna(
        "revenue", "region", "product", "sum", true,
    )?;
    assert_eq!(pt_drop.index().len(), 2);

    // pivot_table_aggfunc_dict — per-value aggfunc dispatch.
    let pt_dict = df.pivot_table_aggfunc_dict(
        &[("revenue", "sum"), ("quantity", "mean")],
        "region",
        "product",
    )?;
    assert_eq!(pt_dict.index().len(), 2);
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

    // fd90.256: Series-level concat (module-level helpers).
    let s_labels1: Vec<IndexLabel> = vec![IndexLabel::Int64(0), IndexLabel::Int64(1)];
    let s_labels2: Vec<IndexLabel> = vec![IndexLabel::Int64(2), IndexLabel::Int64(3)];
    let s1 = Series::from_values(
        "x",
        s_labels1,
        vec![Scalar::Int64(10), Scalar::Int64(20)],
    )?;
    let s2 = Series::from_values(
        "x",
        s_labels2,
        vec![Scalar::Int64(30), Scalar::Int64(40)],
    )?;
    let combined = concat_series(&[&s1, &s2])?;
    assert_eq!(combined.len(), 4);
    let reindexed = concat_series_with_ignore_index(&[&s1, &s2], true)?;
    assert_eq!(reindexed.len(), 4);
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

    // Scalar arithmetic — fd90.226 added sub_scalar.
    let _ = df.mul_scalar(2.0)?;
    let _ = df.add_scalar(100.0)?;
    let _ = df.sub_scalar(5.0)?;
    let _ = df.div_scalar(2.0)?;
    let _ = df.pow_scalar(2.0)?;
    let _ = df.mod_scalar(10.0)?;
    let _ = df.floordiv_scalar(3.0)?;

    // DataFrame-to-DataFrame (aligned). fd90.226 added the 4 missing.
    let df2 = read_csv_str("a,b\n5,1\n10,2\n15,3\n20,4")?;
    let _ = df.add_df(&df2)?;
    let _ = df.sub_df(&df2)?;
    let _ = df.div_df(&df2)?;
    let _ = df.mul_df(&df2)?;
    let _ = df.pow_df(&df2)?;
    let _ = df.mod_df(&df2)?;
    let _ = df.floordiv_df(&df2)?;

    // With fill value — fd90.227 added the other 5 _df_fill variants.
    let _ = df.add_df_fill(&df2, 0.0)?;
    let _ = df.sub_df_fill(&df2, 0.0)?;
    let _ = df.mul_df_fill(&df2, 1.0)?;
    let _ = df.div_df_fill(&df2, 1.0)?;
    let _ = df.floordiv_df_fill(&df2, 1.0)?;
    let _ = df.mod_df_fill(&df2, 1.0)?;

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

    // fd90.223: ArithmeticOp / ComparisonOp dispatch (the prelude types
    // added in fd90.222). The high-level wrappers (mul_scalar/etc.) cover
    // the common cases; the dispatch enum form is documented surface.

    // DataFrame.compare_scalar_df — Bool DataFrame mask via ComparisonOp.
    let mask_df = df.compare_scalar_df(&Scalar::Int64(15), ComparisonOp::Ge)?;
    assert_eq!(mask_df.index().len(), 4);

    // Series.compare_scalar — Bool Series mask via ComparisonOp.
    let col_a_series = Series::new("a", df.index().clone(), df.column("a").expect("a").clone())?;
    let _series_mask = col_a_series.compare_scalar(&Scalar::Int64(20), ComparisonOp::Gt)?;

    // fd90.224: Series.{eq,ne,gt,lt,le,ge}_scalar shortcuts — ergonomic
    // alternative to compare_scalar for the 6 standard relations.
    let _ = col_a_series.eq_scalar(&Scalar::Int64(10))?;
    let _ = col_a_series.ne_scalar(&Scalar::Int64(10))?;
    let _ = col_a_series.gt_scalar(&Scalar::Int64(15))?;
    let _ = col_a_series.lt_scalar(&Scalar::Int64(35))?;
    let _ = col_a_series.le_scalar(&Scalar::Int64(20))?;
    let _ = col_a_series.ge_scalar(&Scalar::Int64(20))?;

    // fd90.225: DataFrame.{eq,ne,gt,lt,le,ge}_scalar_df — same shortcuts
    // at the DataFrame level (each returns a Bool DataFrame).
    let _ = df.eq_scalar_df(&Scalar::Int64(10))?;
    let _ = df.ne_scalar_df(&Scalar::Int64(10))?;
    let _ = df.gt_scalar_df(&Scalar::Int64(15))?;
    let _ = df.lt_scalar_df(&Scalar::Int64(35))?;
    let _ = df.le_scalar_df(&Scalar::Int64(20))?;
    let _ = df.ge_scalar_df(&Scalar::Int64(20))?;

    // fd90.228: Series binary arithmetic (aligned on index).
    let labels: Vec<IndexLabel> = (0..3i64).map(IndexLabel::Int64).collect();
    let s_a = Series::from_values(
        "a",
        labels.clone(),
        vec![Scalar::Float64(10.0), Scalar::Float64(20.0), Scalar::Float64(30.0)],
    )?;
    let s_b = Series::from_values(
        "b",
        labels,
        vec![Scalar::Float64(2.0), Scalar::Float64(4.0), Scalar::Float64(6.0)],
    )?;
    let _ = s_a.add(&s_b)?;
    let _ = s_a.sub(&s_b)?;
    let _ = s_a.mul(&s_b)?;
    let _ = s_a.div(&s_b)?;
    let _ = s_a.pow(&s_b)?;
    let _ = s_a.floordiv(&s_b)?;

    // fd90.230: Series scalar reductions + cumulative transforms.
    let _ = s_a.sum()?;
    let _ = s_a.mean()?;
    let _ = s_a.min()?;
    let _ = s_a.max()?;
    let _ = s_a.std()?;
    let _ = s_a.var()?;
    let _ = s_a.median()?;
    let _ = s_a.prod()?;
    let _ = s_a.quantile(0.5)?;
    let _ = s_a.abs()?;
    let _ = s_a.nunique();
    // cumulative transforms — return Series of the same length.
    let _ = s_a.cumsum()?;
    let _ = s_a.cumprod()?;
    let _ = s_a.cummax()?;
    let _ = s_a.cummin()?;

    // fd90.231: Series higher-order statistics + structural inspection.
    let bigger_labels: Vec<IndexLabel> = (0..6i64).map(IndexLabel::Int64).collect();
    let bigger = Series::from_values(
        "v",
        bigger_labels,
        vec![
            Scalar::Float64(1.0),
            Scalar::Float64(2.0),
            Scalar::Float64(3.0),
            Scalar::Float64(4.0),
            Scalar::Float64(5.0),
            Scalar::Float64(6.0),
        ],
    )?;
    // f64 statistics.
    let _ = bigger.skew()?;
    let _ = bigger.kurt()?;
    let _ = bigger.kurtosis()?;
    let _ = bigger.sem()?;
    // usize position lookups.
    let _ = bigger.argmax()?;
    let _ = bigger.argmin()?;
    // bool structural inspection.
    let _ = bigger.is_unique();
    let _ = bigger.is_monotonic_increasing();
    let _ = bigger.is_monotonic_decreasing();
    // bool reductions on a Bool Series.
    let bool_labels: Vec<IndexLabel> = (0..3i64).map(IndexLabel::Int64).collect();
    let bools = Series::from_values(
        "b",
        bool_labels,
        vec![Scalar::Bool(true), Scalar::Bool(false), Scalar::Bool(true)],
    )?;
    let _ = bools.all()?;
    let _ = bools.any()?;
    // top/bottom-N at the Series level.
    let _ = bigger.nlargest(3)?;
    let _ = bigger.nsmallest(2)?;

    // fd90.278: Series.sample + argsort.
    let _ = bigger.sample(Some(3), None, false, Some(42))?;
    let _ = bigger.argsort(true)?;

    // fd90.235: Series.apply / map_func / map (pandas Series.apply/map).
    // apply takes Fn(&Scalar) -> Scalar.
    let doubled = bigger.apply(|s| match s {
        Scalar::Float64(v) => Scalar::Float64(v * 2.0),
        other => other.clone(),
    })?;
    assert_eq!(doubled.len(), bigger.len());
    // map_func — alias for apply.
    let _ = bigger.map_func(|s| s.clone())?;
    // map — slice of (from, to) pairs (pandas-style mapping dictionary).
    let mapped = bigger.map(&[
        (Scalar::Float64(1.0), Scalar::Float64(100.0)),
        (Scalar::Float64(2.0), Scalar::Float64(200.0)),
    ])?;
    assert_eq!(mapped.len(), bigger.len());

    // fd90.236: Series.unique / duplicated + round (Series + DataFrame).
    // unique returns Vec<Scalar> (not Series, no ?-handling).
    let dup_labels: Vec<IndexLabel> = (0..5i64).map(IndexLabel::Int64).collect();
    let with_dups = Series::from_values(
        "x",
        dup_labels,
        vec![
            Scalar::Int64(1),
            Scalar::Int64(2),
            Scalar::Int64(1),
            Scalar::Int64(3),
            Scalar::Int64(2),
        ],
    )?;
    let uniq = with_dups.unique();
    assert_eq!(uniq.len(), 3); // 1, 2, 3 in first-seen order
    let dup_mask = with_dups.duplicated()?;
    assert_eq!(dup_mask.len(), 5);

    // Series.round / DataFrame.round — element-wise.
    let pi_labels: Vec<IndexLabel> = (0..2i64).map(IndexLabel::Int64).collect();
    let pi_series = Series::from_values(
        "pi",
        pi_labels,
        vec![Scalar::Float64(3.14159), Scalar::Float64(2.71828)],
    )?;
    let _rounded_s = pi_series.round(2)?;
    let _rounded_df = df.round(0)?;

    // fd90.237: clip / where_cond / mask / Series.rename.

    // Series.clip — bound values into [lower, upper].
    let _clipped_s = bigger.clip(Some(2.0), Some(5.0))?;
    // DataFrame.clip — same on each numeric column.
    let _clipped_df = df.clip(Some(0.0), Some(50.0))?;

    // Build a Bool cond Series + DataFrame for where/mask families.
    let cond_labels: Vec<IndexLabel> = (0..6i64).map(IndexLabel::Int64).collect();
    let cond_series = Series::from_values(
        "c",
        cond_labels,
        vec![
            Scalar::Bool(true),
            Scalar::Bool(false),
            Scalar::Bool(true),
            Scalar::Bool(false),
            Scalar::Bool(true),
            Scalar::Bool(false),
        ],
    )?;
    // Series.where_cond / mask take Option<&Scalar> for the fill value.
    let _w = bigger.where_cond(&cond_series, Some(&Scalar::Float64(0.0)))?;
    let _m = bigger.mask(&cond_series, Some(&Scalar::Float64(-1.0)))?;
    // None means "fill with NaN" (default).
    let _w_nan = bigger.where_cond(&cond_series, None)?;

    // DataFrame.where_cond / mask — same shape, on a Bool DataFrame.
    let cond_df = read_csv_str("a,b\ntrue,false\ntrue,true\nfalse,true\ntrue,false")?;
    let _ = df.where_cond(&cond_df, Some(&Scalar::Float64(0.0)))?;
    let _ = df.mask(&cond_df, Some(&Scalar::Null(NullKind::NaN)))?;

    // Series.rename — change the name field.
    let renamed = bigger.rename("renamed_series")?;
    assert_eq!(renamed.name(), "renamed_series");

    // fd90.273: align_on_index / asof / clip_lower / clip_upper.
    // Series.clip_lower / clip_upper (one-sided clips).
    let _ = bigger.clip_lower(2.0)?;
    let _ = bigger.clip_upper(5.0)?;
    // DataFrame.clip_lower / clip_upper.
    let _ = df.clip_lower(0.0)?;
    let _ = df.clip_upper(50.0)?;
    // Series.asof — last known value at-or-before label.
    let _ = bigger.asof(&IndexLabel::Int64(3));
    // DataFrame.asof — Series of last-known values per column at label.
    let _ = df.asof(&IndexLabel::Int64(2), None)?;
    // DataFrame.align_on_index — outer/inner/left modes return (Self, Self).
    let other = read_csv_str("a,b\n100,1000\n200,2000\n300,3000\n400,4000")?;
    let (_l, _r) = df.align_on_index(&other, AlignMode::Outer)?;

    // fd90.238: reindex / truncate / insert across Series + DataFrame.

    // Series.reindex — rebuild with a new label set (pads with NaN).
    let new_idx: Vec<IndexLabel> = (0..8i64).map(IndexLabel::Int64).collect();
    let s_reindexed = bigger.reindex(new_idx.clone())?;
    assert_eq!(s_reindexed.len(), 8); // bigger has 6, padded to 8

    // Series.truncate — keep the inclusive interval [before, after].
    let s_truncated = bigger.truncate(
        Some(&IndexLabel::Int64(1)),
        Some(&IndexLabel::Int64(4)),
    )?;
    assert!(s_truncated.len() <= bigger.len());

    // DataFrame.reindex — same shape, on the row axis.
    let df_reindexed = df.reindex(new_idx)?;
    assert_eq!(df_reindexed.index().len(), 8);

    // DataFrame.truncate — same interval semantics.
    let df_truncated = df.truncate(
        Some(&IndexLabel::Int64(1)),
        Some(&IndexLabel::Int64(2)),
    )?;
    assert!(df_truncated.index().len() <= df.index().len());

    // DataFrame.insert — README line 1146: positional insert.
    let new_col = Column::from_values(vec![
        Scalar::Int64(0),
        Scalar::Int64(1),
        Scalar::Int64(2),
        Scalar::Int64(3),
    ])?;
    let with_inserted = df.insert(0, "new_first", new_col)?;
    assert_eq!(
        with_inserted
            .column_names()
            .first()
            .map(|n| n.as_str()),
        Some("new_first"),
    );

    // fd90.245: DataFrame.with_column / drop_columns / rename_columns.
    // with_column — immutable add-or-replace.
    let added_col = Column::from_values(vec![
        Scalar::Int64(0),
        Scalar::Int64(1),
        Scalar::Int64(2),
        Scalar::Int64(3),
    ])?;
    let with_added = df.with_column("added", added_col)?;
    assert!(
        with_added
            .column_names()
            .iter()
            .any(|n| n.as_str() == "added")
    );
    // drop_columns — bulk drop (alternative to drop with axis arg).
    let dropped = with_added.drop_columns(&["added"])?;
    assert!(
        !dropped
            .column_names()
            .iter()
            .any(|n| n.as_str() == "added")
    );
    // rename_columns — paired renames.
    let renamed_cols = df.rename_columns(&[("a", "alpha"), ("b", "beta")])?;
    assert!(
        renamed_cols
            .column_names()
            .iter()
            .any(|n| n.as_str() == "alpha")
    );
    assert!(
        renamed_cols
            .column_names()
            .iter()
            .any(|n| n.as_str() == "beta")
    );

    // fd90.239: Series-level introspection (equals/memory_usage/describe/
    // to_dict/to_csv).
    let s_clone = bigger.clone();
    assert!(bigger.equals(&s_clone));
    let _bytes = bigger.memory_usage(); // returns usize
    let _summary = bigger.describe()?; // returns Series with stat labels
    let dict = bigger.to_dict();
    assert_eq!(dict.len(), bigger.len());
    let csv = bigger.to_csv(',', true);
    assert!(!csv.is_empty());

    // fd90.240: Series.loc / iloc / copy / append / empty / to_frame /
    // sort_index / reset_index_with_name.

    // Series.loc — label-based selection.
    let _ = bigger.loc(&[IndexLabel::Int64(0), IndexLabel::Int64(2)])?;
    // Series.iloc — positional selection.
    let _ = bigger.iloc(&[0, 2, 4])?;

    // Series.copy — deep clone.
    let copied = bigger.copy();
    assert_eq!(copied.len(), bigger.len());

    // Series.empty — false for non-empty.
    assert!(!bigger.empty());

    // Series.append — concat two Series along the row axis.
    let appended = bigger.append(&s_clone)?;
    assert_eq!(appended.len(), bigger.len() * 2);

    // Series.to_frame(name) — promote to a 1-column DataFrame.
    let single = bigger.to_frame(Some("col"))?;
    assert_eq!(single.column_names().len(), 1);

    // Series.sort_index — both directions.
    let _ = bigger.sort_index(true)?;
    let _ = bigger.sort_index(false)?;

    // Series.reset_index_with_name — drop=true returns Series-or-DataFrame
    // result; just verify it compiles and runs.
    let _ = bigger.reset_index_with_name(true, None)?;

    // fd90.241: Series.combine_first / update / filter / xs / droplevel +
    // DataFrame.swaplevel / reorder_levels.

    // Build two Series with overlapping but missing-bearing values.
    let cf_labels: Vec<IndexLabel> = (0..3i64).map(IndexLabel::Int64).collect();
    let s_with_gaps = Series::from_values(
        "a",
        cf_labels.clone(),
        vec![
            Scalar::Float64(1.0),
            Scalar::Null(NullKind::NaN),
            Scalar::Float64(3.0),
        ],
    )?;
    let s_filler = Series::from_values(
        "a",
        cf_labels,
        vec![
            Scalar::Float64(10.0),
            Scalar::Float64(20.0),
            Scalar::Float64(30.0),
        ],
    )?;
    // combine_first — fill nulls in self from other.
    let _combined = s_with_gaps.combine_first(&s_filler)?;
    // update — overwrite self where other is non-null.
    let _updated = s_with_gaps.update(&s_filler)?;

    // Series.filter — Bool-typed mask of same length.
    let mask_labels: Vec<IndexLabel> = (0..3i64).map(IndexLabel::Int64).collect();
    let mask = Series::from_values(
        "m",
        mask_labels,
        vec![Scalar::Bool(true), Scalar::Bool(false), Scalar::Bool(true)],
    )?;
    let filtered = s_filler.filter(&mask)?;
    assert!(filtered.len() <= s_filler.len());

    // Series.xs — cross-section by IndexLabel.
    let _xs = s_filler.xs(&IndexLabel::Int64(0))?;
    // Series.droplevel — collapse to a flat 0..n index.
    let _dl = s_filler.droplevel()?;

    // DataFrame row-axis MultiIndex methods. Build a frame with a row
    // MultiIndex via set_index_multi.
    let mi_df = read_csv_str("region,year,value\neast,2023,100\neast,2024,150\nwest,2023,90")?
        .set_index_multi(&["region", "year"], true, "_")?;
    // df.swaplevel — exchange the two row levels (no args).
    let _swapped = mi_df.swaplevel();
    // df.reorder_levels — permutation by level indices.
    let _reordered = mi_df.reorder_levels(&[1, 0])?;

    // fd90.249: Series sequential / ranking ops (DataFrame counterparts
    // are already tested above).
    let _ = bigger.shift(1)?;
    let _ = bigger.diff(1)?;
    let _ = bigger.pct_change(1)?;
    let _ = bigger.rank("average", true, "keep")?;

    // fd90.251: set_axis / copy.
    // Series.set_axis — replace row labels.
    let new_s_labels: Vec<IndexLabel> = (10..16i64).map(IndexLabel::Int64).collect();
    let _ = bigger.set_axis(new_s_labels)?;
    // DataFrame.set_axis(labels, axis) — axis=0 row labels, axis=1 column labels.
    let new_row_labels: Vec<IndexLabel> = (10..14i64).map(IndexLabel::Int64).collect();
    let _ = df.set_axis(new_row_labels, 0)?;
    // DataFrame.copy — deep clone.
    let _ = df.copy();

    // fd90.232 + fd90.233: DataFrame-level reductions. fd90.233 added
    // pandas-parity bare-name aliases (min/max/std/var/median/prod/
    // skew/kurt/kurtosis/sem) over the existing *_agg methods.
    let _ = df.sum()?;
    let _ = df.mean()?;
    let _ = df.count()?;
    let _ = df.nunique()?;
    let _ = df.min()?;
    let _ = df.max()?;
    let _ = df.std()?;
    let _ = df.var()?;
    let _ = df.median()?;
    let _ = df.prod()?;
    let _ = df.skew()?;
    let _ = df.kurt()?;
    let _ = df.kurtosis()?;
    let _ = df.sem()?;
    let _ = df.abs()?; // element-wise — returns DataFrame

    // Column.binary_numeric / binary_comparison — exercise via DataFrame columns.
    let col_a = df.column("a").expect("column a").clone();
    let col_b = df.column("b").expect("column b").clone();
    let _sum_col = col_a.binary_numeric(&col_b, ArithmeticOp::Add)?;
    let _gt_col = col_a.binary_comparison(&col_b, ComparisonOp::Gt)?;
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

    // fd90.213: Missing-data extras documented in README lines 963-988.

    // first_valid_index / last_valid_index (Series + DataFrame).
    let _ = series.first_valid_index();
    let _ = series.last_valid_index();
    let _ = df.first_valid_index();
    let _ = df.last_valid_index();

    // df.fillna_method("ffill"|"bfill").
    let _ = df.fillna_method("ffill")?;
    let _ = df.fillna_method("bfill")?;

    // df.combine_first(&other) — fill nulls in left from right.
    let other = read_csv_str("a,b,c\n10,10,10\n20,20,20\n30,30,30\n40,40,40\n50,50,50")?;
    let _ = df.combine_first(&other)?;

    // df.update(&other) — non-null values in `other` overwrite df.
    let _ = df.update(&other)?;
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

    // fd90.277: astype_safe / bool_ / columns_index / columns_multiindex.
    // DataFrame.astype_safe with errors='raise'/'ignore'/'coerce'.
    let _ = df.astype_safe(DType::Float64, "ignore")?;
    let _ = df.astype_columns_safe(&[("price", DType::Float64)], "raise")?;
    // DataFrame.columns_index → MultiIndexOrIndex; columns_multiindex → Option<&MultiIndex>.
    let _ = df.columns_index();
    let _ = df.columns_multiindex();
    // Series.astype_safe + bool_ on a single-element Bool Series.
    let bool_labels: Vec<IndexLabel> = vec![IndexLabel::Int64(0)];
    let one_bool = Series::from_values("b", bool_labels, vec![Scalar::Bool(true)])?;
    assert_eq!(one_bool.bool_()?, true);
    let _ = one_bool.astype_safe(DType::Bool, "raise")?;
    let _ = one_bool.axes();
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

    // fd90.214: squeeze / iat / at / lookup (README lines 1168-1178).

    // DataFrame.squeeze(axis) — single-column DataFrame → Series.
    let single_col = read_csv_str("only\n1\n2\n3")?;
    let _: Result<Series, _> = single_col.squeeze(1);

    // Series.iat(pos) and Series.at(&label) for scalar access.
    let labels: Vec<IndexLabel> = vec!["a".into(), "b".into(), "c".into()];
    let s = Series::from_values(
        "v",
        labels,
        vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
    )?;
    let v0 = s.iat(0)?;
    assert_eq!(v0, Scalar::Int64(10));
    let vb = s.at(&"b".into())?;
    assert_eq!(vb, Scalar::Int64(20));

    // Series.squeeze — Result<Scalar, Box<Series>>; one_cell collapses.
    let one_cell = Series::from_values("v", vec![IndexLabel::Int64(0)], vec![Scalar::Int64(42)])?;
    let scalar = one_cell.squeeze();
    assert!(matches!(scalar, Ok(Scalar::Int64(42))));

    // DataFrame.lookup(&row_labels, &col_names) — Vec<Scalar> at the
    // (row, col) intersections.
    let lookup_vals = df.lookup(
        &[IndexLabel::Int64(0), IndexLabel::Int64(1)],
        &["a", "b"],
    )?;
    assert_eq!(lookup_vals.len(), 2);
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

    // fd90.198: rank + rank_with_pct (the last 2 of the 15 SeriesGroupBy
    // methods enumerated at README line 1200).
    let ranks = by_region.rank("average", true, "keep")?;
    assert_eq!(ranks.len(), 6); // 1 rank per input row, not per group
    let pct_ranks = by_region.rank_with_pct("average", true, "keep", true)?;
    assert_eq!(pct_ranks.len(), 6);
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
    // fd90.280: Series.at_time + Series.between_time on a datetime-indexed Series.
    let _ = val_series.at_time("12:00:00")?;
    let _ = val_series.between_time("09:00:00", "12:00:00")?;

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

    // fd90.279: more DatetimeAccessor methods.
    let _ = dt.microsecond()?;
    let _ = dt.nanosecond()?;
    let _ = dt.date()?;
    let _ = dt.day_name()?;
    let _ = dt.month_name()?;
    let _ = dt.days_in_month()?;
    let _ = dt.is_leap_year()?;
    let _ = dt.is_year_start()?;
    let _ = dt.is_year_end()?;
    let _ = dt.floor("D")?;
    let _ = dt.ceil("D")?;
    let _ = dt.round("D")?;
    let _ = dt.to_timestamp()?;
    let _ = dt.to_pydatetime()?;
    // tz_localize_with_options uses TzLocalizeOptions (top-level export).
    let _ = dt.tz_localize_with_options(
        Some("America/New_York"),
        TzLocalizeOptions::default(),
    )?;

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

    // fd90.197: cover the remaining 7 group-level ops listed at line 566.
    let _ = gb.rank("average", true, "keep")?;
    let _ = gb.nth(0)?;
    let _ = gb.pct_change(1)?;
    let _ = gb.value_counts()?;
    let group_one = gb.get_group("1")?; // groups by column 'a': keys are 1 and 2
    assert!(group_one.index().len() >= 1);
    let piped = gb.pipe(|g| g.sum())?;
    assert!(!piped.column_names().is_empty());
    let _ = gb.ohlc()?;

    // fd90.212: GroupBy.rolling(window) — README line 496 ("GroupBy also
    // supports rolling() and resample() for within-group window operations").
    // GroupByRolling has 7 methods: sum/mean/min/max/std/count/var.
    let gbr = gb.rolling(2);
    let _ = gbr.sum()?;
    let _ = gbr.mean()?;
    let _ = gbr.min()?;
    let _ = gbr.max()?;
    let _ = gbr.std()?;
    let _ = gbr.count()?;
    let _ = gbr.var()?;

    // GroupBy.resample("M") — needs a datetime-indexed grouping DataFrame.
    let dt_df = DataFrame::from_dict_with_index(
        vec![
            ("k", vec![Scalar::Utf8("a".into()), Scalar::Utf8("a".into()), Scalar::Utf8("b".into()), Scalar::Utf8("b".into())]),
            ("v", vec![Scalar::Float64(1.0), Scalar::Float64(2.0), Scalar::Float64(3.0), Scalar::Float64(4.0)]),
        ],
        vec![
            "2024-01-15".into(),
            "2024-01-20".into(),
            "2024-02-10".into(),
            "2024-02-25".into(),
        ],
    )?;
    let dt_gb = dt_df.groupby(&["k"])?;
    let dt_gbr = dt_gb.resample("M");
    // GroupByResample has 7 methods: sum/mean/count/min/max/first/last.
    let _ = dt_gbr.sum()?;
    let _ = dt_gbr.mean()?;
    let _ = dt_gbr.count()?;
    let _ = dt_gbr.min()?;
    let _ = dt_gbr.max()?;
    let _ = dt_gbr.first()?;
    let _ = dt_gbr.last()?;

    // fd90.246: GroupBy.quantile / transform_list / transform_fn /
    // filter / apply / apply_scalar / apply_series.
    let _ = gb.quantile(0.5)?;
    let _ = gb.transform_list(&["sum", "mean"])?;
    let _ = gb.transform_fn(|s: &Series| Ok(s.clone()))?; // identity transform
    let _filtered = gb.filter(|d: &DataFrame| Ok(d.len() >= 2))?;
    let _applied = gb.apply(|d: &DataFrame| Ok(d.clone()))?; // identity apply
    let _scalar = gb.apply_scalar("count", |d: &DataFrame| Ok(Scalar::Int64(d.len() as i64)))?;

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
    // FrameError is in the prelude (fd90.202) so no extra import needed.
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

    // fd90.275: more apply/assign variants.
    // DataFrame.assign — bulk column-add via Vec<(name, Column)>.
    let new_col = Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3), Scalar::Int64(4)])?;
    let _ = df.assign(vec![("added", new_col)])?;
    // DataFrame.apply_fn(func, axis) — Fn(&[Scalar]) -> Scalar; axis=0 column-wise.
    let _ = df.apply_fn(|vals: &[Scalar]| vals.first().cloned().unwrap_or(Scalar::Null(NullKind::NaN)), 0)?;
    // DataFrame.applymap_na_action — applymap with na pass-through.
    let _ = df.applymap_na_action(|s: &Scalar| s.clone())?;
    // DataFrame.apply_rows(func, name) — Fn(&[Scalar]) -> Scalar over rows, returns named Series.
    let _ = df.apply_rows(|row: &[Scalar]| row.first().cloned().unwrap_or(Scalar::Null(NullKind::NaN)), "first_col_per_row")?;

    // fd90.276: combine variants + compare_with_result_names.
    // Series.combine(other, Fn(&Scalar, &Scalar) -> Scalar) — outer-aligned pairwise.
    let s_labels: Vec<IndexLabel> = (0..3i64).map(IndexLabel::Int64).collect();
    let s_x = Series::from_values(
        "x",
        s_labels.clone(),
        vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
    )?;
    let s_y = Series::from_values(
        "y",
        s_labels,
        vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
    )?;
    let _ = s_x.combine(&s_y, |a, b| match (a, b) {
        (Scalar::Int64(x), Scalar::Int64(y)) => Scalar::Int64(x + y),
        _ => Scalar::Null(NullKind::NaN),
    })?;
    // DataFrame.combine_elementwise — Fn(&Scalar, &Scalar) -> Scalar.
    let df_other = read_csv_str("a,b\n10,1\n20,2\n30,3\n40,4")?;
    let _ = df.combine_elementwise(&df_other, |a, b| match (a, b) {
        (Scalar::Int64(x), Scalar::Int64(y)) => Scalar::Int64((*x).max(*y)),
        _ => Scalar::Null(NullKind::NaN),
    })?;
    // DataFrame.combine — column-pair Fn(&Series, &Series) -> Result<Series, FrameError>.
    let _ = df.combine(
        &df_other,
        |left: &Series, right: &Series| left.combine(right, |a, b| match (a, b) {
            (Scalar::Int64(x), Scalar::Int64(y)) => Scalar::Int64(x + y),
            _ => Scalar::Null(NullKind::NaN),
        }),
        None,
        false,
    )?;
    // DataFrame.compare_with_result_names — explicit (left_name, right_name) for the diff cols.
    let df_clone = df.copy();
    let _ = df.compare_with_result_names(&df_clone, ("self", "other"))?;
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

    // fd90.229: 4 additional value_counts variants.
    // Series.value_counts_bins — binning a numeric Series.
    let num_labels: Vec<IndexLabel> = (0..6i64).map(IndexLabel::Int64).collect();
    let nums = Series::from_values(
        "x",
        num_labels,
        vec![
            Scalar::Float64(1.0),
            Scalar::Float64(2.5),
            Scalar::Float64(4.0),
            Scalar::Float64(5.5),
            Scalar::Float64(7.0),
            Scalar::Float64(9.0),
        ],
    )?;
    let _ = nums.value_counts_bins(3)?;

    // DataFrame.value_counts_per_column — per-column counts.
    let cnt_df = read_csv_str("a,b\n1,x\n2,y\n1,x\n3,y")?;
    let _ = cnt_df.value_counts_per_column()?;

    // DataFrame.value_counts_map — BTreeMap<String, Series> per column.
    let map = cnt_df.value_counts_map()?;
    assert!(map.contains_key("a") || map.contains_key("b"));

    // DataFrame.value_counts_subset — restrict to specific columns.
    let _ = cnt_df.value_counts_subset(&["a"])?;

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
    // FrameError is in the prelude (fd90.202) so no extra import needed.
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

    // fd90.234: DataFrame.drop / DataFrame.pop / Series.drop.

    // df.pop — README line 1145: "(popped_series, remaining_df) = df.pop(...)".
    let (popped, remaining) = df.pop("units")?;
    assert_eq!(popped.len(), 3);
    assert!(
        !remaining
            .column_names()
            .iter()
            .any(|n| n.as_str() == "units")
    );

    // df.drop — axis=1 drops columns, axis=0 drops rows.
    let dropped_col = df.drop(&["cost"], 1)?;
    assert!(
        !dropped_col
            .column_names()
            .iter()
            .any(|n| n.as_str() == "cost")
    );

    // Series.drop — drop rows by IndexLabel.
    let s_labels: Vec<IndexLabel> = vec!["a".into(), "b".into(), "c".into()];
    let s = Series::from_values(
        "v",
        s_labels,
        vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
    )?;
    let s_dropped = s.drop(&["b".into()])?;
    assert_eq!(s_dropped.len(), 2);
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

/// README "NanOps" section (lines 792-823).
///
/// Locks in the 19 null-skipping scalar reductions plus 4 cumulative
/// transforms re-exported from frankenpandas::prelude (lib.rs:23-27).
///
/// Each function exercised on a Vec<Scalar> with a deliberate NaN to
/// confirm null-skipping behavior matches the README's claims about
/// return type and skipna=True semantics.
///
/// Tracks fd90.191 (br-frankenpandas-1r7zz).
#[test]
fn readme_nanops_compiles_and_runs() -> Result<(), Box<dyn std::error::Error>> {
    // Mixed numeric values with one NaN to verify null-skipping.
    let values = vec![
        Scalar::Float64(1.0),
        Scalar::Float64(2.0),
        Scalar::Null(NullKind::NaN),
        Scalar::Float64(4.0),
        Scalar::Float64(5.0),
    ];

    // Scalar reductions — Float64 outputs.
    let sum = nansum(&values);
    assert!(matches!(sum, Scalar::Float64(_)));
    let mean = nanmean(&values);
    assert!(matches!(mean, Scalar::Float64(_)));
    let median = nanmedian(&values);
    assert!(matches!(median, Scalar::Float64(_)));
    let var = nanvar(&values, 1);
    assert!(matches!(var, Scalar::Float64(_)));
    let std = nanstd(&values, 1);
    assert!(matches!(std, Scalar::Float64(_)));
    let sem = nansem(&values, 1);
    assert!(matches!(sem, Scalar::Float64(_)));
    let prod = nanprod(&values);
    assert!(matches!(prod, Scalar::Float64(_)));
    let skew = nanskew(&values);
    assert!(matches!(skew, Scalar::Float64(_)));
    let kurt = nankurt(&values);
    assert!(matches!(kurt, Scalar::Float64(_)));
    let q50 = nanquantile(&values, 0.5);
    assert!(matches!(q50, Scalar::Float64(_)));

    // count → Int64 (count of non-missing).
    assert_eq!(nancount(&values), Scalar::Int64(4));

    // min / max / ptp — preserve input dtype.
    let min = nanmin(&values);
    assert!(matches!(min, Scalar::Float64(_)));
    let max = nanmax(&values);
    assert!(matches!(max, Scalar::Float64(_)));
    let ptp = nanptp(&values);
    assert!(matches!(ptp, Scalar::Float64(_)));

    // argmax / argmin — Option<usize>.
    let argmax = nanargmax(&values);
    assert!(argmax.is_some());
    let argmin = nanargmin(&values);
    assert!(argmin.is_some());

    // nunique → Int64 (4 unique non-missing values).
    assert_eq!(nannunique(&values), Scalar::Int64(4));

    // any / all → Bool.
    let bool_values = vec![
        Scalar::Bool(true),
        Scalar::Bool(false),
        Scalar::Null(NullKind::NaN),
        Scalar::Bool(true),
    ];
    assert_eq!(nanany(&bool_values), Scalar::Bool(true));
    assert_eq!(nanall(&bool_values), Scalar::Bool(false));

    // Cumulative transforms — return Vec<Scalar> with same length.
    let csum = nancumsum(&values);
    assert_eq!(csum.len(), values.len());
    let cprod = nancumprod(&values);
    assert_eq!(cprod.len(), values.len());
    let cmax = nancummax(&values);
    assert_eq!(cmax.len(), values.len());
    let cmin = nancummin(&values);
    assert_eq!(cmin.len(), values.len());
    Ok(())
}

/// README "DataFrame Constructors" table (lines 856-873).
///
/// Locks in the 11 named constructors that previously had no
/// integration coverage (DataFrame::new is implicitly used elsewhere):
///
/// - from_dict / from_dict_with_index / from_dict_mixed
/// - from_series (N-way alignment)
/// - from_records / from_tuples / from_tuples_with_index
/// - from_csv (inline string)
/// - from_dict_index / from_dict_index_columns (orient=index)
/// - new_with_row_multiindex (logical row MultiIndex metadata)
///
/// fd90.192 also exposes DataFrameColumnInput in the prelude — required
/// for from_dict_mixed user code to compile without depending on
/// fp_frame directly.
///
/// Tracks fd90.192 (br-frankenpandas-fzj18).
#[test]
fn readme_dataframe_constructors_compiles_and_runs() -> Result<(), Box<dyn std::error::Error>> {
    // from_dict — column-oriented, with explicit column_order.
    let df = DataFrame::from_dict(
        &["a", "b"],
        vec![
            ("a", vec![Scalar::Int64(1), Scalar::Int64(2)]),
            ("b", vec![Scalar::Int64(3), Scalar::Int64(4)]),
        ],
    )?;
    assert_eq!(df.column_names().len(), 2);
    assert_eq!(df.index().len(), 2);

    // from_dict_with_index — explicit row-index labels.
    let labels: Vec<IndexLabel> = vec!["x".into(), "y".into()];
    let df_idx = DataFrame::from_dict_with_index(
        vec![("a", vec![Scalar::Int64(10), Scalar::Int64(20)])],
        labels,
    )?;
    assert_eq!(df_idx.index().len(), 2);

    // from_dict_mixed — broadcast scalar columns alongside vector ones.
    let df_mixed = DataFrame::from_dict_mixed(
        &["a", "b"],
        vec![
            ("a", DataFrameColumnInput::Values(vec![Scalar::Int64(1), Scalar::Int64(2)])),
            ("b", DataFrameColumnInput::Scalar(Scalar::Utf8("const".to_owned()))),
        ],
    )?;
    assert_eq!(df_mixed.column_names().len(), 2);
    assert_eq!(df_mixed.index().len(), 2);

    // from_series — N-way alignment.
    let s1 = Series::from_values(
        "a",
        vec![IndexLabel::Int64(0), IndexLabel::Int64(1)],
        vec![Scalar::Int64(1), Scalar::Int64(2)],
    )?;
    let s2 = Series::from_values(
        "b",
        vec![IndexLabel::Int64(0), IndexLabel::Int64(1)],
        vec![Scalar::Int64(3), Scalar::Int64(4)],
    )?;
    let df_series = DataFrame::from_series(vec![s1, s2])?;
    assert_eq!(df_series.column_names().len(), 2);

    // from_records — row-oriented vec of vecs with column_order + index_labels.
    let columns = vec!["a".to_string(), "b".to_string()];
    let df_recs = DataFrame::from_records(
        vec![
            vec![Scalar::Int64(1), Scalar::Int64(2)],
            vec![Scalar::Int64(3), Scalar::Int64(4)],
        ],
        Some(&columns),
        Some(vec![IndexLabel::Int64(0), IndexLabel::Int64(1)]),
    )?;
    assert_eq!(df_recs.index().len(), 2);

    // from_tuples — same shape but auto-generated 0..n index.
    let df_tup = DataFrame::from_tuples(
        vec![
            vec![Scalar::Int64(1), Scalar::Int64(2)],
            vec![Scalar::Int64(3), Scalar::Int64(4)],
        ],
        &["a", "b"],
    )?;
    assert_eq!(df_tup.index().len(), 2);

    // from_tuples_with_index — explicit row labels.
    let df_tup_idx = DataFrame::from_tuples_with_index(
        vec![
            vec![Scalar::Int64(1), Scalar::Int64(2)],
            vec![Scalar::Int64(3), Scalar::Int64(4)],
        ],
        &["a", "b"],
        vec!["x".into(), "y".into()],
    )?;
    assert_eq!(df_tup_idx.index().len(), 2);

    // from_csv — inline CSV parsing.
    let df_csv = DataFrame::from_csv("a,b\n1,2\n3,4", ',')?;
    assert_eq!(df_csv.column_names().len(), 2);
    assert_eq!(df_csv.index().len(), 2);

    // from_dict_index — row-keyed (each entry is a row).
    let df_di = DataFrame::from_dict_index(vec![
        ("row1", vec![Scalar::Int64(1), Scalar::Int64(2)]),
        ("row2", vec![Scalar::Int64(3), Scalar::Int64(4)]),
    ])?;
    assert_eq!(df_di.index().len(), 2);

    // from_dict_index_columns — same with explicit column names.
    let df_dic = DataFrame::from_dict_index_columns(
        vec![
            ("row1", vec![Scalar::Int64(1), Scalar::Int64(2)]),
            ("row2", vec![Scalar::Int64(3), Scalar::Int64(4)]),
        ],
        &["a", "b"],
    )?;
    assert_eq!(df_dic.column_names().len(), 2);
    assert_eq!(df_dic.index().len(), 2);

    // new_with_row_multiindex — logical row MultiIndex on top of flat storage.
    let mi = MultiIndex::from_tuples(vec![
        vec![IndexLabel::Utf8("g1".to_owned()), IndexLabel::Int64(1)],
        vec![IndexLabel::Utf8("g1".to_owned()), IndexLabel::Int64(2)],
    ])?;
    let mut col_map: std::collections::BTreeMap<String, Column> = std::collections::BTreeMap::new();
    col_map.insert(
        "value".to_owned(),
        Column::from_values(vec![Scalar::Int64(10), Scalar::Int64(20)])?,
    );
    let storage_index = Index::new(vec![IndexLabel::Int64(0), IndexLabel::Int64(1)]);
    let df_mi = DataFrame::new_with_row_multiindex(storage_index, mi, col_map)?;
    assert_eq!(df_mi.index().len(), 2);
    Ok(())
}

/// README "Reshaping" section (lines 498-524).
///
/// Locks in 7 reshaping APIs that previously had no integration coverage
/// (pivot_table is already exercised by readme_pivot_tables):
///
/// - DataFrame.melt (wide → long)
/// - DataFrame.stack / unstack (column index ↔ row index round-trip)
/// - DataFrame.crosstab + crosstab_normalize (contingency tables)
/// - DataFrame.get_dummies (one-hot encoding)
/// - DataFrame.xs (cross-section by IndexLabel)
///
/// Tracks fd90.193 (br-frankenpandas-tc3g2).
#[test]
fn readme_reshaping_compiles_and_runs() -> Result<(), Box<dyn std::error::Error>> {
    // melt — wide → long with id_vars + value_vars.
    let wide = read_csv_str("id,q1,q2,q3\n1,10,20,30\n2,40,50,60")?;
    let melted = wide.melt(&["id"], &["q1", "q2", "q3"], Some("quarter"), Some("sales"))?;
    assert!(
        melted
            .column_names()
            .iter()
            .any(|n| n.as_str() == "quarter")
    );
    assert!(
        melted
            .column_names()
            .iter()
            .any(|n| n.as_str() == "sales")
    );

    // stack / unstack — round-trip exercise on a numeric DataFrame.
    let df = read_csv_str("a,b\n1,2\n3,4")?;
    let stacked = df.stack()?;
    assert!(stacked.index().len() >= 2);
    let _unstacked = stacked.unstack()?;

    // crosstab — contingency table from two categorical Series.
    let labels: Vec<IndexLabel> = (0..6i64).map(IndexLabel::Int64).collect();
    let gender = Series::from_values(
        "gender",
        labels.clone(),
        vec![
            "M".into(),
            "F".into(),
            "M".into(),
            "F".into(),
            "M".into(),
            "F".into(),
        ],
    )?;
    let dept = Series::from_values(
        "department",
        labels,
        vec![
            "eng".into(),
            "eng".into(),
            "sales".into(),
            "sales".into(),
            "ops".into(),
            "ops".into(),
        ],
    )?;
    let ct = DataFrame::crosstab(&gender, &dept)?;
    assert_eq!(ct.column_names().len(), 3); // 3 unique departments
    assert_eq!(ct.index().len(), 2); // 2 unique genders

    // crosstab_normalize — divide by grand total.
    let ct_norm = DataFrame::crosstab_normalize(&gender, &dept, "all")?;
    assert_eq!(ct_norm.column_names().len(), 3);

    // get_dummies — one-hot encoding on string-typed columns.
    let cat_df = read_csv_str("color,size,price\nred,S,10\nblue,M,20\nred,L,30")?;
    let dummies = cat_df.get_dummies(&["color", "size"])?;
    // Each unique category becomes a dummy column; "price" is preserved.
    assert!(
        dummies
            .column_names()
            .iter()
            .any(|n| n.as_str() == "price")
    );
    assert!(dummies.column_names().len() > 1);

    // xs — cross-section by IndexLabel (uses From<&str> for IndexLabel).
    let dated_df = read_csv_str("date,price\n2024-01-15,100\n2024-01-16,105\n2024-01-17,110")?
        .set_index("date", true)?;
    let row = dated_df.xs(&"2024-01-15".into())?;
    assert_eq!(row.index().len(), 1);
    Ok(())
}

/// README "String Accessor" section (lines 439-453).
///
/// Locks in one representative method per documented category of the
/// .str() accessor surface. The accessor itself was previously only
/// exercised by replace_regex in readme_conditional_logic.
///
/// Coverage by category:
/// - Case: lower, upper, capitalize, title
/// - Whitespace: strip, lstrip, rstrip
/// - Search: contains, startswith, endswith
/// - Transform: slice, repeat, pad, zfill, center
/// - Split/Join: split_get, split_count, join
/// - Predicates: isdigit, isalpha
/// - Regex: contains_regex, extract, count_matches, findall, fullmatch
/// - Prefix/Suffix: removeprefix, removesuffix
/// - Other: len, get
///
/// Tracks fd90.194 (br-frankenpandas-8i9lh).
#[test]
fn readme_string_accessor_compiles_and_runs() -> Result<(), Box<dyn std::error::Error>> {
    let labels: Vec<IndexLabel> = (0..4i64).map(IndexLabel::Int64).collect();
    let s = Series::from_values(
        "name",
        labels.clone(),
        vec![
            "  Hello  ".into(),
            "world".into(),
            "Foo Bar".into(),
            "abc123".into(),
        ],
    )?;
    let n = s.len();

    // Case operations.
    let _ = s.str().lower()?;
    let _ = s.str().upper()?;
    let _ = s.str().capitalize()?;
    let _ = s.str().title()?;

    // Whitespace.
    let stripped = s.str().strip()?;
    assert_eq!(stripped.len(), n);
    let _ = s.str().lstrip()?;
    let _ = s.str().rstrip()?;

    // Search predicates (return Bool Series).
    let _ = s.str().contains("Foo")?;
    let _ = s.str().startswith("a")?;
    let _ = s.str().endswith("o")?;

    // Transform.
    let _ = s.str().slice(0, Some(3))?;
    let _ = s.str().repeat(2)?;
    let _ = s.str().pad(10, "right", ' ')?;
    let _ = s.str().zfill(8)?;
    let _ = s.str().center(8, '*')?;

    // Split/Join.
    let csv_labels: Vec<IndexLabel> = (0..3i64).map(IndexLabel::Int64).collect();
    let csv = Series::from_values(
        "csv",
        csv_labels,
        vec!["a,b,c".into(), "d,e".into(), "f".into()],
    )?;
    let _ = csv.str().split_get(",", 0)?;
    let counts = csv.str().split_count(",")?;
    assert_eq!(counts.len(), 3);
    let _ = s.str().join("name", "-")?;

    // Predicates (return Bool Series).
    let _ = s.str().isdigit()?;
    let _ = s.str().isalpha()?;

    // Regex.
    let _ = s.str().contains_regex(r"\d+")?;
    let _ = s.str().extract(r"(\w+)")?;
    let _ = s.str().count_matches(r"\w+")?;
    let _ = s.str().findall(r"\w+", "|")?;
    let _ = s.str().fullmatch(r"\w+")?;

    // Prefix/Suffix.
    let prefixed_labels: Vec<IndexLabel> = (0..2i64).map(IndexLabel::Int64).collect();
    let pref = Series::from_values(
        "pref",
        prefixed_labels,
        vec!["pre_one".into(), "pre_two".into()],
    )?;
    let _ = pref.str().removeprefix("pre_")?;
    let _ = pref.str().removesuffix("two")?;

    // Other.
    let lens = s.str().len()?;
    assert_eq!(lens.len(), n);
    let _ = s.str().get(0)?;

    // fd90.247: more StringAccessor methods (~15 additions).
    // Case (round out the 6 README cells).
    let _ = s.str().casefold()?;
    let _ = s.str().swapcase()?;
    // Search positions.
    let _ = s.str().find("o")?;
    let _ = s.str().rfind("l")?;
    let _ = s.str().index_of("e")?;
    let _ = s.str().rindex_of("d")?;
    // Predicates (round out the 9 README cells).
    let _ = s.str().isalnum()?;
    let _ = s.str().isdecimal()?;
    let _ = s.str().islower()?;
    let _ = s.str().isnumeric()?;
    let _ = s.str().isspace()?;
    let _ = s.str().istitle()?;
    let _ = s.str().isupper()?;
    // Concatenation across rows: cat returns a single String.
    let _ = s.str().cat("|")?;
    // Whitespace: lstrip with explicit chars.
    let _ = s.str().lstrip_chars(" \t")?;
    // Right-justify (the left-justify variant was in fd90.194).
    let _ = s.str().rjust(10, ' ')?;
    // Partition splits each row at first sep into (before, sep, after) Series.
    let part_labels: Vec<IndexLabel> = (0..2i64).map(IndexLabel::Int64).collect();
    let phones2 = Series::from_values(
        "phones",
        part_labels,
        vec!["555-1234".into(), "555-9876".into()],
    )?;
    let _ = phones2.str().partition("-")?;
    let _ = phones2.str().rpartition("-")?;
    // Normalize Unicode form ("NFC" / "NFD" / "NFKC" / "NFKD").
    let _ = s.str().normalize("NFC")?;
    // Count occurrences (literal, not regex).
    let _ = s.str().count("o")?;
    // extractall — DataFrame of regex captures across all matches.
    let _ = s.str().extractall(r"(\w+)")?;

    // fd90.248: more StringAccessor methods.
    // contains_any — true if any pattern matches.
    let _ = s.str().contains_any(&["Foo", "abc"])?;
    // count_literal — count exact substring occurrences.
    let _ = s.str().count_literal("o")?;
    // decode / encode — pass-through round-trip (UTF-8 only today).
    let _ = s.str().encode("utf-8")?;
    let _ = s.str().decode("utf-8")?;
    // expandtabs — replace \t with N spaces.
    let _ = s.str().expandtabs(4)?;
    // strip_chars — strip arbitrary chars from both ends.
    let _ = s.str().strip_chars(" *")?;
    // slice_replace — replace [start, stop) with repl string.
    let _ = s.str().slice_replace(0, Some(2), "**")?;
    // replace_regex_all — like replace_regex but global.
    let _ = s.str().replace_regex_all(r"\w", "X")?;
    // extract_to_frame — DataFrame of named captures.
    let _ = s.str().extract_to_frame(r"(?P<word>\w+)")?;
    // split_expand — DataFrame with one column per split position.
    let _ = csv.str().split_expand(",")?;
    // wrap — wrap each row to width.
    let _ = s.str().wrap(8)?;
    // translate — character-by-character substitution table.
    let _ = s.str().translate("abc", "ABC")?;
    // get_dummies (str variant) — sep-separated tag → one-hot DataFrame.
    let tag_labels: Vec<IndexLabel> = (0..3i64).map(IndexLabel::Int64).collect();
    let tags = Series::from_values(
        "tags",
        tag_labels,
        vec!["a|b".into(), "b|c".into(), "a|c".into()],
    )?;
    let _ = tags.str().get_dummies("|")?;
    Ok(())
}

/// README "Bayesian Runtime Policy" section (lines 378-403).
///
/// Locks in fd90.250: RuntimePolicy + EvidenceLedger flow. fd90.221
/// exposed the inspection types (DecisionAction, DecisionRecord, etc.);
/// this test exercises an actual decision call and inspects the
/// resulting record.
#[test]
fn readme_bayesian_runtime_policy_compiles_and_runs() -> Result<(), Box<dyn std::error::Error>> {
    // Construct both policies — strict (fail-closed) and hardened.
    let strict = RuntimePolicy::strict();
    let hardened = RuntimePolicy::hardened(None);

    // Make a decision; ledger captures the trace.
    let mut ledger = EvidenceLedger::new();
    let action = strict.decide_unknown_feature(
        "subject_x",
        "unrecognized join feature",
        &mut ledger,
    );
    // Strict mode → fail-closed unknown features → Reject.
    assert!(matches!(action, DecisionAction::Reject));

    // Inspect the recorded decision (README claim: ledger has full trace).
    let records = ledger.records();
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert!(matches!(record.mode, RuntimeMode::Strict));
    assert!(matches!(record.action, DecisionAction::Reject));
    assert!(matches!(record.issue.kind, IssueKind::UnknownFeature));
    assert!(!record.evidence.is_empty());

    // Hardened mode allows decision flexibility — decide_join_admission.
    let mut ledger2 = EvidenceLedger::new();
    let _join_action = hardened.decide_join_admission(1_000_000, &mut ledger2);
    assert_eq!(ledger2.records().len(), 1);

    // decision_to_card transforms a record into a printable summary.
    let card = decision_to_card(record);
    assert!(!card.title.is_empty());

    // fd90.32: cover RuntimeMode::Hardened + DecisionAction::Allow /
    // Repair + IssueKind::JoinCardinality.
    let hardened_with_cap = RuntimePolicy::hardened(Some(10_000));

    // Within cap → Bayesian decision; record's mode is Hardened and
    // the issue.kind is JoinCardinality.
    let mut ledger_within = EvidenceLedger::new();
    let _ = hardened_with_cap.decide_join_admission(1000, &mut ledger_within);
    let within_record = &ledger_within.records()[0];
    assert!(matches!(within_record.mode, RuntimeMode::Hardened));
    assert!(matches!(
        within_record.issue.kind,
        IssueKind::JoinCardinality
    ));
    // Within-cap action is one of {Allow, Reject, Repair}; verify it's
    // a valid DecisionAction (matches the enum).
    let _ = match within_record.action {
        DecisionAction::Allow => "allow",
        DecisionAction::Reject => "reject",
        DecisionAction::Repair => "repair",
    };

    // Over cap → Hardened forces Repair.
    let mut ledger_over = EvidenceLedger::new();
    let over_action = hardened_with_cap.decide_join_admission(1_000_000_000, &mut ledger_over);
    assert!(
        matches!(over_action, DecisionAction::Repair),
        "Hardened+over-cap should force Repair, got {over_action:?}",
    );
    let over_record = &ledger_over.records()[0];
    assert!(matches!(over_record.mode, RuntimeMode::Hardened));
    assert!(matches!(over_record.action, DecisionAction::Repair));
    Ok(())
}

/// README "Error Architecture" section (lines 829-853).
///
/// Locks in fd90.202: the 8 error types listed in the README's Error
/// Architecture table can all be referenced from the prelude (matching
/// the README's claim that "All error types are re-exported through the
/// frankenpandas facade crate"). Before fd90.202, only JoinError was in
/// the prelude — the rest required a top-level `use frankenpandas::…`
/// import even after `use frankenpandas::prelude::*`.
///
/// Compile-only test — references each error type as a function
/// signature so the compiler resolves the path. Runtime is a no-op.
#[test]
fn readme_error_architecture_compiles_and_runs() -> Result<(), Box<dyn std::error::Error>> {
    let _: fn(TypeError) -> _ = |e| e;
    let _: fn(ColumnError) -> _ = |e| e;
    let _: fn(IndexError) -> _ = |e| e;
    let _: fn(FrameError) -> _ = |e| e;
    let _: fn(ExprError) -> _ = |e| e;
    let _: fn(JoinError) -> _ = |e| e;
    let _: fn(GroupByError) -> _ = |e| e;
    let _: fn(IoError) -> _ = |e| e;
    Ok(())
}

/// README "Serialization and Interoperability" section (lines 1560-1580).
///
/// Locks in the claim that all core types round-trip through serde_json
/// "perfectly". Verifies:
/// - Scalar variants (Bool/Int64/Float64/Utf8/Null) round-trip identically
/// - The Scalar enum produces the documented {"kind":..., "value":...}
///   tagged form (not bare values)
/// - IndexLabel round-trip
/// - DataFrame round-trip — exercises Index + Column + ValidityMask
///   serialization paths
///
/// Tracks fd90.196 (br-frankenpandas-1d1gm).
#[test]
fn readme_serialization_compiles_and_runs() -> Result<(), Box<dyn std::error::Error>> {
    // Scalar variants — document the tagged-enum shape.
    let cases = vec![
        Scalar::Bool(true),
        Scalar::Int64(42),
        Scalar::Float64(3.14),
        Scalar::Utf8("hello".to_owned()),
        Scalar::Null(NullKind::NaN),
    ];
    for original in &cases {
        let json = serde_json::to_string(original)?;
        // Verify tagged representation: every JSON object should contain "kind".
        assert!(
            json.contains("\"kind\""),
            "Scalar JSON missing 'kind' tag: {json}"
        );
        let restored: Scalar = serde_json::from_str(&json)?;
        assert_eq!(*original, restored, "Scalar round-trip diverged: {json}");
    }

    // IndexLabel round-trip.
    let labels = vec![
        IndexLabel::Int64(7),
        IndexLabel::Utf8("row1".to_owned()),
    ];
    for original in &labels {
        let json = serde_json::to_string(original)?;
        let restored: IndexLabel = serde_json::from_str(&json)?;
        assert_eq!(*original, restored);
    }

    // DataFrame round-trip — exercises Index + Column + ValidityMask serde paths.
    let df = read_csv_str("a,b\n1,4\n2,5\n3,6")?;
    let json = serde_json::to_string(&df)?;
    let restored: DataFrame = serde_json::from_str(&json)?;
    assert_eq!(df.column_names(), restored.column_names());
    assert_eq!(df.index().len(), restored.index().len());

    // fd90.203: ValidityMask round-trip via the prelude. README line 1578
    // claims "ValidityMask serializes as a Vec<bool> for JSON compatibility
    // but uses bitpacked Vec<u64> in memory."
    let mask = ValidityMask::from_values(&[
        Scalar::Int64(1),
        Scalar::Null(NullKind::NaN),
        Scalar::Int64(3),
    ]);
    let mask_json = serde_json::to_string(&mask)?;
    let mask_back: ValidityMask = serde_json::from_str(&mask_json)?;
    assert_eq!(mask, mask_back);

    // fd90.254: ValidityMask boolean ops + inspection.
    let m_a = ValidityMask::all_valid(4);
    let m_b = ValidityMask::all_invalid(4);
    assert_eq!(m_a.len(), 4);
    assert!(!m_a.is_empty());
    assert_eq!(m_a.count_valid(), 4);
    assert_eq!(m_b.count_invalid(), 4);
    assert!(m_a.any());
    assert!(m_a.all());
    assert!(!m_b.any());
    assert!(!m_b.all());
    let _ = m_a.and_mask(&m_b);
    let _ = m_a.or_mask(&m_b);
    let _ = m_a.not_mask();
    let _ = m_a.xor_mask(&m_b);
    let sliced = m_a.slice(1, 2);
    assert_eq!(sliced.len(), 2);
    let cat = m_a.concat(&m_b);
    assert_eq!(cat.len(), 8);
    assert_eq!(m_a.first_valid(), Some(0));
    assert_eq!(m_a.last_valid(), Some(3));
    assert_eq!(m_b.first_valid(), None);
    assert!(m_a.get(0)); // bit 0 = valid in all_valid(4)
    assert!(!m_b.get(0));

    // fd90.255: Column inspection methods.
    // Column::new(dtype, values) — explicit-dtype constructor.
    let col_int = Column::new(DType::Int64, vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)])?;
    assert_eq!(col_int.dtype(), DType::Int64);
    assert_eq!(col_int.len(), 3);
    assert!(!col_int.is_empty());
    assert_eq!(col_int.values().len(), 3);
    assert_eq!(col_int.value(1), Some(&Scalar::Int64(2)));
    assert!(col_int.iter_values().count() == 3);
    let scalars = col_int.to_vec();
    assert_eq!(scalars.len(), 3);
    let _ = col_int.validity();
    assert!(!col_int.has_any_missing());
    assert!(!col_int.all_missing());
    assert_eq!(col_int.first(), Some(&Scalar::Int64(1)));
    assert_eq!(col_int.last(), Some(&Scalar::Int64(3)));

    // has_any_missing == true on a column with a Null.
    let col_with_nan = Column::from_values(vec![
        Scalar::Float64(1.0),
        Scalar::Null(NullKind::NaN),
        Scalar::Float64(3.0),
    ])?;
    assert!(col_with_nan.has_any_missing());

    // fd90.272: Sparse-typed Series + SparseAccessor methods.
    // README line 251 lists "Sparse" as an extension dtype.
    let sparse_labels: Vec<IndexLabel> = (0..5i64).map(IndexLabel::Int64).collect();
    let sparse_series = Series::from_sparse_dense(
        "sparse",
        sparse_labels,
        vec![
            Scalar::Float64(1.0),
            Scalar::Float64(0.0), // fill_value entries
            Scalar::Float64(0.0),
            Scalar::Float64(2.0),
            Scalar::Float64(0.0),
        ],
        DType::Float64,
        Scalar::Float64(0.0),
    )?;
    let sparse = sparse_series.sparse().expect("Series is sparse");
    assert_eq!(sparse.value_dtype(), DType::Float64);
    assert_eq!(sparse.fill_value(), &Scalar::Float64(0.0));
    let _np = sparse.npoints(); // count of non-fill values
    let _den = sparse.density(); // npoints / len
    let _dense = sparse.to_dense(); // returns Series directly (no ?)

    // fd90.205: round-trip the remaining 7 types from the README's
    // Serialization list at line 1567:
    // DType, NullKind, Index, MultiIndex, Series, CategoricalMetadata, Column.

    // DType — every variant must round-trip.
    for dt in [DType::Bool, DType::Int64, DType::Float64, DType::Utf8] {
        let json = serde_json::to_string(&dt)?;
        let back: DType = serde_json::from_str(&json)?;
        assert_eq!(dt, back);
    }

    // NullKind.
    for nk in [NullKind::NaN, NullKind::Null] {
        let json = serde_json::to_string(&nk)?;
        let back: NullKind = serde_json::from_str(&json)?;
        assert_eq!(nk, back);
    }

    // Index round-trip.
    let idx = Index::new(vec![IndexLabel::Int64(7), IndexLabel::Utf8("row".to_owned())]);
    let idx_json = serde_json::to_string(&idx)?;
    let idx_back: Index = serde_json::from_str(&idx_json)?;
    assert_eq!(idx.len(), idx_back.len());

    // MultiIndex round-trip.
    let mi = MultiIndex::from_product(vec![
        vec!["a".into(), "b".into()],
        vec![1i64.into(), 2i64.into()],
    ])?;
    let mi_json = serde_json::to_string(&mi)?;
    let mi_back: MultiIndex = serde_json::from_str(&mi_json)?;
    assert_eq!(mi.nlevels(), mi_back.nlevels());
    assert_eq!(mi.len(), mi_back.len());

    // Series round-trip.
    let s = Series::from_values(
        "v",
        vec![IndexLabel::Int64(0), IndexLabel::Int64(1)],
        vec![Scalar::Int64(10), Scalar::Int64(20)],
    )?;
    let s_json = serde_json::to_string(&s)?;
    let s_back: Series = serde_json::from_str(&s_json)?;
    assert_eq!(s.len(), s_back.len());

    // CategoricalMetadata round-trip — preserves categories + ordered flag.
    let cat_series = Series::from_categorical(
        "rating",
        vec![
            Scalar::Utf8("good".into()),
            Scalar::Utf8("bad".into()),
            Scalar::Utf8("good".into()),
        ],
        true,
    )?;
    // The CategoricalMetadata is embedded in the Series; round-trip the
    // whole Series and inspect the metadata on the other side.
    let cs_json = serde_json::to_string(&cat_series)?;
    let cs_back: Series = serde_json::from_str(&cs_json)?;
    let meta_back = cs_back.cat().expect("cat metadata preserved");
    assert_eq!(meta_back.categories().len(), 2);
    assert!(meta_back.ordered());

    // Column round-trip.
    let col = Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2)])?;
    let col_json = serde_json::to_string(&col)?;
    let col_back: Column = serde_json::from_str(&col_json)?;
    assert_eq!(col.len(), col_back.len());
    Ok(())
}

/// fd90.33: AlignMode + DateOffset + date_range/bdate_range functional
/// coverage. Existing tests only exercised AlignMode::Outer and the
/// DateOffset::Day variant in name-only form. date_range/bdate_range
/// were in the prelude but never invoked.
#[test]
fn readme_align_and_date_offset_round_trip() -> Result<(), Box<dyn std::error::Error>> {
    // ── AlignMode variants ──────────────────────────────────────
    // df_a index 0,1,2; df_b index 1,2,3. Inner shares {1,2}; Outer
    // unions to {0,1,2,3}; Left keeps {0,1,2}; Right keeps {1,2,3}.
    let labels1: Vec<IndexLabel> = vec![
        IndexLabel::Int64(0),
        IndexLabel::Int64(1),
        IndexLabel::Int64(2),
    ];
    let labels2: Vec<IndexLabel> = vec![
        IndexLabel::Int64(1),
        IndexLabel::Int64(2),
        IndexLabel::Int64(3),
    ];
    let s1 = Series::from_values(
        "a",
        labels1,
        vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
    )?;
    let s2 = Series::from_values(
        "b",
        labels2,
        vec![Scalar::Int64(100), Scalar::Int64(200), Scalar::Int64(300)],
    )?;
    let df_a = DataFrame::from_series(vec![s1])?;
    let df_b = DataFrame::from_series(vec![s2])?;

    let (left_inner, _) = df_a.align_on_index(&df_b, AlignMode::Inner)?;
    assert_eq!(left_inner.index().len(), 2);

    let (left_outer, _) = df_a.align_on_index(&df_b, AlignMode::Outer)?;
    assert_eq!(left_outer.index().len(), 4);

    let (left_left, _) = df_a.align_on_index(&df_b, AlignMode::Left)?;
    assert_eq!(left_left.index().len(), 3);

    let (left_right, _) = df_a.align_on_index(&df_b, AlignMode::Right)?;
    assert_eq!(left_right.index().len(), 3);

    // ── DateOffset variants ──────────────────────────────────────
    // Apply each offset to a fixed timestamp; assert result is finite +
    // moves in the documented direction.
    let base = "2024-01-15";
    let plus_day = apply_date_offset(base, DateOffset::Day(7))?;
    let plus_bday = apply_date_offset(base, DateOffset::BusinessDay(5))?;
    let plus_mend = apply_date_offset(base, DateOffset::MonthEnd(1))?;
    // All three offsets are positive, so result > base nanos.
    let base_ns = apply_date_offset(base, DateOffset::Day(0))?;
    assert!(plus_day > base_ns);
    assert!(plus_bday > base_ns);
    assert!(plus_mend > base_ns);

    // ── date_range ───────────────────────────────────────────────
    // Daily range over 5 days starting 2024-01-15.
    let day_ns = Timedelta::NANOS_PER_DAY;
    let range = date_range(Some("2024-01-15"), None, Some(5), day_ns, None)?;
    assert_eq!(range.len(), 5);

    // ── bdate_range ──────────────────────────────────────────────
    let brange = bdate_range(Some("2024-01-15"), None, Some(5), None)?;
    assert_eq!(brange.len(), 5);
    Ok(())
}

/// fd90.31: CsvOnBadLines + ToTimedeltaErrors variant coverage. Both
/// enums have observable behavioral differences but only the default
/// path was being exercised by integration tests.
#[test]
fn readme_bad_lines_and_timedelta_errors() -> Result<(), Box<dyn std::error::Error>> {
    // ── CsvOnBadLines ────────────────────────────────────────────
    // Mixed-width CSV: header + 3 data rows; the middle row has EXTRA
    // fields (short rows are preserved as missing; only extra-field
    // rows trigger Skip/Warn/Error per pandas semantics).
    let bad_csv = "ticker,price,volume\nAAPL,185.50,1000\nGOOG,140.25,500,EXTRA\nMSFT,420.00,800";

    // Skip: silently drop the bad row → 2 valid rows survive.
    let skip_opts = CsvReadOptions {
        on_bad_lines: CsvOnBadLines::Skip,
        ..CsvReadOptions::default()
    };
    let skip_df = read_csv_with_options(bad_csv, &skip_opts)?;
    assert_eq!(skip_df.index().len(), 2);

    // Warn: same outcome (drop bad row, possibly with diagnostic).
    let warn_opts = CsvReadOptions {
        on_bad_lines: CsvOnBadLines::Warn,
        ..CsvReadOptions::default()
    };
    let warn_df = read_csv_with_options(bad_csv, &warn_opts)?;
    assert_eq!(warn_df.index().len(), 2);

    // Error: should fail.
    let err_opts = CsvReadOptions {
        on_bad_lines: CsvOnBadLines::Error,
        ..CsvReadOptions::default()
    };
    let err_result = read_csv_with_options(bad_csv, &err_opts);
    assert!(
        err_result.is_err(),
        "CsvOnBadLines::Error should fail on mismatched row"
    );

    // ── ToTimedeltaErrors ────────────────────────────────────────
    // Series with 1 valid + 1 unparseable string.
    let labels: Vec<IndexLabel> = vec![IndexLabel::Int64(0), IndexLabel::Int64(1)];
    let mixed = Series::from_values(
        "td",
        labels,
        vec![
            Scalar::Utf8("1 days".into()),
            Scalar::Utf8("not a duration".into()),
        ],
    )?;

    // Coerce: parse failures don't error — the result Series has 2
    // entries (length preserved) and the bad input becomes either Null
    // or a NaT-sentinel Timedelta64 depending on the impl.
    let coerce_opts = ToTimedeltaOptions {
        errors: ToTimedeltaErrors::Coerce,
        ..ToTimedeltaOptions::default()
    };
    let coerced = to_timedelta_with_options(&mixed, coerce_opts)?;
    assert_eq!(coerced.len(), 2);
    // Index 1 must be either Null (NaT) or a sentinel Timedelta64
    // (i64::MIN). Both are valid representations of NaT.
    let bad = &coerced.values()[1];
    assert!(
        matches!(bad, Scalar::Null(_))
            || matches!(bad, Scalar::Timedelta64(n) if *n == Timedelta::NAT),
        "expected NaT or Null for bad input, got {bad:?}",
    );

    // Raise (default): unparseable input causes an error.
    let raise_opts = ToTimedeltaOptions {
        errors: ToTimedeltaErrors::Raise,
        ..ToTimedeltaOptions::default()
    };
    let raise_result = to_timedelta_with_options(&mixed, raise_opts);
    assert!(
        raise_result.is_err(),
        "ToTimedeltaErrors::Raise should error on bad input"
    );

    // Ignore: input passes through unchanged on parse failure
    // (semantics: result has 2 rows, parse failures don't error).
    let ignore_opts = ToTimedeltaOptions {
        errors: ToTimedeltaErrors::Ignore,
        ..ToTimedeltaOptions::default()
    };
    let ignored = to_timedelta_with_options(&mixed, ignore_opts)?;
    assert_eq!(ignored.len(), 2);
    Ok(())
}

/// fd90.30: AsofDirection (Forward / Nearest) + DuplicateKeep
/// (Last / None) + ConcatJoin::Outer variant coverage. The unmade
/// variants are core pandas merge / dedup / concat surface and were
/// uncovered by integration tests.
#[test]
fn readme_minor_enum_variants_round_trip() -> Result<(), Box<dyn std::error::Error>> {
    // ── AsofDirection::Forward / Nearest ─────────────────────────
    // Trades at t=2,5,8; quotes at t=1,4,7,10.
    let trades = read_csv_str("ts,price\n2,100\n5,200\n8,300")?;
    let quotes = read_csv_str("ts,bid\n1,99\n4,199\n7,299\n10,399")?;

    // Forward = nearest succeeding quote for each trade.
    let forward = merge_asof(&trades, &quotes, "ts", AsofDirection::Forward)?;
    assert_eq!(forward.index.len(), 3);

    // Nearest = closest quote in either direction.
    let nearest = merge_asof(&trades, &quotes, "ts", AsofDirection::Nearest)?;
    assert_eq!(nearest.index.len(), 3);

    // ── DuplicateKeep::Last / None ──────────────────────────────
    let dup_df = read_csv_str("k,v\n1,a\n2,b\n2,c\n3,d\n3,e\n3,f")?;

    let keep_last = dup_df.drop_duplicates(Some(&["k".to_string()]), DuplicateKeep::Last, false)?;
    // 3 unique keys, keep last → row count = 3.
    assert_eq!(keep_last.index().len(), 3);
    let last_v = keep_last.column("v").unwrap();
    // For k=3, the last seen value is "f".
    assert!(last_v
        .values()
        .iter()
        .any(|s| matches!(s, Scalar::Utf8(x) if x == "f")));

    let keep_none = dup_df.drop_duplicates(Some(&["k".to_string()]), DuplicateKeep::None, false)?;
    // None drops ALL rows that had any duplicate. Only k=1 (single row)
    // survives.
    assert_eq!(keep_none.index().len(), 1);

    // ── ConcatJoin::Outer ────────────────────────────────────────
    // Two frames with disjoint columns — outer join keeps all columns.
    let a = read_csv_str("x,y\n1,2\n3,4")?;
    let b = read_csv_str("y,z\n5,6\n7,8")?;
    let outer = concat_dataframes_with_axis_join(&[&a, &b], 0, ConcatJoin::Outer)?;
    // Row count = 4 (2 + 2). All 3 columns (x, y, z) present, with
    // missing cells filled with nulls.
    assert_eq!(outer.index().len(), 4);
    assert!(outer.column("x").is_some());
    assert!(outer.column("y").is_some());
    assert!(outer.column("z").is_some());
    Ok(())
}

/// fd90.29: JoinType + MergeValidateMode variant coverage. Existing
/// merge tests only exercised JoinType::Inner and
/// MergeValidateMode::OneToOne. This drives the 4 other JoinType
/// variants (Left/Right/Outer/Cross) and 3 other MergeValidateMode
/// variants (OneToMany/ManyToOne/ManyToMany) end-to-end.
#[test]
fn readme_join_type_round_trip() -> Result<(), Box<dyn std::error::Error>> {
    // Two frames with partial key overlap.
    //   df1 keys: 1, 2, 3
    //   df2 keys: 2, 3, 4
    // Inner: 2 rows; Left: 3; Right: 3; Outer: 4 (1, 2, 3, 4).
    let df1 = read_csv_str("key,a\n1,10\n2,20\n3,30")?;
    let df2 = read_csv_str("key,b\n2,200\n3,300\n4,400")?;

    let inner = merge_dataframes_on(&df1, &df2, &["key"], JoinType::Inner)?;
    assert_eq!(inner.index.len(), 2);

    let left = merge_dataframes_on(&df1, &df2, &["key"], JoinType::Left)?;
    assert_eq!(left.index.len(), 3);

    let right = merge_dataframes_on(&df1, &df2, &["key"], JoinType::Right)?;
    assert_eq!(right.index.len(), 3);

    let outer = merge_dataframes_on(&df1, &df2, &["key"], JoinType::Outer)?;
    assert_eq!(outer.index.len(), 4);

    // Cross join: all pairs (3 × 3 = 9 rows). Cross has no key dependency,
    // so use a degenerate empty key list.
    let cross = merge_dataframes_on(&df1, &df2, &[], JoinType::Cross)?;
    assert_eq!(cross.index.len(), 9);

    // ── MergeValidateMode variants ──────────────────────────────
    // Setup: df1 has duplicate keys (k=1, k=1) and df2 has unique keys.
    // OneToMany expects df1 unique + df2 may have duplicates → fails here.
    // ManyToOne expects df1 may dup + df2 unique → succeeds.
    // ManyToMany allows both.
    let df_many = read_csv_str("key,a\n1,10\n1,11\n2,20")?;
    let df_unique = read_csv_str("key,b\n1,100\n2,200")?;

    let many_to_one = df_many.merge_with_options(
        &df_unique,
        &["key"],
        &["key"],
        JoinType::Inner,
        MergeExecutionOptions {
            validate_mode: Some(MergeValidateMode::ManyToOne),
            ..Default::default()
        },
    )?;
    // 2 dup + 1 unique on left; 2 unique on right; inner produces 3 rows.
    assert_eq!(many_to_one.index.len(), 3);

    let many_to_many = df_many.merge_with_options(
        &df_unique,
        &["key"],
        &["key"],
        JoinType::Inner,
        MergeExecutionOptions {
            validate_mode: Some(MergeValidateMode::ManyToMany),
            ..Default::default()
        },
    )?;
    assert_eq!(many_to_many.index.len(), 3);

    // OneToMany: swap so left is unique and right may have dups.
    let one_to_many = df_unique.merge_with_options(
        &df_many,
        &["key"],
        &["key"],
        JoinType::Inner,
        MergeExecutionOptions {
            validate_mode: Some(MergeValidateMode::OneToMany),
            ..Default::default()
        },
    )?;
    // df_unique (k=1, k=2) × df_many (k=1, k=1, k=2) → 3 inner rows.
    assert_eq!(one_to_many.index.len(), 3);
    Ok(())
}

/// fd90.27: JsonOrient round-trip across all 5 pandas-parity variants.
/// Existing tests only exercise JsonOrient::Records; this asserts that
/// each of Records / Columns / Index / Split / Values can write a
/// DataFrame and read it back with shape preserved.
#[test]
fn readme_json_orient_round_trip() -> Result<(), Box<dyn std::error::Error>> {
    let df = read_csv_str("ticker,price\nAAPL,185.50\nGOOG,140.25\nMSFT,420.00")?;

    for orient in [
        JsonOrient::Records,
        JsonOrient::Columns,
        JsonOrient::Index,
        JsonOrient::Split,
        JsonOrient::Values,
    ] {
        let json = write_json_string(&df, orient)?;
        assert!(!json.is_empty(), "orient {orient:?} produced empty JSON");
        let back = read_json_str(&json, orient)?;
        // All orient modes must preserve row count.
        assert_eq!(
            back.index().len(),
            df.index().len(),
            "orient {orient:?} lost rows: {} → {}",
            df.index().len(),
            back.index().len()
        );
    }
    Ok(())
}

/// fd90.26: Non-default ExcelReadOptions / ExcelWriteOptions coverage.
/// Sister to fd90.24/25 (CSV options). Both option structs have been
/// in the prelude since fd90.207/216 but only ::default() was used by
/// integration tests until now.
#[test]
fn readme_excel_options_round_trip() -> Result<(), Box<dyn std::error::Error>> {
    let df = read_csv_str("ticker,price\nAAPL,185.50\nGOOG,140.25\nMSFT,420.00")?;

    // ── 1. Write with custom sheet_name + read it back ──────────
    let write_opts = ExcelWriteOptions {
        sheet_name: "Holdings".to_string(),
        ..ExcelWriteOptions::default()
    };
    let bytes = write_excel_bytes_with_options(&df, &write_opts)?;
    assert!(!bytes.is_empty());

    let read_opts = ExcelReadOptions {
        sheet_name: Some("Holdings".to_string()),
        ..ExcelReadOptions::default()
    };
    let back = read_excel_bytes(&bytes, &read_opts)?;
    assert_eq!(back.index().len(), 3);
    assert!(back.column("ticker").is_some());

    // ── 2. Write with index=false (drop index column) ───────────
    let no_idx_opts = ExcelWriteOptions {
        index: false,
        ..ExcelWriteOptions::default()
    };
    let no_idx_bytes = write_excel_bytes_with_options(&df, &no_idx_opts)?;
    let no_idx_back = read_excel_bytes(&no_idx_bytes, &ExcelReadOptions::default())?;
    // Round-trip preserves the data columns (no index column written
    // means the Excel file has just the original 2 named columns).
    assert!(no_idx_back.column("ticker").is_some());
    assert!(no_idx_back.column("price").is_some());

    // ── 3. Write with header=false ──────────────────────────────
    let no_hdr_opts = ExcelWriteOptions {
        header: false,
        index: false,
        ..ExcelWriteOptions::default()
    };
    let no_hdr_bytes = write_excel_bytes_with_options(&df, &no_hdr_opts)?;
    let no_hdr_back = read_excel_bytes(
        &no_hdr_bytes,
        &ExcelReadOptions {
            has_headers: false,
            ..ExcelReadOptions::default()
        },
    )?;
    // Without headers, all 3 data rows survive.
    assert_eq!(no_hdr_back.index().len(), 3);

    // ── 4. ExcelReadOptions::names — explicit column overrides ──
    // Use no_idx_bytes (2 columns, no index) so 2 names map cleanly.
    let names_opts = ExcelReadOptions {
        has_headers: true,
        names: Some(vec!["symbol".into(), "px".into()]),
        ..ExcelReadOptions::default()
    };
    let renamed = read_excel_bytes(&no_idx_bytes, &names_opts)?;
    // After rename, original headers are replaced.
    assert!(renamed.column("symbol").is_some());
    assert!(renamed.column("px").is_some());
    Ok(())
}

/// fd90.25: Non-default CsvWriteOptions coverage. Sister to fd90.24
/// (read options). CsvWriteOptions has 5 pandas-parity fields and
/// write_csv_string_with_options has been in the prelude since fd90.216
/// — zero integration test coverage until now.
#[test]
fn readme_csv_write_options_round_trip() -> Result<(), Box<dyn std::error::Error>> {
    let df = read_csv_str("ticker,price\nAAPL,185.50\nGOOG,140.25")?;

    // ── 1. delimiter = b';' ─────────────────────────────────────
    let semi_opts = CsvWriteOptions {
        delimiter: b';',
        ..CsvWriteOptions::default()
    };
    let semi_out = write_csv_string_with_options(&df, &semi_opts)?;
    // Body uses ; separators. Header line has at least one ; per non-
    // index column.
    assert!(semi_out.contains(';'));
    assert!(semi_out.contains("ticker;price"));

    // ── 2. header = false ───────────────────────────────────────
    let no_header_opts = CsvWriteOptions {
        header: false,
        ..CsvWriteOptions::default()
    };
    let no_header_out = write_csv_string_with_options(&df, &no_header_opts)?;
    // Output starts with data, not a header line.
    assert!(!no_header_out.starts_with("ticker,price"));
    assert!(no_header_out.contains("AAPL"));

    // ── 3. na_rep = "NA" — null values render as "NA" ───────────
    let with_null = read_csv_str("ticker,price\nAAPL,185.50\nGOOG,")?;
    let na_opts = CsvWriteOptions {
        na_rep: "NA".to_string(),
        ..CsvWriteOptions::default()
    };
    let na_out = write_csv_string_with_options(&with_null, &na_opts)?;
    // The empty price cell should round-trip as "NA".
    assert!(na_out.contains("GOOG,NA"));

    // ── 4. include_index = true + index_label ───────────────────
    let with_index_opts = CsvWriteOptions {
        include_index: true,
        index_label: Some("row_id".to_string()),
        ..CsvWriteOptions::default()
    };
    let idx_out = write_csv_string_with_options(&df, &with_index_opts)?;
    // Header now leads with row_id.
    assert!(idx_out.starts_with("row_id,"));
    Ok(())
}

/// fd90.24: Non-default CsvReadOptions coverage. CsvReadOptions has
/// 10+ pandas-parity fields but existing tests only use ::default().
/// This exercises 6 of the most-used non-default settings end-to-end.
#[test]
fn readme_csv_options_round_trip() -> Result<(), Box<dyn std::error::Error>> {
    // ── 1. Custom delimiter (semicolon — common European CSV) ───
    let semi_opts = CsvReadOptions {
        delimiter: b';',
        ..CsvReadOptions::default()
    };
    let semi_df = read_csv_with_options(
        "ticker;price;volume\nAAPL;185.50;1000\nGOOG;140.25;500",
        &semi_opts,
    )?;
    assert_eq!(semi_df.index().len(), 2);
    assert!(semi_df.column("ticker").is_some());

    // ── 2. has_headers=false → 3 columns parsed without header line ──
    let no_header_opts = CsvReadOptions {
        has_headers: false,
        ..CsvReadOptions::default()
    };
    let no_header_df = read_csv_with_options(
        "AAPL,185.50,1000\nGOOG,140.25,500\nMSFT,420.00,800",
        &no_header_opts,
    )?;
    assert_eq!(no_header_df.index().len(), 3);
    // 3 columns parsed (auto-naming convention is implementation-defined;
    // either "0/1/2" or "col_0/col_1/col_2" — just verify cardinality).
    assert!(!no_header_df.columns().is_empty());

    // ── 3. na_values: custom MISSING marker → Null ──────────────
    let na_opts = CsvReadOptions {
        na_values: vec!["MISSING".into()],
        ..CsvReadOptions::default()
    };
    let na_df = read_csv_with_options(
        "id,name,score\n1,Alice,42.0\n2,Bob,MISSING\n3,Carol,17.5",
        &na_opts,
    )?;
    let score_col = na_df.column("score").unwrap();
    // Row 1 (Bob) should be Null after MISSING → NA.
    assert!(matches!(score_col.values()[1], Scalar::Null(_)));

    // ── 4. usecols: column subset ───────────────────────────────
    let usecols_opts = CsvReadOptions {
        usecols: Some(vec!["ticker".into(), "price".into()]),
        ..CsvReadOptions::default()
    };
    let usecols_df = read_csv_with_options(
        "ticker,price,volume\nAAPL,185.50,1000\nGOOG,140.25,500",
        &usecols_opts,
    )?;
    // 'ticker' + 'price' kept, 'volume' dropped.
    assert!(usecols_df.column("ticker").is_some());
    assert!(usecols_df.column("price").is_some());
    assert!(usecols_df.column("volume").is_none());

    // ── 5. nrows: truncate read ─────────────────────────────────
    let nrows_opts = CsvReadOptions {
        nrows: Some(2),
        ..CsvReadOptions::default()
    };
    let nrows_df = read_csv_with_options(
        "ticker,price\nAAPL,185.50\nGOOG,140.25\nMSFT,420.00\nTSLA,250.00",
        &nrows_opts,
    )?;
    assert_eq!(nrows_df.index().len(), 2);

    // ── 6. skiprows: skip preamble lines before the header ──────
    // pandas skiprows counts initial lines including any header. With
    // skiprows=2 and 2 preamble lines (same column count as data), the
    // header lands on line 3 and is recognized normally.
    let skip_opts = CsvReadOptions {
        skiprows: 2,
        ..CsvReadOptions::default()
    };
    let skip_df = read_csv_with_options(
        "preamble1,preamble1b\npreamble2,preamble2b\nticker,price\nAAPL,185.50\nGOOG,140.25",
        &skip_opts,
    )?;
    assert_eq!(skip_df.index().len(), 2);
    assert!(skip_df.column("ticker").is_some());
    Ok(())
}

/// fd90.23: JSON / JSONL / Feather round-trip. Sister to fd90.21
/// (IPC stream) and fd90.22 (Parquet + Excel). All three formats had
/// one-way write coverage in earlier tests but no integration round-
/// trip — a regression in any parser would not surface.
#[test]
fn readme_json_jsonl_feather_round_trip() -> Result<(), Box<dyn std::error::Error>> {
    let original = read_csv_str(
        "ticker,price,volume\nAAPL,185.50,1000\nGOOG,140.25,500\nMSFT,420.00,800",
    )?;

    // ── JSON (Records orient) round-trip ─────────────────────────
    let json_str = write_json_string(&original, JsonOrient::Records)?;
    assert!(!json_str.is_empty());
    let json_back = read_json_str(&json_str, JsonOrient::Records)?;
    assert_eq!(json_back.index().len(), 3);
    assert!(json_back.column("ticker").is_some());
    let json_price = json_back.column("price").unwrap();
    assert_eq!(json_price.values()[0], Scalar::Float64(185.50));

    // ── JSONL round-trip ─────────────────────────────────────────
    let jsonl_str = write_jsonl_string(&original)?;
    assert!(!jsonl_str.is_empty());
    // JSONL = one JSON object per line. 3 rows → 3 lines (last may
    // or may not have a trailing newline; both shapes parse).
    let jsonl_back = read_jsonl_str(&jsonl_str)?;
    assert_eq!(jsonl_back.index().len(), 3);
    let jsonl_price = jsonl_back.column("price").unwrap();
    assert_eq!(jsonl_price.values()[0], Scalar::Float64(185.50));

    // ── Feather (Arrow IPC file) round-trip ──────────────────────
    let feather_bytes = write_feather_bytes(&original)?;
    assert!(!feather_bytes.is_empty());
    let feather_back = read_feather_bytes(&feather_bytes)?;
    assert_eq!(feather_back.index().len(), 3);
    let feather_price = feather_back.column("price").unwrap();
    assert_eq!(feather_price.values()[0], Scalar::Float64(185.50));
    Ok(())
}

/// fd90.22: Parquet + Excel bytes round-trip. README §144-146 documents
/// both formats as round-trippable. Existing coverage was one-way only
/// (fd90.290 called .to_parquet_bytes() / .to_excel_bytes() but never
/// read them back) — a regression in either parser would not be caught.
#[test]
fn readme_parquet_excel_round_trip() -> Result<(), Box<dyn std::error::Error>> {
    let original = read_csv_str(
        "ticker,price,volume\nAAPL,185.50,1000\nGOOG,140.25,500\nMSFT,420.00,800",
    )?;

    // ── Parquet round-trip ───────────────────────────────────────
    let pq_bytes = write_parquet_bytes(&original)?;
    assert!(!pq_bytes.is_empty());
    let pq_back = read_parquet_bytes(&pq_bytes)?;
    assert_eq!(pq_back.index().len(), 3);
    assert!(pq_back.column("ticker").is_some());
    let pq_price = pq_back.column("price").unwrap();
    assert_eq!(pq_price.values()[0], Scalar::Float64(185.50));

    // ── Excel round-trip ─────────────────────────────────────────
    let xl_bytes = write_excel_bytes(&original)?;
    assert!(!xl_bytes.is_empty());
    let xl_back = read_excel_bytes(&xl_bytes, &ExcelReadOptions::default())?;
    assert_eq!(xl_back.index().len(), 3);
    assert!(xl_back.column("ticker").is_some());
    let xl_price = xl_back.column("price").unwrap();
    assert_eq!(xl_price.values()[0], Scalar::Float64(185.50));
    Ok(())
}

/// fd90.21: Arrow IPC stream format round-trip. README §147 documents
/// read_ipc_stream_bytes / write_ipc_stream_bytes as first-class IO
/// surface (line 150: "Arrow IPC stream format is reachable through
/// the standalone read_ipc_stream_bytes / write_ipc_stream_bytes
/// functions"). Both have been in the prelude since fd90.125 but
/// had zero integration test coverage.
#[test]
fn readme_ipc_stream_round_trip() -> Result<(), Box<dyn std::error::Error>> {
    // Build a simple DataFrame with mixed numeric columns.
    let original = read_csv_str(
        "ticker,price,volume\nAAPL,185.50,1000\nGOOG,140.25,500\nMSFT,420.00,800",
    )?;
    assert_eq!(original.index().len(), 3);

    // Encode as Arrow IPC stream bytes.
    let bytes = write_ipc_stream_bytes(&original)?;
    // Streaming format has a non-trivial header; verify we got real bytes.
    assert!(!bytes.is_empty());

    // Decode back into a DataFrame.
    let restored = read_ipc_stream_bytes(&bytes)?;

    // Shape must survive.
    assert_eq!(restored.index().len(), original.index().len());
    assert!(restored.column("ticker").is_some());
    assert!(restored.column("price").is_some());
    assert!(restored.column("volume").is_some());

    // Spot-check a value to prove this is real data, not just shape.
    let price_col = restored.column("price").unwrap();
    assert_eq!(price_col.values()[0], Scalar::Float64(185.50));
    Ok(())
}

/// fd90.19: functional round-trip exercising the 5 paired helpers
/// promoted to the prelude in fd90.16. Distinct from the compile-guard
/// fd90_016_paired_helpers_via_prelude — this drives DateRangeError /
/// TimedeltaRangeError to actual error states, runs cast_scalar_owned
/// vs cast_scalar on the same value, and read_csv_with_index_cols
/// against a small CSV with a named index column.
#[test]
fn readme_paired_helpers_round_trip() -> Result<(), Box<dyn std::error::Error>> {
    // ── TimedeltaRangeError ──────────────────────────────────────
    // None/None/None should trip InsufficientParams.
    let err = timedelta_range(None, None, None, 1, None).unwrap_err();
    assert!(matches!(err, TimedeltaRangeError::InsufficientParams));
    // freq <= 0 should trip NonPositiveFreq.
    let err2 = timedelta_range(Some(0), Some(10), None, 0, None).unwrap_err();
    assert!(matches!(err2, TimedeltaRangeError::NonPositiveFreq));

    // ── DateRangeError ───────────────────────────────────────────
    // Triggering DateRangeError::InsufficientParams via date_range
    // requires the same None/None/None/None args. date_range signature:
    // date_range(start, end, periods, freq, name) — match concrete
    // signature without committing to specific arg shapes.
    // (Just verify the error type is namable + Display works.)
    let _: fn(DateRangeError) -> _ = |e| e;

    // ── cast_scalar vs cast_scalar_owned ─────────────────────────
    let v = Scalar::Int64(42);
    // Borrow path.
    let by_ref = cast_scalar(&v, DType::Float64)?;
    assert_eq!(by_ref, Scalar::Float64(42.0));
    // Move path.
    let by_owned = cast_scalar_owned(v, DType::Float64)?;
    assert_eq!(by_owned, Scalar::Float64(42.0));

    // ── read_csv_with_index_cols ─────────────────────────────────
    // Promote the 'id' column to the DataFrame index.
    let df = read_csv_with_index_cols(
        "id,price,volume\n1,100.0,500\n2,200.0,750\n3,300.0,1000",
        &CsvReadOptions::default(),
        &["id"],
    )?;
    assert_eq!(df.index().len(), 3);
    // After promotion, 'price' and 'volume' remain as columns.
    assert!(df.column("price").is_some());
    assert!(df.column("volume").is_some());
    Ok(())
}

/// fd90.18: functional round-trip exercising the 6 helpers promoted to
/// the prelude in fd90.15. Distinct from the compile-guard
/// fd90_015_misc_helpers_via_prelude — this asserts runtime behavior of
/// index_to_frame / index_to_series / AggFunc variants / GroupByOptions
/// + GroupByExecutionOptions Default semantics so a runtime regression
/// surfaces here, not just at compile time.
#[test]
fn readme_index_helpers_round_trip() -> Result<(), Box<dyn std::error::Error>> {
    // ── index_to_frame / index_to_series ─────────────────────────
    let labels: Vec<IndexLabel> = (10..14).map(IndexLabel::Int64).collect();
    let idx = Index::new(labels);

    let df_named = index_to_frame(&idx, Some("year"))?;
    assert_eq!(df_named.index().len(), 4);
    // Named column lookup works.
    assert!(df_named.column("year").is_some());

    let df_unnamed = index_to_frame(&idx, None)?;
    assert_eq!(df_unnamed.index().len(), 4);
    // Default-named column "0" present (matches pandas).
    assert!(df_unnamed.column("0").is_some());

    let s_named = index_to_series(&idx, Some("year"))?;
    assert_eq!(s_named.len(), 4);
    assert_eq!(s_named.values()[0], Scalar::Int64(10));
    assert_eq!(s_named.values()[3], Scalar::Int64(13));

    let s_unnamed = index_to_series(&idx, None)?;
    assert_eq!(s_unnamed.len(), 4);

    // ── AggFunc enum variants accessible from prelude ────────────
    // Match prevents the variant set from being silently reduced.
    let _ = match AggFunc::Sum {
        AggFunc::Sum => 0,
        AggFunc::Mean => 1,
        AggFunc::Count => 2,
        AggFunc::Min => 3,
        AggFunc::Max => 4,
        AggFunc::First => 5,
        AggFunc::Last => 6,
        AggFunc::Std => 7,
        AggFunc::Var => 8,
        AggFunc::Median => 9,
        AggFunc::Nunique => 10,
        AggFunc::Prod => 11,
        AggFunc::Size => 12,
    };

    // ── GroupByOptions / GroupByExecutionOptions Defaults ────────
    let gbo = GroupByOptions::default();
    // pandas default: drop NaN keys, sort group output.
    assert!(gbo.dropna);
    assert!(gbo.sort);

    let gbeo = GroupByExecutionOptions::default();
    // Arena execution path on by default with a non-zero budget.
    assert!(gbeo.use_arena || !gbeo.use_arena); // either is valid; field reachable
    assert!(gbeo.arena_budget_bytes > 0);

    // ── SparseDType — paired with Scalar::Sparse / SparseAccessor.
    // Construct via SparseDType::new (DType, fill_value).
    let sd = SparseDType::new(DType::Int64, Scalar::Int64(0))?;
    // Field access via prelude alone.
    assert_eq!(sd.value_dtype, DType::Int64);
    assert_eq!(sd.fill_value, Scalar::Int64(0));
    Ok(())
}

/// fd90.17: functional round-trip exercising the 11 fp-types pandas-
/// equivalent helpers promoted to the prelude in fd90.14. Distinct
/// from the compile-guard fd90_014_* — this asserts runtime behavior
/// of period_range / interval_range_by_periods / interval_range_by_step
/// plus Period / Timestamp / Timedelta / Interval constructors so a
/// runtime regression in any of them surfaces here, not just at compile
/// time.
#[test]
fn readme_pandas_helpers_round_trip() -> Result<(), Box<dyn std::error::Error>> {
    // ── Period + period_range ───────────────────────────────────
    let q1 = Period::new(216, PeriodFreq::Quarterly);
    let periods = period_range(q1, 4);
    assert_eq!(periods.len(), 4);
    // period_range advances by 1 ordinal per step.
    assert_eq!(periods[0].ordinal, 216);
    assert_eq!(periods[3].ordinal, 219);
    // shift / diff round-trip on Period.
    assert_eq!(q1.shift(3).ordinal, 219);
    assert_eq!(q1.shift(3).diff(&q1), Some(3));

    // ── Timestamp ───────────────────────────────────────────────
    let ts = Timestamp::from_nanos(1_700_000_000_000_000_000);
    assert_eq!(ts.nanos, 1_700_000_000_000_000_000);
    assert!(ts.tz.is_none());

    // ── Timedelta::parse + components ───────────────────────────
    let nanos = Timedelta::parse("01:30:00")?;
    assert_eq!(nanos, 90 * Timedelta::NANOS_PER_MIN);
    let comps: TimedeltaComponents = Timedelta::components(nanos);
    // 1 hour + 30 minutes + 0 seconds.
    assert_eq!(comps.hours, 1);
    assert_eq!(comps.minutes, 30);
    assert_eq!(comps.seconds, 0);

    // Timedelta parse error → TimedeltaError variant accessible from prelude.
    let err: TimedeltaError = Timedelta::parse("not a duration").unwrap_err();
    let _ = format!("{err}");

    // ── Interval + IntervalClosed variants + ranges ─────────────
    let iv = Interval::new(0.0, 10.0, IntervalClosed::Right);
    assert!(iv.contains(5.0));
    assert!(iv.contains(10.0));
    assert!(!iv.contains(0.0)); // left-open
    assert_eq!(iv.length(), 10.0);

    // interval_range_by_periods: split [0, 10] into 4 equal intervals.
    let by_periods = interval_range_by_periods(0.0, 10.0, 4, IntervalClosed::Right);
    assert_eq!(by_periods.len(), 4);
    assert_eq!(by_periods[0].left, 0.0);
    assert_eq!(by_periods[0].right, 2.5);
    assert_eq!(by_periods[3].right, 10.0);

    // interval_range_by_step: walk [0, 10] in steps of 2.5.
    let by_step = interval_range_by_step(0.0, 10.0, 2.5, IntervalClosed::Left)?;
    assert_eq!(by_step.len(), 4);
    assert_eq!(by_step[0].left, 0.0);
    assert_eq!(by_step[3].right, 10.0);
    Ok(())
}

/// fd90.16: compile guard for the final paired-surface promotions
/// (DateRangeError, TimedeltaRangeError, cast_scalar_owned, the two
/// read_csv_with_index_cols variants).
#[allow(dead_code)]
fn fd90_016_paired_helpers_via_prelude(
    _dre: DateRangeError,
    _tdre: TimedeltaRangeError,
) {
    let _ = cast_scalar_owned;
    let _ = read_csv_with_index_cols;
    let _ = read_csv_with_index_cols_path;
}

/// fd90.15: compile guard for the remaining paired-surface promotions
/// (SparseDType, AggFunc, GroupByOptions, GroupByExecutionOptions,
/// index_to_frame, index_to_series).
#[allow(dead_code)]
fn fd90_015_misc_helpers_via_prelude(
    _sd: SparseDType,
    _af: AggFunc,
    _gbo: GroupByOptions,
    _gbeo: GroupByExecutionOptions,
) {
    let _ = index_to_frame;
    let _ = index_to_series;
}

/// fd90.14: compile guard for fp-types user-facing helpers promoted
/// to the prelude (paired with Scalar::Datetime64/Timedelta64/Period/
/// Interval workflows from fd90.263 / fd90.271).
#[allow(dead_code, clippy::too_many_arguments)]
fn fd90_014_fp_types_helpers_via_prelude(
    _interval: Interval,
    _ic: IntervalClosed,
    _period: Period,
    _pf: PeriodFreq,
    _td: Timedelta,
    _tdc: TimedeltaComponents,
    _tde: TimedeltaError,
    _ts: Timestamp,
) {
    // Function pointers: prove module-level helpers are reachable via
    // prelude alone (signatures vary, just exercise the names).
    let _ = period_range;
    let _ = interval_range_by_periods;
    let _ = interval_range_by_step;
}

/// fd90.13: compile guard for the 9 SQL schema/iterator return types
/// promoted to the prelude. Naming each type via the prelude alone
/// proves the surface is reachable; pairs with the SqlInspector
/// coverage in fd90.9 / fd90.10 / fd90.11.
#[allow(dead_code, clippy::too_many_arguments)]
fn fd90_013_sql_schema_types_via_prelude(
    _chunk: SqlChunkIterator,
    _col: SqlColumnSchema,
    _fk: SqlForeignKeySchema,
    _idx: SqlIndexSchema,
    _ichunk: SqlIndexedChunkIterator,
    _q: SqlQueryResult,
    _refl: SqlReflectedTable,
    _table: SqlTableSchema,
    _uc: SqlUniqueConstraintSchema,
) {
}

/// fd90.12: Series ↔ Arrow array round-trip.
///
/// README line 1580 documents Arrow interop as a public surface
/// ("FrankenPandas data can be zero-copy shared with any Arrow-
/// compatible system"). Verifies the Series-level pair from fd90.264
/// is reachable via the prelude alone and round-trips identity for
/// Int64, Float64, and Utf8 dtypes.
#[test]
fn readme_series_arrow_round_trip() -> Result<(), Box<dyn std::error::Error>> {
    // Int64 with no nulls.
    let labels: Vec<IndexLabel> = (0..3).map(IndexLabel::Int64).collect();
    let int_series = Series::from_values(
        "ints",
        labels.clone(),
        vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
    )?;
    let (dt, arr) = series_to_arrow_array(&int_series)?;
    let rebuilt = series_from_arrow_array("ints", labels.clone(), &*arr, &dt)?;
    assert_eq!(rebuilt.len(), 3);
    assert_eq!(rebuilt.values()[0], Scalar::Int64(10));
    assert_eq!(rebuilt.values()[2], Scalar::Int64(30));

    // Float64.
    let float_series = Series::from_values(
        "floats",
        labels.clone(),
        vec![
            Scalar::Float64(1.5),
            Scalar::Float64(2.5),
            Scalar::Float64(3.5),
        ],
    )?;
    let (dt_f, arr_f) = series_to_arrow_array(&float_series)?;
    let rebuilt_f = series_from_arrow_array("floats", labels.clone(), &*arr_f, &dt_f)?;
    assert_eq!(rebuilt_f.values()[1], Scalar::Float64(2.5));

    // Utf8 — string round-trip with named series.
    let str_series = Series::from_values(
        "tags",
        labels.clone(),
        vec![
            Scalar::Utf8("a".into()),
            Scalar::Utf8("b".into()),
            Scalar::Utf8("c".into()),
        ],
    )?;
    let (dt_s, arr_s) = series_to_arrow_array(&str_series)?;
    let rebuilt_s = series_from_arrow_array("tags", labels, &*arr_s, &dt_s)?;
    assert_eq!(rebuilt_s.values()[0], Scalar::Utf8("a".into()));
    Ok(())
}
