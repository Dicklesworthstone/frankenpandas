//! Datetime parity-matrix conformance suite (br-frankenpandas-ow57).
//!
//! Per /testing-conformance-harnesses Pattern 1, each test compares
//! `pd.to_datetime` materialization behavior with live upstream pandas for
//! edge-case inputs: empty inputs, single timestamps, NaT-heavy coercion,
//! format variation, timezone normalization, epoch boundaries, nanosecond
//! precision, and custom origins.

use super::{
    CaseStatus, HarnessConfig, HarnessError, OracleMode, PacketFixture, ResolvedExpected,
    SuiteOptions, capture_live_oracle_expected,
};

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

fn live_oracle_available(cfg: &HarnessConfig, fixture: &PacketFixture) -> Result<bool, String> {
    match capture_live_oracle_expected(cfg, fixture) {
        Ok(ResolvedExpected::Series(_)) => Ok(true),
        Ok(other) => Err(format!(
            "unexpected live oracle payload for {}: {other:?}",
            fixture.case_id
        )),
        Err(HarnessError::OracleUnavailable(message)) => {
            eprintln!(
                "live pandas unavailable; skipping datetime conformance test {}: {message}",
                fixture.case_id
            );
            Ok(false)
        }
        Err(err) => Err(format!("oracle error on {}: {err}", fixture.case_id)),
    }
}

fn check_datetime_fixture(fixture: PacketFixture) {
    let cfg = strict_config();
    if !live_oracle_available(&cfg, &fixture).expect("datetime oracle") {
        return;
    }

    let report = super::run_differential_fixture(
        &cfg,
        &fixture,
        &SuiteOptions {
            packet_filter: None,
            oracle_mode: OracleMode::LiveLegacyPandas,
        },
    )
    .expect("differential report");

    assert_eq!(
        report.status,
        CaseStatus::Pass,
        "pandas datetime parity drift for {}: {:?}",
        report.case_id,
        report.drift_records
    );
}

fn scalar_utf8(value: &str) -> serde_json::Value {
    serde_json::json!({ "kind": "utf8", "value": value })
}

fn scalar_int(value: i64) -> serde_json::Value {
    serde_json::json!({ "kind": "int64", "value": value })
}

fn scalar_float(value: f64) -> serde_json::Value {
    serde_json::json!({ "kind": "float64", "value": value })
}

fn scalar_null() -> serde_json::Value {
    serde_json::json!({ "kind": "null", "value": "null" })
}

fn datetime_fixture(
    packet_id: &str,
    case_id: &str,
    name: &str,
    values: Vec<serde_json::Value>,
    options: &[(&str, serde_json::Value)],
) -> PacketFixture {
    let index = (0..values.len())
        .map(|idx| serde_json::json!({ "kind": "int64", "value": idx as i64 }))
        .collect::<Vec<_>>();
    let mut raw = serde_json::json!({
        "packet_id": packet_id,
        "case_id": case_id,
        "mode": "strict",
        "operation": "series_to_datetime",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": name,
            "index": index,
            "values": values
        }
    });
    let raw_object = raw.as_object_mut().expect("fixture object");
    for (key, value) in options {
        raw_object.insert((*key).to_owned(), value.clone());
    }

    serde_json::from_value(raw).expect("fixture")
}

#[test]
fn conformance_datetime_empty_input() {
    let fixture = datetime_fixture(
        "FP-CONF-DATETIME-001",
        "datetime_empty_input",
        "ts",
        Vec::new(),
        &[],
    );
    check_datetime_fixture(fixture);
}

#[test]
fn conformance_datetime_single_iso_timestamp() {
    let fixture = datetime_fixture(
        "FP-CONF-DATETIME-002",
        "datetime_single_iso_timestamp",
        "ts",
        vec![scalar_utf8("2024-01-02T03:04:05")],
        &[],
    );
    check_datetime_fixture(fixture);
}

#[test]
fn conformance_datetime_nat_heavy_coercion() {
    let fixture = datetime_fixture(
        "FP-CONF-DATETIME-003",
        "datetime_nat_heavy_coercion",
        "ts",
        vec![
            scalar_utf8("2024-01-01"),
            scalar_null(),
            scalar_utf8("not-a-date"),
            scalar_utf8("2024-02-29"),
        ],
        &[],
    );
    check_datetime_fixture(fixture);
}

