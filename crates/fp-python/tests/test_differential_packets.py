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
    # __version__
    assert hasattr(fpd, "__version__")
    assert fpd.__version__ == "0.2.0"

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



