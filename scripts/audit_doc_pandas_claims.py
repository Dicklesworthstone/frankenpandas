#!/usr/bin/env python3
"""Resolve every ``pd.*`` surface cited in a Rust doc comment against real pandas.

A doc that says "Matches ``pd.Series.dt.week``" is a testable claim, and the
cheapest half of it is testable without running FP at all: does that attribute
EXIST in the pandas we target? Removed surfaces (``.append``,
``MultiIndex.is_lexsorted``, ``Series.dt.week``) and invented ones
(``Series.str.isascii``, assumed from Python's ``str``) both show up here.

    python3 scripts/audit_doc_pandas_claims.py                 # whole workspace
    python3 scripts/audit_doc_pandas_claims.py crates/fp-index # one crate

Read-only: prints a report, writes nothing. Safe under a build freeze -- it needs
pandas, not cargo.

⚠️ ONE PROTOTYPE PER CLASS IS NOT ENOUGH, and getting this wrong makes the audit
worse than useless: ``.dt.days`` is absent on a datetime Series and present on a
timedelta one, ``.sparse`` needs a sparse dtype, ``Index.str`` needs string
labels. Every accessor path is therefore tried against EVERY plausible dtype and
reported MISSING only if all of them raise. On 2026-08-19 the naive version
reported 56 misses of which 15 were its own prototype choice.

Existence is necessary, not sufficient: an attribute that resolves can still
behave differently (``Timestamp::now`` matched ``utcnow()``, four hours off).
This narrows where to look; it does not certify the survivors.
"""
from __future__ import annotations
import re, sys, pathlib
import pandas as pd

CLAIM_RE = re.compile(r"`(pd\.[A-Za-z_][A-Za-z0-9_.()]*)`")


# A doc that CORRECTS a false claim has to keep citing the missing surface, so a
# bare existence check would stay red forever and quickly be ignored. A block
# carrying one of these markers has already been examined; only unmarked blocks
# are reported. Do not add a marker without measuring first -- that turns the
# gate off, which is the whole value.
ABSENCE_MARKERS = (
    "AttributeError",
    "does NOT exist",
    "do NOT exist",
    "NO pandas",
    "NOT A PARITY SURFACE",
    "REMOVED, not deprecated",
)


def doc_blocks(lines: list[str]):
    """Yield (start, end) index pairs for each run of contiguous doc lines."""
    start = None
    for i, line in enumerate(lines):
        stripped = line.lstrip()
        if stripped.startswith("///") or stripped.startswith("//!"):
            if start is None:
                start = i
        elif start is not None:
            yield start, i
            start = None
    if start is not None:
        yield start, len(lines)


def prototypes():
    """Roots to resolve against. Lists are tried in order; first hit wins."""
    s_dt = pd.Series(pd.to_datetime(["2024-01-04", "2024-06-15"]))
    s_td = pd.Series(pd.to_timedelta(["1 days", "2 days"]))
    s_per = pd.Series(pd.period_range("2024-01", periods=2, freq="M"))
    s_str = pd.Series(["a\tb", "x"])
    s_cat = pd.Series(pd.Categorical(["a", "b"]))
    s_sp = pd.Series(pd.arrays.SparseArray([0.0, 1.0]))
    s_num = pd.Series([1.0, 2.0])
    return {
        # accessor segment -> every dtype whose accessor may carry the member
        "_accessors": {
            "dt": [s_dt, s_td, s_per],
            "str": [s_str],
            "cat": [s_cat],
            "sparse": [s_sp],
        },
        "Series": [s_num, s_dt, s_td, s_per, s_str, s_cat, s_sp],
        "DataFrame": [pd.DataFrame({"a": [1, 2]}),
                      pd.DataFrame({"a": pd.arrays.SparseArray([0.0, 1.0])})],
        "Index": [pd.Index([1, 2]), pd.Index(["a", "b"])],
        "RangeIndex": [pd.RangeIndex(5)],
        "DatetimeIndex": [pd.DatetimeIndex(["2024-01-04", "2024-02-04"])],
        "TimedeltaIndex": [pd.TimedeltaIndex(["1 days"])],
        "PeriodIndex": [pd.PeriodIndex(["2024-01-04"], freq="D")],
        "CategoricalIndex": [pd.CategoricalIndex(["a", "b"])],
        "IntervalIndex": [pd.IntervalIndex.from_breaks([0, 1, 2])],
        "MultiIndex": [pd.MultiIndex.from_tuples([(1, "a"), (2, "b")])],
        "Timestamp": [pd.Timestamp("2024-01-04")],
        "Timedelta": [pd.Timedelta("1 days")],
        "Period": [pd.Period("2024-01-04", freq="D")],
        "Categorical": [pd.Categorical(["a", "b"])],
        "Interval": [pd.Interval(0, 1)],
        # groupby / window objects are cited as `pd.DataFrameGroupBy.x` in this
        # tree; that shorthand names no module attribute, so map it to the real
        # object rather than reporting the whole convention as missing.
        "DataFrameGroupBy": [pd.DataFrame({"a": [1, 2]}).groupby("a")],
        "SeriesGroupBy": [pd.Series([1, 2]).groupby(pd.Series([1, 2]))],
        "GroupBy": [pd.DataFrame({"a": [1, 2]}).groupby("a")],
        "Resampler": [pd.Series([1.0], index=pd.to_datetime(["2024-01-01"])).resample("D")],
        "Rolling": [pd.DataFrame({"a": [1, 2]}).rolling(2)],
        "ExponentialMovingWindow": [pd.DataFrame({"a": [1, 2]}).ewm(2)],
    }


