#!/usr/bin/env python3
"""
Fixture-vs-Live-Pandas Differ

Loads each fixture packet, runs the operation through live pandas oracle,
and compares the result with the pinned expected value.

Output: reports/fixture_defect_set.json

Bead: br-frankenpandas-rg8ys.1.2
"""

from __future__ import annotations

import argparse
import json
import math
import os
import sys
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


@dataclass
class DiffResult:
    packet_id: str
    operation: str
    live_derivable: bool
    matches_pinned: bool
    divergence: str = ""
    live_error: str = ""
    pinned_value: Any = None
    live_value: Any = None


@dataclass
class DifferReport:
    timestamp: str = field(default_factory=lambda: datetime.now(timezone.utc).isoformat())
    total_packets: int = 0
    live_derivable: int = 0
    matches_pinned: int = 0
    fixture_defects: list[str] = field(default_factory=list)
    replay_only: list[str] = field(default_factory=list)
    results: list[dict] = field(default_factory=list)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Fixture-vs-Live-Pandas Differ")
    parser.add_argument(
        "--fixtures-dir",
        type=Path,
        default=Path("crates/fp-conformance/fixtures/packets"),
        help="Directory containing fixture packets",
    )
    parser.add_argument(
        "--legacy-root",
        type=Path,
        default=Path("/dp/frankenpandas/legacy_pandas_code/pandas"),
        help="Path to legacy pandas root",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=Path("reports/fixture_defect_set.json"),
        help="Output JSON file",
    )
    parser.add_argument(
        "--verbose",
        action="store_true",
        help="Verbose output",
    )
    return parser.parse_args()


def scalar_equal(a: Any, b: Any) -> bool:
    if a is None and b is None:
        return True
    if a is None or b is None:
        return False
    if isinstance(a, dict) and isinstance(b, dict):
        return dict_equal(a, b)
    if isinstance(a, float) and isinstance(b, float):
        if math.isnan(a) and math.isnan(b):
            return True
        if math.isinf(a) and math.isinf(b):
            return a > 0 == b > 0
        return abs(a - b) < 1e-9
    return a == b


# "str"/"string" are serde ALIASES for the canonical "utf8" string kind (see
# fp-types Scalar: #[serde(alias = "string", alias = "str")], and the matching
# comment in pandas_oracle.scalar_from_json). Comparing the kind literally made
# every {"kind": "str"} fixture read as a divergence from a live
# {"kind": "utf8"} carrying the SAME value — ~25 spurious rows across the
# string-op packets, none of them real. Normalize before comparing.
KIND_ALIASES = {"str": "utf8", "string": "utf8"}


def normalize_kind(value: dict) -> dict:
    kind = value.get("kind")
    if kind in KIND_ALIASES:
        normalized = dict(value)
        normalized["kind"] = KIND_ALIASES[kind]
        return normalized
    return value


def dict_equal(a: dict, b: dict) -> bool:
    a = normalize_kind(a)
    b = normalize_kind(b)
    if set(a.keys()) != set(b.keys()):
        return False
    for key in a:
        if not scalar_equal(a[key], b[key]):
            return False
    return True


def frame_column_order(frame: dict) -> tuple[list, bool]:
    """Return (column names, whether the frame RECORDED an explicit order).

    Most fixtures store `expected_frame` as {"index": [...], "columns": {...}}
    with no `column_order` key at all. The previous code did
    `pinned.get("column_order", [])` and compared that empty default against the
    live oracle's populated list, so every such fixture reported a
    `column_order mismatch: [] vs [...]` — 269 of the 420 flagged rows, none of
    them real.

    Worse, the value-comparison loop below iterates `pinned_cols`, so when that
    default was empty it compared NO VALUES AT ALL. A fixture whose order
    happened to match would have been declared equal without a single value
    being checked. Deriving the names from the `columns` mapping fixes both the
    false positive and the silent no-op.
    """
    order = frame.get("column_order")
    if isinstance(order, list) and order:
        return list(order), True
    columns = frame.get("columns")
    if isinstance(columns, dict):
        return list(columns.keys()), False
    return [], False


