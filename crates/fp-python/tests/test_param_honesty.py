"""Parameter honesty audit (br-frankenpandas-rc0923-epic-python-honest-dropin-fvsao.5).

A binding that accepts a pandas parameter and ignores it is worse than one
that rejects it: a caller passing ascending=False who gets ascending order has
no way to know. For every public method the binding shares with pandas, each
parameter pandas gives a default is probed on small receivers:

* poison: pass an object no parameter can meaningfully take. FrankenPandas
  returning exactly its default-call result while pandas raises or answers
  differently means the value was never read: DISCARDED.
* typed: PyO3 rejects the poison for bool/str/int parameters before the method
  body runs, so those get real non-default values. FrankenPandas answering as
  with the default while pandas' answer changes: IGNORED.
* keyword: a method taking **kwargs that accepts an unknown keyword pandas
  rejects: SWALLOWED. Parameters FrankenPandas only collects through **kwargs
  are probed like named ones.

The first three verdicts fail the audit unless ALLOWLIST gives a reason.
Raising NotImplementedError for a non-default value is the honest answer
(REFUSED). RESPONSIVE means the value changed FrankenPandas' result; whether it
matches pandas is the differential suites' job, not this audit's. Only pandas'
API surface (inspect.signature) and observable results are used.
"""

from __future__ import annotations

import collections
import inspect
import math
from typing import Any, Callable

import numpy as np
import pandas as pd
import pytest

try:
    import frankenpandas as fpd
except ImportError:
    fpd = None

NAN = float("nan")
DEFAULT = object()
FAILING = ("DISCARDED", "IGNORED", "SWALLOWED")


class Poison:
    """A value no pandas parameter accepts as meaningful input."""

    def __repr__(self) -> str:
        return "<Poison>"


# Receivers per audited class, tried in order until one gives a decisive
# verdict: NaN, duplicates, unsorted labels and mixed dtypes make parameters
# like skipna, keep, sort and numeric_only change pandas' answer.
def _frame(mod: Any, mixed: bool) -> Any:
    data = {"a": [3, 1, 2, 1, 5], "b": [1.5, NAN, 2.5, 0.5, 1.5]}
    if mixed:
        data["c"] = ["x", "y", "x", None, "y"]
    return mod.DataFrame(data, index=["d", "a", "c", "b", "e"])


def _keyed(mod: Any) -> Any:
    return mod.DataFrame(
        {"k": ["x", "y", "x", None, "y", "x"], "a": [3, 1, 2, 1, 5, 4], "b": [1.5, NAN, 2.5, 0.5, 1.5, 3.0]}
    )


RECEIVERS: dict[str, list[Callable[[Any], Any]]] = {
    "Series": [
        lambda m: m.Series([3.0, NAN, 1.0, 2.0, 1.0], index=["d", "a", "c", "b", "e"], name="x"),
        lambda m: m.Series([3, 1, 2, 1, 5], index=[4, 2, 3, 1, 0], name="x"),
        lambda m: m.Series(["b", None, "a", "c", "a"], name="s"),
    ],
    "DataFrame": [
        lambda m: _frame(m, False),
        lambda m: _frame(m, True),
        lambda m: m.DataFrame({"a": [3, 1, 2, 1, 5], "b": [1.5, NAN, 2.5, 0.5, 1.5]}),
    ],
    "Index": [
        lambda m: m.Index([3, 1, 2, 1, 5]),
        lambda m: m.Index([3.0, NAN, 1.0, 1.0]),
        lambda m: m.Index(["b", "a", None, "c", "a"]),
    ],
    "DataFrameGroupBy": [lambda m: _keyed(m).groupby("k")],
    "SeriesGroupBy": [lambda m: _keyed(m).groupby("k")["b"]],
    "Rolling": [lambda m: _frame(m, False).rolling(2)],
    "Expanding": [lambda m: _frame(m, False).expanding()],
    "ExponentialMovingWindow": [lambda m: _frame(m, False).ewm(span=2)],
}

