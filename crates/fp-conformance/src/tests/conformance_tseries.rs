//! pandas.tseries parity-matrix conformance suite (frankenpandas-dh4f).
//!
//! Per /testing-conformance-harnesses Pattern 1, these tests compare the
//! Rust `fp_index` tseries facades against live upstream pandas for
//! edge-case parameter combinations.

use std::{
    io::Write,
    process::{Command, Stdio},
};

use fp_index::{DateOffset, Index, IndexLabel, apply_date_offset, bdate_range, date_range};
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
struct DateOffsetCase<'a> {
    case_id: &'a str,
    timestamp: &'a str,
    pandas_offset: &'a str,
    offset: DateOffset,
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

fn pandas_tseries_range_or_skip(
    config: &HarnessConfig,
    case_id: &str,
    pandas_function: &str,
    start: Option<&str>,
    end: Option<&str>,
    periods: Option<usize>,
    pandas_freq: &str,
    name: Option<&str>,
) -> Result<Option<PandasDateRange>, String> {
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
        "start": start,
        "end": end,
        "periods": periods,
        "freq": pandas_freq,
        "name": name,
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
print(json.dumps({
    "values": [int(value) for value in index.asi8.tolist()],
    "name": index.name,
}, separators=(",", ":")))
"#;

    let mut child = Command::new(&config.python_bin)
        .arg("-c")
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

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| format!("pandas {pandas_function} oracle stdin unavailable"))?;
    stdin
        .write_all(payload.to_string().as_bytes())
        .map_err(|err| format!("write pandas {pandas_function} oracle payload failed: {err}"))?;
    drop(stdin);

    let output = child
        .wait_with_output()
        .map_err(|err| format!("wait for pandas {pandas_function} oracle failed: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "pandas {pandas_function} oracle failed for {case_id}: {}",
            String::from_utf8_lossy(&output.stderr)
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
        case.case_id,
        "date_range",
        case.start,
        case.end,
        case.periods,
        case.pandas_freq,
        case.name,
    )
}

fn pandas_bdate_range_or_skip(
    config: &HarnessConfig,
    case: BusinessDateRangeCase<'_>,
) -> Result<Option<PandasDateRange>, String> {
    pandas_tseries_range_or_skip(
        config,
        case.case_id,
        "bdate_range",
        case.start,
        case.end,
        case.periods,
        "B",
        case.name,
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
        .arg("-c")
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

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "pandas offset oracle stdin unavailable".to_owned())?;
    stdin
        .write_all(payload.to_string().as_bytes())
        .map_err(|err| format!("write pandas offset oracle payload failed: {err}"))?;
    drop(stdin);

    let output = child
        .wait_with_output()
        .map_err(|err| format!("wait for pandas offset oracle failed: {err}"))?;
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
    let config = HarnessConfig::default_paths();
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
        actual.name().map(str::to_owned),
        expected.name,
        "pandas.tseries date_range name parity drift for {}",
        case.case_id
    );
    Ok(())
}

fn assert_bdate_range_matches_pandas(case: BusinessDateRangeCase<'_>) -> Result<(), String> {
    let config = HarnessConfig::default_paths();
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
        actual.name().map(str::to_owned),
        expected.name,
        "pandas.tseries bdate_range name parity drift for {}",
        case.case_id
    );
    Ok(())
}

fn assert_offset_matches_pandas(case: DateOffsetCase<'_>) -> Result<(), String> {
    let config = HarnessConfig::default_paths();
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
