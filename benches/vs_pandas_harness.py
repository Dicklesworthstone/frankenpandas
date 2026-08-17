#!/usr/bin/env python3
"""vs-pandas head-to-head timing harness.

Runs identical workloads on both FrankenPandas (Rust, release-perf) and
pandas 2.2.3, capturing p50/p95/p99 + cv_pct + throughput per engine.

Per BENCH_MATRIX_SPEC.md:
- Uses release-perf profile for FP (not --release)
- Emits executable SHA-256 provenance for both engines
- Measures an interleaved A/A null control inside each engine invocation
- Gates claims on an effect-median bootstrap CI, a 2x null-CI margin, and
  A/A medians within 2% of unity; never on cv
- Population/setup OUTSIDE the timed window
- EngineIdentity Subject!=Oracle on every artifact

Usage:
    python benches/vs_pandas_harness.py --category io --sizes 100k
    python benches/vs_pandas_harness.py --all --sizes 10k,100k,1M
    taskset -c 0-63 python benches/vs_pandas_harness.py \
        --category groupby --workloads groupby_mean_float64 \
        --sizes 1M,10M --thread-count 64
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.metadata
import inspect
import json
import math
import os
import platform
import shutil
import re
import socket
import subprocess
import sys
import threading
import time
from collections.abc import Callable
from dataclasses import dataclass, field
from datetime import datetime, timezone
from functools import cache, partial
from io import StringIO
from json import JSONDecodeError
from pathlib import Path
from statistics import mean, stdev
from typing import Any

import numpy as np

try:
    from pandas.api.types import is_numeric_dtype
except ImportError:
    is_numeric_dtype = None

try:
    import pandas as pd
except ImportError:
    pd = None

try:
    import pyarrow as pa
except ImportError:
    pa = None

PROJECT_ROOT = Path(__file__).parent.parent
RESULTS_DIR = PROJECT_ROOT / "artifacts" / "bench"

CATEGORIES = {
    "io": 0.25,
    "dataframe_ops": 0.20,
    "groupby": 0.20,
    "joins": 0.15,
    "rolling": 0.10,
    "indexing": 0.10,
    "strings": 0.10,
    "linalg": 0.10,
    "datetime": 0.10,
}

# Runnable and fully reported, but deliberately OUTSIDE the weighted kernel
# score above. A six-stage end-to-end job's ratio is not commensurable with a
# geomean over single-op categories, and folding it in would let one whole-job
# number move a headline that means something else. `--all` is unchanged, so
# every existing baseline stays comparable; ask for it by name.
EXTRA_CATEGORIES = {
    "pipeline": "whole-job ETL: load/filter/groupby/join/sort/write",
    "math_unary": "floor/ceil/trunc/round/sqrt/log -- the ISA-blocked ledger family",
}

SIZE_CONFIGS = {
    # SUB-10k LANES. br-frankenpandas-kko5z, CrimsonPine.
    #
    # The corpus had no size below 10k, so no lane could separate a FIXED
    # per-call cost from a per-element one — and that is exactly the open
    # question on `sqrt`, whose ratio climbs 0.518x -> 0.709x -> 0.869x ->
    # 0.895x from 10k to 10M. A four-size fit says FrankenPandas pays
    # `66.5us + 1.204 ns/element` against pandas' `30.4us + 1.079`, but 10k is
    # the smallest point available and the constant is still only 85% of the
    # arm there. At 1k a 66.5us constant would be ~98% of the runtime and at
    # 100 rows ~99%, so these two lanes turn a fitted intercept into a directly
    # observed one: if the constant is real, `sqrt @100` and `sqrt @1k` cost
    # essentially the same as each other, and `floor` at the same sizes does
    # not. Nothing else in the harness changes — these are opt-in via `--sizes`
    # and `--all` does not reach them, so every existing baseline stays
    # comparable.
    "100": {"rows": 100, "cols": 10},
    "1k": {"rows": 1_000, "cols": 10},
    "10k": {"rows": 10_000, "cols": 10},
    "100k": {"rows": 100_000, "cols": 10},
    "1M": {"rows": 1_000_000, "cols": 10},
    "2M": {"rows": 2_000_000, "cols": 10},
    "4M": {"rows": 4_000_000, "cols": 10},
    "6M": {"rows": 6_000_000, "cols": 10},
    "8M": {"rows": 8_000_000, "cols": 10},
    "10M": {"rows": 10_000_000, "cols": 10},
}

PAIRED_ROUNDS = 25
# A complete balanced square gives each arm every early/late position twice.
# Unlike the legacy host-wide gate, this is sound on a shared host: co-tenant
# load and thermal/frequency drift are paired within every round instead of
# being treated as an unsatisfiable admission predicate.
BALANCED_SQUARE = "ABBAABBA"
BALANCED_SQUARE_ROUNDS = 9
BOOTSTRAP_RESAMPLES = 10_000

# Adaptive round scaling for FAST workloads (CrimsonPine, br-frankenpandas-4kig1).
#
# MEASURED over 226 rows in artifacts/bench/: the A/A null gate is far harder for a
# short operation than a long one, at the SAME 2% limit.
#
#     incumbent p50   rows   median worst-arm |dev|   both-arms pass @2%
#     0-200us           54            2.27%                  40.7%
#     200us-1ms         21            3.45%                  33.3%
#     1-5ms             24            1.78%                  50.0%
#     >5ms             127            1.61%                  54.3%
#
# Nine ABBAABBA rounds of a 186us call is under 7ms of actual timed work, so
# per-call overhead and scheduler jitter are a large fraction of it. That bias
# points the wrong way for this campaign: FrankenPandas' WORST ratios are the fast
# ops (floor/ceil/trunc @1M, incumbent ~180us, in the 40.7% bucket), so the gate is
# hardest exactly where the losses are biggest and the rows we most need are the
# ones most often discarded. Six x86-64-v3 floor rows across two windows reproduced
# a 2.8-3.2x effect and not one of them certified.
#
# THE FIX IS NOT TO LOOSEN THE LIMIT. Raising 2% to 3% would lift both-arm pass from
# 48.7% to 65.5%, and that is gate self-weakening -- docs/NEGATIVE_EVIDENCE.md
# records a 2.7x phantom that a clean null was the only thing standing against. Null
# deviation is a SAMPLING property and shrinks with samples, so the measurement is
# scaled to the op instead of the threshold to the failures.
# MEASURED relative between-slot dispersion by incumbent p50, over 463 arm-rows
# carrying per-slot `samples_us` in artifacts/bench/. This REPLACED a duration-based
# rule: I first justified adaptive rounds by "a slot is short for a fast op", which
# is false (fp-bench does WARMUP=3 + ITERS=25 => 50 timed calls per slot at EVERY
# size), and then by fixed per-slot cost, which the data also refutes -- absolute
# spread scales 222x across a 170x p50 range rather than staying flat. The noise is
# MULTIPLICATIVE, and the only surviving fact is that the fastest ops sit at roughly
# twice the relative dispersion of the rest.
ADAPTIVE_DISPERSION_BY_P50_US: tuple[tuple[float, float], ...] = (
    (300.0, 0.1127),
    (1_000.0, 0.0781),
    (3_000.0, 0.0863),
    (20_000.0, 0.0607),
    (float("inf"), 0.0820),
)
# The quietest bucket, used as the yardstick every other bucket is brought up to.
ADAPTIVE_REFERENCE_DISPERSION = 0.0607
ADAPTIVE_MAX_ROUNDS = 120


def adaptive_dispersion_for_p50(p50_us: float) -> float:
    """Measured relative between-slot dispersion for an op of this duration."""
    for upper, dispersion in ADAPTIVE_DISPERSION_BY_P50_US:
        if p50_us < upper:
            return dispersion
    return ADAPTIVE_DISPERSION_BY_P50_US[-1][1]


def rounds_after_first_round(
    incumbent_slot_p50s: list[float],
    current_rounds: int,
    *,
    enabled: bool = True,
) -> int:
    """Total rounds to run, decided from the FIRST round's incumbent slots.

    Called once, at the end of round 0, so no separate pilot slot is paid and the
    decision uses real measured slots rather than a guess. The INCUMBENT arm is the
    yardstick because `ADAPTIVE_DISPERSION_BY_P50_US` is bucketed by incumbent p50.

    Returns `current_rounds` unchanged when disabled or when round 0 produced
    nothing usable, so the caller's loop bound only ever grows.
    """
    if not enabled or not incumbent_slot_p50s:
        return current_rounds
    usable = [p for p in incumbent_slot_p50s if isinstance(p, (int, float)) and p > 0]
    if not usable:
        return current_rounds
    observed = sorted(usable)[len(usable) // 2]
    return max(current_rounds, adaptive_balanced_square_rounds(observed))


def adaptive_balanced_square_rounds(
    observed_p50_us: float,
    *,
    base_rounds: int = BALANCED_SQUARE_ROUNDS,
    reference_dispersion: float = ADAPTIVE_REFERENCE_DISPERSION,
    max_rounds: int = ADAPTIVE_MAX_ROUNDS,
) -> int:
    """Rounds that give this op the same null-median precision as a quiet one.

    The A/A null is a median over per-round ratios, so its standard error goes as
    `cv / sqrt(rounds)`. Equalising that across ops means `rounds` proportional to
    `cv**2`, and the `cv` per duration bucket is measured, not assumed.

    ⚠ THE INVARIANT THAT MAKES THIS A STRENGTHENING AND NOT A GATE CHANGE: the result
    is NEVER below `base_rounds`, for any input including 0, negative, NaN and inf.
    It can only ADD samples. The 2% null limit is untouched -- loosening it to 3%
    would lift both-arm pass from 48.7% to 65.5% and that is gate self-weakening,
    which docs/NEGATIVE_EVIDENCE.md records a 2.7x phantom being caught by.
    """
    if not math.isfinite(observed_p50_us) or observed_p50_us <= 0.0:
        return base_rounds
    if not math.isfinite(reference_dispersion) or reference_dispersion <= 0.0:
        return base_rounds
    ratio = adaptive_dispersion_for_p50(observed_p50_us) / reference_dispersion
    needed = math.ceil(base_rounds * ratio * ratio)
    return max(base_rounds, min(max_rounds, needed))


NULL_CI_CONFIDENCE = 0.95
DECIDABILITY_MARGIN = 2.0
NULL_MEDIAN_MAX_ABS_DEVIATION = 0.02
WARMUP_ITERATIONS = 3
# One second distinguishes sustained host tenancy from ordinary sub-100 ms
# scheduler/kernel bursts on the 128-thread measurement host. The per-CPU 20%
# ceiling still rejects a competing build, benchmark, or repository scan.
CPU_SAMPLE_INTERVAL_SECONDS = 1.0
MAX_HOST_WIDE_BUSY_FRACTION = 0.20
QUIESCENCE_WAIT_MAX_ATTEMPTS = 20
QUIESCENCE_WAIT_RETRY_SECONDS = 0.5
SETUP_QUIESCENCE_SETTLE_SECONDS = 1.0
PROVENANCE_QUIESCENCE_SETTLE_SECONDS = 5.0
TAKE_BATCH = 256
TRANSPOSE_BATCH = 8192
TELEMETRY_STRING_BATCH_ROWS = 250_000
TELEMETRY_MIMALLOC_PURGE_DELAY_MS = "0"


@dataclass
class PairedSamples:
    """One engine's timings plus its same-invocation A/A null control."""
    times_us: list[float] = field(default_factory=list)
    null_arm_a_us: list[float] = field(default_factory=list)
    null_arm_b_us: list[float] = field(default_factory=list)
    null_ratios: list[float] = field(default_factory=list)
    checksum: int = 0
    runtime_available_parallelism: int = 1
    process_threads_before_probe: int = 1
    peak_process_threads: int = 1
    operation_threads_used: int = 1


def _read_text(path: Path) -> str | None:
    try:
        return path.read_text().strip()
    except OSError:
        return None


def _online_cpu_ids() -> list[int]:
    cpu_ids = []
    for cpu_dir in Path("/sys/devices/system/cpu").glob("cpu[0-9]*"):
        try:
            cpu_id = int(cpu_dir.name[3:])
        except ValueError:
            continue
        online = _read_text(cpu_dir / "online")
        if online in (None, "1"):
            cpu_ids.append(cpu_id)
    return sorted(cpu_ids)


def _read_host_cpu_ticks() -> dict[int, tuple[int, int]]:
    """Read total and idle scheduler ticks for every online host CPU."""
    stat = _read_text(Path("/proc/stat"))
    if stat is None:
        raise RuntimeError("host-wide exclusivity requires readable /proc/stat")

    ticks = {}
    for line in stat.splitlines():
        fields = line.split()
        if not fields:
            continue
        label = fields[0]
        if not label.startswith("cpu") or not label[3:].isdigit():
            continue
        try:
            values = [int(value) for value in fields[1:]]
        except ValueError as error:
            raise RuntimeError(f"invalid /proc/stat row for {label}") from error
        if len(values) < 5:
            raise RuntimeError(f"/proc/stat row for {label} is too short")
        total = sum(values)
        idle = values[3] + values[4]
        ticks[int(label[3:])] = (total, idle)
    if not ticks:
        raise RuntimeError("host-wide exclusivity found no CPU rows in /proc/stat")
    return ticks


def _sample_host_cpu_busy() -> dict[int, float]:
    """Sample per-CPU busy fractions over the fleet-standard 300 ms window."""
    before = _read_host_cpu_ticks()
    time.sleep(CPU_SAMPLE_INTERVAL_SECONDS)
    after = _read_host_cpu_ticks()
    busy = {}
    for cpu_id, (total_before, idle_before) in before.items():
        if cpu_id not in after:
            continue
        total_after, idle_after = after[cpu_id]
        total_delta = max(0, total_after - total_before)
        idle_delta = max(0, idle_after - idle_before)
        busy[cpu_id] = (
            1.0
            if total_delta == 0
            else max(0, total_delta - idle_delta) / total_delta
        )
    return busy


def _host_wide_quiescence_observation(
    phase: str,
    expected_cpu_ids: list[int],
    busy_fractions: dict[int, float],
) -> dict[str, Any]:
    """Adjudicate one all-online-CPU quiescence sample."""
    missing_cpu_ids = [
        cpu_id for cpu_id in expected_cpu_ids if cpu_id not in busy_fractions
    ]
    busy_cpu_ids = [
        cpu_id
        for cpu_id in expected_cpu_ids
        if busy_fractions.get(cpu_id, 1.0) > MAX_HOST_WIDE_BUSY_FRACTION
    ]
    observed = {
        f"cpu{cpu_id}": busy_fractions[cpu_id]
        for cpu_id in expected_cpu_ids
        if cpu_id in busy_fractions
    }
    return {
        "phase": phase,
        "scope": "all_online_host_cpus",
        "sample_interval_ms": round(CPU_SAMPLE_INTERVAL_SECONDS * 1000),
        "maximum_busy_fraction": MAX_HOST_WIDE_BUSY_FRACTION,
        "expected_cpu_count": len(expected_cpu_ids),
        "sampled_cpu_count": len(observed),
        "missing_cpu_ids": missing_cpu_ids,
        "busy_cpu_ids_above_limit": busy_cpu_ids,
        "busy_cpu_count_above_limit": len(busy_cpu_ids),
        "maximum_observed_busy_fraction": max(observed.values(), default=1.0),
        "sampled_cpu_busy_fraction": observed,
        "verdict": (
            "clear"
            if expected_cpu_ids and not missing_cpu_ids and not busy_cpu_ids
            else "blocked"
        ),
    }


@dataclass
class HostWideExclusivityGate:
    """Fail closed unless every online host CPU is quiet before each arm."""
    expected_cpu_ids: list[int]
    observations: list[dict[str, Any]] = field(default_factory=list)
    # br-frankenpandas-ooivn: the reason for the most recent fail-closed exit.
    # Recorded so the caller can BANK rows the gate already blessed before it
    # propagates the failure. This does not soften anything: every rejection
    # still raises SystemExit(2) exactly as before, and the recorded detail is
    # written into the artifact so a rejected invocation is self-describing
    # rather than silently absent.
    last_rejection: dict[str, Any] | None = None

    def _sample(self, phase: str, role: str) -> dict[str, Any]:
        try:
            busy_fractions = _sample_host_cpu_busy()
        except RuntimeError as error:
            print(
                f"ERROR: host-wide benchmark exclusivity: {error}",
                file=sys.stderr,
            )
            raise SystemExit(2) from error
        observation = _host_wide_quiescence_observation(
            phase,
            self.expected_cpu_ids,
            busy_fractions,
        )
        observation["role"] = role
        self.observations.append(observation)
        return observation

    def require_quiet(self, phase: str) -> dict[str, Any]:
        observation = self._sample(phase, "adjudicating_checkpoint")
        if observation["verdict"] != "clear":
            print(
                "ERROR: host-wide benchmark exclusivity requires every online "
                "CPU to remain at or below "
                f"{MAX_HOST_WIDE_BUSY_FRACTION * 100:.1f}% busy; "
                f"phase={phase} missing={observation['missing_cpu_ids']} "
                f"busy={observation['busy_cpu_ids_above_limit']}",
                file=sys.stderr,
            )
            self.last_rejection = {
                "phase": phase,
                "kind": "adjudicating_checkpoint_not_clear",
                "missing_cpu_ids": observation["missing_cpu_ids"],
                "busy_cpu_ids_above_limit": observation[
                    "busy_cpu_ids_above_limit"
                ],
                "maximum_busy_fraction": MAX_HOST_WIDE_BUSY_FRACTION,
            }
            raise SystemExit(2)
        print(
            "host_wide_quiescence="
            f"phase={phase} "
            f"online_cpu_count={len(self.expected_cpu_ids)} "
            f"maximum_busy_fraction={MAX_HOST_WIDE_BUSY_FRACTION:.3f} "
            "busy_cpu_count_above_limit=0 verdict=clear"
        )
        return observation

    def wait_until_quiet(self, phase: str) -> dict[str, Any]:
        """Wait boundedly for self-induced setup residue, then re-adjudicate.

        Readiness probes are retained in the artifact but do not replace the
        immediate adjudicating checkpoint. A sustained peer workload still
        fails closed after the predeclared attempt budget.
        """
        previous_probe_was_clear = False
        for attempt in range(1, QUIESCENCE_WAIT_MAX_ATTEMPTS + 1):
            readiness = self._sample(
                f"readiness:{phase}:attempt_{attempt}",
                "readiness_probe",
            )
            readiness["readiness_attempt"] = attempt
            probe_is_clear = readiness["verdict"] == "clear"
            if previous_probe_was_clear and probe_is_clear:
                readiness["readiness_phase"] = readiness["phase"]
                readiness["phase"] = phase
                readiness["role"] = "adjudicating_checkpoint"
                print(
                    "host_wide_quiescence="
                    f"phase={phase} "
                    f"online_cpu_count={len(self.expected_cpu_ids)} "
                    f"maximum_busy_fraction="
                    f"{MAX_HOST_WIDE_BUSY_FRACTION:.3f} "
                    "busy_cpu_count_above_limit=0 verdict=clear"
                )
                return readiness
            previous_probe_was_clear = probe_is_clear
            if attempt < QUIESCENCE_WAIT_MAX_ATTEMPTS and not probe_is_clear:
                time.sleep(QUIESCENCE_WAIT_RETRY_SECONDS)

        print(
            "ERROR: host-wide benchmark exclusivity did not reach a clear "
            f"readiness window for phase={phase} after "
            f"{QUIESCENCE_WAIT_MAX_ATTEMPTS} attempts",
            file=sys.stderr,
        )
        self.last_rejection = {
            "phase": phase,
            "kind": "no_clear_readiness_window",
            "attempts": QUIESCENCE_WAIT_MAX_ATTEMPTS,
            "retry_seconds": QUIESCENCE_WAIT_RETRY_SECONDS,
            "maximum_busy_fraction": MAX_HOST_WIDE_BUSY_FRACTION,
        }
        raise SystemExit(2)

    def artifact(self) -> dict[str, Any]:
        adjudicating = [
            observation
            for observation in self.observations
            if observation.get("role") == "adjudicating_checkpoint"
        ]
        return {
            "required": True,
            "scope": "all_online_host_cpus",
            "online_cpu_ids": self.expected_cpu_ids,
            "maximum_busy_fraction": MAX_HOST_WIDE_BUSY_FRACTION,
            "sample_interval_ms": round(CPU_SAMPLE_INTERVAL_SECONDS * 1000),
            "readiness_wait": {
                "maximum_attempts": QUIESCENCE_WAIT_MAX_ATTEMPTS,
                "retry_interval_ms": round(
                    QUIESCENCE_WAIT_RETRY_SECONDS * 1000
                ),
                "readiness_probes_are_adjudicating": False,
                "clear_probe_is_followed_by_immediate_checkpoint": True,
            },
            "observations": self.observations,
            "valid": bool(adjudicating)
            and all(
                observation["verdict"] == "clear"
                for observation in adjudicating
            ),
        }


def _run_host_exclusive_arm(
    exclusivity_gate: HostWideExclusivityGate,
    phase_suffix: str,
    operation: Callable[[], Any],
) -> tuple[Any, dict[str, Any]]:
    """Run one untimed outer arm only when both host-wide guards are clear."""
    pre_quiescence = exclusivity_gate.wait_until_quiet(
        f"pre_measurement:{phase_suffix}"
    )
    result = operation()
    post_quiescence = exclusivity_gate.require_quiet(
        f"post_measurement:{phase_suffix}"
    )
    return (
        result,
        {
            "pre_measurement": pre_quiescence,
            "post_measurement": post_quiescence,
            "valid": True,
        },
    )


def _busiest_host_processes(limit: int = 6) -> list[str]:
    """Top CPU consumers, for attributing a blocked readiness verdict.

    Diagnostic only — nothing adjudicates on it. Returns an empty list when
    `ps` is unavailable rather than failing the probe.
    """
    try:
        completed = subprocess.run(
            ["ps", "-eo", "pcpu,pid,comm", "--sort=-pcpu"],
            capture_output=True,
            text=True,
            timeout=10,
            check=False,
        )
    except (OSError, subprocess.SubprocessError):
        return []
    if completed.returncode != 0:
        return []
    rows = completed.stdout.strip().splitlines()[1 : limit + 1]
    return [" ".join(row.split()) for row in rows]


def _host_readiness_probe(wait_seconds: float) -> int:
    """Report whether THIS host would pass the exclusivity gate right now.

    br-frankenpandas-hostprobe: the gate is fail-closed and correct, but its
    rejection said only "did not reach a clear readiness window", naming
    neither the CPUs that blocked it nor what was running on them. Finding that
    out meant hand-rolling a /proc/stat sampler — after paying for a repo sync,
    a pandas install and a release-perf build, because the gate fires at
    invocation preflight. This mode answers the same question in ~2s, BEFORE
    any of that cost, and it adjudicates with the very same
    `_host_wide_quiescence_observation` the gate uses, so it cannot drift into
    a softer verdict.

    This does NOT weaken anything: it runs no benchmark, banks no row, and
    changes no threshold. Exit 0 means the host would pass; 2 means it would
    not — the same code the gate exits with, so a caller can gate a queued
    measurement on it.

    Delete this when the swarm grows a real bench-booking mechanism that can
    guarantee host exclusivity, at which point "is the host quiet" stops being
    a question an agent has to ask.
    """
    online_cpu_ids = _online_cpu_ids()
    if not online_cpu_ids:
        print(
            "ERROR: host readiness probe could not enumerate online CPUs",
            file=sys.stderr,
        )
        return 2
    deadline = time.monotonic() + max(0.0, wait_seconds)
    attempt = 0
    while True:
        attempt += 1
        observation = _host_wide_quiescence_observation(
            f"host_readiness_probe:attempt_{attempt}",
            online_cpu_ids,
            _sample_host_cpu_busy(),
        )
        clear = observation["verdict"] == "clear"
        print(
            "host_readiness_probe="
            f"verdict={observation['verdict']} "
            f"attempt={attempt} "
            f"online_cpu_count={observation['expected_cpu_count']} "
            f"maximum_busy_fraction={MAX_HOST_WIDE_BUSY_FRACTION:.3f} "
            f"max_observed_busy="
            f"{observation['maximum_observed_busy_fraction']:.3f} "
            f"busy_cpu_count_above_limit="
            f"{observation['busy_cpu_count_above_limit']} "
            f"busy_cpu_ids={observation['busy_cpu_ids_above_limit']} "
            f"missing_cpu_ids={observation['missing_cpu_ids']} "
            f"hostname={socket.gethostname()}"
        )
        if clear or time.monotonic() >= deadline:
            if not clear:
                for row in _busiest_host_processes():
                    print(f"host_readiness_probe_top_process={row}")
            return 0 if clear else 2
        time.sleep(QUIESCENCE_WAIT_RETRY_SECONDS)


def _host_wide_exclusivity_self_test() -> None:
    """Exercise clear, busy, and incomplete adjudication without timing."""
    clear = _host_wide_quiescence_observation(
        "self-test-clear",
        [0, 1],
        {0: 0.0, 1: MAX_HOST_WIDE_BUSY_FRACTION},
    )
    if clear["verdict"] != "clear":
        raise RuntimeError("threshold-boundary sample must be clear")
    busy = _host_wide_quiescence_observation(
        "self-test-busy",
        [0, 1],
        {0: 0.0, 1: MAX_HOST_WIDE_BUSY_FRACTION + 0.001},
    )
    if (
        busy["verdict"] != "blocked"
        or busy["busy_cpu_ids_above_limit"] != [1]
    ):
        raise RuntimeError("over-threshold sample must identify the busy CPU")
    incomplete = _host_wide_quiescence_observation(
        "self-test-incomplete",
        [0, 1],
        {0: 0.0},
    )
    if (
        incomplete["verdict"] != "blocked"
        or incomplete["missing_cpu_ids"] != [1]
    ):
        raise RuntimeError("incomplete sample must identify the missing CPU")

    class RecordingGate:
        def __init__(self) -> None:
            self.phases: list[str] = []

        def require_quiet(self, phase: str) -> dict[str, Any]:
            self.phases.append(phase)
            return {"phase": phase, "verdict": "clear"}

        def wait_until_quiet(self, phase: str) -> dict[str, Any]:
            self.phases.append(f"wait:{phase}")
            return {"phase": phase, "verdict": "clear"}

    gate = RecordingGate()
    result, artifact = _run_host_exclusive_arm(
        gate,
        "frankenpandas:self-test",
        lambda: "completed",
    )
    if result != "completed":
        raise RuntimeError("exclusive arm must return the operation result")
    if gate.phases != [
        "wait:pre_measurement:frankenpandas:self-test",
        "post_measurement:frankenpandas:self-test",
    ]:
        raise RuntimeError("every exclusive arm must be bracketed by two guards")
    if (
        artifact["pre_measurement"]["verdict"] != "clear"
        or artifact["post_measurement"]["verdict"] != "clear"
        or not artifact["valid"]
    ):
        raise RuntimeError("exclusive arm artifact must retain both clear guards")

    # The readiness probe must report the SAME verdict the gate would, and
    # must never report clear on a host the gate would refuse — otherwise it
    # becomes a way to talk yourself past the gate. Both directions are pinned
    # here by substituting the sampler, so the contract is checked without
    # needing a controllable host. (br-frankenpandas-hostprobe)
    real_sampler = _sample_host_cpu_busy
    real_online = _online_cpu_ids
    try:
        globals()["_online_cpu_ids"] = lambda: [0, 1]
        globals()["_sample_host_cpu_busy"] = lambda: {0: 0.0, 1: 0.0}
        if _host_readiness_probe(0.0) != 0:
            raise RuntimeError("a quiet host must probe as ready (exit 0)")
        globals()["_sample_host_cpu_busy"] = lambda: {
            0: 0.0,
            1: MAX_HOST_WIDE_BUSY_FRACTION + 0.001,
        }
        if _host_readiness_probe(0.0) != 2:
            raise RuntimeError("one over-limit CPU must probe as blocked (exit 2)")
        globals()["_sample_host_cpu_busy"] = lambda: {0: 0.0}
        if _host_readiness_probe(0.0) != 2:
            raise RuntimeError("an unsampled CPU must probe as blocked (exit 2)")
    finally:
        globals()["_sample_host_cpu_busy"] = real_sampler
        globals()["_online_cpu_ids"] = real_online

    class SequenceGate(HostWideExclusivityGate):
        def __init__(self) -> None:
            super().__init__([0])
            self.verdicts = iter(["clear", "blocked", "clear", "clear"])

        def _sample(self, phase: str, role: str) -> dict[str, Any]:
            verdict = next(self.verdicts)
            observation = {
                "phase": phase,
                "role": role,
                "verdict": verdict,
            }
            self.observations.append(observation)
            return observation

    sequence_gate = SequenceGate()
    sequence_checkpoint = sequence_gate.wait_until_quiet("sequence-test")
    if (
        sequence_checkpoint["role"] != "adjudicating_checkpoint"
        or len(sequence_gate.observations) != 4
    ):
        raise RuntimeError(
            "a busy confirmation must resume readiness until two clear "
            "samples are consecutive"
        )

    classification_gate = HostWideExclusivityGate([0])
    classification_gate.observations = [
        {"role": "readiness_probe", "verdict": "blocked"},
        {"role": "adjudicating_checkpoint", "verdict": "clear"},
    ]
    if not classification_gate.artifact()["valid"]:
        raise RuntimeError(
            "recorded readiness retries must not invalidate a later clear "
            "adjudicating checkpoint"
        )


