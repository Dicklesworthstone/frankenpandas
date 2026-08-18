//! Head-to-head vs-pandas timing harness — FrankenPandas side.
//!
//! Driven by `benches/vs_pandas_harness.py`, which invokes:
//!   fp-bench --category C --workload W --size S --dtype D --json
//! and parses `{"times_us": [..]}` from stdout. The Python side runs the
//! identical workload on pandas 2.2.3; the harness computes the head-to-head
//! ratio. This restores the FrankenPandas half of the no-gaps measurement loop
//! (the `fp-bench` binary had never existed, so the harness skipped every FP
//! workload). Setup/population is OUTSIDE the timed window, matching the spec.
//!
//! Coverage (v1): dataframe_ops, groupby, rolling. io/indexing/joins/expanding
//! /ewm map to more setup-heavy harnesses and are filed as follow-up; the Python
//! side simply reports those FP workloads as INCOMPLETE until added here.

#[cfg(feature = "lazy-transpose-prototype")]
use std::sync::Arc;
use std::{
    cell::Cell,
    collections::BTreeMap,
    fmt::Write as _,
    hint::black_box,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use fp_columnar::{Column, ValidityMask};
use fp_frame::{DataFrame, Series, to_datetime};
use fp_index::{DuplicateKeep, Index, IndexLabel, RangeIndex};
use fp_join::{JoinType, merge_dataframes_on_with};
use fp_types::{DType, NullKind, Scalar};
use mimalloc::MiMalloc;
use sha2::{Digest, Sha256};

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

const WARMUP: usize = 3;
const ITERS: usize = 25;
const TAKE_BATCH: usize = 256;
const TELEMETRY_STRING_BATCH_ROWS: usize = 250_000;

#[derive(Debug)]
struct PairedSamples {
    times_us: Vec<f64>,
    null_arm_a_us: Vec<f64>,
    null_arm_b_us: Vec<f64>,
    null_ratios: Vec<f64>,
    checksum: u64,
    thread_probe: ThreadProbe,
}

#[derive(Debug, Clone, Copy)]
struct ThreadProbe {
    runtime_available_parallelism: usize,
    process_threads_before_probe: usize,
    peak_process_threads: usize,
    operation_threads_used: usize,
}

/// SHA-256 of this executable, computed by the process that is actually
/// running. This is deliberately emitted before any benchmark output.
fn self_identity() -> String {
    let Ok(path) = std::env::current_exe() else {
        return "unavailable".to_string();
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return "unavailable".to_string();
    };
    let digest = Sha256::digest(&bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        write!(&mut hex, "{byte:02x}").expect("writing to String cannot fail");
    }
    format!("{hex} ({} bytes) {}", bytes.len(), path.display())
}

/// The ISA features the COMPILER targeted, which is NOT what `runtime_isa_features`
/// reports.
///
/// br-frankenpandas-oxv4u. MEASURED 2026-08-18: a `-C target-feature=+avx2` build
/// and a default build emitted BYTE-IDENTICAL `runtime_detected_isa_features`
/// (`[scalar, sse2, avx2, fma, bmi2, vaes]`) and identical
/// `engine_identity.frankenpandas`, because `is_x86_feature_detected!` asks the
/// CPU what it supports, never what this binary was compiled to use. The two rows
/// differed only in an opaque ELF sha256 — so a row from a specially-flagged
/// build was indistinguishable from a shipping row in every recorded field, and
/// `scripts/assemble_standing_locks.py` would have banked one as a standing lock,
/// asserting a defence the shipped binary does not provide.
///
/// `cfg!` is resolved at COMPILE time, so this reports the build's own target
/// features and closes that hole. It costs nothing at runtime: every branch folds
/// to a constant.
fn compiled_target_features() -> Vec<&'static str> {
    let mut features: Vec<&'static str> = Vec::new();
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if cfg!(target_feature = "sse2") {
            features.push("sse2");
        }
        if cfg!(target_feature = "sse4.1") {
            features.push("sse4.1");
        }
        if cfg!(target_feature = "avx") {
            features.push("avx");
        }
        if cfg!(target_feature = "avx2") {
            features.push("avx2");
        }
        if cfg!(target_feature = "fma") {
            features.push("fma");
        }
        if cfg!(target_feature = "avx512f") {
            features.push("avx512f");
        }
    }
    if features.is_empty() {
        features.push("baseline");
    }
    features
}

fn runtime_isa_features() -> Vec<&'static str> {
    let mut features = vec!["scalar"];
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if std::is_x86_feature_detected!("sse2") {
            features.push("sse2");
        }
        if std::is_x86_feature_detected!("avx2") {
            features.push("avx2");
        }
        if std::is_x86_feature_detected!("fma") {
            features.push("fma");
        }
        if std::is_x86_feature_detected!("bmi2") {
            features.push("bmi2");
        }
        if std::is_x86_feature_detected!("vaes") {
            features.push("vaes");
        }
        if std::is_x86_feature_detected!("avx512f") {
            features.push("avx512f");
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        if std::arch::is_aarch64_feature_detected!("neon") {
            features.push("neon");
        }
        if std::arch::is_aarch64_feature_detected!("dotprod") {
            features.push("dotprod");
        }
        if std::arch::is_aarch64_feature_detected!("i8mm") {
            features.push("i8mm");
        }
    }
    features
}

#[cfg(target_os = "linux")]
fn process_thread_count() -> usize {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status.lines().find_map(|line| {
                line.strip_prefix("Threads:")
                    .and_then(|value| value.trim().parse::<usize>().ok())
            })
        })
        .unwrap_or(1)
}

#[cfg(not(target_os = "linux"))]
fn process_thread_count() -> usize {
    1
}

/// Observe one untimed operation so every benchmark can report how much
/// parallelism it actually exercised, not merely how many CPUs were available.
///
/// FrankenPandas' current parallel paths use scoped worker threads. A monitor
/// thread samples `/proc/self/status`; subtracting the pre-existing process
/// threads and the monitor itself yields the peak operation worker count. A
/// serial operation therefore reports one, while a scoped four-worker path
/// reports four. The probe runs before warmup and never enters a timed region.
fn probe_operation_threads<F, T>(op: &mut F) -> ThreadProbe
where
    F: FnMut() -> T,
{
    use std::sync::{
        Barrier,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    let runtime_available_parallelism =
        std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    let process_threads_before_probe = process_thread_count();
    let peak_process_threads = AtomicUsize::new(process_threads_before_probe);
    let keep_sampling = AtomicBool::new(true);
    let ready = Barrier::new(2);

    std::thread::scope(|scope| {
        let monitor = scope.spawn(|| {
            ready.wait();
            while keep_sampling.load(Ordering::Acquire) {
                peak_process_threads.fetch_max(process_thread_count(), Ordering::Relaxed);
                std::thread::sleep(Duration::from_micros(20));
            }
            peak_process_threads.fetch_max(process_thread_count(), Ordering::Relaxed);
        });
        ready.wait();
        black_box(op());
        keep_sampling.store(false, Ordering::Release);
        monitor.join().expect("thread-count monitor must not panic");
    });

    let peak_process_threads = peak_process_threads.load(Ordering::Relaxed);
    let operation_threads_used = peak_process_threads
        .saturating_sub(process_threads_before_probe.saturating_add(1))
        .max(1);
    ThreadProbe {
        runtime_available_parallelism,
        process_threads_before_probe,
        peak_process_threads,
        operation_threads_used,
    }
}

fn same_worker_python(target_dir: &Path, harness_script: &Path) -> (PathBuf, PathBuf) {
    let python = PathBuf::from("python3");
    let site_packages = target_dir.join("lane-m-python-site");
    let pinned_packages = [
        ["numpy", "2.4.3"].join("=="),
        ["pandas", "2.2.3"].join("=="),
        ["pyarrow", "24.0.0"].join("=="),
    ];
    let import_is_ready = Command::new(&python)
        .arg(harness_script)
        .arg("--dependency-probe")
        .env("PYTHONNOUSERSITE", "1")
        .env("PYTHONPATH", &site_packages)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if import_is_ready {
        return (python, site_packages);
    }

    let pip_status = Command::new(&python)
        .args([
            "-m",
            "pip",
            "install",
            "--disable-pip-version-check",
            "--target",
        ])
        .arg(&site_packages)
        .args(&pinned_packages)
        .status()
        .is_ok_and(|status| status.success());
    let install_status = pip_status
        || ["uv", "/root/.local/bin/uv", "/root/.cargo/bin/uv"]
            .iter()
            .any(|uv| {
                Command::new(uv)
                    .args(["pip", "install", "--python", "python3", "--target"])
                    .arg(&site_packages)
                    .args(&pinned_packages)
                    .status()
                    .is_ok_and(|status| status.success())
            });
    assert!(
        install_status,
        "failed to install pinned same-worker benchmark dependencies"
    );
    let import_status = Command::new(&python)
        .arg(harness_script)
        .arg("--dependency-probe")
        .env("PYTHONNOUSERSITE", "1")
        .env("PYTHONPATH", &site_packages)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("verify same-worker benchmark dependencies");
    assert!(
        import_status.success(),
        "pinned same-worker benchmark dependencies are not importable"
    );
    (python, site_packages)
}

/// Run the Python half of the harness on the same host as this ELF.
///
/// RCH accepts `cargo run` as a remote compilation command but deliberately
/// refuses arbitrary remote Python commands. This bridge lets a strict-remote
/// invocation keep pandas, the Rust child process, and both A/A controls on
/// one worker without copying a target directory back to the coordinator.
fn run_remote_python_harness(args: &[String]) -> Option<i32> {
    let marker = args
        .iter()
        .position(|argument| argument == "--remote-python-harness")?;
    let executable = std::env::current_exe().expect("resolve running fp-bench executable");
    let profile_dir = executable
        .parent()
        .expect("fp-bench executable has a profile directory");
    let target_dir = profile_dir
        .parent()
        .expect("fp-bench profile has a target directory");
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../benches/vs_pandas_harness.py")
        .canonicalize()
        .expect("resolve vs_pandas_harness.py");
    let (python, site_packages) = same_worker_python(target_dir, &script);
    for harness_args in args[marker + 1..]
        .split(|argument| argument == "--next-python-harness")
        .filter(|segment| !segment.is_empty())
    {
        let status = Command::new(&python)
            .arg(&script)
            .args(harness_args)
            .env("CARGO_TARGET_DIR", target_dir)
            .env("PYTHONNOUSERSITE", "1")
            .env("PYTHONPATH", &site_packages)
            .status()
            .expect("run same-worker Python benchmark harness");
        if !status.success() {
            return Some(status.code().unwrap_or(1));
        }
    }
    Some(0)
}

/// splitmix64 — deterministic, seed-stable uniform stream. We only need a
/// fair-distribution data set for TIMING (not bit-identity with numpy's PCG64),
/// so a cheap reproducible generator suffices.
struct SplitMix64(u64);
impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Uniform f64 in [0, 1).
    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn arg<'a>(args: &'a [String], key: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == key)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

/// Rows and columns for a size label.
///
/// FAILS CLOSED ON AN UNKNOWN LABEL, and that is the whole point of this
/// function's shape. br-frankenpandas-kko5z, CrimsonPine 2026-08-17.
///
/// This used to end in `_ => (100_000, 10)`. A silent default here is not a
/// convenience, it is a ratio forger: the Python harness owns its own
/// `SIZE_CONFIGS` and generates the pandas fixture from it, so adding a size
/// there and not here made the incumbent arm run at the new size while
/// FrankenPandas quietly ran at 100_000. OBSERVED, not hypothesised — adding
/// `100` and `1k` to the harness produced these fully provenanced rows:
///
///   sqrt @100   FP 194.49us   pandas 30.57us   0.156x
///   sqrt @1k    FP 190.34us   pandas 34.77us   0.179x
///
/// FrankenPandas' arm is flat because it was measuring 100_000 rows in both,
/// which is also why both readings equal the real `sqrt @100k` of 191.99us.
/// Every one of those rows carried a passing A/A null, a real ELF SHA and a
/// live pandas incumbent, and every one of them was comparing two different
/// problem sizes. Nothing in the provenance could have caught it; only the
/// arithmetic not making sense did.
///
/// So an unknown label must stop the process, not pick a number. The caller
/// prints and exits non-zero rather than panicking, so the harness sees an
/// ordinary failed invocation instead of a stack trace.
fn size_rows_cols_checked(size: &str) -> Option<(usize, usize)> {
    match size {
        // Sub-10k lanes exist to separate a FIXED per-call cost from a
        // per-element one; see the matching note in the harness's SIZE_CONFIGS.
        "100" => Some((100, 10)),
        "1k" => Some((1_000, 10)),
        "10k" => Some((10_000, 10)),
        "100k" => Some((100_000, 10)),
        "1M" => Some((1_000_000, 10)),
        "2M" => Some((2_000_000, 10)),
        "4M" => Some((4_000_000, 10)),
        "6M" => Some((6_000_000, 10)),
        "8M" => Some((8_000_000, 10)),
        "10M" => Some((10_000_000, 10)),
        _ => None,
    }
}

fn size_rows_cols(size: &str) -> (usize, usize) {
    match size_rows_cols_checked(size) {
        Some(pair) => pair,
        None => {
            eprintln!(
                "fp-bench: unknown --size {size:?}. Known sizes: 100, 1k, 10k, 100k, 1M, 2M, 4M, \
                 6M, 8M, 10M. Refusing to guess: a default here would run FrankenPandas at one \
                 row count while the harness ran pandas at another, and the resulting ratio would \
                 look fully provenanced (br-frankenpandas-kko5z)."
            );
            std::process::exit(2);
        }
    }
}

fn arithmetic_take_positions(rows: usize) -> Vec<usize> {
    let start = rows / 8;
    let stop = rows - start;
    (start..stop).step_by(2).collect()
}

fn telemetry_string_batch_ranges(rows: usize) -> Vec<(usize, usize)> {
    (0..rows)
        .step_by(TELEMETRY_STRING_BATCH_ROWS)
        .map(|start| (start, (start + TELEMETRY_STRING_BATCH_ROWS).min(rows)))
        .collect()
}

fn build_telemetry_string_batches(rows: usize) -> Vec<Series> {
    telemetry_string_batch_ranges(rows)
        .into_iter()
        .map(|(start, stop)| {
            let len = stop - start;
            let index = Index::new_known_unique_int64_affine_range(start as i64, 1, len)
                .expect("telemetry batch index");
            let values = (start..stop).map(|row| row as f64 * 1.5).collect();
            Series::new("s", index, Column::from_f64_values_owned(values))
                .expect("telemetry batch series")
        })
        .collect()
}

/// Build one Float64 column of `rows` values per the requested dtype, advancing
/// the shared RNG so columns differ (mirrors numpy's column-by-column fill).
fn gen_f64_column(rng: &mut SplitMix64, rows: usize, dtype: &str) -> Vec<f64> {
    let nan_frac = match dtype {
        "float64_nan10" => 0.10,
        "float64_nan50" => 0.50,
        _ => 0.0,
    };
    (0..rows)
        .map(|_| {
            let value = rng.unit() * 1_000_000.0;
            if nan_frac > 0.0 && rng.unit() < nan_frac {
                f64::NAN
            } else {
                value
            }
        })
        .collect()
}

fn gen_i64_column(rng: &mut SplitMix64, rows: usize) -> Vec<i64> {
    (0..rows)
        .map(|_| (rng.next_u64() % 1_000_000) as i64)
        .collect()
}

fn gen_bool_column(rng: &mut SplitMix64, rows: usize) -> Vec<bool> {
    (0..rows).map(|_| (rng.next_u64() & 1) != 0).collect()
}

/// Ordered, stateful callback used by the large-N `Series.apply` incumbent
/// gate. The recurrence makes one callback invocation per element observable:
/// replacing it with a vectorized reduction changes every subsequent output.
fn stateful_apply_step(state: &Cell<i64>, value: &Scalar) -> Scalar {
    let Scalar::Int64(input) = value else {
        panic!("stateful apply fixture must contain Int64 values");
    };
    let next = state.get().wrapping_mul(31).wrapping_add(*input) & 0x7fff_ffff;
    state.set(next);
    Scalar::Int64(next)
}

/// Ordered callback for the large-N `Rolling.apply` incumbent gate.
///
/// The running state makes every callback invocation observable, while the
/// window sum forces the workload to preserve rolling-window semantics.
fn stateful_rolling_step(state: &Cell<i64>, values: &[f64]) -> f64 {
    let window_sum = values.iter().sum::<f64>() as i64;
    let next = state.get().wrapping_mul(31).wrapping_add(window_sum) & 0x7fff_ffff;
    state.set(next);
    next as f64
}

/// Ordered callback for the large-N `Expanding.apply` incumbent gate.
///
/// Reading both the newest value and the growing prefix length makes the
/// expanding-window contract observable; the running state makes callback
/// order and cardinality observable.
fn stateful_expanding_step(state: &Cell<i64>, values: &[f64]) -> f64 {
    let newest = values.last().copied().unwrap_or_default() as i64;
    let next = state
        .get()
        .wrapping_mul(31)
        .wrapping_add(newest)
        .wrapping_add(values.len() as i64)
        & 0x7fff_ffff;
    state.set(next);
    next as f64
}

fn gen_datetime64_column(rows: usize, column: usize) -> Vec<i64> {
    let base = 1_609_459_200_000_000_000_i64;
    (0..rows)
        .map(|row| base + row as i64 * 1_000_000_000 + column as i64)
        .collect()
}

fn gen_timedelta64_column(rows: usize, column: usize) -> Vec<i64> {
    (0..rows)
        .map(|row| row as i64 * 1_000_000 + column as i64)
        .collect()
}

fn build_frame(rows: usize, cols: usize, dtype: &str) -> (DataFrame, Vec<Vec<f64>>) {
    let mut rng = SplitMix64(0x5151_5151_5151_5151);
    let index = Index::new_known_unique_int64_unit_range(0, rows);
    let mut columns = BTreeMap::new();
    let mut column_order = Vec::with_capacity(cols);
    let mut raw: Vec<Vec<f64>> = Vec::with_capacity(cols);
    for c in 0..cols {
        let name = format!("col_{c}");
        match dtype {
            "int64" => {
                let data = gen_i64_column(&mut rng, rows);
                raw.push(data.iter().map(|&value| value as f64).collect());
                columns.insert(name.clone(), Column::from_i64_values_owned(data));
            }
            "bool" => {
                let data = gen_bool_column(&mut rng, rows);
                raw.push(
                    data.iter()
                        .map(|&value| if value { 1.0 } else { 0.0 })
                        .collect(),
                );
                columns.insert(name.clone(), Column::from_bool_values(data));
            }
            "datetime64" | "datetime64[ns]" => {
                let data = gen_datetime64_column(rows, c);
                raw.push(data.iter().map(|&value| value as f64).collect());
                columns.insert(name.clone(), Column::from_datetime64_values(data));
            }
            "timedelta64" | "timedelta64[ns]" => {
                let data = gen_timedelta64_column(rows, c);
                raw.push(data.iter().map(|&value| value as f64).collect());
                columns.insert(
                    name.clone(),
                    Column::from_timedelta64_values_with_validity(
                        data,
                        ValidityMask::all_valid(rows),
                    ),
                );
            }
            // Canonical nullable Float64: every 7th cell NaN (from_f64_values
            // marks NaN missing -> LazyNullableFloat64 backing).
            "float64_nullable" => {
                let mut data = gen_f64_column(&mut rng, rows, "float64");
                for (i, value) in data.iter_mut().enumerate() {
                    if i % 7 == 0 {
                        *value = f64::NAN;
                    }
                }
                raw.push(data.clone());
                columns.insert(name.clone(), Column::from_f64_values(data));
            }
            // Historical Cod-a GroupBy gauntlet shape: deterministic missing
            // value every 37th row. Kept as an explicit dtype so resurrection
            // runs can reproduce that nullable workload under the v4
            // A/A + median-CI contract.
            "float64_nan37" => {
                let mut data = gen_f64_column(&mut rng, rows, "float64");
                for (i, value) in data.iter_mut().enumerate() {
                    if i % 37 == 0 {
                        *value = f64::NAN;
                    }
                }
                raw.push(data.clone());
                columns.insert(name.clone(), Column::from_f64_values(data));
            }
            // Nullable Int64: every 7th cell is Null. Exercises the
            // single-valued-missing nullable transpose shape.
            "int64_nullable" => {
                let data = gen_i64_column(&mut rng, rows);
                raw.push(
                    data.iter()
                        .enumerate()
                        .map(|(i, &value)| if i % 7 == 0 { f64::NAN } else { value as f64 })
                        .collect(),
                );
                let scalars: Vec<Scalar> = data
                    .iter()
                    .enumerate()
                    .map(|(i, &value)| {
                        if i % 7 == 0 {
                            Scalar::Null(NullKind::Null)
                        } else {
                            Scalar::Int64(value)
                        }
                    })
                    .collect();
                columns.insert(
                    name.clone(),
                    Column::new(DType::Int64, scalars).expect("nullable i64 column"),
                );
            }
            // All-valid contiguous-Utf8 columns (the representation string
            // ops and typed readers produce; plain from_values Utf8 is Eager
            // and takes different paths).
            "utf8" => {
                let mut bytes = Vec::new();
                let mut offsets = Vec::with_capacity(rows + 1);
                offsets.push(0);
                for _ in 0..rows {
                    let value = format!("value_{:016x}", rng.next_u64());
                    bytes.extend_from_slice(value.as_bytes());
                    offsets.push(bytes.len());
                }
                raw.push(vec![0.0; rows]);
                columns.insert(name.clone(), Column::from_utf8_contiguous(bytes, offsets));
            }
            // Alternating all-valid Int64/Float64 columns: the mixed-numeric
            // transpose shape that promotes to Float64 output.
            "mixed_i64_f64" => {
                if c % 2 == 0 {
                    let data = gen_i64_column(&mut rng, rows);
                    raw.push(data.iter().map(|&value| value as f64).collect());
                    columns.insert(name.clone(), Column::from_i64_values_owned(data));
                } else {
                    let data = gen_f64_column(&mut rng, rows, "float64");
                    raw.push(data.clone());
                    columns.insert(name.clone(), Column::from_f64_values(data));
                }
            }
            _ => {
                let data = gen_f64_column(&mut rng, rows, dtype);
                raw.push(data.clone());
                columns.insert(name.clone(), Column::from_f64_values(data));
            }
        }
        column_order.push(name);
    }
    let df = DataFrame::new_with_column_order(index, columns, column_order)
        .expect("fp-bench frame construction");
    (df, raw)
}

