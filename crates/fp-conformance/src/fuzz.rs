use std::collections::{BTreeMap, BTreeSet};

use fp_frame::DataFrame;
use fp_index::{IndexLabel, align_union, validate_alignment_plan};
use fp_io::{
    IoError as FpIoError, read_feather_bytes, read_ipc_stream_bytes, read_parquet_bytes,
    write_feather_bytes, write_ipc_stream_bytes, write_parquet_bytes,
};

use super::{fuzz_feather_frame_from_bytes, fuzz_index_from_bytes};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FuzzArrowFormat {
    Parquet,
    Feather,
    IpcStream,
}

impl FuzzArrowFormat {
    fn from_byte(byte: u8) -> Self {
        match byte % 3 {
            0 => Self::Parquet,
            1 => Self::Feather,
            _ => Self::IpcStream,
        }
    }

    fn distinct_from_byte(primary: Self, byte: u8) -> Self {
        match primary {
            Self::Parquet => {
                if byte.is_multiple_of(2) {
                    Self::Feather
                } else {
                    Self::IpcStream
                }
            }
            Self::Feather => {
                if byte.is_multiple_of(2) {
                    Self::Parquet
                } else {
                    Self::IpcStream
                }
            }
            Self::IpcStream => {
                if byte.is_multiple_of(2) {
                    Self::Parquet
                } else {
                    Self::Feather
                }
            }
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Parquet => "parquet",
            Self::Feather => "feather",
            Self::IpcStream => "ipc_stream",
        }
    }
}

fn fuzz_roundtrip_frame_via_format(
    format: FuzzArrowFormat,
    frame: &DataFrame,
) -> Result<DataFrame, FpIoError> {
    match format {
        FuzzArrowFormat::Parquet => {
            let encoded = write_parquet_bytes(frame)?;
            read_parquet_bytes(&encoded)
        }
        FuzzArrowFormat::Feather => {
            let encoded = write_feather_bytes(frame)?;
            read_feather_bytes(&encoded)
        }
        FuzzArrowFormat::IpcStream => {
            let encoded = write_ipc_stream_bytes(frame)?;
            read_ipc_stream_bytes(&encoded)
        }
    }
}

/// Structure-aware fuzz entrypoint for cross-format Arrow-backed IO parity.
///
/// The input always runs in synth mode: bytes are projected into a small typed
/// `DataFrame`, round-tripped through a primary Arrow-backed format, then
/// round-tripped again through a distinct secondary format. The logical frame
/// recovered from both stages must converge to the same content.
pub fn fuzz_format_cross_round_trip_bytes(input: &[u8]) -> Result<(), FpIoError> {
    let primary = FuzzArrowFormat::from_byte(input.first().copied().unwrap_or_default());
    let secondary =
        FuzzArrowFormat::distinct_from_byte(primary, input.get(1).copied().unwrap_or(1));
    let payload = input.get(2..).unwrap_or(&[]);
    let source = fuzz_feather_frame_from_bytes(payload)?;
    let primary_frame = fuzz_roundtrip_frame_via_format(primary, &source)?;
    let secondary_frame = fuzz_roundtrip_frame_via_format(secondary, &primary_frame)?;

    if !primary_frame.equals(&secondary_frame) {
        return Err(FpIoError::Io(std::io::Error::other(format!(
            "cross-format round-trip drifted between {} and {}",
            primary.name(),
            secondary.name()
        ))));
    }

    Ok(())
}

/// Structure-aware fuzz entrypoint for `fp-index` outer alignment semantics.
///
/// Input is split at the first `|` byte into left/right index payloads. Each
/// payload byte is projected onto a small `IndexLabel` domain so fuzzing
/// naturally exercises duplicates, overlap, and mixed Int64/Utf8 labels.
pub fn fuzz_index_align_bytes(input: &[u8]) -> Result<(), String> {
    let Some(split_at) = input.iter().position(|byte| *byte == b'|') else {
        return Ok(());
    };

    let left = fuzz_index_from_bytes(&input[..split_at]);
    let right = fuzz_index_from_bytes(&input[split_at + 1..]);
    let plan = align_union(&left, &right);
    validate_alignment_plan(&plan).map_err(|err| err.to_string())?;

    let mut output_counts = BTreeMap::<IndexLabel, usize>::new();
    let mut left_position_counts = BTreeMap::<IndexLabel, BTreeMap<usize, usize>>::new();
    let mut right_position_counts = BTreeMap::<IndexLabel, BTreeMap<usize, usize>>::new();

    for row in 0..plan.union_index.len() {
        let label = &plan.union_index.labels()[row];
        *output_counts.entry(label.clone()).or_default() += 1;

        match plan.left_positions[row] {
            Some(left_pos) => {
                let actual = left.labels().get(left_pos).ok_or_else(|| {
                    format!("left position out of bounds: row={row} pos={left_pos}")
                })?;
                if actual != label {
                    return Err(format!(
                        "left alignment label mismatch: row={row} pos={left_pos} \
                         output={label:?} actual={actual:?}"
                    ));
                }
                *left_position_counts
                    .entry(label.clone())
                    .or_default()
                    .entry(left_pos)
                    .or_default() += 1;
            }
            None if left.labels().iter().any(|candidate| candidate == label) => {
                return Err(format!(
                    "left position missing despite label presence: row={row} label={label:?}"
                ));
            }
            None => {}
        }

        match plan.right_positions[row] {
            Some(right_pos) => {
                let actual = right.labels().get(right_pos).ok_or_else(|| {
                    format!("right position out of bounds: row={row} pos={right_pos}")
                })?;
                if actual != label {
                    return Err(format!(
                        "right alignment label mismatch: row={row} pos={right_pos} \
                         output={label:?} actual={actual:?}"
                    ));
                }
                *right_position_counts
                    .entry(label.clone())
                    .or_default()
                    .entry(right_pos)
                    .or_default() += 1;
            }
            None if right.labels().iter().any(|candidate| candidate == label) => {
                return Err(format!(
                    "right position missing despite label presence: row={row} label={label:?}"
                ));
            }
            None => {}
        }
    }

    let mut left_counts = BTreeMap::<IndexLabel, usize>::new();
    let mut right_counts = BTreeMap::<IndexLabel, usize>::new();
    let mut left_expected_position_counts = BTreeMap::<IndexLabel, BTreeMap<usize, usize>>::new();
    let mut right_expected_position_counts = BTreeMap::<IndexLabel, BTreeMap<usize, usize>>::new();

    for (position, label) in left.labels().iter().enumerate() {
        *left_counts.entry(label.clone()).or_default() += 1;
        left_expected_position_counts
            .entry(label.clone())
            .or_default()
            .insert(position, 0);
    }

    for (position, label) in right.labels().iter().enumerate() {
        *right_counts.entry(label.clone()).or_default() += 1;
        right_expected_position_counts
            .entry(label.clone())
            .or_default()
            .insert(position, 0);
    }

    let labels = left_counts
        .keys()
        .chain(right_counts.keys())
        .cloned()
        .collect::<BTreeSet<_>>();

    for label in labels {
        let left_count = *left_counts.get(&label).unwrap_or(&0);
        let right_count = *right_counts.get(&label).unwrap_or(&0);
        let expected_output_count = match (left_count, right_count) {
            (0, count) => count,
            (count, 0) => count,
            (left_count, right_count) => left_count * right_count,
        };
        let actual_output_count = *output_counts.get(&label).unwrap_or(&0);
        if actual_output_count != expected_output_count {
            return Err(format!(
                "alignment multiplicity mismatch for {label:?}: expected={expected_output_count} \
                 actual={actual_output_count} left_count={left_count} right_count={right_count}"
            ));
        }

        if left_count > 0 && right_count > 0 {
            if let Some(expected_counts) = left_expected_position_counts.get_mut(&label) {
                for count in expected_counts.values_mut() {
                    *count = right_count;
                }
            }
            if let Some(expected_counts) = right_expected_position_counts.get_mut(&label) {
                for count in expected_counts.values_mut() {
                    *count = left_count;
                }
            }
        } else if right_count == 0 {
            if let Some(expected_counts) = left_expected_position_counts.get_mut(&label) {
                for count in expected_counts.values_mut() {
                    *count = 1;
                }
            }
            right_expected_position_counts.remove(&label);
        } else {
            left_expected_position_counts.remove(&label);
            if let Some(expected_counts) = right_expected_position_counts.get_mut(&label) {
                for count in expected_counts.values_mut() {
                    *count = 1;
                }
            }
        }

        let actual_left = left_position_counts
            .get(&label)
            .cloned()
            .unwrap_or_default();
        let expected_left = left_expected_position_counts
            .get(&label)
            .cloned()
            .unwrap_or_default();
        if actual_left != expected_left {
            return Err(format!(
                "left position multiplicity mismatch for {label:?}: \
                 expected={expected_left:?} actual={actual_left:?}"
            ));
        }

        let actual_right = right_position_counts
            .get(&label)
            .cloned()
            .unwrap_or_default();
        let expected_right = right_expected_position_counts
            .get(&label)
            .cloned()
            .unwrap_or_default();
        if actual_right != expected_right {
            return Err(format!(
                "right position multiplicity mismatch for {label:?}: \
                 expected={expected_right:?} actual={actual_right:?}"
            ));
        }
    }

    Ok(())
}