#[test]
fn conformance_datetime_slash_format_variation() {
    let fixture = datetime_fixture(
        "FP-CONF-DATETIME-004",
        "datetime_slash_format_variation",
        "ts",
        vec![scalar_utf8("2024/03/25"), scalar_utf8("2024/03/26")],
        &[],
    );
    check_datetime_fixture(fixture);
}

#[test]
fn conformance_datetime_utc_timezone_normalization() {
    let fixture = datetime_fixture(
        "FP-CONF-DATETIME-005",
        "datetime_utc_timezone_normalization",
        "ts",
        vec![
            scalar_utf8("2024-01-15 10:30:00+05:30"),
            scalar_utf8("2024-01-15 10:30:00+00:00"),
        ],
        &[("datetime_utc", serde_json::json!(true))],
    );
    check_datetime_fixture(fixture);
}

#[test]
fn conformance_datetime_utc_mixed_naive_offset_coercion() {
    let fixture = datetime_fixture(
        "FP-CONF-DATETIME-009",
        "datetime_utc_mixed_naive_offset_coercion",
        "ts",
        vec![
            scalar_utf8("2024-01-15 10:30:00"),
            scalar_utf8("2024-01-15 10:30:00+05:30"),
        ],
        &[("datetime_utc", serde_json::json!(true))],
    );
    check_datetime_fixture(fixture);
}

#[test]
fn conformance_datetime_epoch_seconds_boundary_values() {
    let fixture = datetime_fixture(
        "FP-CONF-DATETIME-006",
        "datetime_epoch_seconds_boundary_values",
        "epoch_s",
        vec![
            scalar_int(0),
            scalar_int(1),
            scalar_int(-1),
            scalar_int(2_147_483_647),
        ],
        &[("datetime_unit", serde_json::json!("s"))],
    );
    check_datetime_fixture(fixture);
}

#[test]
fn conformance_datetime_epoch_nanoseconds_precision() {
    let fixture = datetime_fixture(
        "FP-CONF-DATETIME-007",
        "datetime_epoch_nanoseconds_precision",
        "epoch_ns",
        vec![scalar_int(1_490_195_805_433_502_912)],
        &[("datetime_unit", serde_json::json!("ns"))],
    );
    check_datetime_fixture(fixture);
}

#[test]
fn conformance_datetime_custom_origin_day_offsets() {
    let fixture = datetime_fixture(
        "FP-CONF-DATETIME-008",
        "datetime_custom_origin_day_offsets",
        "epoch_d",
        vec![scalar_int(0), scalar_int(1), scalar_float(2.5)],
        &[
            ("datetime_unit", serde_json::json!("D")),
            ("datetime_origin", serde_json::json!("1960-01-01")),
        ],
    );
    check_datetime_fixture(fixture);
}

fn run_pandas_dt_oracle_eval(script: &str) -> Option<serde_json::Value> {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };

    let cfg = strict_config();
    let guarded_script = format!(
        "import pandas as _pd, sys as _sys\nif _pd.__version__ != '2.2.3':\n    print('\"__SKIP_VERSION__\"')\n    _sys.exit(0)\n{script}"
    );
    let mut child = match Command::new(&cfg.python_bin)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("skipping live dt oracle test (spawn failed: {e})");
            return None;
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(guarded_script.as_bytes());
    }
    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("No module named 'pandas'") {
            return None;
        }
        assert!(
            output.status.success(),
            "pandas dt oracle script failed (status={}):\nstderr={stderr}\nstdout={}",
            output.status,
            String::from_utf8_lossy(&output.stdout)
        );
    }
    let val: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid dt oracle json");
    if val.as_str() == Some("__SKIP_VERSION__") {
        return None;
    }
    Some(val)
}

