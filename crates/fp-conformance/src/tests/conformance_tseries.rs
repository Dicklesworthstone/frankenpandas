//! pandas.tseries parity-matrix conformance suite (frankenpandas-dh4f).
//!
//! Per /testing-conformance-harnesses Pattern 1, these tests compare the
//! Rust `fp_index` tseries facades against live upstream pandas for
//! edge-case parameter combinations.

use std::{
    io::Write,
    process::{Command, Stdio},
};

use fp_index::{
    DateOffset, DateRangeError, Index, IndexLabel, apply_date_offset, bdate_range, date_range,
    infer_freq_from_timestamps,
};
use fp_types::Timedelta;
use serde::Deserialize;

use super::HarnessConfig;

#[derive(Debug, Clone, Copy)]
struct DateRangeCase<'a> {
    case_id: &'a str,
    start: Option<&'a str>,
    end: Option<&'a str>,
    periods: Option<usize>,
    pandas_freq: &'a str,
    rust_freq_nanos: i64,
    name: Option<&'a str>,
}

#[derive(Debug, Clone, Copy)]
struct BusinessDateRangeCase<'a> {
    case_id: &'a str,
    start: Option<&'a str>,
    end: Option<&'a str>,
    periods: Option<usize>,
    name: Option<&'a str>,
}

#[derive(Debug, Clone, Copy)]
struct PandasRangeOracleCase<'a> {
    case_id: &'a str,
    pandas_function: &'a str,
    start: Option<&'a str>,
    end: Option<&'a str>,
    periods: Option<usize>,
    pandas_freq: &'a str,
    name: Option<&'a str>,
}

#[derive(Debug, Clone, Copy)]
struct DateOffsetCase<'a> {
    case_id: &'a str,
    timestamp: &'a str,
    pandas_offset: &'a str,
    offset: DateOffset,
}

#[derive(Debug, Clone, Copy)]
struct InferFreqCase<'a> {
    case_id: &'a str,
    timestamps: &'a [&'a str],
}

#[derive(Debug, Deserialize)]
struct PandasDateRange {
    values: Vec<i64>,
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PandasOffsetResult {
    value: i64,
}

#[derive(Debug, Deserialize)]
struct PandasInferFreqResult {
    ok: bool,
    value: Option<String>,
    error_type: Option<String>,
    message: Option<String>,
}

fn pandas_tseries_range_or_skip(
    config: &HarnessConfig,
    case: PandasRangeOracleCase<'_>,
) -> Result<Option<PandasDateRange>, String> {
    let case_id = case.case_id;
    let pandas_function = case.pandas_function;

    if !config.oracle_root.exists() && !config.allows_live_oracle_fallback() {
        eprintln!(
            "live pandas unavailable; skipping tseries conformance test {}: legacy oracle root does not exist: {}",
            case_id,
            config.oracle_root.display()
        );
        return Ok(None);
    }

    let payload = serde_json::json!({
        "function": pandas_function,
        "start": case.start,
        "end": case.end,
        "periods": case.periods,
        "freq": case.pandas_freq,
        "name": case.name,
    });

    let script = r#"
import importlib
import json
import os
import sys

legacy_root = os.path.abspath(sys.argv[1])
allow_system_fallback = sys.argv[2] == "1"
candidate_parent = os.path.dirname(legacy_root)
if os.path.isdir(candidate_parent):
    sys.path.insert(0, candidate_parent)

try:
    import pandas as pd
except Exception:
    if not allow_system_fallback:
        raise
    while candidate_parent in sys.path:
        sys.path.remove(candidate_parent)
    sys.modules.pop("pandas", None)
    pd = importlib.import_module("pandas")

request = json.loads(sys.stdin.read())
kwargs = {"freq": request["freq"]}
for key in ("start", "end", "periods", "name"):
    if request.get(key) is not None:
        kwargs[key] = request[key]

index = getattr(pd, request["function"])(**kwargs)
if hasattr(index, "as_unit"):
    index = index.as_unit("ns")
print(json.dumps({
    "values": [int(value) for value in index.asi8.tolist()],
    "name": index.name,
}, separators=(",", ":")))
"#;

    let mut child = Command::new(&config.python_bin)
        .arg(["-", "c"].concat())
        .arg(script)
        .arg(&config.oracle_root)
        .arg(if config.allows_live_oracle_fallback() {
            "1"
        } else {
            "0"
        })
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("spawn pandas {pandas_function} oracle failed: {err}"))?;

    // Per br-frankenpandas-50b6b0: drain stdin in a worker thread.
    let stdin_handle = child
        .stdin
        .take()
        .ok_or_else(|| format!("pandas {pandas_function} oracle stdin unavailable"))?;
    let payload_bytes = payload.to_string().into_bytes();
    let stdin_writer = std::thread::spawn(move || -> std::io::Result<()> {
        let mut stdin = stdin_handle;
        stdin.write_all(&payload_bytes)
    });
    let output = child
        .wait_with_output()
        .map_err(|err| format!("wait for pandas {pandas_function} oracle failed: {err}"))?;
    let pandas_function_for_err = pandas_function.to_string();
    stdin_writer
        .join()
        .map_err(|_| format!("pandas {pandas_function_for_err} oracle stdin writer panicked"))?
        .map_err(|err| {
            format!("write pandas {pandas_function_for_err} oracle payload failed: {err}")
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("AttributeError: module 'pandas' has no attribute") {
            eprintln!(
                "live pandas unavailable; skipping tseries conformance test {case_id}: {stderr}"
            );
            return Ok(None);
        }
        return Err(format!(
            "pandas {pandas_function} oracle failed for {case_id}: {}",
            stderr
        ));
    }

    serde_json::from_slice(&output.stdout)
        .map(Some)
        .map_err(|err| format!("parse pandas {pandas_function} json failed: {err}"))
}

fn pandas_date_range_or_skip(
    config: &HarnessConfig,
    case: DateRangeCase<'_>,
) -> Result<Option<PandasDateRange>, String> {
    pandas_tseries_range_or_skip(
        config,
        PandasRangeOracleCase {
            case_id: case.case_id,
            pandas_function: "date_range",
            start: case.start,
            end: case.end,
            periods: case.periods,
            pandas_freq: case.pandas_freq,
            name: case.name,
        },
    )
}

fn pandas_bdate_range_or_skip(
    config: &HarnessConfig,
    case: BusinessDateRangeCase<'_>,
) -> Result<Option<PandasDateRange>, String> {
    pandas_tseries_range_or_skip(
        config,
        PandasRangeOracleCase {
            case_id: case.case_id,
            pandas_function: "bdate_range",
            start: case.start,
            end: case.end,
            periods: case.periods,
            pandas_freq: "B",
            name: case.name,
        },
    )
}

fn pandas_offset_or_skip(
    config: &HarnessConfig,
    case: DateOffsetCase<'_>,
) -> Result<Option<PandasOffsetResult>, String> {
    if !config.oracle_root.exists() && !config.allows_live_oracle_fallback() {
        eprintln!(
            "live pandas unavailable; skipping tseries conformance test {}: legacy oracle root does not exist: {}",
            case.case_id,
            config.oracle_root.display()
        );
        return Ok(None);
    }

    let n = match case.offset {
        DateOffset::Day(value) | DateOffset::BusinessDay(value) | DateOffset::MonthEnd(value) => {
            value
        }
    };
    let payload = serde_json::json!({
        "timestamp": case.timestamp,
        "offset": case.pandas_offset,
        "n": n,
    });

    let script = r#"
import importlib
import json
import os
import sys

legacy_root = os.path.abspath(sys.argv[1])
allow_system_fallback = sys.argv[2] == "1"
candidate_parent = os.path.dirname(legacy_root)
if os.path.isdir(candidate_parent):
    sys.path.insert(0, candidate_parent)

try:
    import pandas as pd
except Exception:
    if not allow_system_fallback:
        raise
    while candidate_parent in sys.path:
        sys.path.remove(candidate_parent)
    sys.modules.pop("pandas", None)
    pd = importlib.import_module("pandas")

request = json.loads(sys.stdin.read())
offsets = {
    "Day": pd.offsets.Day,
    "BDay": pd.offsets.BDay,
    "MonthEnd": pd.offsets.MonthEnd,
}
result = pd.Timestamp(request["timestamp"]) + offsets[request["offset"]](request["n"])
print(json.dumps({"value": int(result.value)}, separators=(",", ":")))
"#;

    let mut child = Command::new(&config.python_bin)
        .arg(["-", "c"].concat())
        .arg(script)
        .arg(&config.oracle_root)
        .arg(if config.allows_live_oracle_fallback() {
            "1"
        } else {
            "0"
        })
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("spawn pandas offset oracle failed: {err}"))?;

