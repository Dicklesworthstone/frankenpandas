#!/usr/bin/env python3
"""Regression test for br-frankenpandas-ooivn.

The defect: `vs_pandas_harness.py` discarded `--output` when ANY part of a run
was rejected by the host-wide exclusivity gate, INCLUDING rejections that
happened after a row had already been measured with every one of its own
quiescence phases clear. Two df_transpose @100k rows were lost that way.

This test simulates that exact shape: force a rejection at
`invocation_postflight` only, leaving every other phase under the real gate, and
assert that

  1. the artifact still exists,
  2. it contains the gate-clean row, annotated with its invocation_id,
  3. the rejection is recorded in `invocation_rejection`, and
  4. the process STILL fails closed with exit 2.

Point 4 is the important one: this fix must never make a rejected invocation
look successful. It only stops evidence the gate already blessed from being
thrown away.

Requires a built `target/release-perf/fp-bench` and a quiet host — the row is
measured for real, so the genuine gate must admit it. On a contended host the
run is refused before the row completes; that is the gate working, not a
failure of this fix, so the test reports SKIPPED rather than FAILED in that
case. Re-run when the host is quiet.

Usage:  python3 benches/test_harness_rejection_artifact.py
Exit:   0 = pass or skipped-for-load, 1 = real failure
"""
from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parent.parent
HARNESS = PROJECT_ROOT / "benches" / "vs_pandas_harness.py"
FP_BENCH = PROJECT_ROOT / "target" / "release-perf" / "fp-bench"
REJECT_PHASE = "invocation_postflight"


def load_harness():
    spec = importlib.util.spec_from_file_location("vs_pandas_harness", HARNESS)
    module = importlib.util.module_from_spec(spec)
    sys.modules["vs_pandas_harness"] = module
    spec.loader.exec_module(module)
    return module


def main() -> int:
    if not FP_BENCH.exists():
        print(f"SKIPPED: no fp-bench at {FP_BENCH}")
        print("  build with: cargo build --profile release-perf -p fp-bench")
        return 0

    vsh = load_harness()
    real_require_quiet = vsh.HostWideExclusivityGate.require_quiet
    real_wait_until_quiet = vsh.HostWideExclusivityGate.wait_until_quiet
    # Track how far the run got. A refusal BEFORE any measurement begins
    # (invocation_preflight / post_provenance) legitimately produces no
    # artifact — nothing was blessed, so there is nothing to bank. That is the
    # gate working, and must not be confused with the defect under test.
    phases_reached: list[str] = []

    def recording_wait(self, phase):
        phases_reached.append(phase)
        return real_wait_until_quiet(self, phase)

    vsh.HostWideExclusivityGate.wait_until_quiet = recording_wait

    def only_reject_postflight(self, phase):
        phases_reached.append(phase)
        if phase == REJECT_PHASE:
            self.last_rejection = {
                "phase": phase,
                "kind": "adjudicating_checkpoint_not_clear",
                "missing_cpu_ids": [],
                "busy_cpu_ids_above_limit": [22],
                "maximum_busy_fraction": vsh.MAX_HOST_WIDE_BUSY_FRACTION,
            }
            raise SystemExit(2)
        return real_require_quiet(self, phase)

    vsh.HostWideExclusivityGate.require_quiet = only_reject_postflight

    with tempfile.TemporaryDirectory() as tmp:
        out = Path(tmp) / "artifact.json"
        sys.argv = [
            "vs_pandas_harness.py",
            "--category", "strings",
            "--workloads", "str_startswith_arrow",
            "--sizes", "1M",
            "--frankenpandas-binary", str(FP_BENCH),
            "--output", str(out),
        ]
        exit_code = 0
        try:
            vsh.main()
        except SystemExit as exc:
            exit_code = exc.code if isinstance(exc.code, int) else 1

        if exit_code != 2:
            print(f"FAILED: expected fail-closed exit 2, got {exit_code}")
            return 1
        if not out.exists():
            measured = any("measurement" in p for p in phases_reached)
            if not measured:
                print(
                    "SKIPPED: host too busy — refused at "
                    f"phase={phases_reached[-1] if phases_reached else '?'} "
                    "before any row was measured, so there was nothing to "
                    "bank. Re-run on a quiet host."
                )
                return 0
            print("FAILED: artifact missing — rejected run discarded evidence")
            return 1

        artifact = json.loads(out.read_text())
        rejection = artifact.get("invocation_rejection")
        rows = artifact.get("results", [])

        if rejection is None:
            print("FAILED: rejection not recorded in artifact")
            return 1

        if not rows:
            # The real gate refused the row before it finished. Nothing was
            # blessed, so there is nothing to bank — correct behaviour, but it
            # does not exercise the case this test exists for.
            print(
                "SKIPPED: host too busy — the row itself was rejected at "
                f"phase={rejection.get('phase')}; artifact was still written "
                "with the rejection recorded. Re-run on a quiet host."
            )
            return 0

        if rejection.get("phase") != REJECT_PHASE:
            print(f"SKIPPED: rejected earlier than {REJECT_PHASE}: {rejection}")
            return 0

        row = rows[0]
        if row.get("verdict") not in ("FASTER", "SLOWER"):
            print(f"FAILED: banked row is not decidable: {row.get('verdict')}")
            return 1
        if row.get("invocation_id") is None:
            print("FAILED: banked row lost its invocation_id annotation")
            return 1

        print(
            f"PASSED: exit={exit_code}, {len(rows)} gate-clean row(s) banked "
            f"(verdict={row['verdict']} ratio={row.get('ratio')}), "
            f"rejection recorded at phase={rejection['phase']} "
            f"busy={rejection.get('busy_cpu_ids_above_limit')}"
        )
        return 0


if __name__ == "__main__":
    sys.exit(main())