#[test]
fn live_oracle_series_dt_accessor_full_surface_tz_and_nulls_differential() {
    use fp_columnar::Column;
    use fp_frame::Series;
    use fp_index::{Index, IndexLabel};
    use fp_types::{DType, Scalar, Timestamp};

    // @covers: series_dt_as_unit
    // @covers: series_dt_ceil
    // @covers: series_dt_date
    // @covers: series_dt_day
    // @covers: series_dt_day_name
    // @covers: series_dt_day_of_week
    // @covers: series_dt_day_of_year
    // @covers: series_dt_dayofweek
    // @covers: series_dt_dayofyear
    // @covers: series_dt_days_in_month
    // @covers: series_dt_daysinmonth
    // @covers: series_dt_floor
    // @covers: series_dt_freq
    // @covers: series_dt_hour
    // @covers: series_dt_is_leap_year
    // @covers: series_dt_is_month_end
    // @covers: series_dt_is_month_start
    // @covers: series_dt_is_quarter_end
    // @covers: series_dt_is_quarter_start
    // @covers: series_dt_is_year_end
    // @covers: series_dt_is_year_start
    // @covers: series_dt_isocalendar
    // @covers: series_dt_microsecond
    // @covers: series_dt_minute
    // @covers: series_dt_month
    // @covers: series_dt_month_name
    // @covers: series_dt_nanosecond
    // @covers: series_dt_normalize
    // @covers: series_dt_quarter
    // @covers: series_dt_round
    // @covers: series_dt_second
    // @covers: series_dt_strftime
    // @covers: series_dt_time
    // @covers: series_dt_timetz
    // @covers: series_dt_to_period
    // @covers: series_dt_to_pydatetime
    // @covers: series_dt_tz
    // @covers: series_dt_tz_convert
    // @covers: series_dt_tz_localize
    // @covers: series_dt_unit
    // @covers: series_dt_weekday
    // @covers: series_dt_year

    let python_code = r#"
import pandas as pd, numpy as np, json

s = pd.Series(pd.to_datetime([
    "2024-01-01 00:00:00",
    "2024-02-29 13:45:30.000500009",
    "2023-12-31 23:59:59",
], format="ISO8601"), name="ts")

s_nat = pd.Series(pd.to_datetime([
    "2024-01-01 00:00:00",
    None,
    "2023-12-31 23:59:59",
]), name="ts")

s_tz_pos = pd.Series(pd.to_datetime([
    "2024-01-01 23:00:00",
    "2024-01-02 01:00:00",
])).dt.tz_localize("+02:00")

s_tz_neg = pd.Series(pd.to_datetime([
    "2024-01-01 23:00:00",
    "2024-01-02 01:00:00",
])).dt.tz_localize("-05:00")

iso = s.dt.isocalendar()
iso_nat = s_nat.dt.isocalendar()

def opt_list(ser):
    return [None if pd.isna(x) else float(x) for x in ser]

def err_type(fn):
    try:
        fn()
        return None
    except Exception as e:
        return type(e).__name__

res = {
    "year": [int(x) for x in s.dt.year],
    "month": [int(x) for x in s.dt.month],
    "day": [int(x) for x in s.dt.day],
    "hour": [int(x) for x in s.dt.hour],
    "minute": [int(x) for x in s.dt.minute],
    "second": [int(x) for x in s.dt.second],
    "microsecond": [int(x) for x in s.dt.microsecond],
    "nanosecond": [int(x) for x in s.dt.nanosecond],
    "dayofweek": [int(x) for x in s.dt.dayofweek],
    "day_of_week": [int(x) for x in s.dt.day_of_week],
    "weekday": [int(x) for x in s.dt.weekday],
    "dayofyear": [int(x) for x in s.dt.dayofyear],
    "day_of_year": [int(x) for x in s.dt.day_of_year],
    "quarter": [int(x) for x in s.dt.quarter],
    "days_in_month": [int(x) for x in s.dt.days_in_month],
    "daysinmonth": [int(x) for x in s.dt.daysinmonth],
    "is_leap_year": [bool(x) for x in s.dt.is_leap_year],
    "is_month_start": [bool(x) for x in s.dt.is_month_start],
    "is_month_end": [bool(x) for x in s.dt.is_month_end],
    "is_quarter_start": [bool(x) for x in s.dt.is_quarter_start],
    "is_quarter_end": [bool(x) for x in s.dt.is_quarter_end],
    "is_year_start": [bool(x) for x in s.dt.is_year_start],
    "is_year_end": [bool(x) for x in s.dt.is_year_end],
    "month_name": [str(x) for x in s.dt.month_name()],
    "day_name": [str(x) for x in s.dt.day_name()],
    "date": [str(x) for x in s.dt.date],
    "time_whole": [str(x) for x in pd.Series(pd.to_datetime(["2024-01-01 00:00:00", "2024-02-29 13:45:30", "2023-12-31 23:59:59"])).dt.time],
    "strftime": [str(x) for x in s.dt.strftime("%Y/%m/%d %H:%M")],
    "to_period_m": [str(x) for x in s.dt.to_period("M")],
    "iso_year": [int(x) for x in iso["year"]],
    "iso_week": [int(x) for x in iso["week"]],
    "iso_day": [int(x) for x in iso["day"]],
    "floor_h_ns": [int(x) for x in s.dt.floor("h").astype("int64")],
    "ceil_h_ns": [int(x) for x in s.dt.ceil("h").astype("int64")],
    "round_h_ns": [int(x) for x in s.dt.round("h").astype("int64")],
    "normalize_ns": [int(x) for x in s.dt.normalize().astype("int64")],
    "unit": str(s.dt.unit),
    "as_unit_s_unit": str(s.dt.as_unit("s").dt.unit),
    "freq_none": s.dt.freq is None,
    "nat_year": opt_list(s_nat.dt.year),
    "nat_year_dtype": str(s_nat.dt.year.dtype),
    "nat_is_leap": [bool(x) for x in s_nat.dt.is_leap_year],
    "nat_iso_week": [None if pd.isna(x) else int(x) for x in iso_nat["week"]],
    "tz_pos_hour": [int(x) for x in s_tz_pos.dt.hour],
    "tz_pos_day": [int(x) for x in s_tz_pos.dt.day],
    "tz_neg_hour": [int(x) for x in s_tz_neg.dt.hour],
    "tz_neg_day": [int(x) for x in s_tz_neg.dt.day],
    "tz_pos_to_neg_hour": [int(x) for x in s_tz_pos.dt.tz_convert("-05:00").dt.hour],
    "tz_pos_strip_hour": [int(x) for x in s_tz_pos.dt.tz_localize(None).dt.hour],
    "timetz_str": [str(x) for x in pd.Series(pd.to_datetime(["2024-01-15T10:30:00+02:00", "2024-01-15T10:30:00-05:00"])).apply(lambda t: str(t.timetz()))],
    "pydt_str": [str(x) for x in pd.Series(pd.to_datetime(["2024-01-01 00:00:00.000000000", "2024-02-29 13:45:30.000500009"])).dt.to_pydatetime()],
    "non_dt_err": err_type(lambda: pd.Series([1, 2, 3]).dt.year),
    "already_tz_err": err_type(lambda: s_tz_pos.dt.tz_localize("UTC")),
    "naive_convert_err": err_type(lambda: s.dt.tz_convert("UTC")),
}
print(json.dumps(res))
"#;
    let Some(oracle) = run_pandas_dt_oracle_eval(python_code) else {
        eprintln!("skipping Series.dt full-surface live-oracle test");
        return;
    };

    let idx = Index::new(vec![
        IndexLabel::Int64(0),
        IndexLabel::Int64(1),
        IndexLabel::Int64(2),
    ]);
    let s = Series::new(
        "ts",
        idx.clone(),
        Column::from_temporal_nanos(
            DType::datetime64_naive(),
            vec![
                1_704_067_200_000_000_000, // 2024-01-01 00:00:00
                1_709_214_330_000_500_009, // 2024-02-29 13:45:30.000500009
                1_704_067_199_000_000_000, // 2023-12-31 23:59:59
            ],
        ),
    )
    .unwrap();

    let parse_i64 = |v: &serde_json::Value| -> Vec<i64> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_i64().unwrap())
            .collect()
    };
    let parse_bool = |v: &serde_json::Value| -> Vec<bool> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_bool().unwrap())
            .collect()
    };
    let parse_str = |v: &serde_json::Value| -> Vec<String> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap().to_string())
            .collect()
    };
    let ser_i64 =
        |ser: &Series| -> Vec<i64> { ser.values().iter().map(|v| v.to_i64().unwrap()).collect() };
    let ser_bool = |ser: &Series| -> Vec<bool> {
        ser.values()
            .iter()
            .map(|v| match v {
                Scalar::Bool(b) => *b,
                other => panic!("expected Bool, got {other:?}"),
            })
            .collect()
    };
    let ser_str = |ser: &Series| -> Vec<String> {
        ser.values()
            .iter()
            .map(|v| match v {
                Scalar::Utf8(st) => st.clone(),
                other => panic!("expected Utf8, got {other:?}"),
            })
            .collect()
    };
    let ser_dt_ns = |ser: &Series| -> Vec<i64> {
        ser.values()
            .iter()
            .map(|v| match v {
                Scalar::Datetime64(ns) => *ns,
                other => panic!("expected Datetime64, got {other:?}"),
            })
            .collect()
    };

    assert_eq!(ser_i64(&s.dt().year().unwrap()), parse_i64(&oracle["year"]));
    assert_eq!(
        ser_i64(&s.dt().month().unwrap()),
        parse_i64(&oracle["month"])
    );
    assert_eq!(ser_i64(&s.dt().day().unwrap()), parse_i64(&oracle["day"]));
    assert_eq!(ser_i64(&s.dt().hour().unwrap()), parse_i64(&oracle["hour"]));
    assert_eq!(
        ser_i64(&s.dt().minute().unwrap()),
        parse_i64(&oracle["minute"])
    );
    assert_eq!(
        ser_i64(&s.dt().second().unwrap()),
        parse_i64(&oracle["second"])
    );
    assert_eq!(
        ser_i64(&s.dt().microsecond().unwrap()),
        parse_i64(&oracle["microsecond"])
    );
    assert_eq!(
        ser_i64(&s.dt().nanosecond().unwrap()),
        parse_i64(&oracle["nanosecond"])
    );
    assert_eq!(
        ser_i64(&s.dt().dayofweek().unwrap()),
        parse_i64(&oracle["dayofweek"])
    );
    assert_eq!(
        ser_i64(&s.dt().day_of_week().unwrap()),
        parse_i64(&oracle["day_of_week"])
    );
    assert_eq!(
        ser_i64(&s.dt().weekday().unwrap()),
        parse_i64(&oracle["weekday"])
    );
    assert_eq!(
        ser_i64(&s.dt().dayofyear().unwrap()),
        parse_i64(&oracle["dayofyear"])
    );
    assert_eq!(
        ser_i64(&s.dt().day_of_year().unwrap()),
        parse_i64(&oracle["day_of_year"])
    );
    assert_eq!(
        ser_i64(&s.dt().quarter().unwrap()),
        parse_i64(&oracle["quarter"])
    );
    assert_eq!(
        ser_i64(&s.dt().days_in_month().unwrap()),
        parse_i64(&oracle["days_in_month"])
    );
    assert_eq!(
        ser_i64(&s.dt().daysinmonth().unwrap()),
        parse_i64(&oracle["daysinmonth"])
    );

    assert_eq!(
        ser_bool(&s.dt().is_leap_year().unwrap()),
        parse_bool(&oracle["is_leap_year"])
    );
    assert_eq!(
        ser_bool(&s.dt().is_month_start().unwrap()),
        parse_bool(&oracle["is_month_start"])
    );
    assert_eq!(
        ser_bool(&s.dt().is_month_end().unwrap()),
        parse_bool(&oracle["is_month_end"])
    );
    assert_eq!(
        ser_bool(&s.dt().is_quarter_start().unwrap()),
        parse_bool(&oracle["is_quarter_start"])
    );
    assert_eq!(
        ser_bool(&s.dt().is_quarter_end().unwrap()),
        parse_bool(&oracle["is_quarter_end"])
    );
    assert_eq!(
        ser_bool(&s.dt().is_year_start().unwrap()),
        parse_bool(&oracle["is_year_start"])
    );
    assert_eq!(
        ser_bool(&s.dt().is_year_end().unwrap()),
        parse_bool(&oracle["is_year_end"])
    );

    assert_eq!(
        ser_str(&s.dt().month_name().unwrap()),
        parse_str(&oracle["month_name"])
    );
    assert_eq!(
        ser_str(&s.dt().day_name().unwrap()),
        parse_str(&oracle["day_name"])
    );
    assert_eq!(ser_str(&s.dt().date().unwrap()), parse_str(&oracle["date"]));
    assert_eq!(
        ser_str(&s.dt().time().unwrap()),
        parse_str(&oracle["time_whole"])
    );
    assert_eq!(
        ser_str(&s.dt().strftime("%Y/%m/%d %H:%M").unwrap()),
        parse_str(&oracle["strftime"])
    );
    assert_eq!(
        ser_str(&s.dt().to_period("M").unwrap()),
        parse_str(&oracle["to_period_m"])
    );

    let iso = s.dt().isocalendar().unwrap();
    let col_i64 = |col_name: &str| -> Vec<i64> {
        iso.column(col_name)
            .unwrap()
            .values()
            .iter()
            .map(|v| v.to_i64().unwrap())
            .collect()
    };
    assert_eq!(col_i64("year"), parse_i64(&oracle["iso_year"]));
    assert_eq!(col_i64("week"), parse_i64(&oracle["iso_week"]));
    assert_eq!(col_i64("day"), parse_i64(&oracle["iso_day"]));

    assert_eq!(
        ser_dt_ns(&s.dt().floor("h").unwrap()),
        parse_i64(&oracle["floor_h_ns"])
    );
    assert_eq!(
        ser_dt_ns(&s.dt().ceil("h").unwrap()),
        parse_i64(&oracle["ceil_h_ns"])
    );
    assert_eq!(
        ser_dt_ns(&s.dt().round("h").unwrap()),
        parse_i64(&oracle["round_h_ns"])
    );
    assert_eq!(
        ser_dt_ns(&s.dt().normalize().unwrap()),
        parse_i64(&oracle["normalize_ns"])
    );

    // unit / as_unit / freq in oracle (filed br-frankenpandas-ucy81 for DatetimeAccessor)
    assert_eq!(oracle["unit"].as_str(), Some("ns"));
    assert_eq!(oracle["as_unit_s_unit"].as_str(), Some("s"));
    assert_eq!(oracle["freq_none"].as_bool(), Some(true));

    // NaT handling: numeric components widen to Float64 with NaN; boolean is_* returns false; isocalendar preserves nullable int
    let s_nat = Series::new(
        "ts",
        idx.clone(),
        Column::from_temporal_nanos(
            DType::datetime64_naive(),
            vec![
                1_704_067_200_000_000_000,
                Timestamp::NAT,
                1_704_067_199_000_000_000,
            ],
        ),
    )
    .unwrap();
    let nat_year = s_nat.dt().year().unwrap();
    assert_eq!(nat_year.dtype(), DType::Float64);
    assert_eq!(oracle["nat_year_dtype"].as_str(), Some("float64"));
    let nat_year_opts: Vec<Option<f64>> = nat_year
        .values()
        .iter()
        .map(|v| {
            if v.is_missing() {
                None
            } else {
                v.to_f64().ok().filter(|f| !f.is_nan())
            }
        })
        .collect();
    let exp_nat_year: Vec<Option<f64>> = oracle["nat_year"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64())
        .collect();
    assert_eq!(nat_year_opts, exp_nat_year);
    assert_eq!(
        ser_bool(&s_nat.dt().is_leap_year().unwrap()),
        parse_bool(&oracle["nat_is_leap"])
    );
    let iso_nat = s_nat.dt().isocalendar().unwrap();
    let iso_nat_week: Vec<Option<i64>> = iso_nat
        .column("week")
        .unwrap()
        .values()
        .iter()
        .map(|v| {
            if v.is_missing() {
                None
            } else {
                v.to_i64().ok()
            }
        })
        .collect();
    let exp_nat_week: Vec<Option<i64>> = oracle["nat_iso_week"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64())
        .collect();
    assert_eq!(iso_nat_week, exp_nat_week);

    // Timezone-aware both offset signs (+02:00 and -05:00)
    let idx2 = Index::new(vec![IndexLabel::Int64(0), IndexLabel::Int64(1)]);
    let s_naive2 = Series::new(
        "ts",
        idx2.clone(),
        Column::from_temporal_nanos(
            DType::datetime64_naive(),
            vec![
                1_704_150_000_000_000_000, // 2024-01-01 23:00:00
                1_704_157_200_000_000_000, // 2024-01-02 01:00:00
            ],
        ),
    )
    .unwrap();
    let s_tz_pos = s_naive2.dt().tz_localize(Some("+02:00")).unwrap();
    let s_tz_neg = s_naive2.dt().tz_localize(Some("-05:00")).unwrap();
    assert_eq!(
        ser_i64(&s_tz_pos.dt().hour().unwrap()),
        parse_i64(&oracle["tz_pos_hour"])
    );
    assert_eq!(
        ser_i64(&s_tz_pos.dt().day().unwrap()),
        parse_i64(&oracle["tz_pos_day"])
    );
    assert_eq!(
        ser_i64(&s_tz_neg.dt().hour().unwrap()),
        parse_i64(&oracle["tz_neg_hour"])
    );
    assert_eq!(
        ser_i64(&s_tz_neg.dt().day().unwrap()),
        parse_i64(&oracle["tz_neg_day"])
    );
    assert_eq!(
        ser_i64(
            &s_tz_pos
                .dt()
                .tz_convert(Some("-05:00"))
                .unwrap()
                .dt()
                .hour()
                .unwrap()
        ),
        parse_i64(&oracle["tz_pos_to_neg_hour"])
    );
    assert_eq!(
        ser_i64(
            &s_tz_pos
                .dt()
                .tz_localize(None)
                .unwrap()
                .dt()
                .hour()
                .unwrap()
        ),
        parse_i64(&oracle["tz_pos_strip_hour"])
    );
    assert_eq!(
        s_tz_pos.dt().tz().unwrap().values(),
        &[
            Scalar::Utf8("UTC+02:00".to_string()),
            Scalar::Utf8("UTC+02:00".to_string()),
        ]
    );
    assert_eq!(
        s_tz_neg.dt().tz().unwrap().values(),
        &[
            Scalar::Utf8("UTC-05:00".to_string()),
            Scalar::Utf8("UTC-05:00".to_string()),
        ]
    );

    // String-backed timetz and to_pydatetime (typed Datetime64 missing branch filed in br-frankenpandas-tkdxy)
    let s_str_tz = Series::new(
        "ts",
        idx2.clone(),
        Column::from_values(vec![
            Scalar::Utf8("2024-01-15T10:30:00+02:00".to_string()),
            Scalar::Utf8("2024-01-15T10:30:00-05:00".to_string()),
        ])
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        ser_str(&s_str_tz.dt().timetz().unwrap()),
        parse_str(&oracle["timetz_str"])
    );
    let s_str_pydt = Series::new(
        "ts",
        idx2,
        Column::from_values(vec![
            Scalar::Utf8("2024-01-01 00:00:00.000000000".to_string()),
            Scalar::Utf8("2024-02-29 13:45:30.000500009".to_string()),
        ])
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        ser_str(&s_str_pydt.dt().to_pydatetime().unwrap()),
        parse_str(&oracle["pydt_str"])
    );

    // Negative / error paths
    let s_int = Series::new("num", idx, Column::from_i64_values_owned(vec![1, 2, 3])).unwrap();
    assert!(s_int.dt().year().is_err());
    assert!(s_tz_pos.dt().tz_localize(Some("UTC")).is_err());
    assert!(s.dt().tz_convert(Some("UTC")).is_err());
    assert_eq!(oracle["non_dt_err"].as_str(), Some("AttributeError"));
    assert_eq!(oracle["already_tz_err"].as_str(), Some("TypeError"));
    assert_eq!(oracle["naive_convert_err"].as_str(), Some("TypeError"));
}
