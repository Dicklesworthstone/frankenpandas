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

    # Row-wise function returning dict -> DataFrame
    def row_transform(row):
        return {"sum": row["a"] + row["b"], "diff": row["b"] - row["a"]}

    df_out = df.apply(row_transform, axis=1)
    assert isinstance(df_out, fpd.DataFrame)
    assert list(df_out["sum"].values) == [11, 22, 33]
    assert list(df_out["diff"].values) == [9, 18, 27]


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
    with pytest.raises(NotImplementedError, match="converters"):
        fpd.read_csv(io.StringIO(_CSV), converters={"a": str})
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
def test_tz_methods_refuse_instead_of_returning_the_input() -> None:
    naive = fpd.Series([1, 2], index=fpd.date_range("2024-01-01", periods=2, freq="D"))
    with pytest.raises(NotImplementedError, match="tz_localize"):
        naive.tz_localize("UTC")
    # pandas raises the same TypeError for a tz-naive index.
    with pytest.raises(TypeError, match="Cannot convert tz-naive timestamps"):
        naive.tz_convert("UTC")
    with pytest.raises(TypeError, match="Cannot convert tz-naive timestamps"):
        pd.Series([1, 2], index=pd.date_range("2024-01-01", periods=2, freq="D")).tz_convert("UTC")
    assert naive.tz_localize(None).tolist() == [1, 2]


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
    for kw in ({}, {"sep": ";"}, {"na_rep": "NA"}, {"header": False}, {"index": False},
               {"index_label": "idx"}, {"columns": ["s"]}):
        assert fdf.to_csv(**kw) == pdf.to_csv(**kw), kw
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
    # NEGATIVE: keywords the writer cannot honour raise instead of vanishing.
    for kw in ({"float_format": "%.1f"}, {"quoting": 1}, {"decimal": ","}):
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
        lambda: fpd.crosstab(fpd.Series(["a"]), fpd.Series(["b"]), values=fpd.Series([1]), aggfunc="sum"),
        lambda: _hd(fpd).rolling(2, center=True),
        lambda: _hd(fpd).groupby("a").value_counts(normalize=True),
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
    # compared with pandas in test_groupby_by_array_like_keys_matches_pandas.)
    for call in (
        lambda: _gb_frame(fpd).groupby("k", dropna=False)["a"],
        lambda: _gb_frame(fpd).groupby("k", group_keys=False),
        lambda: fpd.Series(series).groupby([1, 1, 2], dropna=False),
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
    with pytest.raises(NotImplementedError, match="append"):
        _dr(fpd).set_index("a", append=True)
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
@pytest.mark.xfail(strict=True, reason="br-frankenpandas-1tkrg: the Index repr omits dtype=")
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
@pytest.mark.parametrize("agg", ["sum", "mean", "count"])
def test_resample_bins_are_timestamps_like_pandas(freq: str, agg: str) -> None:
    # br-frankenpandas-0yilt: the bins came back as date STRINGS ('2024-01-01');
    # pandas returns a DatetimeIndex of Timestamps.
    def bins(m: Any) -> Any:
        idx = m.to_datetime(
            ["2024-01-01 00:00", "2024-01-01 06:00", "2024-01-02 00:00", "2024-01-05 00:00"]
        )
        out = getattr(m.Series([1.0, 2.0, 3.0, 4.0], index=idx, name="v").resample(freq), agg)()
        return (
            [type(t).__name__ for t in out.index],
            [str(t) for t in out.index],
            [_marker(v) for v in out.tolist()],
        )

    got, want = bins(fpd), bins(pd)
    assert got == want
    # NEGATIVE: not one bin is text.
    assert set(got[0]) == {"Timestamp"}


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
    # observed=False (pandas' default) adds the unused categories as groups;
    # grouping by value would drop them, so it raises instead.
    frame = fpd.DataFrame({"c": fpd.Categorical(["x"], categories=["x", "y"]), "v": [1]})
    with pytest.raises(NotImplementedError, match="observed"):
        frame.groupby("c")["v"].sum()
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
    # pandas builds a datetime64[ns, UTC] column from tz-aware datetimes; the
    # binding's columns built from Python objects are tz-naive, so it refuses
    # instead of dropping the zone.
    aware = [datetime.datetime(2020, 1, 1, tzinfo=datetime.timezone.utc)]
    assert str(pd.Series(aware).dtype) == "datetime64[ns, UTC]"
    with pytest.raises(NotImplementedError, match="tz-aware"):
        fpd.Series(aware)


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
    with pytest.raises(NotImplementedError):
        _dated_sales(fpd).groupby(["k", "p"])["v"].sum().unstack(level=0)


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
        return None if v is pd.NA or (isinstance(v, float) and math.isnan(v)) else v

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