// ============================================================================
// Differential Fuzzing Against Live Pandas Oracle (br-frankenpandas-rc0923-epic-full-surface-conformance-j0lg5.6)
// ============================================================================

use std::{
    fs,
    io::{BufRead, BufReader, BufWriter, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::{Mutex, OnceLock},
};

use fp_frame::Series;
use fp_groupby::{GroupByOptions, groupby_sum};
use fp_runtime::{EvidenceLedger, RuntimePolicy};
use fp_types::{NullKind, Scalar, nanmax, nanmean, nanmin, nansum};
use serde::{Deserialize, Serialize};

use super::{
    FixtureDataFrame, FixtureExpectedDataFrame, FixtureExpectedSeries, FixtureMultiIndex,
    FixtureSeries, HarnessConfig, OrderedColumnData, build_dataframe, build_series,
    compare_dataframe_expected, compare_scalar, compare_series_expected, pinned_pandas_version,
};

/// Swarm-testing configuration decoded from the first byte of a differential fuzz seed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DifferentialSwarmConfig {
    pub allow_nulls: bool,
    pub allow_duplicate_labels: bool,
    pub allow_multiindex: bool,
    pub allow_float64: bool,
    pub allow_bool: bool,
    pub allow_utf8: bool,
    pub op_family_mask: u8,
}

impl DifferentialSwarmConfig {
    #[must_use]
    pub fn from_byte(byte: u8) -> Self {
        let dtype_bits = (byte >> 3) & 0b0111;
        Self {
            allow_nulls: (byte & 0b0001) != 0,
            allow_duplicate_labels: (byte & 0b0010) != 0,
            allow_multiindex: (byte & 0b0100) != 0,
            allow_float64: (dtype_bits & 0b001) != 0,
            allow_bool: (dtype_bits & 0b010) != 0,
            allow_utf8: (dtype_bits & 0b100) != 0,
            op_family_mask: ((byte >> 6) & 0b0011).max(1),
        }
    }

    #[must_use]
    pub fn subset_tag(&self) -> String {
        format!(
            "nulls={}_dups={}_mi={}_f64={}_bool={}_utf8={}_fam={}",
            u8::from(self.allow_nulls),
            u8::from(self.allow_duplicate_labels),
            u8::from(self.allow_multiindex),
            u8::from(self.allow_float64),
            u8::from(self.allow_bool),
            u8::from(self.allow_utf8),
            self.op_family_mask
        )
    }
}

/// Single step in a differential DataFrame operation sequence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum DifferentialChainStep {
    SortIndex { ascending: bool },
    SortValues { column: String, ascending: bool },
    DropNa,
    FillNaZero,
    ResetIndexDrop,
    SelectColumns { columns: Vec<String> },
    Head { n: usize },
    Tail { n: usize },
}

/// Generated differential fuzz operation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DifferentialFuzzCase {
    NanSum {
        swarm: DifferentialSwarmConfig,
        series: FixtureSeries,
    },
    NanMean {
        swarm: DifferentialSwarmConfig,
        series: FixtureSeries,
    },
    NanMin {
        swarm: DifferentialSwarmConfig,
        series: FixtureSeries,
    },
    NanMax {
        swarm: DifferentialSwarmConfig,
        series: FixtureSeries,
    },
    SeriesAdd {
        swarm: DifferentialSwarmConfig,
        left: FixtureSeries,
        right: FixtureSeries,
    },
    GroupBySum {
        swarm: DifferentialSwarmConfig,
        keys: FixtureSeries,
        values: FixtureSeries,
    },
    DataFrameOpChain {
        swarm: DifferentialSwarmConfig,
        frame: FixtureDataFrame,
        steps: Vec<DifferentialChainStep>,
    },
}

