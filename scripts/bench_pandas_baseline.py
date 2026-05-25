#!/usr/bin/env python3
"""
Pandas baseline benchmarks — mirrors frankenpandas vs_pandas.rs benchmarks.

Runs each operation 20+ times and reports p50/p95/p99 + throughput.
Output: JSON for comparison with Rust criterion results.
"""

import json
import os
import platform
import statistics
import sys
import time
from dataclasses import dataclass
from io import StringIO
from typing import Callable

import numpy as np
import pandas as pd

SIZES = [10_000, 100_000]
MIN_RUNS = 20
WARMUP_RUNS = 3


@dataclass
class BenchResult:
    name: str
    size: int
    times_ns: list[float]

    @property
    def p50_ns(self) -> float:
        return statistics.median(self.times_ns)

    @property
    def p95_ns(self) -> float:
        sorted_times = sorted(self.times_ns)
        idx = int(0.95 * len(sorted_times))
        return sorted_times[min(idx, len(sorted_times) - 1)]

    @property
    def p99_ns(self) -> float:
        sorted_times = sorted(self.times_ns)
        idx = int(0.99 * len(sorted_times))
        return sorted_times[min(idx, len(sorted_times) - 1)]

    @property
    def throughput_ops_per_sec(self) -> float:
        return 1e9 / self.p50_ns if self.p50_ns > 0 else 0

    @property
    def p50_ms(self) -> float:
        return self.p50_ns / 1e6

    @property
    def p95_ms(self) -> float:
        return self.p95_ns / 1e6

    @property
    def p99_ms(self) -> float:
        return self.p99_ns / 1e6

    def to_dict(self) -> dict:
        return {
            "name": self.name,
            "size": self.size,
            "runs": len(self.times_ns),
            "p50_ns": self.p50_ns,
            "p95_ns": self.p95_ns,
            "p99_ns": self.p99_ns,
            "p50_ms": self.p50_ns / 1e6,
            "p95_ms": self.p95_ns / 1e6,
            "p99_ms": self.p99_ns / 1e6,
            "throughput_ops_per_sec": self.throughput_ops_per_sec,
            "stddev_ns": statistics.stdev(self.times_ns) if len(self.times_ns) > 1 else 0,
        }


def bench(name: str, size: int, setup: Callable, op: Callable, runs: int = MIN_RUNS) -> BenchResult:
    """Run a benchmark with warmup and collect timing data."""
    data = setup(size)

    # Warmup
    for _ in range(WARMUP_RUNS):
        op(data)

    # Timed runs
    times = []
    for _ in range(runs):
        start = time.perf_counter_ns()
        op(data)
        end = time.perf_counter_ns()
        times.append(end - start)

    return BenchResult(name, size, times)


# ============================================================================
# DATA GENERATION (mirrors Rust helpers)
# ============================================================================

def build_numeric_frame(n: int, cols: int = 10) -> pd.DataFrame:
    data = {f"c{c}": np.arange(n) * (c + 1) * 0.1 for c in range(cols)}
    return pd.DataFrame(data)


def build_groupby_frame(n: int, num_groups: int = 100) -> pd.DataFrame:
    return pd.DataFrame({
        "k": np.arange(n) % num_groups,
        "v": np.arange(n) * 0.1,
    })


def build_join_frames(n: int) -> tuple[pd.DataFrame, pd.DataFrame]:
    left = pd.DataFrame({
        "key": np.arange(n),
        "left_val": np.arange(n, dtype=float),
    })
    right = pd.DataFrame({
        "key": np.arange(n) * 2,
        "right_val": np.arange(n, dtype=float) * 10,
    })
    return left, right


def build_series(n: int) -> pd.Series:
    return pd.Series(np.arange(n) * 0.1, name="s")


def build_csv_string(n: int, cols: int = 10) -> str:
    df = build_numeric_frame(n, cols)
    return df.to_csv(index=False)


# ============================================================================
# IO BENCHMARKS
# ============================================================================

def bench_io_csv_read(size: int) -> BenchResult:
    csv_str = build_csv_string(size, 10)
    return bench(
        "io/csv_read", size,
        lambda n: csv_str,
        lambda csv: pd.read_csv(StringIO(csv))
    )


