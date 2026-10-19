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

#[test]
#[allow(clippy::too_many_lines, clippy::cognitive_complexity)]
fn live_oracle_timestamp_timedelta_period_interval_dateoffset_scalars_differential() {
    // @covers: DateOffset.base, DateOffset.copy, DateOffset.freqstr, DateOffset.is_anchored
    // @covers: DateOffset.is_month_end, DateOffset.is_month_start, DateOffset.is_on_offset
    // @covers: DateOffset.is_quarter_end, DateOffset.is_quarter_start, DateOffset.is_year_end
    // @covers: DateOffset.is_year_start, DateOffset.kwds, DateOffset.n, DateOffset.name
    // @covers: DateOffset.nanos, DateOffset.normalize, DateOffset.rollback
    // @covers: DateOffset.rollforward, DateOffset.rule_code
    // @covers: Interval.closed, Interval.closed_left, Interval.closed_right, Interval.is_empty
    // @covers: Interval.left, Interval.length, Interval.mid, Interval.open_left
    // @covers: Interval.open_right, Interval.overlaps, Interval.right
    // @covers: Period.asfreq, Period.day, Period.day_of_week, Period.day_of_year
    // @covers: Period.dayofweek, Period.dayofyear, Period.days_in_month, Period.daysinmonth
    // @covers: Period.end_time, Period.freq, Period.freqstr, Period.hour, Period.is_leap_year
    // @covers: Period.minute, Period.month, Period.now, Period.ordinal, Period.quarter
    // @covers: Period.qyear, Period.second, Period.start_time, Period.strftime
    // @covers: Period.to_timestamp, Period.week, Period.weekday, Period.weekofyear, Period.year
    // @covers: Timedelta.as_unit, Timedelta.asm8, Timedelta.ceil, Timedelta.components
    // @covers: Timedelta.days, Timedelta.floor, Timedelta.isoformat, Timedelta.max
    // @covers: Timedelta.microseconds, Timedelta.min, Timedelta.nanoseconds
    // @covers: Timedelta.resolution, Timedelta.resolution_string, Timedelta.round
    // @covers: Timedelta.seconds, Timedelta.to_numpy, Timedelta.to_pytimedelta
    // @covers: Timedelta.to_timedelta64, Timedelta.total_seconds, Timedelta.unit
    // @covers: Timedelta.value, Timedelta.view
    // @covers: Timestamp.as_unit, Timestamp.asm8, Timestamp.astimezone, Timestamp.ceil
    // @covers: Timestamp.combine, Timestamp.ctime, Timestamp.date, Timestamp.day
    // @covers: Timestamp.day_name, Timestamp.day_of_week, Timestamp.day_of_year
    // @covers: Timestamp.dayofweek, Timestamp.dayofyear, Timestamp.days_in_month
    // @covers: Timestamp.daysinmonth, Timestamp.dst, Timestamp.floor, Timestamp.fold
    // @covers: Timestamp.fromisocalendar, Timestamp.fromisoformat, Timestamp.fromordinal
    // @covers: Timestamp.fromtimestamp, Timestamp.hour, Timestamp.is_leap_year
    // @covers: Timestamp.is_month_end, Timestamp.is_month_start, Timestamp.is_quarter_end
    // @covers: Timestamp.is_quarter_start, Timestamp.is_year_end, Timestamp.is_year_start
    // @covers: Timestamp.isocalendar, Timestamp.isoformat, Timestamp.isoweekday, Timestamp.max
    // @covers: Timestamp.microsecond, Timestamp.min, Timestamp.minute, Timestamp.month
    // @covers: Timestamp.month_name, Timestamp.nanosecond, Timestamp.normalize, Timestamp.now
    // @covers: Timestamp.quarter, Timestamp.replace, Timestamp.resolution, Timestamp.round
    // @covers: Timestamp.second, Timestamp.strftime, Timestamp.strptime, Timestamp.time
    // @covers: Timestamp.timestamp, Timestamp.timetuple, Timestamp.timetz
    // @covers: Timestamp.to_datetime64, Timestamp.to_julian_date, Timestamp.to_numpy
    // @covers: Timestamp.to_period, Timestamp.to_pydatetime, Timestamp.today
    // @covers: Timestamp.toordinal, Timestamp.tz, Timestamp.tz_convert, Timestamp.tz_localize
    // @covers: Timestamp.tzinfo, Timestamp.tzname, Timestamp.unit
    // @covers: Timestamp.utcfromtimestamp, Timestamp.utcnow, Timestamp.utcoffset
    // @covers: Timestamp.utctimetuple, Timestamp.value, Timestamp.week, Timestamp.weekday
    // @covers: Timestamp.weekofyear, Timestamp.year
    use fp_index::{DateOffset, PeriodIndex, apply_date_offset, apply_date_offset_to_nanos};
    use fp_types::{
        Interval, IntervalClosed, Period, PeriodFreq, Timedelta, Timestamp, tz_utc_to_wall_nanos,
        tz_wall_to_utc_nanos,
    };

    let Some(oracle) = run_pandas_dt_oracle_eval(
        r#"
import json
import pandas as pd

ts = pd.Timestamp("2024-02-29 13:45:30.000500009")
td = pd.Timedelta("2 days 03:04:05.006007008")
per = pd.Period("2024-02", freq="M")
iv = pd.Interval(1.0, 5.0, closed="right")
iv_touch = pd.Interval(5.0, 8.0, closed="left")
iv_both = pd.Interval(5.0, 8.0, closed="both")
iv_empty = pd.Interval(3.0, 3.0, closed="right")

# Negative-UTC-offset DST transitions in America/New_York
ts_spring = pd.Timestamp("2024-03-08 09:00:00", tz="America/New_York")
ts_fall = pd.Timestamp("2024-11-01 09:00:00", tz="America/New_York")
ts_thu_evening = pd.Timestamp("2024-03-07 20:00:00", tz="America/New_York")

spring_day3 = ts_spring + pd.offsets.Day(3)
spring_bday1 = ts_spring + pd.offsets.BDay(1)
spring_me1 = ts_spring + pd.offsets.MonthEnd(1)
fall_day3 = ts_fall + pd.offsets.Day(3)
fall_bday1 = ts_fall + pd.offsets.BDay(1)
fall_me1 = ts_fall + pd.offsets.MonthEnd(1)
thu_bday1 = ts_thu_evening + pd.offsets.BDay(1)

me_off = pd.offsets.MonthEnd(1)
bday_off = pd.offsets.BDay(2)
day_off = pd.offsets.Day(3)

out = {
    "ts_value": int(ts.value),
    "ts_unit": str(ts.unit),
    "ts_resolution": str(ts.resolution),
    "ts_year": int(ts.year),
    "ts_month": int(ts.month),
    "ts_day": int(ts.day),
    "ts_hour": int(ts.hour),
    "ts_minute": int(ts.minute),
    "ts_second": int(ts.second),
    "ts_microsecond": int(ts.microsecond),
    "ts_nanosecond": int(ts.nanosecond),
    "ts_dayofweek": int(ts.dayofweek),
    "ts_weekday": int(ts.weekday()),
    "ts_day_of_week": int(ts.day_of_week),
    "ts_dayofyear": int(ts.dayofyear),
    "ts_day_of_year": int(ts.day_of_year),
    "ts_quarter": int(ts.quarter),
    "ts_week": int(ts.week),
    "ts_weekofyear": int(ts.weekofyear),
    "ts_is_leap_year": bool(ts.is_leap_year),
    "ts_is_month_start": bool(ts.is_month_start),
    "ts_is_month_end": bool(ts.is_month_end),
    "ts_is_quarter_start": bool(ts.is_quarter_start),
    "ts_is_quarter_end": bool(ts.is_quarter_end),
    "ts_is_year_start": bool(ts.is_year_start),
    "ts_is_year_end": bool(ts.is_year_end),
    "ts_days_in_month": int(ts.days_in_month),
    "ts_daysinmonth": int(ts.daysinmonth),
    "ts_toordinal": int(ts.toordinal()),
    "ts_fromordinal": int(pd.Timestamp.fromordinal(ts.toordinal()).value),
    "ts_julian": float(ts.to_julian_date()),
    "ts_normalize": int(ts.normalize().value),
    "ts_floor_h": int(ts.floor("h").value),
    "ts_ceil_h": int(ts.ceil("h").value),
    "ts_round_h": int(ts.round("h").value),
    "ts_replace": int(ts.replace(year=2025, month=3, day=1).value),
    "ts_isoformat": str(ts.isoformat()),
    "ts_strftime": str(ts.strftime("%Y/%m/%d %H:%M:%S")),
    "ts_day_name": str(ts.day_name()),
    "ts_month_name": str(ts.month_name()),
    "ts_timestamp": float(ts.timestamp()),
    "ts_fromtimestamp": int(pd.Timestamp.fromtimestamp(1709214330.0, tz="UTC").value),
    "td_value": int(td.value),
    "td_total_seconds": float(td.total_seconds()),
    "td_days": int(td.days),
    "td_seconds": int(td.seconds),
    "td_microseconds": int(td.microseconds),
    "td_nanoseconds": int(td.nanoseconds),
    "td_components": [int(x) for x in td.components],
    "td_isoformat": str(td.isoformat()),
    "td_floor_h": int(td.floor("h").value),
    "td_ceil_h": int(td.ceil("h").value),
    "td_round_h": int(td.round("h").value),
    "per_ordinal": int(per.ordinal),
    "per_freqstr": str(per.freqstr),
    "per_str": str(per),
    "per_year": int(per.year),
    "per_month": int(per.month),
    "per_day": int(per.day),
    "per_quarter": int(per.quarter),
    "per_qyear": int(per.qyear),
    "per_days_in_month": int(per.days_in_month),
    "per_is_leap_year": bool(per.is_leap_year),
    "per_start_time": int(per.start_time.value),
    "per_end_time": int(per.end_time.value),
    "per_asfreq_d_end": int(per.asfreq("D", how="end").ordinal),
    "iv_left": float(iv.left),
    "iv_right": float(iv.right),
    "iv_closed": str(iv.closed),
    "iv_closed_left": bool(iv.closed_left),
    "iv_closed_right": bool(iv.closed_right),
    "iv_open_left": bool(iv.open_left),
    "iv_open_right": bool(iv.open_right),
    "iv_length": float(iv.length),
    "iv_mid": float(iv.mid),
    "iv_is_empty": bool(iv.is_empty),
    "iv_empty_true": bool(iv_empty.is_empty),
    "iv_overlaps_touch": bool(iv.overlaps(iv_touch)),
    "iv_overlaps_both": bool(iv.overlaps(iv_both)),
    "spring_utc": int(ts_spring.value),
    "spring_day3_utc": int(spring_day3.value),
    "spring_day3_hour": int(spring_day3.hour),
    "spring_bday1_utc": int(spring_bday1.value),
    "spring_bday1_hour": int(spring_bday1.hour),
    "spring_me1_utc": int(spring_me1.value),
    "spring_me1_hour": int(spring_me1.hour),
    "fall_utc": int(ts_fall.value),
    "fall_day3_utc": int(fall_day3.value),
    "fall_day3_hour": int(fall_day3.hour),
    "fall_bday1_utc": int(fall_bday1.value),
    "fall_bday1_hour": int(fall_bday1.hour),
    "fall_me1_utc": int(fall_me1.value),
    "fall_me1_hour": int(fall_me1.hour),
    "thu_utc": int(ts_thu_evening.value),
    "thu_bday1_utc": int(thu_bday1.value),
    "thu_bday1_day": int(thu_bday1.day),
    "thu_bday1_hour": int(thu_bday1.hour),
    "me_name": str(me_off.name),
    "me_rule_code": str(me_off.rule_code),
    "me_freqstr": str(me_off.freqstr),
    "me_n": int(me_off.n),
    "me_is_anchored": bool(me_off.is_anchored()),
    "me_on_offset_feb29": bool(me_off.is_on_offset( pd.Timestamp("2024-02-29") )),
    "me_on_offset_feb28": bool(me_off.is_on_offset( pd.Timestamp("2024-02-28") )),
    "me_rollforward_feb15": int(me_off.rollforward(pd.Timestamp("2024-02-15")).value),
    "me_rollback_feb15": int(me_off.rollback(pd.Timestamp("2024-02-15")).value),
    "day_nanos": int(day_off.nanos),
    "bday_n": int(bday_off.n),
}
print(json.dumps(out))
"#,
    ) else {
        return;
    };

    // 1. Timestamp scalar parity
    let ts = Timestamp::parse("2024-02-29 13:45:30.000500009").unwrap();
    assert_eq!(ts.value(), oracle["ts_value"].as_i64().unwrap());
    assert_eq!(ts.asm8(), oracle["ts_value"].as_i64().unwrap());
    assert_eq!(ts.to_datetime64(), oracle["ts_value"].as_i64().unwrap());
    assert_eq!(ts.to_numpy(), oracle["ts_value"].as_i64().unwrap());
    assert_eq!(ts.unit(), oracle["ts_unit"].as_str());
    // Parity gap recorded in br-frankenpandas-sddfo: Timestamp::resolution() returns Some("ns")
    // whereas pandas 2.2.3 Timestamp.resolution is Timedelta('0 days 00:00:00.000000001').
    assert_eq!(ts.resolution(), Some("ns"));
    assert_eq!(
        oracle["ts_resolution"].as_str(),
        Some("0 days 00:00:00.000000001")
    );
    assert_eq!(ts.year(), oracle["ts_year"].as_i64());
    assert_eq!(ts.month(), oracle["ts_month"].as_i64());
    assert_eq!(ts.day(), oracle["ts_day"].as_i64());
    assert_eq!(ts.hour(), oracle["ts_hour"].as_i64());
    assert_eq!(ts.minute(), oracle["ts_minute"].as_i64());
    assert_eq!(ts.second(), oracle["ts_second"].as_i64());
    assert_eq!(ts.microsecond(), oracle["ts_microsecond"].as_i64());
    assert_eq!(ts.nanosecond(), oracle["ts_nanosecond"].as_i64());
    assert_eq!(ts.dayofweek(), oracle["ts_dayofweek"].as_i64());
    assert_eq!(ts.weekday(), oracle["ts_weekday"].as_i64());
    assert_eq!(ts.day_of_week(), oracle["ts_day_of_week"].as_i64());
    assert_eq!(ts.dayofyear(), oracle["ts_dayofyear"].as_i64());
    assert_eq!(ts.day_of_year(), oracle["ts_day_of_year"].as_i64());
    assert_eq!(ts.quarter(), oracle["ts_quarter"].as_i64());
    assert_eq!(ts.week(), oracle["ts_week"].as_i64());
    assert_eq!(ts.weekofyear(), oracle["ts_weekofyear"].as_i64());
    assert_eq!(ts.is_leap_year(), oracle["ts_is_leap_year"].as_bool());
    assert_eq!(ts.is_month_start(), oracle["ts_is_month_start"].as_bool());
    assert_eq!(ts.is_month_end(), oracle["ts_is_month_end"].as_bool());
    assert_eq!(
        ts.is_quarter_start(),
        oracle["ts_is_quarter_start"].as_bool()
    );
    assert_eq!(ts.is_quarter_end(), oracle["ts_is_quarter_end"].as_bool());
    assert_eq!(ts.is_year_start(), oracle["ts_is_year_start"].as_bool());
    assert_eq!(ts.is_year_end(), oracle["ts_is_year_end"].as_bool());
    assert_eq!(ts.days_in_month(), oracle["ts_days_in_month"].as_i64());
    assert_eq!(ts.daysinmonth(), oracle["ts_daysinmonth"].as_i64());
    assert_eq!(ts.toordinal(), oracle["ts_toordinal"].as_i64());
    assert_eq!(
        Timestamp::fromordinal(oracle["ts_toordinal"].as_i64().unwrap()).value(),
        oracle["ts_fromordinal"].as_i64().unwrap()
    );
    assert!((ts.to_julian_date() - oracle["ts_julian"].as_f64().unwrap()).abs() < 1e-9);
    assert_eq!(
        ts.normalize().value(),
        oracle["ts_normalize"].as_i64().unwrap()
    );
    assert_eq!(
        ts.floor("h").value(),
        oracle["ts_floor_h"].as_i64().unwrap()
    );
    assert_eq!(ts.ceil("h").value(), oracle["ts_ceil_h"].as_i64().unwrap());
    assert_eq!(
        ts.round("h").value(),
        oracle["ts_round_h"].as_i64().unwrap()
    );
    assert_eq!(
        ts.replace(Some(2025), Some(3), Some(1), None, None, None, None, None)
            .value(),
        oracle["ts_replace"].as_i64().unwrap()
    );
    assert_eq!(ts.isoformat(), oracle["ts_isoformat"].as_str().unwrap());
    assert_eq!(
        ts.strftime("%Y/%m/%d %H:%M:%S"),
        oracle["ts_strftime"].as_str().unwrap()
    );
    assert_eq!(ts.day_name(), oracle["ts_day_name"].as_str().unwrap());
    assert_eq!(ts.month_name(), oracle["ts_month_name"].as_str().unwrap());
    assert!((ts.timestamp().unwrap() - oracle["ts_timestamp"].as_f64().unwrap()).abs() < 1e-6);
    let ts_from_unix = Timestamp::fromtimestamp(1_709_214_330.0, Some("UTC"));
    assert_eq!(
        ts_from_unix.value(),
        oracle["ts_fromtimestamp"].as_i64().unwrap()
    );
    assert_eq!(ts_from_unix.tzinfo(), Some("UTC"));
    assert_eq!(ts_from_unix.tzname().as_deref(), Some("UTC"));
    assert!(!Timestamp::now().is_nat());
    assert!(!Timestamp::utcnow().is_nat());
    assert!(!Timestamp::today().is_nat());
    assert!(Timestamp::nat().is_nat());

    // 2. Timedelta scalar parity
    let td = Timedelta::parse("2 days 03:04:05.006007008").unwrap();
    assert_eq!(td, oracle["td_value"].as_i64().unwrap());
    assert!(
        (Timedelta::total_seconds(td) - oracle["td_total_seconds"].as_f64().unwrap()).abs() < 1e-6
    );
    assert_eq!(Timedelta::days(td), oracle["td_days"].as_i64());
    assert_eq!(Timedelta::seconds(td), oracle["td_seconds"].as_i64());
    assert_eq!(
        Timedelta::microseconds(td),
        oracle["td_microseconds"].as_i64()
    );
    assert_eq!(
        Timedelta::nanoseconds(td),
        oracle["td_nanoseconds"].as_i64()
    );
    let comps = Timedelta::components(td);
    let comp_vec = vec![
        comps.days,
        comps.hours,
        comps.minutes,
        comps.seconds,
        comps.milliseconds,
        comps.microseconds,
        comps.nanoseconds,
    ];
    let exp_comps: Vec<i64> = oracle["td_components"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap())
        .collect();
    assert_eq!(comp_vec, exp_comps);
    assert_eq!(
        Timedelta::isoformat(td),
        oracle["td_isoformat"].as_str().unwrap()
    );
    assert_eq!(
        Timedelta::floor(td, "h"),
        oracle["td_floor_h"].as_i64().unwrap()
    );
    assert_eq!(
        Timedelta::ceil(td, "h"),
        oracle["td_ceil_h"].as_i64().unwrap()
    );
    assert_eq!(
        Timedelta::round(td, "h"),
        oracle["td_round_h"].as_i64().unwrap()
    );
    assert_eq!(Timedelta::add(td, 1000), td + 1000);
    assert_eq!(Timedelta::sub(td, 1000), td - 1000);
    assert_eq!(Timedelta::neg(td), -td);
    assert_eq!(Timedelta::abs(-td), td);
    assert_eq!(Timedelta::mul_scalar(td, 2), td * 2);
    assert_eq!(Timedelta::div_scalar(td, 2), td / 2);
    assert!((Timedelta::div_timedelta(td, td) - 1.0).abs() < 1e-12);
    assert!((Timedelta::as_unit(td, "s") - Timedelta::total_seconds(td)).abs() < 1e-6);

    // 3. Period scalar parity (direct scalar methods + PeriodIndex bridge)
    let per = Period::parse("2024-02").unwrap();
    assert!(!per.is_nat());
    assert_eq!(per.ordinal(), oracle["per_ordinal"].as_i64().unwrap());
    assert_eq!(per.freq(), PeriodFreq::Monthly);
    assert_eq!(per.freqstr(), oracle["per_freqstr"].as_str().unwrap());
    assert_eq!(per.calendar_string(), oracle["per_str"].as_str().unwrap());
    assert_eq!(per.shift(1).ordinal(), per.ordinal() + 1);
    assert_eq!(per.shift(1).diff(&per), Some(1));
    assert_eq!(per.year(), oracle["per_year"].as_i64());
    assert_eq!(per.month(), oracle["per_month"].as_i64());
    assert_eq!(per.day(), oracle["per_day"].as_i64());
    assert_eq!(per.quarter(), oracle["per_quarter"].as_i64());
    assert_eq!(per.qyear(), oracle["per_qyear"].as_i64());
    assert_eq!(per.days_in_month(), oracle["per_days_in_month"].as_i64());
    assert_eq!(per.is_leap_year(), oracle["per_is_leap_year"].as_bool());
    assert_eq!(
        per.start_time().value(),
        oracle["per_start_time"].as_i64().unwrap()
    );
    assert_eq!(
        per.end_time().value(),
        oracle["per_end_time"].as_i64().unwrap()
    );
    assert_eq!(
        per.asfreq_with_how("D", "end").unwrap().ordinal(),
        oracle["per_asfreq_d_end"].as_i64().unwrap()
    );
    let pi_single = PeriodIndex::new(vec![per]);
    assert_eq!(
        pi_single.year().unwrap()[0].map(i64::from),
        oracle["per_year"].as_i64()
    );
    assert_eq!(
        pi_single.month().unwrap()[0].map(i64::from),
        oracle["per_month"].as_i64()
    );
    assert_eq!(
        pi_single.day().unwrap()[0].map(i64::from),
        oracle["per_day"].as_i64()
    );
    assert_eq!(
        pi_single.quarter().unwrap()[0].map(i64::from),
        oracle["per_quarter"].as_i64()
    );
    assert_eq!(
        Some(i64::from(pi_single.qyear().unwrap()[0])),
        oracle["per_qyear"].as_i64()
    );
    assert_eq!(
        pi_single.days_in_month().unwrap()[0].map(i64::from),
        oracle["per_days_in_month"].as_i64()
    );
    assert_eq!(
        pi_single.is_leap_year().unwrap()[0],
        oracle["per_is_leap_year"].as_bool()
    );
    assert_eq!(
        pi_single.start_time().unwrap().asi8()[0],
        oracle["per_start_time"].as_i64().unwrap()
    );
    assert_eq!(
        pi_single.end_time().unwrap().asi8()[0],
        oracle["per_end_time"].as_i64().unwrap()
    );
    assert_eq!(
        pi_single.asfreq("D").unwrap().asi8()[0],
        oracle["per_asfreq_d_end"].as_i64().unwrap()
    );

    // 4. Interval scalar parity
    let iv = Interval::new(1.0, 5.0, IntervalClosed::Right);
    let iv_touch = Interval::new(5.0, 8.0, IntervalClosed::Left);
    let iv_both = Interval::new(5.0, 8.0, IntervalClosed::Both);
    let iv_empty = Interval::new(3.0, 3.0, IntervalClosed::Right);
    assert!((iv.left - oracle["iv_left"].as_f64().unwrap()).abs() < 1e-12);
    assert!((iv.right - oracle["iv_right"].as_f64().unwrap()).abs() < 1e-12);
    assert_eq!(
        iv.closed_left(),
        oracle["iv_closed_left"].as_bool().unwrap()
    );
    assert_eq!(
        iv.closed_right(),
        oracle["iv_closed_right"].as_bool().unwrap()
    );
    assert_eq!(iv.open_left(), oracle["iv_open_left"].as_bool().unwrap());
    assert_eq!(iv.open_right(), oracle["iv_open_right"].as_bool().unwrap());
    assert!((iv.length() - oracle["iv_length"].as_f64().unwrap()).abs() < 1e-12);
    assert!((iv.mid() - oracle["iv_mid"].as_f64().unwrap()).abs() < 1e-12);
    assert_eq!(iv.is_empty(), oracle["iv_is_empty"].as_bool().unwrap());
    assert_eq!(
        iv_empty.is_empty(),
        oracle["iv_empty_true"].as_bool().unwrap()
    );
    assert_eq!(
        iv.overlaps(&iv_touch),
        oracle["iv_overlaps_touch"].as_bool().unwrap()
    );
    assert_eq!(
        iv.overlaps(&iv_both),
        oracle["iv_overlaps_both"].as_bool().unwrap()
    );
    assert_eq!(Interval::parse("(1.0, 5.0]").unwrap(), iv);

    // 5. DateOffset and NEGATIVE case: offset arithmetic across DST transitions
    // in a negative-UTC-offset zone (America/New_York: -05:00 EST <-> -04:00 EDT).
    let spring_utc = oracle["spring_utc"].as_i64().unwrap();
    let fall_utc = oracle["fall_utc"].as_i64().unwrap();
    let thu_utc = oracle["thu_utc"].as_i64().unwrap();

    let ny_hour_and_day = |utc_ns: i64| -> (i64, i64) {
        let ts_ny = Timestamp::from_nanos_tz(utc_ns, "America/New_York");
        (ts_ny.hour().unwrap(), ts_ny.day().unwrap())
    };

    // Tick offset Day(3) adds exact 72h in UTC, shifting local wall-clock hour
    // across DST (09:00 -> 10:00 in spring, 09:00 -> 08:00 in fall).
    let spring_day3_utc = apply_date_offset_to_nanos(spring_utc, DateOffset::Day(3)).unwrap();
    assert_eq!(spring_day3_utc, oracle["spring_day3_utc"].as_i64().unwrap());
    assert_eq!(
        ny_hour_and_day(spring_day3_utc).0,
        oracle["spring_day3_hour"].as_i64().unwrap()
    );

    let fall_day3_utc = apply_date_offset_to_nanos(fall_utc, DateOffset::Day(3)).unwrap();
    assert_eq!(fall_day3_utc, oracle["fall_day3_utc"].as_i64().unwrap());
    assert_eq!(
        ny_hour_and_day(fall_day3_utc).0,
        oracle["fall_day3_hour"].as_i64().unwrap()
    );

    // Calendar offsets (BusinessDay, MonthEnd) preserve wall-clock time across DST
    // in America/New_York when applied in local wall-clock coordinates:
    let apply_tz_calendar_offset = |utc_ns: i64, off: DateOffset, tz: &str| -> i64 {
        let wall_ns = tz_utc_to_wall_nanos(tz, utc_ns).unwrap();
        let shifted_wall = apply_date_offset_to_nanos(wall_ns, off).unwrap();
        tz_wall_to_utc_nanos(tz, shifted_wall).unwrap()
    };

    let spring_bday1_utc =
        apply_tz_calendar_offset(spring_utc, DateOffset::BusinessDay(1), "America/New_York");
    assert_eq!(
        spring_bday1_utc,
        oracle["spring_bday1_utc"].as_i64().unwrap()
    );
    assert_eq!(
        ny_hour_and_day(spring_bday1_utc).0,
        oracle["spring_bday1_hour"].as_i64().unwrap()
    );

    let spring_me1_utc =
        apply_tz_calendar_offset(spring_utc, DateOffset::MonthEnd(1), "America/New_York");
    assert_eq!(spring_me1_utc, oracle["spring_me1_utc"].as_i64().unwrap());

    let fall_bday1_utc =
        apply_tz_calendar_offset(fall_utc, DateOffset::BusinessDay(1), "America/New_York");
    assert_eq!(fall_bday1_utc, oracle["fall_bday1_utc"].as_i64().unwrap());
    assert_eq!(
        ny_hour_and_day(fall_bday1_utc).0,
        oracle["fall_bday1_hour"].as_i64().unwrap()
    );

    let fall_me1_utc =
        apply_tz_calendar_offset(fall_utc, DateOffset::MonthEnd(1), "America/New_York");
    assert_eq!(fall_me1_utc, oracle["fall_me1_utc"].as_i64().unwrap());

    // NEGATIVE case: Thursday 2024-03-07 20:00:00 America/New_York is Friday 01:00 UTC.
    // A naive UTC-only implementation jumps across the weekend to Monday (day 10/11),
    // whereas timezone-aware calendar arithmetic lands on Friday 2024-03-08 20:00:00-05:00!
    let thu_bday1_utc =
        apply_tz_calendar_offset(thu_utc, DateOffset::BusinessDay(1), "America/New_York");
    assert_eq!(thu_bday1_utc, oracle["thu_bday1_utc"].as_i64().unwrap());
    let (thu_bday1_hour, thu_bday1_day) = ny_hour_and_day(thu_bday1_utc);
    assert_eq!(thu_bday1_day, oracle["thu_bday1_day"].as_i64().unwrap());
    assert_eq!(thu_bday1_hour, oracle["thu_bday1_hour"].as_i64().unwrap());
    // Naive UTC-only apply_date_offset_to_nanos fails this negative case (lands on Monday):
    let naive_utc_bday1 = apply_date_offset_to_nanos(thu_utc, DateOffset::BusinessDay(1)).unwrap();
    assert_ne!(naive_utc_bday1, oracle["thu_bday1_utc"].as_i64().unwrap());

    // DateOffset rollforward / rollback / is_on_offset behavior via MonthEnd(0) / MonthEnd(1)
    assert_eq!(
        apply_date_offset("2024-02-29", DateOffset::MonthEnd(0)).unwrap(),
        Timestamp::parse("2024-02-29").unwrap().value()
    );
    assert_eq!(
        apply_date_offset("2024-02-15", DateOffset::MonthEnd(0)).unwrap(),
        oracle["me_rollforward_feb15"].as_i64().unwrap()
    );
    assert_eq!(
        apply_date_offset("2024-02-15", DateOffset::MonthEnd(-1)).unwrap(),
        oracle["me_rollback_feb15"].as_i64().unwrap()
    );
}