impl DifferentialFuzzCase {
    #[must_use]
    pub fn op_name(&self) -> &'static str {
        match self {
            Self::NanSum { .. } => "nan_sum",
            Self::NanMean { .. } => "nan_mean",
            Self::NanMin { .. } => "nan_min",
            Self::NanMax { .. } => "nan_max",
            Self::SeriesAdd { .. } => "series_add",
            Self::GroupBySum { .. } => "groupby_sum",
            Self::DataFrameOpChain { .. } => "dataframe_op_chain",
        }
    }

    #[must_use]
    pub fn swarm(&self) -> &DifferentialSwarmConfig {
        match self {
            Self::NanSum { swarm, .. }
            | Self::NanMean { swarm, .. }
            | Self::NanMin { swarm, .. }
            | Self::NanMax { swarm, .. }
            | Self::SeriesAdd { swarm, .. }
            | Self::GroupBySum { swarm, .. }
            | Self::DataFrameOpChain { swarm, .. } => swarm,
        }
    }
}

/// Comparator mode for differential fuzzing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DifferentialComparatorMode {
    /// Full conformance comparator checking exact index order, column order, MultiIndex, and values.
    Strict,
    /// Deliberately broken comparator that ignores index order (for NEGATIVE self-test verification).
    IgnoreIndexOrder,
}

/// Optional fault injection on the FrankenPandas side to verify the harness catches divergences.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DifferentialFaultInjection {
    None,
    /// Reverses the output index labels without reordering the values.
    ReverseIndexOrder,
}

/// Serializable reproduction packet emitted when a divergence is found (never auto-committed to fixtures/).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DifferentialReproPacket {
    pub seed_hex: String,
    pub minimized_seed_hex: String,
    pub case: DifferentialFuzzCase,
    pub mismatch: String,
}

impl DifferentialReproPacket {
    pub fn write_to_dir(&self, dir: &Path) -> std::io::Result<PathBuf> {
        fs::create_dir_all(dir)?;
        let file_name = format!(
            "repro_{}_{}.json",
            self.case.op_name(),
            &self.minimized_seed_hex[..self.minimized_seed_hex.len().min(16)]
        );
        let path = dir.join(file_name);
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        fs::write(&path, json)?;
        Ok(path)
    }
}

/// Summary report from a bounded differential fuzzing campaign.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DifferentialFuzzCampaignReport {
    pub total_seeds: usize,
    pub executed_vs_oracle: usize,
    pub skipped_no_oracle: usize,
    pub divergences_found: usize,
    pub op_counts: BTreeMap<String, usize>,
    pub swarm_subset_counts: BTreeMap<String, usize>,
    pub repros: Vec<DifferentialReproPacket>,
}

struct LongLivedOracleWorker {
    _child: Child,
    stdin: BufWriter<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    pandas_version: String,
}

#[derive(Debug, Deserialize)]
struct BatchWorkerResponse {
    #[serde(default)]
    pandas_version: Option<String>,
    #[serde(default)]
    expected_series: Option<FixtureExpectedSeries>,
    #[serde(default)]
    expected_frame: Option<FixtureExpectedDataFrame>,
    #[serde(default)]
    expected_scalar: Option<Scalar>,
    #[serde(default)]
    error: Option<String>,
}

const BATCH_WORKER_PYTHON_DRIVER: &str = r#"
import argparse
import json
import os
import sys

oracle_dir = sys.argv[1]
legacy_root = sys.argv[2]
sys.path.insert(0, oracle_dir)

import pandas_oracle as po

ns = argparse.Namespace(
    legacy_root=legacy_root,
    strict_legacy=True,
    allow_system_pandas_fallback=True,
)
try:
    pd = po.setup_pandas(ns)
    po._PD = pd
except Exception as exc:
    sys.stdout.write(json.dumps({"error": str(exc), "error_origin": "oracle_unavailable"}) + "\n")
    sys.stdout.flush()
    raise SystemExit(0)

sys.stdout.write(json.dumps({"pandas_version": str(pd.__version__)}) + "\n")
sys.stdout.flush()

def run_op_chain(pd_mod, frame_payload, steps):
    df = po.dataframe_from_json(pd_mod, frame_payload)
    for step in steps:
        kind = step.get("step")
        if kind == "sort_index":
            df = df.sort_index(ascending=bool(step.get("ascending", True)), kind="mergesort")
        elif kind == "sort_values":
            col = str(step.get("column"))
            if col in df.columns:
                df = df.sort_values(
                    by=col,
                    ascending=bool(step.get("ascending", True)),
                    na_position="last",
                    kind="mergesort",
                )
        elif kind == "drop_na":
            df = df.dropna()
        elif kind == "fill_na_zero":
            df = df.fillna(0)
        elif kind == "reset_index_drop":
            df = df.reset_index(drop=True)
        elif kind == "select_columns":
            cols = [str(c) for c in step.get("columns", []) if str(c) in df.columns]
            if cols:
                df = df.loc[:, cols]
        elif kind == "head":
            df = df.head(int(step.get("n", 1)))
        elif kind == "tail":
            df = df.tail(int(step.get("n", 1)))
    return {"expected_frame": po.dataframe_to_json(df)}

for raw_line in sys.stdin:
    line = raw_line.strip()
    if not line:
        continue
    try:
        req = json.loads(line)
        mode = req.get("mode")
        if mode == "dispatch":
            resp = po.dispatch(pd, req["payload"])
        elif mode == "op_chain":
            resp = run_op_chain(pd, req["frame"], req["steps"])
        else:
            raise po.OracleError(f"unknown batch mode: {mode!r}")
        out = {
            "pandas_version": str(pd.__version__),
            "expected_series": resp.get("expected_series"),
            "expected_frame": resp.get("expected_frame"),
            "expected_scalar": resp.get("expected_scalar"),
            "error": None,
            "error_origin": None,
        }
    except po.OracleError as exc:
        out = {
            "pandas_version": str(pd.__version__),
            "error": str(exc),
            "error_origin": po.oracle_error_origin(exc),
        }
    except Exception as exc:
        out = {
            "pandas_version": str(pd.__version__),
            "error": str(exc),
            "error_origin": "pandas",
        }
    sys.stdout.write(json.dumps(out) + "\n")
    sys.stdout.flush()
"#;

static ORACLE_WORKER: OnceLock<Mutex<Option<LongLivedOracleWorker>>> = OnceLock::new();