# Values for parameters pandas requires, keyed by "method.parameter" or by
# parameter name; each takes the library module and the receiver.
REQUIRED: dict[str, Callable[[Any, Any], Any]] = {
    "merge.right": lambda m, r: r,
    "join.other": lambda m, r: r.add_suffix("_r") if hasattr(r, "columns") else r,
    "between.left": lambda m, r: 1,
    "between.right": lambda m, r: 3,
    "apply.func": lambda m, r: (lambda x: x) if type(r).__name__ in ("Rolling", "Expanding") else "sum",
    "other": lambda m, r: r,
    "func": lambda m, r: "sum",
    "arg": lambda m, r: "sum",
    "values": lambda m, r: [1.0, 3],
    "dtype": lambda m, r: "float64",
    "cond": lambda m, r: r.notna(),
    "q": lambda m, r: 0.5,
    "n": lambda m, r: 2,
    "periods": lambda m, r: 1,
    "window": lambda m, r: 2,
    "lower": lambda m, r: 1,
    "upper": lambda m, r: 3,
    "left": lambda m, r: 1,
    "right": lambda m, r: 3,
    "to_replace": lambda m, r: 1.5,
    "value": lambda m, r: 0,
    "by": lambda m, r: "a" if hasattr(r, "columns") else [0, 1, 0, 1, 0][: len(r)],
    "labels": lambda m, r: [0] if not hasattr(r, "columns") else ["a"],
    "item": lambda m, r: 1,
    "key": lambda m, r: "a" if hasattr(r, "columns") else 0,
    "i": lambda m, r: 0,
    "j": lambda m, r: 0,
    "repeats": lambda m, r: 2,
    "indices": lambda m, r: [0, 1],
    "keys": lambda m, r: "a",
    "decimals": lambda m, r: 0,
    "mapper": lambda m, r: str,
    "index": lambda m, r: [0, 1],
    "columns": lambda m, r: ["a"],
    "freq": lambda m, r: "D",
    "rule": lambda m, r: "D",
    "com": lambda m, r: 0.5,
    "pat": lambda m, r: "a",
}

# Arguments a method needs before pandas runs it at all, although its
# signature marks them optional.
BASE: dict[str, dict[str, Any]] = {
    "fillna": {"value": 0},
    "filter": {"like": "a"},
    # Seeded so a sample is the same in both calls of a comparison; three
    # rows, since one draw is the same with or without replacement.
    "sample": {"random_state": 1, "n": 3},
}

# Non-default values tried, by parameter name. bool parameters also get the
# negation of pandas' default and int parameters default + 1.
VALUES: dict[str, list[Any]] = {
    "ascending": [False],
    "skipna": [False],
    "dropna": [False, True],
    "sort": [False, True],
    "normalize": [True],
    "na_position": ["first"],
    "keep": ["last", False, "all"],
    "method": ["ffill", "bfill", "nearest", "min", "max", "first", "dense", "spearman", "kendall", "pad", "linear"],
    "axis": [1, "columns"],
    "inplace": [True],
    "ignore_index": [True],
    "numeric_only": [True],
    "min_count": [1, 3],
    "ddof": [0, 2],
    "limit": [1],
    "periods": [2, -1],
    "fill_value": [0, 99],
    "level": [0],
    "kind": ["mergesort", "stable", "heapsort"],
    "key": [lambda s: -s if s.dtype.kind in "iuf" else s],
    "sort_remaining": [False],
    "errors": ["ignore", "coerce"],
    "downcast": ["infer"],
    "limit_area": ["inside", "outside"],
    "limit_direction": ["backward", "both"],
    "decimals": [1],
    "closed": ["left", "right", "both", "neither"],
    "center": [True],
    "win_type": ["triang"],
    "min_periods": [1, 3],
    "adjust": [False],
    "ignore_na": [True],
    "bias": [True],
    "pct": [True],
    "na_option": ["top", "bottom"],
    "interpolation": ["lower", "higher", "nearest", "midpoint"],
    "q": [0.25, [0.25, 0.75]],
    "n": [2, 1, 3],
    "frac": [0.5],
    "replace": [True],
    "random_state": [1],
    "dtype": ["float64", "object"],
    "na_rep": ["NA"],
    "float_format": ["%.1f"],
    "header": [False],
    "index": [False],
    "sep": [";"],
    "name": ["zz"],
    "subset": [["a"], "a"],
    "observed": [True],
    "group_keys": [False],
    "as_index": [False],
    "regex": [True],
    "case": [False],
    "expand": [True],
    "join": ["inner", "outer"],
    "how": ["left", "inner", "outer", "right", "all", "any"],
    "thresh": [2],
    "orient": ["records", "index", "list", "split"],
    "include": ["all"],
    "percentiles": [[0.1, 0.9]],
    "result_type": ["expand", "reduce", "broadcast"],
    "raw": [True],
    "convert_dtype": [False],
    "na_action": ["ignore"],
    "allow_duplicates": [True],
    "col_level": [1],
    "col_fill": ["x"],
    "drop": [True],
    "append": [True],
    "verify_integrity": [True],
    "validate": ["one_to_one"],
    "indicator": [True],
    "suffixes": [("_l", "_r")],
    "left_index": [True],
    "right_index": [True],
    "weights": [[0.1, 0.2, 0.3, 0.2, 0.2]],
    "freq": ["D", 2],
    "suffix": ["_s"],
    "prefix": ["p_"],
    "lower": [1.0],
    "upper": [2.0],
    "exclude": [["a"]],
    "coerce_float": [True],
    "nrows": [1],
    "mode": ["a"],
    "compression": ["gzip"],
    "engine": ["python", "numba"],
    "copy": [False, True],
    "order": [[1, 0]],
    "dropna_first": [True],
    "tolerance": [1],
    "sort_index": [True],
    "before": [1, "b"],
    "after": [2, "c"],
    "start": [1],
    "end": [2],
    "to_replace": [1.0, 3],
    "value": [0.0],
}

