#![forbid(unsafe_code)]

//! AG-E2E: End-to-End Integration Test Suite for Alien Graveyard Optimizations
//! (bd-2t5e.20)
//!
//! Validates that all AG optimizations work together without interference
//! across the full FrankenPandas pipeline.

use fp_columnar::Column;
use fp_frame::{DataFrame, Series};
use fp_groupby::{GroupByExecutionOptions, GroupByOptions, groupby_sum, groupby_sum_with_options};
use fp_index::{Index, IndexLabel, align_union, validate_alignment_plan};
use fp_join::{JoinExecutionOptions, JoinType, join_series, join_series_with_options};
use fp_runtime::{EvidenceLedger, RuntimePolicy};
use fp_types::{NullKind, Scalar};

// ---------------------------------------------------------------------------
// Scenario 1: Full DataFrame Pipeline (alignment + groupby + join)
// ---------------------------------------------------------------------------

#[test]
fn e2e_scenario1_full_dataframe_pipeline() {
    // Create 5 Series with different indexes.
    let s1 = Series::from_values(
        "revenue",
        vec![
            IndexLabel::Utf8("AAPL".into()),
            IndexLabel::Utf8("GOOG".into()),
            IndexLabel::Utf8("MSFT".into()),
        ],
        vec![
            Scalar::Float64(394.3),
            Scalar::Float64(283.0),
            Scalar::Float64(211.9),
        ],
    )
    .expect("s1");

    let s2 = Series::from_values(
        "profit",
        vec![
            IndexLabel::Utf8("AAPL".into()),
            IndexLabel::Utf8("MSFT".into()),
            IndexLabel::Utf8("AMZN".into()),
        ],
        vec![
            Scalar::Float64(99.8),
            Scalar::Float64(72.7),
            Scalar::Float64(21.3),
        ],
    )
    .expect("s2");

    let s3 = Series::from_values(
        "employees",
        vec![
            IndexLabel::Utf8("GOOG".into()),
            IndexLabel::Utf8("AAPL".into()),
        ],
        vec![Scalar::Int64(182502), Scalar::Int64(164000)],
    )
    .expect("s3");

    let s4 = Series::from_values(
        "founded",
        vec![
            IndexLabel::Utf8("AAPL".into()),
            IndexLabel::Utf8("GOOG".into()),
            IndexLabel::Utf8("MSFT".into()),
            IndexLabel::Utf8("AMZN".into()),
        ],
        vec![
            Scalar::Int64(1976),
            Scalar::Int64(1998),
            Scalar::Int64(1975),
            Scalar::Int64(1994),
        ],
    )
    .expect("s4");

    let s5 = Series::from_values(
        "hq_state",
        vec![
            IndexLabel::Utf8("AAPL".into()),
            IndexLabel::Utf8("GOOG".into()),
            IndexLabel::Utf8("MSFT".into()),
        ],
        vec![
            Scalar::Utf8("CA".into()),
            Scalar::Utf8("CA".into()),
            Scalar::Utf8("WA".into()),
        ],
    )
    .expect("s5");

    // Build DataFrame via from_series (exercises AG-05 N-way union alignment).
    let df = DataFrame::from_series(vec![s1, s2, s3, s4, s5]).expect("dataframe");
    // Union of all tickers: AAPL, GOOG, MSFT, AMZN -> 4 rows
    assert_eq!(df.index().len(), 4);
    assert_eq!(df.columns().len(), 5);

    // Series add (exercises alignment + arithmetic).
    let revenue = Series::from_values(
        "rev",
        vec![
            IndexLabel::Utf8("AAPL".into()),
            IndexLabel::Utf8("GOOG".into()),
        ],
        vec![Scalar::Float64(100.0), Scalar::Float64(200.0)],
    )
    .expect("revenue");

    let cost = Series::from_values(
        "cost",
        vec![
            IndexLabel::Utf8("GOOG".into()),
            IndexLabel::Utf8("AAPL".into()),
            IndexLabel::Utf8("TSLA".into()),
        ],
        vec![
            Scalar::Float64(50.0),
            Scalar::Float64(30.0),
            Scalar::Float64(80.0),
        ],
    )
    .expect("cost");

    let total = revenue.add(&cost).expect("add");
    // Union: AAPL, GOOG, TSLA -> 3 rows
    assert_eq!(total.index().len(), 3);
    // AAPL: 100+30=130, GOOG: 200+50=250, TSLA: missing+80=missing
    let aapl_idx = total
        .index()
        .labels()
        .iter()
        .position(|l| l == &IndexLabel::Utf8("AAPL".into()))
        .expect("AAPL");
    assert_eq!(total.values()[aapl_idx], Scalar::Float64(130.0));

    // Join (exercises AG-02 borrowed keys, AG-06 arena, all 4 join types).
    let left = Series::from_values(
        "left",
        vec![
            IndexLabel::Utf8("a".into()),
            IndexLabel::Utf8("b".into()),
            IndexLabel::Utf8("c".into()),
        ],
        vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
    )
    .expect("left");

    let right = Series::from_values(
        "right",
        vec![IndexLabel::Utf8("b".into()), IndexLabel::Utf8("d".into())],
        vec![Scalar::Int64(20), Scalar::Int64(40)],
    )
    .expect("right");

    let inner = join_series(&left, &right, JoinType::Inner).expect("inner");
    assert_eq!(inner.index.labels().len(), 1); // only "b"

    let left_j = join_series(&left, &right, JoinType::Left).expect("left");
    assert_eq!(left_j.index.labels().len(), 3); // a, b, c

    let right_j = join_series(&left, &right, JoinType::Right).expect("right");
    assert_eq!(right_j.index.labels().len(), 2); // b, d

    let outer = join_series(&left, &right, JoinType::Outer).expect("outer");
    assert_eq!(outer.index.labels().len(), 4); // a, b, c, d

    // GroupBy sum (exercises AG-08 clone elimination + dense Int64 path).
    let gkeys = Series::from_values(
        "dept",
        vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
        vec![
            Scalar::Utf8("eng".into()),
            Scalar::Utf8("sales".into()),
            Scalar::Utf8("eng".into()),
            Scalar::Utf8("sales".into()),
        ],
    )
    .expect("keys");

    let gvals = Series::from_values(
        "salary",
        vec![0_i64.into(), 1_i64.into(), 2_i64.into(), 3_i64.into()],
        vec![
            Scalar::Int64(150_000),
            Scalar::Int64(120_000),
            Scalar::Int64(160_000),
            Scalar::Int64(110_000),
        ],
    )
    .expect("values");

    let mut ledger = EvidenceLedger::new();
    let grouped = groupby_sum(
        &gkeys,
        &gvals,
        GroupByOptions::default(),
        &RuntimePolicy::strict(),
        &mut ledger,
    )
    .expect("groupby");

    assert_eq!(grouped.index().labels(), &["eng".into(), "sales".into()]);
    assert_eq!(
        grouped.values(),
        &[Scalar::Float64(310_000.0), Scalar::Float64(230_000.0)]
    );
}