fn query_oracle_worker(
    config: &HarnessConfig,
    request_json: &serde_json::Value,
) -> Result<BatchWorkerResponse, String> {
    let mutex = ORACLE_WORKER.get_or_init(|| Mutex::new(None));
    let mut guard = mutex
        .lock()
        .map_err(|_| "oracle worker mutex poisoned".to_owned())?;

    if guard.is_none() {
        let script_path = config.oracle_script_path();
        let oracle_dir = script_path
            .parent()
            .ok_or_else(|| "invalid oracle script path".to_owned())?;
        let mut child = match Command::new(&config.python_bin)
            .arg("-u")
            .arg("-c")
            .arg(BATCH_WORKER_PYTHON_DRIVER)
            .arg(oracle_dir)
            .arg(&config.oracle_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(err) => {
                return Err(format!("oracle_unavailable: failed to spawn python: {err}"));
            }
        };

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "oracle_unavailable: missing child stdin".to_owned())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "oracle_unavailable: missing child stdout".to_owned())?;
        let mut reader = BufReader::new(stdout);
        let mut banner_line = String::new();
        if reader
            .read_line(&mut banner_line)
            .map_err(|e| format!("oracle_unavailable: banner read failed: {e}"))?
            == 0
        {
            return Err("oracle_unavailable: empty banner from python worker".to_owned());
        }
        let banner: BatchWorkerResponse = serde_json::from_str(&banner_line)
            .map_err(|e| format!("oracle_unavailable: invalid banner json: {e}"))?;
        if let Some(err) = banner.error {
            return Err(format!("oracle_unavailable: {err}"));
        }
        let version = banner.pandas_version.unwrap_or_default();
        if let Some(pin) = pinned_pandas_version(config)
            && version != pin
        {
            return Err(format!(
                "oracle_unavailable: pandas version {version} != pinned {pin}"
            ));
        }
        *guard = Some(LongLivedOracleWorker {
            _child: child,
            stdin: BufWriter::new(stdin),
            stdout: reader,
            pandas_version: version,
        });
    }

    let worker = guard.as_mut().expect("worker initialized above");
    let _ = &worker.pandas_version;
    let line = serde_json::to_string(request_json).map_err(|e| e.to_string())?;
    if writeln!(worker.stdin, "{line}").is_err() || worker.stdin.flush().is_err() {
        *guard = None;
        return Err("oracle_unavailable: failed to write to oracle worker stdin".to_owned());
    }

    let mut resp_line = String::new();
    match worker.stdout.read_line(&mut resp_line) {
        Ok(0) | Err(_) => {
            *guard = None;
            return Err("oracle_unavailable: oracle worker closed stdout".to_owned());
        }
        Ok(_) => {}
    }

    serde_json::from_str::<BatchWorkerResponse>(&resp_line)
        .map_err(|e| format!("invalid oracle worker response JSON: {e}"))
}

fn byte_at(bytes: &[u8], idx: usize) -> u8 {
    if bytes.is_empty() {
        0
    } else {
        bytes[idx % bytes.len()]
    }
}

fn project_index_labels(
    bytes: &[u8],
    rows: usize,
    allow_duplicates: bool,
    use_utf8: bool,
) -> Vec<IndexLabel> {
    let mut labels = Vec::with_capacity(rows);
    for r in 0..rows {
        let raw = byte_at(bytes, r);
        let label = if allow_duplicates {
            let bucket = i64::from(raw % 4);
            if use_utf8 {
                IndexLabel::Utf8(format!("k{bucket}"))
            } else {
                IndexLabel::Int64(bucket)
            }
        } else {
            // Permutation of distinct labels so sort_index exercises non-trivial reordering
            let val = i64::from((raw as usize + r * 3) as u8);
            let mut candidate = if use_utf8 {
                IndexLabel::Utf8(format!("k{val:03}"))
            } else {
                IndexLabel::Int64(val)
            };
            let mut salt = 1_i64;
            while labels.contains(&candidate) {
                candidate = if use_utf8 {
                    IndexLabel::Utf8(format!("k{:03}", val + salt))
                } else {
                    IndexLabel::Int64(val + salt)
                };
                salt += 1;
            }
            candidate
        };
        labels.push(label);
    }
    labels
}

fn project_numeric_series(
    bytes: &[u8],
    name: &str,
    rows: usize,
    swarm: &DifferentialSwarmConfig,
) -> FixtureSeries {
    let use_utf8_idx = swarm.allow_utf8;
    let index = project_index_labels(bytes, rows, swarm.allow_duplicate_labels, use_utf8_idx);
    let use_float = swarm.allow_float64 || swarm.allow_nulls;
    let mut values = Vec::with_capacity(rows);
    for r in 0..rows {
        let b = byte_at(bytes, r + 1);
        if swarm.allow_nulls && r > 0 && b.is_multiple_of(5) {
            values.push(Scalar::Null(NullKind::NaN));
        } else if use_float {
            let v = f64::from(i16::from(b) - 64) * 0.5;
            values.push(Scalar::Float64(v));
        } else {
            let v = i64::from(b % 31) - 15;
            values.push(Scalar::Int64(v));
        }
    }
    FixtureSeries {
        name: name.to_owned(),
        index,
        values,
    }
}

