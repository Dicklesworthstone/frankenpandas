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