// ---------------------------------------------------------------------------
// Scenario 2: Type Coercion Pipeline (cast + validity + column)
// ---------------------------------------------------------------------------

#[test]
fn e2e_scenario2_type_coercion_pipeline() {
    // Mixed-dtype column requiring coercion to Float64.
    let mixed_values = vec![
        Scalar::Int64(42),
        Scalar::Float64(3.125),
        Scalar::Null(NullKind::Null),
        Scalar::Int64(100),
        Scalar::Float64(f64::NAN),
        Scalar::Null(NullKind::NaN),
    ];

    let col = Column::from_values(mixed_values).expect("column");

    // Verify coercion happened: all numeric values become Float64.
    assert_eq!(col.len(), 6);

    // Verify validity mask correctly tracks null positions.
    let mask = col.validity();
    let bits: Vec<bool> = mask.bits().collect();
    assert!(bits[0]); // Int64(42) -> valid
    assert!(bits[1]); // Float64(3.14) -> valid
    assert!(!bits[2]); // Null(Null) -> invalid
    assert!(bits[3]); // Int64(100) -> valid
    assert!(!bits[4]); // NaN -> invalid
    assert!(!bits[5]); // Null(NaN) -> invalid

    assert_eq!(mask.count_valid(), 3);

    // Verify reindex with missing injection.
    let positions = vec![Some(0), None, Some(3), Some(1)];
    let reindexed = col.reindex_by_positions(&positions).expect("reindex");
    assert_eq!(reindexed.len(), 4);
    // Position None -> missing value injected
    assert!(reindexed.values()[1].is_missing());

    // Verify bitvec AND operation.
    let other_values = vec![
        Scalar::Int64(1),
        Scalar::Null(NullKind::Null),
        Scalar::Int64(3),
        Scalar::Int64(4),
        Scalar::Int64(5),
        Scalar::Int64(6),
    ];
    let other_col = Column::from_values(other_values).expect("other");
    let other_mask = other_col.validity();

    let combined = mask.and_mask(other_mask);
    let combined_bits: Vec<bool> = combined.bits().collect();
    // AND: both must be valid
    assert!(combined_bits[0]); // both valid
    assert!(!combined_bits[1]); // col invalid
    assert!(!combined_bits[2]); // col invalid
    assert!(combined_bits[3]); // both valid
    assert!(!combined_bits[4]); // col invalid (NaN)
    assert!(!combined_bits[5]); // col invalid
}

