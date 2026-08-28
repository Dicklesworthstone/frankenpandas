//! Series parity-matrix conformance suite (br-frankenpandas-3qt8).
//!
//! Per /testing-conformance-harnesses skill Pattern 4 (spec-derived test
//! matrix). Each test picks one Series-family operation and runs it
//! through an edge-case input (empty, single-row, all-NaN, duplicate
//! labels, misaligned indexes). The live pandas oracle is the reference
//! implementation; our Rust result must match via the standard
//! `compare_series_expected` / scalar comparators.
//!
//! Each test skips gracefully (no failure) when the live oracle isn't
//! available — matches the convention of sibling `live_oracle_*` tests.

use super::{
    EvidenceLedger, FixtureExpectedSeries, FrameError, HarnessConfig, HarnessError, IndexLabel,
    NullKind, PacketFixture, ResolvedExpected, RuntimePolicy, Scalar, Series, build_series,
    capture_live_oracle_expected, compare_scalar, compare_series_expected,
};

fn oracle_series_expected(
    cfg: &HarnessConfig,
    fixture: &PacketFixture,
) -> Result<Option<FixtureExpectedSeries>, String> {
    match capture_live_oracle_expected(cfg, fixture) {
        Ok(ResolvedExpected::Series(series)) => Ok(Some(series)),
        Ok(other) => Err(format!("expected series payload, got {other:?}")),
        Err(HarnessError::OracleUnavailable(message)) => {
            eprintln!(
                "live pandas unavailable; skipping Series conformance test {}: {message}",
                fixture.case_id
            );
            Ok(None)
        }
        Err(err) => Err(format!("oracle error on {}: {err}", fixture.case_id)),
    }
}

fn oracle_scalar_expected(
    cfg: &HarnessConfig,
    fixture: &PacketFixture,
) -> Result<Option<Scalar>, String> {
    match capture_live_oracle_expected(cfg, fixture) {
        Ok(ResolvedExpected::Scalar(scalar)) => Ok(Some(scalar)),
        Ok(other) => Err(format!("expected scalar payload, got {other:?}")),
        Err(HarnessError::OracleUnavailable(message)) => {
            eprintln!(
                "live pandas unavailable; skipping Series conformance test {}: {message}",
                fixture.case_id
            );
            Ok(None)
        }
        Err(err) => Err(format!("oracle error on {}: {err}", fixture.case_id)),
    }
}

fn strict_config() -> HarnessConfig {
    let mut cfg = HarnessConfig::default_paths();
    // br-frankenpandas-l7r1p: without this every fixture in this file SKIPS.
    // The legacy oracle root (`legacy_pandas_code/pandas`) does not exist on
    // this host, so `capture_live_oracle_expected` returns OracleUnavailable
    // and the check helper returns before comparing anything -- green while
    // executing no differential at all. The live_oracle_* suites already opt
    // into the SYSTEM pandas (2.2.3); the conformance_* suites never did.
    cfg.allow_system_pandas_fallback = true;
    cfg
}

/// Helper: run series_add against the oracle + compare.
fn check_series_add(fixture: PacketFixture) {
    let cfg = strict_config();
    let Some(expected) = oracle_series_expected(&cfg, &fixture).expect("series oracle") else {
        return;
    };
    let left = build_series(fixture.left.as_ref().expect("left series")).expect("left build");
    let right = build_series(fixture.right.as_ref().expect("right series")).expect("right build");
    let policy = RuntimePolicy::strict();
    let mut ledger = EvidenceLedger::new();
    let actual = left
        .add_with_policy(&right, &policy, &mut ledger)
        .expect("series_add");
    compare_series_expected(&actual, &expected).expect("pandas series_add parity");
}

fn check_series_mode(fixture: PacketFixture) {
    let cfg = strict_config();
    let Some(expected) = oracle_series_expected(&cfg, &fixture).expect("series oracle") else {
        return;
    };
    let series = build_series(fixture.left.as_ref().expect("left series")).expect("series build");
    // Default dropna=true matches pandas Series.mode default.
    let actual = series.mode().expect("series_mode");
    compare_series_expected(&actual, &expected).expect("pandas series_mode parity");
}

