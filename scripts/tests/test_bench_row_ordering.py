#!/usr/bin/env python3
"""The census and scorecard pick each lane's newest row by MEASUREMENT time
(br-frankenpandas-rc0923-epic-zero-certified-losses-bss5q.1).

THE OBSERVED DEFECT: both scripts ordered rows by file mtime. Commit 0cf20bc43
landed 89 rows a week late, so four LOSING rows measured on 2026-08-27 at
17:00 carried 2026-09-04 mtimes and outranked the winning reruns of the same
lanes measured at 17:26-18:15 (series_kurtosis, series_skew, atan, log1p
@100k). A fresh clone, where every mtime is the checkout time, selected rows by
directory order: 3.14x with 35 losses against 3.93x with 14 on the same commit.

Every case below writes documents whose mtimes DISAGREE with their embedded
timestamps, so the old mtime rule fails each one.

Run:  python3 -m pytest scripts/tests/ -v
      python3 scripts/tests/test_bench_row_ordering.py    (no pytest needed)
"""
from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(SCRIPTS))

from current_loss_census import measured_at  # noqa: E402
from gen_perf_scorecard import scan  # noqa: E402

LANE = ("dataframe_ops", "series_kurtosis", "100k", "float64_nan10")


def _document(timestamp: str | None, ratio: float, elf: str) -> dict:
    row = {
        "category": LANE[0],
        "workload": LANE[1],
        "size": LANE[2],
        "dtype": LANE[3],
        "ratio": ratio,
        "verdict": "FASTER" if ratio >= 1.0 else "SLOWER",
        "median_ci_gate": {"decidable": True, "clauses": {"a": True, "b": True, "c": True}},
        "frankenpandas": {"executable": {"sha256": elf}, "thread_count_actually_used": 1},
        "pandas": {"thread_count_actually_used": 1},
    }
    document = {"schema_version": 4, "results": [row]}
    if timestamp is not None:
        document["timestamp"] = timestamp
    return document


def _write(directory: Path, name: str, document: dict, mtime: float) -> None:
    path = directory / name
    path.write_text(json.dumps(document))
    os.utime(path, (mtime, mtime))


def _late_committed_loss(directory: Path) -> None:
    """The 0cf20bc43 shape: the earlier LOSING measurement has the later mtime."""
    _write(directory, "a_loss.json",
           _document("2026-08-27T17:00:49+00:00", 0.919, "838d27886af9" + "0" * 52),
           mtime=1_788_558_533)  # 2026-09-04: committed late
    _write(directory, "b_win.json",
           _document("2026-08-27T17:26:59+00:00", 1.162, "360b7981d9fe" + "0" * 52),
           mtime=1_787_851_653)  # 2026-08-27


def test_scorecard_selects_the_later_measurement_not_the_later_mtime() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        _late_committed_loss(Path(tmp))
        result = scan(Path(tmp))
    record = result["certified"][LANE]
    assert record["ratio"] == 1.162, record
    assert record["file"] == "b_win.json", record
    assert result["unstamped"] == [], result["unstamped"]


def test_census_reports_the_later_measurement_and_its_date() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        _late_committed_loss(Path(tmp))
        out = subprocess.run(
            [sys.executable, str(SCRIPTS / "current_loss_census.py"), "--dir", tmp, "--all"],
            capture_output=True, text=True, check=True,
        ).stdout
    assert "still losing (ratio < 1) : 0" in out, out
    lane_line = next(line for line in out.splitlines() if "series_kurtosis" in line)
    assert lane_line.lstrip().startswith("1.162"), lane_line
    # The Measured column is the embedded timestamp's date, not the mtime's.
    assert "2026-08-27" in lane_line and "2026-09-04" not in lane_line, lane_line


def test_equal_mtimes_do_not_leave_the_choice_to_directory_order() -> None:
    # A fresh clone: identical mtimes. The later timestamp must win in either
    # file-name order.
    for loss_name, win_name in (("a.json", "b.json"), ("b.json", "a.json")):
        with tempfile.TemporaryDirectory() as tmp:
            _write(Path(tmp), loss_name, _document("2026-08-27T17:00:49Z", 0.919, "1" * 64), 1_790_000_000)
            _write(Path(tmp), win_name, _document("2026-08-27T17:26:59Z", 1.162, "2" * 64), 1_790_000_000)
            assert scan(Path(tmp))["certified"][LANE]["ratio"] == 1.162


def test_an_unstamped_document_falls_back_to_mtime_and_is_reported() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        _write(Path(tmp), "stamped.json", _document("2026-08-27T17:00:49+00:00", 0.919, "1" * 64), 1_700_000_000)
        _write(Path(tmp), "unstamped.json", _document(None, 1.5, "2" * 64), 1_900_000_000)
        result = scan(Path(tmp))
        path = Path(tmp) / "unstamped.json"
        assert measured_at(json.loads(path.read_text()), str(path)) == (1_900_000_000, False)
    assert result["unstamped"] == ["unstamped.json"], result["unstamped"]
    assert result["certified"][LANE]["ratio"] == 1.5


def test_timestamp_spellings() -> None:
    path = "/nonexistent"  # never read: every document below is stamped
    utc_z, _ = measured_at({"timestamp": "2026-08-27T17:00:00Z"}, path)
    offset, _ = measured_at({"timestamp": "2026-08-27T19:00:00+02:00"}, path)
    assert utc_z == offset
    assert measured_at({"timestamp_utc": "2026-06-21T00:30:00Z"}, path)[1]
    assert measured_at({"created_at": "2026-06-19T03:23:17.875603+00:00"}, path)[1]
    date_only, stamped = measured_at({"date": "2026-08-17"}, path)
    assert stamped and date_only == measured_at({"timestamp": "2026-08-17T00:00:00Z"}, path)[0]


if __name__ == "__main__":
    for name, fn in sorted(globals().items()):
        if name.startswith("test_") and callable(fn):
            fn()
            print("ok", name)
