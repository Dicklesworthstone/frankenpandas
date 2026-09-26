"""
Differential conformance test harness for frankenpandas vs pandas oracle on packet fixtures.
"""

from __future__ import annotations

import datetime
import glob
import itertools
import json
import math
import os
import warnings
from pathlib import Path
from typing import Any

import numpy as np
import pandas as pd
import pytest
import sqlite3

try:
    import frankenpandas as fpd
except ImportError:
    fpd = None

FIXTURES_DIR = Path(__file__).resolve().parents[2] / "fp-conformance" / "fixtures" / "packets"


def parse_val(v: Any) -> Any:
    if not isinstance(v, dict):
        return v
    kind = v.get("kind")
    val = v.get("value")
    if kind == "null":
        return None
    elif kind == "int64":
        return int(val)
    elif kind == "float64":
        if val == "na_n" or val == "nan":
            return float("nan")
        return float(val)
    elif kind == "utf8":
        return str(val)
    elif kind == "bool":
        return bool(val)
    return val


def build_series(data: dict[str, Any] | None, mod: Any) -> Any:
    if not data:
        return None
    index = [parse_val(x) for x in data.get("index", [])]
    values = [parse_val(x) for x in data.get("values", [])]
    name = data.get("name")
    return mod.Series(data=values, index=index if index else None, name=name)


def build_dataframe(data: dict[str, Any] | None, mod: Any) -> Any:
    if not data:
        return None
    index = [parse_val(x) for x in data.get("index", [])]
    cols = data.get("columns", {})
    if isinstance(cols, dict):
        df_dict = {col: [parse_val(x) for x in vals] for col, vals in cols.items()}
    elif isinstance(cols, list):
        df_dict = {}
        for c in cols:
            df_dict[c["name"]] = [parse_val(x) for x in c.get("values", [])]
    else:
        df_dict = {}
    return mod.DataFrame(data=df_dict, index=index if index else None)


def assert_series_equal(s_fpd: Any, s_pd: Any) -> None:
    assert len(s_fpd) == len(s_pd), f"Length mismatch: {len(s_fpd)} vs {len(s_pd)}"
    v_raw = s_fpd.values() if callable(getattr(s_fpd, "values", None)) else s_fpd.values
    if hasattr(v_raw, "tolist"):
        v_fpd = v_raw.tolist()
    elif hasattr(v_raw, "__iter__") and not isinstance(v_raw, (list, tuple)):
        v_fpd = list(v_raw)
    else:
        v_fpd = v_raw
    v_pd = s_pd.values.tolist() if hasattr(s_pd.values, "tolist") else list(s_pd.values)
    for i, (a, b) in enumerate(zip(v_fpd, v_pd)):
        if a is None or (isinstance(a, float) and math.isnan(a)):
            assert b is None or (isinstance(b, float) and math.isnan(b)), f"At {i}: {a} vs {b}"
        elif isinstance(a, (int, float)) and isinstance(b, (int, float)):
            assert math.isclose(float(a), float(b), rel_tol=1e-5, abs_tol=1e-5), f"At {i}: {a} vs {b}"
        else:
            assert str(a) == str(b), f"At {i}: {a} vs {b}"


def assert_dataframe_equal(df_fpd: Any, df_pd: Any) -> None:
    assert df_fpd.shape == df_pd.shape, f"Shape mismatch: {df_fpd.shape} vs {df_pd.shape}"
    cols_fpd = list(df_fpd.columns)
    cols_pd = list(df_pd.columns)
    assert cols_fpd == cols_pd, f"Columns mismatch: {cols_fpd} vs {cols_pd}"
    for col in cols_fpd:
        assert_series_equal(df_fpd[col], df_pd[col])


def load_fixtures(pattern: str) -> list[Path]:
    return sorted(FIXTURES_DIR.glob(pattern))


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("fixture_path", load_fixtures("fp_p2c_*series_add*.json"))
def test_series_add_differential(fixture_path: Path) -> None:
    with open(fixture_path) as f:
        pkt = json.load(f)
    s_pd_l = build_series(pkt["left"], pd)
    s_pd_r = build_series(pkt["right"], pd)
    res_pd = s_pd_l + s_pd_r

    s_fpd_l = build_series(pkt["left"], fpd)
    s_fpd_r = build_series(pkt["right"], fpd)
    res_fpd = s_fpd_l + s_fpd_r

    assert_series_equal(res_fpd, res_pd)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("fixture_path", load_fixtures("fp_p2c_*dataframe_iloc*.json"))
def test_dataframe_iloc_differential(fixture_path: Path) -> None:
    with open(fixture_path) as f:
        pkt = json.load(f)
    if "frame" not in pkt or "iloc_positions" not in pkt:
        pytest.skip("Packet does not have standard frame + iloc_positions")
    positions = pkt["iloc_positions"]
    df_pd = build_dataframe(pkt["frame"], pd)
    df_fpd = build_dataframe(pkt["frame"], fpd)

    try:
        res_pd = df_pd.iloc[positions]
        res_fpd = df_fpd.iloc[positions]
        assert_dataframe_equal(res_fpd, res_pd)
    except (IndexError, ValueError):
        # Both should raise when out of bounds
        with pytest.raises((IndexError, ValueError)):
            df_fpd.iloc[positions]


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("fixture_path", load_fixtures("fp_p2c_*dropna*.json"))
def test_dropna_differential(fixture_path: Path) -> None:
    with open(fixture_path) as f:
        pkt = json.load(f)
    if "series" in pkt or "left" in pkt:
        s_data = pkt.get("series") or pkt.get("left")
        s_pd = build_series(s_data, pd)
        s_fpd = build_series(s_data, fpd)
        res_pd = s_pd.dropna()
        res_fpd = s_fpd.dropna()
        assert_series_equal(res_fpd, res_pd)
    elif "frame" in pkt:
        df_pd = build_dataframe(pkt["frame"], pd)
        df_fpd = build_dataframe(pkt["frame"], fpd)
        res_pd = df_pd.dropna()
        res_fpd = df_fpd.dropna()
        assert_dataframe_equal(res_fpd, res_pd)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("fixture_path", load_fixtures("fp_p2c_*fillna*.json"))
def test_fillna_differential(fixture_path: Path) -> None:
    with open(fixture_path) as f:
        pkt = json.load(f)
    fill_val = parse_val(pkt.get("fill_value", 0))
    if "series" in pkt or "left" in pkt:
        s_data = pkt.get("series") or pkt.get("left")
        s_pd = build_series(s_data, pd)
        s_fpd = build_series(s_data, fpd)
        res_pd = s_pd.fillna(fill_val)
        res_fpd = s_fpd.fillna(fill_val)
        assert_series_equal(res_fpd, res_pd)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("fixture_path", load_fixtures("fp_p2c_*index_align*.json"))
def test_index_align_differential(fixture_path: Path) -> None:
    with open(fixture_path) as f:
        pkt = json.load(f)
    if "left" not in pkt or "right" not in pkt:
        pytest.skip("Packet missing left or right")
    idx_pd_l = pd.Index([parse_val(x) for x in pkt["left"].get("index", [])])
    idx_pd_r = pd.Index([parse_val(x) for x in pkt["right"].get("index", [])])
    res_pd = idx_pd_l.union(idx_pd_r)

    idx_fpd_l = fpd.Index([parse_val(x) for x in pkt["left"].get("index", [])])
    idx_fpd_r = fpd.Index([parse_val(x) for x in pkt["right"].get("index", [])])
    res_fpd = idx_fpd_l.union(idx_fpd_r)

    assert len(res_fpd) == len(res_pd)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("fixture_path", load_fixtures("fp_p2d_482_dataframe_binary_alias_*.json"))
def test_dataframe_binary_alias_differential(fixture_path: Path) -> None:
    with open(fixture_path) as f:
        pkt = json.load(f)
    method = pkt["dataframe_binary_method"]
    df_pd = build_dataframe(pkt["frame"], pd)
    df_pd_other = build_dataframe(pkt["frame_other"], pd)
    res_pd = getattr(df_pd, method)(df_pd_other)

    df_fpd = build_dataframe(pkt["frame"], fpd)
    df_fpd_other = build_dataframe(pkt["frame_other"], fpd)
    res_fpd = getattr(df_fpd, method)(df_fpd_other)

    assert_dataframe_equal(res_fpd, res_pd)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("fixture_path", load_fixtures("*series_head*.json"))
def test_series_head_differential(fixture_path: Path) -> None:
    with open(fixture_path) as f:
        pkt = json.load(f)
    n = pkt.get("head_n", pkt.get("n", 5))
    s_pd = build_series(pkt["left"], pd)
    res_pd = s_pd.head(n)

    s_fpd = build_series(pkt["left"], fpd)
    res_fpd = s_fpd.head(n)

    assert_series_equal(res_fpd, res_pd)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("fixture_path", load_fixtures("*series_dot*.json"))
def test_series_dot_differential(fixture_path: Path) -> None:
    with open(fixture_path) as f:
        pkt = json.load(f)
    s_pd_l = build_series(pkt["left"], pd)
    s_pd_r = build_series(pkt["right"], pd)
    res_pd = s_pd_l.dot(s_pd_r)

    s_fpd_l = build_series(pkt["left"], fpd)
    s_fpd_r = build_series(pkt["right"], fpd)
    res_fpd = s_fpd_l.dot(s_fpd_r)

    if math.isnan(res_pd):
        assert math.isnan(res_fpd)
    else:
        assert math.isclose(float(res_fpd), float(res_pd), rel_tol=1e-5, abs_tol=1e-5)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("fixture_path", load_fixtures("*series_unique*.json"))
def test_series_unique_differential(fixture_path: Path) -> None:
    with open(fixture_path) as f:
        pkt = json.load(f)
    s_pd = build_series(pkt["left"], pd)
    s_fpd = build_series(pkt["left"], fpd)
    u_pd = list(s_pd.unique())
    u_fpd = s_fpd.unique()
    assert len(u_pd) == len(u_fpd), f"Length mismatch: {len(u_fpd)} vs {len(u_pd)}"


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("fixture_path", load_fixtures("fp_p2c_*groupby_*.json"))
def test_series_groupby_differential(fixture_path: Path) -> None:
    with open(fixture_path) as f:
        pkt = json.load(f)
    if "left" not in pkt or "right" not in pkt:
        pytest.skip("Packet missing left or right series")
    op = pkt.get("operation", "")
    method = op.replace("groupby_", "")
    if method not in ["sum", "mean", "min", "max", "count", "first", "last", "median", "std", "var"]:
        pytest.skip(f"Unhandled groupby method {method}")

    s_pd_key = build_series(pkt["left"], pd)
    s_pd_val = build_series(pkt["right"], pd)
    res_pd = getattr(s_pd_val.groupby(s_pd_key), method)()

    s_fpd_key = build_series(pkt["left"], fpd)
    s_fpd_val = build_series(pkt["right"], fpd)
    res_fpd = getattr(s_fpd_val.groupby(s_fpd_key), method)()

    assert_series_equal(res_fpd, res_pd)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_exception_mapping_differential() -> None:
    s_pd = pd.Series([10, 20, 30])
    s_fpd = fpd.Series([10, 20, 30])

    # Out of bounds iloc raises IndexError
    with pytest.raises(IndexError):
        _ = s_pd.iloc[999]
    with pytest.raises(IndexError):
        _ = s_fpd.iloc[999]

    # Nonexistent column raises KeyError
    df_pd = pd.DataFrame({"a": [1, 2]})
    df_fpd = fpd.DataFrame({"a": [1, 2]})
    with pytest.raises(KeyError):
        _ = df_pd["nonexistent_column"]
    with pytest.raises(KeyError):
        _ = df_fpd["nonexistent_column"]

    # Invalid dtype astype raises TypeError
    with pytest.raises(TypeError):
        _ = s_pd.astype("completely_invalid_dtype_xyz")
    with pytest.raises(TypeError):
        _ = s_fpd.astype("completely_invalid_dtype_xyz")


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_scalar_and_na_differential() -> None:
    # pd.NA vs fpd.NA
    assert repr(fpd.NA) == repr(pd.NA)
    assert str(fpd.NA) == str(pd.NA)
    assert fpd.isna(fpd.NA) is True
    assert fpd.isnull(fpd.NA) is True
    assert fpd.notna(fpd.NA) is False
    assert fpd.notnull(fpd.NA) is False
    with pytest.raises(TypeError):
        bool(fpd.NA)

    # pd.NaT vs fpd.NaT
    assert repr(fpd.NaT) == repr(pd.NaT)
    assert fpd.isna(fpd.NaT) is True
    assert fpd.isnull(fpd.NaT) is True
    assert fpd.notna(fpd.NaT) is False
    assert fpd.notnull(fpd.NaT) is False
    assert fpd.NaT.value == pd.NaT.value

    # Array of NA / NaT
    s_pd = pd.Series([1, pd.NA, 3])
    s_fpd = fpd.Series([1, fpd.NA, 3])
    assert list(s_fpd.isna()) == list(s_pd.isna())
    assert list(s_fpd.notna()) == list(s_pd.notna())


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_timestamp_differential() -> None:
    iso = "2024-03-15 14:30:45"
    ts_pd = pd.Timestamp(iso)
    ts_fpd = fpd.Timestamp(iso)

    assert ts_fpd.year == ts_pd.year == 2024
    assert ts_fpd.month == ts_pd.month == 3
    assert ts_fpd.day == ts_pd.day == 15
    assert ts_fpd.hour == ts_pd.hour == 14
    assert ts_fpd.minute == ts_pd.minute == 30
    assert ts_fpd.second == ts_pd.second == 45
    assert ts_fpd.quarter == ts_pd.quarter == 1
    assert ts_fpd.day_of_week == ts_pd.day_of_week
    assert ts_fpd.day_name() == ts_pd.day_name() == "Friday"
    assert ts_fpd.month_name() == ts_pd.month_name() == "March"
    assert ts_fpd.days_in_month == ts_pd.days_in_month == 31
    assert ts_fpd.is_leap_year == ts_pd.is_leap_year is True
    assert ts_fpd.isoformat() == ts_pd.isoformat()

    # kwargs constructor
    ts_kw_pd = pd.Timestamp(year=2024, month=7, day=4, hour=12, minute=0, second=0)
    ts_kw_fpd = fpd.Timestamp(year=2024, month=7, day=4, hour=12, minute=0, second=0)
    assert ts_kw_fpd.month == ts_kw_pd.month == 7
    assert ts_kw_fpd.day == ts_kw_pd.day == 4

    # Arithmetic with Timedelta
    td_pd = pd.Timedelta(days=2, hours=3)
    td_fpd = fpd.Timedelta(days=2, hours=3)
    add_pd = ts_pd + td_pd
    add_fpd = ts_fpd + td_fpd
    assert add_fpd.day == add_pd.day
    assert add_fpd.hour == add_pd.hour

    sub_pd = ts_pd - td_pd
    sub_fpd = ts_fpd - td_fpd
    assert sub_fpd.day == sub_pd.day
    assert sub_fpd.hour == sub_pd.hour

    # Diff between timestamps
    diff_pd = add_pd - ts_pd
    diff_fpd = add_fpd - ts_fpd
    assert diff_fpd.total_seconds() == diff_pd.total_seconds()

    # Comparison
    assert (ts_fpd < add_fpd) == (ts_pd < add_pd) is True
    assert (ts_fpd == ts_fpd) == (ts_pd == ts_pd) is True


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_timedelta_differential() -> None:
    td_pd = pd.Timedelta(days=3, hours=5, minutes=12, seconds=42)
    td_fpd = fpd.Timedelta(days=3, hours=5, minutes=12, seconds=42)

    assert td_fpd.days == td_pd.days == 3
    assert td_fpd.seconds == td_pd.seconds
    assert td_fpd.total_seconds() == td_pd.total_seconds()

    comp_pd = td_pd.components
    comp_fpd = td_fpd.components
    assert comp_fpd.days == comp_pd.days == 3
    assert comp_fpd.hours == comp_pd.hours == 5
    assert comp_fpd.minutes == comp_pd.minutes == 12
    assert comp_fpd.seconds == comp_pd.seconds == 42

    # Arithmetic
    td2_pd = pd.Timedelta(days=1, hours=2)
    td2_fpd = fpd.Timedelta(days=1, hours=2)
    assert (td_fpd + td2_fpd).total_seconds() == (td_pd + td2_pd).total_seconds()
    assert (td_fpd - td2_fpd).total_seconds() == (td_pd - td2_pd).total_seconds()
    assert (td_fpd * 2).total_seconds() == (td_pd * 2).total_seconds()
    assert (td_fpd / 2).total_seconds() == (td_pd / 2).total_seconds()


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_period_differential() -> None:
    p_pd = pd.Period("2024-03", "M")
    p_fpd = fpd.Period("2024-03", "M")

    assert p_fpd.year == p_pd.year == 2024
    assert p_fpd.month == p_pd.month == 3
    assert p_fpd.quarter == p_pd.quarter == 1
    assert p_fpd.ordinal == p_pd.ordinal
    assert p_fpd.freqstr == p_pd.freqstr == "M"

    # asfreq conversion
    p_d_pd = p_pd.asfreq("D", "end")
    p_d_fpd = p_fpd.asfreq("D", "end")
    assert p_d_fpd.day == p_d_pd.day == 31

    p_ds_pd = p_pd.asfreq("D", "start")
    p_ds_fpd = p_fpd.asfreq("D", "start")
    assert p_ds_fpd.day == p_ds_pd.day == 1

    # Arithmetic
    p_next_pd = p_pd + 1
    p_next_fpd = p_fpd + 1
    assert p_next_fpd.month == p_next_pd.month == 4

    p_prev_pd = p_pd - 2
    p_prev_fpd = p_fpd - 2
    assert p_prev_fpd.month == p_prev_pd.month == 1

    assert (p_next_fpd - p_fpd) == (p_next_pd - p_pd).n == 1


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_flexible_series_constructors_differential() -> None:
    # Empty
    s_empty_pd = pd.Series()
    s_empty_fpd = fpd.Series()
    assert len(s_empty_fpd) == len(s_empty_pd) == 0

    # From dict
    d = {"apple": 5, "banana": 12, "cherry": 7}
    s_dict_pd = pd.Series(d)
    s_dict_fpd = fpd.Series(d)
    assert len(s_dict_fpd) == len(s_dict_pd) == 3
    assert list(s_dict_fpd.index) == list(s_dict_pd.index)
    assert list(s_dict_fpd.values) == list(s_dict_pd.values)

    # From dict with specific index
    idx = ["cherry", "apple", "banana"]
    s_reidx_pd = pd.Series(d, index=idx)
    s_reidx_fpd = fpd.Series(d, index=idx)
    assert list(s_reidx_fpd.index) == list(s_reidx_pd.index)
    assert list(s_reidx_fpd.values) == list(s_reidx_pd.values)

    # Scalar broadcast
    s_broad_pd = pd.Series(99, index=["x", "y", "z"])
    s_broad_fpd = fpd.Series(99, index=["x", "y", "z"])
    assert list(s_broad_fpd.index) == list(s_broad_pd.index)
    assert list(s_broad_fpd.values) == list(s_broad_pd.values)

    # From another Series
    s_copy_pd = pd.Series(s_dict_pd)
    s_copy_fpd = fpd.Series(s_dict_fpd)
    assert list(s_copy_fpd.values) == list(s_copy_pd.values)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_flexible_dataframe_constructors_differential() -> None:
    # Empty
    df_empty_pd = pd.DataFrame()
    df_empty_fpd = fpd.DataFrame()
    assert df_empty_fpd.shape == df_empty_pd.shape == (0, 0)

    # Dict of lists
    data_dict = {"a": [1, 2, 3], "b": ["x", "y", "z"]}
    df_dict_pd = pd.DataFrame(data_dict)
    df_dict_fpd = fpd.DataFrame(data_dict)
    assert df_dict_fpd.shape == df_dict_pd.shape == (3, 2)
    assert list(df_dict_fpd.columns) == list(df_dict_pd.columns)

    # Records (list of dicts)
    records = [{"col1": 10, "col2": 20.5}, {"col1": 30, "col2": 40.5}]
    df_recs_pd = pd.DataFrame(records)
    df_recs_fpd = fpd.DataFrame(records)
    assert df_recs_fpd.shape == df_recs_pd.shape == (2, 2)
    assert list(df_recs_fpd.columns) == list(df_recs_pd.columns)

    # 2D list of lists
    matrix = [[1, 2], [3, 4], [5, 6]]
    df_mat_pd = pd.DataFrame(matrix, columns=["c1", "c2"], index=["r1", "r2", "r3"])
    df_mat_fpd = fpd.DataFrame(matrix, columns=["c1", "c2"], index=["r1", "r2", "r3"])
    assert df_mat_fpd.shape == df_mat_pd.shape == (3, 2)
    assert list(df_mat_fpd.columns) == list(df_mat_pd.columns)
    assert list(df_mat_fpd.index) == list(df_mat_pd.index)

    # Dict of Series with index alignment
    s1_pd = pd.Series([10, 20], index=["a", "b"])
    s2_pd = pd.Series([30, 40], index=["b", "c"])
    s1_fpd = fpd.Series([10, 20], index=["a", "b"])
    s2_fpd = fpd.Series([30, 40], index=["b", "c"])
    df_s_pd = pd.DataFrame({"s1": s1_pd, "s2": s2_pd})
    df_s_fpd = fpd.DataFrame({"s1": s1_fpd, "s2": s2_fpd})
    assert df_s_fpd.shape == df_s_pd.shape == (3, 2)

    # Scalar broadcast
    df_sb_pd = pd.DataFrame(0, index=["i1", "i2"], columns=["c1", "c2"])
    df_sb_fpd = fpd.DataFrame(0, index=["i1", "i2"], columns=["c1", "c2"])
    assert df_sb_fpd.shape == df_sb_pd.shape == (2, 2)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_sqlite_io_differential() -> None:
    conn = sqlite3.connect(":memory:")

    df_pd = pd.DataFrame({"id": [1, 2, 3], "val": [10.5, 20.0, 31.5]})
    df_fpd = fpd.DataFrame({"id": [1, 2, 3], "val": [10.5, 20.0, 31.5]})

    # to_sql index=False
    df_fpd.to_sql("data_table", conn, index=False)

    # read_sql
    read_pd = pd.read_sql("SELECT * FROM data_table", conn)
    read_fpd = fpd.read_sql("SELECT * FROM data_table", conn)
    assert read_fpd.shape == read_pd.shape == (3, 2)
    assert list(read_fpd.columns) == list(read_pd.columns) == ["id", "val"]

    # read_sql_query
    q_pd = pd.read_sql_query("SELECT id FROM data_table WHERE val > 15.0", conn)
    q_fpd = fpd.read_sql_query("SELECT id FROM data_table WHERE val > 15.0", conn)
    assert q_fpd.shape == q_pd.shape == (2, 1)

    # read_sql_table
    t_fpd = fpd.read_sql_table("data_table", conn)
    assert t_fpd.shape == (3, 2)

    # to_sql append
    df_append = fpd.DataFrame({"id": [4], "val": [42.0]})
    df_append.to_sql("data_table", conn, if_exists="append", index=False)
    read_appended = fpd.read_sql("SELECT * FROM data_table", conn)
    assert read_appended.shape == (4, 2)

    # to_sql with index=True
    df_fpd.to_sql("indexed_table", conn, index=True, index_label="row_idx")
    read_idx = fpd.read_sql("SELECT * FROM indexed_table", conn)
    assert "row_idx" in list(read_idx.columns)

    # Series to_sql
    s = fpd.Series([100, 200], name="metric")
    s.to_sql("series_table", conn, index=False)
    s_read = fpd.read_sql("SELECT * FROM series_table", conn)
    assert s_read.shape == (2, 1)
    assert list(s_read.columns) == ["metric"]


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_testing_assertions_differential() -> None:
    df1 = fpd.DataFrame({"a": [1, 2], "b": [3.0, 4.0]})
    df2 = fpd.DataFrame({"a": [1, 2], "b": [3.0, 4.0]})
    df3 = fpd.DataFrame({"a": [1, 2], "b": [3.0, 5.0]})

    fpd.testing.assert_frame_equal(df1, df2)
    with pytest.raises(AssertionError):
        fpd.testing.assert_frame_equal(df1, df3)

    s1 = fpd.Series([10, 20], name="x")
    s2 = fpd.Series([10, 20], name="x")
    s3 = fpd.Series([10, 30], name="x")

    fpd.testing.assert_series_equal(s1, s2)
    with pytest.raises(AssertionError):
        fpd.testing.assert_series_equal(s1, s3)

    idx1 = fpd.Index(["a", "b", "c"])
    idx2 = fpd.Index(["a", "b", "c"])
    idx3 = fpd.Index(["a", "b", "d"])

    fpd.testing.assert_index_equal(idx1, idx2)
    with pytest.raises(AssertionError):
        fpd.testing.assert_index_equal(idx1, idx3)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_errors_hierarchy_differential() -> None:
    assert hasattr(fpd, "errors")
    error_names = [
        "EmptyDataError", "ParserError", "MergeError", "NullFrequencyError",
        "OutOfBoundsDatetime", "OutOfBoundsTimedelta", "DataError", "DatabaseError",
        "DuplicateLabelError", "IndexingError", "IntCastingNaNError", "InvalidIndexError",
        "UnsortedIndexError", "SettingWithCopyError", "SpecificationError",
        "UndefinedVariableError", "UnsupportedFunctionCall", "AbstractMethodError",
        "DtypeWarning", "SettingWithCopyWarning", "PerformanceWarning", "ParserWarning",
        "IncompatibilityWarning", "AttributeConflictWarning", "CategoricalConversionWarning",
        "ChainedAssignmentError",
    ]
    for name in error_names:
        assert hasattr(fpd.errors, name), f"Missing error: {name}"
        fpd_cls = getattr(fpd.errors, name)
        assert issubclass(fpd_cls, Exception) or issubclass(fpd_cls, Warning)
        with pytest.raises(fpd_cls):
            raise fpd_cls("test error message")


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_api_types_differential() -> None:
    import frankenpandas.api.types as fp_types
    import pandas.api.types as pd_types

    # is_bool_dtype
    assert fp_types.is_bool_dtype(bool) == pd_types.is_bool_dtype(bool)
    assert fp_types.is_bool_dtype("bool") == pd_types.is_bool_dtype("bool")
    assert fp_types.is_bool_dtype(int) == pd_types.is_bool_dtype(int)

    # is_numeric_dtype
    assert fp_types.is_numeric_dtype(int) == pd_types.is_numeric_dtype(int)
    assert fp_types.is_numeric_dtype(float) == pd_types.is_numeric_dtype(float)
    assert fp_types.is_numeric_dtype(str) == pd_types.is_numeric_dtype(str)
    assert fp_types.is_numeric_dtype("int64") == pd_types.is_numeric_dtype("int64")

    # is_integer_dtype / is_float_dtype
    assert fp_types.is_integer_dtype(int) == pd_types.is_integer_dtype(int)
    assert fp_types.is_integer_dtype(float) == pd_types.is_integer_dtype(float)
    assert fp_types.is_float_dtype(float) == pd_types.is_float_dtype(float)
    assert fp_types.is_float_dtype(int) == pd_types.is_float_dtype(int)

    # is_string_dtype
    assert fp_types.is_string_dtype(str) == pd_types.is_string_dtype(str)
    assert fp_types.is_string_dtype(int) == pd_types.is_string_dtype(int)

    # is_list_like
    for obj in [[1, 2], (1, 2), {1, 2}, {"a": 1}]:
        assert fp_types.is_list_like(obj) == pd_types.is_list_like(obj)
    for obj in ["abc", b"xyz", 123, 3.14, True, None]:
        assert fp_types.is_list_like(obj) == pd_types.is_list_like(obj)

    # is_dict_like
    assert fp_types.is_dict_like({"a": 1}) == pd_types.is_dict_like({"a": 1})
    assert fp_types.is_dict_like([1, 2]) == pd_types.is_dict_like([1, 2])

    # is_scalar
    for obj in [1, 3.14, "abc", True, None]:
        assert fp_types.is_scalar(obj) == pd_types.is_scalar(obj)
    for obj in [[1, 2], (1, 2), {"a": 1}]:
        assert fp_types.is_scalar(obj) == pd_types.is_scalar(obj)

    # infer_dtype
    assert fp_types.infer_dtype([1, 2, 3]) == pd_types.infer_dtype([1, 2, 3])
    assert fp_types.infer_dtype([1.0, 2.5, 3.0]) == pd_types.infer_dtype([1.0, 2.5, 3.0])
    assert fp_types.infer_dtype(["a", "b", "c"]) == pd_types.infer_dtype(["a", "b", "c"])
    assert fp_types.infer_dtype([True, False]) == pd_types.infer_dtype([True, False])


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_top_level_missing_functions_differential() -> None:
    # __version__ is the Cargo workspace version (0zz8y.3: the 0.3.0 wheel said
    # 0.2.0, and this line pinned the stale literal).
    import tomllib

    with open(Path(__file__).resolve().parents[3] / "Cargo.toml", "rb") as fh:
        cargo_version = tomllib.load(fh)["workspace"]["package"]["version"]
    assert fpd.__version__ == cargo_version

    # unique
    u_pd = list(pd.unique([3, 1, 2, 1, 3]))
    u_fpd = list(fpd.unique([3, 1, 2, 1, 3]))
    assert u_fpd == u_pd

    # value_counts
    vc_pd = pd.value_counts(["a", "b", "a", "c", "a", "b"])
    vc_fpd = fpd.value_counts(["a", "b", "a", "c", "a", "b"])
    assert list(vc_fpd.index) == list(vc_pd.index)
    assert [int(x) for x in vc_fpd.values] == [int(x) for x in vc_pd.values]

    # factorize
    codes_pd, uniques_pd = pd.factorize(["b", "b", "a", "c", "b"])
    codes_fpd, uniques_fpd = fpd.factorize(["b", "b", "a", "c", "b"])
    assert list(codes_fpd) == list(codes_pd)
    assert list(uniques_fpd) == list(uniques_pd)

    # get_dummies on Series
    gd_pd = pd.get_dummies(["a", "b", "a", "c"], dtype=int)
    gd_fpd = fpd.get_dummies(["a", "b", "a", "c"], dtype="int")
    assert gd_fpd.shape == gd_pd.shape
    assert list(gd_fpd.columns) == list(gd_pd.columns)
    for col in gd_pd.columns:
        assert [int(x) for x in gd_fpd[col].values] == [int(x) for x in gd_pd[col].values]

    # get_dummies on DataFrame
    df_src_pd = pd.DataFrame({"cat": ["x", "y", "x"], "num": [10, 20, 30]})
    df_src_fpd = fpd.DataFrame({"cat": ["x", "y", "x"], "num": [10, 20, 30]})
    gd_df_pd = pd.get_dummies(df_src_pd, columns=["cat"], dtype=int)
    gd_df_fpd = fpd.get_dummies(df_src_fpd, columns=["cat"], dtype="int")
    assert list(gd_df_fpd.columns) == list(gd_df_pd.columns)

    # crosstab
    ct_pd = pd.crosstab(["a", "a", "b", "b"], ["x", "y", "x", "y"])
    ct_fpd = fpd.crosstab(["a", "a", "b", "b"], ["x", "y", "x", "y"])
    assert ct_fpd.shape == ct_pd.shape
    assert sorted(list(ct_fpd.columns)) == sorted(list(ct_pd.columns))

    # json_normalize
    data = [
        {"id": 1, "name": "alice", "info": {"age": 30}},
        {"id": 2, "name": "bob", "info": {"age": 25}},
    ]
    jn_pd = pd.json_normalize(data)
    jn_fpd = fpd.json_normalize(data)
    assert jn_fpd.shape == jn_pd.shape
    assert sorted(list(jn_fpd.columns)) == sorted(list(jn_pd.columns))
    for col in jn_pd.columns:
        assert [str(x) for x in jn_fpd[col].values] == [str(x) for x in jn_pd[col].values]


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_extension_dtypes_differential() -> None:
    # 18 extension dtypes
    dtypes = [
        ("BooleanDtype", {}),
        ("Int8Dtype", {}),
        ("Int16Dtype", {}),
        ("Int32Dtype", {}),
        ("Int64Dtype", {}),
        ("UInt8Dtype", {}),
        ("UInt16Dtype", {}),
        ("UInt32Dtype", {}),
        ("UInt64Dtype", {}),
        ("Float32Dtype", {}),
        ("Float64Dtype", {}),
        ("StringDtype", {}),
        ("CategoricalDtype", {}),
        ("DatetimeTZDtype", {"unit": "ns", "tz": "UTC"}),
        ("PeriodDtype", {"freq": "D"}),
        ("IntervalDtype", {"subtype": "float64", "closed": "right"}),
    ]

    for dt_name, kwargs in dtypes:
        assert hasattr(fpd, dt_name), f"fpd missing {dt_name}"
        assert hasattr(pd, dt_name), f"pd missing {dt_name}"
        fp_cls = getattr(fpd, dt_name)
        pd_cls = getattr(pd, dt_name)
        fp_inst = fp_cls(**kwargs)
        pd_inst = pd_cls(**kwargs)
        assert fp_inst.name == pd_inst.name, f"{dt_name} name mismatch: {fp_inst.name} vs {pd_inst.name}"
        assert fp_inst.kind == pd_inst.kind, f"{dt_name} kind mismatch: {fp_inst.kind} vs {pd_inst.kind}"
        assert str(fp_inst) == str(pd_inst)
        assert repr(fp_inst) == repr(pd_inst)
        assert fp_inst == fp_cls(**kwargs)

    # SparseDtype
    assert hasattr(fpd, "SparseDtype")
    assert hasattr(pd, "SparseDtype")
    fp_sp = fpd.SparseDtype("float64", fill_value=0.0)
    pd_sp = pd.SparseDtype("float64", fill_value=0.0)
    assert fp_sp.name == pd_sp.name
    assert fp_sp.kind == pd_sp.kind
    assert fp_sp == fpd.SparseDtype("float64", fill_value=0.0)

    # ArrowDtype
    assert hasattr(fpd, "ArrowDtype")
    assert hasattr(pd, "ArrowDtype")
    fp_arr = fpd.ArrowDtype("int64")
    pd_arr = pd.ArrowDtype(pa.int64()) if "pa" in globals() else None
    assert fp_arr.name == "int64[pyarrow]"
    assert fp_arr.kind == "i"

    # Type checkers
    assert fpd.api.types.is_extension_array_dtype(fpd.Int64Dtype()) is True
    assert fpd.api.types.is_extension_array_dtype(fpd.BooleanDtype()) is True
    assert fpd.api.types.is_categorical_dtype(fpd.CategoricalDtype()) is True
    assert fpd.api.types.is_interval_dtype(fpd.IntervalDtype()) is True


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_options_system_differential() -> None:
    # dot access
    assert hasattr(fpd, "options")
    default_max_rows = fpd.options.display.max_rows
    assert isinstance(default_max_rows, int)

    # set via dot access
    fpd.options.display.max_rows = 42
    assert fpd.options.display.max_rows == 42
    assert fpd.get_option("display.max_rows") == 42

    # reset
    fpd.reset_option("display.max_rows")
    assert fpd.options.display.max_rows == default_max_rows

    # dict access
    fpd.options["display.max_columns"] = 55
    assert fpd.options["display.max_columns"] == 55
    assert fpd.get_option("display.max_columns") == 55
    fpd.reset_option("display.max_columns")

    # get_option / set_option
    fpd.set_option("mode.sim_interactive", True)
    assert fpd.get_option("mode.sim_interactive") is True
    fpd.reset_option("mode.sim_interactive")
    assert fpd.get_option("mode.sim_interactive") is False

    # describe_option
    desc = fpd.describe_option("display.max_rows", _print_desc=False)
    assert isinstance(desc, str)
    assert "display.max_rows" in desc

    # option_context
    with fpd.option_context("display.max_rows", 999):
        assert fpd.get_option("display.max_rows") == 999
        assert fpd.options.display.max_rows == 999
    assert fpd.get_option("display.max_rows") == default_max_rows


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_pickle_differential(tmp_path: Path) -> None:
    # Series pickle
    s_orig = fpd.Series([10, 20, 30], name="test_s")
    pkl_path = tmp_path / "series.pkl"
    fpd.to_pickle(s_orig, str(pkl_path))
    s_loaded = fpd.read_pickle(str(pkl_path))
    assert list(s_loaded.values) == [10, 20, 30]
    assert s_loaded.name == "test_s"

    # Series.to_pickle
    pkl_path2 = tmp_path / "series2.pkl"
    s_orig.to_pickle(str(pkl_path2))
    s_loaded2 = fpd.read_pickle(str(pkl_path2))
    assert list(s_loaded2.values) == [10, 20, 30]

    # DataFrame pickle
    df_orig = fpd.DataFrame({"a": [1, 2, 3], "b": ["x", "y", "z"]})
    df_pkl_path = tmp_path / "df.pkl"
    fpd.to_pickle(df_orig, str(df_pkl_path))
    df_loaded = fpd.read_pickle(str(df_pkl_path))
    assert list(df_loaded.columns) == ["a", "b"]
    assert [int(x) for x in df_loaded["a"].values] == [1, 2, 3]
    assert [str(x) for x in df_loaded["b"].values] == ["x", "y", "z"]

    # DataFrame.to_pickle
    df_pkl_path2 = tmp_path / "df2.pkl"
    df_orig.to_pickle(str(df_pkl_path2))
    df_loaded2 = fpd.read_pickle(str(df_pkl_path2))
    assert list(df_loaded2.columns) == ["a", "b"]

    # Index pickle
    idx_orig = fpd.Index([100, 200, 300], name="test_idx")
    idx_pkl_path = tmp_path / "idx.pkl"
    fpd.to_pickle(idx_orig, str(idx_pkl_path))
    idx_loaded = fpd.read_pickle(str(idx_pkl_path))
    assert list(idx_loaded) == [100, 200, 300]


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_milestone_i_types_and_functions_differential(tmp_path: Path) -> None:
    # read_table
    tsv_file = tmp_path / "test.tsv"
    tsv_file.write_text("colA\tcolB\n1\t2\n3\t4\n")
    df_pd = pd.read_table(str(tsv_file))
    df_fpd = fpd.read_table(str(tsv_file))
    assert list(df_fpd.columns) == list(df_pd.columns)
    assert [int(x) for x in df_fpd["colA"].values] == [int(x) for x in df_pd["colA"].values]
    assert [int(x) for x in df_fpd["colB"].values] == [int(x) for x in df_pd["colB"].values]

    # from_dummies
    df_dummies = pd.DataFrame({"prefix_a": [1, 0, 1], "prefix_b": [0, 1, 0]})
    df_dummies_fpd = fpd.DataFrame({"prefix_a": [1, 0, 1], "prefix_b": [0, 1, 0]})
    rec_pd = pd.from_dummies(df_dummies, sep="_")
    rec_fpd = fpd.from_dummies(df_dummies_fpd, sep="_")
    assert list(rec_fpd.columns) == list(rec_pd.columns)
    assert list(rec_fpd["prefix"].values) == list(rec_pd["prefix"].values)

    # eval
    assert fpd.eval("1 + 2 * 3") == pd.eval("1 + 2 * 3")
    assert fpd.eval("x + y", local_dict={"x": 10, "y": 25}) == pd.eval("x + y", local_dict={"x": 10, "y": 25})

    # array
    arr_pd = pd.array([1, 2, 3])
    arr_fpd = fpd.array([1, 2, 3])
    assert len(arr_fpd) == len(arr_pd)
    assert list(arr_fpd) == list(arr_pd)

    # show_versions
    import io
    from contextlib import redirect_stdout
    f = io.StringIO()
    with redirect_stdout(f):
        fpd.show_versions(as_json=True)
    out = f.getvalue()
    assert "frankenpandas" in out and "system" in out

    # test
    assert callable(fpd.test)

    # IndexSlice
    assert fpd.IndexSlice[0:5] == pd.IndexSlice[0:5]

    # NamedAgg
    na_pd = pd.NamedAgg(column="val", aggfunc="sum")
    na_fpd = fpd.NamedAgg(column="val", aggfunc="sum")
    assert na_fpd.column == na_pd.column
    assert na_fpd.aggfunc == na_pd.aggfunc

    # Grouper
    g_pd = pd.Grouper(key="k", freq="D")
    g_fpd = fpd.Grouper(key="k", freq="D")
    assert g_fpd.key == g_pd.key
    assert g_fpd.freq == g_pd.freq

    # Interval & IntervalIndex
    iv_pd = pd.Interval(1.0, 4.0, closed="right")
    iv_fpd = fpd.Interval(1.0, 4.0, closed="right")
    assert iv_fpd.left == iv_pd.left
    assert iv_fpd.right == iv_pd.right
    assert iv_fpd.closed == iv_pd.closed
    assert iv_fpd.mid == iv_pd.mid
    assert iv_fpd.length == iv_pd.length
    assert 1.0 not in iv_fpd and 1.0 not in iv_pd
    assert 4.0 in iv_fpd and 4.0 in iv_pd
    assert 2.5 in iv_fpd and 2.5 in iv_pd

    ii_pd = pd.IntervalIndex.from_breaks([0, 1, 2, 3])
    ii_fpd = fpd.IntervalIndex.from_breaks([0, 1, 2, 3])
    assert len(ii_fpd) == len(ii_pd)
    assert list(ii_fpd.left) == list(ii_pd.left)
    assert list(ii_fpd.right) == list(ii_pd.right)

    ir_pd = pd.interval_range(start=0, end=4, periods=4)
    ir_fpd = fpd.interval_range(start=0, end=4, periods=4)
    assert len(ir_fpd) == len(ir_pd)
    assert list(ir_fpd.left) == list(ir_pd.left)
    assert list(ir_fpd.right) == list(ir_pd.right)

    # Categorical
    c_pd = pd.Categorical(["a", "b", "c", "a"])
    c_fpd = fpd.Categorical(["a", "b", "c", "a"])
    assert list(c_fpd.codes) == list(c_pd.codes)
    assert list(c_fpd.categories) == list(c_pd.categories)
    assert c_fpd.ordered == c_pd.ordered

    # DateOffset & offsets
    do_pd = pd.DateOffset(days=3)
    do_fpd = fpd.DateOffset(days=3)
    ts_pd = pd.Timestamp("2024-01-01")
    ts_fpd = fpd.Timestamp("2024-01-01")
    assert str(ts_fpd + do_fpd)[:10] == str(ts_pd + do_pd)[:10]
    assert str(ts_fpd - do_fpd)[:10] == str(ts_pd - do_pd)[:10]

    d_pd = pd.offsets.Day(2)
    d_fpd = fpd.offsets.Day(2)
    assert str(ts_fpd + d_fpd)[:10] == str(ts_pd + d_pd)[:10]

    assert hasattr(fpd.offsets, "Hour")
    assert hasattr(fpd.offsets, "Minute")
    assert hasattr(fpd.offsets, "Second")
    assert hasattr(fpd.offsets, "Week")
    assert hasattr(fpd.offsets, "MonthEnd")
    assert hasattr(fpd.offsets, "YearEnd")
    assert fpd.tseries.offsets.Day is fpd.offsets.Day


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_milestone_j_exports_and_submodules_differential(tmp_path: Path) -> None:
    # 1. merge_asof
    left = fpd.DataFrame({"a": [1, 5, 10], "left_val": ["a", "b", "c"]})
    right = fpd.DataFrame({"a": [1, 2, 3, 6, 7], "right_val": [1, 2, 3, 6, 7]})
    res = fpd.merge_asof(left, right, on="a")
    assert len(res) == 3
    assert "left_val" in res.columns and "right_val" in res.columns

    # 2. merge_ordered
    df1 = fpd.DataFrame({"key": ["a", "c", "e"], "lval": [1, 2, 3]})
    df2 = fpd.DataFrame({"key": ["b", "c", "d"], "rval": [4, 5, 6]})
    res_ord = fpd.merge_ordered(df1, df2, on="key")
    assert len(res_ord) == 5
    assert list(res_ord["key"]) == ["a", "b", "c", "d", "e"]

    # 3. infer_freq
    dti = fpd.date_range("2024-01-01", periods=5, freq="D")
    assert fpd.infer_freq(dti) == "D"

    # 4. wide_to_long
    df_wide = fpd.DataFrame({
        "famid": [1, 2],
        "birth": [1, 2],
        "ht1": [2.8, 2.9],
        "ht2": [3.4, 3.8],
    })
    df_long = fpd.wide_to_long(df_wide, stubnames="ht", i="famid", j="age")
    assert "ht" in df_long.columns

    # 5. lreshape
    df_unreshaped = fpd.DataFrame({
        "hr": [1, 2],
        "val1": [10, 20],
        "val2": [30, 40],
    })
    df_reshaped = fpd.lreshape(df_unreshaped, {"val": ["val1", "val2"]})
    assert len(df_reshaped) == 4
    assert "val" in df_reshaped.columns

    # 6. Flags & Series.flags / DataFrame.flags
    s = fpd.Series([1, 2, 3])
    flags = s.flags
    assert hasattr(flags, "allows_duplicate_labels")
    assert flags.allows_duplicate_labels is True

    df_flags = df_wide.flags
    assert hasattr(df_flags, "allows_duplicate_labels")
    assert df_flags.allows_duplicate_labels is True

    # 7. set_eng_float_format
    fpd.set_eng_float_format(accuracy=4, use_eng_prefix=True)
    assert "eng:acc=4,prefix=true" in fpd.get_option("display.float_format")
    fpd.reset_option("display.float_format")

    # 8. Submodules
    assert hasattr(fpd, "plotting")
    assert hasattr(fpd.plotting, "scatter_matrix")
    assert hasattr(fpd.plotting, "andrews_curves")
    assert hasattr(fpd, "arrays")
    assert hasattr(fpd, "io")
    assert hasattr(fpd, "core")
    assert hasattr(fpd, "compat")
    assert hasattr(fpd, "util")
    assert hasattr(fpd, "pandas")
    assert fpd.pandas.DataFrame is fpd.DataFrame

    # 9. Top-level exports 100% parity against pandas
    pd_exports = [x for x in dir(pd) if not x.startswith("_")]
    for exp in pd_exports:
        assert hasattr(fpd, exp), f"Missing top-level export: {exp}"


def test_native_plot_result_and_accessors():
    df = fpd.DataFrame({"x": [1, 2, 3, 4], "y": [10, 20, 15, 30]})
    res = df.plot()
    assert res is not None
    svg = res.to_svg()
    assert isinstance(svg, str)
    assert "<svg" in svg
    assert "</svg>" in svg

    html = res.to_html()
    assert isinstance(html, str)
    assert "<figure" in html
    assert "<svg" in html

    page = res.to_html_page(title="Custom Title")
    assert "<!DOCTYPE html>" in page
    assert "<title>Custom Title</title>" in page
    assert "<svg" in page

    md = res.to_markdown()
    assert "<figure>" in md
    assert "<figcaption>" in md

    # Rich display representation for notebooks
    assert res._repr_svg_() == svg
    assert res._repr_html_() == html
    assert "Plot:" in repr(res)

    for kind in ["line", "bar", "barh", "hist", "box", "kde", "density", "area", "pie"]:
        p = df.plot(kind=kind)
        assert "<svg" in p.to_svg(), f"Failed to render kind: {kind}"

    # Explicit accessor methods
    assert "<svg" in df.plot.line().to_svg()
    assert "<svg" in df.plot.bar().to_svg()
    assert "<svg" in df.plot.barh().to_svg()
    assert "<svg" in df.plot.hist().to_svg()
    assert "<svg" in df.plot.box().to_svg()
    assert "<svg" in df.plot.kde().to_svg()
    assert "<svg" in df.plot.density().to_svg()
    assert "<svg" in df.plot.area().to_svg()
    assert "<svg" in df.plot.pie().to_svg()

    # 2D scatter and hexbin plots
    scatter_res = df.plot.scatter(x="x", y="y")
    assert "<svg" in scatter_res.to_svg()

    hexbin_res = df.plot.hexbin(x="x", y="y")
    assert "<svg" in hexbin_res.to_svg()


def test_series_native_plot_accessor():
    s = fpd.Series([1.5, 2.5, 3.0, 4.2, 5.1], name="metric")
    res = s.plot()
    assert "<svg" in res.to_svg()

    for kind in ["line", "bar", "barh", "hist", "box", "kde", "density", "area", "pie"]:
        p = s.plot(kind=kind)
        assert "<svg" in p.to_svg(), f"Series plot failed for kind: {kind}"

    assert "<svg" in s.plot.line().to_svg()
    assert "<svg" in s.plot.bar().to_svg()
    assert "<svg" in s.plot.barh().to_svg()
    assert "<svg" in s.plot.hist().to_svg()
    assert "<svg" in s.plot.box().to_svg()
    assert "<svg" in s.plot.area().to_svg()
    assert "<svg" in s.plot.pie().to_svg()


def test_plotting_convenience_helpers_and_functions():
    df = fpd.DataFrame({"x": [10.0, 20.0, 30.0], "y": [100.0, 200.0, 300.0]})
    s = fpd.Series([5.0, 10.0, 15.0], name="nums")

    assert "<svg" in df.plot_to_svg()
    assert "<svg" in df.plot_to_html()
    assert "<svg" in s.plot_to_svg()
    assert "<svg" in s.plot_to_html()

    hist_res = df.hist()
    assert hist_res is not None
    assert "<svg" in hist_res.to_svg()

    box_res = df.boxplot()
    assert box_res is not None
    assert "<svg" in box_res.to_svg()

    # Top-level plotting functions
    sm = fpd.plotting.scatter_matrix(df)
    assert sm is not None and "<svg" in sm.to_svg()

    ac = fpd.plotting.autocorrelation_plot(s)
    assert ac is not None and "<svg" in ac.to_svg()

    bp = fpd.plotting.bootstrap_plot(s)
    assert bp is not None and "<svg" in bp.to_svg()

    lp = fpd.plotting.lag_plot(s)
    assert lp is not None and "<svg" in lp.to_svg()

    bx = fpd.plotting.boxplot(df)
    assert bx is not None and "<svg" in bx.to_svg()

    bxf = fpd.plotting.boxplot_frame(df)
    assert bxf is not None and "<svg" in bxf.to_svg()

    hf = fpd.plotting.hist_frame(df)
    assert hf is not None and "<svg" in hf.to_svg()

    hs = fpd.plotting.hist_series(s)
    assert hs is not None and "<svg" in hs.to_svg()

    tbl_df = fpd.plotting.table(None, df)
    assert tbl_df is not None and "<svg" in tbl_df.to_svg()

    tbl_s = fpd.plotting.table(None, s)
    assert tbl_s is not None and "<svg" in tbl_s.to_svg()


def test_dataframe_apply_axis1_and_row_returns():
    df = fpd.DataFrame({"a": [1, 2, 3], "b": [10, 20, 30]})

    # Row-wise sum returning scalar -> Series
    row_sums = df.apply(lambda row: row["a"] + row["b"], axis=1)
    assert list(row_sums.values) == [11, 22, 33]

    # Row-wise function returning dict -> DataFrame under result_type="expand";
    # pandas' default is a Series of the dicts, which needs object cells
    # (fvsao.33) and raises (it used to expand the dicts regardless).
    def row_transform(row):
        return {"sum": row["a"] + row["b"], "diff": row["b"] - row["a"]}

    df_out = df.apply(row_transform, axis=1, result_type="expand")
    assert isinstance(df_out, fpd.DataFrame)
    assert list(df_out["sum"].values) == [11, 22, 33]
    assert list(df_out["diff"].values) == [9, 18, 27]
    with pytest.raises(NotImplementedError):
        df.apply(row_transform, axis=1)


def test_series_map_na_action_and_mapping():
    s = fpd.Series([1, 2, 3], name="x")
    mapped = s.map({1: 10, 2: 20})
    vals = list(mapped.values)
    assert vals[0] == 10
    assert vals[1] == 20
    assert vals[2] != vals[2]  # NaN check

    s_str = fpd.Series(["cat", "dog", None], name="animals")
    upper_mapped = s_str.map(lambda x: x.upper(), na_action="ignore")
    u_vals = list(upper_mapped.values)
    assert u_vals[0] == "CAT"
    assert u_vals[1] == "DOG"
    assert u_vals[2] is None


def test_dataframe_itertuples_options():
    df = fpd.DataFrame({"a": [1, 2], "b": [3, 4]})

    # With default index=True, name="Pandas"
    rows_with_idx = list(df.itertuples())
    assert len(rows_with_idx) == 2
    assert type(rows_with_idx[0]).__name__ == "Pandas"
    assert rows_with_idx[0].Index == 0
    assert rows_with_idx[0].a == 1
    assert rows_with_idx[0].b == 3

    # With index=False
    rows_no_idx = list(df.itertuples(index=False))
    assert len(rows_no_idx) == 2
    assert not hasattr(rows_no_idx[0], "Index")
    assert rows_no_idx[0].a == 1
    assert rows_no_idx[0].b == 3

    # With name=None (returns plain tuples)
    plain_tuples = list(df.itertuples(name=None))
    assert type(plain_tuples[0]) is tuple
    assert plain_tuples[0] == (0, 1, 3)
def test_melt_pivot_sample_explode_differential():
    # 1. melt flexibility & ignore_index
    data = {"A": ["x", "y"], "B": [1, 2], "C": [3, 4]}
    df_fp = fpd.DataFrame(data)
    df_pd = pd.DataFrame(data)

    m_fp_single = df_fp.melt(id_vars="A", value_vars="B")
    m_pd_single = df_pd.melt(id_vars="A", value_vars="B")
    assert m_fp_single.shape == m_pd_single.shape
    assert list(m_fp_single["A"]) == list(m_pd_single["A"])
    assert list(m_fp_single["value"]) == list(m_pd_single["value"])

    m_fp_no_idx = df_fp.melt(id_vars=["A"], value_vars=["B", "C"], ignore_index=False)
    m_pd_no_idx = df_pd.melt(id_vars=["A"], value_vars=["B", "C"], ignore_index=False)
    assert m_fp_no_idx.shape == m_pd_no_idx.shape
    assert list(m_fp_no_idx.index) == list(m_pd_no_idx.index)

    # Top-level melt
    top_m_fp = fpd.melt(df_fp, id_vars="A", value_vars=["B", "C"])
    top_m_pd = pd.melt(df_pd, id_vars="A", value_vars=["B", "C"])
    assert top_m_fp.shape == top_m_pd.shape
    assert list(top_m_fp["variable"]) == list(top_m_pd["variable"])

    # 2. pivot flexibility & keyword ordering
    pdata = {
        "foo": ["one", "one", "two", "two"],
        "bar": ["A", "B", "A", "B"],
        "baz": [1, 2, 3, 4],
    }
    df_p_fp = fpd.DataFrame(pdata)
    df_p_pd = pd.DataFrame(pdata)

    p_fp = df_p_fp.pivot(columns="bar", index="foo", values="baz")
    p_pd = df_p_pd.pivot(columns="bar", index="foo", values="baz")
    assert p_fp.shape == p_pd.shape
    assert sorted(list(p_fp.columns)) == sorted(list(p_pd.columns))

    # Top-level pivot
    top_p_fp = fpd.pivot(df_p_fp, columns="bar", index="foo", values="baz")
    assert top_p_fp.shape == p_pd.shape

    # Pivot without explicit index (uses existing row index)
    p_fp_no_idx = df_p_fp.pivot(columns="bar", values="baz")
    assert p_fp_no_idx.shape == (4, 2)
    assert sorted(list(p_fp_no_idx.columns)) == ["A", "B"]

    # 3. sample ignore_index and axis
    sdata = {"A": [10, 20, 30, 40], "B": [100, 200, 300, 400]}
    df_s_fp = fpd.DataFrame(sdata)
    df_s_pd = pd.DataFrame(sdata)

    samp_fp = df_s_fp.sample(2, random_state=42, ignore_index=True)
    assert samp_fp.shape == (2, 2)
    assert list(samp_fp.index) == [0, 1]

    col_samp_fp = df_s_fp.sample(1, axis=1, random_state=42)
    assert col_samp_fp.shape == (4, 1)

    ser_s_fp = fpd.Series([10, 20, 30, 40])
    ser_samp_fp = ser_s_fp.sample(3, random_state=42, ignore_index=True)
    assert ser_samp_fp.shape == (3,)
    assert list(ser_samp_fp.index) == [0, 1, 2]

    # 4. explode list of columns and ignore_index.
    # GOLDEN-CHANGE (fvsao.5): this block asserted that explode split "1,2" into
    # two rows and never compared with the pandas frame it built. pandas only
    # expands list-likes and keeps strings whole, and explode has no `sep`.
    edata = {"A": ["1,2", "3,4"], "B": [10, 20]}
    df_e_fp = fpd.DataFrame(edata, index=[5, 7])
    df_e_pd = pd.DataFrame(edata, index=[5, 7])

    exp_fp_single = df_e_fp.explode("A")
    exp_pd_single = df_e_pd.explode("A")
    assert exp_fp_single.shape == exp_pd_single.shape == (2, 2)
    assert list(exp_fp_single["A"]) == list(exp_pd_single["A"]) == ["1,2", "3,4"]

    exp_fp_list = df_e_fp.explode(["A"], ignore_index=True)
    exp_pd_list = df_e_pd.explode(["A"], ignore_index=True)
    assert list(exp_fp_list.index) == list(exp_pd_list.index) == [0, 1]

    ser_e_fp = fpd.Series(["x,y", "z,w"], index=[3, 4])
    ser_e_pd = pd.Series(["x,y", "z,w"], index=[3, 4])
    assert ser_e_fp.explode().to_list() == ser_e_pd.explode().to_list() == ["x,y", "z,w"]
    assert list(ser_e_fp.explode(ignore_index=True).index) == [0, 1]
    with pytest.raises(TypeError):
        ser_e_fp.explode(sep=",")
    with pytest.raises(KeyError):
        df_e_fp.explode("missing")
    with pytest.raises(ValueError, match="column must be nonempty"):
        df_e_fp.explode([])


def test_quantile_corr_cov_nlargest_multiindex_differential():
    """Differential test for quantile, corr, cov, nlargest, nsmallest, and MultiIndex constructors."""
    import frankenpandas as fpd
    import pandas as pd
    import numpy as np

    # 1. MultiIndex constructors parity
    tuples = [(1, "red"), (1, "blue"), (2, "red"), (2, "blue")]
    mi_pd_t = pd.MultiIndex.from_tuples(tuples, sortorder=None, names=("num", "color"))
    mi_fp_t = fpd.MultiIndex.from_tuples(tuples, sortorder=None, names=("num", "color"))
    assert list(mi_fp_t.names) == list(mi_pd_t.names)
    assert len(mi_fp_t) == len(mi_pd_t)
    assert list(mi_fp_t.to_list()) == list(mi_pd_t)

    arrays = [[1, 1, 2, 2], ["red", "blue", "red", "blue"]]
    mi_pd_a = pd.MultiIndex.from_arrays(arrays, sortorder=None, names=["num", "color"])
    mi_fp_a = fpd.MultiIndex.from_arrays(arrays, sortorder=None, names=["num", "color"])
    assert list(mi_fp_a.names) == list(mi_pd_a.names)
    assert len(mi_fp_a) == len(mi_pd_a)

    prod = [[1, 2], ["x", "y"]]
    mi_pd_p = pd.MultiIndex.from_product(prod, sortorder=None, names=("first", "second"))
    mi_fp_p = fpd.MultiIndex.from_product(prod, sortorder=None, names=("first", "second"))
    assert list(mi_fp_p.names) == list(mi_pd_p.names)
    assert len(mi_fp_p) == len(mi_pd_p)
    assert list(mi_fp_p.to_list()) == list(mi_pd_p)

    df_base = {"level_0": [10, 20, 30], "level_1": ["a", "b", "c"]}
    df_pd_mi = pd.DataFrame(df_base)
    df_fp_mi = fpd.DataFrame(df_base)
    mi_pd_f = pd.MultiIndex.from_frame(df_pd_mi, sortorder=None)
    mi_fp_f = fpd.MultiIndex.from_frame(df_fp_mi, sortorder=None)
    assert list(mi_fp_f.names) == list(mi_pd_f.names)
    assert len(mi_fp_f) == len(mi_pd_f)

    mi_pd_f_custom = pd.MultiIndex.from_frame(df_pd_mi, sortorder=None, names=("X", "Y"))
    mi_fp_f_custom = fpd.MultiIndex.from_frame(df_fp_mi, sortorder=None, names=("X", "Y"))
    assert list(mi_fp_f_custom.names) == list(mi_pd_f_custom.names)

    # 2. corr & cov parity
    c_data = {"a": [1.0, 2.0, 3.0, 4.0, 5.0], "b": [2.0, 4.0, 5.0, 8.0, 10.0]}
    df_c_pd = pd.DataFrame(c_data)
    df_c_fp = fpd.DataFrame(c_data)

    corr_pd = df_c_pd.corr(method="pearson", min_periods=1, numeric_only=True)
    corr_fp = df_c_fp.corr(method="pearson", min_periods=1, numeric_only=True)
    assert corr_fp.shape == corr_pd.shape
    np.testing.assert_allclose(corr_fp["a"].to_list(), corr_pd["a"].to_list(), rtol=1e-5)
    np.testing.assert_allclose(corr_fp["b"].to_list(), corr_pd["b"].to_list(), rtol=1e-5)

    cov_pd = df_c_pd.cov(min_periods=None, ddof=1, numeric_only=True)
    cov_fp = df_c_fp.cov(min_periods=None, ddof=1, numeric_only=True)
    assert cov_fp.shape == cov_pd.shape
    np.testing.assert_allclose(cov_fp["a"].to_list(), cov_pd["a"].to_list(), rtol=1e-5)
    np.testing.assert_allclose(cov_fp["b"].to_list(), cov_pd["b"].to_list(), rtol=1e-5)

    s1_pd, s2_pd = df_c_pd["a"], df_c_pd["b"]
    s1_fp, s2_fp = df_c_fp["a"], df_c_fp["b"]
    assert abs(s1_fp.corr(s2_fp, method="pearson") - s1_pd.corr(s2_pd, method="pearson")) < 1e-5
    assert abs(s1_fp.cov(s2_fp, ddof=1) - s1_pd.cov(s2_pd, ddof=1)) < 1e-5

    # 3. nlargest & nsmallest parity
    nl_data = {"a": [10.0, 50.0, 20.0, 40.0, 30.0], "b": [1.0, 5.0, 2.0, 4.0, 3.0]}
    df_nl_pd = pd.DataFrame(nl_data)
    df_nl_fp = fpd.DataFrame(nl_data)

    nl_fp_1 = df_nl_fp.nlargest(3, "a", keep="first")
    nl_pd_1 = df_nl_pd.nlargest(3, "a", keep="first")
    assert list(nl_fp_1.index) == list(nl_pd_1.index)
    assert list(nl_fp_1["a"]) == list(nl_pd_1["a"])

    nl_fp_tuple = df_nl_fp.nlargest(3, ("a", "b"))
    nl_pd_tuple = df_nl_pd.nlargest(3, ["a", "b"])
    assert list(nl_fp_tuple.index) == list(nl_pd_tuple.index)

    ns_fp_1 = df_nl_fp.nsmallest(3, "a")
    ns_pd_1 = df_nl_pd.nsmallest(3, "a")
    assert list(ns_fp_1.index) == list(ns_pd_1.index)

    ser_nl_pd = df_nl_pd["a"]
    ser_nl_fp = df_nl_fp["a"]
    assert list(ser_nl_fp.nlargest(3, keep="first").index) == list(ser_nl_pd.nlargest(3, keep="first").index)
    assert list(ser_nl_fp.nsmallest(3, keep="first").index) == list(ser_nl_pd.nsmallest(3, keep="first").index)

    # 4. quantile parity
    q_data = {"a": [1.0, 2.0, 3.0, 4.0], "b": [10.0, 20.0, 30.0, 40.0]}
    df_q_pd = pd.DataFrame(q_data)
    df_q_fp = fpd.DataFrame(q_data)

    # Series quantile default and list
    sq_pd = df_q_pd["a"]
    sq_fp = df_q_fp["a"]
    assert abs(sq_fp.quantile() - sq_pd.quantile()) < 1e-5
    assert abs(sq_fp.quantile(0.25) - sq_pd.quantile(0.25)) < 1e-5
    sq_list_pd = sq_pd.quantile([0.25, 0.75])
    sq_list_fp = sq_fp.quantile([0.25, 0.75])
    assert list(sq_list_fp.index) == [0.25, 0.75]
    np.testing.assert_allclose(sq_list_fp.to_list(), sq_list_pd.to_list(), rtol=1e-5)

    # DataFrame quantile scalar axis 0 and 1
    df_q0_pd = df_q_pd.quantile(0.5, axis=0)
    df_q0_fp = df_q_fp.quantile(0.5, axis=0)
    np.testing.assert_allclose(df_q0_fp.to_list(), df_q0_pd.to_list(), rtol=1e-5)

    df_q1_pd = df_q_pd.quantile(0.5, axis=1)
    df_q1_fp = df_q_fp.quantile(0.5, axis=1)
    np.testing.assert_allclose(df_q1_fp.to_list(), df_q1_pd.to_list(), rtol=1e-5)

    # DataFrame quantile list-like axis 0 and 1
    df_ql0_pd = df_q_pd.quantile([0.25, 0.75], axis=0)
    df_ql0_fp = df_q_fp.quantile([0.25, 0.75], axis=0)
    assert df_ql0_fp.shape == df_ql0_pd.shape
    assert list(df_ql0_fp.index) == [0.25, 0.75]
    np.testing.assert_allclose(df_ql0_fp["a"].to_list(), df_ql0_pd["a"].to_list(), rtol=1e-5)
    np.testing.assert_allclose(df_ql0_fp["b"].to_list(), df_ql0_pd["b"].to_list(), rtol=1e-5)

    df_ql1_pd = df_q_pd.quantile([0.25, 0.75], axis=1)
    df_ql1_fp = df_q_fp.quantile([0.25, 0.75], axis=1)
    assert df_ql1_fp.shape == df_ql1_pd.shape
    assert list(df_ql1_fp.index) == [0.25, 0.75]
    for col in df_ql1_pd.columns:
        np.testing.assert_allclose(df_ql1_fp[str(col)].to_list(), df_ql1_pd[col].to_list(), rtol=1e-5)


def test_row_reductions_corrwith_differential():
    if fpd is None:
        pytest.skip("frankenpandas not installed")

    # 1. DataFrame row reductions (axis=1) and column reductions (axis=0)
    data = {"a": [1.0, 2.0, 3.0], "b": [4.0, 5.0, 6.0], "c": [7.0, 8.0, 9.0]}
    df_pd = pd.DataFrame(data)
    df_fp = fpd.DataFrame(data)

    reductions = [
        "sum", "mean", "min", "max", "std", "var", "median", "prod",
        "count", "sem", "skew", "kurt",
    ]
    for red in reductions:
        # axis=1
        res_pd_1 = getattr(df_pd, red)(axis=1)
        res_fp_1 = getattr(df_fp, red)(axis=1)
        np.testing.assert_allclose(res_fp_1.to_list(), res_pd_1.to_list(), rtol=1e-5, atol=1e-5)

        # axis=0
        res_pd_0 = getattr(df_pd, red)(axis=0)
        res_fp_0 = getattr(df_fp, red)(axis=0)
        np.testing.assert_allclose(res_fp_0.to_list(), res_pd_0.to_list(), rtol=1e-5, atol=1e-5)

    # 2. Mixed DataFrame with numeric_only=True vs numeric_only=False
    data_mixed = {"a": [1.0, 2.0, 3.0], "b": ["x", "y", "z"], "c": [10.0, 20.0, 30.0]}
    df_m_pd = pd.DataFrame(data_mixed)
    df_m_fp = fpd.DataFrame(data_mixed)

    # numeric_only=True should compute on numeric columns only
    res_m_pd_1 = df_m_pd.sum(axis=1, numeric_only=True)
    res_m_fp_1 = df_m_fp.sum(axis=1, numeric_only=True)
    np.testing.assert_allclose(res_m_fp_1.to_list(), res_m_pd_1.to_list(), rtol=1e-5)

    res_m_pd_0 = df_m_pd.sum(axis=0, numeric_only=True)
    res_m_fp_0 = df_m_fp.sum(axis=0, numeric_only=True)
    np.testing.assert_allclose(res_m_fp_0.to_list(), res_m_pd_0.to_list(), rtol=1e-5)

    # numeric_only=False (default) should raise TypeError on row reductions with non-numeric cols
    with pytest.raises(TypeError):
        df_m_fp.sum(axis=1)
    with pytest.raises(TypeError):
        df_m_fp.mean(axis=1)

    # 3. Series reductions with skipna and numeric_only
    s_data = [1.0, 2.0, 3.0, None]
    s_pd = pd.Series(s_data)
    s_fp = fpd.Series(s_data)

    assert abs(s_fp.sum(skipna=True) - s_pd.sum(skipna=True)) < 1e-5
    assert np.isnan(s_fp.sum(skipna=False))
    assert abs(s_fp.mean(skipna=True) - s_pd.mean(skipna=True)) < 1e-5
    assert np.isnan(s_fp.mean(skipna=False))

    s_str_pd = pd.Series(["hello", "world"])
    s_str_fp = fpd.Series(["hello", "world"])
    with pytest.raises(TypeError):
        s_str_fp.mean()
    with pytest.raises(TypeError):
        s_str_fp.mean(numeric_only=True)

    # 4. Correlation & Covariance methods (pearson, spearman, kendall)
    s1_vals = [1.0, 2.0, 3.0, 4.0, 5.0]
    s2_vals = [5.0, 4.0, 2.0, 2.0, 1.0]
    s1_pd = pd.Series(s1_vals)
    s2_pd = pd.Series(s2_vals)
    s1_fp = fpd.Series(s1_vals)
    s2_fp = fpd.Series(s2_vals)

    for method in ["pearson", "spearman", "kendall"]:
        corr_pd = s1_pd.corr(s2_pd, method=method)
        corr_fp = s1_fp.corr(s2_fp, method=method)
        assert abs(corr_fp - corr_pd) < 1e-4

    # Autocorrelation
    assert abs(s1_fp.autocorr() - s1_pd.autocorr()) < 1e-5
    assert abs(s1_fp.autocorr(lag=2) - s1_pd.autocorr(lag=2)) < 1e-5

    # DataFrame corr and cov
    df_corr_pd = df_pd.corr(method="spearman")
    df_corr_fp = df_fp.corr(method="spearman")
    for col in df_corr_pd.columns:
        np.testing.assert_allclose(df_corr_fp[col].to_list(), df_corr_pd[col].to_list(), rtol=1e-4)

    df_cov_pd = df_m_pd.cov(numeric_only=True)
    df_cov_fp = df_m_fp.cov(numeric_only=True)
    for col in df_cov_pd.columns:
        np.testing.assert_allclose(df_cov_fp[col].to_list(), df_cov_pd[col].to_list(), rtol=1e-4)

    # 5. corrwith parity
    cw_s_pd = df_pd.corrwith(s1_pd, axis=0)
    cw_s_fp = df_fp.corrwith(s1_fp, axis=0)
    np.testing.assert_allclose(cw_s_fp.to_list(), cw_s_pd.to_list(), rtol=1e-4)

    df2_data = {"a": [2.0, 3.0, 4.0], "b": [3.0, 5.0, 7.0], "c": [1.0, 2.0, 4.0]}
    df2_pd = pd.DataFrame(df2_data)
    df2_fp = fpd.DataFrame(df2_data)

    cw_df_pd_0 = df_pd.corrwith(df2_pd, axis=0)
    cw_df_fp_0 = df_fp.corrwith(df2_fp, axis=0)
    np.testing.assert_allclose(cw_df_fp_0.to_list(), cw_df_pd_0.to_list(), rtol=1e-4)

    cw_df_pd_1 = df_pd.corrwith(df2_pd, axis=1)
    cw_df_fp_1 = df_fp.corrwith(df2_fp, axis=1)
    np.testing.assert_allclose(cw_df_fp_1.to_list(), cw_df_pd_1.to_list(), rtol=1e-4)

    # 6. mode and nunique parity
    mode_vals = [1, 2, 2, 3, 3, 3]
    s_m_pd = pd.Series(mode_vals)
    s_m_fp = fpd.Series(mode_vals)
    assert s_m_fp.mode().to_list() == s_m_pd.mode().to_list()

    assert s_m_fp.nunique() == s_m_pd.nunique()

    df_nu_pd = pd.DataFrame({"a": [1, 2, 2], "b": [1, 1, 1]})
    df_nu_fp = fpd.DataFrame({"a": [1, 2, 2], "b": [1, 1, 1]})
    np.testing.assert_allclose(df_nu_fp.nunique(axis=0).to_list(), df_nu_pd.nunique(axis=0).to_list())
    np.testing.assert_allclose(df_nu_fp.nunique(axis=1).to_list(), df_nu_pd.nunique(axis=1).to_list())

    # 7. value_counts subset flexible
    vc_pd = df_nu_pd.value_counts(subset="a")
    vc_fp = df_nu_fp.value_counts(subset="a")
    assert len(vc_fp) == len(vc_pd)


def test_rolling_expanding_ewm_df_differential():
    if fpd is None:
        pytest.skip("frankenpandas not installed")

    df_data = {"a": [1.0, 2.0, 3.0, 4.0], "b": [2.0, 4.0, 6.0, 8.0]}
    s_data = [1.0, 3.0, 2.0, 4.0]
    df2_data = {"a": [2.0, 3.0, 4.0, 5.0], "b": [1.0, 2.0, 1.0, 2.0]}

    df_fp = fpd.DataFrame(df_data)
    df_pd = pd.DataFrame(df_data)

    s_fp = fpd.Series(s_data)
    s_pd = pd.Series(s_data)

    df2_fp = fpd.DataFrame(df2_data)
    df2_pd = pd.DataFrame(df2_data)

    # 1. DataFrame rolling apply
    r_apply_fp = df_fp.rolling(2).apply(sum)
    r_apply_pd = df_pd.rolling(2).apply(np.sum)
    for col in ["a", "b"]:
        np.testing.assert_allclose(r_apply_fp[col].to_list(), r_apply_pd[col].to_list(), rtol=1e-5, atol=1e-5)

    # 2. DataFrame expanding apply
    e_apply_fp = df_fp.expanding(2).apply(sum)
    e_apply_pd = df_pd.expanding(2).apply(np.sum)
    for col in ["a", "b"]:
        np.testing.assert_allclose(e_apply_fp[col].to_list(), e_apply_pd[col].to_list(), rtol=1e-5, atol=1e-5)

    # 3. DataFrame rolling/expanding apply on non-numeric columns raises DataError
    df_mixed_fp = fpd.DataFrame({"a": [1.0, 2.0], "b": ["x", "y"]})
    with pytest.raises(Exception) as exc_info:
        df_mixed_fp.rolling(2).apply(sum)
    assert "DataError" in type(exc_info.value).__name__ or isinstance(exc_info.value, TypeError)

    with pytest.raises(Exception) as exc_info:
        df_mixed_fp.expanding(2).apply(sum)
    assert "DataError" in type(exc_info.value).__name__ or isinstance(exc_info.value, TypeError)

    # 4. DataFrame rolling corr/cov with Series and DataFrame
    r_corr_s_fp = df_fp.rolling(2).corr(s_fp)
    r_corr_s_pd = df_pd.rolling(2).corr(s_pd)
    for col in ["a", "b"]:
        np.testing.assert_allclose(r_corr_s_fp[col].to_list(), r_corr_s_pd[col].to_list(), rtol=1e-5, atol=1e-5)

    r_corr_df_fp = df_fp.rolling(2).corr(df2_fp)
    r_corr_df_pd = df_pd.rolling(2).corr(df2_pd)
    for col in ["a", "b"]:
        np.testing.assert_allclose(r_corr_df_fp[col].to_list(), r_corr_df_pd[col].to_list(), rtol=1e-5, atol=1e-5)

    r_cov_s_fp = df_fp.rolling(2).cov(s_fp)
    r_cov_s_pd = df_pd.rolling(2).cov(s_pd)
    for col in ["a", "b"]:
        np.testing.assert_allclose(r_cov_s_fp[col].to_list(), r_cov_s_pd[col].to_list(), rtol=1e-5, atol=1e-5)

    r_cov_df_fp = df_fp.rolling(2).cov(df2_fp)
    r_cov_df_pd = df_pd.rolling(2).cov(df2_pd)
    for col in ["a", "b"]:
        np.testing.assert_allclose(r_cov_df_fp[col].to_list(), r_cov_df_pd[col].to_list(), rtol=1e-5, atol=1e-5)

    # 5. DataFrame expanding corr/cov with Series and DataFrame
    e_corr_s_fp = df_fp.expanding(2).corr(s_fp)
    e_corr_s_pd = df_pd.expanding(2).corr(s_pd)
    for col in ["a", "b"]:
        np.testing.assert_allclose(e_corr_s_fp[col].to_list(), e_corr_s_pd[col].to_list(), rtol=1e-5, atol=1e-5)

    e_corr_df_fp = df_fp.expanding(2).corr(df2_fp)
    e_corr_df_pd = df_pd.expanding(2).corr(df2_pd)
    for col in ["a", "b"]:
        np.testing.assert_allclose(e_corr_df_fp[col].to_list(), e_corr_df_pd[col].to_list(), rtol=1e-5, atol=1e-5)

    e_cov_s_fp = df_fp.expanding(2).cov(s_fp)
    e_cov_s_pd = df_pd.expanding(2).cov(s_pd)
    for col in ["a", "b"]:
        np.testing.assert_allclose(e_cov_s_fp[col].to_list(), e_cov_s_pd[col].to_list(), rtol=1e-5, atol=1e-5)

    e_cov_df_fp = df_fp.expanding(2).cov(df2_fp)
    e_cov_df_pd = df_pd.expanding(2).cov(df2_pd)
    for col in ["a", "b"]:
        np.testing.assert_allclose(e_cov_df_fp[col].to_list(), e_cov_df_pd[col].to_list(), rtol=1e-5, atol=1e-5)

    # 6. DataFrame ewm corr/cov with Series and DataFrame
    ewm_corr_s_fp = df_fp.ewm(span=2).corr(s_fp)
    ewm_corr_s_pd = df_pd.ewm(span=2).corr(s_pd)
    for col in ["a", "b"]:
        np.testing.assert_allclose(ewm_corr_s_fp[col].to_list(), ewm_corr_s_pd[col].to_list(), rtol=1e-5, atol=1e-5)

    ewm_corr_df_fp = df_fp.ewm(span=2).corr(df2_fp)
    ewm_corr_df_pd = df_pd.ewm(span=2).corr(df2_pd)
    for col in ["a", "b"]:
        np.testing.assert_allclose(ewm_corr_df_fp[col].to_list(), ewm_corr_df_pd[col].to_list(), rtol=1e-5, atol=1e-5)

    ewm_cov_s_fp = df_fp.ewm(span=2).cov(s_fp)
    ewm_cov_s_pd = df_pd.ewm(span=2).cov(s_pd)
    for col in ["a", "b"]:
        np.testing.assert_allclose(ewm_cov_s_fp[col].to_list(), ewm_cov_s_pd[col].to_list(), rtol=1e-5, atol=1e-5)

    ewm_cov_df_fp = df_fp.ewm(span=2).cov(df2_fp)
    ewm_cov_df_pd = df_pd.ewm(span=2).cov(df2_pd)
    for col in ["a", "b"]:
        np.testing.assert_allclose(ewm_cov_df_fp[col].to_list(), ewm_cov_df_pd[col].to_list(), rtol=1e-5, atol=1e-5)

    # 7. Series window corr/cov with DataFrame
    s_r_corr_fp = s_fp.rolling(2).corr(df_fp)
    s_r_corr_pd = s_pd.rolling(2).corr(df_pd)
    for col in ["a", "b"]:
        np.testing.assert_allclose(s_r_corr_fp[col].to_list(), s_r_corr_pd[col].to_list(), rtol=1e-5, atol=1e-5)

    s_e_cov_fp = s_fp.expanding(2).cov(df_fp)
    s_e_cov_pd = s_pd.expanding(2).cov(df_pd)
    for col in ["a", "b"]:
        np.testing.assert_allclose(s_e_cov_fp[col].to_list(), s_e_cov_pd[col].to_list(), rtol=1e-5, atol=1e-5)

    s_w_corr_fp = s_fp.ewm(span=2).corr(df_fp)
    s_w_corr_pd = s_pd.ewm(span=2).corr(df_pd)
    for col in ["a", "b"]:
        np.testing.assert_allclose(s_w_corr_fp[col].to_list(), s_w_corr_pd[col].to_list(), rtol=1e-5, atol=1e-5)

    # 8. Type validation for other
    with pytest.raises(TypeError):
        df_fp.rolling(2).corr(42)
    with pytest.raises(TypeError):
        df_fp.expanding(2).cov("invalid")
    with pytest.raises(TypeError):
        df_fp.ewm(span=2).corr([1, 2, 3])
def test_where_mask_combine_parity():
    if fpd is None:
        pytest.skip("frankenpandas not installed")

    # 1. Series where/mask with scalar cond/other
    s_pd = pd.Series([1, 2, 3, 4], index=["a", "b", "c", "d"], name="foo")
    s_fp = fpd.Series([1, 2, 3, 4], index=["a", "b", "c", "d"], name="foo")

    r_pd = s_pd.where(s_pd > 2, -1)
    r_fp = s_fp.where(s_fp > 2, -1)
    assert r_fp.to_dict() == r_pd.to_dict()

    r_pd = s_pd.mask(s_pd > 2, -1)
    r_fp = s_fp.mask(s_fp > 2, -1)
    assert r_fp.to_dict() == r_pd.to_dict()

    # 2. Series where/mask with callable cond and callable other
    r_pd = s_pd.where(lambda s: s % 2 == 0, lambda s: s * 100)
    r_fp = s_fp.where(lambda s: s % 2 == 0, lambda s: s * 100)
    assert r_fp.to_dict() == r_pd.to_dict()

    r_pd = s_pd.mask(lambda s: s % 2 == 0, lambda s: s * 100)
    r_fp = s_fp.mask(lambda s: s % 2 == 0, lambda s: s * 100)
    assert r_fp.to_dict() == r_pd.to_dict()

    # 3. Series where with array/list cond and other
    r_pd = s_pd.where([True, False, True, False], [10, 20, 30, 40])
    r_fp = s_fp.where([True, False, True, False], [10, 20, 30, 40])
    assert r_fp.to_dict() == r_pd.to_dict()

    # 4. Series where inplace
    s_pd_in = s_pd.copy()
    s_fp_in = s_fp.copy()
    ret_pd = s_pd_in.where(s_pd_in > 2, -99, inplace=True)
    ret_fp = s_fp_in.where(s_fp_in > 2, -99, inplace=True)
    assert ret_pd is None and ret_fp is None
    assert s_fp_in.to_dict() == s_pd_in.to_dict()

    # 5. Series where axis validation
    with pytest.raises(ValueError):
        s_fp.where(s_fp > 2, -1, axis=1)

    # 6. Series combine with outer alignment and fill_value
    s1_pd = pd.Series([1, 2], index=["a", "b"], name="x")
    s2_pd = pd.Series([10, 20], index=["b", "c"], name="x")
    s1_fp = fpd.Series([1, 2], index=["a", "b"], name="x")
    s2_fp = fpd.Series([10, 20], index=["b", "c"], name="x")

    c_pd = s1_pd.combine(s2_pd, lambda x, y: x + y, fill_value=0)
    c_fp = s1_fp.combine(s2_fp, lambda x, y: x + y, fill_value=0)
    assert c_fp.to_dict() == c_pd.to_dict()
    assert c_fp.name == c_pd.name

    # 7. DataFrame where/mask with scalar other and callable cond
    df_pd = pd.DataFrame({"A": [1, 2, 3], "B": [4, 5, 6]}, index=["r1", "r2", "r3"])
    df_fp = fpd.DataFrame({"A": [1, 2, 3], "B": [4, 5, 6]}, index=["r1", "r2", "r3"])

    r_df_pd = df_pd.where(lambda df: df > 3, -1)
    r_df_fp = df_fp.where(lambda df: df > 3, -1)
    assert r_df_fp.to_dict() == r_df_pd.to_dict()

    r_df_pd = df_pd.mask(lambda df: df > 3, -1)
    r_df_fp = df_fp.mask(lambda df: df > 3, -1)
    assert r_df_fp.to_dict() == r_df_pd.to_dict()

    # 8. DataFrame where with callable other
    r_df_pd = df_pd.where(df_pd > 3, lambda df: df * 10)
    r_df_fp = df_fp.where(df_fp > 3, lambda df: df * 10)
    assert r_df_fp.to_dict() == r_df_pd.to_dict()

    # 9. DataFrame where with Series other (axis=0 and axis=1)
    s_axis0_pd = pd.Series([100, 200, 300], index=["r1", "r2", "r3"])
    s_axis0_fp = fpd.Series([100, 200, 300], index=["r1", "r2", "r3"])
    r_df_pd = df_pd.where(df_pd > 3, s_axis0_pd, axis=0)
    r_df_fp = df_fp.where(df_fp > 3, s_axis0_fp, axis=0)
    assert r_df_fp.to_dict() == r_df_pd.to_dict()

    s_axis1_pd = pd.Series([10, 20], index=["A", "B"])
    s_axis1_fp = fpd.Series([10, 20], index=["A", "B"])
    r_df_pd = df_pd.where(df_pd > 3, s_axis1_pd, axis=1)
    r_df_fp = df_fp.where(df_fp > 3, s_axis1_fp, axis=1)
    assert r_df_fp.to_dict() == r_df_pd.to_dict()

    # Series other without axis raises ValueError
    with pytest.raises(ValueError, match="Must specify axis=0 or 1"):
        df_fp.where(df_fp > 3, s_axis0_fp)

    # 10. DataFrame where with DataFrame cond and DataFrame other
    df_cond_pd = pd.DataFrame({"A": [True, False, True], "B": [False, True, False]}, index=["r1", "r2", "r3"])
    df_cond_fp = fpd.DataFrame({"A": [True, False, True], "B": [False, True, False]}, index=["r1", "r2", "r3"])
    df_other_pd = pd.DataFrame({"A": [9, 8, 7], "B": [6, 5, 4]}, index=["r1", "r2", "r3"])
    df_other_fp = fpd.DataFrame({"A": [9, 8, 7], "B": [6, 5, 4]}, index=["r1", "r2", "r3"])
    r_df_pd = df_pd.where(df_cond_pd, df_other_pd)
    r_df_fp = df_fp.where(df_cond_fp, df_other_fp)
    assert r_df_fp.to_dict() == r_df_pd.to_dict()

    # 11. DataFrame where inplace
    df_pd_in = df_pd.copy()
    df_fp_in = df_fp.copy()
    ret_pd = df_pd_in.where(df_pd_in > 3, -1, inplace=True)
    ret_fp = df_fp_in.where(df_fp_in > 3, -1, inplace=True)
    assert ret_pd is None and ret_fp is None
    assert df_fp_in.to_dict() == df_pd_in.to_dict()

    # 12. DataFrame combine with column union, row alignment, fill_value, and overwrite
    df1_pd = pd.DataFrame({"A": [1, 2], "B": [3, 4]}, index=[0, 1])
    df2_pd = pd.DataFrame({"B": [30, 40], "C": [50, 60]}, index=[1, 2])
    df1_fp = fpd.DataFrame({"A": [1, 2], "B": [3, 4]}, index=[0, 1])
    df2_fp = fpd.DataFrame({"B": [30, 40], "C": [50, 60]}, index=[1, 2])

    c_df_pd = df1_pd.combine(df2_pd, lambda s1, s2: s1 + s2, fill_value=0)
    c_df_fp = df1_fp.combine(df2_fp, lambda s1, s2: s1 + s2, fill_value=0)
    for col in ["A", "B", "C"]:
        pd.testing.assert_series_equal(pd.Series(c_df_fp[col].to_list(), index=c_df_pd.index, name=col), c_df_pd[col])

    c_df_ow_pd = df1_pd.combine(df2_pd, lambda s1, s2: s1 + s2, fill_value=0, overwrite=False)
    c_df_ow_fp = df1_fp.combine(df2_fp, lambda s1, s2: s1 + s2, fill_value=0, overwrite=False)
    for col in ["A", "B", "C"]:
        pd.testing.assert_series_equal(pd.Series(c_df_ow_fp[col].to_list(), index=c_df_ow_pd.index, name=col), c_df_ow_pd[col])


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_ddof_cov_reductions_parity():
    # 1. Series std, var, sem with ddof=0, 1, 2, 3
    s_vals = [1.0, 2.0, 4.0]
    s_pd = pd.Series(s_vals)
    s_fp = fpd.Series(s_vals)
    for d in [0, 1, 2, 3]:
        r_std_pd = s_pd.std(ddof=d)
        r_std_fp = s_fp.std(ddof=d)
        if np.isnan(r_std_pd):
            assert np.isnan(r_std_fp)
        else:
            assert np.isclose(r_std_fp, r_std_pd)

        r_var_pd = s_pd.var(ddof=d)
        r_var_fp = s_fp.var(ddof=d)
        if np.isnan(r_var_pd):
            assert np.isnan(r_var_fp)
        else:
            assert np.isclose(r_var_fp, r_var_pd)

        r_sem_pd = s_pd.sem(ddof=d)
        r_sem_fp = s_fp.sem(ddof=d)
        if np.isnan(r_sem_pd):
            assert np.isnan(r_sem_fp)
        else:
            assert np.isclose(r_sem_fp, r_sem_pd)

    # 2. Series std, var, sem with skipna=False
    s_nan_pd = pd.Series([1.0, 2.0, np.nan, 4.0])
    s_nan_fp = fpd.Series([1.0, 2.0, np.nan, 4.0])
    assert np.isnan(s_nan_fp.std(skipna=False)) and np.isnan(s_nan_pd.std(skipna=False))
    assert np.isnan(s_nan_fp.var(skipna=False)) and np.isnan(s_nan_pd.var(skipna=False))
    assert np.isnan(s_nan_fp.sem(skipna=False)) and np.isnan(s_nan_pd.sem(skipna=False))

    # 3. Series std on Timedelta, and var/sem raising TypeError
    td_vals = [pd.Timedelta(days=1), pd.Timedelta(days=2), pd.Timedelta(days=4)]
    td_pd = pd.Series(td_vals)
    td_fp = fpd.Series(td_vals)
    assert td_fp.std() == td_pd.std()
    with pytest.raises(TypeError):
        td_fp.var()
    with pytest.raises(TypeError):
        td_fp.sem()

    # 4. Series cov with ddof and min_periods
    s1_pd = pd.Series([1.0, 2.0, 3.0])
    s1_fp = fpd.Series([1.0, 2.0, 3.0])
    s2_pd = pd.Series([4.0, 5.0, 6.0])
    s2_fp = fpd.Series([4.0, 5.0, 6.0])
    assert np.isclose(s1_fp.cov(s2_fp, ddof=0), s1_pd.cov(s2_pd, ddof=0))
    assert np.isclose(s1_fp.cov(s2_fp, ddof=1), s1_pd.cov(s2_pd, ddof=1))
    assert np.isnan(s1_fp.cov(s2_fp, min_periods=4)) and np.isnan(s1_pd.cov(s2_pd, min_periods=4))

    # 5. Series cov with missing values
    s_m1_pd = pd.Series([1.0, 2.0, np.nan, 4.0])
    s_m1_fp = fpd.Series([1.0, 2.0, np.nan, 4.0])
    s_m2_pd = pd.Series([10.0, np.nan, 30.0, 40.0])
    s_m2_fp = fpd.Series([10.0, np.nan, 30.0, 40.0])
    assert np.isclose(s_m1_fp.cov(s_m2_fp, ddof=0), s_m1_pd.cov(s_m2_pd, ddof=0))
    assert np.isclose(s_m1_fp.cov(s_m2_fp, ddof=1), s_m1_pd.cov(s_m2_pd, ddof=1))

    # 6. DataFrame std, var, sem on axis=0 with ddof=0, 1 and skipna
    df_data = {"a": [1.0, 2.0, np.nan, 4.0], "b": [10.0, 20.0, 30.0, 40.0]}
    df_pd = pd.DataFrame(df_data)
    df_fp = fpd.DataFrame(df_data)
    for d in [0, 1]:
        for sk in [True, False]:
            res_std_pd = df_pd.std(axis=0, ddof=d, skipna=sk)
            res_std_fp = df_fp.std(axis=0, ddof=d, skipna=sk)
            pd.testing.assert_series_equal(pd.Series(res_std_fp.to_dict()), res_std_pd)

            res_var_pd = df_pd.var(axis=0, ddof=d, skipna=sk)
            res_var_fp = df_fp.var(axis=0, ddof=d, skipna=sk)
            pd.testing.assert_series_equal(pd.Series(res_var_fp.to_dict()), res_var_pd)

            res_sem_pd = df_pd.sem(axis=0, ddof=d, skipna=sk)
            res_sem_fp = df_fp.sem(axis=0, ddof=d, skipna=sk)
            pd.testing.assert_series_equal(pd.Series(res_sem_fp.to_dict()), res_sem_pd)

    # 7. DataFrame std, var, sem on axis=1 with ddof=0, 1 and skipna
    for d in [0, 1]:
        for sk in [True, False]:
            res_std_pd = df_pd.std(axis=1, ddof=d, skipna=sk)
            res_std_fp = df_fp.std(axis=1, ddof=d, skipna=sk)
            pd.testing.assert_series_equal(pd.Series(res_std_fp.to_dict()), res_std_pd)

            res_var_pd = df_pd.var(axis=1, ddof=d, skipna=sk)
            res_var_fp = df_fp.var(axis=1, ddof=d, skipna=sk)
            pd.testing.assert_series_equal(pd.Series(res_var_fp.to_dict()), res_var_pd)

            res_sem_pd = df_pd.sem(axis=1, ddof=d, skipna=sk)
            res_sem_fp = df_fp.sem(axis=1, ddof=d, skipna=sk)
            pd.testing.assert_series_equal(pd.Series(res_sem_fp.to_dict()), res_sem_pd)

    # 8. DataFrame std on axis=1 with all-Timedelta rows
    df_td_pd = pd.DataFrame({"a": [pd.Timedelta(days=1), pd.Timedelta(days=2)], "b": [pd.Timedelta(days=3), pd.Timedelta(days=4)]})
    df_td_fp = fpd.DataFrame({"a": [pd.Timedelta(days=1), pd.Timedelta(days=2)], "b": [pd.Timedelta(days=3), pd.Timedelta(days=4)]})
    res_td_fp = [pd.Timedelta(v.value, unit="ns") for v in df_td_fp.std(axis=1).to_list()]
    pd.testing.assert_series_equal(pd.Series(res_td_fp), df_td_pd.std(axis=1))

    # 9. DataFrame cov with ddof=0 and ddof=1 on all-valid data
    df_clean_pd = pd.DataFrame({"a": [1.0, 2.0, 3.0], "b": [4.0, 5.0, 6.0]})
    df_clean_fp = fpd.DataFrame({"a": [1.0, 2.0, 3.0], "b": [4.0, 5.0, 6.0]})
    cov0_pd = df_clean_pd.cov(ddof=0)
    cov0_fp = df_clean_fp.cov(ddof=0)
    assert cov0_fp.to_dict() == cov0_pd.to_dict()
    cov1_pd = df_clean_pd.cov(ddof=1)
    cov1_fp = df_clean_fp.cov(ddof=1)
    assert cov1_fp.to_dict() == cov1_pd.to_dict()

    # 10. DataFrame cov with min_periods
    cov_mp_pd = df_clean_pd.cov(min_periods=4)
    cov_mp_fp = df_clean_fp.cov(min_periods=4)
    for col in ["a", "b"]:
        pd.testing.assert_series_equal(pd.Series(cov_mp_fp[col].to_list(), index=cov_mp_pd.index, name=col), cov_mp_pd[col])

    # 11. DataFrame cov with non-numeric column and numeric_only=False raises TypeError
    df_str_fp = fpd.DataFrame({"a": [1, 2], "b": ["x", "y"]})
    with pytest.raises(TypeError):
        df_str_fp.cov(numeric_only=False)

    # 12. DataFrame std, var, sem with numeric_only=True drops non-numeric column
    assert list(df_str_fp.std(numeric_only=True).to_dict().keys()) == ["a"]
    assert list(df_str_fp.var(numeric_only=True).to_dict().keys()) == ["a"]
    assert list(df_str_fp.sem(numeric_only=True).to_dict().keys()) == ["a"]


def _loc_view(obj: Any) -> Any:
    """(kind, index, values, name) with NaN normalised, for strict comparison."""

    def clean(v: Any) -> Any:
        if hasattr(v, "item"):
            v = v.item()
        if isinstance(v, float) and math.isnan(v):
            return "NaN"
        return v

    if hasattr(obj, "columns") and hasattr(obj, "index"):
        cols = [str(c) for c in list(obj.columns)]
        return (
            "frame",
            [clean(x) for x in list(obj.index)],
            cols,
            {c: [clean(x) for x in list(obj[c])] for c in cols},
        )
    if hasattr(obj, "index") and hasattr(obj, "name"):
        return ("series", [clean(x) for x in list(obj.index)], [clean(x) for x in list(obj)], obj.name)
    return ("scalar", clean(obj))


_LOC_CASES = {
    # br-frankenpandas-rc0923-epic-python-honest-dropin-fvsao.2: the boolean mask
    # used to be read as integer labels 1/0 (a Python bool is an int).
    "mask_single_column": lambda df, dk, s: df.loc[df["a"] > 2, "b"],
    "mask_rows": lambda df, dk, s: df.loc[df["a"] > 2],
    "mask_column_list": lambda df, dk, s: df.loc[df["a"] > 2, ["a", "b"]],
    "bool_list_single_column": lambda df, dk, s: df.loc[[True, False, True, False], "a"],
    # label slices are INCLUSIVE of the stop label (was positional, stop-exclusive)
    "label_slice_single_column": lambda df, dk, s: df.loc[1:2, "a"],
    "label_slice_column_list": lambda df, dk, s: df.loc[1:2, ["a", "b"]],
    "column_label_slice": lambda df, dk, s: df.loc[:, "a":"b"],
    # a duplicated label returns EVERY matching row (was only the first)
    "duplicate_label_rows": lambda df, dk, s: dk.loc["x"],
    "duplicate_label_column": lambda df, dk, s: dk.loc["y", "a"],
    "duplicate_label_list": lambda df, dk, s: dk.loc[["x"]],
    "unique_label_scalar": lambda df, dk, s: df.loc[2, "a"],
    "series_label_slice": lambda df, dk, s: s.loc[6:7],
    "series_mask": lambda df, dk, s: s.loc[s > 15],
    "series_label_list": lambda df, dk, s: s.loc[[8, 5]],
    "series_unique_label": lambda df, dk, s: s.loc[6],
    "missing_label_raises": lambda df, dk, s: dk.loc["zz"],
    "unique_label_row": pytest.param(
        lambda df, dk, s: df.loc[2],
        marks=pytest.mark.xfail(
            strict=True,
            reason="row Series name is '2' (str) not 2 (int): Series names are strings "
            "(br-frankenpandas-rc0923-epic-python-honest-dropin-fvsao.7)",
        ),
    ),
}


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_LOC_CASES.values()), ids=list(_LOC_CASES))
def test_loc_indexing_matches_pandas(case: Any) -> None:
    def run(mod: Any) -> Any:
        df = mod.DataFrame({"k": ["x", "y", "x", "y"], "a": [4, 1, 3, 2], "b": [1.5, None, 3.5, 4.0]})
        s = mod.Series([10, 20, 30, 40], index=[5, 6, 7, 8], name="s")
        try:
            return _loc_view(case(df, df.set_index("k"), s))
        except Exception as exc:  # compare the exception CLASS, like pandas users do
            return ("raise", type(exc).__name__)

    assert run(fpd) == run(pd)


def _values(obj: Any) -> list[Any]:
    out = []
    for v in list(obj):
        if hasattr(v, "item"):
            v = v.item()
        if v is None or (isinstance(v, float) and math.isnan(v)):
            out.append("<missing>")
        else:
            out.append(v)
    return out


_VALUE_CASES = {
    # br-frankenpandas-rc0923-epic-python-honest-dropin-fvsao.3: bool columns are
    # numeric for reductions (was string concatenation "FalseTrue...").
    "isna_sum": lambda m: m.DataFrame({"a": [1.0, None, 3.0], "b": ["x", None, None]}).isna().sum(),
    "bool_column_sum": lambda m: m.DataFrame({"a": [True, False, True]}).sum(),
    "bool_column_mean": lambda m: m.DataFrame({"a": [True, False, True, True]}).mean(),
    # every column shifts, object columns included (was left unshifted)
    "shift_object_column": lambda m: m.DataFrame({"k": ["x", "y", "z"], "a": [1, 2, 3]}).shift()["k"],
    # ints mixed with None infer float64 with NaN (was int64 holding None)
    "series_int_with_none": lambda m: m.Series([1, 2, None]),
    "frame_int_with_none": lambda m: m.DataFrame({"a": [1, None, 3]})["a"],
    # default fill_method='pad' forward-fills before the change (was no fill)
    "pct_change_default_pad": lambda m: m.Series([4.0, 2.0, None, 3.0, 6.0]).pct_change(),
    "pct_change_explicit_no_fill": lambda m: m.Series([4.0, 2.0, None, 3.0, 6.0]).pct_change(
        fill_method=None
    ),
}


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_VALUE_CASES.values()), ids=list(_VALUE_CASES))
def test_values_match_pandas(case: Any) -> None:
    import warnings

    with warnings.catch_warnings():
        warnings.simplefilter("ignore", FutureWarning)  # pandas' pct_change default warns
        expected = _values(case(pd))
    assert _values(case(fpd)) == pytest.approx(expected) if all(
        isinstance(v, float) for v in expected
    ) else _values(case(fpd)) == expected


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_int_list_with_none_infers_float64_dtype() -> None:
    assert str(fpd.Series([1, 2, None]).dtype) == str(pd.Series([1, 2, None]).dtype) == "float64"


# Groupby column selection (br-frankenpandas-rc0923-epic-python-honest-dropin-fvsao.6.1).
# Every case raised TypeError ("not subscriptable") or AttributeError before, and
# a SeriesGroupBy over an int64 column returned float64 sums. _strict compares
# dtypes, names and value types, not just values: 4.0 == 4 in Python.
_GB_DATA = {
    "k": ["a", "b", "a", "c", "b"],
    "v": [1, -2, 3, 4, 5],
    "w": [10.0, 20.0, 30.0, 40.0, 50.0],
    "b": [True, False, True, True, False],
    "s": ["x", "y", "z", "u", "t"],
}


def _gb(m: Any) -> Any:
    return m.DataFrame(_GB_DATA).groupby("k")


_GROUPBY_SELECTION_CASES = {
    "col_sum": lambda m: _gb(m)["v"].sum(),
    "col_mean": lambda m: _gb(m)["v"].mean(),
    "col_min": lambda m: _gb(m)["v"].min(),
    "col_prod": lambda m: _gb(m)["v"].prod(),
    "col_count": lambda m: _gb(m)["v"].count(),
    "col_first_str": lambda m: _gb(m)["s"].first(),
    "col_cumsum": lambda m: _gb(m)["v"].cumsum(),
    "col_cummax": lambda m: _gb(m)["v"].cummax(),
    "col_agg_max": lambda m: _gb(m)["w"].agg("max"),
    "col_agg_list": lambda m: _gb(m)["v"].agg(["sum", "max"]),
    "col_transform_sum": lambda m: _gb(m)["v"].transform("sum"),
    "col_transform_mean": lambda m: _gb(m)["v"].transform("mean"),
    "bool_col_sum": lambda m: _gb(m)["b"].sum(),
    "bool_col_max": lambda m: _gb(m)["b"].max(),
    "key_col_count": lambda m: _gb(m)["k"].count(),
    "attr_sum": lambda m: _gb(m).v.sum(),
    "cols_sum": lambda m: _gb(m)[["v", "w"]].sum(),
    "cols_mean": lambda m: _gb(m)[["v", "w"]].mean(),
    "one_col_list_sum": lambda m: _gb(m)[["v"]].sum(),
    "cols_transform_sum": lambda m: _gb(m)[["v", "w"]].transform("sum"),
}


def _is_nan(value: Any) -> bool:
    return isinstance(value, float) and math.isnan(value)


def _marker(value: Any) -> Any:
    """NaN != NaN, so compare it as a marker."""
    # frankenpandas has its own Timestamp/Timedelta classes (pandas-free), so
    # compare those by type name and repr rather than cross-class ==.
    if type(value).__name__ in ("Timestamp", "Timedelta", "NaTType"):
        return (type(value).__name__, repr(value))
    return "<NaN>" if _is_nan(value) else value


def _strict(obj: Any) -> Any:
    if hasattr(obj, "columns"):
        columns = list(obj.columns)
        return (
            "DataFrame",
            columns,
            [str(obj[c].dtype) for c in columns],
            [_strict(obj[c]) for c in columns],
        )
    as_dict = obj.to_dict()
    return (
        "Series",
        str(obj.dtype),
        obj.name,
        obj.index.name,
        [type(v).__name__ for v in as_dict.values()],
        {k: _marker(v) for k, v in as_dict.items()},
    )


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize(
    "case", list(_GROUPBY_SELECTION_CASES.values()), ids=list(_GROUPBY_SELECTION_CASES)
)
def test_groupby_column_selection_matches_pandas(case: Any) -> None:
    assert _strict(case(fpd)) == _strict(case(pd))


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize(
    ("select", "error"),
    [
        (lambda g: g["zz"], KeyError),
        (lambda g: g[["v", "zz"]], KeyError),
        (lambda g: g.zz, AttributeError),
    ],
    ids=["missing_column", "missing_in_list", "missing_attribute"],
)
def test_groupby_missing_selection_raises_like_pandas(select: Any, error: type) -> None:
    with pytest.raises(error) as expected:
        select(_gb(pd))
    with pytest.raises(error) as got:
        select(_gb(fpd))
    assert str(got.value) == str(expected.value)


# read_csv sources and core keywords (br-frankenpandas-rc0923-epic-python-honest-dropin-fvsao.6.2).
# read_csv accepted one str path and no keywords; StringIO raised TypeError.
import io  # noqa: E402

_CSV = "a,b,c,d\n1,x,2.5,2024-01-02\n2,y,,2024-02-03\n3,NA,4.0,2024-03-04\n"
_CSV_QUOTED = 'a,b\n"x,1\ny",2\nz,3\n'  # a quoted field holding the delimiter and a newline


def _csv_path(tmp_path: Path, text: str, name: str = "t.csv") -> str:
    path = tmp_path / name
    path.write_text(text)
    return str(path)


def _read_open_text_file(m: Any, p: Path) -> Any:
    with open(_csv_path(p, _CSV)) as handle:
        return m.read_csv(handle)


_READ_CSV_CASES = {
    "path": lambda m, p: m.read_csv(_csv_path(p, _CSV)),
    "stringio": lambda m, p: m.read_csv(io.StringIO(_CSV)),
    "bytesio": lambda m, p: m.read_csv(io.BytesIO(_CSV.encode())),
    "quoted_delimiter_newline": lambda m, p: m.read_csv(io.StringIO(_CSV_QUOTED)),
    "open_text_file": _read_open_text_file,
    "sep_semicolon": lambda m, p: m.read_csv(io.StringIO(_CSV.replace(",", ";")), sep=";"),
    "delimiter_alias": lambda m, p: m.read_csv(io.StringIO(_CSV.replace(",", "|")), delimiter="|"),
    "header_none": lambda m, p: m.read_csv(io.StringIO("1,2\n3,4\n"), header=None, names=["p", "q"]),
    "names_infer_header": lambda m, p: m.read_csv(io.StringIO("1,2\n3,4\n"), names=["p", "q"]),
    "names_replace_header": lambda m, p: m.read_csv(io.StringIO(_CSV), header=0, names=["w", "x", "y", "z"]),
    "index_col_name": lambda m, p: m.read_csv(io.StringIO(_CSV), index_col="a"),
    "index_col_position": lambda m, p: m.read_csv(io.StringIO(_CSV), index_col=0),
    "index_col_false": lambda m, p: m.read_csv(io.StringIO(_CSV), index_col=False),
    "usecols_names_file_order": lambda m, p: m.read_csv(io.StringIO(_CSV), usecols=["c", "a"]),
    "usecols_positions": lambda m, p: m.read_csv(io.StringIO(_CSV), usecols=[2, 0]),
    "dtype_dict_type": lambda m, p: m.read_csv(io.StringIO(_CSV), dtype={"a": float}),
    "dtype_dict_name": lambda m, p: m.read_csv(io.StringIO(_CSV), dtype={"a": "float64", "b": "str"}),
    "parse_dates": lambda m, p: m.read_csv(io.StringIO(_CSV), parse_dates=["d"]),
    "na_values": lambda m, p: m.read_csv(io.StringIO(_CSV), na_values=["x"]),
    "keep_default_na_false": lambda m, p: m.read_csv(io.StringIO(_CSV), keep_default_na=False),
    "skiprows": lambda m, p: m.read_csv(io.StringIO("junk\n" + _CSV), skiprows=1),
    "nrows": lambda m, p: m.read_csv(io.StringIO(_CSV), nrows=2),
    "encoding_latin1": lambda m, p: m.read_csv(io.BytesIO("a,b\ncafé,1\n".encode("latin-1")), encoding="latin-1"),
    "utf8_bom": lambda m, p: m.read_csv(io.BytesIO(b"\xef\xbb\xbf" + _CSV.encode())),
    "read_table": lambda m, p: m.read_table(io.StringIO(_CSV.replace(",", "\t"))),
}


def _strict_frame(frame: Any) -> Any:
    return (_strict(frame), frame.index.name, list(frame.index))


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_READ_CSV_CASES.values()), ids=list(_READ_CSV_CASES))
def test_read_csv_sources_and_keywords_match_pandas(case: Any, tmp_path: Path) -> None:
    assert _strict_frame(case(fpd, tmp_path)) == _strict_frame(case(pd, tmp_path))


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_read_csv_missing_string_cell_is_nan_like_pandas() -> None:
    # br-frankenpandas-audiv: fp-io marked NA cells NullKind::Null, which the
    # binding renders None; pandas' text parser gives NaN in string columns too.
    # The explicit-None constructor must keep None (pandas does).
    assert _is_nan(pd.read_csv(io.StringIO("a\nx\nNA\n"))["a"].to_dict()[1])
    assert _is_nan(fpd.read_csv(io.StringIO("a\nx\nNA\n"))["a"].to_dict()[1])
    assert fpd.Series(["a", None]).to_dict()[1] is None
    assert pd.Series(["a", None]).to_dict()[1] is None


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_read_excel_blank_cells_match_pandas(tmp_path: Path) -> None:
    # audiv: blank cells read as None, and a whole-number column with a blank
    # stayed int64; pandas gives NaN / float64 (and NaT beside datetimes).
    path = tmp_path / "blanks.xlsx"
    pd.DataFrame(
        {
            "s": ["x", None, "z"],
            "n": [1.0, None, 3.0],
            "t": pd.to_datetime(["2024-01-02", None, "2024-03-04"]),
        }
    ).to_excel(path, index=False)
    got, expected = fpd.read_excel(str(path)), pd.read_excel(str(path))
    assert _strict_frame(got) == _strict_frame(expected)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_read_csv_unsupported_keyword_is_not_silently_ignored() -> None:
    # (converters= was the example until it was implemented; see
    # test_read_csv_parser_options_and_groupby_nth_match_pandas.)
    with pytest.raises(NotImplementedError, match="chunksize"):
        fpd.read_csv(io.StringIO(_CSV), chunksize=10)
    with pytest.raises(ValueError, match="only specify one"):
        fpd.read_csv(io.StringIO(_CSV), sep=",", delimiter=",")


# merge / concat keywords (br-frankenpandas-rc0923-epic-python-honest-dropin-fvsao.6.3).
# Every keyword below raised "unexpected keyword argument", and concat took
# DataFrames only.
_ML = {"k": ["a", "b", "c"], "v": [1, 2, 3]}
_MR = {"k": ["a", "b", "d"], "v": [10, 20, 40], "w": [0.5, 1.5, 2.5]}
_CA = {"x": [1, 2], "y": ["p", "q"]}
_CB = {"x": [3, 4], "z": [1.5, 2.5]}
_CC = {"u": [7, 8], "t": ["s", "r"]}


def _mframes(m: Any) -> Any:
    return m.DataFrame(_ML), m.DataFrame(_MR)


_MERGE_CONCAT_CASES = {
    "merge_on": lambda m: _mframes(m)[0].merge(_mframes(m)[1], on="k"),
    "merge_how_positional": lambda m: _mframes(m)[0].merge(_mframes(m)[1], "left", "k"),
    "merge_outer": lambda m: _mframes(m)[0].merge(_mframes(m)[1], on="k", how="outer"),
    "merge_right": lambda m: _mframes(m)[0].merge(_mframes(m)[1], on="k", how="right"),
    "merge_suffixes": lambda m: _mframes(m)[0].merge(_mframes(m)[1], on="k", suffixes=("_l", "_r")),
    "merge_validate_ok": lambda m: _mframes(m)[0].merge(_mframes(m)[1], on="k", validate="1:1"),
    "merge_left_right_on": lambda m: _mframes(m)[0].merge(
        _mframes(m)[1].rename(columns={"k": "kk"}), left_on="k", right_on="kk"
    ),
    "merge_common_columns": lambda m: _mframes(m)[0].merge(m.DataFrame({"k": ["b", "c"], "q": [1, 2]})),
    "merge_index": lambda m: _mframes(m)[0].set_index("k").merge(
        _mframes(m)[1].set_index("k"), left_index=True, right_index=True, how="outer"
    ),
    "merge_sort": lambda m: _mframes(m)[1].merge(_mframes(m)[0], on="k", how="outer", sort=True),
    "merge_function": lambda m: m.merge(*_mframes(m), on="k", how="left", suffixes=("_a", "_b")),
    # The indicator column's dtype is pinned separately (hrxn9, xfail below);
    # its values are compared in test_merge_indicator_values_match_pandas.
    "merge_indicator_other_columns": lambda m: _mframes(m)[0]
    .merge(_mframes(m)[1], on="k", how="outer", indicator="src")
    .drop(columns=["src"]),
    "concat_rows": lambda m: m.concat([m.DataFrame(_CA), m.DataFrame(_CA)]),
    "concat_ignore_index": lambda m: m.concat([m.DataFrame(_CA), m.DataFrame(_CA)], ignore_index=True),
    "concat_axis1": lambda m: m.concat([m.DataFrame(_CA), m.DataFrame(_CC)], axis=1),
    "concat_axis_columns": lambda m: m.concat([m.DataFrame(_CA), m.DataFrame(_CC)], axis="columns"),
    "concat_join_inner": lambda m: m.concat([m.DataFrame(_CA), m.DataFrame(_CB)], join="inner"),
    "concat_outer_float_gap": lambda m: m.concat([m.DataFrame(_CA), m.DataFrame(_CB)]),
    "concat_skips_none": lambda m: m.concat([None, m.DataFrame(_CA)]),
    "concat_series_rows": lambda m: m.concat([m.Series([1, 2], name="a"), m.Series([3], name="a")]),
    "concat_series_ignore_index": lambda m: m.concat(
        [m.Series([1, 2], name="a"), m.Series([3], name="a")], ignore_index=True
    ),
    "concat_series_axis1": lambda m: m.concat([m.Series([1, 2], name="a"), m.Series([3, 4], name="b")], axis=1),
}


def _strict_ordered(obj: Any) -> Any:
    # concat repeats labels, and to_dict keeps only the last one per label, so
    # also compare the values in row order.
    if hasattr(obj, "columns"):
        ordered = [[_marker(v) for v in obj[c].tolist()] for c in obj.columns]
    else:
        ordered = [_marker(v) for v in obj.tolist()]
    return (_strict(obj), obj.index.name, list(obj.index), ordered)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_MERGE_CONCAT_CASES.values()), ids=list(_MERGE_CONCAT_CASES))
def test_merge_and_concat_keywords_match_pandas(case: Any) -> None:
    assert _strict_ordered(case(fpd)) == _strict_ordered(case(pd))


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_merge_errors_match_pandas() -> None:
    dup = {"k": ["a", "a"], "u": [1, 2]}
    for mod in (pd, fpd):
        left = mod.DataFrame(_ML)
        with pytest.raises(mod.errors.MergeError) as err:
            left.merge(mod.DataFrame(dup), on="k", validate="one_to_one")
        assert str(err.value) == "Merge keys are not unique in right dataset; not a one-to-one merge"
        with pytest.raises(mod.errors.MergeError) as err:
            left.merge(mod.DataFrame({"z": [1]}))
        assert str(err.value).startswith("No common columns to perform merge on.")
    assert issubclass(fpd.errors.MergeError, ValueError)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_concat_refuses_what_it_cannot_match() -> None:
    frame = fpd.DataFrame(_CA)
    # pandas 2.2.3 truncates keys of another length (deprecated); names= only
    # names keys= levels. (keys= side by side is supported now:
    # test_concat_keys_side_by_side_match_pandas; keying frames whose columns
    # are already two-level would need a third level.)
    keyed = fpd.concat([frame, frame], keys=["p", "q"], axis=1)
    with pytest.raises(NotImplementedError, match="MultiIndex"):
        fpd.concat([keyed, keyed], keys=["r", "s"], axis=1)
    with pytest.raises(NotImplementedError, match="different length"):
        fpd.concat([frame, frame], keys=["p"])
    with pytest.raises(NotImplementedError, match="names"):
        fpd.concat([frame, frame], names=["p"])
    with pytest.raises(NotImplementedError, match="sort"):
        fpd.concat([frame, frame], sort=True)
    with pytest.raises(ValueError, match="No objects to concatenate"):
        fpd.concat([])


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_merge_indicator_values_match_pandas() -> None:
    def indicator(m: Any, flag: Any) -> list:
        return m.DataFrame(_ML).merge(m.DataFrame(_MR), on="k", how="outer", indicator=flag)[
            "_merge" if flag is True else flag
        ].tolist()

    for flag in (True, "src"):
        assert indicator(fpd, flag) == [str(v) for v in indicator(pd, flag)]


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_merge_indicator_dtype_is_category_like_pandas() -> None:
    # br-frankenpandas-hrxn9: _merge was an object column; pandas makes it a
    # category with the fixed categories [left_only, right_only, both].
    for how in ("outer", "inner", "left"):
        got, want = (
            m.DataFrame(_ML).merge(m.DataFrame(_MR), on="k", how=how, indicator=True)["_merge"]
            for m in (fpd, pd)
        )
        assert str(got.dtype) == str(want.dtype) == "category"
        assert list(got.cat.categories) == list(want.cat.categories)
        assert got.tolist() == want.tolist()
        assert got.cat.codes.tolist() == want.cat.codes.tolist()


# astype specs and loc slices (br-frankenpandas-rc0923-epic-python-honest-dropin-fvsao.6.4).
# astype took a str only (float/np.float64/np.dtype/dict raised TypeError) and
# mapped "Int64" to int64; an unnamed Series reported name '' (pandas None).
_AD = {"a": [1, 2, 3, 4], "b": [1.5, 2.5, 3.5, 4.5], "c": ["x", "y", "z", "w"]}
_AN = {"a": [1, 2, 3, 4], "b": [1.5, 2.5, 3.5, 4.5]}

_ASTYPE_LOC_CASES = {
    "astype_float_type": lambda m: m.DataFrame(_AN).astype(float),
    "astype_int_type_from_float": lambda m: m.DataFrame({"b": [1.0, 2.0]}).astype(int),
    "astype_str_type": lambda m: m.DataFrame(_AN).astype(str),
    "astype_bool_type": lambda m: m.DataFrame({"a": [0, 1, 2]}).astype(bool),
    "astype_name": lambda m: m.DataFrame(_AN).astype("float64"),
    "astype_numpy_type": lambda m: m.DataFrame(_AN).astype(np.float64),
    "astype_numpy_dtype": lambda m: m.DataFrame(_AN).astype(np.dtype("float64")),
    "astype_dict_names": lambda m: m.DataFrame(_AD).astype({"a": "float64", "b": str}),
    "astype_dict_type": lambda m: m.DataFrame(_AD).astype({"a": float}),
    "astype_nullable_Int64": lambda m: m.DataFrame(_AN).astype({"a": "Int64"}),
    "series_astype_float": lambda m: m.Series([1, 2]).astype(float),
    "series_astype_numpy": lambda m: m.Series([1, 2], name="s").astype(np.float64),
    "series_astype_str": lambda m: m.Series([1, 2]).astype(str),
    "series_astype_Int64": lambda m: m.Series([1, 2]).astype("Int64"),
    "series_astype_dict": lambda m: m.Series([1, 2], name="s").astype({"s": "float64"}),
    "series_astype_errors_ignore": lambda m: m.Series(["x", "1"]).astype("int64", errors="ignore"),
    "loc_slice_cols_list": lambda m: m.DataFrame(_AD).loc[1:2, ["a", "c"]],
    "loc_slice_col_slice": lambda m: m.DataFrame(_AD).loc[1:2, "a":"b"],
    "loc_all_rows_cols": lambda m: m.DataFrame(_AD).loc[:, ["b", "a"]],
    "loc_mask_cols": lambda m: m.DataFrame(_AD).loc[m.DataFrame(_AD)["a"] > 2, ["a", "c"]],
    "loc_str_index_slice": lambda m: m.DataFrame(_AD, index=["p", "q", "r", "s"]).loc["q":"r", ["b"]],
    "loc_slice_one_col": lambda m: m.DataFrame(_AD).loc[1:2, "a"],
}


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_ASTYPE_LOC_CASES.values()), ids=list(_ASTYPE_LOC_CASES))
def test_astype_specs_and_loc_slices_match_pandas(case: Any) -> None:
    assert _strict_ordered(case(fpd)) == _strict_ordered(case(pd))


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_astype_errors_match_pandas() -> None:
    for mod in (pd, fpd):
        with pytest.raises(KeyError):
            mod.DataFrame(_AD).astype({"zz": float})
        with pytest.raises(TypeError, match="data type 'nonsense' not understood"):
            mod.DataFrame(_AD).astype("nonsense")


# Type leaks and no-ops (br-frankenpandas-rc0923-epic-python-honest-dropin-fvsao.4):
# groupby idxmax/idxmin stringified the labels, Index.astype returned the index
# unchanged, and Series/DataFrame.tz_localize/tz_convert were silent no-ops.
_TYPE_LEAK_CASES = {
    "gb_idxmax_int_labels": lambda m: m.Series([3.0, 1.0, 9.0, 2.0])
    .groupby(m.Series(["p", "q", "p", "q"]))
    .idxmax(),
    "gb_idxmin_int_labels": lambda m: m.Series([3.0, 1.0, 9.0, 2.0], name="v")
    .groupby(m.Series(["p", "q", "p", "q"], name="k"))
    .idxmin(),
    "gb_idxmax_str_labels": lambda m: m.Series([3, 1, 9], index=["a", "b", "c"])
    .groupby(m.Series([0, 1, 0], index=["a", "b", "c"]))
    .idxmax(),
}


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_TYPE_LEAK_CASES.values()), ids=list(_TYPE_LEAK_CASES))
def test_type_leaks_match_pandas(case: Any) -> None:
    assert _strict_ordered(case(fpd)) == _strict_ordered(case(pd))


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_index_astype_casts_like_pandas() -> None:
    for spec in ("float64", float, np.float64, "int64", str):
        got, expected = fpd.Index([1, 2]).astype(spec), pd.Index([1, 2]).astype(spec)
        assert (str(got.dtype), list(got)) == (str(expected.dtype), list(expected)), spec
    got, expected = fpd.Index([3, 0]).astype(bool), pd.Index([3, 0]).astype(bool)
    assert (str(got.dtype), list(got)) == (str(expected.dtype), list(expected))
    got, expected = fpd.Index(["a", "b"]).astype("object"), pd.Index(["a", "b"]).astype("object")
    assert (str(got.dtype), list(got)) == (str(expected.dtype), list(expected))
    # pandas keeps the ints in an object index; a typed index cannot, so it
    # refuses rather than stringifying them.
    with pytest.raises(NotImplementedError, match="object"):
        fpd.Index([1, 2]).astype("object")


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_period_index_yields_periods_like_pandas() -> None:
    # It yielded the ordinals ('648' for 2024-01) from tolist/values/[i]/copy.
    def periods(m: Any) -> Any:
        index = m.period_range("2024-01", periods=3, freq="M")
        return (
            [repr(p) for p in index.tolist()],
            [repr(p) for p in index.copy().tolist()],
            repr(index[1]),
            [str(p) for p in index],
        )

    assert periods(fpd) == periods(pd)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_series_tz_methods_localize_and_convert_the_index_like_pandas() -> None:
    # TEST-CHANGE (fvsao.55): this pinned the refusal of Series.tz_localize
    # (NotImplementedError) before the index could carry a zone; it now
    # checks the localized / converted index against pandas.
    def outcome(m: Any) -> Any:
        naive = m.Series([1, 2], index=m.date_range("2024-01-01", periods=2, freq="D"))
        tokyo = naive.tz_localize("Asia/Tokyo")
        utc = tokyo.tz_convert("UTC")
        return (
            [str(t) for t in tokyo.index],
            str(tokyo.index.tz),
            [str(t) for t in utc.index],
            tokyo.tolist(),
            naive.tz_localize(None).tolist(),
        )

    assert outcome(fpd) == outcome(pd)
    # NEGATIVE: pandas raises the same TypeError for a tz-naive index.
    with pytest.raises(TypeError, match="Cannot convert tz-naive timestamps"):
        fpd.Series([1, 2], index=fpd.date_range("2024-01-01", periods=2, freq="D")).tz_convert("UTC")
    with pytest.raises(TypeError, match="Cannot convert tz-naive timestamps"):
        pd.Series([1, 2], index=pd.date_range("2024-01-01", periods=2, freq="D")).tz_convert("UTC")


# Per-column DataFrame ops on non-numeric columns
# (br-frankenpandas-rc0923-epic-rust-parity-bugs-4qg5w.18): object, datetime,
# timedelta and nullable Float64 columns were returned UNCHANGED by ~54 ops.
def _dtype_frame(m: Any, kind: str) -> Any:
    if kind == "object":
        return m.DataFrame({"c": ["b", "a", "c"]})
    if kind == "datetime":
        # (fpd.DataFrame({'c': <DatetimeIndex>}) is the fvsao.6 constructor gap)
        return m.DataFrame({"c": ["2024-01-02", "2024-01-01", "2024-01-03"]}).astype(
            {"c": "datetime64[ns]"}
        )
    if kind == "Float64":
        return m.DataFrame({"c": [1.5, None, -2.5]}).astype({"c": "Float64"})
    raise AssertionError(kind)


_PER_COLUMN_OPS = {
    "abs": lambda df: df.abs(),
    "round": lambda df: df.round(),
    "clip": lambda df: df.clip(0, 1),
    "cumsum": lambda df: df.cumsum(),
    "cumprod": lambda df: df.cumprod(),
    "cummax": lambda df: df.cummax(),
    "cummin": lambda df: df.cummin(),
    "pct_change": lambda df: df.pct_change(fill_method=None),
}


def _outcome(m: Any, op: Any, kind: str) -> Any:
    import warnings

    try:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            out = op(_dtype_frame(m, kind))
    except TypeError:
        return "TypeError"
    return (str(out["c"].dtype), [_marker(v) for v in out["c"].tolist()])


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("kind", ["object", "datetime", "Float64"])
@pytest.mark.parametrize("op", list(_PER_COLUMN_OPS), ids=list(_PER_COLUMN_OPS))
def test_per_column_ops_on_non_numeric_columns_match_pandas(op: str, kind: str) -> None:
    fn = _PER_COLUMN_OPS[op]
    expected = _outcome(pd, fn, kind)
    got = _outcome(fpd, fn, kind)
    if kind == "Float64" and expected != "TypeError":
        # The nullable dtype and its <NA> marker are fvsao.7 territory; the
        # VALUES must match (they were the unchanged input before).
        assert got != "TypeError"

        def clean(vals: list) -> list:
            # pd.NA cannot take part in ==/in, so test its type first.
            return [
                None
                if v is None or type(v).__name__ == "NAType" or (isinstance(v, str) and v == "<NaN>")
                else v
                for v in vals
            ]

        assert clean(got[1]) == clean(expected[1])
    else:
        assert got == expected


# Parameter honesty (br-frankenpandas-rc0923-epic-python-honest-dropin-fvsao.5).
# Each keyword below used to be accepted and dropped: min_count, skipna and the
# sort keys changed nothing, reindex never filled, interpolate always went
# linear, to_json never escaped, to_xml wrote no index, head() needed an
# argument. test_param_honesty.py proves no parameter is dropped; these pin the
# values the implemented ones produce against pandas.
_NAN = float("nan")


def _hs(m: Any) -> Any:
    return m.Series([3.0, _NAN, 1.0, 2.0, 1.0], index=list("dacbe"), name="x")


def _hd(m: Any) -> Any:
    return m.DataFrame({"a": [3, 1, 2, 1, 5], "b": [1.5, _NAN, 2.5, 0.5, 1.5]}, index=list("dacbe"))


def _hg(m: Any) -> Any:
    return m.DataFrame({"k": ["x", "y", "x"], "a": [1, 2, 3], "b": [1.5, 2.5, 3.5]})


_HONEST_CASES = {
    "sum_min_count": lambda m: _hs(m).sum(min_count=5),
    "prod_min_count": lambda m: _hs(m).prod(min_count=5),
    "skew_skipna": lambda m: _hs(m).skew(skipna=False),
    "sort_values_key": lambda m: _hs(m).sort_values(key=lambda s: -s),
    "sort_values_key_descending": lambda m: _hs(m).sort_values(key=lambda s: s.abs(), ascending=False),
    "sort_index_key": lambda m: m.Series([10.0, 20.0, 30.0], index=[3, 1, 2]).sort_index(
        key=lambda i: [-x for x in i]
    ),
    "frame_sort_values_key": lambda m: _hd(m).sort_values("a", key=lambda s: -s),
    "frame_sort_index_key": lambda m: _hd(m).reset_index(drop=True).sort_index(key=lambda i: [-x for x in i]),
    "reindex_fill_value": lambda m: _hs(m).reindex(list("abz"), fill_value=0.0),
    "reindex_method": lambda m: m.Series([1.0, 2.0], index=[0, 2]).reindex([0, 1, 2, 3], method="ffill"),
    "frame_reindex_fill_value": lambda m: _hd(m).reindex(list("abz"), fill_value=0),
    "frame_reindex_new_column_fill": lambda m: _hd(m).reindex(columns=["a", "z"], fill_value=0),
    "interpolate_limit": lambda m: m.Series([1.0, _NAN, _NAN, _NAN, 5.0]).interpolate(limit=1),
    "interpolate_both_directions": lambda m: m.Series([_NAN, 1.0, _NAN, 3.0, _NAN]).interpolate(
        limit_direction="both"
    ),
    "interpolate_inside": lambda m: m.Series([_NAN, 1.0, _NAN, 3.0, _NAN]).interpolate(limit_area="inside"),
    "frame_interpolate_limit": lambda m: m.DataFrame({"a": [1.0, _NAN, _NAN, 4.0]}).interpolate(limit=1),
    "replace_without_value_pads": lambda m: _hs(m).replace(1.0),
    "replace_nothing_pads_missing": lambda m: _hs(m).replace(),
    "frame_replace_without_value": lambda m: _hd(m).replace(1),
    "take_axis": lambda m: _hs(m).take([0, 2], axis=0),
    "head_default": lambda m: _hd(m).head(),
    "tail_default": lambda m: _hs(m).tail(),
    "sum_numpy_out_none": lambda m: _hs(m).sum(out=None),
    "sort_kind_mergesort": lambda m: _hs(m).sort_values(kind="mergesort"),
    "groupby_named_aggregation": lambda m: _hg(m).groupby("k").agg(total=("a", "sum"), top=("b", "max")),
    "to_json_escapes_non_ascii": lambda m: m.Series(["é", "x"]).to_json(),
    "to_json_force_ascii_false": lambda m: m.Series(["é", "x"]).to_json(force_ascii=False),
    "to_json_lines": lambda m: m.DataFrame({"a": [1, 2], "b": ["x", "é"]}).to_json(orient="records", lines=True),
    "to_xml_with_index": lambda m: _hd(m).to_xml(parser="etree"),
    "to_xml_escapes_text": lambda m: m.DataFrame({"s": ["a<b", "x&y"]}).to_xml(parser="etree", index=False),
    "index_factorize_sort": lambda m: [list(x) for x in m.Index([3, 1, 2, 1]).factorize(sort=True)],
    "index_round": lambda m: list(m.Index([1.25, 2.5, 0.125]).round(1)),
    "index_join_sort": lambda m: list(m.Index([3, 1]).join(m.Index([1, 3]), sort=True)),
    "datetimeindex_nat_sorts_last": lambda m: list(
        m.DatetimeIndex(["2024-01-02", None, "2024-01-01"]).sort_values().isna()
    ),
    "datetimeindex_nat_first": lambda m: list(
        m.DatetimeIndex(["2024-01-02", None, "2024-01-01"]).sort_values(na_position="first").isna()
    ),
    "datetimeindex_dropna": lambda m: len(m.DatetimeIndex(["2024-01-02", None, "2024-01-01"]).dropna()),
    "timedeltaindex_duplicated_default": lambda m: list(m.TimedeltaIndex(["1D", "1D"]).duplicated()),
    "index_fillna_default": lambda m: len(m.Index([1.0, 2.0]).fillna()),
    "series_reset_index_level_0": lambda m: _hs(m).reset_index(level=0),
}

# The values match pandas; the result's name does not yet (a DataFrame
# reduction is named after the op, groupby.apply drops the key name), which is
# fvsao.7's and 4qg5w.10's, not a dropped parameter.
_HONEST_VALUE_CASES = {
    "frame_sum_min_count": lambda m: _hd(m).sum(min_count=5),
    "frame_prod_min_count": lambda m: _hd(m).prod(min_count=5),
    "frame_max_skipna": lambda m: _hd(m).max(skipna=False),
    "frame_median_skipna": lambda m: _hd(m).median(skipna=False),
    "frame_min_axis1_skipna": lambda m: _hd(m).min(axis=1, skipna=False),
    "groupby_apply_passes_args": lambda m: _hg(m).groupby("k")["a"].apply(lambda g, k: g.sum() * k, 2),
}


def _honest(obj: Any) -> Any:
    if hasattr(obj, "index") and hasattr(obj, "dtype") or hasattr(obj, "columns"):
        return _strict_ordered(obj)
    return _marker(obj) if isinstance(obj, float) else obj


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_HONEST_CASES.values()), ids=list(_HONEST_CASES))
def test_implemented_parameters_match_pandas(case: Any) -> None:
    import warnings

    with warnings.catch_warnings():
        warnings.simplefilter("ignore")
        expected = _honest(case(pd))
    assert _honest(case(fpd)) == expected


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_HONEST_VALUE_CASES.values()), ids=list(_HONEST_VALUE_CASES))
def test_implemented_parameter_values_match_pandas(case: Any) -> None:
    got, expected = case(fpd), case(pd)
    assert list(got.index) == list(expected.index)
    assert [_marker(v) for v in got.tolist()] == [_marker(v) for v in expected.tolist()]


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_to_csv_keywords_and_targets_match_pandas(tmp_path: Path) -> None:
    frame = {"a": [1.5, _NAN, 3.0], "s": ["x,y", 'q"t', "z"]}
    pdf, fdf = pd.DataFrame(frame, index=["r1", "r2", "r3"]), fpd.DataFrame(frame, index=["r1", "r2", "r3"])
    # float_format moved here from the refusals below when fvsao.31 wrote it;
    # quoting (ALL / NONNUMERIC), quotechar, lineterminator, decimal and a
    # header list when u6p7i wrote them.
    for kw in ({}, {"sep": ";"}, {"na_rep": "NA"}, {"header": False}, {"index": False},
               {"index_label": "idx"}, {"columns": ["s"]}, {"float_format": "%.1f"},
               {"float_format": "%.3e", "na_rep": "NA"}, {"quoting": 1}, {"quoting": 2},
               {"quotechar": "'"}, {"lineterminator": "\r\n"}, {"decimal": ",", "sep": ";"},
               {"header": ["A", "S"]}, {"header": ["A", "S"], "index": False}):
        assert fdf.to_csv(**kw) == pdf.to_csv(**kw), kw
    # date_format renders datetime cells and a datetime index (it was refused).
    dated = {"d": ["2024-01-02 03:04:05", None, "2024-12-31 00:00:00"], "v": [1, 2, 3]}
    pdd, fdd = pd.DataFrame(dated), fpd.DataFrame(dated)
    pdd["d"], fdd["d"] = pd.to_datetime(pdd["d"]), fpd.to_datetime(fdd["d"])
    for kw in ({"date_format": "%d/%m/%Y"}, {"date_format": "%Y %H:%M", "na_rep": "-"}):
        assert fdd.to_csv(**kw) == pdd.to_csv(**kw), kw
        assert fdd.iloc[[0, 2]].set_index("d").to_csv(**kw) == pdd.iloc[[0, 2]].set_index("d").to_csv(**kw), kw
    # NEGATIVE: a header list of the wrong length is pandas' ValueError.
    with pytest.raises(ValueError):
        pdf.to_csv(header=["A"])
    with pytest.raises(ValueError):
        fdf.to_csv(header=["A"])
    s = [1.5, _NAN]
    assert fpd.Series(s).to_csv(na_rep="-") == pd.Series(s).to_csv(na_rep="-")
    buf_fp, buf_pd = io.StringIO(), io.StringIO()
    fdf.to_csv(buf_fp)
    pdf.to_csv(buf_pd)
    assert buf_fp.getvalue() == buf_pd.getvalue()
    fdf.to_csv(tmp_path / "a.csv")
    fdf.to_csv(tmp_path / "a.csv", mode="a", header=False)
    pdf.to_csv(tmp_path / "b.csv")
    pdf.to_csv(tmp_path / "b.csv", mode="a", header=False)
    assert (tmp_path / "a.csv").read_text() == (tmp_path / "b.csv").read_text()
    # NEGATIVE: keywords the writer cannot honour raise instead of vanishing
    # (QUOTE_NONE and doublequote=False / escapechar: Python's csv escaping).
    for kw in ({"quoting": 3}, {"doublequote": False, "escapechar": "\\"}, {"escapechar": "\\"},
               {"lineterminator": "||"}):
        with pytest.raises(NotImplementedError):
            fdf.to_csv(**kw)
    with pytest.raises(NotImplementedError, match="compression"):
        fdf.to_csv(tmp_path / "c.csv.gz")


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_to_records_carries_the_index_like_pandas() -> None:
    frame = {"a": [3, 1], "b": [1.5, _NAN]}
    expected = pd.DataFrame(frame, index=["d", "a"]).to_records()
    got = fpd.DataFrame(frame, index=["d", "a"]).to_records()
    # The records are dicts, not a recarray (fvsao.7); the fields and values
    # must be pandas'. The index used to be missing.
    assert [list(r) for r in got] == [list(expected.dtype.names)] * len(expected)
    assert [tuple(_marker(v) for v in r.values()) for r in got] == [
        tuple(_marker(v) for v in r) for r in expected
    ]
    without = pd.DataFrame(frame).to_records(index=False)
    assert [list(r) for r in fpd.DataFrame(frame).to_records(index=False)] == [list(without.dtype.names)] * 2


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_to_parquet_writes_the_codec_asked_for() -> None:
    pyarrow_parquet = pytest.importorskip("pyarrow.parquet")
    frame = fpd.DataFrame({"a": [3, 1], "b": [1.5, _NAN]})

    def codec(data: bytes) -> str:
        return pyarrow_parquet.ParquetFile(io.BytesIO(data)).metadata.row_group(0).column(0).compression

    snappy = frame.to_parquet()
    assert codec(snappy) == "SNAPPY"
    assert codec(frame.to_parquet(compression=None)) == "UNCOMPRESSED"
    assert pd.read_parquet(io.BytesIO(snappy)).equals(pd.DataFrame({"a": [3, 1], "b": [1.5, _NAN]}))
    with pytest.raises(NotImplementedError, match="gzip"):
        frame.to_parquet(compression="gzip")


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_assert_equal_flags_decide_like_pandas() -> None:
    def verdict(fn: Any, left: Any, right: Any, **kw: Any) -> str:
        try:
            fn(left, right, **kw)
        except AssertionError:
            return "fails"
        return "passes"

    for kw in ({}, {"check_dtype": False}):
        assert verdict(
            fpd.testing.assert_frame_equal, fpd.DataFrame({"a": [1, 2]}), fpd.DataFrame({"a": [1.0, 2.0]}), **kw
        ) == verdict(pd.testing.assert_frame_equal, pd.DataFrame({"a": [1, 2]}), pd.DataFrame({"a": [1.0, 2.0]}), **kw)
        assert verdict(
            fpd.testing.assert_series_equal, fpd.Series([1, 2]), fpd.Series([1.0, 2.0]), **kw
        ) == verdict(pd.testing.assert_series_equal, pd.Series([1, 2]), pd.Series([1.0, 2.0]), **kw)
    for kw in ({}, {"check_names": False}):
        assert verdict(
            fpd.testing.assert_frame_equal,
            fpd.DataFrame({"a": [1]}).rename_axis("x"),
            fpd.DataFrame({"a": [1]}).rename_axis("y"),
            **kw,
        ) == verdict(
            pd.testing.assert_frame_equal,
            pd.DataFrame({"a": [1]}).rename_axis("x"),
            pd.DataFrame({"a": [1]}).rename_axis("y"),
            **kw,
        )
    assert verdict(fpd.testing.assert_index_equal, fpd.Index([1.0]), fpd.Index([1.0 + 1e-9])) == "passes"
    assert verdict(fpd.testing.assert_index_equal, fpd.Index([1.0]), fpd.Index([1.0 + 1e-9]), check_exact=True) == "fails"
    with pytest.raises(NotImplementedError, match="check_like"):
        fpd.testing.assert_frame_equal(fpd.DataFrame({"a": [1]}), fpd.DataFrame({"a": [1]}), check_like=True)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_numpy_keywords_follow_pandas_rule() -> None:
    s_fp, s_pd = fpd.Series([1.0, 2.0]), pd.Series([1.0, 2.0])
    for call in (lambda s: s.sum(foo=1), lambda s: s.mean(foo=1), lambda s: s.transpose(foo=1)):
        with pytest.raises(TypeError, match="unexpected keyword argument 'foo'"):
            call(s_fp)
        with pytest.raises(TypeError):
            call(s_pd)
    with pytest.raises(ValueError, match="'dtype' parameter is not supported"):
        s_fp.sum(dtype="float32")
    with pytest.raises(ValueError, match="sort kind"):
        s_fp.sort_values(kind="bogus")


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize(
    "call",
    [
        lambda: fpd.crosstab(fpd.Series(["a"]), fpd.Series(["b"]), normalize=True, margins=True),
        lambda: _hd(fpd).groupby("a").value_counts(dropna=False),
        lambda: _hd(fpd).pivot_table(index="a", values="b", dropna=False),
        # pandas' Series of lists: object cells (fvsao.33); it returned a bare list.
        lambda: _hd(fpd).apply(lambda r: [r["a"], r["b"]], axis=1),
        lambda: _hs(fpd).interpolate(method="nearest", limit_direction="both"),
        lambda: _hs(fpd).to_string(float_format="{:.1f}".format),
        lambda: _hs(fpd).view("int64"),
        lambda: fpd.PeriodIndex.from_fields(year=[2024]),
        lambda: _hg(fpd).groupby("k").transform("shift", periods=2),
    ],
)
def test_unimplemented_parameters_raise_instead_of_vanishing(call: Any) -> None:
    # Each of these returned a result computed as if the parameter were absent.
    with pytest.raises(NotImplementedError):
        call()


# groupby(as_index=, sort=, dropna=) (br-frankenpandas-n57tz): DataFrame.groupby
# took only `by` (TypeError for each keyword) although fp-frame implements all
# three; size() came back named "size" with an unnamed index.
def _gb_frame(m: Any) -> Any:
    return m.DataFrame({"k": ["y", "x", None, "y", "x"], "a": [1, 2, 3, 4, 5], "b": [1.5, 2.5, 3.5, _NAN, 0.5]})


def _nan_marked(obj: Any) -> Any:
    """A NaN group label is unequal to itself; compare it as a marker."""
    if isinstance(obj, float) and obj != obj:
        return "<NaN>"
    if isinstance(obj, dict):
        return {_nan_marked(k): _nan_marked(v) for k, v in obj.items()}
    if isinstance(obj, (list, tuple)):
        return type(obj)(_nan_marked(v) for v in obj)
    return obj


_GB_OPS = {
    "sum": lambda g: g.sum(),
    "mean": lambda g: g.mean(),
    "count": lambda g: g.count(),
    "max": lambda g: g.max(),
    "first": lambda g: g.first(),
    "size": lambda g: g.size(),
    "nunique": lambda g: g.nunique(),
    "std": lambda g: g.std(),
    "agg_name": lambda g: g.agg("sum"),
    "named_agg": lambda g: g.agg(t=("a", "sum")),
    "column_sum": lambda g: g["a"].sum(),
    "column_mean": lambda g: g["b"].mean(),
    # br-frankenpandas-azq6g: int64 "a" stays int64 when every row has a
    # group (dropna=False) and turns float64 with NaN when one is dropped.
    "cumsum": lambda g: g.cumsum(),
    "cumprod": lambda g: g.cumprod(),
    "cummin": lambda g: g.cummin(),
    "cummax": lambda g: g.cummax(),
    "column_cumsum": lambda g: g["a"].cumsum(),
}
_GB_OPTIONS = {
    "default": {},
    "as_index_false": {"as_index": False},
    "sort_false": {"sort": False},
    "dropna_false": {"dropna": False},
    "as_index_false_sort_false": {"as_index": False, "sort": False},
}
# Selecting a column keeps missing keys only through fp-frame's SeriesGroupBy,
# which cannot yet; that combination raises (asserted below) instead.
_GB_CASES = {
    f"{opt}-{op}": (kw, fn)
    for opt, kw in _GB_OPTIONS.items()
    for op, fn in _GB_OPS.items()
    if not (kw.get("dropna") is False and op.startswith("column_"))
}


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_GB_CASES.values()), ids=list(_GB_CASES))
def test_groupby_options_match_pandas(case: Any) -> None:
    kw, op = case
    expected = _nan_marked(_strict_ordered(op(_gb_frame(pd).groupby("k", **kw))))
    assert _nan_marked(_strict_ordered(op(_gb_frame(fpd).groupby("k", **kw)))) == expected


def _gb_kw_frame(m: Any) -> Any:
    return m.DataFrame(
        {
            "k": ["y", "x", "y", "x", "z"],
            "a": [1, 2, 3, 4, 5],
            "b": [1.5, _NAN, 3.5, _NAN, 0.5],
            "s": ["p", "q", "r", "s", "t"],
            "t": [True, False, True, True, False],
        }
    )


# br-frankenpandas-n57tz: the groupby reductions took no keywords at all.
_GB_KEYWORD_CASES = {
    "sum_numeric_only": lambda m: _gb_kw_frame(m).groupby("k").sum(numeric_only=True),
    "sum_min_count": lambda m: _gb_kw_frame(m).groupby("k").sum(numeric_only=True, min_count=2),
    "sum_min_count_unmasked_int": lambda m: _gb_kw_frame(m).groupby("k")[["a", "b"]].sum(min_count=1),
    "prod_min_count": lambda m: _gb_kw_frame(m).groupby("k")[["a", "t"]].prod(min_count=2),
    "min_min_count_keeps_strings": lambda m: _gb_kw_frame(m).groupby("k")[["a", "b", "s"]].min(min_count=2),
    "max_min_count": lambda m: _gb_kw_frame(m).groupby("k")[["a", "b", "s"]].max(min_count=2),
    "first_min_count_masks_strings": lambda m: _gb_kw_frame(m).groupby("k")[["a", "b", "s"]].first(min_count=2),
    "last_min_count": lambda m: _gb_kw_frame(m).groupby("k")[["a", "b", "s"]].last(min_count=2),
    "first_skipna_false": lambda m: _gb_kw_frame(m).groupby("k")[["a", "b"]].first(skipna=False),
    "last_skipna_false": lambda m: _gb_kw_frame(m).groupby("k")[["a", "b"]].last(skipna=False),
    "mean_numeric_only": lambda m: _gb_kw_frame(m).groupby("k").mean(numeric_only=True),
    "median_numeric_only": lambda m: _gb_kw_frame(m).groupby("k").median(numeric_only=True),
    "std_ddof0": lambda m: _gb_kw_frame(m).groupby("k").std(ddof=0, numeric_only=True),
    "var_ddof2": lambda m: _gb_kw_frame(m).groupby("k")[["a", "b"]].var(ddof=2),
    "sem_ddof0": lambda m: _gb_kw_frame(m).groupby("k")[["a", "b"]].sem(ddof=0),
    "engine_cython": lambda m: _gb_kw_frame(m).groupby("k")[["a"]].sum(engine="cython"),
    "as_index_false_min_count": lambda m: _gb_kw_frame(m).groupby("k", as_index=False)[["a", "b"]].sum(min_count=2),
    "as_index_false_numeric_only": lambda m: _gb_kw_frame(m).groupby("k", as_index=False).mean(numeric_only=True),
    "agg_sem": lambda m: _gb_kw_frame(m).groupby("k")[["a", "b"]].agg("sem"),
    "column_sum_min_count": lambda m: _gb_kw_frame(m).groupby("k")["a"].sum(min_count=2),
    "column_sum_numeric_only": lambda m: _gb_kw_frame(m).groupby("k")["a"].sum(numeric_only=True),
    "column_std_ddof0": lambda m: _gb_kw_frame(m).groupby("k")["b"].std(ddof=0),
    "column_var_ddof0": lambda m: _gb_kw_frame(m).groupby("k")["a"].var(ddof=0),
    "column_sem_ddof0": lambda m: _gb_kw_frame(m).groupby("k")["a"].sem(ddof=0),
    "column_first_skipna_false": lambda m: _gb_kw_frame(m).groupby("k")["b"].first(skipna=False),
    "column_first_min_count_strings": lambda m: _gb_kw_frame(m).groupby("k")["s"].first(min_count=2),
    "column_min_min_count_strings": lambda m: _gb_kw_frame(m).groupby("k")["s"].min(min_count=2),
    "column_agg_sem": lambda m: _gb_kw_frame(m).groupby("k")["a"].agg("sem"),
}


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_GB_KEYWORD_CASES.values()), ids=list(_GB_KEYWORD_CASES))
def test_groupby_reduction_keywords_match_pandas(case: Any) -> None:
    assert _nan_marked(_strict_ordered(case(fpd))) == _nan_marked(_strict_ordered(case(pd)))


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_groupby_reduction_keyword_errors_and_refusals() -> None:
    for m in (pd, fpd):
        gb = _gb_kw_frame(m).groupby("k")
        with pytest.raises(TypeError, match=r"Cannot use numeric_only=True with SeriesGroupBy\.sum"):
            gb["s"].sum(numeric_only=True)
        with pytest.raises(TypeError, match="SeriesGroupBy.sem called with numeric_only=True and dtype object"):
            gb["s"].sem(numeric_only=True)
        # NEGATIVE: without numeric_only a string column still refuses a mean.
        with pytest.raises(TypeError, match="agg function failed"):
            gb.mean()
    gb = _gb_kw_frame(fpd).groupby("k")
    # What the binding cannot run raises instead of being dropped.
    with pytest.raises(NotImplementedError, match="engine='numba'"):
        gb[["a"]].sum(engine="numba")
    with pytest.raises(NotImplementedError, match="ddof=-1"):
        gb[["a"]].std(ddof=-1)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_groupby_option_refusals_and_errors_match_pandas() -> None:
    series = [1.0, 2.0, 3.0]
    for m in (pd, fpd):
        with pytest.raises(TypeError, match="as_index=False only valid with DataFrame"):
            m.Series(series).groupby([1, 1, 2], as_index=False)
        with pytest.raises(TypeError, match="supply one of 'by' and 'level'"):
            _gb_frame(m).groupby()
    # NEGATIVE: options the binding cannot honour raise, never drop.
    # (groupby(level=0) left this list when fvsao.19 implemented it; it is
    # compared with pandas in test_groupby_by_array_like_keys_matches_pandas.
    # dropna=False and group_keys=False left it when they were implemented;
    # see test_centered_windows_reindex_fill_dayfirst_and_groupby_apply_match_pandas.)
    for call in (
        lambda: _gb_frame(fpd).groupby("k", dropna=False)["a"].apply(lambda s: s.sum()),
        lambda: _gb_frame(fpd).groupby("k", as_index=False).apply(lambda d: d["a"].sum(), include_groups=False),
    ):
        with pytest.raises(NotImplementedError):
            call()


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("op", ["mean", "median", "var", "prod", "std", "sem", "skew"])
@pytest.mark.parametrize("select", [None, "s"], ids=["frame", "column"])
def test_groupby_numeric_reductions_refuse_strings_like_pandas(op: str, select: Any) -> None:
    # br-frankenpandas-bcj6d: these returned NaN (or dropped the column) for a
    # string column; pandas raises TypeError or ValueError.
    def call(m: Any) -> Any:
        gb = m.DataFrame({"k": ["y", "x", "y"], "s": ["p", "q", "r"], "a": [1, 2, 3]}).groupby("k")
        return getattr(gb if select is None else gb[select], op)()

    with pytest.raises((TypeError, ValueError)) as expected:
        call(pd)
    with pytest.raises(type(expected.value)) as got:
        call(fpd)
    assert type(got.value) is type(expected.value)
    assert str(expected.value) in str(got.value)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("op", ["mean", "median", "var", "prod", "std", "sem", "skew"])
@pytest.mark.parametrize("select", [None, "a"], ids=["frame", "column"])
def test_groupby_numeric_reductions_match_pandas(op: str, select: Any) -> None:
    # The NEGATIVE of the string refusal above: without a string column these
    # reduce, to pandas' values. sem is pandas' groupby sqrt(var/n), and skew
    # over groups shorter than 3 is float64 NaN (br-frankenpandas-7hxqv).
    def call(m: Any) -> Any:
        gb = m.DataFrame({"k": ["y", "x", "y"], "a": [1.0, 2.0, 4.0]}).groupby("k")
        return getattr(gb if select is None else gb[select], op)()

    assert _nan_marked(_strict(call(fpd))) == _nan_marked(_strict(call(pd)))


def _align_frames(m: Any) -> Any:
    return (
        m.DataFrame({"b": [1.0, 2.0], "a": [3.0, 4.0]}, index=["z", "x"]),
        m.DataFrame({"c": [5.0], "a": [6.0]}, index=["y"]),
    )


# br-frankenpandas-daigh: an outer alignment kept first-seen label order;
# pandas sorts the union of two different indexes (rows and columns).
_ALIGN_CASES = {
    "series_align": lambda m: m.Series([10, 20, 30], index=["x", "y", "z"]).align(
        m.Series([1, 2], index=["x", "w"])
    )[0],
    "series_align_other_side": lambda m: m.Series([10, 20, 30], index=["x", "y", "z"]).align(
        m.Series([1, 2], index=["x", "w"])
    )[1],
    "frame_align": lambda m: _align_frames(m)[0].align(_align_frames(m)[1])[0],
    "frame_add": lambda m: _align_frames(m)[0] + _align_frames(m)[1],
    "frame_add_same_columns_reordered": lambda m: m.DataFrame({"b": [1.0], "a": [2.0]})
    + m.DataFrame({"a": [1.0], "b": [2.0]}),
    # NEGATIVE: equal indexes keep their (unsorted) order; a left join keeps
    # the left order.
    "frame_align_equal_keeps_order": lambda m: _align_frames(m)[0].align(_align_frames(m)[0])[0],
    "series_align_left": lambda m: m.Series([1, 2], index=["z", "a"]).align(
        m.Series([3], index=["b"]), join="left"
    )[0],
}


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_ALIGN_CASES.values()), ids=list(_ALIGN_CASES))
def test_outer_alignment_order_matches_pandas(case: Any) -> None:
    assert _nan_marked(_strict_ordered(case(fpd))) == _nan_marked(_strict_ordered(case(pd)))


def _flex_s(m: Any, vals: Any = (10, 20, 30), idx: Any = ("x", "y", "z")) -> Any:
    return m.Series(list(vals), index=list(idx))


# br-frankenpandas-n57tz: the Series flex methods took `other` alone.
_SERIES_FLEX_CASES = {
    "add_fill_same_index_int": lambda m: _flex_s(m).add(_flex_s(m, (1, 2, 3)), fill_value=0),
    "add_fill_misaligned": lambda m: _flex_s(m).add(_flex_s(m, (1, 2), ("x", "w")), fill_value=0),
    "add_fill_both_missing_stays_missing": lambda m: _flex_s(m, (1.0, _NAN, 3.0)).add(
        _flex_s(m, (_NAN, _NAN, 1.0)), fill_value=0
    ),
    "add_scalar_fill": lambda m: _flex_s(m, (1.0, _NAN, 3.0)).add(5, fill_value=0),
    "sub_fill_float": lambda m: _flex_s(m).sub(_flex_s(m, (1, 2), ("x", "w")), fill_value=1.5),
    "rsub_fill": lambda m: _flex_s(m).rsub(_flex_s(m, (1, 2), ("x", "w")), fill_value=0),
    "mul_fill": lambda m: _flex_s(m).mul(_flex_s(m, (2,), ("y",)), fill_value=1),
    "pow_fill": lambda m: _flex_s(m, (2, 3, 4)).pow(_flex_s(m, (2,), ("x",)), fill_value=1),
    "floordiv_fill": lambda m: _flex_s(m).floordiv(_flex_s(m, (3,), ("x",)), fill_value=1),
    "mod_fill": lambda m: _flex_s(m).mod(_flex_s(m, (3,), ("x",)), fill_value=7),
    "truediv_fill": lambda m: _flex_s(m).truediv(_flex_s(m, (4,), ("x",)), fill_value=2),
    "rtruediv_fill": lambda m: _flex_s(m).rtruediv(_flex_s(m, (4,), ("x",)), fill_value=2),
    "eq_fill": lambda m: _flex_s(m, (1.0, _NAN, 3.0)).eq(_flex_s(m, (1.0, 2.0, _NAN)), fill_value=2.0),
    "lt_fill": lambda m: _flex_s(m, (1.0, _NAN), ("x", "y")).lt(
        _flex_s(m, (2.0, 0.5), ("x", "w")), fill_value=0
    ),
    "divmod_fill": lambda m: _flex_s(m).divmod(_flex_s(m, (3,), ("x",)), fill_value=7)[1],
    "axis_index": lambda m: _flex_s(m).add(_flex_s(m), axis="index"),
    "axis_zero_no_fill": lambda m: _flex_s(m).sub(_flex_s(m, (1, 2, 3)), axis=0),
}


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_SERIES_FLEX_CASES.values()), ids=list(_SERIES_FLEX_CASES))
def test_series_flex_keywords_match_pandas(case: Any) -> None:
    assert _nan_marked(_strict_ordered(case(fpd))) == _nan_marked(_strict_ordered(case(pd)))


def _int_frame(m: Any) -> Any:
    return m.DataFrame({"i": [1, -2, 0, 7], "t": [True, False, True, True], "f": [0.5, 1.5, -2.0, 4.0]})


# br-frankenpandas-c74wi: DataFrame arithmetic made int64 columns float64.
_FRAME_INT_ARITH_CASES = {
    "add_int": lambda m: _int_frame(m) + 3,
    "sub_int": lambda m: _int_frame(m) - 3,
    "mul_int": lambda m: _int_frame(m) * 3,
    "floordiv_int": lambda m: _int_frame(m) // 3,
    "mod_int": lambda m: _int_frame(m) % 3,
    "pow_int": lambda m: _int_frame(m)[["i", "f"]] ** 2,
    "rsub_int": lambda m: 3 - _int_frame(m),
    "radd_int": lambda m: 3 + _int_frame(m),
    "rfloordiv_int_zero_division": lambda m: 10 // _int_frame(m)[["i"]],
    "rmod_int": lambda m: 10 % _int_frame(m)[["i"]].replace(0, 5),
    "floordiv_zero": lambda m: _int_frame(m)[["i"]] // 0,
    "mod_zero": lambda m: _int_frame(m)[["i"]] % 0,
    "overflow_wraps": lambda m: m.DataFrame({"i": [2**62]}) * 4,
    "frame_plus_frame": lambda m: _int_frame(m)[["i", "f"]] + _int_frame(m)[["i", "f"]],
    "frame_floordiv_frame": lambda m: _int_frame(m)[["i"]] // m.DataFrame({"i": [3, 3, 3, 3]}),
    # NEGATIVE: true division, a float scalar and float columns give float64.
    "truediv_int": lambda m: _int_frame(m) / 2,
    "add_float": lambda m: _int_frame(m) + 1.5,
    "rtruediv_int": lambda m: 12 / _int_frame(m)[["i"]].replace(0, 4),
}


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_FRAME_INT_ARITH_CASES.values()), ids=list(_FRAME_INT_ARITH_CASES))
def test_frame_integer_arithmetic_keeps_pandas_dtypes(case: Any) -> None:
    assert _nan_marked(_strict_ordered(case(fpd))) == _nan_marked(_strict_ordered(case(pd)))


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_frame_integer_negative_power_raises_like_pandas() -> None:
    for m in (pd, fpd):
        with pytest.raises(ValueError, match="Integers to negative integer powers are not allowed"):
            _int_frame(m)[["i"]] ** -1


def _fs_frame(m: Any) -> Any:
    return m.DataFrame({"b": [1, 2, 3], "a": [1.5, _NAN, 3.5]}, index=["x", "y", "z"])


def _row_s(m: Any, vals: Any = (1, 2, 3), idx: Any = ("x", "y", "z")) -> Any:
    return m.Series(list(vals), index=list(idx))


# br-frankenpandas-ini2u (a DataFrame took no Series operand) and the
# DataFrame flex keywords axis/fill_value (br-frankenpandas-n57tz).
_FRAME_SERIES_CASES = {
    "plus_series_columns": lambda m: _fs_frame(m) + m.Series([10, 20], index=["a", "b"]),
    "plus_series_extra_label": lambda m: _fs_frame(m) + m.Series([10, 20], index=["a", "c"]),
    "plus_series_float": lambda m: _fs_frame(m) + m.Series([0.5, 1.0], index=["a", "b"]),
    "plus_series_same_order_keeps_columns": lambda m: m.DataFrame({"b": [1], "a": [2]})
    + m.Series([1, 2], index=["b", "a"]),
    "add_axis0": lambda m: _fs_frame(m).add(_row_s(m), axis=0),
    "add_axis0_misaligned": lambda m: _fs_frame(m).add(_row_s(m, (1, 2), ("y", "w")), axis=0),
    "sub_axis_index": lambda m: _fs_frame(m).sub(_row_s(m), axis="index"),
    "rsub_axis0": lambda m: _fs_frame(m).rsub(_row_s(m), axis=0),
    "mul_axis1": lambda m: _fs_frame(m).mul(m.Series([2, 3], index=["a", "b"]), axis=1),
    "truediv_axis0": lambda m: _fs_frame(m).truediv(_row_s(m, (2, 4, 8)), axis=0),
    "floordiv_axis0_zero": lambda m: _fs_frame(m).floordiv(_row_s(m, (2, 0, 8)), axis=0),
    "eq_axis0": lambda m: _fs_frame(m).eq(_row_s(m), axis=0),
    "eq_series_columns": lambda m: _fs_frame(m) == m.Series([1, 1.5], index=["b", "a"]),
    "fill_frame_misaligned": lambda m: _fs_frame(m).add(
        m.DataFrame({"a": [1.0], "c": [2.0]}, index=["y"]), fill_value=0
    ),
    "fill_frame_complete_keeps_int": lambda m: _fs_frame(m)[["b"]].add(_fs_frame(m)[["b"]], fill_value=0),
    "fill_frame_int_beside_nan_column": lambda m: _fs_frame(m).add(_fs_frame(m), fill_value=0),
    "fill_scalar": lambda m: _fs_frame(m).add(1, fill_value=0),
    "axis_ignored_for_scalar": lambda m: _fs_frame(m).add(1, axis=0),
    "axis_ignored_for_frame": lambda m: _fs_frame(m).add(_fs_frame(m), axis=0),
}


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_FRAME_SERIES_CASES.values()), ids=list(_FRAME_SERIES_CASES))
def test_frame_series_arithmetic_and_flex_keywords_match_pandas(case: Any) -> None:
    assert _nan_marked(_strict_ordered(case(fpd))) == _nan_marked(_strict_ordered(case(pd)))


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_frame_flex_keyword_errors_and_refusals() -> None:
    for m in (pd, fpd):
        with pytest.raises(NotImplementedError, match="fill_value 0 not supported"):
            _fs_frame(m).add(_row_s(m), axis=0, fill_value=0)
        with pytest.raises(ValueError, match="No axis named 2 for object type DataFrame"):
            _fs_frame(m).add(_row_s(m), axis=2)
    # level= broadcasts over a MultiIndex level, which the binding cannot.
    with pytest.raises(NotImplementedError, match="level"):
        _fs_frame(fpd).add(fpd.Series([1, 2], index=["a", "b"]), level=0)


def _dr(m: Any) -> Any:
    return m.DataFrame({"a": [1, 2, 3], "b": [4, 5, 6]}, index=["x", "y", "z"])


# br-frankenpandas-n57tz: drop took string row labels and an int axis only,
# and rename's bare mapping renamed the COLUMNS (pandas: the index).
_DROP_RENAME_CASES = {
    "drop_row_label": lambda m: _dr(m).drop("x"),
    "drop_index_kw": lambda m: _dr(m).drop(index=["x", "y"]),
    "drop_columns_kw": lambda m: _dr(m).drop(columns="a"),
    "drop_axis_columns_str": lambda m: _dr(m).drop("a", axis="columns"),
    "drop_axis1": lambda m: _dr(m).drop("a", axis=1),
    "drop_index_and_columns": lambda m: _dr(m).drop(index="x", columns="a"),
    "drop_errors_ignore": lambda m: _dr(m).drop(["q", "x"], errors="ignore"),
    "drop_int_label": lambda m: m.DataFrame({"a": [1, 2]}).drop(1),
    "drop_duplicate_labels": lambda m: m.DataFrame({"a": [1, 2, 3]}, index=["x", "x", "y"]).drop("x"),
    "rename_mapper_targets_index": lambda m: _dr(m).rename({"a": "A", "x": "X"}),
    "rename_columns_dict": lambda m: _dr(m).rename(columns={"a": "A"}),
    "rename_index_dict": lambda m: _dr(m).rename(index={"x": "X"}),
    "rename_callable_axis1": lambda m: _dr(m).rename(str.upper, axis=1),
    "rename_callable_axis_columns": lambda m: _dr(m).rename(str.upper, axis="columns"),
    "rename_columns_callable": lambda m: _dr(m).rename(columns=str.upper),
    "rename_index_callable": lambda m: _dr(m).rename(index=str.upper),
    "rename_errors_ignore_skips": lambda m: _dr(m).rename(columns={"q": "Q", "a": "A"}),
    "set_index_verified": lambda m: _dr(m).set_index("a", verify_integrity=True),
}


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_DROP_RENAME_CASES.values()), ids=list(_DROP_RENAME_CASES))
def test_drop_rename_keywords_match_pandas(case: Any) -> None:
    assert _strict_ordered(case(fpd)) == _strict_ordered(case(pd))


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize(
    "call",
    [
        lambda d: d.drop("x", inplace=True),
        lambda d: d.drop(columns="b", inplace=True),
        lambda d: d.rename(columns={"a": "A"}, inplace=True),
        lambda d: d.rename(index=str.upper, inplace=True),
        lambda d: d.set_index("a", inplace=True),
        lambda d: d.query("a > 1", inplace=True),
        lambda d: d.eval("c = a + b", inplace=True),
        lambda d: d.dropna(inplace=True),
        # fvsao.6: these refused inplace=True.
        lambda d: d.sort_values("a", ascending=False, inplace=True),
        lambda d: d.sort_index(ascending=False, inplace=True),
        lambda d: d.reset_index(inplace=True),
        lambda d: d.reset_index(drop=True, inplace=True),
        lambda d: d.replace(2, 20, inplace=True),
        lambda d: d.clip(2, 5, inplace=True),
        lambda d: d.drop_duplicates(subset=["a"], inplace=True),
    ],
    ids=[
        "drop", "drop_columns", "rename", "rename_callable", "set_index", "query", "eval", "dropna",
        "sort_values", "sort_index", "reset_index", "reset_index_drop", "replace", "clip",
        "drop_duplicates",
    ],
)
def test_inplace_keywords_mutate_and_return_none_like_pandas(call: Any) -> None:
    after = []
    for m in (pd, fpd):
        frame = _dr(m)
        assert call(frame) is None
        after.append(_strict_ordered(frame))
    assert after[1] == after[0]


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_drop_rename_errors_match_pandas() -> None:
    for m in (pd, fpd):
        with pytest.raises(KeyError, match=r"\['q'\] not found in axis"):
            _dr(m).drop(["q"])
        with pytest.raises(KeyError, match=r"\['q', 'r'\] not found in axis"):
            _dr(m).drop(columns=["q", "r"])
        with pytest.raises(ValueError, match="Cannot specify both 'labels' and 'index'/'columns'"):
            _dr(m).drop("x", index="y")
        with pytest.raises(ValueError, match="Need to specify at least one of 'labels', 'index' or 'columns'"):
            _dr(m).drop()
        with pytest.raises(KeyError, match=r"\['q'\] not found in axis"):
            _dr(m).rename(columns={"q": "Q", "a": "A"}, errors="raise")
        with pytest.raises(TypeError, match="Cannot specify both 'mapper' and any of 'index' or 'columns'"):
            _dr(m).rename({"a": "A"}, columns={"b": "B"})
        with pytest.raises(TypeError, match="must pass an index to rename"):
            _dr(m).rename()
        with pytest.raises(ValueError, match=r"Index has duplicate keys: Index\(\[1\]"):
            m.DataFrame({"a": [1, 1]}).set_index("a", verify_integrity=True)
        with pytest.raises(ValueError, match="Cannot operate inplace if there is no assignment"):
            _dr(m).eval("a + 1", inplace=True)
    # What the binding cannot do raises instead of being dropped.
    with pytest.raises(NotImplementedError, match="level"):
        _dr(fpd).drop("x", level=0)


def _ewm_s(m: Any) -> Any:
    return m.Series([1.0, 2.0, _NAN, 4.0, 8.0, 3.0])


def _ewm_df(m: Any) -> Any:
    return m.DataFrame({"a": [1.0, 2.0, _NAN, 4.0], "b": [4, 3, 2, 1]})


# br-frankenpandas-n57tz: ewm took span/alpha only (and span went through
# 2/(span+1), not pandas' com).
_EWM_CASES = {
    "span": lambda m: _ewm_s(m).ewm(span=3).mean(),
    "com": lambda m: _ewm_s(m).ewm(com=0.5).mean(),
    "halflife": lambda m: _ewm_s(m).ewm(halflife=2).mean(),
    "alpha": lambda m: _ewm_s(m).ewm(alpha=0.3).mean(),
    "adjust_false": lambda m: _ewm_s(m).ewm(span=3, adjust=False).mean(),
    "min_periods": lambda m: _ewm_s(m).ewm(span=3, min_periods=3).mean(),
    "sum_halflife": lambda m: _ewm_s(m).ewm(halflife=1.5).sum(),
    "frame_com": lambda m: _ewm_df(m).ewm(com=0.5).mean(),
    "frame_adjust_false_min_periods": lambda m: _ewm_df(m).ewm(span=2, adjust=False, min_periods=2).mean(),
    "std_com": pytest.param(
        lambda m: _ewm_s(m).ewm(com=1).std(),
        marks=pytest.mark.xfail(strict=True, reason="br-frankenpandas-c5nwf: ewm std last-bit order"),
    ),
    "var_adjust_false": pytest.param(
        lambda m: _ewm_s(m).ewm(alpha=0.4, adjust=False).var(),
        marks=pytest.mark.xfail(strict=True, reason="br-frankenpandas-c5nwf: ewm var last-bit order"),
    ),
}


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_EWM_CASES.values()), ids=list(_EWM_CASES))
def test_ewm_keywords_match_pandas(case: Any) -> None:
    assert _nan_marked(_strict_ordered(case(fpd))) == _nan_marked(_strict_ordered(case(pd)))


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize(
    ("kwargs", "message"),
    [
        ({"com": 1, "span": 2}, "comass, span, halflife, and alpha are mutually exclusive"),
        ({}, "Must pass one of comass, span, halflife, or alpha"),
        ({"com": -1}, "comass must satisfy: comass >= 0"),
        ({"span": 0.5}, "span must satisfy: span >= 1"),
        ({"halflife": 0}, "halflife must satisfy: halflife > 0"),
        ({"alpha": 1.5}, "alpha must satisfy: 0 < alpha <= 1"),
    ],
)
def test_ewm_decay_validation_matches_pandas(kwargs: Any, message: str) -> None:
    for m in (pd, fpd):
        with pytest.raises(ValueError, match=message):
            _ewm_s(m).ewm(**kwargs)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_ewm_refuses_what_it_cannot_run() -> None:
    for kwargs in ({"span": 2, "ignore_na": True}, {"halflife": 2, "times": [1, 2, 3, 4, 5, 6]}, {"span": 2, "method": "table"}):
        with pytest.raises(NotImplementedError):
            _ewm_s(fpd).ewm(**kwargs)


def _mk(m: Any) -> Any:
    return m.DataFrame({"k": ["x", "y", "x", "x"], "j": [1, 1, 2, 1], "v": [1.0, 2.0, 3.0, 4.0]})


# br-frankenpandas-rc0923-epic-rust-parity-bugs-4qg5w.9: a multi-key result's
# index surfaced as fp-frame's joined 'x|1' strings; pandas returns a
# MultiIndex of (k, j) tuples named ['k', 'j'].
_MULTI_KEY_CASES = {
    "groupby_sum": lambda m: _mk(m).groupby(["k", "j"]).sum(),
    "groupby_mean": lambda m: _mk(m).groupby(["k", "j"]).mean(),
    "groupby_agg_max": lambda m: _mk(m).groupby(["k", "j"]).agg("max"),
    "groupby_sort_false": lambda m: _mk(m).groupby(["k", "j"], sort=False).sum(),
    "set_index_two_columns": lambda m: _mk(m).set_index(["k", "j"]),
}


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_MULTI_KEY_CASES.values()), ids=list(_MULTI_KEY_CASES))
def test_multi_key_results_carry_pandas_multiindex(case: Any) -> None:
    got, want = case(fpd), case(pd)
    assert type(got.index).__name__ == type(want.index).__name__ == "MultiIndex"
    assert list(got.index.names) == list(want.index.names)
    assert [tuple(label) for label in got.index] == [tuple(label) for label in want.index]
    assert list(got.columns) == list(want.columns)
    for column in want.columns:
        assert got[column].tolist() == want[column].tolist()
    # NEGATIVE: a single-key result keeps a flat Index.
    assert type(_mk(fpd).groupby("k").sum().index).__name__ == "Index"


def _mk_nan(m: Any) -> Any:
    return m.DataFrame({"k": ["x", None, "x", "y"], "j": [1, 1, 2, 2], "v": [1.0, 2.0, 3.0, 4.0]})


def _sgb2(m: Any, **kwargs: Any) -> Any:
    return _mk(m).groupby(["k", "j"], **kwargs)["v"]


# 4qg5w.9: the Series pandas indexes by a MultiIndex - a column of a multi-key
# result, a SeriesGroupBy over several keys (which raised NotImplementedError),
# size, DataFrame.value_counts - and what follows them (positional selection,
# sorting, arithmetic) came back flat, with 'x|1' / 'x, 1' string labels.
_MULTI_KEY_SERIES_CASES = {
    "column_of_groupby_result": lambda m: _mk(m).groupby(["k", "j"]).sum()["v"],
    "column_of_set_index": lambda m: _mk(m).set_index(["k", "j"])["v"],
    "sgb_sum": lambda m: _sgb2(m).sum(),
    "sgb_mean": lambda m: _sgb2(m).mean(),
    "sgb_count": lambda m: _sgb2(m).count(),
    "sgb_max": lambda m: _sgb2(m).max(),
    "sgb_agg_sum": lambda m: _sgb2(m).agg("sum"),
    "sgb_sort_false": lambda m: _sgb2(m, sort=False).sum(),
    "sgb_quantile": lambda m: _sgb2(m).quantile(0.5),
    "sgb_idxmax": lambda m: _sgb2(m).idxmax(),
    "sgb_dropna": lambda m: _mk_nan(m).groupby(["k", "j"])["v"].sum(),
    "sgb_dropna_false": lambda m: _mk_nan(m).groupby(["k", "j"], dropna=False)["v"].sum(),
    "size": lambda m: _mk(m).groupby(["k", "j"]).size(),
    "frame_value_counts": lambda m: _mk(m)[["k", "j"]].value_counts(),
    "frame_value_counts_one_column": lambda m: _mk(m)[["k"]].value_counts(),
    "sort_values": lambda m: _sgb2(m).sum().sort_values(),
    "head": lambda m: _sgb2(m).sum().head(2),
    "iloc_list": lambda m: _sgb2(m).sum().iloc[[2, 0]],
    "times_two": lambda m: _sgb2(m).sum() * 2,
}


def _mi_series(obj: Any) -> Any:
    def key(label: Any) -> Any:
        return tuple(_marker(part) for part in label)

    return (
        type(obj).__name__,
        str(obj.dtype),
        obj.name,
        type(obj.index).__name__,
        list(obj.index.names),
        [key(label) for label in obj.index],
        [_marker(v) for v in obj.tolist()],
        {key(k): _marker(v) for k, v in obj.to_dict().items()},
    )


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize(
    "case", list(_MULTI_KEY_SERIES_CASES.values()), ids=list(_MULTI_KEY_SERIES_CASES)
)
def test_multi_key_series_carry_pandas_multiindex(case: Any) -> None:
    got, want = case(fpd), case(pd)
    assert type(want.index).__name__ == "MultiIndex"
    assert _mi_series(got) == _mi_series(want)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_multi_key_series_reset_into_pandas_columns() -> None:
    def columns(frame: Any) -> Any:
        return [(str(c), [_marker(v) for v in frame[c].tolist()]) for c in frame.columns]

    cases = [
        lambda m: _sgb2(m).sum().reset_index(),
        lambda m: _sgb2(m, as_index=False).sum(),
        lambda m: _mk(m).groupby(["k", "j"]).size().reset_index(),
    ]
    for case in cases:
        assert columns(case(fpd)) == columns(case(pd))
    # NEGATIVE: a flat Series still resets into an 'index' column.
    flat = fpd.Series([1.0, 2.0], name="v").reset_index()
    assert list(flat.columns) == ["index", "v"]


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_multi_key_series_groupby_refuses_what_it_cannot_label() -> None:
    # These results are not relabelled from group codes yet; they raise rather
    # than come back indexed by the codes.
    for op in (
        lambda g: g.value_counts(),
        lambda g: g.unique(),
        lambda g: g.nlargest(1),
        lambda g: g.describe(),
        lambda g: g.apply(lambda s: s.sum()),
        lambda g: g.agg(["sum", "mean"]),
        lambda g: g.get_group(("x", 1)),
    ):
        with pytest.raises(NotImplementedError, match="several keys"):
            op(_sgb2(fpd))
    # NEGATIVE: one key keeps its flat Index and its own labels.
    single = _mk(fpd).groupby("k")["v"].sum()
    assert type(single.index).__name__ == "Index"
    assert list(single.index) == ["x", "y"]


# fvsao.6.3: concat(keys=...) raised NotImplementedError; pandas puts each
# piece's rows under its key in a row MultiIndex.
def _cs1(m: Any) -> Any:
    return m.Series([1.0, 2.0], name="v")


def _cs2(m: Any) -> Any:
    return m.Series([3.0], name="v")


_CONCAT_KEYS_CASES = {
    "series": lambda m: m.concat([_cs1(m), _cs2(m)], keys=["a", "b"]),
    "int_keys": lambda m: m.concat([_cs1(m), _cs2(m)], keys=[1, 2]),
    "names": lambda m: m.concat([_cs1(m), _cs2(m)], keys=["a", "b"], names=["o", "i"]),
    "dict": lambda m: m.concat({"a": _cs1(m), "b": _cs2(m)}),
    "none_dropped_with_its_key": lambda m: m.concat(
        [_cs1(m), None, _cs2(m)], keys=["a", "n", "b"]
    ),
    "shared_index_name": lambda m: m.concat(
        [_cs1(m).rename_axis("r"), _cs2(m).rename_axis("r")], keys=["a", "b"]
    ),
    "mixed_index_names": lambda m: m.concat(
        [_cs1(m).rename_axis("r"), _cs2(m)], keys=["a", "b"]
    ),
    "frames": lambda m: m.concat([_mk(m).head(2), _mk(m).tail(1)], keys=["a", "b"]),
    "frames_inner": lambda m: m.concat(
        [_mk(m).head(2), _mk(m)[["v"]].tail(1)], keys=["a", "b"], join="inner"
    ),
}


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_CONCAT_KEYS_CASES.values()), ids=list(_CONCAT_KEYS_CASES))
def test_concat_keys_match_pandas(case: Any) -> None:
    got, want = case(fpd), case(pd)
    assert type(got).__name__ == type(want).__name__
    assert type(got.index).__name__ == type(want.index).__name__ == "MultiIndex"
    assert list(got.index.names) == list(want.index.names)
    assert [tuple(label) for label in got.index] == [tuple(label) for label in want.index]
    if hasattr(want, "columns"):
        assert list(got.columns) == list(want.columns)
        for column in want.columns:
            assert [_marker(v) for v in got[column].tolist()] == [
                _marker(v) for v in want[column].tolist()
            ]
    else:
        assert _mi_series(got) == _mi_series(want)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_concat_keys_side_cases_match_pandas() -> None:
    for m in (pd, fpd):
        # ignore_index drops the keys; side by side, the keys name the columns.
        flat = m.concat([_cs1(m), _cs2(m)], keys=["a", "b"], ignore_index=True)
        assert list(flat.index) == [0, 1, 2]
        assert flat.tolist() == [1.0, 2.0, 3.0]
        wide = m.concat([_cs1(m), _cs2(m)], keys=["a", "b"], axis=1)
        assert list(wide.columns) == ["a", "b"]


def _gwin(m: Any) -> Any:
    return m.DataFrame({"k": ["x", "y", "x", "y"], "v": [1.0, 2.0, 3.0, 4.0]}).groupby("k")


_GROUPED_WINDOWS = {
    "rolling": lambda g: g.rolling(2).mean(),
    "expanding": lambda g: g.expanding().mean(),
    "ewm": lambda g: g.ewm(span=2).mean(),
    "column_rolling": lambda g: g["v"].rolling(2).mean(),
    "column_expanding": lambda g: g["v"].expanding().mean(),
    "column_ewm": lambda g: g["v"].ewm(span=2).mean(),
}


def _mi_result(obj: Any) -> Any:
    def key(label: Any) -> Any:
        return tuple(_marker(part) for part in label)

    head = (
        type(obj).__name__,
        type(obj.index).__name__,
        list(obj.index.names),
        [key(label) for label in obj.index],
    )
    if hasattr(obj, "columns"):
        return head + (
            [str(c) for c in obj.columns],
            {str(c): (str(obj[c].dtype), [_marker(v) for v in obj[c].tolist()]) for c in obj.columns},
        )
    return head + (obj.name, str(obj.dtype), [_marker(v) for v in obj.tolist()])


def _gw_frame(m: Any) -> Any:
    return m.DataFrame(
        {
            "k": ["b", "a", "b", "a", "b"],
            "j": [1, 1, 1, 2, 1],
            "v": [1.0, 2.0, 3.0, 4.0, 5.0],
            "w": [5, 4, 3, 2, 1],
        },
        index=[10, 11, 12, 13, 14],
    )


# br-frankenpandas-pbpli: pandas runs the window within each group and nests
# the result under (key..., row label), groups in the groupby's order.
_GROUPED_WINDOW_FRAMES = {
    "rolling_sum": lambda m: _gw_frame(m).groupby("k")["v"].rolling(2).sum(),
    "rolling_sort_false": lambda m: _gw_frame(m).groupby("k", sort=False)["v"].rolling(2).sum(),
    "rolling_min_periods": lambda m: _gw_frame(m).groupby("k")["v"].rolling(2, min_periods=1).max(),
    "rolling_count": lambda m: _gw_frame(m).groupby("k")["v"].rolling(2).count(),
    "rolling_std": lambda m: _gw_frame(m).groupby("k")["v"].rolling(2).std(),
    "expanding_max": lambda m: _gw_frame(m).groupby("k")["v"].expanding().max(),
    "frame_rolling": lambda m: _gw_frame(m).groupby("k").rolling(2).sum(),
    "frame_rolling_selection": lambda m: _gw_frame(m).groupby("k")[["v"]].rolling(2).sum(),
    "two_keys": lambda m: _gw_frame(m).groupby(["k", "j"])["v"].rolling(2).sum(),
    "two_keys_frame": lambda m: _gw_frame(m).groupby(["k", "j"]).rolling(2).sum(),
    "series_by_series": lambda m: _gw_frame(m)["v"].groupby(_gw_frame(m)["k"]).rolling(2).sum(),
    "named_index": lambda m: _gw_frame(m).rename_axis("r").groupby("k")["v"].rolling(2).sum(),
    "frame_ewm_alpha": lambda m: _gw_frame(m)[["k", "v"]].groupby("k").ewm(alpha=0.5).mean(),
}


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_GROUPED_WINDOWS.values()), ids=list(_GROUPED_WINDOWS))
def test_grouped_windows_match_pandas(case: Any) -> None:
    assert _mi_result(case(_gwin(fpd))) == _mi_result(case(_gwin(pd)))


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize(
    "case", list(_GROUPED_WINDOW_FRAMES.values()), ids=list(_GROUPED_WINDOW_FRAMES)
)
def test_grouped_window_shapes_match_pandas(case: Any) -> None:
    assert _mi_result(case(fpd)) == _mi_result(case(pd))


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_grouped_windows_run_within_each_group() -> None:
    # NEGATIVE: the old binding ran ONE window over all rows, [nan, 3.0, 5.0,
    # 7.0, 9.0]; within the groups the sums are a: [nan, 6.0], b: [nan, 4.0, 8.0].
    frame = _gw_frame(fpd)
    grouped = frame.groupby("k")["v"].rolling(2).sum()
    assert _marker(grouped.tolist()[1]) == 6.0
    assert [_marker(v) for v in grouped.tolist()] != [
        _marker(v) for v in frame["v"].rolling(2).sum().tolist()
    ]
    # One group is the ungrouped window.
    one = fpd.DataFrame({"k": ["x"] * 4, "v": [1.0, 2.0, 3.0, 4.0]})
    assert one.groupby("k")["v"].ewm(span=2).mean().tolist() == one["v"].ewm(span=2).mean().tolist()
    # The window's own argument checks apply when the grouped window is made.
    with pytest.raises(Exception) as ungrouped:
        frame["v"].rolling(-1)
    with pytest.raises(type(ungrouped.value)):
        frame.groupby("k")["v"].rolling(-1)
    # A shape pandas builds another way is refused, not faked.
    with pytest.raises(NotImplementedError, match="as_index=False"):
        frame.groupby("k", as_index=False)["v"].rolling(2)


def _sr(m: Any) -> Any:
    return m.Series([1, 2, 3], index=["a", "b", "c"], name="v")


# br-frankenpandas-n57tz: Series.rename took a str name only, Series.drop
# positional labels only.
_SERIES_DROP_RENAME_CASES = {
    "rename_name": lambda m: _sr(m).rename("x"),
    "rename_none_clears_name": lambda m: _sr(m).rename(None),
    "rename_no_argument_clears_name": lambda m: _sr(m).rename(),
    "rename_dict_relabels": lambda m: _sr(m).rename({"a": "A"}),
    "rename_callable_relabels": lambda m: _sr(m).rename(str.upper),
    "rename_index_kw": lambda m: _sr(m).rename(index={"a": "A"}),
    "drop_label": lambda m: _sr(m).drop("a"),
    "drop_index_kw": lambda m: _sr(m).drop(index=["a", "b"]),
    "drop_errors_ignore": lambda m: _sr(m).drop(["q", "a"], errors="ignore"),
    "drop_columns_ignored": lambda m: _sr(m).drop(columns="x"),
}


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_SERIES_DROP_RENAME_CASES.values()), ids=list(_SERIES_DROP_RENAME_CASES))
def test_series_drop_rename_keywords_match_pandas(case: Any) -> None:
    assert _strict_ordered(case(fpd)) == _strict_ordered(case(pd))


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_series_drop_rename_inplace_and_errors_match_pandas() -> None:
    for m in (pd, fpd):
        s = _sr(m)
        assert s.drop("a", inplace=True) is None
        assert s.tolist() == [2, 3]
        t = _sr(m)
        assert t.rename(str.upper, inplace=True) is None
        assert list(t.index) == ["A", "B", "C"]
        # pandas returns the Series itself for a new name, even inplace.
        u = _sr(m)
        assert u.rename("w", inplace=True).name == "w"
        assert u.name == "w"
        with pytest.raises(KeyError, match=r"\['q'\] not found in axis"):
            _sr(m).drop(["q"])
        with pytest.raises(KeyError, match=r"\['q'\] not found in axis"):
            _sr(m).rename({"q": "Q"}, errors="raise")
        with pytest.raises(ValueError, match="Need to specify at least one of 'labels', 'index' or 'columns'"):
            _sr(m).drop()
        with pytest.raises(ValueError, match="No axis named 1 for object type Series"):
            _sr(m).drop("a", axis=1)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_set_index_duplicate_keys_message_is_pandas_exactly() -> None:
    messages = []
    for m in (pd, fpd):
        with pytest.raises(ValueError) as err:
            m.DataFrame({"a": [1, 1]}).set_index("a", verify_integrity=True)
        messages.append(str(err.value))
    assert messages[1] == messages[0]


def _nan_pair(m: Any) -> Any:
    return m.Series([1.0, _NAN, 3.0]), m.Series([2.0, _NAN, 1.0])


# br-frankenpandas-zwfz3: a comparison with a missing value was missing
# (None); pandas' numpy dtypes compare it False, and True under !=.
_NAN_COMPARISON_CASES = {
    "lt": lambda m: _nan_pair(m)[0] < _nan_pair(m)[1],
    "eq": lambda m: _nan_pair(m)[0] == _nan_pair(m)[1],
    "ne": lambda m: _nan_pair(m)[0] != _nan_pair(m)[1],
    "ge_scalar": lambda m: _nan_pair(m)[0] >= 2,
    "ne_scalar": lambda m: _nan_pair(m)[0] != 3.0,
    "eq_none": lambda m: m.Series([1.0, 2.0]) == None,  # noqa: E711
    "string_eq": lambda m: m.Series(["a", None, "c"]) == "a",
    "flex_lt_misaligned": lambda m: m.Series([1.0], index=["x"]).lt(m.Series([2.0], index=["y"])),
    "frame_gt_scalar": lambda m: m.DataFrame({"a": [1.0, _NAN], "b": [3, 4]}) > 1,
    "frame_eq_frame": lambda m: m.DataFrame({"a": [1.0, _NAN]}).eq(m.DataFrame({"a": [1.0, 2.0]})),
    "frame_ne_frame": lambda m: m.DataFrame({"a": [1.0, _NAN]}).ne(m.DataFrame({"a": [1.0, 2.0]})),
}


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_NAN_COMPARISON_CASES.values()), ids=list(_NAN_COMPARISON_CASES))
def test_comparisons_with_missing_values_match_pandas(case: Any) -> None:
    assert _nan_marked(_strict_ordered(case(fpd))) == _nan_marked(_strict_ordered(case(pd)))


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_comparison_operators_refuse_different_labels_like_pandas() -> None:
    for m in (pd, fpd):
        with pytest.raises(ValueError, match="Can only compare identically-labeled Series objects"):
            m.Series([1.0], index=["x"]) < m.Series([2.0], index=["y"])
        # NEGATIVE: identically labeled Series compare, and so does a scalar.
        assert (m.Series([1.0], index=["x"]) < m.Series([2.0], index=["x"])).tolist() == [True]


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_series_flex_keyword_errors_and_refusals() -> None:
    for m in (pd, fpd):
        with pytest.raises(ValueError, match="No axis named 1 for object type Series"):
            _flex_s(m).add(_flex_s(m), axis=1)
    # level= broadcasts over a MultiIndex level, which the binding cannot.
    with pytest.raises(NotImplementedError, match="level"):
        _flex_s(fpd).add(_flex_s(fpd), level=0)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_sample_without_random_state_draws_fresh_rows() -> None:
    # br-frankenpandas-u1e54: random_state=None meant seed 42, so every
    # unseeded sample was the same draw; pandas draws anew each call.
    frame = fpd.DataFrame({"v": list(range(1000))})
    first = list(frame.sample(5).index)
    assert any(list(frame.sample(5).index) != first for _ in range(8))
    wide = fpd.DataFrame({f"c{i}": [i] for i in range(200)})
    first_cols = list(wide.sample(5, axis=1).columns)
    assert any(list(wide.sample(5, axis=1).columns) != first_cols for _ in range(8))
    # NEGATIVE: a seed still repeats its draw.
    assert list(frame.sample(5, random_state=7).index) == list(frame.sample(5, random_state=7).index)




@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("freq", ["D", "2D", "W", "ME", "h"])
@pytest.mark.parametrize("agg", ["sum", "mean", "count", "first"])
def test_resample_bins_are_timestamps_like_pandas(freq: str, agg: str) -> None:
    # br-frankenpandas-0yilt: the bins came back as date STRINGS ('2024-01-01');
    # pandas returns a DatetimeIndex of Timestamps (the index class too:
    # it was a plain Index until fvsao.18).
    def bins(m: Any) -> Any:
        idx = m.to_datetime(
            ["2024-01-01 00:00", "2024-01-01 06:00", "2024-01-02 00:00", "2024-01-05 00:00"]
        )
        out = getattr(m.Series([1.0, 2.0, 3.0, 4.0], index=idx, name="v").resample(freq), agg)()
        return (
            type(out.index).__name__,
            [type(t).__name__ for t in out.index],
            [str(t) for t in out.index],
            [_marker(v) for v in out.tolist()],
        )

    got, want = bins(fpd), bins(pd)
    assert got == want
    # NEGATIVE: not one bin is text, and the index is no plain Index.
    assert set(got[1]) == {"Timestamp"}
    assert got[0] == "DatetimeIndex"


# br-frankenpandas-hrxn9: the category dtype. astype('category'), Series(...,
# dtype='category'), Categorical(...) in a Series or a frame all raised; the
# Categorical class stringified every value; .cat on any Series returned empty
# categories and made-up codes.
def _category(obj: Any) -> Any:
    if hasattr(obj, "columns"):
        return (
            "DataFrame",
            [str(obj[c].dtype) for c in obj.columns],
            {c: _category(obj[c]) for c in obj.columns},
        )
    out = ("Series", str(obj.dtype), obj.name, [_marker(v) for v in obj.tolist()])
    if str(obj.dtype) == "category":
        out += (
            [_marker(c) for c in obj.cat.categories],
            obj.cat.codes.tolist(),
            obj.cat.ordered,
        )
    return out


_CATEGORY_CASES = {
    "astype": lambda m: m.Series(["b", "a", "b"], name="s").astype("category"),
    "dtype_keyword": lambda m: m.Series(["b", "a", None, "b"], dtype="category"),
    "positional_dtype": lambda m: m.Series([1, 2], None, "float64", "s"),
    "numbers": lambda m: m.Series([3, 1, 3, 2]).astype("category"),
    "categorical_inferred": lambda m: m.Series(m.Categorical([2, 1, None, 2])),
    "categorical_given": lambda m: m.Series(
        m.Categorical(["b", "a", "z"], categories=["b", "a", "c"], ordered=True)
    ),
    "frame_categorical": lambda m: m.DataFrame({"c": m.Categorical(["x", "y", "x"]), "v": [1, 2, 3]}),
    "frame_astype_dict": lambda m: m.DataFrame({"c": ["x", "y", "x"], "v": [1, 2, 3]}).astype(
        {"c": "category"}
    ),
    "column_of_frame": lambda m: m.DataFrame({"c": m.Categorical(["x", "y"])})["c"],
    "value_counts_keeps_unused": lambda m: m.Series(m.Categorical(["a", "a"], categories=["a", "b"])).value_counts(),
    "sort_by_category_order": lambda m: m.Series(
        m.Categorical(["lo", "hi", "mid"], categories=["lo", "mid", "hi"], ordered=True)
    ).sort_values(),
    "equals_scalar": lambda m: m.Series(["b", "a", "b"]).astype("category") == "b",
    "rename": lambda m: m.Series(["b", "a"]).astype("category").cat.rename_categories(["B", "A"]),
    "add": lambda m: m.Series(["b", "a"]).astype("category").cat.add_categories(["c"]),
    "remove": lambda m: m.Series(["b", "a"]).astype("category").cat.remove_categories(["a"]),
    "remove_unused": lambda m: m.Series(m.Categorical(["a"], categories=["a", "b"])).cat.remove_unused_categories(),
    "reorder": lambda m: m.Series(["b", "a"]).astype("category").cat.reorder_categories(["b", "a"]),
    "set": lambda m: m.Series(["b", "a"]).astype("category").cat.set_categories(["a", "z"]),
    "as_ordered": lambda m: m.Series(["b", "a"]).astype("category").cat.as_ordered(),
    "astype_str": lambda m: m.Series(["b", None]).astype("category").astype(str),
    "head": lambda m: m.Series(["b", "a", "c"]).astype("category").head(2),
    "groupby_observed": lambda m: m.DataFrame({"c": m.Categorical(["y", "x", "y"]), "v": [1, 2, 3]})
    .groupby("c", observed=True)["v"]
    .sum(),
}


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_CATEGORY_CASES.values()), ids=list(_CATEGORY_CASES))
def test_category_dtype_matches_pandas(case: Any) -> None:
    got, want = case(fpd), case(pd)
    if hasattr(want, "columns"):
        assert _category(got) == _category(want)
        return
    got_c, want_c = _category(got), _category(want)
    # pandas' codes are int8; the codes themselves must agree.
    assert got_c == want_c


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_category_refusals_and_errors_match_pandas() -> None:
    for m in (pd, fpd):
        # NEGATIVE: .cat on a Series that is not categorical raises.
        with pytest.raises(AttributeError, match="category"):
            m.Series([1, 2]).cat
        # An unordered categorical has no min.
        with pytest.raises(TypeError):
            m.Series(["b", "a"]).astype("category").min()
    # observed=False (pandas' default) adds the unused categories as groups
    # (fvsao.39; it raised NotImplementedError).
    for m in (pd, fpd):
        frame = m.DataFrame({"c": m.Categorical(["x"], categories=["x", "y"]), "v": [1]})
        with pytest.warns(FutureWarning, match="observed=False"):
            summed = frame.groupby("c")["v"].sum()
        assert ([str(k) for k in summed.index], summed.tolist()) == (["x", "y"], [1, 0])
    # The Categorical class keeps value types (it stringified them).
    assert fpd.Categorical([1, None, 2]).tolist()[0] == 1
    assert list(fpd.Categorical([2, 1]).categories) == [1, 2]


# fvsao.6.3's acceptance: merge/concat keywords in combination, not one at a
# time. The full grids (merge 4x2x3x2x3x2 = 288, concat 2x2x2x2x2 = 32) must
# each match pandas 2.2.3 or refuse with NotImplementedError.
def _grid_describe(obj: Any) -> Any:
    if hasattr(obj, "columns"):
        columns = [obj.iloc[:, i] for i in range(len(obj.columns))]
        return (
            "DataFrame",
            [str(c) for c in obj.columns],
            [str(c.dtype) for c in columns],
            [_marker(x) for x in obj.index],
            [[_marker(v) for v in c.tolist()] for c in columns],
        )
    return ("Series", str(obj.dtype), [_marker(x) for x in obj.index], [_marker(v) for v in obj.tolist()])


def _grid_run(fn: Any) -> Any:
    try:
        return ("ok", _grid_describe(fn()))
    except NotImplementedError:
        return ("refused",)
    except Exception as exc:  # noqa: BLE001
        return ("raise", type(exc).__name__)


def _merge_grid_case(m: Any, how: str, suffixes: Any, indicator: Any, sort: bool, keys: str, validate: Any) -> Any:
    left = m.DataFrame({"k": ["a", "b", "c"], "v": [1, 2, 3], "x": [10, 20, 30]})
    right = m.DataFrame({"k": ["b", "c", "d"], "w": [4, 5, 6], "x": [40, 50, 60]})
    kw: dict[str, Any] = {"how": how, "sort": sort}
    if suffixes is not None:
        kw["suffixes"] = suffixes
    if indicator is not False:
        kw["indicator"] = indicator
    if validate is not None:
        kw["validate"] = validate
    if keys == "on":
        kw["on"] = "k"
    elif keys == "left_right_on":
        right = right.rename(columns={"k": "k2"})
        kw.update(left_on="k", right_on="k2")
    else:
        left, right = left.set_index("k"), right.set_index("k")
        kw.update(left_index=True, right_index=True)
    return left.merge(right, **kw)


_MERGE_GRID = list(
    itertools.product(
        ["inner", "left", "right", "outer"],
        [None, ("_l", "_r")],
        [False, True, "src"],
        [False, True],
        ["on", "left_right_on", "index"],
        [None, "1:1"],
    )
)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_merge_keyword_grid_matches_pandas() -> None:
    bad = []
    for combo in _MERGE_GRID:
        got = _grid_run(lambda: _merge_grid_case(fpd, *combo))
        want = _grid_run(lambda: _merge_grid_case(pd, *combo))
        if got != ("refused",) and got != want:
            bad.append(combo)
    assert bad == []


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_concat_keyword_grid_matches_pandas() -> None:
    # br-frankenpandas-5ihhi: every axis=1 case here shares a column name, and
    # the binding read the first same-named column for both.
    def objs(m: Any, kind: str) -> Any:
        if kind == "frames":
            return [m.DataFrame({"a": [1, 2], "b": [3, 4]}), m.DataFrame({"a": [5], "c": [6]})]
        return [m.Series([1, 2], name="s"), m.Series([3], name="s")]

    bad = []
    for axis, ignore_index, join, keys, kind in itertools.product(
        [0, 1], [False, True], ["outer", "inner"], [None, ["p", "q"]], ["frames", "series"]
    ):
        kw: dict[str, Any] = {"axis": axis, "ignore_index": ignore_index, "join": join}
        if keys is not None:
            kw["keys"] = keys
        got = _grid_run(lambda: fpd.concat(objs(fpd, kind), **kw))
        want = _grid_run(lambda: pd.concat(objs(pd, kind), **kw))
        if got != ("refused",) and got != want:
            bad.append((axis, ignore_index, join, keys, kind))
    assert bad == []


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_duplicate_column_labels_keep_their_own_data() -> None:
    # br-frankenpandas-5ihhi: pandas keeps every same-named column's data.
    for m in (pd, fpd):
        side = m.concat([m.Series([1, 2], name="s"), m.Series([3, 4], name="s")], axis=1)
        assert [list(map(int, row)) for row in side.values] == [[1, 3], [2, 4]]
        assert side.iloc[:, 1].tolist() == [3, 4]
        assert side.iloc[:, [1, 0]].iloc[:, 0].tolist() == [3, 4]
        assert side.to_dict("split")["data"] == [[1, 3], [2, 4]]
        built = m.DataFrame([[1, 3], [2, 4]], columns=["s", "s"])
        assert [list(map(int, row)) for row in built.values] == [[1, 3], [2, 4]]
        assert built.iloc[:, 0].tolist() == [1, 2]
    # NEGATIVE: distinct names are untouched.
    assert fpd.concat([fpd.Series([1], name="a"), fpd.Series([2], name="b")], axis=1).iloc[:, 1].tolist() == [2]


ARRAY_LIKE_COLUMN_VALUES = {
    "np_int": lambda m: np.array([1, 2]),
    "np_float": lambda m: np.array([1.5, np.nan]),
    "np_str": lambda m: np.array(["a", "b"]),
    "np_bool": lambda m: np.array([True, False]),
    "np_dt64": lambda m: np.array(["2020-01-05T06:07:08.000000009", "NaT"], dtype="datetime64[ns]"),
    "np_td64": lambda m: np.array([3_600_000_000_000, -7], dtype="timedelta64[ns]"),
    "range": lambda m: range(2),
    "dti": lambda m: m.to_datetime(["2020-01-05", "2020-01-02"]),
    "dti_nat": lambda m: m.to_datetime(["2020-01-05", None]),
    "date_range": lambda m: m.date_range("2020-01-01", periods=2),
    "tdi": lambda m: m.to_timedelta(["1D", "2h"]),
    "index_int": lambda m: m.Index([7, 8]),
    "index_str": lambda m: m.Index(["p", "q"]),
    "list_datetime": lambda m: [datetime.datetime(2020, 1, 5, 6, 7, 8, 9), datetime.datetime(2020, 1, 2)],
    "list_datetime_none": lambda m: [datetime.datetime(2020, 1, 5), None],
    "list_timedelta": lambda m: [datetime.timedelta(days=1, seconds=2, microseconds=3), datetime.timedelta(hours=-1)],
    "list_np_dt64": lambda m: [np.datetime64("2020-01-05"), np.datetime64("NaT")],
}


def _with_column(m: Any, value: Any) -> Any:
    df = m.DataFrame({"x": [1, 2]})
    df["c"] = value
    return df["c"]


ARRAY_LIKE_COLUMN_PATHS = {
    "Series": lambda m, v: m.Series(v),
    "Series(index=)": lambda m, v: m.Series(v, index=[10, 11]),
    "DataFrame(dict)": lambda m, v: m.DataFrame({"c": v})["c"],
    "setitem": _with_column,
    "assign": lambda m, v: m.DataFrame({"x": [1, 2]}).assign(c=v)["c"],
}


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("path", sorted(ARRAY_LIKE_COLUMN_PATHS))
def test_array_like_column_values_match_pandas(path: str) -> None:
    # br-frankenpandas-rc0923-epic-python-honest-dropin-fvsao.15: numpy arrays,
    # ranges, Index/DatetimeIndex/TimedeltaIndex and datetime/timedelta
    # objects build the column pandas builds (the DataFrame paths raised
    # "Cannot convert ndarray to Scalar"; a DatetimeIndex became strings, NaT
    # as 1970-01-01).
    build = ARRAY_LIKE_COLUMN_PATHS[path]
    for name, make in ARRAY_LIKE_COLUMN_VALUES.items():
        got, want = (build(m, make(m)) for m in (fpd, pd))
        assert str(got.dtype) == str(want.dtype), (name, got.dtype, want.dtype)
        assert [str(v) for v in got.tolist()] == [str(v) for v in want.tolist()], name
        assert [str(v) for v in got.isna().tolist()] == [str(v) for v in want.isna().tolist()], name
        assert [str(v) for v in got.index] == [str(v) for v in want.index], name


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_array_like_column_refusals_match_pandas() -> None:
    # fvsao.15 negatives: what pandas refuses stays refused, and a list of
    # date STRINGS stays object (no datetime inference).
    for m in (pd, fpd):
        assert str(m.Series(["2020-01-05"]).dtype) == "object"
        assert str(m.DataFrame({"c": ["2020-01-05"]})["c"].dtype) == "object"
        assert m.Series(m.Index([1, 2], name="a")).name == "a"
        assert m.Series(m.to_datetime(["2020-01-01"]).rename("d")).name == "d"
        with pytest.raises(TypeError, match="unordered"):
            m.Series({1, 2})
        with pytest.raises(TypeError, match="unordered"):
            m.DataFrame({"c": {1, 2}})
        with pytest.raises(ValueError):
            m.Series(np.array([[1, 2]]))
        with pytest.raises(ValueError):
            m.DataFrame({"x": [1, 2]}).__setitem__("c", np.array([1, 2, 3]))
        with pytest.raises(ValueError):
            m.DataFrame({"c": np.array([1, 2]), "d": [1, 2, 3]})
    # pandas builds a datetime64[ns, UTC] column from tz-aware datetimes.
    # TEST-CHANGE (fvsao.35): the binding refused them (NotImplementedError)
    # while its columns could not carry a zone; it now builds the aware
    # column (test_tz_aware_columns_follow_their_wall_clock_like_pandas).
    aware = [datetime.datetime(2020, 1, 1, tzinfo=datetime.timezone.utc)]
    assert str(pd.Series(aware).dtype) == "datetime64[ns, UTC]"
    assert str(fpd.Series(aware).dtype) == "datetime64[ns, UTC]"


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.xfail(
    strict=True,
    reason="br-frankenpandas-rc0923-epic-python-honest-dropin-fvsao.16: only nanosecond resolution",
)
def test_numpy_second_resolution_is_kept_like_pandas() -> None:
    values = np.array(["2020-01-05", "NaT"], dtype="datetime64[D]")
    assert str(fpd.Series(values).dtype) == str(pd.Series(values).dtype) == "datetime64[s]"


def _dt_column(m: Any) -> Any:
    return m.Series(m.to_datetime(["2020-01-05", "2020-01-02", None, "2020-01-03"]), name="d")


def _td_column(m: Any) -> Any:
    return m.Series(m.to_timedelta(["1D", "2h", None, "30min"]), name="t")


def _keyed_dates(m: Any) -> Any:
    return m.DataFrame({"k": [1, 1, 2, 2], "d": _dt_column(m)}).groupby("k")["d"]


TEMPORAL_COLUMN_OPS = {
    "min": lambda m: _dt_column(m).min(),
    "max": lambda m: _dt_column(m).max(),
    "mean": lambda m: _dt_column(m).mean(),
    "median": lambda m: _dt_column(m).median(),
    "std": lambda m: _dt_column(m).std(),
    "idxmax": lambda m: _dt_column(m).idxmax(),
    "quantile": lambda m: _dt_column(m).quantile(0.25),
    "sort_values": lambda m: _dt_column(m).sort_values(),
    "sort_values_desc_na_first": lambda m: _dt_column(m).sort_values(ascending=False, na_position="first"),
    "rank": lambda m: _dt_column(m).rank(),
    "shift": lambda m: _dt_column(m).shift(1),
    "diff": lambda m: _dt_column(m).diff(),
    "gt_str": lambda m: _dt_column(m) > "2020-01-02",
    "eq_unparseable_str": lambda m: _dt_column(m) == "not a date",
    "ne_unparseable_str": lambda m: _dt_column(m) != "not a date",
    "between_str": lambda m: _dt_column(m).between("2020-01-02", "2020-01-03"),
    "clip_ts": lambda m: _dt_column(m).clip(lower=m.Timestamp("2020-01-03")),
    "clip_str": lambda m: _dt_column(m).clip(upper="2020-01-03"),
    "describe": lambda m: _dt_column(m).describe(),
    "groupby_max": lambda m: _keyed_dates(m).max(),
    "groupby_min": lambda m: _keyed_dates(m).min(),
    "dt.day_name": lambda m: _dt_column(m).dt.day_name(),
    "dt.month_name": lambda m: _dt_column(m).dt.month_name(),
    "dt.strftime": lambda m: _dt_column(m).dt.strftime("%Y/%m/%d"),
    "dt.floor": lambda m: _dt_column(m).dt.floor("D"),
    "dt.is_month_start": lambda m: _dt_column(m).dt.is_month_start,
    "td_mean": lambda m: _td_column(m).mean(),
    "td_median": lambda m: _td_column(m).median(),
    "td_std": lambda m: _td_column(m).std(),
    "td_sort": lambda m: _td_column(m).sort_values(),
    "td_describe": lambda m: _td_column(m).describe(),
    "td_total_seconds": lambda m: _td_column(m).dt.total_seconds(),
    "td_mul_int": lambda m: _td_column(m) * 2,
    "td_rmul_float": lambda m: 1.5 * _td_column(m),
    "td_div_td": lambda m: _td_column(m) / m.Timedelta("1h"),
    "td_div_int": lambda m: _td_column(m) / 3,
    "td_floordiv_td": lambda m: _td_column(m) // m.Timedelta("7min"),
    "td_mod_td": lambda m: _td_column(m) % m.Timedelta("7min"),
}


def _temporal_result(r: Any) -> Any:
    if hasattr(r, "dtype") and hasattr(r, "tolist"):
        return (str(r.dtype), [str(v) for v in r.tolist()], [str(v) for v in r.index])
    return (type(r).__name__, str(r))


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("op", sorted(TEMPORAL_COLUMN_OPS))
def test_temporal_column_ops_match_pandas(op: str) -> None:
    # br-frankenpandas-rc0923-epic-python-honest-dropin-fvsao.17: sort_values
    # left a datetime column unsorted, rank/groupby max/td division gave NaN,
    # min/max/mean/quantile and string comparisons raised, and the .dt methods
    # were missing.
    run = TEMPORAL_COLUMN_OPS[op]
    assert _temporal_result(run(fpd)) == _temporal_result(run(pd))


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_temporal_column_refusals_match_pandas() -> None:
    # fvsao.17 negatives: what pandas refuses on a datetime/timedelta column
    # stays refused (same exception class).
    for m in (pd, fpd):
        d, t = _dt_column(m), _td_column(m)
        for refused in (d.sum, d.var, d.prod, t.var, t.prod):
            with pytest.raises(TypeError):
                refused()
        with pytest.raises(TypeError):
            d > "not a date"
        with pytest.raises(TypeError):
            t * t
    # pandas' .dt.date is datetime.date objects; the binding would give
    # strings, so it is not exposed rather than silently mistyped.
    with pytest.raises(AttributeError):
        _dt_column(fpd).dt.date
    # NEGATIVE: a numeric column's sort/rank/min are unchanged.
    for m in (pd, fpd):
        s = m.Series([3.0, 1.0, float("nan"), 2.0])
        assert [str(v) for v in s.sort_values().tolist()] == ["1.0", "2.0", "3.0", "nan"]
        assert [str(v) for v in s.rank().tolist()] == ["3.0", "1.0", "nan", "2.0"]
        assert s.min() == 1.0


RESAMPLE_OPS = ["sum", "mean", "count", "min", "max", "prod", "first", "last", "median", "std", "var", "nunique", "size"]
# Two rows on 2024-01-01 and two on 2024-01-03: resample('D') has an empty
# middle bin.
RESAMPLE_DTYPE_VALUES = {
    "str": lambda m: ["a", "b", "c", "d"],
    "str_with_none": lambda m: ["a", None, None, None],
    "bool": lambda m: [True, False, True, True],
    "datetime": lambda m: m.to_datetime(["2020-01-05", "2020-01-02", "2020-01-03", "2020-01-04"]),
    "timedelta": lambda m: m.to_timedelta(["1D", "2D", "3h", "4h"]),
    "float_nan": lambda m: [1.0, float("nan"), 3.0, 4.0],
}


def _resample_outcome(run: Any) -> Any:
    try:
        r = run()
    except Exception as e:  # noqa: BLE001
        return ("raise", type(e).__name__, str(e))
    if hasattr(r, "columns"):
        return ("frame", [(str(c), str(r[c].dtype), [str(v) for v in r[c].tolist()]) for c in r.columns],
                [str(t) for t in r.index])
    return (str(r.dtype), [str(v) for v in r.tolist()], [str(t) for t in r.index])


def _assert_same_outcome(got: Any, want: Any) -> None:
    # A refusal must be pandas' exception class, carrying pandas' message
    # verbatim (the binding prefixes its gate text).
    if want[0] == "raise":
        assert got[0] == "raise" and got[1] == want[1], (got, want)
        assert want[2] in got[2], (got, want)
    else:
        assert got == want


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("kind", sorted(RESAMPLE_DTYPE_VALUES))
def test_series_resample_of_every_dtype_matches_pandas(kind: str) -> None:
    # br-frankenpandas-rc0923-epic-rust-parity-bugs-4qg5w.23: str/bool/
    # datetime/timedelta columns resampled to NaN where pandas raises, float
    # where it keeps int/bool/datetime, datetimes as strings; nunique was count.
    # With an empty middle bin (numpy int/bool cannot hold its NaN) and dense.
    for stamps in (
        ["2024-01-01", "2024-01-01", "2024-01-03", "2024-01-03"],
        ["2024-01-01", "2024-01-01", "2024-01-02", "2024-01-02"],
    ):

        def series(m: Any) -> Any:
            return m.Series(RESAMPLE_DTYPE_VALUES[kind](m), index=m.to_datetime(stamps), name="x")

        for op in RESAMPLE_OPS:
            got = _resample_outcome(lambda: getattr(series(fpd).resample("D"), op)())
            want = _resample_outcome(lambda: getattr(series(pd).resample("D"), op)())
            _assert_same_outcome(got, want)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("numeric_only", [False, True])
def test_frame_resample_reduces_every_column_like_pandas(numeric_only: bool) -> None:
    # br-frankenpandas-0yilt: DataFrame.resample dropped every non-int/float
    # column; pandas reduces them all (raising where one cannot be reduced)
    # unless numeric_only=True keeps the int/float/bool columns.
    def frame(m: Any) -> Any:
        idx = m.to_datetime(["2024-01-01", "2024-01-01", "2024-01-03", "2024-01-03"])
        return m.DataFrame(
            {"k": ["a", "b", "c", "d"], "v": [1, 2, 3, 4], "f": [1.5, 2.5, 3.5, 4.5], "b": [True, False, True, True]},
            index=idx,
        )

    kwargs = {"numeric_only": True} if numeric_only else {}
    for op in ["sum", "mean", "min", "max", "prod", "first", "last", "median", "std", "var", "sem"]:
        got = _resample_outcome(lambda: getattr(frame(fpd).resample("D"), op)(**kwargs))
        want = _resample_outcome(lambda: getattr(frame(pd).resample("D"), op)(**kwargs))
        _assert_same_outcome(got, want)
    if not numeric_only:
        for op in ["count", "nunique"]:
            got = _resample_outcome(lambda: getattr(frame(fpd).resample("D"), op)())
            want = _resample_outcome(lambda: getattr(frame(pd).resample("D"), op)())
            _assert_same_outcome(got, want)
        s = fpd.Series(["a"], index=fpd.to_datetime(["2024-01-01"]))
        with pytest.raises(TypeError, match="numeric_only=True"):
            s.resample("D").mean(numeric_only=True)


def _nan_as_text(values: Any) -> Any:
    return ["nan" if isinstance(v, float) and v != v else v for v in values]


def _dated_frame(m: Any) -> Any:
    return m.DataFrame({"v": [1.0, 2.0, 3.0]}, index=m.to_datetime(["2024-01-05", "2024-02-06", None]))


TYPED_INDEX_CASES = {
    "frame index class": lambda m: type(_dated_frame(m).index).__name__,
    "series index class": lambda m: type(m.Series([1.0], index=m.to_datetime(["2024-01-01"])).index).__name__,
    "resample index class": lambda m: type(_dated_frame(m).dropna().resample("D").sum().index).__name__,
    "timedelta index class": lambda m: type(m.Series([1], index=m.to_timedelta(["1D"])).index).__name__,
    "index.year": lambda m: _nan_as_text(list(_dated_frame(m).index.year)),
    "index.dayofweek": lambda m: _nan_as_text(list(_dated_frame(m).index.dayofweek)),
    "index.quarter": lambda m: _nan_as_text(list(_dated_frame(m).index.quarter)),
    "index.month_name()": lambda m: _nan_as_text(list(_dated_frame(m).index.month_name())),
    "index.strftime": lambda m: _nan_as_text(list(_dated_frame(m).index.strftime("%Y/%m"))),
    "index.is_month_start": lambda m: list(_dated_frame(m).index.is_month_start),
    "index.normalize()": lambda m: [str(v) for v in _dated_frame(m).index.normalize()],
    "index item types": lambda m: [type(v).__name__ for v in _dated_frame(m).index],
    "index.tolist() types": lambda m: [type(v).__name__ for v in _dated_frame(m).index.tolist()],
    "index.min()": lambda m: str(_dated_frame(m).index.min()),
    "TimedeltaIndex.tolist()": lambda m: [str(v) for v in m.to_timedelta(["1D", None]).tolist()],
    # NEGATIVES: an index that is not all instants stays a plain Index.
    "string index class": lambda m: type(m.Series([1], index=["a"]).index).__name__,
    "int index class": lambda m: type(m.Series([1, 2], index=[5, 9]).index).__name__,
}


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", sorted(TYPED_INDEX_CASES))
def test_index_is_a_typed_datetime_index_like_pandas(case: str) -> None:
    # br-frankenpandas-rc0923-epic-python-honest-dropin-fvsao.18: .index was
    # always a plain Index, so df.index.year / .month_name() / .normalize()
    # raised; DatetimeIndex.tolist() and iteration gave strings.
    run = TYPED_INDEX_CASES[case]
    assert run(fpd) == run(pd)


def _keyed_rows(m: Any) -> Any:
    return m.DataFrame(
        {"k": ["a", "b", "a", "b"], "v": [1.0, 2.0, 3.0, 4.0]},
        index=m.to_datetime(["2024-01-05", "2024-02-06", "2024-01-07", "2024-03-01"]),
    )


GROUPBY_KEY_FORMS = {
    "Series": lambda m: _keyed_rows(m).groupby(_keyed_rows(m)["k"])["v"].sum(),
    "index.month": lambda m: _keyed_rows(m).groupby(_keyed_rows(m).index.month)["v"].sum(),
    "list of values": lambda m: _keyed_rows(m).groupby([1, 1, 2, 2])["v"].sum(),
    "ndarray": lambda m: _keyed_rows(m).groupby(np.array([1, 1, 2, 2]))["v"].sum(),
    "[column, index.month]": lambda m: _keyed_rows(m).groupby(["k", _keyed_rows(m).index.month])["v"].sum(),
    "callable": lambda m: _keyed_rows(m).groupby(lambda ts: ts.month)["v"].sum(),
    "frame level=0": lambda m: _keyed_rows(m).groupby(level=0)["v"].sum(),
    "series level=0": lambda m: _keyed_rows(m)["v"].groupby(level=0).sum(),
    "Series.groupby(index.year)": lambda m: _keyed_rows(m)["v"].groupby(_keyed_rows(m).index.year).sum(),
    "whole-frame sum by index.month": lambda m: _keyed_rows(m).groupby(_keyed_rows(m).index.month).sum(),
    # A key named like a column but not that column: the column is still
    # aggregated, and the key names the index.
    "renamed key collides with a column": lambda m: _keyed_rows(m).groupby(_keyed_rows(m)["k"].str.upper()).sum(),
}


def _grouped_outcome(r: Any) -> Any:
    names = list(r.index.names) if hasattr(r.index, "names") else [r.index.name]
    if hasattr(r, "columns"):
        values = {str(c): [str(v) for v in r[c].tolist()] for c in r.columns}
    else:
        values = [str(v) for v in r.tolist()]
    return ([str(t) for t in r.index], [str(n) for n in names], values)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("form", sorted(GROUPBY_KEY_FORMS))
def test_groupby_by_array_like_keys_matches_pandas(form: str) -> None:
    # br-frankenpandas-rc0923-epic-python-honest-dropin-fvsao.19: DataFrame
    # groupby took only column names (a Series key's values were read as
    # column names); arrays, index fields, callables and level= raised.
    run = GROUPBY_KEY_FORMS[form]
    assert _grouped_outcome(run(fpd)) == _grouped_outcome(run(pd))


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_groupby_key_errors_match_pandas() -> None:
    for m in (pd, fpd):
        # NEGATIVE: a list of column names still groups by those columns.
        assert list(_keyed_rows(m).groupby(["k"])["v"].sum()) == [4.0, 6.0]
        with pytest.raises(ValueError):
            _keyed_rows(m).groupby(np.array([1, 2, 3]))["v"].sum()
        with pytest.raises(KeyError):
            _keyed_rows(m).groupby("missing")
    # pandas drops an unnamed array key from as_index=False output; not
    # modelled, so refused rather than mislabelled.
    with pytest.raises(NotImplementedError):
        _keyed_rows(fpd).groupby(np.array([1, 1, 2, 2]), as_index=False)


def _dated_sales(m: Any) -> Any:
    return m.DataFrame(
        {"k": ["a", "b", "a", "b", "a"], "p": ["x", "x", "y", "y", "x"], "v": [1, 2, 3, 4, 5], "ok": [True, False, True, True, False]},
        index=m.to_datetime(
            ["2024-01-01 00:00", "2024-01-02 10:00", "2024-01-05 12:00", "2024-01-05 13:45", "2024-02-09 00:00"]
        ),
    )


EVERYDAY_OPS = {
    "~bool mask": lambda m: _dated_sales(m)[~_dated_sales(m)["ok"]],
    "~int": lambda m: ~m.Series([0, 5, -3]),
    "~frame": lambda m: ~_dated_sales(m)[["ok"]],
    "abs(Series)": lambda m: abs(m.Series([-1.5, 2.0])),
    "abs(frame)": lambda m: abs(m.DataFrame({"a": [-1, 2]})),
    "+Series": lambda m: +m.Series([1, -2]),
    "loc date-string window": lambda m: _dated_sales(m).loc["2024-01-02":"2024-01-05"],
    "loc month": lambda m: _dated_sales(m).loc["2024-01":"2024-01"],
    "series loc open end": lambda m: _dated_sales(m)["v"].loc["2024-01-05":],
    "loc missing int bounds": lambda m: m.Series([10, 30, 50], index=[1, 3, 5]).loc[2:4],
    "unstack groupby result": lambda m: _dated_sales(m).groupby(["k", "p"])["v"].sum().unstack(),
    "unstack fill_value": lambda m: _dated_sales(m).iloc[:4].groupby(["k", "p"])["v"].sum().unstack(fill_value=0),
    "pivot_table fill_value": lambda m: _dated_sales(m).iloc[:4].pivot_table(index="k", columns="p", values="v", aggfunc="sum", fill_value=0),
}


def _plain(obj: Any) -> Any:
    if hasattr(obj, "columns"):
        return ("frame", [str(c) for c in obj.columns], [str(obj[c].dtype) for c in obj.columns],
                [str(t) for t in obj.index], [[str(v) for v in obj[c].tolist()] for c in obj.columns])
    return (str(obj.dtype), [str(t) for t in obj.index], [str(v) for v in obj.tolist()])


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("op", sorted(EVERYDAY_OPS))
def test_everyday_selection_and_reshape_match_pandas(op: str) -> None:
    # br-frankenpandas-rc0923-epic-python-honest-dropin-fvsao.13 (found by the
    # time-series journey): ~mask raised, abs()/+ were missing, .loc needed
    # exact labels (no date-period strings, no bounds between labels), and
    # unstack/pivot_table took no fill_value; unstack could not read a
    # groupby result's MultiIndex.
    run = EVERYDAY_OPS[op]
    assert _plain(run(fpd)) == _plain(run(pd))


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_everyday_refusals_match_pandas() -> None:
    for m in (pd, fpd):
        # NEGATIVES: ~float is refused, and a non-monotonic index needs the
        # slice's labels themselves.
        with pytest.raises(TypeError):
            ~m.Series([1.5])
        with pytest.raises(KeyError):
            m.Series([30, 10, 50], index=[3, 1, 5]).loc[2:]
    # unstack(level=0) was refused here until fvsao.36 implemented it; it is
    # now compared with pandas.
    got = _dated_sales(fpd).groupby(["k", "p"])["v"].sum().unstack(level=0).to_dict()
    want = _dated_sales(pd).groupby(["k", "p"])["v"].sum().unstack(level=0).to_dict()
    assert got == want


JOURNEY_CSV = """date,store,product,units,price,returned
2024-01-01 09:15:00,north,apple,3,1.20,False
2024-01-01 11:40:00,south,pear,5,0.80,False
2024-01-02 10:05:00,north,pear,2,0.85,True
2024-01-02 16:30:00,north,apple,,1.25,False
2024-01-03 08:00:00,south,apple,7,1.10,False
2024-01-05 12:00:00,south,plum,1,2.50,False
2024-01-05 13:45:00,north,plum,4,2.40,True
2024-01-06 18:20:00,north,apple,6,1.30,False
2024-01-08 09:00:00,south,pear,3,0.90,False
2024-01-09 17:10:00,south,apple,2,1.15,False
"""


def _journey(m: Any) -> dict:
    out = {}
    raw = m.read_csv(io.StringIO(JOURNEY_CSV), parse_dates=["date"])
    sales = raw.set_index("date").assign(revenue=lambda d: d["units"] * d["price"]).fillna({"units": 0})
    kept = sales[~sales["returned"]]
    daily = kept["revenue"].resample("D").sum()
    out["daily"] = daily
    out["rolling"] = daily.rolling(3, min_periods=1).mean()
    out["unstacked"] = kept.groupby(["store", "product"])["units"].sum().unstack(fill_value=0)
    out["pivot"] = kept.pivot_table(index="store", columns="product", values="revenue", aggfunc="sum", fill_value=0)
    out["by weekday"] = kept.groupby(kept.index.dayofweek)["revenue"].mean()
    out["window"] = kept.loc["2024-01-02":"2024-01-05"]
    out["pct"] = daily.pct_change()
    out["top"] = kept.groupby("product")["revenue"].sum().nlargest(2)
    out["summary"] = kept.groupby("store").agg(total=("revenue", "sum"), n=("units", "count"))
    out["merged"] = out["summary"].reset_index().merge(m.DataFrame({"store": ["north", "south"], "region": ["N", "S"]}), on="store")
    out["round trip"] = m.read_csv(io.StringIO(out["merged"].to_csv(index=False)))
    out["weekly"] = kept[["units", "revenue"]].resample("W").sum()
    out["ewm"] = daily.ewm(span=3).mean()
    return out


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_time_series_journey_matches_pandas() -> None:
    # fvsao.13: a whole time-series ETL (read_csv with dates, set_index,
    # assign, fillna, ~mask filter, resample, rolling, groupby + unstack,
    # pivot_table, groupby by index field, date-string .loc window,
    # pct_change, nlargest, named agg, merge, to_csv round trip, ewm) run
    # under both libraries, every intermediate compared.
    want, got = _journey(pd), _journey(fpd)
    for step in want:
        assert _plain(got[step]) == _plain(want[step]), step


STR_VALUES = ["  Alice Smith ", "bob", None, "CAROL-ann 42", "x1y2", "Äbc déf", ""]

STR_METHOD_CASES = {
    "title": lambda s: s.str.title(),
    "capitalize": lambda s: s.str.capitalize(),
    "swapcase": lambda s: s.str.swapcase(),
    "casefold": lambda s: s.str.casefold(),
    "isdigit": lambda s: s.str.isdigit(),
    "isalpha": lambda s: s.str.isalpha(),
    "isalnum": lambda s: s.str.isalnum(),
    "isspace": lambda s: s.str.isspace(),
    "islower": lambda s: s.str.islower(),
    "isupper": lambda s: s.str.isupper(),
    "isnumeric": lambda s: s.str.isnumeric(),
    "istitle": lambda s: s.str.istitle(),
    "zfill": lambda s: s.str.zfill(6),
    "pad both": lambda s: s.str.pad(8, side="both", fillchar="*"),
    "center": lambda s: s.str.center(9, "-"),
    "ljust": lambda s: s.str.ljust(7, "."),
    "rjust": lambda s: s.str.rjust(7),
    "repeat": lambda s: s.str.repeat(2),
    "removeprefix": lambda s: s.str.removeprefix("bo"),
    "removesuffix": lambda s: s.str.removesuffix("42"),
    "count regex": lambda s: s.str.count(r"[a-z]"),
    "find": lambda s: s.str.find("l"),
    "rfind": lambda s: s.str.rfind("l"),
    "find with start": lambda s: s.str.find("l", 3),
    "get": lambda s: s.str.get(1),
    "str[0]": lambda s: s.str[0],
    "str[-1]": lambda s: s.str[-1],
    "str[1:4]": lambda s: s.str[1:4],
    "str[::2]": lambda s: s.str[::2],
    "slice": lambda s: s.str.slice(1, 3),
    "slice_replace": lambda s: s.str.slice_replace(1, 3, "ZZ"),
    "fullmatch": lambda s: s.str.fullmatch(r"[a-z]+"),
    "match": lambda s: s.str.match(r"[a-z]"),
    "match case=False": lambda s: s.str.match(r"[a-z]", case=False),
    "contains regex": lambda s: s.str.contains(r"\d"),
    "contains literal": lambda s: s.str.contains(".", regex=False),
    "contains na=False": lambda s: s.str.contains("o", na=False),
    "contains case=False": lambda s: s.str.contains("ALICE", case=False),
    "startswith tuple": lambda s: s.str.startswith(("b", "x")),
    "endswith na=False": lambda s: s.str.endswith("2", na=False),
    "replace regex": lambda s: s.str.replace(r"\s+", "_", regex=True),
    "replace literal": lambda s: s.str.replace("a", "@"),
    "replace n": lambda s: s.str.replace("l", "L", n=1),
    "replace case=False": lambda s: s.str.replace("ALICE", "Al", case=False),
    "extract one group": lambda s: s.str.extract(r"([a-z]+)"),
    "extract expand=False": lambda s: s.str.extract(r"([a-z]+)", expand=False),
    "extract two groups": lambda s: s.str.extract(r"([a-zA-Z]+)\s*(\w+)?"),
    "extract named groups": lambda s: s.str.extract(r"(?P<first>[a-z])(?P<rest>\w*)"),
    "split expand": lambda s: s.str.split("-", expand=True),
    "split expand n": lambda s: s.str.split(" ", n=1, expand=True),
    "rsplit expand n": lambda s: s.str.rsplit(" ", n=1, expand=True),
    "partition": lambda s: s.str.partition(" "),
    "rpartition": lambda s: s.str.rpartition("-"),
    "get_dummies": lambda s: s.str.get_dummies(sep=" "),
    "cat to one string": lambda s: s.str.cat(sep="|"),
    "cat others na_rep": lambda s: s.str.cat(s.str.upper(), sep="+", na_rep="?"),
    "cat others": lambda s: s.str.cat(s.str.upper(), sep="+"),
    "strip chars": lambda s: s.str.strip(" A"),
    "lstrip chars": lambda s: s.str.lstrip(" A"),
    "wrap": lambda s: s.str.wrap(4),
}


def _str_outcome(obj: Any) -> Any:
    def one(v: Any) -> str:
        return "nan" if isinstance(v, float) and v != v else repr(v)

    if hasattr(obj, "columns"):
        return ("frame", [str(c) for c in obj.columns], [str(obj[c].dtype) for c in obj.columns],
                [[one(v) for v in obj[c].tolist()] for c in obj.columns])
    if hasattr(obj, "tolist"):
        return (str(obj.dtype), [one(v) for v in obj.tolist()], obj.name)
    return one(obj)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", sorted(STR_METHOD_CASES))
def test_str_methods_match_pandas(case: str) -> None:
    # fvsao.13 (text journey): the .str accessor exposed ten methods; the
    # rest raised AttributeError, contains/replace/startswith refused
    # pandas' keywords, a missing string came back NaN where pandas keeps
    # None, and a bool result with a missing value reported bool, not object.
    run = STR_METHOD_CASES[case]
    assert _str_outcome(run(fpd.Series(STR_VALUES, name="t"))) == _str_outcome(run(pd.Series(STR_VALUES, name="t")))


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_str_refusals() -> None:
    with pytest.raises(NotImplementedError):
        fpd.Series(["a,b"]).str.contains("a", flags=2)
    # NEGATIVE: an all-present bool result stays bool, as pandas.
    for m in (pd, fpd):
        assert str(m.Series(["ab", "c"]).str.contains("a").dtype) == "bool"


# np.nan (not float("nan")): pandas' split(expand=True) spreads a missing row
# across the columns only for the np.nan object.
STR_LIST_VALUES = ["a,b,,c", "  two  words here ", None, "x", "", np.nan, "k1=v1; k2=v2"]

STR_LIST_CASES = {
    "split": lambda s: s.str.split(","),
    "split whitespace": lambda s: s.str.split(),
    "split n=1": lambda s: s.str.split(",", n=1),
    "split multi-char pattern is a regex": lambda s: s.str.split("; "),
    "split regex=True": lambda s: s.str.split(r"[,=;]", regex=True),
    "rsplit n=1": lambda s: s.str.rsplit(",", n=1),
    "split then str[0]": lambda s: s.str.split(",").str[0],
    "split then str[-1]": lambda s: s.str.split(",").str[-1],
    "split then str.get out of range": lambda s: s.str.split(",").str.get(5),
    "split then str.len": lambda s: s.str.split().str.len(),
    "split then str.join": lambda s: s.str.split(",").str.join("|"),
    "split then explode": lambda s: s.str.split(",").explode(),
    "split then explode ignore_index": lambda s: s.str.split(",").explode(ignore_index=True),
    "split then apply": lambda s: s.str.split(",").apply(lambda v: len(v) if isinstance(v, list) else -1),
    "split whitespace expand": lambda s: s.str.split(expand=True),
    "split regex expand": lambda s: s.str.split(r"[,=]", regex=True, expand=True),
    "rsplit whitespace expand": lambda s: s.str.rsplit(n=1, expand=True),
    "findall": lambda s: s.str.findall(r"[a-z]\d?"),
    "findall one group": lambda s: s.str.findall(r"k(\d)"),
    "findall then len": lambda s: s.str.findall(r"\w").str.len(),
}


def _list_outcome(obj: Any) -> Any:
    def one(v: Any) -> str:
        return "nan" if isinstance(v, float) and v != v else repr(v)

    if hasattr(obj, "columns"):
        return ("frame", [str(c) for c in obj.columns], [[one(v) for v in obj[c].tolist()] for c in obj.columns])
    return (str(obj.dtype), [one(v) for v in obj.tolist()], [str(t) for t in obj.index], obj.name)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", sorted(STR_LIST_CASES))
def test_str_list_results_match_pandas(case: str) -> None:
    # fvsao.13: str.split(expand=False) and str.findall return a Series of
    # Python lists in pandas; they raised (split) or were missing (findall),
    # so .str.split(",").str[0], .str.split().str.len() and explode() failed.
    run = STR_LIST_CASES[case]
    index = list(range(10, 17))
    got = run(fpd.Series(STR_LIST_VALUES, name="t", index=index))
    want = run(pd.Series(STR_LIST_VALUES, name="t", index=index))
    assert _list_outcome(got) == _list_outcome(want)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_string_arithmetic_matches_pandas() -> None:
    # fvsao.13: s + t concatenated nothing - "value 'a' has non-numeric dtype".
    for build in (
        lambda m: m.Series(["a", "b", None, np.nan, "e"], name="x") + m.Series(["1", None, "3", "4", np.nan], name="x"),
        lambda m: m.Series(["a", None], name="x") + "z",
        lambda m: "z" + m.Series(["a", None], name="x"),
        lambda m: m.Series(["ab", None], name="x") * 2,
        lambda m: m.Series(["a", "b"], index=[0, 1]) + m.Series(["c"], index=[1]),
    ):
        got, want = build(fpd), build(pd)
        assert _list_outcome(got) == _list_outcome(want)
    # NEGATIVE: a string plus an int is pandas' TypeError.
    for m in (pd, fpd):
        with pytest.raises(TypeError):
            m.Series(["a"]) + 1


JOURNEY_TEXT_CSV = """id,name,email,city,tags,comment
1,  Alice Smith ,ALICE@Example.com,new york,"red,blue",Great product!! 5/5
2,bob jones,bob@test.org,Boston,blue,"Not bad, 3/5"
3,Carol  White,carol@example.com,new york,,Terrible. 1/5
4,dave brown,DAVE@TEST.ORG,boston,"green,red,blue",ok 3/5
5,Eve Black,eve@example.com,Chicago,red,
6,frank green,frank@other.net,chicago,"blue,green",Loved it 5/5
"""


def _text_journey(m: Any) -> dict:
    out = {}
    raw = m.read_csv(io.StringIO(JOURNEY_TEXT_CSV))
    name = raw["name"].str.strip().str.replace(r"\s+", " ", regex=True).str.title()
    out["name"] = name
    email = raw["email"].str.lower()
    out["domain"] = email.str.split("@").str[1]
    out["domain parts"] = email.str.extract(r"@(\w+)\.(\w+)$")
    city = raw["city"].str.title()
    out["city counts"] = city.value_counts()
    out["five stars"] = raw["comment"].str.contains("5/5", na=False)
    out["rating"] = raw["comment"].str.extract(r"(\d)/5", expand=False).astype(float)
    out["tags wide"] = raw["tags"].str.split(",", expand=True)
    out["tag dummies"] = raw["tags"].str.get_dummies(sep=",")
    out["city dummies"] = m.get_dummies(city, prefix="city")
    out["crosstab"] = m.crosstab(city, out["five stars"])
    out["initials"] = name.str[0] + name.str.split(" ").str[1].str[0]
    out["padded id"] = raw["id"].astype(str).str.zfill(4)
    out["tag counts"] = raw["tags"].str.split(",").explode().value_counts()
    out["words"] = raw["comment"].fillna("(none)").str.split().str.len()
    out["digits"] = raw["comment"].fillna("(none)").str.findall(r"\d").str.len()
    out["deduped"] = m.DataFrame({"c": city, "d": out["domain"]}).drop_duplicates()
    return out


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_text_journey_matches_pandas() -> None:
    # fvsao.13 journey (3): string cleaning, extraction, splitting, dummies,
    # crosstab, list-valued splits and string concatenation under both
    # libraries, every intermediate compared.
    want, got = _text_journey(pd), _text_journey(fpd)
    for step in want:
        assert _plain(got[step]) == _plain(want[step]), step


def _plain_any(obj: Any) -> Any:
    if hasattr(obj, "columns") or (hasattr(obj, "tolist") and hasattr(obj, "index")):
        return _plain(obj)
    if hasattr(obj, "tolist"):
        return (type(obj).__name__, [str(v) for v in obj.tolist()])
    return obj


def _axis_assignment(m: Any) -> dict:
    out = {}
    df = m.DataFrame({"A": [1, 2, 3], "B": [4.0, 5.0, 6.0]})
    out["columns"] = df.columns
    out["iterate"] = list(df)
    out["in"] = ("A" in df, "Z" in df, "A" in df.columns)
    out["keys"] = df.keys()
    out["get_loc"] = df.columns.get_loc("B")
    renamed = df.copy()
    renamed.columns = [c.lower() for c in renamed.columns]
    out["set columns"] = renamed
    reindexed = df.copy()
    reindexed.index = ["x", "y", "z"]
    out["set index"] = reindexed
    out["index name"] = reindexed.index.name
    named = df.copy()
    named.index = m.Index([7, 8, 9], name="k")
    out["set named index"] = named
    out["named index name"] = named.index.name
    s = m.Series([1, 2])
    s.name = "v"
    s.index = ["a", "b"]
    out["series name/index"] = s
    out["series name"] = s.name
    return out


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_axis_assignment_matches_pandas() -> None:
    # fvsao.7 / fvsao.13: df.columns was a Python list (no .tolist()), and
    # df.columns = / df.index = / s.name = / s.index = raised AttributeError;
    # `for c in df` raised TypeError.
    want, got = _axis_assignment(pd), _axis_assignment(fpd)
    for step in want:
        assert _plain_any(got[step]) == _plain_any(want[step]), step
    # NEGATIVES: a wrong length or a bare string is refused.
    for m in (pd, fpd):
        df = m.DataFrame({"A": [1, 2]})
        with pytest.raises(ValueError, match="Length mismatch: Expected axis has 1 elements"):
            df.columns = ["a", "b"]
        with pytest.raises(ValueError, match="Length mismatch: Expected axis has 2 elements"):
            df.index = [1, 2, 3]
        with pytest.raises(TypeError):
            df.columns = "A"
        assert list(df.columns) == ["A"]


def _agg_lists(m: Any) -> dict:
    df = m.DataFrame({"g": ["a", "b", "a", "b"], "x": [1, 2, 3, 5], "y": [1.5, 2.5, 3.5, 4.5]})
    gb = df.groupby("g")
    lists = gb.agg({"x": ["sum", "max"], "y": "mean"})
    flat = lists.copy()
    flat.columns = ["_".join(c) for c in flat.columns]
    return {
        "dict order": gb.agg({"y": "sum", "x": "max"}),
        "dict lists": lists,
        "column tuples": [str(c) for c in lists.columns],
        "list funcs": gb.agg(["sum", "max"]),
        "top level": lists["x"],
        "tuple key": lists[("x", "max")].tolist(),
        "tuple in": (("x", "max") in lists, "x" in lists, ("x", "min") in lists),
        "reset_index": lists.reset_index(),
        "reset key": lists.reset_index()["g"],
        "ohlc columns": [str(c) for c in gb.ohlc().columns],
        "flattened": flat,
        "cumsum iloc": gb[["y", "x"]].cumsum().iloc[:, 0],
    }


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_groupby_agg_with_lists_matches_pandas() -> None:
    # fvsao.13 ETL journey: agg({'amount': ['sum', 'max'], ...}) raised; a
    # dict came back in the frame's order, not the request's; the (column,
    # func) column axis never reached Python; and a groupby result's storage
    # order leaked into positional reads (iloc / flattening paired a label
    # with another column's values).
    want, got = _agg_lists(pd), _agg_lists(fpd)
    for step in want:
        assert _plain_any(got[step]) == _plain_any(want[step]), step
    # NEGATIVE: a column the frame lacks is pandas' KeyError.
    for m in (pd, fpd):
        with pytest.raises(KeyError, match="do not exist"):
            m.DataFrame({"g": [1], "x": [2]}).groupby("g").agg({"nope": "sum"})


def _loc_writes(m: Any) -> dict:
    def base() -> Any:
        return m.DataFrame(
            {"a": [1, 2, 3, 4], "b": [1.5, 2.5, 3.5, 4.5], "s": ["w", "x", "y", "z"]},
            index=["p", "q", "r", "t"],
        )

    out = {}
    df = base(); df.loc[df["a"] > 2, "b"] = 0.0; out["mask, column, scalar"] = df
    df = base(); df.loc[df["a"] > 2, "a"] = 0.5; out["int column takes a float"] = df
    df = base(); df.loc[df["a"] > 2, "a"] = 9; out["int column takes an int"] = df
    df = base(); df.loc["q":"r", "b"] = -1.0; out["label slice"] = df
    df = base(); df.loc[["p", "t"], ["a", "b"]] = 0; out["label lists"] = df
    df = base(); df.loc["q", "s"] = "NEW"; out["one cell"] = df
    df = base()[["a", "b"]]; df.loc[df["a"] < 3] = 0; out["whole rows"] = df
    df = base()
    df.loc[df["a"] > 1, "b"] = m.Series([10.0, 20.0, 30.0, 40.0], index=["t", "r", "q", "p"])
    out["series aligned on labels"] = df
    df = base(); df.loc[df["a"] > 2, "b"] = [7.0, 8.0]; out["list"] = df
    df = base(); df.loc[df["a"] > 2, "c"] = 1.0; out["new column"] = df
    df = base(); df.loc["p", "a"] = np.nan; out["nan into int"] = df
    df = base(); df.loc["u"] = [5, 5.5, "v"]; out["append a row"] = df
    df = base(); df.loc["u", "a"] = 5; out["append part of a row"] = df
    df = m.DataFrame({"a": [1, 2], "f": [True, False]}); df.loc[len(df)] = 0; out["append numbers"] = df
    return out


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_loc_writes_match_pandas() -> None:
    # fvsao.13 ETL journey: `df.loc[mask, "col"] = value` raised ("does not
    # support item assignment"); pandas also appends a row for a label the
    # index lacks.
    want, got = _loc_writes(pd), _loc_writes(fpd)
    for step in want:
        assert _plain(got[step]) == _plain(want[step]), step
    # NEGATIVES: a value of the wrong length, a row of the wrong width, and a
    # list with a label the index lacks are refused; the frame is unchanged.
    for m in (pd, fpd):
        df = m.DataFrame({"a": [1, 2, 3], "b": [4.0, 5.0, 6.0]})
        with pytest.raises(ValueError):
            df.loc[df["a"] > 1, "b"] = [7.0]
        with pytest.raises(ValueError, match="cannot set a row with mismatched columns"):
            df.loc[3] = [1]
        with pytest.raises(KeyError):
            df.loc[[0, 9], "a"] = 1
        assert df["b"].tolist() == [4.0, 5.0, 6.0]


JOURNEY_ORDERS_CSV = """order_id,customer,order_date,amount,qty,status,region
1001, acme ,2024-01-03,120.50,3,shipped,EU
1002,globex,2024-01-04,NA,1,pending,US
1003,acme,2024-01-10,75.00,,shipped,EU
1004,initech,2024-02-01,300.00,10,cancelled,US
1005,globex,2024-02-15,42.25,2,shipped,US
1006,umbrella,2024-02-20,999.99,1,shipped,APAC
1007,acme,2024-03-01,15.00,5,returned,EU
1008,initech,2024-03-05,60.00,2,shipped,
"""
JOURNEY_CUSTOMERS_CSV = """customer,segment,since
acme,enterprise,2019
globex,smb,2021
initech,enterprise,2018
hooli,smb,2022
"""


def _etl_journey(m: Any, tmp_path: Path) -> dict:
    tmp_path.mkdir()
    orders_path = tmp_path / "orders.csv"
    customers_path = tmp_path / "customers.csv"
    orders_path.write_text(JOURNEY_ORDERS_CSV)
    customers_path.write_text(JOURNEY_CUSTOMERS_CSV)
    out = {}
    raw = m.read_csv(str(orders_path), parse_dates=["order_date"], dtype={"order_id": "int64"},
                     na_values=["NA"])
    customers = m.read_csv(str(customers_path))
    out["dtypes"] = raw.dtypes.astype(str)
    stripped = raw.assign(customer=lambda d: d["customer"].str.strip())
    filled = stripped.fillna({"region": "UNKNOWN", "qty": 0})
    typed = filled.dropna(subset=["amount"]).astype({"qty": "int64"})
    clean = typed.rename(columns={"amount": "revenue_raw"}).assign(amount=lambda d: d["revenue_raw"])
    out["clean"] = clean
    out["query"] = clean.query("status == 'shipped' and amount > 50")
    written = clean.copy()
    written.loc[written["status"] == "cancelled", "amount"] = 0.0
    out["loc write"] = written
    out["where"] = clean["amount"].where(clean["amount"] < 500, 500.0)
    out["mask"] = clean["qty"].mask(clean["qty"] > 5)
    out["row apply"] = clean.apply(lambda r: r["amount"] / max(r["qty"], 1), axis=1)
    out["unit price"] = (clean["amount"] / clean["qty"].replace(0, np.nan)).round(2)
    named = clean.groupby("customer").agg(total=("amount", "sum"), orders=("order_id", "count"),
                                          avg_qty=("qty", "mean"))
    out["named agg"] = named
    out["agg dict"] = clean.groupby("region").agg({"amount": ["sum", "max"], "qty": "sum"})
    merged = clean.merge(customers, on="customer", how="left", indicator=True)
    out["merge"] = merged
    out["merge suffixes"] = clean[["customer", "amount"]].merge(
        clean[["customer", "qty", "amount"]], on="customer", suffixes=("_l", "_r")).head(6)
    out["merge counts"] = merged["_merge"].value_counts()
    out["pivot_table"] = clean.pivot_table(index="region", columns="status", values="amount",
                                           aggfunc="sum", fill_value=0)
    out["melt"] = named.reset_index().melt(id_vars="customer", value_vars=["total", "orders"])
    out["sort"] = clean.sort_values(["region", "amount"], ascending=[True, False])[
        ["order_id", "region", "amount"]]
    out["rank"] = clean["amount"].rank(ascending=False)
    out["cumsum by group"] = clean.groupby("customer")["amount"].cumsum()
    out["duplicated"] = clean.duplicated(subset=["customer"])
    out["month period"] = clean["order_date"].dt.to_period("M").astype(str)
    out["monthly totals"] = clean.groupby(clean["order_date"].dt.month)["amount"].sum()
    out["categories"] = clean["region"].astype("category").cat.categories.tolist()
    out["concat"] = m.concat([clean.head(2), clean.tail(2)], ignore_index=True)[["order_id", "amount"]]
    out["csv round trip"] = m.read_csv(io.StringIO(named.to_csv()), index_col=0)
    out["describe"] = clean[["amount", "qty"]].describe()
    out["nunique"] = clean.nunique()
    return out


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_etl_journey_matches_pandas(tmp_path: Path) -> None:
    # fvsao.13 journey (1): read two CSVs, clean, filter, write through .loc,
    # aggregate (named and dict-of-lists), merge, reshape, rank and export
    # under both libraries, every intermediate compared.
    want = _etl_journey(pd, tmp_path / "pd")
    got = _etl_journey(fpd, tmp_path / "fpd")
    for step in want:
        assert _plain_any(got[step]) == _plain_any(want[step]), step


_INPLACE_WITH_GAPS = {
    "frame fillna": lambda m, d, s: d.fillna(0, inplace=True),
    "frame ffill": lambda m, d, s: d.ffill(inplace=True),
    "frame bfill": lambda m, d, s: d.bfill(inplace=True),
    "frame interpolate": lambda m, d, s: d.interpolate(inplace=True),
    "series fillna": lambda m, d, s: s.fillna(-1, inplace=True),
    "series ffill": lambda m, d, s: s.ffill(inplace=True),
    "series bfill": lambda m, d, s: s.bfill(inplace=True),
    "series interpolate": lambda m, d, s: s.interpolate(inplace=True),
    "series sort_values": lambda m, d, s: s.sort_values(inplace=True),
    "series sort_index": lambda m, d, s: s.sort_index(ascending=False, inplace=True),
    "series replace": lambda m, d, s: s.replace(3.0, 30.0, inplace=True),
    "series clip": lambda m, d, s: s.clip(1.5, 2.5, inplace=True),
    "series drop_duplicates": lambda m, d, s: s.drop_duplicates(inplace=True),
    "series reset_index drop": lambda m, d, s: s.reset_index(drop=True, inplace=True),
}


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("op", sorted(_INPLACE_WITH_GAPS))
def test_inplace_on_gaps_mutates_and_returns_none_like_pandas(op: str) -> None:
    # fvsao.6: inplace=True was refused by 20 Series/DataFrame methods.
    after = []
    for m in (pd, fpd):
        d = m.DataFrame({"a": [1.0, None, 3.0, None], "b": [None, 2.0, 2.0, 4.0]},
                        index=[3, 1, 2, 0])
        s = m.Series([3.0, None, 1.0, 3.0], index=[3, 1, 2, 0], name="v")
        assert _INPLACE_WITH_GAPS[op](m, d, s) is None, op
        after.append((_plain(d), _plain(s)))
    assert after[1] == after[0], op
    # NEGATIVE: a Series cannot become a frame in place.
    for m in (pd, fpd):
        with pytest.raises(TypeError, match="Cannot reset_index inplace on a Series"):
            m.Series([1, 2]).reset_index(inplace=True)


def _series_writes(m: Any) -> dict:
    out = {}

    def run(name: str, make: Any, write: Any) -> None:
        obj = make()
        write(obj)
        out[name] = obj

    labeled = lambda: m.Series([1, 2, 3], index=["a", "b", "c"])  # noqa: E731
    plain = lambda: m.Series([1, 2, 3, 4])  # noqa: E731
    run("label", labeled, lambda s: s.__setitem__("b", 20))
    run("position on a string index", labeled, lambda s: s.__setitem__(0, 9))
    run("new int label enlarges", lambda: m.Series([1, 2, 3], index=[10, 20, 30]),
        lambda s: s.__setitem__(0, 9))
    run("int slice by position", plain, lambda s: s.__setitem__(slice(1, 3), 0))
    run("label slice", labeled, lambda s: s.__setitem__(slice("a", "b"), 0))
    run("bool list", lambda: m.Series([1, 2, 3]), lambda s: s.__setitem__([True, False, True], 5))
    run("mask", plain, lambda s: s.__setitem__(s > 2, 0))
    run("mask with aligned values", plain, lambda s: s.__setitem__(s > 1, s * 10))
    run("label list", labeled, lambda s: s.__setitem__(["a", "c"], 0))
    run("string into ints", plain, lambda s: s.__setitem__(1, "x"))
    run("nan into ints", plain, lambda s: s.__setitem__(1, np.nan))
    run("float into ints", plain, lambda s: s.__setitem__(1, 2.5))
    run("loc new label", lambda: m.Series([1, 2]), lambda s: s.loc.__setitem__(5, 9))
    run("setitem new label float", lambda: m.Series([1, 2]), lambda s: s.__setitem__(5, 9.5))
    run("loc mask", lambda: m.Series([1.5, 2.5, 3.5]), lambda s: s.loc.__setitem__(s > 2, np.nan))
    run("iloc", plain, lambda s: s.iloc.__setitem__(0, 9))
    run("iloc negative", plain, lambda s: s.iloc.__setitem__(-1, 0))
    run("iloc slice", plain, lambda s: s.iloc.__setitem__(slice(1, 3), 0))
    run("iloc list", plain, lambda s: s.iloc.__setitem__([0, 2], [7, 8]))
    run("at", labeled, lambda s: s.at.__setitem__("c", 30))
    run("iat", labeled, lambda s: s.iat.__setitem__(1, 21))
    run("category keeps its dtype", lambda: m.Series(["x", "y", "x"], dtype="category"),
        lambda s: s.__setitem__(1, "x"))
    run("None into bools is NaN", lambda: m.Series([True, False]), lambda s: s.__setitem__(0, None))
    return out


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_series_writes_match_pandas() -> None:
    # fvsao.6: a Series took no writes at all - s[k] = v, s.loc/.iloc/.at/.iat
    # setters all raised "does not support item assignment".
    want, got = _series_writes(pd), _series_writes(fpd)
    for step in want:
        assert _plain(got[step]) == _plain(want[step]), step
    # NEGATIVES: .iloc cannot enlarge, a value list must match, and the
    # accessor writes the Series it came from, not a copy.
    for m in (pd, fpd):
        s = m.Series([1, 2, 3])
        with pytest.raises(IndexError, match="iloc cannot enlarge its target object"):
            s.iloc[5] = 0
        with pytest.raises(ValueError):
            s.iloc[[0, 1]] = [1]
        loc = s.loc
        s[0] = 7
        loc[1] = 8
        assert s.tolist() == [7, 8, 3]
        categories = m.Series(["x", "y"], dtype="category")
        with pytest.raises(TypeError, match="Cannot setitem on a Categorical with a new category"):
            categories[0] = "z"
        assert categories.tolist() == ["x", "y"]


def _frame_writes(m: Any) -> dict:
    out = {}

    def run(name: str, write: Any) -> None:
        df = m.DataFrame({"a": [1, 2], "b": [3.0, 4.0]})
        write(df)
        out[name] = df

    run("iloc cell", lambda d: d.iloc.__setitem__((0, 1), 9.0))
    run("iloc row list", lambda d: d.iloc.__setitem__(0, [9, 9.5]))
    run("iloc row scalar", lambda d: d.iloc.__setitem__(1, 0))
    run("iloc column", lambda d: d.iloc.__setitem__((slice(None), 0), [5, 6]))
    run("at", lambda d: d.at.__setitem__((1, "b"), 7.5))
    run("at new row", lambda d: d.at.__setitem__((5, "a"), 7))
    run("iat", lambda d: d.iat.__setitem__((0, 0), 5))
    run("bool frame", lambda d: d.__setitem__(d > 2, 0))
    run("bool series rows", lambda d: d.__setitem__(d["a"] > 1, 0))
    run("columns from a frame", lambda d: d.__setitem__(["a", "b"], d[["b", "a"]]))
    run("columns from an aligned frame", lambda d: d.__setitem__(
        ["a", "b"], m.DataFrame({"a": [10, 20], "b": [30.0, 40.0]}, index=[1, 0])))
    run("new columns from rows", lambda d: d.__setitem__(["x", "y"], [[1, 2], [3, 4]]))
    run("columns from a scalar", lambda d: d.__setitem__(["a", "z"], 0))
    run("column from a reordered series", lambda d: d.__setitem__(
        "c", m.Series([10, 20], index=[1, 0])))
    run("column from a partial series", lambda d: d.__setitem__("c", m.Series([10], index=[1])))
    run("loc row by label list", lambda d: d.loc.__setitem__(0, [7, 7.5]))
    run("del", lambda d: d.__delitem__("a"))
    return out


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_frame_writes_match_pandas() -> None:
    # fvsao.6: df.iloc/.at/.iat had no setters; df[bool_frame] = v,
    # df[[cols]] = values and del df[col] raised; df[col] = series took the
    # Series by POSITION (a reordered Series landed on the wrong rows).
    want, got = _frame_writes(pd), _frame_writes(fpd)
    for step in want:
        assert _plain(got[step]) == _plain(want[step]), step
    # NEGATIVES: iloc cannot enlarge, a missing column cannot be deleted, a
    # frame value must have as many columns as the key.
    for m in (pd, fpd):
        df = m.DataFrame({"a": [1, 2], "b": [3.0, 4.0]})
        with pytest.raises(IndexError, match="iloc cannot enlarge its target object"):
            df.iloc[5, 0] = 1
        with pytest.raises(KeyError):
            del df["zz"]
        with pytest.raises(ValueError, match="Columns must be same length as key"):
            df[["a", "b"]] = df[["a"]]
        assert df["a"].tolist() == [1, 2]


_GAP_CSV_LEFT = "k,s\na,x\nb,y\n"
_GAP_CSV_RIGHT = "k,t\nb,p\nc,q\n"


def _gap_frames(m: Any, source: str) -> Any:
    if source == "csv":
        return m.read_csv(io.StringIO(_GAP_CSV_LEFT)), m.read_csv(io.StringIO(_GAP_CSV_RIGHT))
    return (m.DataFrame({"k": ["a", "b"], "s": ["x", "y"]}),
            m.DataFrame({"k": ["b", "c"], "t": ["p", "q"]}))


_GAP_MARKER_CASES = {
    **{
        f"merge {how} ({source})": (lambda how, source: lambda m: _gap_frames(m, source)[0].merge(
            _gap_frames(m, source)[1], on="k", how=how))(how, source)
        for how in ("left", "right", "outer") for source in ("list", "csv")
    },
    **{
        f"merge outer left_on/right_on ({source})": (lambda source: lambda m: _gap_frames(m, source)[0].merge(
            _gap_frames(m, source)[1].rename(columns={"k": "k2"}), left_on="k", right_on="k2",
            how="outer"))(source)
        for source in ("list", "csv")
    },
    "series align": lambda m: m.Series(["a"], index=[0]).align(m.Series(["b"], index=[1]))[0],
    "series align, other side": lambda m: m.Series(["a"], index=[0]).align(m.Series(["b"], index=[1]))[1],
    "frame align": lambda m: m.DataFrame({"s": ["a"]}, index=[0]).align(
        m.DataFrame({"s": ["b"]}, index=[1]))[0],
    "series reindex": lambda m: m.Series(["a", "b"]).reindex([0, 1, 2]),
    "series shift": lambda m: m.Series(["a", "b"]).shift(1),
    "series shift back": lambda m: m.Series(["a", "b"]).shift(-1),
    "frame shift": lambda m: m.DataFrame({"s": ["a", "b"], "v": [1.0, 2.0]}).shift(1),
    "concat outer": lambda m: m.concat(list(_gap_frames(m, "list")), ignore_index=True),
    # NEGATIVE: a supplied None stays None beside the gaps an operation invents.
    "supplied None through merge": lambda m: m.DataFrame({"k": ["a", "b"], "s": [None, "y"]}).merge(
        m.DataFrame({"k": ["b", "c"], "t": ["p", "q"]}), on="k", how="outer"),
    "supplied None through align": lambda m: m.Series([None, "b"]).align(m.Series(["c"], index=[5]))[0],
    "supplied None through reindex": lambda m: m.Series([None, "b"]).reindex([0, 1, 2]),
}


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", sorted(_GAP_MARKER_CASES))
def test_invented_gap_marker_matches_pandas(case: str) -> None:
    # br-frankenpandas-7u2td: an object column's gap that merge/align invent is
    # NaN in pandas (frankenpandas minted None on the typed-Utf8 gathers), an
    # object shift fills None (frankenpandas filled NaN), and a supplied None
    # is kept either way.
    run = _GAP_MARKER_CASES[case]
    assert _plain(run(fpd)) == _plain(run(pd)), case


def _concat_side_keys(m: Any) -> dict:
    left = m.DataFrame({"a": [1, 2], "b": [3.0, 4.0]})
    right = m.DataFrame({"a": [5, 6], "c": ["x", "y"]}, index=[1, 2])
    outer = m.concat([left, right], axis=1, keys=["p", "q"])
    inner = m.concat([left, right], axis=1, keys=["p", "q"], join="inner")
    return {
        "outer": outer,
        "outer columns": [str(c) for c in outer.columns],
        "inner": inner,
        "top level": outer["q"],
        "one column": outer[("p", "b")],
        "the other side's same-named column": outer[("q", "a")],
        "flattened": [f"{k}_{c}" for k, c in outer.columns],
    }


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_concat_keys_side_by_side_match_pandas() -> None:
    # fvsao.6.3: concat(frames, axis=1, keys=[...]) raised - the result's
    # columns are a (key, column) MultiIndex, which the binding now carries.
    want, got = _concat_side_keys(pd), _concat_side_keys(fpd)
    for step in want:
        assert _plain_any(got[step]) == _plain_any(want[step]), step


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_duplicated_label_selects_every_column_like_pandas() -> None:
    # df[name] for a label two columns share is a frame of both, as pandas
    # (frankenpandas returned the first as a Series).
    for m in (pd, fpd):
        side = m.concat([m.DataFrame({"k": [1, 2], "a": [3, 4]}), m.DataFrame({"k": [5, 6]})], axis=1)
        picked = side["k"]
        assert type(picked).__name__ == "DataFrame"
        assert list(picked.columns) == ["k", "k"]
        assert [picked.iloc[:, i].tolist() for i in range(2)] == [[1, 2], [5, 6]]
        # NEGATIVE: a unique label is still a Series.
        assert type(side["a"]).__name__ == "Series"
        assert side["a"].tolist() == [3, 4]


def _array_view(a: Any) -> Any:
    if isinstance(a, np.ndarray):
        return ("ndarray", str(a.dtype), a.shape, [str(v) for v in a.ravel().tolist()])
    return (type(a).__name__, str(a))


def _numpy_results(m: Any) -> dict:
    ints = m.Series([1, 2, 3])
    floats = m.Series([1.5, None, 3.0])
    strings = m.Series(["a", None, "c"])
    frame = m.DataFrame({"a": [1, 2], "b": [3.5, 4.5]})
    int_frame = m.DataFrame({"a": [1, 2], "b": [3, 4]})
    mixed = m.DataFrame({"a": [1, 2], "s": ["x", "y"]})
    return {
        "int values": ints.values,
        "float values": floats.values,
        "object values": strings.values,
        "bool values": m.Series([True, False]).values,
        "datetime values": m.Series(m.to_datetime(["2024-01-01", None])).values,
        "timedelta values": m.Series(m.to_timedelta(["1h", None])).values,
        "to_numpy": ints.to_numpy(),
        "to_numpy dtype": ints.to_numpy(dtype=float),
        "to_numpy na_value": floats.to_numpy(na_value=0.0),
        "np.asarray(series)": np.asarray(floats),
        "np.array(series)": np.array(ints),
        "frame values": frame.values,
        "int frame values": int_frame.values,
        "mixed frame values": mixed.values,
        "frame to_numpy dtype": int_frame.to_numpy(dtype="float64"),
        "np.asarray(frame)": np.asarray(int_frame),
        "values.sum(axis=0)": frame.values.sum(axis=0),
        "index values": frame.index.values,
        "columns values": frame.columns.values,
        "values shape": frame.values.shape,
    }


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_values_and_to_numpy_are_numpy_arrays_like_pandas() -> None:
    # fvsao.7: .values / to_numpy() returned Python lists, to_numpy took no
    # dtype/na_value, and np.asarray(series) made a 0-d object array of the
    # repr text (no __array__).
    want, got = _numpy_results(pd), _numpy_results(fpd)
    for step in want:
        assert _array_view(got[step]) == _array_view(want[step]), step
    # NEGATIVE: the array is a copy - writing it leaves the Series alone.
    for m in (pd, fpd):
        s = m.Series([1, 2, 3])
        a = s.to_numpy(copy=True)
        a[0] = 99
        assert s.tolist() == [1, 2, 3]


_REPR_CASES = {
    "series int": lambda m: m.Series([1, 2, 3]),
    "series named float with gap": lambda m: m.Series([1.5, None], name="v"),
    "series str index": lambda m: m.Series(["a", "bb"], index=["x", "y"]),
    "series index name": lambda m: m.Series([1, 2], index=m.Index(["a", "b"], name="k")),
    "floats mixed decimals": lambda m: m.Series([1.0, 2.25, 3.5]),
    "floats integral": lambda m: m.Series([1.0, 2.0]),
    "floats big -> scientific": lambda m: m.Series([123456789.123, 1.5]),
    "floats tiny": lambda m: m.Series([0.000012345, 1.0]),
    "floats tinier -> scientific": lambda m: m.Series([0.0000001234, 1.0]),
    "floats all NaN": lambda m: m.Series([np.nan, np.nan]),
    "negative floats": lambda m: m.Series([-1.5, 2.0]),
    "ints": lambda m: m.Series([1, -20, 300]),
    "bools": lambda m: m.Series([True, False]),
    "object None and NaN": lambda m: m.Series(["a", None, np.nan]),
    "datetimes": lambda m: m.Series(m.to_datetime(["2024-01-01 00:00:00", "2024-01-02 03:04:05", None])),
    "dates": lambda m: m.Series(m.to_datetime(["2024-01-01", None])),
    "datetime millis": lambda m: m.Series(m.to_datetime(["2024-01-01 00:00:00.500", "2024-01-01 00:00:01.000"])),
    "timedeltas": lambda m: m.Series(m.to_timedelta(["1h", "2 days 3h", None])),
    "whole-day timedeltas": lambda m: m.Series(m.to_timedelta(["1 day", "2 days"])),
    "long (truncated)": lambda m: m.Series(range(70)),
    "long floats (truncated)": lambda m: m.Series([i / 4 for i in range(70)]),
    "nullable Int64": lambda m: m.Series([1, None], dtype="Int64"),
    "category": lambda m: m.Series(["a", "b", "a"], dtype="category"),
    "empty": lambda m: m.Series([], dtype="float64"),
    "empty named": lambda m: m.Series([], dtype="int64", name="v"),
    "unicode": lambda m: m.Series(["é", "日本"]),
    "float index": lambda m: m.Series([1, 2], index=[1.5, 2.0]),
    "datetime index": lambda m: m.Series([1, 2], index=m.to_datetime(["2024-01-01", "2024-01-02"])),
    "frame": lambda m: m.DataFrame({"a": [1, 2], "b": [1.5, None], "s": ["x", "yy"]}),
    "frame index name": lambda m: m.DataFrame({"a": [1, 2]}, index=m.Index(["p", "q"], name="k")),
    "frame renamed axis": lambda m: m.DataFrame({"a": [1, 2]}).rename_axis("k"),
    "empty frame": lambda m: m.DataFrame(),
    "empty frame with columns": lambda m: m.DataFrame({"a": [], "b": []}),
    "frame without columns": lambda m: m.DataFrame(index=[0, 1, 2]),
    "frame long (truncated)": lambda m: m.DataFrame({"a": range(70), "b": [i / 2 for i in range(70)]}),
    "frame bool column": lambda m: m.DataFrame({"f": [True, False], "x": [1, 2]}),
    "frame dates": lambda m: m.DataFrame({"d": m.to_datetime(["2024-01-01", "2024-02-01"]), "v": [1, 2]}),
    "frame long header": lambda m: m.DataFrame({"long_column_name": [1, 2], "b": ["x", "y"]}),
    "frame negatives": lambda m: m.DataFrame({"a": [-1, 2], "b": [-1.5, 2.25]}),
}


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", sorted(_REPR_CASES))
def test_repr_matches_pandas(case: str) -> None:
    # fvsao.7: every repr appended "Name: , Length: n" / "[n rows x m
    # columns]", printed Rust dtype names (Int64, Utf8), dropped the
    # index-name header and padded columns its own way.
    make = _REPR_CASES[case]
    assert repr(make(fpd)) == repr(make(pd)), case


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_index_name_writes_through_and_index_compares_elementwise_like_pandas() -> None:
    # fvsao.7: df.index.name = 'k' renamed a copy (the frame kept no name),
    # and index == x compared the objects, not the labels.
    for m in (pd, fpd):
        df = m.DataFrame({"a": [1, 2]}, index=["p", "q"])
        df.index.name = "k"
        assert df.index.name == "k"
        assert repr(df).splitlines()[1].startswith("k")
        s = m.Series([1, 2])
        s.index.name = "pos"
        assert s.index.name == "pos"
        # NEGATIVE: renaming a copy of the index leaves the frame alone.
        copy = df.index.copy()
        copy.name = "other"
        assert df.index.name == "k"
        frame = m.DataFrame({"a": [1, 2], "b": [3, 4]})
        assert (frame.columns == "a").tolist() == [True, False]
        assert (frame.columns != "a").tolist() == [False, True]
        assert (frame.index == [0, 5]).tolist() == [True, False]
        assert (frame.columns == m.Index(["a", "x"])).tolist() == [True, False]
        assert frame.loc[:, frame.columns != "a"].columns.tolist() == ["b"]


def _abc(m: Any) -> Any:
    return m.DataFrame({"a": [1, 2, 3], "b": [3, 2, 1]})


# fvsao.7: & | ^ raised "unsupported operand type(s)" on every Series and
# DataFrame, so df[(df.a > 1) & (df.b < 3)] could not be written; df.a raised
# AttributeError. Each case runs under pandas and frankenpandas.
_LOGICAL_CASES = {
    "bool & bool": lambda m: m.Series([True, True, False, False], name="m")
    & m.Series([True, False, True, False], name="m"),
    "bool | bool": lambda m: m.Series([True, True, False, False], name="m")
    | m.Series([True, False, True, False], name="m"),
    "bool ^ bool": lambda m: m.Series([True, True, False, False], name="m")
    ^ m.Series([True, False, True, False], name="m"),
    "names differ": lambda m: m.Series([True, False], name="m") & m.Series([True, True], name="n"),
    "reflected scalar": lambda m: False | m.Series([True, False], name="m"),
    # NEGATIVE: bitwise, not truthiness - True & 2 is 0.
    "bool & 2": lambda m: m.Series([True, False]) & 2,
    # NEGATIVE: a dtype-less list is an object array, cast to bool (truthiness).
    "bool & int list": lambda m: m.Series([True, True, False, False]) & [1, 2, 3, 0],
    "bool & int ndarray": lambda m: m.Series([True, True, False, False]) & np.array([1, 2, 3, 0]),
    "list broadcast": lambda m: m.Series([True, True, False]) & [True],
    "list wrong length": lambda m: m.Series([True, True, False]) & [True, False],
    "bool & None": lambda m: m.Series([True, False]) & None,
    "bool & float": lambda m: m.Series([True, False]) & 1.5,
    "int & int": lambda m: m.Series([6, 3, 5]) & m.Series([3, 5, 1]),
    "int ^ scalar": lambda m: m.Series([6, 3, 5]) ^ 1,
    "int & True": lambda m: m.Series([6, 3, 5]) & True,
    "int & Index": lambda m: m.Series([6, 3], name="q") & m.Index([3, 1], name="q"),
    "float & bool": lambda m: m.Series([1.0, 0.0]) & m.Series([True, False]),
    "str & True": lambda m: m.Series(["a", "b"]) & True,
    # NEGATIVE: the alignment fill is one-sided - a missing LEFT value is
    # False even against True (y | x differs from x | y).
    "unaligned x | y": lambda m: m.Series([True, True, False], index=["a", "b", "c"])
    | m.Series([True, False], index=["b", "d"]),
    "unaligned y | x": lambda m: m.Series([True, False], index=["b", "d"])
    | m.Series([True, True, False], index=["a", "b", "c"]),
    "unaligned ints": lambda m: m.Series([6, 3], index=[0, 1]) & m.Series([3, 1], index=[1, 2]),
    "object with None | True": lambda m: m.Series([True, None, False]) | True,
    "bool | object with None": lambda m: m.Series([True, False, False]) | m.Series([True, None, False]),
    # NEGATIVE: the nullable dtype is three-valued (Kleene), numpy bool is not.
    "boolean & boolean": lambda m: m.Series([True, None, False], dtype="boolean")
    & m.Series([None, None, True], dtype="boolean"),
    "boolean | boolean": lambda m: m.Series([True, None, False], dtype="boolean")
    | m.Series([None, None, True], dtype="boolean"),
    "bool & boolean": lambda m: m.Series([True, True, True]) & m.Series([True, None, False], dtype="boolean"),
    "boolean | NA": lambda m: m.Series([True, False], dtype="boolean") | m.NA,
    "bool | NA": lambda m: m.Series([True, False]) | m.NA,
    "boolean & int": lambda m: m.Series([True], dtype="boolean") & 1,
    "Int64 & Int64": lambda m: m.Series([6, None], dtype="Int64") & m.Series([3, 1], dtype="Int64"),
    "Int64 & True": lambda m: m.Series([6, None], dtype="Int64") & True,
    "category & True": lambda m: m.Series(["a"], dtype="category") & True,
    "datetime & True": lambda m: m.Series(m.to_datetime(["2024-01-01"])) & True,
    "frame & frame": lambda m: m.DataFrame({"a": [True, False], "b": [True, True]})
    | m.DataFrame({"a": [True, True], "b": [False, True]}),
    "frame & reordered frame": lambda m: m.DataFrame({"a": [True, False], "b": [True, True]})
    & m.DataFrame({"b": [False, False], "a": [True, True]}),
    "frame ^ scalar": lambda m: True ^ m.DataFrame({"a": [True, False], "b": [True, True]}),
    "frame & Series": lambda m: m.DataFrame({"a": [True, False], "b": [True, True]})
    & m.Series([False, True], index=["b", "a"]),
    "frame & list": lambda m: m.DataFrame({"a": [True, False], "b": [True, True]}) & [True, False],
    "frame & short list": lambda m: m.DataFrame({"a": [True, False], "b": [True, True]}) & [True],
    "frame & 2-D": lambda m: m.DataFrame({"a": [True, False], "b": [True, True]})
    & np.array([[True, True], [False, True]]),
    "int frame & 1": lambda m: m.DataFrame({"a": [3, 2]}) & 1,
    "mixed frame & True": lambda m: m.DataFrame({"a": [True], "s": ["x"]}) & True,
    "filter &": lambda m: _abc(m)[(_abc(m)["a"] > 1) & (_abc(m)["b"] > 1)],
    "filter | ~": lambda m: _abc(m)[~(_abc(m)["a"] > 1) | (_abc(m)["b"] == 1)],
    "loc filter": lambda m: _abc(m).loc[(_abc(m)["a"] > 1) & (_abc(m)["b"] > 0), "a"],
    "isin & notna": lambda m: (lambda df: df[df["k"].isin(["x", "y"]) & df["v"].notna()])(
        m.DataFrame({"k": ["x", "y", "z", "x"], "v": [1.0, None, 2.0, 3.0]})
    ),
    "df.a": lambda m: _abc(m).a,
    "df.a filter": lambda m: (lambda df: df[(df.a > 1) & (df.b > 1)])(_abc(m)),
    "s.label": lambda m: m.Series({"x": 1, "y": 2}).x,
    # NEGATIVE: attribute access reads labels only, not missing names.
    "df.missing": lambda m: _abc(m).nope,
    "s.missing": lambda m: m.Series([1]).nope,
    "hasattr": lambda m: (hasattr(_abc(m), "a"), hasattr(_abc(m), "zz")),
}


def _logical_outcome(m: Any, case: str) -> Any:
    import warnings

    def cell(v: Any) -> Any:
        return None if v is m.NA or (isinstance(v, float) and math.isnan(v)) else v

    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")
        try:
            r = _LOGICAL_CASES[case](m)
        except Exception as e:  # noqa: BLE001 - the exception is the outcome
            r = ("raise", type(e).__name__, str(e))
    categories = sorted({w.category.__name__ for w in caught})
    if hasattr(r, "columns"):
        return (
            [str(d) for d in r.dtypes.tolist()],
            [str(i) for i in r.index.tolist()],
            [str(c) for c in r.columns.tolist()],
            [[cell(v) for v in row] for row in r.values.tolist()],
            categories,
        )
    if hasattr(r, "index") and hasattr(r, "tolist"):
        return (str(r.dtype), r.name, [str(i) for i in r.index.tolist()], [cell(v) for v in r.tolist()], categories)
    return (r.item() if hasattr(r, "item") else r, categories)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_LOGICAL_CASES))
def test_logical_operators_and_attribute_access_match_pandas(case: str) -> None:
    assert _logical_outcome(fpd, case) == _logical_outcome(pd, case), case


# fvsao.22: astype(object) / dtype=object stringified every value (None became
# the string 'None', 1 became '1.0'), and DataFrame took no dtype= at all.
_OBJECT_DTYPE_CASES = {
    "Series(bools with None, dtype=object)": lambda m: m.Series([True, None, False], dtype=object),
    "Series(ints with None, dtype='object')": lambda m: m.Series([1, None, 2], dtype="object"),
    "Series(ints, dtype=np.object_)": lambda m: m.Series([1, 2], dtype=np.object_),
    "Series(mixed, dtype='O')": lambda m: m.Series([1.5, "a", None], dtype="O"),
    "int astype(object)": lambda m: m.Series([1, 2]).astype(object),
    "float with NaN astype('object')": lambda m: m.Series([1.0, None]).astype("object"),
    "astype(np.dtype('O'))": lambda m: m.Series([True, False]).astype(np.dtype("O")),
    "astype({name: object})": lambda m: m.Series([1, 2], name="v").astype({"v": object}),
    "category astype(object)": lambda m: m.Series(["a", "b"], dtype="category").astype(object),
    "datetime astype(object)": lambda m: m.Series(m.to_datetime(["2024-01-01", None])).astype(object),
    "frame astype(object)": lambda m: m.DataFrame({"a": [1, 2], "b": [1.5, None]}).astype(object),
    "frame astype({col: object})": lambda m: m.DataFrame({"a": [1, 2], "b": [1.5, None]}).astype({"b": object}),
    "DataFrame(dtype=object)": lambda m: m.DataFrame({"a": [1, None], "b": ["x", None]}, dtype=object),
    "DataFrame(dtype=float)": lambda m: m.DataFrame({"a": [1, 2], "b": [3, 4]}, dtype=float),
    "DataFrame(dtype='float64')": lambda m: m.DataFrame([[1, 2], [3, 4]], columns=["a", "b"], dtype="float64"),
    # NEGATIVE: astype(str) still stringifies - None is the string 'None'.
    "astype(str) stringifies None": lambda m: m.Series(["a", None]).astype(str),
    "object isna": lambda m: m.Series([1.0, None]).astype(object).isna(),
    "object fillna": lambda m: m.Series(["a", None], dtype=object).fillna("z"),
    "object == 1": lambda m: m.Series([1, 2], dtype=object) == 1,
    "object value_counts": lambda m: m.Series([1, 1, 2], dtype=object).value_counts(),
    "object tolist types": lambda m: [type(v).__name__ for v in m.Series([1, 2.5, True], dtype=object).tolist()],
}


def _object_outcome(m: Any, case: str) -> Any:
    def cell(v: Any) -> Any:
        if v is None or v is m.NA or (isinstance(v, float) and math.isnan(v)):
            return ("missing", type(v).__name__)
        return (type(v).__name__, str(v))

    try:
        r = _OBJECT_DTYPE_CASES[case](m)
    except Exception as e:  # noqa: BLE001 - the exception is the outcome
        return ("raise", type(e).__name__)
    if hasattr(r, "columns"):
        # tolist() gives Python scalars in both (iloc gives numpy's in pandas).
        return (
            [str(d) for d in r.dtypes.tolist()],
            [[cell(v) for v in r[c].tolist()] for c in r.columns],
        )
    if hasattr(r, "index") and hasattr(r, "tolist"):
        return (str(r.dtype), r.name, [str(i) for i in r.index.tolist()], [cell(v) for v in r.tolist()])
    return r


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_OBJECT_DTYPE_CASES))
def test_object_dtype_keeps_values_like_pandas(case: str) -> None:
    assert _object_outcome(fpd, case) == _object_outcome(pd, case), case


def _npv(m: Any) -> Any:
    return m.Series([1.0, 4.0, 9.0], index=["a", "b", "c"], name="v")


def _npi(m: Any) -> Any:
    return m.Series([1, -2, 3], name="i")


def _npdf(m: Any) -> Any:
    return m.DataFrame({"x": [1.0, 4.25], "y": [9.0, 16.0]}, index=["p", "q"])


# fvsao.7: numpy ufuncs returned bare ndarrays (index and name lost), an
# ndarray / list operand raised "Cannot convert ndarray to Scalar", and numpy
# functions that delegate to methods (np.round, np.cumsum, np.clip, np.any,
# np.transpose, np.repeat) fell back to ndarrays because the methods refused
# numpy's out= / dtype= / axis= keywords.
_NUMPY_INTEROP_CASES = {
    "np.sqrt(s)": lambda m: np.sqrt(_npv(m)),
    "np.log(s)": lambda m: np.log(_npv(m)),
    "np.exp(int)": lambda m: np.exp(_npi(m)),
    "np.abs(int) is abs": lambda m: np.abs(_npi(m)),
    "np.negative": lambda m: np.negative(_npi(m)),
    "np.sign": lambda m: np.sign(_npi(m)),
    "np.isnan unnamed": lambda m: np.isnan(m.Series([1.0, np.nan])),
    "np.maximum(s, 0) keeps name": lambda m: np.maximum(_npi(m), 0),
    "np.add(s, s)": lambda m: np.add(_npv(m), _npv(m)),
    "np.power(s, 2)": lambda m: np.power(_npv(m), 2),
    # NEGATIVE: two Series align by label first (outer), as pandas.
    "np.maximum unaligned": lambda m: np.maximum(_npv(m), m.Series([5.0, 0.0], index=["c", "d"], name="v")),
    "np.maximum names differ": lambda m: np.maximum(_npv(m), m.Series([5.0, 0.0, 1.0], index=["a", "b", "c"], name="w")),
    "np.modf tuple": lambda m: np.modf(m.Series([1.5, -2.25])),
    "np.divide int": lambda m: np.divide(_npi(m), 2),
    "np.floor": lambda m: np.floor(m.Series([1.5, -2.5])),
    "np.isfinite": lambda m: np.isfinite(m.Series([1.0, np.inf, np.nan])),
    "np.log1p(df)": lambda m: np.log1p(_npdf(m)),
    "np.sqrt(df)": lambda m: np.sqrt(_npdf(m)),
    "np.maximum(df, 0) 2-D": lambda m: np.maximum(m.DataFrame({"a": [1, -2], "b": [0.5, -1.0]}), 0),
    "np.modf(df)": lambda m: np.modf(_npdf(m)),
    "np.sqrt(mixed df) raises": lambda m: np.sqrt(m.DataFrame({"a": [1.0], "s": ["x"]})),
    "mixed frame and Series raises": lambda m: np.maximum(_npdf(m), m.Series([1.0, 2.0], index=["x", "y"])),
    "np.sqrt(Int64) is Float64": lambda m: np.sqrt(m.Series([1, None, 4], dtype="Int64")),
    "np.isnan(Float64) is boolean": lambda m: np.isnan(m.Series([1.5, None], dtype="Float64")),
    "np.sqrt(category) raises": lambda m: np.sqrt(m.Series([1.0, 4.0], dtype="category")),
    "np.add.reduce skips no NaN": lambda m: np.add.reduce(m.Series([1.0, np.nan])),
    "np.add.accumulate": lambda m: np.add.accumulate(_npv(m)),
    "np.add.outer raises": lambda m: np.add.outer(_npv(m), _npv(m)),
    "out=": lambda m: np.sqrt(_npv(m), out=np.empty(3)),
    "ndarray + s": lambda m: np.array([1.0, 2.0, 3.0]) + _npv(m),
    "s + ndarray": lambda m: _npv(m) + np.array([1.0, 2.0, 3.0]),
    "s * list": lambda m: _npv(m) * [1, 2, 3],
    "s + length-1 list broadcasts": lambda m: _npv(m) + [1],
    "s + wrong-length list": lambda m: _npv(m) + [1, 2],
    "s > ndarray": lambda m: _npv(m) > np.array([0.0, 5.0, 5.0]),
    "s == wrong-length list": lambda m: _npv(m) == [1, 2],
    "s + Index keeps a shared name": lambda m: _npi(m) + m.Index([1, 2, 3], name="i"),
    "np.float64 * s": lambda m: np.float64(2.0) * _npv(m),
    "np.int64 + s": lambda m: np.int64(2) + _npi(m),
    "df + list row": lambda m: _npdf(m) + [1, 2],
    "df + wrong-length list": lambda m: _npdf(m) + [1, 2, 3],
    "df + 2-D": lambda m: _npdf(m) + np.array([[1, 2], [3, 4]]),
    "df + wrong-shape 2-D": lambda m: _npdf(m) + np.ones((3, 2)),
    "df > list": lambda m: _npdf(m) > [1, 10],
    "ndarray + df": lambda m: np.array([[1.0, 1.0], [1.0, 1.0]]) + _npdf(m),
    "np.round(s, 1)": lambda m: np.round(m.Series([3.0, 1.26, 2.5], name="v"), 1),
    "np.round(df, 1)": lambda m: np.round(_npdf(m), 1),
    "np.cumsum(s)": lambda m: np.cumsum(_npi(m)),
    "np.cumprod(s)": lambda m: np.cumprod(_npi(m)),
    "np.cumsum(df)": lambda m: np.cumsum(_npdf(m)),
    "np.clip(s)": lambda m: np.clip(_npv(m), 1.5, 5),
    "np.clip(df)": lambda m: np.clip(_npdf(m), 2, 10),
    "np.any(s)": lambda m: bool(np.any(_npi(m) > 2)),
    "np.all(s)": lambda m: bool(np.all(_npi(m) > 2)),
    "any(skipna=False) NaN is True": lambda m: bool(m.Series([0.0, np.nan]).any(skipna=False)),
    "all(skipna=False) None is False": lambda m: bool(m.Series([True, None]).all(skipna=False)),
    "np.transpose(df)": lambda m: np.transpose(_npdf(m)),
    "np.repeat(s, 2)": lambda m: np.repeat(_npi(m), 2),
    # NEGATIVE: numpy's keywords are accepted only at their defaults.
    "cumsum(out=array) raises": lambda m: _npi(m).cumsum(out=np.empty(3)),
    "round(foo=1) raises": lambda m: _npi(m).round(foo=1),
    "transpose((1, 0)) raises": lambda m: _npdf(m).transpose((1, 0)),
    "repeat(axis=1) raises": lambda m: _npi(m).repeat(2, axis=1),
}


def _numpy_outcome(m: Any, case: str) -> Any:
    def cell(v: Any) -> Any:
        return None if v is m.NA or (isinstance(v, float) and math.isnan(v)) else v

    def shape(r: Any) -> Any:
        if isinstance(r, tuple):
            return tuple(shape(x) for x in r)
        if hasattr(r, "columns"):
            return (
                "frame",
                [str(d) for d in r.dtypes.tolist()],
                [str(i) for i in r.index.tolist()],
                [str(c) for c in r.columns.tolist()],
                [[cell(v) for v in r[c].tolist()] for c in r.columns],
            )
        if hasattr(r, "index") and hasattr(r, "tolist"):
            return ("series", str(r.dtype), r.name, [str(i) for i in r.index.tolist()], [cell(v) for v in r.tolist()])
        if isinstance(r, np.ndarray):
            return ("ndarray", str(r.dtype), [cell(v) for v in r.tolist()])
        # Scalars by value: numpy-vs-Python scalar TYPES are a separate item.
        return ("scalar", cell(r.item() if hasattr(r, "item") else r))

    try:
        return shape(_NUMPY_INTEROP_CASES[case](m))
    except Exception as e:  # noqa: BLE001 - the exception is the outcome
        return ("raise", type(e).__name__, str(e))


def _agg_s(m: Any) -> Any:
    return m.Series([1.0, 4.0, 9.0], name="v")


def _agg_df(m: Any) -> Any:
    return m.DataFrame({"k": ["a", "b", "a"], "v": [1.0, 2.0, 3.0], "w": [10, 20, 30]})


def _agg_num(m: Any) -> Any:
    return m.DataFrame({"x": [1.0, 4.0], "y": [9.0, 16.0]})


# fvsao.7: agg with a callable raised everywhere ("func must be a string or
# list of strings"); df.agg(['min', 'max']) and df.agg({'a': 'sum'}) raised;
# gb['v'].transform(np.mean) returned the per-group aggregate, not the
# broadcast; gb['v'].apply(f) was unnamed.
_AGG_CASES = {
    "s.agg(np.mean)": lambda m: _agg_s(m).agg(np.mean),
    "s.agg(builtin sum)": lambda m: _agg_s(m).agg(sum),
    "s.agg(np.std) is ddof=1": lambda m: _agg_s(m).agg(np.std),
    "s.agg(lambda aggregates)": lambda m: _agg_s(m).agg(lambda x: x.max() - x.min()),
    # NEGATIVE: an elementwise lambda is pandas' deprecated apply path.
    "s.agg(lambda elementwise)": lambda m: _agg_s(m).agg(lambda x: x + 1),
    "s.agg(len)": lambda m: _agg_s(m).agg(len),
    "s.agg(list with callables)": lambda m: _agg_s(m).agg(["min", np.mean, lambda x: x.sum() * 2]),
    "s.agg(dict)": lambda m: _agg_s(m).agg({"lo": "min", "hi": np.max}),
    "s.agg(method name)": lambda m: _agg_s(m).agg("nunique"),
    "s.agg(unknown name) raises": lambda m: _agg_s(m).agg("no_such_function"),
    "s.agg(args, kwargs)": lambda m: _agg_s(m).agg(lambda x, a, b=0: x.sum() * a + b, 0, 2, b=1),
    "df.agg(np.mean)": lambda m: _agg_num(m).agg(np.mean),
    "df.agg(list of names)": lambda m: _agg_num(m).agg(["min", "max"]),
    "df.agg(list with callables)": lambda m: _agg_num(m).agg(["sum", np.min, lambda c: c.max()]),
    "df.agg(dict of names)": lambda m: _agg_num(m).agg({"y": "sum", "x": np.mean}),
    "df.agg(dict with lists)": lambda m: _agg_num(m).agg({"x": ["sum", "max"], "y": "min"}),
    "df.agg(lambda)": lambda m: _agg_num(m).agg(lambda c: c.max() - c.min()),
    "df.agg(axis=1)": lambda m: _agg_num(m).agg("sum", axis=1),
    "df.agg(missing column) raises": lambda m: _agg_num(m).agg({"z": "sum"}),
    "gb.agg(np.mean)": lambda m: _agg_df(m).groupby("k").agg(np.mean),
    "gb.agg(['min', np.max])": lambda m: _agg_df(m).groupby("k").agg(["min", np.max]),
    "gb.agg(dict with callables)": lambda m: _agg_df(m).groupby("k").agg({"v": np.sum, "w": "max"}),
    "gb.agg(dict with a lambda)": lambda m: _agg_df(m).groupby("k").agg({"v": lambda x: x.max() - x.min()}),
    "gb.agg(named with a lambda)": lambda m: _agg_df(m).groupby("k").agg(
        rng=("v", lambda x: x.max() - x.min()), top=("w", "max")
    ),
    "gb.agg(lambda)": lambda m: _agg_df(m).groupby("k").agg(lambda x: x.max()),
    "gb['v'].agg(lambda)": lambda m: _agg_df(m).groupby("k")["v"].agg(lambda x: x.max() - x.min()),
    "gb['v'].agg(np.mean)": lambda m: _agg_df(m).groupby("k")["v"].agg(np.mean),
    "gb['v'].agg(list with lambdas)": lambda m: _agg_df(m).groupby("k")["v"].agg(
        ["sum", lambda x: x.max(), lambda x: x.min()]
    ),
    "gb['v'].agg(named)": lambda m: _agg_df(m).groupby("k")["v"].agg(total="sum", avg=np.mean),
    # NEGATIVE: a dict on a SeriesGroupBy is pandas' "nested renamer" error.
    "gb['v'].agg(dict) raises": lambda m: _agg_df(m).groupby("k")["v"].agg({"a": "sum"}),
    "gb['v'].apply named": lambda m: _agg_df(m).groupby("k")["v"].apply(lambda x: x.max()),
    # NEGATIVE: transform broadcasts to the rows, in row order (groups interleave).
    "gb['v'].transform(np.mean)": lambda m: _agg_df(m).groupby("k")["v"].transform(np.mean),
    "gb['v'].transform(scalar lambda)": lambda m: _agg_df(m).groupby("k")["v"].transform(lambda x: x.sum()),
    "gb['v'].transform(len)": lambda m: _agg_df(m).groupby("k")["v"].transform(len),
    "gb['v'].transform(Series lambda)": lambda m: _agg_df(m).groupby("k")["v"].transform(lambda x: x - x.mean()),
    "rolling.agg(np.mean)": lambda m: _agg_s(m).rolling(2).agg(np.mean),
    "rolling.agg(lambda)": lambda m: _agg_s(m).rolling(2).agg(lambda x: x.max()),
    "rolling.agg(list with a callable)": lambda m: _agg_s(m).rolling(2).agg(["sum", np.max]),
    "resample.agg(np.sum)": lambda m: m.Series(
        [1.0, 2.0, 3.0], index=m.to_datetime(["2024-01-01 00:00", "2024-01-01 12:00", "2024-01-02 00:00"])
    ).resample("D").agg(np.sum),
    "resample.agg(lambda)": lambda m: m.Series(
        [1.0, 2.0, 3.0], index=m.to_datetime(["2024-01-01 00:00", "2024-01-01 12:00", "2024-01-02 00:00"])
    ).resample("D").agg(lambda x: x.max()),
}


def _agg_outcome(m: Any, case: str) -> Any:
    import warnings

    def cell(v: Any) -> Any:
        if isinstance(v, float):
            return None if math.isnan(v) else round(v, 9)
        return v

    with warnings.catch_warnings():
        warnings.simplefilter("ignore")
        try:
            r = _AGG_CASES[case](m)
        except Exception as e:  # noqa: BLE001 - the exception is the outcome
            return ("raise", type(e).__name__)
    if hasattr(r, "columns"):
        return (
            "frame",
            [str(d) for d in r.dtypes.tolist()],
            [str(i) for i in r.index.tolist()],
            [str(c) for c in r.columns.tolist()],
            [[cell(v) for v in r.iloc[:, j].tolist()] for j in range(r.shape[1])],
        )
    if hasattr(r, "index") and hasattr(r, "tolist"):
        return ("series", str(r.dtype), r.name, [str(i) for i in r.index.tolist()], [cell(v) for v in r.tolist()])
    return ("scalar", cell(r.item() if hasattr(r, "item") else r))


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_AGG_CASES))
def test_agg_with_callables_lists_and_dicts_matches_pandas(case: str) -> None:
    assert _agg_outcome(fpd, case) == _agg_outcome(pd, case), case


def _pivot_frame(m: Any) -> Any:
    return m.DataFrame(
        {
            "city": ["NYC", "LA", "NYC", "SF", "LA"],
            "kind": ["a", "b", "b", "a", "a"],
            "age": [34, 45, 29, 51, 38],
            "score": [88.5, None, 92.0, 75.25, 81.0],
        }
    )


# fvsao.29: pivot_table required columns= (index/values/aggfunc alone raised),
# took no lists or callables, and refused margins.
_PIVOT_CASES = {
    "no columns mean": lambda m: _pivot_frame(m).pivot_table(index="city", values="age", aggfunc="mean"),
    "no columns int sum stays int": lambda m: _pivot_frame(m).pivot_table(index="city", values="age", aggfunc="sum"),
    "no columns two values": lambda m: _pivot_frame(m).pivot_table(index="city", values=["age", "score"]),
    "values None": lambda m: _pivot_frame(m)[["city", "age", "score"]].pivot_table(index="city", aggfunc="sum"),
    "aggfunc np.sum": lambda m: _pivot_frame(m).pivot_table(index="city", values="age", aggfunc=np.sum),
    "aggfunc count skips NaN": lambda m: _pivot_frame(m).pivot_table(index="city", values="score", aggfunc="count"),
    "aggfunc list, function level first": lambda m: _pivot_frame(m).pivot_table(
        index="city", values="age", aggfunc=["sum", "max"]
    ),
    "margins no columns": lambda m: _pivot_frame(m).pivot_table(index="city", values="age", aggfunc="sum", margins=True),
    # NEGATIVE: a mean margin is the mean of the original rows, not of the cells.
    "margins mean from rows": lambda m: _pivot_frame(m).pivot_table(
        index="city", columns="kind", values="age", aggfunc="mean", margins=True
    ),
    "margins with columns sum": lambda m: _pivot_frame(m).pivot_table(
        index="city", columns="kind", values="age", aggfunc="sum", margins=True
    ),
    "top-level pd.pivot_table": lambda m: m.pivot_table(
        _pivot_frame(m), values="age", index="city", columns="kind", aggfunc="max"
    ),
    "fill_value": lambda m: _pivot_frame(m).pivot_table(
        index="city", columns="kind", values="age", aggfunc="sum", fill_value=0
    ),
}


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_PIVOT_CASES))
def test_pivot_table_forms_match_pandas(case: str) -> None:
    import warnings

    def shape(r: Any) -> Any:
        def cell(v: Any) -> Any:
            return None if isinstance(v, float) and math.isnan(v) else (round(v, 9) if isinstance(v, float) else v)

        return (
            [str(d) for d in r.dtypes.tolist()],
            [str(i) for i in r.index.tolist()],
            r.index.name,
            [str(c) for c in r.columns.tolist()],
            [[cell(v) for v in r.iloc[:, j].tolist()] for j in range(r.shape[1])],
        )

    with warnings.catch_warnings():
        warnings.simplefilter("ignore")
        assert shape(_PIVOT_CASES[case](fpd)) == shape(_PIVOT_CASES[case](pd)), case


def _people(m: Any) -> Any:
    return m.DataFrame(
        {
            "name": ["Ann", "Bob", "Cid", "Dee", "Eve"],
            "city": ["NYC", "LA", "NYC", "SF", "LA"],
            "age": [34, 45, 29, 51, 38],
            "score": [88.5, None, 92.0, 75.25, 81.0],
        }
    )


# fvsao.30: everyday idioms a wide probe found wrong or raising.
_EVERYDAY_CASES = {
    # NEGATIVE: filter keeps the original row order when groups interleave.
    "gb.filter keeps row order": lambda m: _people(m).groupby("city").filter(lambda g: len(g) > 1),
    "sgb.filter keeps row order": lambda m: _people(m).groupby("city")["age"].filter(lambda g: len(g) > 1),
    "apply(axis=1) returning a Series": lambda m: _people(m)[["age", "score"]].apply(
        lambda r: m.Series({"s": r["age"] + 1}), axis=1
    ),
    "apply(axis=1) returning a scalar": lambda m: _people(m)[["age"]].apply(lambda r: r["age"] + 1, axis=1),
    "value_counts(normalize) name": lambda m: _people(m)["city"].value_counts(normalize=True),
    "Index.value_counts(normalize) name": lambda m: m.Index(["a", "b", "a"]).value_counts(normalize=True),
    "unique is an ndarray": lambda m: (type(_people(m)["city"].unique()).__name__, _people(m)["city"].unique().tolist()),
    "pd.unique(Index) is an ndarray": lambda m: (
        type(m.unique(m.Index([2, 1, 2]))).__name__,
        m.unique(m.Index([2, 1, 2])).tolist(),
    ),
    "memory_usage unnamed, deep=": lambda m: m.DataFrame({"a": [1, 2]}).memory_usage(index=False, deep=True).name,
    "insert a scalar": lambda m: (lambda d: (d.insert(1, "z", 0), d)[1])(_people(m)),
    "get_dummies prefix str": lambda m: m.get_dummies(_people(m)[["city"]], prefix="c"),
    "get_dummies prefix dict": lambda m: m.get_dummies(_people(m)[["name", "city"]], prefix={"city": "t", "name": "n"}),
    "describe(include='all')": lambda m: _people(m).describe(include="all"),
    "describe(include='object')": lambda m: _people(m).describe(include="object"),
    "describe(exclude='number')": lambda m: _people(m).describe(exclude="number"),
    "describe(percentiles=)": lambda m: _people(m).describe(percentiles=[0.1, 0.9]),
}


def _everyday_outcome(m: Any, case: str) -> Any:
    import warnings

    def cell(v: Any) -> Any:
        if hasattr(v, "item"):
            v = v.item()
        if isinstance(v, float):
            return None if math.isnan(v) else round(v, 9)
        return v

    with warnings.catch_warnings():
        warnings.simplefilter("ignore")
        try:
            r = _EVERYDAY_CASES[case](m)
        except Exception as e:  # noqa: BLE001 - the exception is the outcome
            return ("raise", type(e).__name__)
    if hasattr(r, "columns"):
        return (
            "frame",
            [str(d) for d in r.dtypes.tolist()],
            [str(i) for i in r.index.tolist()],
            [str(c) for c in r.columns.tolist()],
            [[cell(v) for v in r.iloc[:, j].tolist()] for j in range(r.shape[1])],
        )
    if hasattr(r, "index") and hasattr(r, "tolist"):
        return ("series", str(r.dtype), r.name, [str(i) for i in r.index.tolist()], [cell(v) for v in r.tolist()])
    return ("value", r)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_EVERYDAY_CASES))
def test_everyday_idioms_match_pandas(case: str) -> None:
    assert _everyday_outcome(fpd, case) == _everyday_outcome(pd, case), case


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_info_writes_pandas_layout_to_buf() -> None:
    import io

    texts = {}
    for m in (pd, fpd):
        buf = io.StringIO()
        assert _people(m).info(buf=buf) is None
        lines = buf.getvalue().splitlines()
        # The class line names each library's own class; the byte count is
        # each library's own (the layout, not the number, is the contract).
        texts[m.__name__] = lines[1:-1] + [lines[-1].split(":")[0]]
    assert texts["frankenpandas"] == texts["pandas"]


_QUERY_GLOBAL_LIMIT = 40


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_query_and_eval_resolve_at_references_like_pandas() -> None:
    # fvsao.28: every @name raised "unknown local reference" - the binding
    # never resolved the caller's variables.
    cities = ["LA", "SF"]
    name_q = "Bob"
    series_ref = None
    outcomes = {}
    for m in (pd, fpd):
        df = m.DataFrame({"name": ["Ann", "Bob", "Cid"], "city": ["NYC", "LA", "SF"], "age": [34, 45, 29]})
        series_ref = m.Series(["NYC"])
        got = {
            "@local scalar": df.query("name == @name_q")["name"].tolist(),
            "@global scalar": df.query("age > @_QUERY_GLOBAL_LIMIT")["name"].tolist(),
            "@list in": df.query("city in @cities")["name"].tolist(),
            "@Series in": df.query("city in @series_ref")["name"].tolist(),
            "@ in quotes is text": df.query("name != 'a@b'")["name"].tolist(),
            "local_dict": df.query("age < @x", local_dict={"x": 40})["name"].tolist(),
            "eval @": df.eval("age + @_QUERY_GLOBAL_LIMIT").tolist(),
            "eval assign @": df.eval("older = age + @_QUERY_GLOBAL_LIMIT")["older"].tolist(),
        }
        # NEGATIVE: an undefined name is pandas' UndefinedVariableError.
        with pytest.raises(Exception) as excinfo:
            df.query("age < @nope")
        got["undefined"] = (type(excinfo.value).__name__, str(excinfo.value))
        outcomes[m.__name__] = got
    assert outcomes["frankenpandas"] == outcomes["pandas"]


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_agg_numpy_callable_warns_like_pandas() -> None:
    import warnings

    for m in (pd, fpd):
        with warnings.catch_warnings(record=True) as caught:
            warnings.simplefilter("always")
            _agg_s(m).agg(np.mean)
        messages = [str(w.message) for w in caught if issubclass(w.category, FutureWarning)]
        assert any("is currently using Series.mean" in text for text in messages), m.__name__


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("axis", [0, 1])
@pytest.mark.parametrize(
    "op",
    ["sum", "prod", "mean", "median", "std", "var", "sem", "min", "max", "count", "nunique",
     "any", "all", "skew", "kurt", "idxmin", "idxmax"],
)
def test_frame_reductions_are_unnamed_like_pandas(op: str, axis: int) -> None:
    # fvsao.7: fp-frame named every DataFrame reduction after its op
    # (df.sum().name == 'sum', df.kurt().name == 'kurtosis'); pandas' is None.
    got = getattr(fpd.DataFrame({"a": [1.0, 2.0, 4.0], "b": [3.0, 5.0, 6.0]}), op)(axis=axis)
    want = getattr(pd.DataFrame({"a": [1.0, 2.0, 4.0], "b": [3.0, 5.0, 6.0]}), op)(axis=axis)
    assert (got.name, got.tolist()) == (want.name, pytest.approx(want.tolist(), nan_ok=True))


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_NUMPY_INTEROP_CASES))
def test_numpy_ufuncs_operands_and_delegation_match_pandas(case: str) -> None:
    import warnings

    with warnings.catch_warnings():
        warnings.simplefilter("ignore")
        assert _numpy_outcome(fpd, case) == _numpy_outcome(pd, case), case


def _jr_left(m: Any) -> Any:
    return m.DataFrame({"city": ["NYC", "LA", "SF"], "k": ["a", "b", "z"]}, index=m.Index([1, 2, 3], name="id"))


def _jr_timestamps(m: Any, idx: Any) -> list[str]:
    return [str(t) for t in idx]


# fvsao.31: join reset the caller's index to 0..n-1; pivot kept int64 beside
# a gap; cumcount/ngroup were named; read_json took only a path; to_csv
# refused float_format; stack returned a one-column frame with 'r|c' labels;
# date_range knew only fixed steps and string endpoints.
_JOIN_RESHAPE_CASES = {
    "join index left": lambda m: _jr_left(m).join(m.DataFrame({"v": [9, 8]}, index=[1, 2])),
    "join index inner": lambda m: _jr_left(m).join(m.DataFrame({"v": [9, 8]}, index=[1, 2]), how="inner"),
    "join index outer": lambda m: _jr_left(m).join(m.DataFrame({"v": [9, 7]}, index=[1, 5]), how="outer"),
    "join index right": lambda m: _jr_left(m).join(m.DataFrame({"v": [9, 8]}, index=[1, 2]), how="right"),
    "join on column": lambda m: _jr_left(m).join(m.DataFrame({"w": [5, 6]}, index=["a", "b"]), on="k"),
    "join on column inner": lambda m: _jr_left(m).join(
        m.DataFrame({"w": [5, 6]}, index=["a", "b"]), on="k", how="inner"
    ),
    "join overlap without suffix": lambda m: _jr_left(m).join(m.DataFrame({"city": ["x"]}, index=[1])),
    "join overlap suffixes": lambda m: _jr_left(m).join(
        m.DataFrame({"city": ["x"]}, index=[1]), lsuffix="_l", rsuffix="_r"
    ),
    "join Series": lambda m: _jr_left(m).join(m.Series([7, 6], index=[2, 3], name="s")),
    "join shared index name": lambda m: _jr_left(m).join(m.DataFrame({"v": [9]}, index=m.Index([1], name="id"))),
    "pivot gap promotes every column": lambda m: m.DataFrame(
        {"k": ["x", "x", "y"], "c": ["p", "q", "p"], "v": [1, 2, 3]}
    ).pivot(index="k", columns="c", values="v"),
    # NEGATIVE: without a gap the pivot stays int64.
    "pivot without gap stays int64": lambda m: m.DataFrame(
        {"k": ["x", "x", "y", "y"], "c": ["p", "q", "p", "q"], "v": [1, 2, 3, 4]}
    ).pivot(index="k", columns="c", values="v"),
    "groupby cumcount": lambda m: m.DataFrame({"g": ["a", "b", "a"]}).groupby("g").cumcount(),
    "groupby ngroup": lambda m: m.DataFrame({"g": ["b", "a", "b"]}).groupby("g").ngroup(),
    "series groupby cumcount": lambda m: m.Series([1, 2, 3]).groupby(m.Series(["a", "b", "a"])).cumcount(),
    "read_json buffer": lambda m: m.read_json(io.StringIO('[{"a":1,"b":"x"},{"a":2,"b":"y"}]')),
    "read_json lines": lambda m: m.read_json(io.StringIO('{"a":1}\n{"a":2}\n'), lines=True),
    "to_csv float_format": lambda m: m.DataFrame({"a": [1.25, None], "b": [1, 2]}).to_csv(index=False, float_format="%.1f"),
    "to_csv float_format callable": lambda m: m.DataFrame({"a": [1.25]}).to_csv(float_format=lambda v: f"<{v}>"),
    "stack drops missing": lambda m: m.DataFrame(
        {"a": [1, 2], "b": [3.0, None]}, index=m.Index(["r0", "r1"], name="rid")
    ).stack(),
    "stack keeps int64": lambda m: m.DataFrame({"a": [1, 2], "b": [3, 4]}).stack(),
    "stack dropna False": lambda m: m.DataFrame({"a": [1.0, None]}).stack(dropna=False),
    "stack future_stack": lambda m: m.DataFrame({"a": [1.0, None]}).stack(future_stack=True),
    "stack all missing keeps float64": lambda m: m.DataFrame({"a": [None, None]}, dtype=float).stack(),
    "stack level 1 raises": lambda m: m.DataFrame({"a": [1]}).stack(level=1),
    "stack dropna with future_stack raises": lambda m: m.DataFrame({"a": [1]}).stack(dropna=False, future_stack=True),
    "date_range W": lambda m: _jr_timestamps(m, m.date_range("2024-01-01", periods=3, freq="W")),
    "date_range W-MON": lambda m: _jr_timestamps(m, m.date_range("2024-01-03", periods=3, freq="W-MON")),
    "date_range MS keeps time": lambda m: _jr_timestamps(m, m.date_range("2024-01-01 10:00", periods=3, freq="MS")),
    "date_range MS rolls forward": lambda m: _jr_timestamps(m, m.date_range("2024-01-15", periods=3, freq="MS")),
    "date_range 2ME": lambda m: _jr_timestamps(m, m.date_range("2024-01-15", periods=3, freq="2ME")),
    "date_range QS start end": lambda m: _jr_timestamps(m, m.date_range("2024-01-15", "2024-06-01", freq="QS")),
    "date_range end periods ME": lambda m: _jr_timestamps(m, m.date_range(end="2024-06-01", periods=3, freq="ME")),
    "date_range B": lambda m: _jr_timestamps(m, m.date_range("2024-01-05", periods=4, freq="B")),
    "date_range YS": lambda m: _jr_timestamps(m, m.date_range("2024-01-01", periods=2, freq="YS")),
    "date_range SMS": lambda m: _jr_timestamps(m, m.date_range("2024-01-01", periods=3, freq="SMS")),
    "date_range periods 0": lambda m: _jr_timestamps(m, m.date_range("2024-01-01", periods=0, freq="W")),
    "date_range Timestamp start": lambda m: _jr_timestamps(m, m.date_range(m.Timestamp("2024-01-01"), periods=2)),
    "date_range date start": lambda m: _jr_timestamps(m, m.date_range(datetime.date(2024, 1, 1), periods=2)),
    "date_range datetime start": lambda m: _jr_timestamps(
        m, m.date_range(datetime.datetime(2024, 1, 1, 6), periods=2, freq="D")
    ),
    "date_range linspace": lambda m: _jr_timestamps(m, m.date_range("2024-01-01", "2024-01-02", periods=4)),
    "date_range normalize": lambda m: _jr_timestamps(m, m.date_range("2024-01-01 10:30", periods=2, normalize=True)),
    "date_range inclusive neither": lambda m: _jr_timestamps(
        m, m.date_range("2024-01-07", "2024-01-21", freq="W", inclusive="neither")
    ),
    "date_range inclusive right periods": lambda m: _jr_timestamps(
        m, m.date_range("2024-01-01", periods=3, freq="D", inclusive="right")
    ),
    # NEGATIVE: an inclusive side the range does not land on drops nothing.
    "date_range inclusive off anchor": lambda m: _jr_timestamps(
        m, m.date_range("2024-01-01", "2024-01-20", freq="W", inclusive="neither")
    ),
    "date_range end before start": lambda m: _jr_timestamps(m, m.date_range("2024-01-05", "2024-01-01", freq="W")),
    "date_range three of four": lambda m: m.date_range("2024-01-01", "2024-01-02", periods=2, freq="D"),
    "date_range bad inclusive": lambda m: m.date_range("2024-01-01", periods=2, inclusive="x"),
    "date_range bad freq": lambda m: m.date_range("2024-01-01", periods=2, freq="XYZ"),
    "bdate_range normalizes": lambda m: _jr_timestamps(m, m.bdate_range("2024-01-05 10:00", periods=3)),
}


def _join_reshape_outcome(m: Any, case: str) -> Any:
    import warnings

    def cell(v: Any) -> Any:
        if hasattr(v, "item"):
            v = v.item()
        if isinstance(v, float):
            return None if math.isnan(v) else round(v, 9)
        return v

    with warnings.catch_warnings():
        warnings.simplefilter("ignore")
        try:
            r = _JOIN_RESHAPE_CASES[case](m)
        except Exception as e:  # noqa: BLE001 - the exception is the outcome
            return ("raise", type(e).__name__, str(e))
    if hasattr(r, "columns"):
        return (
            "frame",
            [str(d) for d in r.dtypes.tolist()],
            [str(i) for i in r.index.tolist()],
            list(r.index.names),
            [str(c) for c in r.columns.tolist()],
            [[cell(v) for v in r.iloc[:, j].tolist()] for j in range(r.shape[1])],
        )
    if hasattr(r, "index") and hasattr(r, "tolist"):
        return (
            "series",
            str(r.dtype),
            r.name,
            [str(i) for i in r.index.tolist()],
            list(r.index.names),
            [cell(v) for v in r.tolist()],
        )
    return ("value", r)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_JOIN_RESHAPE_CASES))
def test_join_reshape_io_and_date_range_match_pandas(case: str) -> None:
    assert _join_reshape_outcome(fpd, case) == _join_reshape_outcome(pd, case), case


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_stack_and_date_range_deprecations_warn_like_pandas() -> None:
    import warnings

    def caught(call: Any) -> list[str]:
        with warnings.catch_warnings(record=True) as record:
            warnings.simplefilter("always")
            call()
        return [f"{w.category.__name__}: {w.message}" for w in record]

    for m in (pd, fpd):
        frame = m.DataFrame({"a": [1.0, None]})
        got = {
            "stack default": caught(lambda: frame.stack()),
            "stack dropna": caught(lambda: frame.stack(dropna=False)),
            "stack future": caught(lambda: frame.stack(future_stack=True)),
            "date_range M": caught(lambda: m.date_range("2024-01-15", periods=2, freq="M")),
            "date_range 2H": caught(lambda: m.date_range("2024-01-01", periods=2, freq="2H")),
            "date_range A-JUN": caught(lambda: m.date_range("2024-01-01", periods=2, freq="A-JUN")),
            # NEGATIVE: the current spellings do not warn.
            "date_range ME": caught(lambda: m.date_range("2024-01-15", periods=2, freq="ME")),
            "date_range h": caught(lambda: m.date_range("2024-01-01", periods=2, freq="h")),
        }
        if m is pd:
            want = got
        else:
            assert got == want


def _mi_frame(m: Any) -> Any:
    return m.DataFrame(
        {"a": ["x", "x", "y", "y"], "b": [1, 2, 1, 2], "v": [1.0, 2.5, 3.0, None], "w": [4, 5, 6, 7]}
    )


def _mi_index(m: Any) -> Any:
    return m.MultiIndex.from_tuples([("x", 1), ("x", 22), ("yy", 1), ("x", 1)], names=["k", None])


def _mi_assigned_index(m: Any) -> Any:
    frame = m.DataFrame({"v": [1, 2]})
    frame.index = m.MultiIndex.from_tuples([("p", 1), ("p", 2)], names=["g", "i"])
    return frame


# fvsao.34: a row or column MultiIndex fell out of pandas' repr layout into
# frankenpandas' Display ('x|p' labels, '[3 rows x 2 columns]', dtype Int64);
# the constructors dropped a MultiIndex index=/columns= to flat labels; and
# to_string printed a different table.
_MULTIINDEX_TEXT_CASES = {
    "groupby two keys frame": lambda m: repr(_mi_frame(m).groupby(["a", "b"]).sum()),
    "groupby two keys series": lambda m: repr(_mi_frame(m).groupby(["a", "b"])["w"].sum()),
    "groupby agg list column MultiIndex": lambda m: repr(_mi_frame(m).groupby("a")[["v", "w"]].agg(["sum", "mean"])),
    "groupby agg dict of lists": lambda m: repr(_mi_frame(m).groupby("a").agg({"v": ["sum", "max"], "w": ["min"]})),
    "set_index two columns": lambda m: repr(_mi_frame(m).set_index(["a", "b"])),
    "stack": lambda m: repr(m.DataFrame({"p": [1, 2], "q": [3.0, None]}, index=m.Index(["r0", "r1"], name="rid")).stack()),
    "series MultiIndex with a NaN value": lambda m: repr(m.Series([1.5, 2.0, None, 4.0], index=_mi_index(m), name="v")),
    "series three levels": lambda m: repr(
        m.Series([1, 2, 3], index=m.MultiIndex.from_tuples([("a", "b", 1), ("a", "b", 2), ("a", "c", 1)], names=["l0", "l1", "l2"]))
    ),
    "frame row MultiIndex": lambda m: repr(m.DataFrame({"a": [1, 2, 3, 4], "bb": ["p", "q", "r", "s"]}, index=_mi_index(m))),
    "frame column MultiIndex named": lambda m: repr(
        m.DataFrame(
            [[1, 2]],
            columns=m.MultiIndex.from_tuples([("a", "b"), ("a", "c")], names=["top", "sub"]),
            index=m.Index(["r"], name="row"),
        )
    ),
    "frame tuple-keyed dict": lambda m: repr(m.DataFrame({("x", "p"): [1, 2], ("x", "q"): [3.5, 4.0], ("y", "p"): ["s", "t"]})),
    "float and bool levels": lambda m: repr(
        m.Series([1, 2, 3], index=m.MultiIndex.from_tuples([(1.5, True), (2.0, False), (2.0, True)], names=["f", "flag"]))
    ),
    "truncated series": lambda m: repr(m.Series(range(100), index=m.MultiIndex.from_product([list("ab"), range(50)]))),
    "truncated frame": lambda m: repr(
        m.DataFrame({"v": range(100)}, index=m.MultiIndex.from_product([list("ab"), range(50)], names=["g", "i"]))
    ),
    "empty frame MultiIndex": lambda m: repr(m.DataFrame({"a": []}, index=m.MultiIndex.from_tuples([], names=["x", "y"]))),
    "no columns MultiIndex": lambda m: repr(m.DataFrame(index=m.MultiIndex.from_tuples([("x", 1), ("y", 2)]))),
    "index setter MultiIndex": lambda m: repr(_mi_assigned_index(m)),
    # NEGATIVE: outer labels that do not repeat blank nothing.
    "no repeated outer labels": lambda m: repr(m.Series([1, 2], index=m.MultiIndex.from_tuples([("a", 1), ("b", 2)]))),
    "frame to_string": lambda m: _mi_frame(m).set_index("a").to_string(),
    "frame to_string index False": lambda m: _mi_frame(m).to_string(index=False),
    "frame to_string MultiIndex": lambda m: _mi_frame(m).set_index(["a", "b"]).to_string(),
    "frame to_string all rows": lambda m: m.DataFrame({"v": range(70)}).to_string(),
    "frame to_string max_rows": lambda m: m.DataFrame({"v": range(10)}).to_string(max_rows=4),
    "frame to_string show_dimensions": lambda m: _mi_frame(m).to_string(show_dimensions=True),
    "frame to_string columns": lambda m: _mi_frame(m).to_string(columns=["w", "a"]),
    "series to_string": lambda m: m.Series([1.5, 2.0], index=["a", "b"], name="v").to_string(),
    "series to_string footer": lambda m: m.Series([1.5, 2.0], index=["a", "b"], name="v").to_string(name=True, dtype=True, length=True),
    "series to_string index False": lambda m: m.Series([1.5, -2.0], name="v").to_string(index=False),
    "series to_string MultiIndex": lambda m: _mi_frame(m).groupby(["a", "b"])["w"].sum().to_string(),
}


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_MULTIINDEX_TEXT_CASES))
def test_multiindex_repr_and_to_string_match_pandas(case: str) -> None:
    assert _MULTIINDEX_TEXT_CASES[case](fpd) == _MULTIINDEX_TEXT_CASES[case](pd), case


def _ts(m: Any) -> Any:
    return m.Timestamp("2024-01-05 10:30:15")


# fvsao.35 (scalars): Timestamp(datetime / date / np.datetime64 / nothing /
# a list) silently returned Timestamp.now(); DatetimeIndex.to_period used the
# day count as every frequency's ordinal ('2024-01-05' -> '3613-12' monthly);
# Timedelta('-1 days +02:03:04') - pandas' own printed form - parsed to
# -(1 day 2:03:04); and the datetime interop pandas users lean on was missing.
_TIMESTAMP_CASES = {
    "from datetime": lambda m: m.Timestamp(datetime.datetime(2024, 1, 5, 1)),
    "from date": lambda m: m.Timestamp(datetime.date(2024, 1, 5)),
    "from numpy datetime64": lambda m: m.Timestamp(np.datetime64("2024-01-05T01:02")),
    "no argument raises": lambda m: m.Timestamp(),
    "a list raises": lambda m: m.Timestamp([1]),
    # NEGATIVE: the string form is unchanged.
    "from string": lambda m: m.Timestamp("2024-01-05 10:30"),
    "date()": lambda m: _ts(m).date(),
    "time()": lambda m: _ts(m).time(),
    "to_pydatetime()": lambda m: _ts(m).to_pydatetime(),
    "weekday()": lambda m: m.Timestamp("2024-01-07").weekday(),
    "isoweekday()": lambda m: m.Timestamp("2024-01-07").isoweekday(),
    "isocalendar()": lambda m: tuple(m.Timestamp("2024-12-30").isocalendar()),
    "replace": lambda m: _ts(m).replace(day=9, hour=0, nanosecond=5),
    "replace out of range": lambda m: _ts(m).replace(month=13),
    "to_period M": lambda m: _ts(m).to_period("M"),
    "to_period without freq": lambda m: _ts(m).to_period(),
    "is_month_end": lambda m: m.Timestamp("2024-02-29").is_month_end,
    "is_quarter_start": lambda m: m.Timestamp("2024-04-01").is_quarter_start,
    "is_year_end": lambda m: m.Timestamp("2024-12-31 10:00").is_year_end,
    "plus datetime.timedelta": lambda m: _ts(m) + datetime.timedelta(days=2),
    "timedelta plus Timestamp": lambda m: datetime.timedelta(days=1) + _ts(m),
    "minus datetime.timedelta": lambda m: _ts(m) - datetime.timedelta(hours=1),
    "minus datetime": lambda m: _ts(m) - datetime.datetime(2024, 1, 5),
    "datetime minus Timestamp": lambda m: datetime.datetime(2024, 1, 6) - _ts(m),
    "equals datetime": lambda m: _ts(m) == datetime.datetime(2024, 1, 5, 10, 30, 15),
    "equals date is False": lambda m: m.Timestamp("2024-01-05") == datetime.date(2024, 1, 5),
    "orders against date raises": lambda m: m.Timestamp("2024-01-05") < datetime.date(2024, 1, 6),
    "equals int is False": lambda m: _ts(m) == 5,
    "orders against int raises": lambda m: _ts(m) < 5,
    "hash matches datetime": lambda m: hash(m.Timestamp("2024-01-05 01:00")) == hash(datetime.datetime(2024, 1, 5, 1)),
    "dict key found by datetime": lambda m: {m.Timestamp("2024-01-05"): 1}[datetime.datetime(2024, 1, 5)],
    "components as a tuple": lambda m: tuple(m.Timedelta("1 days 02:03:04.005006007").components),
    "components index and len": lambda m: (m.Timedelta("36h").components[1], len(m.Timedelta("36h").components)),
    "negative Timedelta printed form": lambda m: m.Timedelta("-1 days +02:03:04"),
    "negative Timedelta clock after days": lambda m: m.Timedelta("-3 days +23:59:59.5"),
    # NEGATIVE: without a clock part the leading sign negates everything.
    "negative Timedelta units only": lambda m: m.Timedelta("-1d2h"),
    "negative clock only": lambda m: m.Timedelta("-02:03:04"),
    "inner sign raises": lambda m: m.Timedelta("1 days -02:00:00"),
    "to_timedelta printed form": lambda m: m.to_timedelta(m.Series(["-1 days +02:03:04"])).dt.total_seconds().tolist(),
    "DatetimeIndex to_period M": lambda m: [str(p) for p in m.DatetimeIndex(["2024-01-05", "2024-03-20"]).to_period("M")],
    "DatetimeIndex to_period Q": lambda m: [str(p) for p in m.DatetimeIndex(["2024-01-05", "2024-08-20"]).to_period("Q")],
    "DatetimeIndex to_period without freq": lambda m: m.DatetimeIndex(["2024-01-05"]).to_period(),
    "DatetimeIndex tz_convert naive raises": lambda m: m.DatetimeIndex(["2024-01-05"]).tz_convert("UTC"),
    "DatetimeIndex tz_localize None": lambda m: [str(t) for t in m.DatetimeIndex(["2024-01-05"]).tz_localize(None)],
    "NaT date": lambda m: m.NaT.date(),
    # (fvsao.35 scalars, round 2) An aware to_pydatetime() dropped its zone;
    # aware minus naive subtracted the wall clock (pandas: TypeError);
    # timetz / to_datetime64 / ctime were missing and replace(tzinfo=)
    # refused. Timedelta: / 0 PANICKED (a PanicException escapes `except
    # Exception`); Timedelta(datetime.timedelta) and Timedelta(1.5, unit='s')
    # were 0 days; a datetime.timedelta / datetime / timedelta64 operand was
    # refused; // % divmod - abs round floor ceil to_pytimedelta were
    # missing; isoformat printed the repr text; a NaT duration moved an
    # instant by i64::MIN.
    "aware to_pydatetime keeps the zone": lambda m: m.Timestamp("2024-03-10 01:30", tz="US/Eastern").to_pydatetime(),
    "UTC to_pydatetime": lambda m: m.Timestamp("2024-03-10 01:30", tz="UTC").to_pydatetime(),
    "fixed offset to_pydatetime": lambda m: m.Timestamp("2024-03-10 01:30", tz="+05:30").to_pydatetime(),
    "aware timetz": lambda m: m.Timestamp("2024-03-10 01:30", tz="UTC").timetz(),
    "naive timetz": lambda m: _ts(m).timetz(),
    # The value (pandas' datetime64 carries the parsed unit, 's' here).
    "to_datetime64 of an aware Timestamp": lambda m: m.Timestamp("2024-03-10 01:30", tz="US/Eastern").to_datetime64() == np.datetime64("2024-03-10T06:30", "ns"),
    "ctime": lambda m: _ts(m).ctime(),
    "replace tzinfo": lambda m: _ts(m).replace(tzinfo=datetime.timezone.utc),
    "replace tzinfo None": lambda m: m.Timestamp("2024-01-05 10:30", tz="Asia/Tokyo").replace(tzinfo=None),
    "replace unknown keyword raises": lambda m: _ts(m).replace(years=1),
    "aware minus naive raises": lambda m: m.Timestamp("2024-01-05", tz="UTC") - _ts(m),
    "naive minus aware datetime raises": lambda m: _ts(m) - datetime.datetime(2024, 1, 1, tzinfo=datetime.timezone.utc),
    "aware minus aware": lambda m: m.Timestamp("2024-01-05", tz="UTC") - m.Timestamp("2024-01-05", tz="Asia/Tokyo"),
    "minus NaT": lambda m: _ts(m) - m.NaT,
    "plus numpy timedelta64": lambda m: _ts(m) + np.timedelta64(90, "m"),
    "past Timestamp.max raises": lambda m: m.Timestamp.max + m.Timedelta(1),
    "Timedelta plus datetime.timedelta": lambda m: m.Timedelta("1h") + datetime.timedelta(hours=2),
    "datetime.timedelta plus Timedelta": lambda m: datetime.timedelta(hours=2) + m.Timedelta("1h"),
    "datetime.timedelta minus Timedelta": lambda m: datetime.timedelta(hours=2) - m.Timedelta("1 days 02:03:04.5"),
    "Timedelta plus datetime": lambda m: m.Timedelta("1h") + datetime.datetime(2024, 1, 1),
    "datetime plus Timedelta": lambda m: datetime.datetime(2024, 1, 1) + m.Timedelta("1h"),
    "datetime minus Timedelta": lambda m: datetime.datetime(2024, 1, 1) - m.Timedelta("1h"),
    "Timedelta minus datetime raises": lambda m: m.Timedelta("1h") - datetime.datetime(2024, 1, 1),
    "Timedelta over zero raises": lambda m: m.Timedelta("1h") / 0,
    "Timedelta floor-divided by zero raises": lambda m: m.Timedelta("1h") // 0,
    "Timedelta over a zero Timedelta raises": lambda m: m.Timedelta("1h") / m.Timedelta(0),
    "floor division by a Timedelta": lambda m: m.Timedelta("1 days 02:03:04.5") // m.Timedelta("1h"),
    "floor division by a negative Timedelta": lambda m: m.Timedelta(7) // m.Timedelta(-2),
    "floor division by an int": lambda m: m.Timedelta(-7) // 2,
    "floor division by datetime.timedelta": lambda m: m.Timedelta("1 days 02:03:04.5") // datetime.timedelta(hours=2),
    "datetime.timedelta floor-divided by Timedelta": lambda m: datetime.timedelta(hours=2) // m.Timedelta("1h"),
    "modulo": lambda m: m.Timedelta("1 days 02:03:04.5") % m.Timedelta("1h"),
    "negative modulo": lambda m: m.Timedelta(-7) % m.Timedelta(2),
    "divmod": lambda m: divmod(m.Timedelta(-7), m.Timedelta(2)),
    "true division by datetime.timedelta": lambda m: m.Timedelta("1 days 02:03:04.5") / datetime.timedelta(hours=2),
    "datetime.timedelta over Timedelta": lambda m: datetime.timedelta(hours=2) / m.Timedelta("1h"),
    "int times Timedelta": lambda m: 3 * m.Timedelta("1h"),
    "float times truncates": lambda m: m.Timedelta(-7) * 0.5,
    "division truncates": lambda m: m.Timedelta(-7) / 2,
    "times NaN is NaT": lambda m: m.Timedelta(1) * float("nan"),
    "plus NaT is NaT": lambda m: m.Timedelta("1h") + m.NaT,
    "over NaT is NaN": lambda m: m.Timedelta("1h") / m.NaT,
    "times a bool raises": lambda m: m.Timedelta("1h") * True,
    "times a Timedelta raises": lambda m: m.Timedelta("1h") * m.Timedelta("1h"),
    "negate": lambda m: -m.Timedelta("1h"),
    "abs": lambda m: abs(m.Timedelta("-1h")),
    "zero is falsy": lambda m: bool(m.Timedelta(0)),
    "isoformat": lambda m: m.Timedelta("1 days 02:03:04.5").isoformat(),
    "isoformat negative": lambda m: m.Timedelta("-1h").isoformat(),
    "isoformat nanosecond": lambda m: m.Timedelta(1).isoformat(),
    "to_pytimedelta": lambda m: m.Timedelta("1 days 02:03:04.5").to_pytimedelta(),
    "to_pytimedelta rounds half to even": lambda m: (m.Timedelta(1500).to_pytimedelta(), m.Timedelta(2500).to_pytimedelta()),
    "to_pytimedelta negative": lambda m: m.Timedelta("-1h").to_pytimedelta(),
    "to_timedelta64": lambda m: m.Timedelta("1 days 02:03:04.5").to_timedelta64(),
    "round half to even": lambda m: (m.Timedelta("1h30min").round("h"), m.Timedelta("2h30min").round("h")),
    "round to 7 minutes": lambda m: m.Timedelta("1h").round("7min"),
    "round negative": lambda m: m.Timedelta("-1h30min").round("h"),
    "floor and ceil negative": lambda m: (m.Timedelta("-1h30min").floor("h"), m.Timedelta("-1h30min").ceil("h")),
    "round to a calendar frequency raises": lambda m: m.Timedelta("1h").round("MS"),
    "from datetime.timedelta": lambda m: m.Timedelta(datetime.timedelta(hours=2)),
    "from float seconds": lambda m: m.Timedelta(1.5, unit="s"),
    "from numpy timedelta64": lambda m: m.Timedelta(np.timedelta64(90, "m")),
    "float components": lambda m: m.Timedelta(days=1.5, minutes=0.5),
    "a unit with a string raises": lambda m: m.Timedelta("1h", unit="s"),
    "an unknown keyword raises": lambda m: m.Timedelta(years=1),
    "from a list raises": lambda m: m.Timedelta([1]),
    "hash matches datetime.timedelta": lambda m: hash(m.Timedelta("1h")) == hash(datetime.timedelta(hours=1)),
    "dict key found by datetime.timedelta": lambda m: {m.Timedelta("1h"): 1}[datetime.timedelta(hours=1)],
    "sub-microsecond hash is the int's": lambda m: hash(m.Timedelta(1500)) == hash(1500),
}


def _timestamp_outcome(m: Any, case: str) -> Any:
    try:
        r = _TIMESTAMP_CASES[case](m)
    except Exception as e:  # noqa: BLE001 - the exception type is the outcome
        return ("raise", type(e).__name__)
    if type(r).__name__ == "Period":
        return ("period", str(r))
    if type(r).__name__ == "NaTType":
        return ("NaT",)
    return ("value", type(r).__name__, repr(r))


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_TIMESTAMP_CASES))
def test_timestamp_timedelta_and_datetimeindex_scalars_match_pandas(case: str) -> None:
    assert _timestamp_outcome(fpd, case) == _timestamp_outcome(pd, case), case


# (4qg5w.13) Extreme arguments reached Rust arithmetic overflow or an
# infallible allocation: a PanicException (a BaseException that `except
# Exception` misses) or an aborted interpreter. Each now answers as pandas.
_EXTREME_ARGUMENT_CASES = {
    "Timedelta of a huge datetime.timedelta": lambda m: m.Timedelta(datetime.timedelta(days=999999999)),
    "Timestamp plus a huge datetime.timedelta": lambda m: m.Timestamp("2024-01-01") + datetime.timedelta(days=999999999),
    "strings times sys.maxsize": lambda m: m.Series(["a"]) * (2**63 - 1),
    "str.repeat(sys.maxsize)": lambda m: m.Series(["a"]).str.repeat(2**63 - 1),
    "empty strings times sys.maxsize": lambda m: (m.Series([""]) * (2**63 - 1)).tolist(),
    "strings times 3": lambda m: [None if v is None or v != v else v for v in (m.Series(["ab", None]) * 3).tolist()],
    "pct_change of floats by -2**63": lambda m: [None if v != v else v for v in m.Series([1.0, 2.0, 3.0]).pct_change(periods=-(2**63)).tolist()],
    "pct_change of ints by -2**63": lambda m: [None if v != v else v for v in m.Series([1, 2, 3]).pct_change(periods=-(2**63)).tolist()],
    "stack(level=sys.maxsize)": lambda m: m.DataFrame({"a": [1]}).stack(level=2**63 - 1),
    "Index.droplevel(sys.maxsize)": lambda m: m.Index([1, 2]).droplevel(2**63 - 1),
    # Second sweep: groupby periods sized a per-group buffer ('capacity
    # overflow'); a window quantile out of [0, 1] indexed past the window (a
    # panic) or, at 1.5 / -0.5, read a wrong row silently; a huge pad width
    # aborted the interpreter.
    **{f"SeriesGroupBy.{op}({n})": (lambda op, n: lambda m: getattr(_keyed_frame(m).groupby("k")["v"], op)(n))(op, n)
       for op in ["shift", "diff", "pct_change"] for n in [2**63 - 1, 10**18, 2**31]},
    **{f"DataFrameGroupBy.{op}({n})": (lambda op, n: lambda m: getattr(_keyed_frame(m).groupby("k"), op)(n))(op, n)
       for op in ["shift", "diff"] for n in [2**63 - 1, 2**31]},
    "Series.shift(sys.maxsize)": lambda m: [_missing_or(v) for v in m.Series([1.0, 2.0]).shift(2**63 - 1).tolist()],
    **{f"{kind} quantile({q})": (lambda kind, q: lambda m: getattr(m.Series([1.5, None, 3.0, 4.0]), kind)(*([2] if kind == "rolling" else [])).quantile(q))(kind, q)
       for kind in ["rolling", "expanding"] for q in [1.5, -0.5, float("inf"), 2**63 - 1]},
    **{f"str.{method}(sys.maxsize)": (lambda method: lambda m: getattr(m.Series(["ab", None]).str, method)(2**63 - 1))(method)
       for method in ["center", "ljust", "rjust", "zfill", "pad"]},
    # Keyword sweep: repeat(huge) overflowed `len * repeats` or the
    # capacity; value_counts(bins=huge) aborted the interpreter; a negative
    # repeat / bins raised the wrong class.
    **{f"{kind}.repeat({n})": (lambda kind, n: lambda m: {
        "Series": lambda: m.Series([1.5, 2.0]),
        "Index": lambda: m.Index([3, 1]),
        "DatetimeIndex": lambda: m.date_range("2024-01-01", periods=2),
    }[kind]().repeat(n))(kind, n)
       for kind in ["Series", "Index", "DatetimeIndex"] for n in [10**18, -1]},
    "value_counts(bins=10**18)": lambda m: m.Series([3, 1, 2, 1]).value_counts(bins=10**18),
    "value_counts(bins=0)": lambda m: m.Series([3, 1, 2, 1]).value_counts(bins=0),
    "value_counts(bins=-1)": lambda m: m.Series([3, 1, 2, 1]).value_counts(bins=-1),
    "cut(bins=10**18)": lambda m: m.cut(m.Series([3, 1, 2, 1]), bins=10**18),
}


def _keyed_frame(m: Any) -> Any:
    return m.DataFrame({"k": ["a", "b", "a", "b"], "v": [1, 2, 3, 4]})


def _extreme_outcome(m: Any, case: str) -> Any:
    try:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            result = _EXTREME_ARGUMENT_CASES[case](m)
    except BaseException as e:  # noqa: BLE001 - a panic is a BaseException; its class is the outcome
        return ("raise", type(e).__name__)
    return ("value", type(result).__name__, repr(result) if not isinstance(result, list) else result)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_EXTREME_ARGUMENT_CASES))
def test_extreme_arguments_raise_as_pandas_instead_of_panicking(case: str) -> None:
    assert _extreme_outcome(fpd, case) == _extreme_outcome(pd, case)


# pandas 2 keeps year 1 at microsecond resolution; frankenpandas stores
# nanoseconds only (Timestamp.unit, fvsao.35) and now raises
# OutOfBoundsDatetime - it PANICKED on the overflow.
@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.xfail(strict=True, reason="nanosecond-only Timestamp: year 1 is out of bounds (fvsao.35)")
def test_timestamp_of_year_one_matches_pandas() -> None:
    assert str(fpd.Timestamp("2024-01-01").replace(year=1)) == str(pd.Timestamp("2024-01-01").replace(year=1))


def _interval_breaks(index: Any) -> Any:
    """An IntervalIndex as its edges (floats), side and name - the int64
    subtype and the edge text are IntervalIndex parity (4qg5w.7)."""
    return ([(float(i.left), float(i.right)) for i in index], index.closed, index.name)


# (4qg5w.13) interval_range took only (start, periods) and (start, end),
# accumulated `cur += freq`, accepted all four parameters, and looped
# forever for freq=0 or a negative freq (memory exhausted); a huge count
# aborted the interpreter.
_INTERVAL_RANGE_CASES = {
    "start end": lambda m: _interval_breaks(m.interval_range(0, 5)),
    "start end freq": lambda m: _interval_breaks(m.interval_range(0, 5, freq=2)),
    "start end float freq": lambda m: _interval_breaks(m.interval_range(0.0, 1.0, freq=0.25)),
    "start periods freq": lambda m: _interval_breaks(m.interval_range(start=1, periods=3, freq=2)),
    "end periods": lambda m: _interval_breaks(m.interval_range(end=10, periods=3)),
    "end periods freq": lambda m: _interval_breaks(m.interval_range(end=10, periods=3, freq=2.5)),
    "start end periods": lambda m: _interval_breaks(m.interval_range(0, 1, periods=4)),
    "closed and name": lambda m: _interval_breaks(m.interval_range(0, 3, closed="left", name="k")),
    "a negative freq is empty": lambda m: _interval_breaks(m.interval_range(0, 5, freq=-1)),
    "an end before the start is empty": lambda m: _interval_breaks(m.interval_range(5, 0)),
    "freq zero raises": lambda m: m.interval_range(0, 5, freq=0),
    "four parameters raise": lambda m: m.interval_range(0, 5, periods=5, freq=1),
    "one parameter raises": lambda m: m.interval_range(start=0),
    "an unknown closed raises": lambda m: m.interval_range(0, 3, closed="bad"),
    "too many periods raise": lambda m: m.interval_range(start=0, periods=2**62),
}


def _interval_range_outcome(m: Any, case: str) -> Any:
    try:
        return _INTERVAL_RANGE_CASES[case](m)
    except BaseException as e:  # noqa: BLE001 - a panic is a BaseException; its class is the outcome
        return ("raise", type(e).__name__)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_INTERVAL_RANGE_CASES))
def test_interval_range_breaks_match_pandas(case: str) -> None:
    assert _interval_range_outcome(fpd, case) == _interval_range_outcome(pd, case)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_timestamp_of_year_one_is_a_typed_refusal_not_a_panic() -> None:
    with pytest.raises(fpd.errors.OutOfBoundsDatetime if hasattr(fpd, "errors") else ValueError):
        fpd.Timestamp("2024-01-01").replace(year=1)
    with pytest.raises(ValueError):
        fpd.Timestamp(datetime.datetime(1, 1, 1))


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_datetimeindex_tz_methods_do_not_ignore_tz() -> None:
    # tz_localize / tz_convert returned the index unchanged whatever tz was.
    # TEST-CHANGE (fvsao.55): tz_localize was then refused (NotImplementedError);
    # it now localizes, checked in test_tz_aware_datetime_index_matches_pandas.
    with pytest.raises(TypeError, match="Cannot convert tz-naive timestamps"):
        fpd.DatetimeIndex(["2024-01-05"]).tz_convert("US/Eastern")
    assert str(fpd.DatetimeIndex(["2024-01-05"]).tz_localize("UTC").tz) == "UTC"
    # Timestamp(tz=) is implemented: test_timezone_timestamps_series_dt_and_utc_parsing_match_pandas.


_OFFSET_TIMESTAMPS = ["2024-01-15 10:00", "2024-01-31 10:00", "2024-02-01", "2024-03-29", "2024-01-13", "2024-12-31 23:00"]


def _offset(m: Any, name: str) -> Any:
    o = m.offsets
    return {
        "MonthEnd": lambda: o.MonthEnd(),
        "MonthEnd(2)": lambda: o.MonthEnd(2),
        "MonthEnd(-1)": lambda: o.MonthEnd(-1),
        "MonthEnd(0)": lambda: o.MonthEnd(0),
        "MonthBegin(-1)": lambda: o.MonthBegin(-1),
        "BMonthEnd": lambda: o.BMonthEnd(),
        "BMonthBegin": lambda: o.BMonthBegin(),
        "QuarterEnd": lambda: o.QuarterEnd(),
        "QuarterEnd(startingMonth=1)": lambda: o.QuarterEnd(startingMonth=1),
        "QuarterBegin": lambda: o.QuarterBegin(),
        "YearEnd(month=6)": lambda: o.YearEnd(month=6),
        "YearBegin(-1)": lambda: o.YearBegin(-1),
        "BDay(3)": lambda: o.BDay(3),
        "BDay(-1)": lambda: o.BDay(-1),
        "Week": lambda: o.Week(),
        "Week(weekday=0)": lambda: o.Week(weekday=0),
        "Week(-1, weekday=4)": lambda: o.Week(-1, weekday=4),
        "SemiMonthEnd": lambda: o.SemiMonthEnd(),
        "SemiMonthBegin": lambda: o.SemiMonthBegin(),
        "Day(2)": lambda: o.Day(2),
        "Hour(-3)": lambda: o.Hour(-3),
        "Milli(5)": lambda: o.Milli(5),
        "DateOffset(months=1)": lambda: m.DateOffset(months=1),
        "DateOffset(months=-1)": lambda: m.DateOffset(months=-1),
        "DateOffset(n=2, months=1)": lambda: m.DateOffset(2, months=1),
        "DateOffset(years=1, days=2)": lambda: m.DateOffset(years=1, days=2),
        "DateOffset(day=1)": lambda: m.DateOffset(day=1),
        "DateOffset()": lambda: m.DateOffset(),
        "MonthEnd(normalize=True)": lambda: o.MonthEnd(normalize=True),
    }[name]()


# fvsao.35: MonthEnd/YearEnd were DateOffset(months=1)/(years=1), which
# added a flat 30/365 days (2024-01-15 + MonthEnd() read 2024-02-14), and
# every DateOffset(months=/years=) did the same; MonthBegin, QuarterEnd,
# BDay, ... did not exist.
@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize(
    "name",
    ["MonthEnd", "MonthEnd(2)", "MonthEnd(-1)", "MonthEnd(0)", "MonthBegin(-1)", "BMonthEnd", "BMonthBegin",
     "QuarterEnd", "QuarterEnd(startingMonth=1)", "QuarterBegin", "YearEnd(month=6)", "YearBegin(-1)", "BDay(3)",
     "BDay(-1)", "Week", "Week(weekday=0)", "Week(-1, weekday=4)", "SemiMonthEnd", "SemiMonthBegin", "Day(2)",
     "Hour(-3)", "Milli(5)", "DateOffset(months=1)", "DateOffset(months=-1)", "DateOffset(n=2, months=1)",
     "DateOffset(years=1, days=2)", "DateOffset(day=1)", "DateOffset()", "MonthEnd(normalize=True)"],
)
def test_date_offsets_move_timestamps_like_pandas(name: str) -> None:
    def outcome(m: Any) -> Any:
        off = _offset(m, name)
        return (
            repr(off),
            [str(m.Timestamp(t) + off) for t in _OFFSET_TIMESTAMPS],
            [str(m.Timestamp(t) - off) for t in _OFFSET_TIMESTAMPS],
        )

    assert outcome(fpd) == outcome(pd)


_OFFSET_SURFACE_CASES = {
    "Series + MonthEnd": lambda m: (m.Series(m.to_datetime(["2024-01-15", "2024-01-31", None])) + m.offsets.MonthEnd()).tolist(),
    "Series - MonthBegin": lambda m: (m.Series(m.to_datetime(["2024-01-15", None])) - m.offsets.MonthBegin()).tolist(),
    "offset + Series": lambda m: (m.offsets.MonthEnd() + m.Series(m.to_datetime(["2024-01-15"]))).tolist(),
    "Series + DateOffset(months=1)": lambda m: (m.Series(m.to_datetime(["2024-01-31", "2024-03-31"])) + m.DateOffset(months=1)).tolist(),
    "DatetimeIndex + MonthEnd": lambda m: [str(t) for t in m.DatetimeIndex(["2024-01-15", "2024-02-29"]) + m.offsets.MonthEnd()],
    "DatetimeIndex + Timedelta": lambda m: [str(t) for t in m.DatetimeIndex(["2024-01-15"]) + m.Timedelta(hours=1)],
    "offset times 2": lambda m: repr(m.offsets.MonthEnd() * 2),
    "negated": lambda m: repr(-m.offsets.QuarterEnd()),
    "equal": lambda m: m.offsets.MonthEnd(2) == m.offsets.MonthEnd(2),
    # NEGATIVE: a different count is a different offset.
    "not equal": lambda m: m.offsets.MonthEnd(2) == m.offsets.MonthEnd(1),
    "freqstr": lambda m: [m.offsets.MonthEnd(2).freqstr, m.offsets.Week(weekday=0).freqstr, m.offsets.QuarterBegin().freqstr, m.offsets.BDay().freqstr, m.offsets.Day(3).freqstr],
    "tick nanos": lambda m: m.offsets.Hour(2).nanos,
    "anchored nanos raises": lambda m: m.offsets.MonthEnd().nanos,
    "rollforward": lambda m: str(m.offsets.MonthEnd().rollforward(m.Timestamp("2024-01-15 10:00"))),
    "rollback": lambda m: str(m.offsets.MonthEnd().rollback(m.Timestamp("2024-01-15 10:00"))),
    "is_on_offset": lambda m: m.offsets.MonthEnd().is_on_offset(m.Timestamp("2024-01-31 10:00")),
    "date_range freq offset": lambda m: [str(t) for t in m.date_range("2024-01-01", periods=3, freq=m.offsets.MonthEnd())],
    "resample by offset": lambda m: m.Series([1, 2, 3], index=m.date_range("2024-01-01", periods=3, freq="D")).resample(m.offsets.MonthEnd()).sum().tolist(),
    # resample('MS') binned NOTHING (MS read as milliseconds, then an empty
    # grouping); QS was refused; on= raised; the Resampler was not
    # subscriptable.
    "resample MS": lambda m: m.Series([10.5, None, 7.25], index=m.to_datetime(["2024-01-05", "2024-01-06", "2024-03-01"])).resample("MS").sum().to_dict(),
    "resample 2MS": lambda m: m.Series([1.0, 2.0, 3.0], index=m.to_datetime(["2024-01-05", "2024-02-06", "2024-05-01"])).resample("2MS").sum().to_dict(),
    "resample QS": lambda m: m.Series([1.0, 2.0, 3.0], index=m.to_datetime(["2024-01-05", "2024-02-06", "2024-08-01"])).resample("QS").mean().to_dict(),
    "resample YS": lambda m: m.Series([1.0, 2.0], index=m.to_datetime(["2024-01-05", "2025-06-06"])).resample("YS").sum().to_dict(),
    "resample on column": lambda m: m.DataFrame({"when": m.to_datetime(["2024-01-05", "2024-01-06", "2024-02-01"]), "amount": [10.5, 1.0, 7.25]}).resample("MS", on="when")["amount"].sum().to_dict(),
    "resample on column subset": lambda m: m.DataFrame({"when": m.to_datetime(["2024-01-05", "2024-02-01"]), "a": [1, 2], "b": [3.0, 4.0]}).resample("MS", on="when")[["b"]].sum().to_dict(),
    # NEGATIVE: an integer index is pandas' TypeError, not an empty result.
    "resample int index raises": lambda m: m.Series([1, 2, 3], index=m.Index([1, 2, 3])).resample("MS").sum(),
    "Series index from a Series": lambda m: (lambda s: (s.index.tolist(), s.index.name))(m.Series([1, 2], index=m.Series(["a", "b"], index=[5, 6], name="k"))),
    "frame index from a Series": lambda m: (lambda d: (d.index.tolist(), d.index.name))(m.DataFrame({"v": [1, 2]}, index=m.Series(["a", "b"], index=[5, 6], name="k"))),
}


def _offset_surface_outcome(m: Any, case: str) -> Any:
    try:
        r = _OFFSET_SURFACE_CASES[case](m)
    except Exception as e:  # noqa: BLE001 - the exception type is the outcome
        return ("raise", type(e).__name__)
    if isinstance(r, dict):
        return {str(k): (None if isinstance(v, float) and math.isnan(v) else v) for k, v in r.items()}
    if isinstance(r, list):
        return [str(v) for v in r]
    return r


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_OFFSET_SURFACE_CASES))
def test_offsets_on_series_indexes_and_resample_match_pandas(case: str) -> None:
    assert _offset_surface_outcome(fpd, case) == _offset_surface_outcome(pd, case), case


def _timed_series(m: Any) -> Any:
    idx = m.to_datetime(["2024-01-05 00:00", "2024-01-06 00:00", "2024-01-09 00:00", "2024-02-01 00:00", "2024-02-02 12:00"])
    return m.Series([10.5, None, 7.25, 3.0, 12.0], index=idx, name="v")


def _grouper_frame(m: Any) -> Any:
    return m.DataFrame({"when": m.to_datetime(["2024-01-05", "2024-01-20", "2024-03-01"]), "city": ["a", "b", "a"], "amount": [1.0, 2.0, 4.0]})


# fvsao.35: rolling('7D') raised TypeError; pd.Grouper(key=, freq=) raised
# KeyError 'Grouper(...)'; resample('MS').sum() printed -0.0 for an empty
# month (Rust's f64 Sum starts from -0.0); SeriesGroupBy.sum/prod of an
# all-NaN group was NaN (pandas' min_count=0 gives 0.0 / 1.0); to_datetime
# turned a string it could not parse into a silent NaT, took no errors=,
# and read format='mixed' / 'ISO8601' as strftime patterns (all NaT).
_TIME_WINDOW_CASES = {
    "rolling 7D sum": lambda m: _timed_series(m).rolling("7D").sum().tolist(),
    "rolling 2D mean": lambda m: _timed_series(m).rolling("2D").mean().tolist(),
    "rolling 36h count": lambda m: _timed_series(m).rolling("36h").count().tolist(),
    "rolling 7D max": lambda m: _timed_series(m).rolling("7D").max().tolist(),
    "rolling 3D min_periods 2": lambda m: _timed_series(m).rolling("3D", min_periods=2).sum().tolist(),
    "rolling Day(3) offset": lambda m: _timed_series(m).rolling(m.offsets.Day(3)).sum().tolist(),
    # NEGATIVE: a time window over an integer index is pandas' ValueError.
    "rolling time window int index raises": lambda m: m.Series([1.0, 2.0]).rolling("2D").sum(),
    # NEGATIVE: a row-count window is unchanged.
    "rolling count window": lambda m: _timed_series(m).rolling(2).sum().tolist(),
    "Grouper key freq sum": lambda m: _grouper_frame(m).groupby(m.Grouper(key="when", freq="MS"))["amount"].sum().to_dict(),
    "Grouper key freq size": lambda m: _grouper_frame(m).groupby(m.Grouper(key="when", freq="MS")).size().to_dict(),
    "Grouper index freq mean": lambda m: _grouper_frame(m).set_index("when").groupby(m.Grouper(freq="MS"))["amount"].mean().to_dict(),
    "Grouper key only": lambda m: _grouper_frame(m).groupby(m.Grouper(key="city"))["amount"].sum().to_dict(),
    "Grouper ME count": lambda m: _grouper_frame(m).groupby(m.Grouper(key="when", freq="ME"))["amount"].count().to_dict(),
    "Series groupby Grouper": lambda m: _grouper_frame(m).set_index("when")["amount"].groupby(m.Grouper(freq="MS")).sum().to_dict(),
    "resample MS empty month is 0.0": lambda m: [repr(float(v)) for v in m.Series([1.0, 2.0], index=m.to_datetime(["2024-01-05", "2024-03-05"])).resample("MS").sum().tolist()],
    "groupby sum all-NaN group": lambda m: [repr(float(v)) for v in m.DataFrame({"g": ["a", "b", "b"], "v": [None, 1.0, None]}).groupby("g")["v"].sum().tolist()],
    "groupby prod all-NaN group": lambda m: [repr(float(v)) for v in m.DataFrame({"g": ["a", "b"], "v": [None, 2.0]}).groupby("g")["v"].prod().tolist()],
    # NEGATIVE: min_count=1 keeps the all-NaN group NaN.
    "groupby sum min_count 1": lambda m: [repr(float(v)) for v in m.DataFrame({"g": ["a", "b"], "v": [None, 1.0]}).groupby("g")["v"].sum(min_count=1).tolist()],
    "to_datetime format mismatch raises": lambda m: m.to_datetime(["2024-01-05", "2024-02-02 12:00"]),
    "to_datetime unparseable raises": lambda m: m.to_datetime(["2024-01-05", "not a date"]),
    "to_datetime coerce": lambda m: [str(t) for t in m.to_datetime(["2024-01-05", "not a date"], errors="coerce")],
    "to_datetime mixed": lambda m: [str(t) for t in m.to_datetime(["2024-01-05", "2024-02-02 12:00"], format="mixed")],
    "to_datetime ISO8601": lambda m: [str(t) for t in m.to_datetime(["2024-01-05", "2024-02-02T12:00:00"], format="ISO8601")],
    "to_datetime month names": lambda m: [str(t) for t in m.to_datetime(["Jan 5 2024", "Feb 2 2024"])],
    # NEGATIVE: null tokens stay NaT under errors='raise'.
    "to_datetime null tokens": lambda m: [str(t) for t in m.to_datetime(["2024-01-05", None, "NaT", ""])],
    "DatetimeIndex unparseable raises": lambda m: m.DatetimeIndex(["2024-01-05", "garbage"]),
}


def _time_window_outcome(m: Any, case: str) -> Any:
    try:
        r = _TIME_WINDOW_CASES[case](m)
    except ValueError:
        # pandas raises its DateParseError subclass for some of these; both
        # are caught by `except ValueError`, which is the contract.
        return ("raise", "ValueError")
    except Exception as e:  # noqa: BLE001 - the exception type is the outcome
        return ("raise", type(e).__name__)
    if isinstance(r, dict):
        return {str(k): (None if isinstance(v, float) and math.isnan(v) else v) for k, v in r.items()}
    if isinstance(r, list):
        return [None if isinstance(v, float) and math.isnan(v) else v for v in r]
    return r


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_TIME_WINDOW_CASES))
def test_time_windows_groupers_and_to_datetime_errors_match_pandas(case: str) -> None:
    assert _time_window_outcome(fpd, case) == _time_window_outcome(pd, case), case


def _mi_df(m: Any) -> Any:
    return m.DataFrame(
        {"a": ["x", "x", "y", "y"], "b": [1, 2, 1, 2], "v": [1.0, 2.5, 3.0, 4.0], "w": [4, 5, 6, 7]}
    ).set_index(["a", "b"])


# fvsao.36: a MultiIndex frame / Series kept flat 'x/1' storage labels, so
# every key that is a level value or a tuple missed (KeyError), level-
# addressed operations were refused, names were refused where positions
# worked, and flat labels leaked out of idxmax / T / isin.
_MULTIINDEX_SELECTION_CASES = {
    "loc outer label": lambda m: repr(_mi_df(m).loc["x"]),
    "loc full tuple": lambda m: _mi_df(m).loc[("x", 2)].tolist(),
    "loc full tuple and column": lambda m: float(_mi_df(m).loc[("x", 2), "v"]),
    "loc outer and column": lambda m: _mi_df(m).loc["y", "w"].tolist(),
    "loc list of tuples": lambda m: repr(_mi_df(m).loc[[("x", 1), ("y", 2)]]),
    "loc outer slice": lambda m: repr(_mi_df(m).loc["x":"y"]),
    "xs level name": lambda m: repr(_mi_df(m).xs(1, level="b")),
    "xs level keep": lambda m: repr(_mi_df(m).xs(1, level="b", drop_level=False)),
    "series loc outer": lambda m: repr(_mi_df(m)["v"].loc["y"]),
    "series getitem outer": lambda m: repr(_mi_df(m)["v"]["y"]),
    "series getitem tuple": lambda m: float(_mi_df(m)["v"][("y", 1)]),
    "reset_index one level": lambda m: repr(_mi_df(m).reset_index(level="b")),
    "reset_index drop level": lambda m: repr(_mi_df(m).reset_index(level=0, drop=True)),
    "droplevel name": lambda m: repr(_mi_df(m).droplevel("a")),
    "series droplevel": lambda m: repr(_mi_df(m)["w"].droplevel(1)),
    "sort_index level": lambda m: repr(_mi_df(m).sort_index(level="b")),
    "sort_index descending": lambda m: repr(_mi_df(m).sort_index(ascending=False)),
    "groupby level name": lambda m: repr(_mi_df(m).groupby(level="a").sum()),
    "groupby level int": lambda m: repr(_mi_df(m).groupby(level=0)["w"].sum()),
    "series groupby level": lambda m: repr(_mi_df(m)["w"].groupby(level=1).mean()),
    "unstack level 0": lambda m: _mi_df(m)["w"].unstack(level=0).to_dict(),
    "get_level_values name": lambda m: list(_mi_df(m).index.get_level_values("b")),
    "isin tuples": lambda m: [bool(v) for v in _mi_df(m).index.isin([("x", 1)])],
    "idxmax tuple": lambda m: tuple(v.item() if hasattr(v, "item") else v for v in _mi_df(m)["w"].idxmax()),
    "T column MultiIndex": lambda m: [tuple(c) for c in _mi_df(m).T.columns.tolist()],
    # NEGATIVE: a missing outer key is pandas' KeyError, not an empty frame.
    "loc missing outer raises": lambda m: _mi_df(m).loc["z"],
    # NEGATIVE: an unknown level name is pandas' KeyError.
    "droplevel unknown name raises": lambda m: _mi_df(m).droplevel("zz"),
    # NEGATIVE: a flat-index frame is unaffected.
    "flat loc": lambda m: m.DataFrame({"v": [1, 2]}, index=["p", "q"]).loc["q"].tolist(),
}


def _multiindex_selection_outcome(m: Any, case: str) -> Any:
    try:
        return _MULTIINDEX_SELECTION_CASES[case](m)
    except Exception as e:  # noqa: BLE001 - the exception type is the outcome
        return ("raise", type(e).__name__)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_MULTIINDEX_SELECTION_CASES))
def test_multiindex_selection_and_level_ops_match_pandas(case: str) -> None:
    assert _multiindex_selection_outcome(fpd, case) == _multiindex_selection_outcome(pd, case), case


def _ages(m: Any) -> Any:
    return m.DataFrame(
        {"age": [3, 7, 12, 25, 31, 8, 45, 15], "v": [1, 2, 3, 4, 5, 6, 7, 8], "w": [1.5, 2.5, 3.5, 4.5, 5.5, 6.5, 7.5, 8.5], "x": list("abababab")}
    ).assign(bin=lambda d: m.cut(d["age"], bins=[0, 5, 10, 20, 50]))


def _bools(m: Any) -> tuple[Any, Any]:
    return m.Series([True, False, True]), m.Series([True, True, False])


def _frame_pair(m: Any, kind: str) -> tuple[Any, Any]:
    left, right = {
        "str": ({"s": ["a", "b"]}, {"s": ["x", "y"]}),
        "bool": ({"b": [True, False]}, {"b": [True, True]}),
        "dt": ({"t": m.to_datetime(["2024-01-02", "2024-01-05"])}, {"t": m.to_datetime(["2024-01-01", "2024-01-01"])}),
        "td": ({"d": m.to_timedelta(["1h", "2h"])}, {"d": m.to_timedelta(["1h", "2h"])}),
        "mixed": ({"n": [1, 2], "s": ["a", "b"]}, {"n": [10, 20], "s": ["x", "y"]}),
    }[kind]
    return m.DataFrame(left), m.DataFrame(right)


def _frame_result(r: Any) -> Any:
    return ([str(d) for d in r.dtypes], {k: [str(v) for v in vs] for k, vs in r.to_dict("list").items()})


# The numeric sweep: bool Series arithmetic fell to float math (True + True
# was 2.0, -s negated to floats, a - b gave numbers where numpy refuses);
# DataFrame + DataFrame over a str / bool / datetime / timedelta column
# silently returned the LEFT operand's column; qcut/cut rejected
# labels=False; cut/qcut returned object strings, so sort_values, groupby
# and value_counts ordered bins as label text ("(10, 20]" before
# "(5, 10]"), .cat was refused, and a groupby over the bins raised;
# gb.groups gave positions (not row labels) in hash order, apply/filter ran
# groups in plain label order whatever sort= said, and agg({col: 'mean'})
# refused when an unrelated string column sat in the frame.
_NUMERIC_SWEEP_CASES = {
    "bool a+b": lambda m: (lambda a, b: (str((a + b).dtype), (a + b).tolist()))(*_bools(m)),
    "bool a*b": lambda m: (lambda a, b: (str((a * b).dtype), (a * b).tolist()))(*_bools(m)),
    "bool a+1": lambda m: (lambda a, b: (str((a + 1).dtype), (a + 1).tolist()))(*_bools(m)),
    "bool True+a": lambda m: (lambda a, b: (str((True + a).dtype), (True + a).tolist()))(*_bools(m)),
    "bool neg": lambda m: (lambda a, b: (str((-a).dtype), (-a).tolist()))(*_bools(m)),
    "bool a+b float": lambda m: (lambda a, b: (str((a + b.astype(float)).dtype), (a + b.astype(float)).tolist()))(*_bools(m)),
    # NEGATIVE: numpy refuses bool subtraction; pandas refuses / and **.
    "bool a-b raises": lambda m: (lambda a, b: a - b)(*_bools(m)),
    "bool a/b raises": lambda m: (lambda a, b: a / b)(*_bools(m)),
    "bool a**b raises": lambda m: (lambda a, b: a**b)(*_bools(m)),
    "frame str+str": lambda m: _frame_result(operator_add(*_frame_pair(m, "str"))),
    "frame bool+bool": lambda m: _frame_result(operator_add(*_frame_pair(m, "bool"))),
    "frame bool*bool": lambda m: (lambda l, r: _frame_result(l * r))(*_frame_pair(m, "bool")),
    "frame dt-dt": lambda m: (lambda l, r: _frame_result(l - r))(*_frame_pair(m, "dt")),
    "frame td+td": lambda m: _frame_result(operator_add(*_frame_pair(m, "td"))),
    "frame mixed+mixed": lambda m: _frame_result(operator_add(*_frame_pair(m, "mixed"))),
    "frame str+scalar": lambda m: _frame_result(_frame_pair(m, "str")[0] + "!"),
    "frame str*2": lambda m: _frame_result(_frame_pair(m, "str")[0] * 2),
    "frame bool+1": lambda m: _frame_result(_frame_pair(m, "bool")[0] + 1),
    "frame dt+Timedelta": lambda m: _frame_result(_frame_pair(m, "dt")[0] + m.Timedelta("1D")),
    # NEGATIVE: the operations pandas refuses still raise TypeError.
    "frame str*str raises": lambda m: (lambda l, r: l * r)(*_frame_pair(m, "str")),
    "frame dt+dt raises": lambda m: operator_add(*_frame_pair(m, "dt")),
    "frame mixed-mixed raises": lambda m: (lambda l, r: l - r)(*_frame_pair(m, "mixed")),
    "frame bool-bool raises": lambda m: (lambda l, r: l - r)(*_frame_pair(m, "bool")),
    "qcut labels False": lambda m: (lambda r: (str(r.dtype), r.tolist()))(m.qcut(m.Series([1, 2, 3, 4, 5, 6]), q=3, labels=False)),
    "qcut labels False NaN": lambda m: (lambda r: (str(r.dtype), r.tolist()))(m.qcut(m.Series([1, None, 3, 4]), q=2, labels=False)),
    "cut labels False": lambda m: (lambda r: (str(r.dtype), r.tolist()))(m.cut(m.Series([1, 5, 9]), bins=3, labels=False)),
    "cut labels False out of range": lambda m: m.cut(m.Series([1, 5, 20]), bins=[0, 5, 10], labels=False).tolist(),
    "cut dtype and codes": lambda m: (lambda b: (str(b.dtype), bool(b.cat.ordered), b.cat.codes.tolist()))(_ages(m)["bin"]),
    "cut categories": lambda m: [str(c) for c in _ages(m)["bin"].cat.categories],
    "cut labels categories": lambda m: list(m.cut(m.Series([1, 7, 12]), bins=[0, 5, 10, 20], labels=["lo", "mid", "hi"]).cat.categories),
    "qcut codes": lambda m: m.qcut(m.Series([1, 2, 3, 4, 5, 6, 7, 8]), 4).cat.codes.tolist(),
    "cut sort_values in bin order": lambda m: [str(v) for v in _ages(m)["bin"].sort_values().tolist()],
    "cut value_counts empty bin": lambda m: m.cut(m.Series([1, 2, 25]), bins=[0, 5, 10, 50]).value_counts(sort=False).to_dict(),
    "groupby bins sum": lambda m: _ages(m).groupby("bin", observed=True)["v"].sum().to_dict(),
    "groupby bins frame mean": lambda m: _ages(m).groupby("bin", observed=True)[["v", "w"]].mean().to_dict("list"),
    "groupby bins size": lambda m: _ages(m).groupby("bin", observed=True).size().to_dict(),
    "groupby bins observed False all seen": lambda m: _ages(m).groupby("bin", observed=False)["v"].sum().to_dict(),
    "groupby bins agg dict": lambda m: _ages(m).groupby("bin", observed=True).agg({"v": "sum", "w": "mean"}).to_dict("list"),
    "series groupby bins": lambda m: (lambda d: d["v"].groupby(d["bin"], observed=True).median().to_dict())(_ages(m)),
    "groupby bins and key": lambda m: _ages(m).groupby(["bin", "x"], observed=True)["v"].sum().to_dict(),
    "groupby bins sort False": lambda m: [str(k) for k in _ages(m).groupby("bin", observed=True, sort=False)["v"].sum().to_dict()],
    "groupby qcut": lambda m: (lambda d: d.groupby(m.qcut(d["w"], 3), observed=True)["v"].sum().to_dict())(_ages(m)),
    "groups keys bin order": lambda m: [str(k) for k in _ages(m).groupby("bin", observed=True).groups],
    "groups row labels": lambda m: {k: list(v) for k, v in m.DataFrame({"k": list("bacab"), "v": range(5)}, index=[10, 20, 30, 40, 50]).groupby("k").groups.items()},
    "groups sort False order": lambda m: list(m.DataFrame({"k": list("bacab"), "v": range(5)}).groupby("k", sort=False).groups),
    "series groups row labels": lambda m: (lambda d: {k: list(v) for k, v in d["v"].groupby(d["k"]).groups.items()})(m.DataFrame({"k": list("bacab"), "v": range(5)}, index=[10, 20, 30, 40, 50])),
    "indices positions": lambda m: {k: [int(i) for i in v] for k, v in m.DataFrame({"k": list("bacab"), "v": range(5)}, index=[10, 20, 30, 40, 50]).groupby("k").indices.items()},
    "apply sort False order": lambda m: m.DataFrame({"k": list("bacab"), "v": range(5)}).groupby("k", sort=False)["v"].apply(lambda s: int(s.sum())).to_dict(),
    "float key agg dict with str column": lambda m: m.DataFrame({"k": [1.5, 1.5, 2.5], "w": [1.5, 2.5, 3.5], "s": list("abc")}).groupby("k").agg({"w": "mean"}).to_dict("list"),
    # NEGATIVE: a plain string key still groups in label order.
    "str key order": lambda m: list(m.DataFrame({"k": ["b", "a", "c"], "v": [1, 2, 3]}).groupby("k")["v"].sum().to_dict()),
}


def operator_add(left: Any, right: Any) -> Any:
    return left + right


def _numeric_sweep_outcome(m: Any, case: str) -> Any:
    try:
        r = _NUMERIC_SWEEP_CASES[case](m)
    except Exception as e:  # noqa: BLE001 - the exception type is the outcome
        return ("raise", type(e).__name__)
    return _sweep_plain(r)


def _sweep_plain(r: Any) -> Any:
    """Keys as text (a tuple key's parts each as text: pandas' are Interval
    objects, frankenpandas' bin labels), NaN as None, recursively."""
    if isinstance(r, dict):
        return {
            str(tuple(map(str, k)) if isinstance(k, tuple) else k): _sweep_plain(v)
            for k, v in r.items()
        }
    if isinstance(r, (list, tuple)):
        return type(r)(_sweep_plain(v) for v in r)
    if isinstance(r, float) and math.isnan(r):
        return None
    return r


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_NUMERIC_SWEEP_CASES))
def test_bool_frame_arithmetic_and_binned_categoricals_match_pandas(case: str) -> None:
    assert _numeric_sweep_outcome(fpd, case) == _numeric_sweep_outcome(pd, case), case


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_category_groupby_default_observed_warns_and_unused_combinations_refuse() -> None:
    for m in (pd, fpd):
        d = _ages(m)
        with pytest.warns(FutureWarning, match="default of observed=False is deprecated"):
            d.groupby("bin")["v"].sum()
        with pytest.warns(FutureWarning, match="default of observed=False is deprecated"):
            d["v"].groupby(d["bin"]).sum()
    # Over SEVERAL keys pandas adds every unused (category x value)
    # combination; frankenpandas does not model that yet and refuses rather
    # than dropping the rows (a single key's unused categories answer: see
    # test_unused_categories_and_category_key_methods_match_pandas).
    sparse = {"age": [3, 7, 25], "v": [1, 2, 3]}
    expected = pd.DataFrame(sparse).assign(bin=lambda d: pd.cut(d["age"], [0, 5, 10, 20, 50])).groupby(["bin", "age"], observed=False)["v"].sum()
    assert len(expected) == 12
    frame = fpd.DataFrame(sparse).assign(bin=lambda d: fpd.cut(d["age"], [0, 5, 10, 20, 50]))
    with pytest.raises(NotImplementedError):
        frame.groupby(["bin", "age"], observed=False)["v"].sum()
    # The per-group operations without a modelled empty-group answer refuse
    # over unused categories too, rather than dropping the unused rows.
    unmodelled = {
        "describe": lambda g: g.describe(),
        "quantile": lambda g: g.quantile(0.5),
        "idxmax": lambda g: g.idxmax(),
        "ohlc": lambda g: g.ohlc(),
        "corr": lambda g: g.corr(),
        "skew": lambda g: g.skew(),
        "ngroup": lambda g: g.ngroup(),
        "series describe": lambda g: g["v"].describe(),
        "series ngroup": lambda g: g["v"].ngroup(),
        "series value_counts": lambda g: g["v"].value_counts(),
    }
    for op, call in unmodelled.items():
        with pytest.raises(NotImplementedError):
            call(frame.groupby("bin", observed=False))
            pytest.fail(op)
    # observed=True over the same frame answers.
    assert frame.groupby("bin", observed=True)["v"].sum().tolist() == [1, 2, 3]


def _sparse_bins(m: Any) -> Any:
    return m.DataFrame(
        {"age": [3, 7, 25, 30], "v": [1, 2, 3, 4], "w": [1.5, 2.5, 3.5, None], "s": list("abcd"), "b": [True, False, True, True]}
    ).assign(bin=lambda d: m.cut(d["age"], [0, 5, 10, 20, 50]))


def _unused(m: Any) -> Any:
    return _sparse_bins(m).groupby("bin", observed=False)


def _shuffled_bins(m: Any) -> Any:
    return m.DataFrame({"age": [25, 3, 7, 30, 12, 8], "v": [1, 5, 2, 4, 3, 6]}).assign(
        bin=lambda d: m.cut(d["age"], [0, 5, 10, 20, 50])
    )


def _vc_frame(m: Any) -> Any:
    return m.DataFrame({"k": ["b", "a", "b", "a", "c", "b", "b"], "v": [9, 5, 1, 5, 6, 1, 3]})


def _labelled(r: Any) -> Any:
    """A result as (row keys as text, column -> (dtype, values)) - pandas'
    keys are Interval objects, frankenpandas' the bin labels."""
    if hasattr(r, "columns"):
        return (
            [str(tuple(map(str, k))) if isinstance(k, tuple) else str(k) for k in r.index],
            {str(c): (str(r[c].dtype), _sweep_plain(r[c].tolist())) for c in r.columns},
        )
    return (
        [str(tuple(map(str, k))) if isinstance(k, tuple) else str(k) for k in r.index],
        str(r.dtype),
        _sweep_plain(r.tolist()),
    )


# fvsao.39: groupby(observed=False) - pandas 2.2's default - over a pd.cut
# column with an EMPTY bin refused (NotImplementedError); pandas adds the
# unused category's row holding each reduction's answer over no rows (0 for
# sum/count/size/nunique, 1 for prod, NaN otherwise with int turning float,
# False/True for any/all, None for an object first). Over a category key
# (observed=True) SeriesGroupBy ngroup / describe / ohlc / value_counts came
# in first-seen order, value_counts' index was one level of "group, value"
# strings (pandas: a (key, value) MultiIndex, count ties in value order),
# and gb.ngroups was a method where pandas has a property.
_UNUSED_CATEGORY_CASES = {
    **{
        f"frame {op}": (lambda op: lambda m: _labelled(getattr(_unused(m)[["v", "w"]], op)()))(op)
        for op in ["sum", "count", "mean", "median", "min", "max", "std", "var", "first", "last", "prod", "nunique", "sem"]
    },
    "frame size": lambda m: _labelled(_unused(m).size()),
    "frame any": lambda m: _labelled(_unused(m)[["b"]].any()),
    "frame all": lambda m: _labelled(_unused(m)[["b"]].all()),
    "frame sum min_count": lambda m: _labelled(_unused(m)[["v", "w"]].sum(min_count=1)),
    "series groupby sum": lambda m: (lambda d: _labelled(d["v"].groupby(d["bin"], observed=False).sum()))(_sparse_bins(m)),
    "series groupby mean": lambda m: (lambda d: _labelled(d["v"].groupby(d["bin"], observed=False).mean()))(_sparse_bins(m)),
    "series groupby size": lambda m: (lambda d: _labelled(d["v"].groupby(d["bin"], observed=False).size()))(_sparse_bins(m)),
    "column min": lambda m: _labelled(_unused(m)["v"].min()),
    "column str first": lambda m: _labelled(_unused(m)["s"].first()),
    "column str min": lambda m: _labelled(_unused(m)["s"].min()),
    "column str count": lambda m: _labelled(_unused(m)["s"].count()),
    "agg list": lambda m: _labelled(_unused(m)["v"].agg(["sum", "mean"])),
    "agg dict": lambda m: _labelled(_unused(m).agg({"v": "sum", "w": "mean"})),
    "agg named": lambda m: _labelled(_unused(m).agg(total=("v", "sum"), avg=("w", "mean"))),
    "frame agg list": lambda m: _labelled(_unused(m)[["v", "w"]].agg(["sum", "max"])),
    "apply sees empty group": lambda m: _labelled(_unused(m)["v"].apply(lambda s: int(s.sum()))),
    "ngroups property": lambda m: _unused(m).ngroups,
    "groups include unused": lambda m: [str(k) for k in _unused(m).groups],
    "cumsum unaffected": lambda m: _unused(m)["v"].cumsum().tolist(),
    "transform unaffected": lambda m: _unused(m)["v"].transform("sum").tolist(),
    "sort False appends unused": lambda m: _labelled(_sparse_bins(m).groupby("bin", observed=False, sort=False)["v"].sum()),
    # NEGATIVE: observed=True leaves the empty bin out.
    "observed True omits unused": lambda m: _labelled(_sparse_bins(m).groupby("bin", observed=True)["v"].sum()),
    "category key ngroup": lambda m: _shuffled_bins(m).groupby("bin", observed=True)["v"].ngroup().tolist(),
    "category key describe": lambda m: _labelled(_shuffled_bins(m).groupby("bin", observed=True)["v"].describe()),
    "category key ohlc order": lambda m: [str(k) for k in _shuffled_bins(m).groupby("bin", observed=True)["v"].ohlc().index],
    "category key value_counts": lambda m: _labelled(_shuffled_bins(m).groupby("bin", observed=True)["v"].value_counts()),
    "value_counts MultiIndex": lambda m: (lambda r: (_labelled(r), list(r.index.names), r.name, r.index.nlevels))(_vc_frame(m).groupby("k")["v"].value_counts()),
    "series value_counts": lambda m: (lambda d: _labelled(d["v"].groupby(d["k"]).value_counts()))(_vc_frame(m)),
}


def _unused_category_outcome(m: Any, case: str) -> Any:
    with warnings.catch_warnings():
        warnings.simplefilter("ignore", FutureWarning)
        try:
            return _UNUSED_CATEGORY_CASES[case](m)
        except Exception as e:  # noqa: BLE001 - the exception type is the outcome
            return ("raise", type(e).__name__)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_UNUSED_CATEGORY_CASES))
def test_unused_categories_and_category_key_methods_match_pandas(case: str) -> None:
    assert _unused_category_outcome(fpd, case) == _unused_category_outcome(pd, case), case


def _steps_frame(m: Any) -> Any:
    return m.DataFrame({"a": [10, 11, 12, 13, 14], "b": [0.5, 1.5, 2.5, 3.5, 4.5]})


def _lettered(m: Any) -> Any:
    return m.Series([1, 2, 3, 4], index=["a", "b", "c", "d"])


# Slices with a step were read as start:stop alone at every positional read:
# s.iloc[::2] / df.iloc[::2] returned EVERY row, s[::-1] / df.iloc[::-1] an
# EMPTY result; .loc refused any step; s['b':'d'] (label bounds) raised
# TypeError. The Index repr had no dtype, never wrapped or truncated;
# Index(series) took the Series' INDEX labels (pandas: its values), a bool
# list became 1/0, dtype= raised; set operations and get_indexer refused a
# list (df.columns.difference(['id'])), union came back unsorted,
# get_indexer returned a list, equals(list) raised; df.index.name = 'x' on
# a datetime index renamed a copy and a TimedeltaIndex name was read-only.
_SLICE_AND_INDEX_CASES = {
    "iloc step": lambda m: _steps_frame(m).iloc[::2]["a"].tolist(),
    "iloc step index": lambda m: list(_steps_frame(m).iloc[::2].index),
    "iloc offset step": lambda m: _steps_frame(m).iloc[1::2]["a"].tolist(),
    "iloc reverse": lambda m: _steps_frame(m).iloc[::-1]["a"].tolist(),
    "iloc reverse bounded": lambda m: _steps_frame(m).iloc[4:0:-2]["a"].tolist(),
    "series iloc step": lambda m: _steps_frame(m)["a"].iloc[::2].tolist(),
    "series reverse": lambda m: _steps_frame(m)["a"][::-1].tolist(),
    "frame getitem reverse": lambda m: _steps_frame(m)[::-1]["a"].tolist(),
    "iloc rows step col": lambda m: _steps_frame(m).iloc[::2, 0].tolist(),
    "iloc 2d reverse both": lambda m: _steps_frame(m).iloc[::-1, ::-1].values.tolist(),
    "iloc row reverse cols": lambda m: _steps_frame(m).iloc[0, ::-1].tolist(),
    # NEGATIVE: unit steps and negative bounds are unchanged.
    "iloc negative bounds": lambda m: (_steps_frame(m)["a"].iloc[-3:].tolist(), _steps_frame(m)["a"].iloc[:-2].tolist()),
    "loc step": lambda m: _steps_frame(m).loc[::2, "a"].tolist(),
    "loc reverse bounded": lambda m: _steps_frame(m).loc[3:1:-1, "a"].tolist(),
    "loc reverse from": lambda m: _steps_frame(m).loc[3::-2, "a"].tolist(),
    "series loc reverse": lambda m: _steps_frame(m)["a"].loc[::-1].tolist(),
    "loc column step": lambda m: list(m.DataFrame({"a": [1], "b": [2], "c": [3], "d": [4]}).loc[:, "a":"c":2].columns),
    "loc column reverse": lambda m: list(m.DataFrame({"a": [1], "b": [2], "c": [3], "d": [4]}).loc[:, "c":"a":-1].columns),
    "label slice getitem": lambda m: _lettered(m)["b":"c"].tolist(),
    "label slice open": lambda m: (_lettered(m)["b":].tolist(), _lettered(m)[:"b"].tolist()),
    "label slice reverse": lambda m: _lettered(m)["d":"b":-1].tolist(),
    "frame label slice": lambda m: m.DataFrame({"v": [1, 2, 3, 4]}, index=list("abcd"))["b":"c"]["v"].tolist(),
    "date string slice": lambda m: m.Series([1, 2, 3, 4], index=m.to_datetime(["2024-01-05", "2024-02-01", "2024-02-20", "2024-03-02"]))["2024-02":"2024-03"].tolist(),
    # NEGATIVE: integer bounds on an integer index stay positional.
    "int index positional": lambda m: m.Series([10, 20, 30], index=[5, 6, 7])[1:3].tolist(),
    **{
        f"repr {label}": (lambda make: lambda m: repr(make(m)))(make)
        for label, make in {
            "ints": lambda m: m.Index([0, 1, 2]),
            "strings": lambda m: m.Index(["a", "b"]),
            "empty": lambda m: m.Index([]),
            "floats": lambda m: m.Index([1.5, 2.0]),
            "bools": lambda m: m.Index([True, False]),
            "named": lambda m: m.Index(["a", "b"], name="k"),
            "wrapped strings": lambda m: m.Index(list("abcdefghijklmnopqrstuvwxyz") * 2),
            "wrapped justified ints": lambda m: m.Index(list(range(95, 130))),
            "truncated ints": lambda m: m.Index(list(range(1000))),
            "truncated strings": lambda m: m.Index([f"x{i}" for i in range(200)]),
            "float nan": lambda m: m.Index([1.0, None, 3.5]),
            "object none": lambda m: m.Index(["a", None]),
            "mixed": lambda m: m.Index([1, "a", 2.5]),
            "columns": lambda m: m.DataFrame({"alpha": [1], "beta": [2]}).columns,
            "set_index name": lambda m: m.DataFrame({"alpha": [1, 2], "b": [3, 4]}).set_index("alpha").index,
            "datetime dates": lambda m: m.to_datetime(["2024-01-01", "2024-01-02"]),
            "datetime nat named": lambda m: m.DatetimeIndex(["2024-01-01", None], name="d"),
            "datetime times": lambda m: m.to_datetime(["2024-01-01 12:30", "2024-01-02 00:00"]),
            "datetime fraction": lambda m: m.to_datetime(["2024-01-01 00:00:00.5"]),
            "datetime wrapped": lambda m: m.to_datetime(["2024-01-01"] * 8),
            "datetime truncated": lambda m: m.to_datetime(["2024-01-01"] * 120),
            "datetime frame index": lambda m: m.Series([1, 2], index=m.to_datetime(["2024-01-01", "2024-01-02"])).index,
            "timedelta days": lambda m: m.to_timedelta(["1D", "2D"]),
            "timedelta long nat": lambda m: m.to_timedelta(["1D", "2h", None]),
            "timedelta named": lambda m: m.TimedeltaIndex(["1D"], name="lag"),
        }.items()
    },
    "Index of Series values": lambda m: repr(m.Index(m.Series([3, 1], index=["a", "b"], name="v"))),
    "Index dtype float": lambda m: repr(m.Index([1, 2], dtype="float64")),
    "Index dtype parses": lambda m: repr(m.Index(["1", "2"], dtype="int64")),
    "columns difference list": lambda m: repr(m.DataFrame({"id": [1], "a": [2], "b": [3]}).columns.difference(["id"])),
    "columns union sorts": lambda m: repr(m.DataFrame({"id": [1], "a": [2], "b": [3]}).columns.union(["c"])),
    "union sort False": lambda m: repr(m.Index(["c", "a", "b"]).union(["d"], sort=False)),
    "union equal unsorted": lambda m: repr(m.Index(["c", "a", "b"]).union(m.Index(["c", "a", "b"]))),
    "union empty unsorted": lambda m: repr(m.Index(["c", "a", "b"]).union([])),
    "intersection keeps order": lambda m: repr(m.Index(["c", "a", "b"]).intersection(["b", "c"])),
    "symmetric_difference": lambda m: repr(m.Index(["c", "a", "b"]).symmetric_difference(["z", "a"])),
    "get_indexer list": lambda m: m.Index(["id", "a", "b"]).get_indexer(["b", "q"]).tolist(),
    "get_indexer Series": lambda m: m.Index([10, 20, 30]).get_indexer(m.Series([20, 40])).tolist(),
    "equals": lambda m: (m.Index([1, 2]).equals(m.Index([1, 2], name="x")), m.Index([1, 2]).equals([1, 2])),
    "datetime index name writes through": lambda m: (lambda d: (setattr(d.index, "name", "date"), d.index.name, list(d.reset_index().columns))[1:])(
        m.DataFrame({"v": [1, 2]}, index=m.to_datetime(["2024-01-01", "2024-01-02"]))
    ),
    "timedelta index name writes through": lambda m: (lambda s: (setattr(s.index, "name", "lag"), s.index.name)[1])(
        m.Series([1, 2], index=m.to_timedelta(["1D", "2D"]))
    ),
    # NEGATIVE: a plain index still writes through.
    "plain index name writes through": lambda m: (lambda d: (setattr(d.index, "name", "row"), list(d.reset_index().columns))[1])(m.DataFrame({"v": [1, 2]})),
}


def _slice_and_index_outcome(m: Any, case: str) -> Any:
    try:
        return _SLICE_AND_INDEX_CASES[case](m)
    except Exception as e:  # noqa: BLE001 - the exception type is the outcome
        return ("raise", type(e).__name__)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_SLICE_AND_INDEX_CASES))
def test_stepped_and_label_slices_and_index_basics_match_pandas(case: str) -> None:
    assert _slice_and_index_outcome(fpd, case) == _slice_and_index_outcome(pd, case), case


def _dated(m: Any) -> Any:
    return m.Series([1, 2], index=m.to_datetime(["2024-01-01", "2024-01-02"]), name="x")


def _dtype_values(r: Any) -> Any:
    """dtype and values, NaN and pd.NA as None (frankenpandas' tolist gives
    None for a nullable dtype's NA where pandas gives pd.NA - a separate,
    known divergence; these cases pin the dtype)."""
    missing = lambda v: v is None or (isinstance(v, float) and math.isnan(v)) or type(v).__name__ == "NAType"  # noqa: E731
    return (str(r.dtype), [None if missing(v) else v for v in r.tolist()])


# DataFrame construction kept int64 with a missing value (dict of lists /
# tuples / iterables, records, list rows, a flat list) where pandas - and
# the binding's own Series constructor - give float64 with NaN (DISC-011);
# value_counts(bins=) raised (and dropna sat in its 4th positional slot);
# shift(freq=) was refused; pd.cut with an integer bin count skipped the
# 0.1% range widening outside the default path, so right=False dropped the
# maximum to NaN and include_lowest printed (0.999, ...] for pandas'
# (0.995, ...]; a constant series printed (5.0, 5.0]; labels used a fixed 3
# digits where pandas raises the precision until the edges differ.
_CONSTRUCT_BIN_SHIFT_CASES = {
    "frame dict none int": lambda m: _dtype_values(m.DataFrame({"a": [None, 1]})["a"]),
    "frame dict int none": lambda m: _dtype_values(m.DataFrame({"a": [1, None]})["a"]),
    "frame dict tuple": lambda m: _dtype_values(m.DataFrame({"a": (1, None)})["a"]),
    "frame dict generator": lambda m: _dtype_values(m.DataFrame({"a": (x for x in [1, None])})["a"]),
    "frame records": lambda m: _dtype_values(m.DataFrame([{"a": 1}, {"a": None}])["a"]),
    "frame records missing key": lambda m: _dtype_values(m.DataFrame([{"a": 1}, {"b": 2}])["a"]),
    "frame list rows": lambda m: _dtype_values(m.DataFrame([[1, None], [2, 3]], columns=["x", "y"])["y"]),
    "frame flat list": lambda m: _dtype_values(m.DataFrame([1, None]).iloc[:, 0]),
    "frame dropna how all": lambda m: _dtype_values(m.DataFrame({"a": [None, 1], "b": [None, None]}).dropna(how="all")["a"]),
    # NEGATIVE: no missing value stays int64; bool / str with None stay object.
    "frame no missing": lambda m: _dtype_values(m.DataFrame({"a": [1, 2]})["a"]),
    "frame bool none": lambda m: _dtype_values(m.DataFrame({"a": [True, None]})["a"]),
    "frame str none": lambda m: _dtype_values(m.DataFrame({"a": ["x", None]})["a"]),
    "value_counts bins": lambda m: (lambda r: ([str(i) for i in r.index], r.tolist(), r.name))(m.Series([3, 1, 4, 1, 5]).value_counts(bins=2)),
    "value_counts bins edges": lambda m: (lambda r: ([str(i) for i in r.index], r.tolist()))(m.Series([3, 1, 4, 1, 5]).value_counts(bins=[0, 2, 10], sort=False)),
    "value_counts bins normalize": lambda m: (lambda r: ([str(i) for i in r.index], r.tolist()))(m.Series([3, 1, 4, 1, 5]).value_counts(bins=3, normalize=True)),
    "shift freq hours": lambda m: ([str(t) for t in _dated(m).shift(2, freq="h").index], _dated(m).shift(2, freq="h").tolist(), _dated(m).shift(1, freq="D").name),
    "shift freq negative": lambda m: [str(t) for t in _dated(m).shift(-1, freq="30min").index],
    "frame shift freq": lambda m: (lambda d: ([str(t) for t in d.index], d["v"].tolist()))(m.DataFrame({"v": [1, 2]}, index=m.to_datetime(["2024-01-01", "2024-01-02"])).shift(1, freq="D")),
    "timedelta index shift freq": lambda m: [str(t) for t in m.Series([5, 6], index=m.to_timedelta(["1h", "2h"])).shift(1, freq="h").index],
    # NEGATIVE: freq on a non-temporal index is pandas' NotImplementedError.
    "shift freq int index raises": lambda m: m.Series([1, 2]).shift(1, freq="D"),
    "cut include_lowest int bins": lambda m: m.cut(m.Series([3, 1, 4, 1, 5]), 2, include_lowest=True).astype(str).tolist(),
    "cut right False keeps max": lambda m: m.cut(m.Series([3, 1, 4, 1, 5]), 3, right=False).astype(str).tolist(),
    "cut constant": lambda m: m.cut(m.Series([5, 5, 5]), 2).astype(str).tolist(),
    "cut constant right False": lambda m: m.cut(m.Series([0.5, 0.5]), 2, right=False).astype(str).tolist(),
    "cut constant zero": lambda m: m.cut(m.Series([0.0, 0.0]), 2).astype(str).tolist(),
    "cut int edges include_lowest": lambda m: m.cut(m.Series([0, 5, 7]), [0, 5, 10], include_lowest=True).astype(str).tolist(),
    "cut precision grows": lambda m: m.cut(m.Series([1.0001, 1.0002, 1.0003]), 2).astype(str).tolist(),
    "qcut precision grows": lambda m: m.qcut(m.Series([1.0001, 1.0002, 1.0003, 1.0004]), 2).astype(str).tolist(),
    "qcut quartiles": lambda m: m.qcut(m.Series(range(10)), 4).astype(str).tolist(),
    "cut small fractions": lambda m: m.cut(m.Series([0.001, 0.002, 0.004]), 2).astype(str).tolist(),
    # shift filled with NaN: int64 -> float64 (it stayed int64 with a null),
    # a numeric column shifted wholly out -> float64 (object), the nullable
    # dtypes keep theirs (int64 / float64 / object), groupby shift and a
    # vacated axis=1 column the same.
    "shift int": lambda m: _dtype_values(m.Series([1, 2, 3]).shift(1)),
    "shift int back": lambda m: _dtype_values(m.Series([1, 2]).shift(-1)),
    "shift int all out": lambda m: _dtype_values(m.Series([1, 2]).shift(5)),
    "shift Int64": lambda m: _dtype_values(m.Series([1, 2], dtype="Int64").shift(1)),
    "shift Float64": lambda m: _dtype_values(m.Series([1.5, 2.0], dtype="Float64").shift(1)),
    "shift boolean": lambda m: _dtype_values(m.Series([True, False], dtype="boolean").shift(1)),
    "frame shift int": lambda m: _dtype_values(m.DataFrame({"a": [1, 2]}).shift(1)["a"]),
    "groupby shift int": lambda m: _dtype_values(m.DataFrame({"g": ["x", "x", "y"], "v": [1, 2, 3]}).groupby("g")["v"].shift(1)),
    "frame groupby shift int": lambda m: _dtype_values(m.DataFrame({"g": ["x", "x", "y"], "v": [1, 2, 3]}).groupby("g").shift(1)["v"]),
    "shift axis1 vacated": lambda m: (lambda r: ([str(d) for d in r.dtypes], [[None if isinstance(v, float) and math.isnan(v) else v for v in row] for row in r.values.tolist()]))(
        m.DataFrame({"a": [1, 2], "b": [3, 4], "c": [5, 6]}).shift(-1, axis=1)
    ),
    "shift axis1 object edge": lambda m: [str(d) for d in m.DataFrame({"s": ["x", "y"], "a": [1, 2]}).shift(1, axis=1).dtypes],
    # NEGATIVE: no missing introduced keeps int64; a fill value keeps int64; bool stays object.
    "shift int zero": lambda m: _dtype_values(m.Series([1, 2]).shift(0)),
    "shift int fill": lambda m: _dtype_values(m.Series([1, 2]).shift(1, fill_value=0)),
    "shift bool": lambda m: _dtype_values(m.Series([True, False, True]).shift(1)),
}


def _construct_bin_shift_outcome(m: Any, case: str) -> Any:
    with warnings.catch_warnings():
        warnings.simplefilter("ignore", FutureWarning)
        try:
            return _CONSTRUCT_BIN_SHIFT_CASES[case](m)
        except Exception as e:  # noqa: BLE001 - the exception type is the outcome
            return ("raise", type(e).__name__)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_CONSTRUCT_BIN_SHIFT_CASES))
def test_null_int_construction_value_count_bins_shift_freq_and_cut_edges_match_pandas(case: str) -> None:
    assert _construct_bin_shift_outcome(fpd, case) == _construct_bin_shift_outcome(pd, case), case


def _csv_frame(r: Any) -> Any:
    return (
        [str(c) for c in r.columns],
        [str(d) for d in r.dtypes],
        [[None if isinstance(v, float) and math.isnan(v) else v for v in r[c].tolist()] for c in r.columns],
    )


def _nth_frame(m: Any) -> Any:
    return m.DataFrame({"g": ["a", "a", "b"], "v": [1, 2, 3], "w": [4, 5, 6]}, index=[10, 20, 30])


# read_csv refused every parser option fp-io's CsvReadOptions already
# implements (thousands=',' raised NotImplementedError); fp-io's comment=
# only skipped lines that START with it ('2 # note' stayed text); a too-long
# row / unterminated quote raised ValueError where pandas raises its
# ParserError (a ValueError); DataFrameGroupBy.nth dropped the key column
# (pandas 2.x nth is a row filter keeping every column).
_READ_CSV_NTH_CASES = {
    "thousands": lambda m: _csv_frame(m.read_csv(io.StringIO('a,b\n"1,234",x\n5,y\n'), thousands=",")),
    "thousands float": lambda m: _csv_frame(m.read_csv(io.StringIO('a\n"1,234.5"\n2\n'), thousands=",")),
    "decimal comma": lambda m: _csv_frame(m.read_csv(io.StringIO("a;b\n1,5;2\n3,25;4\n"), sep=";", decimal=",")),
    "decimal and thousands": lambda m: _csv_frame(m.read_csv(io.StringIO("a;b\n1.234,5;2\n"), sep=";", decimal=",", thousands=".")),
    "comment inline": lambda m: _csv_frame(m.read_csv(io.StringIO("a,b\n1,2 # note\n# whole line\n3,4\n"), comment="#")),
    "comment quoted": lambda m: _csv_frame(m.read_csv(io.StringIO('a,b\n"x#y",2\n'), comment="#")),
    "comment before header": lambda m: _csv_frame(m.read_csv(io.StringIO("# top\na,b\n1,2\n"), comment="#")),
    "comment crlf": lambda m: _csv_frame(m.read_csv(io.StringIO("a,b\r\n# c\r\n1,2 #x\r\n"), comment="#")),
    "true false values": lambda m: _csv_frame(m.read_csv(io.StringIO("a\nyes\nno\nyes\n"), true_values=["yes"], false_values=["no"])),
    "na_filter false": lambda m: _csv_frame(m.read_csv(io.StringIO("a,b\n1,\n2,NA\n"), na_filter=False)),
    "skipfooter": lambda m: _csv_frame(m.read_csv(io.StringIO("a\n1\n2\ntotal\n"), skipfooter=1, engine="python")),
    "quotechar": lambda m: _csv_frame(m.read_csv(io.StringIO("a,b\n'x,y',2\n"), quotechar="'")),
    "escapechar": lambda m: _csv_frame(m.read_csv(io.StringIO('a,b\n"x\\"y",2\n'), escapechar="\\", doublequote=False)),
    "skipinitialspace": lambda m: _csv_frame(m.read_csv(io.StringIO("a, b\n1, 2\n"), skipinitialspace=True)),
    "on_bad_lines skip": lambda m: _csv_frame(m.read_csv(io.StringIO("a,b\n1,2\n3,4,5\n6,7\n"), on_bad_lines="skip")),
    "lineterminator": lambda m: _csv_frame(m.read_csv(io.StringIO("a,b~1,2~3,4"), lineterminator="~")),
    # NEGATIVES: a bad row / unterminated quote is ParserError; a 2-char marker is ValueError.
    "on_bad_lines error": lambda m: m.read_csv(io.StringIO("a,b\n1,2\n3,4,5\n"), on_bad_lines="error"),
    "unterminated quote": lambda m: m.read_csv(io.StringIO('a,b\n"x,2\n')),
    "thousands two chars": lambda m: m.read_csv(io.StringIO("a\n1\n"), thousands=",,"),
    "no options": lambda m: _csv_frame(m.read_csv(io.StringIO("a,b\n1,x\n"))),
    # Repeated header names are renamed a, a.1, ... skipping a suffix the
    # header already holds (the read raised DuplicateColumnName; 4qg5w.21).
    "duplicate headers": lambda m: _csv_frame(m.read_csv(io.StringIO("a,a,a.1,,b,b\n1,2,3,4,5,6\n"))),
    "repeated columns round trip": lambda m: _csv_frame(m.read_csv(io.StringIO(pd.DataFrame([[1, 2, 3]], columns=["x", "x", "y"]).to_csv(index=False)))),
    # converters= (it was refused): the function gets each cell's RAW text,
    # empty and 'NA' included; the dtype is inferred from its results.
    "converters raw text": lambda m: _csv_frame(m.read_csv(io.StringIO("a,b\n1,\n2,y\n3,NA\n"), converters={"b": str.upper})),
    "converters typed result": lambda m: _csv_frame(m.read_csv(io.StringIO("a,b\n1,5\n2,7\n"), converters={"b": lambda s: int(s) * 10})),
    "converters positional key": lambda m: _csv_frame(m.read_csv(io.StringIO("a,b\n1,5\n2,7\n"), converters={1: lambda s: float(s) / 2})),
    "converters keep leading zeros": lambda m: _csv_frame(m.read_csv(io.StringIO("a,b\n01,5\n"), converters={"a": str})),
    "converters unknown column ignored": lambda m: _csv_frame(m.read_csv(io.StringIO("a,b\n1,5\n"), converters={"z": str})),
    # parse_dates=True parses the index (it was refused); a text index that
    # does not read as dates stays as it is.
    "parse_dates index": lambda m: (lambda r: ([str(i) for i in r.index], str(r.index.dtype), _csv_frame(r)))(m.read_csv(io.StringIO("k,v\n2024-01-02,1\n2024-01-03,2\n"), index_col=0, parse_dates=True)),
    "parse_dates text index stays": lambda m: (lambda r: ([str(i) for i in r.index], _csv_frame(r)))(m.read_csv(io.StringIO("k,v\nx,1\ny,2\n"), index_col=0, parse_dates=True)),
    "nth first": lambda m: (lambda r: (list(r.columns), list(r.index), r.values.tolist()))(_nth_frame(m).groupby("g").nth(0)),
    "nth last": lambda m: (lambda r: (list(r.columns), list(r.index), r.values.tolist()))(_nth_frame(m).groupby("g").nth(-1)),
    "nth list": lambda m: (lambda r: (list(r.columns), list(r.index), r.values.tolist()))(_nth_frame(m).groupby("g").nth([0, 1])),
    "nth two keys": lambda m: (lambda r: (list(r.columns), list(r.index)))(_nth_frame(m).groupby(["g", "w"]).nth(0)),
    "nth array key": lambda m: (lambda r: (list(r.columns), list(r.index)))(_nth_frame(m).groupby(np.array(["x", "y", "x"])).nth(0)),
    # NEGATIVE: a position past every group is an empty frame with the columns.
    "nth out of range": lambda m: (lambda r: (list(r.columns), list(r.index)))(_nth_frame(m).groupby("g").nth(5)),
}


def _read_csv_nth_outcome(m: Any, case: str) -> Any:
    try:
        return _READ_CSV_NTH_CASES[case](m)
    except Exception as e:  # noqa: BLE001 - the exception type is the outcome
        return ("raise", type(e).__name__)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_READ_CSV_NTH_CASES))
def test_read_csv_parser_options_and_groupby_nth_match_pandas(case: str) -> None:
    assert _read_csv_nth_outcome(fpd, case) == _read_csv_nth_outcome(pd, case), case


def _shaped(r: Any, digits: int | None = None) -> Any:
    """A frame or Series as its kind, name, index (names and labels),
    columns and values, NaN as None; floats to `digits` places when given
    (a moving correlation's last bits depend on summation order)."""
    plain = lambda v: None if isinstance(v, float) and math.isnan(v) else (round(v, digits) if digits is not None and isinstance(v, float) else v)  # noqa: E731
    index = (list(r.index.names), [str(i) for i in r.index])
    if hasattr(r, "columns"):
        return ("frame", index, [str(c) for c in r.columns], [[plain(v) for v in r[c].tolist()] for c in r.columns])
    return ("series", r.name, index, [plain(v) for v in r.tolist()])


def _apply_frame(m: Any) -> Any:
    return m.DataFrame({"g": ["b", "a", "b"], "v": [1, 2, 3], "w": [4.0, 5.0, 6.0]})


def _five(m: Any) -> Any:
    return m.DataFrame({"a": [1.0, 2.0, 3.0, 4.0, 5.0], "b": [2.0, 1.0, 5.0, 3.0, 8.0]})


# Finished code the binding refused or never reached, and results that were
# silently wrong:
# - DataFrame.rolling(center=True) was refused although fp-frame's Series
#   windows centre; Series rolling rank / agg([...]) / apply / corr / cov
#   ignored center, and apply counted NaN rows toward min_periods;
# - reindex(method='ffill'/'bfill') carried the previous TARGET row's value
#   instead of searching the source index;
# - DataFrame.from_dict(orient='index'/'tight') and set_index(append=True)
#   were refused (a missing key raised ValueError, pandas' KeyError);
# - to_datetime(dayfirst=True) was refused, a first value only readable
#   day-first (13/02/2024) raised, and format='...%S.%f' read '.5' as 5 ns;
# - groupby.apply concatenated without the group keys, handed func the key
#   columns whatever include_groups said, and dropped None results;
#   group_keys=False and dropna=False (Series groupby) were refused;
#   groupby.transform(func) returned rows in group order with the keys.
_WIRE_CASES = {
    "frame rolling center mean": lambda m: _shaped(_five(m).rolling(3, center=True).mean()),
    "frame rolling center even sum": lambda m: _shaped(_five(m).rolling(2, center=True).sum()),
    "frame rolling center min_periods": lambda m: _shaped(_five(m).rolling(4, center=True, min_periods=1).max()),
    "frame rolling center agg": lambda m: _shaped(_five(m).rolling(3, center=True).agg(["sum", "min"])),
    "series rolling center rank": lambda m: _shaped(m.Series([3.0, 1.0, 4.0, 1.0, 5.0]).rolling(3, center=True).rank()),
    "series rolling center agg list": lambda m: _shaped(m.Series([3.0, 1.0, 4.0, 1.0, 5.0]).rolling(3, center=True).agg(["sum", "max"])),
    "series rolling center apply": lambda m: _shaped(m.Series([1.0, 2.0, 3.0, 4.0, 5.0]).rolling(3, center=True, min_periods=1).apply(lambda x: len(x))),
    "series rolling apply nan min_periods": lambda m: _shaped(m.Series([1.0, float("nan"), 3.0, 4.0, 5.0]).rolling(2).apply(lambda x: x.sum())),
    "series rolling center corr": lambda m: _shaped(_five(m)["a"].rolling(3, center=True, min_periods=2).corr(_five(m)["b"]), 10),
    "frame rolling center cov series": lambda m: _shaped(_five(m).rolling(3, center=True).cov(m.Series([2.0, 1.0, 5.0, 3.0, 8.0])), 10),
    # pandas names a moving corr / cov only when both Series share the name.
    "rolling corr names differ": lambda m: m.Series([1.0, 2.0, 4.0], name="a").rolling(2).corr(m.Series([2.0, 1.0, 5.0], name="b")).name,
    "expanding cov same name": lambda m: m.Series([1.0, 2.0, 4.0], name="a").expanding().cov(m.Series([2.0, 1.0, 5.0], name="a")).name,
    "rolling rank center edges": lambda m: _shaped(m.Series([3.0, 1.0, 4.0, 1.0, 5.0, 9.0, 2.0]).rolling(4, center=True, min_periods=1).rank(method="max")),
    # NEGATIVE: pandas' rolling rank takes only average / min / max.
    "rolling rank first raises": lambda m: m.Series([1.0, 2.0]).rolling(2).rank(method="first"),
    "expanding agg list": lambda m: _shaped(_five(m).expanding().agg(["sum", "max"])),
    "frame rolling center apply raw": lambda m: _shaped(_five(m).rolling(3, center=True).apply(lambda x: x[0], raw=True)),
    # NEGATIVE: center=False keeps the trailing windows.
    "frame rolling trailing": lambda m: _shaped(_five(m).rolling(3).mean()),
    "reindex ffill": lambda m: _shaped(m.Series([1.0, 2.0, 3.0], index=[0, 2, 4]).reindex([5, 1, 3, 0, -1], method="ffill")),
    "reindex bfill": lambda m: _shaped(m.Series([1.0, 2.0, 3.0], index=[0, 2, 4]).reindex([5, 1, 3, 0, -1], method="bfill")),
    "reindex ffill decreasing": lambda m: _shaped(m.Series([1.0, 2.0, 3.0], index=[4, 2, 0]).reindex([5, 3, 1], method="ffill")),
    "frame reindex bfill": lambda m: _shaped(m.DataFrame({"v": [1.0, 2.0, 3.0]}, index=[0, 2, 4]).reindex([0, 1, 3], method="bfill")),
    "reindex pad index target": lambda m: _shaped(m.Series([1.0, 2.0], index=[0, 10]).reindex(m.Index([3, 12]), method="pad")),
    # NEGATIVE: a source that is not monotonic is pandas' ValueError.
    "reindex ffill unsorted source": lambda m: m.Series([1.0, 2.0, 3.0], index=[2, 0, 4]).reindex([1], method="ffill"),
    "from_dict index": lambda m: _shaped(m.DataFrame.from_dict({"r1": [1, 2], "r2": [3, 4]}, orient="index")),
    "from_dict index columns": lambda m: _shaped(m.DataFrame.from_dict({"r1": [1, 2], "r2": [3, 4]}, orient="index", columns=["a", "b"])),
    "from_dict index of dicts": lambda m: _shaped(m.DataFrame.from_dict({"r1": {"a": 1, "b": 2}, "r2": {"a": 3}}, orient="index")),
    "from_dict tight": lambda m: _shaped(m.DataFrame.from_dict({"index": ["x", "y"], "columns": ["a"], "data": [[1], [2]], "index_names": ["k"], "column_names": [None]}, orient="tight")),
    # NEGATIVE: an unknown orient is pandas' ValueError.
    "from_dict bad orient": lambda m: m.DataFrame.from_dict({"a": [1]}, orient="rows"),
    "set_index append": lambda m: _shaped(m.DataFrame({"a": [1, 2], "b": ["x", "y"]}).set_index("b", append=True)),
    "set_index append twice": lambda m: _shaped(m.DataFrame({"a": [1, 2], "b": ["x", "y"], "c": [5, 6]}).set_index("b", append=True).set_index("c", append=True)),
    "set_index append keep": lambda m: _shaped(m.DataFrame({"a": [1, 2], "b": ["x", "y"]}, index=m.Index([7, 8], name="k")).set_index(["b"], append=True, drop=False)),
    # NEGATIVE: a key that is not a column is pandas' KeyError (it was ValueError).
    "set_index missing": lambda m: m.DataFrame({"a": [1]}).set_index("zz"),
    "set_index append missing": lambda m: m.DataFrame({"a": [1]}).set_index(["a", "zz"], append=True),
    "dayfirst slash": lambda m: [str(t) for t in m.to_datetime(m.Series(["01/02/2024", "03/04/2024"]), dayfirst=True)],
    "dayfirst dash time": lambda m: [str(t) for t in m.to_datetime(m.Series(["01-02-2024 10:30", "05-06-2024 11:45"]), dayfirst=True)],
    "dayfirst dot fraction": lambda m: [str(t) for t in m.to_datetime(m.Series(["01.02.2024 10:30:15.5"]), dayfirst=True)],
    "dayfirst iso": lambda m: [str(t) for t in m.to_datetime(m.Series(["2024-01-02", "2024-03-04"]), dayfirst=True)],
    "dayfirst list": lambda m: [str(t) for t in m.to_datetime(["01/02/2024", "03/04/2024"], dayfirst=True)],
    "dayfirst coerce": lambda m: [str(t) for t in m.to_datetime(m.Series(["01/02/2024", "2024-03-04", "31/12/2024"]), dayfirst=True, errors="coerce")],
    "dayfirst null first": lambda m: [str(t) for t in m.to_datetime(m.Series([None, "01/02/2024"]), dayfirst=True)],
    "day over twelve": lambda m: [str(t) for t in m.to_datetime(m.Series(["13/02/2024", "01/03/2024"]))],
    "month over twelve dayfirst": lambda m: [str(t) for t in m.to_datetime(m.Series(["01/13/2024", "02/14/2024"]), dayfirst=True)],
    "format fraction": lambda m: [str(t) for t in m.to_datetime(m.Series(["01/02/2024 10:30:15.25"]), format="%d/%m/%Y %H:%M:%S.%f")],
    # Month-first dates with a time, '-' / '.' separators or AM/PM raised.
    "monthfirst time": lambda m: [str(t) for t in m.to_datetime(m.Series(["01/02/2024 10:30", "03/04/2024 11:15"]))],
    "monthfirst fraction": lambda m: [str(t) for t in m.to_datetime(m.Series(["01/02/2024 10:30:15.5"]))],
    "monthfirst dash dot": lambda m: [[str(t) for t in m.to_datetime(m.Series([v]))] for v in ("01-02-2024", "01.02.2024")],
    "monthfirst pm": lambda m: [str(t) for t in m.to_datetime(m.Series(["01/02/2024 10:30 PM"]))],
    "scalar day over twelve": lambda m: str(m.to_datetime("13/02/2024")),
    # NEGATIVES: month-first stays the default; a row off the guessed format raises.
    "monthfirst default": lambda m: [str(t) for t in m.to_datetime(m.Series(["01/02/2024", "03/04/2024"]))],
    "dayfirst mismatch raises": lambda m: m.to_datetime(m.Series(["01/02/2024", "2024-03-04"]), dayfirst=True),
    "apply frame keyed": lambda m: _shaped(_apply_frame(m).groupby("g").apply(lambda d: d * 1, include_groups=False)),
    "apply frame group_keys false": lambda m: _shaped(_apply_frame(m).groupby("g", group_keys=False).apply(lambda d: d * 1, include_groups=False)),
    "apply head keyed": lambda m: _shaped(_apply_frame(m).groupby("g").apply(lambda d: d.head(1), include_groups=False)),
    "apply head group_keys false": lambda m: _shaped(_apply_frame(m).groupby("g", group_keys=False).apply(lambda d: d.head(1), include_groups=False)),
    "apply scalar": lambda m: _shaped(_apply_frame(m).groupby("g").apply(lambda d: d["v"].sum(), include_groups=False)),
    "apply scalar sort false": lambda m: _shaped(_apply_frame(m).groupby("g", sort=False).apply(lambda d: d["v"].sum(), include_groups=False)),
    "apply scalar none": lambda m: _shaped(_apply_frame(m).groupby("g").apply(lambda d: None if len(d) == 1 else d["v"].sum(), include_groups=False)),
    "apply frame none": lambda m: _shaped(_apply_frame(m).groupby("g").apply(lambda d: None if len(d) == 1 else d.head(1), include_groups=False)),
    "apply series stacked": lambda m: _shaped(_apply_frame(m).groupby("g").apply(lambda d: d.sum(), include_groups=False)),
    "apply series varying": lambda m: _shaped(_apply_frame(m).groupby("g").apply(lambda d: d["v"] * 2, include_groups=False)),
    "apply series varying group_keys false": lambda m: _shaped(_apply_frame(m).groupby("g", group_keys=False).apply(lambda d: d["v"] * 2, include_groups=False)),
    "apply agg shaped frame": lambda m: _shaped(_apply_frame(m).groupby("g").apply(lambda d: m.DataFrame({"s": [d["v"].sum()]}), include_groups=False)),
    "apply two keys scalar": lambda m: _shaped(_apply_frame(m).groupby(["g", "v"]).apply(lambda d: d["w"].sum(), include_groups=False)),
    "apply two keys frame": lambda m: _shaped(_apply_frame(m).groupby(["g", "v"]).apply(lambda d: d * 2, include_groups=False)),
    "apply as_index false frame": lambda m: _shaped(_apply_frame(m).groupby("g", as_index=False).apply(lambda d: d.head(1), include_groups=False)),
    "apply as_index false stacked": lambda m: _shaped(_apply_frame(m).groupby("g", as_index=False).apply(lambda d: d.sum(), include_groups=False)),
    "apply array key": lambda m: _shaped(_apply_frame(m).groupby(np.array(["x", "y", "x"])).apply(lambda d: d["v"].sum())),
    "apply include_groups default": lambda m: _shaped(_apply_frame(m).groupby("g").apply(lambda d: d.head(1))),
    "sgb apply scalar": lambda m: _shaped(_apply_frame(m).groupby("g")["v"].apply(lambda s: s.sum())),
    "sgb apply series keyed": lambda m: _shaped(_apply_frame(m).groupby("g")["v"].apply(lambda s: s * 2)),
    "sgb apply series group_keys false": lambda m: _shaped(_apply_frame(m).groupby("g", group_keys=False)["v"].apply(lambda s: s * 2)),
    "Series groupby apply group_keys false": lambda m: _shaped(m.Series([1, 2, 3]).groupby([0, 1, 0], group_keys=False).apply(lambda s: s * 2)),
    "transform func": lambda m: _shaped(_apply_frame(m).groupby("g").transform(lambda d: d - d.min())),
    "dropna false column": lambda m: _shaped(m.DataFrame({"g": ["a", None, "a"], "v": [1, 2, 3]}).groupby("g", dropna=False)["v"].sum()),
    "dropna false Series": lambda m: _shaped(m.Series([1, 2, 3], index=["x", "y", "z"]).groupby(m.Series(["a", None, "a"], index=["x", "y", "z"]), dropna=False).sum()),
    "dropna false mean unsorted": lambda m: _shaped(m.Series([1.0, 2.0, 3.0, 4.0]).groupby(["b", None, "a", "b"], dropna=False, sort=False).mean()),
    # NEGATIVE: dropna=True (the default) drops the missing key's rows.
    "dropna true Series": lambda m: _shaped(m.Series([1, 2, 3]).groupby(["a", None, "a"]).sum()),
    "concat keys multiindex pieces": lambda m: _shaped(m.concat([m.DataFrame({"v": [1]}, index=m.MultiIndex.from_tuples([("a", 1)])), m.DataFrame({"v": [2]}, index=m.MultiIndex.from_tuples([("b", 2)]))], keys=["x", "y"])),
}


def _wire_outcome(m: Any, case: str) -> Any:
    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")
        try:
            result = _WIRE_CASES[case](m)
        except Exception as e:  # noqa: BLE001 - the exception type is the outcome
            result = ("raise", type(e).__name__)
    return result, sorted({w.category.__name__ for w in caught if w.category in (UserWarning, DeprecationWarning)})


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_WIRE_CASES))
def test_centered_windows_reindex_fill_dayfirst_and_groupby_apply_match_pandas(case: str) -> None:
    assert _wire_outcome(fpd, case) == _wire_outcome(pd, case), case


def _typed(r: Any) -> Any:
    """`_shaped` plus the dtypes."""
    dtypes = [str(d) for d in r.dtypes] if hasattr(r, "columns") else str(r.dtype)
    return (_shaped(r), dtypes)


def _xy(m: Any) -> Any:
    return m.DataFrame({"x": [1, 2], "y": [3, 4]}, index=["r", "s"])


# DataFrame.apply modelled only scalar and Series results: a list / tuple /
# dict result came back as a bare Python list, axis=1 dicts were expanded,
# raw= and result_type= were refused, args= reached func as a keyword and a
# string / list / dict func raised TypeError; transform accepted reductions.
# The DataFrame constructor kept a dict of differently indexed Series in
# first-seen order (pandas sorts the union) and made an empty list object.
_FRAME_APPLY_CASES = {
    "axis0 scalar": lambda m: _typed(_xy(m).apply(lambda c: c.sum())),
    "axis0 list same length": lambda m: _typed(_xy(m).apply(lambda c: [1, 2])),
    "axis0 list other length": lambda m: _typed(_xy(m).apply(lambda c: [1, 2, 3])),
    "axis0 tuple": lambda m: _typed(_xy(m).apply(lambda c: (c.sum(), c.max()))),
    "axis0 series": lambda m: _typed(_xy(m).apply(lambda c: c * 2)),
    "axis0 series other index": lambda m: _typed(_xy(m).apply(lambda c: m.Series({"a": c.sum(), "b": c.max()}))),
    "axis0 series union sorted": lambda m: _typed(_xy(m).apply(lambda c: m.Series({"b": c.sum()}) if c.name == "x" else m.Series({"a": c.max()}))),
    "axis1 scalar": lambda m: _typed(_xy(m).apply(lambda r: r.sum(), axis=1)),
    "axis1 row name": lambda m: _xy(m).apply(lambda r: r.name, axis=1).tolist(),
    "axis1 series": lambda m: _typed(_xy(m).apply(lambda r: m.Series({"a": r["x"], "b": r["y"]}), axis=1)),
    "axis1 series union": lambda m: _typed(_xy(m).apply(lambda r: m.Series({"b": r["x"]}) if r.name == "r" else m.Series({"a": r["y"]}), axis=1)),
    "axis1 expand list": lambda m: _typed(_xy(m).apply(lambda r: [r["x"], r["y"] * 2], axis=1, result_type="expand")),
    "axis1 expand dict": lambda m: _typed(_xy(m).apply(lambda r: {"a": r["x"]}, axis=1, result_type="expand")),
    "axis1 expand dict union": lambda m: _typed(_xy(m).apply(lambda r: {"b": r["x"]} if r.name == "r" else {"a": r["y"], "b": 0}, axis=1, result_type="expand")),
    "axis1 expand scalar": lambda m: _typed(_xy(m).apply(lambda r: r.sum(), axis=1, result_type="expand")),
    "axis1 reduce series": lambda m: _typed(_xy(m).apply(lambda r: r * 2, axis=1, result_type="reduce")),
    "axis0 expand list": lambda m: _typed(_xy(m).apply(lambda c: [1, 2, 3], result_type="expand")),
    "broadcast axis1 list": lambda m: _typed(_xy(m).apply(lambda r: [r["x"], r["y"] * 2], axis=1, result_type="broadcast")),
    "broadcast axis1 scalar": lambda m: _typed(_xy(m).apply(lambda r: r.sum(), axis=1, result_type="broadcast")),
    "broadcast axis0 scalar": lambda m: _typed(_xy(m).apply(lambda c: c.sum(), result_type="broadcast")),
    "broadcast float into int": lambda m: _typed(_xy(m).apply(lambda c: c.mean(), result_type="broadcast")),
    "broadcast mixed dtypes": lambda m: _typed(m.DataFrame({"x": [1, 2], "s": ["a", "b"]}).apply(lambda c: c.iloc[0], result_type="broadcast")),
    "raw false gets series": lambda m: _typed(_xy(m).apply(lambda a: type(a).__name__ + str(a.sum()))),
    "raw axis0 ndarray": lambda m: _typed(_xy(m).apply(lambda a: type(a).__name__ + str(a.sum()), raw=True)),
    "raw axis1": lambda m: _typed(_xy(m).apply(lambda a: a[0] * 10 + a[1], axis=1, raw=True)),
    "raw array result": lambda m: _typed(_xy(m).apply(lambda a: a * 2, raw=True)),
    "raw axis1 array result": lambda m: _typed(_xy(m).apply(lambda a: a * 2, axis=1, raw=True)),
    "raw mixed int float": lambda m: _typed(m.DataFrame({"x": [1, 2], "y": [0.5, 1.5]}).apply(lambda a: a.dtype.kind, axis=1, raw=True)),
    "empty frame reduces": lambda m: _typed(m.DataFrame({"x": []}).apply(lambda c: c.sum())),
    "empty axis1": lambda m: _typed(m.DataFrame({"x": [], "y": []}).apply(lambda r: r.sum(), axis=1)),
    "empty keeps frame": lambda m: _typed(m.DataFrame({"x": []}).apply(lambda c: c * 2)),
    "args and kwargs": lambda m: _typed(_xy(m).apply(lambda c, k, z=0: c.sum() * k + z, args=(2,), z=1)),
    "string func": lambda m: _typed(_xy(m).apply("sum")),
    "string func axis1": lambda m: _typed(_xy(m).apply("sum", axis=1)),
    "list funcs": lambda m: _typed(_xy(m).apply(["sum", "max"])),
    "dict funcs": lambda m: _typed(_xy(m).apply({"x": "sum"})),
    "numpy ufunc": lambda m: _typed(_xy(m).apply(np.sqrt)),
    "numpy reduction": lambda m: _typed(_xy(m).apply(np.sum)),
    "transform lambda": lambda m: _typed(_xy(m).transform(lambda c: c * 2)),
    "transform string": lambda m: _typed(_xy(m).transform("cumsum")),
    "transform axis1": lambda m: _typed(_xy(m).transform(lambda r: r - r.min(), axis=1)),
    "frame from differently indexed series": lambda m: _typed(m.DataFrame({"x": m.Series({"b": 3, "c": 1}), "y": m.Series({"c": 4, "b": 2})})),
    "frame from same indexed series": lambda m: _typed(m.DataFrame({"x": m.Series({"b": 3, "a": 1}), "y": m.Series({"b": 4, "a": 2})})),
    "frame from empty list": lambda m: _typed(m.DataFrame({"x": []})),
    # NEGATIVES
    "empty list object dtype": lambda m: _typed(m.DataFrame({"x": []}, dtype=object)),
    "bad result_type": lambda m: _xy(m).apply(lambda c: c, result_type="bogus"),
    "broadcast wrong length": lambda m: _xy(m).apply(lambda r: [1, 2, 3], axis=1, result_type="broadcast"),
    "raw wrong length": lambda m: _xy(m).apply(lambda a: [1, 2, 3], raw=True),
    "expand ragged": lambda m: _xy(m).apply(lambda r: [1] if r.name == "r" else [1, 2], axis=1, result_type="expand"),
    "by_row bad": lambda m: _xy(m).apply(lambda c: c.sum(), by_row="x"),
    "transform reduction": lambda m: _xy(m).transform(lambda c: c.sum()),
    "transform string reduction": lambda m: _xy(m).transform("sum"),
    "series union mixed labels": lambda m: m.DataFrame({"x": m.Series({"b": 3}), "y": m.Series({"a": 4, 1: 5})}),
}


def _frame_apply_outcome(m: Any, case: str) -> Any:
    try:
        return _FRAME_APPLY_CASES[case](m)
    except Exception as e:  # noqa: BLE001 - the exception type is the outcome
        return ("raise", type(e).__name__)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_FRAME_APPLY_CASES))
def test_frame_apply_result_shapes_raw_broadcast_and_transform_match_pandas(case: str) -> None:
    assert _frame_apply_outcome(fpd, case) == _frame_apply_outcome(pd, case), case


def _ab(m: Any) -> Any:
    return m.DataFrame({"a": ["x", "y", "x", "y", "x"], "b": ["p", "p", "q", "q", "q"], "v": [1, 2, 3, 4, 5], "w": [1.5, None, 2.5, 3.5, 0.5]})


def _q_named(r: Any) -> Any:
    return (str(r.name), [str(i) for i in r.index], _dtype_values(r))


def _number(v: Any) -> Any:
    """A scalar and whether it is an integer (pandas' np.int64 vs a float)."""
    return (v, isinstance(v, (int, np.integer)))


def _records() -> Any:
    return [{"id": 1, "info": {"n": "p"}, "items": [{"k": "a", "z": {"q": 1}}, {"k": "b", "z": {"q": 2}}]}, {"id": 2, "info": {"n": "r"}, "items": [{"k": "c", "z": {"q": 3}}]}]


# Silently wrong: DataFrame.quantile ignored interpolation for a single q
# (nearest gave 2.2), gb.value_counts() counted the keys with one column as
# a frame, frame value_counts labelled rows 'x, q' (pandas' MultiIndex),
# crosstab sorted int keys as text, pivot_table sum of ints came back
# float64, sample drew other rows than pandas for the same seed. Refused:
# value_counts sort / dropna / normalize / subset, crosstab values /
# aggfunc / margins / names / normalize='index', json_normalize
# record_path / meta, merge_asof suffixes, groupby quantile interpolation,
# pivot_table sort=False, sample weights.
_VALUE_COUNTS_CASES = {
    # (Series names are text here - fvsao.32 - so the q name compares as str.)
    "frame quantile nearest": lambda m: _q_named(_ab(m)[["v", "w"]].quantile(0.3, interpolation="nearest")),
    "frame quantile lower int": lambda m: _q_named(_ab(m)[["v"]].quantile(0.3, interpolation="lower")),
    "frame quantile midpoint": lambda m: _q_named(_ab(m)[["v", "w"]].quantile(0.3, interpolation="midpoint")),
    "frame quantile higher axis1": lambda m: _q_named(_ab(m)[["v", "w"]].quantile(0.5, interpolation="higher", axis=1)),
    "frame quantile linear axis1": lambda m: _q_named(_ab(m)[["v", "w"]].quantile(0.5, axis=1)),
    "series quantile lower int": lambda m: _number(_ab(m)["v"].quantile(0.3, interpolation="lower")),
    "series quantile linear float": lambda m: _number(_ab(m)["v"].quantile(0.3)),
    "gb quantile higher": lambda m: _typed(_ab(m).groupby("a")["v"].quantile(0.5, interpolation="higher")),
    "gb frame quantile lower": lambda m: _typed(_ab(m).groupby("a")[["v", "w"]].quantile(0.5, interpolation="lower")),
    "value_counts": lambda m: _typed(_ab(m)[["a", "b"]].value_counts()),
    "value_counts sort false": lambda m: _typed(_ab(m)[["a", "b"]].value_counts(sort=False)),
    "value_counts normalize": lambda m: _typed(_ab(m)[["a", "b"]].value_counts(normalize=True)),
    "value_counts ascending": lambda m: _typed(_ab(m)[["a"]].value_counts(ascending=True)),
    "value_counts dropna false": lambda m: _typed(m.DataFrame({"a": ["x", None, "x"]}).value_counts(dropna=False)),
    "value_counts subset str": lambda m: _typed(_ab(m).value_counts(subset="a")),
    "value_counts subset list": lambda m: _typed(_ab(m).value_counts(subset=["b"])),
    "gb value_counts": lambda m: _typed(_ab(m).groupby("a").value_counts()),
    "gb value_counts normalize": lambda m: _typed(_ab(m).groupby("a").value_counts(normalize=True)),
    "gb value_counts subset": lambda m: _typed(_ab(m).groupby("a").value_counts(subset=["b"])),
    "gb value_counts sort false": lambda m: _typed(_ab(m).groupby("a").value_counts(subset=["b"], sort=False)),
    "gb value_counts ascending": lambda m: _typed(_ab(m).groupby("a").value_counts(subset=["b"], ascending=True)),
    "gb value_counts as_index false": lambda m: _typed(_ab(m).groupby("a", as_index=False).value_counts(subset=["b"])),
    "crosstab": lambda m: _typed(m.crosstab(_ab(m)["a"], _ab(m)["b"])),
    "crosstab int keys": lambda m: _typed(m.crosstab(m.Series([10, 2, 10], name="k"), m.Series([1, 1, 2], name="j"))),
    "crosstab margins": lambda m: _typed(m.crosstab(_ab(m)["a"], _ab(m)["b"], margins=True)),
    "crosstab margins_name": lambda m: _typed(m.crosstab(_ab(m)["a"], _ab(m)["b"], margins=True, margins_name="Total")),
    "crosstab values sum": lambda m: _typed(m.crosstab(_ab(m)["a"], _ab(m)["b"], values=_ab(m)["v"], aggfunc="sum")),
    "crosstab values mean margins": lambda m: _typed(m.crosstab(_ab(m)["a"], _ab(m)["b"], values=_ab(m)["v"], aggfunc="mean", margins=True)),
    "crosstab normalize": lambda m: _typed(m.crosstab(_ab(m)["a"], _ab(m)["b"], normalize=True)),
    "crosstab normalize index": lambda m: _typed(m.crosstab(_ab(m)["a"], _ab(m)["b"], normalize="index")),
    "crosstab normalize columns": lambda m: _typed(m.crosstab(_ab(m)["a"], _ab(m)["b"], normalize="columns")),
    "crosstab names": lambda m: _typed(m.crosstab(_ab(m)["a"], _ab(m)["b"], rownames=["R"], colnames=["C"])),
    "pivot int sum complete": lambda m: _typed(_ab(m).pivot_table(index="a", columns="b", values="v", aggfunc="sum")),
    "pivot int sum missing": lambda m: _typed(m.DataFrame({"a": ["x", "y"], "b": ["p", "q"], "v": [1, 2]}).pivot_table(index="a", columns="b", values="v", aggfunc="sum")),
    "pivot count": lambda m: _typed(_ab(m).pivot_table(index="a", columns="b", values="w", aggfunc="count")),
    "pivot sort false columns": lambda m: _typed(m.DataFrame({"a": ["y", "x", "y"], "b": ["q", "p", "p"], "v": [1, 2, 3]}).pivot_table(index="a", columns="b", values="v", aggfunc="sum", sort=False)),
    "pivot sort false": lambda m: _typed(m.DataFrame({"a": ["y", "x", "y"], "v": [1, 2, 3]}).pivot_table(index="a", values="v", aggfunc="sum", sort=False)),
    "json record_path meta": lambda m: _typed(m.json_normalize(_records(), record_path="items", meta=["id"])),
    "json record prefix": lambda m: _typed(m.json_normalize(_records(), record_path="items", meta=["id"], record_prefix="it.")),
    "json nested meta": lambda m: _typed(m.json_normalize(_records(), record_path="items", meta=["id", ["info", "n"]])),
    "json meta_prefix": lambda m: _typed(m.json_normalize(_records(), record_path="items", meta=["id"], meta_prefix="m_")),
    "json meta ignore": lambda m: _typed(m.json_normalize([{"items": [{"k": "a"}]}], record_path="items", meta=["id"], errors="ignore")),
    "json record_path list": lambda m: _typed(m.json_normalize({"a": {"items": [{"k": 1}]}}, record_path=["a", "items"])),
    "merge_asof suffixes": lambda m: _typed(m.merge_asof(m.DataFrame({"t": [1, 5], "v": [1, 2]}), m.DataFrame({"t": [2, 3], "v": [7, 8]}), on="t", suffixes=("_l", "_r"))),
    "sample n": lambda m: _ab(m).sample(n=3, random_state=0).index.tolist(),
    "sample frac": lambda m: _ab(m).sample(frac=0.5, random_state=2).index.tolist(),
    "sample replace": lambda m: _ab(m).sample(n=7, replace=True, random_state=4).index.tolist(),
    "sample weights list": lambda m: _ab(m).sample(n=2, weights=[0, 0, 0, 1, 1], random_state=0).index.tolist(),
    "sample weights series": lambda m: _ab(m).sample(n=2, weights=m.Series([5, 0, 0, 1, 0]), random_state=3).index.tolist(),
    "sample weights column": lambda m: _ab(m).sample(n=2, weights="v", random_state=1).index.tolist(),
    "series sample": lambda m: _ab(m)["v"].sample(n=3, random_state=5).tolist(),
    "sample axis1": lambda m: _ab(m).sample(n=2, axis=1, random_state=0).columns.tolist(),
    # NEGATIVES
    "gb value_counts subset clash": lambda m: _ab(m).groupby("a").value_counts(subset=["a"]),
    "gb value_counts subset missing": lambda m: _ab(m).groupby("a").value_counts(subset=["zz"]),
    "crosstab values without aggfunc": lambda m: m.crosstab(_ab(m)["a"], _ab(m)["b"], values=_ab(m)["v"]),
    "crosstab aggfunc without values": lambda m: m.crosstab(_ab(m)["a"], _ab(m)["b"], aggfunc="sum"),
    "crosstab bad normalize": lambda m: m.crosstab(_ab(m)["a"], _ab(m)["b"], normalize="rows"),
    "json missing meta": lambda m: m.json_normalize([{"items": [{"k": "a"}]}], record_path="items", meta=["id"]),
    "json missing record path": lambda m: m.json_normalize([{"id": 1}], record_path="items"),
    "json meta conflict": lambda m: m.json_normalize([{"k": 1, "items": [{"k": "a"}]}], record_path="items", meta=["k"]),
    "sample n and frac": lambda m: _ab(m).sample(n=1, frac=0.5),
    "sample negative weights": lambda m: _ab(m).sample(n=1, weights=[1, -1, 0, 0, 0]),
    "sample zero weights": lambda m: _ab(m).sample(n=1, weights=[0, 0, 0, 0, 0]),
    "sample weights length": lambda m: _ab(m).sample(n=1, weights=[1, 2]),
    "sample upsample without replace": lambda m: _ab(m).sample(frac=1.5),
}


def _value_counts_outcome(m: Any, case: str) -> Any:
    try:
        return _VALUE_COUNTS_CASES[case](m)
    except Exception as e:  # noqa: BLE001 - the exception type is the outcome
        return ("raise", type(e).__name__)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_VALUE_COUNTS_CASES))
def test_value_counts_crosstab_quantile_json_normalize_and_sample_match_pandas(case: str) -> None:
    assert _value_counts_outcome(fpd, case) == _value_counts_outcome(pd, case), case


def _dtype_facts(dtype: Any) -> Any:
    """What user code reads off a dtype."""
    return (str(dtype), dtype.name, dtype.kind)


# Series.dtype was the name as a str: `s.dtype == np.float64` was False and
# `.kind` / `.name` raised; Series.unstack made an all-NaN column object
# (pandas float64); df.T of ints and floats inferred each row on its own (a
# [1, NaN] row became a nullable int64 column; pandas: the common float64);
# get_dummies(dtype=int) raised TypeError (and dtype=float gave bools);
# a typed index's astype(str) raised TypeError.
_DTYPE_CASES = {
    "float dtype facts": lambda m: _dtype_facts(m.Series([1.5]).dtype),
    "float dtype is np float64": lambda m: m.Series([1.5]).dtype == np.float64,
    "int dtype is np int64": lambda m: m.Series([1]).dtype == np.int64,
    "bool dtype facts": lambda m: _dtype_facts(m.Series([True]).dtype),
    "object dtype facts": lambda m: _dtype_facts(m.Series(["x"]).dtype),
    "datetime dtype facts": lambda m: _dtype_facts(m.to_datetime(m.Series(["2024-01-01"])).dtype),
    "timedelta dtype facts": lambda m: _dtype_facts(m.to_timedelta(m.Series(["1h"])).dtype),
    "issubdtype number": lambda m: np.issubdtype(m.Series([1]).dtype, np.number),
    "nullable int dtype": lambda m: (str(m.Series([1, None], dtype="Int64").dtype), m.Series([1, None], dtype="Int64").dtype == "Int64"),
    "category dtype": lambda m: (str(m.Series(["a", "b"], dtype="category").dtype), m.Series(["a"], dtype="category").dtype == "category", m.Series(["a"], dtype="category").dtype.name),
    "dtype equals name": lambda m: (m.Series([1.5]).dtype == "float64", m.Series([1]).dtype == "int64"),
    # NEGATIVE: a float column is not int64, nor object.
    "float dtype is not int64": lambda m: (m.Series([1.5]).dtype == np.int64, m.Series([1.5]).dtype == object),
    "unstack all-missing column": lambda m: _typed(m.DataFrame({"k": ["a", "b", "a"], "x": [1, 2, 3], "y": [1.5, None, 2.5]}).set_index(["k", "x"])["y"].unstack()),
    "unstack int with gaps": lambda m: _typed(m.DataFrame({"k": ["a", "b"], "x": [1, 2], "v": [5, 6]}).set_index(["k", "x"])["v"].unstack()),
    # NEGATIVE: an int unstack with every cell present stays int64.
    "unstack int complete": lambda m: _typed(m.DataFrame({"k": ["a", "a", "b", "b"], "x": [1, 2, 1, 2], "v": [5, 6, 7, 8]}).set_index(["k", "x"])["v"].unstack()),
    "transpose int float": lambda m: _typed(m.DataFrame({"x": [3, 1], "y": [1.5, None]}, index=["r", "s"]).T),
    "transpose int float complete": lambda m: _typed(m.DataFrame({"x": [3, 1], "y": [1.5, 2.5]}, index=["r", "s"]).T),
    # NEGATIVES: all-int stays int64; text with numbers stays object.
    "transpose ints": lambda m: _typed(m.DataFrame({"x": [3, 1], "y": [2, 4]}, index=["r", "s"]).T),
    "transpose mixed text": lambda m: _typed(m.DataFrame({"x": [3, 1], "s": ["a", "b"]}, index=["r", "s"]).T),
    "get_dummies dtype int": lambda m: _typed(m.get_dummies(m.Series(["a", "b", "a"]), dtype=int)),
    "get_dummies dtype float": lambda m: _typed(m.get_dummies(m.Series(["a", "b", "a"]), dtype=float)),
    "get_dummies dtype name": lambda m: _typed(m.get_dummies(m.Series(["a", "b", "a"]), dtype="int64", drop_first=True)),
    "get_dummies frame dtype": lambda m: _typed(m.get_dummies(m.DataFrame({"k": ["a", "b"], "x": [1, 2]}), columns=["k"], dtype=float)),
    # NEGATIVE: the default stays bool.
    "get_dummies default bool": lambda m: _typed(m.get_dummies(m.Series(["a", "b"]))),
    "period_range astype str": lambda m: m.period_range("2024-01", periods=3, freq="M").astype(str).tolist(),
    "date_range astype str": lambda m: m.date_range("2024-01-01", periods=2).astype(str).tolist(),
    "timedelta index astype str": lambda m: m.to_timedelta(["1h", "2h"]).astype(str).tolist(),
}


def _dtype_outcome(m: Any, case: str) -> Any:
    try:
        return _DTYPE_CASES[case](m)
    except Exception as e:  # noqa: BLE001 - the exception type is the outcome
        return ("raise", type(e).__name__)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_DTYPE_CASES))
def test_dtype_objects_unstack_transpose_and_get_dummies_dtypes_match_pandas(case: str) -> None:
    assert _dtype_outcome(fpd, case) == _dtype_outcome(pd, case), case


def _string_series(m: Any) -> Any:
    return m.Series(["Apple pie", "banana", None, "Cherry-tart", "e1f2", "a9b8"], name="t")


def _raises_value_error(call: Any) -> bool:
    try:
        call()
    except ValueError:
        return True
    return False


def _timed(m: Any) -> Any:
    return m.Series([1, 2, 3, 4], index=m.to_datetime(["2024-01-01 09:00", "2024-01-01 15:30", "2024-01-02 23:00", "2024-01-03 01:00"]))


# Silently wrong: Index.drop([label]) dropped nothing (the list was read as
# one label); between_time / at_time on a real DatetimeIndex selected
# nothing (only text labels were read); str.replace(regex=True) wrote a
# `\2\1` backreference as text and read `$1` as a group. Raised or missing:
# callable str.replace, searchsorted of a list, str.join / translate /
# extractall, str.cat(na_rep=) without others; Index isin / duplicated /
# argsort / isna were lists (`.any()` raised AttributeError).
_TEXT_TIME_CASES = {
    "index drop list": lambda m: m.Index([3, 1, 2, 1]).drop([1]).tolist(),
    "index drop label": lambda m: m.Index(["a", "b"]).drop("a").tolist(),
    "index drop ignore": lambda m: m.Index([3, 1]).drop([9], errors="ignore").tolist(),
    # NEGATIVE: a label that is not there is pandas' KeyError.
    "index drop missing raises": lambda m: m.Index([3, 1]).drop([9]),
    "between_time": lambda m: _shaped(_timed(m).between_time("08:00", "16:00")),
    "between_time wraps midnight": lambda m: _shaped(_timed(m).between_time("22:00", "02:00")),
    "between_time seconds": lambda m: _shaped(_timed(m).between_time("09:00:00", "15:30:00")),
    "at_time": lambda m: _shaped(_timed(m).at_time("15:30")),
    "frame between_time": lambda m: _shaped(_timed(m).to_frame("v").between_time("00:00", "10:00")),
    "frame at_time pm": lambda m: _shaped(_timed(m).to_frame("v").at_time("11:00PM")),
    # NEGATIVES: no match is empty; an unreadable time raises.
    "at_time no match": lambda m: _shaped(_timed(m).at_time("12:00")),
    # (pandas raises dateutil's ParserError, a ValueError; compared as one.)
    "at_time bad time": lambda m: _raises_value_error(lambda: _timed(m).at_time("25:99")),
    "replace backrefs": lambda m: _shaped(_string_series(m).str.replace(r"(\w)(\d)", r"\2\1", regex=True)),
    "replace named group": lambda m: _shaped(_string_series(m).str.replace(r"(?P<l>[a-z])(?P<d>\d)", r"\g<d>", regex=True)),
    "replace dollar literal": lambda m: _shaped(_string_series(m).str.replace(r"(a)", "$1", regex=True)),
    "replace callable": lambda m: _shaped(_string_series(m).str.replace(r"[aeiou]", lambda found: found.group(0).upper(), regex=True)),
    "replace case insensitive": lambda m: _shaped(_string_series(m).str.replace("A", "_", case=False, regex=True)),
    "replace flags": lambda m: _shaped(_string_series(m).str.replace("^a", "_", flags=__import__("re").IGNORECASE, regex=True)),
    "replace n": lambda m: _shaped(_string_series(m).str.replace("a", "_", n=1)),
    "replace literal backslash": lambda m: _shaped(m.Series(["a.b"]).str.replace(".", r"\\", regex=False)),
    # NEGATIVE: a callable needs regex=True.
    "replace callable without regex": lambda m: _string_series(m).str.replace("a", lambda found: "x", regex=False),
    "str join": lambda m: _shaped(_string_series(m).str.join("-")),
    "str translate": lambda m: _shaped(_string_series(m).str.translate(str.maketrans("ae", "AE"))),
    "str extractall": lambda m: _shaped(_string_series(m).str.extractall(r"([a-z])(\d)")),
    "str extractall named": lambda m: _shaped(_string_series(m).str.extractall(r"(?P<letter>[a-z])(?P<digit>\d)")),
    # NEGATIVE: a pattern without groups is pandas' ValueError.
    "str extractall no groups": lambda m: _string_series(m).str.extractall(r"[a-z]\d"),
    "str cat na_rep": lambda m: _string_series(m).str.cat(sep=",", na_rep="?"),
    "str cat skips missing": lambda m: _string_series(m).str.cat(sep=","),
    "searchsorted list": lambda m: m.Series([1, 3, 5]).searchsorted([2, 5], side="right").tolist(),
    "searchsorted scalar": lambda m: int(m.Series([1, 3, 5]).searchsorted(4)),
    "index duplicated any": lambda m: (m.Index([3, 1, 3]).duplicated().any(), m.Index([3, 1, 3]).duplicated().tolist()),
    "index isin sum": lambda m: (int(m.Index([3, 1, 2]).isin([1, 3]).sum()), m.Index([3, 1, 2]).isin([1, 3]).tolist()),
    "index argsort": lambda m: m.Index([3, 1, 2]).argsort().tolist(),
    "index isna any": lambda m: bool(m.Index([1.0, None]).isna().any()),
}


def _text_time_outcome(m: Any, case: str) -> Any:
    try:
        return _TEXT_TIME_CASES[case](m)
    except Exception as e:  # noqa: BLE001 - the exception type is the outcome
        return ("raise", type(e).__name__)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_TEXT_TIME_CASES))
def test_index_drop_time_selection_regex_replace_and_text_methods_match_pandas(case: str) -> None:
    assert _text_time_outcome(fpd, case) == _text_time_outcome(pd, case), case


def _levels(m: Any) -> Any:
    return m.Series(["lo", "hi", "mid", "lo"], dtype=m.CategoricalDtype(["lo", "mid", "hi"], ordered=True))


def _grouped(m: Any) -> Any:
    return m.DataFrame({"g": ["a", "b", "a", "b", "c"], "h": [1, 1, 2, 2, 1], "x": [1.0, 2.0, None, 4.0, 5.0], "y": [5, 4, 3, 2, 1]})


# Silently wrong: a CategoricalDtype(categories, ordered=True) given to
# Series(...) / astype lost its category order and ordering (re-sorted as
# text), so sorts, max, codes, value_counts and groupby followed 'hi' < 'lo'
# < 'mid'; an all-NaN rolling corr / cov was object dtype. Raised: `for key,
# group in gb` (TypeError), named ("col", "size") aggregation, an ordered
# categorical against a scalar (`cat > 'lo'`), rename_categories with a dict
# or a callable.
_CATEGORY_GROUP_CASES = {
    "ordered sort": lambda m: _shaped(_levels(m).sort_values()),
    "ordered categories": lambda m: _levels(m).cat.categories.tolist(),
    "ordered codes": lambda m: _levels(m).cat.codes.tolist(),
    "ordered max min": lambda m: (_levels(m).max(), _levels(m).min()),
    "ordered value_counts": lambda m: _shaped(_levels(m).value_counts()),
    "ordered groupby": lambda m: _shaped(m.DataFrame({"c": _levels(m), "v": [1, 2, 3, 4]}).groupby("c", observed=False)["v"].sum()),
    "ordered compare scalar": lambda m: _shaped(_levels(m) > "lo"),
    "ordered compare reflected": lambda m: _shaped("mid" >= _levels(m)),
    "astype categorical dtype": lambda m: m.Series(["b", "a", "c"]).astype(m.CategoricalDtype(["c", "b", "a"], ordered=True)).sort_values().tolist(),
    "value outside categories": lambda m: _shaped(m.Series(["lo", "zz"], dtype=m.CategoricalDtype(["lo", "hi"]))),
    "rename categories dict": lambda m: _shaped(_levels(m).cat.rename_categories({"lo": "L"})),
    "rename categories callable": lambda m: _shaped(_levels(m).cat.rename_categories(lambda c: c.upper())),
    # NEGATIVES: a scalar outside the categories and an unordered
    # categorical's ordering comparison raise TypeError.
    "compare scalar not a category": lambda m: _levels(m) > "zz",
    "unordered compare": lambda m: m.Series(["a", "b"], dtype="category") > "a",
    "groupby iterate": lambda m: [(k, _shaped(g)) for k, g in _grouped(m).groupby("g")],
    "groupby iterate two keys": lambda m: [(k, len(g)) for k, g in _grouped(m).groupby(["g", "h"])],
    "groupby iterate unsorted": lambda m: [k for k, _ in _grouped(m).groupby("g", sort=False)],
    "series groupby iterate": lambda m: [(k, g.tolist()) for k, g in _grouped(m).groupby("g")["y"]],
    "named agg size": lambda m: _shaped(_grouped(m).groupby("g").agg(x_sum=("x", "sum"), n=("y", "size"), x_n=("x", "size"))),
    "rolling corr all undefined": lambda m: (str(_grouped(m)["y"].rolling(3).corr(_grouped(m)["x"]).dtype), _shaped(_grouped(m)["y"].rolling(3).corr(_grouped(m)["x"]))),
}


def _category_group_outcome(m: Any, case: str) -> Any:
    try:
        return _CATEGORY_GROUP_CASES[case](m)
    except Exception as e:  # noqa: BLE001 - the exception type is the outcome
        return ("raise", type(e).__name__)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_CATEGORY_GROUP_CASES))
def test_categorical_dtype_order_groupby_iteration_and_named_size_match_pandas(case: str) -> None:
    assert _category_group_outcome(fpd, case) == _category_group_outcome(pd, case), case


def _daily(m: Any) -> Any:
    return m.Series(range(6), index=m.date_range("2024-01-30", periods=6, freq="D"))


def _hourly(m: Any) -> Any:
    return m.Series(range(4), index=m.date_range("2024-01-01 22:00", periods=4, freq="h"))


def _daily_frame(m: Any) -> Any:
    return m.DataFrame({"v": range(6), "w": range(6)}, index=m.date_range("2024-01-30", periods=6, freq="D"))


def _written(obj: Any, write: Any) -> Any:
    write(obj)
    return _shaped(obj)


# Date text on a DatetimeIndex was compared as text: ts.loc['2024-02-01'],
# ts['2024-02'] (partial-string indexing), df.loc['2024-02', 'w'] and
# ts.at['2024-02-01'] raised KeyError, s['2024-02'] = 0 appended a row
# labeled '2024-02', truncate(before='2024-02-01') kept every row (or none),
# and first('3D') / last('2D') kept one row / every row; ts[Timestamp]
# raised TypeError.
_DATE_TEXT_CASES = {
    "month loc": lambda m: _shaped(_daily(m).loc["2024-02"]),
    "month getitem": lambda m: _shaped(_daily(m)["2024-02"]),
    "year": lambda m: _shaped(_daily(m)["2024"]),
    "day exact": lambda m: _number(_daily(m).loc["2024-02-01"]),
    "day on hourly": lambda m: _shaped(_hourly(m).loc["2024-01-02"]),
    "hour on hourly": lambda m: _number(_hourly(m).loc["2024-01-02 00"]),
    "non-monotonic month": lambda m: _shaped(m.Series([1, 2, 3], index=m.to_datetime(["2024-02-03", "2024-01-05", "2024-02-01"])).loc["2024-02"]),
    "frame month": lambda m: _shaped(_daily_frame(m).loc["2024-02"]),
    "frame month column": lambda m: _shaped(_daily_frame(m).loc["2024-02", "w"]),
    # The row's name is left out: Series names are text in the binding, so
    # pandas' Timestamp name comes back as text
    # (br-frankenpandas-rc0923-epic-python-honest-dropin-fvsao.32).
    "frame day row": lambda m: (_daily_frame(m).loc["2024-02-01"].tolist(), [str(i) for i in _daily_frame(m).loc["2024-02-01"].index]),
    "list of dates": lambda m: _shaped(_daily(m).loc[["2024-02-01", "2024-01-30"]]),
    "timestamp key": lambda m: _number(_daily(m)[m.Timestamp("2024-02-02")]),
    "at text": lambda m: _number(_daily(m).at["2024-02-01"]),
    "frame at text": lambda m: _number(_daily_frame(m).at["2024-02-02", "v"]),
    "set month": lambda m: _written(_daily(m), lambda s: s.__setitem__("2024-02", 0)),
    "frame loc set month": lambda m: _written(_daily_frame(m), lambda d: d.loc.__setitem__(("2024-02", "v"), 0)),
    "set out of range month appends": lambda m: _written(_daily(m), lambda s: s.__setitem__("2025-02", 1)),
    "loc set new day appends": lambda m: _written(_daily(m), lambda s: s.loc.__setitem__("2024-03-01", 9)),
    "duplicated text label": lambda m: _shaped(m.Series([1, 2, 3], index=["a", "b", "a"])["a"]),
    "truncate text": lambda m: _shaped(_daily(m).truncate(before="2024-01-31", after="2024-02-02")),
    "truncate month bound": lambda m: _shaped(_daily(m).truncate(after="2024-01")),
    "frame truncate": lambda m: _shaped(_daily_frame(m).truncate(before="2024-02-03")),
    "truncate decreasing": lambda m: _shaped(m.Series(range(3), index=m.to_datetime(["2024-01-03", "2024-01-02", "2024-01-01"])).truncate(before="2024-01-02")),
    "first 3D": lambda m: _shaped(_daily(m).first("3D")),
    "first 1M": lambda m: _shaped(_daily(m).first("1M")),
    "first 36h": lambda m: _shaped(_daily(m).first("36h")),
    "first ME on anchor": lambda m: _shaped(m.Series(range(4), index=m.date_range("2024-01-31", periods=4, freq="D")).first("1ME")),
    "first 2h hourly": lambda m: _shaped(_hourly(m).first("2h")),
    "last 2D": lambda m: _shaped(_daily(m).last("2D")),
    "last 1M": lambda m: _shaped(_daily(m).last("1M")),
    "last 1W": lambda m: _shaped(_daily(m).last("1W")),
    "frame first 2D": lambda m: _shaped(_daily_frame(m).first("2D")),
    # NEGATIVES: a period wholly outside a sorted index and a missing day are
    # KeyError; truncate of an unsorted index or with before > after is a
    # ValueError; a missing text label on a text index stays a KeyError.
    "missing month": lambda m: _daily(m).loc["2025-02"],
    "missing day": lambda m: _daily(m).loc["2024-03-01"],
    "truncate unsorted": lambda m: m.Series(range(3), index=m.to_datetime(["2024-01-03", "2024-01-01", "2024-01-02"])).truncate(before="2024-01-02"),
    "truncate inverted": lambda m: _daily(m).truncate(before="2024-02-03", after="2024-02-01"),
    "missing text label": lambda m: m.Series([1, 2], index=["a", "b"])["z"],
}


def _date_text_outcome(m: Any, case: str) -> Any:
    try:
        with warnings.catch_warnings():
            # pandas deprecates first / last (FutureWarning).
            warnings.simplefilter("ignore", FutureWarning)
            return _DATE_TEXT_CASES[case](m)
    except Exception as e:  # noqa: BLE001 - the exception type is the outcome
        return ("raise", type(e).__name__)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_DATE_TEXT_CASES))
def test_date_text_keys_truncate_and_first_last_on_a_datetime_index_match_pandas(case: str) -> None:
    assert _date_text_outcome(fpd, case) == _date_text_outcome(pd, case), case


def _halfdays(m: Any) -> Any:
    return m.DataFrame({"v": [1.0, 2.0, 3.0, 4.0], "w": [4, 3, 2, 1]}, index=m.date_range("2024-01-01", periods=4, freq="12h"))


def _levels_frame(m: Any) -> Any:
    return m.DataFrame({"v": [1.0, 2.0, 3.0, 4.0]}, index=m.MultiIndex.from_product([["a", "b"], [1, 2]], names=["k", "n"]))


def _missing_or(v: Any) -> Any:
    """A cell with every missing marker (None, NaN, pd.NA, frankenpandas'
    NA, which pd.isna does not know) read as None."""
    return None if v is getattr(fpd, "NA", None) or pd.isna(v) else v


def _frame_facts(r: Any) -> Any:
    """Column labels, dtypes and cells, by position (a label may repeat)."""
    return ([str(c) for c in r.columns], [str(d) for d in r.dtypes], [[_missing_or(v) for v in r.iloc[:, i].tolist()] for i in range(r.shape[1])])


# Silently wrong: dtypes of a frame with a repeated column label read the
# first column's dtype (pivot_table(aggfunc=['sum', 'mean']) reported int64
# for the float means); corrwith named its result 'corrwith';
# Series(..., dtype='string') / astype('string') wrote None as 'None' (isna
# False, str.upper 'NONE'); resample().agg([...]) on a frame gave (function,
# column) columns. Raised: Int64 (nullable) mean; Int64 sum / max came back
# float; resample().agg(dict); rename_axis with a list / dict / function;
# Interval.overlaps.
_REDUCE_RENAME_CASES = {
    "dtypes repeated label": lambda m: _frame_facts(m.concat([m.DataFrame({"v": [3, 3]}), m.DataFrame({"v": [1.5, 3.0]})], axis=1)),
    "pivot_table aggfunc list dtypes": lambda m: _frame_facts(m.DataFrame({"g": ["a", "a", "b"], "v": [1, 2, 3]}).pivot_table(index="g", values="v", aggfunc=["sum", "mean"])),
    "corrwith frame name": lambda m: _five(m).corrwith(_five(m) * 2).name,
    "corrwith series name": lambda m: _shaped(_five(m).corrwith(_five(m)["a"]), digits=12),
    "string dtype missing": lambda m: (lambda s: (s.isna().tolist(), [_missing_or(v) for v in s.str.upper().tolist()]))(m.Series(["a", None], dtype="string")),
    "astype string missing": lambda m: m.Series(["a", None]).astype("string").isna().tolist(),
    # NEGATIVE: numpy's str still writes the missing value as text.
    "astype str writes None": lambda m: m.Series(["a", None]).astype(str).tolist(),
    "Int64 sum": lambda m: _number(m.Series([1, None, 3], dtype="Int64").sum()),
    "Int64 max min": lambda m: (_number(m.Series([1, None, 3], dtype="Int64").max()), _number(m.Series([1, None, 3], dtype="Int64").min())),
    "Int64 mean": lambda m: _number(m.Series([1, None, 3], dtype="Int64").mean()),
    "Int64 all missing": lambda m: (_number(m.Series([None, None], dtype="Int64").sum()), type(m.Series([None, None], dtype="Int64").max()).__name__),
    "resample agg dict": lambda m: _frame_facts(_halfdays(m).resample("D").agg({"w": "max", "v": "sum"})),
    "resample agg dict with list": lambda m: _frame_facts(_halfdays(m).resample("D").agg({"v": "sum", "w": ["max", "min"]})),
    "resample agg list frame": lambda m: _frame_facts(_halfdays(m).resample("D").agg(["sum", "max"])),
    "series resample agg dict": lambda m: _frame_facts(_halfdays(m)["v"].resample("D").agg({"total": "sum"})),
    "rename_axis list": lambda m: list(_levels_frame(m).rename_axis(["K", "N"]).index.names),
    "rename_axis index dict": lambda m: list(_levels_frame(m).rename_axis(index={"k": "KK"}).index.names),
    "rename_axis index function": lambda m: list(_levels_frame(m).rename_axis(index=str.upper).index.names),
    "series rename_axis list": lambda m: list(_levels_frame(m)["v"].rename_axis(["K", "N"]).index.names),
    "rename_axis flat": lambda m: _halfdays(m).rename_axis("when").index.name,
    "interval overlaps": lambda m: (m.Interval(0, 5).overlaps(m.Interval(4, 6)), m.Interval(0, 5).overlaps(m.Interval(5, 6)), m.Interval(0, 5, closed="both").overlaps(m.Interval(5, 6, closed="left"))),
    # NEGATIVES: a missing column in the dict is KeyError, a dict mapper or
    # a wrong-length list is ValueError, overlaps of a non-Interval TypeError.
    "resample agg dict missing column": lambda m: _halfdays(m).resample("D").agg({"z": "sum"}),
    "rename_axis mapper dict": lambda m: _levels_frame(m).rename_axis({"k": "KK"}),
    "rename_axis wrong length": lambda m: _levels_frame(m).rename_axis(["K"]),
    "interval overlaps number": lambda m: m.Interval(0, 5).overlaps(3),
}


def _reduce_rename_outcome(m: Any, case: str) -> Any:
    try:
        return _REDUCE_RENAME_CASES[case](m)
    except Exception as e:  # noqa: BLE001 - the exception type is the outcome
        return ("raise", type(e).__name__)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_REDUCE_RENAME_CASES))
def test_nullable_reductions_string_dtype_resample_agg_and_rename_axis_match_pandas(case: str) -> None:
    assert _reduce_rename_outcome(fpd, case) == _reduce_rename_outcome(pd, case), case


def _two_level(m: Any) -> Any:
    return m.DataFrame({"v": [1.0, 2.0, 3.0, 4.0], "w": [10, 20, 30, 40]}, index=m.MultiIndex.from_product([["a", "b"], [1, 2]], names=["k", "n"]))


def _unstacked(r: Any) -> Any:
    """An unstacked frame: its index, its (column, value) labels as text -
    column labels are text in the binding
    (br-frankenpandas-rc0923-epic-python-honest-dropin-fvsao.32) - dtypes
    and cells by position."""
    return ([str(i) for i in r.index], [tuple(str(part) for part in c) for c in r.columns], [str(d) for d in r.dtypes], [[_missing_or(v) for v in r.iloc[:, i].tolist()] for i in range(r.shape[1])])


# Raised: per-level MultiIndex keys - df.loc[pd.IndexSlice['a', :], :],
# df.loc[(slice(None), 1), :], s.loc[pd.IndexSlice[:, 2]] (read as one text
# label: KeyError); DataFrame.unstack() of several columns (refused) and
# unstack(level) / unstack(fill_value=) (no arguments); Series([Interval,
# ...]) ('Cannot convert Interval to Scalar'). Silently wrong: an Interval
# cell came back None, Interval labels printed as Rust debug text, and
# Interval(0, 3) == Interval(0, 3) was False (identity).
_LEVEL_KEY_CASES = {
    "indexslice outer": lambda m: _shaped(_two_level(m).loc[m.IndexSlice["a", :], :]),
    "indexslice inner column": lambda m: _shaped(_two_level(m).loc[m.IndexSlice[:, 2], "v"]),
    "indexslice list": lambda m: _shaped(_two_level(m).loc[m.IndexSlice[["a", "b"], 1], ["w"]]),
    "indexslice range": lambda m: _shaped(_two_level(m).loc[m.IndexSlice["a":"b", 2:2], :]),
    "slice none tuple": lambda m: _shaped(_two_level(m).loc[(slice(None), 1), :]),
    # (rows, cols), not a per-level key: the outer label drops its level.
    "outer label all columns": lambda m: _shaped(_two_level(m).loc["a", :]),
    "series indexslice": lambda m: _shaped(_two_level(m)["v"].loc[m.IndexSlice[:, 2]]),
    "series tuple slice": lambda m: _shaped(_two_level(m)["v"].loc[("a", slice(None))]),
    "unstack two columns": lambda m: _unstacked(_two_level(m).unstack()),
    "unstack level 0": lambda m: _unstacked(_two_level(m).unstack(0)),
    "unstack level name": lambda m: _unstacked(_two_level(m).unstack("k")),
    "unstack one column": lambda m: _unstacked(_two_level(m)[["v"]].unstack()),
    "unstack fill_value": lambda m: _unstacked(_two_level(m).iloc[:3].unstack(fill_value=0)),
    "unstack missing": lambda m: _unstacked(_two_level(m).iloc[:3].unstack()),
    "interval cells": lambda m: [(type(v).__name__, v.left, v.right, v.closed) for v in m.Series([m.Interval(0, 1), m.Interval(1, 2, closed="left")]).tolist()],
    "interval equality": lambda m: (m.Interval(0, 3) == m.Interval(0, 3), m.Interval(0, 3) == m.Interval(0, 3, closed="left"), len({m.Interval(0, 3), m.Interval(0, 3)})),
    # NEGATIVE: a level label the index lacks is KeyError.
    "indexslice missing label": lambda m: _two_level(m).loc[m.IndexSlice["z", :], :],
}


def _level_key_outcome(m: Any, case: str) -> Any:
    try:
        return _LEVEL_KEY_CASES[case](m)
    except Exception as e:  # noqa: BLE001 - the exception type is the outcome
        return ("raise", type(e).__name__)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_LEVEL_KEY_CASES))
def test_multiindex_level_keys_unstack_columns_and_interval_cells_match_pandas(case: str) -> None:
    assert _level_key_outcome(fpd, case) == _level_key_outcome(pd, case), case


def _tokyo(m: Any) -> Any:
    return m.Timestamp("2024-03-10 14:30:15").tz_localize("UTC").tz_convert("Asia/Tokyo")


def _naive_times(m: Any) -> Any:
    return m.Series(m.to_datetime(["2024-03-10 01:30", "2024-06-01 12:00", None]))


def _texts_of(r: Any) -> Any:
    """A Series as its dtype and each cell's text (pandas' Timestamp str)."""
    return (str(r.dtype), [str(v) for v in r.tolist()])


# Timezones (fvsao.35): Timestamp.tz_localize / tz_convert and
# Timestamp(..., tz=) were missing or refused; Series.dt.tz_localize /
# tz_convert / .tz raised AttributeError although fp-frame implements them;
# to_datetime(utc=True) returned a naive column; an offset with no seconds
# ('2024-01-01 00:00+05:00') did not parse. Silently wrong once wired: a
# tz-aware column's cells came back naive UTC wall times and dt.hour / day
# read the UTC clock.
_TZ_CASES = {
    "localize repr": lambda m: repr(m.Timestamp("2024-03-10 14:30:15").tz_localize("US/Eastern")),
    "convert str repr": lambda m: (str(_tokyo(m)), repr(_tokyo(m))),
    "wall fields": lambda m: (lambda t: (t.year, t.month, t.day, t.hour, t.minute, t.dayofweek, t.day_name(), t.is_month_start))(_tokyo(m)),
    "value and tz": lambda m: (_tokyo(m).value, str(_tokyo(m).tz)),
    "isoformat strftime": lambda m: (_tokyo(m).isoformat(), _tokyo(m).strftime("%Y-%m-%d %H:%M")),
    "drop zone": lambda m: (str(_tokyo(m).tz_localize(None)), str(_tokyo(m).tz_convert(None))),
    "same instant equal": lambda m: (_tokyo(m) == m.Timestamp("2024-03-10 14:30:15", tz="UTC"), hash(_tokyo(m)) == hash(m.Timestamp("2024-03-10 14:30:15", tz="UTC"))),
    "floor normalize replace": lambda m: (str(_tokyo(m).floor("D")), str(_tokyo(m).normalize()), str(_tokyo(m).replace(hour=1))),
    "timedelta and offset": lambda m: (str(_tokyo(m) + m.Timedelta(hours=12)), str(m.Timestamp("2024-03-09 12:00", tz="US/Eastern") + m.DateOffset(days=1))),
    "aware difference across dst": lambda m: str(m.Timestamp("2024-11-03 12:00", tz="US/Eastern") - m.Timestamp("2024-11-02 12:00", tz="US/Eastern")),
    "constructor tz": lambda m: (str(m.Timestamp("2024-01-01 12:00", tz="US/Eastern")), m.Timestamp("2024-07-01 12:00", tz="US/Eastern").value),
    "series localize": lambda m: _texts_of(_naive_times(m).dt.tz_localize("US/Eastern")),
    "series convert": lambda m: _texts_of(_naive_times(m).dt.tz_localize("UTC").dt.tz_convert("Asia/Tokyo")),
    "series tz attr": lambda m: (str(_naive_times(m).dt.tz_localize("UTC").dt.tz), _naive_times(m).dt.tz),
    "series drop zone": lambda m: (_texts_of(_naive_times(m).dt.tz_localize("Asia/Tokyo").dt.tz_localize(None)), _texts_of(_naive_times(m).dt.tz_localize("Asia/Tokyo").dt.tz_convert(None))),
    "series wall fields": lambda m: (lambda s: ([_missing_or(v) for v in s.dt.hour.tolist()], [_missing_or(v) for v in s.dt.day.tolist()], [_missing_or(v) for v in s.dt.day_name().tolist()]))(_naive_times(m).dt.tz_localize("UTC").dt.tz_convert("US/Eastern")),
    "series cells": lambda m: (lambda s: (str(s[1]), str(s.iloc[0]), str(s.iat[1])))(_naive_times(m).dt.tz_localize("UTC").dt.tz_convert("Asia/Tokyo")),
    "to_datetime utc": lambda m: _texts_of(m.to_datetime(m.Series(["2024-01-01 00:00", "2024-01-02 06:00"]), utc=True)),
    "to_datetime utc offsets": lambda m: _texts_of(m.to_datetime(m.Series(["2024-01-01 00:00+05:00", "2024-01-02 06:00-02:00"]), utc=True)),
    # NEGATIVES: localizing an aware value / converting a naive one are
    # TypeError; an unknown zone is pytz's KeyError subclass; a wall time the
    # DST change skips raises.
    "localize aware": lambda m: _tokyo(m).tz_localize("UTC"),
    "convert naive": lambda m: m.Timestamp("2024-03-10").tz_convert("UTC"),
    "series localize aware": lambda m: _naive_times(m).dt.tz_localize("UTC").dt.tz_localize("UTC"),
    "series convert naive": lambda m: _naive_times(m).dt.tz_convert("UTC"),
    "unknown zone is a KeyError": lambda m: _raises_key_error(lambda: m.Timestamp("2024-03-10").tz_localize("Mars/Olympus")),
    "skipped wall time raises": lambda m: _raises(lambda: m.Timestamp("2024-03-10 02:30").tz_localize("US/Eastern")),
}


def _raises_key_error(f: Any) -> Any:
    try:
        f()
    except KeyError:
        return "KeyError"
    return "no error"


def _raises(f: Any) -> Any:
    try:
        f()
    except Exception:  # noqa: BLE001 - pytz's NonExistentTimeError is not a ValueError
        return "raised"
    return "no error"


def _tz_outcome(m: Any, case: str) -> Any:
    try:
        return _TZ_CASES[case](m)
    except Exception as e:  # noqa: BLE001 - the exception type is the outcome
        return ("raise", type(e).__name__)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_TZ_CASES))
def test_timezone_timestamps_series_dt_and_utc_parsing_match_pandas(case: str) -> None:
    assert _tz_outcome(fpd, case) == _tz_outcome(pd, case), case


def _keyed(m: Any, keys: Any) -> Any:
    return m.DataFrame({"k": keys, "j": [1, 2, 3], "v": [10, 20, 30]})


# set_index with a missing key raised "set_index does not support missing
# label values" (pandas keeps None / NaN / NaT as missing labels), and a
# two-key set_index silently wrote the missing key as '' where pandas' level
# holds NaN.
_SET_INDEX_NA_CASES = {
    "object key": lambda m: _shaped(_keyed(m, ["a", None, "c"]).set_index("k")),
    "float key": lambda m: _shaped(_keyed(m, [1.5, None, 3.5]).set_index("k")),
    "datetime key": lambda m: _shaped(_keyed(m, m.to_datetime(["2024-01-01", None, "2024-01-03"])).set_index("k")),
    "two keys": lambda m: _shaped(_keyed(m, ["a", None, "c"]).set_index(["k", "j"])),
    "append": lambda m: _shaped(_keyed(m, ["a", None, "c"]).set_index("k", append=True)),
    "index isna": lambda m: _keyed(m, ["a", None, "c"]).set_index("k").index.isna().tolist(),
    "groupby drops the missing key": lambda m: _shaped(_keyed(m, ["a", None, "a"]).set_index("k").groupby(level=0)["v"].sum()),
    "reset_index round trip": lambda m: _shaped(_keyed(m, [1.5, None, 3.5]).set_index("k").reset_index()),
    # NEGATIVE: a column that does not exist is still a KeyError.
    "missing column": lambda m: _keyed(m, ["a", None, "c"]).set_index("zz"),
}


def _set_index_na_outcome(m: Any, case: str) -> Any:
    try:
        return _SET_INDEX_NA_CASES[case](m)
    except Exception as e:  # noqa: BLE001 - the exception type is the outcome
        return ("raise", type(e).__name__)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_SET_INDEX_NA_CASES))
def test_set_index_keeps_missing_keys_as_missing_labels_like_pandas(case: str) -> None:
    assert _set_index_na_outcome(fpd, case) == _set_index_na_outcome(pd, case), case


def _ranked(m: Any) -> Any:
    return m.Series([3.0, 1.0, _NAN, 4.0, 1.0, 5.0])


def _gb_named_frame(m: Any) -> Any:
    return m.DataFrame({"g": ["a", "b", "a"], "h": [1, 1, 2], "v": [1, 2, 3]})


# (fvsao.46) rolling/expanding rank(pct=True) was refused and rank took an
# na_option pandas' window rank does not have; groupby.apply's group had no
# `.name` (pandas sets it to the group key) - DataFrame groups raised
# AttributeError and Series groups kept the column's name.
_RANK_NAME_CASES = {
    "rolling pct": lambda m: [_missing_or(v) for v in _ranked(m).rolling(3).rank(pct=True).tolist()],
    "rolling pct min_periods": lambda m: [_missing_or(v) for v in _ranked(m).rolling(3, min_periods=1).rank(pct=True).tolist()],
    "rolling pct centered tail": lambda m: [_missing_or(v) for v in _ranked(m).rolling(3, center=True, min_periods=1).rank(pct=True).tolist()],
    "rolling min pct": lambda m: [_missing_or(v) for v in _ranked(m).rolling(3, min_periods=1).rank(method="min", pct=True).tolist()],
    "expanding pct": lambda m: [_missing_or(v) for v in _ranked(m).expanding().rank(pct=True).tolist()],
    "frame rolling pct": lambda m: [_missing_or(v) for v in m.DataFrame({"a": [3.0, 1.0, 2.0], "b": [1.0, 2.0, 3.0]}).rolling(2, min_periods=1).rank(pct=True)["a"].tolist()],
    "apply group name": lambda m: _shaped(_gb_named_frame(m).groupby("g").apply(lambda d: d.name, include_groups=False)),
    # The key tuple joined as text: tuple-valued cells are fvsao.33.
    "apply group name two keys": lambda m: _gb_named_frame(m).groupby(["g", "h"]).apply(lambda d: "|".join(map(str, d.name)), include_groups=False).tolist(),
    "apply uses the name": lambda m: _shaped(_gb_named_frame(m).groupby("g").apply(lambda d: d["v"].sum() if d.name == "a" else 0, include_groups=False)),
    "series apply group name": lambda m: _shaped(_gb_named_frame(m).groupby("g")["v"].apply(lambda s: s.name)),
    # NEGATIVES: pandas' window rank has no na_option; a column called
    # 'name' is still reached by attribute outside groupby.
    "na_option rejected": lambda m: _ranked(m).rolling(3).rank(na_option="keep"),
    "column called name": lambda m: m.DataFrame({"name": [1, 2]}).name.tolist(),
}


def _rank_name_outcome(m: Any, case: str) -> Any:
    try:
        return _RANK_NAME_CASES[case](m)
    except Exception as e:  # noqa: BLE001 - the exception type is the outcome
        return ("raise", type(e).__name__)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_RANK_NAME_CASES))
def test_window_rank_pct_and_groupby_apply_group_name_match_pandas(case: str) -> None:
    assert _rank_name_outcome(fpd, case) == _rank_name_outcome(pd, case), case


def _scalar_kind(x: Any) -> Any:
    """A value's kind and text: a numpy scalar as `numpy.<type>`, anything
    else by its type's name (fp's classes live in their own module)."""
    kind = ("numpy." if isinstance(x, np.generic) else "") + type(x).__name__
    return (kind, x if isinstance(x, (bool, str)) else str(x))


def _nullable_ints(m: Any) -> Any:
    return m.Series([1, None], dtype="Int64")


# (fvsao.25) Reductions and element access returned Python scalars where
# pandas returns numpy scalars (repr np.float64(5.0), isinstance(x, int) is
# False, json.dumps raises); iterating a nullable Series gave Python scalars
# where pandas iterates the masked array (numpy scalars, pd.NA); items() /
# iterrows() / itertuples() were lists, so next(...) raised.
_SCALAR_TYPE_CASES = {
    "s[i] int": lambda m: _scalar_kind(m.Series([1, 2])[0]),
    "s[i] float": lambda m: _scalar_kind(m.Series([1.5, 2.0])[0]),
    "s[i] nan": lambda m: _scalar_kind(m.Series([_NAN, 2.0])[0]),
    "s[i] bool": lambda m: _scalar_kind(m.Series([True])[0]),
    "iloc": lambda m: _scalar_kind(m.Series([1, 2]).iloc[1]),
    "at": lambda m: _scalar_kind(m.Series([1, 2]).at[0]),
    "loc label": lambda m: _scalar_kind(m.Series([1.5], index=["a"]).loc["a"]),
    "df iloc": lambda m: _scalar_kind(m.DataFrame({"a": [1]}).iloc[0, 0]),
    "df iat last column": lambda m: _scalar_kind(m.DataFrame({"a": [1], "b": [2.5]}).iat[0, -1]),
    "df at": lambda m: _scalar_kind(m.DataFrame({"a": [1.5]}).at[0, "a"]),
    "df loc": lambda m: _scalar_kind(m.DataFrame({"a": [True]}).loc[0, "a"]),
    "Int64 element": lambda m: _scalar_kind(_nullable_ints(m)[0]),
    "boolean element": lambda m: _scalar_kind(m.Series([True, None], dtype="boolean")[0]),
    "sum int": lambda m: _scalar_kind(m.Series([1, 2]).sum()),
    "sum float": lambda m: _scalar_kind(m.Series([1.5]).sum()),
    "sum bool": lambda m: _scalar_kind(m.Series([True, True]).sum()),
    "sum empty": lambda m: _scalar_kind(m.Series([], dtype="int64").sum()),
    "mean": lambda m: _scalar_kind(m.Series([1, 2]).mean()),
    "mean of NaN": lambda m: _scalar_kind(m.Series([_NAN]).mean()),
    "min": lambda m: _scalar_kind(m.Series([1, 2]).min()),
    "max float": lambda m: _scalar_kind(m.Series([1.5]).max()),
    "min of NaN": lambda m: _scalar_kind(m.Series([_NAN]).min()),
    "min empty": lambda m: _scalar_kind(m.Series([], dtype=float).min()),
    "std": lambda m: _scalar_kind(m.Series([1, 2]).std()),
    "var": lambda m: _scalar_kind(m.Series([1, 2]).var()),
    "median": lambda m: _scalar_kind(m.Series([1, 2, 3]).median()),
    "prod": lambda m: _scalar_kind(m.Series([1, 2]).prod()),
    "count": lambda m: _scalar_kind(m.Series([1, 2]).count()),
    "any": lambda m: _scalar_kind(m.Series([1, 0]).any()),
    "all": lambda m: _scalar_kind(m.Series([True]).all()),
    "quantile": lambda m: _scalar_kind(m.Series([1, 2]).quantile(0.5)),
    "sem": lambda m: _scalar_kind(m.Series([1, 2]).sem()),
    "Int64 sum": lambda m: _scalar_kind(_nullable_ints(m).sum()),
    "Int64 mean": lambda m: _scalar_kind(m.Series([1, 2, None], dtype="Int64").mean()),
    "Int64 count": lambda m: _scalar_kind(m.Series([1, 2, None], dtype="Int64").count()),
    "frame sum element": lambda m: _scalar_kind(m.DataFrame({"a": [1, 2]}).sum()["a"]),
    "np.sum delegates": lambda m: _scalar_kind(np.sum(m.Series([1, 2]))),
    "np.max delegates": lambda m: _scalar_kind(np.max(m.Series([1.5, 2.5]))),
    "repr of sum": lambda m: repr(m.Series([1.0, 4.0]).sum()),
    "repr of any": lambda m: repr((m.Series([1, 2]) > 1).any()),
    "isinstance int": lambda m: isinstance(m.Series([1, 2])[0], int),
    "item of sum": lambda m: _scalar_kind(m.Series([1, 2]).sum().item()),
    "iterate Int64": lambda m: [_scalar_kind(v) for v in _nullable_ints(m)],
    "iterate Float64": lambda m: [_scalar_kind(v) for v in m.Series([1.5, None], dtype="Float64")],
    "items Int64": lambda m: [_scalar_kind(v) for _, v in _nullable_ints(m).items()],
    "Int64 missing element is NA": lambda m: _nullable_ints(m)[1] is m.NA,
    "Int64 missing max is NA": lambda m: m.Series([None, None], dtype="Int64").max() is m.NA,
    "iterated missing is NA": lambda m: list(_nullable_ints(m))[1] is m.NA,
    "next iterrows": lambda m: [_scalar_kind(v) for v in next(m.DataFrame({"a": [1], "b": [1.5]}).iterrows())[1]],
    "next itertuples": lambda m: [_scalar_kind(v) for v in next(m.DataFrame({"a": [1], "b": [1.5]}).itertuples(index=False))],
    "next items": lambda m: next(m.Series([5], index=["k"]).items()),
    # NEGATIVES: tolist / iteration / to_dict / item / nunique of a numpy
    # dtype stay Python scalars, a nullable tolist too; object cells stay
    # themselves; json.dumps refuses a numpy integer but takes tolist().
    "tolist int": lambda m: [_scalar_kind(v) for v in m.Series([1, 2]).tolist()],
    "tolist Int64": lambda m: [_scalar_kind(v) for v in _nullable_ints(m).tolist()],
    "iterate int": lambda m: [_scalar_kind(v) for v in m.Series([1, 2])],
    "iterate float": lambda m: [_scalar_kind(v) for v in m.Series([1.5, _NAN])],
    "items int": lambda m: [_scalar_kind(v) for _, v in m.Series([1, 2]).items()],
    "to_dict": lambda m: [_scalar_kind(v) for v in m.Series([1, 2]).to_dict().values()],
    "item": lambda m: _scalar_kind(m.Series([1]).item()),
    "nunique": lambda m: _scalar_kind(m.Series([1]).nunique()),
    "object element": lambda m: _scalar_kind(m.Series(["a", None])[1]),
    "max of text": lambda m: _scalar_kind(m.Series(["a", "b"]).max()),
    "json of sum": lambda m: json.dumps(m.Series([1, 2]).sum()),
    "json of tolist": lambda m: json.dumps(m.Series([1, 2]).tolist()),
}


def _scalar_type_outcome(m: Any, case: str) -> Any:
    try:
        return _SCALAR_TYPE_CASES[case](m)
    except Exception as e:  # noqa: BLE001 - the exception type is the outcome
        return ("raise", type(e).__name__)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_SCALAR_TYPE_CASES))
def test_reductions_and_elements_return_numpy_scalars_like_pandas(case: str) -> None:
    assert _scalar_type_outcome(fpd, case) == _scalar_type_outcome(pd, case), case


def test_no_test_module_defines_a_top_level_name_twice() -> None:
    """A second top-level `_X_CASES = {...}` or `def _helper` silently
    replaces the first: the earlier group's parametrize list is fixed at
    import, but its outcome function reads the later table, so each case
    raised KeyError in both arms and passed. fvsao.30's 17 everyday-idiom
    cases were vacuous that way from 31cf429d6 until 2026-09-25, and two
    shadowed helpers (_gb_frame, _typed) broke other groups."""
    import ast

    repeated = {}
    for path in sorted(Path(__file__).parent.glob("*.py")):
        names = []
        for node in ast.parse(path.read_text()).body:
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
                names.append(node.name)
            elif isinstance(node, ast.Assign):
                names.extend(t.id for t in node.targets if isinstance(t, ast.Name))
            elif isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name):
                names.append(node.target.id)
        twice = sorted({n for n in names if names.count(n) > 1})
        if twice:
            repeated[path.name] = twice
    assert repeated == {}, repeated


def _texts(r: Any) -> Any:
    """An index / Series / array as its elements' texts (Timestamps print
    their zone), or the value's text."""
    if hasattr(r, "tolist") and not isinstance(r, str):
        return [str(v) for v in r.tolist()]
    return str(r)


def _eastern_range(m: Any, freq: str = "12h", tz: Any = "US/Eastern") -> Any:
    return m.date_range("2024-03-09 12:00", periods=3, freq=freq, tz=tz)


def _eastern_index(m: Any) -> Any:
    """Wall times across the 2024-03-10 DST change, localized (no freq)."""
    return m.DatetimeIndex(["2024-03-09 18:00", "2024-03-10 00:00", "2024-03-10 07:00", "NaT"]).tz_localize("US/Eastern")


# (fvsao.55) A DatetimeIndex carried no time zone: date_range(tz=),
# DatetimeIndex.tz_localize / tz_convert and DatetimeIndex(tz=) were refused,
# an aware column set as the index became naive UTC labels, and the fields,
# repr and elements of an index read the UTC clock.
_TZ_INDEX_CASES = {
    "date_range 12h across DST": lambda m: _texts(_eastern_range(m)),
    "date_range tz and dtype": lambda m: (str(_eastern_range(m).tz), str(_eastern_range(m).dtype)),
    "date_range daily wall clock": lambda m: _texts(_eastern_range(m, "D")),
    "date_range UTC": lambda m: _texts(_eastern_range(m, "12h", "UTC")),
    "date_range fixed offset": lambda m: (_texts(_eastern_range(m, "h", "+05:30")), str(_eastern_range(m, "h", "+05:30").tz)),
    "date_range month end": lambda m: _texts(m.date_range("2024-01-31", periods=3, freq="ME", tz="Europe/Paris")),
    "date_range aware endpoint": lambda m: _texts(m.date_range(m.Timestamp("2024-01-01", tz="Asia/Tokyo"), periods=2, freq="D")),
    "localize": lambda m: _texts(_eastern_index(m)),
    "localize tz": lambda m: str(_eastern_index(m).tz),
    "convert": lambda m: _texts(_eastern_index(m).tz_convert("Asia/Tokyo")),
    "convert None": lambda m: _texts(_eastern_index(m).tz_convert(None)),
    "localize None": lambda m: _texts(_eastern_index(m).tz_localize(None)),
    "constructor tz": lambda m: _texts(m.DatetimeIndex(["2024-01-01 09:00"], tz="US/Eastern")),
    "to_datetime utc": lambda m: str(m.to_datetime(["2024-01-01"], utc=True).tz),
    "repr": lambda m: repr(_eastern_index(m)),
    "repr named UTC": lambda m: repr(m.DatetimeIndex(["2024-01-01"], name="when").tz_localize("UTC")),
    "fields read the wall clock": lambda m: (_texts(_eastern_index(m).hour), _texts(_eastern_index(m).day), _texts(_eastern_index(m).dayofweek)),
    "strftime with offset and name": lambda m: _texts(_eastern_index(m)[:3].strftime("%Y-%m-%d %H:%M %z %Z")),
    "normalize": lambda m: _texts(_eastern_index(m).normalize()),
    "floor to the day": lambda m: _texts(_eastern_index(m)[:3].floor("D")),
    "elements are aware Timestamps": lambda m: [f"{type(v).__name__} {v.tz} {v}" for v in _eastern_index(m)[:3].tolist()],
    "element, min, max": lambda m: (str(_eastern_index(m)[1]), str(_eastern_index(m).min()), str(_eastern_index(m).max())),
    "slice keeps the zone": lambda m: str(_eastern_index(m)[1:].tz),
    "sort_values keeps the zone": lambda m: _texts(_eastern_index(m)[:3][::-1].sort_values()),
    "equals across zones": lambda m: (_eastern_index(m).equals(_eastern_index(m).tz_convert("UTC")), _eastern_index(m).equals(_eastern_index(m).copy())),
    "isin an aware Timestamp": lambda m: _texts(_eastern_index(m).isin([m.Timestamp("2024-03-10 00:00", tz="US/Eastern")])),
    "minus a Timestamp": lambda m: _texts(_eastern_index(m)[:3] - m.Timestamp("2024-03-09 18:00", tz="US/Eastern")),
    "plus a Timedelta": lambda m: _texts(_eastern_index(m)[:3] + m.Timedelta("6h")),
    "plus a DateOffset is wall clock": lambda m: _texts(_eastern_index(m)[:2] + m.DateOffset(days=1)),
    "astype str": lambda m: _texts(_eastern_index(m)[:3].astype(str)),
    "to_series dtype": lambda m: str(_eastern_index(m).to_series().dtype),
    "Series of the index": lambda m: (str(m.Series(_eastern_index(m)).dtype), _texts(m.Series(_eastern_index(m)))),
    "series repr": lambda m: repr(m.Series([1, 2, 3, 4], index=_eastern_index(m))),
    "frame repr": lambda m: repr(m.DataFrame({"v": [1, 2, 3, 4]}, index=_eastern_index(m))),
    "series index tz": lambda m: str(m.Series([1, 2, 3, 4], index=_eastern_index(m)).index.tz),
    "head, sort, filter keep the zone": lambda m: (lambda s: (str(s.head(2).index.tz), str(s.sort_values().index.tz), str(s[s > 1].index.tz)))(m.Series([3, 1, 2, 4], index=_eastern_index(m))),
    "frame filter keeps the zone": lambda m: (lambda d: str(d[d.v > 2].index.tz))(m.DataFrame({"v": [1, 2, 3, 4]}, index=_eastern_index(m))),
    "set_index of an aware column": lambda m: (lambda d: (str(d.index.tz), _texts(d.index)))(m.DataFrame({"t": _eastern_index(m)[:3], "v": [1, 2, 3]}).set_index("t")),
    "reset_index to an aware column": lambda m: (lambda d: (str(d.dtypes.iloc[0]), _texts(d.iloc[:, 0])))(m.DataFrame({"v": [1, 2, 3]}, index=_eastern_index(m)[:3]).reset_index()),
    "Series.tz_convert": lambda m: _texts(m.Series([1, 2, 3], index=_eastern_index(m)[:3]).tz_convert("UTC").index),
    "DataFrame.tz_localize": lambda m: _texts(m.DataFrame({"v": [1]}, index=m.DatetimeIndex(["2024-01-01"])).tz_localize("Asia/Tokyo").index),
    "same zone arithmetic": lambda m: _texts((m.Series([1, 2, 3], index=_eastern_index(m)[:3]) * 2).index),
    "two zones meet in UTC": lambda m: (lambda s: _texts((s + s.tz_convert("Asia/Tokyo")).index))(m.Series([1, 2, 3], index=_eastern_index(m)[:3])),
    "between_time reads the wall clock": lambda m: m.Series([1, 2, 3], index=_eastern_index(m)[:3]).between_time("00:00", "07:00").tolist(),
    "concat keeps a shared zone": lambda m: (lambda s: str(m.concat([s, s]).index.tz))(m.Series([1, 2, 3], index=_eastern_index(m)[:3])),
    "index setter keeps the zone": lambda m: (lambda d: (setattr(d, "index", _eastern_index(m)[:3]), str(d.index.tz))[1])(m.DataFrame({"v": [1, 2, 3]})),
    "shift by a fixed freq keeps the zone": lambda m: _texts(m.Series([1, 2, 3], index=_eastern_index(m)[:3]).shift(1, freq="h").index),
    "unique of an aware column": lambda m: str(m.Series(_eastern_index(m)[:3]).unique()[0]),
    # NEGATIVES (pandas' errors): localizing an aware index, converting a
    # naive one, aware against naive, a skipped wall time, an unknown zone.
    "localize an aware index": lambda m: _eastern_index(m).tz_localize("UTC"),
    "convert a naive index": lambda m: m.DatetimeIndex(["2024-01-01"]).tz_convert("UTC"),
    "aware joined with naive": lambda m: (lambda s: s + s.tz_localize(None))(m.Series([1, 2, 3], index=_eastern_index(m)[:3])),
    "nonexistent wall time": lambda m: m.DatetimeIndex(["2024-03-10 02:30"]).tz_localize("US/Eastern"),
    "ambiguous wall time": lambda m: m.DatetimeIndex(["2024-11-03 01:30"]).tz_localize("US/Eastern"),
    "unknown zone": lambda m: m.DatetimeIndex(["2024-01-01"]).tz_localize("Mars/Base"),
    "minus a naive Timestamp": lambda m: _eastern_index(m) - m.Timestamp("2024-01-01"),
}


def _tz_index_outcome(m: Any, case: str) -> Any:
    try:
        return _TZ_INDEX_CASES[case](m)
    except Exception as e:  # noqa: BLE001 - the exception type is the outcome
        return ("raise", type(e).__name__)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_TZ_INDEX_CASES))
def test_tz_aware_datetime_index_matches_pandas(case: str) -> None:
    assert _tz_index_outcome(fpd, case) == _tz_index_outcome(pd, case), case


def _six_days(m: Any) -> Any:
    return m.date_range("2024-01-01", periods=6, freq="D")


def _freq_of(index: Any) -> Any:
    """An index's freqstr and its offset's repr."""
    return (index.freqstr, str(index.freq))


# (fvsao.61) DatetimeIndex.freq was never tracked: date_range(...).freq /
# freqstr / inferred_freq were None, the repr printed freq=None, a Series
# repr had no 'Freq: ...' line, and DatetimeIndex(freq=) was refused (its
# second positional argument was name). With it: a list of Timestamps built
# an all-NaT DatetimeIndex, a DatetimeIndex could not be compared (so
# df[df.index > '2024-01-01'] raised) or indexed by a mask, df[callable]
# raised, and a number against a datetime column raised instead of pandas'
# invalid comparison.
_FREQ_CASES = {
    **{f"date_range freq {alias}": (lambda alias: lambda m: _freq_of(m.date_range("2024-01-01", periods=3, freq=alias)))(alias)
       for alias in ["D", "2D", "h", "12h", "12H", "min", "30s", "ms", "W", "W-MON", "MS", "ME", "QS", "QE", "YS", "YE", "B", "SME", "BME", "BMS", "M", "T", "1D"]},
    "an invalid alias raises": lambda m: m.date_range("2024-01-01", periods=3, freq="D1"),
    "repr": lambda m: repr(_six_days(m)),
    "repr 12h": lambda m: repr(m.date_range("2024-01-01", periods=2, freq="12h")),
    "repr tz-aware": lambda m: repr(m.date_range("2024-01-01", periods=2, freq="h", tz="UTC")),
    "Series repr footer": lambda m: repr(m.Series([1, 2, 3], index=m.date_range("2024-01-01", periods=3, freq="12h"))),
    "Series repr footer with a name": lambda m: repr(m.Series([1.5, 2.5], index=m.date_range("2024-01-01", periods=2, freq="W"), name="x")),
    "Series to_string footer": lambda m: m.Series([1, 2], index=m.date_range("2024-01-01", periods=2), name="x").to_string(),
    "DataFrame index": lambda m: _freq_of(m.DataFrame({"a": range(6)}, index=_six_days(m)).index),
    "Series index": lambda m: _freq_of(m.Series(range(6), index=_six_days(m)).index),
    "slice": lambda m: _freq_of(_six_days(m)[1:]),
    "slice with a step": lambda m: _freq_of(_six_days(m)[::2]),
    "reversed": lambda m: _freq_of(_six_days(m)[::-1]),
    "integer positions drop it": lambda m: _freq_of(_six_days(m)[[0, 2]]),
    "a mask selecting a run keeps it": lambda m: _freq_of(_six_days(m)[_six_days(m) > "2024-01-02"]),
    "a gapped mask drops it": lambda m: _freq_of(_six_days(m)[np.array([True, False, True, False, False, False])]),
    "head": lambda m: _freq_of(m.Series(range(6), index=_six_days(m)).head(2).index),
    "tail": lambda m: _freq_of(m.Series(range(6), index=_six_days(m)).tail(2).index),
    "iloc slice": lambda m: _freq_of(m.Series(range(6), index=_six_days(m)).iloc[1:4].index),
    "loc slice": lambda m: _freq_of(m.Series(range(6), index=_six_days(m)).loc["2024-01-02":"2024-01-04"].index),
    "sort_values drops it": lambda m: _freq_of(m.Series(range(6), index=_six_days(m)).sort_values(ascending=False).index),
    "sort_index keeps it": lambda m: _freq_of(m.Series(range(6), index=_six_days(m)).sort_index().index),
    "arithmetic keeps it": lambda m: _freq_of((m.Series(range(6), index=_six_days(m)) + 1).index),
    "shift": lambda m: _freq_of(_six_days(m).shift(1, freq="D")),
    "plus a Timedelta": lambda m: _freq_of(_six_days(m) + m.Timedelta("1h")),
    "plus a DateOffset drops it": lambda m: _freq_of(_six_days(m) + m.DateOffset(days=1)),
    "tz_localize UTC": lambda m: _freq_of(_six_days(m).tz_localize("UTC")),
    "tz_convert": lambda m: _freq_of(_six_days(m).tz_localize("UTC").tz_convert("US/Eastern")),
    "normalize": lambda m: _freq_of(m.date_range("2024-01-01 10:00", periods=3, freq="D").normalize()),
    "rename and copy": lambda m: (_freq_of(_six_days(m).rename("t")), _freq_of(_six_days(m).copy())),
    "unique": lambda m: _freq_of(_six_days(m).unique()),
    "append drops it": lambda m: _freq_of(_six_days(m).append(_six_days(m))),
    "union of adjacent runs": lambda m: _freq_of(_six_days(m)[:3].union(_six_days(m)[3:])),
    "from strings": lambda m: _freq_of(m.DatetimeIndex(["2024-01-01", "2024-01-02"])),
    "freq=D": lambda m: _freq_of(m.DatetimeIndex(["2024-01-01", "2024-01-02"], freq="D")),
    "freq=infer": lambda m: _freq_of(m.DatetimeIndex(["2024-01-01", "2024-01-02", "2024-01-03"], freq="infer")),
    "a freq the labels do not follow raises": lambda m: m.DatetimeIndex(["2024-01-01", "2024-01-03"], freq="D"),
    "from Timestamps": lambda m: [str(t) for t in m.DatetimeIndex([m.Timestamp("2024-01-01"), m.Timestamp("2024-01-02")])],
    "from aware Timestamps": lambda m: repr(m.DatetimeIndex([m.Timestamp("2024-01-01", tz="UTC"), m.NaT])),
    **{f"inferred_freq {label}": (lambda dates: lambda m: m.DatetimeIndex(dates).inferred_freq)(dates) for label, dates in [
        ("D", ["2024-01-01", "2024-01-02", "2024-01-03"]),
        ("h", ["2024-01-01 00:00", "2024-01-01 01:00", "2024-01-01 02:00"]),
        ("15min", ["2024-01-01 00:00", "2024-01-01 00:15", "2024-01-01 00:30"]),
        ("MS", ["2024-01-01", "2024-02-01", "2024-03-01"]),
        ("ME", ["2024-01-31", "2024-02-29", "2024-03-31"]),
        ("QS", ["2024-01-01", "2024-04-01", "2024-07-01"]),
        ("QE", ["2024-03-31", "2024-06-30", "2024-09-30"]),
        ("YE", ["2021-12-31", "2022-12-31", "2023-12-31"]),
        ("W", ["2024-01-07", "2024-01-14", "2024-01-21"]),
        ("B", ["2024-01-04", "2024-01-05", "2024-01-08", "2024-01-09"]),
        ("-1D", ["2024-01-03", "2024-01-02", "2024-01-01"]),
        ("irregular", ["2024-01-01", "2024-01-02", "2024-01-04"]),
        ("two labels", ["2024-01-01", "2024-01-02"]),
    ]},
    "resample index": lambda m: _freq_of(m.Series(range(6), index=_six_days(m)).resample("2D").sum().index),
    "resample ME index": lambda m: _freq_of(m.Series(range(6), index=_six_days(m)).resample("ME").sum().index),
    "asfreq": lambda m: _freq_of(m.Series([1, 2], index=m.DatetimeIndex(["2024-01-01", "2024-01-03"])).asfreq("D").index),
    "set_index of a column has none": lambda m: _freq_of(m.DataFrame({"t": _six_days(m), "v": range(6)}).set_index("t").index),
    "freq equals its alias": lambda m: (_six_days(m).freq == "D", _six_days(m).freq == "h"),
    "freq n and name": lambda m: (m.date_range("2024-01-01", periods=3, freq="12h").freq.n, m.date_range("2024-01-01", periods=3, freq="12h").freq.name),
    "equals ignores freq": lambda m: _six_days(m).equals(m.DatetimeIndex(list(_six_days(m)))),
    "compare with a date string": lambda m: (_six_days(m) > "2024-01-02").tolist(),
    "compare with a Timestamp": lambda m: (_six_days(m) <= m.Timestamp("2024-01-02")).tolist(),
    "compare with a datetime": lambda m: (_six_days(m) == datetime.datetime(2024, 1, 3)).tolist(),
    "compare with NaT labels": lambda m: (m.DatetimeIndex(["2024-01-01", None]) != m.Timestamp("2024-01-01")).tolist(),
    "compare two indexes": lambda m: (_six_days(m)[:2] == m.DatetimeIndex(["2024-01-01", "2024-01-05"])).tolist(),
    "compare tz-aware with a string": lambda m: (m.date_range("2024-01-01", periods=3, tz="US/Eastern") > "2024-01-02").tolist(),
    "order tz-aware against naive raises": lambda m: m.date_range("2024-01-01", periods=3, tz="US/Eastern") > m.Timestamp("2024-01-02"),
    "order against a number raises": lambda m: _six_days(m) > 5,
    "equal to a number is all False": lambda m: (_six_days(m) == 5).tolist(),
    "datetime column equal to a number": lambda m: (m.Series(_six_days(m)) != 5).tolist(),
    "compare lengths must match": lambda m: _six_days(m) == m.DatetimeIndex(["2024-01-01"]),
    "filter a frame by its index": lambda m: (lambda d: d[d.index >= "2024-01-04"]["v"].tolist())(m.DataFrame({"v": range(6)}, index=_six_days(m))),
    "filter with a callable": lambda m: m.DataFrame({"v": range(6)}, index=_six_days(m))[lambda d: d.index > "2024-01-04"]["v"].tolist(),
    # shift (it moved every index by a day whatever its freq).
    "shift by a given tick keeps the freq": lambda m: (lambda r: (r.freqstr, [str(t) for t in r]))(_six_days(m)[:3].shift(1, freq="h")),
    "shift by a given calendar offset drops it": lambda m: (lambda r: (r.freqstr, [str(t) for t in r]))(_six_days(m)[:3].shift(1, freq="ME")),
    "shift backwards": lambda m: (lambda r: (r.freqstr, [str(t) for t in r]))(_six_days(m)[:3].shift(-2)),
    "shift an hourly index by its freq": lambda m: (lambda r: (r.freqstr, [str(t) for t in r]))(m.date_range("2024-03-08", periods=3, freq="h").shift(1)),
    "shift without a freq raises": lambda m: m.DatetimeIndex(["2024-01-01", "2024-01-05"]).shift(1),
    "shift without a freq by a given one": lambda m: (lambda r: (r.freqstr, [str(t) for t in r]))(m.DatetimeIndex(["2024-01-01", "2024-01-05"]).shift(1, freq="D")),
    "shift tz-aware by its hours": lambda m: [str(t) for t in m.date_range("2024-03-10 00:00", periods=3, freq="h", tz="US/Eastern").shift(2)],
    "minus a Timedelta from month ends drops it": lambda m: _freq_of(m.date_range("2024-01-31", periods=3, freq="ME") - m.Timedelta("1D")),
    # union sorts (it kept first-seen order) and infers the freq.
    "union sorts": lambda m: (lambda r: ([str(t) for t in r], r.freqstr))(m.DatetimeIndex(["2024-01-03", "2024-01-01"]).union(m.DatetimeIndex(["2024-01-02"]))),
    "union sort=False keeps the order": lambda m: [str(t) for t in m.DatetimeIndex(["2024-01-03", "2024-01-01"]).union(m.DatetimeIndex(["2024-01-02"]), sort=False)],
    "union with an empty index is itself": lambda m: [str(t) for t in m.DatetimeIndex(["2024-01-03", "2024-01-01"]).union(m.DatetimeIndex([]))],
    "union of overlapping runs": lambda m: _freq_of(m.date_range("2024-01-01", periods=3).union(m.date_range("2024-01-02", periods=4))),
    "union of runs with a gap": lambda m: _freq_of(m.date_range("2024-01-01", periods=3).union(m.date_range("2024-01-05", periods=2))),
    "intersection of runs": lambda m: _freq_of(m.date_range("2024-01-01", periods=5).intersection(m.date_range("2024-01-03", periods=5))),
    "intersection with a gap": lambda m: _freq_of(m.date_range("2024-01-01", periods=5).intersection(m.DatetimeIndex(["2024-01-01", "2024-01-03", "2024-01-04"]))),
    # The bead's probe matrix: each freq naive and tz-aware, as built and
    # after [::2], [1:], take, sort, shift(1) (the index's own freq) and
    # + a Timedelta.
    **{f"{alias} {zone or 'naive'} {step}": (lambda alias, zone, step: lambda m: _freq_step(m, alias, zone, step))(alias, zone, step)
       for alias in ["h", "12h", "D", "ME", "W-SUN", "B"]
       for zone in [None, "US/Eastern"]
       for step in ["built", "[::2]", "[1:]", "take", "sort", "shift(1)", "+ Timedelta"]},
}


def _tdr(m: Any, freq: str = "h") -> Any:
    return m.timedelta_range("1D", periods=6, freq=freq)


# (nfc91) TimedeltaIndex.freq was never set (timedelta_range's too) and a
# PeriodIndex's freq was its alias text, not pandas' offset. With it:
# TimedeltaIndex([Timedelta, ...]) raised, freq= was refused, and
# DatetimeIndex / TimedeltaIndex .take() turned an out-of-range position
# into NaT and dropped the freq a constant step keeps.
_TIMEDELTA_FREQ_CASES = {
    **{f"timedelta_range {alias}": (lambda alias: lambda m: (_freq_of(_tdr(m, alias)), repr(_tdr(m, alias)[:3])))(alias)
       for alias in ["h", "30min", "D", "2D", "s"]},
    "slice with a step": lambda m: _freq_of(_tdr(m)[::2]),
    "slice": lambda m: _freq_of(_tdr(m)[1:]),
    "Series repr footer": lambda m: repr(m.Series([1, 2], index=m.timedelta_range("1D", periods=2, freq="h"))),
    "from strings has none": lambda m: _freq_of(m.TimedeltaIndex(["1h", "3h"])),
    "from Timedeltas": lambda m: [str(t) for t in m.TimedeltaIndex([m.Timedelta("1h"), m.NaT, m.Timedelta("2h")])],
    "inferred": lambda m: m.TimedeltaIndex(list(_tdr(m, "30min"))).inferred_freq,
    "inferred uneven": lambda m: m.TimedeltaIndex(["1h", "2h", "4h"]).inferred_freq,
    "inferred two": lambda m: m.TimedeltaIndex(["1h", "2h"]).inferred_freq,
    "inferred whole weeks": lambda m: m.TimedeltaIndex(["7D", "14D", "21D"]).inferred_freq,
    "freq=h": lambda m: _freq_of(m.TimedeltaIndex(["1h", "2h"], freq="h")),
    "freq=infer": lambda m: _freq_of(m.TimedeltaIndex(["1h", "2h", "3h"], freq="infer")),
    "a freq the durations do not follow raises": lambda m: m.TimedeltaIndex(["1h", "3h"], freq="h"),
    **{f"{kind} take {positions}": (lambda kind, positions: lambda m: (_tdr(m) if kind == "tdi" else _six_days(m)).take(positions).freqstr)(kind, positions)
       for kind in ["dti", "tdi"] for positions in ([0, 2], [3, 2], [0, 1, 3], [5, 3, 1])},
    "take out of range raises": lambda m: _six_days(m).take([9]),
    "take negative": lambda m: [str(t) for t in _tdr(m).take([-1, 0])],
    "take keeps the zone and name": lambda m: repr(m.date_range("2024-01-01", periods=3, tz="UTC", name="n").take([2, 0])),
    **{f"period_range {freq} freq": (lambda freq: lambda m: (str(m.period_range("2024-01-01", periods=3, freq=freq).freq), m.period_range("2024-01-01", periods=3, freq=freq).freqstr))(freq)
       for freq in ["M", "D", "Q", "Y", "h", "W", "B", "min"]},
}


def _timedelta_freq_outcome(m: Any, case: str) -> Any:
    try:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            return _TIMEDELTA_FREQ_CASES[case](m)
    except Exception as e:  # noqa: BLE001 - the exception type is the outcome
        return ("raise", type(e).__name__)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_TIMEDELTA_FREQ_CASES))
def test_timedelta_and_period_index_freq_match_pandas(case: str) -> None:
    assert _timedelta_freq_outcome(fpd, case) == _timedelta_freq_outcome(pd, case)


def _freq_step(m: Any, alias: str, zone: Any, step: str) -> Any:
    index = m.date_range("2024-03-08", periods=5, freq=alias, tz=zone)
    if step == "built":
        return (_freq_of(index), index.inferred_freq, repr(index))
    moved = {
        "[::2]": lambda: index[::2],
        "[1:]": lambda: index[1:],
        "take": lambda: index.take([0, 2, 3]),
        "sort": lambda: index.sort_values(ascending=False),
        "shift(1)": lambda: index.shift(1),
        "+ Timedelta": lambda: index + m.Timedelta("1h"),
    }[step]()
    return (_freq_of(moved), [str(t) for t in moved])


def _freq_case_outcome(m: Any, case: str) -> Any:
    try:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            result = _FREQ_CASES[case](m)
    except Exception as e:  # noqa: BLE001 - the exception type is the outcome
        return ("raise", type(e).__name__)
    return result


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_FREQ_CASES))
def test_datetime_index_freq_and_comparisons_match_pandas(case: str) -> None:
    assert _freq_case_outcome(fpd, case) == _freq_case_outcome(pd, case)


def _resample_facts(r: Any) -> Any:
    """A resample result: its index's zone and labels, then its cells."""
    index = (str(getattr(r.index, "tz", None)), [str(v) for v in r.index.tolist()])
    if hasattr(r, "columns"):
        return (index, _frame_facts(r))
    return (index, str(r.dtype), [_missing_or(v) for v in r.tolist()])


def _spring(m: Any) -> Any:
    """3-hourly readings across the 2024-03-10 spring-forward change."""
    return m.Series(range(8), index=m.date_range("2024-03-09 18:00", periods=8, freq="3h", tz="US/Eastern"))


def _fall(m: Any) -> Any:
    """3-hourly readings across the 2024-11-03 fall-back change."""
    return m.Series(range(8), index=m.date_range("2024-11-02 18:00", periods=8, freq="3h", tz="US/Eastern"))


def _uneven(m: Any, dtype: Any = None) -> Any:
    """Naive readings in days of 1, 4, 0 and 2 rows."""
    index = m.DatetimeIndex(["2024-01-01 01:00", "2024-01-02 03:00", "2024-01-02 05:00", "2024-01-02 09:00", "2024-01-02 23:00", "2024-01-04 02:00", "2024-01-04 04:00"])
    return m.Series([1, 2, 3, 4, 5, 6, 7], index=index, name="x", dtype=dtype)


def _paris(m: Any) -> Any:
    return m.Series(range(4), index=m.date_range("2024-01-15", periods=4, freq="20D", tz="Europe/Paris"))


# (fvsao.35) Resampling a tz-aware index was refused (the bins read UTC):
# pandas bins days and calendar rules on the wall clock and sub-day steps
# from the first local midnight, labelled in the zone. With it:
# origin='end' / 'end_day' closed and labelled on the left (pandas: right)
# and end_day was the next midnight even at midnight (pandas: last.ceil);
# get_group sliced every bin as if equally long (an uneven bin's rows were
# wrong, an empty bin returned a neighbour's); transform raised on a name and
# ran a callable on each VALUE (a frame's name reduced whole columns); ohlc
# was always float64 (int / bool / nullable dtypes are kept, ints past 2**53
# rounded).
_RESAMPLE_CASES = {
    "D across spring forward": lambda m: _spring(m).resample("D").sum(),
    "D across fall back": lambda m: _fall(m).resample("D").sum(),
    "D count": lambda m: _spring(m).resample("D").count(),
    "12h from local midnight": lambda m: _spring(m).resample("12h").sum(),
    "6h across fall back": lambda m: _fall(m).resample("6h").sum(),
    "6h origin epoch": lambda m: _spring(m).resample("6h", origin="epoch").sum(),
    "6h origin start": lambda m: _spring(m).resample("6h", origin="start").sum(),
    "5h origin end_day": lambda m: _spring(m).resample("5h", origin="end_day").sum(),
    "6h label right": lambda m: _spring(m).resample("6h", label="right").sum(),
    "6h closed right": lambda m: _spring(m).resample("6h", closed="right").sum(),
    "h first": lambda m: _spring(m).resample("h").first().head(4),
    "2D mean": lambda m: _spring(m).resample("2D").mean(),
    "MS in Paris": lambda m: _paris(m).resample("MS").sum(),
    "ME in Paris": lambda m: _paris(m).resample("ME").sum(),
    "W": lambda m: _spring(m).resample("W").sum(),
    "DataFrame D": lambda m: m.DataFrame({"v": range(8), "w": [1.5] * 8}, index=_spring(m).index).resample("D").sum(),
    "DataFrame 6h mean": lambda m: m.DataFrame({"v": range(8), "w": [1.5] * 8}, index=_spring(m).index).resample("6h").mean(),
    "aware ohlc": lambda m: _spring(m).resample("D").ohlc(),
    "aware asfreq": lambda m: _spring(m).resample("6h").asfreq(),
    "aware apply": lambda m: _spring(m).resample("D").apply(lambda w: w.max() - w.min()),
    "aware agg list": lambda m: _spring(m).resample("D").agg(["sum", "max"]),
    "Grouper freq": lambda m: _spring(m).groupby(m.Grouper(freq="D")).sum(),
    "UTC D": lambda m: m.Series(range(4), index=m.date_range("2024-01-01 20:00", periods=4, freq="3h", tz="UTC")).resample("D").sum(),
    "fixed offset D": lambda m: m.Series(range(4), index=m.date_range("2024-01-01 20:00", periods=4, freq="3h", tz="+05:30")).resample("D").sum(),
    "aware get_group": lambda m: _fall(m).resample("D").get_group(m.Timestamp("2024-11-03", tz="US/Eastern")),
    "aware 6h get_group": lambda m: _fall(m).resample("6h").get_group(m.Timestamp("2024-11-03 00:00", tz="US/Eastern")),
    "aware transform": lambda m: _fall(m).resample("D").transform("sum"),
    "aware 6h transform": lambda m: _fall(m).resample("6h").transform("max"),
    "origin end": lambda m: _uneven(m).resample("10h", origin="end").sum(),
    "origin end_day": lambda m: _uneven(m).resample("10h", origin="end_day").sum(),
    "get_group of an uneven bin": lambda m: _uneven(m).resample("D").get_group(m.Timestamp("2024-01-02")),
    "get_group by string": lambda m: _uneven(m).resample("D").get_group("2024-01-04"),
    "get_group of an empty bin": lambda m: _uneven(m).resample("D").get_group(m.Timestamp("2024-01-03")),
    "get_group of no bin": lambda m: _uneven(m).resample("D").get_group(m.Timestamp("2024-01-09")),
    "DataFrame get_group": lambda m: _uneven(m).to_frame().assign(y=1.5).resample("D").get_group(m.Timestamp("2024-01-02")),
    "transform sum": lambda m: _uneven(m).resample("D").transform("sum"),
    "transform max beside an empty bin": lambda m: _uneven(m).resample("D").transform("max"),
    "transform count": lambda m: _uneven(m).resample("D").transform("count"),
    "transform 12h mean": lambda m: _uneven(m).resample("12h").transform("mean"),
    "transform callable to a scalar": lambda m: _uneven(m).resample("D").transform(lambda w: w.max() - w.min()),
    "transform callable to a Series": lambda m: _uneven(m).resample("D").transform(lambda w: w - w.mean()),
    "transform cumsum": lambda m: _uneven(m).resample("D").transform("cumsum"),
    "DataFrame transform sum": lambda m: _uneven(m).to_frame().assign(y=1.5).resample("D").transform("sum"),
    "DataFrame transform callable": lambda m: _uneven(m).to_frame().assign(y=1.5).resample("D").transform(lambda w: w - w.min()),
    "ohlc of ints": lambda m: _uneven(m).resample("2D").ohlc(),
    "ohlc of ints beside an empty bin": lambda m: _uneven(m).resample("D").ohlc(),
    "ohlc of Int64": lambda m: _uneven(m, "Int64").resample("D").ohlc(),
    "ohlc of bools": lambda m: (_uneven(m) > 3).resample("2D").ohlc(),
    "ohlc of big ints": lambda m: (_uneven(m) + 2**60).resample("2D").ohlc(),
    "DataFrame ohlc": lambda m: _uneven(m).to_frame().assign(y=1.5).resample("2D").ohlc(),
}


def _resample_case_outcome(m: Any, case: str) -> Any:
    try:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            return _resample_facts(_RESAMPLE_CASES[case](m))
    except Exception as e:  # noqa: BLE001 - the exception type is the outcome
        return ("raise", type(e).__name__)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_RESAMPLE_CASES))
def test_resample_bins_rows_and_prices_like_pandas(case: str) -> None:
    assert _resample_case_outcome(fpd, case) == _resample_case_outcome(pd, case)


def _three_days(m: Any, values: Any = (1, 2, 3), dtype: Any = None) -> Any:
    return m.Series(list(values), index=m.date_range("2024-01-01", periods=len(values), freq="D"), dtype=dtype)


def _off_grid(m: Any) -> Any:
    return m.Series([1, 2, 3], index=m.DatetimeIndex(["2024-01-01 01:30", "2024-01-01 03:10", "2024-01-01 05:00"]))


def _sparse(m: Any) -> Any:
    return m.Series([1, 2, 3], index=m.DatetimeIndex(["2024-01-01 00:00", "2024-01-01 06:00", "2024-01-03 12:00"]))


# (4qg5w.4) Upsampling is pandas' reindex onto the bin edges. nearest()
# returned first() (NaN at every edge without a row in its bin) and took no
# limit; ffill / bfill filled first() (a downsample took each bin's FIRST
# row), skipped NaN rows and followed the label; asfreq took Series.asfreq's
# grid from the first timestamp (off the edges, ignoring origin) and filled
# the rows' own NaNs with fill_value; fillna('ffill') raised a cast error.
_UPSAMPLE_CASES = {
    "nearest D to 6h": lambda m: _three_days(m).resample("6h").nearest(),
    "nearest D to 8h": lambda m: _three_days(m).resample("8h").nearest(),
    "nearest tie takes the later row": lambda m: _three_days(m).resample("12h").nearest(),
    "nearest limit 1": lambda m: _three_days(m).resample("6h").nearest(limit=1),
    "nearest limit 2": lambda m: _three_days(m).resample("4h").nearest(limit=2),
    "nearest over a NaN row": lambda m: _three_days(m, (1.5, None, 3.5)).resample("12h").nearest(),
    "nearest off-grid": lambda m: _off_grid(m).resample("h").nearest(),
    "nearest downsample": lambda m: m.Series(range(10), index=m.date_range("2024-01-01", periods=10, freq="h")).resample("3h").nearest(),
    "nearest unsorted": lambda m: m.Series([3, 1, 2], index=m.DatetimeIndex(["2024-01-03", "2024-01-01", "2024-01-02"])).resample("12h").nearest(),
    "nearest label right": lambda m: _three_days(m).resample("8h", label="right").nearest(),
    "nearest closed right": lambda m: _three_days(m).resample("8h", closed="right").nearest(),
    "nearest Int64": lambda m: _three_days(m, (1, None, 3), "Int64").resample("12h").nearest(),
    "nearest strings": lambda m: _three_days(m, ("a", "b", "c")).resample("12h").nearest(),
    "nearest frame with a limit": lambda m: m.DataFrame({"a": [1, 2, 3], "b": [1.5, 2.5, 3.5]}, index=_three_days(m).index).resample("6h").nearest(limit=1),
    "nearest month ends": lambda m: m.Series([1, 2], index=m.DatetimeIndex(["2024-01-31", "2024-03-31"])).resample("ME").nearest(),
    "nearest tz-aware": lambda m: m.Series([1, 2, 3], index=m.date_range("2024-03-09", periods=3, freq="D", tz="US/Eastern")).resample("8h").nearest(),
    "agg nearest": lambda m: _three_days(m).resample("6h").agg("nearest"),
    "nearest limit 0 raises": lambda m: _three_days(m).resample("6h").nearest(limit=0),
    "nearest over a repeated timestamp raises": lambda m: m.Series([1, 2, 3], index=m.DatetimeIndex(["2024-01-01", "2024-01-01", "2024-01-02"])).resample("12h").nearest(),
    "ffill over a NaN row": lambda m: _three_days(m, (1.5, None, 3.5)).resample("12h").ffill(),
    "bfill over a NaN row": lambda m: _three_days(m, (1.5, None, 3.5)).resample("12h").bfill(),
    "ffill limit over a NaN row": lambda m: _three_days(m, (1.5, None, 3.5)).resample("6h").ffill(limit=1),
    "ffill downsample": lambda m: _sparse(m).resample("D").ffill(),
    "ffill downsample limit": lambda m: _sparse(m).resample("D").ffill(limit=1),
    "bfill downsample": lambda m: _sparse(m).resample("D").bfill(),
    "ffill off-grid": lambda m: _off_grid(m).resample("h").ffill(),
    "bfill off-grid": lambda m: _off_grid(m).resample("h").bfill(),
    "ffill label right": lambda m: _three_days(m).resample("8h", label="right").ffill(),
    "bfill closed right": lambda m: _three_days(m).resample("8h", closed="right").bfill(),
    "ffill weekly": lambda m: m.Series([1, 2], index=m.DatetimeIndex(["2024-01-07", "2024-01-21"])).resample("W").ffill(),
    "ffill Int64 with a limit": lambda m: m.Series([1, 2], index=m.DatetimeIndex(["2024-01-01", "2024-01-04"]), dtype="Int64").resample("D").ffill(limit=1),
    "ffill frame": lambda m: m.DataFrame({"a": [1, 2, 3], "b": ["x", "y", "z"]}, index=_off_grid(m).index).resample("h").ffill(),
    "ffill tz-aware": lambda m: m.Series([1, 2, 3], index=m.date_range("2024-03-09 22:00", periods=3, freq="3h", tz="US/Eastern")).resample("h").ffill(),
    "ffill limit 0 raises": lambda m: _three_days(m).resample("6h").ffill(limit=0),
    "asfreq off-grid": lambda m: _off_grid(m).resample("h").asfreq(),
    "asfreq origin epoch": lambda m: _off_grid(m).resample("7h", origin="epoch").asfreq(),
    "asfreq label right": lambda m: _three_days(m).resample("8h", label="right").asfreq(),
    "asfreq fill_value keeps a row's NaN": lambda m: _three_days(m, (1.0, None, 3.0)).resample("12h").asfreq(fill_value=0),
    "asfreq fill_value keeps ints": lambda m: m.Series([1, 2], index=m.DatetimeIndex(["2024-01-01", "2024-01-03"])).resample("D").asfreq(fill_value=0),
    "fillna ffill": lambda m: _three_days(m, (1.0, None, 3.0)).resample("12h").fillna("ffill"),
    "fillna nearest": lambda m: _three_days(m, (1.0, None, 3.0)).resample("12h").fillna("nearest"),
    "fillna an unknown method raises": lambda m: _three_days(m).resample("12h").fillna("foo"),
}


def _upsample_outcome(m: Any, case: str) -> Any:
    try:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            return _resample_facts(_UPSAMPLE_CASES[case](m))
    except Exception as e:  # noqa: BLE001 - the exception type is the outcome
        return ("raise", type(e).__name__)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_UPSAMPLE_CASES))
def test_resample_upsampling_reindexes_onto_the_bin_edges_like_pandas(case: str) -> None:
    assert _upsample_outcome(fpd, case) == _upsample_outcome(pd, case)


_STRFTIME_FORMATS = [
    "%B %d, %Y",
    "%a %b %e %H:%M",
    "%A %j %U %W %w",
    "%I:%M %p",
    "%y-%m-%d %f",
    "%c",
    "%x %X",
    "%G-W%V-%u",
    "100%% %Y",
    "%Q %Y",
    "%Y-%m-%dT%H:%M:%S%z|%Z",
    "%-d/%-m %^a",
]


def _strftime_series(m: Any) -> Any:
    return m.Series(m.to_datetime(["2024-03-05 14:07:09.123456", "2023-12-31 00:00:00", None], format="ISO8601"))


def _strftime_outcome(m: Any, target: str, fmt: str) -> Any:
    try:
        if target == "Timestamp":
            return m.Timestamp("2024-03-05 14:07:09.123456").strftime(fmt)
        if target == "aware Timestamp":
            return m.Timestamp("2024-03-05 14:07:09.123456", tz="US/Eastern").strftime(fmt)
        if target == "Series.dt":
            return [_missing_or(v) for v in _strftime_series(m).dt.strftime(fmt).tolist()]
        if target == "DatetimeIndex":
            return m.DatetimeIndex(["2024-03-05 14:07:09.123456", "2023-01-01 00:00:00"]).strftime(fmt).tolist()
        if target == "aware Series.dt":
            return [_missing_or(v) for v in _strftime_series(m).dt.tz_localize("US/Eastern").dt.strftime(fmt).tolist()]
        raise AssertionError(target)
    except Exception as e:  # noqa: BLE001 - the exception type is the outcome
        return ("raise", type(e).__name__)


# (strftime) Series.dt.strftime and Timestamp.strftime knew only %Y %m %d
# %H %M %S (%f): '%B %d' printed '%B 05', %% printed '%%' and a tz-aware
# column formatted the UTC clock; DatetimeIndex.strftime (chrono) printed %f
# with nine digits and PANICKED on an unknown directive such as %Q.
@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("target", ["Timestamp", "aware Timestamp", "Series.dt", "DatetimeIndex", "aware Series.dt"])
@pytest.mark.parametrize("fmt", _STRFTIME_FORMATS)
def test_strftime_follows_python_directives_everywhere(target: str, fmt: str) -> None:
    assert _strftime_outcome(fpd, target, fmt) == _strftime_outcome(pd, target, fmt)


def _aware_column(m: Any) -> Any:
    return m.Series(m.to_datetime(["2024-03-09 23:30", "2024-03-10 12:15"])).dt.tz_localize("US/Eastern")


# (fvsao.35) A tz-aware COLUMN read its UTC clock in round / floor /
# normalize / to_period, a Series of aware Timestamps (or aware datetimes,
# refused) was naive, min / max / astype(str) / value_counts came back naive,
# comparing with an aware Timestamp raised a dtype mismatch, and fillna with
# an aware Timestamp was an invalid cast; value_counts labelled datetimes and
# durations with their text.
_AWARE_COLUMN_CASES = {
    "dt.round": lambda m: _texts_of(_aware_column(m).dt.round("h")),
    "dt.floor to the day": lambda m: _texts_of(_aware_column(m).dt.floor("D")),
    "dt.normalize": lambda m: _texts_of(_aware_column(m).dt.normalize()),
    # The periods' values (the wall-clock days); their period[D] dtype is a
    # separate gap for naive columns too (fvsao.62).
    "dt.to_period": lambda m: [str(v) for v in _aware_column(m).dt.to_period("D").tolist()],
    "dt.day_name": lambda m: _aware_column(m).dt.day_name().tolist(),
    "min and max": lambda m: (str(_aware_column(m).min()), str(_aware_column(m).max())),
    "astype str": lambda m: _aware_column(m).astype(str).tolist(),
    "sort descending": lambda m: _texts_of(_aware_column(m).sort_values(ascending=False)),
    "compare with an aware Timestamp": lambda m: (_aware_column(m) > m.Timestamp("2024-03-10", tz="US/Eastern")).tolist(),
    "compare with a date string": lambda m: (_aware_column(m) > "2024-03-10").tolist(),
    "equal to a naive Timestamp": lambda m: (_aware_column(m) == m.Timestamp("2024-03-10 12:15")).tolist(),
    "order against a naive Timestamp": lambda m: (_aware_column(m) > m.Timestamp("2024-03-10")).tolist(),
    "Series of aware Timestamps": lambda m: (lambda s: (str(s.dtype), _texts_of(s)))(m.Series([m.Timestamp("2024-01-01", tz="US/Eastern"), m.NaT, m.Timestamp("2024-07-01", tz="US/Eastern")])),
    "Series of aware datetimes": lambda m: (lambda s: (str(s.dtype), _texts_of(s)))(m.Series([datetime.datetime(2024, 1, 1, tzinfo=datetime.timezone.utc)])),
    "DataFrame of aware Timestamps": lambda m: str(m.DataFrame({"t": [m.Timestamp("2024-01-01", tz="Asia/Tokyo")]})["t"].dtype),
    "Timestamp of an aware datetime": lambda m: (lambda ts: (str(ts), str(ts.tz)))(m.Timestamp(datetime.datetime(2024, 1, 1, 9, tzinfo=datetime.timezone.utc))),
    "fillna with an aware Timestamp": lambda m: (lambda s: _texts_of(s.fillna(m.Timestamp("2024-02-01", tz="US/Eastern"))))(m.Series([m.Timestamp("2024-01-01", tz="US/Eastern"), m.NaT])),
    "value_counts of an aware column": lambda m: _texts_of(_aware_column(m).value_counts().index),
    "value_counts of datetimes": lambda m: (lambda r: (type(r.index).__name__, _texts_of(r.index), r.tolist()))(m.Series(m.to_datetime(["2024-01-02", "2024-01-01", "2024-01-02"])).value_counts()),
    "value_counts of durations": lambda m: (lambda r: (type(r.index).__name__, _texts_of(r.index)))(m.Series(m.to_timedelta(["1D", "2h", "1D"])).value_counts()),
    "value_counts repr": lambda m: repr(m.Series(m.to_datetime(["2024-01-02", "2024-01-01", "2024-01-02"])).value_counts()),
}


def _aware_column_outcome(m: Any, case: str) -> Any:
    try:
        return _AWARE_COLUMN_CASES[case](m)
    except Exception as e:  # noqa: BLE001 - the exception type is the outcome
        return ("raise", type(e).__name__)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_AWARE_COLUMN_CASES))
def test_tz_aware_columns_follow_their_wall_clock_like_pandas(case: str) -> None:
    assert _aware_column_outcome(fpd, case) == _aware_column_outcome(pd, case), case


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.xfail(strict=True, reason="fvsao.60: Timestamps of two zones make pandas an object column of each Timestamp; fp keeps naive UTC instants (no object-Timestamp cells)")
def test_series_of_two_zones_is_an_object_column_like_pandas() -> None:
    def dtype(m: Any) -> str:
        return str(m.Series([m.Timestamp("2024-01-01", tz="UTC"), m.Timestamp("2024-01-01", tz="Asia/Tokyo")]).dtype)

    assert dtype(fpd) == dtype(pd)


# br-frankenpandas-rc0923-epic-python-honest-dropin-fvsao.18: a default index
# is pandas' RangeIndex. Every Series and DataFrame built without an index,
# reset_index, a concat with ignore_index, a merge and a reader handed back
# a plain Index of 0..n; RangeIndex itself stood apart from Index, so on it a
# comparison was one bool, `.values` a list, and arithmetic or a list of
# positions raised.
def _range_view(x: Any) -> Any:
    if type(x).__name__ == "ndarray":
        return ("ndarray", str(x.dtype), x.tolist())
    if x is None or isinstance(x, (bool, int, float, str, tuple, list)):
        return x
    return (type(x).__name__, repr(x))


def _rng_series(m: Any) -> Any:
    return m.Series([10, 20, 30, 40])


def _rng_frame(m: Any) -> Any:
    return m.DataFrame({"a": [1, 2, 3, 4], "b": list("wxyw")})


def _rng_named(m: Any) -> Any:
    return m.Series([1, 2, 3], index=m.RangeIndex(2, 11, 3, name="k")).index


def _renamed_owner(m: Any) -> Any:
    frame = _rng_frame(m)
    frame.index.name = "id"
    return list(frame.reset_index().columns)


_RANGE_INDEX_CASES = {
    "Series default": lambda m: _rng_series(m).index,
    "DataFrame default": lambda m: _rng_frame(m).index,
    "explicit labels stay an Index": lambda m: m.Series([1, 2], index=[0, 1]).index,
    "index=RangeIndex keeps its stop": lambda m: _rng_named(m),
    "index=range": lambda m: m.Series([1, 2], index=range(1, 4, 2)).index,
    "Series of a Series": lambda m: m.Series(_rng_series(m)).index,
    "DataFrame of a DataFrame": lambda m: m.DataFrame(_rng_frame(m)).index,
    "DataFrame of Series": lambda m: m.DataFrame({"a": _rng_series(m), "b": _rng_series(m)}).index,
    "Series of a dict": lambda m: m.Series({"x": 1, "y": 2}).index,
    "empty Series": lambda m: m.Series([], dtype="float64").index,
    **{f"iloc[{text}]": (lambda key: lambda m: _rng_series(m).iloc[key].index)(key)
       for text, key in [("1:3", slice(1, 3)), ("::2", slice(None, None, 2)), ("1::2", slice(1, None, 2)),
                         ("::-1", slice(None, None, -1)), ("::-2", slice(None, None, -2)), ("3:1", slice(3, 1)),
                         ("5:", slice(5, None))]},
    "slice of a slice": lambda m: _rng_series(m).iloc[1:4].iloc[::2].index,
    "s[::2]": lambda m: _rng_series(m)[::2].index,
    "df[::2]": lambda m: _rng_frame(m)[::2].index,
    "df.iloc[1::2]": lambda m: _rng_frame(m).iloc[1::2].index,
    "df.iloc[1:3, 0]": lambda m: _rng_frame(m).iloc[1:3, 0].index,
    "df.iloc[::2, [0]]": lambda m: _rng_frame(m).iloc[::2, [0]].index,
    "head": lambda m: _rng_series(m).head(2).index,
    "tail": lambda m: _rng_series(m).tail(2).index,
    "mask is an Index": lambda m: (lambda s: s[s > 10].index)(_rng_series(m)),
    "take is an Index": lambda m: _rng_series(m).take([0, 1, 2, 3]).index,
    "iloc list is an Index": lambda m: _rng_series(m).iloc[[0, 2]].index,
    "sort_values sorted": lambda m: _rng_series(m).sort_values().index,
    "sort_values reorders": lambda m: _rng_series(m).sort_values(ascending=False).index,
    "sort_index": lambda m: _rng_series(m).sort_index().index,
    "reversed sort_index descending": lambda m: _rng_series(m).iloc[::-1].sort_index(ascending=False).index,
    "df sort_values by two sorted": lambda m: m.DataFrame({"a": [1, 2], "b": [3, 4]}).sort_values(["a", "b"]).index,
    "reset_index": lambda m: m.Series([1, 2], index=["a", "b"]).reset_index().index,
    "concat ignore_index": lambda m: m.concat([_rng_series(m), _rng_series(m)], ignore_index=True).index,
    "concat frames ignore_index": lambda m: m.concat([_rng_frame(m), _rng_frame(m)], ignore_index=True).index,
    "merge": lambda m: _rng_frame(m).merge(m.DataFrame({"a": [1, 2, 5], "c": [7, 8, 9]}), on="a").index,
    "merge left": lambda m: _rng_frame(m).merge(m.DataFrame({"a": [1, 2, 5], "c": [7, 8, 9]}), on="a", how="left").index,
    "join on the index": lambda m: _rng_frame(m).join(m.DataFrame({"c": [7, 8, 9]})).index,
    "read_csv": lambda m: m.read_csv(io.StringIO("a,b\n1,2\n3,4\n")).index,
    "from_records": lambda m: m.DataFrame.from_records([{"a": 1}, {"a": 2}]).index,
    "groupby as_index=False": lambda m: _rng_frame(m).groupby("b", as_index=False).sum().index,
    "melt": lambda m: _rng_frame(m).melt(id_vars="b").index,
    "mode": lambda m: _rng_series(m).mode().index,
    "arithmetic keeps it": lambda m: (_rng_series(m) + 1).index,
    "name writes through": _renamed_owner,
    "isinstance Index": lambda m: isinstance(_rng_series(m).index, m.Index),
    "values": lambda m: _rng_series(m).index.values,
    "== scalar": lambda m: _rng_series(m).index == 2,
    "== list": lambda m: _rng_series(m).index == [0, 5, 2, 3],
    "< scalar": lambda m: _rng_series(m).index < 2,
    "+ 1": lambda m: _rng_named(m) + 1,
    "* 2": lambda m: _rng_named(m) * 2,
    "1 - range": lambda m: 1 - _rng_named(m),
    "- range": lambda m: -_rng_named(m),
    "/ 2": lambda m: _rng_named(m) / 2,
    "// 2": lambda m: _rng_named(m) // 2,
    "+ an Index": lambda m: _rng_series(m).index + m.Index([1, 1, 1, 1]),
    "positions": lambda m: _rng_series(m).index[[0, 2]],
    "mask": lambda m: (lambda idx: idx[idx > 1])(_rng_series(m).index),
    "[::-1]": lambda m: _rng_named(m)[::-1],
    "sort_values descending": lambda m: _rng_named(m).sort_values(ascending=False),
    "union of ranges": lambda m: m.RangeIndex(4).union(m.RangeIndex(2, 6)),
    "union with an Index": lambda m: m.RangeIndex(4).union(m.Index([7, 9])),
    "intersection": lambda m: m.RangeIndex(4).intersection(m.RangeIndex(2, 6)),
    "append a range": lambda m: m.RangeIndex(4).append(m.RangeIndex(4, 6)),
    "append an Index": lambda m: m.RangeIndex(4).append(m.Index([9])),
    "delete first": lambda m: m.RangeIndex(4).delete(0),
    "delete inside": lambda m: m.RangeIndex(4).delete(1),
    "insert at the end": lambda m: m.RangeIndex(4).insert(4, 4),
    "insert inside": lambda m: m.RangeIndex(4).insert(1, 9),
    "rename": lambda m: m.RangeIndex(3).rename("z"),
    "identical to its Index": lambda m: m.RangeIndex(4).identical(m.Index([0, 1, 2, 3])),
    "identical to itself": lambda m: m.RangeIndex(4).identical(m.RangeIndex(4)),
    "equals its Index": lambda m: m.RangeIndex(4).equals(m.Index([0, 1, 2, 3])),
    "get_loc missing": lambda m: m.RangeIndex(4).get_loc(9),
    "Index get_loc missing": lambda m: m.Index([1, 2]).get_loc(9),
    "reversed": lambda m: [int(v) for v in reversed(_rng_named(m))],
    "inferred_type": lambda m: _rng_named(m).inferred_type,
    "2.0 in range": lambda m: 2.0 in m.RangeIndex(4),
    "droplevel raises": lambda m: m.RangeIndex(4).droplevel(0),
    "Index droplevel raises": lambda m: m.Index([1, 2]).droplevel(0),
    "value_counts keeps the name": lambda m: repr(_rng_named(m).value_counts()),
    "Index of a numpy array": lambda m: m.Index(np.array([1, 2])),
}


def _range_index_outcome(m: Any, case: str) -> Any:
    try:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            return _range_view(_RANGE_INDEX_CASES[case](m))
    except Exception as e:  # noqa: BLE001 - the exception type is the outcome
        return ("raise", type(e).__name__, str(e) if isinstance(e, KeyError) else "")


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
@pytest.mark.parametrize("case", list(_RANGE_INDEX_CASES))
def test_default_index_is_a_range_index_like_pandas(case: str) -> None:
    assert _range_index_outcome(fpd, case) == _range_index_outcome(pd, case), case
