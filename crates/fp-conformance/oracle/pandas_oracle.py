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
import importlib
import json
import math
import os
import sys
from dataclasses import dataclass
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
    if kind == "int64":
        return int(raw)
    if kind == "utf8":
        return str(raw)
    raise OracleError(f"unsupported index label kind: {kind!r}")


def scalar_from_json(value: dict[str, Any]) -> Any:
    kind = value.get("kind")
    raw = value.get("value")
    if kind == "null":
        marker = str(raw)
        if marker == "nan":
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
            return {"kind": "null", "value": "nan"}
        return {"kind": "float64", "value": value}
    return {"kind": "utf8", "value": str(value)}


def label_to_json(value: Any) -> dict[str, Any]:
    if isinstance(value, bool):
        return {"kind": "utf8", "value": str(value)}
    if isinstance(value, int):
        return {"kind": "int64", "value": value}
    return {"kind": "utf8", "value": str(value)}


def op_series_add(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    right = payload.get("right")
    if left is None or right is None:
        raise OracleError("series_add requires left and right payloads")

    left_index = [label_from_json(item) for item in left["index"]]
    right_index = [label_from_json(item) for item in right["index"]]
    left_values = [scalar_from_json(item) for item in left["values"]]
    right_values = [scalar_from_json(item) for item in right["values"]]

    lhs = pd.Series(left_values, index=left_index, dtype="float64")
    rhs = pd.Series(right_values, index=right_index, dtype="float64")
    out = lhs + rhs

    return {
        "expected_series": {
            "index": [label_to_json(v) for v in out.index.tolist()],
            "values": [scalar_to_json(v) for v in out.tolist()],
        }
    }


def op_series_join(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    right = payload.get("right")
    join_type = payload.get("join_type")
    if left is None or right is None:
        raise OracleError("series_join requires left and right payloads")
    if join_type not in {"inner", "left"}:
        raise OracleError(f"series_join requires join_type=inner|left, got {join_type!r}")

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
    left = payload.get("left")
    right = payload.get("right")
    if left is None or right is None:
        raise OracleError(f"{op_name} requires left(keys) and right(values) payloads")

    key_index = [label_from_json(item) for item in left["index"]]
    value_index = [label_from_json(item) for item in right["index"]]
    keys = [scalar_from_json(item) for item in left["values"]]
    values = [scalar_from_json(item) for item in right["values"]]

    key_series = pd.Series(keys, index=key_index, dtype="object")
    value_series = pd.Series(values, index=value_index, dtype="float64")

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
    else:
        raise OracleError(f"unsupported groupby aggregation: {agg!r}")

    return {
        "expected_series": {
            "index": [label_to_json(v) for v in out.index.tolist()],
            "values": [scalar_to_json(v) for v in out.tolist()],
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


def dataframe_from_json(pd, payload: dict[str, Any]):
    index_raw = payload.get("index")
    columns_raw = payload.get("columns")
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

    return pd.DataFrame(columns, index=index)


def dataframe_to_json(frame) -> dict[str, Any]:
    return {
        "index": [label_to_json(v) for v in frame.index.tolist()],
        "columns": {
            str(name): [scalar_to_json(v) for v in frame[name].tolist()]
            for name in frame.columns.tolist()
        },
    }


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
        out = frame.loc[labels]
    except KeyError as exc:
        raise OracleError(f"dataframe_loc label lookup failed: {exc}") from exc

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


def dispatch(pd, payload: dict[str, Any]) -> dict[str, Any]:
    op = payload.get("operation")
    if op == "series_add":
        return op_series_add(pd, payload)
    if op == "series_join":
        return op_series_join(pd, payload)
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
    if op == "index_align_union":
        return op_index_align_union(pd, payload)
    if op == "index_has_duplicates":
        return op_index_has_duplicates(pd, payload)
    if op == "index_first_positions":
        return op_index_first_positions(pd, payload)
    if op == "series_loc":
        return op_series_loc(pd, payload)
    if op == "series_iloc":
        return op_series_iloc(pd, payload)
    if op == "series_filter":
        return op_series_filter(pd, payload)
    if op == "series_head":
        return op_series_head(pd, payload)
    if op == "dataframe_loc":
        return op_dataframe_loc(pd, payload)
    if op == "dataframe_iloc":
        return op_dataframe_iloc(pd, payload)
    raise OracleError(f"unsupported operation: {op!r}")


def main() -> int:
    args = parse_args()
    try:
        pd = setup_pandas(args)
        payload = json.load(sys.stdin)
        response = dispatch(pd, payload)
        response.setdefault("expected_series", None)
        response.setdefault("expected_join", None)
        response.setdefault("expected_frame", None)
        response.setdefault("expected_alignment", None)
        response.setdefault("expected_bool", None)
        response.setdefault("expected_positions", None)
        response["error"] = None
        json.dump(response, sys.stdout)
        return 0
    except OracleError as exc:
        json.dump(
            {
                "expected_series": None,
                "expected_join": None,
                "expected_frame": None,
                "expected_alignment": None,
                "expected_bool": None,
                "expected_positions": None,
                "error": str(exc),
            },
            sys.stdout,
        )
        return 1
    except Exception as exc:  # pragma: no cover - defensive
        json.dump(
            {
                "expected_series": None,
                "expected_join": None,
                "expected_frame": None,
                "expected_alignment": None,
                "expected_bool": None,
                "expected_positions": None,
                "error": f"unexpected oracle failure: {exc}",
            },
            sys.stdout,
        )
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