def series_equal(pinned: dict, live: dict) -> tuple[bool, str]:
    pinned_index = pinned.get("index", [])
    live_index = live.get("index", [])
    pinned_values = pinned.get("values", [])
    live_values = live.get("values", [])

    if len(pinned_index) != len(live_index):
        return False, f"index length mismatch: {len(pinned_index)} vs {len(live_index)}"
    if len(pinned_values) != len(live_values):
        return False, f"values length mismatch: {len(pinned_values)} vs {len(live_values)}"

    for i, (pi, li) in enumerate(zip(pinned_index, live_index)):
        if not dict_equal(pi, li):
            return False, f"index[{i}] mismatch: {pi} vs {li}"

    for i, (pv, lv) in enumerate(zip(pinned_values, live_values)):
        if not dict_equal(pv, lv):
            return False, f"values[{i}] mismatch: {pv} vs {lv}"

    return True, ""


def frame_equal(pinned: dict, live: dict) -> tuple[bool, str]:
    pinned_cols, pinned_explicit = frame_column_order(pinned)
    live_cols, live_explicit = frame_column_order(live)

    # Order is only a divergence when BOTH sides actually recorded an order.
    # If either side's order was derived from a `columns` mapping, compare the
    # column SET instead — a derived order carries no ordering claim, and
    # asserting one manufactures a mismatch out of nothing.
    if pinned_explicit and live_explicit:
        if pinned_cols != live_cols:
            return False, f"column_order mismatch: {pinned_cols} vs {live_cols}"
    elif set(pinned_cols) != set(live_cols):
        return (
            False,
            f"column set mismatch: {sorted(pinned_cols)} vs {sorted(live_cols)}",
        )

    pinned_index = pinned.get("index", [])
    live_index = live.get("index", [])

    if len(pinned_index) != len(live_index):
        return False, f"index length mismatch: {len(pinned_index)} vs {len(live_index)}"

    for i, (pi, li) in enumerate(zip(pinned_index, live_index)):
        if not dict_equal(pi, li):
            return False, f"index[{i}] mismatch: {pi} vs {li}"

    pinned_columns = pinned.get("columns", {})
    live_columns = live.get("columns", {})

    for col in pinned_cols:
        if col not in live_columns:
            return False, f"column '{col}' missing in live"
        pinned_col = pinned_columns.get(col, [])
        live_col = live_columns.get(col, [])
        if len(pinned_col) != len(live_col):
            return False, f"column '{col}' length mismatch"
        for i, (pv, lv) in enumerate(zip(pinned_col, live_col)):
            if not dict_equal(pv, lv):
                return False, f"column '{col}'[{i}] mismatch: {pv} vs {lv}"

    return True, ""


def compare_expected(pinned: dict, live: dict) -> tuple[bool, str]:
    for key in ["expected_series", "expected_frame", "expected_scalar", "expected_bool"]:
        if pinned.get(key) is not None or live.get(key) is not None:
            pval = pinned.get(key)
            lval = live.get(key)

            if pval is None and lval is not None:
                return False, f"{key}: pinned is None but live is not"
            if pval is not None and lval is None:
                return False, f"{key}: live is None but pinned is not"

            if key == "expected_series":
                return series_equal(pval, lval)
            elif key == "expected_frame":
                return frame_equal(pval, lval)
            elif key == "expected_scalar":
                if not dict_equal(pval, lval):
                    return False, f"{key} mismatch: {pval} vs {lval}"
                return True, ""
            elif key == "expected_bool":
                if pval != lval:
                    return False, f"{key} mismatch: {pval} vs {lval}"
                return True, ""

    return True, ""


def run_live_oracle(packet: dict, legacy_root: Path, oracle_path: Path) -> dict | None:
    import subprocess
    import tempfile

    operation = packet.get("operation", "")

    payload = {
        "operation": operation,
    }

    if "frame" in packet:
        payload["frame"] = packet["frame"]
    if "frame_right" in packet:
        payload["frame_right"] = packet["frame_right"]
    if "left" in packet:
        payload["left"] = packet["left"]
    if "right" in packet:
        payload["right"] = packet["right"]

    for key in packet:
        if key not in ("packet_id", "case_id", "mode", "operation", "fixture_provenance",
                       "expected_series", "expected_frame", "expected_scalar", "expected_bool",
                       "expected_join", "expected_alignment", "expected_positions", "expected_dtype"):
            payload[key] = packet[key]

    with tempfile.NamedTemporaryFile(mode="w", suffix=".json", delete=False) as tmp:
        json.dump(payload, tmp)
        tmp_path = tmp.name

    try:
        # PRE-EXISTING, fixed in passing (br-frankenpandas-live-oracle-passes-
        # by-skip-l7r1p): `stdin=open(tmp_path)` leaked one file descriptor per
        # oracle invocation. That is 1258 leaked handles in a full corpus run —
        # squarely inside the default 1024 soft limit, so a whole-corpus sweep
        # can die of fd exhaustion partway through and report a truncated
        # comparison as if it were complete.
        with open(tmp_path) as stdin_handle:
            result = subprocess.run(
                [
                    sys.executable,
                    str(oracle_path),
                    "--legacy-root", str(legacy_root),
                    "--allow-system-pandas-fallback",
                ],
                stdin=stdin_handle,
                capture_output=True,
                timeout=30,
            )

        if result.returncode != 0:
            return None

        return json.loads(result.stdout)
    except Exception:
        return None
    finally:
        os.unlink(tmp_path)