# Methods not called by the audit, with the reason.
SKIP: dict[str, str] = {
    "plot": "draws with matplotlib",
    "hist": "draws with matplotlib",
    "boxplot": "draws with matplotlib",
    "to_clipboard": "writes the system clipboard",
    "to_pickle": "writes a file; a path argument is required",
    "to_hdf": "writes a file; a path argument is required",
    "to_parquet": "writes a file when given a path",
    "to_feather": "writes a file; a path argument is required",
    "to_orc": "writes a file when given a path",
    "to_excel": "writes a file; a path argument is required",
    "to_stata": "writes a file; a path argument is required",
    "to_sql": "needs a database connection",
    "to_gbq": "needs Google BigQuery",
    "pipe": "calls a user function with the receiver",
}

# (class, method, parameter) -> why its verdict is not a defect.
ALLOWLIST: dict[tuple[str, str, str], str] = {
    ("DataFrame", "fillna", "axis"): (
        "axis is honoured for method fills (ffill_axis1/bfill_axis1). With a scalar value pandas' "
        "axis=1 fills a transposed copy, so its only effect is upcasting int columns to float64; "
        "the values are the same"
    ),
}


def _deferred(x: Any) -> bool:
    return type(x).__name__.endswith(("GroupBy", "Rolling", "Expanding", "ExponentialMovingWindow", "Resampler"))


def _snap(x: Any) -> Any:
    """A comparable picture of a result, stable across identical calls."""
    if isinstance(x, (float, np.floating)):
        # Rounded: pandas' spearman of a series with itself is
        # 0.9999999999999998 where pearson is 1.0, which is not a difference
        # a parameter made.
        return ("scalar", "nan" if math.isnan(x) else float(f"{float(x):.12g}"))
    if x is None or isinstance(x, (bool, int, str, bytes, np.integer, np.bool_)):
        return ("scalar", x)
    if _deferred(x):
        try:
            return ("deferred", type(x).__name__, _snap(x.sum()))
        except Exception as exc:  # noqa: BLE001 - the error class is the picture
            return ("deferred", type(x).__name__, "raise", type(exc).__name__)
    if inspect.isgenerator(x) or type(x).__name__.endswith("iterator"):
        x = list(x)
    if isinstance(x, (list, tuple)):
        return ("seq", tuple(_snap(v) for v in x))
    parts: list[Any] = [type(x).__name__, repr(x)]
    for attr in ("dtype", "dtypes", "index", "columns", "name"):
        try:
            parts.append(repr(getattr(x, attr)))
        except Exception:  # noqa: BLE001 - absent attributes are part of the picture
            parts.append(None)
    return tuple(parts)