def bench_io_csv_write(size: int) -> BenchResult:
    return bench(
        "io/csv_write", size,
        lambda n: build_numeric_frame(n, 10),
        lambda df: df.to_csv(index=False)
    )


def bench_io_json_write(size: int) -> BenchResult:
    return bench(
        "io/json_write", size,
        lambda n: build_numeric_frame(n, 10),
        lambda df: df.to_json(orient="records")
    )


# ============================================================================
# DATAFRAME OPS BENCHMARKS
# ============================================================================

def bench_df_sort_single(size: int) -> BenchResult:
    return bench(
        "dataframe_ops/sort_single", size,
        lambda n: build_numeric_frame(n, 10),
        lambda df: df.sort_values("c0")
    )


def bench_df_sort_multi(size: int) -> BenchResult:
    return bench(
        "dataframe_ops/sort_multi", size,
        lambda n: build_numeric_frame(n, 10),
        lambda df: df.sort_values(["c0", "c1"], ascending=[True, False])
    )


def bench_df_filter_bool(size: int) -> BenchResult:
    return bench(
        "dataframe_ops/filter_bool", size,
        lambda n: build_numeric_frame(n, 10),
        lambda df: df[df["c0"] > df["c0"].median()]
    )


def bench_df_drop_duplicates(size: int) -> BenchResult:
    return bench(
        "dataframe_ops/drop_duplicates", size,
        lambda n: build_numeric_frame(n, 10),
        lambda df: df.drop_duplicates()
    )


def bench_df_value_counts(size: int) -> BenchResult:
    def setup(n):
        df = build_numeric_frame(n, 10)
        df["cat"] = (np.arange(n) % 100).astype(str)
        return df
    return bench(
        "dataframe_ops/value_counts", size,
        setup,
        lambda df: df["cat"].value_counts()
    )


def bench_df_cumsum(size: int) -> BenchResult:
    return bench(
        "dataframe_ops/cumsum", size,
        lambda n: build_numeric_frame(n, 10),
        lambda df: df.cumsum()
    )


# ============================================================================
# GROUPBY BENCHMARKS
# ============================================================================

def bench_groupby_sum(size: int) -> BenchResult:
    return bench(
        "groupby/sum", size,
        lambda n: build_groupby_frame(n, 100),
        lambda df: df.groupby("k")["v"].sum()
    )


def bench_groupby_mean(size: int) -> BenchResult:
    return bench(
        "groupby/mean", size,
        lambda n: build_groupby_frame(n, 100),
        lambda df: df.groupby("k")["v"].mean()
    )


def bench_groupby_agg_multi(size: int) -> BenchResult:
    return bench(
        "groupby/agg_multi", size,
        lambda n: build_groupby_frame(n, 100),
        lambda df: df.groupby("k")["v"].agg(["sum", "mean", "std", "min", "max"])
    )


def bench_groupby_ngroup(size: int) -> BenchResult:
    return bench(
        "groupby/ngroup", size,
        lambda n: build_groupby_frame(n, 100),
        lambda df: df.groupby("k").ngroup()
    )


# ============================================================================
# JOIN BENCHMARKS
# ============================================================================

def bench_join_inner(size: int) -> BenchResult:
    left, right = build_join_frames(size)
    return bench(
        "joins/inner", size,
        lambda n: (left.copy(), right.copy()),
        lambda data: pd.merge(data[0], data[1], on="key", how="inner")
    )


def bench_join_left(size: int) -> BenchResult:
    left, right = build_join_frames(size)
    return bench(
        "joins/left", size,
        lambda n: (left.copy(), right.copy()),
        lambda data: pd.merge(data[0], data[1], on="key", how="left")
    )


def bench_join_outer(size: int) -> BenchResult:
    left, right = build_join_frames(size)
    return bench(
        "joins/outer", size,
        lambda n: (left.copy(), right.copy()),
        lambda data: pd.merge(data[0], data[1], on="key", how="outer")
    )