fn check_series_nunique(fixture: PacketFixture) {
    let cfg = strict_config();
    let Some(expected) = oracle_scalar_expected(&cfg, &fixture).expect("scalar oracle") else {
        return;
    };
    let series = build_series(fixture.left.as_ref().expect("left series")).expect("series build");
    let actual = Scalar::Int64(series.nunique() as i64);
    compare_scalar(&actual, &expected, "series_nunique").expect("pandas series_nunique parity");
}

// ── series_add edge matrix ────────────────────────────────────────────

#[test]
fn conformance_series_add_empty_pair() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-SERIES-ADD-001",
        "case_id": "series_add_empty_pair",
        "mode": "strict",
        "operation": "series_add",
        "oracle_source": "live_legacy_pandas",
        "left":  { "name": "l", "index": [], "values": [] },
        "right": { "name": "r", "index": [], "values": [] }
    }))
    .expect("fixture");
    check_series_add(fixture);
}

#[test]
fn conformance_series_add_single_row() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-SERIES-ADD-002",
        "case_id": "series_add_single_row",
        "mode": "strict",
        "operation": "series_add",
        "oracle_source": "live_legacy_pandas",
        "left":  { "name": "l", "index": [{ "kind": "int64", "value": 0 }],
                   "values": [{ "kind": "float64", "value": 42.0 }] },
        "right": { "name": "r", "index": [{ "kind": "int64", "value": 0 }],
                   "values": [{ "kind": "float64", "value": 8.0 }] }
    }))
    .expect("fixture");
    check_series_add(fixture);
}

#[test]
fn conformance_series_add_all_nan() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-SERIES-ADD-003",
        "case_id": "series_add_all_nan",
        "mode": "strict",
        "operation": "series_add",
        "oracle_source": "live_legacy_pandas",
        "left":  { "name": "l", "index": [
                       { "kind": "int64", "value": 0 },
                       { "kind": "int64", "value": 1 },
                       { "kind": "int64", "value": 2 }],
                   "values": [
                       { "kind": "null", "value": "na_n" },
                       { "kind": "null", "value": "na_n" },
                       { "kind": "null", "value": "na_n" }] },
        "right": { "name": "r", "index": [
                       { "kind": "int64", "value": 0 },
                       { "kind": "int64", "value": 1 },
                       { "kind": "int64", "value": 2 }],
                   "values": [
                       { "kind": "null", "value": "na_n" },
                       { "kind": "float64", "value": 1.0 },
                       { "kind": "null", "value": "na_n" }] }
    }))
    .expect("fixture");
    check_series_add(fixture);
}

#[test]
fn conformance_series_add_duplicate_labels() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-SERIES-ADD-004",
        "case_id": "series_add_duplicate_labels",
        "mode": "strict",
        "operation": "series_add",
        "oracle_source": "live_legacy_pandas",
        "left":  { "name": "l", "index": [
                       { "kind": "int64", "value": 1 },
                       { "kind": "int64", "value": 1 }],
                   "values": [
                       { "kind": "float64", "value": 10.0 },
                       { "kind": "float64", "value": 20.0 }] },
        "right": { "name": "r", "index": [
                       { "kind": "int64", "value": 1 },
                       { "kind": "int64", "value": 1 }],
                   "values": [
                       { "kind": "float64", "value": 100.0 },
                       { "kind": "float64", "value": 200.0 }] }
    }))
    .expect("fixture");
    check_series_add(fixture);
}

#[test]
fn conformance_series_add_misaligned_indexes() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-SERIES-ADD-005",
        "case_id": "series_add_misaligned_indexes",
        "mode": "strict",
        "operation": "series_add",
        "oracle_source": "live_legacy_pandas",
        "left":  { "name": "l", "index": [
                       { "kind": "int64", "value": 0 },
                       { "kind": "int64", "value": 1 }],
                   "values": [
                       { "kind": "int64", "value": 10 },
                       { "kind": "int64", "value": 20 }] },
        "right": { "name": "r", "index": [
                       { "kind": "int64", "value": 2 },
                       { "kind": "int64", "value": 3 }],
                   "values": [
                       { "kind": "int64", "value": 100 },
                       { "kind": "int64", "value": 200 }] }
    }))
    .expect("fixture");
    check_series_add(fixture);
}

// ── series_mode edge matrix ───────────────────────────────────────────