// ---------------------------------------------------------------------------
// Scenario 3: CSV Round-Trip (IO + alignment + groupby)
// ---------------------------------------------------------------------------

#[test]
fn e2e_scenario3_csv_round_trip() {
    // Build CSV with multiple columns and many rows.
    let mut csv = String::from("dept,employee,salary\n");
    let departments = ["engineering", "sales", "marketing", "hr"];
    for i in 0..1000 {
        let dept = departments[i % departments.len()];
        let salary = 50_000 + (i * 100);
        csv.push_str(&format!("{},emp_{},{}\n", dept, i, salary));
    }

    let df = fp_io::read_csv_str(&csv).expect("parse CSV");
    assert_eq!(df.index().len(), 1000);
    assert_eq!(df.columns().len(), 3);

    // Write CSV back and re-parse.
    let csv_out = fp_io::write_csv_string(&df).expect("write CSV");
    let df2 = fp_io::read_csv_str(&csv_out).expect("re-parse CSV");
    assert_eq!(df2.index().len(), df.index().len());
    assert_eq!(df2.columns().len(), df.columns().len());

    // Extract columns for groupby.
    let dept_col = df.column("dept").expect("dept column");
    let salary_col = df.column("salary").expect("salary column");

    // Create Series from columns for groupby.
    let dept_series =
        Series::new("dept", df.index().clone(), dept_col.clone()).expect("dept series");
    let salary_series =
        Series::new("salary", df.index().clone(), salary_col.clone()).expect("salary series");

    let mut ledger = EvidenceLedger::new();
    let grouped = groupby_sum(
        &dept_series,
        &salary_series,
        GroupByOptions::default(),
        &RuntimePolicy::strict(),
        &mut ledger,
    )
    .expect("groupby");

    // 4 departments
    assert_eq!(grouped.index().labels().len(), 4);

    // Verify arena vs global allocator produce identical groupby results.
    let global_grouped = groupby_sum_with_options(
        &dept_series,
        &salary_series,
        GroupByOptions::default(),
        &RuntimePolicy::strict(),
        &mut ledger,
        GroupByExecutionOptions {
            use_arena: false,
            arena_budget_bytes: 0,
        },
    )
    .expect("global groupby");

    let arena_grouped = groupby_sum_with_options(
        &dept_series,
        &salary_series,
        GroupByOptions::default(),
        &RuntimePolicy::strict(),
        &mut ledger,
        GroupByExecutionOptions::default(),
    )
    .expect("arena groupby");

    assert_eq!(
        global_grouped.index().labels(),
        arena_grouped.index().labels()
    );
    assert_eq!(global_grouped.values(), arena_grouped.values());
}

// ---------------------------------------------------------------------------
// Scenario 4: Index Operation Stress (dedup + alignment + lookup)
// ---------------------------------------------------------------------------