fn project_dataframe_and_chain(
    bytes: &[u8],
    swarm: &DifferentialSwarmConfig,
    chain_len: usize,
) -> (FixtureDataFrame, Vec<DifferentialChainStep>) {
    let rows = usize::from(byte_at(bytes, 0) % 5) + 1;
    let col_count = usize::from(byte_at(bytes, 1) % 3) + 1;
    let col_names = ["a", "b", "c"][..col_count]
        .iter()
        .map(|s| (*s).to_owned())
        .collect::<Vec<_>>();

    let (index, row_multiindex) = if swarm.allow_multiindex {
        let mut tuples = Vec::with_capacity(rows);
        let mut flat = Vec::with_capacity(rows);
        for r in 0..rows {
            let b0 = byte_at(bytes, 2 + r * 2);
            let b1 = byte_at(bytes, 3 + r * 2);
            let l0_val = if swarm.allow_duplicate_labels {
                i64::from(b0 % 3) * 8 + 2
            } else {
                i64::from((b0 % 7) + (r as u8) * 3)
            };
            let l1_str = format!("s{}", b1 % 3);
            flat.push(IndexLabel::Utf8(format!("{l0_val}|{l1_str}")));
            tuples.push(vec![IndexLabel::Int64(l0_val), IndexLabel::Utf8(l1_str)]);
        }
        (
            flat,
            Some(FixtureMultiIndex {
                tuples,
                names: vec![Some("l0".to_owned()), Some("l1".to_owned())],
            }),
        )
    } else {
        let use_utf8_idx = (byte_at(bytes, 2) & 1) != 0;
        (
            project_index_labels(
                bytes.get(3..).unwrap_or(bytes),
                rows,
                swarm.allow_duplicate_labels,
                use_utf8_idx,
            ),
            None,
        )
    };

    let mut enabled_kinds = vec![0_u8]; // 0 = int64/float64 numeric
    if swarm.allow_float64 {
        enabled_kinds.push(1);
    }
    if swarm.allow_bool && !swarm.allow_nulls {
        enabled_kinds.push(2);
    }
    if swarm.allow_utf8 && !swarm.allow_nulls {
        enabled_kinds.push(3);
    }

    let mut col_map = BTreeMap::new();
    let mut all_numeric = true;
    for (c_idx, col_name) in col_names.iter().enumerate() {
        let kind_tag =
            enabled_kinds[(usize::from(byte_at(bytes, 4 + c_idx))) % enabled_kinds.len()];
        let mut vals = Vec::with_capacity(rows);
        for r in 0..rows {
            let b = byte_at(bytes, 7 + c_idx * rows + r);
            match kind_tag {
                0 => {
                    if swarm.allow_nulls && r > 0 && b.is_multiple_of(4) {
                        vals.push(Scalar::Null(NullKind::NaN));
                    } else if swarm.allow_nulls {
                        vals.push(Scalar::Float64(f64::from(i16::from(b % 23) - 10)));
                    } else {
                        vals.push(Scalar::Int64(i64::from(b % 23) - 10));
                    }
                }
                1 => {
                    if swarm.allow_nulls && r > 0 && b.is_multiple_of(4) {
                        vals.push(Scalar::Null(NullKind::NaN));
                    } else {
                        vals.push(Scalar::Float64(f64::from(i16::from(b % 29) - 14) * 0.5));
                    }
                }
                2 => {
                    all_numeric = false;
                    vals.push(Scalar::Bool(b.is_multiple_of(2)));
                }
                _ => {
                    all_numeric = false;
                    vals.push(Scalar::Utf8(format!("v{}", b % 5)));
                }
            }
        }
        col_map.insert(col_name.clone(), vals);
    }

    let frame = FixtureDataFrame {
        index,
        columns: OrderedColumnData::from(col_map),
        column_order: Some(col_names.clone()),
        categorical_columns: None,
        row_multiindex,
    };

    let mut steps = Vec::with_capacity(chain_len);
    let mut has_multiindex = frame.row_multiindex.is_some();
    let mut current_cols = col_names;
    for s in 0..chain_len {
        let step_byte = byte_at(bytes, 16 + s);
        let raw_choice = (step_byte.wrapping_add(swarm.op_family_mask)) % 7;
        let step = match raw_choice {
            0 => DifferentialChainStep::SortIndex {
                ascending: (step_byte & 0b1000) == 0,
            },
            1 => {
                let col = current_cols[usize::from(step_byte) % current_cols.len()].clone();
                DifferentialChainStep::SortValues {
                    column: col,
                    ascending: (step_byte & 0b1000) == 0,
                }
            }
            2 if !has_multiindex => DifferentialChainStep::DropNa,
            3 if all_numeric => DifferentialChainStep::FillNaZero,
            4 => {
                has_multiindex = false;
                DifferentialChainStep::ResetIndexDrop
            }
            5 => {
                let keep_n = (usize::from(step_byte) % current_cols.len()).max(1);
                let kept = current_cols
                    .iter()
                    .take(keep_n)
                    .cloned()
                    .collect::<Vec<_>>();
                current_cols.clone_from(&kept);
                DifferentialChainStep::SelectColumns { columns: kept }
            }
            6 => DifferentialChainStep::Head {
                n: usize::from(step_byte % 4) + 1,
            },
            _ => DifferentialChainStep::Tail {
                n: usize::from(step_byte % 4) + 1,
            },
        };
        steps.push(step);
    }

    (frame, steps)
}

/// Deterministic projection from arbitrary bytes into a [`DifferentialFuzzCase`].
#[must_use]
pub fn project_differential_fuzz_case(input: &[u8]) -> DifferentialFuzzCase {
    let swarm = DifferentialSwarmConfig::from_byte(byte_at(input, 0));
    let op_selector = byte_at(input, 1);
    let payload = input.get(2..).unwrap_or(&[]);
    let rows = usize::from(byte_at(payload, 0) % 5) + 1;

    match op_selector % 8 {
        0 => DifferentialFuzzCase::NanSum {
            series: project_numeric_series(payload, "s", rows, &swarm),
            swarm,
        },
        1 => DifferentialFuzzCase::NanMean {
            series: project_numeric_series(payload, "s", rows, &swarm),
            swarm,
        },
        2 => {
            if (op_selector & 0b1000) == 0 {
                DifferentialFuzzCase::NanMin {
                    series: project_numeric_series(payload, "s", rows, &swarm),
                    swarm,
                }
            } else {
                DifferentialFuzzCase::NanMax {
                    series: project_numeric_series(payload, "s", rows, &swarm),
                    swarm,
                }
            }
        }
        3 => {
            let mid = payload.len() / 2;
            let left = project_numeric_series(
                &payload[..mid.max(1).min(payload.len())],
                "s",
                rows,
                &swarm,
            );
            let right_rows = usize::from(byte_at(payload, 3) % 5) + 1;
            let right = project_numeric_series(
                payload.get(mid..).unwrap_or(payload),
                "s",
                right_rows,
                &swarm,
            );
            DifferentialFuzzCase::SeriesAdd { swarm, left, right }
        }
        4 => {
            let mut key_values = Vec::with_capacity(rows);
            for r in 0..rows {
                let b = byte_at(payload, r + 1);
                key_values.push(Scalar::Utf8(format!("g{}", b % 3)));
            }
            let values =
                project_numeric_series(payload.get(1..).unwrap_or(payload), "val", rows, &swarm);
            let keys = FixtureSeries {
                name: "key".to_owned(),
                index: values.index.clone(),
                values: key_values,
            };
            DifferentialFuzzCase::GroupBySum {
                swarm,
                keys,
                values,
            }
        }
        _ => {
            let chain_len = usize::from((op_selector >> 3) % 4) + 1;
            let (frame, steps) = project_dataframe_and_chain(payload, &swarm, chain_len);
            DifferentialFuzzCase::DataFrameOpChain {
                swarm,
                frame,
                steps,
            }
        }
    }
}

fn compare_series_with_mode(
    actual: &Series,
    expected: &FixtureExpectedSeries,
    mode: DifferentialComparatorMode,
) -> Result<(), String> {
    match mode {
        DifferentialComparatorMode::Strict => compare_series_expected(actual, expected),
        DifferentialComparatorMode::IgnoreIndexOrder => {
            if actual.values().len() != expected.values.len() {
                return Err(format!(
                    "value length mismatch: actual={}, expected={}",
                    actual.values().len(),
                    expected.values.len()
                ));
            }
            let mut actual_labels = actual.index().labels().to_vec();
            let mut expected_labels = expected.index.clone();
            actual_labels.sort();
            expected_labels.sort();
            if actual_labels != expected_labels {
                return Err(format!(
                    "index multiset mismatch: actual={actual_labels:?}, expected={expected_labels:?}"
                ));
            }
            for (idx, (left, right)) in actual
                .values()
                .iter()
                .zip(expected.values.iter())
                .enumerate()
            {
                if !left.semantic_eq(right) {
                    return Err(format!(
                        "value mismatch at idx={idx}: actual={left:?}, expected={right:?}"
                    ));
                }
            }
            Ok(())
        }
    }
}

