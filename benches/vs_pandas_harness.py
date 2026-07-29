#!/usr/bin/env python3
"""vs-pandas head-to-head timing harness.

Runs identical workloads on both FrankenPandas (Rust, release-perf) and
pandas 2.2.3, capturing p50/p95/p99 + cv_pct + throughput per engine.

Per BENCH_MATRIX_SPEC.md:
- Uses release-perf profile for FP (not --release)
- Emits executable SHA-256 provenance for both engines
- Measures an interleaved A/A null control inside each engine invocation
- Gates claims on the null-median bootstrap 95% CI, never on cv
- Population/setup OUTSIDE the timed window
- EngineIdentity Subject!=Oracle on every artifact

Usage:
    python benches/vs_pandas_harness.py --category io --sizes 100k
    python benches/vs_pandas_harness.py --all --sizes 10k,100k,1M
    taskset -c 0-63 python benches/vs_pandas_harness.py \
        --category groupby --workloads groupby_mean_float64 \
        --sizes 1M,10M --thread-count 64
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.metadata
import json
import math
import os
import platform
import re
import socket
import subprocess
import sys
import threading
import time
from dataclasses import dataclass, field
from datetime import datetime, timezone
from io import StringIO
from json import JSONDecodeError
from pathlib import Path
from statistics import mean, stdev
from typing import Any

import numpy as np

try:
    import pandas as pd
except ImportError:
    pd = None

try:
    import pyarrow as pa
except ImportError:
    pa = None

PROJECT_ROOT = Path(__file__).parent.parent
RESULTS_DIR = PROJECT_ROOT / "artifacts" / "bench"

CATEGORIES = {
    "io": 0.25,
    "dataframe_ops": 0.20,
    "groupby": 0.20,
    "joins": 0.15,
    "rolling": 0.10,
    "indexing": 0.10,
    "strings": 0.10,
    "linalg": 0.10,
    "datetime": 0.10,
}

SIZE_CONFIGS = {
    "10k": {"rows": 10_000, "cols": 10},
    "100k": {"rows": 100_000, "cols": 10},
    "1M": {"rows": 1_000_000, "cols": 10},
    "2M": {"rows": 2_000_000, "cols": 10},
    "4M": {"rows": 4_000_000, "cols": 10},
    "6M": {"rows": 6_000_000, "cols": 10},
    "8M": {"rows": 8_000_000, "cols": 10},
    "10M": {"rows": 10_000_000, "cols": 10},
}

PAIRED_ROUNDS = 25
BOOTSTRAP_RESAMPLES = 10_000
NULL_CI_CONFIDENCE = 0.95
DECIDABILITY_MARGIN = 2.0
WARMUP_ITERATIONS = 3
CPU_SAMPLE_INTERVAL_SECONDS = 0.300
MAX_HOST_WIDE_BUSY_FRACTION = 0.20
TAKE_BATCH = 256
TRANSPOSE_BATCH = 8192


@dataclass
class PairedSamples:
    """One engine's timings plus its same-invocation A/A null control."""
    times_us: list[float] = field(default_factory=list)
    null_arm_a_us: list[float] = field(default_factory=list)
    null_arm_b_us: list[float] = field(default_factory=list)
    null_ratios: list[float] = field(default_factory=list)
    checksum: int = 0
    runtime_available_parallelism: int = 1
    process_threads_before_probe: int = 1
    peak_process_threads: int = 1
    operation_threads_used: int = 1


def _read_text(path: Path) -> str | None:
    try:
        return path.read_text().strip()
    except OSError:
        return None


def _online_cpu_ids() -> list[int]:
    cpu_ids = []
    for cpu_dir in Path("/sys/devices/system/cpu").glob("cpu[0-9]*"):
        try:
            cpu_id = int(cpu_dir.name[3:])
        except ValueError:
            continue
        online = _read_text(cpu_dir / "online")
        if online in (None, "1"):
            cpu_ids.append(cpu_id)
    return sorted(cpu_ids)


def _read_host_cpu_ticks() -> dict[int, tuple[int, int]]:
    """Read total and idle scheduler ticks for every online host CPU."""
    stat = _read_text(Path("/proc/stat"))
    if stat is None:
        raise RuntimeError("host-wide exclusivity requires readable /proc/stat")

    ticks = {}
    for line in stat.splitlines():
        fields = line.split()
        if not fields:
            continue
        label = fields[0]
        if not label.startswith("cpu") or not label[3:].isdigit():
            continue
        try:
            values = [int(value) for value in fields[1:]]
        except ValueError as error:
            raise RuntimeError(f"invalid /proc/stat row for {label}") from error
        if len(values) < 5:
            raise RuntimeError(f"/proc/stat row for {label} is too short")
        total = sum(values)
        idle = values[3] + values[4]
        ticks[int(label[3:])] = (total, idle)
    if not ticks:
        raise RuntimeError("host-wide exclusivity found no CPU rows in /proc/stat")
    return ticks


def _sample_host_cpu_busy() -> dict[int, float]:
    """Sample per-CPU busy fractions over the fleet-standard 300 ms window."""
    before = _read_host_cpu_ticks()
    time.sleep(CPU_SAMPLE_INTERVAL_SECONDS)
    after = _read_host_cpu_ticks()
    busy = {}
    for cpu_id, (total_before, idle_before) in before.items():
        if cpu_id not in after:
            continue
        total_after, idle_after = after[cpu_id]
        total_delta = max(0, total_after - total_before)
        idle_delta = max(0, idle_after - idle_before)
        busy[cpu_id] = (
            1.0
            if total_delta == 0
            else max(0, total_delta - idle_delta) / total_delta
        )
    return busy


def _host_wide_quiescence_observation(
    phase: str,
    expected_cpu_ids: list[int],
    busy_fractions: dict[int, float],
) -> dict[str, Any]:
    """Adjudicate one all-online-CPU quiescence sample."""
    missing_cpu_ids = [
        cpu_id for cpu_id in expected_cpu_ids if cpu_id not in busy_fractions
    ]
    busy_cpu_ids = [
        cpu_id
        for cpu_id in expected_cpu_ids
        if busy_fractions.get(cpu_id, 1.0) > MAX_HOST_WIDE_BUSY_FRACTION
    ]
    observed = {
        f"cpu{cpu_id}": busy_fractions[cpu_id]
        for cpu_id in expected_cpu_ids
        if cpu_id in busy_fractions
    }
    return {
        "phase": phase,
        "scope": "all_online_host_cpus",
        "sample_interval_ms": round(CPU_SAMPLE_INTERVAL_SECONDS * 1000),
        "maximum_busy_fraction": MAX_HOST_WIDE_BUSY_FRACTION,
        "expected_cpu_count": len(expected_cpu_ids),
        "sampled_cpu_count": len(observed),
        "missing_cpu_ids": missing_cpu_ids,
        "busy_cpu_ids_above_limit": busy_cpu_ids,
        "busy_cpu_count_above_limit": len(busy_cpu_ids),
        "maximum_observed_busy_fraction": max(observed.values(), default=1.0),
        "sampled_cpu_busy_fraction": observed,
        "verdict": (
            "clear"
            if expected_cpu_ids and not missing_cpu_ids and not busy_cpu_ids
            else "blocked"
        ),
    }


@dataclass
class HostWideExclusivityGate:
    """Fail closed unless every online host CPU is quiet before each arm."""
    expected_cpu_ids: list[int]
    observations: list[dict[str, Any]] = field(default_factory=list)

    def require_quiet(self, phase: str) -> dict[str, Any]:
        try:
            busy_fractions = _sample_host_cpu_busy()
        except RuntimeError as error:
            print(
                f"ERROR: host-wide benchmark exclusivity: {error}",
                file=sys.stderr,
            )
            raise SystemExit(2) from error
        observation = _host_wide_quiescence_observation(
            phase,
            self.expected_cpu_ids,
            busy_fractions,
        )
        self.observations.append(observation)
        if observation["verdict"] != "clear":
            print(
                "ERROR: host-wide benchmark exclusivity requires every online "
                "CPU to remain at or below "
                f"{MAX_HOST_WIDE_BUSY_FRACTION * 100:.1f}% busy; "
                f"phase={phase} missing={observation['missing_cpu_ids']} "
                f"busy={observation['busy_cpu_ids_above_limit']}",
                file=sys.stderr,
            )
            raise SystemExit(2)
        print(
            "host_wide_quiescence="
            f"phase={phase} "
            f"online_cpu_count={len(self.expected_cpu_ids)} "
            f"maximum_busy_fraction={MAX_HOST_WIDE_BUSY_FRACTION:.3f} "
            "busy_cpu_count_above_limit=0 verdict=clear"
        )
        return observation

    def artifact(self) -> dict[str, Any]:
        return {
            "required": True,
            "scope": "all_online_host_cpus",
            "online_cpu_ids": self.expected_cpu_ids,
            "maximum_busy_fraction": MAX_HOST_WIDE_BUSY_FRACTION,
            "sample_interval_ms": round(CPU_SAMPLE_INTERVAL_SECONDS * 1000),
            "observations": self.observations,
            "valid": bool(self.observations)
            and all(
                observation["verdict"] == "clear"
                for observation in self.observations
            ),
        }


def _host_wide_exclusivity_self_test() -> None:
    """Exercise clear, busy, and incomplete adjudication without timing."""
    clear = _host_wide_quiescence_observation(
        "self-test-clear",
        [0, 1],
        {0: 0.0, 1: MAX_HOST_WIDE_BUSY_FRACTION},
    )
    if clear["verdict"] != "clear":
        raise RuntimeError("threshold-boundary sample must be clear")
    busy = _host_wide_quiescence_observation(
        "self-test-busy",
        [0, 1],
        {0: 0.0, 1: MAX_HOST_WIDE_BUSY_FRACTION + 0.001},
    )
    if (
        busy["verdict"] != "blocked"
        or busy["busy_cpu_ids_above_limit"] != [1]
    ):
        raise RuntimeError("over-threshold sample must identify the busy CPU")
    incomplete = _host_wide_quiescence_observation(
        "self-test-incomplete",
        [0, 1],
        {0: 0.0},
    )
    if (
        incomplete["verdict"] != "blocked"
        or incomplete["missing_cpu_ids"] != [1]
    ):
        raise RuntimeError("incomplete sample must identify the missing CPU")


def _cpu_flags() -> set[str]:
    cpuinfo = _read_text(Path("/proc/cpuinfo")) or ""
    for line in cpuinfo.splitlines():
        if line.lower().startswith(("flags", "features")) and ":" in line:
            return set(line.split(":", 1)[1].split())
    return set()


def _cpu_model() -> str:
    cpuinfo = _read_text(Path("/proc/cpuinfo")) or ""
    for line in cpuinfo.splitlines():
        if line.lower().startswith(("model name", "hardware")) and ":" in line:
            return line.split(":", 1)[1].strip()
    return platform.processor() or "unknown"


def _git_sha() -> str:
    try:
        result = subprocess.run(
            ["git", "rev-parse", "HEAD"],
            cwd=PROJECT_ROOT,
            capture_output=True,
            check=False,
            text=True,
            timeout=10,
        )
    except (OSError, subprocess.TimeoutExpired):
        return "unavailable"
    return result.stdout.strip() if result.returncode == 0 else "unavailable"