#[test]
fn e2e_scenario4_index_stress() {
    // Create Index with 100K labels (50K unique, each repeated twice).
    let labels: Vec<IndexLabel> = (0..100_000)
        .map(|i| IndexLabel::Int64((i % 50_000) as i64))
        .collect();
    let index = Index::new(labels);

    // Verify has_duplicates() is correct and O(1) on second call (Round 5 OnceCell).
    assert!(index.has_duplicates());
    // Second call should be instant (OnceCell cached).
    assert!(index.has_duplicates());

    // Create two 50K-element indexes for alignment.
    let left_labels: Vec<IndexLabel> = (0..50_000).map(|i| IndexLabel::Int64(i as i64)).collect();
    let right_labels: Vec<IndexLabel> = (25_000..75_000)
        .map(|i| IndexLabel::Int64(i as i64))
        .collect();
    let left_idx = Index::new(left_labels);
    let right_idx = Index::new(right_labels);

    // AG-02: align_union with borrowed-key HashMap.
    let plan = align_union(&left_idx, &right_idx);
    validate_alignment_plan(&plan).expect("valid alignment");

    // Union should contain 75K unique labels (0..75000).
    assert_eq!(plan.union_index.len(), 75_000);
    assert_eq!(plan.left_positions.len(), 75_000);
    assert_eq!(plan.right_positions.len(), 75_000);

    // Verify position integrity: left labels 0..50K should have Some positions.
    for i in 0..50_000 {
        assert!(
            plan.left_positions[i].is_some(),
            "left position {} should be Some",
            i
        );
    }
    // Labels 50K..75K are right-only, left should be None.
    for i in 50_000..75_000 {
        assert!(
            plan.left_positions[i].is_none(),
            "left position {} should be None",
            i
        );
    }

    // Arena join on large index sets.
    let left_series = Series::from_values(
        "left",
        (0..1000).map(|i| IndexLabel::Int64(i % 200)).collect(),
        (0..1000).map(Scalar::Int64).collect(),
    )
    .expect("left series");

    let right_series = Series::from_values(
        "right",
        (0..1000).map(|i| IndexLabel::Int64(i % 200)).collect(),
        (0..1000).map(|i| Scalar::Int64(i * 10)).collect(),
    )
    .expect("right series");

    // Arena join should work correctly.
    let arena_result = join_series_with_options(
        &left_series,
        &right_series,
        JoinType::Inner,
        JoinExecutionOptions::default(),
    )
    .expect("arena join");

    let global_result = join_series_with_options(
        &left_series,
        &right_series,
        JoinType::Inner,
        JoinExecutionOptions {
            use_arena: false,
            arena_budget_bytes: 0,
        },
    )
    .expect("global join");

    assert_eq!(
        arena_result.index.labels().len(),
        global_result.index.labels().len()
    );
}

// ---------------------------------------------------------------------------
// Scenario 5: Evidence Ledger + Decision Engine
// ---------------------------------------------------------------------------