def bench_concat_axis0(size: int) -> BenchResult:
    frames = [build_numeric_frame(size // 4, 10) for _ in range(4)]
    return bench(
        "joins/concat_axis0", size,
        lambda n: frames,
        lambda dfs: pd.concat(dfs, axis=0)
    )


def bench_concat_axis1(size: int) -> BenchResult:
    frames = [build_numeric_frame(size, 3) for _ in range(4)]
    return bench(
        "joins/concat_axis1", size,
        lambda n: frames,
        lambda dfs: pd.concat(dfs, axis=1)
    )


# ============================================================================
# ROLLING BENCHMARKS
# ============================================================================

def bench_rolling_mean(size: int) -> BenchResult:
    return bench(
        "rolling/mean", size,
        lambda n: build_series(n),
        lambda s: s.rolling(100).mean()
    )


def bench_rolling_std(size: int) -> BenchResult:
    return bench(
        "rolling/std", size,
        lambda n: build_series(n),
        lambda s: s.rolling(100).std()
    )


def bench_expanding_sum(size: int) -> BenchResult:
    return bench(
        "rolling/expanding_sum", size,
        lambda n: build_series(n),
        lambda s: s.expanding().sum()
    )


def bench_ewm_mean(size: int) -> BenchResult:
    return bench(
        "rolling/ewm_mean", size,
        lambda n: build_series(n),
        lambda s: s.ewm(span=20).mean()
    )


# ============================================================================
# INDEXING BENCHMARKS
# ============================================================================

def bench_iloc_slice(size: int) -> BenchResult:
    return bench(
        "indexing/iloc_slice", size,
        lambda n: build_numeric_frame(n, 10),
        lambda df: df.iloc[100:size - 100]
    )


def bench_loc_labels(size: int) -> BenchResult:
    labels = list(range(0, min(10000, size), 10))
    return bench(
        "indexing/loc_labels", size,
        lambda n: build_numeric_frame(n, 10),
        lambda df: df.loc[labels]
    )


def bench_at_scalar(size: int) -> BenchResult:
    return bench(
        "indexing/at_scalar", size,
        lambda n: build_series(n),
        lambda s: s.at[size // 2]
    )


def bench_reindex(size: int) -> BenchResult:
    new_idx = [(i * 3) % (size * 2) for i in range(size)]
    return bench(
        "indexing/reindex", size,
        lambda n: build_series(n),
        lambda s: s.reindex(new_idx)
    )


# ============================================================================
# MAIN
# ============================================================================

ALL_BENCHMARKS = [
    # IO
    bench_io_csv_read,
    bench_io_csv_write,
    bench_io_json_write,
    # DataFrame ops
    bench_df_sort_single,
    bench_df_sort_multi,
    bench_df_filter_bool,
    bench_df_drop_duplicates,
    bench_df_value_counts,
    bench_df_cumsum,
    # GroupBy
    bench_groupby_sum,
    bench_groupby_mean,
    bench_groupby_agg_multi,
    bench_groupby_ngroup,
    # Joins
    bench_join_inner,
    bench_join_left,
    bench_join_outer,
    bench_concat_axis0,
    bench_concat_axis1,
    # Rolling
    bench_rolling_mean,
    bench_rolling_std,
    bench_expanding_sum,
    bench_ewm_mean,
    # Indexing
    bench_iloc_slice,
    bench_loc_labels,
    bench_at_scalar,
    bench_reindex,
]


def environment_fingerprint() -> dict:
    """Capture environment for reproducibility."""
    return {
        "python_version": sys.version,
        "pandas_version": pd.__version__,
        "numpy_version": np.__version__,
        "platform": platform.platform(),
        "processor": platform.processor(),
        "cpu_count": os.cpu_count(),
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    }


def main():
    print(f"Running pandas {pd.__version__} benchmarks...", file=sys.stderr)
    print(f"Sizes: {SIZES}, Runs per benchmark: {MIN_RUNS}", file=sys.stderr)

    results = []
    env = environment_fingerprint()

    for bench_fn in ALL_BENCHMARKS:
        for size in SIZES:
            print(f"  {bench_fn.__name__}(n={size})...", file=sys.stderr, end=" ")
            try:
                result = bench_fn(size)
                results.append(result.to_dict())
                print(f"p50={result.p50_ms:.2f}ms", file=sys.stderr)
            except Exception as e:
                print(f"FAILED: {e}", file=sys.stderr)
                results.append({
                    "name": bench_fn.__name__.replace("bench_", "").replace("_", "/", 1),
                    "size": size,
                    "error": str(e),
                })

    output = {
        "environment": env,
        "results": results,
    }

    print(json.dumps(output, indent=2))


if __name__ == "__main__":
    main()