#[test]
fn conformance_series_mode_empty() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-SERIES-MODE-001",
        "case_id": "series_mode_empty",
        "mode": "strict",
        "operation": "series_mode",
        "oracle_source": "live_legacy_pandas",
        "left": { "name": "s", "index": [], "values": [] }
    }))
    .expect("fixture");
    check_series_mode(fixture);
}

#[test]
fn conformance_series_mode_unique_no_mode() {
    // All values distinct → pandas returns every value (everything ties at 1).
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-SERIES-MODE-002",
        "case_id": "series_mode_unique_no_mode",
        "mode": "strict",
        "operation": "series_mode",
        "oracle_source": "live_legacy_pandas",
        "left": { "name": "s", "index": [
                      { "kind": "int64", "value": 0 },
                      { "kind": "int64", "value": 1 },
                      { "kind": "int64", "value": 2 }],
                  "values": [
                      { "kind": "int64", "value": 1 },
                      { "kind": "int64", "value": 2 },
                      { "kind": "int64", "value": 3 }] }
    }))
    .expect("fixture");
    check_series_mode(fixture);
}

// ── series_nunique edge matrix ───────────────────────────────────────

#[test]
fn conformance_series_nunique_empty() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-SERIES-NUNIQUE-001",
        "case_id": "series_nunique_empty",
        "mode": "strict",
        "operation": "series_nunique",
        "oracle_source": "live_legacy_pandas",
        "left": { "name": "s", "index": [], "values": [] }
    }))
    .expect("fixture");
    check_series_nunique(fixture);
}

#[test]
fn conformance_series_nunique_all_nan() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-SERIES-NUNIQUE-002",
        "case_id": "series_nunique_all_nan",
        "mode": "strict",
        "operation": "series_nunique",
        "oracle_source": "live_legacy_pandas",
        "left": { "name": "s", "index": [
                      { "kind": "int64", "value": 0 },
                      { "kind": "int64", "value": 1 },
                      { "kind": "int64", "value": 2 }],
                  "values": [
                      { "kind": "null", "value": "na_n" },
                      { "kind": "null", "value": "na_n" },
                      { "kind": "null", "value": "na_n" }] }
    }))
    .expect("fixture");
    check_series_nunique(fixture);
}

#[test]
fn conformance_series_nunique_all_duplicates() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-SERIES-NUNIQUE-003",
        "case_id": "series_nunique_all_duplicates",
        "mode": "strict",
        "operation": "series_nunique",
        "oracle_source": "live_legacy_pandas",
        "left": { "name": "s", "index": [
                      { "kind": "int64", "value": 0 },
                      { "kind": "int64", "value": 1 },
                      { "kind": "int64", "value": 2 }],
                  "values": [
                      { "kind": "utf8", "value": "x" },
                      { "kind": "utf8", "value": "x" },
                      { "kind": "utf8", "value": "x" }] }
    }))
    .expect("fixture");
    check_series_nunique(fixture);
}

// ── list/struct accessor compatibility contracts ─────────────────────