#[cfg(feature = "lazy-transpose-prototype")]
struct PrototypeF64Block {
    values: Arc<[f64]>,
    rows: usize,
    cols: usize,
}

#[cfg(feature = "lazy-transpose-prototype")]
impl PrototypeF64Block {
    fn from_column_vectors(raw: &[Vec<f64>]) -> Self {
        let cols = raw.len();
        let rows = raw.first().map_or(0, Vec::len);
        let mut values = Vec::with_capacity(rows * cols);
        for column in raw {
            debug_assert_eq!(column.len(), rows);
            values.extend_from_slice(column);
        }
        Self {
            values: Arc::from(values),
            rows,
            cols,
        }
    }

    fn transpose_view(&self) -> PrototypeF64TransposeView {
        PrototypeF64TransposeView {
            values: Arc::clone(&self.values),
            source_rows: self.rows,
            rows: self.cols,
            cols: self.rows,
        }
    }
}

#[cfg(feature = "lazy-transpose-prototype")]
struct PrototypeF64TransposeView {
    values: Arc<[f64]>,
    source_rows: usize,
    rows: usize,
    cols: usize,
}

#[cfg(feature = "lazy-transpose-prototype")]
impl PrototypeF64TransposeView {
    fn shape(&self) -> (usize, usize) {
        (self.rows, self.cols)
    }

    fn get(&self, row: usize, col: usize) -> f64 {
        let Some(offset) = row
            .checked_mul(self.source_rows)
            .and_then(|base| base.checked_add(col))
        else {
            return f64::NAN;
        };
        self.values.get(offset).copied().unwrap_or(f64::NAN)
    }
}

/// Build the two merge inputs for the joins category — mirrors the criterion
/// `build_join_frames` and the pandas `_build_join_frames` (left key 0..n,
/// right key 0,2,..,2(n-1); a unique-key Int64 join whose inner result keeps
/// ~n/2 matched rows).
fn build_join_frames(n: usize) -> (DataFrame, DataFrame) {
    let left_index = Index::new_known_unique_int64_unit_range(0, n);
    let mut left_cols = BTreeMap::new();
    left_cols.insert(
        "key".to_string(),
        Column::from_i64_values((0..n as i64).collect()),
    );
    left_cols.insert(
        "left_val".to_string(),
        Column::from_f64_values((0..n).map(|i| i as f64).collect()),
    );
    let left = DataFrame::new_with_column_order(
        left_index,
        left_cols,
        vec!["key".to_string(), "left_val".to_string()],
    )
    .expect("fp-bench left join frame");

    let right_index = Index::new_known_unique_int64_unit_range(0, n);
    let mut right_cols = BTreeMap::new();
    right_cols.insert(
        "key".to_string(),
        Column::from_i64_values((0..n as i64).map(|i| i * 2).collect()),
    );
    right_cols.insert(
        "right_val".to_string(),
        Column::from_f64_values((0..n).map(|i| i as f64 * 10.0).collect()),
    );
    let right = DataFrame::new_with_column_order(
        right_index,
        right_cols,
        vec!["key".to_string(), "right_val".to_string()],
    )
    .expect("fp-bench right join frame");
    (left, right)
}

/// Build a string-column frame for the strings category — mirrors the pandas
/// `_build_str_frame`: `key` is a ~1000-distinct group label (g0000..g0999),
/// `name` is a unique ~15-byte id (for the sort key), `val` is a Float64.
fn build_str_frame(n: usize) -> DataFrame {
    let mut key_bytes = Vec::new();
    let mut key_off = Vec::with_capacity(n + 1);
    key_off.push(0usize);
    let mut name_bytes = Vec::new();
    let mut name_off = Vec::with_capacity(n + 1);
    name_off.push(0usize);
    for i in 0..n {
        let key = format!("g{:04}", i % 1000);
        key_bytes.extend_from_slice(key.as_bytes());
        key_off.push(key_bytes.len());
        let name = format!("item_{i:010}");
        name_bytes.extend_from_slice(name.as_bytes());
        name_off.push(name_bytes.len());
    }
    let vals: Vec<f64> = (0..n).map(|i| i as f64).collect();
    let index = Index::new_known_unique_int64_unit_range(0, n);
    let mut cols = BTreeMap::new();
    cols.insert(
        "key".to_string(),
        Column::from_utf8_contiguous(key_bytes, key_off),
    );
    cols.insert(
        "name".to_string(),
        Column::from_utf8_contiguous(name_bytes, name_off),
    );
    cols.insert("val".to_string(), Column::from_f64_values(vals));
    DataFrame::new_with_column_order(
        index,
        cols,
        vec!["key".to_string(), "name".to_string(), "val".to_string()],
    )
    .expect("fp-bench string frame")
}

/// Build a square `dim x dim` all-finite Float64 frame for the df.dot GEMM
/// workload (col_0..col_{dim-1}), mirroring the pandas `_build_square_frame`.
fn build_square_f64_frame(dim: usize) -> DataFrame {
    let mut rng = SplitMix64(0x1234_5678_9abc_def0);
    let index = Index::new_known_unique_int64_unit_range(0, dim);
    let mut columns = BTreeMap::new();
    let mut column_order = Vec::with_capacity(dim);
    for c in 0..dim {
        let name = format!("col_{c}");
        let data: Vec<f64> = (0..dim).map(|_| rng.unit()).collect();
        columns.insert(name.clone(), Column::from_f64_values(data));
        column_order.push(name);
    }
    DataFrame::new_with_column_order(index, columns, column_order).expect("fp-bench square frame")
}

fn timed_batch_us<F, T>(
    op: &mut F,
    repeat: usize,
    divide_by_repeat: bool,
    checksum: &mut u64,
) -> f64
where
    F: FnMut() -> T,
{
    debug_assert!(repeat > 0);
    let started = Instant::now();
    let mut last_result = None;
    for _ in 0..repeat {
        last_result = Some(black_box(op()));
    }
    let mut elapsed_us = started.elapsed().as_secs_f64() * 1e6;
    if divide_by_repeat {
        elapsed_us /= repeat as f64;
    }

    let result = last_result.expect("repeat is non-zero");
    *checksum =
        checksum.rotate_left(9) ^ (std::mem::size_of_val(&result) as u64) ^ 0x9e37_79b9_7f4a_7c15;
    black_box(result);
    elapsed_us
}

/// Measure an identical arm twice inside every round. Order alternates so
/// first-mover cache and scheduler bias cancel. The median of `null_ratios` is
/// the A/A point estimate; the caller gates on its bootstrap median CI.
fn paired_time_us<F, T>(mut op: F, repeat: usize, divide_by_repeat: bool) -> PairedSamples
where
    F: FnMut() -> T,
{
    let thread_probe = probe_operation_threads(&mut op);
    for _ in 0..WARMUP {
        for _ in 0..repeat {
            black_box(op());
        }
    }

    let mut times_us = Vec::with_capacity(ITERS * 2);
    let mut null_arm_a_us = Vec::with_capacity(ITERS);
    let mut null_arm_b_us = Vec::with_capacity(ITERS);
    let mut null_ratios = Vec::with_capacity(ITERS);
    let mut checksum = 0_u64;
    for round in 0..ITERS {
        let (arm_a_us, arm_b_us) = if round % 2 == 0 {
            let arm_a_us = timed_batch_us(&mut op, repeat, divide_by_repeat, &mut checksum);
            let arm_b_us = timed_batch_us(&mut op, repeat, divide_by_repeat, &mut checksum);
            (arm_a_us, arm_b_us)
        } else {
            let arm_b_us = timed_batch_us(&mut op, repeat, divide_by_repeat, &mut checksum);
            let arm_a_us = timed_batch_us(&mut op, repeat, divide_by_repeat, &mut checksum);
            (arm_a_us, arm_b_us)
        };
        times_us.extend([arm_a_us, arm_b_us]);
        null_arm_a_us.push(arm_a_us);
        null_arm_b_us.push(arm_b_us);
        null_ratios.push(arm_a_us / arm_b_us);
    }
    PairedSamples {
        times_us,
        null_arm_a_us,
        null_arm_b_us,
        null_ratios,
        checksum,
        thread_probe,
    }
}

/// Variant for cache-populating APIs: build a fresh subject before each arm,
/// outside the timed region, so arm B cannot inherit arm A's materialization.
fn paired_time_us_with_setup<Setup, Subject, Op, Output>(
    mut setup: Setup,
    mut op: Op,
) -> PairedSamples
where
    Setup: FnMut() -> Subject,
    Op: FnMut(&Subject) -> Output,
{
    let mut checksum = 0_u64;
    let probe_subject = black_box(setup());
    let thread_probe = probe_operation_threads(&mut || op(black_box(&probe_subject)));
    black_box(probe_subject);
    let (times_us, null_arm_a_us, null_arm_b_us, null_ratios) = {
        let mut time_arm = || {
            let subject = black_box(setup());
            let started = Instant::now();
            let result = black_box(op(black_box(&subject)));
            let elapsed_us = started.elapsed().as_secs_f64() * 1e6;
            checksum = checksum.rotate_left(9)
                ^ (std::mem::size_of_val(&result) as u64)
                ^ 0x9e37_79b9_7f4a_7c15;
            black_box(result);
            elapsed_us
        };

        for _ in 0..WARMUP {
            black_box(time_arm());
        }

        let mut times_us = Vec::with_capacity(ITERS * 2);
        let mut null_arm_a_us = Vec::with_capacity(ITERS);
        let mut null_arm_b_us = Vec::with_capacity(ITERS);
        let mut null_ratios = Vec::with_capacity(ITERS);
        for round in 0..ITERS {
            let (arm_a_us, arm_b_us) = if round % 2 == 0 {
                (time_arm(), time_arm())
            } else {
                let arm_b_us = time_arm();
                let arm_a_us = time_arm();
                (arm_a_us, arm_b_us)
            };
            times_us.extend([arm_a_us, arm_b_us]);
            null_arm_a_us.push(arm_a_us);
            null_arm_b_us.push(arm_b_us);
            null_ratios.push(arm_a_us / arm_b_us);
        }
        (times_us, null_arm_a_us, null_arm_b_us, null_ratios)
    };

    PairedSamples {
        times_us,
        null_arm_a_us,
        null_arm_b_us,
        null_ratios,
        checksum,
        thread_probe,
    }
}

/// Time a closure after warmup and emit a same-invocation A/A control.
fn time_us<F, T>(op: F) -> PairedSamples
where
    F: FnMut() -> T,
{
    paired_time_us(op, 1, false)
}

#[cfg(feature = "lazy-transpose-prototype")]
fn time_us_repeated<F, T>(repeat: usize, op: F) -> PairedSamples
where
    F: FnMut() -> T,
{
    paired_time_us(op, repeat, true)
}

#[cfg(feature = "lazy-transpose-view")]
fn time_us_repeated_total<F, T>(repeat: usize, op: F) -> PairedSamples
where
    F: FnMut() -> T,
{
    paired_time_us(op, repeat, false)
}

/// Whole-job ETL pipeline — deliberately NOT a kernel benchmark.
///
/// One timed closure runs a complete job of the shape a pandas user actually
/// writes, end to end. The Python driver runs this exact job on live pandas
/// in the same invocation:
///
/// ```python
/// sales  = pd.read_csv(sales_path)
/// stores = pd.read_csv(stores_path)
/// kept   = sales[sales["amount"] > 0.0]
/// agg    = kept.groupby("store_id", as_index=False).sum()
/// joined = agg.merge(stores, on="store_id", how="inner")
/// ranked = joined.sort_values(["amount", "store_id"], ascending=[False, True])
/// ranked.to_csv(out_path, index=False)
/// ```
///
/// Both engines read the SAME two input CSVs, materialized once by the driver
/// outside every timed window, and each writes its own output CSV that the
/// driver then diffs byte-for-byte. A whole-job ratio only means something if
/// both arms did the same job; that diff is what proves it, because the
/// per-engine `checksum` is a liveness token (`size_of_val`) and cannot
/// compare content across engines.
///
/// The sort carries an explicit `store_id` tiebreak so the row order is total
/// and the byte diff cannot fail on tied `amount` sums alone — pandas'
/// default `quicksort` is not stable, so ties would otherwise be free to
/// disagree without either engine being wrong.
fn run_pipeline(workload: &str, data_dir: Option<&Path>) -> Option<PairedSamples> {
    if !matches!(workload, "etl_job" | "etl_job_parquet") {
        return None;
    }
    // `etl_job` at 1M is 82.3% read_csv on the pandas side (measured; see
    // artifacts/bench/cc_thinkstation1_pipeline_whole_job_20260730.md), so a
    // whole-job ratio from it is mostly a CSV-parse ratio. That is realistic --
    // real ETL is parse-dominated -- but it means the shape cannot answer
    // whether a whole-job win survives when parsing is NOT the bulk of the job.
    // `etl_job_parquet` runs the identical six stages off Parquet, where load
    // is cheap, so the compute stages carry real weight. Same job, same
    // outputs, different input format: the pair brackets the answer.
    let parquet = workload == "etl_job_parquet";
    let dir = data_dir.expect(
        "pipeline/etl_job requires --data-dir; the Python driver materializes \
         sales.csv and stores.csv there before either arm is timed",
    );
    let (sales_path, stores_path, out_path) = if parquet {
        (
            dir.join("sales.parquet"),
            dir.join("stores.parquet"),
            dir.join("out_frankenpandas_parquet.csv"),
        )
    } else {
        (
            dir.join("sales.csv"),
            dir.join("stores.csv"),
            dir.join("out_frankenpandas.csv"),
        )
    };
    assert!(
        sales_path.is_file(),
        "pipeline: missing input {}",
        sales_path.display()
    );
    assert!(
        stores_path.is_file(),
        "pipeline: missing input {}",
        stores_path.display()
    );

    Some(time_us(|| {
        // 1. load
        let (sales, stores) = if parquet {
            (
                fp_io::read_parquet(&sales_path).expect("pipeline: read sales.parquet"),
                fp_io::read_parquet(&stores_path).expect("pipeline: read stores.parquet"),
            )
        } else {
            (
                fp_io::read_csv(&sales_path).expect("pipeline: read sales.csv"),
                fp_io::read_csv(&stores_path).expect("pipeline: read stores.csv"),
            )
        };

        // 2. filter -- sales[sales["amount"] > 0.0]
        let keep = sales
            .get_column("amount")
            .gt_scalar(&Scalar::Float64(0.0))
            .expect("pipeline: amount > 0");
        let mask = keep
            .column()
            .as_bool_slice()
            .expect("pipeline: filter mask is an all-valid Bool column");
        let kept = sales.loc_bool(mask).expect("pipeline: filter");

        // 3. groupby -- kept.groupby("store_id", as_index=False).sum()
        let agg = kept
            .groupby_with_as_index(&["store_id"], false)
            .expect("pipeline: groupby store_id")
            .sum()
            .expect("pipeline: sum");

        // 4. join -- agg.merge(stores, on="store_id", how="inner")
        let merged =
            merge_dataframes_on_with(&agg, &stores, &["store_id"], &["store_id"], JoinType::Inner)
                .expect("pipeline: merge stores");
        let joined =
            DataFrame::new_with_column_order(merged.index, merged.columns, merged.column_order)
                .expect("pipeline: materialize merge");

        // 5. sort -- descending revenue, store_id tiebreak
        let ranked = joined
            .sort_values_multi(&["amount", "store_id"], &[false, true], "last")
            .expect("pipeline: rank");

        // 6. write
        fp_io::write_csv(&ranked, &out_path).expect("pipeline: write output");
        ranked
    }))
}