def host_fingerprint() -> dict[str, Any]:
    """Capture host-wide topology plus the process' effective CPU budget."""
    online_cpus = _online_cpu_ids()
    affinity_cpus = (
        sorted(os.sched_getaffinity(0))
        if hasattr(os, "sched_getaffinity")
        else online_cpus
    )
    physical_cores: set[tuple[str, str]] = set()
    for cpu_id in online_cpus:
        topology = Path(f"/sys/devices/system/cpu/cpu{cpu_id}/topology")
        package = _read_text(topology / "physical_package_id")
        core = _read_text(topology / "core_id")
        if package is not None and core is not None:
            physical_cores.add((package, core))

    flags = _cpu_flags()
    tracked_isa = (
        "sse2",
        "avx",
        "avx2",
        "fma",
        "bmi1",
        "bmi2",
        "aes",
        "vaes",
        "avx512f",
    )
    mem_total_kib = 0
    meminfo = _read_text(Path("/proc/meminfo")) or ""
    for line in meminfo.splitlines():
        if line.startswith("MemTotal:"):
            mem_total_kib = int(line.split()[1])
            break

    governors = sorted(
        {
            value
            for cpu_id in affinity_cpus
            if (
                value := _read_text(
                    Path(
                        f"/sys/devices/system/cpu/cpu{cpu_id}/cpufreq/"
                        "scaling_governor"
                    )
                )
            )
        }
    )
    numa_nodes = sorted(
        path.name
        for path in Path("/sys/devices/system/node").glob("node[0-9]*")
    )
    return {
        "host_identity": socket.gethostname(),
        "platform_node": platform.node(),
        "architecture": platform.machine(),
        "cpu_model": _cpu_model(),
        "physical_cores": len(physical_cores),
        "logical_threads": len(online_cpus),
        "threads_per_core": (
            len(online_cpus) // len(physical_cores) if physical_cores else None
        ),
        "ram_bytes": mem_total_kib * 1024,
        "numa_nodes": len(numa_nodes),
        "kernel": platform.release(),
        "git_sha": _git_sha(),
        "cpu_governors": governors,
        "smt_active": _read_text(Path("/sys/devices/system/cpu/smt/active")),
        "frequency_boost": _read_text(
            Path("/sys/devices/system/cpu/cpufreq/boost")
        ),
        "affinity_cpus": affinity_cpus,
        "affinity_logical_cpu_cap": len(affinity_cpus),
        "runtime_detected_isa_features": [
            feature for feature in tracked_isa if feature in flags
        ],
        "runtime_absent_isa_features": [
            feature for feature in tracked_isa if feature not in flags
        ],
    }


def _process_thread_count() -> int:
    status = _read_text(Path("/proc/self/status")) or ""
    for line in status.splitlines():
        if line.startswith("Threads:"):
            return int(line.split(":", 1)[1].strip())
    return 1


def _thread_cpu_ticks() -> dict[int, int]:
    ticks: dict[int, int] = {}
    for task_dir in Path("/proc/self/task").glob("[0-9]*"):
        stat = _read_text(task_dir / "stat")
        if not stat:
            continue
        close_paren = stat.rfind(")")
        if close_paren < 0:
            continue
        fields = stat[close_paren + 1 :].split()
        if len(fields) <= 12:
            continue
        try:
            ticks[int(task_dir.name)] = int(fields[11]) + int(fields[12])
        except ValueError:
            continue
    return ticks


def probe_operation_threads(func) -> dict[str, int]:
    """Observe one untimed call and report active/peak process threads."""
    runtime_available_parallelism = (
        len(os.sched_getaffinity(0))
        if hasattr(os, "sched_getaffinity")
        else (os.cpu_count() or 1)
    )
    process_threads_before_probe = _process_thread_count()
    ticks_before = _thread_cpu_ticks()
    peak_process_threads = process_threads_before_probe
    ready = threading.Event()
    stop = threading.Event()

    def monitor() -> None:
        nonlocal peak_process_threads
        ready.set()
        while not stop.is_set():
            peak_process_threads = max(
                peak_process_threads,
                _process_thread_count(),
            )
            time.sleep(0.000_020)
        peak_process_threads = max(peak_process_threads, _process_thread_count())

    monitor_thread = threading.Thread(
        target=monitor,
        name="fp-bench-thread-probe",
        daemon=True,
    )
    monitor_thread.start()
    ready.wait()
    monitor_native_id = monitor_thread.native_id
    try:
        func()
    finally:
        stop.set()
        monitor_thread.join()

    ticks_after = _thread_cpu_ticks()
    cpu_active_threads = sum(
        ticks > ticks_before.get(tid, 0)
        for tid, ticks in ticks_after.items()
        if tid != monitor_native_id
    )
    newly_spawned_workers = max(
        0,
        peak_process_threads - process_threads_before_probe - 1,
    )
    return {
        "runtime_available_parallelism": runtime_available_parallelism,
        "process_threads_before_probe": process_threads_before_probe,
        "peak_process_threads": peak_process_threads,
        "operation_threads_used": max(
            1,
            cpu_active_threads,
            newly_spawned_workers,
        ),
    }


def executable_identity(path: Path) -> dict[str, Any]:
    """Hash the executable that actually hosts this engine."""
    resolved = path.resolve(strict=True)
    digest = hashlib.sha256()
    byte_count = 0
    with resolved.open("rb") as executable:
        while chunk := executable.read(1024 * 1024):
            digest.update(chunk)
            byte_count += len(chunk)
    return {
        "sha256": digest.hexdigest(),
        "bytes": byte_count,
        "path": str(resolved),
    }


def distribution_artifact_identity(
    distribution_name: str,
    module_file: str,
) -> dict[str, Any]:
    """Hash one installed distribution that participates in an engine arm.

    Hashing only ``sys.executable`` identifies the Python host, not the legacy
    incumbent or its optional storage backend. The distribution file list
    comes from the installed wheel's metadata; folding each relative path,
    byte length, and file body produces one deterministic identity for the
    actual package loaded here.
    """
    distribution = importlib.metadata.distribution(distribution_name)
    package_files = sorted(distribution.files or (), key=lambda item: str(item))
    if not package_files:
        raise RuntimeError(
            f"installed {distribution_name} distribution has no file manifest"
        )

    digest = hashlib.sha256()
    byte_count = 0
    file_count = 0
    for package_file in package_files:
        resolved = Path(distribution.locate_file(package_file))
        if not resolved.is_file():
            continue
        relative = str(package_file).replace(os.sep, "/").encode("utf-8")
        file_size = resolved.stat().st_size
        digest.update(len(relative).to_bytes(8, "little"))
        digest.update(relative)
        digest.update(file_size.to_bytes(8, "little"))
        with resolved.open("rb") as artifact_file:
            while chunk := artifact_file.read(1024 * 1024):
                digest.update(chunk)
                byte_count += len(chunk)
        file_count += 1

    if file_count == 0:
        raise RuntimeError(
            f"installed {distribution_name} distribution has no readable files"
        )
    return {
        "sha256": digest.hexdigest(),
        "bytes": byte_count,
        "files": file_count,
        "path": str(Path(module_file).resolve()),
        "scheme": "importlib-metadata-content-tree-v1",
    }


def pandas_artifact_identity() -> dict[str, Any]:
    """Hash the installed pandas distribution imported by this process."""
    return distribution_artifact_identity("pandas", pd.__file__)


def pyarrow_artifact_identity() -> dict[str, Any]:
    """Hash the optional Arrow backend used by string[pyarrow] arms."""
    return distribution_artifact_identity("pyarrow", pa.__file__)


def bootstrap_median_ci(values: list[float]) -> tuple[float, float]:
    """Deterministic percentile-bootstrap CI for the sample median."""
    if not values:
        raise ValueError("cannot bootstrap an empty null-control sample")
    sample = np.asarray(values, dtype=np.float64)
    rng = np.random.default_rng(0xF2A_2026_0725)
    indices = rng.integers(
        0,
        len(sample),
        size=(BOOTSTRAP_RESAMPLES, len(sample)),
    )
    medians = np.median(sample[indices], axis=1)
    tail_pct = (1.0 - NULL_CI_CONFIDENCE) * 50.0
    low, high = np.percentile(medians, [tail_pct, 100.0 - tail_pct])
    return float(low), float(high)


@dataclass
class TimingResult:
    """Raw timing measurements for a single workload."""
    workload: str
    category: str
    size: str
    dtype: str
    engine: str
    times_us: list[float] = field(default_factory=list)
    null_arm_a_us: list[float] = field(default_factory=list)
    null_arm_b_us: list[float] = field(default_factory=list)
    null_ratios: list[float] = field(default_factory=list)
    checksum: str | None = None
    executable_sha256: str | None = None
    executable_bytes: int | None = None
    executable_path: str | None = None
    runtime_available_parallelism: int | None = None
    process_threads_before_probe: int | None = None
    peak_process_threads: int | None = None
    operation_threads_used: int | None = None
    runtime_detected_isa_features: list[str] = field(default_factory=list)

    @property
    def p50_us(self) -> float:
        return float(np.percentile(self.times_us, 50))

    @property
    def p95_us(self) -> float:
        return float(np.percentile(self.times_us, 95))

    @property
    def p99_us(self) -> float:
        return float(np.percentile(self.times_us, 99))

    @property
    def mean_us(self) -> float:
        return mean(self.times_us)

    @property
    def stddev_us(self) -> float:
        return stdev(self.times_us) if len(self.times_us) > 1 else 0.0

    @property
    def cv_pct(self) -> float:
        return (self.stddev_us / self.mean_us * 100) if self.mean_us > 0 else 0.0

    @property
    def null_median_ratio(self) -> float:
        return float(np.median(self.null_ratios))

    @property
    def null_median_ci(self) -> tuple[float, float]:
        return bootstrap_median_ci(self.null_ratios)

    @property
    def null_log_half_width(self) -> float:
        low, high = self.null_median_ci
        if low <= 0.0 or high <= 0.0:
            return math.inf
        return max(abs(math.log(low)), abs(math.log(high)))

    @property
    def is_valid(self) -> bool:
        """Contract-valid measurement; cv is provenance, never a gate."""
        return bool(
            self.times_us
            and self.null_ratios
            and len(self.null_arm_a_us) == len(self.null_ratios)
            and len(self.null_arm_b_us) == len(self.null_ratios)
            and self.checksum
            and self.executable_sha256
            and self.runtime_available_parallelism
            and self.operation_threads_used
            and self.runtime_detected_isa_features
        )

    def to_metrics(self, rows: int) -> dict[str, Any]:
        null_ci_low, null_ci_high = self.null_median_ci
        null_log_half_width = self.null_log_half_width
        return {
            "p50_us": round(self.p50_us, 2),
            "p95_us": round(self.p95_us, 2),
            "p99_us": round(self.p99_us, 2),
            "mean_us": round(self.mean_us, 2),
            "stddev_us": round(self.stddev_us, 2),
            "cv_pct": round(self.cv_pct, 2),
            "throughput_rows_sec": round(rows / (self.p50_us / 1_000_000)),
            "checksum": self.checksum,
            "thread_count_actually_used": self.operation_threads_used,
            "runtime_available_parallelism": self.runtime_available_parallelism,
            "process_threads_before_probe": self.process_threads_before_probe,
            "peak_process_threads": self.peak_process_threads,
            "runtime_detected_isa_features": self.runtime_detected_isa_features,
            "samples_us": self.times_us,
            "executable": {
                "sha256": self.executable_sha256,
                "bytes": self.executable_bytes,
                "path": self.executable_path,
            },
            "null_control": {
                "rounds": len(self.null_ratios),
                "arm_a_times_us": self.null_arm_a_us,
                "arm_b_times_us": self.null_arm_b_us,
                "ratios": self.null_ratios,
                "median_ratio": round(self.null_median_ratio, 6),
                "median_ci_95": [
                    round(null_ci_low, 6),
                    round(null_ci_high, 6),
                ],
                "log_half_width": round(null_log_half_width, 8),
                "two_x_decidable_interval": [
                    round(math.exp(-DECIDABILITY_MARGIN * null_log_half_width), 6),
                    round(math.exp(DECIDABILITY_MARGIN * null_log_half_width), 6),
                ],
            },
        }


