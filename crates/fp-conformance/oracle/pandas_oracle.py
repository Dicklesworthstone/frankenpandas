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


def require_join_type(payload: dict[str, Any], op_name: str) -> str:
    join_type = payload.get("join_type")
    if join_type not in {"inner", "left", "right", "outer"}:
        raise OracleError(
            f"{op_name} requires join_type=inner|left|right|outer, got {join_type!r}"
        )
    return str(join_type)


def dataframe_with_index_key(frame, key_name: str):
    out = frame.copy()
    out[key_name] = frame.index.tolist()
    return out


def op_dataframe_merge(
    pd, payload: dict[str, Any], *, use_index_keys: bool = False
) -> dict[str, Any]:
    frame_payload = payload.get("frame")
    frame_right_payload = payload.get("frame_right")
    if frame_payload is None or frame_right_payload is None:
        raise OracleError("dataframe_merge requires frame and frame_right payloads")

    how = require_join_type(
        payload, "dataframe_merge_index" if use_index_keys else "dataframe_merge"
    )

    left = dataframe_from_json(pd, frame_payload)
    right = dataframe_from_json(pd, frame_right_payload)

    if use_index_keys:
        merge_on_raw = payload.get("merge_on")
        merge_on = (
            str(merge_on_raw)
            if isinstance(merge_on_raw, str) and merge_on_raw
            else "__index_key"
        )
        left = dataframe_with_index_key(left, merge_on)
        right = dataframe_with_index_key(right, merge_on)
    else:
        merge_on_raw = payload.get("merge_on")
        if not isinstance(merge_on_raw, str) or not merge_on_raw:
            raise OracleError("dataframe_merge requires merge_on string payload")
        merge_on = merge_on_raw

    out = left.merge(
        right,
        on=merge_on,
        how=how,
        sort=False,
        copy=False,
        suffixes=("_left", "_right"),
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

    if sorted(left.columns.tolist()) != sorted(right.columns.tolist()):
        raise OracleError(
            "dataframe_concat column mismatch: right frame columns do not match left frame"
        )

    out = pd.concat([left, right], axis=0, sort=False)
    return {"expected_frame": dataframe_to_json(out)}


def dispatch(pd, payload: dict[str, Any]) -> dict[str, Any]:
    op = payload.get("operation")
    if op == "series_add":
        return op_series_add(pd, payload)
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
    if op == "dataframe_loc":
        return op_dataframe_loc(pd, payload)
    if op == "dataframe_iloc":
        return op_dataframe_iloc(pd, payload)
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