#[test]
fn conformance_series_list_json_accessor_contract_zzbqc() {
    let series = Series::from_values(
        "items",
        vec![
            IndexLabel::Utf8("a".into()),
            IndexLabel::Utf8("b".into()),
            IndexLabel::Utf8("c".into()),
            IndexLabel::Utf8("d".into()),
        ],
        vec![
            Scalar::Utf8(r#"[1,"x",null]"#.into()),
            Scalar::Null(NullKind::Null),
            Scalar::Utf8("[]".into()),
            Scalar::Utf8("[true,2.5]".into()),
        ],
    )
    .expect("list contract series");

    let accessor = series.list();
    assert!(accessor.is_supported());

    let lengths = accessor.len().expect("list lengths");
    assert_eq!(
        lengths.values(),
        &[
            Scalar::Int64(3),
            Scalar::Null(NullKind::Null),
            Scalar::Int64(0),
            Scalar::Int64(2),
        ]
    );
    assert_eq!(lengths.index().labels(), series.index().labels());

    let second = accessor.get(1).expect("list get");
    assert_eq!(
        second.values(),
        &[
            Scalar::Utf8("x".into()),
            Scalar::Null(NullKind::Null),
            Scalar::Null(NullKind::Null),
            Scalar::Float64(2.5),
        ]
    );

    let missing = accessor.get(9).expect("list missing index");
    assert_eq!(
        missing.values(),
        &[
            Scalar::Null(NullKind::Null),
            Scalar::Null(NullKind::Null),
            Scalar::Null(NullKind::Null),
            Scalar::Null(NullKind::Null),
        ]
    );

    let flattened = accessor.flatten().expect("list flatten");
    assert_eq!(
        flattened.values(),
        &[
            Scalar::Int64(1),
            Scalar::Utf8("x".into()),
            Scalar::Null(NullKind::Null),
            Scalar::Bool(true),
            Scalar::Float64(2.5),
        ]
    );

    let scalar_series =
        Series::from_values("bad", vec![IndexLabel::Int64(0)], vec![Scalar::Int64(1)])
            .expect("bad list series");
    let err = scalar_series.list().len().expect_err("non-list rejects");
    assert!(
        matches!(err, FrameError::CompatibilityRejected(msg) if msg.contains("UTF-8 JSON arrays") && msg.contains("position 0"))
    );

    let nested_series = Series::from_values(
        "nested",
        vec![IndexLabel::Int64(0)],
        vec![Scalar::Utf8("[[1]]".into())],
    )
    .expect("nested list series");
    let err = nested_series
        .list()
        .flatten()
        .expect_err("nested list rejects");
    assert!(
        matches!(err, FrameError::CompatibilityRejected(msg) if msg.contains("nested JSON arrays/objects"))
    );
}

#[test]
fn conformance_series_struct_json_accessor_contract_zzbqc() {
    let series = Series::from_values(
        "records",
        vec![
            IndexLabel::Utf8("a".into()),
            IndexLabel::Utf8("b".into()),
            IndexLabel::Utf8("c".into()),
            IndexLabel::Utf8("d".into()),
        ],
        vec![
            Scalar::Utf8(r#"{"id":1,"name":"Ada"}"#.into()),
            Scalar::Utf8(r#"{"id":null}"#.into()),
            Scalar::Null(NullKind::Null),
            Scalar::Utf8("{}".into()),
        ],
    )
    .expect("struct contract series");

    let accessor = series.r#struct();
    assert!(accessor.is_supported());
    assert_eq!(
        accessor.field_names().expect("field names"),
        vec!["id", "name"]
    );

    let ids = accessor.field("id").expect("id field");
    assert_eq!(ids.name(), "id");
    assert_eq!(
        ids.values(),
        &[
            Scalar::Int64(1),
            Scalar::Null(NullKind::Null),
            Scalar::Null(NullKind::Null),
            Scalar::Null(NullKind::Null),
        ]
    );
    assert_eq!(ids.index().labels(), series.index().labels());

    let names = accessor.field("name").expect("name field");
    assert_eq!(
        names.values(),
        &[
            Scalar::Utf8("Ada".into()),
            Scalar::Null(NullKind::Null),
            Scalar::Null(NullKind::Null),
            Scalar::Null(NullKind::Null),
        ]
    );

    let missing = accessor.field("missing").expect("missing field");
    assert_eq!(
        missing.values(),
        &[
            Scalar::Null(NullKind::Null),
            Scalar::Null(NullKind::Null),
            Scalar::Null(NullKind::Null),
            Scalar::Null(NullKind::Null),
        ]
    );

    let array_series = Series::from_values(
        "array",
        vec![IndexLabel::Int64(0)],
        vec![Scalar::Utf8("[1]".into())],
    )
    .expect("array struct series");
    let err = array_series
        .r#struct()
        .field_names()
        .expect_err("non-struct rejects");
    assert!(
        matches!(err, FrameError::CompatibilityRejected(msg) if msg.contains("non-object JSON") && msg.contains("position 0"))
    );

    let nested_series = Series::from_values(
        "nested",
        vec![IndexLabel::Int64(0)],
        vec![Scalar::Utf8(r#"{"payload":{"x":1}}"#.into())],
    )
    .expect("nested struct series");
    let err = nested_series
        .r#struct()
        .field("payload")
        .expect_err("nested struct rejects");
    assert!(
        matches!(err, FrameError::CompatibilityRejected(msg) if msg.contains("nested JSON arrays/objects"))
    );
}