fn compare_dataframe_with_mode(
    actual: &DataFrame,
    expected: &FixtureExpectedDataFrame,
    mode: DifferentialComparatorMode,
) -> Result<(), String> {
    match mode {
        DifferentialComparatorMode::Strict => compare_dataframe_expected(actual, expected),
        DifferentialComparatorMode::IgnoreIndexOrder => {
            let mut actual_labels = actual.index().labels().to_vec();
            let mut expected_labels = expected.index.clone();
            actual_labels.sort();
            expected_labels.sort();
            if actual_labels != expected_labels {
                return Err(format!(
                    "dataframe index multiset mismatch: actual={actual_labels:?}, expected={expected_labels:?}"
                ));
            }
            for (name, expected_values) in &expected.columns {
                let Some(column) = actual.column(name) else {
                    return Err(format!("dataframe column missing in actual: {name}"));
                };
                if column.values().len() != expected_values.len() {
                    return Err(format!(
                        "dataframe column '{name}' length mismatch: actual={}, expected={}",
                        column.values().len(),
                        expected_values.len()
                    ));
                }
            }
            Ok(())
        }
    }
}

fn apply_fault_injection_series(
    series: Series,
    fault: DifferentialFaultInjection,
) -> Result<Series, String> {
    match fault {
        DifferentialFaultInjection::None => Ok(series),
        DifferentialFaultInjection::ReverseIndexOrder => {
            let mut rev_labels = series.index().labels().to_vec();
            rev_labels.reverse();
            Series::new(
                series.name().to_owned(),
                fp_index::Index::new(rev_labels),
                series.column().clone(),
            )
            .map_err(|e| e.to_string())
        }
    }
}

fn apply_fault_injection_dataframe(
    df: DataFrame,
    fault: DifferentialFaultInjection,
) -> Result<DataFrame, String> {
    match fault {
        DifferentialFaultInjection::None => Ok(df),
        DifferentialFaultInjection::ReverseIndexOrder => {
            let mut rev_labels = df.index().labels().to_vec();
            rev_labels.reverse();
            let mut cols = BTreeMap::new();
            let order = df
                .column_names()
                .into_iter()
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>();
            for name in &order {
                if let Some(col) = df.column(name) {
                    cols.insert(name.clone(), col.clone());
                }
            }
            let out =
                DataFrame::new_with_column_order(fp_index::Index::new(rev_labels), cols, order)
                    .map_err(|e| e.to_string())?;
            if let Some(multi) = df.row_multiindex() {
                let rev_pos = (0..multi.len()).rev().collect::<Vec<_>>();
                let rev_multi = multi.take(&rev_pos).map_err(|e| e.to_string())?;
                out.with_row_multiindex(rev_multi)
                    .map_err(|e| e.to_string())
            } else {
                Ok(out)
            }
        }
    }
}

/// Execute a [`DifferentialFuzzCase`] in FrankenPandas and compare against the live pandas 2.2.3 oracle.
///
/// Returns `Ok(true)` when compared against the live oracle and matched,
/// `Ok(false)` when the live oracle is unavailable and `!config.require_live_oracle`,
/// or `Err(mismatch)` on divergence.
pub fn execute_differential_fuzz_case(
    config: &HarnessConfig,
    case: &DifferentialFuzzCase,
    comparator_mode: DifferentialComparatorMode,
    fault: DifferentialFaultInjection,
) -> Result<bool, String> {
    match case {
        DifferentialFuzzCase::NanSum { series, .. }
        | DifferentialFuzzCase::NanMean { series, .. }
        | DifferentialFuzzCase::NanMin { series, .. }
        | DifferentialFuzzCase::NanMax { series, .. } => {
            let fp_series = build_series(series)?;
            let actual = if matches!(case, DifferentialFuzzCase::NanSum { .. }) {
                nansum(fp_series.values())
            } else if matches!(case, DifferentialFuzzCase::NanMean { .. }) {
                nanmean(fp_series.values())
            } else if matches!(case, DifferentialFuzzCase::NanMin { .. }) {
                match nanmin(fp_series.values()) {
                    Scalar::Int64(i) => Scalar::Float64(i as f64),
                    other => other,
                }
            } else {
                match nanmax(fp_series.values()) {
                    Scalar::Int64(i) => Scalar::Float64(i as f64),
                    other => other,
                }
            };

            let req = serde_json::json!({
                "mode": "dispatch",
                "payload": {
                    "operation": case.op_name(),
                    "left": series,
                }
            });
            let resp = match query_oracle_worker(config, &req) {
                Ok(resp) => resp,
                Err(err)
                    if err.starts_with("oracle_unavailable:") && !config.require_live_oracle =>
                {
                    return Ok(false);
                }
                Err(err) => return Err(err),
            };
            if let Some(err) = resp.error {
                return Err(format!("oracle error for {}: {err}", case.op_name()));
            }
            let expected = resp
                .expected_scalar
                .ok_or_else(|| format!("oracle omitted expected_scalar for {}", case.op_name()))?;
            compare_scalar(&actual, &expected, case.op_name())?;
            Ok(true)
        }
        DifferentialFuzzCase::SeriesAdd { left, right, .. } => {
            let left_s = build_series(left)?;
            let right_s = build_series(right)?;
            let policy = RuntimePolicy::strict();
            let mut ledger = EvidenceLedger::new();
            let actual = left_s
                .add_with_policy(&right_s, &policy, &mut ledger)
                .map_err(|e| e.to_string())?;
            let actual = apply_fault_injection_series(actual, fault)?;

            let req = serde_json::json!({
                "mode": "dispatch",
                "payload": {
                    "operation": "series_add",
                    "left": left,
                    "right": right,
                }
            });
            let resp = match query_oracle_worker(config, &req) {
                Ok(resp) => resp,
                Err(err)
                    if err.starts_with("oracle_unavailable:") && !config.require_live_oracle =>
                {
                    return Ok(false);
                }
                Err(err) => return Err(err),
            };
            if let Some(err) = resp.error {
                return Err(format!("oracle error for series_add: {err}"));
            }
            let expected = resp
                .expected_series
                .ok_or_else(|| "oracle omitted expected_series for series_add".to_owned())?;
            compare_series_with_mode(&actual, &expected, comparator_mode)?;
            Ok(true)
        }
        DifferentialFuzzCase::GroupBySum { keys, values, .. } => {
            let keys_s = build_series(keys)?;
            let vals_s = build_series(values)?;
            let policy = RuntimePolicy::strict();
            let mut ledger = EvidenceLedger::new();
            let actual = groupby_sum(
                &keys_s,
                &vals_s,
                GroupByOptions::default(),
                &policy,
                &mut ledger,
            )
            .map_err(|e| e.to_string())?;
            let actual = apply_fault_injection_series(actual, fault)?;

            let req = serde_json::json!({
                "mode": "dispatch",
                "payload": {
                    "operation": "groupby_sum",
                    "left": keys,
                    "right": values,
                }
            });
            let resp = match query_oracle_worker(config, &req) {
                Ok(resp) => resp,
                Err(err)
                    if err.starts_with("oracle_unavailable:") && !config.require_live_oracle =>
                {
                    return Ok(false);
                }
                Err(err) => return Err(err),
            };
            if let Some(err) = resp.error {
                return Err(format!("oracle error for groupby_sum: {err}"));
            }
            let expected = resp
                .expected_series
                .ok_or_else(|| "oracle omitted expected_series for groupby_sum".to_owned())?;
            compare_series_with_mode(&actual, &expected, comparator_mode)?;
            Ok(true)
        }
        DifferentialFuzzCase::DataFrameOpChain { frame, steps, .. } => {
            let mut df = build_dataframe(frame)?;
            for step in steps {
                df = match step {
                    DifferentialChainStep::SortIndex { ascending } => {
                        df.sort_index(*ascending).map_err(|e| e.to_string())?
                    }
                    DifferentialChainStep::SortValues { column, ascending } => {
                        if df.column(column).is_some() {
                            df.sort_values(column, *ascending)
                                .map_err(|e| e.to_string())?
                        } else {
                            df
                        }
                    }
                    DifferentialChainStep::DropNa => df.dropna().map_err(|e| e.to_string())?,
                    DifferentialChainStep::FillNaZero => {
                        df.fillna(&Scalar::Int64(0)).map_err(|e| e.to_string())?
                    }
                    DifferentialChainStep::ResetIndexDrop => {
                        df.reset_index(true).map_err(|e| e.to_string())?
                    }
                    DifferentialChainStep::SelectColumns { columns } => {
                        let valid = columns
                            .iter()
                            .filter(|c| df.column(c).is_some())
                            .map(String::as_str)
                            .collect::<Vec<_>>();
                        if valid.is_empty() {
                            df
                        } else {
                            df.select_columns(&valid).map_err(|e| e.to_string())?
                        }
                    }
                    DifferentialChainStep::Head { n } => df
                        .head(i64::try_from(*n).unwrap_or(i64::MAX))
                        .map_err(|e| e.to_string())?,
                    DifferentialChainStep::Tail { n } => df
                        .tail(i64::try_from(*n).unwrap_or(i64::MAX))
                        .map_err(|e| e.to_string())?,
                };
            }
            let actual = apply_fault_injection_dataframe(df, fault)?;

            let req = serde_json::json!({
                "mode": "op_chain",
                "frame": frame,
                "steps": steps,
            });
            let resp = match query_oracle_worker(config, &req) {
                Ok(resp) => resp,
                Err(err)
                    if err.starts_with("oracle_unavailable:") && !config.require_live_oracle =>
                {
                    return Ok(false);
                }
                Err(err) => return Err(err),
            };
            if let Some(err) = resp.error {
                return Err(format!("oracle error for dataframe_op_chain: {err}"));
            }
            let expected = resp
                .expected_frame
                .ok_or_else(|| "oracle omitted expected_frame for dataframe_op_chain".to_owned())?;
            compare_dataframe_with_mode(&actual, &expected, comparator_mode)?;
            Ok(true)
        }
    }
}

