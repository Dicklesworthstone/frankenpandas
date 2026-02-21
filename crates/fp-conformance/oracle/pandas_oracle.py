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
import io
import json
import math
import os
import struct
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
        return {"kind": "utf8", "value": str(value)}
    if isinstance(value, int):
        return {"kind": "int64", "value": value}
    return {"kind": "utf8", "value": str(value)}


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
    payload: dict[str, Any], operation: str
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
    return op_series_binary_numeric(payload, "series_add")


def op_series_sub(pd, payload: dict[str, Any]) -> dict[str, Any]:
    return op_series_binary_numeric(payload, "series_sub")


def op_series_mul(pd, payload: dict[str, Any]) -> dict[str, Any]:
    return op_series_binary_numeric(payload, "series_mul")


def op_series_div(pd, payload: dict[str, Any]) -> dict[str, Any]:
    return op_series_binary_numeric(payload, "series_div")


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


def series_to_expected(series) -> dict[str, Any]:
    return {
        "index": [label_to_json(v) for v in series.index.tolist()],
        "values": [scalar_to_json(v) for v in series.tolist()],
    }


def op_series_constructor(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    series = fixture_series_from_payload(pd, left, "series_constructor")
    return {"expected_series": series_to_expected(series)}


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
    raw_records = payload.get("records")
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

    column_order = parse_constructor_column_order(payload, "dataframe_from_records")
    index = parse_constructor_index(payload, "dataframe_from_records")

    try:
        frame = pd.DataFrame(records, columns=column_order, index=index)
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


def op_series_value_counts(pd, payload: dict[str, Any]) -> dict[str, Any]:
    left = payload.get("left")
    if left is None:
        raise OracleError("series_value_counts requires left payload")

    index = [label_from_json(item) for item in left["index"]]
    values = [scalar_from_json(item) for item in left["values"]]
    series = pd.Series(values, index=index, name=left.get("name", "series"))
    out = series.value_counts(dropna=True)

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


def dataframe_from_json(pd, payload: dict[str, Any]):
    index_raw = payload.get("index")
    columns_raw = payload.get("columns")
    column_order_raw = payload.get("column_order")
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
    return frame.reindex(columns=column_order)


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
    return {"expected_series": series_to_json(out)}


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

    frame = dataframe_from_json(pd, frame_payload)
    column_name = set_index_column.strip()
    if column_name not in frame.columns:
        raise OracleError(f"dataframe_set_index column '{column_name}' not found")
    try:
        out = frame.set_index(column_name, drop=set_index_drop)
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
    if op == "series_join":
        return op_series_join(pd, payload)
    if op == "series_constructor":
        return op_series_constructor(pd, payload)
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
    if op == "series_any":
        return op_series_any(pd, payload)
    if op == "series_all":
        return op_series_all(pd, payload)
    if op == "series_value_counts":
        return op_series_value_counts(pd, payload)
    if op == "series_sort_index":
        return op_series_sort_index(pd, payload)
    if op == "series_sort_values":
        return op_series_sort_values(pd, payload)
    if op == "dataframe_loc":
        return op_dataframe_loc(pd, payload)
    if op == "dataframe_iloc":
        return op_dataframe_iloc(pd, payload)
    if op in {"dataframe_head", "data_frame_head"}:
        return op_dataframe_head(pd, payload)
    if op in {"dataframe_tail", "data_frame_tail"}:
        return op_dataframe_tail(pd, payload)
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
    if op in {"dataframe_fillna", "data_frame_fillna"}:
        return op_dataframe_fillna(pd, payload)
    if op in {"dataframe_dropna", "data_frame_dropna"}:
        return op_dataframe_dropna(pd, payload)
    if op in {"dataframe_dropna_columns", "data_frame_dropna_columns"}:
        return op_dataframe_dropna_columns(pd, payload)
    if op in {"dataframe_duplicated", "data_frame_duplicated"}:
        return op_dataframe_duplicated(pd, payload)
    if op in {"dataframe_drop_duplicates", "data_frame_drop_duplicates"}:
        return op_dataframe_drop_duplicates(pd, payload)
    if op in {"dataframe_set_index", "data_frame_set_index"}:
        return op_dataframe_set_index(pd, payload)
    if op in {"dataframe_reset_index", "data_frame_reset_index"}:
        return op_dataframe_reset_index(pd, payload)
    if op in {"dataframe_sort_index", "data_frame_sort_index"}:
        return op_dataframe_sort_index(pd, payload)
    if op in {"dataframe_sort_values", "data_frame_sort_values"}:
        return op_dataframe_sort_values(pd, payload)
    if op in {"dataframe_merge", "data_frame_merge"}:
        return op_dataframe_merge(pd, payload)
    if op in {"dataframe_merge_index", "data_frame_merge_index"}:
        return op_dataframe_merge_index(pd, payload)
    if op in {"dataframe_concat", "data_frame_concat"}:
        return op_dataframe_concat(pd, payload)
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
        response.setdefault("expected_scalar", None)
        response.setdefault("expected_dtype", None)
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
                "expected_scalar": None,
                "expected_dtype": None,
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
                "expected_scalar": None,
                "expected_dtype": None,
                "error": f"unexpected oracle failure: {exc}",
            },
            sys.stdout,
        )
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