def _call(receiver: Callable[[], Any], method: str, required: dict[str, Any], param: str | None, value: Any) -> tuple:
    """(True, picture) for a result, (False, exception class name) for a raise."""
    try:
        recv = receiver()
        kwargs = dict(required)
        if param is not None and value is not DEFAULT:
            kwargs[param] = value
        result = getattr(recv, method)(**kwargs)
        return (True, (_snap(result), _snap(recv)))
    except Exception as exc:  # noqa: BLE001 - any raise is an outcome
        return (False, type(exc).__name__)


def _candidates(param: str, default: Any) -> list[Any]:
    out = [v for v in VALUES.get(param, []) if not _same(v, default)]
    if isinstance(default, bool):
        out.append(not default)
    elif isinstance(default, int) and param not in VALUES:
        out.append(default + 1)
    return out


def _same(a: Any, b: Any) -> bool:
    try:
        return bool(a == b) and type(a) is type(b)
    except Exception:  # noqa: BLE001 - array-likes compare elementwise
        return False


def probe(fp_recv: Callable[[], Any], pd_recv: Callable[[], Any], method: str, param: str,
          default: Any, fp_required: dict[str, Any], pd_required: dict[str, Any]) -> tuple[str, str]:
    """Verdict for one parameter on one receiver pair."""
    fp_base = _call(fp_recv, method, fp_required, None, DEFAULT)
    pd_base = _call(pd_recv, method, pd_required, None, DEFAULT)
    if not pd_base[0]:
        return ("UNPROBED", f"pandas default call raises {pd_base[1]}")
    if not fp_base[0]:
        return ("UNPROBED", f"default call raises {fp_base[1]}")
    if _call(fp_recv, method, fp_required, None, DEFAULT) != fp_base:
        return ("UNPROBED", "result differs between identical calls")
    if _call(pd_recv, method, pd_required, None, DEFAULT) != pd_base:
        return ("UNPROBED", "pandas' result differs between identical calls")

    garbage_accepted = False
    fp_poison = _call(fp_recv, method, fp_required, param, Poison())
    if fp_poison == fp_base:
        pd_poison = _call(pd_recv, method, pd_required, param, Poison())
        if pd_poison == pd_base:
            return ("INCONCLUSIVE", "pandas also ignores a garbage value here")
        # A value-like parameter (to_replace, after, ...) can take garbage as a
        # value that matches nothing; only a real value tells whether it is read.
        garbage_accepted = True
    elif fp_poison == (False, "NotImplementedError"):
        return ("REFUSED", "garbage value raises NotImplementedError")
    elif fp_poison[0]:
        return ("RESPONSIVE", "the garbage value changed the result")

    for value in _candidates(param, default):
        pd_value = _call(pd_recv, method, pd_required, param, value)
        if not pd_value[0] or pd_value == pd_base:
            continue
        fp_value = _call(fp_recv, method, fp_required, param, value)
        if fp_value == (False, "NotImplementedError"):
            return ("REFUSED", f"{value!r} raises NotImplementedError")
        if fp_value == fp_base:
            return ("IGNORED", f"{value!r} changes pandas' result but not FrankenPandas'")
        return ("RESPONSIVE", f"{value!r} changed the result")
    if garbage_accepted:
        return ("DISCARDED", "a garbage value returns the default result and no real value shows it is read")
    return ("INCONCLUSIVE", "no candidate value changes pandas' result on these receivers")


def _params(fn: Any) -> dict[str, inspect.Parameter] | None:
    try:
        return dict(inspect.signature(fn).parameters)
    except (TypeError, ValueError):
        return None


