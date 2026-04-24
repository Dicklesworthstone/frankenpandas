#!/usr/bin/env python3
"""
FrankenPandas live oracle adapter.

Reads a JSON request from stdin and emits a normalized JSON response to stdout.
This script is strict by default when --strict-legacy is provided:
- It MUST import pandas with legacy source path precedence.
- It fails closed on import/runtime errors.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib
import io
import json
import math
import os
import struct
import sys
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Any


@dataclass
class OracleError(Exception):
    message: str

    def __str__(self) -> str:
        return self.message


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="FrankenPandas pandas oracle adapter")
    parser.add_argument("--legacy-root", required=True, help="Path to legacy pandas root")
    parser.add_argument(
        "--strict-legacy",
        action="store_true",
        help="Fail closed if legacy-root import path cannot be used",
    )
    parser.add_argument(
        "--allow-system-pandas-fallback",
        action="store_true",
        help="Allow fallback to system pandas if strict legacy import fails",
    )
    return parser.parse_args()


def base_oracle_response() -> dict[str, Any]:
    return {
        "expected_series": None,
        "expected_join": None,
        "expected_frame": None,
        "expected_alignment": None,
        "expected_bool": None,
        "expected_positions": None,
        "expected_scalar": None,
        "expected_dtype": None,
        "fixture_provenance": None,
        "error": None,
    }


def oracle_script_sha256() -> str:
    with open(__file__, "rb") as script_handle:
        return hashlib.sha256(script_handle.read()).hexdigest()


def build_fixture_provenance(pd_mod: Any) -> dict[str, str]:
    return {
        "pandas_version": str(pd_mod.__version__),
        "oracle_script_sha256": oracle_script_sha256(),
        "generated_at": datetime.now(timezone.utc)
        .replace(microsecond=0)
        .isoformat()
        .replace("+00:00", "Z"),
    }


def error_response(message: str, pd_mod: Any | None = None) -> dict[str, Any]:
    response = base_oracle_response()
    if pd_mod is not None:
        response["fixture_provenance"] = build_fixture_provenance(pd_mod)
    response["error"] = message
    return response


def setup_pandas(args: argparse.Namespace):
    def validate_pandas_module(pd_mod: Any) -> None:
        required_attrs = ("Series", "DataFrame", "Index")
        missing = [name for name in required_attrs if not hasattr(pd_mod, name)]
        if missing:
            raise OracleError(
                f"imported pandas module missing required attributes: {', '.join(missing)}"
            )

    legacy_root = os.path.abspath(args.legacy_root)
    candidate_parent = os.path.dirname(legacy_root)
    if os.path.isdir(candidate_parent):
        sys.path.insert(0, candidate_parent)

    try:
        import pandas as pd  # type: ignore

        validate_pandas_module(pd)
        return pd
    except Exception as exc:
        if args.strict_legacy and not args.allow_system_pandas_fallback:
            raise OracleError(
                f"strict legacy pandas import failed from {legacy_root}: {exc}"
            ) from exc

        try:
            # Remove legacy path and cached module, then resolve system pandas.
            while candidate_parent in sys.path:
                sys.path.remove(candidate_parent)
            sys.modules.pop("pandas", None)
            pd = importlib.import_module("pandas")

            validate_pandas_module(pd)
            return pd
        except Exception as fallback_exc:
            raise OracleError(f"system pandas import failed: {fallback_exc}") from fallback_exc


def label_from_json(value: dict[str, Any]) -> Any:
    kind = value.get("kind")
    raw = value.get("value")
    if kind == "bool":
        return bool(raw)
    if kind == "int64":
        return int(raw)
    if kind == "float64":
        return float(raw)
    if kind == "utf8":
        return str(raw)
    raise OracleError(f"unsupported index label kind: {kind!r}")


def scalar_from_json(value: dict[str, Any]) -> Any:
    kind = value.get("kind")
    raw = value.get("value")
    if kind == "null":
        marker = str(raw)
        if marker in {"nan", "na_n"}:
            return float("nan")
        return None
    if kind == "bool":
        return bool(raw)
    if kind == "int64":
        return int(raw)
    if kind == "float64":
        return float(raw)
    if kind == "utf8":
        return str(raw)
    raise OracleError(f"unsupported scalar kind: {kind!r}")


def scalar_to_json(value: Any) -> dict[str, Any]:
    if hasattr(value, "item") and callable(value.item):
        try:
            value = value.item()
        except Exception:
            pass
    if value is None:
        return {"kind": "null", "value": "null"}
    if isinstance(value, bool):
        return {"kind": "bool", "value": value}
    if isinstance(value, int):
        return {"kind": "int64", "value": value}
    if isinstance(value, float):
        if math.isnan(value):
            return {"kind": "null", "value": "na_n"}
        return {"kind": "float64", "value": value}
    return {"kind": "utf8", "value": str(value)}


def label_to_json(value: Any) -> dict[str, Any]:
    if isinstance(value, bool):
        return {"kind": "bool", "value": value}
    if isinstance(value, int):
        return {"kind": "int64", "value": value}
    return {"kind": "utf8", "value": str(value)}


def tuple_label_to_flat_string(values: tuple[Any, ...]) -> str:
    return "|".join(str(value) for value in values)


def multiindex_from_json(pd, raw: dict[str, Any]):
    tuples_raw = raw.get("tuples")
    if not isinstance(tuples_raw, list):
        raise OracleError("row_multiindex.tuples must be a list")

    tuples: list[tuple[Any, ...]] = []
    for position, tuple_raw in enumerate(tuples_raw):
        if not isinstance(tuple_raw, list):
            raise OracleError(
                f"row_multiindex.tuples[{position}] must be a list of labels"
            )
        tuples.append(tuple(label_from_json(item) for item in tuple_raw))

    names_raw = raw.get("names", [])
    if not isinstance(names_raw, list):
        raise OracleError("row_multiindex.names must be a list when provided")
    names = [None if name is None else str(name) for name in names_raw]
    return pd.MultiIndex.from_tuples(tuples, names=names or None)


def multiindex_to_json(index) -> dict[str, Any]:
    return {
        "tuples": [
            [label_to_json(value) for value in values] for values in index.tolist()
        ],
        "names": [None if name is None else str(name) for name in index.names],
    }


def scalar_is_missing(value: Any) -> bool:
    return value is None or (isinstance(value, float) and math.isnan(value))


def encode_groupby_key_component(value: Any) -> str:
    if isinstance(value, bool):
        return f"b:{str(value).lower()}"
    if isinstance(value, int):
        return f"i:{value}"
    if isinstance(value, float):
        if math.isnan(value):
            raise OracleError("groupby composite key component cannot be NaN")
        bits = struct.unpack(">Q", struct.pack(">d", value))[0]
        return f"f_bits:{bits:016x}"
    escaped = json.dumps(str(value), ensure_ascii=False, separators=(",", ":"))
    return f"s:{escaped}"


def encode_groupby_composite_key(values: list[Any]) -> str:
    return "|".join(encode_groupby_key_component(value) for value in values)


def build_groupby_composite_key_series(
    pd, payload: dict[str, Any], value_index: list[Any]
) -> tuple[Any, list[Any]]:
    groupby_keys = payload.get("groupby_keys")
    if not isinstance(groupby_keys, list) or not groupby_keys:
        raise OracleError(
            "groupby_keys must be a non-empty list for multi-key groupby payloads"
        )

    union_index: list[Any] = []
    seen_labels: set[Any] = set()
    key_maps: list[dict[Any, Any]] = []

    for key_payload in groupby_keys:
        key_idx = [label_from_json(item) for item in key_payload["index"]]
        key_vals = [scalar_from_json(item) for item in key_payload["values"]]
        if len(key_idx) != len(key_vals):
            raise OracleError(
                "groupby_keys index/value length mismatch in multi-key payload"
            )

        for label in key_idx:
            if label not in seen_labels:
                seen_labels.add(label)
                union_index.append(label)

        first_map: dict[Any, Any] = {}
        for label, value in zip(key_idx, key_vals):
            first_map.setdefault(label, value)
        key_maps.append(first_map)

    composite_values: list[Any] = []
    for label in union_index:
        components: list[Any] = []
        has_missing = False
        for key_map in key_maps:
            if label not in key_map or scalar_is_missing(key_map[label]):
                has_missing = True
                break
            components.append(key_map[label])

        if has_missing:
            composite_values.append(None)
        else:
            composite_values.append(encode_groupby_composite_key(components))

    key_series = pd.Series(composite_values, index=union_index, dtype="object")

    combined_index = list(union_index)
    seen = set(union_index)
    for label in value_index:
        if label not in seen:
            seen.add(label)
            combined_index.append(label)

    return key_series, combined_index


def op_series_binary_numeric(
    pd, payload: dict[str, Any], operation: str
) -> dict[str, Any]:
    left = payload.get("left")
    right = payload.get("right")
    if left is None or right is None:
        raise OracleError(f"{operation} requires left and right payloads")

    left_index = [label_from_json(item) for item in left["index"]]
    right_index = [label_from_json(item) for item in right["index"]]
    left_values = [scalar_from_json(item) for item in left["values"]]
    right_values = [scalar_from_json(item) for item in right["values"]]

    lhs = pd.Series(left_values, index=left_index, dtype="float64")
    rhs = pd.Series(right_values, index=right_index, dtype="float64")
    if operation == "series_add":
        out = lhs + rhs
    elif operation == "series_sub":
        out = lhs - rhs
    elif operation == "series_mul":
        out = lhs * rhs
    elif operation == "series_div":
        out = lhs / rhs
    else:
        raise OracleError(f"unsupported series arithmetic operation: {operation!r}")

    return {
        "expected_series": {
            "index": [label_to_json(v) for v in out.index.tolist()],
            "values": [scalar_to_json(v) for v in out.tolist()],
        }
    }


def op_series_add(pd, payload: dict[str, Any]) -> dict[str, Any]:
    return op_series_binary_numeric(pd, payload, "series_add")


def op_series_sub(pd, payload: dict[str, Any]) -> dict[str, Any]:
    return op_series_binary_numeric(pd, payload, "series_sub")


def op_series_mul(pd, payload: dict[str, Any]) -> dict[str, Any]:
    return op_series_binary_numeric(pd, payload, "series_mul")


def op_series_div(pd, payload: dict[str, Any]) -> dict[str, Any]:
    return op_series_binary_numeric(pd, payload, "series_div")


def op_series_mode(pd, payload: dict[str, Any]) -> dict[str, Any]:
    series = fixture_series_from_payload(pd, payload.get("series"), "series_mode")
    dropna = payload.get("mode_dropna")
    if dropna is None:
        dropna = True
    out = series.mode(dropna=bool(dropna))
    return {"expected_series": series_to_expected(out)}


def op_series_nunique(pd, payload: dict[str, Any]) -> dict[str, Any]:
    series = fixture_series_from_payload(pd, payload.get("series"), "series_nunique")
    dropna = payload.get("nunique_dropna")
    if dropna is None:
        dropna = True
    out = series.nunique(dropna=bool(dropna))
    return {"expected_scalar": scalar_to_json(out)}


def op_series_join(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    right = payload.get("right")
    join_type = payload.get("join_type")
    if left is None or right is None:
        raise OracleError("series_join requires left and right payloads")
    if join_type not in {"inner", "left", "right", "outer"}:
        raise OracleError(
            f"series_join requires join_type=inner|left|right|outer, got {join_type!r}"
        )

    left_index = [label_from_json(item) for item in left["index"]]
    right_index = [label_from_json(item) for item in right["index"]]
    left_values = [scalar_from_json(item) for item in left["values"]]
    right_values = [scalar_from_json(item) for item in right["values"]]

    lhs = pd.Series(left_values, index=left_index, name="left")
    rhs = pd.Series(right_values, index=right_index, name="right")
    merged = lhs.to_frame().merge(
        rhs.to_frame(),
        left_index=True,
        right_index=True,
        how=join_type,
        sort=False,
        copy=False,
    )

    def join_scalar_to_json(value: Any) -> dict[str, Any]:
        if pd.isna(value):
            return {"kind": "null", "value": "null"}
        return scalar_to_json(value)

    return {
        "expected_join": {
            "index": [label_to_json(v) for v in merged.index.tolist()],
            "left_values": [join_scalar_to_json(v) for v in merged["left"].tolist()],
            "right_values": [join_scalar_to_json(v) for v in merged["right"].tolist()],
        }
    }


def op_groupby_agg(pd, payload: dict[str, Any], agg: str, op_name: str) -> dict[str, Any]:
    right = payload.get("right")
    if right is None:
        raise OracleError(f"{op_name} requires right(values) payload")

    value_index = [label_from_json(item) for item in right["index"]]
    values = [scalar_from_json(item) for item in right["values"]]
    value_series = pd.Series(values, index=value_index, dtype="float64")
    groupby_keys = payload.get("groupby_keys")

    if isinstance(groupby_keys, list) and groupby_keys:
        key_series, union_index = build_groupby_composite_key_series(
            pd, payload, value_index
        )
    else:
        left = payload.get("left")
        if left is None:
            raise OracleError(
                f"{op_name} requires left(keys) payload when groupby_keys is absent"
            )
        key_index = [label_from_json(item) for item in left["index"]]
        keys = [scalar_from_json(item) for item in left["values"]]
        key_series = pd.Series(keys, index=key_index, dtype="object")

        union_index = list(key_index)
        seen = set(key_index)
        for label in value_index:
            if label not in seen:
                seen.add(label)
                union_index.append(label)

    aligned_keys = key_series.reindex(union_index)
    aligned_values = value_series.reindex(union_index)

    grouped = pd.DataFrame({"key": aligned_keys, "value": aligned_values}).groupby(
        "key", sort=False, dropna=True
    )["value"]
    if agg == "sum":
        out = grouped.sum()
    elif agg == "mean":
        out = grouped.mean()
    elif agg == "count":
        out = grouped.count()
    elif agg == "min":
        out = grouped.min()
    elif agg == "max":
        out = grouped.max()
    elif agg == "first":
        out = grouped.first()
    elif agg == "last":
        out = grouped.last()
    elif agg == "std":
        out = grouped.std(ddof=1)
    elif agg == "var":
        out = grouped.var(ddof=1)
    elif agg == "median":
        out = grouped.median()
    else:
        raise OracleError(f"unsupported groupby aggregation: {agg!r}")

    def groupby_agg_scalar_to_json(value: Any) -> dict[str, Any]:
        if agg in {"std", "var"} and scalar_is_missing(value):
            # Runtime currently models n<2 std/var as null (not NaN) for parity.
            return {"kind": "null", "value": "null"}
        return scalar_to_json(value)

    return {
        "expected_series": {
            "index": [label_to_json(v) for v in out.index.tolist()],
            "values": [groupby_agg_scalar_to_json(v) for v in out.tolist()],
        }
    }


def op_groupby_sum(pd, payload: dict[str, Any]) -> dict[str, Any]:
    return op_groupby_agg(pd, payload, "sum", "groupby_sum")


def op_groupby_mean(pd, payload: dict[str, Any]) -> dict[str, Any]:
    return op_groupby_agg(pd, payload, "mean", "groupby_mean")


def op_groupby_count(pd, payload: dict[str, Any]) -> dict[str, Any]:
    return op_groupby_agg(pd, payload, "count", "groupby_count")


def op_groupby_min(pd, payload: dict[str, Any]) -> dict[str, Any]:
    return op_groupby_agg(pd, payload, "min", "groupby_min")


def op_groupby_max(pd, payload: dict[str, Any]) -> dict[str, Any]:
    return op_groupby_agg(pd, payload, "max", "groupby_max")


def op_groupby_first(pd, payload: dict[str, Any]) -> dict[str, Any]:
    return op_groupby_agg(pd, payload, "first", "groupby_first")


def op_groupby_last(pd, payload: dict[str, Any]) -> dict[str, Any]:
    return op_groupby_agg(pd, payload, "last", "groupby_last")


def op_groupby_std(pd, payload: dict[str, Any]) -> dict[str, Any]:
    return op_groupby_agg(pd, payload, "std", "groupby_std")


def op_groupby_var(pd, payload: dict[str, Any]) -> dict[str, Any]:
    return op_groupby_agg(pd, payload, "var", "groupby_var")


def op_groupby_median(pd, payload: dict[str, Any]) -> dict[str, Any]:
    return op_groupby_agg(pd, payload, "median", "groupby_median")


def op_nan_agg(pd, payload: dict[str, Any], agg: str, op_name: str) -> dict[str, Any]:
    left = payload.get("left")
    if left is None:
        raise OracleError(f"{op_name} requires left(values) payload")

    values = [scalar_from_json(item) for item in left["values"]]
    series = pd.Series(values, dtype="float64")

    if agg == "sum":
        out = series.sum(skipna=True)
    elif agg == "mean":
        out = series.mean(skipna=True)
    elif agg == "min":
        out = series.min(skipna=True)
    elif agg == "max":
        out = series.max(skipna=True)
    elif agg == "std":
        out = series.std(skipna=True, ddof=1)
    elif agg == "var":
        out = series.var(skipna=True, ddof=1)
    elif agg == "count":
        out = int(series.count())
    else:
        raise OracleError(f"unsupported nan aggregation: {agg!r}")

    return {"expected_scalar": scalar_to_json(out)}


def op_nan_sum(pd, payload: dict[str, Any]) -> dict[str, Any]:
    return op_nan_agg(pd, payload, "sum", "nan_sum")


def op_nan_mean(pd, payload: dict[str, Any]) -> dict[str, Any]:
    return op_nan_agg(pd, payload, "mean", "nan_mean")


def op_nan_min(pd, payload: dict[str, Any]) -> dict[str, Any]:
    return op_nan_agg(pd, payload, "min", "nan_min")


def op_nan_max(pd, payload: dict[str, Any]) -> dict[str, Any]:
    return op_nan_agg(pd, payload, "max", "nan_max")


def op_nan_std(pd, payload: dict[str, Any]) -> dict[str, Any]:
    return op_nan_agg(pd, payload, "std", "nan_std")


def op_nan_var(pd, payload: dict[str, Any]) -> dict[str, Any]:
    return op_nan_agg(pd, payload, "var", "nan_var")


def op_nan_count(pd, payload: dict[str, Any]) -> dict[str, Any]:
    return op_nan_agg(pd, payload, "count", "nan_count")


def csv_dataframes_semantically_equal(left, right) -> bool:
    if left.columns.tolist() != right.columns.tolist():
        return False
    if len(left.index) != len(right.index):
        return False

    for name in left.columns.tolist():
        left_values = left[name].tolist()
        right_values = right[name].tolist()
        if len(left_values) != len(right_values):
            return False
        for left_value, right_value in zip(left_values, right_values):
            if scalar_is_missing(left_value) and scalar_is_missing(right_value):
                continue
            if left_value != right_value:
                return False
    return True


def op_csv_round_trip(pd, payload: dict[str, Any]) -> dict[str, Any]:
    csv_input = payload.get("csv_input")
    if not isinstance(csv_input, str):
        raise OracleError("csv_round_trip requires csv_input payload")

    try:
        frame = pd.read_csv(io.StringIO(csv_input))
        output = frame.to_csv(index=False, lineterminator="\n")
        reparsed = pd.read_csv(io.StringIO(output))
    except Exception as exc:
        raise OracleError(f"csv_round_trip failed: {exc}") from exc

    return {
        "expected_bool": bool(csv_dataframes_semantically_equal(frame, reparsed)),
    }


def op_index_align_union(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    right = payload.get("right")
    if left is None or right is None:
        raise OracleError("index_align_union requires left and right payloads")

    left_labels = [label_from_json(item) for item in left["index"]]
    right_labels = [label_from_json(item) for item in right["index"]]

    left_index = pd.Index(left_labels)
    right_index = pd.Index(right_labels)
    union = left_index.union(right_index, sort=False)

    left_positions: list[int | None] = []
    right_positions: list[int | None] = []
    for label in union.tolist():
        left_hits = [i for i, v in enumerate(left_labels) if v == label]
        right_hits = [i for i, v in enumerate(right_labels) if v == label]
        left_positions.append(left_hits[0] if left_hits else None)
        right_positions.append(right_hits[0] if right_hits else None)

    return {
        "expected_alignment": {
            "union_index": [label_to_json(v) for v in union.tolist()],
            "left_positions": left_positions,
            "right_positions": right_positions,
        }
    }


def op_index_has_duplicates(pd, payload: dict[str, Any]) -> dict[str, Any]:
    labels_raw = payload.get("index")
    if labels_raw is None:
        raise OracleError("index_has_duplicates requires index payload")
    labels = [label_from_json(item) for item in labels_raw]
    idx = pd.Index(labels)
    return {"expected_bool": bool(idx.has_duplicates)}


def op_index_is_monotonic_increasing(pd, payload: dict[str, Any]) -> dict[str, Any]:
    labels_raw = payload.get("index")
    if labels_raw is None:
        raise OracleError("index_is_monotonic_increasing requires index payload")
    labels = [label_from_json(item) for item in labels_raw]
    idx = pd.Index(labels)
    return {"expected_bool": bool(idx.is_monotonic_increasing)}


def op_index_is_monotonic_decreasing(pd, payload: dict[str, Any]) -> dict[str, Any]:
    labels_raw = payload.get("index")
    if labels_raw is None:
        raise OracleError("index_is_monotonic_decreasing requires index payload")
    labels = [label_from_json(item) for item in labels_raw]
    idx = pd.Index(labels)
    return {"expected_bool": bool(idx.is_monotonic_decreasing)}


def op_index_first_positions(pd, payload: dict[str, Any]) -> dict[str, Any]:
    labels_raw = payload.get("index")
    if labels_raw is None:
        raise OracleError("index_first_positions requires index payload")
    labels = [label_from_json(item) for item in labels_raw]

    first_map: dict[Any, int] = {}
    for pos, label in enumerate(labels):
        if label not in first_map:
            first_map[label] = pos

    return {
        "expected_positions": [first_map.get(label, None) for label in labels],
    }


def fixture_series_from_payload(pd, payload: dict[str, Any], op_name: str):
    if payload is None:
        raise OracleError(f"{op_name} requires series payload")
    index = [label_from_json(item) for item in payload["index"]]
    values = [scalar_from_json(item) for item in payload["values"]]
    return pd.Series(values, index=index, name=payload.get("name", "series"))


def optional_series_payload(
    pd, payload: dict[str, Any], key: str, op_name: str
):
    value = payload.get(key)
    if value is None:
        return None
    if not isinstance(value, dict):
        raise OracleError(f"{op_name} {key} must be a series payload")
    return fixture_series_from_payload(pd, value, op_name)


def series_to_expected(series) -> dict[str, Any]:
    return {
        "index": [label_to_json(v) for v in series.index.tolist()],
        "values": [scalar_to_json(v) for v in series.tolist()],
    }


def op_series_constructor(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    series = fixture_series_from_payload(pd, left, "series_constructor")
    return {"expected_series": series_to_expected(series)}


def op_series_combine_first(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = fixture_series_from_payload(pd, payload.get("left"), "series_combine_first")
    right = fixture_series_from_payload(pd, payload.get("right"), "series_combine_first")
    try:
        out = left.combine_first(right)
    except Exception as exc:
        raise OracleError(f"series_combine_first failed: {exc}") from exc
    return {"expected_series": series_to_expected(out)}


def op_series_asof(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    if left is None:
        raise OracleError("series_asof requires left payload")
    asof_label = payload.get("asof_label")
    if asof_label is None:
        raise OracleError("series_asof requires asof_label payload")

    series = fixture_series_from_payload(pd, left, "series_asof")
    label = label_from_json(asof_label)
    try:
        out = series.asof(label)
    except Exception as exc:
        raise OracleError(f"series_asof failed: {exc}") from exc
    return {"expected_scalar": scalar_to_json(out)}


def op_series_clip(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    if left is None:
        raise OracleError("series_clip requires left payload")

    series = fixture_series_from_payload(pd, left, "series_clip")
    lower_series = optional_series_payload(
        pd, payload, "clip_lower_series", "series_clip"
    )
    upper_series = optional_series_payload(
        pd, payload, "clip_upper_series", "series_clip"
    )
    lower = None if lower_series is not None else optional_float_payload(
        payload, "clip_lower", "series_clip"
    )
    upper = None if upper_series is not None else optional_float_payload(
        payload, "clip_upper", "series_clip"
    )

    try:
        out = series.clip(
            lower=lower_series if lower_series is not None else lower,
            upper=upper_series if upper_series is not None else upper,
        )
    except Exception as exc:
        raise OracleError(f"series_clip failed: {exc}") from exc
    return {"expected_series": series_to_expected(out)}


def op_series_to_datetime(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    series = fixture_series_from_payload(pd, left, "series_to_datetime")
    unit = payload.get("datetime_unit")
    origin = payload.get("datetime_origin")
    utc = payload.get("datetime_utc")

    kwargs: dict[str, Any] = {"errors": "coerce"}
    if unit is not None:
        if not isinstance(unit, str) or unit.strip() == "":
            raise OracleError("series_to_datetime datetime_unit must be a non-empty string")
        kwargs["unit"] = unit
    if origin is not None:
        if isinstance(origin, str):
            if origin.strip() == "":
                raise OracleError(
                    "series_to_datetime datetime_origin must be a non-empty string"
                )
        elif isinstance(origin, bool) or not isinstance(origin, (int, float)):
            raise OracleError(
                "series_to_datetime datetime_origin must be a string, integer, or float"
            )
        kwargs["origin"] = origin
    if utc is not None:
        if not isinstance(utc, bool):
            raise OracleError("series_to_datetime datetime_utc must be a boolean")
        kwargs["utc"] = utc

    try:
        out = pd.to_datetime(series, **kwargs)
    except Exception as exc:
        raise OracleError(f"series_to_datetime failed: {exc}") from exc

    def datetime_scalar_to_json(value: Any) -> dict[str, Any]:
        if pd.isna(value):
            return {"kind": "null", "value": "null"}
        return {"kind": "utf8", "value": str(value)}

    return {
        "expected_series": {
            "index": [label_to_json(v) for v in out.index.tolist()],
            "values": [datetime_scalar_to_json(v) for v in out.tolist()],
        }
    }


def op_series_dt_to_pydatetime(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    if left is None:
        raise OracleError("series_dt_to_pydatetime requires left payload")
    warn = payload.get("dt_warn")
    if warn is not None and not isinstance(warn, bool):
        raise OracleError("series_dt_to_pydatetime dt_warn must be a boolean")

    series = fixture_series_from_payload(pd, left, "series_dt_to_pydatetime")
    try:
        out = pd.to_datetime(series, errors="coerce").dt.to_pydatetime(
            warn=True if warn is None else warn
        )
    except Exception as exc:
        raise OracleError(f"series_dt_to_pydatetime failed: {exc}") from exc

    def pydatetime_scalar_to_json(value: Any) -> dict[str, Any]:
        if pd.isna(value):
            return {"kind": "null", "value": "null"}
        return {"kind": "utf8", "value": str(value)}

    return {
        "expected_series": {
            "index": [label_to_json(v) for v in series.index.tolist()],
            "values": [pydatetime_scalar_to_json(v) for v in out.tolist()],
        }
    }


def op_dataframe_from_series(pd, payload: dict[str, Any]) -> dict[str, Any]:
    payloads = collect_constructor_series_payloads(payload, "dataframe_from_series")
    series_list = [
        fixture_series_from_payload(pd, series_payload, "dataframe_from_series")
        for series_payload in payloads
    ]
    frame = pd.concat(series_list, axis=1, sort=False)
    return {"expected_frame": dataframe_to_json(frame)}


def collect_constructor_series_payloads(
    payload: dict[str, Any], op_name: str
) -> list[dict[str, Any]]:
    payloads: list[dict[str, Any]] = []
    left = payload.get("left")
    right = payload.get("right")
    if isinstance(left, dict):
        payloads.append(left)
    if isinstance(right, dict):
        payloads.append(right)
    extra = payload.get("groupby_keys")
    if isinstance(extra, list):
        payloads.extend(item for item in extra if isinstance(item, dict))

    if not payloads:
        raise OracleError(
            f"{op_name} requires at least one series payload (left/right/groupby_keys)"
        )
    return payloads


def parse_constructor_dict_columns(
    payload: dict[str, Any], op_name: str
) -> dict[str, list[Any]]:
    raw = payload.get("dict_columns")
    if not isinstance(raw, dict):
        raise OracleError(f"{op_name} requires dict_columns object payload")

    parsed: dict[str, list[Any]] = {}
    for name, values in raw.items():
        if not isinstance(values, list):
            raise OracleError(f"{op_name} column {name!r} must be a list")
        parsed[str(name)] = [scalar_from_json(item) for item in values]
    return parsed


def parse_constructor_column_order(payload: dict[str, Any], op_name: str) -> list[str] | None:
    raw = payload.get("column_order")
    if raw is None:
        return None
    if not isinstance(raw, list):
        raise OracleError(f"{op_name} column_order must be a list when provided")
    return [str(item) for item in raw]


def parse_constructor_index(payload: dict[str, Any], op_name: str) -> list[Any] | None:
    raw = payload.get("index")
    if raw is None:
        return None
    if not isinstance(raw, list):
        raise OracleError(f"{op_name} index must be a list when provided")
    return [label_from_json(item) for item in raw]


def parse_optional_string_list(
    payload: dict[str, Any], key: str, op_name: str
) -> list[str]:
    raw = payload.get(key)
    if raw is None:
        return []
    if not isinstance(raw, list):
        raise OracleError(f"{op_name} {key} must be a list when provided")

    values: list[str] = []
    for item in raw:
        value = str(item).strip()
        if not value:
            raise OracleError(f"{op_name} {key} entries must be non-empty strings")
        values.append(value)
    return values


def parse_constructor_matrix_rows(
    payload: dict[str, Any], op_name: str
) -> list[list[Any]]:
    raw = payload.get("matrix_rows")
    if not isinstance(raw, list):
        raise OracleError(f"{op_name} requires matrix_rows list payload")

    matrix_rows: list[list[Any]] = []
    for row in raw:
        if not isinstance(row, list):
            raise OracleError(f"{op_name} requires each matrix row to be a list")
        matrix_rows.append([scalar_from_json(item) for item in row])
    return matrix_rows


def op_dataframe_from_dict(pd, payload: dict[str, Any]) -> dict[str, Any]:
    data = parse_constructor_dict_columns(payload, "dataframe_from_dict")
    column_order = parse_constructor_column_order(payload, "dataframe_from_dict")
    index = parse_constructor_index(payload, "dataframe_from_dict")

    if column_order is not None and len(column_order) > 0:
        selected: dict[str, list[Any]] = {}
        for name in column_order:
            if name not in data:
                raise OracleError(f"dataframe_from_dict column '{name}' not found in data")
            selected[name] = data[name]
        data = selected

    try:
        frame = pd.DataFrame(data, index=index)
    except Exception as exc:
        raise OracleError(f"dataframe_from_dict failed: {exc}") from exc

    return {"expected_frame": dataframe_to_json(frame)}


def op_dataframe_from_records(pd, payload: dict[str, Any]) -> dict[str, Any]:
    column_order = parse_constructor_column_order(payload, "dataframe_from_records")
    index = parse_constructor_index(payload, "dataframe_from_records")
    raw_records = payload.get("records")
    raw_matrix_rows = payload.get("matrix_rows")

    if raw_records is not None and raw_matrix_rows is not None:
        raise OracleError(
            "dataframe_from_records cannot define both records and matrix_rows"
        )

    data: list[Any]
    if raw_records is not None:
        if not isinstance(raw_records, list):
            raise OracleError("dataframe_from_records requires records list payload")

        records: list[dict[str, Any]] = []
        for row in raw_records:
            if not isinstance(row, dict):
                raise OracleError(
                    "dataframe_from_records requires each record to be an object"
                )
            parsed_row: dict[str, Any] = {}
            for key, value in row.items():
                parsed_row[str(key)] = scalar_from_json(value)
            records.append(parsed_row)
        data = records
    elif raw_matrix_rows is not None:
        data = parse_constructor_matrix_rows(payload, "dataframe_from_records")
    else:
        raise OracleError(
            "dataframe_from_records requires records or matrix_rows payload"
        )

    try:
        frame = pd.DataFrame.from_records(data, columns=column_order, index=index)
    except Exception as exc:
        raise OracleError(f"dataframe_from_records failed: {exc}") from exc

    return {"expected_frame": dataframe_to_json(frame)}


def op_dataframe_constructor_kwargs(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_constructor_kwargs requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    column_order = parse_constructor_column_order(payload, "dataframe_constructor_kwargs")
    index = parse_constructor_index(payload, "dataframe_constructor_kwargs")

    try:
        out = pd.DataFrame(frame, index=index, columns=column_order)
    except Exception as exc:
        raise OracleError(f"dataframe_constructor_kwargs failed: {exc}") from exc

    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_constructor_scalar(pd, payload: dict[str, Any]) -> dict[str, Any]:
    fill_value_raw = payload.get("fill_value")
    if fill_value_raw is None:
        raise OracleError("dataframe_constructor_scalar requires fill_value payload")
    fill_value = scalar_from_json(fill_value_raw)

    column_order = parse_constructor_column_order(payload, "dataframe_constructor_scalar")
    index = parse_constructor_index(payload, "dataframe_constructor_scalar")

    try:
        out = pd.DataFrame(fill_value, index=index, columns=column_order)
    except Exception as exc:
        raise OracleError(f"dataframe_constructor_scalar failed: {exc}") from exc

    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_constructor_dict_of_series(pd, payload: dict[str, Any]) -> dict[str, Any]:
    payloads = collect_constructor_series_payloads(
        payload, "dataframe_constructor_dict_of_series"
    )
    column_order = parse_constructor_column_order(
        payload, "dataframe_constructor_dict_of_series"
    )
    index = parse_constructor_index(payload, "dataframe_constructor_dict_of_series")

    data: dict[str, Any] = {}
    for series_payload in payloads:
        series = fixture_series_from_payload(
            pd, series_payload, "dataframe_constructor_dict_of_series"
        )
        data[str(series.name)] = series

    try:
        out = pd.DataFrame(data, index=index, columns=column_order)
    except Exception as exc:
        raise OracleError(
            f"dataframe_constructor_dict_of_series failed: {exc}"
        ) from exc

    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_constructor_list_like(pd, payload: dict[str, Any]) -> dict[str, Any]:
    matrix_rows = parse_constructor_matrix_rows(payload, "dataframe_constructor_list_like")
    column_order = parse_constructor_column_order(payload, "dataframe_constructor_list_like")
    index = parse_constructor_index(payload, "dataframe_constructor_list_like")

    try:
        out = pd.DataFrame(matrix_rows, index=index, columns=column_order)
    except Exception as exc:
        raise OracleError(f"dataframe_constructor_list_like failed: {exc}") from exc

    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_melt(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_melt requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    id_vars = parse_optional_string_list(payload, "melt_id_vars", "dataframe_melt")
    value_vars = parse_optional_string_list(
        payload, "melt_value_vars", "dataframe_melt"
    )
    var_name = payload.get("melt_var_name")
    value_name = payload.get("melt_value_name")

    kwargs: dict[str, Any] = {}
    if id_vars:
        kwargs["id_vars"] = id_vars
    if value_vars:
        kwargs["value_vars"] = value_vars
    if var_name is not None:
        kwargs["var_name"] = str(var_name)
    if value_name is not None:
        kwargs["value_name"] = str(value_name)

    try:
        out = frame.melt(**kwargs)
    except Exception as exc:
        raise OracleError(f"dataframe_melt failed: {exc}") from exc

    return {"expected_frame": dataframe_to_json(out)}


def op_series_loc(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    loc_labels = payload.get("loc_labels")
    if left is None:
        raise OracleError("series_loc requires left payload")
    if not isinstance(loc_labels, list):
        raise OracleError("series_loc requires loc_labels list payload")

    index = [label_from_json(item) for item in left["index"]]
    values = [scalar_from_json(item) for item in left["values"]]
    labels = [label_from_json(item) for item in loc_labels]

    series = pd.Series(values, index=index, name=left.get("name", "series"))
    try:
        out = series.loc[labels]
    except KeyError as exc:
        raise OracleError(f"series_loc label lookup failed: {exc}") from exc

    return {
        "expected_series": {
            "index": [label_to_json(v) for v in out.index.tolist()],
            "values": [scalar_to_json(v) for v in out.tolist()],
        }
    }


def op_series_iloc(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    iloc_positions = payload.get("iloc_positions")
    if left is None:
        raise OracleError("series_iloc requires left payload")
    if not isinstance(iloc_positions, list):
        raise OracleError("series_iloc requires iloc_positions list payload")

    index = [label_from_json(item) for item in left["index"]]
    values = [scalar_from_json(item) for item in left["values"]]

    try:
        positions = [int(value) for value in iloc_positions]
    except Exception as exc:  # pragma: no cover - defensive conversion
        raise OracleError(f"series_iloc positions must be integers: {exc}") from exc

    series = pd.Series(values, index=index, name=left.get("name", "series"))
    try:
        out = series.iloc[positions]
    except IndexError as exc:
        raise OracleError(f"series_iloc position lookup failed: {exc}") from exc

    return {
        "expected_series": {
            "index": [label_to_json(v) for v in out.index.tolist()],
            "values": [scalar_to_json(v) for v in out.tolist()],
        }
    }


def op_series_take(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    take_indices = payload.get("take_indices")
    if left is None:
        raise OracleError("series_take requires left payload")
    if not isinstance(take_indices, list):
        raise OracleError("series_take requires take_indices list payload")

    index = [label_from_json(item) for item in left["index"]]
    values = [scalar_from_json(item) for item in left["values"]]

    try:
        indices = [int(value) for value in take_indices]
    except Exception as exc:  # pragma: no cover - defensive conversion
        raise OracleError(f"series_take indices must be integers: {exc}") from exc

    series = pd.Series(values, index=index, name=left.get("name", "series"))
    try:
        out = series.take(indices)
    except IndexError as exc:
        raise OracleError(f"series_take position lookup failed: {exc}") from exc

    return {
        "expected_series": {
            "index": [label_to_json(v) for v in out.index.tolist()],
            "values": [scalar_to_json(v) for v in out.tolist()],
        }
    }


def op_series_repeat(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    repeat_n = payload.get("repeat_n")
    repeat_counts = payload.get("repeat_counts")
    if left is None:
        raise OracleError("series_repeat requires left payload")
    if (repeat_n is None) == (repeat_counts is None):
        raise OracleError("series_repeat requires exactly one of repeat_n or repeat_counts")

    index = [label_from_json(item) for item in left["index"]]
    values = [scalar_from_json(item) for item in left["values"]]
    series = pd.Series(values, index=index, name=left.get("name", "series"))

    if repeat_n is not None:
        try:
            repeats: Any = int(repeat_n)
        except Exception as exc:  # pragma: no cover - defensive conversion
            raise OracleError(f"series_repeat repeat_n must be an integer: {exc}") from exc
    else:
        if not isinstance(repeat_counts, list):
            raise OracleError("series_repeat repeat_counts must be a list")
        try:
            repeats = [int(value) for value in repeat_counts]
        except Exception as exc:  # pragma: no cover - defensive conversion
            raise OracleError(f"series_repeat repeat_counts must be integers: {exc}") from exc

    try:
        out = series.repeat(repeats)
    except Exception as exc:
        raise OracleError(f"series_repeat failed: {exc}") from exc

    return {
        "expected_series": {
            "index": [label_to_json(v) for v in out.index.tolist()],
            "values": [scalar_to_json(v) for v in out.tolist()],
        }
    }


def op_series_at_time(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    time_value = payload.get("time_value")
    if left is None:
        raise OracleError("series_at_time requires left payload")
    if not isinstance(time_value, str) or not time_value:
        raise OracleError("series_at_time requires non-empty time_value payload")

    index = pd.DatetimeIndex([label_from_json(item) for item in left["index"]])
    values = [scalar_from_json(item) for item in left["values"]]

    series = pd.Series(values, index=index, name=left.get("name", "series"))
    try:
        out = series.at_time(time_value)
    except Exception as exc:
        raise OracleError(f"series_at_time selection failed: {exc}") from exc

    return {
        "expected_series": {
            "index": [label_to_json(v.isoformat()) for v in out.index.tolist()],
            "values": [scalar_to_json(v) for v in out.tolist()],
        }
    }


def op_series_between_time(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    start_time = payload.get("start_time")
    end_time = payload.get("end_time")
    if left is None:
        raise OracleError("series_between_time requires left payload")
    if not isinstance(start_time, str) or not start_time:
        raise OracleError("series_between_time requires non-empty start_time payload")
    if not isinstance(end_time, str) or not end_time:
        raise OracleError("series_between_time requires non-empty end_time payload")

    index = pd.DatetimeIndex([label_from_json(item) for item in left["index"]])
    values = [scalar_from_json(item) for item in left["values"]]

    series = pd.Series(values, index=index, name=left.get("name", "series"))
    try:
        out = series.between_time(start_time, end_time)
    except Exception as exc:
        raise OracleError(f"series_between_time selection failed: {exc}") from exc

    return {
        "expected_series": {
            "index": [label_to_json(v.isoformat()) for v in out.index.tolist()],
            "values": [scalar_to_json(v) for v in out.tolist()],
        }
    }


def op_series_filter(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    right = payload.get("right")
    if left is None or right is None:
        raise OracleError("series_filter requires left(data) and right(mask) payloads")

    data_index = [label_from_json(item) for item in left["index"]]
    data_values = [scalar_from_json(item) for item in left["values"]]
    mask_index = [label_from_json(item) for item in right["index"]]
    mask_values = [scalar_from_json(item) for item in right["values"]]

    data = pd.Series(data_values, index=data_index, name=left.get("name", "data"))
    mask = pd.Series(mask_values, index=mask_index, name=right.get("name", "mask"))

    try:
        out = data[mask]
    except Exception as exc:
        raise OracleError(f"series_filter mask application failed: {exc}") from exc

    return {
        "expected_series": {
            "index": [label_to_json(v) for v in out.index.tolist()],
            "values": [scalar_to_json(v) for v in out.tolist()],
        }
    }


def op_series_head(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    head_n = payload.get("head_n")
    if left is None:
        raise OracleError("series_head requires left payload")
    if head_n is None:
        raise OracleError("series_head requires head_n payload")

    index = [label_from_json(item) for item in left["index"]]
    values = [scalar_from_json(item) for item in left["values"]]

    try:
        n = int(head_n)
    except Exception as exc:  # pragma: no cover - defensive conversion
        raise OracleError(f"series_head head_n must be an integer: {exc}") from exc

    series = pd.Series(values, index=index, name=left.get("name", "series"))
    out = series.head(n)

    return {
        "expected_series": {
            "index": [label_to_json(v) for v in out.index.tolist()],
            "values": [scalar_to_json(v) for v in out.tolist()],
        }
    }


def op_series_tail(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    tail_n = payload.get("tail_n")
    if left is None:
        raise OracleError("series_tail requires left payload")
    if tail_n is None:
        raise OracleError("series_tail requires tail_n payload")

    index = [label_from_json(item) for item in left["index"]]
    values = [scalar_from_json(item) for item in left["values"]]

    try:
        n = int(tail_n)
    except Exception as exc:  # pragma: no cover - defensive conversion
        raise OracleError(f"series_tail tail_n must be an integer: {exc}") from exc

    series = pd.Series(values, index=index, name=left.get("name", "series"))
    out = series.tail(n)

    return {
        "expected_series": {
            "index": [label_to_json(v) for v in out.index.tolist()],
            "values": [scalar_to_json(v) for v in out.tolist()],
        }
    }


def op_series_isna(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    if left is None:
        raise OracleError("series_isna requires left payload")

    index = [label_from_json(item) for item in left["index"]]
    values = [scalar_from_json(item) for item in left["values"]]
    series = pd.Series(values, index=index, name=left.get("name", "series"))
    out = series.isna()

    return {
        "expected_series": {
            "index": [label_to_json(v) for v in out.index.tolist()],
            "values": [scalar_to_json(v) for v in out.tolist()],
        }
    }


def op_series_notna(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    if left is None:
        raise OracleError("series_notna requires left payload")

    index = [label_from_json(item) for item in left["index"]]
    values = [scalar_from_json(item) for item in left["values"]]
    series = pd.Series(values, index=index, name=left.get("name", "series"))
    out = series.notna()

    return {
        "expected_series": {
            "index": [label_to_json(v) for v in out.index.tolist()],
            "values": [scalar_to_json(v) for v in out.tolist()],
        }
    }


def op_series_isnull(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    if left is None:
        raise OracleError("series_isnull requires left payload")

    index = [label_from_json(item) for item in left["index"]]
    values = [scalar_from_json(item) for item in left["values"]]
    series = pd.Series(values, index=index, name=left.get("name", "series"))
    out = series.isnull()

    return {
        "expected_series": {
            "index": [label_to_json(v) for v in out.index.tolist()],
            "values": [scalar_to_json(v) for v in out.tolist()],
        }
    }


def op_series_notnull(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    if left is None:
        raise OracleError("series_notnull requires left payload")

    index = [label_from_json(item) for item in left["index"]]
    values = [scalar_from_json(item) for item in left["values"]]
    series = pd.Series(values, index=index, name=left.get("name", "series"))
    out = series.notnull()

    return {
        "expected_series": {
            "index": [label_to_json(v) for v in out.index.tolist()],
            "values": [scalar_to_json(v) for v in out.tolist()],
        }
    }


def op_series_fillna(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    fill_value_payload = payload.get("fill_value")
    if left is None:
        raise OracleError("series_fillna requires left payload")
    if fill_value_payload is None:
        raise OracleError("series_fillna requires fill_value payload")

    index = [label_from_json(item) for item in left["index"]]
    values = [scalar_from_json(item) for item in left["values"]]
    fill_value = scalar_from_json(fill_value_payload)
    series = pd.Series(values, index=index, name=left.get("name", "series"))
    try:
        out = series.fillna(fill_value)
    except Exception as exc:
        raise OracleError(f"series_fillna failed: {exc}") from exc

    return {
        "expected_series": {
            "index": [label_to_json(v) for v in out.index.tolist()],
            "values": [scalar_to_json(v) for v in out.tolist()],
        }
    }


def op_series_dropna(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    if left is None:
        raise OracleError("series_dropna requires left payload")

    index = [label_from_json(item) for item in left["index"]]
    values = [scalar_from_json(item) for item in left["values"]]
    series = pd.Series(values, index=index, name=left.get("name", "series"))
    out = series.dropna()

    return {
        "expected_series": {
            "index": [label_to_json(v) for v in out.index.tolist()],
            "values": [scalar_to_json(v) for v in out.tolist()],
        }
    }


def op_series_count(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    if left is None:
        raise OracleError("series_count requires left payload")

    index = [label_from_json(item) for item in left["index"]]
    values = [scalar_from_json(item) for item in left["values"]]
    series = pd.Series(values, index=index, name=left.get("name", "series"))
    out = int(series.count())

    return {"expected_scalar": scalar_to_json(out)}


def op_series_rank(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    if left is None:
        raise OracleError("series_rank requires left payload")

    method = payload.get("rank_method") or "average"
    na_option = payload.get("rank_na_option") or "keep"
    ascending = payload.get("sort_ascending")
    if ascending is None:
        ascending = True
    pct = bool(payload.get("rank_pct", False))

    index = [label_from_json(item) for item in left["index"]]
    values = [scalar_from_json(item) for item in left["values"]]
    series = pd.Series(values, index=index, name=left.get("name", "series"))
    out = series.rank(method=method, ascending=ascending, na_option=na_option, pct=pct)

    return {
        "expected_series": {
            "index": [label_to_json(v) for v in out.index.tolist()],
            "values": [scalar_to_json(v) for v in out.tolist()],
        }
    }


def op_series_any(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    if left is None:
        raise OracleError("series_any requires left payload")

    index = [label_from_json(item) for item in left["index"]]
    values = [scalar_from_json(item) for item in left["values"]]
    series = pd.Series(values, index=index, name=left.get("name", "series"))
    return {"expected_bool": bool(series.any(skipna=True))}


def op_series_all(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    if left is None:
        raise OracleError("series_all requires left payload")

    index = [label_from_json(item) for item in left["index"]]
    values = [scalar_from_json(item) for item in left["values"]]
    series = pd.Series(values, index=index, name=left.get("name", "series"))
    return {"expected_bool": bool(series.all(skipna=True))}


def op_series_bool(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    if left is None:
        raise OracleError("series_bool requires left payload")

    index = [label_from_json(item) for item in left["index"]]
    values = [scalar_from_json(item) for item in left["values"]]
    series = pd.Series(values, index=index, name=left.get("name", "series"))
    try:
        out = bool(series.bool())
    except Exception as exc:
        raise OracleError(f"series_bool failed: {exc}") from exc
    return {"expected_bool": out}


def op_series_to_numeric(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    if left is None:
        raise OracleError("series_to_numeric requires left payload")

    index = [label_from_json(item) for item in left["index"]]
    values = [scalar_from_json(item) for item in left["values"]]
    series = pd.Series(values, index=index, name=left.get("name", "series"))
    try:
        out = pd.to_numeric(series, errors="coerce")
    except Exception as exc:
        raise OracleError(f"series_to_numeric failed: {exc}") from exc

    return {
        "expected_series": {
            "index": [label_to_json(v) for v in out.index.tolist()],
            "values": [scalar_to_json(v) for v in out.tolist()],
        }
    }


def op_series_cut(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    bins = payload.get("cut_bins")
    if left is None:
        raise OracleError("series_cut requires left payload")
    if bins is None:
        raise OracleError("series_cut requires cut_bins payload")

    index = [label_from_json(item) for item in left["index"]]
    values = [scalar_from_json(item) for item in left["values"]]
    series = pd.Series(values, index=index, name=left.get("name", "series"))
    try:
        out = pd.cut(series, bins=int(bins))
    except Exception as exc:
        raise OracleError(f"series_cut failed: {exc}") from exc

    return {
        "expected_series": {
            "index": [label_to_json(v) for v in out.index.tolist()],
            "values": [scalar_to_json(v) for v in out.tolist()],
        }
    }


def op_series_qcut(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    quantiles = payload.get("qcut_quantiles")
    if left is None:
        raise OracleError("series_qcut requires left payload")
    if quantiles is None:
        raise OracleError("series_qcut requires qcut_quantiles payload")

    index = [label_from_json(item) for item in left["index"]]
    values = [scalar_from_json(item) for item in left["values"]]
    series = pd.Series(values, index=index, name=left.get("name", "series"))
    try:
        out = pd.qcut(series, q=int(quantiles))
    except Exception as exc:
        raise OracleError(f"series_qcut failed: {exc}") from exc

    return {
        "expected_series": {
            "index": [label_to_json(v) for v in out.index.tolist()],
            "values": [scalar_to_json(v) for v in out.tolist()],
        }
    }


def op_series_xs(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    xs_key = payload.get("xs_key")
    if left is None:
        raise OracleError("series_xs requires left payload")
    if xs_key is None:
        raise OracleError("series_xs requires xs_key payload")

    index = [label_from_json(item) for item in left["index"]]
    values = [scalar_from_json(item) for item in left["values"]]
    key = label_from_json(xs_key)
    series = pd.Series(values, index=index, name=left.get("name", "series"))
    try:
        out = series.xs(key)
    except Exception as exc:
        raise OracleError(f"series_xs failed: {exc}") from exc

    if not hasattr(out, "index") or not hasattr(out, "tolist"):
        raise OracleError(
            "series_xs currently requires duplicate-label selections that return a Series"
        )

    return {
        "expected_series": {
            "index": [label_to_json(v) for v in out.index.tolist()],
            "values": [scalar_to_json(v) for v in out.tolist()],
        }
    }


def op_series_value_counts(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    if left is None:
        raise OracleError("series_value_counts requires left payload")

    normalize = payload.get("value_counts_normalize", False)
    ascending = payload.get("sort_ascending", False)
    index = [label_from_json(item) for item in left["index"]]
    values = [scalar_from_json(item) for item in left["values"]]
    series = pd.Series(values, index=index, name=left.get("name", "series"))
    out = series.value_counts(normalize=normalize, sort=True, ascending=ascending, dropna=True)

    return {
        "expected_series": {
            "index": [label_to_json(v) for v in out.index.tolist()],
            "values": [scalar_to_json(v) for v in out.tolist()],
        }
    }


def op_series_sort_index(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    if left is None:
        raise OracleError("series_sort_index requires left payload")

    ascending = payload.get("sort_ascending", True)
    index = [label_from_json(item) for item in left["index"]]
    values = [scalar_from_json(item) for item in left["values"]]
    series = pd.Series(values, index=index, name=left.get("name", "series"))
    out = series.sort_index(ascending=bool(ascending))

    return {
        "expected_series": {
            "index": [label_to_json(v) for v in out.index.tolist()],
            "values": [scalar_to_json(v) for v in out.tolist()],
        }
    }


def op_series_sort_values(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    if left is None:
        raise OracleError("series_sort_values requires left payload")

    ascending = payload.get("sort_ascending", True)
    index = [label_from_json(item) for item in left["index"]]
    values = [scalar_from_json(item) for item in left["values"]]
    series = pd.Series(values, index=index, name=left.get("name", "series"))
    out = series.sort_values(ascending=bool(ascending), na_position="last")

    return {
        "expected_series": {
            "index": [label_to_json(v) for v in out.index.tolist()],
            "values": [scalar_to_json(v) for v in out.tolist()],
        }
    }


def op_series_diff(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    if left is None:
        raise OracleError("series_diff requires left payload")

    periods = payload.get("diff_periods", 1)
    index = [label_from_json(item) for item in left["index"]]
    values = [scalar_from_json(item) for item in left["values"]]
    series = pd.Series(values, index=index, name=left.get("name", "series"))
    out = series.diff(periods=int(periods))

    return {
        "expected_series": {
            "index": [label_to_json(v) for v in out.index.tolist()],
            "values": [scalar_to_json(v) for v in out.tolist()],
        }
    }


def op_series_shift(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    if left is None:
        raise OracleError("series_shift requires left payload")

    periods = payload.get("shift_periods", 1)
    index = [label_from_json(item) for item in left["index"]]
    values = [scalar_from_json(item) for item in left["values"]]
    series = pd.Series(values, index=index, name=left.get("name", "series"))
    out = series.shift(periods=int(periods))

    return {
        "expected_series": {
            "index": [label_to_json(v) for v in out.index.tolist()],
            "values": [scalar_to_json(v) for v in out.tolist()],
        }
    }


def op_dataframe_shift(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_shift requires frame payload")

    periods = payload.get("shift_periods", 1)
    axis = payload.get("shift_axis", 0)
    if axis not in (0, 1):
        raise OracleError(f"dataframe_shift shift_axis must be 0 or 1 (got {axis!r})")

    frame = dataframe_from_json(pd, frame_payload)
    out = frame.shift(periods=int(periods), axis=axis)
    return {"expected_frame": dataframe_to_json(out)}


def op_series_pct_change(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    if left is None:
        raise OracleError("series_pct_change requires left payload")

    periods = payload.get("pct_change_periods", 1)
    index = [label_from_json(item) for item in left["index"]]
    values = [scalar_from_json(item) for item in left["values"]]
    series = pd.Series(values, index=index, name=left.get("name", "series"))
    out = series.pct_change(periods=int(periods))

    return {
        "expected_series": {
            "index": [label_to_json(v) for v in out.index.tolist()],
            "values": [scalar_to_json(v) for v in out.tolist()],
        }
    }


def op_series_extractall(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    if left is None:
        raise OracleError("series_extractall requires left payload")
    regex_pattern = payload.get("regex_pattern")
    if not isinstance(regex_pattern, str) or regex_pattern == "":
        raise OracleError("series_extractall requires non-empty regex_pattern")

    series = fixture_series_from_payload(pd, left, "series_extractall")
    try:
        out = normalize_series_extractall_frame(series.str.extractall(regex_pattern))
    except Exception as exc:
        raise OracleError(f"series_extractall failed: {exc}") from exc

    return {"expected_frame": dataframe_to_json(out)}


def op_series_extract_df(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    if left is None:
        raise OracleError("series_extract_df requires left payload")
    regex_pattern = payload.get("regex_pattern")
    if not isinstance(regex_pattern, str) or regex_pattern == "":
        raise OracleError("series_extract_df requires non-empty regex_pattern")

    series = fixture_series_from_payload(pd, left, "series_extract_df")
    try:
        out = series.str.extract(regex_pattern, expand=True)
    except Exception as exc:
        raise OracleError(f"series_extract_df failed: {exc}") from exc

    return {"expected_frame": dataframe_to_json(out)}


def op_series_partition_df(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    if left is None:
        raise OracleError("series_partition_df requires left payload")
    string_sep = payload.get("string_sep")
    if not isinstance(string_sep, str):
        raise OracleError("series_partition_df requires string_sep")

    series = fixture_series_from_payload(pd, left, "series_partition_df")
    try:
        out = series.str.partition(string_sep, expand=True)
    except Exception as exc:
        raise OracleError(f"series_partition_df failed: {exc}") from exc

    return {"expected_frame": dataframe_to_json(out)}


def op_series_rpartition_df(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    if left is None:
        raise OracleError("series_rpartition_df requires left payload")
    string_sep = payload.get("string_sep")
    if not isinstance(string_sep, str):
        raise OracleError("series_rpartition_df requires string_sep")

    series = fixture_series_from_payload(pd, left, "series_rpartition_df")
    try:
        out = series.str.rpartition(string_sep, expand=True)
    except Exception as exc:
        raise OracleError(f"series_rpartition_df failed: {exc}") from exc

    return {"expected_frame": dataframe_to_json(out)}


def op_series_split_df(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    if left is None:
        raise OracleError("series_split_df requires left payload")
    str_split_pat = payload.get("str_split_pat")
    if not isinstance(str_split_pat, str):
        raise OracleError("series_split_df requires str_split_pat")

    series = fixture_series_from_payload(pd, left, "series_split_df")
    try:
        out = series.str.split(str_split_pat, expand=True)
    except Exception as exc:
        raise OracleError(f"series_split_df failed: {exc}") from exc

    return {"expected_frame": dataframe_to_json(out)}


def dataframe_from_json(pd, payload: dict[str, Any]):
    index_raw = payload.get("index")
    columns_raw = payload.get("columns")
    column_order_raw = payload.get("column_order")
    categorical_columns_raw = payload.get("categorical_columns")
    row_multiindex_raw = payload.get("row_multiindex")
    if not isinstance(index_raw, list):
        raise OracleError("frame payload requires index list")
    if not isinstance(columns_raw, dict):
        raise OracleError("frame payload requires columns object")

    index = [label_from_json(item) for item in index_raw]
    columns: dict[str, list[Any]] = {}
    for name, values in columns_raw.items():
        if not isinstance(values, list):
            raise OracleError(f"frame column {name!r} must be a list")
        parsed = [scalar_from_json(item) for item in values]
        if len(parsed) != len(index):
            raise OracleError(
                f"frame column {name!r} length {len(parsed)} does not match index length {len(index)}"
            )
        columns[str(name)] = parsed

    input_order = [str(name) for name in columns.keys()]
    if column_order_raw is None:
        column_order = input_order
    else:
        if not isinstance(column_order_raw, list):
            raise OracleError("frame payload column_order must be a list")
        column_order = []
        seen: set[str] = set()
        for raw in column_order_raw:
            name = str(raw)
            if name not in columns:
                raise OracleError(
                    f"frame payload column_order references unknown column {name!r}"
                )
            if name in seen:
                raise OracleError(
                    f"frame payload column_order contains duplicate column {name!r}"
                )
            seen.add(name)
            column_order.append(name)
        for name in input_order:
            if name not in seen:
                column_order.append(name)

    frame = pd.DataFrame(columns, index=index)
    frame = frame.reindex(columns=column_order)

    if row_multiindex_raw is not None:
        if not isinstance(row_multiindex_raw, dict):
            raise OracleError("frame payload row_multiindex must be an object")
        frame.index = multiindex_from_json(pd, row_multiindex_raw)

    if categorical_columns_raw is not None:
        if not isinstance(categorical_columns_raw, dict):
            raise OracleError("frame payload categorical_columns must be an object")
        for raw_name, raw_spec in categorical_columns_raw.items():
            name = str(raw_name)
            if name not in frame.columns:
                raise OracleError(
                    f"frame payload categorical_columns references unknown column {name!r}"
                )
            if not isinstance(raw_spec, dict):
                raise OracleError(
                    f"frame payload categorical_columns[{name!r}] must be an object"
                )
            categories_raw = raw_spec.get("categories")
            if not isinstance(categories_raw, list):
                raise OracleError(
                    f"frame payload categorical_columns[{name!r}].categories must be a list"
                )
            ordered_raw = raw_spec.get("ordered", False)
            if not isinstance(ordered_raw, bool):
                raise OracleError(
                    f"frame payload categorical_columns[{name!r}].ordered must be a boolean"
                )
            categories = [scalar_from_json(item) for item in categories_raw]
            frame[name] = pd.Categorical(
                frame[name], categories=categories, ordered=ordered_raw
            )

    return frame


def dataframe_to_json(frame) -> dict[str, Any]:
    columns: dict[str, list[dict[str, Any]]] = {}
    column_order: list[str] = []
    for position, name in enumerate(frame.columns.tolist()):
        key = str(name)
        values = [scalar_to_json(v) for v in frame.iloc[:, position].tolist()]
        if key in columns and columns[key] != values:
            raise OracleError(
                f"duplicate column label {key!r} has non-identical values and cannot be represented"
            )
        columns[key] = values
        column_order.append(key)

    response = {
        "index": [label_to_json(v) for v in frame.index.tolist()],
        "columns": columns,
        "column_order": column_order,
    }
    if hasattr(frame.index, "nlevels") and getattr(frame.index, "nlevels", 1) > 1:
        response["index"] = [
            label_to_json(tuple_label_to_flat_string(values))
            for values in frame.index.tolist()
        ]
        response["row_multiindex"] = multiindex_to_json(frame.index)
    return response


def require_expr_payload(payload: dict[str, Any], op_name: str) -> str:
    expr = payload.get("expr")
    if not isinstance(expr, str) or expr.strip() == "":
        raise OracleError(f"{op_name} requires non-empty expr")
    return expr


def locals_from_payload(payload: dict[str, Any], op_name: str) -> dict[str, Any]:
    locals_raw = payload.get("locals") or {}
    if not isinstance(locals_raw, dict):
        raise OracleError(f"{op_name} locals must be an object")
    return {str(name): scalar_from_json(value) for name, value in locals_raw.items()}


def op_dataframe_expression(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_eval requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    expr = require_expr_payload(payload, "dataframe_eval")
    local_dict = locals_from_payload(payload, "dataframe_eval")
    try:
        eval_method = getattr(frame, "eval")
        out = eval_method(expr, local_dict=local_dict)
    except Exception as exc:
        raise OracleError(f"dataframe_eval failed: {exc}") from exc
    return {"expected_series": series_to_expected(out)}


def op_dataframe_query(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_query requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    expr = require_expr_payload(payload, "dataframe_query")
    local_dict = locals_from_payload(payload, "dataframe_query")
    try:
        out = frame.query(expr, local_dict=local_dict)
    except Exception as exc:
        raise OracleError(f"dataframe_query failed: {exc}") from exc
    return {"expected_frame": dataframe_to_json(out)}


def required_string_payload(payload: dict[str, Any], key: str, op_name: str) -> str:
    value = payload.get(key)
    if not isinstance(value, str) or value.strip() == "":
        raise OracleError(f"{op_name} requires non-empty {key}")
    return value.strip()


def optional_float_payload(payload: dict[str, Any], key: str, op_name: str) -> float | None:
    value = payload.get(key)
    if value is None:
        return None
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise OracleError(f"{op_name} {key} must be numeric when provided")
    return float(value)


def pandas_dtype_from_constructor_spec(dtype_spec: str) -> str:
    normalized = dtype_spec.strip().lower()
    if normalized in {"bool", "boolean"}:
        return "bool"
    if normalized in {"int64", "int", "i64"}:
        return "int64"
    if normalized in {"float64", "float", "f64"}:
        return "float64"
    if normalized in {"utf8", "string", "str"}:
        return "string"
    raise OracleError(f"unsupported constructor dtype {dtype_spec!r}")


def op_dataframe_astype(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_astype requires frame payload")

    dtype_spec = required_string_payload(payload, "constructor_dtype", "dataframe_astype")
    frame = dataframe_from_json(pd, frame_payload)
    try:
        out = frame.astype(pandas_dtype_from_constructor_spec(dtype_spec))
    except Exception as exc:
        raise OracleError(f"dataframe_astype failed: {exc}") from exc
    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_clip(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_clip requires frame payload")

    lower = optional_float_payload(payload, "clip_lower", "dataframe_clip")
    upper = optional_float_payload(payload, "clip_upper", "dataframe_clip")
    frame = dataframe_from_json(pd, frame_payload)
    try:
        out = frame.clip(lower=lower, upper=upper)
    except Exception as exc:
        raise OracleError(f"dataframe_clip failed: {exc}") from exc
    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_abs(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_abs requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    try:
        out = frame.abs()
    except Exception as exc:
        raise OracleError(f"dataframe_abs failed: {exc}") from exc
    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_describe(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_describe requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    percentiles = payload.get("describe_percentiles")
    include = payload.get("describe_include")
    exclude = payload.get("describe_exclude")

    kwargs: dict[str, Any] = {}
    if percentiles is not None:
        kwargs["percentiles"] = percentiles
    if include is not None:
        kwargs["include"] = include
    if exclude is not None:
        kwargs["exclude"] = exclude

    try:
        out = frame.describe(**kwargs)
    except Exception as exc:
        raise OracleError(f"dataframe_describe failed: {exc}") from exc
    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_corr(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_corr requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    method = payload.get("corr_method", "pearson")
    min_periods = payload.get("corr_min_periods")

    kwargs: dict[str, Any] = {"method": method}
    if min_periods is not None:
        kwargs["min_periods"] = min_periods

    try:
        out = frame.corr(**kwargs)
    except Exception as exc:
        raise OracleError(f"dataframe_corr failed: {exc}") from exc
    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_cov(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_cov requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    min_periods = payload.get("cov_min_periods")
    ddof = payload.get("cov_ddof", 1)

    kwargs: dict[str, Any] = {"ddof": ddof}
    if min_periods is not None:
        kwargs["min_periods"] = min_periods

    try:
        out = frame.cov(**kwargs)
    except Exception as exc:
        raise OracleError(f"dataframe_cov failed: {exc}") from exc
    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_idxmin(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_idxmin requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    axis = payload.get("idxmin_axis", 0)
    skipna = payload.get("idxmin_skipna", True)

    try:
        out = frame.idxmin(axis=axis, skipna=skipna)
    except Exception as exc:
        raise OracleError(f"dataframe_idxmin failed: {exc}") from exc
    return {"expected_series": series_to_expected(out)}


def op_dataframe_idxmax(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_idxmax requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    axis = payload.get("idxmax_axis", 0)
    skipna = payload.get("idxmax_skipna", True)

    try:
        out = frame.idxmax(axis=axis, skipna=skipna)
    except Exception as exc:
        raise OracleError(f"dataframe_idxmax failed: {exc}") from exc
    return {"expected_series": series_to_expected(out)}


def op_dataframe_sem(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_sem requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    axis = payload.get("sem_axis", 0)
    skipna = payload.get("sem_skipna", True)
    ddof = payload.get("sem_ddof", 1)

    try:
        out = frame.sem(axis=axis, skipna=skipna, ddof=ddof)
    except Exception as exc:
        raise OracleError(f"dataframe_sem failed: {exc}") from exc
    return {"expected_series": series_to_expected(out)}


def op_dataframe_apply_builtin(
    pd, payload: dict[str, Any], func: str, axis: int, op_name: str
) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError(f"{op_name} requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)

    try:
        out = frame.apply(func, axis=axis)
    except Exception as exc:
        raise OracleError(f"{op_name} failed: {exc}") from exc
    return {"expected_series": series_to_expected(out)}


def op_dataframe_apply_sem_axis0(pd, payload: dict[str, Any]) -> dict[str, Any]:
    return op_dataframe_apply_builtin(
        pd, payload, "sem", 0, "dataframe_apply_sem_axis0"
    )


def op_dataframe_skew(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_skew requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    axis = payload.get("skew_axis", 0)
    skipna = payload.get("skew_skipna", True)

    try:
        out = frame.skew(axis=axis, skipna=skipna)
    except Exception as exc:
        raise OracleError(f"dataframe_skew failed: {exc}") from exc
    return {"expected_series": series_to_expected(out)}


def op_dataframe_kurtosis(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_kurtosis requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    axis = payload.get("kurtosis_axis", 0)
    skipna = payload.get("kurtosis_skipna", True)

    try:
        out = frame.kurtosis(axis=axis, skipna=skipna)
    except Exception as exc:
        raise OracleError(f"dataframe_kurtosis failed: {exc}") from exc
    return {"expected_series": series_to_expected(out)}


def op_dataframe_prod(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_prod requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    axis = payload.get("prod_axis", 0)
    skipna = payload.get("prod_skipna", True)

    try:
        out = frame.prod(axis=axis, skipna=skipna)
    except Exception as exc:
        raise OracleError(f"dataframe_prod failed: {exc}") from exc
    return {"expected_series": series_to_expected(out)}


def op_dataframe_apply_prod_axis1(pd, payload: dict[str, Any]) -> dict[str, Any]:
    return op_dataframe_apply_builtin(
        pd, payload, "prod", 1, "dataframe_apply_prod_axis1"
    )


def op_dataframe_apply_product_axis1(pd, payload: dict[str, Any]) -> dict[str, Any]:
    return op_dataframe_apply_builtin(
        pd, payload, "product", 1, "dataframe_apply_product_axis1"
    )


def op_dataframe_sum(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_sum requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    axis = payload.get("sum_axis", 0)
    skipna = payload.get("sum_skipna", True)

    try:
        out = frame.sum(axis=axis, skipna=skipna)
    except Exception as exc:
        raise OracleError(f"dataframe_sum failed: {exc}") from exc
    return {"expected_series": series_to_expected(out)}


def op_dataframe_mean(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_mean requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    axis = payload.get("mean_axis", 0)
    skipna = payload.get("mean_skipna", True)

    try:
        out = frame.mean(axis=axis, skipna=skipna)
    except Exception as exc:
        raise OracleError(f"dataframe_mean failed: {exc}") from exc
    return {"expected_series": series_to_expected(out)}


def op_dataframe_std(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_std requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    axis = payload.get("std_axis", 0)
    skipna = payload.get("std_skipna", True)
    ddof = payload.get("std_ddof", 1)

    try:
        out = frame.std(axis=axis, skipna=skipna, ddof=ddof)
    except Exception as exc:
        raise OracleError(f"dataframe_std failed: {exc}") from exc
    return {"expected_series": series_to_expected(out)}


def op_dataframe_var(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_var requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    axis = payload.get("var_axis", 0)
    skipna = payload.get("var_skipna", True)
    ddof = payload.get("var_ddof", 1)

    try:
        out = frame.var(axis=axis, skipna=skipna, ddof=ddof)
    except Exception as exc:
        raise OracleError(f"dataframe_var failed: {exc}") from exc
    return {"expected_series": series_to_expected(out)}


def op_dataframe_min(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_min requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    axis = payload.get("min_axis", 0)
    skipna = payload.get("min_skipna", True)

    try:
        out = frame.min(axis=axis, skipna=skipna)
    except Exception as exc:
        raise OracleError(f"dataframe_min failed: {exc}") from exc
    return {"expected_series": series_to_expected(out)}


def op_dataframe_max(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_max requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    axis = payload.get("max_axis", 0)
    skipna = payload.get("max_skipna", True)

    try:
        out = frame.max(axis=axis, skipna=skipna)
    except Exception as exc:
        raise OracleError(f"dataframe_max failed: {exc}") from exc
    return {"expected_series": series_to_expected(out)}


def op_dataframe_median(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_median requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    axis = payload.get("median_axis", 0)
    skipna = payload.get("median_skipna", True)

    try:
        out = frame.median(axis=axis, skipna=skipna)
    except Exception as exc:
        raise OracleError(f"dataframe_median failed: {exc}") from exc
    return {"expected_series": series_to_expected(out)}


def op_dataframe_any(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_any requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    axis = payload.get("any_axis", 0)
    skipna = payload.get("any_skipna", True)

    try:
        out = frame.any(axis=axis, skipna=skipna)
    except Exception as exc:
        raise OracleError(f"dataframe_any failed: {exc}") from exc
    return {"expected_series": series_to_expected(out)}


def op_dataframe_all(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_all requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    axis = payload.get("all_axis", 0)
    skipna = payload.get("all_skipna", True)

    try:
        out = frame.all(axis=axis, skipna=skipna)
    except Exception as exc:
        raise OracleError(f"dataframe_all failed: {exc}") from exc
    return {"expected_series": series_to_expected(out)}


def op_dataframe_nunique(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_nunique requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    axis = payload.get("nunique_axis", 0)
    dropna = payload.get("nunique_dropna", True)

    try:
        out = frame.nunique(axis=axis, dropna=dropna)
    except Exception as exc:
        raise OracleError(f"dataframe_nunique failed: {exc}") from exc
    return {"expected_series": series_to_expected(out)}


def op_dataframe_apply_nunique_axis0(pd, payload: dict[str, Any]) -> dict[str, Any]:
    return op_dataframe_apply_builtin(
        pd, payload, "nunique", 0, "dataframe_apply_nunique_axis0"
    )


def op_dataframe_quantile(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_quantile requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    q = payload.get("quantile_q", 0.5)
    axis = payload.get("quantile_axis", 0)

    try:
        out = frame.quantile(q=q, axis=axis)
    except Exception as exc:
        raise OracleError(f"dataframe_quantile failed: {exc}") from exc
    return {"expected_series": series_to_expected(out)}


def op_dataframe_value_counts(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_value_counts requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    normalize = payload.get("value_counts_normalize", False)
    sort = payload.get("value_counts_sort", True)
    ascending = payload.get("value_counts_ascending", False)
    dropna = payload.get("value_counts_dropna", True)

    try:
        out = frame.value_counts(normalize=normalize, sort=sort, ascending=ascending, dropna=dropna)
    except Exception as exc:
        raise OracleError(f"dataframe_value_counts failed: {exc}") from exc
    return {"expected_series": series_to_expected(out)}


def op_dataframe_memory_usage(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_memory_usage requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    index = payload.get("memory_usage_index", True)
    deep = payload.get("memory_usage_deep", False)

    try:
        out = frame.memory_usage(index=index, deep=deep)
    except Exception as exc:
        raise OracleError(f"dataframe_memory_usage failed: {exc}") from exc
    return {"expected_series": series_to_expected(out)}


def op_dataframe_identity(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_identity requires frame payload")
    frame = dataframe_from_json(pd, frame_payload)
    return {"expected_frame": dataframe_to_json(frame)}


def op_dataframe_round(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_round requires frame payload")

    decimals = payload.get("round_decimals", 0)
    if isinstance(decimals, bool) or not isinstance(decimals, int):
        raise OracleError("dataframe_round round_decimals must be an integer when provided")
    frame = dataframe_from_json(pd, frame_payload)
    try:
        out = frame.round(decimals=int(decimals))
    except Exception as exc:
        raise OracleError(f"dataframe_round failed: {exc}") from exc
    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_pivot(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_pivot requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    values = parse_optional_string_list(payload, "pivot_values", "dataframe_pivot")
    if len(values) != 1:
        raise OracleError("dataframe_pivot requires exactly one pivot_values entry")
    index = required_string_payload(payload, "pivot_index", "dataframe_pivot")
    columns = required_string_payload(payload, "pivot_columns", "dataframe_pivot")

    try:
        out = frame.pivot(index=index, columns=columns, values=values[0])
    except Exception as exc:
        raise OracleError(f"dataframe_pivot failed: {exc}") from exc
    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_pivot_table(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_pivot_table requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    values = parse_optional_string_list(payload, "pivot_values", "dataframe_pivot_table")
    if not values:
        raise OracleError("dataframe_pivot_table requires pivot_values")
    index = required_string_payload(payload, "pivot_index", "dataframe_pivot_table")
    columns = required_string_payload(payload, "pivot_columns", "dataframe_pivot_table")
    aggfunc = required_string_payload(payload, "pivot_aggfunc", "dataframe_pivot_table")

    kwargs: dict[str, Any] = {
        "values": values[0] if len(values) == 1 else values,
        "index": index,
        "columns": columns,
        "aggfunc": aggfunc,
        "sort": False,
        "dropna": False,
        "margins": bool(payload.get("pivot_margins", False)),
    }
    if payload.get("fill_value") is not None:
        kwargs["fill_value"] = scalar_from_json(payload["fill_value"])
    margins_name = payload.get("pivot_margins_name")
    if margins_name is not None:
        kwargs["margins_name"] = str(margins_name)

    try:
        out = frame.pivot_table(**kwargs)
    except Exception as exc:
        raise OracleError(f"dataframe_pivot_table failed: {exc}") from exc

    if hasattr(out.columns, "to_flat_index"):
        flattened = []
        for name in out.columns.to_flat_index():
            if isinstance(name, tuple):
                flattened.append("_".join(str(part) for part in name if str(part) != ""))
            else:
                flattened.append(str(name))
        out = out.copy()
        out.columns = flattened
    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_stack(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_stack requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    try:
        out = frame.stack(dropna=False)
    except TypeError:
        out = frame.stack()
    except Exception as exc:
        raise OracleError(f"dataframe_stack failed: {exc}") from exc

    labels = []
    for row_key, column_key in out.index.tolist():
        labels.append({"kind": "utf8", "value": f"{row_key}|{column_key}"})
    return {
        "expected_frame": {
            "index": labels,
            "columns": {"value": [scalar_to_json(value) for value in out.tolist()]},
            "column_order": ["value"],
        }
    }


def op_dataframe_transpose(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_transpose requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    try:
        out = frame.transpose()
    except Exception as exc:
        raise OracleError(f"dataframe_transpose failed: {exc}") from exc

    return {"expected_frame": dataframe_to_json(out)}


def op_series_unstack(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    if left is None:
        raise OracleError("series_unstack requires left payload")

    labels = [label_from_json(item) for item in left["index"]]
    values = [scalar_from_json(item) for item in left["values"]]
    tuples = []
    for label in labels:
        text = str(label)
        if ", " not in text:
            raise OracleError("series_unstack index labels must contain ', '")
        row_key, column_key = text.split(", ", 1)
        tuples.append((row_key.strip(), column_key.strip()))

    series = pd.Series(values, index=pd.MultiIndex.from_tuples(tuples), name=left.get("name"))
    try:
        out = series.unstack()
    except Exception as exc:
        raise OracleError(f"series_unstack failed: {exc}") from exc
    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_crosstab(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    right = payload.get("right")
    if left is None or right is None:
        raise OracleError("dataframe_crosstab requires left and right series payloads")

    index_values = [scalar_from_json(item) for item in left["values"]]
    column_values = [scalar_from_json(item) for item in right["values"]]
    try:
        out = pd.crosstab(index_values, column_values, dropna=True)
    except Exception as exc:
        raise OracleError(f"dataframe_crosstab failed: {exc}") from exc
    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_crosstab_normalize(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    right = payload.get("right")
    if left is None or right is None:
        raise OracleError(
            "dataframe_crosstab_normalize requires left and right series payloads"
        )

    normalize = required_string_payload(
        payload, "crosstab_normalize", "dataframe_crosstab_normalize"
    )
    index_values = [scalar_from_json(item) for item in left["values"]]
    column_values = [scalar_from_json(item) for item in right["values"]]
    try:
        out = pd.crosstab(index_values, column_values, normalize=normalize, dropna=True)
    except Exception as exc:
        raise OracleError(f"dataframe_crosstab_normalize failed: {exc}") from exc
    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_get_dummies(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_get_dummies requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    columns = parse_optional_string_list(payload, "dummy_columns", "dataframe_get_dummies")
    kwargs: dict[str, Any] = {"dtype": int}
    if columns:
        kwargs["columns"] = columns
    try:
        out = pd.get_dummies(frame, **kwargs)
    except Exception as exc:
        raise OracleError(f"dataframe_get_dummies failed: {exc}") from exc
    return {"expected_frame": dataframe_to_json(out)}


def op_series_str_get_dummies(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    if left is None:
        raise OracleError("series_str_get_dummies requires left payload")
    sep = payload.get("string_sep", "|")
    if not isinstance(sep, str):
        raise OracleError("series_str_get_dummies string_sep must be a string")

    series = fixture_series_from_payload(pd, left, "series_str_get_dummies")
    try:
        out = series.str.get_dummies(sep=sep)
    except Exception as exc:
        raise OracleError(f"series_str_get_dummies failed: {exc}") from exc
    return {"expected_frame": dataframe_to_json(out)}


def rust_debug_index_label(value: Any) -> str:
    if isinstance(value, int):
        return f"Int64({value})"
    return f"Utf8({json.dumps(str(value), ensure_ascii=False)})"


def normalize_series_extractall_frame(frame):
    out = frame.copy()
    out.columns = [str(i) for i, _ in enumerate(out.columns.tolist())]
    out.index = [
        f"{rust_debug_index_label(label[0])}, {label[1]}"
        if isinstance(label, tuple) and len(label) == 2
        else str(label)
        for label in out.index.tolist()
    ]
    return out


def normalize_groupby_ohlc_frame(frame):
    if getattr(frame.columns, "nlevels", 1) <= 1:
        return frame

    top_level = [str(value) for value in frame.columns.get_level_values(0)]
    unique_top_level = list(dict.fromkeys(top_level))
    single_value_column = len(unique_top_level) == 1

    flattened_names: list[str] = []
    for column_name, stat_name in frame.columns.tolist():
        if single_value_column:
            flattened_names.append(str(stat_name))
        else:
            flattened_names.append(f"{column_name}_{stat_name}")

    out = frame.copy()
    out.columns = flattened_names
    return out


def op_dataframe_loc(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    loc_labels = payload.get("loc_labels")
    if frame_payload is None:
        raise OracleError("dataframe_loc requires frame payload")
    if not isinstance(loc_labels, list):
        raise OracleError("dataframe_loc requires loc_labels list payload")

    frame = dataframe_from_json(pd, frame_payload)
    labels = [label_from_json(item) for item in loc_labels]

    try:
        if hasattr(frame.index, "nlevels") and getattr(frame.index, "nlevels", 1) > 1:
            out = frame.loc[tuple(labels)]
        else:
            out = frame.loc[labels]
    except KeyError as exc:
        raise OracleError(f"dataframe_loc label lookup failed: {exc}") from exc

    if not hasattr(out, "columns"):
        raise OracleError("dataframe_loc currently requires DataFrame-shaped selections")

    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_xs(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    xs_key = payload.get("xs_key")
    xs_level = payload.get("xs_level")
    if frame_payload is None:
        raise OracleError("dataframe_xs requires frame payload")
    if xs_key is None:
        raise OracleError("dataframe_xs requires xs_key payload")
    if xs_level is not None and (isinstance(xs_level, bool) or not isinstance(xs_level, int)):
        raise OracleError("dataframe_xs xs_level must be an integer when provided")

    frame = dataframe_from_json(pd, frame_payload)
    key = label_from_json(xs_key)
    try:
        if xs_level is None:
            out = frame.xs(key)
        else:
            out = frame.xs(key, level=int(xs_level))
    except Exception as exc:
        raise OracleError(f"dataframe_xs failed: {exc}") from exc

    if not hasattr(out, "columns"):
        raise OracleError(
            "dataframe_xs currently requires duplicate-label selections that return a DataFrame"
        )

    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_iloc(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    iloc_positions = payload.get("iloc_positions")
    if frame_payload is None:
        raise OracleError("dataframe_iloc requires frame payload")
    if not isinstance(iloc_positions, list):
        raise OracleError("dataframe_iloc requires iloc_positions list payload")

    frame = dataframe_from_json(pd, frame_payload)
    try:
        positions = [int(value) for value in iloc_positions]
    except Exception as exc:  # pragma: no cover - defensive conversion
        raise OracleError(f"dataframe_iloc positions must be integers: {exc}") from exc

    try:
        out = frame.iloc[positions]
    except IndexError as exc:
        raise OracleError(f"dataframe_iloc position lookup failed: {exc}") from exc

    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_take(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    take_indices = payload.get("take_indices")
    axis = payload.get("take_axis", 0)
    if frame_payload is None:
        raise OracleError("dataframe_take requires frame payload")
    if not isinstance(take_indices, list):
        raise OracleError("dataframe_take requires take_indices list payload")
    if axis not in (0, 1):
        raise OracleError(f"dataframe_take take_axis must be 0 or 1 (got {axis!r})")

    frame = dataframe_from_json(pd, frame_payload)
    try:
        indices = [int(value) for value in take_indices]
    except Exception as exc:  # pragma: no cover - defensive conversion
        raise OracleError(f"dataframe_take indices must be integers: {exc}") from exc

    try:
        out = frame.take(indices, axis=axis)
    except IndexError as exc:
        raise OracleError(f"dataframe_take position lookup failed: {exc}") from exc

    return {"expected_frame": dataframe_to_json(out)}


def required_groupby_columns(payload: dict[str, Any], op_name: str) -> list[str]:
    columns = parse_optional_string_list(payload, "groupby_columns", op_name)
    if not columns:
        raise OracleError(f"{op_name} requires non-empty groupby_columns list")
    return columns


def format_groupby_resample_bucket_label(value: Any, freq: str) -> str:
    if freq == "M" and hasattr(value, "strftime"):
        try:
            return value.strftime("%Y-%m")
        except Exception:
            pass
    return str(value)


def normalize_groupby_resample_frame(frame, groupby_columns: list[str], freq: str):
    out = frame.copy()
    if getattr(out.index, "nlevels", 1) > 1:
        group_levels = list(range(out.index.nlevels - 1))
        out = out.reset_index(level=group_levels)
        rename_map: dict[Any, str] = {}
        for position, column in enumerate(groupby_columns):
            actual = out.columns[position]
            if actual != column:
                rename_map[actual] = column
        if rename_map:
            out = out.rename(columns=rename_map)
    labels = []
    for label in out.index.tolist():
        labels.append(format_groupby_resample_bucket_label(label, freq))
    out.index = labels
    return out


def normalize_groupby_rolling_frame(frame, groupby_columns: list[str]):
    out = frame.copy()
    if getattr(out.index, "nlevels", 1) > 1:
        group_levels = list(range(out.index.nlevels - 1))
        out = out.reset_index(level=group_levels)
        rename_map: dict[Any, str] = {}
        for position, column in enumerate(groupby_columns):
            actual = out.columns[position]
            if actual != column:
                rename_map[actual] = column
        if rename_map:
            out = out.rename(columns=rename_map)
    return out


def op_dataframe_groupby_rolling_builtin(
    pd, payload: dict[str, Any], func: str, op_name: str
) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError(f"{op_name} requires frame payload")

    columns = required_groupby_columns(payload, op_name)
    window_size = payload.get("window_size", 3)
    if not isinstance(window_size, int) or window_size <= 0:
        raise OracleError(f"{op_name} requires positive integer window_size")

    frame = dataframe_from_json(pd, frame_payload)

    try:
        out = getattr(frame.groupby(columns).rolling(window_size), func)()
        out = normalize_groupby_rolling_frame(out, columns)
    except Exception as exc:
        raise OracleError(f"{op_name} failed: {exc}") from exc

    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_groupby_resample_builtin(
    pd, payload: dict[str, Any], func: str, op_name: str
) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError(f"{op_name} requires frame payload")

    columns = required_groupby_columns(payload, op_name)
    freq = required_string_payload(payload, "resample_freq", op_name)
    frame = dataframe_from_json(pd, frame_payload)
    frame.index = pd.DatetimeIndex(frame.index)

    try:
        out = getattr(frame.groupby(columns).resample(freq), func)()
        out = normalize_groupby_resample_frame(out, columns, freq)
    except Exception as exc:
        raise OracleError(f"{op_name} failed: {exc}") from exc

    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_groupby_idxmin(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    groupby_columns = payload.get("groupby_columns")
    if frame_payload is None:
        raise OracleError("dataframe_groupby_idxmin requires frame payload")
    if not isinstance(groupby_columns, list) or not groupby_columns:
        raise OracleError("dataframe_groupby_idxmin requires non-empty groupby_columns list")

    columns: list[str] = []
    for entry in groupby_columns:
        if not isinstance(entry, str) or not entry.strip():
            raise OracleError(
                "dataframe_groupby_idxmin groupby_columns entries must be non-empty strings"
            )
        columns.append(entry.strip())

    frame = dataframe_from_json(pd, frame_payload)
    try:
        out = frame.groupby(columns).idxmin()
    except Exception as exc:
        raise OracleError(f"dataframe_groupby_idxmin failed: {exc}") from exc

    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_groupby_sum(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    groupby_columns = payload.get("groupby_columns")
    observed = payload.get("groupby_observed", True)
    if frame_payload is None:
        raise OracleError("dataframe_groupby_sum requires frame payload")
    if not isinstance(groupby_columns, list) or not groupby_columns:
        raise OracleError("dataframe_groupby_sum requires non-empty groupby_columns list")
    if not isinstance(observed, bool):
        raise OracleError("dataframe_groupby_sum groupby_observed must be a boolean")

    columns: list[str] = []
    for entry in groupby_columns:
        if not isinstance(entry, str) or not entry.strip():
            raise OracleError(
                "dataframe_groupby_sum groupby_columns entries must be non-empty strings"
            )
        columns.append(entry.strip())

    frame = dataframe_from_json(pd, frame_payload)
    try:
        out = frame.groupby(columns, observed=observed).sum()
    except Exception as exc:
        raise OracleError(f"dataframe_groupby_sum failed: {exc}") from exc

    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_groupby_agg_multi(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    groupby_columns = payload.get("groupby_columns")
    agg_multi = payload.get("groupby_agg_multi")
    observed = payload.get("groupby_observed", True)
    if frame_payload is None:
        raise OracleError("dataframe_groupby_agg_multi requires frame payload")
    if not isinstance(groupby_columns, list) or not groupby_columns:
        raise OracleError(
            "dataframe_groupby_agg_multi requires non-empty groupby_columns list"
        )
    if not isinstance(agg_multi, dict) or not agg_multi:
        raise OracleError(
            "dataframe_groupby_agg_multi requires non-empty groupby_agg_multi object"
        )
    if not isinstance(observed, bool):
        raise OracleError("dataframe_groupby_agg_multi groupby_observed must be a boolean")

    columns = [str(entry).strip() for entry in groupby_columns]
    if any(not entry for entry in columns):
        raise OracleError(
            "dataframe_groupby_agg_multi groupby_columns entries must be non-empty strings"
        )

    func_map: dict[str, list[str]] = {}
    for raw_name, raw_funcs in agg_multi.items():
        name = str(raw_name)
        if not isinstance(raw_funcs, list) or not raw_funcs:
            raise OracleError(
                f"dataframe_groupby_agg_multi groupby_agg_multi[{name!r}] must be a non-empty list"
            )
        funcs = [str(func).strip() for func in raw_funcs]
        if any(not func for func in funcs):
            raise OracleError(
                f"dataframe_groupby_agg_multi groupby_agg_multi[{name!r}] contains empty aggfuncs"
            )
        func_map[name] = funcs

    frame = dataframe_from_json(pd, frame_payload)
    try:
        out = frame.groupby(columns, observed=observed).agg(func_map)
    except Exception as exc:
        raise OracleError(f"dataframe_groupby_agg_multi failed: {exc}") from exc

    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_groupby_idxmax(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    groupby_columns = payload.get("groupby_columns")
    if frame_payload is None:
        raise OracleError("dataframe_groupby_idxmax requires frame payload")
    if not isinstance(groupby_columns, list) or not groupby_columns:
        raise OracleError("dataframe_groupby_idxmax requires non-empty groupby_columns list")

    columns: list[str] = []
    for entry in groupby_columns:
        if not isinstance(entry, str) or not entry.strip():
            raise OracleError(
                "dataframe_groupby_idxmax groupby_columns entries must be non-empty strings"
            )
        columns.append(entry.strip())

    frame = dataframe_from_json(pd, frame_payload)
    try:
        out = frame.groupby(columns).idxmax()
    except Exception as exc:
        raise OracleError(f"dataframe_groupby_idxmax failed: {exc}") from exc

    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_groupby_any(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    groupby_columns = payload.get("groupby_columns")
    if frame_payload is None:
        raise OracleError("dataframe_groupby_any requires frame payload")
    if not isinstance(groupby_columns, list) or not groupby_columns:
        raise OracleError("dataframe_groupby_any requires non-empty groupby_columns list")

    columns: list[str] = []
    for entry in groupby_columns:
        if not isinstance(entry, str) or not entry.strip():
            raise OracleError(
                "dataframe_groupby_any groupby_columns entries must be non-empty strings"
            )
        columns.append(entry.strip())

    frame = dataframe_from_json(pd, frame_payload)
    try:
        out = frame.groupby(columns).any()
    except Exception as exc:
        raise OracleError(f"dataframe_groupby_any failed: {exc}") from exc

    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_groupby_all(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    groupby_columns = payload.get("groupby_columns")
    if frame_payload is None:
        raise OracleError("dataframe_groupby_all requires frame payload")
    if not isinstance(groupby_columns, list) or not groupby_columns:
        raise OracleError("dataframe_groupby_all requires non-empty groupby_columns list")

    columns: list[str] = []
    for entry in groupby_columns:
        if not isinstance(entry, str) or not entry.strip():
            raise OracleError(
                "dataframe_groupby_all groupby_columns entries must be non-empty strings"
            )
        columns.append(entry.strip())

    frame = dataframe_from_json(pd, frame_payload)
    try:
        out = frame.groupby(columns).all()
    except Exception as exc:
        raise OracleError(f"dataframe_groupby_all failed: {exc}") from exc

    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_groupby_get_group(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    groupby_columns = payload.get("groupby_columns")
    group_name = payload.get("group_name")
    if frame_payload is None:
        raise OracleError("dataframe_groupby_get_group requires frame payload")
    if not isinstance(groupby_columns, list) or not groupby_columns:
        raise OracleError("dataframe_groupby_get_group requires non-empty groupby_columns list")
    if not isinstance(group_name, str) or not group_name:
        raise OracleError("dataframe_groupby_get_group requires non-empty group_name")

    columns: list[str] = []
    for entry in groupby_columns:
        if not isinstance(entry, str) or not entry.strip():
            raise OracleError(
                "dataframe_groupby_get_group groupby_columns entries must be non-empty strings"
            )
        columns.append(entry.strip())

    frame = dataframe_from_json(pd, frame_payload)
    try:
        out = frame.groupby(columns).get_group(group_name)
    except Exception as exc:
        raise OracleError(f"dataframe_groupby_get_group failed: {exc}") from exc

    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_groupby_ffill(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    groupby_columns = payload.get("groupby_columns")
    if frame_payload is None:
        raise OracleError("dataframe_groupby_ffill requires frame payload")
    if not isinstance(groupby_columns, list) or not groupby_columns:
        raise OracleError("dataframe_groupby_ffill requires non-empty groupby_columns list")

    columns: list[str] = []
    for entry in groupby_columns:
        if not isinstance(entry, str) or not entry.strip():
            raise OracleError(
                "dataframe_groupby_ffill groupby_columns entries must be non-empty strings"
            )
        columns.append(entry.strip())

    frame = dataframe_from_json(pd, frame_payload)
    try:
        out = frame.groupby(columns).ffill()
    except Exception as exc:
        raise OracleError(f"dataframe_groupby_ffill failed: {exc}") from exc

    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_groupby_bfill(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    groupby_columns = payload.get("groupby_columns")
    if frame_payload is None:
        raise OracleError("dataframe_groupby_bfill requires frame payload")
    if not isinstance(groupby_columns, list) or not groupby_columns:
        raise OracleError("dataframe_groupby_bfill requires non-empty groupby_columns list")

    columns: list[str] = []
    for entry in groupby_columns:
        if not isinstance(entry, str) or not entry.strip():
            raise OracleError(
                "dataframe_groupby_bfill groupby_columns entries must be non-empty strings"
            )
        columns.append(entry.strip())

    frame = dataframe_from_json(pd, frame_payload)
    try:
        out = frame.groupby(columns).bfill()
    except Exception as exc:
        raise OracleError(f"dataframe_groupby_bfill failed: {exc}") from exc

    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_groupby_sem(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    groupby_columns = payload.get("groupby_columns")
    if frame_payload is None:
        raise OracleError("dataframe_groupby_sem requires frame payload")
    if not isinstance(groupby_columns, list) or not groupby_columns:
        raise OracleError("dataframe_groupby_sem requires non-empty groupby_columns list")

    columns: list[str] = []
    for entry in groupby_columns:
        if not isinstance(entry, str) or not entry.strip():
            raise OracleError(
                "dataframe_groupby_sem groupby_columns entries must be non-empty strings"
            )
        columns.append(entry.strip())

    frame = dataframe_from_json(pd, frame_payload)
    try:
        out = frame.groupby(columns).sem()
    except Exception as exc:
        raise OracleError(f"dataframe_groupby_sem failed: {exc}") from exc

    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_groupby_skew(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    groupby_columns = payload.get("groupby_columns")
    if frame_payload is None:
        raise OracleError("dataframe_groupby_skew requires frame payload")
    if not isinstance(groupby_columns, list) or not groupby_columns:
        raise OracleError("dataframe_groupby_skew requires non-empty groupby_columns list")

    columns: list[str] = []
    for entry in groupby_columns:
        if not isinstance(entry, str) or not entry.strip():
            raise OracleError(
                "dataframe_groupby_skew groupby_columns entries must be non-empty strings"
            )
        columns.append(entry.strip())

    frame = dataframe_from_json(pd, frame_payload)
    try:
        out = frame.groupby(columns).skew()
    except Exception as exc:
        raise OracleError(f"dataframe_groupby_skew failed: {exc}") from exc

    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_groupby_kurtosis(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    groupby_columns = payload.get("groupby_columns")
    if frame_payload is None:
        raise OracleError("dataframe_groupby_kurtosis requires frame payload")
    if not isinstance(groupby_columns, list) or not groupby_columns:
        raise OracleError("dataframe_groupby_kurtosis requires non-empty groupby_columns list")

    columns: list[str] = []
    for entry in groupby_columns:
        if not isinstance(entry, str) or not entry.strip():
            raise OracleError(
                "dataframe_groupby_kurtosis groupby_columns entries must be non-empty strings"
            )
        columns.append(entry.strip())

    frame = dataframe_from_json(pd, frame_payload)
    try:
        out = frame.groupby(columns).kurt()
    except Exception as exc:
        raise OracleError(f"dataframe_groupby_kurtosis failed: {exc}") from exc

    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_groupby_ohlc(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    groupby_columns = payload.get("groupby_columns")
    if frame_payload is None:
        raise OracleError("dataframe_groupby_ohlc requires frame payload")
    if not isinstance(groupby_columns, list) or not groupby_columns:
        raise OracleError("dataframe_groupby_ohlc requires non-empty groupby_columns list")

    columns: list[str] = []
    for entry in groupby_columns:
        if not isinstance(entry, str) or not entry.strip():
            raise OracleError(
                "dataframe_groupby_ohlc groupby_columns entries must be non-empty strings"
            )
        columns.append(entry.strip())

    frame = dataframe_from_json(pd, frame_payload)
    try:
        out = normalize_groupby_ohlc_frame(frame.groupby(columns).ohlc())
    except Exception as exc:
        raise OracleError(f"dataframe_groupby_ohlc failed: {exc}") from exc

    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_groupby_resample_min(pd, payload: dict[str, Any]) -> dict[str, Any]:
    return op_dataframe_groupby_resample_builtin(
        pd, payload, "min", "dataframe_groupby_resample_min"
    )


def op_dataframe_groupby_resample_max(pd, payload: dict[str, Any]) -> dict[str, Any]:
    return op_dataframe_groupby_resample_builtin(
        pd, payload, "max", "dataframe_groupby_resample_max"
    )


def op_dataframe_groupby_resample_count(pd, payload: dict[str, Any]) -> dict[str, Any]:
    return op_dataframe_groupby_resample_builtin(
        pd, payload, "count", "dataframe_groupby_resample_count"
    )


def op_dataframe_groupby_resample_first(pd, payload: dict[str, Any]) -> dict[str, Any]:
    return op_dataframe_groupby_resample_builtin(
        pd, payload, "first", "dataframe_groupby_resample_first"
    )


def op_dataframe_groupby_resample_last(pd, payload: dict[str, Any]) -> dict[str, Any]:
    return op_dataframe_groupby_resample_builtin(
        pd, payload, "last", "dataframe_groupby_resample_last"
    )


def op_dataframe_groupby_rolling_mean(pd, payload: dict[str, Any]) -> dict[str, Any]:
    return op_dataframe_groupby_rolling_builtin(
        pd, payload, "mean", "dataframe_groupby_rolling_mean"
    )


def op_dataframe_groupby_rolling_sum(pd, payload: dict[str, Any]) -> dict[str, Any]:
    return op_dataframe_groupby_rolling_builtin(
        pd, payload, "sum", "dataframe_groupby_rolling_sum"
    )


def op_dataframe_groupby_rolling_min(pd, payload: dict[str, Any]) -> dict[str, Any]:
    return op_dataframe_groupby_rolling_builtin(
        pd, payload, "min", "dataframe_groupby_rolling_min"
    )


def op_dataframe_groupby_rolling_max(pd, payload: dict[str, Any]) -> dict[str, Any]:
    return op_dataframe_groupby_rolling_builtin(
        pd, payload, "max", "dataframe_groupby_rolling_max"
    )


def op_dataframe_groupby_rolling_count(pd, payload: dict[str, Any]) -> dict[str, Any]:
    return op_dataframe_groupby_rolling_builtin(
        pd, payload, "count", "dataframe_groupby_rolling_count"
    )


def op_dataframe_groupby_rolling_std(pd, payload: dict[str, Any]) -> dict[str, Any]:
    return op_dataframe_groupby_rolling_builtin(
        pd, payload, "std", "dataframe_groupby_rolling_std"
    )


def op_dataframe_groupby_rolling_var(pd, payload: dict[str, Any]) -> dict[str, Any]:
    return op_dataframe_groupby_rolling_builtin(
        pd, payload, "var", "dataframe_groupby_rolling_var"
    )


def op_dataframe_groupby_cumcount(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    groupby_columns = payload.get("groupby_columns")
    if frame_payload is None:
        raise OracleError("dataframe_groupby_cumcount requires frame payload")
    if not isinstance(groupby_columns, list) or not groupby_columns:
        raise OracleError("dataframe_groupby_cumcount requires non-empty groupby_columns list")

    columns: list[str] = []
    for entry in groupby_columns:
        if not isinstance(entry, str) or not entry.strip():
            raise OracleError(
                "dataframe_groupby_cumcount groupby_columns entries must be non-empty strings"
            )
        columns.append(entry.strip())

    frame = dataframe_from_json(pd, frame_payload)
    ascending = _resolve_sort_ascending(payload, "dataframe_groupby_cumcount")
    try:
        out = frame.groupby(columns).cumcount(ascending=ascending)
    except Exception as exc:
        raise OracleError(f"dataframe_groupby_cumcount failed: {exc}") from exc

    return {"expected_series": series_to_expected(out)}


def op_dataframe_groupby_ngroup(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    groupby_columns = payload.get("groupby_columns")
    if frame_payload is None:
        raise OracleError("dataframe_groupby_ngroup requires frame payload")
    if not isinstance(groupby_columns, list) or not groupby_columns:
        raise OracleError("dataframe_groupby_ngroup requires non-empty groupby_columns list")

    columns: list[str] = []
    for entry in groupby_columns:
        if not isinstance(entry, str) or not entry.strip():
            raise OracleError(
                "dataframe_groupby_ngroup groupby_columns entries must be non-empty strings"
            )
        columns.append(entry.strip())

    frame = dataframe_from_json(pd, frame_payload)
    try:
        out = frame.groupby(columns).ngroup()
    except Exception as exc:
        raise OracleError(f"dataframe_groupby_ngroup failed: {exc}") from exc

    return {"expected_series": series_to_expected(out)}


def op_dataframe_asof(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    asof_label = payload.get("asof_label")
    subset = payload.get("subset")
    if frame_payload is None:
        raise OracleError("dataframe_asof requires frame payload")
    if asof_label is None:
        raise OracleError("dataframe_asof requires asof_label payload")
    if subset is not None and not isinstance(subset, list):
        raise OracleError("dataframe_asof subset must be a list when provided")

    frame = dataframe_from_json(pd, frame_payload)
    frame.index = pd.DatetimeIndex(frame.index)
    label = label_from_json(asof_label)
    subset_columns = None
    if subset is not None:
        subset_columns = []
        for entry in subset:
            if not isinstance(entry, str) or not entry.strip():
                raise OracleError("dataframe_asof subset entries must be non-empty strings")
            subset_columns.append(entry.strip())

    try:
        out = frame.asof(label, subset=subset_columns)
    except Exception as exc:
        raise OracleError(f"dataframe_asof selection failed: {exc}") from exc

    return {"expected_series": series_to_expected(out)}


def op_dataframe_at_time(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    time_value = payload.get("time_value")
    if frame_payload is None:
        raise OracleError("dataframe_at_time requires frame payload")
    if not isinstance(time_value, str) or not time_value:
        raise OracleError("dataframe_at_time requires non-empty time_value payload")

    frame = dataframe_from_json(pd, frame_payload)
    frame.index = pd.DatetimeIndex(frame.index)
    try:
        out = frame.at_time(time_value)
    except Exception as exc:
        raise OracleError(f"dataframe_at_time selection failed: {exc}") from exc

    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_between_time(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    start_time = payload.get("start_time")
    end_time = payload.get("end_time")
    if frame_payload is None:
        raise OracleError("dataframe_between_time requires frame payload")
    if not isinstance(start_time, str) or not start_time:
        raise OracleError("dataframe_between_time requires non-empty start_time payload")
    if not isinstance(end_time, str) or not end_time:
        raise OracleError("dataframe_between_time requires non-empty end_time payload")

    frame = dataframe_from_json(pd, frame_payload)
    frame.index = pd.DatetimeIndex(frame.index)
    try:
        out = frame.between_time(start_time, end_time)
    except Exception as exc:
        raise OracleError(f"dataframe_between_time selection failed: {exc}") from exc

    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_head(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    head_n = payload.get("head_n")
    if frame_payload is None:
        raise OracleError("dataframe_head requires frame payload")
    if head_n is None:
        raise OracleError("dataframe_head requires head_n payload")

    frame = dataframe_from_json(pd, frame_payload)
    try:
        n = int(head_n)
    except Exception as exc:  # pragma: no cover - defensive conversion
        raise OracleError(f"dataframe_head head_n must be an integer: {exc}") from exc

    out = frame.head(n)
    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_tail(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    tail_n = payload.get("tail_n")
    if frame_payload is None:
        raise OracleError("dataframe_tail requires frame payload")
    if tail_n is None:
        raise OracleError("dataframe_tail requires tail_n payload")

    frame = dataframe_from_json(pd, frame_payload)
    try:
        n = int(tail_n)
    except Exception as exc:  # pragma: no cover - defensive conversion
        raise OracleError(f"dataframe_tail tail_n must be an integer: {exc}") from exc

    out = frame.tail(n)
    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_isna(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_isna requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    out = frame.isna()
    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_notna(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_notna requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    out = frame.notna()
    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_isnull(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_isnull requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    out = frame.isnull()
    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_notnull(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_notnull requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    out = frame.notnull()
    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_count(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_count requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    out = frame.count(axis=0, numeric_only=False)
    return {
        "expected_series": {
            "index": [label_to_json(v) for v in out.index.tolist()],
            "values": [scalar_to_json(v) for v in out.tolist()],
        }
    }


def op_dataframe_mode(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_mode requires frame payload")

    axis = payload.get("mode_axis")
    if axis is None:
        axis = 0
    if axis not in (0, 1):
        raise OracleError(f"dataframe_mode mode_axis must be 0 or 1 (got {axis!r})")

    numeric_only = payload.get("mode_numeric_only")
    if numeric_only is None:
        numeric_only = False

    dropna = payload.get("mode_dropna")
    if dropna is None:
        dropna = True

    frame = dataframe_from_json(pd, frame_payload)
    out = frame.mode(axis=axis, numeric_only=bool(numeric_only), dropna=bool(dropna))
    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_rank(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_rank requires frame payload")

    method = payload.get("rank_method") or "average"
    na_option = payload.get("rank_na_option") or "keep"
    ascending = payload.get("sort_ascending")
    if ascending is None:
        ascending = True
    pct = bool(payload.get("rank_pct", False))
    axis = payload.get("rank_axis")
    if axis is None:
        axis = 0
    if axis not in (0, 1):
        raise OracleError(f"dataframe_rank rank_axis must be 0 or 1 (got {axis!r})")

    frame = dataframe_from_json(pd, frame_payload)
    out = frame.rank(
        method=method,
        ascending=ascending,
        na_option=na_option,
        axis=axis,
        pct=pct,
    )
    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_fillna(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    fill_value_payload = payload.get("fill_value")
    if frame_payload is None:
        raise OracleError("dataframe_fillna requires frame payload")
    if fill_value_payload is None:
        raise OracleError("dataframe_fillna requires fill_value payload")

    frame = dataframe_from_json(pd, frame_payload)
    fill_value = scalar_from_json(fill_value_payload)
    try:
        out = frame.fillna(fill_value)
    except Exception as exc:
        raise OracleError(f"dataframe_fillna failed: {exc}") from exc
    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_dropna(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_dropna requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    out = frame.dropna()
    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_dropna_columns(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_dropna_columns requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    out = frame.dropna(axis=1)
    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_bool(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_bool requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    try:
        out = bool(frame.bool())
    except Exception as exc:
        raise OracleError(f"dataframe_bool failed: {exc}") from exc
    return {"expected_bool": out}


def _resolve_duplicate_subset(payload: dict[str, Any], op_name: str):
    raw_subset = payload.get("subset")
    if raw_subset is None:
        return None
    if not isinstance(raw_subset, list):
        raise OracleError(f"{op_name} subset must be an array of strings")

    subset: list[str] = []
    for value in raw_subset:
        if not isinstance(value, str) or value.strip() == "":
            raise OracleError(f"{op_name} subset entries must be non-empty strings")
        subset.append(value.strip())
    return subset


def _resolve_duplicate_keep(payload: dict[str, Any], op_name: str):
    raw_keep = payload.get("keep")
    if raw_keep is None:
        return "first"
    if not isinstance(raw_keep, str):
        raise OracleError(f"{op_name} keep must be a string")
    keep = raw_keep.strip().lower()
    if keep == "first":
        return "first"
    if keep == "last":
        return "last"
    if keep == "none":
        return False
    raise OracleError(
        f"{op_name} keep must be one of 'first', 'last', or 'none' (got {raw_keep!r})"
    )


def _resolve_drop_duplicates_ignore_index(payload: dict[str, Any], op_name: str) -> bool:
    raw = payload.get("ignore_index")
    if raw is None:
        return False
    if isinstance(raw, bool):
        return raw
    raise OracleError(f"{op_name} ignore_index must be a boolean")


def _require_explode_column(payload: dict[str, Any], op_name: str) -> str:
    raw = payload.get("explode_column")
    if not isinstance(raw, str) or raw.strip() == "":
        raise OracleError(f"{op_name} explode_column must be a non-empty string")
    return raw


def op_dataframe_duplicated(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_duplicated requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    subset = _resolve_duplicate_subset(payload, "dataframe_duplicated")
    keep = _resolve_duplicate_keep(payload, "dataframe_duplicated")
    try:
        out = frame.duplicated(subset=subset, keep=keep)
    except Exception as exc:
        raise OracleError(f"dataframe_duplicated failed: {exc}") from exc
    return {"expected_series": series_to_expected(out)}


def op_dataframe_drop_duplicates(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_drop_duplicates requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    subset = _resolve_duplicate_subset(payload, "dataframe_drop_duplicates")
    keep = _resolve_duplicate_keep(payload, "dataframe_drop_duplicates")
    ignore_index = _resolve_drop_duplicates_ignore_index(
        payload, "dataframe_drop_duplicates"
    )
    try:
        out = frame.drop_duplicates(
            subset=subset, keep=keep, ignore_index=ignore_index
        )
    except Exception as exc:
        raise OracleError(f"dataframe_drop_duplicates failed: {exc}") from exc
    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_explode(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_explode requires frame payload")

    frame = dataframe_from_json(pd, frame_payload).copy()
    explode_column = _require_explode_column(payload, "dataframe_explode")
    if explode_column not in frame.columns:
        raise OracleError(
            f"dataframe_explode explode_column {explode_column!r} not found"
        )

    string_sep = payload.get("string_sep")
    if not isinstance(string_sep, str) or string_sep == "":
        raise OracleError("dataframe_explode string_sep must be a non-empty string")

    ignore_index = _resolve_drop_duplicates_ignore_index(payload, "dataframe_explode")

    def _prepare_explode_value(value: Any) -> Any:
        if isinstance(value, str):
            return [part.strip() for part in value.split(string_sep)]
        return value

    frame[explode_column] = frame[explode_column].map(_prepare_explode_value)
    try:
        out = frame.explode(explode_column, ignore_index=ignore_index)
    except Exception as exc:
        raise OracleError(f"dataframe_explode failed: {exc}") from exc
    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_set_index(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_set_index requires frame payload")

    set_index_column = payload.get("set_index_column")
    if not isinstance(set_index_column, str) or set_index_column.strip() == "":
        raise OracleError(
            "dataframe_set_index requires set_index_column string payload"
        )

    set_index_drop = payload.get("set_index_drop")
    if not isinstance(set_index_drop, bool):
        raise OracleError("dataframe_set_index requires set_index_drop boolean payload")

    set_index_verify_integrity = payload.get("set_index_verify_integrity", False)
    if not isinstance(set_index_verify_integrity, bool):
        raise OracleError(
            "dataframe_set_index requires set_index_verify_integrity boolean payload"
        )

    frame = dataframe_from_json(pd, frame_payload)
    column_name = set_index_column.strip()
    if column_name not in frame.columns:
        raise OracleError(f"dataframe_set_index column '{column_name}' not found")
    try:
        out = frame.set_index(
            column_name,
            drop=set_index_drop,
            verify_integrity=set_index_verify_integrity,
        )
    except Exception as exc:
        raise OracleError(f"dataframe_set_index failed: {exc}") from exc
    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_reset_index(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_reset_index requires frame payload")

    reset_index_drop = payload.get("reset_index_drop")
    if not isinstance(reset_index_drop, bool):
        raise OracleError(
            "dataframe_reset_index requires reset_index_drop boolean payload"
        )

    frame = dataframe_from_json(pd, frame_payload)
    try:
        out = frame.reset_index(drop=reset_index_drop)
    except Exception as exc:
        raise OracleError(f"dataframe_reset_index failed: {exc}") from exc
    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_insert(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    loc = payload.get("insert_loc")
    column = payload.get("insert_column")
    values = payload.get("insert_values")
    if frame_payload is None:
        raise OracleError("dataframe_insert requires frame payload")
    if not isinstance(loc, int) or isinstance(loc, bool) or loc < 0:
        raise OracleError("dataframe_insert requires non-negative integer insert_loc")
    if not isinstance(column, str) or column.strip() == "":
        raise OracleError("dataframe_insert requires insert_column string payload")
    if not isinstance(values, list):
        raise OracleError("dataframe_insert requires insert_values list payload")

    frame = dataframe_from_json(pd, frame_payload)
    parsed_values = [scalar_from_json(value) for value in values]
    try:
        out = frame.copy()
        out.insert(loc=loc, column=column, value=parsed_values)
    except Exception as exc:
        raise OracleError(f"dataframe_insert failed: {exc}") from exc
    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_assign(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    assignments = payload.get("assignments")
    if frame_payload is None:
        raise OracleError("dataframe_assign requires frame payload")
    if not isinstance(assignments, list) or not assignments:
        raise OracleError("dataframe_assign requires non-empty assignments list")

    frame = dataframe_from_json(pd, frame_payload)
    out = frame.copy()
    for assignment in assignments:
        if not isinstance(assignment, dict):
            raise OracleError("dataframe_assign assignments must be objects")
        name = assignment.get("name")
        values = assignment.get("values")
        if not isinstance(name, str):
            raise OracleError("dataframe_assign assignment name must be a string")
        if not isinstance(values, list):
            raise OracleError("dataframe_assign assignment values must be a list")
        parsed_values = [scalar_from_json(value) for value in values]
        try:
            out = out.assign(**{name: parsed_values})
        except Exception as exc:
            raise OracleError(f"dataframe_assign failed: {exc}") from exc
    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_rename_columns(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    renames = payload.get("rename_columns")
    if frame_payload is None:
        raise OracleError("dataframe_rename_columns requires frame payload")
    if not isinstance(renames, list) or not renames:
        raise OracleError("dataframe_rename_columns requires non-empty rename_columns list")

    mapping: dict[str, str] = {}
    for rename in renames:
        if not isinstance(rename, dict):
            raise OracleError("dataframe_rename_columns entries must be objects")
        source = rename.get("from")
        target = rename.get("to")
        if not isinstance(source, str) or not isinstance(target, str):
            raise OracleError("dataframe_rename_columns entries require string from/to")
        mapping[source] = target

    frame = dataframe_from_json(pd, frame_payload)
    try:
        out = frame.rename(columns=mapping)
    except Exception as exc:
        raise OracleError(f"dataframe_rename_columns failed: {exc}") from exc
    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_reindex(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    labels = payload.get("reindex_labels")
    if frame_payload is None:
        raise OracleError("dataframe_reindex requires frame payload")
    if not isinstance(labels, list):
        raise OracleError("dataframe_reindex requires reindex_labels list")

    frame = dataframe_from_json(pd, frame_payload)
    parsed_labels = [label_from_json(label) for label in labels]
    try:
        out = frame.reindex(parsed_labels)
    except Exception as exc:
        raise OracleError(f"dataframe_reindex failed: {exc}") from exc
    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_reindex_columns(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    columns = payload.get("reindex_columns")
    if frame_payload is None:
        raise OracleError("dataframe_reindex_columns requires frame payload")
    if not isinstance(columns, list):
        raise OracleError("dataframe_reindex_columns requires reindex_columns list")
    if not all(isinstance(column, str) for column in columns):
        raise OracleError("dataframe_reindex_columns entries must be strings")

    frame = dataframe_from_json(pd, frame_payload)
    try:
        out = frame.reindex(columns=columns)
    except Exception as exc:
        raise OracleError(f"dataframe_reindex_columns failed: {exc}") from exc
    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_drop_columns(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    columns = payload.get("drop_columns")
    if frame_payload is None:
        raise OracleError("dataframe_drop_columns requires frame payload")
    if not isinstance(columns, list):
        raise OracleError("dataframe_drop_columns requires drop_columns list")
    if not all(isinstance(column, str) for column in columns):
        raise OracleError("dataframe_drop_columns entries must be strings")

    frame = dataframe_from_json(pd, frame_payload)
    try:
        out = frame.drop(columns=columns)
    except Exception as exc:
        raise OracleError(f"dataframe_drop_columns failed: {exc}") from exc
    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_replace(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    to_find = payload.get("replace_to_find")
    to_value = payload.get("replace_to_value")
    if frame_payload is None:
        raise OracleError("dataframe_replace requires frame payload")
    if not isinstance(to_find, list):
        raise OracleError("dataframe_replace requires replace_to_find list")
    if not isinstance(to_value, list):
        raise OracleError("dataframe_replace requires replace_to_value list")
    if len(to_find) != len(to_value):
        raise OracleError("dataframe_replace replacement lists must have the same length")

    frame = dataframe_from_json(pd, frame_payload)
    find_values = [scalar_from_json(value) for value in to_find]
    replace_values = [scalar_from_json(value) for value in to_value]
    try:
        out = frame.replace(find_values, replace_values)
    except Exception as exc:
        raise OracleError(f"dataframe_replace failed: {exc}") from exc
    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_where(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    cond_payload = payload.get("frame_right")
    if frame_payload is None:
        raise OracleError("dataframe_where requires frame payload")
    if cond_payload is None:
        raise OracleError("dataframe_where requires frame_right condition payload")

    frame = dataframe_from_json(pd, frame_payload)
    cond = dataframe_from_json(pd, cond_payload)
    for column in frame.columns.tolist():
        if column not in cond.columns.tolist():
            raise OracleError(f"where: condition missing column {column!r}")

    fill_value = payload.get("fill_value")
    try:
        if fill_value is None:
            out = frame.where(cond)
        else:
            out = frame.where(cond, other=scalar_from_json(fill_value))
    except Exception as exc:
        raise OracleError(f"dataframe_where failed: {exc}") from exc
    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_where_df(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    cond_payload = payload.get("frame_right")
    other_payload = payload.get("frame_other")
    if frame_payload is None:
        raise OracleError("dataframe_where_df requires frame payload")
    if cond_payload is None:
        raise OracleError("dataframe_where_df requires frame_right condition payload")
    if other_payload is None:
        raise OracleError("dataframe_where_df requires frame_other payload")

    frame = dataframe_from_json(pd, frame_payload)
    cond = dataframe_from_json(pd, cond_payload)
    other = dataframe_from_json(pd, other_payload)
    for column in frame.columns.tolist():
        if column not in cond.columns.tolist():
            raise OracleError(f"where_cond_df: condition missing column {column!r}")

    try:
        out = frame.where(cond, other=other)
    except Exception as exc:
        raise OracleError(f"dataframe_where_df failed: {exc}") from exc
    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_mask(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    cond_payload = payload.get("frame_right")
    if frame_payload is None:
        raise OracleError("dataframe_mask requires frame payload")
    if cond_payload is None:
        raise OracleError("dataframe_mask requires frame_right condition payload")

    frame = dataframe_from_json(pd, frame_payload)
    cond = dataframe_from_json(pd, cond_payload)
    for column in frame.columns.tolist():
        if column not in cond.columns.tolist():
            raise OracleError(f"mask: condition missing column {column!r}")

    fill_value = payload.get("fill_value")
    try:
        if fill_value is None:
            out = frame.mask(cond)
        else:
            out = frame.mask(cond, other=scalar_from_json(fill_value))
    except Exception as exc:
        raise OracleError(f"dataframe_mask failed: {exc}") from exc
    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_mask_df(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    cond_payload = payload.get("frame_right")
    other_payload = payload.get("frame_other")
    if frame_payload is None:
        raise OracleError("dataframe_mask_df requires frame payload")
    if cond_payload is None:
        raise OracleError("dataframe_mask_df requires frame_right condition payload")
    if other_payload is None:
        raise OracleError("dataframe_mask_df requires frame_other payload")

    frame = dataframe_from_json(pd, frame_payload)
    cond = dataframe_from_json(pd, cond_payload)
    other = dataframe_from_json(pd, other_payload)
    for column in frame.columns.tolist():
        if column not in cond.columns.tolist():
            raise OracleError(f"mask_df_other: condition missing column {column!r}")

    try:
        out = frame.mask(cond, other=other)
    except Exception as exc:
        raise OracleError(f"dataframe_mask_df failed: {exc}") from exc
    return {"expected_frame": dataframe_to_json(out)}


def _resolve_sort_ascending(payload: dict[str, Any], op_name: str) -> bool:
    raw = payload.get("sort_ascending")
    if raw is None:
        return True
    if isinstance(raw, bool):
        return raw
    raise OracleError(f"{op_name} sort_ascending must be a boolean")


def op_dataframe_sort_index(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    if frame_payload is None:
        raise OracleError("dataframe_sort_index requires frame payload")

    frame = dataframe_from_json(pd, frame_payload)
    out = frame.sort_index(ascending=_resolve_sort_ascending(payload, "dataframe_sort_index"))
    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_sort_values(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    sort_column = payload.get("sort_column")
    if frame_payload is None:
        raise OracleError("dataframe_sort_values requires frame payload")
    if not isinstance(sort_column, str) or sort_column.strip() == "":
        raise OracleError("dataframe_sort_values requires sort_column string payload")

    frame = dataframe_from_json(pd, frame_payload)
    ascending = _resolve_sort_ascending(payload, "dataframe_sort_values")
    try:
        out = frame.sort_values(
            by=sort_column.strip(),
            ascending=ascending,
            na_position="last",
            kind="mergesort",
        )
    except KeyError as exc:
        raise OracleError(
            f"dataframe_sort_values column '{sort_column}' not found"
        ) from exc

    return {"expected_frame": dataframe_to_json(out)}


def _resolve_topn_payload(
    payload: dict[str, Any], op_name: str
) -> tuple[Any, int, str, str]:
    frame_payload = payload.get("frame")
    n = payload.get("nlargest_n")
    sort_column = payload.get("sort_column")
    keep = payload.get("keep", "first")

    if frame_payload is None:
        raise OracleError(f"{op_name} requires frame payload")
    if not isinstance(n, int) or isinstance(n, bool) or n < 0:
        raise OracleError(f"{op_name} requires non-negative integer nlargest_n")
    if not isinstance(sort_column, str) or sort_column.strip() == "":
        raise OracleError(f"{op_name} requires sort_column string payload")
    if keep is None:
        keep = "first"
    if keep not in {"first", "last", "all"}:
        raise OracleError(f"{op_name} keep must be one of first|last|all")

    return frame_payload, n, sort_column.strip(), keep


def op_dataframe_nlargest(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload, n, sort_column, keep = _resolve_topn_payload(
        payload, "dataframe_nlargest"
    )
    frame = dataframe_from_json(pd, frame_payload)
    try:
        out = frame.nlargest(n=n, columns=sort_column, keep=keep)
    except KeyError as exc:
        raise OracleError(
            f"dataframe_nlargest column '{sort_column}' not found"
        ) from exc

    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_nsmallest(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload, n, sort_column, keep = _resolve_topn_payload(
        payload, "dataframe_nsmallest"
    )
    frame = dataframe_from_json(pd, frame_payload)
    try:
        out = frame.nsmallest(n=n, columns=sort_column, keep=keep)
    except KeyError as exc:
        raise OracleError(
            f"dataframe_nsmallest column '{sort_column}' not found"
        ) from exc

    return {"expected_frame": dataframe_to_json(out)}


def require_join_type(payload: dict[str, Any], op_name: str, *, allow_cross: bool = False) -> str:
    join_type = payload.get("join_type")
    allowed = {"inner", "left", "right", "outer"}
    if allow_cross:
        allowed.add("cross")
    if join_type not in allowed:
        if allow_cross:
            raise OracleError(
                f"{op_name} requires join_type=inner|left|right|outer|cross, got {join_type!r}"
            )
        raise OracleError(
            f"{op_name} requires join_type=inner|left|right|outer, got {join_type!r}"
        )
    return str(join_type)


def _normalize_key_list(payload_key: Any, op_name: str, field_name: str) -> list[str]:
    if not isinstance(payload_key, list) or len(payload_key) == 0:
        raise OracleError(f"{op_name} requires non-empty {field_name} list payload")
    keys: list[str] = []
    for idx, key in enumerate(payload_key):
        if not isinstance(key, str) or key.strip() == "":
            raise OracleError(f"{op_name} {field_name}[{idx}] must be a non-empty string")
        keys.append(key.strip())
    return keys


def _resolve_index_flag(payload: dict[str, Any], field_name: str, op_name: str, default: bool) -> bool:
    raw = payload.get(field_name)
    if raw is None:
        return default
    if not isinstance(raw, bool):
        raise OracleError(f"{op_name} {field_name} must be a boolean when provided")
    return raw


def resolve_merge_key_pairs(
    payload: dict[str, Any], op_name: str, *, default_key: str | None = None
) -> tuple[list[str], list[str]]:
    left_on_keys_raw = payload.get("left_on_keys")
    right_on_keys_raw = payload.get("right_on_keys")
    if left_on_keys_raw is not None or right_on_keys_raw is not None:
        if left_on_keys_raw is None or right_on_keys_raw is None:
            raise OracleError(
                f"{op_name} requires both left_on_keys and right_on_keys when either is provided"
            )
        left_keys = _normalize_key_list(left_on_keys_raw, op_name, "left_on_keys")
        right_keys = _normalize_key_list(right_on_keys_raw, op_name, "right_on_keys")
        if len(left_keys) != len(right_keys):
            raise OracleError(
                f"{op_name} left_on_keys and right_on_keys must have equal length"
            )
        return left_keys, right_keys

    merge_on_keys_raw = payload.get("merge_on_keys")
    if merge_on_keys_raw is not None:
        keys = _normalize_key_list(merge_on_keys_raw, op_name, "merge_on_keys")
        return keys, keys

    merge_on_raw = payload.get("merge_on")
    if isinstance(merge_on_raw, str) and merge_on_raw.strip():
        key = merge_on_raw.strip()
        return [key], [key]

    if default_key is not None:
        return [default_key], [default_key]

    raise OracleError(
        f"{op_name} requires merge_on string, merge_on_keys list, or left_on_keys/right_on_keys lists"
    )


def resolve_merge_indicator(payload: dict[str, Any], op_name: str) -> bool | str | None:
    indicator_raw = payload.get("merge_indicator")
    if indicator_raw is not None and not isinstance(indicator_raw, bool):
        raise OracleError(f"{op_name} merge_indicator must be a boolean when provided")

    indicator_name_raw = payload.get("merge_indicator_name")
    if indicator_name_raw is not None:
        if not isinstance(indicator_name_raw, str):
            raise OracleError(f"{op_name} merge_indicator_name must be a string when provided")
        if not indicator_name_raw.strip():
            raise OracleError(f"{op_name} merge_indicator_name must be a non-empty string")
        if indicator_raw is not None and not indicator_raw:
            raise OracleError(
                f"{op_name} merge_indicator_name requires merge_indicator=true when explicitly provided"
            )
        return indicator_name_raw

    if indicator_raw:
        return True
    return None


def resolve_merge_validate(payload: dict[str, Any], op_name: str) -> str | None:
    validate_raw = payload.get("merge_validate")
    if validate_raw is None:
        return None
    if not isinstance(validate_raw, str):
        raise OracleError(f"{op_name} merge_validate must be a string when provided")
    normalized = validate_raw.strip().lower()
    if normalized in {"1:1", "one_to_one"}:
        return "one_to_one"
    if normalized in {"1:m", "one_to_many"}:
        return "one_to_many"
    if normalized in {"m:1", "many_to_one"}:
        return "many_to_one"
    if normalized in {"m:m", "many_to_many"}:
        return "many_to_many"
    raise OracleError(
        f"{op_name} merge_validate must be one_to_one, one_to_many, many_to_one, or many_to_many"
    )


def resolve_merge_suffixes(payload: dict[str, Any], op_name: str) -> tuple[str | None, str | None]:
    suffixes_raw = payload.get("merge_suffixes")
    if suffixes_raw is None:
        return ("_left", "_right")
    if not isinstance(suffixes_raw, (list, tuple)) or len(suffixes_raw) != 2:
        raise OracleError(f"{op_name} merge_suffixes must be a two-item array when provided")

    normalized: list[str | None] = []
    for index, suffix in enumerate(suffixes_raw):
        if suffix is None:
            normalized.append(None)
        elif isinstance(suffix, str):
            normalized.append(suffix)
        else:
            raise OracleError(
                f"{op_name} merge_suffixes[{index}] must be a string or null when provided"
            )
    return (normalized[0], normalized[1])


def resolve_merge_sort(payload: dict[str, Any], op_name: str) -> bool:
    sort_raw = payload.get("merge_sort")
    if sort_raw is None:
        return False
    if not isinstance(sort_raw, bool):
        raise OracleError(f"{op_name} merge_sort must be a boolean when provided")
    return sort_raw


def validate_cross_merge_payload(
    payload: dict[str, Any], op_name: str, *, use_index_keys: bool
) -> None:
    if use_index_keys:
        raise OracleError(f"{op_name} does not support join_type='cross'")

    if (
        payload.get("merge_on") is not None
        or payload.get("merge_on_keys") is not None
        or payload.get("left_on_keys") is not None
        or payload.get("right_on_keys") is not None
    ):
        raise OracleError(
            f"{op_name} join_type='cross' does not allow merge_on/merge_on_keys/left_on_keys/right_on_keys"
        )

    left_index_raw = payload.get("left_index")
    right_index_raw = payload.get("right_index")
    if left_index_raw is not None and not isinstance(left_index_raw, bool):
        raise OracleError(f"{op_name} left_index must be a boolean when provided")
    if right_index_raw is not None and not isinstance(right_index_raw, bool):
        raise OracleError(f"{op_name} right_index must be a boolean when provided")
    if bool(left_index_raw) or bool(right_index_raw):
        raise OracleError(f"{op_name} join_type='cross' does not allow left_index/right_index")


def dataframe_with_index_keys(frame, key_names: list[str]):
    out = frame.copy()
    for key_name in key_names:
        out[key_name] = frame.index.tolist()
    return out


def op_dataframe_merge(
    pd, payload: dict[str, Any], *, use_index_keys: bool = False
) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    frame_right_payload = payload.get("frame_right")
    if frame_payload is None or frame_right_payload is None:
        raise OracleError("dataframe_merge requires frame and frame_right payloads")

    op_name = "dataframe_merge_index" if use_index_keys else "dataframe_merge"
    how = require_join_type(payload, op_name, allow_cross=True)

    if how == "cross":
        validate_cross_merge_payload(payload, op_name, use_index_keys=use_index_keys)
        left_use_index = False
        right_use_index = False
    else:
        left_use_index = _resolve_index_flag(payload, "left_index", op_name, use_index_keys)
        right_use_index = _resolve_index_flag(payload, "right_index", op_name, use_index_keys)

    left = dataframe_from_json(pd, frame_payload)
    right = dataframe_from_json(pd, frame_right_payload)

    if how == "cross":
        left_merge_keys, right_merge_keys = [], []
    else:
        left_merge_keys, right_merge_keys = resolve_merge_key_pairs(
            payload,
            op_name,
            default_key="__index_key" if left_use_index and right_use_index else None,
        )
    indicator = resolve_merge_indicator(payload, op_name)
    validate_mode = resolve_merge_validate(payload, op_name)
    suffixes = resolve_merge_suffixes(payload, op_name)
    merge_sort = resolve_merge_sort(payload, op_name)

    if left_use_index:
        left = dataframe_with_index_keys(left, left_merge_keys)
    if right_use_index:
        right = dataframe_with_index_keys(right, right_merge_keys)

    merge_kwargs = {
        "how": how,
        "sort": merge_sort,
        "copy": False,
        "suffixes": suffixes,
    }
    if indicator is not None:
        merge_kwargs["indicator"] = indicator
    if validate_mode is not None:
        merge_kwargs["validate"] = validate_mode

    if how == "cross":
        out = left.merge(right, **merge_kwargs)
    elif left_merge_keys == right_merge_keys:
        out = left.merge(right, on=left_merge_keys, **merge_kwargs)
    else:
        out = left.merge(
            right, left_on=left_merge_keys, right_on=right_merge_keys, **merge_kwargs
        )
    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_merge_index(pd, payload: dict[str, Any]) -> dict[str, Any]:
    return op_dataframe_merge(pd, payload, use_index_keys=True)


def op_dataframe_merge_asof(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    frame_right_payload = payload.get("frame_right")
    if frame_payload is None or frame_right_payload is None:
        raise OracleError("dataframe_merge_asof requires frame and frame_right payloads")

    left = dataframe_from_json(pd, frame_payload)
    right = dataframe_from_json(pd, frame_right_payload)

    on = payload.get("merge_on")
    if on is None:
        on = payload.get("on")
    if on is None:
        raise OracleError("dataframe_merge_asof requires 'merge_on' column string payload")

    direction = payload.get("direction", "backward")

    # New options for pandas parity
    allow_exact_matches = payload.get("allow_exact_matches", True)
    tolerance = payload.get("tolerance")  # None means no tolerance limit
    by = payload.get("by")  # str or list of str for equi-join columns

    try:
        out = pd.merge_asof(
            left,
            right,
            on=on,
            direction=direction,
            allow_exact_matches=allow_exact_matches,
            tolerance=tolerance,
            by=by,
        )
    except Exception as exc:
        raise OracleError(f"dataframe_merge_asof failed: {exc}") from exc
    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_merge_ordered(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    frame_right_payload = payload.get("frame_right")
    if frame_payload is None or frame_right_payload is None:
        raise OracleError("dataframe_merge_ordered requires frame and frame_right payloads")

    left = dataframe_from_json(pd, frame_payload)
    right = dataframe_from_json(pd, frame_right_payload)

    on_keys = payload.get("merge_on_keys")
    if on_keys is None:
        merge_on = payload.get("merge_on")
        if merge_on is None:
            raise OracleError(
                "dataframe_merge_ordered requires 'merge_on' or 'merge_on_keys' payload"
            )
        on_keys = [merge_on]

    fill_method = payload.get("merge_fill_method")

    try:
        out = pd.merge_ordered(left, right, on=on_keys, fill_method=fill_method)
    except Exception as exc:
        raise OracleError(f"dataframe_merge_ordered failed: {exc}") from exc
    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_combine_first(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    frame_right_payload = payload.get("frame_right")
    if frame_payload is None or frame_right_payload is None:
        raise OracleError("dataframe_combine_first requires frame and frame_right payloads")

    left = dataframe_from_json(pd, frame_payload)
    right = dataframe_from_json(pd, frame_right_payload)
    try:
        out = left.combine_first(right)
    except Exception as exc:
        raise OracleError(f"dataframe_combine_first failed: {exc}") from exc
    return {"expected_frame": dataframe_to_json(out)}


def op_dataframe_concat(pd, payload: dict[str, Any]) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    frame_right_payload = payload.get("frame_right")
    if frame_payload is None or frame_right_payload is None:
        raise OracleError("dataframe_concat requires frame and frame_right payloads")

    left = dataframe_from_json(pd, frame_payload)
    right = dataframe_from_json(pd, frame_right_payload)
    axis_raw = payload.get("concat_axis", 0)
    try:
        axis = int(axis_raw)
    except (TypeError, ValueError) as exc:
        raise OracleError(
            f"dataframe_concat concat_axis must be an integer: {exc}"
        ) from exc
    if axis not in (0, 1):
        raise OracleError(f"dataframe_concat concat_axis must be 0 or 1, got {axis}")

    join_raw = payload.get("concat_join", "outer")
    if not isinstance(join_raw, str):
        raise OracleError("dataframe_concat concat_join must be a string")
    join = join_raw.lower()
    if join not in {"outer", "inner"}:
        raise OracleError(
            f"dataframe_concat concat_join must be 'outer' or 'inner', got {join_raw}"
        )

    if axis == 0:
        out = pd.concat([left, right], axis=0, join=join, sort=False)
    else:
        overlapping = sorted(set(left.columns.tolist()) & set(right.columns.tolist()))
        if overlapping:
            joined = ", ".join(map(str, overlapping))
            raise OracleError(
                f"dataframe_concat axis=1 duplicate columns unsupported: {joined}"
            )
        out = pd.concat([left, right], axis=1, join=join, sort=False)
    expected_frame = dataframe_to_json(out)
    expected_frame["column_order"] = [str(name) for name in out.columns.tolist()]
    return {"expected_frame": expected_frame}


def dispatch(pd, payload: dict[str, Any]) -> dict[str, Any]:
    op = payload.get("operation")
    if op == "series_add":
        return op_series_add(pd, payload)
    if op == "series_sub":
        return op_series_sub(pd, payload)
    if op == "series_mul":
        return op_series_mul(pd, payload)
    if op == "series_div":
        return op_series_div(pd, payload)
    if op == "series_mode":
        return op_series_mode(pd, payload)
    if op == "series_nunique":
        return op_series_nunique(pd, payload)
    if op == "series_join":
        return op_series_join(pd, payload)
    if op == "series_constructor":
        return op_series_constructor(pd, payload)
    if op == "series_combine_first":
        return op_series_combine_first(pd, payload)
    if op == "series_asof":
        return op_series_asof(pd, payload)
    if op == "series_clip":
        return op_series_clip(pd, payload)
    if op in {"series_to_datetime", "to_datetime"}:
        return op_series_to_datetime(pd, payload)
    if op == "series_dt_to_pydatetime":
        return op_series_dt_to_pydatetime(pd, payload)
    if op in {"dataframe_from_series", "data_frame_from_series"}:
        return op_dataframe_from_series(pd, payload)
    if op in {"dataframe_from_dict", "data_frame_from_dict"}:
        return op_dataframe_from_dict(pd, payload)
    if op in {"dataframe_from_records", "data_frame_from_records"}:
        return op_dataframe_from_records(pd, payload)
    if op in {"dataframe_constructor_kwargs", "data_frame_constructor_kwargs"}:
        return op_dataframe_constructor_kwargs(pd, payload)
    if op in {"dataframe_constructor_scalar", "data_frame_constructor_scalar"}:
        return op_dataframe_constructor_scalar(pd, payload)
    if op in {
        "dataframe_constructor_dict_of_series",
        "data_frame_constructor_dict_of_series",
    }:
        return op_dataframe_constructor_dict_of_series(pd, payload)
    if op in {
        "dataframe_constructor_list_like",
        "data_frame_constructor_list_like",
        "dataframe_constructor_2d",
        "data_frame_constructor_2d",
    }:
        return op_dataframe_constructor_list_like(pd, payload)
    if op in {"dataframe_eval", "data_frame_eval"}:
        return op_dataframe_expression(pd, payload)
    if op in {"dataframe_query", "data_frame_query"}:
        return op_dataframe_query(pd, payload)
    if op in {"dataframe_pivot", "data_frame_pivot"}:
        return op_dataframe_pivot(pd, payload)
    if op in {"dataframe_pivot_table", "data_frame_pivot_table"}:
        return op_dataframe_pivot_table(pd, payload)
    if op in {"dataframe_stack", "data_frame_stack"}:
        return op_dataframe_stack(pd, payload)
    if op in {"dataframe_transpose", "data_frame_transpose"}:
        return op_dataframe_transpose(pd, payload)
    if op in {"series_unstack", "series_unstack_default"}:
        return op_series_unstack(pd, payload)
    if op in {"dataframe_crosstab", "data_frame_crosstab"}:
        return op_dataframe_crosstab(pd, payload)
    if op in {"dataframe_crosstab_normalize", "data_frame_crosstab_normalize"}:
        return op_dataframe_crosstab_normalize(pd, payload)
    if op in {"dataframe_get_dummies", "data_frame_get_dummies"}:
        return op_dataframe_get_dummies(pd, payload)
    if op in {"series_str_get_dummies", "series_str_get_dummies_default"}:
        return op_series_str_get_dummies(pd, payload)
    if op in {"groupby_sum", "group_by_sum"}:
        return op_groupby_sum(pd, payload)
    if op in {"groupby_mean", "group_by_mean"}:
        return op_groupby_mean(pd, payload)
    if op in {"groupby_count", "group_by_count"}:
        return op_groupby_count(pd, payload)
    if op in {"groupby_min", "group_by_min"}:
        return op_groupby_min(pd, payload)
    if op in {"groupby_max", "group_by_max"}:
        return op_groupby_max(pd, payload)
    if op in {"groupby_first", "group_by_first"}:
        return op_groupby_first(pd, payload)
    if op in {"groupby_last", "group_by_last"}:
        return op_groupby_last(pd, payload)
    if op in {"groupby_std", "group_by_std"}:
        return op_groupby_std(pd, payload)
    if op in {"groupby_var", "group_by_var"}:
        return op_groupby_var(pd, payload)
    if op in {"groupby_median", "group_by_median"}:
        return op_groupby_median(pd, payload)
    if op in {"nan_sum", "nansum"}:
        return op_nan_sum(pd, payload)
    if op in {"nan_mean", "nanmean"}:
        return op_nan_mean(pd, payload)
    if op in {"nan_min", "nanmin"}:
        return op_nan_min(pd, payload)
    if op in {"nan_max", "nanmax"}:
        return op_nan_max(pd, payload)
    if op in {"nan_std", "nanstd"}:
        return op_nan_std(pd, payload)
    if op in {"nan_var", "nanvar"}:
        return op_nan_var(pd, payload)
    if op in {"nan_count", "nancount"}:
        return op_nan_count(pd, payload)
    if op == "csv_round_trip":
        return op_csv_round_trip(pd, payload)
    if op == "index_align_union":
        return op_index_align_union(pd, payload)
    if op == "index_has_duplicates":
        return op_index_has_duplicates(pd, payload)
    if op == "index_is_monotonic_increasing":
        return op_index_is_monotonic_increasing(pd, payload)
    if op == "index_is_monotonic_decreasing":
        return op_index_is_monotonic_decreasing(pd, payload)
    if op == "index_first_positions":
        return op_index_first_positions(pd, payload)
    if op == "series_loc":
        return op_series_loc(pd, payload)
    if op == "series_iloc":
        return op_series_iloc(pd, payload)
    if op == "series_take":
        return op_series_take(pd, payload)
    if op == "series_xs":
        return op_series_xs(pd, payload)
    if op == "series_repeat":
        return op_series_repeat(pd, payload)
    if op == "series_at_time":
        return op_series_at_time(pd, payload)
    if op == "series_between_time":
        return op_series_between_time(pd, payload)
    if op == "series_filter":
        return op_series_filter(pd, payload)
    if op == "series_head":
        return op_series_head(pd, payload)
    if op == "series_tail":
        return op_series_tail(pd, payload)
    if op == "series_isna":
        return op_series_isna(pd, payload)
    if op == "series_notna":
        return op_series_notna(pd, payload)
    if op == "series_isnull":
        return op_series_isnull(pd, payload)
    if op == "series_notnull":
        return op_series_notnull(pd, payload)
    if op == "series_fillna":
        return op_series_fillna(pd, payload)
    if op == "series_dropna":
        return op_series_dropna(pd, payload)
    if op == "series_count":
        return op_series_count(pd, payload)
    if op in {"series_rank", "series_rank_default"}:
        return op_series_rank(pd, payload)
    if op == "series_any":
        return op_series_any(pd, payload)
    if op == "series_all":
        return op_series_all(pd, payload)
    if op == "series_bool":
        return op_series_bool(pd, payload)
    if op == "series_to_numeric":
        return op_series_to_numeric(pd, payload)
    if op == "series_cut":
        return op_series_cut(pd, payload)
    if op == "series_qcut":
        return op_series_qcut(pd, payload)
    if op == "series_value_counts":
        return op_series_value_counts(pd, payload)
    if op == "series_sort_index":
        return op_series_sort_index(pd, payload)
    if op == "series_sort_values":
        return op_series_sort_values(pd, payload)
    if op == "series_diff":
        return op_series_diff(pd, payload)
    if op == "series_shift":
        return op_series_shift(pd, payload)
    if op == "series_pct_change":
        return op_series_pct_change(pd, payload)
    if op == "series_partition_df":
        return op_series_partition_df(pd, payload)
    if op == "series_rpartition_df":
        return op_series_rpartition_df(pd, payload)
    if op == "series_split_df":
        return op_series_split_df(pd, payload)
    if op == "series_extract_df":
        return op_series_extract_df(pd, payload)
    if op == "series_extractall":
        return op_series_extractall(pd, payload)
    if op in {"dataframe_identity", "data_frame_identity"}:
        return op_dataframe_identity(pd, payload)
    if op == "dataframe_loc":
        return op_dataframe_loc(pd, payload)
    if op in {"dataframe_xs", "data_frame_xs"}:
        return op_dataframe_xs(pd, payload)
    if op == "dataframe_iloc":
        return op_dataframe_iloc(pd, payload)
    if op in {"dataframe_take", "data_frame_take"}:
        return op_dataframe_take(pd, payload)
    if op in {"dataframe_groupby_idxmin", "data_frame_groupby_idxmin"}:
        return op_dataframe_groupby_idxmin(pd, payload)
    if op in {"dataframe_groupby_idxmax", "data_frame_groupby_idxmax"}:
        return op_dataframe_groupby_idxmax(pd, payload)
    if op in {"dataframe_groupby_any", "data_frame_groupby_any"}:
        return op_dataframe_groupby_any(pd, payload)
    if op in {"dataframe_groupby_all", "data_frame_groupby_all"}:
        return op_dataframe_groupby_all(pd, payload)
    if op in {"dataframe_groupby_sum", "data_frame_groupby_sum"}:
        return op_dataframe_groupby_sum(pd, payload)
    if op in {"dataframe_groupby_agg_multi", "data_frame_groupby_agg_multi"}:
        return op_dataframe_groupby_agg_multi(pd, payload)
    if op in {"dataframe_groupby_get_group", "data_frame_groupby_get_group"}:
        return op_dataframe_groupby_get_group(pd, payload)
    if op in {"dataframe_groupby_ffill", "data_frame_groupby_ffill"}:
        return op_dataframe_groupby_ffill(pd, payload)
    if op in {"dataframe_groupby_bfill", "data_frame_groupby_bfill"}:
        return op_dataframe_groupby_bfill(pd, payload)
    if op in {"dataframe_groupby_sem", "data_frame_groupby_sem"}:
        return op_dataframe_groupby_sem(pd, payload)
    if op in {"dataframe_groupby_skew", "data_frame_groupby_skew"}:
        return op_dataframe_groupby_skew(pd, payload)
    if op in {"dataframe_groupby_kurtosis", "data_frame_groupby_kurtosis"}:
        return op_dataframe_groupby_kurtosis(pd, payload)
    if op in {"dataframe_groupby_ohlc", "data_frame_groupby_ohlc"}:
        return op_dataframe_groupby_ohlc(pd, payload)
    if op in {"dataframe_groupby_resample_min", "data_frame_groupby_resample_min"}:
        return op_dataframe_groupby_resample_min(pd, payload)
    if op in {"dataframe_groupby_resample_max", "data_frame_groupby_resample_max"}:
        return op_dataframe_groupby_resample_max(pd, payload)
    if op in {"dataframe_groupby_resample_count", "data_frame_groupby_resample_count"}:
        return op_dataframe_groupby_resample_count(pd, payload)
    if op in {"dataframe_groupby_resample_first", "data_frame_groupby_resample_first"}:
        return op_dataframe_groupby_resample_first(pd, payload)
    if op in {"dataframe_groupby_resample_last", "data_frame_groupby_resample_last"}:
        return op_dataframe_groupby_resample_last(pd, payload)
    if op in {"dataframe_groupby_rolling_mean", "data_frame_groupby_rolling_mean"}:
        return op_dataframe_groupby_rolling_mean(pd, payload)
    if op in {"dataframe_groupby_rolling_sum", "data_frame_groupby_rolling_sum"}:
        return op_dataframe_groupby_rolling_sum(pd, payload)
    if op in {"dataframe_groupby_rolling_min", "data_frame_groupby_rolling_min"}:
        return op_dataframe_groupby_rolling_min(pd, payload)
    if op in {"dataframe_groupby_rolling_max", "data_frame_groupby_rolling_max"}:
        return op_dataframe_groupby_rolling_max(pd, payload)
    if op in {"dataframe_groupby_rolling_count", "data_frame_groupby_rolling_count"}:
        return op_dataframe_groupby_rolling_count(pd, payload)
    if op in {"dataframe_groupby_rolling_std", "data_frame_groupby_rolling_std"}:
        return op_dataframe_groupby_rolling_std(pd, payload)
    if op in {"dataframe_groupby_rolling_var", "data_frame_groupby_rolling_var"}:
        return op_dataframe_groupby_rolling_var(pd, payload)
    if op in {"dataframe_groupby_cumcount", "data_frame_groupby_cumcount"}:
        return op_dataframe_groupby_cumcount(pd, payload)
    if op in {"dataframe_groupby_ngroup", "data_frame_groupby_ngroup"}:
        return op_dataframe_groupby_ngroup(pd, payload)
    if op in {"dataframe_asof", "data_frame_asof"}:
        return op_dataframe_asof(pd, payload)
    if op in {"dataframe_at_time", "data_frame_at_time"}:
        return op_dataframe_at_time(pd, payload)
    if op in {"dataframe_between_time", "data_frame_between_time"}:
        return op_dataframe_between_time(pd, payload)
    if op in {"dataframe_head", "data_frame_head"}:
        return op_dataframe_head(pd, payload)
    if op in {"dataframe_tail", "data_frame_tail"}:
        return op_dataframe_tail(pd, payload)
    if op in {"dataframe_melt", "data_frame_melt"}:
        return op_dataframe_melt(pd, payload)
    if op in {"dataframe_isna", "data_frame_isna"}:
        return op_dataframe_isna(pd, payload)
    if op in {"dataframe_notna", "data_frame_notna"}:
        return op_dataframe_notna(pd, payload)
    if op in {"dataframe_isnull", "data_frame_isnull"}:
        return op_dataframe_isnull(pd, payload)
    if op in {"dataframe_notnull", "data_frame_notnull"}:
        return op_dataframe_notnull(pd, payload)
    if op in {"dataframe_count", "data_frame_count"}:
        return op_dataframe_count(pd, payload)
    if op in {"dataframe_mode", "data_frame_mode"}:
        return op_dataframe_mode(pd, payload)
    if op in {"dataframe_rank", "data_frame_rank"}:
        return op_dataframe_rank(pd, payload)
    if op in {"dataframe_astype", "data_frame_astype"}:
        return op_dataframe_astype(pd, payload)
    if op in {"dataframe_clip", "data_frame_clip"}:
        return op_dataframe_clip(pd, payload)
    if op in {"dataframe_abs", "data_frame_abs"}:
        return op_dataframe_abs(pd, payload)
    if op in {"dataframe_describe", "data_frame_describe"}:
        return op_dataframe_describe(pd, payload)
    if op in {"dataframe_corr", "data_frame_corr"}:
        return op_dataframe_corr(pd, payload)
    if op in {"dataframe_cov", "data_frame_cov"}:
        return op_dataframe_cov(pd, payload)
    if op in {"dataframe_idxmin", "data_frame_idxmin"}:
        return op_dataframe_idxmin(pd, payload)
    if op in {"dataframe_idxmax", "data_frame_idxmax"}:
        return op_dataframe_idxmax(pd, payload)
    if op in {"dataframe_sem", "data_frame_sem"}:
        return op_dataframe_sem(pd, payload)
    if op in {"dataframe_apply_sem_axis0", "data_frame_apply_sem_axis0"}:
        return op_dataframe_apply_sem_axis0(pd, payload)
    if op in {"dataframe_skew", "data_frame_skew"}:
        return op_dataframe_skew(pd, payload)
    if op in {"dataframe_kurtosis", "data_frame_kurtosis"}:
        return op_dataframe_kurtosis(pd, payload)
    if op in {"dataframe_prod", "data_frame_prod"}:
        return op_dataframe_prod(pd, payload)
    if op in {"dataframe_apply_prod_axis1", "data_frame_apply_prod_axis1"}:
        return op_dataframe_apply_prod_axis1(pd, payload)
    if op in {"dataframe_apply_product_axis1", "data_frame_apply_product_axis1"}:
        return op_dataframe_apply_product_axis1(pd, payload)
    if op in {"dataframe_sum", "data_frame_sum"}:
        return op_dataframe_sum(pd, payload)
    if op in {"dataframe_mean", "data_frame_mean"}:
        return op_dataframe_mean(pd, payload)
    if op in {"dataframe_std", "data_frame_std"}:
        return op_dataframe_std(pd, payload)
    if op in {"dataframe_var", "data_frame_var"}:
        return op_dataframe_var(pd, payload)
    if op in {"dataframe_min", "data_frame_min"}:
        return op_dataframe_min(pd, payload)
    if op in {"dataframe_max", "data_frame_max"}:
        return op_dataframe_max(pd, payload)
    if op in {"dataframe_median", "data_frame_median"}:
        return op_dataframe_median(pd, payload)
    if op in {"dataframe_any", "data_frame_any"}:
        return op_dataframe_any(pd, payload)
    if op in {"dataframe_all", "data_frame_all"}:
        return op_dataframe_all(pd, payload)
    if op in {"dataframe_nunique", "data_frame_nunique"}:
        return op_dataframe_nunique(pd, payload)
    if op in {"dataframe_apply_nunique_axis0", "data_frame_apply_nunique_axis0"}:
        return op_dataframe_apply_nunique_axis0(pd, payload)
    if op in {"dataframe_quantile", "data_frame_quantile"}:
        return op_dataframe_quantile(pd, payload)
    if op in {"dataframe_value_counts", "data_frame_value_counts"}:
        return op_dataframe_value_counts(pd, payload)
    if op in {"dataframe_memory_usage", "data_frame_memory_usage"}:
        return op_dataframe_memory_usage(pd, payload)
    if op in {"dataframe_round", "data_frame_round"}:
        return op_dataframe_round(pd, payload)
    if op in {"dataframe_shift", "data_frame_shift"}:
        return op_dataframe_shift(pd, payload)
    if op in {"dataframe_fillna", "data_frame_fillna"}:
        return op_dataframe_fillna(pd, payload)
    if op in {"dataframe_dropna", "data_frame_dropna"}:
        return op_dataframe_dropna(pd, payload)
    if op in {"dataframe_dropna_columns", "data_frame_dropna_columns"}:
        return op_dataframe_dropna_columns(pd, payload)
    if op in {"dataframe_bool", "data_frame_bool"}:
        return op_dataframe_bool(pd, payload)
    if op in {"dataframe_duplicated", "data_frame_duplicated"}:
        return op_dataframe_duplicated(pd, payload)
    if op in {"dataframe_drop_duplicates", "data_frame_drop_duplicates"}:
        return op_dataframe_drop_duplicates(pd, payload)
    if op in {"dataframe_explode", "data_frame_explode"}:
        return op_dataframe_explode(pd, payload)
    if op in {"dataframe_set_index", "data_frame_set_index"}:
        return op_dataframe_set_index(pd, payload)
    if op in {"dataframe_reset_index", "data_frame_reset_index"}:
        return op_dataframe_reset_index(pd, payload)
    if op in {"dataframe_insert", "data_frame_insert"}:
        return op_dataframe_insert(pd, payload)
    if op in {"dataframe_assign", "data_frame_assign"}:
        return op_dataframe_assign(pd, payload)
    if op in {"dataframe_rename_columns", "data_frame_rename_columns"}:
        return op_dataframe_rename_columns(pd, payload)
    if op in {"dataframe_reindex", "data_frame_reindex"}:
        return op_dataframe_reindex(pd, payload)
    if op in {"dataframe_reindex_columns", "data_frame_reindex_columns"}:
        return op_dataframe_reindex_columns(pd, payload)
    if op in {"dataframe_drop_columns", "data_frame_drop_columns"}:
        return op_dataframe_drop_columns(pd, payload)
    if op in {"dataframe_replace", "data_frame_replace"}:
        return op_dataframe_replace(pd, payload)
    if op in {"dataframe_where", "data_frame_where"}:
        return op_dataframe_where(pd, payload)
    if op in {"dataframe_where_df", "data_frame_where_df"}:
        return op_dataframe_where_df(pd, payload)
    if op in {"dataframe_mask", "data_frame_mask"}:
        return op_dataframe_mask(pd, payload)
    if op in {"dataframe_mask_df", "data_frame_mask_df"}:
        return op_dataframe_mask_df(pd, payload)
    if op in {"dataframe_sort_index", "data_frame_sort_index"}:
        return op_dataframe_sort_index(pd, payload)
    if op in {"dataframe_sort_values", "data_frame_sort_values"}:
        return op_dataframe_sort_values(pd, payload)
    if op in {"dataframe_nlargest", "data_frame_nlargest"}:
        return op_dataframe_nlargest(pd, payload)
    if op in {"dataframe_nsmallest", "data_frame_nsmallest"}:
        return op_dataframe_nsmallest(pd, payload)
    if op in {"dataframe_merge", "data_frame_merge"}:
        return op_dataframe_merge(pd, payload)
    if op in {"dataframe_merge_index", "data_frame_merge_index"}:
        return op_dataframe_merge_index(pd, payload)
    if op in {"dataframe_merge_asof", "data_frame_merge_asof"}:
        return op_dataframe_merge_asof(pd, payload)
    if op in {"dataframe_merge_ordered", "data_frame_merge_ordered"}:
        return op_dataframe_merge_ordered(pd, payload)
    if op in {"dataframe_combine_first", "data_frame_combine_first"}:
        return op_dataframe_combine_first(pd, payload)
    if op in {"dataframe_concat", "data_frame_concat"}:
        return op_dataframe_concat(pd, payload)
    raise OracleError(f"unsupported operation: {op!r}")


def main() -> int:
    args = parse_args()
    pd = None
    try:
        pd = setup_pandas(args)
        try:
            payload = json.load(sys.stdin)
        except json.JSONDecodeError as exc:
            raise OracleError(f"invalid oracle request JSON: {exc}") from exc
        response = dispatch(pd, payload)
        for key, value in base_oracle_response().items():
            response.setdefault(key, value)
        response["fixture_provenance"] = build_fixture_provenance(pd)
        response["error"] = None
        json.dump(response, sys.stdout)
        return 0
    except OracleError as exc:
        json.dump(error_response(str(exc), pd), sys.stdout)
        return 1
    except Exception as exc:  # pragma: no cover - defensive
        json.dump(error_response(f"unexpected oracle failure: {exc}", pd), sys.stdout)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
