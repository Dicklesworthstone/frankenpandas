"""
Differential conformance test harness for frankenpandas vs pandas oracle on packet fixtures.
"""

from __future__ import annotations

import glob
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

    # 4. explode list of columns and ignore_index
    edata = {"A": ["1,2", "3,4"], "B": [10, 20]}
    df_e_fp = fpd.DataFrame(edata)
    df_e_pd = pd.DataFrame(edata)

    exp_fp_single = df_e_fp.explode("A")
    assert exp_fp_single.shape == (4, 2)
    assert list(exp_fp_single["A"]) == ["1", "2", "3", "4"]

    exp_fp_list = df_e_fp.explode(["A"], ignore_index=True)
    assert exp_fp_list.shape == (4, 2)
    assert list(exp_fp_list.index) == [0, 1, 2, 3]

    ser_e_fp = fpd.Series(["x,y", "z,w"])
    ser_exp_fp = ser_e_fp.explode(ignore_index=True)
    assert list(ser_exp_fp.to_list()) == ["x", "y", "z", "w"]
    assert list(ser_exp_fp.index) == [0, 1, 2, 3]


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


_READ_CSV_CASES = {
    "path": lambda m, p: m.read_csv(_csv_path(p, _CSV)),
    "stringio": lambda m, p: m.read_csv(io.StringIO(_CSV)),
    "bytesio": lambda m, p: m.read_csv(io.BytesIO(_CSV.encode())),
    "quoted_delimiter_newline": lambda m, p: m.read_csv(io.StringIO(_CSV_QUOTED)),
    "open_text_file": lambda m, p: m.read_csv(open(_csv_path(p, _CSV))),  # noqa: SIM115
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
    with pytest.raises(NotImplementedError, match="keys"):
        fpd.concat([frame, frame], keys=["p", "q"])
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
@pytest.mark.xfail(strict=True, reason="br-frankenpandas-hrxn9: _merge is object, pandas category")
def test_merge_indicator_dtype_is_category_like_pandas() -> None:
    merged = fpd.DataFrame(_ML).merge(fpd.DataFrame(_MR), on="k", how="outer", indicator=True)
    assert str(merged["_merge"].dtype) == "category"


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
    with pytest.raises(NotImplementedError, match="category"):
        fpd.Series(["a", "b"]).astype("category")


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