#[test]
fn e2e_scenario5_evidence_ledger_and_policy() {
    let mut ledger = EvidenceLedger::new();

    // Strict mode rejects duplicate indexes.
    let dup_labels = vec![IndexLabel::Int64(1), IndexLabel::Int64(1)];
    let dup_values = vec![Scalar::Int64(10), Scalar::Int64(20)];
    let dup_series = Series::from_values("dup", dup_labels, dup_values).expect("dup series");

    let ok_labels = vec![IndexLabel::Int64(1), IndexLabel::Int64(2)];
    let ok_values = vec![Scalar::Int64(10), Scalar::Int64(20)];
    let ok_series = Series::from_values("ok", ok_labels, ok_values).expect("ok series");

    let strict = RuntimePolicy::strict();
    let result = dup_series.add_with_policy(&ok_series, &strict, &mut ledger);
    // Strict mode should reject duplicate indices.
    assert!(
        result.is_err(),
        "strict mode should reject duplicate indices"
    );

    // Hardened mode admits with logged decision.
    let hardened = RuntimePolicy::hardened(Some(100_000));
    let mut ledger2 = EvidenceLedger::new();
    let result = dup_series.add_with_policy(&ok_series, &hardened, &mut ledger2);
    assert!(
        result.is_ok(),
        "hardened mode should admit duplicate indices"
    );

    // Non-duplicate operations work in both modes.
    let s1 = Series::from_values(
        "a",
        vec![IndexLabel::Int64(1), IndexLabel::Int64(2)],
        vec![Scalar::Int64(10), Scalar::Int64(20)],
    )
    .expect("s1");

    let s2 = Series::from_values(
        "b",
        vec![IndexLabel::Int64(2), IndexLabel::Int64(3)],
        vec![Scalar::Int64(30), Scalar::Int64(40)],
    )
    .expect("s2");

    let mut ledger3 = EvidenceLedger::new();
    let strict_result = s1.add_with_policy(&s2, &strict, &mut ledger3);
    assert!(strict_result.is_ok(), "strict mode should allow clean add");

    let mut ledger4 = EvidenceLedger::new();
    let hardened_result = s1.add_with_policy(&s2, &hardened, &mut ledger4);
    assert!(
        hardened_result.is_ok(),
        "hardened mode should allow clean add"
    );

    // Both should produce identical results.
    let strict_out = strict_result.unwrap();
    let hardened_out = hardened_result.unwrap();
    assert_eq!(strict_out.index().labels(), hardened_out.index().labels());
    assert_eq!(strict_out.values(), hardened_out.values());

    // GroupBy in both modes.
    let keys = Series::from_values(
        "key",
        vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
        vec![Scalar::Int64(1), Scalar::Int64(1), Scalar::Int64(2)],
    )
    .expect("keys");

    let values = Series::from_values(
        "val",
        vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
        vec![
            Scalar::Float64(10.0),
            Scalar::Float64(20.0),
            Scalar::Float64(30.0),
        ],
    )
    .expect("values");

    let mut ledger5 = EvidenceLedger::new();
    let strict_gb = groupby_sum(
        &keys,
        &values,
        GroupByOptions::default(),
        &strict,
        &mut ledger5,
    )
    .expect("strict groupby");

    let mut ledger6 = EvidenceLedger::new();
    let hardened_gb = groupby_sum(
        &keys,
        &values,
        GroupByOptions::default(),
        &hardened,
        &mut ledger6,
    )
    .expect("hardened groupby");

    assert_eq!(strict_gb.index().labels(), hardened_gb.index().labels());
    assert_eq!(strict_gb.values(), hardened_gb.values());
}

// ---------------------------------------------------------------------------
// Cross-optimization interference: verifies no regressions from combined AG work
// ---------------------------------------------------------------------------

#[test]
fn e2e_cross_optimization_no_interference() {
    // Exercise multiple AG optimizations in sequence on shared data.
    let n = 5000;
    let labels: Vec<IndexLabel> = (0..n).map(|i| IndexLabel::Int64(i as i64)).collect();
    let values: Vec<Scalar> = (0..n).map(|i| Scalar::Float64(i as f64 * 1.5)).collect();

    let series_a = Series::from_values("a", labels.clone(), values.clone()).expect("a");
    let series_b = Series::from_values("b", labels.clone(), values).expect("b");

    // Step 1: Add (alignment + arithmetic).
    let sum = series_a.add(&series_b).expect("add");
    assert_eq!(sum.index().len(), n);

    // Step 2: Join with arena.
    let joined = join_series_with_options(
        &series_a,
        &series_b,
        JoinType::Inner,
        JoinExecutionOptions::default(),
    )
    .expect("join");
    assert_eq!(joined.index.labels().len(), n);

    // Step 3: GroupBy with arena (dense int64 path, all unique keys -> n groups).
    let key_labels: Vec<IndexLabel> = (0..n).map(|i| IndexLabel::Int64(i as i64)).collect();
    let key_values: Vec<Scalar> = (0..n).map(|i| Scalar::Int64(i as i64 % 100)).collect();
    let val_values: Vec<Scalar> = (0..n).map(|i| Scalar::Float64(i as f64)).collect();

    let keys = Series::from_values("k", key_labels.clone(), key_values).expect("keys");
    let vals = Series::from_values("v", key_labels, val_values).expect("vals");

    let mut ledger = EvidenceLedger::new();
    let grouped = groupby_sum_with_options(
        &keys,
        &vals,
        GroupByOptions::default(),
        &RuntimePolicy::strict(),
        &mut ledger,
        GroupByExecutionOptions::default(),
    )
    .expect("groupby");
    assert_eq!(grouped.index().labels().len(), 100); // 100 unique groups

    // Step 4: CSV round-trip on a small frame.
    let df = DataFrame::from_series(vec![series_a, series_b]).expect("df");
    let csv = fp_io::write_csv_string(&df).expect("csv");
    let df2 = fp_io::read_csv_str(&csv).expect("re-read");
    assert_eq!(df2.index().len(), n);
}