/// Deterministic shrinker for differential fuzzing seeds.
pub fn shrink_differential_fuzz_input(
    seed: &[u8],
    mut is_failing: impl FnMut(&[u8]) -> bool,
) -> Vec<u8> {
    let mut best = seed.to_vec();
    // Pass 1: truncate tail bytes down to minimum 3 bytes
    while best.len() > 3 {
        let candidate_len = (best.len() - 1).max(3);
        let candidate = best[..candidate_len].to_vec();
        if is_failing(&candidate) {
            best = candidate;
        } else {
            break;
        }
    }
    // Pass 2: reduce chain length bits in byte 1 if possible
    if best.len() >= 2 {
        let op_byte = best[1];
        for chain_bits in 0..(op_byte >> 3) % 4 {
            let mut candidate = best.clone();
            candidate[1] = (op_byte & 0b0000_0111) | (chain_bits << 3);
            if is_failing(&candidate) {
                best = candidate;
                break;
            }
        }
    }
    // Pass 3: simplify individual payload bytes toward 0
    for idx in 2..best.len() {
        if best[idx] != 0 {
            let mut candidate = best.clone();
            candidate[idx] = 0;
            if is_failing(&candidate) {
                best = candidate;
            }
        }
    }
    best
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

/// Differential fuzz entrypoint against the live pandas 2.2.3 oracle (`DIFFERENTIAL_FUZZ_DESIGN.md`).
pub fn fuzz_differential_vs_pandas_bytes(input: &[u8]) -> Result<(), String> {
    if input.is_empty() || input.len() > 4096 {
        return Ok(());
    }
    let config = HarnessConfig::default_paths();
    let case = project_differential_fuzz_case(input);
    execute_differential_fuzz_case(
        &config,
        &case,
        DifferentialComparatorMode::Strict,
        DifferentialFaultInjection::None,
    )
    .map(|_| ())
}

/// Run a bounded differential fuzzing campaign over deterministic swarm-varied seeds.
pub fn run_differential_fuzz_campaign(
    config: &HarnessConfig,
    num_seeds: usize,
    repro_dir: Option<&Path>,
) -> Result<DifferentialFuzzCampaignReport, String> {
    let mut op_counts = BTreeMap::new();
    let mut swarm_subset_counts = BTreeMap::new();
    let mut executed_vs_oracle = 0usize;
    let mut skipped_no_oracle = 0usize;
    let mut repros = Vec::new();

    for seed_idx in 0..num_seeds {
        // Generate deterministic 24-byte seed covering swarm configurations and op sequences
        let mut seed = Vec::with_capacity(24);
        let mut state = (seed_idx as u64)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .wrapping_add(0xBF58_476D_1CE4_E5B9);
        for i in 0..24_u64 {
            state ^= state >> 12;
            state ^= state << 25;
            state ^= state >> 27;
            let mixed = state.wrapping_mul(0x2545_F491_4F6C_DD1D);
            seed.push(((mixed >> ((i % 8) * 8)) & 0xFF) as u8);
        }
        // Ensure every op tag 0..8 and swarm combination is evenly sampled
        seed[0] = (seed_idx & 0xFF) as u8;
        seed[1] = (((seed_idx / 8) % 4) as u8) << 3 | ((seed_idx % 8) as u8);

        let case = project_differential_fuzz_case(&seed);
        *op_counts.entry(case.op_name().to_owned()).or_insert(0) += 1;
        *swarm_subset_counts
            .entry(case.swarm().subset_tag())
            .or_insert(0) += 1;

        match execute_differential_fuzz_case(
            config,
            &case,
            DifferentialComparatorMode::Strict,
            DifferentialFaultInjection::None,
        ) {
            Ok(true) => executed_vs_oracle += 1,
            Ok(false) => skipped_no_oracle += 1,
            Err(mismatch) => {
                executed_vs_oracle += 1;
                let minimized = shrink_differential_fuzz_input(&seed, |candidate| {
                    let cand_case = project_differential_fuzz_case(candidate);
                    execute_differential_fuzz_case(
                        config,
                        &cand_case,
                        DifferentialComparatorMode::Strict,
                        DifferentialFaultInjection::None,
                    )
                    .is_err()
                });
                let min_case = project_differential_fuzz_case(&minimized);
                let min_mismatch = execute_differential_fuzz_case(
                    config,
                    &min_case,
                    DifferentialComparatorMode::Strict,
                    DifferentialFaultInjection::None,
                )
                .err()
                .unwrap_or(mismatch);
                let packet = DifferentialReproPacket {
                    seed_hex: hex_encode(&seed),
                    minimized_seed_hex: hex_encode(&minimized),
                    case: min_case,
                    mismatch: min_mismatch,
                };
                if let Some(dir) = repro_dir {
                    let _ = packet.write_to_dir(dir);
                }
                repros.push(packet);
            }
        }
    }

    Ok(DifferentialFuzzCampaignReport {
        total_seeds: num_seeds,
        executed_vs_oracle,
        skipped_no_oracle,
        divergences_found: repros.len(),
        op_counts,
        swarm_subset_counts,
        repros,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn differential_fuzz_self_test_catches_injected_index_order_bug() {
        let mut config = HarnessConfig::default_paths();
        config.allow_system_pandas_fallback = true;
        config.require_live_oracle = true;

        // Construct a 32-byte seed that maps to DataFrameOpChain (op_selector % 8 == 5)
        // with chain_len = 2 (`SortIndex`), 4 rows, and distinct row labels so reversing
        // index order is observable even after shrinking down to 3 bytes.
        let seed: Vec<u8> = vec![
            0b0111_1100,  // swarm: multiindex + float64 + bool + utf8, distinct labels, no nulls
            (1 << 3) | 5, // chain_len = 2, op = 5 (DataFrameOpChain)
            3,            // rows = 4
            2,            // 3 columns
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            11,
            22,
            33,
            44,
        ];
        let case = project_differential_fuzz_case(&seed);
        assert_eq!(case.op_name(), "dataframe_op_chain");

        // 1. Baseline without fault injection must match live oracle under Strict comparator
        let baseline = execute_differential_fuzz_case(
            &config,
            &case,
            DifferentialComparatorMode::Strict,
            DifferentialFaultInjection::None,
        )
        .expect("baseline execution against live oracle must succeed");
        assert!(baseline, "live oracle must be active");

        // 2. Weak comparator (IgnoreIndexOrder) misses the injected ReverseIndexOrder bug
        let weak_result = execute_differential_fuzz_case(
            &config,
            &case,
            DifferentialComparatorMode::IgnoreIndexOrder,
            DifferentialFaultInjection::ReverseIndexOrder,
        );
        assert!(
            weak_result.is_ok(),
            "weak comparator (IgnoreIndexOrder) should miss index order reversal: {weak_result:?}"
        );

        // 3. Strict comparator catches the injected ReverseIndexOrder bug
        let strict_err = execute_differential_fuzz_case(
            &config,
            &case,
            DifferentialComparatorMode::Strict,
            DifferentialFaultInjection::ReverseIndexOrder,
        )
        .expect_err("Strict comparator must catch injected ReverseIndexOrder bug");
        assert!(
            strict_err.contains("index mismatch"),
            "expected index mismatch diagnostic, got: {strict_err}"
        );

        // 4. Deterministic shrinker minimizes the failing seed while preserving the failure
        let minimized = shrink_differential_fuzz_input(&seed, |candidate| {
            let cand_case = project_differential_fuzz_case(candidate);
            execute_differential_fuzz_case(
                &config,
                &cand_case,
                DifferentialComparatorMode::Strict,
                DifferentialFaultInjection::ReverseIndexOrder,
            )
            .is_err()
        });
        assert!(
            minimized.len() < seed.len(),
            "shrinker must reduce seed length ({} -> {})",
            seed.len(),
            minimized.len()
        );

        let min_case = project_differential_fuzz_case(&minimized);
        let min_mismatch = execute_differential_fuzz_case(
            &config,
            &min_case,
            DifferentialComparatorMode::Strict,
            DifferentialFaultInjection::ReverseIndexOrder,
        )
        .expect_err("minimized seed must still fail under Strict comparator");

        // 5. Emit repro packet to scratch temp dir (never touching committed fixtures/)
        let packet = DifferentialReproPacket {
            seed_hex: hex_encode(&seed),
            minimized_seed_hex: hex_encode(&minimized),
            case: min_case,
            mismatch: min_mismatch,
        };
        let repro_dir =
            std::env::temp_dir().join(format!("fp_diff_fuzz_self_test_{}", std::process::id()));
        let path = packet
            .write_to_dir(&repro_dir)
            .expect("writing repro packet must succeed");
        let raw = fs::read_to_string(&path).expect("read repro packet");
        let round_tripped: DifferentialReproPacket =
            serde_json::from_str(&raw).expect("deserialize repro packet");
        assert_eq!(round_tripped, packet);
    }

    #[test]
    fn differential_fuzz_campaign_matches_live_pandas_oracle_j0lg5_6() {
        let mut config = HarnessConfig::default_paths();
        config.allow_system_pandas_fallback = true;
        config.require_live_oracle = true;

        let report = run_differential_fuzz_campaign(&config, 256, None)
            .expect("differential fuzz campaign must succeed");
        println!(
            "differential_fuzz_campaign_report: total={} executed={} divergences={} ops={:?} swarms={}",
            report.total_seeds,
            report.executed_vs_oracle,
            report.divergences_found,
            report.op_counts,
            report.swarm_subset_counts.len()
        );
        assert_eq!(report.executed_vs_oracle, 256);
        assert_eq!(
            report.divergences_found, 0,
            "unexpected divergences vs live pandas 2.2.3: {:?}",
            report.repros
        );
        for expected_op in [
            "nan_sum",
            "nan_mean",
            "nan_min",
            "nan_max",
            "series_add",
            "groupby_sum",
            "dataframe_op_chain",
        ] {
            assert!(
                report.op_counts.get(expected_op).copied().unwrap_or(0) > 0,
                "campaign must exercise op family {expected_op}"
            );
        }
        assert!(
            report.swarm_subset_counts.len() >= 16,
            "campaign must sample diverse swarm configurations"
        );
    }
}