def generate_test_data(rows: int, cols: int, dtype: str, seed: int = 42) -> pd.DataFrame:
    """Generate test DataFrame OUTSIDE the timed window."""
    rng = np.random.default_rng(seed)

    if dtype == "int64":
        data = {f"col_{i}": rng.integers(0, 1_000_000, size=rows) for i in range(cols)}
    elif dtype == "bool":
        data = {f"col_{i}": rng.integers(0, 2, size=rows, dtype=np.int8).astype(bool)
                for i in range(cols)}
    elif dtype in ("datetime64", "datetime64[ns]"):
        base = np.datetime64("2021-01-01T00:00:00", "ns")
        offsets = np.arange(rows, dtype=np.int64) * 1_000_000_000
        data = {f"col_{i}": base + (offsets + i).astype("timedelta64[ns]")
                for i in range(cols)}
    elif dtype in ("timedelta64", "timedelta64[ns]"):
        offsets = np.arange(rows, dtype=np.int64) * 1_000_000
        data = {f"col_{i}": (offsets + i).astype("timedelta64[ns]")
                for i in range(cols)}
    elif dtype == "float64":
        data = {f"col_{i}": rng.random(rows) * 1_000_000 for i in range(cols)}
    elif dtype == "float64_nan10":
        data = {}
        for i in range(cols):
            arr = rng.random(rows) * 1_000_000
            mask = rng.random(rows) < 0.10
            arr[mask] = np.nan
            data[f"col_{i}"] = arr
    elif dtype == "float64_nan50":
        data = {}
        for i in range(cols):
            arr = rng.random(rows) * 1_000_000
            mask = rng.random(rows) < 0.50
            arr[mask] = np.nan
            data[f"col_{i}"] = arr
    elif dtype == "float64_nan37":
        data = {}
        for i in range(cols):
            arr = rng.random(rows) * 1_000_000
            arr[::37] = np.nan
            data[f"col_{i}"] = arr
    else:
        raise ValueError(f"Unknown dtype: {dtype}")

    return pd.DataFrame(data)


def _observation_token(result: Any) -> tuple[Any, ...]:
    """Small deterministic observation folded outside the measured region."""
    shape = getattr(result, "shape", None)
    dtype = getattr(result, "dtype", None)
    dtypes = getattr(result, "dtypes", None)
    if dtypes is not None:
        try:
            dtype_token: Any = tuple(str(item) for item in dtypes)
        except TypeError:
            dtype_token = str(dtypes)
    else:
        dtype_token = str(dtype) if dtype is not None else None
    try:
        length = len(result)
    except TypeError:
        length = None
    return (
        type(result).__qualname__,
        tuple(shape) if shape is not None else None,
        dtype_token,
        length,
    )


def _fold_checksum(checksum: int, result: Any) -> int:
    encoded = repr(_observation_token(result)).encode("utf-8")
    observed = int.from_bytes(hashlib.sha256(encoded).digest()[:8], "little")
    rotated = ((checksum << 9) | (checksum >> 55)) & ((1 << 64) - 1)
    return rotated ^ observed


def paired_operation(func, repeat: int = 1,
                     warmup: int = WARMUP_ITERATIONS,
                     rounds: int = PAIRED_ROUNDS) -> PairedSamples:
    """Time identical arms back-to-back, alternating order every round."""
    thread_probe = probe_operation_threads(func)
    for _ in range(warmup):
        for _ in range(repeat):
            func()

    def time_arm(checksum: int) -> tuple[float, int]:
        start = time.perf_counter_ns()
        result = None
        for _ in range(repeat):
            result = func()
        elapsed_us = (time.perf_counter_ns() - start) / 1000
        return elapsed_us, _fold_checksum(checksum, result)

    times_us: list[float] = []
    null_arm_a_us: list[float] = []
    null_arm_b_us: list[float] = []
    null_ratios: list[float] = []
    checksum = 0
    for round_index in range(rounds):
        if round_index % 2 == 0:
            arm_a_us, checksum = time_arm(checksum)
            arm_b_us, checksum = time_arm(checksum)
        else:
            arm_b_us, checksum = time_arm(checksum)
            arm_a_us, checksum = time_arm(checksum)
        times_us.extend((arm_a_us, arm_b_us))
        null_arm_a_us.append(arm_a_us)
        null_arm_b_us.append(arm_b_us)
        null_ratios.append(arm_a_us / arm_b_us)

    return PairedSamples(
        times_us=times_us,
        null_arm_a_us=null_arm_a_us,
        null_arm_b_us=null_arm_b_us,
        null_ratios=null_ratios,
        checksum=checksum,
        runtime_available_parallelism=thread_probe[
            "runtime_available_parallelism"
        ],
        process_threads_before_probe=thread_probe[
            "process_threads_before_probe"
        ],
        peak_process_threads=thread_probe["peak_process_threads"],
        operation_threads_used=thread_probe["operation_threads_used"],
    )


def time_operation(func, warmup: int = WARMUP_ITERATIONS) -> PairedSamples:
    """Time an operation with an interleaved same-invocation A/A control."""
    return paired_operation(func, warmup=warmup)


def time_operation_repeated(func, repeat: int,
                            warmup: int = WARMUP_ITERATIONS) -> PairedSamples:
    """Time a fixed-size batch with an interleaved A/A null control."""
    return paired_operation(func, repeat=repeat, warmup=warmup)


# IO Workloads (pandas)
def bench_csv_read_pandas(df: pd.DataFrame, tmp_path: Path) -> float:
    csv_path = tmp_path / "bench.csv"
    df.to_csv(csv_path, index=False)
    return time_operation(lambda: pd.read_csv(csv_path))

def bench_csv_write_pandas(df: pd.DataFrame, tmp_path: Path) -> float:
    csv_path = tmp_path / "bench_out.csv"
    return time_operation(lambda: df.to_csv(csv_path, index=False))


def bench_json_read_records_pandas(df: pd.DataFrame, tmp_path: Path) -> list[float]:
    del tmp_path
    payload = df.to_json(orient="records")
    return time_operation(
        lambda: pd.read_json(StringIO(payload), orient="records")
    )


def bench_json_read_columns_pandas(df: pd.DataFrame, tmp_path: Path) -> list[float]:
    del tmp_path
    payload = df.to_json(orient="columns")
    return time_operation(
        lambda: pd.read_json(StringIO(payload), orient="columns")
    )


def bench_json_read_index_pandas(df: pd.DataFrame, tmp_path: Path) -> list[float]:
    del tmp_path
    payload = df.to_json(orient="index")
    return time_operation(
        lambda: pd.read_json(StringIO(payload), orient="index")
    )


def bench_json_read_split_pandas(df: pd.DataFrame, tmp_path: Path) -> list[float]:
    del tmp_path
    payload = df.to_json(orient="split")
    return time_operation(
        lambda: pd.read_json(StringIO(payload), orient="split")
    )


def bench_json_read_values_pandas(df: pd.DataFrame, tmp_path: Path) -> list[float]:
    del tmp_path
    payload = df.to_json(orient="values")
    return time_operation(
        lambda: pd.read_json(StringIO(payload), orient="values")
    )


def bench_parquet_read_pandas(df: pd.DataFrame, tmp_path: Path) -> float:
    pq_path = tmp_path / "bench.parquet"
    df.to_parquet(pq_path, index=False)
    return time_operation(lambda: pd.read_parquet(pq_path))

def bench_parquet_write_pandas(df: pd.DataFrame, tmp_path: Path) -> float:
    pq_path = tmp_path / "bench_out.parquet"
    return time_operation(lambda: df.to_parquet(pq_path, index=False))


# DataFrame Ops Workloads (pandas)
def bench_sort_values_single_pandas(df: pd.DataFrame) -> list[float]:
    return time_operation(lambda: df.sort_values("col_0"))

def bench_sort_values_multi_pandas(df: pd.DataFrame) -> list[float]:
    return time_operation(lambda: df.sort_values(["col_0", "col_1", "col_2"]))

def bench_filter_bool_mask_pandas(df: pd.DataFrame) -> list[float]:
    return time_operation(lambda: df[df["col_0"] > df["col_0"].median()])

def bench_drop_duplicates_pandas(df: pd.DataFrame) -> list[float]:
    return time_operation(lambda: df.drop_duplicates(subset=["col_0"]))

def bench_value_counts_pandas(df: pd.DataFrame) -> list[float]:
    return time_operation(lambda: df["col_0"].value_counts())

def bench_cumsum_pandas(df: pd.DataFrame) -> list[float]:
    return time_operation(lambda: df.cumsum())


def bench_df_abs_pandas(df: pd.DataFrame) -> list[float]:
    """Elementwise absolute value over the same dense Float64 frame as fp-bench."""
    return time_operation(lambda: df.abs())


def bench_df_melt_pandas(df: pd.DataFrame) -> list[float]:
    # Exact counterpart to fp-bench dataframe_ops/df_melt:
    # DataFrame::melt([], [], None, None) selects every input column as a
    # value column and uses the default "variable"/"value" output names.
    return time_operation(lambda: df.melt())


def bench_df_transpose_pandas(df: pd.DataFrame) -> list[float]:
    return time_operation_repeated(lambda: df.T, TRANSPOSE_BATCH)

def bench_df_transpose_materialize_pandas(df: pd.DataFrame) -> list[float]:
    # Counterpart to fp-bench dataframe_ops/df_transpose_materialize. `df.T` alone
    # can be a lazy/blockwise construction on both sides, so this row crosses the
    # materialization boundary explicitly by reading real values out of the
    # transposed frame, matching what the Rust row does.
    def op():
        t = df.T
        col = t.columns[0]
        return len(t[col].to_numpy())

    return time_operation(op)


def bench_df_to_dict_index_materialize_pandas(df: pd.DataFrame) -> list[float]:
    # Counterpart to fp-bench dataframe_ops/df_to_dict_index_materialize:
    # to_dict('index') is fully materialized in pandas, so the plain call is
    # the honest boundary-crossing row (the fp side forces as_mapping()).
    def op():
        return len(df.to_dict("index"))

    return time_operation(op)