def _cpu_flags() -> set[str]:
    cpuinfo = _read_text(Path("/proc/cpuinfo")) or ""
    for line in cpuinfo.splitlines():
        if line.lower().startswith(("flags", "features")) and ":" in line:
            return set(line.split(":", 1)[1].split())
    return set()


def _cpu_model() -> str:
    cpuinfo = _read_text(Path("/proc/cpuinfo")) or ""
    for line in cpuinfo.splitlines():
        if line.lower().startswith(("model name", "hardware")) and ":" in line:
            return line.split(":", 1)[1].strip()
    return platform.processor() or "unknown"


def _git_sha() -> str:
    try:
        result = subprocess.run(
            ["git", "rev-parse", "HEAD"],
            cwd=PROJECT_ROOT,
            capture_output=True,
            check=False,
            text=True,
            timeout=10,
        )
    except (OSError, subprocess.TimeoutExpired):
        return "unavailable"
    return result.stdout.strip() if result.returncode == 0 else "unavailable"


def host_fingerprint() -> dict[str, Any]:
    """Capture host-wide topology plus the process' effective CPU budget."""
    online_cpus = _online_cpu_ids()
    affinity_cpus = (
        sorted(os.sched_getaffinity(0))
        if hasattr(os, "sched_getaffinity")
        else online_cpus
    )
    physical_cores: set[tuple[str, str]] = set()
    for cpu_id in online_cpus:
        topology = Path(f"/sys/devices/system/cpu/cpu{cpu_id}/topology")
        package = _read_text(topology / "physical_package_id")
        core = _read_text(topology / "core_id")
        if package is not None and core is not None:
            physical_cores.add((package, core))

    flags = _cpu_flags()
    tracked_isa = (
        "sse2",
        "avx",
        "avx2",
        "fma",
        "bmi1",
        "bmi2",
        "aes",
        "vaes",
        "avx512f",
    )
    mem_total_kib = 0
    meminfo = _read_text(Path("/proc/meminfo")) or ""
    for line in meminfo.splitlines():
        if line.startswith("MemTotal:"):
            mem_total_kib = int(line.split()[1])
            break

    governors = sorted(
        {
            value
            for cpu_id in affinity_cpus
            if (
                value := _read_text(
                    Path(
                        f"/sys/devices/system/cpu/cpu{cpu_id}/cpufreq/"
                        "scaling_governor"
                    )
                )
            )
        }
    )
    numa_nodes = sorted(
        path.name
        for path in Path("/sys/devices/system/node").glob("node[0-9]*")
    )
    return {
        "host_identity": socket.gethostname(),
        "platform_node": platform.node(),
        "architecture": platform.machine(),
        "cpu_model": _cpu_model(),
        "physical_cores": len(physical_cores),
        "logical_threads": len(online_cpus),
        "threads_per_core": (
            len(online_cpus) // len(physical_cores) if physical_cores else None
        ),
        "ram_bytes": mem_total_kib * 1024,
        "numa_nodes": len(numa_nodes),
        "kernel": platform.release(),
        "git_sha": _git_sha(),
        "cpu_governors": governors,
        "smt_active": _read_text(Path("/sys/devices/system/cpu/smt/active")),
        "frequency_boost": _read_text(
            Path("/sys/devices/system/cpu/cpufreq/boost")
        ),
        "affinity_cpus": affinity_cpus,
        "affinity_logical_cpu_cap": len(affinity_cpus),
        "runtime_detected_isa_features": [
            feature for feature in tracked_isa if feature in flags
        ],
        "runtime_absent_isa_features": [
            feature for feature in tracked_isa if feature not in flags
        ],
    }


def _process_thread_count() -> int:
    status = _read_text(Path("/proc/self/status")) or ""
    for line in status.splitlines():
        if line.startswith("Threads:"):
            return int(line.split(":", 1)[1].strip())
    return 1


def _thread_cpu_ticks() -> dict[int, int]:
    ticks: dict[int, int] = {}
    for task_dir in Path("/proc/self/task").glob("[0-9]*"):
        stat = _read_text(task_dir / "stat")
        if not stat:
            continue
        close_paren = stat.rfind(")")
        if close_paren < 0:
            continue
        fields = stat[close_paren + 1 :].split()
        if len(fields) <= 12:
            continue
        try:
            ticks[int(task_dir.name)] = int(fields[11]) + int(fields[12])
        except ValueError:
            continue
    return ticks


def probe_operation_threads(func) -> dict[str, int]:
    """Observe one untimed call and report active/peak process threads."""
    runtime_available_parallelism = (
        len(os.sched_getaffinity(0))
        if hasattr(os, "sched_getaffinity")
        else (os.cpu_count() or 1)
    )
    process_threads_before_probe = _process_thread_count()
    ticks_before = _thread_cpu_ticks()
    peak_process_threads = process_threads_before_probe
    ready = threading.Event()
    stop = threading.Event()

    def monitor() -> None:
        nonlocal peak_process_threads
        ready.set()
        while not stop.is_set():
            peak_process_threads = max(
                peak_process_threads,
                _process_thread_count(),
            )
            time.sleep(0.000_020)
        peak_process_threads = max(peak_process_threads, _process_thread_count())

    monitor_thread = threading.Thread(
        target=monitor,
        name="fp-bench-thread-probe",
        daemon=True,
    )
    monitor_thread.start()
    ready.wait()
    monitor_native_id = monitor_thread.native_id
    try:
        func()
    finally:
        stop.set()
        monitor_thread.join()

    ticks_after = _thread_cpu_ticks()
    cpu_active_threads = sum(
        ticks > ticks_before.get(tid, 0)
        for tid, ticks in ticks_after.items()
        if tid != monitor_native_id
    )
    newly_spawned_workers = max(
        0,
        peak_process_threads - process_threads_before_probe - 1,
    )
    return {
        "runtime_available_parallelism": runtime_available_parallelism,
        "process_threads_before_probe": process_threads_before_probe,
        "peak_process_threads": peak_process_threads,
        "operation_threads_used": max(
            1,
            cpu_active_threads,
            newly_spawned_workers,
        ),
    }


@cache
def executable_identity(path: Path) -> dict[str, Any]:
    """Hash the executable that actually hosts this engine."""
    resolved = path.resolve(strict=True)
    digest = hashlib.sha256()
    byte_count = 0
    with resolved.open("rb") as executable:
        while chunk := executable.read(1024 * 1024):
            digest.update(chunk)
            byte_count += len(chunk)
    return {
        "sha256": digest.hexdigest(),
        "bytes": byte_count,
        "path": str(resolved),
    }


def distribution_artifact_identity(
    distribution_name: str,
    module_file: str,
) -> dict[str, Any]:
    """Hash one installed distribution that participates in an engine arm.

    Hashing only ``sys.executable`` identifies the Python host, not the legacy
    incumbent or its optional storage backend. The distribution file list
    comes from the installed wheel's metadata; folding each relative path,
    byte length, and file body produces one deterministic identity for the
    actual package loaded here.
    """
    distribution = importlib.metadata.distribution(distribution_name)
    package_files = sorted(distribution.files or (), key=lambda item: str(item))
    if not package_files:
        raise RuntimeError(
            f"installed {distribution_name} distribution has no file manifest"
        )

    digest = hashlib.sha256()
    byte_count = 0
    file_count = 0
    for package_file in package_files:
        resolved = Path(distribution.locate_file(package_file))
        if not resolved.is_file():
            continue
        relative = str(package_file).replace(os.sep, "/").encode("utf-8")
        file_size = resolved.stat().st_size
        digest.update(len(relative).to_bytes(8, "little"))
        digest.update(relative)
        digest.update(file_size.to_bytes(8, "little"))
        with resolved.open("rb") as artifact_file:
            while chunk := artifact_file.read(1024 * 1024):
                digest.update(chunk)
                byte_count += len(chunk)
        file_count += 1

    if file_count == 0:
        raise RuntimeError(
            f"installed {distribution_name} distribution has no readable files"
        )
    return {
        "sha256": digest.hexdigest(),
        "bytes": byte_count,
        "files": file_count,
        "path": str(Path(module_file).resolve()),
        "scheme": "importlib-metadata-content-tree-v1",
    }


def pandas_artifact_identity() -> dict[str, Any]:
    """Hash the installed pandas distribution imported by this process."""
    return distribution_artifact_identity("pandas", pd.__file__)


def pyarrow_artifact_identity() -> dict[str, Any]:
    """Hash the optional Arrow backend used by string[pyarrow] arms."""
    return distribution_artifact_identity("pyarrow", pa.__file__)


def bootstrap_median_ci(values: list[float]) -> tuple[float, float]:
    """Deterministic percentile-bootstrap CI for the sample median."""
    if not values:
        raise ValueError("cannot bootstrap an empty null-control sample")
    sample = np.asarray(values, dtype=np.float64)
    rng = np.random.default_rng(0xF2A_2026_0725)
    indices = rng.integers(
        0,
        len(sample),
        size=(BOOTSTRAP_RESAMPLES, len(sample)),
    )
    medians = np.median(sample[indices], axis=1)
    tail_pct = (1.0 - NULL_CI_CONFIDENCE) * 50.0
    low, high = np.percentile(medians, [tail_pct, 100.0 - tail_pct])
    return float(low), float(high)


def bootstrap_median_ratio_ci(
    numerator: list[float],
    denominator: list[float],
) -> tuple[float, float]:
    """Independent-sample bootstrap CI for a ratio of engine medians."""
    if not numerator or not denominator:
        raise ValueError("cannot bootstrap an empty effect sample")
    numerator_sample = np.asarray(numerator, dtype=np.float64)
    denominator_sample = np.asarray(denominator, dtype=np.float64)
    if np.any(numerator_sample <= 0.0) or np.any(denominator_sample <= 0.0):
        raise ValueError("effect timing samples must be positive")

    rng = np.random.default_rng(0xF2A_2026_0731)
    numerator_indices = rng.integers(
        0,
        len(numerator_sample),
        size=(BOOTSTRAP_RESAMPLES, len(numerator_sample)),
    )
    denominator_indices = rng.integers(
        0,
        len(denominator_sample),
        size=(BOOTSTRAP_RESAMPLES, len(denominator_sample)),
    )
    ratios = np.median(numerator_sample[numerator_indices], axis=1) / np.median(
        denominator_sample[denominator_indices],
        axis=1,
    )
    tail_pct = (1.0 - NULL_CI_CONFIDENCE) * 50.0
    low, high = np.percentile(ratios, [tail_pct, 100.0 - tail_pct])
    return float(low), float(high)


def corrected_null_gate(
    ratio: float,
    effect_ci: tuple[float, float],
    required_log_effect: float,
    subject_null_median: float,
    incumbent_null_median: float,
    subject_label: str = "frankenpandas",
    incumbent_label: str = "pandas",
) -> dict[str, Any]:
    """Apply the fleet's corrected three-clause null-control gate."""
    effect_ci_low, effect_ci_high = effect_ci
    claim_log_effect = abs(math.log(ratio)) if ratio > 0.0 else math.inf
    effect_ci_excludes_unity = effect_ci_high < 1.0 or effect_ci_low > 1.0
    effect_exceeds_null_margin = claim_log_effect >= required_log_effect
    subject_null_median_within_limit = (
        abs(subject_null_median - 1.0) <= NULL_MEDIAN_MAX_ABS_DEVIATION
    )
    incumbent_null_median_within_limit = (
        abs(incumbent_null_median - 1.0) <= NULL_MEDIAN_MAX_ABS_DEVIATION
    )
    null_medians_within_limit = (
        subject_null_median_within_limit
        and incumbent_null_median_within_limit
    )
    return {
        "decidable": (
            effect_ci_excludes_unity
            and effect_exceeds_null_margin
            and null_medians_within_limit
        ),
        "effect_median_ratio_ci_95": [
            round(effect_ci_low, 8),
            round(effect_ci_high, 8),
        ],
        "clauses": {
            "effect_ci_excludes_unity": effect_ci_excludes_unity,
            "effect_exceeds_two_x_null_margin": effect_exceeds_null_margin,
            "null_medians_within_2pct_unity": null_medians_within_limit,
        },
        "null_median_unity": {
            "maximum_absolute_deviation": NULL_MEDIAN_MAX_ABS_DEVIATION,
            subject_label: round(subject_null_median, 8),
            f"{subject_label}_within_limit": subject_null_median_within_limit,
            incumbent_label: round(incumbent_null_median, 8),
            f"{incumbent_label}_within_limit": (
                incumbent_null_median_within_limit
            ),
        },
        "claim_log_effect": round(claim_log_effect, 8),
        "required_log_effect": round(required_log_effect, 8),
    }


def _corrected_null_gate_self_test() -> None:
    _math_unary_input_self_test()
    assert bootstrap_median_ratio_ci([4.0] * 5, [2.0] * 5) == (2.0, 2.0)

    passing = corrected_null_gate(1.5, (1.4, 1.6), 0.1, 1.01, 0.99)
    assert passing["decidable"]

    ci_straddles = corrected_null_gate(1.5, (0.99, 1.6), 0.1, 1.0, 1.0)
    assert not ci_straddles["decidable"]
    assert not ci_straddles["clauses"]["effect_ci_excludes_unity"]

    below_margin = corrected_null_gate(1.01, (1.001, 1.02), 0.1, 1.0, 1.0)
    assert not below_margin["decidable"]
    assert not below_margin["clauses"]["effect_exceeds_two_x_null_margin"]

    fp_median_outside = corrected_null_gate(1.5, (1.4, 1.6), 0.1, 1.021, 1.0)
    assert not fp_median_outside["decidable"]
    assert not fp_median_outside["clauses"]["null_medians_within_2pct_unity"]

    pandas_median_outside = corrected_null_gate(
        1.5,
        (1.4, 1.6),
        0.1,
        1.0,
        0.979,
    )
    assert not pandas_median_outside["decidable"]
    assert not pandas_median_outside["clauses"]["null_medians_within_2pct_unity"]

    labeled = corrected_null_gate(
        1.5,
        (1.4, 1.6),
        0.1,
        1.0,
        1.0,
        "candidate",
        "reference",
    )
    assert labeled["null_median_unity"]["candidate"] == 1.0
    assert labeled["null_median_unity"]["reference"] == 1.0

    def synthetic_result(times_us: list[float], executable: str) -> TimingResult:
        return TimingResult(
            workload="synthetic",
            category="math_unary",
            size="1M",
            dtype="float64",
            engine="frankenpandas",
            times_us=times_us,
            null_arm_a_us=[1.0] * 5,
            null_arm_b_us=[1.0] * 5,
            null_ratios=[1.0] * 5,
            checksum="contract-witness",
            executable_sha256=executable,
            runtime_available_parallelism=10,
            operation_threads_used=1,
            runtime_detected_isa_features=["avx2"],
        )

    whole_binary = compute_candidate_vs_reference(
        synthetic_result([2.0] * 10, "candidate"),
        synthetic_result([4.0] * 10, "reference"),
    )
    assert whole_binary["ratio"] == 2.0
    assert whole_binary["verdict"] == "CANDIDATE_FASTER"


@dataclass
class TimingResult:
    """Raw timing measurements for a single workload."""
    workload: str
    category: str
    size: str
    dtype: str
    engine: str
    times_us: list[float] = field(default_factory=list)
    null_arm_a_us: list[float] = field(default_factory=list)
    null_arm_b_us: list[float] = field(default_factory=list)
    null_ratios: list[float] = field(default_factory=list)
    checksum: str | None = None
    executable_sha256: str | None = None
    executable_bytes: int | None = None
    executable_path: str | None = None
    runtime_available_parallelism: int | None = None
    process_threads_before_probe: int | None = None
    peak_process_threads: int | None = None
    operation_threads_used: int | None = None
    runtime_detected_isa_features: list[str] = field(default_factory=list)

    @property
    def p50_us(self) -> float:
        return float(np.percentile(self.times_us, 50))

    @property
    def p95_us(self) -> float:
        return float(np.percentile(self.times_us, 95))

    @property
    def p99_us(self) -> float:
        return float(np.percentile(self.times_us, 99))

    @property
    def mean_us(self) -> float:
        return mean(self.times_us)

    @property
    def stddev_us(self) -> float:
        return stdev(self.times_us) if len(self.times_us) > 1 else 0.0

    @property
    def cv_pct(self) -> float:
        return (self.stddev_us / self.mean_us * 100) if self.mean_us > 0 else 0.0

    @property
    def null_median_ratio(self) -> float:
        return float(np.median(self.null_ratios))

    @property
    def null_median_ci(self) -> tuple[float, float]:
        return bootstrap_median_ci(self.null_ratios)

    @property
    def null_log_half_width(self) -> float:
        low, high = self.null_median_ci
        if low <= 0.0 or high <= 0.0:
            return math.inf
        return max(abs(math.log(low)), abs(math.log(high)))

    @property
    def is_valid(self) -> bool:
        """Contract-valid measurement; cv is provenance, never a gate."""
        return bool(
            self.times_us
            and self.null_ratios
            and len(self.null_arm_a_us) == len(self.null_ratios)
            and len(self.null_arm_b_us) == len(self.null_ratios)
            and self.checksum
            and self.executable_sha256
            and self.runtime_available_parallelism
            and self.operation_threads_used
            and self.runtime_detected_isa_features
        )

    def to_metrics(self, rows: int) -> dict[str, Any]:
        null_ci_low, null_ci_high = self.null_median_ci
        null_log_half_width = self.null_log_half_width
        return {
            "p50_us": round(self.p50_us, 2),
            "p95_us": round(self.p95_us, 2),
            "p99_us": round(self.p99_us, 2),
            "mean_us": round(self.mean_us, 2),
            "stddev_us": round(self.stddev_us, 2),
            "cv_pct": round(self.cv_pct, 2),
            "throughput_rows_sec": round(rows / (self.p50_us / 1_000_000)),
            "checksum": self.checksum,
            "thread_count_actually_used": self.operation_threads_used,
            "runtime_available_parallelism": self.runtime_available_parallelism,
            "process_threads_before_probe": self.process_threads_before_probe,
            "peak_process_threads": self.peak_process_threads,
            "runtime_detected_isa_features": self.runtime_detected_isa_features,
            "samples_us": self.times_us,
            "executable": {
                "sha256": self.executable_sha256,
                "bytes": self.executable_bytes,
                "path": self.executable_path,
            },
            "null_control": {
                "rounds": len(self.null_ratios),
                "arm_a_times_us": self.null_arm_a_us,
                "arm_b_times_us": self.null_arm_b_us,
                "ratios": self.null_ratios,
                "median_ratio": round(self.null_median_ratio, 6),
                "median_ci_95": [
                    round(null_ci_low, 6),
                    round(null_ci_high, 6),
                ],
                "log_half_width": round(null_log_half_width, 8),
                "two_x_decidable_interval": [
                    round(math.exp(-DECIDABILITY_MARGIN * null_log_half_width), 6),
                    round(math.exp(DECIDABILITY_MARGIN * null_log_half_width), 6),
                ],
            },
        }


def generate_test_data(rows: int, cols: int, dtype: str, seed: int = 42) -> pd.DataFrame:
    """Generate test DataFrame OUTSIDE the timed window."""
    rng = np.random.default_rng(seed)

    if dtype == "int64":
        data = {f"col_{i}": rng.integers(0, 1_000_000, size=rows) for i in range(cols)}
    elif dtype == "bool":
        data = {f"col_{i}": rng.integers(0, 2, size=rows, dtype=np.int8).astype(bool)
                for i in range(cols)}
    elif dtype in ("datetime64", "datetime64[ns]"):
        base = np.datetime64("2021-01-01T00:00:00", "ns")
        offsets = np.arange(rows, dtype=np.int64) * 1_000_000_000
        data = {f"col_{i}": base + (offsets + i).astype("timedelta64[ns]")
                for i in range(cols)}
    elif dtype in ("timedelta64", "timedelta64[ns]"):
        offsets = np.arange(rows, dtype=np.int64) * 1_000_000
        data = {f"col_{i}": (offsets + i).astype("timedelta64[ns]")
                for i in range(cols)}
    elif dtype == "float64":
        data = {f"col_{i}": rng.random(rows) * 1_000_000 for i in range(cols)}
    elif dtype == "float64_nan10":
        data = {}
        for i in range(cols):
            arr = rng.random(rows) * 1_000_000
            mask = rng.random(rows) < 0.10
            arr[mask] = np.nan
            data[f"col_{i}"] = arr
    elif dtype == "float64_nan50":
        data = {}
        for i in range(cols):
            arr = rng.random(rows) * 1_000_000
            mask = rng.random(rows) < 0.50
            arr[mask] = np.nan
            data[f"col_{i}"] = arr
    elif dtype == "float64_nan37":
        data = {}
        for i in range(cols):
            arr = rng.random(rows) * 1_000_000
            arr[::37] = np.nan
            data[f"col_{i}"] = arr
    else:
        raise ValueError(f"Unknown dtype: {dtype}")

    return pd.DataFrame(data)


def _observation_token(result: Any) -> tuple[Any, ...]:
    """Small deterministic observation folded outside the measured region."""
    shape = getattr(result, "shape", None)
    dtype = getattr(result, "dtype", None)
    dtypes = getattr(result, "dtypes", None)
    if dtypes is not None:
        try:
            dtype_token: Any = tuple(str(item) for item in dtypes)
        except TypeError:
            dtype_token = str(dtypes)
    else:
        dtype_token = str(dtype) if dtype is not None else None
    try:
        length = len(result)
    except TypeError:
        length = None
    return (
        type(result).__qualname__,
        tuple(shape) if shape is not None else None,
        dtype_token,
        length,
    )


def _fold_checksum(checksum: int, result: Any) -> int:
    encoded = repr(_observation_token(result)).encode("utf-8")
    observed = int.from_bytes(hashlib.sha256(encoded).digest()[:8], "little")
    rotated = ((checksum << 9) | (checksum >> 55)) & ((1 << 64) - 1)
    return rotated ^ observed


def paired_operation(func, repeat: int = 1,
                     warmup: int = WARMUP_ITERATIONS,
                     rounds: int = PAIRED_ROUNDS) -> PairedSamples:
    """Time identical arms back-to-back, alternating order every round."""
    thread_probe = probe_operation_threads(func)
    for _ in range(warmup):
        for _ in range(repeat):
            func()

    def time_arm(checksum: int) -> tuple[float, int]:
        start = time.perf_counter_ns()
        result = None
        for _ in range(repeat):
            result = func()
        elapsed_us = (time.perf_counter_ns() - start) / 1000
        return elapsed_us, _fold_checksum(checksum, result)

    times_us: list[float] = []
    null_arm_a_us: list[float] = []
    null_arm_b_us: list[float] = []
    null_ratios: list[float] = []
    checksum = 0
    for round_index in range(rounds):
        if round_index % 2 == 0:
            arm_a_us, checksum = time_arm(checksum)
            arm_b_us, checksum = time_arm(checksum)
        else:
            arm_b_us, checksum = time_arm(checksum)
            arm_a_us, checksum = time_arm(checksum)
        times_us.extend((arm_a_us, arm_b_us))
        null_arm_a_us.append(arm_a_us)
        null_arm_b_us.append(arm_b_us)
        null_ratios.append(arm_a_us / arm_b_us)

    return PairedSamples(
        times_us=times_us,
        null_arm_a_us=null_arm_a_us,
        null_arm_b_us=null_arm_b_us,
        null_ratios=null_ratios,
        checksum=checksum,
        runtime_available_parallelism=thread_probe[
            "runtime_available_parallelism"
        ],
        process_threads_before_probe=thread_probe[
            "process_threads_before_probe"
        ],
        peak_process_threads=thread_probe["peak_process_threads"],
        operation_threads_used=thread_probe["operation_threads_used"],
    )


def time_operation(func, warmup: int = WARMUP_ITERATIONS) -> PairedSamples:
    """Time an operation with an interleaved same-invocation A/A control."""
    return paired_operation(func, warmup=warmup)


def time_operation_repeated(func, repeat: int,
                            warmup: int = WARMUP_ITERATIONS) -> PairedSamples:
    """Time a fixed-size batch with an interleaved A/A null control."""
    return paired_operation(func, repeat=repeat, warmup=warmup)


# IO Workloads (pandas)
def bench_csv_read_pandas(df: pd.DataFrame, tmp_path: Path) -> float:
    csv_path = tmp_path / "bench.csv"
    df.to_csv(csv_path, index=False)
    return time_operation(lambda: pd.read_csv(csv_path))


def bench_csv_read_block_view_pandas(df: pd.DataFrame, tmp_path: Path) -> float:
    """Read a homogeneous Float64 CSV and take pandas' no-copy array view."""
    csv_path = tmp_path / "bench.csv"
    df.to_csv(csv_path, index=False)
    return time_operation(lambda: pd.read_csv(csv_path).to_numpy(copy=False))


def bench_csv_write_pandas(df: pd.DataFrame, tmp_path: Path) -> float:
    csv_path = tmp_path / "bench_out.csv"
    return time_operation(lambda: df.to_csv(csv_path, index=False))


def bench_json_read_records_pandas(df: pd.DataFrame, tmp_path: Path) -> list[float]:
    del tmp_path
    payload = df.to_json(orient="records")
    return time_operation(
        lambda: pd.read_json(StringIO(payload), orient="records")
    )


def bench_json_read_columns_pandas(df: pd.DataFrame, tmp_path: Path) -> list[float]:
    del tmp_path
    payload = df.to_json(orient="columns")
    return time_operation(
        lambda: pd.read_json(StringIO(payload), orient="columns")
    )


def bench_json_read_index_pandas(df: pd.DataFrame, tmp_path: Path) -> list[float]:
    del tmp_path
    payload = df.to_json(orient="index")
    return time_operation(
        lambda: pd.read_json(StringIO(payload), orient="index")
    )


def bench_json_read_split_pandas(df: pd.DataFrame, tmp_path: Path) -> list[float]:
    del tmp_path
    payload = df.to_json(orient="split")
    return time_operation(
        lambda: pd.read_json(StringIO(payload), orient="split")
    )


def bench_json_read_values_pandas(df: pd.DataFrame, tmp_path: Path) -> list[float]:
    del tmp_path
    payload = df.to_json(orient="values")
    return time_operation(
        lambda: pd.read_json(StringIO(payload), orient="values")
    )


def bench_parquet_read_pandas(df: pd.DataFrame, tmp_path: Path) -> float:
    pq_path = tmp_path / "bench.parquet"
    df.to_parquet(pq_path, index=False)
    return time_operation(lambda: pd.read_parquet(pq_path))

def bench_parquet_write_pandas(df: pd.DataFrame, tmp_path: Path) -> float:
    pq_path = tmp_path / "bench_out.parquet"
    return time_operation(lambda: df.to_parquet(pq_path, index=False))


# Whole-job pipeline workload (pandas)
#
# Not a kernel benchmark: one timed closure is a complete star-schema rollup of
# the shape a pandas user actually writes -- load, filter, groupby, join, sort,
# write. The FrankenPandas arm (fp-bench --category pipeline) runs the same six
# stages over the same two input CSVs and writes its own output, which
# `compare_pipeline_outputs` then diffs. Whole-job wall time is the headline;
# the per-stage split is reported separately as a diagnostic, because a job
# this shape is normally dominated by one stage and a reader who is not told
# which one will misread the ratio.
PIPELINE_ROWS_PER_STORE = 200
PIPELINE_REGIONS = 12
# Amounts sit on a $0.25 tick, which is exactly representable in binary
# float64. Sums of such values are therefore exact and order-independent, so
# the two engines' outputs can be required to agree EXACTLY instead of within
# a tolerance -- pandas and FrankenPandas reduce each group in their own
# order, and on arbitrary decimal cents that alone would shift the last ULP
# and make a legitimate result look like a mismatch.
PIPELINE_AMOUNT_TICK = 0.25
PIPELINE_TICK_LOW = -3000   # -$750.00 (refunds)
PIPELINE_TICK_HIGH = 7000   # +$1750.00
PIPELINE_SEED = 20260730


def pipeline_input_paths(tmp_path: Path) -> tuple[Path, Path]:
    return tmp_path / "sales.csv", tmp_path / "stores.csv"