def diff_packet(packet: dict, legacy_root: Path, oracle_path: Path) -> DiffResult:
    packet_id = packet.get("packet_id", "unknown")
    operation = packet.get("operation", "unknown")

    live_result = run_live_oracle(packet, legacy_root, oracle_path)

    if live_result is None:
        return DiffResult(
            packet_id=packet_id,
            operation=operation,
            live_derivable=False,
            matches_pinned=False,
            divergence="",
            live_error="Failed to run live oracle",
        )

    if live_result.get("error"):
        return DiffResult(
            packet_id=packet_id,
            operation=operation,
            live_derivable=False,
            matches_pinned=False,
            divergence="",
            live_error=live_result.get("error", "Unknown error"),
        )

    matches, divergence = compare_expected(packet, live_result)

    return DiffResult(
        packet_id=packet_id,
        operation=operation,
        live_derivable=True,
        matches_pinned=matches,
        divergence=divergence,
        pinned_value=packet.get("expected_series") or packet.get("expected_frame") or packet.get("expected_scalar"),
        live_value=live_result.get("expected_series") or live_result.get("expected_frame") or live_result.get("expected_scalar"),
    )


def main() -> int:
    args = parse_args()

    script_dir = Path(__file__).parent
    project_root = script_dir.parent.parent.parent

    fixtures_dir = project_root / args.fixtures_dir if not args.fixtures_dir.is_absolute() else args.fixtures_dir
    output_path = project_root / args.output if not args.output.is_absolute() else args.output
    oracle_path = script_dir / "pandas_oracle.py"

    if not fixtures_dir.is_dir():
        print(f"Fixtures directory not found: {fixtures_dir}", file=sys.stderr)
        return 1

    if not oracle_path.is_file():
        print(f"Oracle script not found: {oracle_path}", file=sys.stderr)
        return 1

    packets = list(fixtures_dir.glob("*.json"))
    print(f"Found {len(packets)} fixture packets")

    report = DifferReport()
    report.total_packets = len(packets)

    for packet_path in packets:
        try:
            with open(packet_path) as f:
                packet = json.load(f)
        except Exception as e:
            print(f"Failed to load {packet_path}: {e}", file=sys.stderr)
            continue

        result = diff_packet(packet, args.legacy_root, oracle_path)

        if result.live_derivable:
            report.live_derivable += 1
            if result.matches_pinned:
                report.matches_pinned += 1
            else:
                report.fixture_defects.append(result.packet_id)
                if args.verbose:
                    print(f"FIXTURE DEFECT: {result.packet_id} - {result.divergence}")
        else:
            report.replay_only.append(result.packet_id)
            if args.verbose:
                print(f"REPLAY-ONLY: {result.packet_id} - {result.live_error}")

        report.results.append({
            "packet_id": result.packet_id,
            "operation": result.operation,
            "live_derivable": result.live_derivable,
            "matches_pinned": result.matches_pinned,
            "divergence": result.divergence,
            "live_error": result.live_error,
        })

    output_path.parent.mkdir(parents=True, exist_ok=True)
    with open(output_path, "w") as f:
        json.dump({
            "timestamp": report.timestamp,
            "total_packets": report.total_packets,
            "live_derivable": report.live_derivable,
            "matches_pinned": report.matches_pinned,
            "fixture_defects": sorted(report.fixture_defects),
            "replay_only": sorted(report.replay_only),
            "results": report.results,
        }, f, indent=2)

    print(f"\n=== Fixture Differ Report ===")
    print(f"Total packets: {report.total_packets}")
    print(f"Live-derivable: {report.live_derivable} ({100*report.live_derivable//max(1,report.total_packets)}%)")
    print(f"Matches pinned: {report.matches_pinned}")
    print(f"Fixture defects: {len(report.fixture_defects)}")
    print(f"Replay-only: {len(report.replay_only)}")
    print(f"Output: {output_path}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