def resolve(path: str, protos) -> tuple[str, str]:
    """-> (OK | MISSING | UNVERIFIABLE, detail)."""
    segs = path.split(".")[1:]
    if not segs:
        return "UNVERIFIABLE", "bare pd"
    head = re.sub(r"\(.*", "", segs[0])
    from_class = head in protos
    roots, rest = (protos[head], segs[1:]) if from_class else ([pd], segs)
    # An accessor narrows which prototypes can possibly work -- but ONLY when the
    # path started at a class. `pd.to_datetime(x).dt.day_name()` also contains
    # `dt`, and swapping its root to a Series would resolve `to_datetime` against
    # a Series and report a false miss.
    if from_class:
        for seg in rest:
            base = re.sub(r"\(.*", "", seg)
            if base in protos["_accessors"]:
                roots = protos["_accessors"][base]
                break
    last_detail = ""
    for root in roots:
        obj, failed = root, None
        for seg in rest:
            base = re.sub(r"\(.*", "", seg)
            if not base:
                return "UNVERIFIABLE", "empty segment"
            try:
                obj = getattr(obj, base)
            except AttributeError:
                owner = obj.__name__ if isinstance(obj, type) else type(obj).__name__
                failed = f"no {base!r} on {owner}"
                break
            except Exception as exc:                      # noqa: BLE001
                return "UNVERIFIABLE", f"{type(exc).__name__} on {base}"
            if seg != base and seg is not rest[-1]:
                return "UNVERIFIABLE", f"call mid-path at {base}"
        if failed is None:
            return "OK", ""
        last_detail = failed
    return "MISSING", last_detail


def main(argv: list[str]) -> int:
    roots = [pathlib.Path(a) for a in argv[1:]] or [pathlib.Path("crates")]
    claims: dict[str, list[str]] = {}
    acknowledged: set[tuple[str, str]] = set()
    for root in roots:
        for rs in root.rglob("*.rs"):
            lines = rs.read_text(encoding="utf-8").splitlines()
            for start, end in doc_blocks(lines):
                block = "\n".join(lines[start:end])
                low = block.lower()
                marked = any(k.lower() in low for k in ABSENCE_MARKERS)
                for i in range(start, end):
                    for m in CLAIM_RE.finditer(lines[i]):
                        site = f"{rs}:{i + 1}"
                        claims.setdefault(m.group(1), []).append(site)
                        if marked:
                            acknowledged.add((m.group(1), site))

    protos = prototypes()
    missing, ack, unver, ok = [], 0, 0, 0
    for path in sorted(claims):
        status, detail = resolve(path, protos)
        if status == "MISSING":
            live = [s for s in claims[path] if (path, s) not in acknowledged]
            ack += len(claims[path]) - len(live)
            if live:
                missing.append((path, detail, live))
        elif status == "UNVERIFIABLE":
            unver += 1
        else:
            ok += 1

    print(f"pandas {pd.__version__}: {len(claims)} cited surfaces -- "
          f"{ok} resolve, {unver} unverifiable, {ack} absences already documented, "
          f"{len(missing)} UNEXAMINED")
    for path, detail, sites in missing:
        print(f"\n  MISSING  {path}   ({detail})")
        for site in sites[:6]:
            print(f"           {site}")
    if not missing:
        print("\nNo undocumented false claims. (Absences that ARE documented stay "
              "cited on purpose -- a doc saying 'X does not exist' must name X.)")
    return 1 if missing else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