def _required(mod: Any, recv: Any, method: str, pd_params: dict[str, inspect.Parameter]) -> dict[str, Any] | None:
    """Arguments every call of `method` carries: pandas' required parameters
    and the BASE arguments pandas needs before it runs at all."""
    out = dict(BASE.get(method, {}))
    for name, p in pd_params.items():
        if name == "self" or p.kind in (p.VAR_POSITIONAL, p.VAR_KEYWORD) or p.default is not p.empty:
            continue
        maker = REQUIRED.get(f"{method}.{name}", REQUIRED.get(name))
        if maker is None:
            return None
        try:
            out[name] = maker(mod, recv)
        except Exception:  # noqa: BLE001 - the method stays unprobed on this receiver
            return None
    return out


def _audited_classes() -> dict[str, tuple[type, type]]:
    return {name: (type(makers[0](fpd)), type(makers[0](pd))) for name, makers in RECEIVERS.items()}


def run_audit() -> list[tuple[str, str, str, str, str]]:
    """(class, method, parameter, verdict, detail) for every probed parameter."""
    rows = []
    for cls_name, (fp_cls, pd_cls) in _audited_classes().items():
        for method in sorted(dir(pd_cls)):
            if method.startswith("_") or method in SKIP or not hasattr(fp_cls, method):
                continue
            if isinstance(inspect.getattr_static(pd_cls, method, None), property):
                continue
            pd_params, fp_params = _params(getattr(pd_cls, method)), _params(getattr(fp_cls, method))
            if pd_params is None or fp_params is None:
                continue
            fp_var_kw = any(p.kind == p.VAR_KEYWORD for p in fp_params.values())
            rows.extend(_audit_method(cls_name, method, pd_params, fp_params, fp_var_kw))
    return rows


def _audit_method(cls_name: str, method: str, pd_params: dict, fp_params: dict, fp_var_kw: bool) -> list:
    rows = []
    makers = RECEIVERS[cls_name]
    probed = [
        name for name, p in pd_params.items()
        if name != "self" and p.kind not in (p.VAR_POSITIONAL, p.VAR_KEYWORD) and p.default is not p.empty
        and (name in fp_params or fp_var_kw)
    ]
    if fp_var_kw:
        rows.append((cls_name, method, "**kwargs", *_keyword_probe(makers, method, pd_params)))
    for param in probed:
        verdict = ("UNPROBED", f"no value for a required parameter of {method}")
        for make in makers:
            fp_req = _required(fpd, make(fpd), method, pd_params)
            pd_req = _required(pd, make(pd), method, pd_params)
            if fp_req is None or pd_req is None:
                break
            if param in fp_req:
                verdict = ("UNPROBED", f"{param} is one of the arguments {method} needs to run")
                break
            found = probe(lambda make=make: make(fpd), lambda make=make: make(pd), method, param,
                          pd_params[param].default, fp_req, pd_req)
            # Keep the most informative verdict: an inconclusive run beats a
            # receiver the method cannot run on.
            if verdict[0] == "UNPROBED" or found[0] != "UNPROBED":
                verdict = found
            if found[0] not in ("INCONCLUSIVE", "UNPROBED"):
                break
        rows.append((cls_name, method, param, *verdict))
    return rows


def _keyword_probe(makers: list, method: str, pd_params: dict) -> tuple[str, str]:
    for make in makers:
        fp_req = _required(fpd, make(fpd), method, pd_params)
        pd_req = _required(pd, make(pd), method, pd_params)
        if fp_req is None or pd_req is None:
            return ("UNPROBED", f"no value for a required parameter of {method}")
        fp_out = _call(lambda: make(fpd), method, fp_req, "fp_honesty_probe", 1)
        pd_out = _call(lambda: make(pd), method, pd_req, "fp_honesty_probe", 1)
        if fp_out[0] and not pd_out[0]:
            return ("SWALLOWED", "an unknown keyword pandas rejects is accepted")
        if not fp_out[0]:
            return ("RESPONSIVE", f"an unknown keyword raises {fp_out[1]}")
    return ("INCONCLUSIVE", "pandas also accepts an unknown keyword")


