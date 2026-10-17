#!/usr/bin/env python3
"""Enumerate the pandas public API surface we intend to mirror.

Per br-frankenpandas-zk1j: frankenpandas claims pandas parity but has
no machine-readable list of "what pandas exposes". This script walks
`pd.{Series,DataFrame,GroupBy,Index,Rolling,Expanding,Ewm,Resampler}`
(plus top-level helpers) and emits a JSON catalog to
`artifacts/pandas_api_listing.json`.

Usage:
    python3 scripts/gen_pandas_api_listing.py
    python3 scripts/gen_pandas_api_listing.py --out custom/path.json

The generated JSON feeds `scripts/gen_coverage_matrix.py` which diffs
it against the operation strings present in our fixture corpus.
"""
from __future__ import annotations

import argparse
import inspect
import json
from pathlib import Path
from typing import Any


# Classes + the dotted path used to address them in the JSON output.
# Keeps future additions (e.g. pd.api.*, pd.tseries.*) additive.
CLASSES_TO_ENUMERATE = [
    ("pandas.Series", "Series"),
    ("pandas.DataFrame", "DataFrame"),
    ("pandas.Index", "Index"),
    ("pandas.MultiIndex", "MultiIndex"),
    ("pandas.DatetimeIndex", "DatetimeIndex"),
    ("pandas.CategoricalIndex", "CategoricalIndex"),
    ("pandas.IntervalIndex", "IntervalIndex"),
    ("pandas.TimedeltaIndex", "TimedeltaIndex"),
    ("pandas.PeriodIndex", "PeriodIndex"),
    ("pandas.RangeIndex", "RangeIndex"),
    ("pandas.core.groupby.DataFrameGroupBy", "DataFrameGroupBy"),
    ("pandas.core.groupby.SeriesGroupBy", "SeriesGroupBy"),
    ("pandas.core.window.Rolling", "Rolling"),
    ("pandas.core.window.Expanding", "Expanding"),
    ("pandas.core.window.ExponentialMovingWindow", "ExponentialMovingWindow"),
    ("pandas.core.resample.Resampler", "Resampler"),
]

# Accessor namespaces (pd.Series.str.*, pd.Series.dt.*, pd.Series.cat.*,
# pd.Series.sparse.*, pd.Series.list.*, pd.Series.struct.*).
ACCESSOR_NAMESPACES = [
    ("pandas.core.strings.accessor.StringMethods", "Series.str"),
    ("pandas.core.indexes.accessors.DatetimeProperties", "Series.dt"),
    ("pandas.core.arrays.categorical.CategoricalAccessor", "Series.cat"),
    ("pandas.core.arrays.sparse.accessor.SparseAccessor", "Series.sparse"),
    ("pandas.core.arrays.arrow.accessors.ListAccessor", "Series.list"),
    ("pandas.core.arrays.arrow.accessors.StructAccessor", "Series.struct"),
]

SCALAR_AND_OFFSET_TYPES = [
    ("pandas.Timestamp", "Timestamp"),
    ("pandas.Timedelta", "Timedelta"),
    ("pandas.Period", "Period"),
    ("pandas.Interval", "Interval"),
    ("pandas.DateOffset", "DateOffset"),
]

FAMILY_BY_ALIAS = {
    "DataFrame": "core",
    "Series": "core",
    "Index": "index",
    "MultiIndex": "index",
    "DatetimeIndex": "index",
    "CategoricalIndex": "index",
    "IntervalIndex": "index",
    "TimedeltaIndex": "index",
    "PeriodIndex": "index",
    "RangeIndex": "index",
    "DataFrameGroupBy": "groupby_window",
    "SeriesGroupBy": "groupby_window",
    "Rolling": "groupby_window",
    "Expanding": "groupby_window",
    "ExponentialMovingWindow": "groupby_window",
    "Resampler": "groupby_window",
    "Series.str": "accessor",
    "Series.dt": "accessor",
    "Series.cat": "accessor",
    "Series.sparse": "accessor",
    "Series.list": "accessor",
    "Series.struct": "accessor",
    "Timestamp": "scalar_offset",
    "Timedelta": "scalar_offset",
    "Period": "scalar_offset",
    "Interval": "scalar_offset",
    "DateOffset": "scalar_offset",
    "pandas_top_level": "top_level",
}