def materialize_pipeline_inputs(rows: int, tmp_path: Path) -> tuple[Path, Path]:
    """Write the job's two input CSVs once, OUTSIDE every timed window.

    Both engines read these exact bytes, so the comparison cannot be an
    artifact of one engine getting easier input than the other.
    """
    sales_path, stores_path = pipeline_input_paths(tmp_path)
    n_stores = max(1, rows // PIPELINE_ROWS_PER_STORE)
    rng = np.random.default_rng(PIPELINE_SEED)
    ticks = rng.integers(
        PIPELINE_TICK_LOW, PIPELINE_TICK_HIGH, size=rows, dtype=np.int64
    )
    pd.DataFrame(
        {
            "store_id": rng.integers(0, n_stores, size=rows, dtype=np.int64),
            "units": rng.integers(1, 50, size=rows, dtype=np.int64),
            "amount": ticks * PIPELINE_AMOUNT_TICK,
        }
    ).to_csv(sales_path, index=False)

    store_ids = np.arange(n_stores, dtype=np.int64)
    pd.DataFrame(
        {
            "store_id": store_ids,
            "store_name": [f"store_{i:06d}" for i in store_ids],
            "region": [f"region_{i % PIPELINE_REGIONS:02d}" for i in store_ids],
        }
    ).to_csv(stores_path, index=False)
    return sales_path, stores_path


def materialize_pipeline_inputs_parquet(
    rows: int, tmp_path: Path
) -> tuple[Path, Path]:
    """Same two tables, Parquet-encoded, written outside every timed window.

    `etl_job` at 1M is 82.3% read_csv on the pandas side, so its whole-job ratio
    is mostly a CSV-parse ratio. This variant runs the identical six stages off
    Parquet, where load is cheap and the compute stages carry real weight. The
    pair brackets the question a single shape cannot answer: does a whole-job
    win survive when parsing is not the bulk of the job?

    Derived by re-reading the CSVs rather than re-generating, so both formats
    provably carry the same values.
    """
    sales_csv, stores_csv = materialize_pipeline_inputs(rows, tmp_path)
    sales_pq = tmp_path / "sales.parquet"
    stores_pq = tmp_path / "stores.parquet"
    pd.read_csv(sales_csv).to_parquet(sales_pq, index=False)
    pd.read_csv(stores_csv).to_parquet(stores_pq, index=False)
    return sales_pq, stores_pq


def _pipeline_job_pandas(sales_path: Path, stores_path: Path, out_path: Path):
    """The six stages, in idiomatic pandas 2.2.3."""
    sales = pd.read_csv(sales_path)                                   # 1. load
    stores = pd.read_csv(stores_path)
    kept = sales[sales["amount"] > 0.0]                               # 2. filter
    agg = kept.groupby("store_id", as_index=False).sum()              # 3. groupby
    joined = agg.merge(stores, on="store_id", how="inner")            # 4. join
    ranked = joined.sort_values(                                      # 5. sort
        ["amount", "store_id"], ascending=[False, True]
    )
    ranked.to_csv(out_path, index=False)                              # 6. write
    return ranked


def _pipeline_job_pandas_parquet(sales_path: Path, stores_path: Path,
                                 out_path: Path):
    """Identical six stages; only the load format differs."""
    sales = pd.read_parquet(sales_path)                               # 1. load
    stores = pd.read_parquet(stores_path)
    kept = sales[sales["amount"] > 0.0]                               # 2. filter
    agg = kept.groupby("store_id", as_index=False).sum()              # 3. groupby
    joined = agg.merge(stores, on="store_id", how="inner")            # 4. join
    ranked = joined.sort_values(                                      # 5. sort
        ["amount", "store_id"], ascending=[False, True]
    )
    ranked.to_csv(out_path, index=False)                              # 6. write
    return ranked


def bench_pipeline_etl_job_pandas(
    df: pd.DataFrame, tmp_path: Path
) -> PairedSamples:
    rows = len(df)
    sales_path, stores_path = materialize_pipeline_inputs(rows, tmp_path)
    out_path = tmp_path / "out_pandas.csv"
    return time_operation(
        partial(_pipeline_job_pandas, sales_path, stores_path, out_path)
    )


def bench_pipeline_etl_job_parquet_pandas(
    df: pd.DataFrame, tmp_path: Path
) -> PairedSamples:
    rows = len(df)
    sales_path, stores_path = materialize_pipeline_inputs_parquet(rows, tmp_path)
    out_path = tmp_path / "out_pandas_parquet.csv"
    return time_operation(
        partial(_pipeline_job_pandas_parquet, sales_path, stores_path, out_path)
    )


def _pipeline_columns_equal(left: pd.Series, right: pd.Series) -> bool:
    """Same values, ignoring how each engine chose to render them."""
    if is_numeric_dtype(left) and is_numeric_dtype(right):
        lhs = left.to_numpy(dtype=np.float64, copy=False)
        rhs = right.to_numpy(dtype=np.float64, copy=False)
        # Exact equality is the right test here, not a tolerance: the $0.25
        # amount tick makes every group sum exactly representable, so any
        # real difference is a difference in the work, not in rounding.
        return bool(
            np.array_equal(lhs, rhs)
            or np.array_equal(lhs, rhs, equal_nan=True)
        )
    return bool(left.astype(str).equals(right.astype(str)))


def compare_pipeline_outputs(tmp_path: Path,
                             workload: str = "etl_job") -> dict[str, Any]:
    """Diff what the two engines actually produced.

    Byte identity is the strong result, and the $0.25 amount tick is chosen to
    make it attainable. When the bytes differ, say exactly how: a column-order
    or rendering difference is a very different finding from a value
    difference, and collapsing both into "mismatch" would bury the useful one.
    """
    if workload == "etl_job_parquet":
        fp_path = tmp_path / "out_frankenpandas_parquet.csv"
        pd_path = tmp_path / "out_pandas_parquet.csv"
    else:
        fp_path = tmp_path / "out_frankenpandas.csv"
        pd_path = tmp_path / "out_pandas.csv"
    report: dict[str, Any] = {
        "frankenpandas_output": str(fp_path),
        "pandas_output": str(pd_path),
    }
    for label, path in (("frankenpandas", fp_path), ("pandas", pd_path)):
        if not path.is_file():
            report["equivalent"] = False
            report["reason"] = f"{label} arm wrote no output at {path}"
            return report

    fp_bytes = fp_path.read_bytes()
    pd_bytes = pd_path.read_bytes()
    report["frankenpandas_sha256"] = hashlib.sha256(fp_bytes).hexdigest()
    report["pandas_sha256"] = hashlib.sha256(pd_bytes).hexdigest()
    report["frankenpandas_bytes"] = len(fp_bytes)
    report["pandas_bytes"] = len(pd_bytes)
    if fp_bytes == pd_bytes:
        report["equivalent"] = True
        report["match"] = "byte_identical"
        return report

    fp_df = pd.read_csv(fp_path)
    pd_df = pd.read_csv(pd_path)
    report["frankenpandas_shape"] = list(fp_df.shape)
    report["pandas_shape"] = list(pd_df.shape)
    report["frankenpandas_columns"] = list(fp_df.columns)
    report["pandas_columns"] = list(pd_df.columns)

    if set(fp_df.columns) != set(pd_df.columns):
        report["equivalent"] = False
        report["reason"] = "output column sets differ"
        return report
    if len(fp_df) != len(pd_df):
        report["equivalent"] = False
        report["reason"] = "output row counts differ"
        return report

    # Same columns and same row count. Compare values in row order -- row
    # order IS part of this job's output, because stage 5 is the ranking.
    mismatched: list[dict[str, Any]] = []
    for col in pd_df.columns:
        lhs = fp_df[col].reset_index(drop=True)
        rhs = pd_df[col].reset_index(drop=True)
        if _pipeline_columns_equal(lhs, rhs):
            continue
        differing = np.flatnonzero(
            (lhs.astype(str) != rhs.astype(str)).to_numpy()
        )
        first = int(differing[0]) if differing.size else 0
        mismatched.append({
            "column": col,
            "differing_rows": int(differing.size),
            "first_differing_row": first,
            "frankenpandas_value": str(lhs.iloc[first]),
            "pandas_value": str(rhs.iloc[first]),
        })
    if mismatched:
        report["equivalent"] = False
        report["reason"] = "value mismatch"
        report["mismatched_columns"] = mismatched
        return report

    report["equivalent"] = True
    report["match"] = (
        "values_identical_column_order_differs"
        if list(fp_df.columns) != list(pd_df.columns)
        else "values_identical_rendering_differs"
    )
    return report


# Math-unary workloads (pandas)
#
# The family docs/NEGATIVE_EVIDENCE.md recorded as blocked on the build target:
# floor 0.089x, ceil 0.11x, trunc 0.13x, round(decimals) 0.090x, sqrt ~0.085x,
# log 0.20x vs pandas -- "NOT source-fixable ... the ceiling for the math-unary
# family until that build-target call is revisited". numpy runtime-dispatches
# AVX regardless of compile target; FrankenPandas built for generic x86-64, so
# these lower to libm libcalls and scalar sqrtsd. The fleet ISA floor moved to
# x86-64-v3 on 2026-07-25, satisfying that retry condition.
#
# Input is bit-identical to the Rust arm: the vectorized generator below
# reproduces `SplitMix64::unit` rather than merely choosing a similar random
# distribution. It is strictly positive and overwhelmingly NON-INTEGRAL, so
# the integral-value floor/ceil/trunc identity witness cannot short-circuit the
# kernel and sqrt/log stay finite.
MATH_UNARY_SEED = 0x1234_5678_9ABC_DEF0
MATH_UNARY_LOW = 1.0
MATH_UNARY_HIGH = 100_000.0
SPLITMIX64_GAMMA = 0x9E37_79B9_7F4A_7C15
SPLITMIX64_MUL1 = 0xBF58_476D_1CE4_E5B9
SPLITMIX64_MUL2 = 0x94D0_49BB_1331_11EB
SPLITMIX64_MASK = (1 << 64) - 1


def _math_unary_values(rows: int) -> np.ndarray:
    """Reproduce fp-bench's SplitMix64-generated f64 input exactly."""
    steps = np.arange(1, rows + 1, dtype=np.uint64)
    z = np.uint64(MATH_UNARY_SEED) + steps * np.uint64(SPLITMIX64_GAMMA)
    z = (z ^ (z >> np.uint64(30))) * np.uint64(SPLITMIX64_MUL1)
    z = (z ^ (z >> np.uint64(27))) * np.uint64(SPLITMIX64_MUL2)
    z ^= z >> np.uint64(31)
    units = (z >> np.uint64(11)).astype(np.float64) / float(1 << 53)
    return MATH_UNARY_LOW + units * (MATH_UNARY_HIGH - MATH_UNARY_LOW)


def _math_unary_input_self_test() -> None:
    """Cross-check the vector stream against scalar Rust-equivalent steps."""
    expected = []
    state = MATH_UNARY_SEED
    for _ in range(257):
        state = (state + SPLITMIX64_GAMMA) & SPLITMIX64_MASK
        z = state
        z = ((z ^ (z >> 30)) * SPLITMIX64_MUL1) & SPLITMIX64_MASK
        z = ((z ^ (z >> 27)) * SPLITMIX64_MUL2) & SPLITMIX64_MASK
        z ^= z >> 31
        unit = (z >> 11) / float(1 << 53)
        expected.append(MATH_UNARY_LOW + unit * (MATH_UNARY_HIGH - MATH_UNARY_LOW))
    actual = _math_unary_values(len(expected))
    assert actual.dtype == np.float64
    assert np.array_equal(actual, np.asarray(expected, dtype=np.float64))


def _math_unary_input(rows: int) -> pd.Series:
    return pd.Series(_math_unary_values(rows), copy=False)


def _math_unary_int_input(rows: int) -> pd.Series:
    """The same value stream truncated to int64.

    br-frankenpandas-4kig1. Keeping the values identical to the float fixture is
    what makes the two dtypes comparable; a separately-seeded integer stream would
    measure different numbers as well as a different dtype. pandas widens int64 to
    float64 for sqrt/log exactly as FrankenPandas does, so both engines do the same
    work on the same values.
    """
    return pd.Series(_math_unary_values(rows).astype("int64"), copy=False)


def _bench_math_unary_int(df: pd.DataFrame, op) -> PairedSamples:
    s = _math_unary_int_input(len(df))
    return time_operation(partial(op, s))


def bench_math_sqrt_int64_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_math_unary_int(df, np.sqrt)


def bench_math_log_int64_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_math_unary_int(df, np.log)


def _math_unary_rhs_input(rows: int) -> pd.Series:
    """Right-hand operand for the binary lanes.

    br-frankenpandas-4kig1. Same generator family as the unary fixture, advanced
    by one draw and narrowed to [1, 10] so `pow` stays finite — a large exponent
    would overflow to inf and turn the lane into an infinity benchmark. Strictly
    positive for the same reason the unary fixture is: a negative base with a
    fractional exponent is NaN, and the lane should measure arithmetic rather than
    the missing-value path.
    """
    steps = np.arange(1, rows + 1, dtype=np.uint64)
    z = np.uint64(0x0FEDCBA987654321) + steps * np.uint64(SPLITMIX64_GAMMA)
    z = (z ^ (z >> np.uint64(30))) * np.uint64(SPLITMIX64_MUL1)
    z = (z ^ (z >> np.uint64(27))) * np.uint64(SPLITMIX64_MUL2)
    z ^= z >> np.uint64(31)
    units = (z >> np.uint64(11)).astype(np.float64) / float(1 << 53)
    return pd.Series(1.0 + units * 9.0, copy=False)


def _bench_math_binary(df: pd.DataFrame, op) -> PairedSamples:
    left = _math_unary_input(len(df))
    right = _math_unary_rhs_input(len(df))
    return time_operation(partial(op, left, right))


def bench_math_pow_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_math_binary(df, np.power)


def bench_math_atan2_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_math_binary(df, np.arctan2)


def bench_math_hypot_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_math_binary(df, np.hypot)


def bench_math_mod_pandas(df: pd.DataFrame) -> PairedSamples:
    # Series.mod, not np.mod: pandas' own operator is the incumbent surface
    # FrankenPandas claims parity with, and it carries pandas' sign convention
    # for negative operands rather than numpy's.
    return _bench_math_binary(df, lambda a, b: a.mod(b))


def bench_math_floordiv_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_math_binary(df, lambda a, b: a.floordiv(b))


def bench_math_add_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_math_binary(df, lambda a, b: a.add(b))


def bench_math_div_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_math_binary(df, lambda a, b: a.div(b))


def _bench_math_unary(df: pd.DataFrame, op) -> PairedSamples:
    s = _math_unary_input(len(df))
    return time_operation(partial(op, s))


def bench_math_floor_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_math_unary(df, np.floor)


def bench_math_ceil_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_math_unary(df, np.ceil)


def bench_math_trunc_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_math_unary(df, np.trunc)


def bench_math_round2_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_math_unary(df, lambda s: s.round(2))


def bench_math_sqrt_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_math_unary(df, np.sqrt)


def bench_math_log_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_math_unary(df, np.log)


def bench_math_log10_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_math_unary(df, np.log10)


def bench_math_log2_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_math_unary(df, np.log2)


def bench_math_log1p_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_math_unary(df, np.log1p)


def bench_math_expm1_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_math_unary(df, np.expm1)


def bench_math_cbrt_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_math_unary(df, np.cbrt)


def bench_math_sin_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_math_unary(df, np.sin)


def bench_math_atan_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_math_unary(df, np.arctan)


# DataFrame Ops Workloads (pandas)
def bench_sort_values_single_pandas(df: pd.DataFrame) -> list[float]:
    return time_operation(lambda: df.sort_values("col_0"))

def bench_sort_values_multi_pandas(df: pd.DataFrame) -> list[float]:
    return time_operation(lambda: df.sort_values(["col_0", "col_1", "col_2"]))

def bench_filter_bool_mask_pandas(df: pd.DataFrame) -> list[float]:
    return time_operation(lambda: df[df["col_0"] > df["col_0"].median()])

def bench_drop_duplicates_pandas(df: pd.DataFrame) -> list[float]:
    return time_operation(lambda: df.drop_duplicates(subset=["col_0"]))

def bench_value_counts_pandas(df: pd.DataFrame) -> list[float]:
    return time_operation(lambda: df["col_0"].value_counts())

def bench_cumsum_pandas(df: pd.DataFrame) -> list[float]:
    return time_operation(lambda: df.cumsum())


def bench_df_abs_pandas(df: pd.DataFrame) -> list[float]:
    """Elementwise absolute value over the same dense Float64 frame as fp-bench."""
    return time_operation(lambda: df.abs())


def bench_df_melt_pandas(df: pd.DataFrame) -> list[float]:
    # Exact counterpart to fp-bench dataframe_ops/df_melt:
    # DataFrame::melt([], [], None, None) selects every input column as a
    # value column and uses the default "variable"/"value" output names.
    return time_operation(lambda: df.melt())


def bench_df_transpose_pandas(df: pd.DataFrame) -> list[float]:
    return time_operation_repeated(lambda: df.T, TRANSPOSE_BATCH)

def bench_df_transpose_materialize_pandas(df: pd.DataFrame) -> list[float]:
    # Counterpart to fp-bench dataframe_ops/df_transpose_materialize. `df.T` alone
    # can be a lazy/blockwise construction on both sides, so this row crosses the
    # materialization boundary explicitly by reading real values out of the
    # transposed frame, matching what the Rust row does.
    def op():
        t = df.T
        col = t.columns[0]
        return len(t[col].to_numpy())

    return time_operation(op)


def bench_df_to_dict_index_materialize_pandas(df: pd.DataFrame) -> list[float]:
    # Counterpart to fp-bench dataframe_ops/df_to_dict_index_materialize:
    # to_dict('index') is fully materialized in pandas, so the plain call is
    # the honest boundary-crossing row (the fp side forces as_mapping()).
    def op():
        return len(df.to_dict("index"))

    return time_operation(op)


def bench_df_pivot_pandas(df: pd.DataFrame) -> list[float]:
    # Counterpart to fp-bench dataframe_ops/df_pivot, whose comment specifies
    # df.pivot(index="r", columns="c", values="v") with UNIQUE (r, c) pairs
    # (pivot raises on duplicates): r = i // 10, c = i % 10.
    n = len(df)
    rows = np.arange(n)
    f = pd.DataFrame({"r": rows // 10, "c": rows % 10, "v": df["col_0"].to_numpy()})
    return time_operation(lambda: f.pivot(index="r", columns="c", values="v"))


def bench_df_pivot_table_pandas(df: pd.DataFrame) -> list[float]:
    # Counterpart to fp-bench dataframe_ops/df_pivot_table: r = i % 100 (100
    # rows), c = i % 10 (10 cols), aggfunc="mean" -> a 100x10 result.
    n = len(df)
    rows = np.arange(n)
    f = pd.DataFrame({"r": rows % 100, "c": rows % 10, "v": df["col_0"].to_numpy()})
    return time_operation(
        lambda: f.pivot_table(values="v", index="r", columns="c", aggfunc="mean")
    )


def bench_df_iterrows_pandas(df: pd.DataFrame) -> list[float]:
    # Counterpart to fp-bench dataframe_ops/df_iterrows, whose comment specifies
    # `list(df.iterrows())`. Class-1 structural shape: pandas constructs a Series
    # object per row, so the cost is per-element interpreter work rather than
    # kernel work. `list(...)` forces the generator so both sides do the full
    # materialization the Rust row does.
    def op():
        return len(list(df.iterrows()))

    return time_operation(op)


def bench_df_itertuples_pandas(df: pd.DataFrame) -> list[float]:
    # Counterpart to fp-bench dataframe_ops/df_itertuples (`list(df.itertuples())`).
    # itertuples is pandas' *fast* row iterator -- a namedtuple per row instead of
    # a Series -- so it is the conservative member of this family and the fairer
    # headline of the two.
    def op():
        return len(list(df.itertuples()))

    return time_operation(op)


def bench_df_row_tuples_fastest_pandas(df: pd.DataFrame) -> list[float]:
    # Fairness control for the iterator/callback retry predicate.  A local
    # four-idiom screen found to_records(...).tolist() faster than default
    # itertuples(), itertuples(name=None), and to_numpy()+map(tuple).  It yields
    # the same task-level product -- a fully materialized Python tuple per row,
    # including the index -- without charging pandas for namedtuple machinery.
    def op():
        return len(df.to_records(index=True).tolist())

    return time_operation(op)


def bench_df_apply_row_pandas(df: pd.DataFrame) -> list[float]:
    # Counterpart to fp-bench dataframe_ops/df_apply_row. The Rust row sums the
    # Float64 cells of each row via apply_fn(.., axis=1); the pandas expression of
    # the same user intent is df.apply(<row sum>, axis=1), which invokes a Python
    # callable per row.
    def op():
        return len(df.apply(lambda row: row.sum(), axis=1))

    return time_operation(op)


def bench_series_apply_stateful_pandas(df: pd.DataFrame) -> PairedSamples:
    """Run the fastest pandas route for an ordered, stateful callback.

    A six-route 1M screen compared ``Series.apply``, ``Series.map``,
    ``Series.transform``, an explicit Python loop, ``Series(map(...))``, and
    ``Series(np.fromiter(...))``. ``Series.map`` was the fastest
    task-equivalent route and produced the exact same output and final state.
    Unlike the row-sum apply trap, a reduction cannot preserve this
    recurrence's ordered callback outputs.
    """
    series = pd.Series(np.arange(len(df), dtype=np.int64), copy=False)

    def op():
        state = 0

        def step(value):
            nonlocal state
            state = (state * 31 + int(value)) & 0x7fff_ffff
            return state

        result = series.map(step)
        return result, state

    return time_operation(op)


def _bench_df_explode_pandas(
    df: pd.DataFrame,
    storage_dtype: object,
) -> PairedSamples:
    """Split three-part strings and explode, with setup outside timing."""
    n = len(df)
    values = [f"a{i % 97},b{i % 89},c{i % 83}" for i in range(n)]
    series = pd.Series(values, dtype=storage_dtype)
    return time_operation(lambda: series.str.split(",").explode())


def bench_df_explode_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_df_explode_pandas(df, object)


def bench_df_explode_string_python_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_df_explode_pandas(df, "string[python]")


def bench_df_explode_string_arrow_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_df_explode_pandas(df, "string[pyarrow]")


def bench_astype_str_f64_pandas(df: pd.DataFrame) -> PairedSamples:
    """Materialize the exact Float64 display strings through the fastest route.

    A same-worker nine-route screen on the exact 1M-row fixture selected a
    complete pandas Series built from ``np.frompyfunc("{:.1f}".format)``:
    168.554 ms median versus 460.534 ms for direct ``Series.astype(str)``.
    Every route produced the same object-dtype Series, RangeIndex, name, and
    ordered values, and each timed call included result destruction.

    This task-equivalent route is valid only for this deliberately bounded
    fixture: finite ``i * 1.5`` values through 10M rows are exact binary
    half-integers, so fixed one-decimal spelling equals pandas' shortest
    Float64 spelling. It must not headline arbitrary Float64, null, infinity,
    scientific-notation, locale, or precision workloads without a new screen.
    """
    values = np.arange(len(df), dtype="float64") * 1.5
    series = pd.Series(values, name="s", copy=False)
    formatter = "{:.1f}".format
    format_ufunc = np.frompyfunc(formatter, 1, 1)

    def operation():
        rendered = format_ufunc(values)
        result = pd.Series(
            rendered,
            index=series.index,
            name=series.name,
            copy=False,
        )
        # Rust's timed closure drops its Utf8 Series before returning. Observe
        # and drop the complete pandas result here as well so both arms include
        # result destruction instead of leaving Python's 10M-object teardown
        # outside the timer and immediately before the host-wide post gate.
        return len(result), result.iat[0], result.iat[-1]

    return time_operation(operation)


def bench_astype_str_f64_telemetry_batches_pandas(
    df: pd.DataFrame,
) -> PairedSamples:
    """Format and consume ordered telemetry strings in bounded-memory batches.

    Both engines prebuild the same finite ``i * 1.5`` Float64 sequence, retain
    global RangeIndex labels, and materialize one complete 250k-row string
    Series at a time. Each timed call observes cardinality and endpoints and
    destroys every batch before returning. This is a streaming sink contract,
    not a claim about retaining one monolithic 1M/10M object Series.
    """
    rows = len(df)
    values = np.arange(rows, dtype="float64") * 1.5
    batches = [
        pd.Series(
            values[start:stop],
            index=pd.RangeIndex(start, stop),
            name="s",
            copy=False,
        )
        for start in range(0, rows, TELEMETRY_STRING_BATCH_ROWS)
        for stop in [min(start + TELEMETRY_STRING_BATCH_ROWS, rows)]
    ]
    formatter = "{:.1f}".format
    format_ufunc = np.frompyfunc(formatter, 1, 1)

    def operation():
        observed_rows = 0
        first = None
        last = None
        for batch in batches:
            rendered = format_ufunc(batch.to_numpy(copy=False))
            result = pd.Series(
                rendered,
                index=batch.index,
                name=batch.name,
                copy=False,
            )
            if first is None:
                first = result.iat[0]
            last = result.iat[-1]
            observed_rows += len(result)
            del result
        return observed_rows, first, last

    return time_operation(operation)


# GroupBy Workloads (pandas)
def bench_groupby_sum_pandas(df: pd.DataFrame) -> list[float]:
    df = df.copy()
    df["key"] = (df["col_0"] % 100).astype("int64")
    return time_operation(lambda: df.groupby("key")["col_1"].sum())

def bench_groupby_mean_pandas(df: pd.DataFrame) -> list[float]:
    df = df.copy()
    df["key"] = (df["col_0"] % 100).astype("int64")
    return time_operation(lambda: df.groupby("key")["col_1"].mean())

def bench_groupby_agg_multi_pandas(df: pd.DataFrame) -> list[float]:
    df = df.copy()
    df["key"] = (df["col_0"] % 100).astype("int64")
    return time_operation(lambda: df.groupby("key").agg({"col_1": ["sum", "mean", "std"]}))

def bench_groupby_transform_mean_pandas(df: pd.DataFrame) -> list[float]:
    df = df.copy()
    df["key"] = (df["col_0"] % 100).astype("int64")
    return time_operation(lambda: df.groupby("key")["col_1"].transform("mean"))

def bench_groupby_mean_str_pandas(df: pd.DataFrame) -> list[float]:
    df = df.copy()
    df["key"] = ("g" + (df["col_0"] % 1000).astype("int64").map(lambda v: f"{v:04}"))
    return time_operation(lambda: df.groupby("key")["col_1"].mean())


def _groupby_str_op_pandas(df: pd.DataFrame, op):
    """Shared setup for str-keyed groupby aggregation benches: key =
    'g{col_0 % 1000:04}' (~1000 distinct), value = col_1 (matches fp-bench)."""
    df = df.copy()
    key_codes = (df["col_0"] % 1000).fillna(0).astype("int64")
    df["key"] = "g" + key_codes.map(lambda v: f"{v:04}")
    # fp-bench constructs SeriesGroupBy inside every timed iteration. Keep the
    # pandas call inline too: reusing `g` would cache its grouper after warmup
    # and compare reduction-only pandas against factorize-plus-reduce Rust.
    return time_operation(lambda: op(df.groupby("key")["col_1"]))


def bench_groupby_median_str_pandas(df: pd.DataFrame) -> list[float]:
    return _groupby_str_op_pandas(df, lambda g: g.median())


def bench_groupby_std_str_pandas(df: pd.DataFrame) -> list[float]:
    return _groupby_str_op_pandas(df, lambda g: g.std())


def bench_groupby_var_str_pandas(df: pd.DataFrame) -> list[float]:
    return _groupby_str_op_pandas(df, lambda g: g.var())


def bench_groupby_min_str_pandas(df: pd.DataFrame) -> list[float]:
    return _groupby_str_op_pandas(df, lambda g: g.min())


def bench_groupby_max_str_pandas(df: pd.DataFrame) -> list[float]:
    return _groupby_str_op_pandas(df, lambda g: g.max())


def bench_groupby_prod_str_pandas(df: pd.DataFrame) -> list[float]:
    return _groupby_str_op_pandas(df, lambda g: g.prod())


def bench_groupby_sem_str_pandas(df: pd.DataFrame) -> list[float]:
    return _groupby_str_op_pandas(df, lambda g: g.sem())


def bench_groupby_skew_str_pandas(df: pd.DataFrame) -> list[float]:
    return _groupby_str_op_pandas(df, lambda g: g.skew())


def bench_groupby_nunique_str_pandas(df: pd.DataFrame) -> list[float]:
    return _groupby_str_op_pandas(df, lambda g: g.nunique())


def bench_groupby_all_str_pandas(df: pd.DataFrame) -> list[float]:
    return _groupby_str_op_pandas(df, lambda g: g.all())


def bench_groupby_rank_str_pandas(df: pd.DataFrame) -> list[float]:
    return _groupby_str_op_pandas(
        df,
        lambda g: g.rank(method="average", ascending=True, na_option="keep"),
    )


def bench_groupby_kurt_str_pandas(df: pd.DataFrame) -> list[float]:
    # pandas 2.2.3 has no direct SeriesGroupBy.kurt method; public apply with
    # Series.kurt is its behavior-equivalent full-call counterpart.
    return _groupby_str_op_pandas(df, lambda g: g.apply(pd.Series.kurt))


def bench_groupby_quantile_str_pandas(df: pd.DataFrame) -> list[float]:
    return _groupby_str_op_pandas(df, lambda g: g.quantile(0.5))


def bench_groupby_unique_str_pandas(df: pd.DataFrame) -> list[float]:
    return _groupby_str_op_pandas(df, lambda g: g.unique())


def bench_groupby_unique_i64_pandas(df: pd.DataFrame) -> list[float]:
    df = df.copy()
    df["key"] = ("g" + (df["col_0"] % 1000).astype("int64").map(lambda v: f"{v:04}"))
    values = df["col_1"].astype("int64") % 50_000
    return time_operation(lambda: values.groupby(df["key"]).unique())


def bench_groupby_multi_str_pandas(df: pd.DataFrame) -> list[float]:
    # One grouper per timed iteration, reused for three reductions, matching
    # fp-bench's cached-within-call SeriesGroupBy shape.
    return _groupby_str_op_pandas(df, lambda g: (g.mean(), g.std(), g.var()))


def bench_groupby_agg3_str_pandas(df: pd.DataFrame) -> list[float]:
    return _groupby_str_op_pandas(df, lambda g: g.agg(["mean", "std", "max"]))


def bench_df_groupby_str_sum_pandas(df: pd.DataFrame) -> list[float]:
    n = len(df)
    gdf = pd.DataFrame(
        {
            "key": pd.Series(np.arange(n) % 1000).map(lambda v: f"g{v:04}"),
            "v0": df["col_0"],
            "v1": df["col_1"],
            "v2": df["col_2"],
        }
    )
    return time_operation(lambda: gdf.groupby("key")[["v0", "v1", "v2"]].sum())


def bench_df_groupby_2key_sum_pandas(df: pd.DataFrame) -> list[float]:
    n = len(df)
    rows = np.arange(n)
    value_cols = ["v0", "v1", "v2"]
    gdf = pd.DataFrame(
        {
            "k1": rows % 100,
            "k2": (rows // 100) % 50,
            "v0": df["col_0"],
            "v1": df["col_1"],
            "v2": df["col_2"],
        }
    )
    return time_operation(lambda: gdf.groupby(["k1", "k2"])[value_cols].sum())


def bench_df_groupby_2strkey_sum_pandas(df: pd.DataFrame) -> list[float]:
    n = len(df)
    rows = np.arange(n)
    value_cols = ["v0", "v1", "v2"]
    gdf = pd.DataFrame(
        {
            "k1": pd.Series(rows % 100).map(lambda v: f"a{v:03}"),
            "k2": pd.Series((rows // 100) % 50).map(lambda v: f"b{v:03}"),
            "v0": df["col_0"],
            "v1": df["col_1"],
            "v2": df["col_2"],
        }
    )
    return time_operation(lambda: gdf.groupby(["k1", "k2"])[value_cols].sum())


def bench_df_groupby_int_var_pandas(df: pd.DataFrame) -> list[float]:
    # Int key (i%1000, fast dense-histogram factorization) + 3 f64 value cols,
    # df.groupby(key).var() — matches fp-bench df_groupby_int_var. A loss here
    # is in the var computation, NOT factorization.
    df = df.copy()
    df["key"] = np.arange(len(df)) % 1000
    cols = ["col_0", "col_1", "col_2"]
    return time_operation(lambda: df.groupby("key")[cols].var())


def bench_df_groupby_int_mean_pandas(df: pd.DataFrame) -> list[float]:
    df = df.copy()
    df["key"] = np.arange(len(df)) % 1000
    cols = ["col_0", "col_1", "col_2"]
    return time_operation(lambda: df.groupby("key")[cols].mean())


def _widekey(n: int) -> "np.ndarray":
    # Matches fp-bench: (i * golden) as i64 >> 1 — ~n distinct keys, spread
    # across the i64 range (exercises the non-dense wide-i64 factorization).
    return (np.arange(n, dtype=np.uint64) * np.uint64(0x9E37_79B9_7F4A_7C15)).astype(
        np.int64
    ) >> 1


def bench_groupby_widekey_sum_pandas(df: pd.DataFrame) -> list[float]:
    df = df.copy()
    df["key"] = _widekey(len(df))
    return time_operation(lambda: df.groupby("key")["col_1"].sum())


def bench_df_groupby_widekey_sum_pandas(df: pd.DataFrame) -> list[float]:
    df = df.copy()
    df["key"] = _widekey(len(df))
    cols = ["col_0", "col_1", "col_2"]
    return time_operation(lambda: df.groupby("key")[cols].sum())

def bench_groupby_transform_mean_str_pandas(df: pd.DataFrame) -> list[float]:
    df = df.copy()
    df["key"] = ("g" + (df["col_0"] % 1000).astype("int64").map(lambda v: f"{v:04}"))
    return time_operation(lambda: df.groupby("key")["col_1"].transform("mean"))

def bench_groupby_cumcount_pandas(df: pd.DataFrame) -> list[float]:
    df = df.copy()
    df["key"] = (df["col_0"] % 100).astype("int64")
    return time_operation(lambda: df.groupby("key").cumcount())

def bench_groupby_count_pandas(df: pd.DataFrame) -> list[float]:
    df = df.copy()
    df["key"] = (df["col_0"] % 100).astype("int64")
    return time_operation(lambda: df.groupby("key")["col_1"].count())


# Rolling Workloads (pandas)
def bench_rolling_mean_w10_pandas(df: pd.DataFrame) -> list[float]:
    return time_operation(lambda: df["col_0"].rolling(10).mean())

def bench_rolling_std_w50_pandas(df: pd.DataFrame) -> list[float]:
    return time_operation(lambda: df["col_0"].rolling(50).std())


def bench_groupby_rolling_mean_w10_pandas(df: pd.DataFrame) -> list[float]:
    """Grouped rolling mean, flattened back onto the original row order.

    br-frankenpandas-u5cg4. FrankenPandas' ``SeriesGroupBy.rolling`` returns a
    FLAT series on the original index; pandas' ``groupby().rolling()`` returns a
    two-level (key, original-index) MultiIndex whose rows come out group-by-group.
    ``droplevel(0).sort_index()`` is what a pandas user writes to get
    FrankenPandas' answer, so it is part of the task, not overhead added to the
    incumbent — FrankenPandas pays the equivalent cost in its scatter.

    THE FASTER PANDAS ROUTE IS THE ONE MEASURED. The obvious alternative,
    ``groupby(key).transform(lambda x: x.rolling(10).mean())``, is a Python-level
    callback per group and is far slower; picking it would inflate the ratio by
    choosing a bad incumbent rather than by being fast.

    The key is derived from the ROW INDEX rather than from a value column so that
    it matches fp-bench's ``i % 100`` exactly without having to reproduce that
    crate's value generator.
    """
    key = pd.Series(np.arange(len(df)) % 100, index=df.index)
    series = df["col_0"]
    return time_operation(
        lambda: series.groupby(key).rolling(10).mean().droplevel(0).sort_index()
    )


def bench_rolling_apply_stateful_pandas(df: pd.DataFrame) -> PairedSamples:
    """Run the fastest pandas route for an ordered rolling callback.

    An eight-route 1M screen compared exact ``Rolling.apply`` with ``raw=True``
    and ``raw=False`` against task-equivalent ``rolling.sum()`` followed by
    ``Series.map``, ``Series.apply``, ``Series.transform``, ``np.fromiter``,
    ``itertools.accumulate``, and ``ufunc.accumulate``. Building the rolling
    sums once and driving a stateful generator into ``np.fromiter`` was
    fastest. Every screened route produced the same Float64 Series and final
    state.
    """
    window = 10
    mask = 0x7FFF_FFFF
    series = pd.Series(np.arange(len(df), dtype=np.int64) % 997, copy=False)

    def op():
        state = 0
        valid_sums = (
            series.rolling(window, min_periods=window)
            .sum()
            .to_numpy(copy=False)[window - 1 :]
        )

        def values():
            nonlocal state
            for value in valid_sums:
                state = (state * 31 + int(value)) & mask
                yield float(state)

        computed = np.fromiter(
            values(),
            dtype=np.float64,
            count=len(valid_sums),
        )
        output = np.empty(len(series), dtype=np.float64)
        output[: window - 1] = np.nan
        output[window - 1 :] = computed
        return pd.Series(output, index=series.index, copy=False), state

    return time_operation(op)


def bench_expanding_sum_pandas(df: pd.DataFrame) -> list[float]:
    return time_operation(lambda: df["col_0"].expanding().sum())


def bench_expanding_apply_stateful_pandas(df: pd.DataFrame) -> PairedSamples:
    """Run an ordered stateful callback over every growing prefix.

    An eight-route same-worker 1M screen found ``np.fromiter(map(...))`` over
    a stateful scalar callback fastest. It beat a generator-driven
    ``np.fromiter``, ``itertools.accumulate``, ``Series.map``,
    ``Series.apply``, ``Series.transform``, and exact ``Expanding.apply`` with
    both ``raw=True`` and ``raw=False``. Every route produced the same Float64
    Series and final state.
    """
    mask = 0x7FFF_FFFF
    series = pd.Series(np.arange(len(df), dtype=np.int64) % 997, copy=False)
    raw = series.to_numpy(copy=False)

    def op():
        state = 0
        prefix_len = 0

        def step(value):
            nonlocal state, prefix_len
            prefix_len += 1
            state = (state * 31 + int(value) + prefix_len) & mask
            return float(state)

        output = np.fromiter(map(step, raw), dtype=np.float64, count=len(raw))
        return pd.Series(output, index=series.index, copy=False), state

    return time_operation(op)


def bench_ewm_mean_pandas(df: pd.DataFrame) -> list[float]:
    return time_operation(lambda: df["col_0"].ewm(span=10).mean())


# Indexing Workloads (pandas)
def bench_iloc_slice_pandas(df: pd.DataFrame) -> list[float]:
    n = len(df)
    return time_operation(lambda: df.iloc[n//4:3*n//4])

def bench_loc_labels_pandas(df: pd.DataFrame) -> list[float]:
    df = df.set_index(pd.Index(range(len(df))))
    n = len(df)
    labels = list(range(n//4, 3*n//4))
    return time_operation(lambda: df.loc[labels])

def bench_reindex_pandas(df: pd.DataFrame) -> list[float]:
    n = len(df)
    new_index = pd.Index(range(0, n*2, 2))
    return time_operation(lambda: df.reindex(new_index))


def _range_take_positions(n: int) -> np.ndarray:
    start = n // 8
    return np.arange(start, n - start, 2, dtype=np.intp)


def bench_range_index_take_arithmetic_pandas(df: pd.DataFrame) -> list[float]:
    n = len(df)
    idx = pd.RangeIndex(10, 10 + n * 3, 3)
    positions = _range_take_positions(n)

    def op():
        result = None
        for _ in range(TAKE_BATCH):
            result = idx.take(positions)
        return result

    return time_operation(op)


def bench_affine_index_take_arithmetic_pandas(df: pd.DataFrame) -> list[float]:
    n = len(df)
    idx = pd.Index(np.arange(10, 10 + n * 3, 3, dtype=np.int64))
    positions = _range_take_positions(n)

    def op():
        result = None
        for _ in range(TAKE_BATCH):
            result = idx.take(positions)
        return result

    return time_operation(op)


def _build_join_frames(n: int):
    # Mirrors the fp-bench / criterion build_join_frames: left key 0..n, right
    # key 0,2,..,2(n-1) — a unique-key int64 join (inner keeps ~n/2 rows).
    left = pd.DataFrame({
        "key": np.arange(n, dtype=np.int64),
        "left_val": np.arange(n, dtype=np.float64),
    })
    right = pd.DataFrame({
        "key": np.arange(n, dtype=np.int64) * 2,
        "right_val": np.arange(n, dtype=np.float64) * 10.0,
    })
    return left, right

def bench_join_inner_pandas(df: pd.DataFrame) -> list[float]:
    left, right = _build_join_frames(len(df))
    return time_operation(lambda: left.merge(right, on="key", how="inner"))

def bench_join_left_pandas(df: pd.DataFrame) -> list[float]:
    left, right = _build_join_frames(len(df))
    return time_operation(lambda: left.merge(right, on="key", how="left"))

def bench_join_outer_pandas(df: pd.DataFrame) -> list[float]:
    left, right = _build_join_frames(len(df))
    return time_operation(lambda: left.merge(right, on="key", how="outer"))

def _build_str_join_frames(n: int):
    # String-key variant of _build_join_frames: left key "k{i:08}" (unique),
    # right key "k{2i:08}" — ~n/2 inner matches, exercises the Utf8 key path.
    left = pd.DataFrame({
        "key": [f"k{i:08}" for i in range(n)],
        "left_val": np.arange(n, dtype=np.float64),
    })
    right = pd.DataFrame({
        "key": [f"k{i*2:08}" for i in range(n)],
        "right_val": np.arange(n, dtype=np.float64),
    })
    return left, right

def bench_join_inner_str_pandas(df: pd.DataFrame) -> list[float]:
    left, right = _build_str_join_frames(len(df))
    return time_operation(lambda: left.merge(right, on="key", how="inner"))


def pandas_string_backend() -> str:
    """Which pandas string backend the incumbent arm should use.

    `object` is pandas 2.x's default and what this harness has always measured.
    It is ALSO pandas' slow path: measured here at 1M on this exact fixture,
    `string[pyarrow]` is 6.51x faster on sort_values, 3.21x on value_counts and
    1.89x on groupby-sum. Reporting only the object arm compares FrankenPandas
    against the incumbent's worst configuration -- the same class of error as
    quoting `df.apply(..., axis=1)` when `df.sum(axis=1)` exists. pandas 3 makes
    arrow-backed strings the default, so the object arm is a shrinking baseline.

    Set `FP_HARNESS_PANDAS_STRING_BACKEND=arrow` to measure against pandas' best.
    Run BOTH and report fp against the better one; keep object as a labelled
    secondary row, never as the headline. Per br-frankenpandas-ltmk9.
    """
    backend = (
        os.environ.get("FP_HARNESS_PANDAS_STRING_BACKEND", "object")
        .strip()
        .lower()
    )
    if backend not in {"object", "arrow"}:
        raise ValueError(
            f"FP_HARNESS_PANDAS_STRING_BACKEND must be 'object' or 'arrow', got {backend!r}"
        )
    return backend


def _as_string_column(values: list[str], backend: str):
    """Build one key/name column in an explicitly named pandas backend."""
    if backend == "arrow":
        # Fails loudly rather than silently degrading to object: a silent
        # fallback would report an object-arm number as an arrow-arm result.
        return pd.array(values, dtype="string[pyarrow]")
    if backend == "object":
        return values
    raise ValueError(f"unknown pandas string backend: {backend!r}")


def _build_str_frame(n: int, backend: str | None = None) -> pd.DataFrame:
    # Mirrors fp-bench build_str_frame: key = ~1000-distinct group label,
    # name = unique ~15-byte id (sort key), val = float64.
    selected_backend = backend or pandas_string_backend()
    keys = [f"g{i % 1000:04d}" for i in range(n)]
    names = [f"item_{i:010d}" for i in range(n)]
    return pd.DataFrame({
        "key": _as_string_column(keys, selected_backend),
        "name": _as_string_column(names, selected_backend),
        "val": np.arange(n, dtype=np.float64),
    })


def _bench_str_sort_pandas(df: pd.DataFrame, backend: str) -> PairedSamples:
    f = _build_str_frame(len(df), backend)
    return time_operation(lambda: f.sort_values("name"))


def bench_str_sort_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_str_sort_pandas(df, pandas_string_backend())


def bench_str_sort_object_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_str_sort_pandas(df, "object")


def bench_str_sort_arrow_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_str_sort_pandas(df, "arrow")


def _bench_str_value_counts_pandas(
    df: pd.DataFrame,
    backend: str,
) -> PairedSamples:
    f = _build_str_frame(len(df), backend)
    return time_operation(lambda: f["key"].value_counts())


def bench_str_value_counts_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_str_value_counts_pandas(df, pandas_string_backend())


def bench_str_value_counts_object_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_str_value_counts_pandas(df, "object")


def bench_str_value_counts_arrow_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_str_value_counts_pandas(df, "arrow")


def _bench_str_groupby_sum_pandas(
    df: pd.DataFrame,
    backend: str,
) -> PairedSamples:
    f = _build_str_frame(len(df), backend)
    return time_operation(lambda: f.groupby("key")["val"].sum())


def bench_str_groupby_sum_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_str_groupby_sum_pandas(df, pandas_string_backend())


def bench_str_groupby_sum_object_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_str_groupby_sum_pandas(df, "object")


def bench_str_groupby_sum_arrow_pandas(df: pd.DataFrame) -> PairedSamples:
    return _bench_str_groupby_sum_pandas(df, "arrow")


def bench_str_len_arrow_pandas(df: pd.DataFrame) -> PairedSamples:
    """Measure character counts on pandas' Arrow-backed string storage."""
    names = [f"item_{i:010d}" for i in range(len(df))]
    series = pd.Series(_as_string_column(names, "arrow"))
    return time_operation(lambda: series.str.len())


def bench_str_contains_arrow_pandas(df: pd.DataFrame) -> PairedSamples:
    """Run literal contains on the fastest screened pandas string backend.

    A same-worker 1M screen compared object, ``string[python]``, and
    ``string[pyarrow]`` storage. Arrow was fastest, and all three produced the
    same boolean output. ``regex=False`` matches FrankenPandas' literal
    substring contract without charging pandas for an unnecessary regex.
    """
    names = [f"item_{i:010d}" for i in range(len(df))]
    series = pd.Series(_as_string_column(names, "arrow"))
    return time_operation(lambda: series.str.contains("5", regex=False))


def bench_str_startswith_arrow_pandas(df: pd.DataFrame) -> PairedSamples:
    """Run prefix matching on the fastest screened pandas string backend.

    A same-worker 1M screen compared object, ``string[python]``, and
    ``string[pyarrow]`` storage. Arrow was fastest, and all three produced the
    same all-true boolean output for the exact benchmark names.
    """
    names = [f"item_{i:010d}" for i in range(len(df))]
    series = pd.Series(_as_string_column(names, "arrow"))
    return time_operation(lambda: series.str.startswith("item"))


def bench_df_dot_pandas(df: pd.DataFrame) -> list[float]:
    import math
    dim = math.isqrt(len(df))
    m = pd.DataFrame(np.random.default_rng(7).random((dim, dim)))
    return time_operation(lambda: m.dot(m))


def bench_to_datetime_pandas(df: pd.DataFrame) -> list[float]:
    n = len(df)
    s = pd.Series([f"2020-01-{i % 28 + 1:02d}" for i in range(n)])
    return time_operation(lambda: pd.to_datetime(s))


def bench_dt_floor_pandas(df: pd.DataFrame) -> list[float]:
    n = len(df)
    s = pd.Series(pd.date_range("2000-01-01", periods=n, freq="37s"))
    return time_operation(lambda: s.dt.floor("D"))


# `600s` (10 min) exactly matches fp-bench's in-range nanosecond generator
# through 10M rows. `dt_strftime` is the representation-equivalent incumbent:
# both engines emit `%Y-%m-%d` strings. The same-named pandas `.dt.date` /
# `.dt.time` arms below return Python object arrays, while FrankenPandas returns
# ISO-8601 Utf8, so those two remain diagnostic-only and must not be gated.
def bench_dt_strftime_pandas(df: pd.DataFrame) -> PairedSamples:
    n = len(df)
    s = pd.Series(pd.date_range("2000-01-01", periods=n, freq="600s"))
    return time_operation(lambda: s.dt.strftime("%Y-%m-%d"))


def bench_dt_date_pandas(df: pd.DataFrame) -> list[float]:
    n = len(df)
    s = pd.Series(pd.date_range("2000-01-01", periods=n, freq="600s"))
    return time_operation(lambda: s.dt.date)


def bench_dt_time_pandas(df: pd.DataFrame) -> list[float]:
    n = len(df)
    s = pd.Series(pd.date_range("2000-01-01", periods=n, freq="600s"))
    return time_operation(lambda: s.dt.time)


def bench_dt_day_name_pandas(df: pd.DataFrame) -> PairedSamples:
    """Emit English weekday names through the fastest screened pandas route.

    On the exact 1M-row fixture, ``dt.dayofweek`` followed by a NumPy object
    gather and full Series construction was 10.4x faster than
    ``dt.day_name()`` and produced an exactly equal Series. The name table and
    Datetime64 population remain outside timing.
    """
    n = len(df)
    series = pd.Series(
        pd.date_range("2000-01-01", periods=n, freq="600s"),
        name="d",
    )
    day_names = np.asarray(
        [
            "Monday",
            "Tuesday",
            "Wednesday",
            "Thursday",
            "Friday",
            "Saturday",
            "Sunday",
        ],
        dtype=object,
    )

    def operation():
        codes = series.dt.dayofweek.to_numpy(copy=False)
        return pd.Series(day_names[codes], index=series.index, name=series.name)

    return time_operation(operation)


def bench_dt_month_name_pandas(df: pd.DataFrame) -> PairedSamples:
    """Emit English month names through the fastest screened pandas route.

    On the exact 1M-row fixture, the public DatetimeArray ``month`` property
    followed by a NumPy object gather and full Series construction was 6.9x
    faster than ``dt.month_name()`` and produced an exactly equal Series. The
    name table and Datetime64 population remain outside timing.
    """
    n = len(df)
    series = pd.Series(
        pd.date_range("2000-01-01", periods=n, freq="600s"),
        name="d",
    )
    month_names = np.asarray(
        [
            "January",
            "February",
            "March",
            "April",
            "May",
            "June",
            "July",
            "August",
            "September",
            "October",
            "November",
            "December",
        ],
        dtype=object,
    )

    def operation():
        codes = series.array.month
        return pd.Series(
            month_names[codes - 1],
            index=series.index,
            name=series.name,
        )

    return time_operation(operation)


PANDAS_WORKLOADS = {
    "io": {
        "csv_read": bench_csv_read_pandas,
        "csv_read_block_view": bench_csv_read_block_view_pandas,
        "csv_write": bench_csv_write_pandas,
        "json_read_records": bench_json_read_records_pandas,
        "json_read_columns": bench_json_read_columns_pandas,
        "json_read_index": bench_json_read_index_pandas,
        "json_read_split": bench_json_read_split_pandas,
        "json_read_values": bench_json_read_values_pandas,
        "parquet_read": bench_parquet_read_pandas,
        "parquet_write": bench_parquet_write_pandas,
    },
    "pipeline": {
        "etl_job": bench_pipeline_etl_job_pandas,
        "etl_job_parquet": bench_pipeline_etl_job_parquet_pandas,
    },
    "math_unary": {
        "floor": bench_math_floor_pandas,
        "ceil": bench_math_ceil_pandas,
        "trunc": bench_math_trunc_pandas,
        "round2": bench_math_round2_pandas,
        "sqrt": bench_math_sqrt_pandas,
        "log": bench_math_log_pandas,
        "log10": bench_math_log10_pandas,
        "log2": bench_math_log2_pandas,
        "log1p": bench_math_log1p_pandas,
        "pow": bench_math_pow_pandas,
        "atan2": bench_math_atan2_pandas,
        "hypot": bench_math_hypot_pandas,
        "mod": bench_math_mod_pandas,
        "floordiv": bench_math_floordiv_pandas,
        "add": bench_math_add_pandas,
        "div": bench_math_div_pandas,
        "sqrt_int64": bench_math_sqrt_int64_pandas,
        "log_int64": bench_math_log_int64_pandas,
        "expm1": bench_math_expm1_pandas,
        "cbrt": bench_math_cbrt_pandas,
        "sin": bench_math_sin_pandas,
        "atan": bench_math_atan_pandas,
    },
    "dataframe_ops": {
        "sort_values_single": bench_sort_values_single_pandas,
        "sort_values_multi": bench_sort_values_multi_pandas,
        "filter_bool_mask": bench_filter_bool_mask_pandas,
        "drop_duplicates": bench_drop_duplicates_pandas,
        "value_counts": bench_value_counts_pandas,
        "cumsum": bench_cumsum_pandas,
        "df_abs": bench_df_abs_pandas,
        "df_melt": bench_df_melt_pandas,
        "df_transpose": bench_df_transpose_pandas,
        "df_transpose_materialize": bench_df_transpose_materialize_pandas,
        "df_to_dict_index_materialize": bench_df_to_dict_index_materialize_pandas,
        "astype_str_f64": bench_astype_str_f64_pandas,
        "astype_str_f64_telemetry_batches": (
            bench_astype_str_f64_telemetry_batches_pandas
        ),
        "df_iterrows": bench_df_iterrows_pandas,
        "df_itertuples": bench_df_itertuples_pandas,
        "df_row_tuples_fastest": bench_df_row_tuples_fastest_pandas,
        "df_apply_row": bench_df_apply_row_pandas,
        "series_apply_stateful": bench_series_apply_stateful_pandas,
        "df_pivot": bench_df_pivot_pandas,
        "df_pivot_table": bench_df_pivot_table_pandas,
        "df_explode": bench_df_explode_pandas,
        "df_explode_string_python": bench_df_explode_string_python_pandas,
        "df_explode_string_arrow": bench_df_explode_string_arrow_pandas,
    },
    "groupby": {
        "groupby_sum_int64": bench_groupby_sum_pandas,
        "groupby_mean_float64": bench_groupby_mean_pandas,
        "groupby_agg_multi": bench_groupby_agg_multi_pandas,
        "groupby_mean_str": bench_groupby_mean_str_pandas,
        "groupby_transform_mean": bench_groupby_transform_mean_pandas,
        "groupby_transform_mean_str": bench_groupby_transform_mean_str_pandas,
        "groupby_cumcount": bench_groupby_cumcount_pandas,
        "groupby_count": bench_groupby_count_pandas,
        "groupby_median_str": bench_groupby_median_str_pandas,
        "groupby_std_str": bench_groupby_std_str_pandas,
        "groupby_var_str": bench_groupby_var_str_pandas,
        "groupby_min_str": bench_groupby_min_str_pandas,
        "groupby_max_str": bench_groupby_max_str_pandas,
        "groupby_prod_str": bench_groupby_prod_str_pandas,
        "groupby_sem_str": bench_groupby_sem_str_pandas,
        "groupby_skew_str": bench_groupby_skew_str_pandas,
        "groupby_nunique_str": bench_groupby_nunique_str_pandas,
        "groupby_all_str": bench_groupby_all_str_pandas,
        "groupby_rank_str": bench_groupby_rank_str_pandas,
        "groupby_kurt_str": bench_groupby_kurt_str_pandas,
        "groupby_quantile_str": bench_groupby_quantile_str_pandas,
        "groupby_unique_str": bench_groupby_unique_str_pandas,
        "groupby_unique_i64": bench_groupby_unique_i64_pandas,
        "groupby_multi_str": bench_groupby_multi_str_pandas,
        "groupby_agg3_str": bench_groupby_agg3_str_pandas,
        "df_groupby_str_sum": bench_df_groupby_str_sum_pandas,
        "df_groupby_2key_sum": bench_df_groupby_2key_sum_pandas,
        "df_groupby_2strkey_sum": bench_df_groupby_2strkey_sum_pandas,
        "df_groupby_int_var": bench_df_groupby_int_var_pandas,
        "df_groupby_int_mean": bench_df_groupby_int_mean_pandas,
        "groupby_widekey_sum": bench_groupby_widekey_sum_pandas,
        "df_groupby_widekey_sum": bench_df_groupby_widekey_sum_pandas,
    },
    "rolling": {
        "rolling_mean_w10": bench_rolling_mean_w10_pandas,
        "rolling_std_w50": bench_rolling_std_w50_pandas,
        "groupby_rolling_mean_w10": bench_groupby_rolling_mean_w10_pandas,
        "rolling_apply_stateful": bench_rolling_apply_stateful_pandas,
        "expanding_sum": bench_expanding_sum_pandas,
        "expanding_apply_stateful": bench_expanding_apply_stateful_pandas,
        "ewm_mean": bench_ewm_mean_pandas,
    },
    "indexing": {
        "iloc_slice": bench_iloc_slice_pandas,
        "loc_labels": bench_loc_labels_pandas,
        "reindex": bench_reindex_pandas,
        "range_index_take_arithmetic": bench_range_index_take_arithmetic_pandas,
        "affine_index_take_arithmetic": bench_affine_index_take_arithmetic_pandas,
    },
    "joins": {
        "join_inner": bench_join_inner_pandas,
        "join_left": bench_join_left_pandas,
        "join_outer": bench_join_outer_pandas,
        "join_inner_str": bench_join_inner_str_pandas,
    },
    "strings": {
        "str_sort": bench_str_sort_pandas,
        "str_sort_object": bench_str_sort_object_pandas,
        "str_sort_arrow": bench_str_sort_arrow_pandas,
        "str_value_counts": bench_str_value_counts_pandas,
        "str_value_counts_object": bench_str_value_counts_object_pandas,
        "str_value_counts_arrow": bench_str_value_counts_arrow_pandas,
        "str_groupby_sum": bench_str_groupby_sum_pandas,
        "str_groupby_sum_object": bench_str_groupby_sum_object_pandas,
        "str_groupby_sum_arrow": bench_str_groupby_sum_arrow_pandas,
        "str_len": bench_str_len_arrow_pandas,
        "str_contains_arrow": bench_str_contains_arrow_pandas,
        "str_startswith_arrow": bench_str_startswith_arrow_pandas,
    },
    "linalg": {
        "df_dot": bench_df_dot_pandas,
    },
    "datetime": {
        "to_datetime": bench_to_datetime_pandas,
        "dt_floor": bench_dt_floor_pandas,
        "dt_strftime": bench_dt_strftime_pandas,
        "dt_date": bench_dt_date_pandas,
        "dt_time": bench_dt_time_pandas,
        "dt_day_name": bench_dt_day_name_pandas,
        "dt_month_name": bench_dt_month_name_pandas,
    },
}


def run_pandas_workload(
    category: str,
    workload: str,
    size: str,
    dtype: str,
    tmp_path: Path,
    fingerprint: dict[str, Any],
    exclusivity_gate: HostWideExclusivityGate | None,
) -> tuple[TimingResult, dict[str, Any]]:
    """Run a single pandas workload and return timing result."""
    config = SIZE_CONFIGS[size]
    if category in ("pipeline", "math_unary"):
        # These build their own exact inputs. The pipeline workload writes them
        # to CSV, and reads them back inside the timed job. It never touches
        # the synthetic ten-column frame; populating one at 10M rows would
        # cost ~800 MB for nothing and leave setup allocator work that can
        # outlive the settle window and trip the immediate pre-arm host gate.
        df = pd.DataFrame(index=pd.RangeIndex(config["rows"]))
    elif category == "dataframe_ops" and workload in {
        "astype_str_f64",
        "astype_str_f64_telemetry_batches",
    }:
        # The workload constructs its exact one-column Float64 Series below and
        # uses only this frame's row count. Avoid populating an unrelated dense
        # 10-column frame whose setup-only allocator work can outlive the
        # settle window and correctly trip the immediate pre-arm host gate.
        df = pd.DataFrame(index=pd.RangeIndex(config["rows"]))
    else:
        df = generate_test_data(config["rows"], config["cols"], dtype)
    bench_func = PANDAS_WORKLOADS[category][workload]
    if category in ("io", "pipeline"):
        operation = partial(bench_func, df, tmp_path)
    else:
        operation = partial(bench_func, df)
    if exclusivity_gate is None:
        # Balanced-square mode uses temporal pairing rather than a global
        # machine-idle predicate.  Do not insert a one-second idle wait here:
        # it would merely make co-tenant drift asymmetric between slots.
        samples = operation()
        quiescence = {
            "mode": "balanced_square",
            "valid": True,
            "host_wide_quiescence_required": False,
        }
    else:
        # Population is outside the timed arm but can leave short-lived
        # allocator or kernel page work on CPUs outside this process's
        # affinity. Let that setup-only activity drain before the mandatory
        # immediate pre-arm sample; the sample still fails closed if any work
        # remains.
        time.sleep(SETUP_QUIESCENCE_SETTLE_SECONDS)
        samples, quiescence = _run_host_exclusive_arm(
            exclusivity_gate,
            f"pandas:{category}/{workload}/{size}/{dtype}",
            operation,
        )

    if not isinstance(samples, PairedSamples):
        raise TypeError(f"{category}/{workload} did not use the paired timing contract")
    identity = executable_identity(Path(sys.executable))
    return (
        TimingResult(
            workload=workload,
            category=category,
            size=size,
            dtype=dtype,
            engine="pandas",
            times_us=samples.times_us,
            null_arm_a_us=samples.null_arm_a_us,
            null_arm_b_us=samples.null_arm_b_us,
            null_ratios=samples.null_ratios,
            checksum=f"{samples.checksum:016x}",
            executable_sha256=identity["sha256"],
            executable_bytes=identity["bytes"],
            executable_path=identity["path"],
            runtime_available_parallelism=samples.runtime_available_parallelism,
            process_threads_before_probe=samples.process_threads_before_probe,
            peak_process_threads=samples.peak_process_threads,
            operation_threads_used=samples.operation_threads_used,
            runtime_detected_isa_features=fingerprint[
                "runtime_detected_isa_features"
            ],
        ),
        quiescence,
    )


def run_fp_workload_subprocess(
    category: str,
    workload: str,
    size: str,
    dtype: str,
    data_dir: Path | None = None,
    bench_binary_override: Path | None = None,
) -> TimingResult:
    """Run FrankenPandas workload via subprocess."""
    if bench_binary_override is None:
        # Respect CARGO_TARGET_DIR (rch/remote builds set a custom target dir);
        # fall back to the in-tree ./target.
        target_dir = Path(
            os.environ.get("CARGO_TARGET_DIR", str(PROJECT_ROOT / "target"))
        )
        bench_binary = target_dir / "release-perf" / "fp-bench"
    else:
        # An explicit path lets a whole-binary experiment retain two immutable
        # RCH-built ELFs without minting two Cargo target directories. The
        # subprocess still proves the executing file through its mandatory
        # line-one self-hash, and shell execution remains disabled below.
        bench_binary = bench_binary_override

    if not bench_binary.exists():
        print(f"[WARN] fp-bench binary not found at {bench_binary}", file=sys.stderr)
        print("[INFO] Skipping FrankenPandas workload - build with:", file=sys.stderr)
        print("  cargo build --profile release-perf -p fp-bench", file=sys.stderr)
        return TimingResult(
            workload=workload,
            category=category,
            size=size,
            dtype=dtype,
            engine="frankenpandas",
            times_us=[],
        )

    bench_binary = bench_binary.resolve(strict=True)
    # Confine the executable to a trusted root. The project root is one; the
    # configured CARGO_TARGET_DIR is the other, and it is trusted for the same
    # reason this function honours it three lines above -- rch/remote builds and
    # the shared-target-dir disk policy both point it outside the repo, and the
    # in-tree ./target/release-perf/fp-bench is itself a symlink into it. Checking
    # the project root ALONE rejects every valid configuration on this host
    # (CARGO_TARGET_DIR=/data/tmp/cargo-target is set session-wide), which blocks
    # all vs-incumbent measurement. Arbitrary paths are still refused.
    trusted_roots = [PROJECT_ROOT.resolve(strict=True)]
    configured_target = os.environ.get("CARGO_TARGET_DIR")
    if configured_target:
        try:
            trusted_roots.append(Path(configured_target).resolve(strict=True))
        except OSError:
            pass
    explicit_binary = bench_binary_override is not None
    if not explicit_binary and not any(
        bench_binary.is_relative_to(root) for root in trusted_roots
    ):
        raise ValueError(
            "Refusing fp-bench executable outside the project root and the "
            f"configured CARGO_TARGET_DIR: {bench_binary}"
        )
    if not (
        bench_binary.name == "fp-bench"
        or (explicit_binary and bench_binary.name.startswith("fp-bench-"))
    ):
        raise ValueError(f"Unexpected fp-bench executable path: {bench_binary}")

    child_env = os.environ.copy()
    if workload == "astype_str_f64_telemetry_batches":
        # The linked mimalloc v2 otherwise schedules purges after a delay.
        # This workload explicitly drops each rendered batch inside the timed
        # closure, so purge immediately at that free boundary as well. A
        # delayed purge can escape both the timer and the child lifetime and
        # correctly trip the mandatory all-CPU post-arm gate.
        child_env["MIMALLOC_PURGE_DELAY"] = TELEMETRY_MIMALLOC_PURGE_DELAY_MS

    # nosec B603: fp-bench is resolved, confined to the project root, and
    # name-checked above; shell=False and category/workload values are selected
    # from the static workload matrix.
    argv = [str(bench_binary), "--category", category, "--workload", workload,
            "--size", size, "--dtype", dtype, "--json"]
    # br-frankenpandas-633fb: pin the child to the SAME cpus the pandas arm runs
    # on. `taskset` is preferred over preexec_fn because it is safe regardless of
    # this process's thread state; if it is unavailable the child still inherits
    # the mask, and the row records `mask_source` so the difference is visible.
    mask_spec = cpu_mask_spec(arm_cpu_mask())
    taskset = shutil.which("taskset")
    if mask_spec and taskset:
        argv = [taskset, "-c", mask_spec, *argv]
    if data_dir is not None:
        # Only the pipeline category consumes this. The pandas arm has already
        # materialized the job's inputs here, so both engines read identical
        # bytes and each writes an output the driver can diff.
        argv += ["--data-dir", str(data_dir)]

    result = subprocess.run(
        argv,
        capture_output=True,
        check=False,
        env=child_env,
        text=True,
        timeout=1800 if size == "10M" else 300,
    )

    if result.returncode != 0:
        print(f"[WARN] fp-bench failed: {result.stderr}", file=sys.stderr)
        return TimingResult(
            workload=workload,
            category=category,
            size=size,
            dtype=dtype,
            engine="frankenpandas",
            times_us=[],
        )

    output_lines = result.stdout.splitlines()
    if not output_lines:
        print("[WARN] fp-bench emitted no stdout", file=sys.stderr)
        return TimingResult(
            workload=workload,
            category=category,
            size=size,
            dtype=dtype,
            engine="frankenpandas",
            times_us=[],
        )

    identity_match = re.fullmatch(
        r"bench_elf_sha256=([0-9a-f]{64}) \((\d+) bytes\) (.+)",
        output_lines[0],
    )
    if identity_match is None:
        print(
            f"[WARN] fp-bench missing line-1 ELF identity: {output_lines[0]!r}",
            file=sys.stderr,
        )
        return TimingResult(
            workload=workload,
            category=category,
            size=size,
            dtype=dtype,
            engine="frankenpandas",
            times_us=[],
        )

    try:
        data = json.loads("\n".join(output_lines[1:]))
    except JSONDecodeError as exc:
        print(f"[WARN] fp-bench emitted invalid JSON: {exc}", file=sys.stderr)
        return TimingResult(
            workload=workload,
            category=category,
            size=size,
            dtype=dtype,
            engine="frankenpandas",
            times_us=[],
        )

    null_control = data.get("null_control", {})
    thread_provenance = data.get("thread_provenance", {})
    return TimingResult(
        workload=workload,
        category=category,
        size=size,
        dtype=dtype,
        engine="frankenpandas",
        times_us=data["times_us"],
        null_arm_a_us=null_control.get("arm_a_times_us", []),
        null_arm_b_us=null_control.get("arm_b_times_us", []),
        null_ratios=null_control.get("ratios", []),
        checksum=data.get("checksum"),
        executable_sha256=identity_match.group(1),
        executable_bytes=int(identity_match.group(2)),
        executable_path=identity_match.group(3),
        runtime_available_parallelism=thread_provenance.get(
            "runtime_available_parallelism"
        ),
        process_threads_before_probe=thread_provenance.get(
            "process_threads_before_probe"
        ),
        peak_process_threads=thread_provenance.get("peak_process_threads"),
        operation_threads_used=thread_provenance.get("operation_threads_used"),
        runtime_detected_isa_features=thread_provenance.get(
            "runtime_detected_isa_features",
            [],
        ),
    )


def _balanced_square_aggregate(
    slots: list[TimingResult],
    *,
    engine: str,
) -> TimingResult:
    """Turn one arm's balanced-square slots into a contract-valid sample.

    Each slot already includes its engine-local alternating A/A control.  This
    aggregate adds the outer A/A control required for a busy-host comparison:
    the first two and last two placements of the same arm in each square must
    agree.  Failing either control leaves the row NULL_UNDECIDABLE.
    """
    if len(slots) < 4 or len(slots) % 4:
        raise ValueError("balanced square needs four slots per arm per round")
    first = slots[0]
    if any(not slot.is_valid for slot in slots):
        raise RuntimeError(f"{engine} emitted an invalid balanced-square slot")
    identity_fields = (
        "workload",
        "category",
        "size",
        "dtype",
        "engine",
        "executable_sha256",
        "executable_bytes",
        "executable_path",
    )
    for slot in slots[1:]:
        if any(getattr(slot, field) != getattr(first, field) for field in identity_fields):
            raise RuntimeError(f"{engine} identity changed inside balanced square")

    slot_p50s = [slot.p50_us for slot in slots]
    null_arm_a_us = [
        float(np.median(slot_p50s[offset : offset + 2]))
        for offset in range(0, len(slot_p50s), 4)
    ]
    null_arm_b_us = [
        float(np.median(slot_p50s[offset + 2 : offset + 4]))
        for offset in range(0, len(slot_p50s), 4)
    ]
    checksums = "|".join(str(slot.checksum) for slot in slots).encode("utf-8")
    return TimingResult(
        workload=first.workload,
        category=first.category,
        size=first.size,
        dtype=first.dtype,
        engine=first.engine,
        times_us=slot_p50s,
        null_arm_a_us=null_arm_a_us,
        null_arm_b_us=null_arm_b_us,
        null_ratios=[left / right for left, right in zip(null_arm_a_us, null_arm_b_us)],
        checksum=hashlib.sha256(checksums).hexdigest(),
        executable_sha256=first.executable_sha256,
        executable_bytes=first.executable_bytes,
        executable_path=first.executable_path,
        runtime_available_parallelism=first.runtime_available_parallelism,
        process_threads_before_probe=first.process_threads_before_probe,
        peak_process_threads=max(
            slot.peak_process_threads or 0 for slot in slots
        ),
        operation_threads_used=max(slot.operation_threads_used or 0 for slot in slots),
        runtime_detected_isa_features=first.runtime_detected_isa_features,
    )


ARM_CPU_MASK: list[int] | None = None
ARM_CPU_MASK_SOURCE = "inherited"


def cpu_mask_spec(cpus: list[int]) -> str:
    """Compact "0-7,16-23" rendering of a CPU set."""
    if not cpus:
        return ""
    ordered = sorted(cpus)
    parts: list[str] = []
    start = previous = ordered[0]
    for cpu in ordered[1:]:
        if cpu == previous + 1:
            previous = cpu
            continue
        parts.append(str(start) if start == previous else f"{start}-{previous}")
        start = previous = cpu
    parts.append(str(start) if start == previous else f"{start}-{previous}")
    return ",".join(parts)


def parse_cpu_spec(spec: str) -> list[int]:
    """Inverse of `cpu_mask_spec`, for --pin-cpus.

    Also accepts the literal `one-per-core`, which resolves to the lowest logical
    CPU of each physical core. br-frankenpandas-633fb: the DEFAULT mask on this
    host folds SMT — 64 logical over 32 physical, `one_thread_per_core: false` —
    so an arm can be placed on a sibling of a core the other arm is using. A
    caller who wants that impossible needs a portable way to ask for it, and
    hand-writing `0-31` is exactly the non-portable assumption that put both of
    frankenfs' arms on one core.
    """
    if spec.strip() == "one-per-core":
        topo = cpu_topology()
        first: dict[int, int] = {}
        for cpu in sorted(topo):
            first.setdefault(topo[cpu], cpu)
        return sorted(first.values())
    cpus: set[int] = set()
    for chunk in spec.split(","):
        chunk = chunk.strip()
        if not chunk:
            continue
        if "-" in chunk:
            low, high = chunk.split("-", 1)
            cpus.update(range(int(low), int(high) + 1))
        else:
            cpus.add(int(chunk))
    return sorted(cpus)


def arm_cpu_mask() -> list[int]:
    """The CPU set BOTH arms are made to run on.

    br-frankenpandas-633fb. A ratio whose arms sat on differently-clocked cores is
    a frequency ratio in disguise, and inheritance is not enforcement: the pandas
    arm runs in this process and the FrankenPandas arm in a child, so without an
    explicit step the row can only assert that they matched. This returns the mask
    that is applied to BOTH — reasserted on the child at exec — so the row can say
    HOW it was ensured rather than that it was observed afterwards.
    """
    if ARM_CPU_MASK is not None:
        return list(ARM_CPU_MASK)
    try:
        return sorted(os.sched_getaffinity(0))
    except (AttributeError, OSError):
        return []


def arm_core_placement() -> dict[str, Any]:
    """How both arms were made to run on comparable cores, for the row."""
    mask = arm_cpu_mask()
    spec = cpu_mask_spec(mask)
    mechanism = (
        f"identical CPU mask applied to both arms ({ARM_CPU_MASK_SOURCE}); "
        f"the pandas arm runs in this process and the FrankenPandas child is "
        f"launched under `taskset -c {spec}`, so neither arm can be scheduled "
        f"onto cores the other was excluded from"
    )
    return {
        "cpu_mask": spec,
        "cpus": len(mask),
        "mask_source": ARM_CPU_MASK_SOURCE,
        "same_cores_ensured_by": mechanism,
        "mask_composition": mask_composition(mask),
    }


def cpu_topology() -> dict[int, int]:
    """cpu -> physical core_id. Empty when the kernel does not expose topology."""
    mapping: dict[int, int] = {}
    try:
        cpus = sorted(os.sched_getaffinity(0))
    except (AttributeError, OSError):
        return mapping
    for cpu in cpus:
        try:
            with open(
                f"/sys/devices/system/cpu/cpu{cpu}/topology/core_id", encoding="utf-8"
            ) as handle:
                mapping[cpu] = int(handle.read().strip())
        except (OSError, ValueError):
            continue
    return mapping


def mask_composition(mask: list[int]) -> dict[str, Any]:
    """How many PHYSICAL cores a logical CPU mask actually buys.

    br-frankenpandas-633fb. frankenfs found both its arms sharing ONE physical
    core. The trap is that CPU numbering is not portable: on this host siblings
    are `n` and `n+32`, so `0-7` is eight distinct cores — but on a host numbered
    `0,1 = one core` the same spec is four cores with both arms fighting over each
    one's execution units. A mask must therefore be reported by its PHYSICAL
    composition, never by its logical width.
    """
    topo = cpu_topology()
    if not topo:
        return {"logical": len(mask), "physical_cores": None, "topology_available": False}
    cores: dict[int, list[int]] = {}
    for cpu in mask:
        core = topo.get(cpu)
        if core is not None:
            cores.setdefault(core, []).append(cpu)
    folded = {core: cpus for core, cpus in cores.items() if len(cpus) > 1}
    return {
        "logical": len(mask),
        "physical_cores": len(cores),
        "smt_folded_cores": len(folded),
        "smt_sibling_pairs": sorted(sorted(v) for v in folded.values())[:8],
        "one_thread_per_core": not folded,
        "topology_available": True,
    }


def self_thread_cpus() -> list[int]:
    """CPUs this process's own threads last ran on, from /proc/self/task/*/stat.

    br-frankenpandas-633fb. Exact for the pandas arm, which runs INSIDE this
    process. The /proc/stat delta method cannot be exact on a shared box — it
    reports the busiest CPUs during a slot, which may belong to another tenant,
    and a --pin-cpus 0-31 run proved it by attributing cpus 51 and 39 to arms that
    could not possibly have run there.
    """
    cpus: list[int] = []
    try:
        for entry in os.listdir("/proc/self/task"):
            try:
                with open(f"/proc/self/task/{entry}/stat", encoding="utf-8") as handle:
                    fields = handle.read().rsplit(") ", 1)[-1].split()
                cpus.append(int(fields[36]))
            except (OSError, ValueError, IndexError):
                continue
    except OSError:
        return []
    return sorted(set(cpus))


def cpu_busy_snapshot() -> dict[int, int]:
    """Per-CPU busy jiffies from /proc/stat (user+nice+system+irq+softirq+steal)."""
    busy: dict[int, int] = {}
    try:
        with open("/proc/stat", encoding="utf-8") as handle:
            for line in handle:
                if not line.startswith("cpu") or line.startswith("cpu "):
                    continue
                parts = line.split()
                try:
                    cpu = int(parts[0][3:])
                except ValueError:
                    continue
                fields = [int(v) for v in parts[1:9]]
                # user, nice, system, [idle], [iowait], irq, softirq, steal
                busy[cpu] = fields[0] + fields[1] + fields[2] + sum(fields[5:8])
    except (OSError, ValueError, IndexError):
        return {}
    return busy


def summarize_arm_cpus(
    busy_by_arm: dict[str, dict[int, int]],
    threads_by_arm: dict[str, int] | None,
) -> dict[str, Any]:
    """Which CPUs each arm actually ran on, and whether the arms collided.

    br-frankenpandas-633fb. Attribution is by /proc/stat busy-jiffy delta across
    each arm's slots, so it reports the CPUs MOST ACTIVE while that arm ran. On a
    shared host those deltas include other tenants — this is an attribution, not a
    thread census — but it is enough to answer the two questions that matter: did
    the arms run on the same physical cores, and did either arm land on both SMT
    siblings of one core while the other sat elsewhere.
    """
    topo = cpu_topology()
    mask = set(arm_cpu_mask())
    out: dict[str, Any] = {
        "attribution": (
            "busiest CPUs WITHIN THE ARM MASK by /proc/stat delta across the arm's "
            "slots; a proxy on a shared host, exact only for arms whose mask "
            "excludes other tenants"
        ),
        "confined_to_mask": cpu_mask_spec(sorted(mask)),
    }
    picked: dict[str, list[int]] = {}
    for arm, deltas in busy_by_arm.items():
        if not deltas:
            continue
        k = max(1, int((threads_by_arm or {}).get(arm, 1)))
        # An arm cannot have run outside the mask it was confined to; without this
        # the busiest CPU on the box wins even when it belongs to another tenant.
        in_mask = {cpu: d for cpu, d in deltas.items() if not mask or cpu in mask}
        ranked = sorted(in_mask.items(), key=lambda kv: kv[1], reverse=True)
        top = [cpu for cpu, delta in ranked[:k] if delta > 0]
        picked[arm] = top
        cores = sorted({topo[cpu] for cpu in top if cpu in topo})
        out[arm] = {
            "threads": k,
            "cpus": top[:16],
            "physical_cores": len(cores) if topo else None,
            "cores": cores[:16] if topo else None,
        }
    exact = self_thread_cpus()
    if exact:
        # NOT "the pandas arm's cpus": these are every thread of the harness
        # process, timing code and interpreter included. Named for what it is.
        out["harness_process_thread_cpus"] = [c for c in exact if not mask or c in mask][:16]
    if len(picked) == 2 and topo:
        (arm_a, cpus_a), (arm_b, cpus_b) = picked.items()
        cores_a = {topo[c] for c in cpus_a if c in topo}
        cores_b = {topo[c] for c in cpus_b if c in topo}
        shared_cores = sorted(cores_a & cores_b)
        out["arms_shared_physical_cores"] = shared_cores[:16]
        out["arms_shared_any_core"] = bool(shared_cores)
        out["arms_shared_logical_cpus"] = sorted(set(cpus_a) & set(cpus_b))[:16]
        out["arms_on_smt_siblings"] = bool(
            shared_cores and not (set(cpus_a) & set(cpus_b))
        )
    return out


class SlotClockSampler:
    """Sample host state WHILE a slot runs, not after it.

    br-frankenpandas-oxv4u. Post-slot sampling reads an arm's cores after its
    threads have exited. FrankenPandas spawns a `thread::scope` per `df.dot`, so
    its cores are already ramping down at that instant, while an incumbent with a
    persistent pool is still hot — measured as a 1.2796x apparent clock skew that
    was teardown, not frequency. Keying the flag on the busy core cut that to
    1.0863, and this removes the rest of the cause rather than compensating for
    it: the samples are taken mid-flight, when both arms are actually working.

    Bounded on purpose. One background thread, `interval` seconds apart, at most
    `max_samples`, reading the same sysfs files the post-slot path reads. At the
    default 50 ms cadence a one-second slot costs ~20 snapshots on ONE core while
    the measured arm holds 63, and the sampler never touches the timed region —
    the timing happens inside the engine, which this thread does not enter.
    """

    def __init__(self, interval: float = 0.05, max_samples: int = 40) -> None:
        self.interval = interval
        self.max_samples = max_samples
        self.samples: list[dict[str, Any]] = []
        self._stop = threading.Event()
        self._thread: threading.Thread | None = None

    def _run(self) -> None:
        # WAIT FIRST, then sample. A snapshot taken at slot start catches the arm
        # before it has ramped up, which is the same class of error as sampling
        # after it has torn down. A slot shorter than one interval therefore
        # yields no samples and falls back, rather than contributing a reading
        # from the wrong instant.
        while len(self.samples) < self.max_samples:
            if self._stop.wait(self.interval):
                return
            self.samples.append(host_state_snapshot())

    def __enter__(self) -> "SlotClockSampler":
        self._thread = threading.Thread(target=self._run, daemon=True)
        self._thread.start()
        return self

    def __exit__(self, *_exc: object) -> None:
        self._stop.set()
        if self._thread is not None:
            self._thread.join(timeout=2.0)


def representative_slot_sample(
    mid_slot: list[dict[str, Any]],
    fallback: dict[str, Any],
) -> dict[str, Any]:
    """The mid-flight sample whose busy core was FASTEST, else the post-slot one.

    br-frankenpandas-oxv4u. Taking the max over mid-flight snapshots answers "what
    clock did this arm run at when it was running", which is the question the
    like-for-like check needs. A slot that produced no samples — too short for one
    interval — falls back to the post-slot reading rather than dropping the arm.
    """
    usable = [s for s in mid_slot if s.get("cpu_mhz")]
    if not usable:
        return fallback
    return max(usable, key=lambda s: s["cpu_mhz"]["max"])


def host_state_snapshot() -> dict[str, Any]:
    """Loadavg and observed CPU MHz at this instant.

    br-frankenpandas-633fb. THIS HOST RUNS THE POWERSAVE GOVERNOR, so a quiet
    window is also a DOWNCLOCKED window: waiting for low load trades contention
    noise for frequency error, and neither the row nor the ledger could
    previously tell them apart. Read from this host's own sysfs rather than
    quoted: `scaling_min_freq` 1429 MHz, `scaling_max_freq` 4562 MHz,
    `cpuinfo_min_freq` 412 MHz. (An earlier "1429-4292 MHz" figure circulating in
    the fleet was retracted by its author; the floor matched, the ceiling did
    not. Record the observed value per row and do not quote a range.) `perf stat` on one df_dot
    invocation already showed the same binary running its serial arm at 4.104 GHz
    and its 63-worker arm at 3.099 GHz — a 32% clock difference INSIDE one job,
    invisible in every row banked before today.

    Read from `scaling_cur_freq` (kHz) across the CPUs in this process's affinity,
    falling back to `/proc/cpuinfo`. Sampling happens strictly BETWEEN timed
    slots, never inside one.
    """
    load: list[float] = []
    try:
        with open("/proc/loadavg", encoding="utf-8") as handle:
            load = [float(part) for part in handle.read().split()[:3]]
    except (OSError, ValueError):
        load = []

    mhz: list[float] = []
    try:
        cpus = sorted(os.sched_getaffinity(0))
    except (AttributeError, OSError):
        cpus = []
    for cpu in cpus:
        path = f"/sys/devices/system/cpu/cpu{cpu}/cpufreq/scaling_cur_freq"
        try:
            with open(path, encoding="utf-8") as handle:
                mhz.append(float(handle.read().strip()) / 1000.0)
        except (OSError, ValueError):
            continue
    if not mhz:
        try:
            with open("/proc/cpuinfo", encoding="utf-8") as handle:
                for line in handle:
                    if line.startswith("cpu MHz"):
                        mhz.append(float(line.split(":", 1)[1]))
        except (OSError, ValueError, IndexError):
            mhz = []

    snapshot: dict[str, Any] = {"loadavg": load}
    if mhz:
        snapshot["cpu_mhz"] = {
            "min": round(min(mhz), 1),
            "median": round(float(np.median(mhz)), 1),
            "max": round(max(mhz), 1),
            "cpus_sampled": len(mhz),
        }
        # Sorted descending, kept for the top-k estimator in
        # `summarize_host_state`. In-memory only: the leading underscore marks it
        # as not for the artifact, and the summariser never copies it out.
        snapshot["_sorted_mhz_desc"] = sorted(mhz, reverse=True)
    return snapshot


def summarize_host_state(
    samples: list[tuple[str, dict[str, Any]]],
    threads_by_arm: dict[str, int] | None = None,
) -> dict[str, Any]:
    """Per-arm clock and whole-cell load, plus a clock-skew flag.

    br-frankenpandas-633fb. The campaign's rule is that both arms must sit inside
    ONE window; this is the evidence for or against that in each row. `arms_saw_
    same_clock` is false when the two arms' median observed MHz differ by more
    than 5%, which is the case a balanced square cannot cancel: the interleave
    protects against drift BETWEEN rounds, not against the two arms systematically
    provoking different clock states — a 63-thread arm pulls the all-core boost
    ceiling down while a 2-thread arm does not.

    Diagnostic only; it feeds no clause of the gate.
    """
    summary: dict[str, Any] = {"gate_input": False, "arm_core_placement": arm_core_placement()}
    loads = [s["loadavg"][0] for _, s in samples if s.get("loadavg")]
    if loads:
        summary["loadavg_1min"] = {
            "first": round(loads[0], 2),
            "last": round(loads[-1], 2),
            "min": round(min(loads), 2),
            "max": round(max(loads), 2),
        }
    # ⚠ TWO DIFFERENT QUANTITIES, and conflating them cost me a wrong reading.
    # The MEDIAN over every CPU in the affinity measures the BOX's clock state;
    # for a 1-thread arm it is dominated by the 63 idle cores that have already
    # downclocked, so it reports LOW for an arm whose own core is at full boost.
    # Measured: a forced-serial FrankenPandas arm read 3246.1 MHz median against
    # a 64-thread pandas arm's 3972.9 — the exact opposite of what the arms' own
    # cores were doing. The MAX is the busy-core proxy and is what determines an
    # arm's execution speed, so the same-window flag is computed from it.
    busy: dict[str, float] = {}
    host: dict[str, float] = {}
    topk: dict[str, dict[str, Any]] = {}
    for arm in ("frankenpandas", "pandas"):
        rows = [s["cpu_mhz"] for tag, s in samples if tag == arm and s.get("cpu_mhz")]
        if rows:
            busy[arm] = round(float(np.median([r["max"] for r in rows])), 1)
            host[arm] = round(float(np.median([r["median"] for r in rows])), 1)
        # ⚠ THE ESTIMATOR THAT ANSWERS "DID BOTH ARMS RUN ON COMPARABLE CORES".
        # An arm using k threads occupies roughly the k fastest cores, so its
        # cores are estimated by the top k of each sample rather than by the max
        # (which is one core, blind to a 64-thread arm's slow tail) or the median
        # (which is dominated by idle cores). k comes from the arm's OBSERVED
        # thread count, never a requested one.
        k = (threads_by_arm or {}).get(arm)
        vectors = [s["_sorted_mhz_desc"] for tag, s in samples
                   if tag == arm and s.get("_sorted_mhz_desc")]
        if k and vectors:
            heads = [v[: max(1, min(int(k), len(v)))] for v in vectors]
            topk[arm] = {
                "k": int(k),
                "median": round(float(np.median([float(np.median(h)) for h in heads])), 1),
                "slowest": round(float(np.median([min(h) for h in heads])), 1),
                "fastest": round(float(np.median([max(h) for h in heads])), 1),
            }
    if busy:
        summary["busy_core_mhz_by_arm"] = busy
        summary["host_median_mhz_after_arm"] = host
    if topk:
        summary["arm_core_mhz_top_k"] = topk
    # ⚠ THE SAME-WINDOW JUDGEMENT USES THE BUSY-CORE FIGURE, NOT THE TOP-K MEDIAN,
    # and the reason is a sampling artifact I measured on my own rows.
    #
    # Sampling happens AFTER each slot. An arm whose threads EXIT at the end of a
    # call — FrankenPandas spawns a `thread::scope` per `df.dot` — leaves cores
    # already ramping down when the sample is taken, while an incumbent with a
    # persistent pool (OpenBLAS) keeps its cores hot. Measured on
    # `oxv4u_1M_pair_*`: FrankenPandas' top-58 spanned 2508.4-3730.8 MHz
    # (median 3014.5) against pandas' top-64 at 3844.0-3868.0 (median 3857.5) —
    # k nearly equal, so it is not a k mismatch. Keying the flag on those medians
    # called the row 1.2796x apart and would disqualify essentially every
    # parallel row for TEARDOWN rather than for a real frequency difference.
    #
    # The fastest cores at sample time are the ones that were still working, so
    # `busy` is the better estimate of what the arm RAN at: the same pair reads
    # 3730.8 against 3868.0, a ratio of 1.037, inside the 5% band. The top-k
    # detail is still recorded — it is what exposed the artifact — but it does
    # not drive the verdict.
    if len(busy) == 2:
        low, high = sorted(busy.values())
        summary["arms_saw_same_clock"] = bool(high <= low * 1.05)
        summary["arm_clock_ratio"] = round(high / low, 4) if low else None
        summary["same_clock_basis"] = "busy_core_max"
        # The busy core cannot see a WIDE arm whose own threads straddle fast and
        # slow cores, so record that separately rather than pretending one number
        # covers both. `arm_core_spread_ratio` is fastest/slowest within an arm's
        # own top-k: ~1.0 when its cores agree, large when they do not.
        for arm, detail in topk.items():
            if detail.get("slowest"):
                summary.setdefault("arm_core_spread_ratio", {})[arm] = round(
                    detail["fastest"] / detail["slowest"], 4
                )
        summary["same_clock_note"] = (
            "sampled after each slot; an arm whose threads exit leaves cores "
            "ramping down, so the top-k median understates it"
        )
    every = [s["cpu_mhz"]["median"] for _, s in samples if s.get("cpu_mhz")]
    if every:
        summary["observed_cpu_mhz"] = {
            "min": round(min(every), 1),
            "median": round(float(np.median(every)), 1),
            "max": round(max(every), 1),
            "samples": len(every),
        }
    return summary


def run_balanced_square_cell(
    category: str,
    workload: str,
    size: str,
    dtype: str,
    tmp_path: Path,
    fingerprint: dict[str, Any],
    fp_binary: Path | None,
    rounds: int,
    adaptive_rounds: bool = False,
) -> tuple[TimingResult, TimingResult, dict[str, Any]]:
    """Interleave incumbent and subject according to the sanctioned ABBA square."""
    pandas_slots: list[TimingResult] = []
    fp_slots: list[TimingResult] = []
    round_ratios: list[float] = []
    rounds_artifact = []
    # br-frankenpandas-633fb: sampled BETWEEN slots, never inside a timed region.
    host_state_samples: list[tuple[str, dict[str, Any]]] = []
    busy_by_arm: dict[str, dict[int, int]] = {"frankenpandas": {}, "pandas": {}}
    # `while`, not `range(rounds)`: with --adaptive-rounds the bound is recomputed
    # once at the end of round 0 from the incumbent slots just measured, and
    # `range` would have frozen it. The bound only ever GROWS.
    round_index = 0
    while round_index < rounds:
        pandas_round: list[TimingResult] = []
        fp_round: list[TimingResult] = []
        for slot_index, arm in enumerate(BALANCED_SQUARE):
            busy_before = cpu_busy_snapshot()
            # Sample WHILE the slot runs (br-frankenpandas-oxv4u): a post-slot
            # reading catches an arm whose threads have already exited.
            slot_sampler = SlotClockSampler()
            slot_sampler.__enter__()
            if arm == "A":
                result, _ = run_pandas_workload(
                    category,
                    workload,
                    size,
                    dtype,
                    tmp_path,
                    fingerprint,
                    None,
                )
                pandas_slots.append(result)
                pandas_round.append(result)
            else:
                result = run_fp_workload_subprocess(
                    category,
                    workload,
                    size,
                    dtype,
                    tmp_path if category == "pipeline" else None,
                    fp_binary,
                )
                fp_slots.append(result)
                fp_round.append(result)
            slot_sampler.__exit__()
            # Which CPUs were busiest while THIS arm ran, accumulated per arm.
            arm_name = "pandas" if arm == "A" else "frankenpandas"
            busy_after = cpu_busy_snapshot()
            for cpu, after in busy_after.items():
                delta = after - busy_before.get(cpu, after)
                if delta > 0:
                    busy_by_arm[arm_name][cpu] = busy_by_arm[arm_name].get(cpu, 0) + delta
            # AFTER the slot, tagged with the arm that just ran. Sampling BEFORE
            # it (as this did until br-frankenpandas-633fb's own audit) reads the
            # clock state left by the PREVIOUS slot, which in ABBAABBA is usually
            # the OTHER arm — so every per-arm median was attributed to its
            # neighbour. Still outside every timed region: the timing happens
            # inside the engine subprocess.
            host_state_samples.append(
                (
                    arm_name,
                    representative_slot_sample(slot_sampler.samples, host_state_snapshot()),
                )
            )
        if any(not result.is_valid for result in (*pandas_round, *fp_round)):
            # Match the legacy harness's behavior for a missing/invalid FP
            # executable: emit an INCOMPLETE row rather than treating an
            # infrastructure failure as a measurement or throwing away the
            # artifact.  One completed incumbent slot is enough provenance to
            # make that diagnosis explicit; it is never used for a ratio.
            return (
                fp_round[0],
                pandas_round[0],
                {
                    "design": "balanced-square-abbaabba-v1",
                    "incomplete": True,
                    "reason": "slot timing contract invalid",
                    "host_wide_quiescence_required": False,
                },
            )
        pandas_median = float(np.median([result.p50_us for result in pandas_round]))
        fp_median = float(np.median([result.p50_us for result in fp_round]))
        if fp_median <= 0.0:
            raise RuntimeError("balanced-square subject median must be positive")
        round_ratios.append(pandas_median / fp_median)
        rounds_artifact.append(
            {
                "round": round_index,
                "order": BALANCED_SQUARE,
                "pandas_slot_p50_us": [result.p50_us for result in pandas_round],
                "frankenpandas_slot_p50_us": [result.p50_us for result in fp_round],
                "ratio_pandas_over_frankenpandas": round_ratios[-1],
            }
        )
        if round_index == 0:
            rounds = rounds_after_first_round(
                [result.p50_us for result in pandas_round],
                rounds,
                enabled=adaptive_rounds,
            )
        round_index += 1
    fp_aggregate = _balanced_square_aggregate(fp_slots, engine="frankenpandas")
    pandas_aggregate = _balanced_square_aggregate(pandas_slots, engine="pandas")
    observed_threads = {
        arm: value
        for arm, value in (
            ("frankenpandas", fp_aggregate.operation_threads_used),
            ("pandas", pandas_aggregate.operation_threads_used),
        )
        if value
    }
    return (
        fp_aggregate,
        pandas_aggregate,
        {
            "design": "balanced-square-abbaabba-v1",
            "incumbent_arm": "A=pandas",
            "subject_arm": "B=frankenpandas",
            "rounds": rounds,
            "slots_per_arm_per_round": BALANCED_SQUARE.count("A"),
            "round_ratio_pandas_over_frankenpandas": round_ratios,
            "rounds_detail": rounds_artifact,
            "host_wide_quiescence_required": False,
            "host_state": summarize_host_state(
                host_state_samples + [("final", host_state_snapshot())],
                observed_threads,
            )
            | {"arm_cpu_attribution": summarize_arm_cpus(busy_by_arm, observed_threads)},
        },
    )


def best_vs_best(fp_result: TimingResult, pd_result: TimingResult) -> dict[str, Any]:
    """Each arm's FASTEST observed sample, and whether it agrees with the median.

    br-frankenpandas-mti15. A gated row is a comparison of MEDIANS, and a median
    is only a property of the code when both arms are tight. On 2026-08-16 a
    `df_dot @1M` row certified at 1.187x FASTER and was retracted two hours later:
    FrankenPandas' p50 reproduced to 0.5% across two runs while the incumbent's
    samples spanned 9.07-29.79 ms inside single invocations, so the crossing was a
    property of where the incumbent's median happened to land. Comparing the two
    arms' MINIMA — the least contended sample each engine managed — said 0.620 and
    0.509, i.e. a loss, in both runs.

    So this records the minima beside the gated median and, more usefully, flags
    when the two statistics point in OPPOSITE directions. That disagreement is the
    signature of a dispersion-driven row, and it is what a reader needs in order
    not to repeat my mistake.

    This is diagnostic only. It does NOT feed the verdict, the gate, or any
    clause: the three-clause contract is shared across the campaign and is not
    something one agent rewrites. It only adds fields.
    """
    fp_min = float(min(fp_result.times_us))
    pd_min = float(min(pd_result.times_us))
    ratio = pd_min / fp_min if fp_min > 0 else float("inf")
    return {
        "frankenpandas_min_us": round(fp_min, 4),
        "pandas_min_us": round(pd_min, 4),
        "ratio": round(ratio, 4),
        "definition": "pandas_min_us / frankenpandas_min_us, over every timed sample in this invocation",
        "gate_input": False,
    }


def annotate_best_vs_best(
    comparison: dict[str, Any],
    fp_result: TimingResult,
    pd_result: TimingResult,
) -> None:
    """Attach `best_vs_best` and raise a dispersion warning on disagreement.

    br-frankenpandas-mti15. `direction_agrees_with_median` is false when one
    statistic says FrankenPandas is faster and the other says the incumbent is —
    the exact shape of the retracted row. A row carrying
    `dispersion_warning: true` should not be quoted as a property of the code
    without a replication, whatever its verdict says.
    """
    if not fp_result.times_us or not pd_result.times_us:
        return
    detail = best_vs_best(fp_result, pd_result)
    median_ratio = comparison.get("ratio")
    if isinstance(median_ratio, (int, float)) and median_ratio > 0:
        agrees = (detail["ratio"] > 1.0) == (float(median_ratio) > 1.0)
        detail["median_ratio"] = float(median_ratio)
        detail["direction_agrees_with_median"] = agrees
        comparison["dispersion_warning"] = not agrees
    comparison["best_vs_best"] = detail


def like_for_like(comparison: dict[str, Any]) -> dict[str, Any]:
    """Is this row a comparison of the two ENGINES, or of their circumstances?

    br-frankenpandas-oxv4u. On 2026-08-16 a `df_dot @1M` row passed all three
    gate clauses at 1.680x FASTER with BOTH A/A nulls clean (0.998, 0.994) — and
    was not bankable: FrankenPandas' busy cores ran at 3730.8 MHz against pandas'
    3868.0, and best-vs-best said 0.9106, i.e. the minima put FrankenPandas
    slightly SLOWER while the median put it 68% faster. The incumbent had
    degraded more than the subject under load the run itself created.

    Both facts were already in the row, in two separate fields, and it still took
    a human read to catch. This collapses them into ONE field a banker cannot
    miss, with the reasons named. DIAGNOSTIC ONLY: `gate_input` is false, no
    clause is touched, and a refused row stays refused — the three-clause
    contract is shared across the campaign and is not one agent's to rewrite.
    What changes is what a reader sees beside the verdict.
    """
    reasons: list[str] = []
    host_state = comparison.get("balanced_square", {}).get("host_state", {})
    if host_state.get("arms_saw_same_clock") is False:
        reasons.append(
            f"arms ran at different clocks (ratio {host_state.get('arm_clock_ratio')}); "
            "the measured ratio carries a frequency term"
        )
    detail = comparison.get("best_vs_best", {})
    if detail.get("direction_agrees_with_median") is False:
        reasons.append(
            f"median says {detail.get('median_ratio')}x but best-vs-best says "
            f"{detail.get('ratio')}x — they disagree in DIRECTION"
        )
    # THIRD REASON: the subject was denied cores the incumbent never needed.
    # br-frankenpandas-4kig1, and the fixture is again a real row rather than an
    # invented one. `str_startswith_arrow @1M` certified 3/3 clauses at 1.275x
    # under `affinity_cpus=[0]` and, 27 minutes later on the same host with no
    # source change, at 4.824x unconstrained. pandas is single-threaded on that
    # workload so its arm moved 0.6%; FrankenPandas' moved 3.74x. The entire 3.8x
    # spread between two certified ratios was our own parallelism being taken away
    # and given back, and NOTHING in the row said so -- the capped run had the
    # CLEANEST A/A nulls of its batch (0.99960/0.99997) and a cv of 0.28% against
    # 17.54%, because one busy core on a 64-thread host is a quiet host. Null
    # quality measures how still the machine was, not how representative the row
    # is, and here the stillest row understated the subject by 3.74x.
    provenance = comparison.get("thread_provenance", {})
    available = provenance.get("runtime_available_parallelism")
    host_threads = provenance.get("logical_threads")
    if isinstance(available, dict) and isinstance(host_threads, int):
        subject_saw = available.get("frankenpandas")
        if isinstance(subject_saw, int) and 0 < subject_saw < host_threads:
            reasons.append(
                f"FrankenPandas saw {subject_saw} of the host's {host_threads} logical "
                "CPUs; the ratio measures a THREAD CAP as much as the two engines, and "
                "must not be tabulated beside unconstrained rows"
            )
    return {"ok": not reasons, "reasons": reasons, "gate_input": False}


def apply_balanced_square_gate(
    comparison: dict[str, Any],
    fp_result: TimingResult,
    pd_result: TimingResult,
    experiment: dict[str, Any],
) -> None:
    """Replace independent-sample inference with paired round-ratio inference."""
    round_ratios = experiment["round_ratio_pandas_over_frankenpandas"]
    ratio = float(np.median(round_ratios))
    effect_ci = bootstrap_median_ci(round_ratios)
    required_log_effect = DECIDABILITY_MARGIN * max(
        fp_result.null_log_half_width,
        pd_result.null_log_half_width,
    )
    gate = corrected_null_gate(
        ratio,
        effect_ci,
        required_log_effect,
        fp_result.null_median_ratio,
        pd_result.null_median_ratio,
    )
    comparison["ratio"] = round(ratio, 3)
    comparison["median_ci_gate"] = gate | {
        "margin_multiplier": DECIDABILITY_MARGIN,
        "combined_two_x_null_interval": [
            round(math.exp(-required_log_effect), 6),
            round(math.exp(required_log_effect), 6),
        ],
        "effect_ci_method": "paired-bootstrap-median-of-round-ratios",
        "cv_is_provenance_only": True,
    }
    comparison["verdict"] = (
        "FASTER" if gate["decidable"] and ratio > 1.0 else
        "SLOWER" if gate["decidable"] else
        "NULL_UNDECIDABLE"
    )
    comparison["balanced_square"] = experiment
    # Diagnostic, after the verdict and deliberately not an input to it.
    annotate_best_vs_best(comparison, fp_result, pd_result)
    comparison["like_for_like"] = like_for_like(comparison)


def resolve_results_path(output: Path | None, timestamp: str) -> Path:
    """Where this invocation's row will be written. Never None.

    A measured row ALWAYS lands on disk. `--json-stdout` is a request for the
    row on stdout as well, never instead — printing a fingerprint to a terminal
    that is later closed does not bank it. (br-frankenpandas-s7x8z)
    """
    if output is not None:
        return output
    return RESULTS_DIR / f"bench_{timestamp.replace(':', '-')}.json"


def _row_persistence_self_test() -> None:
    """A measured row always has a destination, whatever the output flags.

    THE DEFECT THIS PINS: the writer used to read

        if args.output:      write to args.output
        elif not args.json_stdout:   write to artifacts/bench/...

    so `--json-stdout` with no `--output` measured a row and persisted NOTHING.
    That is the sanctioned recipe in docs/NEGATIVE_EVIDENCE.md, which is why
    the str_startswith_arrow @1M = 5.105x row has no artifact and its worker is
    attested only by that ledger's prose. Every field that makes a row
    comparable to another -- host_identity, cpu_model, logical_threads, the ISA
    set, harness_source.sha256, the self-reported ELF SHA -- lives in the file
    that was not written.

    `json_stdout` is deliberately absent from `resolve_results_path`'s
    signature: the destination cannot depend on it, so the regression cannot be
    reintroduced by re-adding a branch on it.
    """
    stamp = "2026-08-16T00:00:00Z"
    default = resolve_results_path(None, stamp)
    if default.parent != RESULTS_DIR:
        raise RuntimeError(f"default row destination escaped {RESULTS_DIR}: {default}")
    if ":" in default.name:
        raise RuntimeError(f"timestamp colons must be sanitized for the filename: {default}")
    if not default.name.startswith("bench_") or not default.name.endswith(".json"):
        raise RuntimeError(f"unexpected default artifact name: {default.name}")

    explicit = Path("/tmp/somewhere/row.json")
    if resolve_results_path(explicit, stamp) != explicit:
        raise RuntimeError("an explicit --output must be honoured verbatim")

    # The contract that actually regressed: the destination is a function of
    # --output alone. Reject any signature that lets an output-format flag
    # decide whether the row is kept.
    parameters = list(inspect.signature(resolve_results_path).parameters)
    if parameters != ["output", "timestamp"]:
        raise RuntimeError(
            "resolve_results_path must depend only on --output and the timestamp; "
            f"got {parameters}"
        )


def _slot_sampler_self_test() -> None:
    """The sampler must capture mid-flight samples and prefer the working clock.

    br-frankenpandas-oxv4u. Written because the artifact it removes was invisible
    to every other check: the post-slot reading is a VALID snapshot, just of the
    wrong instant, so nothing failed while the clock attribution was wrong.
    """
    import time as _time

    with SlotClockSampler(interval=0.02, max_samples=10) as sampler:
        _time.sleep(0.12)
    captured = len(sampler.samples)
    if captured < 2:
        raise RuntimeError(f"a 120ms slot at 20ms cadence must yield samples, got {captured}")
    if captured > 10:
        raise RuntimeError("max_samples must bound the sampler")
    if any("loadavg" not in s for s in sampler.samples):
        raise RuntimeError("each mid-slot sample must carry host state")

    # A slot shorter than one interval yields nothing and must fall back rather
    # than drop the arm.
    with SlotClockSampler(interval=5.0, max_samples=4) as brief:
        pass
    fallback = {"loadavg": [1.0, 1, 1],
                "cpu_mhz": {"min": 1.0, "median": 2.0, "max": 3.0, "cpus_sampled": 4}}
    if representative_slot_sample(brief.samples, fallback) is not fallback:
        raise RuntimeError("an unsampled slot must fall back to the post-slot reading")

    # Among mid-flight samples, the one whose busy core was FASTEST wins — that is
    # the arm running, not the arm winding down.
    winding_down = {"cpu_mhz": {"min": 1429.0, "median": 1500.0, "max": 2100.0,
                                "cpus_sampled": 64}}
    working = {"cpu_mhz": {"min": 3800.0, "median": 3900.0, "max": 4100.0,
                           "cpus_sampled": 64}}
    picked = representative_slot_sample([winding_down, working, winding_down], fallback)
    if picked is not working:
        raise RuntimeError("the working sample must win over the winding-down ones")

    # Samples without a clock reading must not be chosen over one that has it.
    if representative_slot_sample([{"loadavg": [1.0, 1, 1]}, working], fallback) is not working:
        raise RuntimeError("a sample lacking cpu_mhz must not be selected")


def _like_for_like_self_test() -> None:
    """Pin the combined verdict on the row that made it necessary.

    br-frankenpandas-oxv4u. The fixture IS the refused row: gate FASTER at
    1.680x with both A/A nulls clean, arms at 3730.8 vs 3868.0 MHz, best-vs-best
    0.9106 against a median of 1.680. A diagnostic that calls that row
    like-for-like is worthless, so this asserts it does not — and that a clean
    row is not flagged, which is the other half.
    """
    refused = {
        "verdict": "FASTER",
        "ratio": 1.68,
        "balanced_square": {
            "host_state": {"arms_saw_same_clock": False, "arm_clock_ratio": 1.2796}
        },
        "best_vs_best": {
            "ratio": 0.9106,
            "median_ratio": 1.68,
            "direction_agrees_with_median": False,
        },
    }
    verdict = like_for_like(refused)
    if verdict["ok"]:
        raise RuntimeError("a row with skewed clocks AND inverted minima is not like-for-like")
    if len(verdict["reasons"]) != 2:
        raise RuntimeError(f"both reasons must be named, got {verdict['reasons']}")
    if not any("frequency term" in reason for reason in verdict["reasons"]):
        raise RuntimeError("the clock reason must say why it matters")
    if not any("DIRECTION" in reason for reason in verdict["reasons"]):
        raise RuntimeError("the minima reason must name the disagreement")
    if verdict["gate_input"]:
        raise RuntimeError("like_for_like must declare itself a non-gate input")

    # Either flag alone is enough to disqualify a row.
    clock_only = {
        "balanced_square": {
            "host_state": {"arms_saw_same_clock": False, "arm_clock_ratio": 1.3775}
        },
        "best_vs_best": {"ratio": 0.51, "median_ratio": 0.51,
                         "direction_agrees_with_median": True},
    }
    if like_for_like(clock_only)["ok"] or len(like_for_like(clock_only)["reasons"]) != 1:
        raise RuntimeError("clock skew alone must disqualify, with one reason")

    minima_only = {
        "balanced_square": {
            "host_state": {"arms_saw_same_clock": True, "arm_clock_ratio": 1.0013}
        },
        "best_vs_best": {"ratio": 0.62, "median_ratio": 1.187,
                         "direction_agrees_with_median": False},
    }
    if like_for_like(minima_only)["ok"]:
        raise RuntimeError("an inverted best-vs-best alone must disqualify")

    # The certified dim=100 row: one clock, minima agreeing. Must NOT be flagged.
    clean = {
        "balanced_square": {
            "host_state": {"arms_saw_same_clock": True, "arm_clock_ratio": 1.0013}
        },
        "best_vs_best": {"ratio": 0.4653, "median_ratio": 0.573,
                         "direction_agrees_with_median": True},
    }
    if not like_for_like(clean)["ok"] or like_for_like(clean)["reasons"]:
        raise RuntimeError("a same-clock, direction-agreeing row must pass unflagged")

    # A row missing the diagnostics entirely must not be flagged on absence.
    if not like_for_like({"verdict": "SLOWER"})["ok"]:
        raise RuntimeError("absent diagnostics must not be read as a failure")

    # br-frankenpandas-4kig1: the thread-capped row, as it actually appeared on
    # disk. Everything else about it is immaculate -- clean clocks, agreeing
    # minima, the batch's best nulls -- and it is still not a comparison of the
    # two engines.
    capped = {
        "verdict": "FASTER",
        "ratio": 1.275,
        "balanced_square": {
            "host_state": {"arms_saw_same_clock": True, "arm_clock_ratio": 1.0004}
        },
        "best_vs_best": {"ratio": 1.27, "median_ratio": 1.275,
                         "direction_agrees_with_median": True},
        "thread_provenance": {
            "logical_threads": 64,
            "runtime_available_parallelism": {"frankenpandas": 1, "pandas": 1},
        },
    }
    verdict = like_for_like(capped)
    if verdict["ok"]:
        raise RuntimeError("a row measured under a thread cap is not like-for-like")
    if len(verdict["reasons"]) != 1:
        raise RuntimeError(f"the cap must be the only reason, got {verdict['reasons']}")
    if "THREAD CAP" not in verdict["reasons"][0]:
        raise RuntimeError("the cap reason must name what it is")

    # The SAME workload unconstrained, which certified at 4.824x. Must pass clean,
    # or the check would flag every row on a busy host and be worthless.
    uncapped = dict(capped) | {
        "ratio": 4.824,
        "thread_provenance": {
            "logical_threads": 64,
            "runtime_available_parallelism": {"frankenpandas": 64, "pandas": 64},
        },
    }
    if not like_for_like(uncapped)["ok"]:
        raise RuntimeError("an unconstrained row must not be flagged as capped")

    # A row whose provenance is absent or malformed must not be flagged on
    # absence -- the same rule the clock and minima checks already follow.
    for missing in (
        {"thread_provenance": {}},
        {"thread_provenance": {"logical_threads": 64}},
        {"thread_provenance": {"runtime_available_parallelism": {"frankenpandas": 1}}},
        {"thread_provenance": {"logical_threads": 64,
                               "runtime_available_parallelism": {"frankenpandas": None}}},
    ):
        if not like_for_like(missing)["ok"]:
            raise RuntimeError(f"absent thread provenance must not flag: {missing}")


def _host_state_self_test() -> None:
    """Pin the clock/load instrument, including the skew it exists to catch.

    br-frankenpandas-633fb. The negative case is not invented: `perf stat` on one
    df_dot invocation measured the same binary at 4.104 GHz serial and 3.099 GHz
    across 63 workers. A row whose two arms sit at those clocks is not a
    like-for-like comparison, and before this instrument nothing in the row said
    so.
    """
    live = host_state_snapshot()
    if len(live.get("loadavg", [])) != 3:
        raise RuntimeError("host state must carry the three loadavg figures")
    freq = live.get("cpu_mhz")
    if freq is not None:
        if not freq["min"] <= freq["median"] <= freq["max"]:
            raise RuntimeError("cpu_mhz min/median/max must be ordered")
        if freq["cpus_sampled"] < 1 or not 100.0 < freq["median"] < 10000.0:
            raise RuntimeError(f"implausible observed clock: {freq}")

    def sample(arm: str, mhz: float) -> tuple[str, dict[str, Any]]:
        return (arm, {"loadavg": [1.0, 2.0, 3.0],
                      "cpu_mhz": {"min": mhz, "median": mhz, "max": mhz, "cpus_sampled": 4}})

    # NEGATIVE CASE: the measured 4.104 / 3.099 GHz split must be flagged.
    skewed = summarize_host_state(
        [sample("pandas", 4104.0), sample("frankenpandas", 3099.0)] * 3
    )
    if skewed["arms_saw_same_clock"]:
        raise RuntimeError("a 32% inter-arm clock split must not read as one window")
    if round(skewed["arm_clock_ratio"], 3) != 1.324:
        raise RuntimeError(f"clock ratio mis-computed: {skewed['arm_clock_ratio']}")
    if skewed["busy_core_mhz_by_arm"] != {"frankenpandas": 3099.0, "pandas": 4104.0}:
        raise RuntimeError("per-arm busy-core clocks must be reported separately")

    # NEGATIVE CASE: the flag must follow the BUSY CORE, not the box median. A
    # 1-thread arm at full boost sits beside idle, downclocked neighbours; if the
    # summary keyed on the median it would call this pair mismatched when their
    # busy cores agree, which is the reading that misled me on a live row.
    idle_skew = summarize_host_state([
        ("frankenpandas", {"loadavg": [1.0, 1, 1],
                           "cpu_mhz": {"min": 1429.0, "median": 1500.0, "max": 4000.0,
                                       "cpus_sampled": 64}}),
        ("pandas", {"loadavg": [1.0, 1, 1],
                    "cpu_mhz": {"min": 3800.0, "median": 3950.0, "max": 4000.0,
                                "cpus_sampled": 64}}),
    ])
    if not idle_skew["arms_saw_same_clock"]:
        raise RuntimeError("equal busy-core clocks must read as one window")
    if idle_skew["host_median_mhz_after_arm"] != {"frankenpandas": 1500.0, "pandas": 3950.0}:
        raise RuntimeError("the box-median must still be recorded, separately labelled")

    # SMT TRAP: a logical mask says nothing about physical cores, and the
    # numbering is not portable. frankenfs found BOTH arms on one physical core.
    topo = cpu_topology()
    if topo:
        composition = mask_composition(sorted(topo)[:8])
        if composition["physical_cores"] is None or composition["physical_cores"] < 1:
            raise RuntimeError("a mask must report its physical-core count")
        if composition["logical"] != 8:
            raise RuntimeError("mask composition must report the logical width too")
        # This host numbers siblings n and n+32, so 0-7 is eight distinct cores.
        # Assert the FOLDED case is detected rather than the host's happy accident.
        siblings = [cpu for cpu, core in topo.items() if core == topo[sorted(topo)[0]]]
        if len(siblings) > 1:
            folded = mask_composition(sorted(siblings))
            if folded["one_thread_per_core"]:
                raise RuntimeError("a mask of two SMT siblings is NOT one thread per core")
            if folded["physical_cores"] != 1 or folded["smt_folded_cores"] != 1:
                raise RuntimeError(f"sibling folding mis-detected: {folded}")

    # one-per-core must resolve to EXACTLY one logical CPU per physical core.
    if topo:
        opc = parse_cpu_spec("one-per-core")
        composition = mask_composition(opc)
        if not composition["one_thread_per_core"]:
            raise RuntimeError("one-per-core must not fold SMT siblings")
        if composition["physical_cores"] != len(opc):
            raise RuntimeError("one-per-core must yield one CPU per physical core")
        if composition["physical_cores"] != len(set(topo.values())):
            raise RuntimeError("one-per-core must cover every physical core once")

    # ARM CPU ATTRIBUTION: the two arms colliding on one core must be visible.
    collided = summarize_arm_cpus(
        {"frankenpandas": {0: 900, 1: 5}, "pandas": {0: 850, 2: 4}},
        {"frankenpandas": 1, "pandas": 1},
    )
    if collided["frankenpandas"]["cpus"] != [0] or collided["pandas"]["cpus"] != [0]:
        raise RuntimeError("attribution must pick the busiest CPU per arm")
    if topo and not collided["arms_shared_any_core"]:
        raise RuntimeError("two arms on cpu0 must be reported as sharing a core")
    if collided["arms_shared_logical_cpus"] != [0]:
        raise RuntimeError("the shared logical CPU must be named")

    # TEARDOWN ARTIFACT, measured on artifacts/bench/oxv4u_1M_pair_*: an arm whose
    # threads exit has cores ramping down at sample time, so its top-k MEDIAN
    # collapses while its busy cores were fine. Keying the same-window flag on the
    # median disqualified a row for teardown; keying it on the busy core does not.
    teardown = summarize_host_state(
        [
            ("frankenpandas", {"loadavg": [1.0, 1, 1],
                               "cpu_mhz": {"min": 1429.0, "median": 3014.5, "max": 3730.8,
                                           "cpus_sampled": 64},
                               "_sorted_mhz_desc": [3730.8] * 8 + [3014.5] * 28 + [1429.0] * 28}),
            ("pandas", {"loadavg": [1.0, 1, 1],
                        "cpu_mhz": {"min": 3844.0, "median": 3857.5, "max": 3868.0,
                                    "cpus_sampled": 64},
                        "_sorted_mhz_desc": [3868.0] * 8 + [3857.5] * 56}),
        ] * 3,
        {"frankenpandas": 58, "pandas": 64},
    )
    if not teardown["arms_saw_same_clock"]:
        raise RuntimeError("busy cores 3730.8 vs 3868.0 are one window; teardown is not skew")
    if teardown["same_clock_basis"] != "busy_core_max":
        raise RuntimeError("the same-window flag must key on the busy core")
    if round(teardown["arm_clock_ratio"], 3) != 1.037:
        raise RuntimeError(f"busy-core ratio mis-computed: {teardown['arm_clock_ratio']}")
    if "arm_core_mhz_top_k" not in teardown:
        raise RuntimeError("the top-k detail must still be recorded — it exposed the artifact")

    # A REAL skew must still be caught: busy cores far apart.
    real_skew = summarize_host_state(
        [
            ("frankenpandas", {"loadavg": [1.0, 1, 1],
                               "cpu_mhz": {"min": 2000.0, "median": 2000.0, "max": 2000.0,
                                           "cpus_sampled": 8},
                               "_sorted_mhz_desc": [2000.0] * 8}),
            ("pandas", {"loadavg": [1.0, 1, 1],
                        "cpu_mhz": {"min": 4000.0, "median": 4000.0, "max": 4000.0,
                                    "cpus_sampled": 8},
                        "_sorted_mhz_desc": [4000.0] * 8}),
        ] * 3,
        {"frankenpandas": 8, "pandas": 8},
    )
    if real_skew["arms_saw_same_clock"]:
        raise RuntimeError("2000MHz against 4000MHz busy cores is a real skew")

    # NEGATIVE CASE from a real failure: a busy CPU OUTSIDE the mask must never
    # be attributed to an arm. A --pin-cpus 0-31 run reported cpus 51 and 39 —
    # other tenants — before this constraint existed.
    global ARM_CPU_MASK
    saved_mask = ARM_CPU_MASK
    try:
        ARM_CPU_MASK = [0, 1, 2, 3]
        outside = summarize_arm_cpus(
            {"frankenpandas": {51: 99999, 2: 10}, "pandas": {39: 99999, 3: 10}},
            {"frankenpandas": 1, "pandas": 1},
        )
        if outside["frankenpandas"]["cpus"] != [2] or outside["pandas"]["cpus"] != [3]:
            raise RuntimeError("attribution must stay inside the arm mask")
        if outside["confined_to_mask"] != "0-3":
            raise RuntimeError("the row must record the mask attribution was confined to")
    finally:
        ARM_CPU_MASK = saved_mask

    apart = summarize_arm_cpus(
        {"frankenpandas": {0: 900}, "pandas": {5: 900}},
        {"frankenpandas": 1, "pandas": 1},
    )
    if topo and apart["arms_shared_any_core"]:
        raise RuntimeError("arms on different cores must not read as sharing")

    # CORE PLACEMENT: the row must SAY how both arms were kept comparable.
    placement = summarize_host_state([])["arm_core_placement"]
    if not placement["cpu_mask"] or placement["cpus"] < 1:
        raise RuntimeError("the arm CPU mask must be recorded, not left empty")
    if "taskset" not in placement["same_cores_ensured_by"]:
        raise RuntimeError("the row must name the mechanism, not just assert parity")
    if placement["cpu_mask"] not in placement["same_cores_ensured_by"]:
        raise RuntimeError("the stated mechanism must quote the mask it applied")

    # The mask spec round-trips, including the disjoint case a NUMA pin produces.
    for spec in ("0", "0-3", "0-7,32-39", "1,3,5"):
        if cpu_mask_spec(parse_cpu_spec(spec)) != spec:
            raise RuntimeError(f"cpu mask spec failed to round-trip: {spec}")
    if parse_cpu_spec("0-3") != [0, 1, 2, 3]:
        raise RuntimeError("cpu spec ranges must be inclusive")
    if parse_cpu_spec(" , ") != []:
        raise RuntimeError("an empty spec must parse to an empty set, not raise")

    # TOP-K: an arm's cores are the k fastest, k = its OBSERVED thread count.
    def vec(arm: str, freqs: list[float]) -> tuple[str, dict[str, Any]]:
        ordered = sorted(freqs, reverse=True)
        return (arm, {"loadavg": [1.0, 1, 1],
                      "cpu_mhz": {"min": min(ordered), "median": float(np.median(ordered)),
                                  "max": max(ordered), "cpus_sampled": len(ordered)},
                      "_sorted_mhz_desc": ordered})

    # NEGATIVE CASE, and the one this estimator exists for: a 1-thread arm on a
    # single fast core against a 64-thread arm spread over fast AND slow ones.
    # The single busy core reads 4000 for both and calls it one window; the top-k
    # medians do not, because the wide arm's own cores really are slower.
    spread = [4000.0] * 8 + [2000.0] * 56
    mixed = summarize_host_state(
        [vec("frankenpandas", spread), vec("pandas", spread)] * 3,
        {"frankenpandas": 1, "pandas": 64},
    )
    if mixed["arm_core_mhz_top_k"]["frankenpandas"]["median"] != 4000.0:
        raise RuntimeError("a 1-thread arm must be credited with its one fast core")
    if mixed["arm_core_mhz_top_k"]["pandas"]["median"] != 2000.0:
        raise RuntimeError("a 64-thread arm's cores must include its slow tail")
    # The busy-core basis reads these arms as one window — both peak at 4000 —
    # and that is CORRECT for the teardown case it exists to survive. What it
    # cannot see is that the wide arm's own cores straddle 4000 and 2000, so that
    # is carried separately and asserted here: a reader gets both facts.
    if not mixed["arms_saw_same_clock"]:
        raise RuntimeError("equal busy cores are one window on the busy-core basis")
    if mixed["same_clock_basis"] != "busy_core_max":
        raise RuntimeError("the same-window flag must key on the busy core")
    # NOTE: not named `spread` — that name holds the frequency vector these
    # fixtures are built from, and shadowing it fed a dict to the next builder.
    spread_ratio = mixed["arm_core_spread_ratio"]
    if round(spread_ratio["pandas"], 2) != 2.0:
        raise RuntimeError(f"the wide arm's 4000/2000 spread must be reported: {spread_ratio}")
    if round(spread_ratio["frankenpandas"], 2) != 1.0:
        raise RuntimeError("a 1-thread arm's own spread is 1.0 by construction")
    if mixed["arm_core_mhz_top_k"]["pandas"]["slowest"] != 2000.0:
        raise RuntimeError("the slow tail must be reported, not averaged away")

    # Equal thread counts on the same machine state remain one window.
    even = summarize_host_state(
        [vec("frankenpandas", spread), vec("pandas", spread)] * 3,
        {"frankenpandas": 8, "pandas": 8},
    )
    if not even["arms_saw_same_clock"]:
        raise RuntimeError("identical thread counts must read as one window")

    # k larger than the machine must clamp rather than raise.
    clamped = summarize_host_state(
        [vec("frankenpandas", [3000.0, 2000.0]), vec("pandas", [3000.0, 2000.0])] * 2,
        {"frankenpandas": 999, "pandas": 999},
    )
    if clamped["arm_core_mhz_top_k"]["pandas"]["median"] != 2500.0:
        raise RuntimeError("k beyond the CPU count must clamp to the sampled cores")

    # The in-memory vector must never reach the artifact.
    if any(key.startswith("_") for key in mixed):
        raise RuntimeError("host state must not emit the raw frequency vector")

    # Arms inside 5% are one window.
    tight = summarize_host_state(
        [sample("pandas", 3000.0), sample("frankenpandas", 3050.0)] * 3
    )
    if not tight["arms_saw_same_clock"]:
        raise RuntimeError("arms within 5% must read as the same window")

    # Load range is reported across the whole cell, not just its ends.
    ranged = summarize_host_state([
        ("pandas", {"loadavg": [10.0, 1, 1]}),
        ("frankenpandas", {"loadavg": [70.0, 1, 1]}),
        ("final", {"loadavg": [40.0, 1, 1]}),
    ])
    if ranged["loadavg_1min"] != {"first": 10.0, "last": 40.0, "min": 10.0, "max": 70.0}:
        raise RuntimeError(f"loadavg summary must span the cell: {ranged['loadavg_1min']}")

    # Degenerate input must not raise, and nothing here may gate.
    empty = summarize_host_state([])
    if empty.get("gate_input") is not False or "arms_saw_same_clock" in empty:
        raise RuntimeError("empty host state must be inert and non-gating")
    if skewed["gate_input"] is not False:
        raise RuntimeError("host state must declare itself a non-gate input")


def _best_vs_best_self_test() -> None:
    """Pin the dispersion diagnostic on the row that made it necessary.

    br-frankenpandas-mti15. The fixture is not synthetic-pretty: it is the
    retracted `df_dot @1M` row. FrankenPandas tight around 20 ms (min 18.63),
    pandas dispersed with a 23.67 ms median but an 11.55 ms best. The gated
    median said 1.187x FASTER; the minima said 0.620, a loss. A diagnostic that
    cannot separate those two is worthless, so this asserts it does.
    """
    def arm(engine: str, samples: list[float]) -> TimingResult:
        return TimingResult(
            workload="synthetic",
            category="linalg",
            size="1M",
            dtype="float64",
            engine=engine,
            times_us=samples,
            null_arm_a_us=samples[:2],
            null_arm_b_us=samples[:2],
            null_ratios=[1.0, 1.0],
        )

    # 1. AGREEMENT: both statistics say FrankenPandas is faster.
    fp_tight = arm("frankenpandas", [50.0, 51.0, 52.0, 50.5])
    pd_tight = arm("pandas", [100.0, 101.0, 99.0, 100.5])
    agreeing: dict[str, Any] = {"ratio": 2.0}
    annotate_best_vs_best(agreeing, fp_tight, pd_tight)
    detail = agreeing["best_vs_best"]
    if detail["frankenpandas_min_us"] != 50.0 or detail["pandas_min_us"] != 99.0:
        raise RuntimeError("best-vs-best must report each arm's fastest sample")
    if round(detail["ratio"], 4) != 1.98:
        raise RuntimeError("best-vs-best ratio must be pandas_min / frankenpandas_min")
    if not detail["direction_agrees_with_median"] or agreeing["dispersion_warning"]:
        raise RuntimeError("a tight, agreeing pair must not raise a dispersion warning")

    # 2. NEGATIVE CASE: the retracted row. Median says FASTER, minima say LOSS.
    fp_real = arm("frankenpandas", [18.63e3, 20.12e3, 21.65e3, 22.89e3])
    pd_real = arm("pandas", [11.55e3, 23.67e3, 28.10e3, 29.79e3])
    retracted: dict[str, Any] = {"ratio": 1.187}
    annotate_best_vs_best(retracted, fp_real, pd_real)
    detail = retracted["best_vs_best"]
    if detail["ratio"] >= 1.0:
        raise RuntimeError("the retracted fixture's minima must show a LOSS")
    if detail["direction_agrees_with_median"]:
        raise RuntimeError("median and minima disagree here — the diagnostic missed it")
    if not retracted["dispersion_warning"]:
        raise RuntimeError("a direction disagreement must raise the dispersion warning")

    # 3. The diagnostic must never touch the verdict.
    if "verdict" in retracted or "median_ci_gate" in retracted:
        raise RuntimeError("best-vs-best must not write verdict or gate fields")
    if detail["gate_input"]:
        raise RuntimeError("best-vs-best must declare itself a non-gate input")

    # 4. An empty arm must be skipped rather than divide by zero.
    empty: dict[str, Any] = {"ratio": 1.0}
    annotate_best_vs_best(empty, arm("frankenpandas", []), pd_tight)
    if "best_vs_best" in empty:
        raise RuntimeError("an arm with no samples must not produce a best-vs-best row")


def _balanced_square_self_test() -> None:
    """Pin the busy-host pairing and its outer A/A null control."""
    if BALANCED_SQUARE != "ABBAABBA" or BALANCED_SQUARE.count("A") != 4:
        raise RuntimeError("balanced square must contain four placements per arm")
    if [index for index, arm in enumerate(BALANCED_SQUARE) if arm == "A"] != [0, 3, 4, 7]:
        raise RuntimeError("pandas positions drifted from the sanctioned square")

    def slot(engine: str, p50_us: float) -> TimingResult:
        return TimingResult(
            workload="synthetic",
            category="strings",
            size="1M",
            dtype="float64",
            engine=engine,
            times_us=[p50_us] * 4,
            null_arm_a_us=[p50_us, p50_us],
            null_arm_b_us=[p50_us, p50_us],
            null_ratios=[1.0, 1.0],
            checksum=f"{engine}-{p50_us}",
            executable_sha256=f"{engine}-sha",
            executable_bytes=1,
            executable_path=f"/{engine}",
            runtime_available_parallelism=1,
            process_threads_before_probe=1,
            peak_process_threads=1,
            operation_threads_used=1,
            runtime_detected_isa_features=["avx2"],
        )

    fp_result = _balanced_square_aggregate(
        [slot("frankenpandas", 50.0)] * 4,
        engine="frankenpandas",
    )
    pd_result = _balanced_square_aggregate(
        [slot("pandas", 100.0)] * 4,
        engine="pandas",
    )
    comparison = compute_comparison(fp_result, pd_result, 1)
    experiment = {
        "round_ratio_pandas_over_frankenpandas": [2.0] * 5,
    }
    apply_balanced_square_gate(comparison, fp_result, pd_result, experiment)
    if comparison["ratio"] != 2.0 or comparison["verdict"] != "FASTER":
        raise RuntimeError("paired balanced-square ratio must remain decidable")
    if fp_result.null_median_ratio != 1.0 or pd_result.null_median_ratio != 1.0:
        raise RuntimeError("balanced-square A/A null must land at 1.0")


def compute_comparison(fp_result: TimingResult, pd_result: TimingResult,
                       rows: int) -> dict[str, Any]:
    """Compute head-to-head comparison metrics."""
    result = {
        "workload": fp_result.workload,
        "category": fp_result.category,
        "size": fp_result.size,
        "dtype": fp_result.dtype,
    }

    if fp_result.is_valid:
        result["frankenpandas"] = fp_result.to_metrics(rows)
        result["frankenpandas"]["iterations"] = len(fp_result.times_us)
        result["frankenpandas"]["valid"] = True
    else:
        result["frankenpandas"] = {"error": "contract_invalid_or_no_data"}

    if pd_result.is_valid:
        result["pandas"] = pd_result.to_metrics(rows)
        result["pandas"]["iterations"] = len(pd_result.times_us)
        result["pandas"]["valid"] = True
    else:
        result["pandas"] = {"error": "contract_invalid_or_no_data"}

    if fp_result.times_us and pd_result.times_us:
        if fp_result.is_valid and pd_result.is_valid:
            ratio = pd_result.p50_us / fp_result.p50_us if fp_result.p50_us > 0 else 0
            effect_ci = bootstrap_median_ratio_ci(
                pd_result.times_us,
                fp_result.times_us,
            )
            combined_null_log_half_width = max(
                fp_result.null_log_half_width,
                pd_result.null_log_half_width,
            )
            required_log_effect = DECIDABILITY_MARGIN * combined_null_log_half_width
            gate = corrected_null_gate(
                ratio,
                effect_ci,
                required_log_effect,
                fp_result.null_median_ratio,
                pd_result.null_median_ratio,
            )
            result["ratio"] = round(ratio, 3)
            result["median_ci_gate"] = gate | {
                "margin_multiplier": DECIDABILITY_MARGIN,
                "combined_two_x_null_interval": [
                    round(math.exp(-required_log_effect), 6),
                    round(math.exp(required_log_effect), 6),
                ],
                "cv_is_provenance_only": True,
            }
            result["verdict"] = (
                "FASTER" if gate["decidable"] and ratio > 1.0 else
                "SLOWER" if gate["decidable"] else
                "NULL_UNDECIDABLE"
            )
            # br-frankenpandas-mti15: the legacy independent-sample path gets the
            # same diagnostic, so no gated row anywhere can lack it.
            annotate_best_vs_best(result, fp_result, pd_result)
        else:
            result["ratio"] = None
            result["verdict"] = "CONTRACT_INVALID"
    else:
        result["ratio"] = None
        result["verdict"] = "INCOMPLETE"

    return result


def compute_candidate_vs_reference(
    candidate: TimingResult,
    reference: TimingResult,
) -> dict[str, Any]:
    """Adjudicate a whole-binary candidate against its default-build control.

    The ratio is reference/candidate, so values above one mean the candidate
    is faster. Both subprocesses execute inside the same harness invocation,
    and each contributes its own same-process A/A null control.
    """
    if not candidate.is_valid or not reference.is_valid:
        return {
            "ratio": None,
            "ratio_definition": "reference_p50 / candidate_p50",
            "verdict": "CONTRACT_INVALID",
        }

    ratio = reference.p50_us / candidate.p50_us
    effect_ci = bootstrap_median_ratio_ci(
        reference.times_us,
        candidate.times_us,
    )
    combined_null_log_half_width = max(
        candidate.null_log_half_width,
        reference.null_log_half_width,
    )
    required_log_effect = DECIDABILITY_MARGIN * combined_null_log_half_width
    gate = corrected_null_gate(
        ratio,
        effect_ci,
        required_log_effect,
        candidate.null_median_ratio,
        reference.null_median_ratio,
        "candidate",
        "reference",
    )
    return {
        "ratio": round(ratio, 8),
        "ratio_definition": "reference_p50 / candidate_p50",
        "median_ci_gate": gate | {
            "margin_multiplier": DECIDABILITY_MARGIN,
            "combined_two_x_null_interval": [
                round(math.exp(-required_log_effect), 6),
                round(math.exp(required_log_effect), 6),
            ],
            "cv_is_provenance_only": True,
        },
        "verdict": (
            "CANDIDATE_FASTER" if gate["decidable"] and ratio > 1.0 else
            "CANDIDATE_SLOWER" if gate["decidable"] else
            "NULL_UNDECIDABLE"
        ),
    }


def build_thread_provenance(
    fingerprint: dict[str, Any],
    requested_thread_count: int | None,
    fp_result: TimingResult,
    pd_result: TimingResult,
) -> dict[str, Any]:
    """Build and validate the mandatory per-cell scaling provenance."""
    affinity_cap = fingerprint["affinity_logical_cpu_cap"]
    errors = []
    if requested_thread_count is not None and affinity_cap != requested_thread_count:
        errors.append(
            "requested thread count does not match the effective affinity cap"
        )
    for result in (fp_result, pd_result):
        if result.runtime_available_parallelism != affinity_cap:
            errors.append(
                f"{result.engine} runtime_available_parallelism="
                f"{result.runtime_available_parallelism} != affinity cap {affinity_cap}"
            )
        if not result.operation_threads_used or result.operation_threads_used < 1:
            errors.append(f"{result.engine} did not report actual operation threads")
        elif result.operation_threads_used > affinity_cap:
            errors.append(
                f"{result.engine} operation thread count "
                f"{result.operation_threads_used} exceeds affinity cap {affinity_cap}"
            )
        if not result.runtime_detected_isa_features:
            errors.append(f"{result.engine} did not report runtime ISA features")

    return {
        "host_identity": fingerprint["host_identity"],
        "cpu_model": fingerprint["cpu_model"],
        "physical_cores": fingerprint["physical_cores"],
        "logical_threads": fingerprint["logical_threads"],
        "thread_count_requested": requested_thread_count,
        "thread_count_actually_used": {
            "frankenpandas": fp_result.operation_threads_used,
            "pandas": pd_result.operation_threads_used,
        },
        "runtime_available_parallelism": {
            "frankenpandas": fp_result.runtime_available_parallelism,
            "pandas": pd_result.runtime_available_parallelism,
        },
        "runtime_detected_isa_features": {
            "host": fingerprint["runtime_detected_isa_features"],
            "frankenpandas": fp_result.runtime_detected_isa_features,
            "pandas": pd_result.runtime_detected_isa_features,
        },
        "affinity_or_cpuset_cap": {
            "logical_cpu_count": affinity_cap,
            "cpu_ids": fingerprint["affinity_cpus"],
        },
        "contract_errors": errors,
        "valid": not errors,
    }


def run_category(category: str, sizes: list[str], dtypes: list[str],
                 tmp_path: Path, fingerprint: dict[str, Any],
                 requested_thread_count: int | None,
                 exclusivity_gate: HostWideExclusivityGate | None,
                 measurement_mode: str,
                 balanced_square_rounds: int,
                 adaptive_rounds: bool = False,
                 fp_binary: Path | None = None,
                 fp_reference_binary: Path | None = None,
                 workload_filter: set[str] | None = None,
                 result_sink: list[dict[str, Any]] | None = None,
                 ) -> list[dict[str, Any]]:
    """Run all workloads in a category for given sizes and dtypes.

    `result_sink`, per br-frankenpandas-ooivn, receives each comparison AS IT
    COMPLETES rather than only via the return value. A gate rejection on a later
    row raises `SystemExit` out of this function, which used to discard every
    row measured before it — they lived only in the local list below. Rows
    reaching the sink have already passed their own pre/post measurement guards.
    """
    results = []
    workloads = PANDAS_WORKLOADS.get(category, {})
    if workload_filter is not None:
        unknown = workload_filter - set(workloads)
        if unknown:
            raise ValueError(f"Unknown workload(s) for {category}: {sorted(unknown)}")
        workloads = {name: func for name, func in workloads.items() if name in workload_filter}

    cell_index = 0
    for workload in workloads:
        for size in sizes:
            for dtype in dtypes:
                config = SIZE_CONFIGS[size]
                print(f"  [{category}] {workload} @ {size}/{dtype}...", end=" ", flush=True)

                if measurement_mode == "balanced-square":
                    fp_result, pd_result, experiment = run_balanced_square_cell(
                        category,
                        workload,
                        size,
                        dtype,
                        tmp_path,
                        fingerprint,
                        fp_binary,
                        balanced_square_rounds,
                        adaptive_rounds,
                    )
                    comparison = compute_comparison(
                        fp_result,
                        pd_result,
                        config["rows"],
                    )
                    if fp_result.is_valid and pd_result.is_valid:
                        apply_balanced_square_gate(
                            comparison,
                            fp_result,
                            pd_result,
                            experiment,
                        )
                    else:
                        comparison["balanced_square"] = experiment
                    comparison["host_wide_quiescence"] = {
                        "required": False,
                        "replacement": "balanced-square-abbaabba-v1",
                        "valid": True,
                    }
                    if category == "pipeline":
                        equivalence = compare_pipeline_outputs(tmp_path, workload)
                        comparison["output_equivalence"] = equivalence
                        if not equivalence["equivalent"]:
                            comparison["verdict"] = "OUTPUT_MISMATCH"
                    if workload == "astype_str_f64_telemetry_batches":
                        comparison["allocator_provenance"] = {
                            "frankenpandas": {
                                "allocator": "mimalloc",
                                "purge_delay_ms": int(
                                    TELEMETRY_MIMALLOC_PURGE_DELAY_MS
                                ),
                                "reason": "purge rendered-batch pages at the timed drop boundary",
                            }
                        }
                    comparison["thread_provenance"] = build_thread_provenance(
                        fingerprint,
                        requested_thread_count,
                        fp_result,
                        pd_result,
                    )
                    # RECOMPUTED, because `like_for_like` runs inside the gate and
                    # `thread_provenance` is only attached here -- so the thread-cap
                    # reason would be permanently invisible if it were left as the
                    # gate computed it. Idempotent: same inputs, same answer, plus
                    # the one field that was not populated yet.
                    comparison["like_for_like"] = like_for_like(comparison)
                    if not comparison["thread_provenance"]["valid"]:
                        comparison["verdict"] = "CONTRACT_INVALID"
                        comparison["ratio"] = None
                    results.append(comparison)
                    if result_sink is not None:
                        result_sink.append(comparison)
                    verdict = comparison.get("verdict", "N/A")
                    ratio = comparison.get("ratio")
                    ratio_str = f"{ratio:.2f}x" if ratio else "N/A"
                    print(f"{verdict} ({ratio_str})")
                    # A verdict a reader can act on: an unlike-for-like row says so HERE,
                    # where the number is read, not only in the artifact.
                    if comparison.get("like_for_like", {}).get("ok") is False:
                        for reason in comparison["like_for_like"]["reasons"]:
                            print(f"    NOT LIKE-FOR-LIKE: {reason}")
                    cell_index += 1
                    continue

                pd_result, pandas_quiescence = run_pandas_workload(
                    category,
                    workload,
                    size,
                    dtype,
                    tmp_path,
                    fingerprint,
                    exclusivity_gate,
                )
                fp_arms = [("candidate", fp_binary)]
                if fp_reference_binary is not None:
                    reference_arm = ("reference", fp_reference_binary)
                    # Alternate whole-binary order across cells so a fixed
                    # candidate-first drift cannot decide the family result.
                    fp_arms = (
                        [reference_arm, fp_arms[0]]
                        if cell_index % 2 == 0
                        else [fp_arms[0], reference_arm]
                    )
                fp_results: dict[str, TimingResult] = {}
                fp_quiescence_by_arm: dict[str, dict[str, Any]] = {}
                for arm_label, arm_binary in fp_arms:
                    arm_result, arm_quiescence = _run_host_exclusive_arm(
                        exclusivity_gate,
                        (
                            f"frankenpandas-{arm_label}:"
                            f"{category}/{workload}/{size}/{dtype}"
                        ),
                        partial(
                            run_fp_workload_subprocess,
                            category,
                            workload,
                            size,
                            dtype,
                            tmp_path if category == "pipeline" else None,
                            arm_binary,
                        ),
                    )
                    fp_results[arm_label] = arm_result
                    fp_quiescence_by_arm[arm_label] = arm_quiescence

                fp_result = fp_results["candidate"]
                fp_quiescence = fp_quiescence_by_arm["candidate"]

                comparison = compute_comparison(fp_result, pd_result, config["rows"])
                comparison["host_wide_quiescence"] = {
                    "pandas": pandas_quiescence,
                    "frankenpandas": fp_quiescence,
                    "valid": True,
                }
                if fp_reference_binary is not None:
                    reference_result = fp_results["reference"]
                    reference_comparison = compute_comparison(
                        reference_result,
                        pd_result,
                        config["rows"],
                    )
                    reference_thread_provenance = build_thread_provenance(
                        fingerprint,
                        requested_thread_count,
                        reference_result,
                        pd_result,
                    )
                    comparison["whole_binary_reference"] = {
                        "frankenpandas": reference_comparison["frankenpandas"],
                        "ratio_vs_pandas": reference_comparison["ratio"],
                        "median_ci_gate_vs_pandas": reference_comparison.get(
                            "median_ci_gate"
                        ),
                        "verdict_vs_pandas": reference_comparison["verdict"],
                        "thread_provenance_vs_pandas": (
                            reference_thread_provenance
                        ),
                    }
                    comparison["candidate_vs_reference"] = (
                        compute_candidate_vs_reference(
                            fp_result,
                            reference_result,
                        )
                    )
                    comparison["whole_binary_execution_order"] = [
                        label for label, _ in fp_arms
                    ]
                    comparison["host_wide_quiescence"]["reference"] = (
                        fp_quiescence_by_arm["reference"]
                    )
                    if not reference_thread_provenance["valid"]:
                        comparison["whole_binary_reference"][
                            "ratio_vs_pandas"
                        ] = None
                        comparison["whole_binary_reference"][
                            "verdict_vs_pandas"
                        ] = "CONTRACT_INVALID"
                        comparison["candidate_vs_reference"] = {
                            "ratio": None,
                            "ratio_definition": (
                                "reference_p50 / candidate_p50"
                            ),
                            "verdict": "CONTRACT_INVALID",
                        }
                if category == "pipeline":
                    # A whole-job ratio is only meaningful if both arms did the
                    # same job. The per-engine `checksum` is a liveness token,
                    # not a content hash, and cannot compare across engines --
                    # so diff what the job actually produced. Disagreement
                    # voids the verdict rather than being reported alongside a
                    # number that no longer means anything.
                    equivalence = compare_pipeline_outputs(tmp_path, workload)
                    comparison["output_equivalence"] = equivalence
                    if not equivalence["equivalent"]:
                        comparison["verdict"] = "OUTPUT_MISMATCH"
                if workload == "astype_str_f64_telemetry_batches":
                    comparison["allocator_provenance"] = {
                        "frankenpandas": {
                            "allocator": "mimalloc",
                            "purge_delay_ms": int(
                                TELEMETRY_MIMALLOC_PURGE_DELAY_MS
                            ),
                            "reason": (
                                "purge rendered-batch pages at the timed "
                                "drop boundary"
                            ),
                        }
                    }
                comparison["thread_provenance"] = build_thread_provenance(
                    fingerprint,
                    requested_thread_count,
                    fp_result,
                    pd_result,
                )
                if not comparison["thread_provenance"]["valid"]:
                    comparison["verdict"] = "CONTRACT_INVALID"
                    comparison["ratio"] = None
                    if "candidate_vs_reference" in comparison:
                        comparison["candidate_vs_reference"] = {
                            "ratio": None,
                            "ratio_definition": (
                                "reference_p50 / candidate_p50"
                            ),
                            "verdict": "CONTRACT_INVALID",
                        }
                results.append(comparison)
                if result_sink is not None:
                    # br-frankenpandas-ooivn: publish immediately so a later
                    # rejection cannot take this row down with it.
                    result_sink.append(comparison)
                cell_index += 1

                verdict = comparison.get("verdict", "N/A")
                ratio = comparison.get("ratio")
                ratio_str = f"{ratio:.2f}x" if ratio else "N/A"
                print(f"{verdict} ({ratio_str})")
                # A verdict a reader can act on: an unlike-for-like row says so HERE,
                # where the number is read, not only in the artifact.
                if comparison.get("like_for_like", {}).get("ok") is False:
                    for reason in comparison["like_for_like"]["reasons"]:
                        print(f"    NOT LIKE-FOR-LIKE: {reason}")

    return results


def main():
    harness_identity = executable_identity(Path(sys.executable))
    harness_source_identity = executable_identity(Path(__file__))
    print(
        "bench_elf_sha256="
        f"{harness_identity['sha256']} "
        f"({harness_identity['bytes']} bytes) "
        f"{harness_identity['path']}"
    )
    print(
        "bench_harness_source_sha256="
        f"{harness_source_identity['sha256']} "
        f"({harness_source_identity['bytes']} bytes) "
        f"{harness_source_identity['path']}"
    )

    parser = argparse.ArgumentParser(description="vs-pandas head-to-head timing harness")
    parser.add_argument("--category",
                        choices=list(CATEGORIES.keys())
                        + list(EXTRA_CATEGORIES.keys()),
                        help="Run specific category")
    parser.add_argument("--all", action="store_true", help="Run all categories")
    parser.add_argument("--sizes", default="10k,100k,1M",
                        help="Comma-separated sizes (10k,100k,1M,2M,4M,6M,8M,10M)")
    parser.add_argument("--dtypes", default="float64",
                        help="Comma-separated dtypes")
    parser.add_argument("--workloads",
                        help="Comma-separated workload names within the selected category")
    parser.add_argument("--output", type=Path, help="Output JSON file")
    parser.add_argument(
        "--json-stdout",
        action="store_true",
        help="Emit one bench_result_json=<compact JSON> line for remote capture",
    )
    parser.add_argument(
        "--dependency-probe",
        action="store_true",
        help="Exit 0 only when the pinned pandas incumbent is importable",
    )
    parser.add_argument(
        "--thread-count",
        type=int,
        help=(
            "Required effective logical-CPU budget for this invocation; the "
            "harness refuses to run unless it equals the process affinity cap"
        ),
    )
    parser.add_argument(
        "--expected-hostname",
        help="Fail closed unless the benchmark host has this exact hostname",
    )
    parser.add_argument(
        "--expected-physical-cores",
        type=int,
        help="Fail closed unless host topology reports this physical-core count",
    )
    parser.add_argument(
        "--expected-logical-threads",
        type=int,
        help="Fail closed unless host topology reports this logical-thread count",
    )
    parser.add_argument(
        "--frankenpandas-build-worker",
        help=(
            "Remote worker identity that built the measured FrankenPandas ELF; "
            "recorded beside its executable SHA-256"
        ),
    )
    parser.add_argument(
        "--frankenpandas-binary",
        type=Path,
        help=(
            "Explicit immutable fp-bench ELF to execute; avoids separate Cargo "
            "target directories in whole-binary experiments"
        ),
    )
    parser.add_argument(
        "--frankenpandas-reference-binary",
        type=Path,
        help=(
            "Optional immutable default-build fp-bench ELF; executes beside "
            "the candidate and live pandas in the same invocation"
        ),
    )
    parser.add_argument(
        "--frankenpandas-reference-build-worker",
        help=(
            "Remote worker identity that built the optional whole-binary "
            "reference ELF"
        ),
    )
    parser.add_argument(
        "--measurement-mode",
        choices=("balanced-square", "host-wide-exclusive"),
        default="balanced-square",
        help=(
            "Comparison design: balanced-square interleaves pandas and FP on "
            "a shared host; host-wide-exclusive retains the legacy all-CPU "
            "quiescence gate for a booked machine"
        ),
    )
    parser.add_argument(
        "--adaptive-rounds",
        action="store_true",
        help=(
            "Recompute the balanced-square round count once, at the end of round 0, "
            "from the incumbent slot p50s just measured, so a noisy workload gets "
            "the same null-median precision as a quiet one. Default OFF: this "
            "changes how many samples a row is built from, so no previously banked "
            "row's methodology moves unless it is asked for. The round count can "
            "only GROW (br-frankenpandas-flicz)."
        ),
    )
    parser.add_argument(
        "--balanced-square-rounds",
        type=int,
        default=BALANCED_SQUARE_ROUNDS,
        help="Number of ABBAABBA paired rounds in balanced-square mode",
    )
    parser.add_argument(
        "--balanced-square-self-test",
        action="store_true",
        help="Exercise the balanced-square paired ratio and A/A null contract",
    )
    parser.add_argument(
        "--pin-cpus",
        help=(
            "CPU set (e.g. 0-15 or 0-7,32-39) applied to BOTH arms: this process "
            "runs the pandas arm there and the FrankenPandas child is launched "
            "under taskset with the same set. Recorded in every row"
        ),
    )
    parser.add_argument(
        "--slot-sampler-self-test",
        action="store_true",
        help="Exercise mid-slot clock sampling and its fallback",
    )
    parser.add_argument(
        "--like-for-like-self-test",
        action="store_true",
        help="Exercise the combined clock/minima like-for-like verdict",
    )
    parser.add_argument(
        "--host-state-self-test",
        action="store_true",
        help="Exercise the per-arm CPU MHz / loadavg instrument and its clock-skew flag",
    )
    parser.add_argument(
        "--best-vs-best-self-test",
        action="store_true",
        help=(
            "Exercise the best-vs-best dispersion diagnostic on the retracted "
            "df_dot @1M fixture (median says FASTER, minima say LOSS)"
        ),
    )
    parser.add_argument(
        "--host-exclusivity-self-test",
        action="store_true",
        help="Exercise the fail-closed host-wide quiescence adjudicator and exit",
    )
    parser.add_argument(
        "--row-persistence-self-test",
        action="store_true",
        help="Exercise the contract that a measured row is always written to disk",
    )
    parser.add_argument(
        "--corrected-null-gate-self-test",
        action="store_true",
        help="Exercise the corrected three-clause null gate and exit",
    )
    parser.add_argument(
        "--host-readiness-probe",
        action="store_true",
        help=(
            "Report whether this host would pass the exclusivity gate right "
            "now (exit 0 clear, 2 blocked) and exit, without building or "
            "measuring anything"
        ),
    )
    parser.add_argument(
        "--readiness-wait-seconds",
        type=float,
        default=0.0,
        help=(
            "With --host-readiness-probe, keep sampling for up to this many "
            "seconds and exit 0 on the first clear window"
        ),
    )
    args = parser.parse_args()

    if args.host_readiness_probe:
        sys.exit(_host_readiness_probe(args.readiness_wait_seconds))

    if args.host_exclusivity_self_test:
        _host_wide_exclusivity_self_test()
        print("host_wide_exclusivity_self_test=pass")
        return

    if args.corrected_null_gate_self_test:
        _corrected_null_gate_self_test()
        print("corrected_null_gate_self_test=pass")
        return

    if args.balanced_square_self_test:
        _balanced_square_self_test()
        print("balanced_square_self_test=pass")
        return

    if args.best_vs_best_self_test:
        _best_vs_best_self_test()
        print("best_vs_best_self_test=pass")
        return

    if args.host_state_self_test:
        _host_state_self_test()
        print("host_state_self_test=pass")
        return

    if args.like_for_like_self_test:
        _like_for_like_self_test()
        print("like_for_like_self_test=pass")
        return

    if args.slot_sampler_self_test:
        _slot_sampler_self_test()
        print("slot_sampler_self_test=pass")
        return

    if args.pin_cpus:
        requested = parse_cpu_spec(args.pin_cpus)
        if not requested:
            parser.error(f"--pin-cpus parsed to an empty CPU set: {args.pin_cpus!r}")
        try:
            os.sched_setaffinity(0, set(requested))
        except (AttributeError, OSError) as error:
            parser.error(f"--pin-cpus could not be applied: {error}")
        applied = sorted(os.sched_getaffinity(0))
        if applied != requested:
            parser.error(
                f"--pin-cpus asked for {cpu_mask_spec(requested)} but the kernel "
                f"granted {cpu_mask_spec(applied)}"
            )
        globals()["ARM_CPU_MASK"] = applied
        globals()["ARM_CPU_MASK_SOURCE"] = f"--pin-cpus {args.pin_cpus}"
        print(f"arm_cpu_mask={cpu_mask_spec(applied)} source=--pin-cpus")

    if args.row_persistence_self_test:
        _row_persistence_self_test()
        print("row_persistence_self_test=pass")
        return

    if args.dependency_probe:
        if pd is None:
            print("ERROR: pandas not installed", file=sys.stderr)
            sys.exit(1)
        if pd.__version__ != "2.2.3":
            print(
                f"ERROR: expected pandas 2.2.3, found {pd.__version__}",
                file=sys.stderr,
            )
            sys.exit(1)
        if pa is None:
            print("ERROR: pyarrow not installed", file=sys.stderr)
            sys.exit(1)
        if pa.__version__ != "24.0.0":
            print(
                f"ERROR: expected pyarrow 24.0.0, found {pa.__version__}",
                file=sys.stderr,
            )
            sys.exit(1)
        print(
            "pandas_dependency_probe=ready "
            f"version={pd.__version__} pyarrow_version={pa.__version__}"
        )
        return

    if (
        args.frankenpandas_reference_binary is not None
        and args.frankenpandas_binary is None
    ):
        parser.error(
            "--frankenpandas-reference-binary requires "
            "--frankenpandas-binary"
        )
    if (
        args.frankenpandas_reference_binary is not None
        and args.frankenpandas_binary is not None
        and args.frankenpandas_reference_binary.resolve(strict=True)
        == args.frankenpandas_binary.resolve(strict=True)
    ):
        parser.error("candidate and reference binaries must be distinct files")

    if not args.category and not args.all:
        parser.error("Specify --category or --all")

    if args.balanced_square_rounds < 3:
        parser.error("--balanced-square-rounds must be at least 3")

    if pd is None:
        print("ERROR: pandas not installed", file=sys.stderr)
        sys.exit(1)
    if pa is None:
        print("ERROR: pyarrow not installed", file=sys.stderr)
        sys.exit(1)

    fingerprint = host_fingerprint()
    host_contract_errors = []
    if args.thread_count is not None:
        if args.thread_count < 1:
            host_contract_errors.append("--thread-count must be positive")
        elif fingerprint["affinity_logical_cpu_cap"] != args.thread_count:
            host_contract_errors.append(
                f"thread_count={args.thread_count} but affinity cap is "
                f"{fingerprint['affinity_logical_cpu_cap']}"
            )
    if (
        args.expected_hostname is not None
        and fingerprint["host_identity"] != args.expected_hostname
    ):
        host_contract_errors.append(
            f"hostname={fingerprint['host_identity']!r}, expected "
            f"{args.expected_hostname!r}"
        )
    if (
        args.expected_physical_cores is not None
        and fingerprint["physical_cores"] != args.expected_physical_cores
    ):
        host_contract_errors.append(
            f"physical_cores={fingerprint['physical_cores']}, expected "
            f"{args.expected_physical_cores}"
        )
    if (
        args.expected_logical_threads is not None
        and fingerprint["logical_threads"] != args.expected_logical_threads
    ):
        host_contract_errors.append(
            f"logical_threads={fingerprint['logical_threads']}, expected "
            f"{args.expected_logical_threads}"
        )
    if host_contract_errors:
        for error in host_contract_errors:
            print(f"ERROR: thread provenance contract: {error}", file=sys.stderr)
        sys.exit(2)

    exclusivity_gate: HostWideExclusivityGate | None = None
    if args.measurement_mode == "host-wide-exclusive":
        online_cpu_ids = _online_cpu_ids()
        if not online_cpu_ids:
            print(
                "ERROR: host-wide benchmark exclusivity found no online CPUs",
                file=sys.stderr,
            )
            sys.exit(2)
        exclusivity_gate = HostWideExclusivityGate(online_cpu_ids)
        # Admission must precede our own 228 MiB provenance hash. Otherwise
        # the first host sample can reject an idle machine for work this
        # harness just created, misclassifying self-load as a co-tenant.
        exclusivity_gate.wait_until_quiet("invocation_preflight")

    timestamp = datetime.now(timezone.utc).isoformat()
    invocation_id = (
        "vs-pandas-"
        f"{datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')}-"
        f"pid{os.getpid()}"
    )
    pandas_artifact = pandas_artifact_identity()
    pyarrow_artifact = pyarrow_artifact_identity()
    print(
        "pandas_artifact_sha256="
        f"{pandas_artifact['sha256']} "
        f"({pandas_artifact['bytes']} bytes, "
        f"{pandas_artifact['files']} files) "
        f"{pandas_artifact['path']}"
    )
    print(
        "pyarrow_artifact_sha256="
        f"{pyarrow_artifact['sha256']} "
        f"({pyarrow_artifact['bytes']} bytes, "
        f"{pyarrow_artifact['files']} files) "
        f"{pyarrow_artifact['path']}"
    )
    print(f"bench_invocation_id={invocation_id}")
    print(
        "benchmark_host_fingerprint_json="
        + json.dumps(fingerprint, separators=(",", ":"), sort_keys=True)
    )
    if exclusivity_gate is not None:
        # Hashing the pandas + pyarrow installation can wake filesystem kernel
        # workers outside this process's affinity mask. Wait boundedly for
        # that self-induced residue, then demand a fresh immediate checkpoint
        # before fixture setup. Readiness retries are retained in the JSON.
        time.sleep(PROVENANCE_QUIESCENCE_SETTLE_SECONDS)
        exclusivity_gate.wait_until_quiet("post_provenance")

    sizes = [s.strip() for s in args.sizes.split(",")]
    dtypes = [d.strip() for d in args.dtypes.split(",")]
    workload_filter = (
        {w.strip() for w in args.workloads.split(",") if w.strip()}
        if args.workloads
        else None
    )
    categories = list(CATEGORIES.keys()) if args.all else [args.category]

    if args.measurement_mode == "balanced-square":
        if args.all or len(categories) != 1:
            parser.error("balanced-square mode requires exactly one category")
        if workload_filter is None or len(workload_filter) != 1:
            parser.error("balanced-square mode requires exactly one --workloads entry")
        if len(sizes) != 1 or len(dtypes) != 1:
            parser.error("balanced-square mode requires exactly one size and dtype")
        if args.frankenpandas_reference_binary is not None:
            parser.error(
                "balanced-square mode compares pandas and one FP ELF; "
                "whole-binary reference requires --measurement-mode host-wide-exclusive"
            )

    import tempfile
    with tempfile.TemporaryDirectory() as tmp_dir:
        tmp_path = Path(tmp_dir)

        all_results = []

        print("=== vs-pandas Benchmark Harness ===")
        print(f"Timestamp: {timestamp}")
        print(f"Categories: {', '.join(categories)}")
        print(f"Sizes: {', '.join(sizes)}")
        print(f"Dtypes: {', '.join(dtypes)}")
        if workload_filter is not None:
            print(f"Workloads: {', '.join(sorted(workload_filter))}")
        print(f"pandas version: {pd.__version__}")
        print()

        # br-frankenpandas-ooivn: rows land in `all_results` as they complete
        # (via result_sink), so a rejection part-way through a category keeps
        # everything measured before it. The rejection is still fatal below.
        invocation_rejection: dict[str, Any] | None = None

        def annotate(result: dict[str, Any]) -> None:
            """Stamp invocation/build provenance onto a completed row."""
            result["invocation_id"] = invocation_id
            if args.frankenpandas_build_worker is not None:
                result["frankenpandas"]["executable"]["build_worker"] = (
                    args.frankenpandas_build_worker
                )
            if (
                args.frankenpandas_reference_build_worker is not None
                and "whole_binary_reference" in result
            ):
                result["whole_binary_reference"]["frankenpandas"][
                    "executable"
                ]["build_worker"] = args.frankenpandas_reference_build_worker

        try:
            for category in categories:
                weight = CATEGORIES.get(category)
                label = (
                    f"weight: {weight}" if weight is not None
                    else "outside the weighted score"
                )
                print(f"\n[{category.upper()}] ({label})")
                # Rows are published into all_results by run_category as each
                # one completes, so nothing here re-extends it.
                run_category(
                    category,
                    sizes,
                    dtypes,
                    tmp_path,
                    fingerprint,
                    args.thread_count,
                    exclusivity_gate,
                    args.measurement_mode,
                    args.balanced_square_rounds,
                    args.adaptive_rounds,
                    args.frankenpandas_binary,
                    args.frankenpandas_reference_binary,
                    workload_filter,
                    result_sink=all_results,
                )
            if exclusivity_gate is not None:
                exclusivity_gate.require_quiet("invocation_postflight")
        except SystemExit:
            if exclusivity_gate is None:
                raise
            invocation_rejection = exclusivity_gate.last_rejection or {
                "phase": "unknown",
                "kind": "unrecorded",
            }
        # Annotate whatever survived, including rows banked before a rejection.
        for result in all_results:
            annotate(result)

        # br-frankenpandas-ooivn: a gate rejection anywhere above — inside a
        # category, or at invocation_postflight — used to abort BEFORE this
        # artifact was written, discarding rows whose OWN phases were all clear.
        # Two df_transpose @100k rows were lost that way, one to a rejection on
        # an unrelated row and one to this very postflight check, after both had
        # already measured cleanly.
        #
        # The rejection is still fatal and the process still exits 2. What
        # changes is only that evidence the gate already blessed gets banked
        # first, with the rejection recorded IN the artifact so the run is
        # self-describing. No threshold, phase or verdict is altered, and no row
        # is promoted: a row whose own phases were not clear never reaches
        # `all_results` in the first place.
        output = {
            "schema_version": "v4",
            "timestamp": timestamp,
            "invocation_id": invocation_id,
            "engine_identity": {
                "frankenpandas": {
                    "version": "0.1.2",
                    "profile": "release-perf",
                    "role": "Subject",
                    "build_worker": args.frankenpandas_build_worker,
                },
                "frankenpandas_reference": (
                    {
                        "version": "0.1.2",
                        "profile": "release-perf",
                        "role": "Whole-binary default-build control",
                        "build_worker": (
                            args.frankenpandas_reference_build_worker
                        ),
                    }
                    if args.frankenpandas_reference_binary is not None
                    else None
                ),
                "pandas": {
                    "version": pd.__version__,
                    "role": "Oracle",
                    "executable": harness_identity,
                    "artifact": pandas_artifact,
                    "optional_backends": {
                        "pyarrow": {
                            "version": pa.__version__,
                            "artifact": pyarrow_artifact,
                        },
                    },
                },
            },
            "harness_source": harness_source_identity,
            "host_fingerprint": fingerprint,
            "host_wide_exclusivity": (
                exclusivity_gate.artifact()
                if exclusivity_gate is not None
                else {
                    "required": False,
                    "replacement": "balanced-square-abbaabba-v1",
                    "valid": True,
                }
            ),
            "parameters": {
                "sizes": sizes,
                "dtypes": dtypes,
                "categories": categories,
                "paired_rounds": PAIRED_ROUNDS,
                "measurement_mode": args.measurement_mode,
                "balanced_square": (
                    {
                        "order": BALANCED_SQUARE,
                        "rounds": args.balanced_square_rounds,
                        "same_invocation_incumbent": True,
                        "paired_effect_ci": (
                            "bootstrap median of per-round pandas/FP ratios"
                        ),
                        "outer_aa_null": (
                            "first two versus final two placements per arm"
                        ),
                    }
                    if args.measurement_mode == "balanced-square"
                    else None
                ),
                "warmup_iterations": WARMUP_ITERATIONS,
                "bootstrap_resamples": BOOTSTRAP_RESAMPLES,
                "null_ci_confidence": NULL_CI_CONFIDENCE,
                "decidability_margin": DECIDABILITY_MARGIN,
                "gate": "corrected_three_clause_median_bootstrap_ci",
                "null_median_maximum_absolute_deviation": (
                    NULL_MEDIAN_MAX_ABS_DEVIATION
                ),
                "cv_role": "provenance_only",
                "frankenpandas_binary_requested": (
                    str(args.frankenpandas_binary.resolve(strict=True))
                    if args.frankenpandas_binary is not None
                    else None
                ),
                "frankenpandas_reference_binary_requested": (
                    str(
                        args.frankenpandas_reference_binary.resolve(
                            strict=True
                        )
                    )
                    if args.frankenpandas_reference_binary is not None
                    else None
                ),
                "whole_binary_order_policy": (
                    "alternate reference/candidate order by benchmark cell"
                    if args.frankenpandas_reference_binary is not None
                    else None
                ),
                "host_wide_exclusivity_contract": {
                    "required": args.measurement_mode == "host-wide-exclusive",
                    "scope": "all_online_host_cpus",
                    "maximum_busy_fraction": MAX_HOST_WIDE_BUSY_FRACTION,
                    "sample_interval_ms": round(
                        CPU_SAMPLE_INTERVAL_SECONDS * 1000
                    ),
                    "post_setup_settle_ms": round(
                        SETUP_QUIESCENCE_SETTLE_SECONDS * 1000
                    ),
                    "post_provenance_hash_settle_ms": round(
                        PROVENANCE_QUIESCENCE_SETTLE_SECONDS * 1000
                    ),
                    "readiness_wait_maximum_attempts": (
                        QUIESCENCE_WAIT_MAX_ATTEMPTS
                    ),
                    "readiness_wait_retry_ms": round(
                        QUIESCENCE_WAIT_RETRY_SECONDS * 1000
                    ),
                    "checks": (
                        "invocation admission before provenance hashing; "
                        "post-provenance readiness plus immediate checkpoint; "
                        "bounded readiness plus immediate checkpoint before "
                        "each pandas and FrankenPandas arm; immediate "
                        "post-arm checkpoints; invocation postflight"
                    ) if args.measurement_mode == "host-wide-exclusive" else (
                        "replaced by paired ABBAABBA slots on the shared host; "
                        "foreign load is paired instead of used as admission"
                    ),
                },
                "pandas_string_backend_policy": {
                    "unsuffixed_workloads": pandas_string_backend(),
                    "explicit_workload_suffixes": {
                        "_object": "object",
                        "_arrow": "string[pyarrow]",
                    },
                },
                "workload_allocator_policy": {
                    "astype_str_f64_telemetry_batches": {
                        "frankenpandas_mimalloc_purge_delay_ms": int(
                            TELEMETRY_MIMALLOC_PURGE_DELAY_MS
                        )
                    }
                },
                "shared_invocation_id": invocation_id,
                "thread_provenance_contract": {
                    "thread_count_requested": args.thread_count,
                    "expected_hostname": args.expected_hostname,
                    "expected_physical_cores": args.expected_physical_cores,
                    "expected_logical_threads": args.expected_logical_threads,
                    "affinity_or_cpuset_cap": {
                        "logical_cpu_count": fingerprint[
                            "affinity_logical_cpu_cap"
                        ],
                        "cpu_ids": fingerprint["affinity_cpus"],
                    },
                },
            },
            "results": all_results,
            "summary": compute_summary(all_results),
            # br-frankenpandas-ooivn: null on a fully clean run. When present,
            # the invocation FAILED CLOSED and the rows below are only those the
            # gate had already blessed individually — never treat such an
            # artifact as a complete run.
            "invocation_rejection": invocation_rejection,
        }

        # A MEASURED ROW ALWAYS LANDS ON DISK. `--json-stdout` used to SUPPRESS
        # the artifact (`elif not args.json_stdout`), so the sanctioned recipe
        # -- which passes `--json-stdout` and no `--output` -- measured a row
        # and persisted nothing. Every fingerprint that makes a row comparable
        # lives in that artifact: host_fingerprint.host_identity, cpu_model,
        # logical_threads, the ISA feature set, harness_source.sha256 and the
        # self-reported executing ELF SHA-256. Printing them to a terminal that
        # is later closed is not banking them.
        #
        # THIS IS NOT HYPOTHETICAL. The str_startswith_arrow @1M = 5.105x row in
        # docs/NEGATIVE_EVIDENCE.md was taken with exactly that recipe, so no
        # file exists for it and its host is attested only by the ledger's prose.
        # It is a fully compliant same-invocation balanced-square measurement
        # that cannot be audited, purely because it was not saved.
        # (br-frankenpandas-s7x8z)
        out_file = resolve_results_path(args.output, timestamp)
        out_file.parent.mkdir(parents=True, exist_ok=True)
        out_file.write_text(json.dumps(output, indent=2))
        print(f"\nResults written to: {out_file}")
        if args.json_stdout:
            print(
                "bench_result_json="
                + json.dumps(output, separators=(",", ":"), sort_keys=True)
            )

        # br-frankenpandas-ooivn: fail closed, AFTER the blessed rows are safely
        # on disk. The exit status is unchanged from before this fix — a
        # rejected invocation is still an error — so every caller that checks
        # the exit code keeps behaving exactly as it did.
        if invocation_rejection is not None:
            print(
                "ERROR: invocation failed closed at "
                f"phase={invocation_rejection.get('phase')}; "
                f"{len(all_results)} gate-clean row(s) were still written, "
                "and the artifact records the rejection",
                file=sys.stderr,
            )
            raise SystemExit(2)


def compute_summary(results: list[dict[str, Any]]) -> dict[str, Any]:
    """Compute weighted summary scores per category."""
    from collections import defaultdict
    import math

    by_category = defaultdict(list)
    for r in results:
        if r.get("verdict") in ("FASTER", "SLOWER"):
            by_category[r["category"]].append(r["ratio"])

    category_scores = {}
    for cat, ratios in by_category.items():
        if ratios:
            geomean = math.exp(sum(math.log(r) for r in ratios) / len(ratios))
            category_scores[cat] = round(geomean, 3)

    weighted_score = sum(
        category_scores.get(cat, 1.0) * weight
        for cat, weight in CATEGORIES.items()
    )

    contract_valid_count = sum(
        1
        for r in results
        if r.get("verdict") not in ("CONTRACT_INVALID", "INCOMPLETE")
    )
    decidable_count = sum(
        1 for r in results if r.get("verdict") in ("FASTER", "SLOWER")
    )
    null_count = sum(
        1 for r in results if r.get("verdict") == "NULL_UNDECIDABLE"
    )

    return {
        "total_workloads": len(results),
        "contract_valid_workloads": contract_valid_count,
        "decidable_workloads": decidable_count,
        "null_undecidable_workloads": null_count,
        "category_scores": category_scores,
        "weighted_score": round(weighted_score, 3),
        "claim_validated": all(
            category_scores.get(cat, 0) > 1.0
            for cat in CATEGORIES
        ),
    }


if __name__ == "__main__":
    main()
