#!/usr/bin/env python3
"""vs-pandas head-to-head timing harness.

Runs identical workloads on both FrankenPandas (Rust, release-perf) and
pandas 2.2.3, capturing p50/p95/p99 + cv_pct + throughput per engine.

Per BENCH_MATRIX_SPEC.md:
- Uses release-perf profile for FP (not --release)
- Wraps BOTH engines in identical retry shells
- Drops results with cv > 5% (noise)
- Population/setup OUTSIDE the timed window
- EngineIdentity Subject!=Oracle on every artifact

Usage:
    python benches/vs_pandas_harness.py --category io --size 100k
    python benches/vs_pandas_harness.py --all --sizes 10k,100k
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from statistics import mean, stdev
from typing import Any

import numpy as np

try:
    import pandas as pd
except ImportError:
    pd = None

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
}

SIZE_CONFIGS = {
    "10k": {"rows": 10_000, "cols": 10},
    "100k": {"rows": 100_000, "cols": 10},
    "1M": {"rows": 1_000_000, "cols": 10},
}

CV_THRESHOLD = 5.0  # Drop results with cv > 5%
MIN_ITERATIONS = 10
MAX_ITERATIONS = 100
WARMUP_ITERATIONS = 3


@dataclass
class TimingResult:
    """Raw timing measurements for a single workload."""
    workload: str
    category: str
    size: str
    dtype: str
    engine: str
    times_us: list[float] = field(default_factory=list)

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
    def is_valid(self) -> bool:
        return self.cv_pct <= CV_THRESHOLD

    def to_metrics(self, rows: int) -> dict[str, Any]:
        return {
            "p50_us": round(self.p50_us, 2),
            "p95_us": round(self.p95_us, 2),
            "p99_us": round(self.p99_us, 2),
            "mean_us": round(self.mean_us, 2),
            "stddev_us": round(self.stddev_us, 2),
            "cv_pct": round(self.cv_pct, 2),
            "throughput_rows_sec": round(rows / (self.p50_us / 1_000_000)),
        }


def generate_test_data(rows: int, cols: int, dtype: str, seed: int = 42) -> pd.DataFrame:
    """Generate test DataFrame OUTSIDE the timed window."""
    rng = np.random.default_rng(seed)

    if dtype == "int64":
        data = {f"col_{i}": rng.integers(0, 1_000_000, size=rows) for i in range(cols)}
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
    else:
        raise ValueError(f"Unknown dtype: {dtype}")

    return pd.DataFrame(data)


def time_operation(func, warmup: int = WARMUP_ITERATIONS,
                   min_iters: int = MIN_ITERATIONS,
                   max_iters: int = MAX_ITERATIONS) -> list[float]:
    """Time an operation with warmup and adaptive iterations."""
    for _ in range(warmup):
        func()

    times = []
    for _ in range(max_iters):
        start = time.perf_counter_ns()
        func()
        elapsed_us = (time.perf_counter_ns() - start) / 1000
        times.append(elapsed_us)

        if len(times) >= min_iters:
            cv = (stdev(times) / mean(times) * 100) if mean(times) > 0 else 0
            if cv <= CV_THRESHOLD:
                break

    return times


# IO Workloads (pandas)
def bench_csv_read_pandas(df: pd.DataFrame, tmp_path: Path) -> float:
    csv_path = tmp_path / "bench.csv"
    df.to_csv(csv_path, index=False)
    return time_operation(lambda: pd.read_csv(csv_path))

def bench_csv_write_pandas(df: pd.DataFrame, tmp_path: Path) -> float:
    csv_path = tmp_path / "bench_out.csv"
    return time_operation(lambda: df.to_csv(csv_path, index=False))

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


# Rolling Workloads (pandas)
def bench_rolling_mean_w10_pandas(df: pd.DataFrame) -> list[float]:
    return time_operation(lambda: df["col_0"].rolling(10).mean())

def bench_rolling_std_w50_pandas(df: pd.DataFrame) -> list[float]:
    return time_operation(lambda: df["col_0"].rolling(50).std())

def bench_expanding_sum_pandas(df: pd.DataFrame) -> list[float]:
    return time_operation(lambda: df["col_0"].expanding().sum())

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


def _build_str_frame(n: int) -> pd.DataFrame:
    # Mirrors fp-bench build_str_frame: key = ~1000-distinct group label,
    # name = unique ~15-byte id (sort key), val = float64.
    keys = [f"g{i % 1000:04d}" for i in range(n)]
    names = [f"item_{i:010d}" for i in range(n)]
    return pd.DataFrame({
        "key": keys,
        "name": names,
        "val": np.arange(n, dtype=np.float64),
    })

def bench_str_sort_pandas(df: pd.DataFrame) -> list[float]:
    f = _build_str_frame(len(df))
    return time_operation(lambda: f.sort_values("name"))

def bench_str_value_counts_pandas(df: pd.DataFrame) -> list[float]:
    f = _build_str_frame(len(df))
    return time_operation(lambda: f["key"].value_counts())

def bench_str_groupby_sum_pandas(df: pd.DataFrame) -> list[float]:
    f = _build_str_frame(len(df))
    return time_operation(lambda: f.groupby("key")["val"].sum())


PANDAS_WORKLOADS = {
    "io": {
        "csv_read": bench_csv_read_pandas,
        "csv_write": bench_csv_write_pandas,
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
    },
    "groupby": {
        "groupby_sum_int64": bench_groupby_sum_pandas,
        "groupby_mean_float64": bench_groupby_mean_pandas,
        "groupby_agg_multi": bench_groupby_agg_multi_pandas,
    },
    "rolling": {
        "rolling_mean_w10": bench_rolling_mean_w10_pandas,
        "rolling_std_w50": bench_rolling_std_w50_pandas,
        "expanding_sum": bench_expanding_sum_pandas,
        "ewm_mean": bench_ewm_mean_pandas,
    },
    "indexing": {
        "iloc_slice": bench_iloc_slice_pandas,
        "loc_labels": bench_loc_labels_pandas,
        "reindex": bench_reindex_pandas,
    },
    "joins": {
        "join_inner": bench_join_inner_pandas,
        "join_left": bench_join_left_pandas,
        "join_outer": bench_join_outer_pandas,
    },
    "strings": {
        "str_sort": bench_str_sort_pandas,
        "str_value_counts": bench_str_value_counts_pandas,
        "str_groupby_sum": bench_str_groupby_sum_pandas,
    },
}


def run_pandas_workload(category: str, workload: str, size: str,
                        dtype: str, tmp_path: Path) -> TimingResult:
    """Run a single pandas workload and return timing result."""
    config = SIZE_CONFIGS[size]
    df = generate_test_data(config["rows"], config["cols"], dtype)

    bench_func = PANDAS_WORKLOADS[category][workload]

    if category == "io":
        times = bench_func(df, tmp_path)
    else:
        times = bench_func(df)

    return TimingResult(
        workload=workload,
        category=category,
        size=size,
        dtype=dtype,
        engine="pandas",
        times_us=times,
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
        print(f"  cargo build --profile release-perf -p fp-bench", file=sys.stderr)
        return TimingResult(
            workload=workload,
            category=category,
            size=size,
            dtype=dtype,
            engine="frankenpandas",
            times_us=[],
        )

    result = subprocess.run(
        [str(bench_binary), "--category", category, "--workload", workload,
         "--size", size, "--dtype", dtype, "--json"],
        capture_output=True,
        text=True,
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

    data = json.loads(result.stdout)
    return TimingResult(
        workload=workload,
        category=category,
        size=size,
        dtype=dtype,
        engine="frankenpandas",
        times_us=data["times_us"],
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

    if fp_result.times_us:
        result["frankenpandas"] = fp_result.to_metrics(rows)
        result["frankenpandas"]["iterations"] = len(fp_result.times_us)
        result["frankenpandas"]["valid"] = fp_result.is_valid
    else:
        result["frankenpandas"] = {"error": "no_data"}

    if pd_result.times_us:
        result["pandas"] = pd_result.to_metrics(rows)
        result["pandas"]["iterations"] = len(pd_result.times_us)
        result["pandas"]["valid"] = pd_result.is_valid
    else:
        result["pandas"] = {"error": "no_data"}

    if fp_result.times_us and pd_result.times_us:
        if fp_result.is_valid and pd_result.is_valid:
            ratio = pd_result.p50_us / fp_result.p50_us if fp_result.p50_us > 0 else 0
            result["ratio"] = round(ratio, 3)
            result["verdict"] = (
                "FASTER" if ratio > 1.05 else
                "PARITY" if ratio >= 0.95 else
                "SLOWER"
            )
        else:
            result["ratio"] = None
            result["verdict"] = "DROPPED_HIGH_CV"
    else:
        result["ratio"] = None
        result["verdict"] = "INCOMPLETE"

    return result


def run_category(category: str, sizes: list[str], dtypes: list[str],
                 tmp_path: Path) -> list[dict[str, Any]]:
    """Run all workloads in a category for given sizes and dtypes."""
    results = []
    workloads = PANDAS_WORKLOADS.get(category, {})

    for workload in workloads:
        for size in sizes:
            for dtype in dtypes:
                config = SIZE_CONFIGS[size]
                print(f"  [{category}] {workload} @ {size}/{dtype}...", end=" ", flush=True)

                pd_result = run_pandas_workload(category, workload, size, dtype, tmp_path)
                fp_result = run_fp_workload_subprocess(category, workload, size, dtype)

                comparison = compute_comparison(fp_result, pd_result, config["rows"])
                results.append(comparison)

                verdict = comparison.get("verdict", "N/A")
                ratio = comparison.get("ratio")
                ratio_str = f"{ratio:.2f}x" if ratio else "N/A"
                print(f"{verdict} ({ratio_str})")

    return results


def main():
    parser = argparse.ArgumentParser(description="vs-pandas head-to-head timing harness")
    parser.add_argument("--category", choices=list(CATEGORIES.keys()),
                        help="Run specific category")
    parser.add_argument("--all", action="store_true", help="Run all categories")
    parser.add_argument("--sizes", default="10k,100k",
                        help="Comma-separated sizes (10k,100k,1M)")
    parser.add_argument("--dtypes", default="float64",
                        help="Comma-separated dtypes")
    parser.add_argument("--output", type=Path, help="Output JSON file")
    args = parser.parse_args()

    if not args.category and not args.all:
        parser.error("Specify --category or --all")

    if pd is None:
        print("ERROR: pandas not installed", file=sys.stderr)
        sys.exit(1)

    sizes = [s.strip() for s in args.sizes.split(",")]
    dtypes = [d.strip() for d in args.dtypes.split(",")]
    categories = list(CATEGORIES.keys()) if args.all else [args.category]

    import tempfile
    with tempfile.TemporaryDirectory() as tmp_dir:
        tmp_path = Path(tmp_dir)

        all_results = []
        timestamp = datetime.now(timezone.utc).isoformat()

        print(f"=== vs-pandas Benchmark Harness ===")
        print(f"Timestamp: {timestamp}")
        print(f"Categories: {', '.join(categories)}")
        print(f"Sizes: {', '.join(sizes)}")
        print(f"Dtypes: {', '.join(dtypes)}")
        print(f"pandas version: {pd.__version__}")
        print()

        for category in categories:
            print(f"\n[{category.upper()}] (weight: {CATEGORIES[category]})")
            results = run_category(category, sizes, dtypes, tmp_path)
            all_results.extend(results)

        output = {
            "schema_version": "v3",
            "timestamp": timestamp,
            "engine_identity": {
                "frankenpandas": {
                    "version": "0.1.0",
                    "profile": "release-perf",
                    "role": "Subject",
                },
                "pandas": {
                    "version": pd.__version__,
                    "role": "Oracle",
                },
            },
            "parameters": {
                "sizes": sizes,
                "dtypes": dtypes,
                "categories": categories,
                "cv_threshold": CV_THRESHOLD,
                "min_iterations": MIN_ITERATIONS,
                "warmup_iterations": WARMUP_ITERATIONS,
            },
            "results": all_results,
            "summary": compute_summary(all_results),
        }

        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(json.dumps(output, indent=2))
            print(f"\nResults written to: {args.output}")
        else:
            RESULTS_DIR.mkdir(parents=True, exist_ok=True)
            out_file = RESULTS_DIR / f"bench_{timestamp.replace(':', '-')}.json"
            out_file.write_text(json.dumps(output, indent=2))
            print(f"\nResults written to: {out_file}")


def compute_summary(results: list[dict[str, Any]]) -> dict[str, Any]:
    """Compute weighted summary scores per category."""
    from collections import defaultdict
    import math

    by_category = defaultdict(list)
    for r in results:
        if r.get("ratio") is not None:
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

    valid_count = sum(1 for r in results if r.get("verdict") not in ("DROPPED_HIGH_CV", "INCOMPLETE"))
    dropped_count = sum(1 for r in results if r.get("verdict") == "DROPPED_HIGH_CV")

    return {
        "total_workloads": len(results),
        "valid_workloads": valid_count,
        "dropped_high_cv": dropped_count,
        "category_scores": category_scores,
        "weighted_score": round(weighted_score, 3),
        "claim_validated": all(
            category_scores.get(cat, 0) > 1.0
            for cat in CATEGORIES
        ),
    }


if __name__ == "__main__":
    main()