def bench_df_pivot_pandas(df: pd.DataFrame) -> list[float]:
    # Counterpart to fp-bench dataframe_ops/df_pivot, whose comment specifies
    # df.pivot(index="r", columns="c", values="v") with UNIQUE (r, c) pairs
    # (pivot raises on duplicates): r = i // 10, c = i % 10.
    n = len(df)
    rows = np.arange(n)
    f = pd.DataFrame({"r": rows // 10, "c": rows % 10, "v": df["col_0"].to_numpy()})
    return time_operation(lambda: f.pivot(index="r", columns="c", values="v"))


def bench_df_pivot_table_pandas(df: pd.DataFrame) -> list[float]:
    # Counterpart to fp-bench dataframe_ops/df_pivot_table: r = i % 100 (100
    # rows), c = i % 10 (10 cols), aggfunc="mean" -> a 100x10 result.
    n = len(df)
    rows = np.arange(n)
    f = pd.DataFrame({"r": rows % 100, "c": rows % 10, "v": df["col_0"].to_numpy()})
    return time_operation(
        lambda: f.pivot_table(values="v", index="r", columns="c", aggfunc="mean")
    )


def bench_df_iterrows_pandas(df: pd.DataFrame) -> list[float]:
    # Counterpart to fp-bench dataframe_ops/df_iterrows, whose comment specifies
    # `list(df.iterrows())`. Class-1 structural shape: pandas constructs a Series
    # object per row, so the cost is per-element interpreter work rather than
    # kernel work. `list(...)` forces the generator so both sides do the full
    # materialization the Rust row does.
    def op():
        return len(list(df.iterrows()))

    return time_operation(op)


def bench_df_itertuples_pandas(df: pd.DataFrame) -> list[float]:
    # Counterpart to fp-bench dataframe_ops/df_itertuples (`list(df.itertuples())`).
    # itertuples is pandas' *fast* row iterator -- a namedtuple per row instead of
    # a Series -- so it is the conservative member of this family and the fairer
    # headline of the two.
    def op():
        return len(list(df.itertuples()))

    return time_operation(op)


def bench_df_row_tuples_fastest_pandas(df: pd.DataFrame) -> list[float]:
    # Fairness control for the iterator/callback retry predicate.  A local
    # four-idiom screen found to_records(...).tolist() faster than default
    # itertuples(), itertuples(name=None), and to_numpy()+map(tuple).  It yields
    # the same task-level product -- a fully materialized Python tuple per row,
    # including the index -- without charging pandas for namedtuple machinery.
    def op():
        return len(df.to_records(index=True).tolist())

    return time_operation(op)


def bench_df_apply_row_pandas(df: pd.DataFrame) -> list[float]:
    # Counterpart to fp-bench dataframe_ops/df_apply_row. The Rust row sums the
    # Float64 cells of each row via apply_fn(.., axis=1); the pandas expression of
    # the same user intent is df.apply(<row sum>, axis=1), which invokes a Python
    # callable per row.
    def op():
        return len(df.apply(lambda row: row.sum(), axis=1))

    return time_operation(op)


def bench_series_apply_stateful_pandas(df: pd.DataFrame) -> PairedSamples:
    """Run the fastest pandas route for an ordered, stateful callback.

    A six-route 1M screen compared ``Series.apply``, ``Series.map``,
    ``Series.transform``, an explicit Python loop, ``Series(map(...))``, and
    ``Series(np.fromiter(...))``. ``Series.map`` was the fastest
    task-equivalent route and produced the exact same output and final state.
    Unlike the row-sum apply trap, a reduction cannot preserve this
    recurrence's ordered callback outputs.
    """
    series = pd.Series(np.arange(len(df), dtype=np.int64), copy=False)

    def op():
        state = 0

        def step(value):
            nonlocal state
            state = (state * 31 + int(value)) & 0x7fff_ffff
            return state

        result = series.map(step)
        return result, state

    return time_operation(op)


def _bench_df_explode_pandas(
    df: pd.DataFrame,
    storage_dtype: object,
) -> PairedSamples:
    """Split three-part strings and explode, with setup outside timing."""
    n = len(df)
    values = [f"a{i % 97},b{i % 89},c{i % 83}" for i in range(n)]
    series = pd.Series(values, dtype=storage_dtype)
    return time_operation(lambda: series.str.split(",").explode())


def bench_df_explode_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_df_explode_pandas(df, object)


def bench_df_explode_string_python_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_df_explode_pandas(df, "string[python]")


def bench_df_explode_string_arrow_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_df_explode_pandas(df, "string[pyarrow]")


def bench_astype_str_f64_pandas(df: pd.DataFrame) -> list[float]:
    # Mirrors fp-bench dataframe_ops/astype_str_f64 exactly: a Float64 column
    # holding i * 1.5 for i in 0..rows, cast to str. Built here (not taken from
    # `df`) so both engines format the identical value sequence.
    series = pd.Series(np.arange(len(df), dtype="float64") * 1.5)
    return time_operation(lambda: series.astype(str))


# GroupBy Workloads (pandas)
def bench_groupby_sum_pandas(df: pd.DataFrame) -> list[float]:
    df = df.copy()
    df["key"] = (df["col_0"] % 100).astype("int64")
    return time_operation(lambda: df.groupby("key")["col_1"].sum())

def bench_groupby_mean_pandas(df: pd.DataFrame) -> list[float]:
    df = df.copy()
    df["key"] = (df["col_0"] % 100).astype("int64")
    return time_operation(lambda: df.groupby("key")["col_1"].mean())

def bench_groupby_agg_multi_pandas(df: pd.DataFrame) -> list[float]:
    df = df.copy()
    df["key"] = (df["col_0"] % 100).astype("int64")
    return time_operation(lambda: df.groupby("key").agg({"col_1": ["sum", "mean", "std"]}))

def bench_groupby_transform_mean_pandas(df: pd.DataFrame) -> list[float]:
    df = df.copy()
    df["key"] = (df["col_0"] % 100).astype("int64")
    return time_operation(lambda: df.groupby("key")["col_1"].transform("mean"))

def bench_groupby_mean_str_pandas(df: pd.DataFrame) -> list[float]:
    df = df.copy()
    df["key"] = ("g" + (df["col_0"] % 1000).astype("int64").map(lambda v: f"{v:04}"))
    return time_operation(lambda: df.groupby("key")["col_1"].mean())


def _groupby_str_op_pandas(df: pd.DataFrame, op):
    """Shared setup for str-keyed groupby aggregation benches: key =
    'g{col_0 % 1000:04}' (~1000 distinct), value = col_1 (matches fp-bench)."""
    df = df.copy()
    key_codes = (df["col_0"] % 1000).fillna(0).astype("int64")
    df["key"] = "g" + key_codes.map(lambda v: f"{v:04}")
    # fp-bench constructs SeriesGroupBy inside every timed iteration. Keep the
    # pandas call inline too: reusing `g` would cache its grouper after warmup
    # and compare reduction-only pandas against factorize-plus-reduce Rust.
    return time_operation(lambda: op(df.groupby("key")["col_1"]))


def bench_groupby_median_str_pandas(df: pd.DataFrame) -> list[float]:
    return _groupby_str_op_pandas(df, lambda g: g.median())


def bench_groupby_std_str_pandas(df: pd.DataFrame) -> list[float]:
    return _groupby_str_op_pandas(df, lambda g: g.std())


def bench_groupby_var_str_pandas(df: pd.DataFrame) -> list[float]:
    return _groupby_str_op_pandas(df, lambda g: g.var())


def bench_groupby_min_str_pandas(df: pd.DataFrame) -> list[float]:
    return _groupby_str_op_pandas(df, lambda g: g.min())


def bench_groupby_max_str_pandas(df: pd.DataFrame) -> list[float]:
    return _groupby_str_op_pandas(df, lambda g: g.max())


def bench_groupby_prod_str_pandas(df: pd.DataFrame) -> list[float]:
    return _groupby_str_op_pandas(df, lambda g: g.prod())


def bench_groupby_sem_str_pandas(df: pd.DataFrame) -> list[float]:
    return _groupby_str_op_pandas(df, lambda g: g.sem())


def bench_groupby_skew_str_pandas(df: pd.DataFrame) -> list[float]:
    return _groupby_str_op_pandas(df, lambda g: g.skew())


def bench_groupby_nunique_str_pandas(df: pd.DataFrame) -> list[float]:
    return _groupby_str_op_pandas(df, lambda g: g.nunique())


def bench_groupby_all_str_pandas(df: pd.DataFrame) -> list[float]:
    return _groupby_str_op_pandas(df, lambda g: g.all())


def bench_groupby_rank_str_pandas(df: pd.DataFrame) -> list[float]:
    return _groupby_str_op_pandas(
        df,
        lambda g: g.rank(method="average", ascending=True, na_option="keep"),
    )


def bench_groupby_kurt_str_pandas(df: pd.DataFrame) -> list[float]:
    # pandas 2.2.3 has no direct SeriesGroupBy.kurt method; public apply with
    # Series.kurt is its behavior-equivalent full-call counterpart.
    return _groupby_str_op_pandas(df, lambda g: g.apply(pd.Series.kurt))


def bench_groupby_quantile_str_pandas(df: pd.DataFrame) -> list[float]:
    return _groupby_str_op_pandas(df, lambda g: g.quantile(0.5))


def bench_groupby_unique_str_pandas(df: pd.DataFrame) -> list[float]:
    return _groupby_str_op_pandas(df, lambda g: g.unique())


def bench_groupby_unique_i64_pandas(df: pd.DataFrame) -> list[float]:
    df = df.copy()
    df["key"] = ("g" + (df["col_0"] % 1000).astype("int64").map(lambda v: f"{v:04}"))
    values = df["col_1"].astype("int64") % 50_000
    return time_operation(lambda: values.groupby(df["key"]).unique())


def bench_groupby_multi_str_pandas(df: pd.DataFrame) -> list[float]:
    # One grouper per timed iteration, reused for three reductions, matching
    # fp-bench's cached-within-call SeriesGroupBy shape.
    return _groupby_str_op_pandas(df, lambda g: (g.mean(), g.std(), g.var()))


def bench_groupby_agg3_str_pandas(df: pd.DataFrame) -> list[float]:
    return _groupby_str_op_pandas(df, lambda g: g.agg(["mean", "std", "max"]))


def bench_df_groupby_str_sum_pandas(df: pd.DataFrame) -> list[float]:
    n = len(df)
    gdf = pd.DataFrame(
        {
            "key": pd.Series(np.arange(n) % 1000).map(lambda v: f"g{v:04}"),
            "v0": df["col_0"],
            "v1": df["col_1"],
            "v2": df["col_2"],
        }
    )
    return time_operation(lambda: gdf.groupby("key")[["v0", "v1", "v2"]].sum())


def bench_df_groupby_2key_sum_pandas(df: pd.DataFrame) -> list[float]:
    n = len(df)
    rows = np.arange(n)
    value_cols = ["v0", "v1", "v2"]
    gdf = pd.DataFrame(
        {
            "k1": rows % 100,
            "k2": (rows // 100) % 50,
            "v0": df["col_0"],
            "v1": df["col_1"],
            "v2": df["col_2"],
        }
    )
    return time_operation(lambda: gdf.groupby(["k1", "k2"])[value_cols].sum())


def bench_df_groupby_2strkey_sum_pandas(df: pd.DataFrame) -> list[float]:
    n = len(df)
    rows = np.arange(n)
    value_cols = ["v0", "v1", "v2"]
    gdf = pd.DataFrame(
        {
            "k1": pd.Series(rows % 100).map(lambda v: f"a{v:03}"),
            "k2": pd.Series((rows // 100) % 50).map(lambda v: f"b{v:03}"),
            "v0": df["col_0"],
            "v1": df["col_1"],
            "v2": df["col_2"],
        }
    )
    return time_operation(lambda: gdf.groupby(["k1", "k2"])[value_cols].sum())


def bench_df_groupby_int_var_pandas(df: pd.DataFrame) -> list[float]:
    # Int key (i%1000, fast dense-histogram factorization) + 3 f64 value cols,
    # df.groupby(key).var() — matches fp-bench df_groupby_int_var. A loss here
    # is in the var computation, NOT factorization.
    df = df.copy()
    df["key"] = np.arange(len(df)) % 1000
    cols = ["col_0", "col_1", "col_2"]
    return time_operation(lambda: df.groupby("key")[cols].var())


def bench_df_groupby_int_mean_pandas(df: pd.DataFrame) -> list[float]:
    df = df.copy()
    df["key"] = np.arange(len(df)) % 1000
    cols = ["col_0", "col_1", "col_2"]
    return time_operation(lambda: df.groupby("key")[cols].mean())


def _widekey(n: int) -> "np.ndarray":
    # Matches fp-bench: (i * golden) as i64 >> 1 — ~n distinct keys, spread
    # across the i64 range (exercises the non-dense wide-i64 factorization).
    return (np.arange(n, dtype=np.uint64) * np.uint64(0x9E37_79B9_7F4A_7C15)).astype(
        np.int64
    ) >> 1


def bench_groupby_widekey_sum_pandas(df: pd.DataFrame) -> list[float]:
    df = df.copy()
    df["key"] = _widekey(len(df))
    return time_operation(lambda: df.groupby("key")["col_1"].sum())


def bench_df_groupby_widekey_sum_pandas(df: pd.DataFrame) -> list[float]:
    df = df.copy()
    df["key"] = _widekey(len(df))
    cols = ["col_0", "col_1", "col_2"]
    return time_operation(lambda: df.groupby("key")[cols].sum())

def bench_groupby_transform_mean_str_pandas(df: pd.DataFrame) -> list[float]:
    df = df.copy()
    df["key"] = ("g" + (df["col_0"] % 1000).astype("int64").map(lambda v: f"{v:04}"))
    return time_operation(lambda: df.groupby("key")["col_1"].transform("mean"))

def bench_groupby_cumcount_pandas(df: pd.DataFrame) -> list[float]:
    df = df.copy()
    df["key"] = (df["col_0"] % 100).astype("int64")
    return time_operation(lambda: df.groupby("key").cumcount())

def bench_groupby_count_pandas(df: pd.DataFrame) -> list[float]:
    df = df.copy()
    df["key"] = (df["col_0"] % 100).astype("int64")
    return time_operation(lambda: df.groupby("key")["col_1"].count())


# Rolling Workloads (pandas)
def bench_rolling_mean_w10_pandas(df: pd.DataFrame) -> list[float]:
    return time_operation(lambda: df["col_0"].rolling(10).mean())

def bench_rolling_std_w50_pandas(df: pd.DataFrame) -> list[float]:
    return time_operation(lambda: df["col_0"].rolling(50).std())


def bench_rolling_apply_stateful_pandas(df: pd.DataFrame) -> PairedSamples:
    """Run the fastest pandas route for an ordered rolling callback.

    An eight-route 1M screen compared exact ``Rolling.apply`` with ``raw=True``
    and ``raw=False`` against task-equivalent ``rolling.sum()`` followed by
    ``Series.map``, ``Series.apply``, ``Series.transform``, ``np.fromiter``,
    ``itertools.accumulate``, and ``ufunc.accumulate``. Building the rolling
    sums once and driving a stateful generator into ``np.fromiter`` was
    fastest. Every screened route produced the same Float64 Series and final
    state.
    """
    window = 10
    mask = 0x7FFF_FFFF
    series = pd.Series(np.arange(len(df), dtype=np.int64) % 997, copy=False)

    def op():
        state = 0
        valid_sums = (
            series.rolling(window, min_periods=window)
            .sum()
            .to_numpy(copy=False)[window - 1 :]
        )

        def values():
            nonlocal state
            for value in valid_sums:
                state = (state * 31 + int(value)) & mask
                yield float(state)

        computed = np.fromiter(
            values(),
            dtype=np.float64,
            count=len(valid_sums),
        )
        output = np.empty(len(series), dtype=np.float64)
        output[: window - 1] = np.nan
        output[window - 1 :] = computed
        return pd.Series(output, index=series.index, copy=False), state

    return time_operation(op)


def bench_expanding_sum_pandas(df: pd.DataFrame) -> list[float]:
    return time_operation(lambda: df["col_0"].expanding().sum())


def bench_expanding_apply_stateful_pandas(df: pd.DataFrame) -> PairedSamples:
    """Run an ordered stateful callback over every growing prefix.

    An eight-route same-worker 1M screen found ``np.fromiter(map(...))`` over
    a stateful scalar callback fastest. It beat a generator-driven
    ``np.fromiter``, ``itertools.accumulate``, ``Series.map``,
    ``Series.apply``, ``Series.transform``, and exact ``Expanding.apply`` with
    both ``raw=True`` and ``raw=False``. Every route produced the same Float64
    Series and final state.
    """
    mask = 0x7FFF_FFFF
    series = pd.Series(np.arange(len(df), dtype=np.int64) % 997, copy=False)
    raw = series.to_numpy(copy=False)

    def op():
        state = 0
        prefix_len = 0

        def step(value):
            nonlocal state, prefix_len
            prefix_len += 1
            state = (state * 31 + int(value) + prefix_len) & mask
            return float(state)

        output = np.fromiter(map(step, raw), dtype=np.float64, count=len(raw))
        return pd.Series(output, index=series.index, copy=False), state

    return time_operation(op)


def bench_ewm_mean_pandas(df: pd.DataFrame) -> list[float]:
    return time_operation(lambda: df["col_0"].ewm(span=10).mean())


# Indexing Workloads (pandas)
def bench_iloc_slice_pandas(df: pd.DataFrame) -> list[float]:
    n = len(df)
    return time_operation(lambda: df.iloc[n//4:3*n//4])

def bench_loc_labels_pandas(df: pd.DataFrame) -> list[float]:
    df = df.set_index(pd.Index(range(len(df))))
    n = len(df)
    labels = list(range(n//4, 3*n//4))
    return time_operation(lambda: df.loc[labels])

def bench_reindex_pandas(df: pd.DataFrame) -> list[float]:
    n = len(df)
    new_index = pd.Index(range(0, n*2, 2))
    return time_operation(lambda: df.reindex(new_index))


def _range_take_positions(n: int) -> np.ndarray:
    start = n // 8
    return np.arange(start, n - start, 2, dtype=np.intp)


def bench_range_index_take_arithmetic_pandas(df: pd.DataFrame) -> list[float]:
    n = len(df)
    idx = pd.RangeIndex(10, 10 + n * 3, 3)
    positions = _range_take_positions(n)

    def op():
        result = None
        for _ in range(TAKE_BATCH):
            result = idx.take(positions)
        return result

    return time_operation(op)


def bench_affine_index_take_arithmetic_pandas(df: pd.DataFrame) -> list[float]:
    n = len(df)
    idx = pd.Index(np.arange(10, 10 + n * 3, 3, dtype=np.int64))
    positions = _range_take_positions(n)

    def op():
        result = None
        for _ in range(TAKE_BATCH):
            result = idx.take(positions)
        return result

    return time_operation(op)


def _build_join_frames(n: int):
    # Mirrors the fp-bench / criterion build_join_frames: left key 0..n, right
    # key 0,2,..,2(n-1) — a unique-key int64 join (inner keeps ~n/2 rows).
    left = pd.DataFrame({
        "key": np.arange(n, dtype=np.int64),
        "left_val": np.arange(n, dtype=np.float64),
    })
    right = pd.DataFrame({
        "key": np.arange(n, dtype=np.int64) * 2,
        "right_val": np.arange(n, dtype=np.float64) * 10.0,
    })
    return left, right

def bench_join_inner_pandas(df: pd.DataFrame) -> list[float]:
    left, right = _build_join_frames(len(df))
    return time_operation(lambda: left.merge(right, on="key", how="inner"))

def bench_join_left_pandas(df: pd.DataFrame) -> list[float]:
    left, right = _build_join_frames(len(df))
    return time_operation(lambda: left.merge(right, on="key", how="left"))

def bench_join_outer_pandas(df: pd.DataFrame) -> list[float]:
    left, right = _build_join_frames(len(df))
    return time_operation(lambda: left.merge(right, on="key", how="outer"))

def _build_str_join_frames(n: int):
    # String-key variant of _build_join_frames: left key "k{i:08}" (unique),
    # right key "k{2i:08}" — ~n/2 inner matches, exercises the Utf8 key path.
    left = pd.DataFrame({
        "key": [f"k{i:08}" for i in range(n)],
        "left_val": np.arange(n, dtype=np.float64),
    })
    right = pd.DataFrame({
        "key": [f"k{i*2:08}" for i in range(n)],
        "right_val": np.arange(n, dtype=np.float64),
    })
    return left, right

def bench_join_inner_str_pandas(df: pd.DataFrame) -> list[float]:
    left, right = _build_str_join_frames(len(df))
    return time_operation(lambda: left.merge(right, on="key", how="inner"))


def pandas_string_backend() -> str:
    """Which pandas string backend the incumbent arm should use.

    `object` is pandas 2.x's default and what this harness has always measured.
    It is ALSO pandas' slow path: measured here at 1M on this exact fixture,
    `string[pyarrow]` is 6.51x faster on sort_values, 3.21x on value_counts and
    1.89x on groupby-sum. Reporting only the object arm compares FrankenPandas
    against the incumbent's worst configuration -- the same class of error as
    quoting `df.apply(..., axis=1)` when `df.sum(axis=1)` exists. pandas 3 makes
    arrow-backed strings the default, so the object arm is a shrinking baseline.

    Set `FP_HARNESS_PANDAS_STRING_BACKEND=arrow` to measure against pandas' best.
    Run BOTH and report fp against the better one; keep object as a labelled
    secondary row, never as the headline. Per br-frankenpandas-ltmk9.
    """
    backend = (
        os.environ.get("FP_HARNESS_PANDAS_STRING_BACKEND", "object")
        .strip()
        .lower()
    )
    if backend not in {"object", "arrow"}:
        raise ValueError(
            f"FP_HARNESS_PANDAS_STRING_BACKEND must be 'object' or 'arrow', got {backend!r}"
        )
    return backend


def _as_string_column(values: list[str], backend: str):
    """Build one key/name column in an explicitly named pandas backend."""
    if backend == "arrow":
        # Fails loudly rather than silently degrading to object: a silent
        # fallback would report an object-arm number as an arrow-arm result.
        return pd.array(values, dtype="string[pyarrow]")
    if backend == "object":
        return values
    raise ValueError(f"unknown pandas string backend: {backend!r}")


def _build_str_frame(n: int, backend: str | None = None) -> pd.DataFrame:
    # Mirrors fp-bench build_str_frame: key = ~1000-distinct group label,
    # name = unique ~15-byte id (sort key), val = float64.
    selected_backend = backend or pandas_string_backend()
    keys = [f"g{i % 1000:04d}" for i in range(n)]
    names = [f"item_{i:010d}" for i in range(n)]
    return pd.DataFrame({
        "key": _as_string_column(keys, selected_backend),
        "name": _as_string_column(names, selected_backend),
        "val": np.arange(n, dtype=np.float64),
    })


def _bench_str_sort_pandas(df: pd.DataFrame, backend: str) -> PairedSamples:
    f = _build_str_frame(len(df), backend)
    return time_operation(lambda: f.sort_values("name"))


def bench_str_sort_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_str_sort_pandas(df, pandas_string_backend())


def bench_str_sort_object_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_str_sort_pandas(df, "object")


def bench_str_sort_arrow_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_str_sort_pandas(df, "arrow")


def _bench_str_value_counts_pandas(
    df: pd.DataFrame,
    backend: str,
) -> PairedSamples:
    f = _build_str_frame(len(df), backend)
    return time_operation(lambda: f["key"].value_counts())


def bench_str_value_counts_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_str_value_counts_pandas(df, pandas_string_backend())


def bench_str_value_counts_object_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_str_value_counts_pandas(df, "object")


def bench_str_value_counts_arrow_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_str_value_counts_pandas(df, "arrow")


def _bench_str_groupby_sum_pandas(
    df: pd.DataFrame,
    backend: str,
) -> PairedSamples:
    f = _build_str_frame(len(df), backend)
    return time_operation(lambda: f.groupby("key")["val"].sum())


def bench_str_groupby_sum_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_str_groupby_sum_pandas(df, pandas_string_backend())


def bench_str_groupby_sum_object_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_str_groupby_sum_pandas(df, "object")


def bench_str_groupby_sum_arrow_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_str_groupby_sum_pandas(df, "arrow")


def bench_str_contains_arrow_pandas(df: pd.DataFrame) -> PairedSamples:
    """Run literal contains on the fastest screened pandas string backend.

    A same-worker 1M screen compared object, ``string[python]``, and
    ``string[pyarrow]`` storage. Arrow was fastest, and all three produced the
    same boolean output. ``regex=False`` matches FrankenPandas' literal
    substring contract without charging pandas for an unnecessary regex.
    """
    names = [f"item_{i:010d}" for i in range(len(df))]
    series = pd.Series(_as_string_column(names, "arrow"))
    return time_operation(lambda: series.str.contains("5", regex=False))


def bench_str_startswith_arrow_pandas(df: pd.DataFrame) -> PairedSamples:
    """Run prefix matching on the fastest screened pandas string backend.

    A same-worker 1M screen compared object, ``string[python]``, and
    ``string[pyarrow]`` storage. Arrow was fastest, and all three produced the
    same all-true boolean output for the exact benchmark names.
    """
    names = [f"item_{i:010d}" for i in range(len(df))]
    series = pd.Series(_as_string_column(names, "arrow"))
    return time_operation(lambda: series.str.startswith("item"))


def bench_df_dot_pandas(df: pd.DataFrame) -> list[float]:
    import math
    dim = math.isqrt(len(df))
    m = pd.DataFrame(np.random.default_rng(7).random((dim, dim)))
    return time_operation(lambda: m.dot(m))


def bench_to_datetime_pandas(df: pd.DataFrame) -> list[float]:
    n = len(df)
    s = pd.Series([f"2020-01-{i % 28 + 1:02d}" for i in range(n)])
    return time_operation(lambda: pd.to_datetime(s))


def bench_dt_floor_pandas(df: pd.DataFrame) -> list[float]:
    n = len(df)
    s = pd.Series(pd.date_range("2000-01-01", periods=n, freq="37s"))
    return time_operation(lambda: s.dt.floor("D"))


# `600s` (10 min) exactly matches fp-bench's in-range nanosecond generator
# through 10M rows. `dt_strftime` is the representation-equivalent incumbent:
# both engines emit `%Y-%m-%d` strings. The same-named pandas `.dt.date` /
# `.dt.time` arms below return Python object arrays, while FrankenPandas returns
# ISO-8601 Utf8, so those two remain diagnostic-only and must not be gated.
def bench_dt_strftime_pandas(df: pd.DataFrame) -> PairedSamples:
    n = len(df)
    s = pd.Series(pd.date_range("2000-01-01", periods=n, freq="600s"))
    return time_operation(lambda: s.dt.strftime("%Y-%m-%d"))


def bench_dt_date_pandas(df: pd.DataFrame) -> list[float]:
    n = len(df)
    s = pd.Series(pd.date_range("2000-01-01", periods=n, freq="600s"))
    return time_operation(lambda: s.dt.date)


def bench_dt_time_pandas(df: pd.DataFrame) -> list[float]:
    n = len(df)
    s = pd.Series(pd.date_range("2000-01-01", periods=n, freq="600s"))
    return time_operation(lambda: s.dt.time)


def bench_dt_day_name_pandas(df: pd.DataFrame) -> PairedSamples:
    """Emit English weekday names through the fastest screened pandas route.

    On the exact 1M-row fixture, ``dt.dayofweek`` followed by a NumPy object
    gather and full Series construction was 10.4x faster than
    ``dt.day_name()`` and produced an exactly equal Series. The name table and
    Datetime64 population remain outside timing.
    """
    n = len(df)
    series = pd.Series(
        pd.date_range("2000-01-01", periods=n, freq="600s"),
        name="d",
    )
    day_names = np.asarray(
        [
            "Monday",
            "Tuesday",
            "Wednesday",
            "Thursday",
            "Friday",
            "Saturday",
            "Sunday",
        ],
        dtype=object,
    )

    def operation():
        codes = series.dt.dayofweek.to_numpy(copy=False)
        return pd.Series(day_names[codes], index=series.index, name=series.name)

    return time_operation(operation)


def bench_dt_month_name_pandas(df: pd.DataFrame) -> PairedSamples:
    """Emit English month names through the fastest screened pandas route.

    On the exact 1M-row fixture, the public DatetimeArray ``month`` property
    followed by a NumPy object gather and full Series construction was 6.9x
    faster than ``dt.month_name()`` and produced an exactly equal Series. The
    name table and Datetime64 population remain outside timing.
    """
    n = len(df)
    series = pd.Series(
        pd.date_range("2000-01-01", periods=n, freq="600s"),
        name="d",
    )
    month_names = np.asarray(
        [
            "January",
            "February",
            "March",
            "April",
            "May",
            "June",
            "July",
            "August",
            "September",
            "October",
            "November",
            "December",
        ],
        dtype=object,
    )

    def operation():
        codes = series.array.month
        return pd.Series(
            month_names[codes - 1],
            index=series.index,
            name=series.name,
        )

    return time_operation(operation)


PANDAS_WORKLOADS = {
    "io": {
        "csv_read": bench_csv_read_pandas,
        "csv_write": bench_csv_write_pandas,
        "json_read_records": bench_json_read_records_pandas,
        "json_read_columns": bench_json_read_columns_pandas,
        "json_read_index": bench_json_read_index_pandas,
        "json_read_split": bench_json_read_split_pandas,
        "json_read_values": bench_json_read_values_pandas,
        "parquet_read": bench_parquet_read_pandas,
        "parquet_write": bench_parquet_write_pandas,
    },
    "dataframe_ops": {
        "sort_values_single": bench_sort_values_single_pandas,
        "sort_values_multi": bench_sort_values_multi_pandas,
        "filter_bool_mask": bench_filter_bool_mask_pandas,
        "drop_duplicates": bench_drop_duplicates_pandas,
        "value_counts": bench_value_counts_pandas,
        "cumsum": bench_cumsum_pandas,
        "df_abs": bench_df_abs_pandas,
        "df_melt": bench_df_melt_pandas,
        "df_transpose": bench_df_transpose_pandas,
        "df_transpose_materialize": bench_df_transpose_materialize_pandas,
        "df_to_dict_index_materialize": bench_df_to_dict_index_materialize_pandas,
        "astype_str_f64": bench_astype_str_f64_pandas,
        "df_iterrows": bench_df_iterrows_pandas,
        "df_itertuples": bench_df_itertuples_pandas,
        "df_row_tuples_fastest": bench_df_row_tuples_fastest_pandas,
        "df_apply_row": bench_df_apply_row_pandas,
        "series_apply_stateful": bench_series_apply_stateful_pandas,
        "df_pivot": bench_df_pivot_pandas,
        "df_pivot_table": bench_df_pivot_table_pandas,
        "df_explode": bench_df_explode_pandas,
        "df_explode_string_python": bench_df_explode_string_python_pandas,
        "df_explode_string_arrow": bench_df_explode_string_arrow_pandas,
    },
    "groupby": {
        "groupby_sum_int64": bench_groupby_sum_pandas,
        "groupby_mean_float64": bench_groupby_mean_pandas,
        "groupby_agg_multi": bench_groupby_agg_multi_pandas,
        "groupby_mean_str": bench_groupby_mean_str_pandas,
        "groupby_transform_mean": bench_groupby_transform_mean_pandas,
        "groupby_transform_mean_str": bench_groupby_transform_mean_str_pandas,
        "groupby_cumcount": bench_groupby_cumcount_pandas,
        "groupby_count": bench_groupby_count_pandas,
        "groupby_median_str": bench_groupby_median_str_pandas,
        "groupby_std_str": bench_groupby_std_str_pandas,
        "groupby_var_str": bench_groupby_var_str_pandas,
        "groupby_min_str": bench_groupby_min_str_pandas,
        "groupby_max_str": bench_groupby_max_str_pandas,
        "groupby_prod_str": bench_groupby_prod_str_pandas,
        "groupby_sem_str": bench_groupby_sem_str_pandas,
        "groupby_skew_str": bench_groupby_skew_str_pandas,
        "groupby_nunique_str": bench_groupby_nunique_str_pandas,
        "groupby_all_str": bench_groupby_all_str_pandas,
        "groupby_rank_str": bench_groupby_rank_str_pandas,
        "groupby_kurt_str": bench_groupby_kurt_str_pandas,
        "groupby_quantile_str": bench_groupby_quantile_str_pandas,
        "groupby_unique_str": bench_groupby_unique_str_pandas,
        "groupby_unique_i64": bench_groupby_unique_i64_pandas,
        "groupby_multi_str": bench_groupby_multi_str_pandas,
        "groupby_agg3_str": bench_groupby_agg3_str_pandas,
        "df_groupby_str_sum": bench_df_groupby_str_sum_pandas,
        "df_groupby_2key_sum": bench_df_groupby_2key_sum_pandas,
        "df_groupby_2strkey_sum": bench_df_groupby_2strkey_sum_pandas,
        "df_groupby_int_var": bench_df_groupby_int_var_pandas,
        "df_groupby_int_mean": bench_df_groupby_int_mean_pandas,
        "groupby_widekey_sum": bench_groupby_widekey_sum_pandas,
        "df_groupby_widekey_sum": bench_df_groupby_widekey_sum_pandas,
    },
    "rolling": {
        "rolling_mean_w10": bench_rolling_mean_w10_pandas,
        "rolling_std_w50": bench_rolling_std_w50_pandas,
        "rolling_apply_stateful": bench_rolling_apply_stateful_pandas,
        "expanding_sum": bench_expanding_sum_pandas,
        "expanding_apply_stateful": bench_expanding_apply_stateful_pandas,
        "ewm_mean": bench_ewm_mean_pandas,
    },
    "indexing": {
        "iloc_slice": bench_iloc_slice_pandas,
        "loc_labels": bench_loc_labels_pandas,
        "reindex": bench_reindex_pandas,
        "range_index_take_arithmetic": bench_range_index_take_arithmetic_pandas,
        "affine_index_take_arithmetic": bench_affine_index_take_arithmetic_pandas,
    },
    "joins": {
        "join_inner": bench_join_inner_pandas,
        "join_left": bench_join_left_pandas,
        "join_outer": bench_join_outer_pandas,
        "join_inner_str": bench_join_inner_str_pandas,
    },
    "strings": {
        "str_sort": bench_str_sort_pandas,
        "str_sort_object": bench_str_sort_object_pandas,
        "str_sort_arrow": bench_str_sort_arrow_pandas,
        "str_value_counts": bench_str_value_counts_pandas,
        "str_value_counts_object": bench_str_value_counts_object_pandas,
        "str_value_counts_arrow": bench_str_value_counts_arrow_pandas,
        "str_groupby_sum": bench_str_groupby_sum_pandas,
        "str_groupby_sum_object": bench_str_groupby_sum_object_pandas,
        "str_groupby_sum_arrow": bench_str_groupby_sum_arrow_pandas,
        "str_contains_arrow": bench_str_contains_arrow_pandas,
        "str_startswith_arrow": bench_str_startswith_arrow_pandas,
    },
    "linalg": {
        "df_dot": bench_df_dot_pandas,
    },
    "datetime": {
        "to_datetime": bench_to_datetime_pandas,
        "dt_floor": bench_dt_floor_pandas,
        "dt_strftime": bench_dt_strftime_pandas,
        "dt_date": bench_dt_date_pandas,
        "dt_time": bench_dt_time_pandas,
        "dt_day_name": bench_dt_day_name_pandas,
        "dt_month_name": bench_dt_month_name_pandas,
    },
}


def run_pandas_workload(
    category: str,
    workload: str,
    size: str,
    dtype: str,
    tmp_path: Path,
    fingerprint: dict[str, Any],
    exclusivity_gate: HostWideExclusivityGate,
) -> tuple[TimingResult, dict[str, Any]]:
    """Run a single pandas workload and return timing result."""
    config = SIZE_CONFIGS[size]
    df = generate_test_data(config["rows"], config["cols"], dtype)

    bench_func = PANDAS_WORKLOADS[category][workload]
    quiescence = exclusivity_gate.require_quiet(
        f"pre_measurement:pandas:{category}/{workload}/{size}/{dtype}"
    )

    if category == "io":
        samples = bench_func(df, tmp_path)
    else:
        samples = bench_func(df)

    if not isinstance(samples, PairedSamples):
        raise TypeError(f"{category}/{workload} did not use the paired timing contract")
    identity = executable_identity(Path(sys.executable))
    return (
        TimingResult(
            workload=workload,
            category=category,
            size=size,
            dtype=dtype,
            engine="pandas",
            times_us=samples.times_us,
            null_arm_a_us=samples.null_arm_a_us,
            null_arm_b_us=samples.null_arm_b_us,
            null_ratios=samples.null_ratios,
            checksum=f"{samples.checksum:016x}",
            executable_sha256=identity["sha256"],
            executable_bytes=identity["bytes"],
            executable_path=identity["path"],
            runtime_available_parallelism=samples.runtime_available_parallelism,
            process_threads_before_probe=samples.process_threads_before_probe,
            peak_process_threads=samples.peak_process_threads,
            operation_threads_used=samples.operation_threads_used,
            runtime_detected_isa_features=fingerprint[
                "runtime_detected_isa_features"
            ],
        ),
        quiescence,
    )


def run_fp_workload_subprocess(category: str, workload: str, size: str,
                               dtype: str) -> TimingResult:
    """Run FrankenPandas workload via subprocess."""
    # Respect CARGO_TARGET_DIR (rch/remote builds set a custom target dir);
    # fall back to the in-tree ./target.
    target_dir = Path(os.environ.get("CARGO_TARGET_DIR", str(PROJECT_ROOT / "target")))
    bench_binary = target_dir / "release-perf" / "fp-bench"

    if not bench_binary.exists():
        print(f"[WARN] fp-bench binary not found at {bench_binary}", file=sys.stderr)
        print("[INFO] Skipping FrankenPandas workload - build with:", file=sys.stderr)
        print("  cargo build --profile release-perf -p fp-bench", file=sys.stderr)
        return TimingResult(
            workload=workload,
            category=category,
            size=size,
            dtype=dtype,
            engine="frankenpandas",
            times_us=[],
        )

    bench_binary = bench_binary.resolve(strict=True)
    # Confine the executable to a trusted root. The project root is one; the
    # configured CARGO_TARGET_DIR is the other, and it is trusted for the same
    # reason this function honours it three lines above -- rch/remote builds and
    # the shared-target-dir disk policy both point it outside the repo, and the
    # in-tree ./target/release-perf/fp-bench is itself a symlink into it. Checking
    # the project root ALONE rejects every valid configuration on this host
    # (CARGO_TARGET_DIR=/data/tmp/cargo-target is set session-wide), which blocks
    # all vs-incumbent measurement. Arbitrary paths are still refused.
    trusted_roots = [PROJECT_ROOT.resolve(strict=True)]
    configured_target = os.environ.get("CARGO_TARGET_DIR")
    if configured_target:
        try:
            trusted_roots.append(Path(configured_target).resolve(strict=True))
        except OSError:
            pass
    if not any(bench_binary.is_relative_to(root) for root in trusted_roots):
        raise ValueError(
            "Refusing fp-bench executable outside the project root and the "
            f"configured CARGO_TARGET_DIR: {bench_binary}"
        )
    if bench_binary.name != "fp-bench":
        raise ValueError(f"Unexpected fp-bench executable path: {bench_binary}")

    # nosec B603: fp-bench is resolved, confined to the project root, and
    # name-checked above; shell=False and category/workload values are selected
    # from the static workload matrix.
    result = subprocess.run(
        [str(bench_binary), "--category", category, "--workload", workload,
         "--size", size, "--dtype", dtype, "--json"],
        capture_output=True,
        check=False,
        text=True,
        timeout=1800 if size == "10M" else 300,
    )

    if result.returncode != 0:
        print(f"[WARN] fp-bench failed: {result.stderr}", file=sys.stderr)
        return TimingResult(
            workload=workload,
            category=category,
            size=size,
            dtype=dtype,
            engine="frankenpandas",
            times_us=[],
        )

    output_lines = result.stdout.splitlines()
    if not output_lines:
        print("[WARN] fp-bench emitted no stdout", file=sys.stderr)
        return TimingResult(
            workload=workload,
            category=category,
            size=size,
            dtype=dtype,
            engine="frankenpandas",
            times_us=[],
        )

    identity_match = re.fullmatch(
        r"bench_elf_sha256=([0-9a-f]{64}) \((\d+) bytes\) (.+)",
        output_lines[0],
    )
    if identity_match is None:
        print(
            f"[WARN] fp-bench missing line-1 ELF identity: {output_lines[0]!r}",
            file=sys.stderr,
        )
        return TimingResult(
            workload=workload,
            category=category,
            size=size,
            dtype=dtype,
            engine="frankenpandas",
            times_us=[],
        )

    try:
        data = json.loads("\n".join(output_lines[1:]))
    except JSONDecodeError as exc:
        print(f"[WARN] fp-bench emitted invalid JSON: {exc}", file=sys.stderr)
        return TimingResult(
            workload=workload,
            category=category,
            size=size,
            dtype=dtype,
            engine="frankenpandas",
            times_us=[],
        )

    null_control = data.get("null_control", {})
    thread_provenance = data.get("thread_provenance", {})
    return TimingResult(
        workload=workload,
        category=category,
        size=size,
        dtype=dtype,
        engine="frankenpandas",
        times_us=data["times_us"],
        null_arm_a_us=null_control.get("arm_a_times_us", []),
        null_arm_b_us=null_control.get("arm_b_times_us", []),
        null_ratios=null_control.get("ratios", []),
        checksum=data.get("checksum"),
        executable_sha256=identity_match.group(1),
        executable_bytes=int(identity_match.group(2)),
        executable_path=identity_match.group(3),
        runtime_available_parallelism=thread_provenance.get(
            "runtime_available_parallelism"
        ),
        process_threads_before_probe=thread_provenance.get(
            "process_threads_before_probe"
        ),
        peak_process_threads=thread_provenance.get("peak_process_threads"),
        operation_threads_used=thread_provenance.get("operation_threads_used"),
        runtime_detected_isa_features=thread_provenance.get(
            "runtime_detected_isa_features",
            [],
        ),
    )


def compute_comparison(fp_result: TimingResult, pd_result: TimingResult,
                       rows: int) -> dict[str, Any]:
    """Compute head-to-head comparison metrics."""
    result = {
        "workload": fp_result.workload,
        "category": fp_result.category,
        "size": fp_result.size,
        "dtype": fp_result.dtype,
    }

    if fp_result.is_valid:
        result["frankenpandas"] = fp_result.to_metrics(rows)
        result["frankenpandas"]["iterations"] = len(fp_result.times_us)
        result["frankenpandas"]["valid"] = True
    else:
        result["frankenpandas"] = {"error": "contract_invalid_or_no_data"}

    if pd_result.is_valid:
        result["pandas"] = pd_result.to_metrics(rows)
        result["pandas"]["iterations"] = len(pd_result.times_us)
        result["pandas"]["valid"] = True
    else:
        result["pandas"] = {"error": "contract_invalid_or_no_data"}

    if fp_result.times_us and pd_result.times_us:
        if fp_result.is_valid and pd_result.is_valid:
            ratio = pd_result.p50_us / fp_result.p50_us if fp_result.p50_us > 0 else 0
            combined_null_log_half_width = max(
                fp_result.null_log_half_width,
                pd_result.null_log_half_width,
            )
            claim_log_effect = abs(math.log(ratio)) if ratio > 0.0 else math.inf
            required_log_effect = DECIDABILITY_MARGIN * combined_null_log_half_width
            decidable = claim_log_effect >= required_log_effect
            result["ratio"] = round(ratio, 3)
            result["median_ci_gate"] = {
                "decidable": decidable,
                "margin_multiplier": DECIDABILITY_MARGIN,
                "claim_log_effect": round(claim_log_effect, 8),
                "required_log_effect": round(required_log_effect, 8),
                "combined_two_x_null_interval": [
                    round(math.exp(-required_log_effect), 6),
                    round(math.exp(required_log_effect), 6),
                ],
                "cv_is_provenance_only": True,
            }
            result["verdict"] = (
                "FASTER" if decidable and ratio > 1.0 else
                "SLOWER" if decidable else
                "NULL_UNDECIDABLE"
            )
        else:
            result["ratio"] = None
            result["verdict"] = "CONTRACT_INVALID"
    else:
        result["ratio"] = None
        result["verdict"] = "INCOMPLETE"

    return result


def build_thread_provenance(
    fingerprint: dict[str, Any],
    requested_thread_count: int | None,
    fp_result: TimingResult,
    pd_result: TimingResult,
) -> dict[str, Any]:
    """Build and validate the mandatory per-cell scaling provenance."""
    affinity_cap = fingerprint["affinity_logical_cpu_cap"]
    errors = []
    if requested_thread_count is not None and affinity_cap != requested_thread_count:
        errors.append(
            "requested thread count does not match the effective affinity cap"
        )
    for result in (fp_result, pd_result):
        if result.runtime_available_parallelism != affinity_cap:
            errors.append(
                f"{result.engine} runtime_available_parallelism="
                f"{result.runtime_available_parallelism} != affinity cap {affinity_cap}"
            )
        if not result.operation_threads_used or result.operation_threads_used < 1:
            errors.append(f"{result.engine} did not report actual operation threads")
        elif result.operation_threads_used > affinity_cap:
            errors.append(
                f"{result.engine} operation thread count "
                f"{result.operation_threads_used} exceeds affinity cap {affinity_cap}"
            )
        if not result.runtime_detected_isa_features:
            errors.append(f"{result.engine} did not report runtime ISA features")

    return {
        "host_identity": fingerprint["host_identity"],
        "cpu_model": fingerprint["cpu_model"],
        "physical_cores": fingerprint["physical_cores"],
        "logical_threads": fingerprint["logical_threads"],
        "thread_count_requested": requested_thread_count,
        "thread_count_actually_used": {
            "frankenpandas": fp_result.operation_threads_used,
            "pandas": pd_result.operation_threads_used,
        },
        "runtime_available_parallelism": {
            "frankenpandas": fp_result.runtime_available_parallelism,
            "pandas": pd_result.runtime_available_parallelism,
        },
        "runtime_detected_isa_features": {
            "host": fingerprint["runtime_detected_isa_features"],
            "frankenpandas": fp_result.runtime_detected_isa_features,
            "pandas": pd_result.runtime_detected_isa_features,
        },
        "affinity_or_cpuset_cap": {
            "logical_cpu_count": affinity_cap,
            "cpu_ids": fingerprint["affinity_cpus"],
        },
        "contract_errors": errors,
        "valid": not errors,
    }


def run_category(category: str, sizes: list[str], dtypes: list[str],
                 tmp_path: Path, fingerprint: dict[str, Any],
                 requested_thread_count: int | None,
                 exclusivity_gate: HostWideExclusivityGate,
                 workload_filter: set[str] | None = None) -> list[dict[str, Any]]:
    """Run all workloads in a category for given sizes and dtypes."""
    results = []
    workloads = PANDAS_WORKLOADS.get(category, {})
    if workload_filter is not None:
        unknown = workload_filter - set(workloads)
        if unknown:
            raise ValueError(f"Unknown workload(s) for {category}: {sorted(unknown)}")
        workloads = {name: func for name, func in workloads.items() if name in workload_filter}

    for workload in workloads:
        for size in sizes:
            for dtype in dtypes:
                config = SIZE_CONFIGS[size]
                print(f"  [{category}] {workload} @ {size}/{dtype}...", end=" ", flush=True)

                pd_result, pandas_quiescence = run_pandas_workload(
                    category,
                    workload,
                    size,
                    dtype,
                    tmp_path,
                    fingerprint,
                    exclusivity_gate,
                )
                fp_quiescence = exclusivity_gate.require_quiet(
                    "pre_measurement:frankenpandas:"
                    f"{category}/{workload}/{size}/{dtype}"
                )
                fp_result = run_fp_workload_subprocess(category, workload, size, dtype)

                comparison = compute_comparison(fp_result, pd_result, config["rows"])
                comparison["host_wide_quiescence"] = {
                    "pandas": pandas_quiescence,
                    "frankenpandas": fp_quiescence,
                    "valid": True,
                }
                comparison["thread_provenance"] = build_thread_provenance(
                    fingerprint,
                    requested_thread_count,
                    fp_result,
                    pd_result,
                )
                if not comparison["thread_provenance"]["valid"]:
                    comparison["verdict"] = "CONTRACT_INVALID"
                    comparison["ratio"] = None
                results.append(comparison)

                verdict = comparison.get("verdict", "N/A")
                ratio = comparison.get("ratio")
                ratio_str = f"{ratio:.2f}x" if ratio else "N/A"
                print(f"{verdict} ({ratio_str})")

    return results


def main():
    harness_identity = executable_identity(Path(sys.executable))
    harness_source_identity = executable_identity(Path(__file__))
    print(
        "bench_elf_sha256="
        f"{harness_identity['sha256']} "
        f"({harness_identity['bytes']} bytes) "
        f"{harness_identity['path']}"
    )
    print(
        "bench_harness_source_sha256="
        f"{harness_source_identity['sha256']} "
        f"({harness_source_identity['bytes']} bytes) "
        f"{harness_source_identity['path']}"
    )

    parser = argparse.ArgumentParser(description="vs-pandas head-to-head timing harness")
    parser.add_argument("--category", choices=list(CATEGORIES.keys()),
                        help="Run specific category")
    parser.add_argument("--all", action="store_true", help="Run all categories")
    parser.add_argument("--sizes", default="10k,100k,1M",
                        help="Comma-separated sizes (10k,100k,1M,2M,4M,6M,8M,10M)")
    parser.add_argument("--dtypes", default="float64",
                        help="Comma-separated dtypes")
    parser.add_argument("--workloads",
                        help="Comma-separated workload names within the selected category")
    parser.add_argument("--output", type=Path, help="Output JSON file")
    parser.add_argument(
        "--json-stdout",
        action="store_true",
        help="Emit one bench_result_json=<compact JSON> line for remote capture",
    )
    parser.add_argument(
        "--dependency-probe",
        action="store_true",
        help="Exit 0 only when the pinned pandas incumbent is importable",
    )
    parser.add_argument(
        "--thread-count",
        type=int,
        help=(
            "Required effective logical-CPU budget for this invocation; the "
            "harness refuses to run unless it equals the process affinity cap"
        ),
    )
    parser.add_argument(
        "--expected-hostname",
        help="Fail closed unless the benchmark host has this exact hostname",
    )
    parser.add_argument(
        "--expected-physical-cores",
        type=int,
        help="Fail closed unless host topology reports this physical-core count",
    )
    parser.add_argument(
        "--expected-logical-threads",
        type=int,
        help="Fail closed unless host topology reports this logical-thread count",
    )
    parser.add_argument(
        "--host-exclusivity-self-test",
        action="store_true",
        help="Exercise the fail-closed host-wide quiescence adjudicator and exit",
    )
    args = parser.parse_args()

    if args.host_exclusivity_self_test:
        _host_wide_exclusivity_self_test()
        print("host_wide_exclusivity_self_test=pass")
        return

    if args.dependency_probe:
        if pd is None:
            print("ERROR: pandas not installed", file=sys.stderr)
            sys.exit(1)
        if pd.__version__ != "2.2.3":
            print(
                f"ERROR: expected pandas 2.2.3, found {pd.__version__}",
                file=sys.stderr,
            )
            sys.exit(1)
        if pa is None:
            print("ERROR: pyarrow not installed", file=sys.stderr)
            sys.exit(1)
        if pa.__version__ != "24.0.0":
            print(
                f"ERROR: expected pyarrow 24.0.0, found {pa.__version__}",
                file=sys.stderr,
            )
            sys.exit(1)
        print(
            "pandas_dependency_probe=ready "
            f"version={pd.__version__} pyarrow_version={pa.__version__}"
        )
        return

    if not args.category and not args.all:
        parser.error("Specify --category or --all")

    if pd is None:
        print("ERROR: pandas not installed", file=sys.stderr)
        sys.exit(1)
    if pa is None:
        print("ERROR: pyarrow not installed", file=sys.stderr)
        sys.exit(1)

    fingerprint = host_fingerprint()
    host_contract_errors = []
    if args.thread_count is not None:
        if args.thread_count < 1:
            host_contract_errors.append("--thread-count must be positive")
        elif fingerprint["affinity_logical_cpu_cap"] != args.thread_count:
            host_contract_errors.append(
                f"thread_count={args.thread_count} but affinity cap is "
                f"{fingerprint['affinity_logical_cpu_cap']}"
            )
    if (
        args.expected_hostname is not None
        and fingerprint["host_identity"] != args.expected_hostname
    ):
        host_contract_errors.append(
            f"hostname={fingerprint['host_identity']!r}, expected "
            f"{args.expected_hostname!r}"
        )
    if (
        args.expected_physical_cores is not None
        and fingerprint["physical_cores"] != args.expected_physical_cores
    ):
        host_contract_errors.append(
            f"physical_cores={fingerprint['physical_cores']}, expected "
            f"{args.expected_physical_cores}"
        )
    if (
        args.expected_logical_threads is not None
        and fingerprint["logical_threads"] != args.expected_logical_threads
    ):
        host_contract_errors.append(
            f"logical_threads={fingerprint['logical_threads']}, expected "
            f"{args.expected_logical_threads}"
        )
    if host_contract_errors:
        for error in host_contract_errors:
            print(f"ERROR: thread provenance contract: {error}", file=sys.stderr)
        sys.exit(2)

    online_cpu_ids = _online_cpu_ids()
    if not online_cpu_ids:
        print(
            "ERROR: host-wide benchmark exclusivity found no online CPUs",
            file=sys.stderr,
        )
        sys.exit(2)
    exclusivity_gate = HostWideExclusivityGate(online_cpu_ids)
    exclusivity_gate.require_quiet("invocation_preflight")

    timestamp = datetime.now(timezone.utc).isoformat()
    invocation_id = (
        "vs-pandas-"
        f"{datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')}-"
        f"pid{os.getpid()}"
    )
    pandas_artifact = pandas_artifact_identity()
    pyarrow_artifact = pyarrow_artifact_identity()
    print(
        "pandas_artifact_sha256="
        f"{pandas_artifact['sha256']} "
        f"({pandas_artifact['bytes']} bytes, "
        f"{pandas_artifact['files']} files) "
        f"{pandas_artifact['path']}"
    )
    print(
        "pyarrow_artifact_sha256="
        f"{pyarrow_artifact['sha256']} "
        f"({pyarrow_artifact['bytes']} bytes, "
        f"{pyarrow_artifact['files']} files) "
        f"{pyarrow_artifact['path']}"
    )
    print(f"bench_invocation_id={invocation_id}")
    print(
        "benchmark_host_fingerprint_json="
        + json.dumps(fingerprint, separators=(",", ":"), sort_keys=True)
    )

    sizes = [s.strip() for s in args.sizes.split(",")]
    dtypes = [d.strip() for d in args.dtypes.split(",")]
    workload_filter = (
        {w.strip() for w in args.workloads.split(",") if w.strip()}
        if args.workloads
        else None
    )
    categories = list(CATEGORIES.keys()) if args.all else [args.category]

    import tempfile
    with tempfile.TemporaryDirectory() as tmp_dir:
        tmp_path = Path(tmp_dir)

        all_results = []

        print("=== vs-pandas Benchmark Harness ===")
        print(f"Timestamp: {timestamp}")
        print(f"Categories: {', '.join(categories)}")
        print(f"Sizes: {', '.join(sizes)}")
        print(f"Dtypes: {', '.join(dtypes)}")
        if workload_filter is not None:
            print(f"Workloads: {', '.join(sorted(workload_filter))}")
        print(f"pandas version: {pd.__version__}")
        print()

        for category in categories:
            print(f"\n[{category.upper()}] (weight: {CATEGORIES[category]})")
            results = run_category(
                category,
                sizes,
                dtypes,
                tmp_path,
                fingerprint,
                args.thread_count,
                exclusivity_gate,
                workload_filter,
            )
            for result in results:
                result["invocation_id"] = invocation_id
            all_results.extend(results)

        exclusivity_gate.require_quiet("invocation_postflight")
        output = {
            "schema_version": "v4",
            "timestamp": timestamp,
            "invocation_id": invocation_id,
            "engine_identity": {
                "frankenpandas": {
                    "version": "0.1.2",
                    "profile": "release-perf",
                    "role": "Subject",
                },
                "pandas": {
                    "version": pd.__version__,
                    "role": "Oracle",
                    "executable": harness_identity,
                    "artifact": pandas_artifact,
                    "optional_backends": {
                        "pyarrow": {
                            "version": pa.__version__,
                            "artifact": pyarrow_artifact,
                        },
                    },
                },
            },
            "harness_source": harness_source_identity,
            "host_fingerprint": fingerprint,
            "host_wide_exclusivity": exclusivity_gate.artifact(),
            "parameters": {
                "sizes": sizes,
                "dtypes": dtypes,
                "categories": categories,
                "paired_rounds": PAIRED_ROUNDS,
                "warmup_iterations": WARMUP_ITERATIONS,
                "bootstrap_resamples": BOOTSTRAP_RESAMPLES,
                "null_ci_confidence": NULL_CI_CONFIDENCE,
                "decidability_margin": DECIDABILITY_MARGIN,
                "gate": "median_bootstrap_ci",
                "cv_role": "provenance_only",
                "host_wide_exclusivity_contract": {
                    "required": True,
                    "scope": "all_online_host_cpus",
                    "maximum_busy_fraction": MAX_HOST_WIDE_BUSY_FRACTION,
                    "sample_interval_ms": round(
                        CPU_SAMPLE_INTERVAL_SECONDS * 1000
                    ),
                    "checks": (
                        "invocation preflight, immediately before each pandas "
                        "and FrankenPandas arm, and invocation postflight"
                    ),
                },
                "pandas_string_backend_policy": {
                    "unsuffixed_workloads": pandas_string_backend(),
                    "explicit_workload_suffixes": {
                        "_object": "object",
                        "_arrow": "string[pyarrow]",
                    },
                },
                "shared_invocation_id": invocation_id,
                "thread_provenance_contract": {
                    "thread_count_requested": args.thread_count,
                    "expected_hostname": args.expected_hostname,
                    "expected_physical_cores": args.expected_physical_cores,
                    "expected_logical_threads": args.expected_logical_threads,
                    "affinity_or_cpuset_cap": {
                        "logical_cpu_count": fingerprint[
                            "affinity_logical_cpu_cap"
                        ],
                        "cpu_ids": fingerprint["affinity_cpus"],
                    },
                },
            },
            "results": all_results,
            "summary": compute_summary(all_results),
        }

        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(json.dumps(output, indent=2))
            print(f"\nResults written to: {args.output}")
        elif not args.json_stdout:
            RESULTS_DIR.mkdir(parents=True, exist_ok=True)
            out_file = RESULTS_DIR / f"bench_{timestamp.replace(':', '-')}.json"
            out_file.write_text(json.dumps(output, indent=2))
            print(f"\nResults written to: {out_file}")
        if args.json_stdout:
            print(
                "bench_result_json="
                + json.dumps(output, separators=(",", ":"), sort_keys=True)
            )


def compute_summary(results: list[dict[str, Any]]) -> dict[str, Any]:
    """Compute weighted summary scores per category."""
    from collections import defaultdict
    import math

    by_category = defaultdict(list)
    for r in results:
        if r.get("verdict") in ("FASTER", "SLOWER"):
            by_category[r["category"]].append(r["ratio"])

    category_scores = {}
    for cat, ratios in by_category.items():
        if ratios:
            geomean = math.exp(sum(math.log(r) for r in ratios) / len(ratios))
            category_scores[cat] = round(geomean, 3)

    weighted_score = sum(
        category_scores.get(cat, 1.0) * weight
        for cat, weight in CATEGORIES.items()
    )

    contract_valid_count = sum(
        1
        for r in results
        if r.get("verdict") not in ("CONTRACT_INVALID", "INCOMPLETE")
    )
    decidable_count = sum(
        1 for r in results if r.get("verdict") in ("FASTER", "SLOWER")
    )
    null_count = sum(
        1 for r in results if r.get("verdict") == "NULL_UNDECIDABLE"
    )

    return {
        "total_workloads": len(results),
        "contract_valid_workloads": contract_valid_count,
        "decidable_workloads": decidable_count,
        "null_undecidable_workloads": null_count,
        "category_scores": category_scores,
        "weighted_score": round(weighted_score, 3),
        "claim_validated": all(
            category_scores.get(cat, 0) > 1.0
            for cat in CATEGORIES
        ),
    }


if __name__ == "__main__":
    main()