def resolve_class(dotted: str) -> Any:
    """Resolve a dotted class path to the class object."""
    parts = dotted.split(".")
    module_parts: list[str] = []
    for i, part in enumerate(parts):
        module_parts.append(part)
        candidate = ".".join(module_parts)
        try:
            obj = __import__(candidate)
            for inner in parts[i + 1 :]:
                obj = getattr(obj, inner)
            return obj
        except (ImportError, AttributeError):
            continue
    raise ImportError(f"cannot resolve {dotted}")


def _is_public(name: str) -> bool:
    return not name.startswith("_")


def _first_line(text: str | None) -> str:
    if not text:
        return ""
    for line in text.splitlines():
        stripped = line.strip()
        if stripped:
            return stripped
    return ""


def enumerate_class(cls: Any) -> list[dict[str, Any]]:
    members = []
    for name in sorted(dir(cls)):
        if not _is_public(name):
            continue
        try:
            attr = getattr(cls, name)
        except AttributeError:
            continue

        kind = "method" if callable(attr) else "property"
        doc = _first_line(getattr(attr, "__doc__", None))
        signature = ""
        if callable(attr):
            try:
                signature = f"{name}{inspect.signature(attr)}"
            except (TypeError, ValueError):
                signature = f"{name}(...)"
        members.append(
            {
                "name": name,
                "kind": kind,
                "signature": signature,
                "docstring_first_line": doc,
            }
        )
    return members


def enumerate_top_level_functions(pd_mod: Any) -> list[dict[str, Any]]:
    members = []
    for name in sorted(dir(pd_mod)):
        if not _is_public(name):
            continue
        try:
            attr = getattr(pd_mod, name)
        except AttributeError:
            continue
        if inspect.isclass(attr) or inspect.ismodule(attr):
            continue
        if not callable(attr):
            continue
        doc = _first_line(getattr(attr, "__doc__", None))
        try:
            signature = f"{name}{inspect.signature(attr)}"
        except (TypeError, ValueError):
            signature = f"{name}(...)"
        members.append(
            {
                "name": name,
                "kind": "function",
                "signature": signature,
                "docstring_first_line": doc,
            }
        )
    return members


def build_listing(pd_mod: Any) -> dict[str, Any]:
    pandas_version = str(getattr(pd_mod, "__version__", "unknown"))
    listing: dict[str, Any] = {
        "pandas_version": pandas_version,
        "generated_by": "scripts/gen_pandas_api_listing.py",
        "classes": {},
    }
    all_targets = (
        list(CLASSES_TO_ENUMERATE)
        + list(ACCESSOR_NAMESPACES)
        + list(SCALAR_AND_OFFSET_TYPES)
    )
    for dotted, alias in all_targets:
        try:
            cls = resolve_class(dotted)
        except ImportError:
            listing["classes"][alias] = {"error": f"failed to resolve {dotted}"}
            continue
        listing["classes"][alias] = {
            "dotted_path": dotted,
            "family": FAMILY_BY_ALIAS.get(alias, "other"),
            "members": enumerate_class(cls),
        }
    listing["classes"]["pandas_top_level"] = {
        "dotted_path": "pandas",
        "family": FAMILY_BY_ALIAS["pandas_top_level"],
        "members": enumerate_top_level_functions(pd_mod),
    }
    return listing


def main() -> int:
    parser = argparse.ArgumentParser(description="Enumerate pandas public API")
    parser.add_argument(
        "--out",
        default="artifacts/pandas_api_listing.json",
        help="Output JSON path (default: artifacts/pandas_api_listing.json)",
    )
    args = parser.parse_args()

    import pandas as pd

    listing = build_listing(pd)
    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(listing, indent=2, sort_keys=True) + "\n")

    total = sum(
        len(entry.get("members", []))
        for entry in listing["classes"].values()
        if isinstance(entry, dict)
    )
    by_family: dict[str, int] = {}
    for entry in listing["classes"].values():
        if isinstance(entry, dict) and "members" in entry:
            fam = str(entry.get("family", "other"))
            by_family[fam] = by_family.get(fam, 0) + len(entry["members"])
    fam_summary = ", ".join(f"{k}={v}" for k, v in sorted(by_family.items()))
    print(
        f"pandas_api_listing: pandas={pd.__version__} "
        f"surfaces={len(listing['classes'])} members={total} ({fam_summary}) → {out_path}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
