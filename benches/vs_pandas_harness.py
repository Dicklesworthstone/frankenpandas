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
    python benches/vs_pandas_harness.py --category io --sizes 100k
    python benches/vs_pandas_harness.py --all --sizes 10k,100k,1M
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
}

CV_THRESHOLD = 5.0  # Drop results with cv > 5%
MIN_ITERATIONS = 10
MAX_ITERATIONS = 100
WARMUP_ITERATIONS = 3
TAKE_BATCH = 256
TRANSPOSE_BATCH = 8192


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

def time_operation_repeated(func, repeat: int, warmup: int = WARMUP_ITERATIONS,
                            min_iters: int = MIN_ITERATIONS,
                            max_iters: int = MAX_ITERATIONS) -> list[float]:
    """Time a tiny operation as a fixed-size batch and return batch microseconds."""
    for _ in range(warmup):
        for _ in range(repeat):
            func()

    times = []
    for _ in range(max_iters):
        start = time.perf_counter_ns()
        for _ in range(repeat):
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
    df["key"] = ("g" + (df["col_0"] % 1000).astype("int64").map(lambda v: f"{v:04}"))
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


# `600s` (10 min) makes both the date and the time-of-day vary while keeping 1M
# rows inside datetime64[ns].
#
# NOT YET APPLES-TO-APPLES — do not gate on these rows. fp-bench's `datetime`
# arm generates `base + i * 86_437_000_000_000` nanos, which OVERFLOWS i64 at
# n >= 100_000 (release wraps silently), so pandas cannot build the same series
# (`date_range` raises OutOfBoundsDatetime). The fp-bench generator has to be
# fixed to a non-overflowing step before these two halves measure one workload.
# Until then the pandas half stands alone as a cost anchor for `.dt.date` /
# `.dt.time` on 1M in-range datetimes. Note also that pandas' `.dt.date` /
# `.dt.time` return object arrays of Python `datetime.date` / `datetime.time`,
# whereas FrankenPandas returns an ISO-8601 Utf8 column; the representation-
# equivalent pandas call is `s.dt.strftime(...)`.
def bench_dt_date_pandas(df: pd.DataFrame) -> list[float]:
    n = len(df)
    s = pd.Series(pd.date_range("2000-01-01", periods=n, freq="600s"))
    return time_operation(lambda: s.dt.date)


def bench_dt_time_pandas(df: pd.DataFrame) -> list[float]:
    n = len(df)
    s = pd.Series(pd.date_range("2000-01-01", periods=n, freq="600s"))
    return time_operation(lambda: s.dt.time)


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
        "df_transpose": bench_df_transpose_pandas,
        "df_transpose_materialize": bench_df_transpose_materialize_pandas,
        "df_to_dict_index_materialize": bench_df_to_dict_index_materialize_pandas,
        "astype_str_f64": bench_astype_str_f64_pandas,
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
        "df_groupby_int_var": bench_df_groupby_int_var_pandas,
        "df_groupby_int_mean": bench_df_groupby_int_mean_pandas,
        "groupby_widekey_sum": bench_groupby_widekey_sum_pandas,
        "df_groupby_widekey_sum": bench_df_groupby_widekey_sum_pandas,
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
        "str_value_counts": bench_str_value_counts_pandas,
        "str_groupby_sum": bench_str_groupby_sum_pandas,
    },
    "linalg": {
        "df_dot": bench_df_dot_pandas,
    },
    "datetime": {
        "to_datetime": bench_to_datetime_pandas,
        "dt_floor": bench_dt_floor_pandas,
        "dt_date": bench_dt_date_pandas,
        "dt_time": bench_dt_time_pandas,
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
    if bench_binary.name != "fp-bench":
        raise ValueError(f"Unexpected fp-bench executable path: {bench_binary}")

    # nosec B603: fp-bench is resolved and name-checked above; shell=False and
    # category/workload values are selected from the static workload matrix.
    result = subprocess.run(
        [str(bench_binary), "--category", category, "--workload", workload,
         "--size", size, "--dtype", dtype, "--json"],
        capture_output=True,
        check=False,
        text=True,
        timeout=300,
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

    try:
        data = json.loads(result.stdout)
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
                 tmp_path: Path, workload_filter: set[str] | None = None) -> list[dict[str, Any]]:
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
    parser.add_argument("--sizes", default="10k,100k,1M",
                        help="Comma-separated sizes (10k,100k,1M)")
    parser.add_argument("--dtypes", default="float64",
                        help="Comma-separated dtypes")
    parser.add_argument("--workloads",
                        help="Comma-separated workload names within the selected category")
    parser.add_argument("--output", type=Path, help="Output JSON file")
    args = parser.parse_args()

    if not args.category and not args.all:
        parser.error("Specify --category or --all")

    if pd is None:
        print("ERROR: pandas not installed", file=sys.stderr)
        sys.exit(1)

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
        timestamp = datetime.now(timezone.utc).isoformat()

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
            results = run_category(category, sizes, dtypes, tmp_path, workload_filter)
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
