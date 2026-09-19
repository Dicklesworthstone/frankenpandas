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
# pd.DataFrame.plot.*, ...).  Addressed via attribute chain.
ACCESSOR_NAMESPACES = [
    ("Series.str", "pandas.core.strings.accessor.StringMethods"),
    ("Series.dt", "pandas.core.indexes.accessors.DatetimeProperties"),
    ("Series.cat", "pandas.core.arrays.categorical.CategoricalAccessor"),
]


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


def build_listing(pandas_version: str) -> dict[str, Any]:
    listing: dict[str, Any] = {
        "pandas_version": pandas_version,
        "generated_by": "scripts/gen_pandas_api_listing.py",
        "classes": {},
    }
    for dotted, alias in CLASSES_TO_ENUMERATE:
        try:
            cls = resolve_class(dotted)
        except ImportError:
            listing["classes"][alias] = {"error": f"failed to resolve {dotted}"}
            continue
        listing["classes"][alias] = {
            "dotted_path": dotted,
            "members": enumerate_class(cls),
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

    listing = build_listing(pd.__version__)
    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(listing, indent=2, sort_keys=True))

    total = sum(
        len(entry.get("members", []))
        for entry in listing["classes"].values()
        if isinstance(entry, dict)
    )
    print(
        f"pandas_api_listing: pandas={pd.__version__} "
        f"classes={len(listing['classes'])} members={total} → {out_path}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