def _summary(rows: list) -> str:
    by_class: dict[str, collections.Counter] = collections.defaultdict(collections.Counter)
    for cls_name, _method, _param, verdict, _detail in rows:
        by_class[cls_name][verdict] += 1
    total = collections.Counter(r[3] for r in rows)
    lines = [f"parameter honesty: {len(rows)} probed parameters: " + ", ".join(f"{k} {v}" for k, v in sorted(total.items()))]
    for cls_name, counts in by_class.items():
        lines.append(f"  {cls_name}: " + ", ".join(f"{k} {v}" for k, v in sorted(counts.items())))
    return "\n".join(lines)


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_no_pandas_parameter_is_accepted_and_ignored() -> None:
    rows = run_audit()
    print(_summary(rows))
    bad = [r for r in rows if r[3] in FAILING and (r[0], r[1], r[2]) not in ALLOWLIST]
    stale = [k for k in ALLOWLIST if not any((r[0], r[1], r[2]) == k and r[3] in FAILING for r in rows)]
    assert not stale, f"ALLOWLIST entries whose parameter is no longer flagged: {stale}"
    assert not bad, f"{len(bad)} parameters accepted and not honoured:\n" + "\n".join(
        f"  {c}.{m}({p}=...): {v} - {d}" for c, m, p, v, d in bad
    )


# NEGATIVE: the audit must catch a binding that drops a parameter. The doubles
# wrap a real FrankenPandas Series and each drop one parameter a different way.
class _Double:
    def __init__(self, inner: Any) -> None:
        self.inner = inner

    def __repr__(self) -> str:
        # The audit pictures the receiver too; show the Series, not an address.
        return repr(self.inner)


class _DropsAscending(_Double):
    """Rejects a non-bool as PyO3's extraction does, then drops the value."""

    def sort_values(self, ascending: bool = True) -> Any:
        if not isinstance(ascending, bool):
            raise TypeError("argument 'ascending': not a bool")
        return self.inner.sort_values()


class _DropsN(_Double):
    def head(self, n: Any = 5) -> Any:
        return self.inner.head()


class _DropsUnknown(_Double):
    """Takes a keyword pandas does not have and no real value exists for."""

    def abs(self, zz: Any = None) -> Any:
        return self.inner.abs()


class _SwallowsKeywords(_Double):
    def sort_values(self, **kwargs: Any) -> Any:
        return self.inner.sort_values()


class _Honours(_Double):
    def sort_values(self, ascending: bool = True) -> Any:
        if not isinstance(ascending, bool):
            raise TypeError("ascending must be a bool")
        return self.inner.sort_values(ascending=ascending)

    def head(self, n: int = 5) -> Any:
        if not isinstance(n, int):
            raise TypeError("n must be an int")
        return self.inner.head(n)


def _series(mod: Any) -> Any:
    return mod.Series([3.0, 1.0, 2.0, 5.0, 4.0, 0.0], name="x")


@pytest.mark.skipif(fpd is None, reason="frankenpandas not installed")
def test_the_audit_catches_dropped_parameters() -> None:
    pd_series = lambda: _series(pd)  # noqa: E731
    # A typed parameter (garbage rejected) whose real value is dropped.
    dropped = probe(lambda: _DropsAscending(_series(fpd)), pd_series, "sort_values", "ascending", True, {}, {})
    assert dropped[0] == "IGNORED", dropped
    # Garbage accepted, and a real value then shows the drop.
    garbage_then_real = probe(lambda: _DropsN(_series(fpd)), pd_series, "head", "n", 5, {}, {})
    assert garbage_then_real[0] == "IGNORED", garbage_then_real
    # Garbage accepted and no real value to try.
    discarded = probe(lambda: _DropsUnknown(_series(fpd)), pd_series, "abs", "zz", None, {}, {})
    assert discarded[0] == "DISCARDED", discarded
    swallowed = _keyword_probe([lambda m: _SwallowsKeywords(_series(fpd)) if m is fpd else _series(pd)],
                               "sort_values", {})
    assert swallowed[0] == "SWALLOWED", swallowed
    # ...and must not flag a double that reads its parameters.
    honest = [probe(lambda: _Honours(_series(fpd)), pd_series, m, p, d, {}, {})
              for m, p, d in (("sort_values", "ascending", True), ("head", "n", 5))]
    assert [v for v, _ in honest] == ["RESPONSIVE", "RESPONSIVE"], honest