/// The math-unary family the ledger recorded as blocked on the build target.
///
/// `docs/NEGATIVE_EVIDENCE.md` (2026-06-26) records floor 0.089x, ceil 0.11x,
/// trunc 0.13x, round(decimals) 0.090x, sqrt ~0.085x, log 0.20x vs pandas, with
/// the explicit finding that they are "NOT source-fixable": they need `vroundpd`
/// / wide `vsqrtpd`, and FrankenPandas builds for generic x86-64, so
/// `f64::floor/ceil/trunc/round_ties_even` lower to libm libcalls and sqrt to
/// scalar `sqrtsd`, while numpy runtime-dispatches AVX regardless of compile
/// target. That row ends: "This is the ceiling for the math-unary family until
/// that build-target call is revisited."
///
/// The fleet's ISA floor moved to x86-64-v3 on 2026-07-25 (workers.toml: ovh-b,
/// the only non-AVX2 worker, dropped the `rust` tag), which satisfies that
/// retry condition. This lane exists so the re-test is a whole-binary timed A/B
/// rather than an instruction count -- fewer instructions is the mechanism, not
/// a proxy for the result.
///
/// Input is deliberately NON-INTEGRAL: an integral-valued Float64 column hits
/// the `floor`/`ceil`/`trunc` semantic-identity bit witness (landed 2026-06-26)
/// and short-circuits the kernel entirely, so an integral fixture would measure
/// the guard instead of the arithmetic. It is also strictly positive so `sqrt`
/// and `log` stay finite and neither engine drifts onto a NaN path.
fn run_math_unary(workload: &str, rows: usize) -> Option<PairedSamples> {
    let mut rng = SplitMix64(0x1234_5678_9ABC_DEF0);
    let data: Vec<f64> = (0..rows).map(|_| 1.0 + rng.unit() * 99_999.0).collect();
    let index = Index::new_known_unique_int64_unit_range(0, rows);

    // INT64-INPUT LANES. br-frankenpandas-4kig1.
    //
    // `2ce78044e` taught the domain-fused arm to accept Int64 input, and the whole
    // math_unary fixture is Float64, so that change landed with no way to measure
    // it — the same gap that left log10/log2/log1p unmeasurable until they got
    // lanes. Two lanes, not seventeen: `sqrt` and `log` are the two ops on that arm
    // whose predicate is non-trivial (`x >= 0.0`), so they exercise the widening
    // AND the domain test, which a total op like `cbrt` would not.
    //
    // The values are the SAME stream truncated to integers, so the two dtypes are
    // comparable rather than measuring different numbers. pandas widens int64 to
    // float64 for these ops exactly as FrankenPandas does, so both engines are
    // doing the same work.
    if let Some(op) = workload.strip_suffix("_int64") {
        let ints: Vec<i64> = data.iter().map(|&x| x as i64).collect();
        let int_series =
            Series::new("s", index, Column::from_i64_values_owned(ints)).expect("int math series");
        return match op {
            "sqrt" => Some(time_us(|| int_series.sqrt().expect("sqrt_int64"))),
            "log" => Some(time_us(|| int_series.log().expect("log_int64"))),
            _ => None,
        };
    }

    let series = Series::new("s", index, Column::from_f64_values(data)).expect("math series");

    // BINARY LANES. br-frankenpandas-4kig1.
    //
    // `342cd07f6` took pow/atan2/hypot off fn-pointer dispatch, and they had no
    // workload, so the change landed unmeasurable — the same gap that left
    // log10/log2/log1p unmeasurable until they got lanes, after which they
    // certified on first use.
    //
    // The right-hand series is the SAME generator advanced by one draw, so both
    // operands come from one distribution and the lane measures the op rather than
    // a difference between two fixtures. Both are strictly positive, which keeps
    // `pow` finite (a negative base with a fractional exponent is NaN) so the lane
    // measures arithmetic and not the missing-value path — the same reasoning the
    // unary fixture uses.
    if matches!(
        workload,
        "pow" | "atan2" | "hypot" | "mod" | "floordiv" | "add" | "div"
    ) {
        let mut rhs_rng = SplitMix64(0x0FED_CBA9_8765_4321);
        let rhs: Vec<f64> = (0..rows).map(|_| 1.0 + rhs_rng.unit() * 9.0).collect();
        let rhs_index = Index::new_known_unique_int64_unit_range(0, rows);
        let rhs_series =
            Series::new("rhs", rhs_index, Column::from_f64_values(rhs)).expect("rhs series");
        return match workload {
            "pow" => Some(time_us(|| series.pow(&rhs_series).expect("pow"))),
            "atan2" => Some(time_us(|| series.atan2(&rhs_series).expect("atan2"))),
            "hypot" => Some(time_us(|| series.hypot(&rhs_series).expect("hypot"))),
            // br-frankenpandas-4kig1. `e7d87c811` moved mod/floordiv onto the
            // compute-bound threshold alongside `pow`, and neither had a workload,
            // so two thirds of that change landed unmeasurable. The right-hand
            // series is strictly positive and bounded away from zero by the same
            // generator, so neither op takes a divide-by-zero path and the lane
            // measures arithmetic rather than the promotion-to-float rule.
            "mod" => Some(time_us(|| series.r#mod(&rhs_series).expect("mod"))),
            "floordiv" => Some(time_us(|| series.floordiv(&rhs_series).expect("floordiv"))),
            // br-frankenpandas-4kig1. add/sub/mul/div have NEVER been measured
            // against pandas in this harness — the four commonest operations in the
            // library had no lane at all. They also still sit on the `1 << 20`
            // threshold that cost `pow` a certified loss at 1M, and my reason for
            // leaving them there ("bandwidth-bound, parallelism will not pay") is
            // reasoning, not measurement. Two lanes, not four: `add` is the
            // cheapest of the group and `div` the most expensive (divpd is ~13
            // cycles against addpd's ~4), so they bracket the cost range that
            // decides whether the threshold matters. `sub` mirrors `add` and `mul`
            // mirrors neither usefully.
            "add" => Some(time_us(|| series.add(&rhs_series).expect("add"))),
            "div" => Some(time_us(|| series.div(&rhs_series).expect("div"))),
            _ => None,
        };
    }

    let samples = match workload {
        "floor" => time_us(|| series.floor().expect("floor")),
        "ceil" => time_us(|| series.ceil().expect("ceil")),
        "trunc" => time_us(|| series.trunc().expect("trunc")),
        "round2" => time_us(|| series.round(2).expect("round")),
        "sqrt" => time_us(|| series.sqrt().expect("sqrt")),
        "log" => time_us(|| series.log().expect("log")),
        // br-frankenpandas-4kig1. These three joined the domain-fused arm with NO
        // workload covering them, so the change landed unmeasurable. A lever with
        // no post-fix ratio is not a win, and the missing lane was the reason.
        "log10" => time_us(|| series.log10().expect("log10")),
        "log2" => time_us(|| series.log2().expect("log2")),
        "log1p" => time_us(|| series.log1p().expect("log1p")),
        // The remaining `typed_float_unary_par` residents. Four, not seventeen:
        // one representative per cost class, because the point is to learn where
        // the family sits, not to grow the matrix. expm1 and cbrt are the two
        // non-trig residents; sin is the cheapest trig and the one numpy is most
        // likely to have vectorised; atan is among the most expensive. All are
        // total on the strictly-positive fixture, so no arm drifts onto a NaN
        // path. br-frankenpandas-4kig1.
        "expm1" => time_us(|| series.expm1().expect("expm1")),
        "cbrt" => time_us(|| series.cbrt().expect("cbrt")),
        "sin" => time_us(|| series.sin().expect("sin")),
        "atan" => time_us(|| series.atan().expect("atan")),
        _ => return None,
    };
    Some(samples)
}

/// Rounds per candidate in the elementwise-policy sweep. Predeclared, not chosen
/// after looking at the numbers: MagentaFortress' note on br-frankenpandas-284ul
/// records FP's CV going 2.07% → 14.53% when a worker cap was raised to 64, so the
/// raised-cap arms are expected to be the noisy ones and the round count has to be
/// fixed in the source before the run rather than grown until an arm certifies.
const POLICY_SWEEP_ROUNDS: usize = 15;

/// The setting every candidate is compared against: today's shipped defaults.
const POLICY_SWEEP_BASELINE: (usize, usize, bool) = (8, 200_000, false);

/// `(max_workers, par_min, write_once)` arms, in the order they are measured.
///
/// `par_min = 1` forces the parallel arm so the worker cap is the only variable;
/// `(8, 2_000_000)` is above the row count and therefore forces the SERIAL arm,
/// which is the direct test of the bead's question (1). `(1, 1)` reaches serial by
/// the other door — through the `workers <= 1` guard — and the two must agree, or
/// the instrument is not measuring what it says.
///
/// The last two arms are br-frankenpandas-tyiss: `write_once = true` routes the
/// domain-fused map through per-worker owned chunks instead of one pre-zeroed
/// shared buffer, which is the third 8 MB pass numpy does not make. They are
/// carried at the SHIPPED policy and at the 4-worker policy, because 284ul found
/// 4 and 8 workers indistinguishable on throughput and the allocation cost may
/// not scale the same way the compute does.
const POLICY_SWEEP_ARMS: [(usize, usize, bool); 10] = [
    (8, 200_000, false),
    (1, 1, false),
    (2, 1, false),
    (4, 1, false),
    (16, 1, false),
    (32, 1, false),
    (64, 1, false),
    (8, 2_000_000, false),
    (8, 200_000, true),
    (4, 1, true),
];

/// br-frankenpandas-284ul: ONE process, ONE binary, both knobs varied, arms
/// interleaved ABBA against the shipped default.
///
/// This exists because the alternatives are all known to be unsound here. Separate
/// BUILDS are out: the 2026-08-16 A/A control in `docs/NEGATIVE_EVIDENCE.md` found
/// a cross-binary null swinging 0.960x–1.161x on byte-identical code, four times
/// the same-binary null, so anything under ~16% measured across two ELFs is code
/// layout. Separate PROCESSES of one ELF drop that hazard but still hand each arm
/// its own allocator and page-cache history, and they cannot interleave, so a load
/// swing lands entirely on whichever arm ran during it — this host moved 6.56 →
/// 28.64 loadavg in about forty seconds while this bead was being scoped.
///
/// The candidate list includes the baseline itself as its first entry. That row is
/// the A/A NULL: identical settings on both arms, same interleave, same rounds. A
/// candidate whose effect does not clear that null's spread has not been measured.
///
/// Every arm's output is checksummed over the raw bits and compared to the
/// baseline's. An A/B between settings that disagree on the answer is meaningless,
/// and the settings CAN disagree in principle — they move the block boundaries the
/// validity words and the `all_valid`/`all_finite` reductions are computed over.
/// Worker counts measured against the FORCED-SERIAL baseline, in order.
/// `None` means "let the group-count routing decide", which is what ships.
const SGB_ROLLING_ARMS: [Option<usize>; 5] = [None, Some(2), Some(4), Some(8), Some(16)];

/// FP-vs-FP: does group-parallelising `SeriesGroupBy.rolling` actually pay?
///
/// br-frankenpandas-u5cg4. THIS IS THE INSTRUMENT THE BEAD HAS BEEN BLOCKED ON,
/// and it is deliberately not a vs-pandas lane. The bead's question is a SELF
/// comparison — the July attempt measured 12.20ms serial against 12.29ms parallel
/// and saw nothing — and the 7.505x vs-pandas row banked on 2026-08-17 cannot
/// answer it, because a ratio against the incumbent cannot separate "the parallel
/// arm paid" from "the serial kernel was already this fast".
///
/// It could not be run before because `set_sgb_rolling_max_workers` is a
/// THREAD-LOCAL Rust API with no environment toggle, so the Python driver has no
/// way to reach it. That is on purpose: July's `FP_SGBROLL_SERIAL` was
/// process-global, and a leaked global silently re-labels a measurement (the
/// `FP_DOT_SERIAL` incident on `6df71eae2`). The answer is an in-process arm —
/// this one — not an env var.
///
/// Per section 1 of the campaign law, whatever this prints is a MAINTENANCE
/// self-speedup and NOT a win: there is no incumbent in this process.
fn run_sgb_rolling_policy_sweep(workload: &str, rows: usize, groups: i64) -> bool {
    if workload != "mean" {
        return false;
    }

    // GROUP COUNT IS AN AXIS, not a constant. br-frankenpandas-u5cg4.
    //
    // The first sweep measured only 100 groups at 1M rows — 10,000 rows per
    // group, a shape where per-worker fixed cost is trivially amortised and more
    // workers can only help. It showed the shipping default (6 workers) leaving
    // ~0.1x on the table against 8 or 16, which is a real finding and an
    // INSUFFICIENT basis for changing `SGBROLL_PAR_MIN_PER_WORKER`, because this
    // bead's own fixture is 2,000 groups over 200k rows — 100 rows each — where
    // per-group fixed cost is ~4,324 instructions and the same change could
    // lose. Varying groups at FIXED rows holds per-row work constant by
    // construction and is the measurement that decides the constant.
    //
    // Keys come from the row index so this fixture and the
    // `rolling/groupby_rolling_mean_w10` vs-incumbent lane group identically at
    // the shared default of 100.
    let keys: Vec<i64> = (0..rows as i64).map(|i| i % groups).collect();
    let mut rng = SplitMix64(0x1234_5678_9ABC_DEF0);
    let values: Vec<f64> = (0..rows).map(|_| 1.0 + rng.unit() * 99_999.0).collect();
    let index = Index::new_known_unique_int64_unit_range(0, rows);
    let value_series = Series::new("v", index.clone(), Column::from_f64_values(values))
        .expect("sgb rolling sweep value series");
    let key_series = Series::new("k", index, Column::from_i64_values(keys))
        .expect("sgb rolling sweep key series");

    let apply = |v: &Series, k: &Series| -> Series {
        v.groupby(k)
            .expect("sgb rolling sweep groupby")
            .rolling(10)
            .mean()
            .expect("sgb rolling sweep mean")
    };

    // Bits AND validity: grouped rolling emits nullable-f64 and the first
    // `window-1` slots of every group are legitimately missing, so a values-only
    // checksum would ignore exactly the part a broken window boundary corrupts.
    let checksum = |s: &Series| -> u64 {
        let column = s.column();
        let (data, valid) = column.as_f64_slice_with_validity().map_or_else(
            || {
                let d = column.as_f64_slice().expect("grouped rolling emits f64");
                (d.to_vec(), vec![true; d.len()])
            },
            |(d, v)| (d.to_vec(), (0..d.len()).map(|i| v.get(i)).collect()),
        );
        data.iter().zip(valid).fold(0_u64, |acc, (v, ok)| {
            acc.rotate_left(7) ^ v.to_bits() ^ u64::from(ok) ^ 0x9e37_79b9_7f4a_7c15
        })
    };

    // Installed OUTSIDE the clock; the kernel reads the thread-local at call time,
    // so the arm that follows is the arm that was requested. The worker count is
    // read back AFTER the call — requested is not observed, and this bead exists
    // because nobody could tell the two apart in July.
    let time_arm = |workers: Option<usize>| -> (f64, usize) {
        fp_frame::set_sgb_rolling_max_workers(workers);
        let started = Instant::now();
        let out = apply(black_box(&value_series), black_box(&key_series));
        let elapsed_us = started.elapsed().as_secs_f64() * 1e6;
        let observed = fp_frame::sgb_rolling_last_worker_count();
        black_box(out);
        (elapsed_us, observed)
    };

    const SERIAL: Option<usize> = Some(1);
    let mut rows_json: Vec<String> = Vec::with_capacity(SGB_ROLLING_ARMS.len());
    for candidate in SGB_ROLLING_ARMS {
        fp_frame::set_sgb_rolling_max_workers(candidate);
        let candidate_checksum = checksum(&apply(&value_series, &key_series));
        fp_frame::set_sgb_rolling_max_workers(SERIAL);
        let serial_checksum = checksum(&apply(&value_series, &key_series));

        for _ in 0..WARMUP {
            black_box(time_arm(SERIAL));
            black_box(time_arm(candidate));
        }

        let mut serial_us = Vec::with_capacity(POLICY_SWEEP_ROUNDS);
        let mut candidate_us = Vec::with_capacity(POLICY_SWEEP_ROUNDS);
        let mut candidate_workers = 0_usize;
        let mut serial_workers = 0_usize;
        for round in 0..POLICY_SWEEP_ROUNDS {
            // ABBA so drift and foreign load land on both arms equally.
            if round % 2 == 0 {
                let (s, sw) = time_arm(SERIAL);
                let (c, cw) = time_arm(candidate);
                serial_us.push(s);
                candidate_us.push(c);
                serial_workers = sw;
                candidate_workers = cw;
            } else {
                let (c, cw) = time_arm(candidate);
                let (s, sw) = time_arm(SERIAL);
                candidate_us.push(c);
                serial_us.push(s);
                candidate_workers = cw;
                serial_workers = sw;
            }
        }
        fp_frame::set_sgb_rolling_max_workers(None);

        let fmt = |xs: &[f64]| -> String {
            xs.iter()
                .map(|x| format!("{x}"))
                .collect::<Vec<_>>()
                .join(",")
        };
        rows_json.push(format!(
            concat!(
                "{{\"requested_workers\":{},\"observed_workers\":{},",
                "\"serial_observed_workers\":{},",
                "\"serial_checksum\":\"{:016x}\",\"candidate_checksum\":\"{:016x}\",",
                "\"bit_identical_to_serial\":{},",
                "\"serial_us\":[{}],\"candidate_us\":[{}]}}"
            ),
            candidate.map_or(-1_i64, |w| w as i64),
            candidate_workers,
            serial_workers,
            serial_checksum,
            candidate_checksum,
            serial_checksum == candidate_checksum,
            fmt(&serial_us),
            fmt(&candidate_us),
        ));
    }

    println!(
        "sgb_rolling_policy_json={{\"rows\":{},\"groups\":{},\"window\":10,\"warmup\":{},\"rounds\":{},\"arms\":[{}]}}",
        rows,
        groups,
        WARMUP,
        POLICY_SWEEP_ROUNDS,
        rows_json.join(","),
    );
    true
}

fn run_elementwise_policy_sweep(workload: &str, rows: usize, consume: bool) -> bool {
    let apply: fn(&Series) -> Series = match workload {
        "sqrt" => |s: &Series| s.sqrt().expect("policy sweep: sqrt"),
        "log" => |s: &Series| s.log().expect("policy sweep: log"),
        _ => return false,
    };

    // Same fixture as `run_math_unary`: strictly positive so `sqrt`/`log` stay in
    // domain and both engines keep the all-valid arm, non-integral so no
    // semantic-identity guard short-circuits the kernel.
    let mut rng = SplitMix64(0x1234_5678_9ABC_DEF0);
    let data: Vec<f64> = (0..rows).map(|_| 1.0 + rng.unit() * 99_999.0).collect();
    let index = Index::new_known_unique_int64_unit_range(0, rows);
    let series =
        Series::new("s", index, Column::from_f64_values(data)).expect("policy sweep series");

    let bits_checksum = |s: &Series| -> u64 {
        let values = s
            .column()
            .as_f64_slice()
            .expect("policy sweep output is an all-valid Float64 column");
        values.iter().fold(0_u64, |acc, v| {
            acc.rotate_left(7) ^ v.to_bits() ^ 0x9e37_79b9_7f4a_7c15
        })
    };

    // ⚠ CONSUME OR NOT IS THE WHOLE ANSWER FOR THE WRITE-ONCE ARM, so it is an
    // explicit axis rather than an accident. The shared-buffer arm produces a
    // contiguous `Vec<f64>`; the chunked arm produces per-worker chunks and
    // materializes them LAZILY, in a `OnceLock` that `as_f64_slice` drives and
    // whose own doc comment prices it at ~5.7ms per 1M of page faults. Timing
    // only the producer therefore credits the chunked arm for work it has merely
    // DEFERRED. `consume` touches the result the way any consumer needing a
    // contiguous slice would, inside the clock.
    let time_arm = |policy: (usize, usize, bool), consume: bool| -> f64 {
        // Installed OUTSIDE the clock; the kernel reads it on this thread at call
        // time, so the arm that follows is the arm that was requested.
        fp_columnar::set_elementwise_witness_policy(policy.0, policy.1);
        fp_columnar::set_elementwise_write_once(policy.2);
        let started = Instant::now();
        let out = apply(black_box(&series));
        if consume {
            black_box(out.column().as_f64_slice().map(<[f64]>::len));
        }
        let elapsed_us = started.elapsed().as_secs_f64() * 1e6;
        black_box(out);
        elapsed_us
    };

    let mut rows_json: Vec<String> = Vec::with_capacity(POLICY_SWEEP_ARMS.len());
    for candidate in POLICY_SWEEP_ARMS {
        fp_columnar::set_elementwise_witness_policy(candidate.0, candidate.1);
        fp_columnar::set_elementwise_write_once(candidate.2);
        let in_effect = fp_columnar::elementwise_witness_policy_in_effect();
        let candidate_checksum = bits_checksum(&apply(&series));
        // OBSERVED, not requested. `operation_threads_used` is a 20us sampler over
        // /proc/self/status and it UNDER-reports short `thread::scope` kernels, so
        // it is provenance and not proof — but a raised cap that never widens the
        // peak is a raised cap that never arrived, and that is worth seeing.
        let candidate_probe = probe_operation_threads(&mut || apply(black_box(&series)));
        fp_columnar::set_elementwise_witness_policy(
            POLICY_SWEEP_BASELINE.0,
            POLICY_SWEEP_BASELINE.1,
        );
        fp_columnar::set_elementwise_write_once(POLICY_SWEEP_BASELINE.2);
        let baseline_checksum = bits_checksum(&apply(&series));
        let baseline_probe = probe_operation_threads(&mut || apply(black_box(&series)));

        for _ in 0..WARMUP {
            black_box(time_arm(POLICY_SWEEP_BASELINE, consume));
            black_box(time_arm(candidate, consume));
        }

        let mut baseline_us = Vec::with_capacity(POLICY_SWEEP_ROUNDS);
        let mut candidate_us = Vec::with_capacity(POLICY_SWEEP_ROUNDS);
        for round in 0..POLICY_SWEEP_ROUNDS {
            if round % 2 == 0 {
                let b = time_arm(POLICY_SWEEP_BASELINE, consume);
                let c = time_arm(candidate, consume);
                baseline_us.push(b);
                candidate_us.push(c);
            } else {
                let c = time_arm(candidate, consume);
                let b = time_arm(POLICY_SWEEP_BASELINE, consume);
                candidate_us.push(c);
                baseline_us.push(b);
            }
        }

        let fmt = |xs: &[f64]| -> String {
            xs.iter()
                .map(|x| format!("{x}"))
                .collect::<Vec<_>>()
                .join(",")
        };
        rows_json.push(format!(
            concat!(
                "{{\"max_workers\":{},\"par_min\":{},\"write_once\":{},",
                "\"in_effect\":[{},{}],",
                "\"baseline_checksum\":\"{:016x}\",\"candidate_checksum\":\"{:016x}\",",
                "\"bit_identical_to_baseline\":{},",
                "\"candidate_peak_threads\":{},\"candidate_operation_threads\":{},",
                "\"baseline_peak_threads\":{},\"baseline_operation_threads\":{},",
                "\"baseline_us\":[{}],\"candidate_us\":[{}]}}"
            ),
            candidate.0,
            candidate.1,
            candidate.2,
            in_effect.0,
            in_effect.1,
            baseline_checksum,
            candidate_checksum,
            baseline_checksum == candidate_checksum,
            candidate_probe.peak_process_threads,
            candidate_probe.operation_threads_used,
            baseline_probe.peak_process_threads,
            baseline_probe.operation_threads_used,
            fmt(&baseline_us),
            fmt(&candidate_us),
        ));
    }
    fp_columnar::clear_elementwise_witness_policy();
    fp_columnar::clear_elementwise_write_once();

    println!(
        concat!(
            "{{\"sweep\":\"elementwise_policy\",\"workload\":\"{}\",\"rows\":{},",
            "\"rounds_per_arm\":{},\"warmup\":{},\"consume\":{},",
            "\"baseline\":[{},{}],\"baseline_write_once\":{},",
            "\"runtime_available_parallelism\":{},",
            "\"arms\":[{}]}}"
        ),
        workload,
        rows,
        POLICY_SWEEP_ROUNDS,
        WARMUP,
        consume,
        POLICY_SWEEP_BASELINE.0,
        POLICY_SWEEP_BASELINE.1,
        POLICY_SWEEP_BASELINE.2,
        std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get),
        rows_json.join(","),
    );
    true
}

fn run(
    category: &str,
    workload: &str,
    size: &str,
    dtype: &str,
    data_dir: Option<&Path>,
) -> Option<PairedSamples> {
    let (rows, cols) = size_rows_cols(size);
    // The pipeline category reads its inputs from disk and never touches the
    // synthetic ten-column frame. Dispatch before `build_frame` so a 10M-row
    // run does not allocate ~800 MB of unrelated columns and hold them live
    // for the whole measurement.
    if category == "pipeline" {
        return run_pipeline(workload, data_dir);
    }
    // Same reason: math_unary builds its exact one-column input below.
    if category == "math_unary" {
        return run_math_unary(workload, rows);
    }
    // The astype workloads construct their exact one-column input below.
    // Avoid retaining an unrelated ten-column frame during measurement.
    let base_cols = if category == "dataframe_ops"
        && matches!(
            workload,
            "astype_str_f64" | "astype_str_f64_telemetry_batches"
        ) {
        0
    } else {
        cols
    };
    let (df, raw) = build_frame(rows, base_cols, dtype);
    #[cfg(feature = "lazy-transpose-prototype")]
    let transpose_block = PrototypeF64Block::from_column_vectors(&raw);

    let times = match (category, workload) {
        ("dataframe_ops", "sort_values_single") => time_us(|| {
            let _ = df.sort_values("col_0", true).expect("sort_values");
        }),
        ("dataframe_ops", "sort_values_multi") => time_us(|| {
            let _ = df
                .sort_values_multi(&["col_0", "col_1", "col_2"], &[true, true, true], "last")
                .expect("sort_values_multi");
        }),
        ("dataframe_ops", "filter_bool_mask") => {
            // df[df.col_0 > df.col_0.median()]
            let med = df
                .get_column("col_0")
                .median()
                .ok()
                .and_then(|s| s.to_f64().ok())
                .unwrap_or(f64::NAN);
            let mask: Vec<bool> = raw[0].iter().map(|&v| v > med).collect();
            time_us(|| {
                let _ = df.loc_bool(&mask).expect("loc_bool");
            })
        }
        ("dataframe_ops", "drop_duplicates") => {
            let subset = vec!["col_0".to_string()];
            time_us(|| {
                let _ = df
                    .drop_duplicates(Some(&subset), DuplicateKeep::First, false)
                    .expect("drop_duplicates");
            })
        }
        ("dataframe_ops", "value_counts_i64") => {
            // value_counts on a bounded Int64 column (i%1000) — 1000 distinct.
            let col = Column::from_i64_values((0..rows as i64).map(|i| i % 1000).collect());
            let series = Series::new("s", Index::new_known_unique_int64_unit_range(0, rows), col)
                .expect("vc i64 series");
            time_us(|| {
                let _ = series.value_counts().expect("value_counts");
            })
        }
        ("dataframe_ops", "value_counts") => {
            let series = df.get_column("col_0");
            time_us(|| {
                let _ = series.value_counts().expect("value_counts");
            })
        }
        ("dataframe_ops", "cumsum_batched") => time_us_repeated_total(1000, || {
            // br-frankenpandas-d4cs8. SIBLING of `cumsum`, identical work, timed as a
            // BATCH of 1000 calls instead of one.
            //
            // MEASURED (ledger e8d10f48e): at size 100 `cumsum`'s FP arm is 3.3us and
            // its own A/A null sits 11% off unity — five times the gate's limit —
            // while the effect it refuses to certify is 9.5x and not in doubt.
            // Corpus-wide, arms under 100us fail their null ~60% of the time. Fixed-cost
            // regressions live at small n by construction, so the gate is blindest
            // exactly where that class of defect appears.
            //
            // This lane tests ONE half of the proposed remedy: does amortising the fixed
            // per-measurement cost over 1000 calls lengthen the arm enough for the A/A
            // control to hold unity? That question is answerable FP-SIDE ALONE, because
            // `null_control` is computed inside fp-bench — so it costs no harness edit
            // and orphans no standing lock.
            //
            // ⚠️ IT DOES NOT MAKE A vs-PANDAS ROW. The incumbent arm is still timed
            // single-shot by the harness, so any ratio against `cumsum_batched` compares
            // a batched arm to an unbatched one and IS MEANINGLESS WHILE LOOKING VALID.
            // A real vs-pandas row needs `time_operation_repeated` on the pandas side
            // with a MATCHED repeat count, which moves the harness sha and orphans all
            // standing locks — deliberately out of scope here.
            //
            // ⚠️ USES `time_us_repeated_total`, WHICH REPORTS THE BATCH TOTAL, NOT a
            // per-call figure — so its p50 is ~1000x `cumsum`'s and the two are NOT
            // directly comparable without dividing. The per-call helper
            // (`time_us_repeated`) is gated behind `lazy-transpose-prototype`, which is
            // NOT a default feature, so it is unreachable in a shipping build. That gating
            // is itself part of why 4 of 218 lanes use repeated timing at all.
            //
            // ⚠️ BATCHING IS NOT A PURE TIMER CHANGE: 1000 consecutive cumsum calls on
            // one frame run warmer than one call. If the per-call time moves materially
            // versus `cumsum`, that is the semantic difference showing up, and it is a
            // reason NOT to adopt batching rather than a measurement artifact to
            // explain away.
            let _ = df.cumsum().expect("cumsum");
        }),
        ("dataframe_ops", "cumsum") => time_us(|| {
            let _ = df.cumsum().expect("cumsum");
        }),
        ("dataframe_ops", "describe") => time_us(|| {
            // pandas: df.describe()
            let _ = df.describe().expect("describe");
        }),
        ("dataframe_ops", "rank") => time_us(|| {
            // pandas: df.rank() — method='average', ascending=True, na_option='keep'
            let _ = df.rank("average", true, "keep").expect("rank");
        }),
        ("dataframe_ops", "df_rank_axis1") => time_us(|| {
            let _ = df.rank_axis1("average", true, "keep").expect("rank_axis1");
        }),
        ("dataframe_ops", "df_rank_axis1_min") => time_us(|| {
            let _ = df.rank_axis1("min", true, "keep").expect("rank_axis1");
        }),
        ("dataframe_ops", "df_idxmax_axis1") => time_us(|| {
            let _ = df.idxmax_axis1().expect("idxmax_axis1");
        }),
        ("dataframe_ops", "df_idxmin_axis1") => time_us(|| {
            let _ = df.idxmin_axis1().expect("idxmin_axis1");
        }),
        ("dataframe_ops", "df_mean_axis1") => time_us(|| {
            let _ = df.mean_axis1().expect("x");
        }),
        ("dataframe_ops", "df_max_axis1") => time_us(|| {
            let _ = df.max_axis1().expect("x");
        }),
        ("dataframe_ops", "df_var_axis1") => time_us(|| {
            let _ = df.var_axis1().expect("x");
        }),
        ("dataframe_ops", "df_prod_axis1") => time_us(|| {
            let _ = df.prod_axis1().expect("x");
        }),
        ("dataframe_ops", "df_count_axis1") => time_us(|| {
            let _ = df.count_axis1().expect("x");
        }),
        ("dataframe_ops", "df_argmax_axis1") => time_us(|| {
            let _ = df.argmax_axis1().expect("x");
        }),
        ("dataframe_ops", "df_std_axis1") => time_us(|| {
            let _ = df.std_axis1().expect("std_axis1");
        }),
        ("dataframe_ops", "df_median_axis1") => time_us(|| {
            let _ = df.median_axis1().expect("median_axis1");
        }),
        ("dataframe_ops", "df_abs") => time_us(|| {
            // pandas: df.abs()
            let _ = df.abs().expect("abs");
        }),
        #[cfg(feature = "lazy-transpose-view")]
        ("dataframe_ops", "df_transpose") => time_us_repeated_total(1024, || {
            // pandas: df.T. Exercise the actual public DataFrame::transpose
            // call; shape inspection is metadata-only for the feature-gated
            // homogeneous storage and does not cross the materialization
            // boundary.
            let transposed = df.transpose().expect("transpose");
            let shape = transposed.shape();
            black_box((&transposed, shape));
        }),
        // BENCHMARK-INTEGRITY SIBLING (cc_fp). `df_transpose` above deliberately
        // stops at metadata (`shape()`), so under `lazy-transpose-view` it never
        // crosses the materialization boundary. That makes it a DEAD BENCHMARK for
        // any lever aimed at the materialization path (the ledger's own named next
        // frontier: an indexable per-output-column lazy slot store) -- such a lever
        // would show ~0% self-time here and a REJECT measured on it would be a
        // dead-code reject. This row forces the observer: it reads real VALUES out
        // of a transposed column, which per the lazy-storage contract materializes
        // the whole output map. Both the eager and the lazy build execute the same
        // work here, so the two are comparable.
        ("dataframe_ops", "df_transpose_materialize") => time_us(|| {
            let transposed = df.transpose().expect("transpose");
            // Touch a real column's values: the first value observer crosses
            // the public-column boundary and materializes ONE output column.
            // Mirror the pandas row exactly (`t.columns[0]` then read that
            // column) — the O(1) positional name accessor, not a full
            // column-axis label materialization, which pandas' block axis
            // never does for a single lookup.
            let first = transposed
                .column_name_at(0)
                .expect("transposed frame has columns");
            let col = transposed
                .column(first.as_str())
                .expect("named column exists");
            let vals = col.values();
            black_box((&transposed, vals.len()));
        }),
        #[cfg(not(feature = "lazy-transpose-view"))]
        ("dataframe_ops", "df_transpose") => time_us(|| {
            // pandas: df.T
            let _ = df.transpose().expect("transpose");
        }),
        #[cfg(feature = "lazy-transpose-prototype")]
        ("dataframe_ops", "df_transpose_2d_block_view_proto") => time_us_repeated(65_536, || {
            let view = transpose_block.transpose_view();
            let shape = view.shape();
            let first = if shape.0 > 0 && shape.1 > 0 {
                view.get(0, 0)
            } else {
                0.0
            };
            let last = if shape.0 > 0 && shape.1 > 0 {
                view.get(shape.0 - 1, shape.1 - 1)
            } else {
                0.0
            };
            black_box((shape, first, last));
        }),
        ("dataframe_ops", "df_diff") => time_us(|| {
            // pandas: df.diff()
            let _ = df.diff(1).expect("diff");
        }),
        ("dataframe_ops", "df_notna") => time_us(|| {
            // pandas: df.notna()
            let _ = df.notna().expect("notna");
        }),
        ("dataframe_ops", "df_pivot_table") => {
            // pandas: df.pivot_table(values="v", index="r", columns="c",
            // aggfunc="mean"); r=i%100 (100 rows), c=i%10 (10 cols) -> 100x10.
            let r: Vec<i64> = (0..rows as i64).map(|i| i % 100).collect();
            let c: Vec<i64> = (0..rows as i64).map(|i| i % 10).collect();
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let mut columns = BTreeMap::new();
            columns.insert("r".to_string(), Column::from_i64_values(r));
            columns.insert("c".to_string(), Column::from_i64_values(c));
            columns.insert("v".to_string(), Column::from_f64_values(raw[0].clone()));
            let pframe = DataFrame::new_with_column_order(
                index,
                columns,
                vec!["r".to_string(), "c".to_string(), "v".to_string()],
            )
            .expect("pivot frame");
            time_us(|| {
                let _ = pframe
                    .pivot_table("v", "r", "c", "mean")
                    .expect("pivot_table");
            })
        }
        ("dataframe_ops", "df_pivot_table_std") => {
            let r: Vec<i64> = (0..rows as i64).map(|i| i % 100).collect();
            let c: Vec<i64> = (0..rows as i64).map(|i| i % 10).collect();
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let mut columns = BTreeMap::new();
            columns.insert("r".to_string(), Column::from_i64_values(r));
            columns.insert("c".to_string(), Column::from_i64_values(c));
            columns.insert("v".to_string(), Column::from_f64_values(raw[0].clone()));
            let pframe = DataFrame::new_with_column_order(
                index,
                columns,
                vec!["r".to_string(), "c".to_string(), "v".to_string()],
            )
            .expect("pivot frame");
            time_us(|| {
                let _ = pframe
                    .pivot_table("v", "r", "c", "std")
                    .expect("pivot_table");
            })
        }
        ("dataframe_ops", "df_pivot_table_median") => {
            let r: Vec<i64> = (0..rows as i64).map(|i| i % 100).collect();
            let c: Vec<i64> = (0..rows as i64).map(|i| i % 10).collect();
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let mut columns = BTreeMap::new();
            columns.insert("r".to_string(), Column::from_i64_values(r));
            columns.insert("c".to_string(), Column::from_i64_values(c));
            columns.insert("v".to_string(), Column::from_f64_values(raw[0].clone()));
            let pframe = DataFrame::new_with_column_order(
                index,
                columns,
                vec!["r".to_string(), "c".to_string(), "v".to_string()],
            )
            .expect("pivot frame");
            time_us(|| {
                let _ = pframe
                    .pivot_table("v", "r", "c", "median")
                    .expect("pivot_table");
            })
        }
        ("dataframe_ops", "df_pivot") => {
            // pandas: df.pivot(index="r", columns="c", values="v"); UNIQUE (r,c)
            // pairs (pivot errors on dups): r=i/10 (rows/10 distinct), c=i%10.
            let r: Vec<i64> = (0..rows as i64).map(|i| i / 10).collect();
            let c: Vec<i64> = (0..rows as i64).map(|i| i % 10).collect();
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let mut columns = BTreeMap::new();
            columns.insert("r".to_string(), Column::from_i64_values(r));
            columns.insert("c".to_string(), Column::from_i64_values(c));
            columns.insert("v".to_string(), Column::from_f64_values(raw[0].clone()));
            let pframe = DataFrame::new_with_column_order(
                index,
                columns,
                vec!["r".to_string(), "c".to_string(), "v".to_string()],
            )
            .expect("pivot frame");
            time_us(|| {
                let _ = pframe.pivot("r", "c", "v").expect("pivot");
            })
        }
        ("dataframe_ops", "df_crosstab") => {
            // pandas: pd.crosstab(a, b); a=i%100, b=i%10 -> 100x10 counts.
            let a = Column::from_i64_values((0..rows as i64).map(|i| i % 100).collect());
            let b = Column::from_i64_values((0..rows as i64).map(|i| i % 10).collect());
            let s1 = Series::new("a", Index::new_known_unique_int64_unit_range(0, rows), a)
                .expect("crosstab s1");
            let s2 = Series::new("b", Index::new_known_unique_int64_unit_range(0, rows), b)
                .expect("crosstab s2");
            time_us(|| {
                let _ = DataFrame::crosstab(&s1, &s2).expect("crosstab");
            })
        }
        ("dataframe_ops", "series_map") => {
            // pandas: s.map(mapper); s values in 0..100, mapper maps 0..99.
            let vals = Column::from_i64_values((0..rows as i64).map(|i| i % 100).collect());
            let s = Series::new("s", Index::new_known_unique_int64_unit_range(0, rows), vals)
                .expect("map self");
            let mvals = Column::from_i64_values((0..100).collect());
            let mapper = Series::new("m", Index::new_known_unique_int64_unit_range(0, 100), mvals)
                .expect("mapper");
            time_us(|| {
                let _ = s.map_series(&mapper).expect("map");
            })
        }
        ("dataframe_ops", "df_unstack") => {
            // Series with "r, c" composite labels (r=i/10, c=i%10) -> unstack to
            // (rows/10) x 10. Mirrors df_pivot shape via the composite-key path.
            let labels: Vec<IndexLabel> = (0..rows)
                .map(|i| IndexLabel::Utf8(format!("{}, {}", i / 10, i % 10)))
                .collect();
            let s = Series::new(
                "s",
                Index::new(labels),
                Column::from_f64_values(raw[0].clone()),
            )
            .expect("unstack series");
            time_us(|| {
                let _ = s.unstack().expect("unstack");
            })
        }
        ("dataframe_ops", "df_get_dummies") => {
            // pandas: pd.get_dummies(df, columns=["cat"]); cat=i%100 -> 100 dummies.
            let cat = Column::from_i64_values((0..rows as i64).map(|i| i % 100).collect());
            let mut columns = BTreeMap::new();
            columns.insert("cat".to_string(), cat);
            let df = DataFrame::new_with_column_order(
                Index::new_known_unique_int64_unit_range(0, rows),
                columns,
                vec!["cat".to_string()],
            )
            .expect("gd frame");
            time_us(|| {
                let _ = df.get_dummies(&["cat"]).expect("get_dummies");
            })
        }
        ("dataframe_ops", "series_categorical") => {
            // pandas: pd.Series(arr).astype("category"); arr=i%100 (100 cats).
            let values: Vec<fp_types::Scalar> = (0..rows as i64)
                .map(|i| fp_types::Scalar::Int64(i % 100))
                .collect();
            time_us(|| {
                let _ = Series::from_categorical("c", values.clone(), false).expect("categorical");
            })
        }
        ("dataframe_ops", "df_quantile") => time_us(|| {
            // pandas: df.quantile(0.5)
            let _ = df.quantile(0.5).expect("quantile");
        }),
        ("dataframe_ops", "series_skew") | ("dataframe_ops", "series_kurtosis") => {
            // br-frankenpandas-8s4mb. `Series::skew`/`kurtosis` had NO lane — the
            // only skew/kurt lanes in this binary are the GROUPBY ones, which route
            // through a different kernel entirely. So the blocked-moment change
            // (8-lane mean + 8-lane m2/m3/m4 replacing the left-fold and the ordered
            // scalar loop) shipped with no way to measure it, exactly the gap
            // `4kig1` records for the Int64 math_unary arms.
            //
            // One column, not the ten-column frame: these are Series methods, and
            // the typed fast path this change touches is `as_f64_slice`, which needs
            // a contiguous all-valid f64 backing. Built OUTSIDE the timed closure so
            // the clone is not in the measurement.
            let series = Series::new(
                "s",
                Index::new_known_unique_int64_unit_range(0, rows),
                Column::from_f64_values(raw[0].clone()),
            )
            .expect("moment series");
            if workload == "series_skew" {
                time_us(|| {
                    let _ = series.skew().expect("skew");
                })
            } else {
                time_us(|| {
                    let _ = series.kurtosis().expect("kurtosis");
                })
            }
        }
        ("dataframe_ops", "df_transpose_full_materialize") => time_us(|| {
            // br-frankenpandas-l4vzc, requested by the other pane, and it exists
            // because the two lanes that look like they cover this DO NOT.
            //
            // `df_transpose` times the shell and `df_transpose_materialize` reads
            // ONE column — and at these shapes the frame is rows x 10, so the
            // transposed frame is 10 rows x `rows` COLUMNS and "one column" is ten
            // numbers. Both arms mirror each other honestly and both stop short of
            // the boundary l4vzc is actually about: pandas' `.T` is an O(1) view of
            // its 2D BlockManager, while FrankenPandas must build one column per
            // output. That claim (fp 80ms vs pandas 52us, 2026-06-21) has never been
            // re-tested, because nothing in this harness crosses the boundary.
            //
            // So: read EVERY output column. Named to be unmistakable — a third
            // `*_materialize` would have been read as a variant of the existing one.
            //
            // ⚠️ THE TWO ARMS DO NOT DO EQUAL WORK, AND THAT IS THE POINT RATHER
            // THAN A DEFECT. The pandas arm is `df.T.to_numpy()`, which it answers
            // from its 2D block without ever building the transposed frame's
            // `rows`-long column index — measured flat at ~114-179us from 1k to
            // 100k rows before this lane landed. FrankenPandas has no whole-frame
            // array output, so this arm must build one Column per output column.
            // That asymmetry is exactly l4vzc's claim. Do NOT read the ratio as
            // "our transpose kernel is N times slower"; read it as "each engine
            // materialises the transposed result in its best available form, and
            // pandas' best form is structurally cheaper". Forcing pandas onto a
            // per-column route to even the work up would be choosing a bad opponent.
            //
            // ⚠️ SECOND CONFOUND, AND IT IS THE ONE THAT FLATTERS US — added after
            // the other pane read the source (br-frankenpandas-3ya6b). The caveat
            // above names the REPRESENTATIONAL gap and stops there. This arm ALSO
            // pays an ordinary addressable constant: there is no positional column
            // accessor, so the only public route from a position to a column is
            //     column_name_at(i) -> owned String -> column(name.as_str())
            // and `name_at` on the Int64UnitRange variant is `start + i` followed by
            // `.to_string()`. Worse, it is a ROUND TRIP: `get_one` then parses the
            // name back to an i64 and re-formats it to validate, so it is TWO
            // allocations and a parse per column to get from a position back to the
            // same position. At 1M source rows the transposed frame has 1,000,000
            // columns, so this arm pays ~2M allocations and ~1M parses BEFORE any
            // materialization. This is the only shape in the corpus where column
            // COUNT scales with data size, which is why a per-column constant that
            // is invisible everywhere else can dominate here.
            //
            // THE LANE IS STILL RIGHT AND SHOULD NOT BE "FIXED" TO AVOID IT. The
            // name route was the only public one when this lane was written, so it
            // measures what the API actually costs a caller. The finding is that the
            // API had a hole, not that the lane chose badly. But a reader must not
            // attribute the whole ratio to representation: it is representation PLUS
            // an addressable constant, and `3ya6b` predicts removing the constant
            // cuts a large ABSOLUTE cost with NO sign change.
            let transposed = df.transpose().expect("transpose");
            let mut touched = 0usize;
            for position in 0..transposed.num_columns() {
                let name = transposed
                    .column_name_at(position)
                    .expect("transposed frame column");
                let col = transposed.column(name.as_str()).expect("named column");
                touched += col.values().len();
            }
            black_box((&transposed, touched));
        }),
        ("dataframe_ops", "df_transpose_full_materialize_positional") => time_us(|| {
            // br-frankenpandas-3ya6b. SIBLING OF `df_transpose_full_materialize`,
            // identical in every respect except HOW a column is addressed. It exists
            // so the API tax and the representational gap can be separated, because
            // the older lane necessarily conflates them.
            //
            // The sibling reaches each column by label — `column_name_at(i)` hands
            // back an OWNED String and `column(name)` parses it back into the
            // position it was formatted from, then re-formats it to validate. Two
            // allocations and a parse per column to arrive where the caller already
            // was. This lane calls `column_at(i)` instead, which goes straight to the
            // lazy plan's cached column.
            //
            // ⚠️ WHAT A DIFFERENCE BETWEEN THESE TWO LANES DOES *NOT* MEAN. It does
            // NOT close the l4vzc gap and must not be reported as doing so. pandas
            // answers `df.T.to_numpy()` from its 2D BlockManager and measured FLAT at
            // 45.2 / 44.6 / 44.6us from 10k to 1M rows (ledger 778a7eeb2), while BOTH
            // of these lanes still build one Column per output column. The prediction
            // recorded on 3ya6b before this lane existed is a large ABSOLUTE saving
            // with NO sign change against the incumbent; if a measurement ever shows a
            // sign flip, l4vzc's structural account is wrong and THAT is the finding.
            //
            // ⚠️ BOTH LANES MUST KEEP READING EVERY COLUMN AND SUMMING EVERY LENGTH.
            // The `touched` accumulator and the `black_box` are load-bearing: without
            // them an optimiser is free to drop the loop, and a lane that measures
            // nothing would report an enormous and entirely false improvement over its
            // sibling. That failure would look exactly like the win this lane exists
            // to detect.
            let transposed = df.transpose().expect("transpose");
            let mut touched = 0usize;
            for position in 0..transposed.num_columns() {
                let col = transposed
                    .column_at(position)
                    .expect("positional column present");
                touched += col.values().len();
            }
            black_box((&transposed, touched));
        }),
        ("dataframe_ops", "df_skew") => time_us(|| {
            // pandas: df.skew()
            let _ = df.skew().expect("skew");
        }),
        ("dataframe_ops", "df_sem") => time_us(|| {
            // pandas: df.sem()
            let _ = df.sem().expect("sem");
        }),
        ("dataframe_ops", "df_nunique") => time_us(|| {
            // pandas: df.nunique()
            let _ = df.nunique().expect("nunique");
        }),
        ("dataframe_ops", "df_cumprod") => time_us(|| {
            // pandas: df.cumprod()
            let _ = df.cumprod().expect("cumprod");
        }),
        ("dataframe_ops", "df_shift") => time_us(|| {
            // pandas: df.shift(1)
            let _ = df.shift(1).expect("shift");
        }),
        ("dataframe_ops", "df_pct_change") => time_us(|| {
            // pandas: df.pct_change()
            let _ = df.pct_change(1).expect("pct_change");
        }),
        ("dataframe_ops", "df_ffill") => time_us(|| {
            // pandas: df.ffill() — run with --dtype float64_nan10/nan50
            let _ = df.ffill(None).expect("ffill");
        }),
        ("dataframe_ops", "df_interpolate") => time_us(|| {
            // pandas: df.interpolate() — run with --dtype float64_nan10
            let _ = df.interpolate().expect("interpolate");
        }),
        ("dataframe_ops", "df_set_index") => time_us(|| {
            // pandas: df.set_index("col_0")
            let _ = df.set_index("col_0", true).expect("set_index");
        }),
        ("dataframe_ops", "df_reset_index") => time_us(|| {
            // pandas: df.reset_index()
            let _ = df.reset_index(false).expect("reset_index");
        }),
        ("dataframe_ops", "df_sort_index") => time_us(|| {
            // pandas: df.sort_index()
            let _ = df.sort_index(true).expect("sort_index");
        }),
        ("dataframe_ops", "astype_str_i64" | "astype_str_f64" | "astype_str_bool") => {
            let col = match workload {
                "astype_str_i64" => Column::from_i64_values((0..rows as i64).collect()),
                "astype_str_bool" => {
                    Column::from_bool_values((0..rows).map(|i| i % 2 == 0).collect())
                }
                _ => Column::from_f64_values((0..rows).map(|i| i as f64 * 1.5).collect()),
            };
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let series = Series::new("s", index, col).expect("astype series");
            time_us(|| {
                let _ = series.astype(fp_types::DType::Utf8).expect("astype str");
            })
        }
        ("dataframe_ops", "astype_str_f64_telemetry_batches") => {
            // Realistic bounded-memory sink: format every finite telemetry
            // value into an ordered Utf8 Series, consume each 250k-row batch,
            // then release it before advancing. Input population is untimed.
            let batches = build_telemetry_string_batches(rows);
            time_us(|| {
                let mut observed_rows = 0_usize;
                for batch in &batches {
                    let rendered = batch
                        .astype(fp_types::DType::Utf8)
                        .expect("telemetry batch astype str");
                    black_box((
                        rendered.values().first().expect("nonempty batch"),
                        rendered.values().last().expect("nonempty batch"),
                    ));
                    observed_rows += rendered.len();
                    black_box(rendered);
                }
                black_box(observed_rows)
            })
        }
        ("dataframe_ops", "df_melt") => time_us(|| {
            // pandas: df.melt()
            let _ = df.melt(&[], &[], None, None).expect("melt");
        }),
        (
            "dataframe_ops",
            "df_explode" | "df_explode_string_python" | "df_explode_string_arrow",
        ) => {
            // Series of comma-separated strings "aN,bN,cN" (3 parts each).
            // pandas: s.str.split(",").explode().
            let mut bytes: Vec<u8> = Vec::new();
            let mut offsets: Vec<usize> = vec![0];
            for i in 0..rows {
                bytes.extend_from_slice(format!("a{},b{},c{}", i % 97, i % 89, i % 83).as_bytes());
                offsets.push(bytes.len());
            }
            let col = Column::from_utf8_contiguous(bytes, offsets);
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let series = Series::new("s", index, col).expect("explode series");
            time_us(|| {
                let _ = series.explode(",").expect("explode");
            })
        }
        ("dataframe_ops", "df_nlargest") => time_us(|| {
            // pandas: df.nlargest(100, "col_0")
            let _ = df.nlargest(100, "col_0").expect("nlargest");
        }),
        ("dataframe_ops", "df_stack") => time_us(|| {
            // pandas: df.stack()
            let _ = df.stack().expect("stack");
        }),
        ("dataframe_ops", "df_duplicated") => time_us(|| {
            // pandas: df.duplicated()
            let _ = df
                .duplicated(None, DuplicateKeep::First)
                .expect("duplicated");
        }),
        ("dataframe_ops", "df_idxmax") => time_us(|| {
            // pandas: df.idxmax()
            let _ = df.idxmax().expect("idxmax");
        }),
        ("dataframe_ops", "df_count") => time_us(|| {
            // pandas: df.count()
            let _ = df.count().expect("count");
        }),
        ("dataframe_ops", "df_to_numpy") => paired_time_us_with_setup(
            || build_frame(rows, cols, dtype).0,
            |fresh| {
                // pandas: df.to_numpy(). Fresh construction is outside the
                // timer, preventing a cached consolidation from posing as a
                // first-call materialization win.
                fresh.to_numpy()
            },
        ),
        ("dataframe_ops", "df_values") => paired_time_us_with_setup(
            || build_frame(rows, cols, dtype).0,
            |fresh| {
                // pandas: df.values (Vec<Vec<Scalar>> row-major materialization).
                // Each arm gets a fresh frame outside the timed boundary.
                fresh.values()
            },
        ),
        ("dataframe_ops", "df_iterrows") => time_us(|| {
            // pandas: list(df.iterrows())
            let _ = df.iterrows();
        }),
        ("dataframe_ops", "df_itertuples" | "df_row_tuples_fastest") => time_us(|| {
            // pandas exact arm: list(df.itertuples()). The
            // df_row_tuples_fastest fairness arm uses the fastest independently
            // screened pandas route to a fully materialized tuple per row.
            let _ = df.itertuples();
        }),
        ("dataframe_ops", "df_mode") => time_us(|| {
            // pandas: df.mode()
            let _ = df.mode().expect("mode");
        }),
        ("dataframe_ops", "df_fillna") => {
            // pandas: df.fillna(0.0) — run with --dtype float64_nan10/nan50
            let fill = fp_types::Scalar::Float64(0.0);
            time_us(|| {
                let _ = df.fillna(&fill).expect("fillna");
            })
        }
        ("dataframe_ops", "df_add_scalar") => time_us(|| {
            // pandas: df + 5.0
            let _ = df.add_scalar(5.0).expect("add_scalar");
        }),
        ("dataframe_ops", "df_sign") => time_us(|| {
            // pandas: np.sign(df)
            let _ = df.sign().expect("sign");
        }),
        ("dataframe_ops", "df_neg") => time_us(|| {
            // pandas: -df
            let _ = df.neg().expect("neg");
        }),
        ("dataframe_ops", "df_floor") => time_us(|| {
            // pandas: np.floor(df)
            let _ = df.floor().expect("floor");
        }),
        ("dataframe_ops", "df_ceil") => time_us(|| {
            // pandas: np.ceil(df)
            let _ = df.ceil().expect("ceil");
        }),
        ("dataframe_ops", "df_round") => time_us(|| {
            // pandas: df.round(2)
            let _ = df.round(2).expect("round");
        }),
        ("dataframe_ops", "df_clip") => time_us(|| {
            // pandas: df.clip(lower=0, upper=500000)
            let _ = df.clip(Some(0.0), Some(500_000.0)).expect("clip");
        }),
        ("dataframe_ops", "df_isna") => time_us(|| {
            // pandas: df.isna()
            let _ = df.isna().expect("isna");
        }),
        (
            "groupby",
            "groupby_sum_int64"
            | "groupby_mean_float64"
            | "groupby_agg_multi"
            | "groupby_std"
            | "groupby_median"
            | "groupby_nunique"
            | "groupby_first"
            | "groupby_max",
        ) => {
            // pandas: key = (col_0 % 100).astype(int64); groupby(key)[col_1].agg
            let keys: Vec<i64> = raw[0].iter().map(|&v| (v as i64).rem_euclid(100)).collect();
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let mut columns = BTreeMap::new();
            columns.insert("key".to_string(), Column::from_i64_values(keys));
            columns.insert("col_1".to_string(), Column::from_f64_values(raw[1].clone()));
            let gframe = DataFrame::new_with_column_order(
                index,
                columns,
                vec!["key".to_string(), "col_1".to_string()],
            )
            .expect("fp-bench groupby frame");
            match workload {
                "groupby_std" => time_us(|| {
                    let _ = gframe
                        .groupby(&["key"])
                        .expect("groupby")
                        .std()
                        .expect("std");
                }),
                "groupby_median" => time_us(|| {
                    let _ = gframe
                        .groupby(&["key"])
                        .expect("groupby")
                        .median()
                        .expect("median");
                }),
                "groupby_nunique" => time_us(|| {
                    let _ = gframe
                        .groupby(&["key"])
                        .expect("groupby")
                        .nunique()
                        .expect("nunique");
                }),
                "groupby_first" => time_us(|| {
                    let _ = gframe
                        .groupby(&["key"])
                        .expect("groupby")
                        .first()
                        .expect("first");
                }),
                "groupby_max" => time_us(|| {
                    let _ = gframe
                        .groupby(&["key"])
                        .expect("groupby")
                        .max()
                        .expect("max");
                }),
                "groupby_sum_int64" => time_us(|| {
                    let _ = gframe
                        .groupby(&["key"])
                        .expect("groupby")
                        .sum()
                        .expect("sum");
                }),
                "groupby_mean_float64" => time_us(|| {
                    let _ = gframe
                        .groupby(&["key"])
                        .expect("groupby")
                        .mean()
                        .expect("mean");
                }),
                _ => time_us(|| {
                    // pandas: df.groupby("key").agg({"col_1": ["sum","mean","std"]})
                    // — one multi-agg call. Mirror it with the canonical agg API
                    // instead of three separate gb.sum()/mean()/std() calls so the
                    // workload measures the path br-frankenpandas-m0gcq will fuse.
                    let gb = gframe.groupby(&["key"]).expect("groupby");
                    let _ = gb.agg_list(&["sum", "mean", "std"]).expect("agg_list");
                }),
            }
        }
        ("groupby", "groupby_transform_mean") => {
            // pandas: s = df["col_1"]; s.groupby(key).transform("mean")
            // — SeriesGroupBy.transform (broadcast each group's mean back to its
            // rows). key = (col_0 % 100).astype(int64).
            let keys: Vec<i64> = raw[0].iter().map(|&v| (v as i64).rem_euclid(100)).collect();
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new(
                "key".to_string(),
                index.clone(),
                Column::from_i64_values(keys),
            )
            .expect("key series");
            let val_series = Series::new(
                "col_1".to_string(),
                index,
                Column::from_f64_values(raw[1].clone()),
            )
            .expect("val series");
            time_us(|| {
                let _ = val_series
                    .groupby(&key_series)
                    .expect("groupby")
                    .transform("mean")
                    .expect("transform");
            })
        }
        ("groupby", "groupby_mean_str") => {
            // String-key aggregation: s.groupby(str_key).mean(). key =
            // "g{col_0 % 1000:04}" (~1000 distinct categorical labels).
            let mut kb = Vec::with_capacity(rows * 5);
            let mut ko = Vec::with_capacity(rows + 1);
            ko.push(0usize);
            for &v in raw[0].iter() {
                kb.extend_from_slice(format!("g{:04}", (v as i64).rem_euclid(1000)).as_bytes());
                ko.push(kb.len());
            }
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new(
                "key".to_string(),
                index.clone(),
                Column::from_utf8_contiguous(kb, ko),
            )
            .expect("key series");
            let val_series = Series::new(
                "col_1".to_string(),
                index,
                Column::from_f64_values(raw[1].clone()),
            )
            .expect("val series");
            time_us(|| {
                let _ = val_series
                    .groupby(&key_series)
                    .expect("groupby")
                    .mean()
                    .expect("mean");
            })
        }
        ("groupby", "groupby_median_str") => {
            let mut kb = Vec::with_capacity(rows * 5);
            let mut ko = Vec::with_capacity(rows + 1);
            ko.push(0usize);
            for &v in raw[0].iter() {
                kb.extend_from_slice(format!("g{:04}", (v as i64).rem_euclid(1000)).as_bytes());
                ko.push(kb.len());
            }
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new(
                "key".to_string(),
                index.clone(),
                Column::from_utf8_contiguous(kb, ko),
            )
            .expect("key");
            let val_series = Series::new(
                "col_1".to_string(),
                index,
                Column::from_f64_values(raw[1].clone()),
            )
            .expect("val");
            time_us(|| {
                let _ = val_series
                    .groupby(&key_series)
                    .expect("groupby")
                    .median()
                    .expect("median");
            })
        }
        ("groupby", "groupby_std_str") => {
            let mut kb = Vec::with_capacity(rows * 5);
            let mut ko = Vec::with_capacity(rows + 1);
            ko.push(0usize);
            for &v in raw[0].iter() {
                kb.extend_from_slice(format!("g{:04}", (v as i64).rem_euclid(1000)).as_bytes());
                ko.push(kb.len());
            }
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new(
                "key".to_string(),
                index.clone(),
                Column::from_utf8_contiguous(kb, ko),
            )
            .expect("key");
            let val_series = Series::new(
                "col_1".to_string(),
                index,
                Column::from_f64_values(raw[1].clone()),
            )
            .expect("val");
            time_us(|| {
                let _ = val_series
                    .groupby(&key_series)
                    .expect("groupby")
                    .std()
                    .expect("std");
            })
        }
        ("groupby", "groupby_var_str") => {
            let mut kb = Vec::with_capacity(rows * 5);
            let mut ko = Vec::with_capacity(rows + 1);
            ko.push(0usize);
            for &v in raw[0].iter() {
                kb.extend_from_slice(format!("g{:04}", (v as i64).rem_euclid(1000)).as_bytes());
                ko.push(kb.len());
            }
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new(
                "key".to_string(),
                index.clone(),
                Column::from_utf8_contiguous(kb, ko),
            )
            .expect("key");
            let val_series = Series::new(
                "col_1".to_string(),
                index,
                Column::from_f64_values(raw[1].clone()),
            )
            .expect("val");
            time_us(|| {
                let _ = val_series
                    .groupby(&key_series)
                    .expect("groupby")
                    .var()
                    .expect("var");
            })
        }
        ("groupby", "groupby_multi_str") => {
            let mut kb = Vec::with_capacity(rows * 5);
            let mut ko = Vec::with_capacity(rows + 1);
            ko.push(0usize);
            for &v in raw[0].iter() {
                kb.extend_from_slice(format!("g{:04}", (v as i64).rem_euclid(1000)).as_bytes());
                ko.push(kb.len());
            }
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new(
                "key".to_string(),
                index.clone(),
                Column::from_utf8_contiguous(kb, ko),
            )
            .expect("key");
            let val_series = Series::new(
                "col_1".to_string(),
                index,
                Column::from_f64_values(raw[1].clone()),
            )
            .expect("val");
            time_us(|| {
                let g = val_series.groupby(&key_series).expect("groupby");
                let _ = g.mean().expect("mean");
                let _ = g.std().expect("std");
                let _ = g.var().expect("var");
            })
        }
        ("groupby", "groupby_min_str") => {
            let mut kb = Vec::with_capacity(rows * 5);
            let mut ko = Vec::with_capacity(rows + 1);
            ko.push(0usize);
            for &v in raw[0].iter() {
                kb.extend_from_slice(format!("g{:04}", (v as i64).rem_euclid(1000)).as_bytes());
                ko.push(kb.len());
            }
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new(
                "key".to_string(),
                index.clone(),
                Column::from_utf8_contiguous(kb, ko),
            )
            .expect("key");
            let val_series = Series::new(
                "col_1".to_string(),
                index,
                Column::from_f64_values(raw[1].clone()),
            )
            .expect("val");
            time_us(|| {
                let _ = val_series
                    .groupby(&key_series)
                    .expect("groupby")
                    .min()
                    .expect("min");
            })
        }
        ("groupby", "groupby_max_str") => {
            let mut kb = Vec::with_capacity(rows * 5);
            let mut ko = Vec::with_capacity(rows + 1);
            ko.push(0usize);
            for &v in raw[0].iter() {
                kb.extend_from_slice(format!("g{:04}", (v as i64).rem_euclid(1000)).as_bytes());
                ko.push(kb.len());
            }
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new(
                "key".to_string(),
                index.clone(),
                Column::from_utf8_contiguous(kb, ko),
            )
            .expect("key");
            let val_series = Series::new(
                "col_1".to_string(),
                index,
                Column::from_f64_values(raw[1].clone()),
            )
            .expect("val");
            time_us(|| {
                let _ = val_series
                    .groupby(&key_series)
                    .expect("groupby")
                    .max()
                    .expect("max");
            })
        }
        ("groupby", "groupby_prod_str") => {
            let mut kb = Vec::with_capacity(rows * 5);
            let mut ko = Vec::with_capacity(rows + 1);
            ko.push(0usize);
            for &v in raw[0].iter() {
                kb.extend_from_slice(format!("g{:04}", (v as i64).rem_euclid(1000)).as_bytes());
                ko.push(kb.len());
            }
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new(
                "key".to_string(),
                index.clone(),
                Column::from_utf8_contiguous(kb, ko),
            )
            .expect("key");
            let val_series = Series::new(
                "col_1".to_string(),
                index,
                Column::from_f64_values(raw[1].clone()),
            )
            .expect("val");
            time_us(|| {
                let _ = val_series
                    .groupby(&key_series)
                    .expect("groupby")
                    .prod()
                    .expect("prod");
            })
        }
        ("groupby", "groupby_rank_str") => {
            let mut kb = Vec::with_capacity(rows * 5);
            let mut ko = Vec::with_capacity(rows + 1);
            ko.push(0usize);
            for &v in raw[0].iter() {
                kb.extend_from_slice(format!("g{:04}", (v as i64).rem_euclid(1000)).as_bytes());
                ko.push(kb.len());
            }
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new(
                "key".to_string(),
                index.clone(),
                Column::from_utf8_contiguous(kb, ko),
            )
            .expect("key");
            let val_series = Series::new(
                "col_1".to_string(),
                index,
                Column::from_f64_values(raw[1].clone()),
            )
            .expect("val");
            time_us(|| {
                let _ = val_series
                    .groupby(&key_series)
                    .expect("groupby")
                    .rank("average", true, "keep")
                    .expect("rank");
            })
        }
        ("groupby", "groupby_sem_str") => {
            let mut kb = Vec::with_capacity(rows * 5);
            let mut ko = Vec::with_capacity(rows + 1);
            ko.push(0usize);
            for &v in raw[0].iter() {
                kb.extend_from_slice(format!("g{:04}", (v as i64).rem_euclid(1000)).as_bytes());
                ko.push(kb.len());
            }
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new(
                "key".to_string(),
                index.clone(),
                Column::from_utf8_contiguous(kb, ko),
            )
            .expect("key");
            let val_series = Series::new(
                "col_1".to_string(),
                index,
                Column::from_f64_values(raw[1].clone()),
            )
            .expect("val");
            time_us(|| {
                let _ = val_series
                    .groupby(&key_series)
                    .expect("groupby")
                    .sem()
                    .expect("sem");
            })
        }
        ("groupby", "groupby_skew_str") => {
            let mut kb = Vec::with_capacity(rows * 5);
            let mut ko = Vec::with_capacity(rows + 1);
            ko.push(0usize);
            for &v in raw[0].iter() {
                kb.extend_from_slice(format!("g{:04}", (v as i64).rem_euclid(1000)).as_bytes());
                ko.push(kb.len());
            }
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new(
                "key".to_string(),
                index.clone(),
                Column::from_utf8_contiguous(kb, ko),
            )
            .expect("key");
            let val_series = Series::new(
                "col_1".to_string(),
                index,
                Column::from_f64_values(raw[1].clone()),
            )
            .expect("val");
            time_us(|| {
                let _ = val_series
                    .groupby(&key_series)
                    .expect("groupby")
                    .skew()
                    .expect("skew");
            })
        }
        ("groupby", "groupby_nunique_str") => {
            let mut kb = Vec::with_capacity(rows * 5);
            let mut ko = Vec::with_capacity(rows + 1);
            ko.push(0usize);
            for &v in raw[0].iter() {
                kb.extend_from_slice(format!("g{:04}", (v as i64).rem_euclid(1000)).as_bytes());
                ko.push(kb.len());
            }
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new(
                "key".to_string(),
                index.clone(),
                Column::from_utf8_contiguous(kb, ko),
            )
            .expect("key");
            let val_series = Series::new(
                "col_1".to_string(),
                index,
                Column::from_f64_values(raw[1].clone()),
            )
            .expect("val");
            time_us(|| {
                let _ = val_series
                    .groupby(&key_series)
                    .expect("groupby")
                    .nunique()
                    .expect("nunique");
            })
        }
        ("groupby", "groupby_all_str") => {
            let mut kb = Vec::with_capacity(rows * 5);
            let mut ko = Vec::with_capacity(rows + 1);
            ko.push(0usize);
            for &v in raw[0].iter() {
                kb.extend_from_slice(format!("g{:04}", (v as i64).rem_euclid(1000)).as_bytes());
                ko.push(kb.len());
            }
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new(
                "key".to_string(),
                index.clone(),
                Column::from_utf8_contiguous(kb, ko),
            )
            .expect("key");
            let val_series = Series::new(
                "col_1".to_string(),
                index,
                Column::from_f64_values(raw[1].clone()),
            )
            .expect("val");
            time_us(|| {
                let _ = val_series
                    .groupby(&key_series)
                    .expect("groupby")
                    .all()
                    .expect("all");
            })
        }
        ("groupby", "groupby_unique_str") => {
            let mut kb = Vec::with_capacity(rows * 5);
            let mut ko = Vec::with_capacity(rows + 1);
            ko.push(0usize);
            for &v in raw[0].iter() {
                kb.extend_from_slice(format!("g{:04}", (v as i64).rem_euclid(1000)).as_bytes());
                ko.push(kb.len());
            }
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new(
                "key".to_string(),
                index.clone(),
                Column::from_utf8_contiguous(kb, ko),
            )
            .expect("key");
            let val_series = Series::new(
                "col_1".to_string(),
                index,
                Column::from_f64_values(raw[1].clone()),
            )
            .expect("val");
            time_us(|| {
                let _ = val_series
                    .groupby(&key_series)
                    .expect("groupby")
                    .unique()
                    .expect("unique");
            })
        }
        ("groupby", "groupby_unique_i64") => {
            // Int64-VALUE variant of groupby_unique_str: str key (1000 groups),
            // i64 value column (col_1 % 50000 -> ~50 distinct per group).
            let mut kb = Vec::with_capacity(rows * 5);
            let mut ko = Vec::with_capacity(rows + 1);
            ko.push(0usize);
            for &v in raw[0].iter() {
                kb.extend_from_slice(format!("g{:04}", (v as i64).rem_euclid(1000)).as_bytes());
                ko.push(kb.len());
            }
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new(
                "key".to_string(),
                index.clone(),
                Column::from_utf8_contiguous(kb, ko),
            )
            .expect("key");
            let ivals: Vec<i64> = raw[1]
                .iter()
                .map(|&v| (v as i64).rem_euclid(50_000))
                .collect();
            let val_series =
                Series::new("col_1".to_string(), index, Column::from_i64_values(ivals))
                    .expect("val");
            time_us(|| {
                let _ = val_series
                    .groupby(&key_series)
                    .expect("groupby")
                    .unique()
                    .expect("unique");
            })
        }
        ("groupby", "groupby_kurt_str") => {
            let mut kb = Vec::with_capacity(rows * 5);
            let mut ko = Vec::with_capacity(rows + 1);
            ko.push(0usize);
            for &v in raw[0].iter() {
                kb.extend_from_slice(format!("g{:04}", (v as i64).rem_euclid(1000)).as_bytes());
                ko.push(kb.len());
            }
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new(
                "key".to_string(),
                index.clone(),
                Column::from_utf8_contiguous(kb, ko),
            )
            .expect("key");
            let val_series = Series::new(
                "col_1".to_string(),
                index,
                Column::from_f64_values(raw[1].clone()),
            )
            .expect("val");
            time_us(|| {
                let _ = val_series
                    .groupby(&key_series)
                    .expect("groupby")
                    .kurt()
                    .expect("kurt");
            })
        }
        ("groupby", "groupby_quantile_str") => {
            let mut kb = Vec::with_capacity(rows * 5);
            let mut ko = Vec::with_capacity(rows + 1);
            ko.push(0usize);
            for &v in raw[0].iter() {
                kb.extend_from_slice(format!("g{:04}", (v as i64).rem_euclid(1000)).as_bytes());
                ko.push(kb.len());
            }
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new(
                "key".to_string(),
                index.clone(),
                Column::from_utf8_contiguous(kb, ko),
            )
            .expect("key");
            let val_series = Series::new(
                "col_1".to_string(),
                index,
                Column::from_f64_values(raw[1].clone()),
            )
            .expect("val");
            time_us(|| {
                let _ = val_series
                    .groupby(&key_series)
                    .expect("groupby")
                    .quantile(0.5)
                    .expect("q");
            })
        }
        ("groupby", "groupby_transform_mean_str") => {
            // String-key variant: s.groupby(str_key).transform("mean"). key =
            // "g{col_0 % 1000:04}" (~1000 distinct categorical labels).
            let mut kb = Vec::with_capacity(rows * 5);
            let mut ko = Vec::with_capacity(rows + 1);
            ko.push(0usize);
            for &v in raw[0].iter() {
                kb.extend_from_slice(format!("g{:04}", (v as i64).rem_euclid(1000)).as_bytes());
                ko.push(kb.len());
            }
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new(
                "key".to_string(),
                index.clone(),
                Column::from_utf8_contiguous(kb, ko),
            )
            .expect("key series");
            let val_series = Series::new(
                "col_1".to_string(),
                index,
                Column::from_f64_values(raw[1].clone()),
            )
            .expect("val series");
            time_us(|| {
                let _ = val_series
                    .groupby(&key_series)
                    .expect("groupby")
                    .transform("mean")
                    .expect("transform");
            })
        }
        ("groupby", "groupby_cumcount") => {
            // pandas: df.groupby("key").cumcount() — within-group 0-based row
            // number. key = (col_0 % 100).astype(int64).
            let keys: Vec<i64> = raw[0].iter().map(|&v| (v as i64).rem_euclid(100)).collect();
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new(
                "key".to_string(),
                index.clone(),
                Column::from_i64_values(keys),
            )
            .expect("key series");
            let val_series = Series::new(
                "col_1".to_string(),
                index,
                Column::from_f64_values(raw[1].clone()),
            )
            .expect("val series");
            time_us(|| {
                let _ = val_series
                    .groupby(&key_series)
                    .expect("groupby")
                    .cumcount()
                    .expect("cumcount");
            })
        }
        ("groupby", "groupby_count") => {
            // pandas: df.groupby("key")["col_1"].count() — non-null count per
            // group. key = (col_0 % 100).astype(int64).
            let keys: Vec<i64> = raw[0].iter().map(|&v| (v as i64).rem_euclid(100)).collect();
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new(
                "key".to_string(),
                index.clone(),
                Column::from_i64_values(keys),
            )
            .expect("key series");
            let val_series = Series::new(
                "col_1".to_string(),
                index,
                Column::from_f64_values(raw[1].clone()),
            )
            .expect("val series");
            time_us(|| {
                let _ = val_series
                    .groupby(&key_series)
                    .expect("groupby")
                    .count()
                    .expect("count");
            })
        }
        ("groupby", "df_groupby_str_sum") => {
            // pandas: df.groupby("key")[["v0","v1","v2"]].sum(); key=g{i%1000}
            // (1000 string groups). Exercises DataFrameGroupBy's GroupMap.
            let mut kb = Vec::new();
            let mut ko = vec![0usize];
            for i in 0..rows {
                let k = format!("g{:04}", i % 1000);
                kb.extend_from_slice(k.as_bytes());
                ko.push(kb.len());
            }
            let mut columns = BTreeMap::new();
            columns.insert("key".to_string(), Column::from_utf8_contiguous(kb, ko));
            let mut order = vec!["key".to_string()];
            for (c, column) in raw.iter().enumerate().take(3) {
                let n = format!("v{c}");
                columns.insert(n.clone(), Column::from_f64_values(column.clone()));
                order.push(n);
            }
            let gdf = DataFrame::new_with_column_order(
                Index::new_known_unique_int64_unit_range(0, rows),
                columns,
                order,
            )
            .expect("gb frame");
            time_us(|| {
                let _ = gdf.groupby(&["key"]).expect("groupby").sum().expect("sum");
            })
        }
        ("groupby", "df_groupby_2key_sum") => {
            let mut columns = BTreeMap::new();
            let k1: Vec<i64> = (0..rows).map(|i| (i % 100) as i64).collect();
            let k2: Vec<i64> = (0..rows).map(|i| ((i / 100) % 50) as i64).collect();
            columns.insert("k1".to_string(), Column::from_i64_values(k1));
            columns.insert("k2".to_string(), Column::from_i64_values(k2));
            let mut order = vec!["k1".to_string(), "k2".to_string()];
            for (c, column) in raw.iter().enumerate().take(3) {
                let n = format!("v{c}");
                columns.insert(n.clone(), Column::from_f64_values(column.clone()));
                order.push(n);
            }
            let gdf = DataFrame::new_with_column_order(
                Index::new_known_unique_int64_unit_range(0, rows),
                columns,
                order,
            )
            .expect("gb frame");
            time_us(|| {
                let _ = gdf
                    .groupby(&["k1", "k2"])
                    .expect("groupby")
                    .sum()
                    .expect("sum");
            })
        }
        ("groupby", "df_groupby_2strkey_sum") => {
            let mut columns = BTreeMap::new();
            let mut kb1 = Vec::new();
            let mut ko1 = vec![0usize];
            for i in 0..rows {
                let k = format!("a{:03}", i % 100);
                kb1.extend_from_slice(k.as_bytes());
                ko1.push(kb1.len());
            }
            columns.insert("k1".to_string(), Column::from_utf8_contiguous(kb1, ko1));
            let mut kb2 = Vec::new();
            let mut ko2 = vec![0usize];
            for i in 0..rows {
                let k = format!("b{:03}", (i / 100) % 50);
                kb2.extend_from_slice(k.as_bytes());
                ko2.push(kb2.len());
            }
            columns.insert("k2".to_string(), Column::from_utf8_contiguous(kb2, ko2));
            let mut order = vec!["k1".to_string(), "k2".to_string()];
            for (c, column) in raw.iter().enumerate().take(3) {
                let n = format!("v{c}");
                columns.insert(n.clone(), Column::from_f64_values(column.clone()));
                order.push(n);
            }
            let gdf = DataFrame::new_with_column_order(
                Index::new_known_unique_int64_unit_range(0, rows),
                columns,
                order,
            )
            .expect("gb frame");
            time_us(|| {
                let _ = gdf
                    .groupby(&["k1", "k2"])
                    .expect("groupby")
                    .sum()
                    .expect("sum");
            })
        }
        ("groupby", "groupby_agg3_str") => {
            let mut kb = Vec::with_capacity(rows * 5);
            let mut ko = Vec::with_capacity(rows + 1);
            ko.push(0usize);
            for &v in raw[0].iter() {
                kb.extend_from_slice(format!("g{:04}", (v as i64).rem_euclid(1000)).as_bytes());
                ko.push(kb.len());
            }
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new(
                "key".to_string(),
                index.clone(),
                Column::from_utf8_contiguous(kb, ko),
            )
            .expect("key");
            let val_series = Series::new(
                "col_1".to_string(),
                index,
                Column::from_f64_values(raw[1].clone()),
            )
            .expect("val");
            time_us(|| {
                let _ = val_series
                    .groupby(&key_series)
                    .expect("groupby")
                    .agg(&["mean", "std", "max"])
                    .expect("agg");
            })
        }
        ("groupby", "groupby_widekey_sum") => {
            // Single WIDE-range i64 key (sparse, ~rows/2 distinct over a huge
            // span): exercises the non-dense build_groups path (dense gate needs
            // a bounded range). key = i * 0x9E3779B97F4A7C15 (wraps -> spread
            // across full i64), value = col_1. groupby(key).sum().
            let key_vals: Vec<i64> = (0..rows as i64)
                .map(|i| (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) as i64 >> 1)
                .collect();
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let key_series = Series::new(
                "key".to_string(),
                index.clone(),
                Column::from_i64_values(key_vals),
            )
            .expect("widekey key");
            let val_series = Series::new(
                "col_1".to_string(),
                index,
                Column::from_f64_values(raw[1].clone()),
            )
            .expect("widekey val");
            time_us(|| {
                let _ = val_series
                    .groupby(&key_series)
                    .expect("groupby")
                    .sum()
                    .expect("sum");
            })
        }
        ("groupby", "df_groupby_widekey_sum") => {
            // DataFrameGroupBy sibling of groupby_widekey_sum: single WIDE-range
            // i64 key col (~rows distinct) + 3 f64 value cols, df.groupby(key).sum().
            let mut columns = BTreeMap::new();
            let key_vals: Vec<i64> = (0..rows as i64)
                .map(|i| (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) as i64 >> 1)
                .collect();
            columns.insert("key".to_string(), Column::from_i64_values(key_vals));
            let mut order = vec!["key".to_string()];
            for (c, column) in raw.iter().enumerate().take(3) {
                let n = format!("v{c}");
                columns.insert(n.clone(), Column::from_f64_values(column.clone()));
                order.push(n);
            }
            let gdf = DataFrame::new_with_column_order(
                Index::new_known_unique_int64_unit_range(0, rows),
                columns,
                order,
            )
            .expect("widekey gb frame");
            time_us(|| {
                let _ = gdf.groupby(&["key"]).expect("groupby").sum().expect("sum");
            })
        }
        ("groupby", "df_groupby_int_var") => {
            let mut columns = BTreeMap::new();
            let key_vals: Vec<i64> = (0..rows).map(|i| (i % 1000) as i64).collect();
            columns.insert("key".to_string(), Column::from_i64_values(key_vals));
            let mut order = vec!["key".to_string()];
            for (c, column) in raw.iter().enumerate().take(3) {
                let n = format!("v{c}");
                columns.insert(n.clone(), Column::from_f64_values(column.clone()));
                order.push(n);
            }
            let gdf = DataFrame::new_with_column_order(
                Index::new_known_unique_int64_unit_range(0, rows),
                columns,
                order,
            )
            .expect("gb frame");
            time_us(|| {
                let _ = gdf.groupby(&["key"]).expect("groupby").var().expect("var");
            })
        }
        ("groupby", "df_groupby_int_mean") => {
            let mut columns = BTreeMap::new();
            let key_vals: Vec<i64> = (0..rows).map(|i| (i % 1000) as i64).collect();
            columns.insert("key".to_string(), Column::from_i64_values(key_vals));
            let mut order = vec!["key".to_string()];
            for (c, column) in raw.iter().enumerate().take(3) {
                let n = format!("v{c}");
                columns.insert(n.clone(), Column::from_f64_values(column.clone()));
                order.push(n);
            }
            let gdf = DataFrame::new_with_column_order(
                Index::new_known_unique_int64_unit_range(0, rows),
                columns,
                order,
            )
            .expect("gb frame");
            time_us(|| {
                let _ = gdf
                    .groupby(&["key"])
                    .expect("groupby")
                    .mean()
                    .expect("mean");
            })
        }
        ("rolling", "rolling_mean_w10") => {
            let series = df.get_column("col_0");
            time_us(|| {
                let _ = series.rolling(10, Some(10)).mean().expect("rolling mean");
            })
        }
        ("rolling", "groupby_rolling_mean_w10") => {
            // br-frankenpandas-u5cg4. SeriesGroupBy grouped rolling had NO lane at
            // all, so the op this bead is about had never been measured against the
            // incumbent — the same gap that left mod/floordiv unmeasurable until
            // e7d87c811 got lanes.
            //
            // The key is derived from the ROW INDEX, not from a value column, on
            // both sides: `i % 100` here and `np.arange(rows) % 100` in the
            // harness. A key derived from a random column would have to reproduce
            // fp-bench's generator exactly to stay like-for-like, and getting that
            // wrong is invisible in the ratio. 100 groups also clears the
            // 64-group parallel threshold, which is the point of the lane.
            let keys: Vec<i64> = (0..rows as i64).map(|i| i % 100).collect();
            let key_series = Series::new(
                "key",
                Index::new_known_unique_int64_unit_range(0, rows),
                Column::from_i64_values(keys),
            )
            .expect("grouped rolling key series");
            let values = df.get_column("col_0");
            time_us(|| {
                let _ = values
                    .groupby(&key_series)
                    .expect("groupby")
                    .rolling(10)
                    .mean()
                    .expect("grouped rolling mean");
            })
        }
        ("rolling", "groupby_expanding_mean") => {
            // br-frankenpandas-vw0uu. SeriesGroupBy grouped EXPANDING had no lane,
            // so `SeriesGroupByExpanding::apply_grouped_expanding` has never been
            // measured — while its rolling sibling has had one since u5cg4. That
            // gap is why the index-clone and pre-size fixes could be verified on
            // rolling and only ASSUMED on expanding, despite both landing in the
            // same two code paths.
            //
            // Deliberately identical to `groupby_rolling_mean_w10` except for the
            // aggregation: same `i % 100` key derived from the ROW INDEX, same 100
            // groups (clearing the 64-group parallel threshold), same value column.
            // Anything else and the two lanes stop being comparable, which is the
            // whole reason to mirror rather than invent one.
            //
            // ⚠️ FP-BENCH-ONLY BY DESIGN — THERE IS NO PANDAS ARM FOR THIS LANE.
            // `benches/vs_pandas_harness.py` has no `groupby_expanding_mean` key
            // (its `df_groupby_expanding_mean` is a different lane), so running
            // this through the harness yields NO incumbent and the row can never
            // certify. That is deliberate, not half-built: adding a pandas arm
            // means editing the harness, which changes `harness_source.sha256`
            // and ORPHANS ALL 49 STANDING LOCKS including floordiv/mod @10M. The
            // question this lane was built to answer — does grouped expanding run
            // its groups in parallel? — is answered by fp-bench's own
            // `thread_provenance` without an incumbent at all, and it answered it:
            // ONE thread against its rolling sibling's twelve.
            //
            // SO WHAT THIS LANE MAY AND MAY NOT CLAIM: FP-side absolute cost,
            // thread counts and before/after self-speedups, yes. Anything
            // vs-pandas, no — not "unmeasured", STRUCTURALLY UNAVAILABLE until a
            // harness batch that is already paying the orphan cost adds the arm.
            // Whoever pays that pass should add it; it is about six lines.
            //
            // ⚠️ EXPANDING IS NOT ROLLING WITH A BIG WINDOW. Each group's window
            // grows from 1 to the group's length, so the per-element work differs
            // from `rolling(10)` and the two lanes' ABSOLUTE times must not be
            // compared to each other — only each against its own before/after.
            let keys: Vec<i64> = (0..rows as i64).map(|i| i % 100).collect();
            let key_series = Series::new(
                "key",
                Index::new_known_unique_int64_unit_range(0, rows),
                Column::from_i64_values(keys),
            )
            .expect("grouped expanding key series");
            let values = df.get_column("col_0");
            time_us(|| {
                let _ = values
                    .groupby(&key_series)
                    .expect("groupby")
                    .expanding(Some(1))
                    .mean()
                    .expect("grouped expanding mean");
            })
        }
        ("rolling", "df_groupby_rolling_mean_w10") => {
            // br-frankenpandas-vw0uu. The DataFrameGroupBy grouped-rolling surface
            // had NO lane, so its "~14-32ms residual" has never been compared to
            // the incumbent — same gap that left SeriesGroupBy rolling unmeasured
            // until `groupby_rolling_mean_w10`.
            //
            // THREE value columns, not one, because this path's parallel arm is
            // per-COLUMN (`GBROLL_PAR_MIN_COLS = 2`, worker_count capped at
            // ncols) rather than per-group like its Series sibling. A one-column
            // fixture would route serial and measure the wrong thing entirely.
            //
            // Key from the row index (`i % 100`) exactly as the Series lane does,
            // so the two lanes group identically and their numbers are
            // comparable.
            let keys: Vec<i64> = (0..rows as i64).map(|i| i % 100).collect();
            let mut columns = BTreeMap::new();
            columns.insert("key".to_string(), Column::from_i64_values(keys));
            let mut order = vec!["key".to_string()];
            for (c, column) in raw.iter().enumerate().take(3) {
                let n = format!("v{c}");
                columns.insert(n.clone(), Column::from_f64_values(column.clone()));
                order.push(n);
            }
            let gdf = DataFrame::new_with_column_order(
                Index::new_known_unique_int64_unit_range(0, rows),
                columns,
                order,
            )
            .expect("df grouped rolling frame");
            time_us(|| {
                let _ = gdf
                    .groupby(&["key"])
                    .expect("groupby")
                    .rolling(10)
                    .mean()
                    .expect("df grouped rolling mean");
            })
        }
        ("rolling", "df_groupby_expanding_mean") | ("rolling", "df_groupby_ewm_mean") => {
            // br-frankenpandas-vw0uu, the two siblings of df_groupby_rolling_mean_w10.
            //
            // Batched into ONE harness landing with each other on purpose: every
            // harness edit moves the whole-file sha and orphans every live lock,
            // including the STANDING floordiv/mod pair, so N separate landings
            // cost N re-lock passes and one landing costs one.
            //
            // vw0uu names GroupByEwm as a remaining engine, but dde7be739 already
            // fixed it in-tree. So the open question is not "is it on the slow
            // path" — it is whether these two, like grouped rolling (7.251x
            // certified), are already AHEAD of the incumbent, which decides
            // whether this bead has a gap left at all.
            //
            // Same fixture as the rolling lane, for the same reasons: THREE value
            // columns because this family parallelises per-COLUMN and a
            // one-column frame routes serial, and a key from the row index so all
            // three lanes group identically.
            let keys: Vec<i64> = (0..rows as i64).map(|i| i % 100).collect();
            let mut columns = BTreeMap::new();
            columns.insert("key".to_string(), Column::from_i64_values(keys));
            let mut order = vec!["key".to_string()];
            for (c, column) in raw.iter().enumerate().take(3) {
                let n = format!("v{c}");
                columns.insert(n.clone(), Column::from_f64_values(column.clone()));
                order.push(n);
            }
            let gdf = DataFrame::new_with_column_order(
                Index::new_known_unique_int64_unit_range(0, rows),
                columns,
                order,
            )
            .expect("df grouped window frame");
            if workload == "df_groupby_expanding_mean" {
                time_us(|| {
                    let _ = gdf
                        .groupby(&["key"])
                        .expect("groupby")
                        .expanding(Some(1))
                        .mean()
                        .expect("df grouped expanding mean");
                })
            } else {
                time_us(|| {
                    let _ = gdf
                        .groupby(&["key"])
                        .expect("groupby")
                        .ewm(Some(10.0), None)
                        .mean()
                        .expect("df grouped ewm mean");
                })
            }
        }
        ("rolling", "expanding_skew") => {
            let series = df.get_column("col_0");
            time_us(|| {
                let _ = series.expanding(Some(1)).skew().expect("expanding skew");
            })
        }
        ("rolling", "rolling_std_w50") => {
            let series = df.get_column("col_0");
            time_us(|| {
                let _ = series.rolling(50, Some(50)).std().expect("rolling std");
            })
        }
        ("rolling", "rolling_apply_stateful") => {
            // pandas task-equivalent winner after an eight-route 1M screen:
            // rolling(10).sum() followed by an ordered stateful callback. The
            // callback output depends on every earlier valid window.
            let series = Series::new(
                "s",
                Index::new_known_unique_int64_unit_range(0, rows),
                Column::from_f64_values_owned((0..rows).map(|row| (row % 997) as f64).collect()),
            )
            .expect("stateful rolling series");
            time_us(|| {
                let state = Cell::new(0_i64);
                let result = series
                    .rolling(10, Some(10))
                    .apply(|values| stateful_rolling_step(&state, values))
                    .expect("stateful rolling apply");
                (result, state.get())
            })
        }
        ("rolling", "expanding_sum") => {
            let series = df.get_column("col_0");
            time_us(|| {
                let _ = series.expanding(Some(1)).sum().expect("expanding sum");
            })
        }
        ("rolling", "expanding_apply_stateful") => {
            // pandas task-equivalent winner after an eight-route 1M screen:
            // np.fromiter over the same ordered recurrence. Prefix length,
            // newest value, and all prior callback states remain observable.
            let series = Series::new(
                "s",
                Index::new_known_unique_int64_unit_range(0, rows),
                Column::from_f64_values_owned((0..rows).map(|row| (row % 997) as f64).collect()),
            )
            .expect("stateful expanding series");
            time_us(|| {
                let state = Cell::new(0_i64);
                let result = series
                    .expanding(Some(1))
                    .apply(|values| stateful_expanding_step(&state, values))
                    .expect("stateful expanding apply");
                (result, state.get())
            })
        }
        ("rolling", "ewm_mean") => {
            let series = df.get_column("col_0");
            time_us(|| {
                let _ = series.ewm(Some(10.0), None).mean().expect("ewm mean");
            })
        }
        // The fp-bench frame uses a default 0..rows Int64 index (matching the
        // pandas side's set_index(range(n))), so loc/reindex labels line up.
        ("indexing", "iloc_slice") => {
            // pandas: df.iloc[n/4 : 3n/4] — a contiguous SLICE (returns a view).
            // Match the slice semantics with DataFrame::iloc_slice(start, stop)
            // (the O(1) lazy-slice path), NOT df.iloc(&positions): passing an
            // explicit 50k-element position Vec measures pandas' *list* indexer
            // (df.iloc[list(...)] is ~200x slower than the slice) against fp's
            // list API — an apples-to-oranges comparison. iloc_slice is the
            // same contiguous-range operation pandas' slice performs.
            let start = Some((rows / 4) as i64);
            let stop = Some((3 * rows / 4) as i64);
            time_us(|| {
                let _ = df.iloc_slice(start, stop).expect("iloc_slice");
            })
        }
        ("indexing", "loc_labels") => {
            // pandas: df.loc[list(range(n/4, 3n/4))]
            let labels: Vec<IndexLabel> = ((rows / 4) as i64..(3 * rows / 4) as i64)
                .map(IndexLabel::Int64)
                .collect();
            time_us(|| {
                let _ = df.loc(&labels).expect("loc");
            })
        }
        ("io", "csv_read") => {
            // pandas: df.to_csv(file, index=False) [setup]; time pd.read_csv(file).
            // FP: serialize once (setup), time read_csv_str of the same text.
            let csv = fp_io::write_csv_string(&df).expect("csv serialize");
            time_us(|| {
                let _ = fp_io::read_csv_str(&csv).expect("read_csv");
            })
        }
        #[cfg(feature = "block-storage")]
        ("io", "csv_read_block_view") => {
            // pandas: pd.read_csv(...).to_numpy(copy=False). The Float64 CSV
            // parser is deliberately inside the timed closure; each A/A arm
            // therefore proves a fresh frame becomes block-backed before its
            // first array observation. CSV serialization remains setup.
            let csv = fp_io::write_csv_string(&df).expect("csv serialize");
            time_us(|| {
                let frame = fp_io::read_csv_str(&csv).expect("read_csv");
                let view = frame
                    .to_numpy_block_view()
                    .expect("homogeneous Float64 CSV must be block-backed");
                black_box((view.rows, view.cols, view.block));
            })
        }
        #[cfg(not(feature = "block-storage"))]
        ("io", "csv_read_block_view") => {
            panic!("csv_read_block_view requires fp-bench --features block-storage")
        }
        ("io", "csv_write") => {
            // pandas: time df.to_csv(file, index=False). FP: time write_csv_string.
            time_us(|| {
                let _ = fp_io::write_csv_string(&df).expect("write_csv");
            })
        }
        ("io", "parquet_read") => {
            // pandas: df.to_parquet(file, index=False) [setup]; time
            // pd.read_parquet(file). FP: serialize once (setup), time
            // read_parquet_bytes over the same Arrow bytes.
            let bytes = fp_io::write_parquet_bytes(&df).expect("parquet serialize");
            time_us(|| {
                let _ = fp_io::read_parquet_bytes(&bytes).expect("read_parquet");
            })
        }
        ("io", "parquet_write") => {
            // pandas: time df.to_parquet(file, index=False). FP: time
            // write_parquet_bytes.
            time_us(|| {
                let _ = fp_io::write_parquet_bytes(&df).expect("write_parquet");
            })
        }
        ("indexing", "reindex") => {
            // pandas: df.reindex(Index(range(0, n*2, 2)))
            let new_labels: Vec<IndexLabel> = (0..(rows * 2) as i64)
                .step_by(2)
                .map(IndexLabel::Int64)
                .collect();
            time_us(|| {
                let _ = df.reindex(new_labels.clone()).expect("reindex");
            })
        }
        ("indexing", "range_index_take_arithmetic") => {
            // pandas: pd.RangeIndex(10, 10 + 3*n, 3).take(arithmetic_positions).
            // Batch repeated takes inside the timed unit so FP's lazy affine
            // output path stays above timer noise without charging setup.
            let range = RangeIndex::new(10, 10 + rows as i64 * 3, 3).expect("range index fixture");
            let positions = arithmetic_take_positions(rows);
            time_us(|| {
                for _ in 0..TAKE_BATCH {
                    black_box(
                        range
                            .take(black_box(positions.as_slice()))
                            .expect("range take"),
                    );
                }
            })
        }
        ("indexing", "affine_index_take_arithmetic") => {
            // pandas: pd.Index(np.arange(10, 10 + 3*n, 3)).take(arithmetic_positions).
            let index =
                Index::new_known_unique_int64_affine_range(10, 3, rows).expect("affine index");
            let positions = arithmetic_take_positions(rows);
            time_us(|| {
                for _ in 0..TAKE_BATCH {
                    black_box(index.take(black_box(positions.as_slice())));
                }
            })
        }
        // pandas: left.merge(right, on="key", how=inner|left|outer). The frame
        // built above is unused for joins; build the two merge inputs sized to
        // `rows` (outside the timed window) instead.
        ("joins", "join_inner" | "join_left" | "join_outer") => {
            let (left, right) = build_join_frames(rows);
            let join_type = match workload {
                "join_inner" => JoinType::Inner,
                "join_left" => JoinType::Left,
                _ => JoinType::Outer,
            };
            time_us(|| {
                let _ = merge_dataframes_on_with(&left, &right, &["key"], &["key"], join_type)
                    .expect("merge");
            })
        }
        ("joins", "join_inner_shuffled") => {
            // Inner merge on SHUFFLED (non-monotonic) i64 keys: the sequential
            // build_join_frames keys hit an ordered/dense fast path; shuffling
            // both key columns (LCG permutation) forces the hash-join path.
            let mut lk: Vec<i64> = (0..rows as i64).collect();
            let mut rk: Vec<i64> = (0..rows as i64).map(|i| i * 2).collect();
            let mut st: u64 = 0x243F_6A88_85A3_08D3;
            let mut shuffle = |v: &mut Vec<i64>| {
                for i in (1..v.len()).rev() {
                    st = st
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add(1442695040888963407);
                    let j = (st >> 33) as usize % (i + 1);
                    v.swap(i, j);
                }
            };
            shuffle(&mut lk);
            shuffle(&mut rk);
            let mut left_cols = BTreeMap::new();
            left_cols.insert("key".to_string(), Column::from_i64_values(lk));
            left_cols.insert(
                "left_val".to_string(),
                Column::from_f64_values((0..rows).map(|i| i as f64).collect()),
            );
            let left = DataFrame::new_with_column_order(
                Index::new_known_unique_int64_unit_range(0, rows),
                left_cols,
                vec!["key".to_string(), "left_val".to_string()],
            )
            .expect("shuffled left");
            let mut right_cols = BTreeMap::new();
            right_cols.insert("key".to_string(), Column::from_i64_values(rk));
            right_cols.insert(
                "right_val".to_string(),
                Column::from_f64_values((0..rows).map(|i| i as f64 * 10.0).collect()),
            );
            let right = DataFrame::new_with_column_order(
                Index::new_known_unique_int64_unit_range(0, rows),
                right_cols,
                vec!["key".to_string(), "right_val".to_string()],
            )
            .expect("shuffled right");
            time_us(|| {
                let _ =
                    merge_dataframes_on_with(&left, &right, &["key"], &["key"], JoinType::Inner)
                        .expect("merge");
            })
        }
        ("joins", "join_inner_str") => {
            // String-key inner merge: left key = "k{i:08}" (unique), right key =
            // "k{2i:08}" — mirrors the int64 join shape (~n/2 inner matches) but
            // exercises the Utf8 key path. pandas: left.merge(right, on="key").
            let build = |stride: usize, valname: &str| -> DataFrame {
                let mut bytes = Vec::with_capacity(rows * 9);
                let mut off = Vec::with_capacity(rows + 1);
                off.push(0usize);
                for i in 0..rows {
                    bytes.extend_from_slice(format!("k{:08}", i * stride).as_bytes());
                    off.push(bytes.len());
                }
                let index = Index::new_known_unique_int64_unit_range(0, rows);
                let mut cols = BTreeMap::new();
                cols.insert("key".to_string(), Column::from_utf8_contiguous(bytes, off));
                cols.insert(
                    valname.to_string(),
                    Column::from_f64_values((0..rows).map(|i| i as f64).collect()),
                );
                DataFrame::new_with_column_order(
                    index,
                    cols,
                    vec!["key".to_string(), valname.to_string()],
                )
                .expect("fp-bench str join frame")
            };
            let left = build(1, "left_val");
            let right = build(2, "right_val");
            time_us(|| {
                let _ =
                    merge_dataframes_on_with(&left, &right, &["key"], &["key"], JoinType::Inner)
                        .expect("merge");
            })
        }
        // String-column ops (the rest of the matrix is numeric-only). pandas:
        // f.sort_values("name") / f["key"].value_counts() / f.groupby("key")
        // ["val"].sum(). The numeric `df` built above is unused here.
        (
            "strings",
            "str_len"
            | "str_upper"
            | "str_contains"
            | "str_contains_arrow"
            | "str_startswith"
            | "str_startswith_arrow",
        ) => {
            let frame = build_str_frame(rows);
            let series = frame.get_column("name");
            let base_workload = workload.strip_suffix("_arrow").unwrap_or(workload);
            match base_workload {
                "str_len" => time_us(|| {
                    let _ = series.str().len().expect("str len");
                }),
                "str_upper" => time_us(|| {
                    let _ = series.str().upper().expect("str upper");
                }),
                "str_contains" => time_us(|| {
                    let _ = series.str().contains("5").expect("str contains");
                }),
                _ => time_us(|| {
                    let _ = series.str().startswith("item").expect("str startswith");
                }),
            }
        }
        // apply_str-backed transforms (zfill/pad/repeat): output Utf8 columns.
        ("strings", "str_zfill" | "str_pad" | "str_repeat") => {
            let frame = build_str_frame(rows);
            let series = frame.get_column("name");
            match workload {
                "str_zfill" => time_us(|| {
                    let _ = series.str().zfill(20).expect("str zfill");
                }),
                "str_pad" => time_us(|| {
                    let _ = series.str().pad(20, "left", ' ').expect("str pad");
                }),
                _ => time_us(|| {
                    let _ = series.str().repeat(2).expect("str repeat");
                }),
            }
        }
        (
            "strings",
            "str_sort"
            | "str_sort_object"
            | "str_sort_arrow"
            | "str_value_counts"
            | "str_value_counts_object"
            | "str_value_counts_arrow"
            | "str_groupby_sum"
            | "str_groupby_sum_object"
            | "str_groupby_sum_arrow",
        ) => {
            let frame = build_str_frame(rows);
            let base_workload = workload
                .strip_suffix("_object")
                .or_else(|| workload.strip_suffix("_arrow"))
                .unwrap_or(workload);
            match base_workload {
                "str_sort" => time_us(|| {
                    let _ = frame.sort_values("name", true).expect("str sort");
                }),
                "str_value_counts" => {
                    let series = frame.get_column("key");
                    time_us(|| {
                        let _ = series.value_counts().expect("str value_counts");
                    })
                }
                _ => {
                    // pandas: f.groupby("key")["val"].sum() — sums ONLY val.
                    // Drop the unrelated "name" column (a ~1M-unique string)
                    // first so fp's groupby(key).sum() likewise aggregates only
                    // val, instead of also concatenating the string column per
                    // group (which made this workload look ~2x slower).
                    let gframe = frame.drop_columns(&["name"]).expect("drop name");
                    time_us(|| {
                        let _ = gframe
                            .groupby(&["key"])
                            .expect("groupby")
                            .sum()
                            .expect("sum");
                    })
                }
            }
        }
        // df.dot GEMM (br no-gaps flagship): square (dim x dim).(dim x dim) where
        // THE GEMM KERNEL ALONE, WITH ALL CONSTRUCTION HOISTED OUT OF THE TIMED
        // REGION (br-frankenpandas-03fp5).
        //
        // `df_dot` measures construction + GEMM together, and three successive
        // attempts to split them by curve-fitting the total were each refuted by
        // the next: every fit has to assume one arithmetic rate across dim, and
        // the rate implied by the totals moves 253x from dim=10 to dim=1414. Two
        // unknowns move together and the total is one observable, so the split
        // is not recoverable from `df_dot` timings at all.
        //
        // This lane measures r(dim) DIRECTLY. It builds the same square frame,
        // then the same `Float64DotAPanel` and the same `Arc<[f64]>` B columns
        // that `DataFrame::dot` builds — all OUTSIDE `time_us` — and times only
        // the n `Column::dot_column_data` calls. Those are the identical public
        // kernel entry points `dot` itself dispatches to the worker pool, so this
        // is not a shadow reimplementation: the arithmetic, the operand layout
        // and the `j = 0..k` order are the same code.
        //
        // Deliberately SERIAL, one thread. The question is per-thread kernel
        // efficiency versus dim, not how well the pool scales, and a serial lane
        // answers it without the pool's dispatch confounding the small sizes.
        // That also means this number must NOT be subtracted from a parallel
        // `df_dot` total to get "construction" — it is a throughput curve, not a
        // term in that sum.
        ("linalg", "df_dot_kernel") => {
            let dim = (rows as f64).sqrt() as usize;
            let frame = build_square_f64_frame(dim);
            let mut a_views: Vec<(std::sync::Arc<[f64]>, usize)> = Vec::with_capacity(dim);
            let mut b_cols: Vec<std::sync::Arc<[f64]>> = Vec::with_capacity(dim);
            for name in frame.column_names() {
                let col = frame.column(name.as_str()).expect("col");
                let values: Vec<f64> = col
                    .values()
                    .iter()
                    .map(|v| match v {
                        Scalar::Float64(f) => *f,
                        other => panic!("square frame must be Float64, got {other:?}"),
                    })
                    .collect();
                let arc: std::sync::Arc<[f64]> = std::sync::Arc::from(values);
                a_views.push((std::sync::Arc::clone(&arc), 0));
                b_cols.push(arc);
            }
            let panel = fp_columnar::Float64DotAPanel::new(a_views, dim);
            time_us(|| {
                let mut acc = 0usize;
                for b in &b_cols {
                    let out = Column::dot_column_data(&panel, b, dim);
                    acc += out.len();
                }
                black_box(acc);
            })
        }
        // dim = isqrt(rows). pandas df.dot delegates to numpy/OpenBLAS; fp uses
        // its own safe-Rust kernel.
        ("linalg", "df_dot") => {
            let dim = (rows as f64).sqrt() as usize;
            let frame = build_square_f64_frame(dim);
            // The dot result is a lazy plan; a real consumer reads it, so
            // materialize every output column's values to measure the true
            // construction + GEMM cost (mirrors pandas' eager m.dot(m)) rather
            // than the lazy-plan shell the drop-only form measured.
            time_us(|| {
                let result = frame.dot(&frame).expect("dot");
                let mut acc = 0usize;
                for name in result.column_names() {
                    acc += result.column(name.as_str()).expect("col").values().len();
                }
                black_box((&result, acc));
            })
        }
        // to_datetime parse throughput: `rows` ISO date strings (2020-01-DD,
        // ~28 distinct), same strings on both engines. pandas: pd.to_datetime(s).
        ("datetime", "to_datetime") => {
            let mut date_bytes: Vec<u8> = Vec::new();
            let mut date_off: Vec<usize> = Vec::with_capacity(rows + 1);
            date_off.push(0);
            for i in 0..rows {
                let s = format!("2020-01-{:02}", i % 28 + 1);
                date_bytes.extend_from_slice(s.as_bytes());
                date_off.push(date_bytes.len());
            }
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let series = Series::new(
                "d".to_string(),
                index,
                Column::from_utf8_contiguous(date_bytes, date_off),
            )
            .expect("date series");
            time_us(|| {
                let _ = to_datetime(&series).expect("to_datetime");
            })
        }
        ("datetime", "resample_mean") => {
            // s.resample("M").mean(): `rows` daily points from 2000-01-01,
            // datetime index -> ~rows/30 month buckets.
            let base: i64 = 946_684_800_000_000_000;
            // hourly so 1M points stay within datetime64[ns] range (<=2262).
            let nanos: Vec<i64> = (0..rows as i64)
                .map(|i| base + i * 3_600_000_000_000)
                .collect();
            let vals = Column::from_f64_values((0..rows).map(|i| i as f64).collect());
            let series =
                Series::new("s", Index::from_datetime64(nanos), vals).expect("resample series");
            time_us(|| {
                let _ = series.resample("M").mean().expect("resample mean");
            })
        }
        ("datetime", "resample_hourly") => {
            // s.resample("h").mean(): `rows` minutely points -> hourly bins
            // (60 rows/bin), exercises the sub-daily ns-bucketing path.
            let base: i64 = 946_684_800_000_000_000;
            let nanos: Vec<i64> = (0..rows as i64)
                .map(|i| base + i * 60_000_000_000)
                .collect();
            let vals = Column::from_f64_values((0..rows).map(|i| i as f64).collect());
            let series =
                Series::new("s", Index::from_datetime64(nanos), vals).expect("resample series");
            time_us(|| {
                let _ = series.resample("h").mean().expect("resample hourly");
            })
        }
        ("datetime", "resample_std") => {
            let base: i64 = 946_684_800_000_000_000;
            let nanos: Vec<i64> = (0..rows as i64)
                .map(|i| base + i * 3_600_000_000_000)
                .collect();
            let vals = Column::from_f64_values((0..rows).map(|i| i as f64).collect());
            let series =
                Series::new("s", Index::from_datetime64(nanos), vals).expect("resample series");
            time_us(|| {
                let _ = series.resample("M").std().expect("resample std");
            })
        }
        ("datetime", "resample_median") => {
            let base: i64 = 946_684_800_000_000_000;
            let nanos: Vec<i64> = (0..rows as i64)
                .map(|i| base + i * 3_600_000_000_000)
                .collect();
            let vals = Column::from_f64_values((0..rows).map(|i| i as f64).collect());
            let series =
                Series::new("s", Index::from_datetime64(nanos), vals).expect("resample series");
            time_us(|| {
                let _ = series.resample("M").median().expect("resample median");
            })
        }
        ("datetime", "resample_agg3") => {
            let base: i64 = 946_684_800_000_000_000;
            let nanos: Vec<i64> = (0..rows as i64)
                .map(|i| base + i * 3_600_000_000_000)
                .collect();
            let vals = Column::from_f64_values((0..rows).map(|i| i as f64).collect());
            let series =
                Series::new("s", Index::from_datetime64(nanos), vals).expect("resample series");
            time_us(|| {
                let _ = series
                    .resample("M")
                    .agg(&["mean", "std", "max"])
                    .expect("resample agg");
            })
        }
        ("datetime", "resample_sum") => {
            let base: i64 = 946_684_800_000_000_000;
            let nanos: Vec<i64> = (0..rows as i64)
                .map(|i| base + i * 3_600_000_000_000)
                .collect();
            let vals = Column::from_f64_values((0..rows).map(|i| i as f64).collect());
            let series =
                Series::new("s", Index::from_datetime64(nanos), vals).expect("resample series");
            time_us(|| {
                let _ = series.resample("M").sum().expect("resample sum");
            })
        }
        ("datetime", "resample_max") => {
            let base: i64 = 946_684_800_000_000_000;
            let nanos: Vec<i64> = (0..rows as i64)
                .map(|i| base + i * 3_600_000_000_000)
                .collect();
            let vals = Column::from_f64_values((0..rows).map(|i| i as f64).collect());
            let series =
                Series::new("s", Index::from_datetime64(nanos), vals).expect("resample series");
            time_us(|| {
                let _ = series.resample("M").max().expect("resample max");
            })
        }
        ("dataframe_ops", "qcut_bins") => {
            // pandas: pd.qcut(s, 10) — quantile-bin a Float64 series into 10 bins.
            let series = df.get_column("col_0");
            time_us(|| {
                let _ = fp_frame::qcut(&series, 10).expect("qcut");
            })
        }
        ("dataframe_ops", "wide_to_long") => {
            // pandas: pd.wide_to_long(df, ["A","B"], i="id", j="year", sep="_",
            // suffix=r"\\d+"); m=rows/2 rows x 2 suffixes -> ~rows long rows.
            let m = (rows / 2).max(1);
            let mut columns = BTreeMap::new();
            columns.insert(
                "id".to_string(),
                Column::from_i64_values((0..m as i64).collect()),
            );
            let mut order = vec!["id".to_string()];
            for stub in ["A", "B"] {
                for suf in ["2000", "2001"] {
                    let name = format!("{stub}_{suf}");
                    columns.insert(
                        name.clone(),
                        Column::from_f64_values((0..m).map(|i| i as f64).collect()),
                    );
                    order.push(name);
                }
            }
            let wdf = DataFrame::new_with_column_order(
                Index::new_known_unique_int64_unit_range(0, m),
                columns,
                order,
            )
            .expect("w2l frame");
            time_us(|| {
                let _ = fp_frame::wide_to_long(&wdf, &["A", "B"], &["id"], "year", "_", r"\d+")
                    .expect("wide_to_long");
            })
        }
        ("dataframe_ops", "str_split_expand") => {
            // pandas: s.str.split(",", expand=True); each cell "a{i},b{i},c{i}".
            let mut bytes = Vec::new();
            let mut off = vec![0usize];
            for i in 0..rows {
                let cell = format!("a{},b{},c{}", i % 97, i % 89, i % 83);
                bytes.extend_from_slice(cell.as_bytes());
                off.push(bytes.len());
            }
            let series = Series::new(
                "s",
                Index::new_known_unique_int64_unit_range(0, rows),
                Column::from_utf8_contiguous(bytes, off),
            )
            .expect("split series");
            time_us(|| {
                let _ = series.str().split_df_n(",", None).expect("split");
            })
        }
        ("io", "json_read_records") => {
            // pandas: pd.read_json(json, orient="records"); parse a records JSON.
            let json = df.to_json("records").expect("to_json setup");
            time_us(|| {
                let _ = fp_io::read_json_str(&json, fp_io::JsonOrient::Records).expect("read_json");
            })
        }
        ("io", "json_read_columns") => {
            // pandas: pd.read_json(json, orient="columns"); parse a column-map JSON.
            let json = df.to_json("columns").expect("to_json setup");
            time_us(|| {
                let _ = fp_io::read_json_str(&json, fp_io::JsonOrient::Columns).expect("read_json");
            })
        }
        ("io", "json_read_index") => {
            // pandas: pd.read_json(json, orient="index"); parse an index-map JSON.
            let json = df.to_json("index").expect("to_json setup");
            time_us(|| {
                let _ = fp_io::read_json_str(&json, fp_io::JsonOrient::Index).expect("read_json");
            })
        }
        ("io", "json_read_split") => {
            // pandas: pd.read_json(json, orient="split"); parse split JSON.
            let json = df.to_json("split").expect("to_json setup");
            time_us(|| {
                let _ = fp_io::read_json_str(&json, fp_io::JsonOrient::Split).expect("read_json");
            })
        }
        ("io", "json_read_values") => {
            // pandas: pd.read_json(json, orient="values"); parse row-array JSON.
            let json = df.to_json("values").expect("to_json setup");
            time_us(|| {
                let _ = fp_io::read_json_str(&json, fp_io::JsonOrient::Values).expect("read_json");
            })
        }
        ("io", "json_write_records") => {
            // pandas: df.to_json(orient="records"); 10-col f64 frame.
            time_us(|| {
                let _ = df.to_json("records").expect("to_json");
            })
        }
        ("io", "json_write_columns") => time_us(|| {
            let _ = df.to_json("columns").expect("to_json");
        }),
        ("io", "json_write_index") => time_us(|| {
            let _ = df.to_json("index").expect("to_json");
        }),
        ("io", "json_write_split") => time_us(|| {
            let _ = df.to_json("split").expect("to_json");
        }),
        ("io", "json_write_values") => time_us(|| {
            let _ = df.to_json("values").expect("to_json");
        }),
        ("dataframe_ops", "df_to_dict_records") => time_us(|| {
            let _ = df.to_dict("records").expect("to_dict");
        }),
        ("dataframe_ops", "df_to_dict_dict") => time_us(|| {
            let _ = df.to_dict("dict").expect("to_dict");
        }),
        ("dataframe_ops", "df_to_dict_index") => time_us(|| {
            let _ = df.to_dict("index").expect("to_dict");
        }),
        // BENCHMARK-INTEGRITY SIBLING: `df_to_dict_index` above drops the
        // IndexMappingLazy result without calling `as_mapping()`, so it
        // measures ONLY construction of the lazy shell (cf. `df_transpose`
        // vs `df_transpose_materialize`). This row forces the
        // materialization boundary a real consumer pays.
        ("dataframe_ops", "df_to_dict_index_materialize") => time_us(|| {
            let result = df.to_dict("index").expect("to_dict");
            let (keys, values) = result.as_mapping().expect("index mapping");
            black_box(keys.len() + values.len());
        }),
        ("dataframe_ops", "df_to_records") => time_us(|| {
            let _ = df.to_records();
        }),
        ("dataframe_ops", "df_apply_row") => time_us(|| {
            let _ = df
                .apply_fn(
                    |row| {
                        let s: f64 = row
                            .iter()
                            .filter_map(|v| match v {
                                fp_types::Scalar::Float64(f) => Some(*f),
                                _ => None,
                            })
                            .sum();
                        fp_types::Scalar::Float64(s)
                    },
                    1,
                )
                .expect("apply");
        }),
        ("dataframe_ops", "series_apply_stateful") => {
            // pandas: Series(range(n)).apply(stateful_step). The callback is an
            // ordered recurrence, so each call affects all later outputs.
            // Population stays outside the timed closure on both engines.
            let series = Series::new(
                "s",
                Index::new_known_unique_int64_unit_range(0, rows),
                Column::from_i64_values((0..rows as i64).collect()),
            )
            .expect("stateful apply series");
            time_us(|| {
                let state = Cell::new(0_i64);
                let result = series
                    .apply(|value| stateful_apply_step(&state, value))
                    .expect("stateful series apply");
                (result, state.get())
            })
        }
        ("dataframe_ops", "cut_explicit") => {
            // pandas: pd.cut(s, bins=[-1,1e5,...,1.1e6]) — explicit edges spanning
            // the [0,1e6] data (all in-range -> all-valid). Exercises cut_bins.
            let series = df.get_column("col_0");
            let mut edges: Vec<fp_types::Scalar> = vec![fp_types::Scalar::Float64(-1.0)];
            for i in 1..=9 {
                edges.push(fp_types::Scalar::Float64(i as f64 * 1e5));
            }
            edges.push(fp_types::Scalar::Float64(1.1e6));
            time_us(|| {
                let _ = fp_frame::cut_bins(&series, &edges, true, None, false).expect("cut_bins");
            })
        }
        ("dataframe_ops", "cut_bins") => {
            // pandas: pd.cut(s, 10) — bin a Float64 series into 10 bins.
            let series = df.get_column("col_0");
            time_us(|| {
                let _ = fp_frame::cut(&series, 10).expect("cut");
            })
        }
        ("datetime", "resample_daily") => {
            // s.resample("D").mean(): `rows` hourly points -> daily bins
            // (24 rows/bin), exercises the daily-contiguous bucketing path.
            let base: i64 = 946_684_800_000_000_000;
            let nanos: Vec<i64> = (0..rows as i64)
                .map(|i| base + i * 3_600_000_000_000)
                .collect();
            let vals = Column::from_f64_values((0..rows).map(|i| i as f64).collect());
            let series =
                Series::new("s", Index::from_datetime64(nanos), vals).expect("resample series");
            time_us(|| {
                let _ = series.resample("D").mean().expect("resample daily");
            })
        }
        (
            "datetime",
            "resample_2d" | "resample_bday" | "resample_w" | "resample_q" | "resample_y",
        ) => {
            // hourly points -> 2D / B / W / Q / Y bins.
            let base: i64 = 946_684_800_000_000_000;
            let nanos: Vec<i64> = (0..rows as i64)
                .map(|i| base + i * 3_600_000_000_000)
                .collect();
            let vals = Column::from_f64_values((0..rows).map(|i| i as f64).collect());
            let series =
                Series::new("s", Index::from_datetime64(nanos), vals).expect("resample series");
            let freq = match workload {
                "resample_2d" => "2D",
                "resample_bday" => "B",
                "resample_w" => "W",
                "resample_q" => "Q",
                _ => "Y",
            };
            time_us(|| {
                let _ = series.resample(freq).mean().expect("resample");
            })
        }
        // dt.floor("D") over `rows` Datetime64 nanos at 37s intervals from
        // 2000-01-01. pandas: s.dt.floor("D").
        ("datetime", "dt_floor") => {
            let base: i64 = 946_684_800_000_000_000; // 2000-01-01 00:00:00 UTC, ns
            let nanos: Vec<i64> = (0..rows as i64)
                .map(|i| base + i * 37_000_000_000)
                .collect();
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let series = Series::new(
                "d".to_string(),
                index,
                Column::from_datetime64_values(nanos),
            )
            .expect("dt series");
            time_us(|| {
                let _ = series.dt().floor("D").expect("dt floor");
            })
        }
        ("datetime", "dt_dayofyear") => {
            let base: i64 = 946_684_800_000_000_000;
            let nanos: Vec<i64> = (0..rows as i64)
                .map(|i| base + i * 37_000_000_000)
                .collect();
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let series = Series::new(
                "d".to_string(),
                index,
                Column::from_datetime64_values(nanos),
            )
            .expect("dt series");
            time_us(|| {
                let _ = series.dt().dayofyear().expect("dt dayofyear");
            })
        }
        ("datetime", "dt_strftime" | "dt_date" | "dt_time" | "dt_day_name" | "dt_month_name") => {
            let base: i64 = 946_684_800_000_000_000;
            // Ten minutes per row makes both the date and time-of-day vary,
            // remains inside datetime64[ns] through 10M rows, and exactly
            // matches the live pandas incumbent arms.
            let nanos: Vec<i64> = (0..rows as i64)
                .map(|i| base + i * 600_000_000_000)
                .collect();
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let series = Series::new(
                "d".to_string(),
                index,
                Column::from_datetime64_values(nanos),
            )
            .expect("dt series");
            match workload {
                "dt_strftime" => time_us(|| {
                    let _ = series.dt().strftime("%Y-%m-%d").expect("strftime");
                }),
                "dt_time" => time_us(|| {
                    let _ = series.dt().time().expect("time");
                }),
                "dt_day_name" => time_us(|| {
                    let _ = series.dt().day_name().expect("day_name");
                }),
                "dt_month_name" => time_us(|| {
                    let _ = series.dt().month_name().expect("month_name");
                }),
                _ => time_us(|| {
                    let _ = series.dt().date().expect("date");
                }),
            }
        }
        ("datetime", "dt_hour" | "dt_minute" | "dt_quarter") => {
            let base: i64 = 946_684_800_000_000_000;
            let nanos: Vec<i64> = (0..rows as i64)
                .map(|i| base + i * 37_000_000_000)
                .collect();
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let series = Series::new(
                "d".to_string(),
                index,
                Column::from_datetime64_values(nanos),
            )
            .expect("dt series");
            match workload {
                "dt_hour" => time_us(|| {
                    let _ = series.dt().hour().expect("dt hour");
                }),
                "dt_minute" => time_us(|| {
                    let _ = series.dt().minute().expect("dt minute");
                }),
                _ => time_us(|| {
                    let _ = series.dt().quarter().expect("dt quarter");
                }),
            }
        }
        ("datetime", "dt_year" | "dt_month" | "dt_dayofweek") => {
            let base: i64 = 946_684_800_000_000_000;
            let nanos: Vec<i64> = (0..rows as i64)
                .map(|i| base + i * 37_000_000_000)
                .collect();
            let index = Index::new_known_unique_int64_unit_range(0, rows);
            let series = Series::new(
                "d".to_string(),
                index,
                Column::from_datetime64_values(nanos),
            )
            .expect("dt series");
            match workload {
                "dt_year" => time_us(|| {
                    let _ = series.dt().year().expect("dt year");
                }),
                "dt_month" => time_us(|| {
                    let _ = series.dt().month().expect("dt month");
                }),
                _ => time_us(|| {
                    let _ = series.dt().dayofweek().expect("dt dayofweek");
                }),
            }
        }
        _ => return None,
    };
    Some(times)
}

fn main() {
    println!("bench_elf_sha256={}", self_identity());

    let args: Vec<String> = std::env::args().collect();
    if let Some(status) = run_remote_python_harness(&args) {
        std::process::exit(status);
    }
    let category = arg(&args, "--category").unwrap_or("dataframe_ops");
    let workload = arg(&args, "--workload").unwrap_or("sort_single");
    let size = arg(&args, "--size").unwrap_or("100k");
    let dtype = arg(&args, "--dtype").unwrap_or("float64");
    // Only the pipeline category consumes this: the driver materializes the
    // job's input CSVs there so both engines read byte-identical inputs.
    let data_dir = arg(&args, "--data-dir").map(Path::new);

    // br-frankenpandas-284ul. FP-vs-FP, not vs-pandas: this lane picks the
    // elementwise worker cap and parallel threshold, and it prints its own
    // schema rather than the `times_us` the Python driver parses.
    // br-frankenpandas-u5cg4. FP-vs-FP: serial against group-parallel grouped
    // rolling, both arms in ONE process, with the worker count OBSERVED per arm.
    // Prints its own schema, not the `times_us` the Python driver parses.
    if category == "sgb_rolling_policy" {
        let (rows, _cols) = size_rows_cols(size);
        // Groups is an axis (br-frankenpandas-u5cg4); default matches the
        // vs-incumbent lane's cardinality so the two stay comparable.
        let groups: i64 = arg(&args, "--groups")
            .and_then(|g| g.parse().ok())
            .filter(|g| *g >= 1)
            .unwrap_or(100);
        if !run_sgb_rolling_policy_sweep(workload, rows, groups) {
            eprintln!("fp-bench: unsupported sgb_rolling_policy/{workload} (mean)");
            std::process::exit(2);
        }
        return;
    }

    if category == "elementwise_policy" {
        let (rows, _cols) = size_rows_cols(size);
        let consume = args.iter().any(|a| a == "--consume");
        if !run_elementwise_policy_sweep(workload, rows, consume) {
            eprintln!("fp-bench: unsupported elementwise_policy/{workload} (sqrt, log)");
            std::process::exit(2);
        }
        return;
    }

    match run(category, workload, size, dtype, data_dir) {
        Some(samples) => {
            let times: Vec<String> = samples.times_us.iter().map(|t| format!("{t}")).collect();
            let null_arm_a: Vec<String> = samples
                .null_arm_a_us
                .iter()
                .map(|t| format!("{t}"))
                .collect();
            let null_arm_b: Vec<String> = samples
                .null_arm_b_us
                .iter()
                .map(|t| format!("{t}"))
                .collect();
            let null_ratios: Vec<String> = samples
                .null_ratios
                .iter()
                .map(|ratio| format!("{ratio}"))
                .collect();
            let runtime_isa_features = runtime_isa_features()
                .into_iter()
                .map(|feature| format!("\"{feature}\""))
                .collect::<Vec<_>>()
                .join(",");
            let compiled_target_features = compiled_target_features()
                .into_iter()
                .map(|feature| format!("\"{feature}\""))
                .collect::<Vec<_>>()
                .join(",");
            println!(
                concat!(
                    "{{\"times_us\":[{}],",
                    "\"null_control\":{{\"arm_a_times_us\":[{}],",
                    "\"arm_b_times_us\":[{}],\"ratios\":[{}]}},",
                    "\"checksum\":\"{:016x}\",",
                    "\"thread_provenance\":{{",
                    "\"runtime_available_parallelism\":{},",
                    "\"process_threads_before_probe\":{},",
                    "\"peak_process_threads\":{},",
                    "\"operation_threads_used\":{},",
                    "\"runtime_detected_isa_features\":[{}],",
                    "\"compiled_target_features\":[{}]",
                    "}}}}"
                ),
                times.join(","),
                null_arm_a.join(","),
                null_arm_b.join(","),
                null_ratios.join(","),
                samples.checksum,
                samples.thread_probe.runtime_available_parallelism,
                samples.thread_probe.process_threads_before_probe,
                samples.thread_probe.peak_process_threads,
                samples.thread_probe.operation_threads_used,
                runtime_isa_features,
                compiled_target_features,
            );
        }
        None => {
            eprintln!(
                "fp-bench: unsupported {category}/{workload} (v1 coverage: dataframe_ops, groupby, rolling)"
            );
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod harness_contract_tests {
    use std::cell::Cell;

    use fp_types::Scalar;

    use super::{
        ITERS, TELEMETRY_STRING_BATCH_ROWS, paired_time_us, runtime_isa_features, self_identity,
        size_rows_cols, size_rows_cols_checked, stateful_apply_step, stateful_expanding_step,
        stateful_rolling_step, telemetry_string_batch_ranges,
    };

    #[test]
    fn executable_identity_is_a_lowercase_sha256() {
        let identity = self_identity();
        let digest = identity.split_whitespace().next().expect("identity digest");
        assert_eq!(digest.len(), 64);
        assert!(
            digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        );
        assert!(identity.contains(" bytes) "));
    }

    #[test]
    fn paired_timing_emits_one_interleaved_null_ratio_per_round() {
        let mut value = 0_u64;
        let samples = paired_time_us(
            || {
                value = value.wrapping_add(1);
                value
            },
            1,
            false,
        );
        assert_eq!(samples.times_us.len(), ITERS * 2);
        assert_eq!(samples.null_arm_a_us.len(), ITERS);
        assert_eq!(samples.null_arm_b_us.len(), ITERS);
        assert_eq!(samples.null_ratios.len(), ITERS);
        assert!(
            samples
                .null_ratios
                .iter()
                .all(|ratio| ratio.is_finite() && *ratio > 0.0)
        );
        assert_ne!(samples.checksum, 0);
        assert!(samples.thread_probe.runtime_available_parallelism >= 1);
        assert!(samples.thread_probe.operation_threads_used >= 1);
        assert!(
            samples.thread_probe.peak_process_threads
                >= samples.thread_probe.process_threads_before_probe
        );
    }

    #[test]
    fn compiled_target_features_are_a_subset_of_what_the_cpu_offers_oxv4u() {
        // The provenance bug this closes: `runtime_detected_isa_features` asks the
        // CPU what it supports, so a +avx2 build and a default build emit the SAME
        // list and a specially-flagged row is indistinguishable from a shipping one.
        // MEASURED on this host: both builds report
        // [scalar, sse2, avx2, fma, bmi2, vaes] at runtime, while compiled reports
        // ["sse2"] for the default build and ["sse2", "sse4.1", "avx", "avx2"] under
        // `-C target-feature=+avx2`.
        let compiled = crate::compiled_target_features();
        assert!(
            !compiled.is_empty(),
            "must never report an empty target set"
        );

        // Anything the COMPILER targeted must be present on the CPU, or this binary
        // could not be running at all — an illegal instruction would have fired
        // before reaching a test. This is the invariant that makes the field
        // trustworthy as provenance rather than a free-text label.
        let runtime = crate::runtime_isa_features();
        for feature in &compiled {
            if *feature == "baseline" || *feature == "sse4.1" {
                continue; // "baseline" is the non-x86 sentinel; sse4.1 is not probed
            }
            assert!(
                runtime.contains(feature),
                "compiled for {feature} but the CPU does not report it"
            );
        }
    }

    #[test]
    fn runtime_isa_provenance_always_includes_scalar_fallback() {
        assert!(runtime_isa_features().contains(&"scalar"));
    }

    #[test]
    fn large_thread_scaling_sizes_route_to_the_requested_rust_rows() {
        assert_eq!(size_rows_cols("2M"), (2_000_000, 10));
        assert_eq!(size_rows_cols("4M"), (4_000_000, 10));
        assert_eq!(size_rows_cols("6M"), (6_000_000, 10));
        assert_eq!(size_rows_cols("8M"), (8_000_000, 10));
        assert_eq!(size_rows_cols("10M"), (10_000_000, 10));
    }

    /// br-frankenpandas-kko5z. THE NEGATIVE CASE, and it is the one a naive
    /// implementation fails: the old `_ => (100_000, 10)` arm passes every
    /// positive assertion above and still silently ran FrankenPandas at 100_000
    /// rows whenever the Python harness knew a size Rust did not. It cannot be
    /// asserted through `size_rows_cols`, which now exits the process, so the
    /// checked form is what the test pins.
    #[test]
    fn an_unknown_size_label_must_not_resolve_to_a_row_count_kko5z() {
        assert_eq!(size_rows_cols_checked("1k"), Some((1_000, 10)));
        assert_eq!(size_rows_cols_checked("100"), Some((100, 10)));
        // The exact value the old default returned: if this ever comes back as
        // `Some((100_000, 10))` the forger is back.
        assert_eq!(size_rows_cols_checked("500k"), None);
        assert_eq!(size_rows_cols_checked(""), None);
        assert_eq!(size_rows_cols_checked("1000"), None);
        assert_eq!(size_rows_cols_checked("1M "), None);
    }

    /// Every label the Python harness's `SIZE_CONFIGS` knows must resolve here
    /// to the SAME row count, because a disagreement between the two tables is
    /// exactly the defect above wearing different clothes.
    #[test]
    fn rust_and_harness_size_tables_agree_kko5z() {
        for (label, rows) in [
            ("100", 100_usize),
            ("1k", 1_000),
            ("10k", 10_000),
            ("100k", 100_000),
            ("1M", 1_000_000),
            ("2M", 2_000_000),
            ("4M", 4_000_000),
            ("6M", 6_000_000),
            ("8M", 8_000_000),
            ("10M", 10_000_000),
        ] {
            assert_eq!(
                size_rows_cols_checked(label),
                Some((rows, 10)),
                "size label {label:?} disagrees with benches/vs_pandas_harness.py SIZE_CONFIGS"
            );
        }
    }

    #[test]
    fn telemetry_string_batches_cover_each_row_once() {
        assert!(telemetry_string_batch_ranges(0).is_empty());
        assert_eq!(
            telemetry_string_batch_ranges(TELEMETRY_STRING_BATCH_ROWS),
            vec![(0, TELEMETRY_STRING_BATCH_ROWS)]
        );
        assert_eq!(
            telemetry_string_batch_ranges(TELEMETRY_STRING_BATCH_ROWS * 2 + 1),
            vec![
                (0, TELEMETRY_STRING_BATCH_ROWS),
                (TELEMETRY_STRING_BATCH_ROWS, TELEMETRY_STRING_BATCH_ROWS * 2,),
                (
                    TELEMETRY_STRING_BATCH_ROWS * 2,
                    TELEMETRY_STRING_BATCH_ROWS * 2 + 1,
                ),
            ]
        );
    }

    #[test]
    fn stateful_apply_fixture_is_order_dependent_and_deterministic() {
        let state = Cell::new(0_i64);
        let actual: Vec<Scalar> = (0..8)
            .map(|value| stateful_apply_step(&state, &Scalar::Int64(value)))
            .collect();
        assert_eq!(
            actual,
            vec![
                Scalar::Int64(0),
                Scalar::Int64(1),
                Scalar::Int64(33),
                Scalar::Int64(1_026),
                Scalar::Int64(31_810),
                Scalar::Int64(986_115),
                Scalar::Int64(30_569_571),
                Scalar::Int64(947_656_708),
            ]
        );
        assert_eq!(state.get(), 947_656_708);
    }

    #[test]
    fn stateful_rolling_fixture_preserves_window_and_callback_order() {
        let state = Cell::new(0_i64);
        let actual = [
            stateful_rolling_step(&state, &[0.0, 1.0, 2.0]),
            stateful_rolling_step(&state, &[1.0, 2.0, 3.0]),
            stateful_rolling_step(&state, &[2.0, 3.0, 4.0]),
        ];
        assert_eq!(actual, [3.0, 99.0, 3_078.0]);
        assert_eq!(state.get(), 3_078);
    }

    #[test]
    fn stateful_expanding_fixture_preserves_prefix_and_callback_order() {
        let state = Cell::new(0_i64);
        let actual = [
            stateful_expanding_step(&state, &[0.0]),
            stateful_expanding_step(&state, &[0.0, 1.0]),
            stateful_expanding_step(&state, &[0.0, 1.0, 2.0]),
        ];
        assert_eq!(actual, [1.0, 34.0, 1_059.0]);
        assert_eq!(state.get(), 1_059);
    }
}

#[cfg(all(test, feature = "lazy-transpose-prototype"))]
mod tests {
    use super::PrototypeF64Block;

    #[test]
    fn lazy_transpose_prototype_indexes_column_major_block() {
        let raw = vec![vec![1.0, 2.0, 3.0], vec![10.0, 20.0, 30.0]];
        let block = PrototypeF64Block::from_column_vectors(&raw);
        let view = block.transpose_view();

        assert_eq!(view.shape(), (2, 3));
        assert_eq!(view.get(0, 0), 1.0);
        assert_eq!(view.get(0, 2), 3.0);
        assert_eq!(view.get(1, 0), 10.0);
        assert_eq!(view.get(1, 2), 30.0);
    }
}