    // Per br-frankenpandas-50b6b0: drain stdin in a worker thread.
    let stdin_handle = child
        .stdin
        .take()
        .ok_or_else(|| "pandas offset oracle stdin unavailable".to_owned())?;
    let payload_bytes = payload.to_string().into_bytes();
    let stdin_writer = std::thread::spawn(move || -> std::io::Result<()> {
        let mut stdin = stdin_handle;
        stdin.write_all(&payload_bytes)
    });
    let output = child
        .wait_with_output()
        .map_err(|err| format!("wait for pandas offset oracle failed: {err}"))?;
    stdin_writer
        .join()
        .map_err(|_| "pandas offset oracle stdin writer panicked".to_owned())?
        .map_err(|err| format!("write pandas offset oracle payload failed: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "pandas offset oracle failed for {}: {}",
            case.case_id,
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    serde_json::from_slice(&output.stdout)
        .map(Some)
        .map_err(|err| format!("parse pandas offset json failed: {err}"))
}

fn pandas_infer_freq_or_skip(
    config: &HarnessConfig,
    case: InferFreqCase<'_>,
) -> Result<Option<PandasInferFreqResult>, String> {
    if !config.oracle_root.exists() && !config.allows_live_oracle_fallback() {
        eprintln!(
            "live pandas unavailable; skipping tseries conformance test {}: legacy oracle root does not exist: {}",
            case.case_id,
            config.oracle_root.display()
        );
        return Ok(None);
    }

    let payload = serde_json::json!({
        "timestamps": case.timestamps,
    });

    let script = r#"
import importlib
import json
import os
import sys

legacy_root = os.path.abspath(sys.argv[1])
allow_system_fallback = sys.argv[2] == "1"
candidate_parent = os.path.dirname(legacy_root)
if os.path.isdir(candidate_parent):
    sys.path.insert(0, candidate_parent)

try:
    import pandas as pd
except Exception:
    if not allow_system_fallback:
        raise
    while candidate_parent in sys.path:
        sys.path.remove(candidate_parent)
    sys.modules.pop("pandas", None)
    pd = importlib.import_module("pandas")

request = json.loads(sys.stdin.read())
try:
    value = pd.infer_freq(pd.DatetimeIndex(request["timestamps"]))
    print(json.dumps({"ok": True, "value": value}, separators=(",", ":")))
except Exception as exc:
    print(json.dumps({
        "ok": False,
        "error_type": type(exc).__name__,
        "message": str(exc),
    }, separators=(",", ":")))
"#;

    let mut child = Command::new(&config.python_bin)
        .arg(["-", "c"].concat())
        .arg(script)
        .arg(&config.oracle_root)
        .arg(if config.allows_live_oracle_fallback() {
            "1"
        } else {
            "0"
        })
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("spawn pandas infer_freq oracle failed: {err}"))?;

    // Per br-frankenpandas-50b6b0: drain stdin in a worker thread.
    let stdin_handle = child
        .stdin
        .take()
        .ok_or_else(|| "pandas infer_freq oracle stdin unavailable".to_owned())?;
    let payload_bytes = payload.to_string().into_bytes();
    let stdin_writer = std::thread::spawn(move || -> std::io::Result<()> {
        let mut stdin = stdin_handle;
        stdin.write_all(&payload_bytes)
    });
    let output = child
        .wait_with_output()
        .map_err(|err| format!("wait for pandas infer_freq oracle failed: {err}"))?;
    stdin_writer
        .join()
        .map_err(|_| "pandas infer_freq oracle stdin writer panicked".to_owned())?
        .map_err(|err| format!("write pandas infer_freq oracle payload failed: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "pandas infer_freq oracle failed for {}: {}",
            case.case_id,
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    serde_json::from_slice(&output.stdout)
        .map(Some)
        .map_err(|err| format!("parse pandas infer_freq json failed: {err}"))
}

fn datetime_labels(index: &Index) -> Result<Vec<i64>, String> {
    index
        .labels()
        .iter()
        .map(|label| match label {
            IndexLabel::Datetime64(value) => Ok(*value),
            other => Err(format!("expected datetime64 label, got {other:?}")),
        })
        .collect()
}

fn assert_date_range_matches_pandas(case: DateRangeCase<'_>) -> Result<(), String> {
    // br-frankenpandas-l7r1p: opt into the SYSTEM pandas. Without it the
    // legacy oracle root is absent, the helper below returns early, and the
    // test reports green while comparing nothing.
    let mut config = HarnessConfig::default_paths();
    config.allow_system_pandas_fallback = true;
    let Some(expected) = pandas_date_range_or_skip(&config, case)? else {
        return Ok(());
    };

    let actual = date_range(
        case.start,
        case.end,
        case.periods,
        case.rust_freq_nanos,
        case.name,
    )
    .map_err(|err| format!("fp date_range failed for {}: {err}", case.case_id))?;

    assert_eq!(
        datetime_labels(&actual)?,
        expected.values,
        "pandas.tseries date_range value parity drift for {}",
        case.case_id
    );
    assert_eq!(
        actual.name().map(String::from),
        expected.name,
        "pandas.tseries date_range name parity drift for {}",
        case.case_id
    );
    Ok(())
}

fn assert_date_range_rejects_like_pandas<F>(
    case: DateRangeCase<'_>,
    expected_pandas_error: &str,
    matches_fp_error: F,
) -> Result<(), String>
where
    F: FnOnce(&DateRangeError) -> bool,
{
    // br-frankenpandas-l7r1p: opt into the SYSTEM pandas. Without it the
    // legacy oracle root is absent, the helper below returns early, and the
    // test reports green while comparing nothing.
    let mut config = HarnessConfig::default_paths();
    config.allow_system_pandas_fallback = true;
    match pandas_date_range_or_skip(&config, case) {
        Ok(None) => return Ok(()),
        Ok(Some(expected)) => {
            return Err(format!(
                "pandas date_range unexpectedly accepted {} with values {:?}",
                case.case_id, expected.values
            ));
        }
        Err(err) if err.contains(expected_pandas_error) => {}
        Err(err) => {
            return Err(format!(
                "pandas date_range rejected {} with unexpected error: {err}",
                case.case_id
            ));
        }
    }

    let actual = date_range(
        case.start,
        case.end,
        case.periods,
        case.rust_freq_nanos,
        case.name,
    );
    let actual_err = match actual {
        Ok(index) => {
            return Err(format!(
                "fp date_range unexpectedly accepted {} with labels {:?}",
                case.case_id,
                index.labels()
            ));
        }
        Err(err) => err,
    };
    assert!(
        matches_fp_error(&actual_err),
        "pandas.tseries date_range error parity drift for {}: fp returned {actual_err:?}",
        case.case_id
    );
    Ok(())
}

fn assert_bdate_range_matches_pandas(case: BusinessDateRangeCase<'_>) -> Result<(), String> {
    // br-frankenpandas-l7r1p: opt into the SYSTEM pandas. Without it the
    // legacy oracle root is absent, the helper below returns early, and the
    // test reports green while comparing nothing.
    let mut config = HarnessConfig::default_paths();
    config.allow_system_pandas_fallback = true;
    let Some(expected) = pandas_bdate_range_or_skip(&config, case)? else {
        return Ok(());
    };

    let actual = bdate_range(case.start, case.end, case.periods, case.name)
        .map_err(|err| format!("fp bdate_range failed for {}: {err}", case.case_id))?;

    assert_eq!(
        datetime_labels(&actual)?,
        expected.values,
        "pandas.tseries bdate_range value parity drift for {}",
        case.case_id
    );
    assert_eq!(
        actual.name().map(String::from),
        expected.name,
        "pandas.tseries bdate_range name parity drift for {}",
        case.case_id
    );
    Ok(())
}

fn assert_offset_matches_pandas(case: DateOffsetCase<'_>) -> Result<(), String> {
    // br-frankenpandas-l7r1p: opt into the SYSTEM pandas. Without it the
    // legacy oracle root is absent, the helper below returns early, and the
    // test reports green while comparing nothing.
    let mut config = HarnessConfig::default_paths();
    config.allow_system_pandas_fallback = true;
    let Some(expected) = pandas_offset_or_skip(&config, case)? else {
        return Ok(());
    };

    let actual = apply_date_offset(case.timestamp, case.offset)
        .map_err(|err| format!("fp offset failed for {}: {err}", case.case_id))?;

    assert_eq!(
        actual, expected.value,
        "pandas.tseries offset value parity drift for {}",
        case.case_id
    );
    Ok(())
}

fn assert_infer_freq_matches_pandas(case: InferFreqCase<'_>) -> Result<(), String> {
    // br-frankenpandas-l7r1p: opt into the SYSTEM pandas. Without it the
    // legacy oracle root is absent, the helper below returns early, and the
    // test reports green while comparing nothing.
    let mut config = HarnessConfig::default_paths();
    config.allow_system_pandas_fallback = true;
    let Some(expected) = pandas_infer_freq_or_skip(&config, case)? else {
        return Ok(());
    };

    let actual = infer_freq_from_timestamps(case.timestamps);
    if expected.ok {
        let actual =
            actual.map_err(|err| format!("fp infer_freq failed for {}: {err}", case.case_id))?;
        assert_eq!(
            actual, expected.value,
            "pandas.tseries infer_freq value parity drift for {}",
            case.case_id
        );
    } else {
        assert!(
            actual.is_err(),
            "pandas.tseries infer_freq error parity drift for {}: pandas returned {:?} {:?}, fp returned {:?}",
            case.case_id,
            expected.error_type,
            expected.message,
            actual
        );
    }
    Ok(())
}

#[test]
fn conformance_tseries_date_range_start_periods_daily() -> Result<(), String> {
    assert_date_range_matches_pandas(DateRangeCase {
        case_id: "tseries_date_range_start_periods_daily",
        start: Some("2024-01-01"),
        end: None,
        periods: Some(3),
        pandas_freq: "D",
        rust_freq_nanos: Timedelta::NANOS_PER_DAY,
        name: None,
    })
}

#[test]
fn conformance_tseries_date_range_start_end_two_day_frequency() -> Result<(), String> {
    assert_date_range_matches_pandas(DateRangeCase {
        case_id: "tseries_date_range_start_end_two_day_frequency",
        start: Some("2024-01-01"),
        end: Some("2024-01-07"),
        periods: None,
        pandas_freq: "2D",
        rust_freq_nanos: 2 * Timedelta::NANOS_PER_DAY,
        name: None,
    })
}

#[test]
fn conformance_tseries_date_range_end_periods_hourly() -> Result<(), String> {
    assert_date_range_matches_pandas(DateRangeCase {
        case_id: "tseries_date_range_end_periods_hourly",
        start: None,
        end: Some("2024-01-01 12:00:00"),
        periods: Some(3),
        pandas_freq: "6h",
        rust_freq_nanos: 6 * Timedelta::NANOS_PER_HOUR,
        name: None,
    })
}

#[test]
fn conformance_tseries_date_range_zero_periods() -> Result<(), String> {
    assert_date_range_matches_pandas(DateRangeCase {
        case_id: "tseries_date_range_zero_periods",
        start: Some("2024-01-01"),
        end: None,
        periods: Some(0),
        pandas_freq: "D",
        rust_freq_nanos: Timedelta::NANOS_PER_DAY,
        name: None,
    })
}

#[test]
fn conformance_tseries_date_range_preserves_name() -> Result<(), String> {
    assert_date_range_matches_pandas(DateRangeCase {
        case_id: "tseries_date_range_preserves_name",
        start: Some("2024-02-01"),
        end: None,
        periods: Some(2),
        pandas_freq: "D",
        rust_freq_nanos: Timedelta::NANOS_PER_DAY,
        name: Some("when"),
    })
}

#[test]
fn conformance_tseries_date_range_rejects_over_specified_params() -> Result<(), String> {
    assert_date_range_rejects_like_pandas(
        DateRangeCase {
            case_id: "tseries_date_range_rejects_over_specified_params",
            start: Some("2020-01-01"),
            end: Some("2020-01-03"),
            periods: Some(2),
            pandas_freq: "D",
            rust_freq_nanos: Timedelta::NANOS_PER_DAY,
            name: None,
        },
        "ValueError",
        |err| matches!(err, DateRangeError::TooManyParams),
    )
}

#[test]
fn conformance_tseries_bdate_range_start_periods_weekdays() -> Result<(), String> {
    assert_bdate_range_matches_pandas(BusinessDateRangeCase {
        case_id: "tseries_bdate_range_start_periods_weekdays",
        start: Some("2024-01-01"),
        end: None,
        periods: Some(3),
        name: None,
    })
}

#[test]
fn conformance_tseries_bdate_range_weekend_start_rolls_forward() -> Result<(), String> {
    assert_bdate_range_matches_pandas(BusinessDateRangeCase {
        case_id: "tseries_bdate_range_weekend_start_rolls_forward",
        start: Some("2024-01-06"),
        end: None,
        periods: Some(3),
        name: None,
    })
}

#[test]
fn conformance_tseries_bdate_range_start_end_skips_weekend() -> Result<(), String> {
    assert_bdate_range_matches_pandas(BusinessDateRangeCase {
        case_id: "tseries_bdate_range_start_end_skips_weekend",
        start: Some("2024-01-05"),
        end: Some("2024-01-09"),
        periods: None,
        name: None,
    })
}

#[test]
fn conformance_tseries_bdate_range_weekend_end_rolls_backward() -> Result<(), String> {
    assert_bdate_range_matches_pandas(BusinessDateRangeCase {
        case_id: "tseries_bdate_range_weekend_end_rolls_backward",
        start: None,
        end: Some("2024-01-07"),
        periods: Some(3),
        name: None,
    })
}

#[test]
fn conformance_tseries_bdate_range_zero_periods() -> Result<(), String> {
    assert_bdate_range_matches_pandas(BusinessDateRangeCase {
        case_id: "tseries_bdate_range_zero_periods",
        start: Some("2024-01-01"),
        end: None,
        periods: Some(0),
        name: None,
    })
}

#[test]
fn conformance_tseries_bdate_range_preserves_name() -> Result<(), String> {
    assert_bdate_range_matches_pandas(BusinessDateRangeCase {
        case_id: "tseries_bdate_range_preserves_name",
        start: Some("2024-02-02"),
        end: None,
        periods: Some(2),
        name: Some("bizday"),
    })
}

#[test]
fn conformance_tseries_offsets_day_preserves_time() -> Result<(), String> {
    assert_offset_matches_pandas(DateOffsetCase {
        case_id: "tseries_offsets_day_preserves_time",
        timestamp: "2024-01-01 12:30:00",
        pandas_offset: "Day",
        offset: DateOffset::Day(2),
    })
}

#[test]
fn conformance_tseries_offsets_day_zero_is_noop() -> Result<(), String> {
    assert_offset_matches_pandas(DateOffsetCase {
        case_id: "tseries_offsets_day_zero_is_noop",
        timestamp: "2024-01-01",
        pandas_offset: "Day",
        offset: DateOffset::Day(0),
    })
}

#[test]
fn conformance_tseries_offsets_bday_friday_to_monday() -> Result<(), String> {
    assert_offset_matches_pandas(DateOffsetCase {
        case_id: "tseries_offsets_bday_friday_to_monday",
        timestamp: "2024-01-05",
        pandas_offset: "BDay",
        offset: DateOffset::BusinessDay(1),
    })
}

#[test]
fn conformance_tseries_offsets_bday_weekend_rolls_forward() -> Result<(), String> {
    assert_offset_matches_pandas(DateOffsetCase {
        case_id: "tseries_offsets_bday_weekend_rolls_forward",
        timestamp: "2024-01-06",
        pandas_offset: "BDay",
        offset: DateOffset::BusinessDay(1),
    })
}

#[test]
fn conformance_tseries_offsets_bday_negative_skips_weekend() -> Result<(), String> {
    assert_offset_matches_pandas(DateOffsetCase {
        case_id: "tseries_offsets_bday_negative_skips_weekend",
        timestamp: "2024-01-08",
        pandas_offset: "BDay",
        offset: DateOffset::BusinessDay(-1),
    })
}

#[test]
fn conformance_tseries_offsets_bday_zero_weekend_rolls_forward() -> Result<(), String> {
    assert_offset_matches_pandas(DateOffsetCase {
        case_id: "tseries_offsets_bday_zero_weekend_rolls_forward",
        timestamp: "2024-01-06",
        pandas_offset: "BDay",
        offset: DateOffset::BusinessDay(0),
    })
}

#[test]
fn conformance_tseries_offsets_month_end_midmonth_leap_year() -> Result<(), String> {
    assert_offset_matches_pandas(DateOffsetCase {
        case_id: "tseries_offsets_month_end_midmonth_leap_year",
        timestamp: "2024-02-10",
        pandas_offset: "MonthEnd",
        offset: DateOffset::MonthEnd(1),
    })
}

#[test]
fn conformance_tseries_offsets_month_end_on_anchor_advances() -> Result<(), String> {
    assert_offset_matches_pandas(DateOffsetCase {
        case_id: "tseries_offsets_month_end_on_anchor_advances",
        timestamp: "2024-02-29",
        pandas_offset: "MonthEnd",
        offset: DateOffset::MonthEnd(1),
    })
}

#[test]
fn conformance_tseries_offsets_month_end_negative_rolls_back() -> Result<(), String> {
    assert_offset_matches_pandas(DateOffsetCase {
        case_id: "tseries_offsets_month_end_negative_rolls_back",
        timestamp: "2024-02-10",
        pandas_offset: "MonthEnd",
        offset: DateOffset::MonthEnd(-1),
    })
}

#[test]
fn conformance_tseries_infer_freq_daily() -> Result<(), String> {
    assert_infer_freq_matches_pandas(InferFreqCase {
        case_id: "tseries_infer_freq_daily",
        timestamps: &["2024-01-01", "2024-01-02", "2024-01-03", "2024-01-04"],
    })
}

#[test]
fn conformance_tseries_infer_freq_two_day() -> Result<(), String> {
    assert_infer_freq_matches_pandas(InferFreqCase {
        case_id: "tseries_infer_freq_two_day",
        timestamps: &["2024-01-01", "2024-01-03", "2024-01-05", "2024-01-07"],
    })
}

#[test]
fn conformance_tseries_infer_freq_six_hour() -> Result<(), String> {
    assert_infer_freq_matches_pandas(InferFreqCase {
        case_id: "tseries_infer_freq_six_hour",
        timestamps: &[
            "2024-01-01 00:00:00",
            "2024-01-01 06:00:00",
            "2024-01-01 12:00:00",
            "2024-01-01 18:00:00",
        ],
    })
}

#[test]
fn conformance_tseries_infer_freq_business_day() -> Result<(), String> {
    assert_infer_freq_matches_pandas(InferFreqCase {
        case_id: "tseries_infer_freq_business_day",
        timestamps: &[
            "2024-01-01",
            "2024-01-02",
            "2024-01-03",
            "2024-01-04",
            "2024-01-05",
            "2024-01-08",
            "2024-01-09",
        ],
    })
}

#[test]
fn conformance_tseries_infer_freq_month_end_leap_year() -> Result<(), String> {
    assert_infer_freq_matches_pandas(InferFreqCase {
        case_id: "tseries_infer_freq_month_end_leap_year",
        timestamps: &["2024-01-31", "2024-02-29", "2024-03-31", "2024-04-30"],
    })
}

#[test]
fn conformance_tseries_infer_freq_irregular_returns_none() -> Result<(), String> {
    assert_infer_freq_matches_pandas(InferFreqCase {
        case_id: "tseries_infer_freq_irregular_returns_none",
        timestamps: &["2024-01-01", "2024-01-02", "2024-01-04", "2024-01-07"],
    })
}

#[test]
fn conformance_tseries_infer_freq_duplicate_returns_none() -> Result<(), String> {
    assert_infer_freq_matches_pandas(InferFreqCase {
        case_id: "tseries_infer_freq_duplicate_returns_none",
        timestamps: &["2024-01-01", "2024-01-02", "2024-01-02", "2024-01-03"],
    })
}

#[test]
fn conformance_tseries_infer_freq_too_few_dates_errors() -> Result<(), String> {
    assert_infer_freq_matches_pandas(InferFreqCase {
        case_id: "tseries_infer_freq_too_few_dates_errors",
        timestamps: &["2024-01-01", "2024-01-02"],
    })
}

fn run_pandas_oracle_eval(script: &str) -> serde_json::Value {
    let mut cfg = HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = true;
    let mut child = Command::new(&cfg.python_bin)
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn python oracle");
    child
        .stdin
        .take()
        .expect("python stdin")
        .write_all(script.as_bytes())
        .expect("write python script");
    let output = child.wait_with_output().expect("wait python oracle");
    assert!(
        output.status.success(),
        "temporal python oracle failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("valid temporal oracle json")
}

#[test]
fn live_oracle_datetime_index_tz_fields_rounding_and_conversion_differential() {
    // @covers: DatetimeIndex.tz_localize
    // @covers: DatetimeIndex.tz_convert
    // @covers: DatetimeIndex.tz
    // @covers: DatetimeIndex.tzinfo
    // @covers: DatetimeIndex.year
    // @covers: DatetimeIndex.month
    // @covers: DatetimeIndex.day
    // @covers: DatetimeIndex.hour
    // @covers: DatetimeIndex.minute
    // @covers: DatetimeIndex.second
    // @covers: DatetimeIndex.microsecond
    // @covers: DatetimeIndex.nanosecond
    // @covers: DatetimeIndex.date
    // @covers: DatetimeIndex.time
    // @covers: DatetimeIndex.timetz
    // @covers: DatetimeIndex.dayofweek
    // @covers: DatetimeIndex.day_of_week
    // @covers: DatetimeIndex.weekday
    // @covers: DatetimeIndex.dayofyear
    // @covers: DatetimeIndex.day_of_year
    // @covers: DatetimeIndex.quarter
    // @covers: DatetimeIndex.is_leap_year
    // @covers: DatetimeIndex.days_in_month
    // @covers: DatetimeIndex.daysinmonth
    // @covers: DatetimeIndex.is_month_start
    // @covers: DatetimeIndex.is_month_end
    // @covers: DatetimeIndex.is_quarter_start
    // @covers: DatetimeIndex.is_quarter_end
    // @covers: DatetimeIndex.is_year_start
    // @covers: DatetimeIndex.is_year_end
    // @covers: DatetimeIndex.month_name
    // @covers: DatetimeIndex.day_name
    // @covers: DatetimeIndex.isocalendar
    // @covers: DatetimeIndex.strftime
    // @covers: DatetimeIndex.to_period
    // @covers: DatetimeIndex.floor
    // @covers: DatetimeIndex.ceil
    // @covers: DatetimeIndex.round
    // @covers: DatetimeIndex.snap
    // @covers: DatetimeIndex.normalize
    // @covers: DatetimeIndex.is_normalized
    // @covers: DatetimeIndex.indexer_at_time
    // @covers: DatetimeIndex.indexer_between_time
    // @covers: DatetimeIndex.to_julian_date
    // @covers: DatetimeIndex.to_pydatetime
    use fp_index::DatetimeIndex;
    use fp_types::{AmbiguousTime, NonexistentTime, Timestamp};

    // Instant 0: 2024-01-01 02:30:45.123456789 UTC
    //   In America/New_York (-05:00): 2023-12-31 21:30:45.123456789 (different year, month, day, quarter, year_end, leap_year!)
    //   In Asia/Tokyo (+09:00):       2024-01-01 11:30:45.123456789
    // Instant 1: 2024-03-01 03:15:00.000000000 UTC
    //   In America/New_York (-05:00): 2024-02-29 22:15:00 (leap day month-end in NY, March 1 month-start in UTC!)
    // Instant 2: 2024-04-01 01:00:00.000000000 UTC
    //   In America/New_York (-04:00 EDT): 2024-03-31 21:00:00 (Q1 quarter-end in NY, Q2 quarter-start in UTC!)
    // Instant 3: NaT (i64::MIN)
    let t0 = Timestamp::parse("2024-01-01 02:30:45.123456789")
        .expect("t0")
        .nanos;
    let t1 = Timestamp::parse("2024-03-01 03:15:00").expect("t1").nanos;
    let t2 = Timestamp::parse("2024-04-01 01:00:00").expect("t2").nanos;
    let raw_nanos = vec![t0, t1, t2, i64::MIN];

    let naive = DatetimeIndex::new(raw_nanos.clone()).set_name("ts");
    let utc = naive.tz_localize(Some("UTC")).expect("utc localize");
    // NEGATIVE-UTC-offset zone (America/New_York: UTC-5 / UTC-4) and positive-UTC-offset zone (Asia/Tokyo: UTC+9)
    let ny = utc
        .tz_convert(Some("America/New_York"))
        .expect("ny convert");
    let tokyo = utc.tz_convert(Some("Asia/Tokyo")).expect("tokyo convert");

    let oracle = run_pandas_oracle_eval(
        r#"
import json
import pandas as pd

idx_naive = pd.DatetimeIndex([
    "2024-01-01 02:30:45.123456789",
    "2024-03-01 03:15:00",
    "2024-04-01 01:00:00",
    pd.NaT,
], name="ts")
idx_utc = idx_naive.tz_localize("UTC")
idx_ny = idx_utc.tz_convert("America/New_York")
idx_tokyo = idx_utc.tz_convert("Asia/Tokyo")

def opt_int(vals):
    return [None if pd.isna(v) else int(v) for v in vals]

def opt_bool(vals, mask):
    return [None if m else bool(v) for v, m in zip(vals, mask)]

def opt_str(vals):
    return [None if pd.isna(v) else str(v) for v in vals]

def dump_dti(dti):
    na_mask = dti.isna()
    iso = dti.isocalendar()
    iso_rows = []
    for i in range(len(dti)):
        if na_mask[i]:
            iso_rows.append(None)
        else:
            row = iso.iloc[i]
            iso_rows.append([int(row["year"]), int(row["week"]), int(row["day"])])
    return {
        "tz": None if dti.tz is None else str(dti.tz),
        "year": opt_int(dti.year),
        "month": opt_int(dti.month),
        "day": opt_int(dti.day),
        "hour": opt_int(dti.hour),
        "minute": opt_int(dti.minute),
        "second": opt_int(dti.second),
        "microsecond": opt_int(dti.microsecond),
        "nanosecond": opt_int(dti.nanosecond),
        "date": [None if pd.isna(v) else v.isoformat() for v in dti.date],
        "time": [None if pd.isna(v) else v.strftime("%H:%M:%S.%f") for v in dti.time],
        "dayofweek": opt_int(dti.dayofweek),
        "day_of_week": opt_int(dti.day_of_week),
        "weekday": opt_int(dti.weekday),
        "dayofyear": opt_int(dti.dayofyear),
        "day_of_year": opt_int(dti.day_of_year),
        "quarter": opt_int(dti.quarter),
        "is_leap_year": opt_bool(dti.is_leap_year, na_mask),
        "days_in_month": opt_int(dti.days_in_month),
        "daysinmonth": opt_int(dti.daysinmonth),
        "is_month_start": opt_bool(dti.is_month_start, na_mask),
        "is_month_end": opt_bool(dti.is_month_end, na_mask),
        "is_quarter_start": opt_bool(dti.is_quarter_start, na_mask),
        "is_quarter_end": opt_bool(dti.is_quarter_end, na_mask),
        "is_year_start": opt_bool(dti.is_year_start, na_mask),
        "is_year_end": opt_bool(dti.is_year_end, na_mask),
        "month_name": opt_str(dti.month_name()),
        "day_name": opt_str(dti.day_name()),
        "isocalendar": iso_rows,
        "strftime": opt_str(dti.strftime("%Y-%m-%d %H:%M:%S %z")),
        "period_m": [int(v) for v in dti.to_period("M").asi8.tolist()],
        "floor_h": [int(v) for v in dti.floor("h").asi8.tolist()],
        "ceil_h": [int(v) for v in dti.ceil("h").asi8.tolist()],
        "round_h": [int(v) for v in dti.round("h").asi8.tolist()],
        "normalize": [int(v) for v in dti.normalize().asi8.tolist()],
        "is_normalized": bool(dti.is_normalized),
        "norm_is_normalized": bool(dti.normalize().is_normalized),
        "norm_dropna_is_normalized": bool(dti.dropna().normalize().is_normalized),
        "at_time": [int(v) for v in dti.indexer_at_time(dti[1].time().isoformat()).tolist()],
        "between_time": [int(v) for v in dti.indexer_between_time("20:00:00", "23:59:59").tolist()],
        "julian": [None if pd.isna(v) else float(v) for v in dti.to_julian_date()],
        "relocalized_none": [int(v) for v in dti.tz_localize(None).asi8.tolist()],
    }

# DST ambiguous & nonexistent in America/New_York
amb = pd.DatetimeIndex(["2024-11-03 01:30:00"])
nonex = pd.DatetimeIndex(["2024-03-10 02:30:00"])

out = {
    "naive": dump_dti(idx_naive),
    "ny": dump_dti(idx_ny),
    "tokyo": dump_dti(idx_tokyo),
    "amb_nat": [int(v) for v in amb.tz_localize("America/New_York", ambiguous="NaT").asi8.tolist()],
    "amb_true": [int(v) for v in amb.tz_localize("America/New_York", ambiguous=True).asi8.tolist()],
    "amb_false": [int(v) for v in amb.tz_localize("America/New_York", ambiguous=False).asi8.tolist()],
    "nonex_nat": [int(v) for v in nonex.tz_localize("America/New_York", nonexistent="NaT").asi8.tolist()],
    "nonex_fwd": [int(v) for v in nonex.tz_localize("America/New_York", nonexistent="shift_forward").asi8.tolist()],
    "nonex_bwd": [int(v) for v in nonex.tz_localize("America/New_York", nonexistent="shift_backward").asi8.tolist()],
}
print(json.dumps(out))
"#,
    );

    for (label, dti, expected_time_str) in [
        ("naive", &naive, "03:15:00"),
        ("ny", &ny, "22:15:00"),
        ("tokyo", &tokyo, "12:15:00"),
    ] {
        let exp = &oracle[label];
        let exp_tz = exp["tz"].as_str().map(str::to_owned);
        assert_eq!(dti.tz(), exp_tz, "{label}: tz");
        assert_eq!(dti.tzinfo(), exp_tz, "{label}: tzinfo");

        let to_opt_i32 = |key: &str| -> Vec<Option<i32>> {
            exp[key]
                .as_array()
                .expect("array")
                .iter()
                .map(|v| v.as_i64().map(|n| n as i32))
                .collect()
        };
        let to_opt_u32 = |key: &str| -> Vec<Option<u32>> {
            exp[key]
                .as_array()
                .expect("array")
                .iter()
                .map(|v| v.as_u64().map(|n| n as u32))
                .collect()
        };
        let to_opt_bool = |key: &str| -> Vec<Option<bool>> {
            exp[key]
                .as_array()
                .expect("array")
                .iter()
                .map(serde_json::Value::as_bool)
                .collect()
        };
        let to_opt_str = |key: &str| -> Vec<Option<String>> {
            exp[key]
                .as_array()
                .expect("array")
                .iter()
                .map(|v| v.as_str().map(str::to_owned))
                .collect()
        };
        let to_i64_vec = |key: &str| -> Vec<i64> {
            exp[key]
                .as_array()
                .expect("array")
                .iter()
                .map(|v| v.as_i64().expect("i64"))
                .collect()
        };

        assert_eq!(dti.year(), to_opt_i32("year"), "{label}: year");
        assert_eq!(dti.month(), to_opt_u32("month"), "{label}: month");
        assert_eq!(dti.day(), to_opt_u32("day"), "{label}: day");
        assert_eq!(dti.hour(), to_opt_u32("hour"), "{label}: hour");
        assert_eq!(dti.minute(), to_opt_u32("minute"), "{label}: minute");
        assert_eq!(dti.second(), to_opt_u32("second"), "{label}: second");
        assert_eq!(
            dti.microsecond(),
            to_opt_u32("microsecond"),
            "{label}: microsecond"
        );
        assert_eq!(
            dti.nanosecond(),
            to_opt_u32("nanosecond"),
            "{label}: nanosecond"
        );

        let actual_dates: Vec<Option<String>> = dti
            .date()
            .into_iter()
            .map(|d| d.map(|v| v.format("%Y-%m-%d").to_string()))
            .collect();
        assert_eq!(actual_dates, to_opt_str("date"), "{label}: date");

        let actual_times: Vec<Option<String>> = dti
            .time()
            .into_iter()
            .map(|t| t.map(|v| v.format("%H:%M:%S%.6f").to_string()))
            .collect();
        assert_eq!(actual_times, to_opt_str("time"), "{label}: time");
        assert_eq!(dti.timetz(), dti.time(), "{label}: timetz");

        assert_eq!(
            dti.dayofweek(),
            to_opt_u32("dayofweek"),
            "{label}: dayofweek"
        );
        assert_eq!(
            dti.day_of_week(),
            to_opt_u32("day_of_week"),
            "{label}: day_of_week"
        );
        assert_eq!(dti.weekday(), to_opt_u32("weekday"), "{label}: weekday");
        assert_eq!(
            dti.dayofyear(),
            to_opt_u32("dayofyear"),
            "{label}: dayofyear"
        );
        assert_eq!(
            dti.day_of_year(),
            to_opt_u32("day_of_year"),
            "{label}: day_of_year"
        );
        assert_eq!(dti.quarter(), to_opt_u32("quarter"), "{label}: quarter");
        assert_eq!(
            dti.is_leap_year(),
            to_opt_bool("is_leap_year"),
            "{label}: is_leap_year"
        );
        assert_eq!(
            dti.days_in_month(),
            to_opt_u32("days_in_month"),
            "{label}: days_in_month"
        );
        assert_eq!(
            dti.daysinmonth(),
            to_opt_u32("daysinmonth"),
            "{label}: daysinmonth"
        );
        assert_eq!(
            dti.is_month_start(),
            to_opt_bool("is_month_start"),
            "{label}: is_month_start"
        );
        assert_eq!(
            dti.is_month_end(),
            to_opt_bool("is_month_end"),
            "{label}: is_month_end"
        );
        assert_eq!(
            dti.is_quarter_start(),
            to_opt_bool("is_quarter_start"),
            "{label}: is_quarter_start"
        );
        assert_eq!(
            dti.is_quarter_end(),
            to_opt_bool("is_quarter_end"),
            "{label}: is_quarter_end"
        );
        assert_eq!(
            dti.is_year_start(),
            to_opt_bool("is_year_start"),
            "{label}: is_year_start"
        );
        assert_eq!(
            dti.is_year_end(),
            to_opt_bool("is_year_end"),
            "{label}: is_year_end"
        );
        assert_eq!(
            dti.month_name(),
            to_opt_str("month_name"),
            "{label}: month_name"
        );
        assert_eq!(dti.day_name(), to_opt_str("day_name"), "{label}: day_name");

        let exp_iso: Vec<Option<(i32, u32, u32)>> = exp["isocalendar"]
            .as_array()
            .expect("iso array")
            .iter()
            .map(|row| {
                row.as_array().map(|arr| {
                    (
                        arr[0].as_i64().expect("y") as i32,
                        arr[1].as_u64().expect("w") as u32,
                        arr[2].as_u64().expect("d") as u32,
                    )
                })
            })
            .collect();
        assert_eq!(dti.isocalendar(), exp_iso, "{label}: isocalendar");

        assert_eq!(
            dti.strftime("%Y-%m-%d %H:%M:%S %z"),
            to_opt_str("strftime"),
            "{label}: strftime"
        );
        assert_eq!(
            dti.to_period("M").expect("to_period").asi8(),
            to_i64_vec("period_m"),
            "{label}: to_period(M)"
        );
        assert_eq!(
            dti.floor("h").expect("floor").asi8(),
            to_i64_vec("floor_h"),
            "{label}: floor(h)"
        );
        assert_eq!(
            dti.ceil("h").expect("ceil").asi8(),
            to_i64_vec("ceil_h"),
            "{label}: ceil(h)"
        );
        assert_eq!(
            dti.round("h").expect("round").asi8(),
            to_i64_vec("round_h"),
            "{label}: round(h)"
        );
        assert_eq!(
            dti.snap("D").expect("snap").asi8(),
            dti.asi8(),
            "{label}: snap(D)"
        );
        let norm = dti.normalize().expect("normalize");
        assert_eq!(norm.asi8(), to_i64_vec("normalize"), "{label}: normalize");
        assert_eq!(
            dti.is_normalized(),
            exp["is_normalized"].as_bool().expect("bool"),
            "{label}: is_normalized"
        );
        assert_eq!(
            norm.is_normalized(),
            exp["norm_is_normalized"].as_bool().expect("bool"),
            "{label}: norm.is_normalized"
        );
        assert_eq!(
            dti.dropna()
                .normalize()
                .expect("norm dropna")
                .is_normalized(),
            exp["norm_dropna_is_normalized"].as_bool().expect("bool"),
            "{label}: norm_dropna.is_normalized"
        );
        let exp_at_time: Vec<usize> = exp["at_time"]
            .as_array()
            .expect("at_time")
            .iter()
            .map(|v| v.as_u64().expect("usize") as usize)
            .collect();
        assert_eq!(
            dti.indexer_at_time(expected_time_str)
                .expect("indexer_at_time"),
            exp_at_time,
            "{label}: indexer_at_time"
        );
        let exp_between: Vec<usize> = exp["between_time"]
            .as_array()
            .expect("between_time")
            .iter()
            .map(|v| v.as_u64().expect("usize") as usize)
            .collect();
        assert_eq!(
            dti.indexer_between_time("20:00:00", "23:59:59", true, true)
                .expect("indexer_between_time"),
            exp_between,
            "{label}: indexer_between_time"
        );

        let exp_julian: Vec<Option<f64>> = exp["julian"]
            .as_array()
            .expect("julian")
            .iter()
            .map(serde_json::Value::as_f64)
            .collect();
        let actual_julian = dti.to_julian_date();
        assert_eq!(actual_julian.len(), exp_julian.len());
        for (a, e) in actual_julian.iter().zip(exp_julian.iter()) {
            match (a, e) {
                (Some(av), Some(ev)) => {
                    assert!((av - ev).abs() < 1e-9, "{label}: julian {av} vs {ev}");
                }
                (None, None) => {}
                _ => assert_eq!(a, e, "{label}: julian mismatch"),
            }
        }
        let pydt = dti.to_pydatetime();
        assert_eq!(pydt.len(), 4);
        assert!(pydt[0].is_some() && pydt[3].is_none());

        assert_eq!(
            dti.tz_localize(None).expect("tz_localize(None)").asi8(),
            to_i64_vec("relocalized_none"),
            "{label}: tz_localize(None)"
        );
    }

    // Verify that NY (-05:00/-04:00) genuinely differs from naive on tz-dependent fields!
    assert_ne!(ny.year(), naive.year());
    assert_ne!(ny.month(), naive.month());
    assert_ne!(ny.day(), naive.day());
    assert_ne!(ny.hour(), naive.hour());
    assert_ne!(
        ny.dropna().to_period("M").expect("ny period").asi8(),
        naive.dropna().to_period("M").expect("naive period").asi8()
    );

    // DST Ambiguous & Nonexistent handling in America/New_York
    let amb = DatetimeIndex::new(vec![
        Timestamp::parse("2024-11-03 01:30:00").expect("amb").nanos,
    ]);
    assert!(amb.tz_localize(Some("America/New_York")).is_err());
    assert_eq!(
        amb.tz_localize_with(
            Some("America/New_York"),
            AmbiguousTime::NaT,
            NonexistentTime::Raise,
        )
        .expect("amb NaT")
        .asi8(),
        vec![oracle["amb_nat"][0].as_i64().expect("i64")]
    );
    assert_eq!(
        amb.tz_localize_with(
            Some("America/New_York"),
            AmbiguousTime::Earliest,
            NonexistentTime::Raise,
        )
        .expect("amb true")
        .asi8(),
        vec![oracle["amb_true"][0].as_i64().expect("i64")]
    );
    assert_eq!(
        amb.tz_localize_with(
            Some("America/New_York"),
            AmbiguousTime::Latest,
            NonexistentTime::Raise,
        )
        .expect("amb false")
        .asi8(),
        vec![oracle["amb_false"][0].as_i64().expect("i64")]
    );

    let nonex = DatetimeIndex::new(vec![
        Timestamp::parse("2024-03-10 02:30:00")
            .expect("nonex")
            .nanos,
    ]);
    assert!(nonex.tz_localize(Some("America/New_York")).is_err());
    assert_eq!(
        nonex
            .tz_localize_with(
                Some("America/New_York"),
                AmbiguousTime::Raise,
                NonexistentTime::NaT,
            )
            .expect("nonex NaT")
            .asi8(),
        vec![oracle["nonex_nat"][0].as_i64().expect("i64")]
    );
    assert_eq!(
        nonex
            .tz_localize_with(
                Some("America/New_York"),
                AmbiguousTime::Raise,
                NonexistentTime::ShiftForward,
            )
            .expect("nonex fwd")
            .asi8(),
        vec![oracle["nonex_fwd"][0].as_i64().expect("i64")]
    );
    assert_eq!(
        nonex
            .tz_localize_with(
                Some("America/New_York"),
                AmbiguousTime::Raise,
                NonexistentTime::ShiftBackward,
            )
            .expect("nonex bwd")
            .asi8(),
        vec![oracle["nonex_bwd"][0].as_i64().expect("i64")]
    );

    // NEGATIVE: localizing an already tz-aware index or converting a tz-naive index must fail
    assert!(ny.tz_localize(Some("UTC")).is_err());
    assert!(naive.tz_convert(Some("America/New_York")).is_err());
}

#[test]
fn live_oracle_datetime_index_set_ops_indexing_reductions_and_metadata_differential() {
    // @covers: DatetimeIndex.T
    // @covers: DatetimeIndex.all
    // @covers: DatetimeIndex.any
    // @covers: DatetimeIndex.append
    // @covers: DatetimeIndex.argmax
    // @covers: DatetimeIndex.argmin
    // @covers: DatetimeIndex.argsort
    // @covers: DatetimeIndex.array
    // @covers: DatetimeIndex.as_unit
    // @covers: DatetimeIndex.asi8
    // @covers: DatetimeIndex.asof
    // @covers: DatetimeIndex.asof_locs
    // @covers: DatetimeIndex.astype
    // @covers: DatetimeIndex.copy
    // @covers: DatetimeIndex.delete
    // @covers: DatetimeIndex.diff
    // @covers: DatetimeIndex.difference
    // @covers: DatetimeIndex.drop
    // @covers: DatetimeIndex.drop_duplicates
    // @covers: DatetimeIndex.droplevel
    // @covers: DatetimeIndex.dropna
    // @covers: DatetimeIndex.dtype
    // @covers: DatetimeIndex.duplicated
    // @covers: DatetimeIndex.empty
    // @covers: DatetimeIndex.equals
    // @covers: DatetimeIndex.factorize
    // @covers: DatetimeIndex.fillna
    // @covers: DatetimeIndex.format
    // @covers: DatetimeIndex.freq
    // @covers: DatetimeIndex.freqstr
    // @covers: DatetimeIndex.get_indexer
    // @covers: DatetimeIndex.get_indexer_for
    // @covers: DatetimeIndex.get_indexer_non_unique
    // @covers: DatetimeIndex.get_level_values
    // @covers: DatetimeIndex.get_loc
    // @covers: DatetimeIndex.get_slice_bound
    // @covers: DatetimeIndex.groupby
    // @covers: DatetimeIndex.has_duplicates
    // @covers: DatetimeIndex.hasnans
    // @covers: DatetimeIndex.holds_integer
    // @covers: DatetimeIndex.identical
    // @covers: DatetimeIndex.infer_objects
    // @covers: DatetimeIndex.inferred_freq
    // @covers: DatetimeIndex.inferred_type
    // @covers: DatetimeIndex.insert
    // @covers: DatetimeIndex.intersection
    // @covers: DatetimeIndex.is_
    // @covers: DatetimeIndex.is_boolean
    // @covers: DatetimeIndex.is_categorical
    // @covers: DatetimeIndex.is_floating
    // @covers: DatetimeIndex.is_integer
    // @covers: DatetimeIndex.is_interval
    // @covers: DatetimeIndex.is_monotonic_decreasing
    // @covers: DatetimeIndex.is_monotonic_increasing
    // @covers: DatetimeIndex.is_numeric
    // @covers: DatetimeIndex.is_object
    // @covers: DatetimeIndex.is_unique
    // @covers: DatetimeIndex.isin
    // @covers: DatetimeIndex.isna
    // @covers: DatetimeIndex.isnull
    // @covers: DatetimeIndex.item
    // @covers: DatetimeIndex.join
    // @covers: DatetimeIndex.map
    // @covers: DatetimeIndex.max
    // @covers: DatetimeIndex.mean
    // @covers: DatetimeIndex.memory_usage
    // @covers: DatetimeIndex.min
    // @covers: DatetimeIndex.name
    // @covers: DatetimeIndex.names
    // @covers: DatetimeIndex.nbytes
    // @covers: DatetimeIndex.ndim
    // @covers: DatetimeIndex.nlevels
    // @covers: DatetimeIndex.notna
    // @covers: DatetimeIndex.notnull
    // @covers: DatetimeIndex.nunique
    // @covers: DatetimeIndex.putmask
    // @covers: DatetimeIndex.ravel
    // @covers: DatetimeIndex.reindex
    // @covers: DatetimeIndex.rename
    // @covers: DatetimeIndex.repeat
    // @covers: DatetimeIndex.resolution
    // @covers: DatetimeIndex.searchsorted
    // @covers: DatetimeIndex.set_names
    // @covers: DatetimeIndex.shape
    // @covers: DatetimeIndex.shift
    // @covers: DatetimeIndex.size
    // @covers: DatetimeIndex.slice_indexer
    // @covers: DatetimeIndex.slice_locs
    // @covers: DatetimeIndex.sort
    // @covers: DatetimeIndex.sort_values
    // @covers: DatetimeIndex.sortlevel
    // @covers: DatetimeIndex.std
    // @covers: DatetimeIndex.str
    // @covers: DatetimeIndex.symmetric_difference
    // @covers: DatetimeIndex.take
    // @covers: DatetimeIndex.to_flat_index
    // @covers: DatetimeIndex.to_frame
    // @covers: DatetimeIndex.to_list
    // @covers: DatetimeIndex.to_numpy
    // @covers: DatetimeIndex.to_series
    // @covers: DatetimeIndex.tolist
    // @covers: DatetimeIndex.transpose
    // @covers: DatetimeIndex.union
    // @covers: DatetimeIndex.unique
    // @covers: DatetimeIndex.unit
    // @covers: DatetimeIndex.value_counts
    // @covers: DatetimeIndex.values
    // @covers: DatetimeIndex.view
    // @covers: DatetimeIndex.where
    use fp_index::{DatetimeIndex, DuplicateKeep};
    use fp_types::Timestamp;

    let d0 = Timestamp::parse("2024-01-01 00:00:00").expect("d0").nanos;
    let d1 = Timestamp::parse("2024-01-02 00:00:00").expect("d1").nanos;
    let d2 = Timestamp::parse("2024-01-03 00:00:00").expect("d2").nanos;
    let d3 = Timestamp::parse("2024-01-04 00:00:00").expect("d3").nanos;
    let d_fill = Timestamp::parse("2024-01-05 00:00:00")
        .expect("d_fill")
        .nanos;

    let sorted = DatetimeIndex::new(vec![d0, d1, d2, d3])
        .set_name("dt")
        .with_freq(Some("D".to_owned()));
    let other = DatetimeIndex::new(vec![d1, d3, d_fill]).set_name("dt");
    let with_dups_and_nat = DatetimeIndex::new(vec![d2, d0, d2, i64::MIN]).set_name("dt");

    let oracle = run_pandas_oracle_eval(
        r#"
import json
import pandas as pd

s = pd.DatetimeIndex(["2024-01-01", "2024-01-02", "2024-01-03", "2024-01-04"], name="dt", freq="D")
other = pd.DatetimeIndex(["2024-01-02", "2024-01-04", "2024-01-05"], name="dt")
dups = pd.DatetimeIndex(["2024-01-03", "2024-01-01", "2024-01-03", pd.NaT], name="dt")
fill_ts = pd.Timestamp("2024-01-05")

codes, uniques = dups.factorize()
out = {
    "asi8": [int(v) for v in s.asi8.tolist()],
    "inferred_freq": s.inferred_freq,
    "freqstr": s.freqstr,
    "unit": str(s.unit),
    "resolution_ns": str(pd.DatetimeIndex([s[0] + pd.Timedelta(1, unit="ns")]).resolution),
    "min": int(s.min().value),
    "max": int(s.max().value),
    "mean": int(s.mean().value),
    "std": int(s.std().value),
    "argmin": int(s.argmin()),
    "argmax": int(s.argmax()),
    "argsort_dups": [int(v) for v in s.argsort().tolist()],
    "shift_2d": [int(v) for v in s.shift(2, freq="D").asi8.tolist()],
    "diff_1": [int(v) for v in s.diff(1).asi8.tolist()],
    "intersection": [int(v) for v in s.intersection(other).asi8.tolist()],
    "union": [int(v) for v in s.union(other).asi8.tolist()],
    "difference": [int(v) for v in s.difference(other).asi8.tolist()],
    "sym_diff": [int(v) for v in s.symmetric_difference(other).asi8.tolist()],
    "unique": [int(v) for v in dups.unique().asi8.tolist()],
    "nunique": int(dups.nunique()),
    "factorize_codes": [int(v) for v in codes.tolist()],
    "factorize_uniques": [int(v) for v in uniques.asi8.tolist()],
    "duplicated_first": [bool(v) for v in dups.duplicated(keep="first").tolist()],
    "drop_duplicates": [int(v) for v in dups.drop_duplicates().asi8.tolist()],
    "dropna": [int(v) for v in dups.dropna().asi8.tolist()],
    "fillna": [int(v) for v in dups.fillna(fill_ts).asi8.tolist()],
    "isna": [bool(v) for v in dups.isna().tolist()],
    "notna": [bool(v) for v in dups.notna().tolist()],
    "hasnans": bool(dups.hasnans),
    "isin": [bool(v) for v in s.isin([pd.Timestamp("2024-01-02"), pd.Timestamp("2024-01-04")]).tolist()],
    "where": [int(v) for v in s.where([True, False, True, False], fill_ts).asi8.tolist()],
    "putmask": [int(v) for v in s.putmask([False, True, False, True], fill_ts).asi8.tolist()],
    "take": [int(v) for v in s.take([3, 0, 2]).asi8.tolist()],
    "repeat": [int(v) for v in s.repeat(2).asi8.tolist()],
    "append": [int(v) for v in s.append(other).asi8.tolist()],
    "insert": [int(v) for v in s.insert(1, fill_ts).asi8.tolist()],
    "delete": [int(v) for v in s.delete(2).asi8.tolist()],
    "sort_values": [int(v) for v in dups.sort_values().asi8.tolist()],
    "get_loc": int(s.get_loc(pd.Timestamp("2024-01-03"))),
    "get_indexer": [int(v) for v in s.get_indexer(other).tolist()],
    "searchsorted_left": int(s.searchsorted(pd.Timestamp("2024-01-03"), side="left")),
    "searchsorted_right": int(s.searchsorted(pd.Timestamp("2024-01-03"), side="right")),
    "slice_locs": [int(v) for v in s.slice_locs(pd.Timestamp("2024-01-02"), pd.Timestamp("2024-01-03"))],
    "slice_bound_left": int(s.get_slice_bound(pd.Timestamp("2024-01-02"), side="left")),
    "slice_bound_right": int(s.get_slice_bound(pd.Timestamp("2024-01-03"), side="right")),
}
print(json.dumps(out))
"#,
    );

    let to_i64_vec = |key: &str| -> Vec<i64> {
        oracle[key]
            .as_array()
            .expect("array")
            .iter()
            .map(|v| v.as_i64().expect("i64"))
            .collect()
    };
    let to_bool_vec = |key: &str| -> Vec<bool> {
        oracle[key]
            .as_array()
            .expect("array")
            .iter()
            .map(|v| v.as_bool().expect("bool"))
            .collect()
    };

    assert_eq!(sorted.asi8(), to_i64_vec("asi8"));
    assert_eq!(sorted.freq(), Some("D".to_owned()));
    assert_eq!(
        sorted.freqstr(),
        oracle["freqstr"].as_str().map(str::to_owned)
    );
    assert_eq!(
        sorted.inferred_freq(),
        oracle["inferred_freq"].as_str().map(str::to_owned)
    );
    assert_eq!(sorted.unit(), oracle["unit"].as_str().expect("unit"));
    assert_eq!(sorted.as_unit("ns").expect("as_unit").asi8(), sorted.asi8());
    assert_eq!(sorted.resolution(), "day");
    assert_eq!(
        DatetimeIndex::new(vec![d0 + 1]).resolution(),
        oracle["resolution_ns"].as_str().expect("resolution")
    );
    assert_eq!(sorted.min(), oracle["min"].as_i64());
    assert_eq!(sorted.max(), oracle["max"].as_i64());
    assert_eq!(sorted.mean(), oracle["mean"].as_i64());
    assert_eq!(sorted.median(), oracle["mean"].as_i64());
    assert_eq!(sorted.std(), oracle["std"].as_i64());
    assert_eq!(
        sorted.argmin().expect("argmin"),
        oracle["argmin"].as_u64().expect("u64") as usize
    );
    assert_eq!(
        sorted.argmax().expect("argmax"),
        oracle["argmax"].as_u64().expect("u64") as usize
    );
    assert_eq!(
        sorted.argsort(),
        oracle["argsort_dups"]
            .as_array()
            .expect("arr")
            .iter()
            .map(|v| v.as_u64().expect("u64") as usize)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        sorted.shift(2, 86_400_000_000_000).asi8(),
        to_i64_vec("shift_2d")
    );
    assert_eq!(sorted.diff(1).asi8(), to_i64_vec("diff_1"));
    assert_eq!(
        sorted.intersection(&other).asi8(),
        to_i64_vec("intersection")
    );
    assert_eq!(sorted.union(&other).asi8(), to_i64_vec("union"));
    assert_eq!(sorted.difference(&other).asi8(), to_i64_vec("difference"));
    assert_eq!(
        sorted.symmetric_difference(&other).asi8(),
        to_i64_vec("sym_diff")
    );

    assert_eq!(
        with_dups_and_nat.unique().expect("unique").asi8(),
        to_i64_vec("unique")
    );
    assert_eq!(
        with_dups_and_nat.nunique(),
        oracle["nunique"].as_u64().expect("u64") as usize
    );
    let (codes, uniques) = with_dups_and_nat.factorize().expect("factorize");
    let exp_codes: Vec<isize> = oracle["factorize_codes"]
        .as_array()
        .expect("arr")
        .iter()
        .map(|v| v.as_i64().expect("i64") as isize)
        .collect();
    assert_eq!(codes, exp_codes);
    assert_eq!(uniques.asi8(), to_i64_vec("factorize_uniques"));
    assert_eq!(with_dups_and_nat.value_counts().len(), 2);
    assert_eq!(
        with_dups_and_nat.duplicated(DuplicateKeep::First),
        to_bool_vec("duplicated_first")
    );
    assert_eq!(
        with_dups_and_nat
            .drop_duplicates()
            .expect("drop_duplicates")
            .asi8(),
        to_i64_vec("drop_duplicates")
    );
    assert_eq!(with_dups_and_nat.dropna().asi8(), to_i64_vec("dropna"));
    assert_eq!(
        with_dups_and_nat.fillna(d_fill).asi8(),
        to_i64_vec("fillna")
    );
    assert_eq!(with_dups_and_nat.isna(), to_bool_vec("isna"));
    assert_eq!(with_dups_and_nat.isnull(), to_bool_vec("isna"));
    assert_eq!(with_dups_and_nat.notna(), to_bool_vec("notna"));
    assert_eq!(with_dups_and_nat.notnull(), to_bool_vec("notna"));
    assert_eq!(
        with_dups_and_nat.hasnans(),
        oracle["hasnans"].as_bool().expect("bool")
    );
    assert_eq!(sorted.isin(&[d1, d3]), to_bool_vec("isin"));
    assert_eq!(
        sorted
            .r#where(&[true, false, true, false], d_fill)
            .expect("where")
            .asi8(),
        to_i64_vec("where")
    );
    assert_eq!(
        sorted
            .putmask(&[false, true, false, true], d_fill)
            .expect("putmask")
            .asi8(),
        to_i64_vec("putmask")
    );
    assert_eq!(
        sorted.take(&[3, 0, 2]).expect("take").asi8(),
        to_i64_vec("take")
    );
    assert_eq!(sorted.repeat(2).asi8(), to_i64_vec("repeat"));
    assert_eq!(sorted.append(&other).asi8(), to_i64_vec("append"));
    assert_eq!(
        sorted.insert(1, d_fill).expect("insert").asi8(),
        to_i64_vec("insert")
    );
    assert_eq!(
        sorted.delete(2).expect("delete").asi8(),
        to_i64_vec("delete")
    );
    assert_eq!(
        with_dups_and_nat.sort_values().asi8(),
        to_i64_vec("sort_values")
    );
    assert_eq!(with_dups_and_nat.sort().asi8(), to_i64_vec("sort_values"));

    assert_eq!(
        sorted.get_loc(d2).expect("get_loc"),
        oracle["get_loc"].as_u64().expect("u64") as usize
    );
    let exp_indexer: Vec<isize> = oracle["get_indexer"]
        .as_array()
        .expect("arr")
        .iter()
        .map(|v| v.as_i64().expect("i64") as isize)
        .collect();
    let other_targets = vec![d1, d3, d_fill];
    assert_eq!(sorted.get_indexer(&other_targets), exp_indexer);
    assert_eq!(sorted.get_indexer_for(&other_targets), exp_indexer);
    let (non_uniq_idx, missing_pos) = sorted.get_indexer_non_unique(&other_targets);
    assert_eq!(non_uniq_idx, exp_indexer);
    assert_eq!(missing_pos, vec![2]);
    let (reindexed, reindex_indexer) = sorted.reindex(&other);
    assert_eq!(reindexed.asi8(), other.asi8());
    assert_eq!(reindex_indexer, exp_indexer);

    assert_eq!(
        sorted.searchsorted(d2, "left").expect("searchsorted left"),
        oracle["searchsorted_left"].as_u64().expect("u64") as usize
    );
    assert_eq!(
        sorted
            .searchsorted(d2, "right")
            .expect("searchsorted right"),
        oracle["searchsorted_right"].as_u64().expect("u64") as usize
    );
    let exp_slice_locs = (
        oracle["slice_locs"][0].as_u64().expect("u64") as usize,
        oracle["slice_locs"][1].as_u64().expect("u64") as usize,
    );
    assert_eq!(
        sorted.slice_locs(d1, d2).expect("slice_locs"),
        exp_slice_locs
    );
    assert_eq!(
        sorted.slice_indexer(d1, d2).expect("slice_indexer"),
        exp_slice_locs.0..exp_slice_locs.1
    );
    assert_eq!(
        sorted.get_slice_bound(d1, "left").expect("bound left"),
        oracle["slice_bound_left"].as_u64().expect("u64") as usize
    );
    assert_eq!(
        sorted.get_slice_bound(d2, "right").expect("bound right"),
        oracle["slice_bound_right"].as_u64().expect("u64") as usize
    );

    // Metadata, shape, type predicates, and Index conversions
    assert_eq!(sorted.shape(), (4,));
    assert_eq!(sorted.size(), 4);
    assert!(!sorted.empty());
    assert_eq!(sorted.ndim(), 1);
    assert_eq!(sorted.nlevels(), 1);
    assert_eq!(sorted.dtype(), "datetime64[ns]");
    assert!(sorted.memory_usage(false) >= 32);
    assert_eq!(sorted.nbytes(), sorted.memory_usage(false));
    assert!(sorted.is_unique());
    assert!(!sorted.has_duplicates());
    assert!(sorted.is_monotonic_increasing());
    assert!(!sorted.is_monotonic_decreasing());
    assert!(!sorted.holds_integer());
    assert_eq!(sorted.inferred_type(), "datetime64");
    assert!(!sorted.is_boolean());
    assert!(!sorted.is_categorical());
    assert!(!sorted.is_floating());
    assert!(!sorted.is_integer());
    assert!(!sorted.is_interval());
    assert!(!sorted.is_numeric());
    assert!(!sorted.is_object());
    assert!(sorted.equals(&sorted.copy()));
    assert!(sorted.identical(&sorted.copy()));
    assert!(sorted.is_(&sorted));
    assert!(!sorted.is_(&sorted.copy()));
    assert_eq!(sorted.name().map(|n| n.as_str()), Some("dt"));
    assert_eq!(sorted.names(), vec![Some("dt".to_owned())]);
    assert_eq!(
        sorted.set_names(Some("dt2")).names(),
        vec![Some("dt2".to_owned())]
    );
    assert_eq!(sorted.rename("dt3").names(), vec![Some("dt3".to_owned())]);
    assert_eq!(sorted.view().asi8(), sorted.asi8());
    assert_eq!(sorted.transpose().asi8(), sorted.asi8());
    assert_eq!(sorted.T().asi8(), sorted.asi8());
    assert_eq!(sorted.ravel(), sorted.values());
    assert_eq!(sorted.to_list(), sorted.values());
    assert_eq!(sorted.tolist(), sorted.values());
    assert_eq!(sorted.to_numpy(), sorted.values());
    assert_eq!(sorted.array(), sorted.values());
    assert_eq!(sorted.infer_objects().asi8(), sorted.asi8());
    assert_eq!(sorted.format().len(), 4);
    assert_eq!(sorted.to_flat_index().len(), 4);
    assert_eq!(sorted.to_frame().len(), 4);
    assert_eq!(sorted.to_series().len(), 4);
    assert_eq!(sorted.r#str().len(), vec![None; 4]);
    assert!(sorted.any());
    assert!(sorted.all());
    assert_eq!(sorted.get_level_values(0).expect("level 0").len(), 4);
    assert!(sorted.droplevel(0).is_err());
    assert_eq!(sorted.groupby_positions().len(), 4);
    assert_eq!(
        sorted
            .groupby(sorted.to_flat_index().labels())
            .expect("groupby")
            .len(),
        4
    );
    assert_eq!(sorted.map(|lbl| lbl.clone()).len(), 4);
    assert_eq!(sorted.astype("string").expect("astype").len(), 4);
    assert_eq!(
        sorted.asof(&IndexLabel::Datetime64(d2 + 1_000)),
        Some(IndexLabel::Datetime64(d2))
    );
    assert_eq!(sorted.asof_locs(other.as_index(), None).len(), 3);
    assert_eq!(sorted.drop(&[IndexLabel::Datetime64(d1)]).len(), 3);
    assert_eq!(
        sorted.join(other.as_index(), "inner").expect("join").len(),
        2
    );
    assert_eq!(sorted.sortlevel().0.len(), 4);

    let single = DatetimeIndex::new(vec![d0]);
    assert_eq!(single.item().expect("item"), Some(d0));

    // NEGATIVE checks
    assert!(sorted.item().is_err());
    assert!(sorted.get_loc(d_fill).is_err());
    assert!(sorted.as_unit("s").is_err());
}

#[test]
fn live_oracle_timedelta_index_components_rounding_reductions_and_ops_differential() {
    // @covers: TimedeltaIndex.T
    // @covers: TimedeltaIndex.all
    // @covers: TimedeltaIndex.any
    // @covers: TimedeltaIndex.append
    // @covers: TimedeltaIndex.argmax
    // @covers: TimedeltaIndex.argmin
    // @covers: TimedeltaIndex.argsort
    // @covers: TimedeltaIndex.array
    // @covers: TimedeltaIndex.as_unit
    // @covers: TimedeltaIndex.asi8
    // @covers: TimedeltaIndex.asof
    // @covers: TimedeltaIndex.asof_locs
    // @covers: TimedeltaIndex.astype
    // @covers: TimedeltaIndex.ceil
    // @covers: TimedeltaIndex.components
    // @covers: TimedeltaIndex.copy
    // @covers: TimedeltaIndex.days
    // @covers: TimedeltaIndex.delete
    // @covers: TimedeltaIndex.diff
    // @covers: TimedeltaIndex.difference
    // @covers: TimedeltaIndex.drop
    // @covers: TimedeltaIndex.drop_duplicates
    // @covers: TimedeltaIndex.droplevel
    // @covers: TimedeltaIndex.dropna
    // @covers: TimedeltaIndex.dtype
    // @covers: TimedeltaIndex.duplicated
    // @covers: TimedeltaIndex.empty
    // @covers: TimedeltaIndex.equals
    // @covers: TimedeltaIndex.factorize
    // @covers: TimedeltaIndex.fillna
    // @covers: TimedeltaIndex.floor
    // @covers: TimedeltaIndex.format
    // @covers: TimedeltaIndex.freq
    // @covers: TimedeltaIndex.freqstr
    // @covers: TimedeltaIndex.get_indexer
    // @covers: TimedeltaIndex.get_indexer_for
    // @covers: TimedeltaIndex.get_indexer_non_unique
    // @covers: TimedeltaIndex.get_level_values
    // @covers: TimedeltaIndex.get_loc
    // @covers: TimedeltaIndex.get_slice_bound
    // @covers: TimedeltaIndex.groupby
    // @covers: TimedeltaIndex.has_duplicates
    // @covers: TimedeltaIndex.hasnans
    // @covers: TimedeltaIndex.holds_integer
    // @covers: TimedeltaIndex.identical
    // @covers: TimedeltaIndex.infer_objects
    // @covers: TimedeltaIndex.inferred_freq
    // @covers: TimedeltaIndex.inferred_type
    // @covers: TimedeltaIndex.insert
    // @covers: TimedeltaIndex.intersection
    // @covers: TimedeltaIndex.is_
    // @covers: TimedeltaIndex.is_boolean
    // @covers: TimedeltaIndex.is_categorical
    // @covers: TimedeltaIndex.is_floating
    // @covers: TimedeltaIndex.is_integer
    // @covers: TimedeltaIndex.is_interval
    // @covers: TimedeltaIndex.is_monotonic_decreasing
    // @covers: TimedeltaIndex.is_monotonic_increasing
    // @covers: TimedeltaIndex.is_numeric
    // @covers: TimedeltaIndex.is_object
    // @covers: TimedeltaIndex.is_unique
    // @covers: TimedeltaIndex.isin
    // @covers: TimedeltaIndex.isna
    // @covers: TimedeltaIndex.isnull
    // @covers: TimedeltaIndex.item
    // @covers: TimedeltaIndex.join
    // @covers: TimedeltaIndex.map
    // @covers: TimedeltaIndex.max
    // @covers: TimedeltaIndex.mean
    // @covers: TimedeltaIndex.median
    // @covers: TimedeltaIndex.memory_usage
    // @covers: TimedeltaIndex.microseconds
    // @covers: TimedeltaIndex.min
    // @covers: TimedeltaIndex.name
    // @covers: TimedeltaIndex.names
    // @covers: TimedeltaIndex.nanoseconds
    // @covers: TimedeltaIndex.nbytes
    // @covers: TimedeltaIndex.ndim
    // @covers: TimedeltaIndex.nlevels
    // @covers: TimedeltaIndex.notna
    // @covers: TimedeltaIndex.notnull
    // @covers: TimedeltaIndex.nunique
    // @covers: TimedeltaIndex.putmask
    // @covers: TimedeltaIndex.ravel
    // @covers: TimedeltaIndex.reindex
    // @covers: TimedeltaIndex.rename
    // @covers: TimedeltaIndex.repeat
    // @covers: TimedeltaIndex.resolution
    // @covers: TimedeltaIndex.round
    // @covers: TimedeltaIndex.searchsorted
    // @covers: TimedeltaIndex.seconds
    // @covers: TimedeltaIndex.set_names
    // @covers: TimedeltaIndex.shape
    // @covers: TimedeltaIndex.shift
    // @covers: TimedeltaIndex.size
    // @covers: TimedeltaIndex.slice_indexer
    // @covers: TimedeltaIndex.slice_locs
    // @covers: TimedeltaIndex.sort
    // @covers: TimedeltaIndex.sort_values
    // @covers: TimedeltaIndex.sortlevel
    // @covers: TimedeltaIndex.std
    // @covers: TimedeltaIndex.str
    // @covers: TimedeltaIndex.sum
    // @covers: TimedeltaIndex.symmetric_difference
    // @covers: TimedeltaIndex.take
    // @covers: TimedeltaIndex.to_flat_index
    // @covers: TimedeltaIndex.to_frame
    // @covers: TimedeltaIndex.to_list
    // @covers: TimedeltaIndex.to_numpy
    // @covers: TimedeltaIndex.to_pytimedelta
    // @covers: TimedeltaIndex.to_series
    // @covers: TimedeltaIndex.tolist
    // @covers: TimedeltaIndex.total_seconds
    // @covers: TimedeltaIndex.transpose
    // @covers: TimedeltaIndex.union
    // @covers: TimedeltaIndex.unique
    // @covers: TimedeltaIndex.unit
    // @covers: TimedeltaIndex.value_counts
    // @covers: TimedeltaIndex.values
    // @covers: TimedeltaIndex.view
    // @covers: TimedeltaIndex.where
    use fp_index::{DuplicateKeep, TimedeltaIndex};

    // Regular 1D spaced timedeltas + another with sub-second components and NaT
    let td0: i64 = 86_400_000_000_000; // 1 day
    let td1: i64 = 2 * 86_400_000_000_000; // 2 days
    let td2: i64 = 3 * 86_400_000_000_000; // 3 days
    let td3: i64 = 4 * 86_400_000_000_000; // 4 days
    let td_fill: i64 = 5 * 86_400_000_000_000; // 5 days
    let td_mixed: i64 = 86_400_000_000_000 + 3_661_000_000_000 + 123_456_789; // 1d 01:01:01.123456789

    let regular = TimedeltaIndex::new(vec![td0, td1, td2, td3])
        .set_name("td")
        .with_freq(Some("D".to_owned()));
    let other = TimedeltaIndex::new(vec![td1, td3, td_fill]).set_name("td");
    let mixed = TimedeltaIndex::new(vec![td_mixed, -td0, td_mixed, i64::MIN]).set_name("td");

    let oracle = run_pandas_oracle_eval(
        r#"
import json
import pandas as pd

td0 = 86_400_000_000_000
td1 = 2 * td0
td2 = 3 * td0
td3 = 4 * td0
td_fill = 5 * td0
td_mixed = td0 + 3_661_000_000_000 + 123_456_789

reg = pd.TimedeltaIndex([td0, td1, td2, td3], name="td", freq="D")
other = pd.TimedeltaIndex([td1, td3, td_fill], name="td")
mixed = pd.TimedeltaIndex([td_mixed, -td0, td_mixed, pd.NaT], name="td")

def opt_int(vals):
    return [None if pd.isna(v) else int(v) for v in vals]

def opt_float(vals):
    return [None if pd.isna(v) else float(v) for v in vals]

comp_df = mixed.components
comp_rows = []
for i in range(len(mixed)):
    if pd.isna(mixed[i]):
        comp_rows.append(None)
    else:
        r = comp_df.iloc[i]
        comp_rows.append([
            int(r["days"]),
            int(r["hours"]),
            int(r["minutes"]),
            int(r["seconds"]),
            int(r["milliseconds"]),
            int(r["microseconds"]),
            int(r["nanoseconds"]),
        ])

codes, uniques = mixed.factorize()
out = {
    "asi8": [int(v) for v in reg.asi8.tolist()],
    "days": opt_int(mixed.days),
    "seconds": opt_int(mixed.seconds),
    "microseconds": opt_int(mixed.microseconds),
    "nanoseconds": opt_int(mixed.nanoseconds),
    "total_seconds": opt_float(mixed.total_seconds()),
    "components": comp_rows,
    "floor_h": [int(v) for v in mixed.floor("h").asi8.tolist()],
    "ceil_h": [int(v) for v in mixed.ceil("h").asi8.tolist()],
    "round_h": [int(v) for v in mixed.round("h").asi8.tolist()],
    "shift_2d": [int(v) for v in reg.shift(2, freq="D").asi8.tolist()],
    "diff_1": [int(v) for v in reg.diff(1).asi8.tolist()],
    "sum": int(reg.sum().value),
    "mean": int(reg.mean().value),
    "median": int(reg.median().value),
    "std": int(reg.std().value),
    "min": int(reg.min().value),
    "max": int(reg.max().value),
    "argmin": int(reg.argmin()),
    "argmax": int(reg.argmax()),
    "inferred_freq": reg.inferred_freq,
    "unit": str(reg.unit),
    "intersection": [int(v) for v in reg.intersection(other).asi8.tolist()],
    "union": [int(v) for v in reg.union(other).asi8.tolist()],
    "difference": [int(v) for v in reg.difference(other).asi8.tolist()],
    "sym_diff": [int(v) for v in reg.symmetric_difference(other).asi8.tolist()],
    "unique": [int(v) for v in mixed.unique().asi8.tolist()],
    "nunique": int(mixed.nunique()),
    "factorize_codes": [int(v) for v in codes.tolist()],
    "factorize_uniques": [int(v) for v in uniques.asi8.tolist()],
    "dropna": [int(v) for v in mixed.dropna().asi8.tolist()],
    "fillna": [int(v) for v in mixed.fillna(pd.Timedelta(td_fill, unit="ns")).asi8.tolist()],
    "sort_values": [int(v) for v in mixed.sort_values().asi8.tolist()],
    "get_loc": int(reg.get_loc(pd.Timedelta(td2, unit="ns"))),
    "get_indexer": [int(v) for v in reg.get_indexer(other).tolist()],
    "slice_locs": [int(v) for v in reg.slice_locs(pd.Timedelta(td1, unit="ns"), pd.Timedelta(td2, unit="ns"))],
}
print(json.dumps(out))
"#,
    );

    let to_i64_vec = |key: &str| -> Vec<i64> {
        oracle[key]
            .as_array()
            .expect("array")
            .iter()
            .map(|v| v.as_i64().expect("i64"))
            .collect()
    };
    let to_opt_i64 = |key: &str| -> Vec<Option<i64>> {
        oracle[key]
            .as_array()
            .expect("array")
            .iter()
            .map(serde_json::Value::as_i64)
            .collect()
    };

    assert_eq!(regular.asi8(), to_i64_vec("asi8"));
    assert_eq!(mixed.days(), to_opt_i64("days"));
    assert_eq!(mixed.seconds(), to_opt_i64("seconds"));
    assert_eq!(mixed.microseconds(), to_opt_i64("microseconds"));
    assert_eq!(mixed.nanoseconds(), to_opt_i64("nanoseconds"));

    let exp_tot_sec: Vec<Option<f64>> = oracle["total_seconds"]
        .as_array()
        .expect("arr")
        .iter()
        .map(serde_json::Value::as_f64)
        .collect();
    for (a, e) in mixed.total_seconds().iter().zip(exp_tot_sec.iter()) {
        match (a, e) {
            (Some(av), Some(ev)) => assert!((av - ev).abs() < 1e-9),
            (None, None) => {}
            _ => assert_eq!(a, e),
        }
    }

    let comps = mixed.components();
    let exp_comps = oracle["components"].as_array().expect("comps");
    assert_eq!(comps.len(), exp_comps.len());
    for (c, exp_c) in comps.iter().zip(exp_comps.iter()) {
        match (c, exp_c.as_array()) {
            (Some(comp), Some(arr)) => {
                assert_eq!(
                    [
                        comp.days,
                        comp.hours,
                        comp.minutes,
                        comp.seconds,
                        comp.milliseconds,
                        comp.microseconds,
                        comp.nanoseconds,
                    ],
                    [
                        arr[0].as_i64().expect("d"),
                        arr[1].as_i64().expect("h"),
                        arr[2].as_i64().expect("m"),
                        arr[3].as_i64().expect("s"),
                        arr[4].as_i64().expect("ms"),
                        arr[5].as_i64().expect("us"),
                        arr[6].as_i64().expect("ns"),
                    ]
                );
            }
            (None, None) => {}
            _ => assert_eq!(c.is_some(), exp_c.is_array(), "components mismatch"),
        }
    }

    assert_eq!(
        mixed.floor("h").expect("floor").asi8(),
        to_i64_vec("floor_h")
    );
    assert_eq!(mixed.ceil("h").expect("ceil").asi8(), to_i64_vec("ceil_h"));
    assert_eq!(
        mixed.round("h").expect("round").asi8(),
        to_i64_vec("round_h")
    );
    assert_eq!(regular.shift(2, td0).asi8(), to_i64_vec("shift_2d"));
    assert_eq!(regular.diff(1).asi8(), to_i64_vec("diff_1"));
    assert_eq!(regular.sum(), oracle["sum"].as_i64());
    assert_eq!(regular.mean(), oracle["mean"].as_i64());
    assert_eq!(regular.median(), oracle["median"].as_i64());
    assert_eq!(regular.std(), oracle["std"].as_i64());
    assert_eq!(regular.min(), oracle["min"].as_i64());
    assert_eq!(regular.max(), oracle["max"].as_i64());
    assert_eq!(
        regular.argmin().expect("argmin"),
        oracle["argmin"].as_u64().expect("u64") as usize
    );
    assert_eq!(
        regular.argmax().expect("argmax"),
        oracle["argmax"].as_u64().expect("u64") as usize
    );
    assert_eq!(regular.argsort(), vec![0, 1, 2, 3]);
    assert_eq!(regular.freq(), Some("D".to_owned()));
    assert_eq!(regular.freqstr(), Some("D".to_owned()));
    assert_eq!(
        regular.inferred_freq(),
        oracle["inferred_freq"].as_str().map(str::to_owned)
    );
    assert_eq!(regular.unit(), oracle["unit"].as_str().expect("unit"));
    assert_eq!(
        regular.as_unit("ns").expect("as_unit").asi8(),
        regular.asi8()
    );
    assert_eq!(regular.resolution(), "day");
    assert_eq!(mixed.resolution(), "nanosecond");
    assert_eq!(
        regular.intersection(&other).asi8(),
        to_i64_vec("intersection")
    );
    assert_eq!(regular.union(&other).asi8(), to_i64_vec("union"));
    assert_eq!(regular.difference(&other).asi8(), to_i64_vec("difference"));
    assert_eq!(
        regular.symmetric_difference(&other).asi8(),
        to_i64_vec("sym_diff")
    );
    assert_eq!(mixed.unique().expect("unique").asi8(), to_i64_vec("unique"));
    assert_eq!(
        mixed.nunique(),
        oracle["nunique"].as_u64().expect("u64") as usize
    );
    let (codes, uniques) = mixed.factorize().expect("factorize");
    let exp_codes: Vec<isize> = oracle["factorize_codes"]
        .as_array()
        .expect("arr")
        .iter()
        .map(|v| v.as_i64().expect("i64") as isize)
        .collect();
    assert_eq!(codes, exp_codes);
    assert_eq!(uniques.asi8(), to_i64_vec("factorize_uniques"));
    assert_eq!(mixed.value_counts().len(), 2);
    assert_eq!(
        mixed.duplicated(DuplicateKeep::First),
        vec![false, false, true, false]
    );
    assert_eq!(
        mixed.drop_duplicates().expect("drop_duplicates").asi8(),
        to_i64_vec("unique")
    );
    assert_eq!(mixed.dropna().asi8(), to_i64_vec("dropna"));
    assert_eq!(mixed.fillna(td_fill).asi8(), to_i64_vec("fillna"));
    assert_eq!(mixed.isna(), vec![false, false, false, true]);
    assert_eq!(mixed.isnull(), vec![false, false, false, true]);
    assert_eq!(mixed.notna(), vec![true, true, true, false]);
    assert_eq!(mixed.notnull(), vec![true, true, true, false]);
    assert!(mixed.hasnans());
    assert_eq!(regular.isin(&[td1, td3]), vec![false, true, false, true]);
    assert_eq!(
        regular
            .r#where(&[true, false, true, false], td_fill)
            .expect("where")
            .asi8(),
        vec![td0, td_fill, td2, td_fill]
    );
    assert_eq!(
        regular
            .putmask(&[false, true, false, true], td_fill)
            .expect("putmask")
            .asi8(),
        vec![td0, td_fill, td2, td_fill]
    );
    assert_eq!(regular.take(&[2, 0]).expect("take").asi8(), vec![td2, td0]);
    assert_eq!(regular.repeat(2).len(), 8);
    assert_eq!(regular.append(&other).len(), 7);
    assert_eq!(regular.insert(1, td_fill).expect("insert").len(), 5);
    assert_eq!(regular.delete(1).expect("delete").len(), 3);
    assert_eq!(mixed.sort_values().asi8(), to_i64_vec("sort_values"));
    assert_eq!(mixed.sort().asi8(), to_i64_vec("sort_values"));
    assert_eq!(
        regular.get_loc(td2).expect("get_loc"),
        oracle["get_loc"].as_u64().expect("u64") as usize
    );
    let targets = vec![td1, td3, td_fill];
    let exp_indexer: Vec<isize> = oracle["get_indexer"]
        .as_array()
        .expect("arr")
        .iter()
        .map(|v| v.as_i64().expect("i64") as isize)
        .collect();
    assert_eq!(regular.get_indexer(&targets), exp_indexer);
    assert_eq!(regular.get_indexer_for(&targets), exp_indexer);
    assert_eq!(regular.get_indexer_non_unique(&targets).0, exp_indexer);
    assert_eq!(regular.reindex(&other).1, exp_indexer);
    assert_eq!(regular.searchsorted(td2, "left").expect("ss"), 2);
    assert_eq!(
        regular.slice_locs(td1, td2).expect("slice_locs"),
        (
            oracle["slice_locs"][0].as_u64().expect("u64") as usize,
            oracle["slice_locs"][1].as_u64().expect("u64") as usize,
        )
    );
    assert_eq!(
        regular.slice_indexer(td1, td2).expect("slice_indexer"),
        1..3
    );
    assert_eq!(regular.get_slice_bound(td1, "left").expect("bound"), 1);

    // Metadata & conversions
    assert_eq!(regular.shape(), (4,));
    assert_eq!(regular.size(), 4);
    assert!(!regular.empty());
    assert_eq!(regular.ndim(), 1);
    assert_eq!(regular.nlevels(), 1);
    assert_eq!(regular.dtype(), "timedelta64[ns]");
    assert!(regular.memory_usage(false) >= 32);
    assert_eq!(regular.nbytes(), regular.memory_usage(false));
    assert!(regular.is_unique());
    assert!(!regular.has_duplicates());
    assert!(regular.is_monotonic_increasing());
    assert!(!regular.is_monotonic_decreasing());
    assert!(!regular.holds_integer());
    assert_eq!(regular.inferred_type(), "timedelta64");
    assert!(!regular.is_boolean());
    assert!(!regular.is_categorical());
    assert!(!regular.is_floating());
    assert!(!regular.is_integer());
    assert!(!regular.is_interval());
    assert!(!regular.is_numeric());
    assert!(!regular.is_object());
    assert!(regular.equals(&regular.copy()));
    assert!(regular.identical(&regular.copy()));
    assert!(regular.is_(&regular));
    assert_eq!(regular.name().map(|n| n.as_str()), Some("td"));
    assert_eq!(regular.names(), vec![Some("td".to_owned())]);
    assert_eq!(
        regular.set_names(Some("td2")).names(),
        vec![Some("td2".to_owned())]
    );
    assert_eq!(regular.rename("td3").names(), vec![Some("td3".to_owned())]);
    assert_eq!(regular.view().asi8(), regular.asi8());
    assert_eq!(regular.transpose().asi8(), regular.asi8());
    assert_eq!(regular.T().asi8(), regular.asi8());
    assert_eq!(regular.ravel(), regular.values());
    assert_eq!(regular.to_list(), regular.values());
    assert_eq!(regular.tolist(), regular.values());
    assert_eq!(regular.to_numpy(), regular.values());
    assert_eq!(regular.array(), regular.values());
    assert_eq!(regular.to_pytimedelta().len(), 4);
    assert_eq!(regular.format().len(), 4);
    assert_eq!(regular.to_flat_index().len(), 4);
    assert_eq!(regular.to_frame().len(), 4);
    assert_eq!(regular.to_series().len(), 4);
    assert_eq!(regular.r#str().len().len(), 4);
    assert!(regular.any());
    assert!(regular.all());
    assert_eq!(regular.get_level_values(0).expect("lvl").len(), 4);
    assert!(regular.droplevel(0).is_err());
    assert_eq!(regular.groupby_positions().len(), 4);
    assert_eq!(
        regular
            .groupby(regular.to_flat_index().labels())
            .expect("groupby")
            .len(),
        4
    );
    assert_eq!(regular.map(|l| l.clone()).len(), 4);
    assert_eq!(regular.astype("string").expect("astype").len(), 4);
    assert_eq!(
        regular.asof(&IndexLabel::Timedelta64(td2 + 10)),
        Some(IndexLabel::Timedelta64(td2))
    );
    assert_eq!(regular.asof_locs(other.as_index(), None).len(), 3);
    assert_eq!(regular.drop(&[IndexLabel::Timedelta64(td1)]).len(), 3);
    assert_eq!(
        regular.join(other.as_index(), "inner").expect("join").len(),
        2
    );
    assert_eq!(regular.sortlevel().0.len(), 4);
    assert_eq!(regular.infer_objects().asi8(), regular.asi8());
    assert_eq!(
        TimedeltaIndex::new(vec![td0]).item().expect("item"),
        Some(td0)
    );

    // NEGATIVE checks
    assert!(regular.item().is_err());
    assert!(regular.get_loc(td_fill).is_err());
    assert!(regular.as_unit("s").is_err());
}

#[test]
fn live_oracle_period_index_fields_asfreq_timestamps_and_ops_differential() {
    // @covers: PeriodIndex.T
    // @covers: PeriodIndex.all
    // @covers: PeriodIndex.any
    // @covers: PeriodIndex.append
    // @covers: PeriodIndex.argmax
    // @covers: PeriodIndex.argmin
    // @covers: PeriodIndex.argsort
    // @covers: PeriodIndex.array
    // @covers: PeriodIndex.asfreq
    // @covers: PeriodIndex.asi8
    // @covers: PeriodIndex.asof
    // @covers: PeriodIndex.asof_locs
    // @covers: PeriodIndex.astype
    // @covers: PeriodIndex.copy
    // @covers: PeriodIndex.day
    // @covers: PeriodIndex.day_of_week
    // @covers: PeriodIndex.day_of_year
    // @covers: PeriodIndex.dayofweek
    // @covers: PeriodIndex.dayofyear
    // @covers: PeriodIndex.days_in_month
    // @covers: PeriodIndex.daysinmonth
    // @covers: PeriodIndex.delete
    // @covers: PeriodIndex.diff
    // @covers: PeriodIndex.difference
    // @covers: PeriodIndex.drop
    // @covers: PeriodIndex.drop_duplicates
    // @covers: PeriodIndex.droplevel
    // @covers: PeriodIndex.dropna
    // @covers: PeriodIndex.dtype
    // @covers: PeriodIndex.duplicated
    // @covers: PeriodIndex.empty
    // @covers: PeriodIndex.end_time
    // @covers: PeriodIndex.equals
    // @covers: PeriodIndex.factorize
    // @covers: PeriodIndex.fillna
    // @covers: PeriodIndex.format
    // @covers: PeriodIndex.freq
    // @covers: PeriodIndex.freqstr
    // @covers: PeriodIndex.from_fields
    // @covers: PeriodIndex.from_ordinals
    // @covers: PeriodIndex.get_indexer
    // @covers: PeriodIndex.get_indexer_for
    // @covers: PeriodIndex.get_indexer_non_unique
    // @covers: PeriodIndex.get_level_values
    // @covers: PeriodIndex.get_loc
    // @covers: PeriodIndex.get_slice_bound
    // @covers: PeriodIndex.groupby
    // @covers: PeriodIndex.has_duplicates
    // @covers: PeriodIndex.hasnans
    // @covers: PeriodIndex.holds_integer
    // @covers: PeriodIndex.hour
    // @covers: PeriodIndex.identical
    // @covers: PeriodIndex.infer_objects
    // @covers: PeriodIndex.inferred_type
    // @covers: PeriodIndex.insert
    // @covers: PeriodIndex.intersection
    // @covers: PeriodIndex.is_
    // @covers: PeriodIndex.is_boolean
    // @covers: PeriodIndex.is_categorical
    // @covers: PeriodIndex.is_floating
    // @covers: PeriodIndex.is_full
    // @covers: PeriodIndex.is_integer
    // @covers: PeriodIndex.is_interval
    // @covers: PeriodIndex.is_leap_year
    // @covers: PeriodIndex.is_monotonic_decreasing
    // @covers: PeriodIndex.is_monotonic_increasing
    // @covers: PeriodIndex.is_numeric
    // @covers: PeriodIndex.is_object
    // @covers: PeriodIndex.is_unique
    // @covers: PeriodIndex.isin
    // @covers: PeriodIndex.isna
    // @covers: PeriodIndex.isnull
    // @covers: PeriodIndex.item
    // @covers: PeriodIndex.join
    // @covers: PeriodIndex.map
    // @covers: PeriodIndex.max
    // @covers: PeriodIndex.mean
    // @covers: PeriodIndex.memory_usage
    // @covers: PeriodIndex.min
    // @covers: PeriodIndex.minute
    // @covers: PeriodIndex.month
    // @covers: PeriodIndex.name
    // @covers: PeriodIndex.names
    // @covers: PeriodIndex.nbytes
    // @covers: PeriodIndex.ndim
    // @covers: PeriodIndex.nlevels
    // @covers: PeriodIndex.notna
    // @covers: PeriodIndex.notnull
    // @covers: PeriodIndex.nunique
    // @covers: PeriodIndex.putmask
    // @covers: PeriodIndex.quarter
    // @covers: PeriodIndex.qyear
    // @covers: PeriodIndex.ravel
    // @covers: PeriodIndex.reindex
    // @covers: PeriodIndex.rename
    // @covers: PeriodIndex.repeat
    // @covers: PeriodIndex.resolution
    // @covers: PeriodIndex.round
    // @covers: PeriodIndex.searchsorted
    // @covers: PeriodIndex.second
    // @covers: PeriodIndex.set_names
    // @covers: PeriodIndex.shape
    // @covers: PeriodIndex.shift
    // @covers: PeriodIndex.size
    // @covers: PeriodIndex.slice_indexer
    // @covers: PeriodIndex.slice_locs
    // @covers: PeriodIndex.sort
    // @covers: PeriodIndex.sort_values
    // @covers: PeriodIndex.sortlevel
    // @covers: PeriodIndex.start_time
    // @covers: PeriodIndex.str
    // @covers: PeriodIndex.strftime
    // @covers: PeriodIndex.symmetric_difference
    // @covers: PeriodIndex.take
    // @covers: PeriodIndex.to_flat_index
    // @covers: PeriodIndex.to_frame
    // @covers: PeriodIndex.to_list
    // @covers: PeriodIndex.to_numpy
    // @covers: PeriodIndex.to_series
    // @covers: PeriodIndex.to_timestamp
    // @covers: PeriodIndex.tolist
    // @covers: PeriodIndex.transpose
    // @covers: PeriodIndex.union
    // @covers: PeriodIndex.unique
    // @covers: PeriodIndex.value_counts
    // @covers: PeriodIndex.values
    // @covers: PeriodIndex.view
    // @covers: PeriodIndex.week
    // @covers: PeriodIndex.weekday
    // @covers: PeriodIndex.weekofyear
    // @covers: PeriodIndex.where
    // @covers: PeriodIndex.year
    use fp_index::{DuplicateKeep, PeriodFields, PeriodIndex};
    use fp_types::{Period, PeriodFreq};

    let p0 = Period::parse("2024-01").expect("p0");
    let p1 = Period::parse("2024-02").expect("p1");
    let p2 = Period::parse("2024-03").expect("p2");
    let p3 = Period::parse("2024-04").expect("p3");
    let p_fill = Period::parse("2024-05").expect("p_fill");

    let pi = PeriodIndex::from_range(p0, 4).set_name("pi");
    let other = PeriodIndex::new(vec![p1, p3, p_fill]).set_name("pi");
    let dups = PeriodIndex::new(vec![p2, p0, p2]).set_name("pi");

    let oracle = run_pandas_oracle_eval(
        r#"
import json
import pandas as pd

pi = pd.period_range("2024-01", periods=4, freq="M", name="pi")
other = pd.PeriodIndex(["2024-02", "2024-04", "2024-05"], freq="M", name="pi")
dups = pd.PeriodIndex(["2024-03", "2024-01", "2024-03"], freq="M", name="pi")
from_f = pd.PeriodIndex.from_fields(year=[2024, 2024, 2024], month=[1, 2, 3], freq="M")
from_o = pd.PeriodIndex.from_ordinals(pi.asi8, freq="M")

codes, uniques = dups.factorize()
out = {
    "asi8": [int(v) for v in pi.asi8.tolist()],
    "from_fields": [int(v) for v in from_f.asi8.tolist()],
    "from_ordinals": [int(v) for v in from_o.asi8.tolist()],
    "dtype": str(pi.dtype),
    "freqstr": str(pi.freqstr),
    "is_full": bool(pi.is_full),
    "other_is_full": bool(other.is_full),
    "year": [int(v) for v in pi.year.tolist()],
    "month": [int(v) for v in pi.month.tolist()],
    "day": [int(v) for v in pi.day.tolist()],
    "hour": [int(v) for v in pi.hour.tolist()],
    "minute": [int(v) for v in pi.minute.tolist()],
    "second": [int(v) for v in pi.second.tolist()],
    "quarter": [int(v) for v in pi.quarter.tolist()],
    "qyear": [int(v) for v in pi.qyear.tolist()],
    "weekday": [int(v) for v in pi.weekday.tolist()],
    "dayofweek": [int(v) for v in pi.dayofweek.tolist()],
    "day_of_week": [int(v) for v in pi.day_of_week.tolist()],
    "dayofyear": [int(v) for v in pi.dayofyear.tolist()],
    "day_of_year": [int(v) for v in pi.day_of_year.tolist()],
    "days_in_month": [int(v) for v in pi.days_in_month.tolist()],
    "daysinmonth": [int(v) for v in pi.daysinmonth.tolist()],
    "week": [int(v) for v in pi.week.tolist()],
    "weekofyear": [int(v) for v in pi.weekofyear.tolist()],
    "is_leap_year": [bool(v) for v in pi.is_leap_year.tolist()],
    "strftime": [str(v) for v in pi.strftime("%Y-%m").tolist()],
    "asfreq_d_end": [int(v) for v in pi.asfreq("D", how="end").asi8.tolist()],
    "asfreq_d_start": [int(v) for v in pi.asfreq("D", how="start").asi8.tolist()],
    "start_time": [int(v) for v in pi.start_time.asi8.tolist()],
    "end_time": [int(v) for v in pi.end_time.asi8.tolist()],
    "to_ts_start": [int(v) for v in pi.to_timestamp(how="start").asi8.tolist()],
    "to_ts_end": [int(v) for v in pi.to_timestamp(how="end").asi8.tolist()],
    "shift_2": [int(v) for v in pi.shift(2).asi8.tolist()],
    "intersection": [int(v) for v in pi.intersection(other).asi8.tolist()],
    "union": [int(v) for v in pi.union(other).asi8.tolist()],
    "difference": [int(v) for v in pi.difference(other).asi8.tolist()],
    "sym_diff": [int(v) for v in pi.symmetric_difference(other).asi8.tolist()],
    "unique": [int(v) for v in dups.unique().asi8.tolist()],
    "factorize_codes": [int(v) for v in codes.tolist()],
    "factorize_uniques": [int(v) for v in uniques.asi8.tolist()],
    "sort_values": [int(v) for v in dups.sort_values().asi8.tolist()],
    "min": int(pi.min().ordinal),
    "max": int(pi.max().ordinal),
    "argmin": int(pi.argmin()),
    "argmax": int(pi.argmax()),
    "get_loc": int(pi.get_loc(pd.Period("2024-03", freq="M"))),
    "get_indexer": [int(v) for v in pi.get_indexer(other).tolist()],
    "slice_locs": [int(v) for v in pi.slice_locs(pd.Period("2024-02", freq="M"), pd.Period("2024-03", freq="M"))],
}
print(json.dumps(out))
"#,
    );

    let to_i64_vec = |key: &str| -> Vec<i64> {
        oracle[key]
            .as_array()
            .expect("array")
            .iter()
            .map(|v| v.as_i64().expect("i64"))
            .collect()
    };
    let to_opt_u32 = |key: &str| -> Vec<Option<u32>> {
        oracle[key]
            .as_array()
            .expect("array")
            .iter()
            .map(|v| v.as_u64().map(|n| n as u32))
            .collect()
    };

    assert_eq!(pi.asi8(), to_i64_vec("asi8"));
    let from_f = PeriodIndex::from_fields(PeriodFields {
        year: &[2024, 2024, 2024],
        quarter: None,
        month: Some(&[1, 2, 3]),
        day: None,
        hour: None,
        minute: None,
        second: None,
        freq: Some(PeriodFreq::Monthly),
    })
    .expect("from_fields");
    assert_eq!(from_f.asi8(), to_i64_vec("from_fields"));
    let from_o = PeriodIndex::from_ordinals(&pi.asi8(), PeriodFreq::Monthly);
    assert_eq!(from_o.asi8(), to_i64_vec("from_ordinals"));

    assert_eq!(pi.dtype(), oracle["dtype"].as_str().expect("dtype"));
    assert_eq!(pi.freq(), Some(PeriodFreq::Monthly));
    assert_eq!(pi.freqstr(), oracle["freqstr"].as_str().map(str::to_owned));
    assert_eq!(pi.is_full(), oracle["is_full"].as_bool().expect("bool"));
    assert_eq!(
        other.is_full(),
        oracle["other_is_full"].as_bool().expect("bool")
    );

    let exp_years: Vec<Option<i32>> = oracle["year"]
        .as_array()
        .expect("year")
        .iter()
        .map(|v| v.as_i64().map(|n| n as i32))
        .collect();
    assert_eq!(pi.year().expect("year"), exp_years);
    assert_eq!(pi.month().expect("month"), to_opt_u32("month"));
    assert_eq!(pi.day().expect("day"), to_opt_u32("day"));
    assert_eq!(pi.hour().expect("hour"), to_opt_u32("hour"));
    assert_eq!(pi.minute().expect("minute"), to_opt_u32("minute"));
    assert_eq!(pi.second().expect("second"), to_opt_u32("second"));
    assert_eq!(pi.quarter().expect("quarter"), to_opt_u32("quarter"));
    let exp_qyears: Vec<i32> = oracle["qyear"]
        .as_array()
        .expect("qyear")
        .iter()
        .map(|v| v.as_i64().expect("i64") as i32)
        .collect();
    assert_eq!(pi.qyear().expect("qyear"), exp_qyears);
    assert_eq!(pi.weekday().expect("weekday"), to_opt_u32("weekday"));
    assert_eq!(pi.dayofweek().expect("dayofweek"), to_opt_u32("dayofweek"));
    assert_eq!(
        pi.day_of_week().expect("day_of_week"),
        to_opt_u32("day_of_week")
    );
    assert_eq!(pi.dayofyear().expect("dayofyear"), to_opt_u32("dayofyear"));
    assert_eq!(
        pi.day_of_year().expect("day_of_year"),
        to_opt_u32("day_of_year")
    );
    assert_eq!(
        pi.days_in_month().expect("days_in_month"),
        to_opt_u32("days_in_month")
    );
    assert_eq!(
        pi.daysinmonth().expect("daysinmonth"),
        to_opt_u32("daysinmonth")
    );
    assert_eq!(pi.week().expect("week"), to_opt_u32("week"));
    assert_eq!(
        pi.weekofyear().expect("weekofyear"),
        to_opt_u32("weekofyear")
    );
    let exp_leap: Vec<Option<bool>> = oracle["is_leap_year"]
        .as_array()
        .expect("leap")
        .iter()
        .map(serde_json::Value::as_bool)
        .collect();
    assert_eq!(pi.is_leap_year().expect("is_leap_year"), exp_leap);

    let exp_strftime: Vec<Option<String>> = oracle["strftime"]
        .as_array()
        .expect("strftime")
        .iter()
        .map(|v| v.as_str().map(str::to_owned))
        .collect();
    assert_eq!(pi.strftime("%Y-%m").expect("strftime"), exp_strftime);
    assert_eq!(
        pi.asfreq("D").expect("asfreq D").asi8(),
        to_i64_vec("asfreq_d_end")
    );
    assert_eq!(
        pi.asfreq_with_how("D", "start")
            .expect("asfreq D start")
            .asi8(),
        to_i64_vec("asfreq_d_start")
    );
    assert_eq!(
        pi.start_time().expect("start_time").asi8(),
        to_i64_vec("start_time")
    );
    assert_eq!(
        pi.end_time().expect("end_time").asi8(),
        to_i64_vec("end_time")
    );
    assert_eq!(
        pi.to_timestamp("start").expect("to_timestamp start").asi8(),
        to_i64_vec("to_ts_start")
    );
    assert_eq!(
        pi.to_timestamp("end").expect("to_timestamp end").asi8(),
        to_i64_vec("to_ts_end")
    );
    assert_eq!(pi.shift(2).expect("shift").asi8(), to_i64_vec("shift_2"));
    assert_eq!(pi.diff(1), vec![None, Some(1), Some(1), Some(1)]);
    assert_eq!(
        pi.intersection(&other).expect("intersection").asi8(),
        to_i64_vec("intersection")
    );
    assert_eq!(pi.union(&other).expect("union").asi8(), to_i64_vec("union"));
    assert_eq!(
        pi.difference(&other).expect("difference").asi8(),
        to_i64_vec("difference")
    );
    assert_eq!(
        pi.symmetric_difference(&other).expect("sym_diff").asi8(),
        to_i64_vec("sym_diff")
    );
    assert_eq!(dups.unique().asi8(), to_i64_vec("unique"));
    assert_eq!(dups.nunique(), 2);
    let (codes, uniques) = dups.factorize();
    let exp_codes: Vec<isize> = oracle["factorize_codes"]
        .as_array()
        .expect("arr")
        .iter()
        .map(|v| v.as_i64().expect("i64") as isize)
        .collect();
    assert_eq!(codes, exp_codes);
    assert_eq!(uniques.asi8(), to_i64_vec("factorize_uniques"));
    assert_eq!(dups.value_counts().len(), 2);
    assert_eq!(
        dups.duplicated(DuplicateKeep::First),
        vec![false, false, true]
    );
    assert_eq!(dups.drop_duplicates().asi8(), to_i64_vec("unique"));
    assert_eq!(
        dups.sort_values().expect("sort_values").asi8(),
        to_i64_vec("sort_values")
    );
    assert_eq!(dups.sort().expect("sort").asi8(), to_i64_vec("sort_values"));
    assert_eq!(
        pi.min().expect("min").map(|p| p.ordinal),
        oracle["min"].as_i64()
    );
    assert_eq!(
        pi.max().expect("max").map(|p| p.ordinal),
        oracle["max"].as_i64()
    );
    assert!(pi.mean().expect("mean").is_some());
    assert_eq!(
        pi.argmin().expect("argmin"),
        oracle["argmin"].as_u64().expect("u64") as usize
    );
    assert_eq!(
        pi.argmax().expect("argmax"),
        oracle["argmax"].as_u64().expect("u64") as usize
    );
    assert_eq!(pi.argsort().expect("argsort"), vec![0, 1, 2, 3]);
    assert_eq!(
        pi.get_loc(p2).expect("get_loc"),
        oracle["get_loc"].as_u64().expect("u64") as usize
    );
    let exp_indexer: Vec<isize> = oracle["get_indexer"]
        .as_array()
        .expect("arr")
        .iter()
        .map(|v| v.as_i64().expect("i64") as isize)
        .collect();
    assert_eq!(pi.get_indexer(other.values()), exp_indexer);
    assert_eq!(pi.get_indexer_for(other.values()), exp_indexer);
    assert_eq!(pi.get_indexer_non_unique(other.values()).0, exp_indexer);
    assert_eq!(pi.reindex(&other).1, exp_indexer);
    assert_eq!(pi.searchsorted(p2, "left").expect("ss"), 2);
    assert_eq!(pi.get_slice_bound(p2, "left").expect("bound left"), 2);
    assert_eq!(pi.get_slice_bound(p2, "right").expect("bound right"), 3);
    assert_eq!(
        pi.slice_locs(p1, p2).expect("slice_locs"),
        (
            oracle["slice_locs"][0].as_u64().expect("u64") as usize,
            oracle["slice_locs"][1].as_u64().expect("u64") as usize,
        )
    );
    assert_eq!(pi.slice_indexer(p1, p2).expect("slice_indexer"), 1..3);

    // Metadata, shape, masks, mutations & conversions
    assert_eq!(pi.shape(), (4,));
    assert_eq!(pi.size(), 4);
    assert!(!pi.empty());
    assert_eq!(pi.ndim(), 1);
    assert_eq!(pi.nlevels(), 1);
    assert!(pi.memory_usage(false) >= 32);
    assert_eq!(pi.nbytes(), pi.memory_usage(false));
    assert!(!pi.hasnans());
    assert_eq!(pi.isna(), vec![false; 4]);
    assert_eq!(pi.isnull(), vec![false; 4]);
    assert_eq!(pi.notna(), vec![true; 4]);
    assert_eq!(pi.notnull(), vec![true; 4]);
    assert_eq!(pi.dropna().asi8(), pi.asi8());
    let pi_with_nat = PeriodIndex::new(vec![p0, Period::new(i64::MIN, PeriodFreq::Monthly), p2]);
    assert_eq!(
        pi_with_nat.fillna(p1).expect("fillna").values(),
        &[p0, p1, p2]
    );
    assert_eq!(pi_with_nat.dropna().values(), &[p0, p2]);
    assert!(pi.is_unique());
    assert!(!pi.has_duplicates());
    assert!(pi.is_monotonic_increasing());
    assert!(!pi.is_monotonic_decreasing());
    assert!(!pi.holds_integer());
    assert_eq!(pi.inferred_type(), "period");
    assert!(!pi.is_boolean());
    assert!(!pi.is_categorical());
    assert!(!pi.is_floating());
    assert!(!pi.is_integer());
    assert!(!pi.is_interval());
    assert!(!pi.is_numeric());
    assert!(!pi.is_object());
    assert!(pi.equals(&pi.copy()));
    assert!(pi.identical(&pi.copy()));
    assert!(pi.is_(&pi));
    assert_eq!(pi.name().map(|n| n.as_str()), Some("pi"));
    assert_eq!(pi.names(), vec![Some("pi".to_owned())]);
    assert_eq!(
        pi.set_names(Some("pi2")).names(),
        vec![Some("pi2".to_owned())]
    );
    assert_eq!(pi.rename("pi3").names(), vec![Some("pi3".to_owned())]);
    assert_eq!(pi.resolution(), Some("month"));
    assert_eq!(pi.round("M").asi8(), pi.asi8());
    assert_eq!(pi.view().asi8(), pi.asi8());
    assert_eq!(pi.transpose().asi8(), pi.asi8());
    assert_eq!(pi.T().asi8(), pi.asi8());
    assert_eq!(pi.ravel(), pi.values());
    assert_eq!(pi.to_list(), pi.values());
    assert_eq!(pi.tolist(), pi.values());
    assert_eq!(pi.to_numpy(), pi.values());
    assert_eq!(pi.array(), pi.values());
    assert_eq!(pi.infer_objects().asi8(), pi.asi8());
    assert_eq!(pi.isin(&[p1, p3]), vec![false, true, false, true]);
    assert_eq!(
        pi.r#where(&[true, false, true, false], p_fill)
            .expect("where")
            .values(),
        &[p0, p_fill, p2, p_fill]
    );
    assert_eq!(
        pi.putmask(&[false, true, false, true], p_fill)
            .expect("putmask")
            .values(),
        &[p0, p_fill, p2, p_fill]
    );
    assert_eq!(pi.take(&[2, 0]).expect("take").values(), &[p2, p0]);
    assert_eq!(pi.repeat(2).len(), 8);
    assert_eq!(pi.append(&other).len(), 7);
    assert_eq!(pi.insert(1, p_fill).expect("insert").len(), 5);
    assert_eq!(pi.delete(1).expect("delete").len(), 3);
    assert_eq!(pi.format().len(), 4);
    assert_eq!(pi.to_flat_index().len(), 4);
    assert_eq!(pi.to_frame().len(), 4);
    assert_eq!(pi.to_series().len(), 4);
    assert_eq!(pi.r#str().len(), vec![None; 4]);
    assert!(pi.any());
    assert!(pi.all());
    assert_eq!(pi.get_level_values(0).expect("lvl").len(), 4);
    assert!(pi.droplevel(0).is_err());
    assert_eq!(pi.groupby_positions().len(), 4);
    assert_eq!(
        pi.groupby(pi.to_flat_index().labels())
            .expect("groupby")
            .len(),
        4
    );
    assert_eq!(pi.map(|l| l.clone()).len(), 4);
    assert_eq!(pi.astype("string").expect("astype").len(), 4);
    assert_eq!(
        pi.asof(&IndexLabel::Period(p2)),
        Some(IndexLabel::Period(p2))
    );
    assert_eq!(pi.asof_locs(&other.to_index(), None).len(), 3);
    assert_eq!(pi.drop(&[IndexLabel::Period(p1)]).len(), 3);
    assert_eq!(pi.join(&other.to_index(), "inner").expect("join").len(), 2);
    assert_eq!(pi.sortlevel().0.len(), 4);
    assert_eq!(PeriodIndex::new(vec![p0]).item().expect("item"), p0);

    // NEGATIVE checks: missing lookup, mismatched frequency in searchsorted, invalid asfreq target
    assert!(pi.item().is_err());
    assert!(pi.get_loc(p_fill).is_err());
    let daily_p = Period::parse("2024-01-15").expect("daily_p");
    assert!(pi.searchsorted(daily_p, "left").is_err());
    assert!(pi.asfreq("INVALID_FREQ").is_err());
}
