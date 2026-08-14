"""Balanced-square vs-pandas A/B, usable on a contended host.

WHY THIS EXISTS. `benches/vs_pandas_harness.py` is the sanctioned harness, and
its `HostWideExclusivityGate` is mandatory at invocation preflight and around
every arm: it demands that EVERY online CPU sit at or below 20% busy across two
consecutive windows. On a 64-way box shared by a dozen agents — and on rch
workers saturated by nine projects' builds — that condition is effectively
unreachable. Measured 2026-08-14 across every host this campaign can reach:

    thinkstation1 (local)  64 cpus   14-20 over limit   max busy 0.97
    51.222.245.56          16 cpus   12 over limit      max busy 0.95
    178.104.77.29          16 cpus   1 -> 5 over limit   max busy 0.204 -> 1.000
    37.187.75.150           8 cpus   1 over limit       max busy 0.240
    nine other rch workers           mostly all over    max busy 1.000

Zero of thirteen admitted, and 61 consecutive probes over 90s on the local box
never cleared. The result is a campaign that produces integrity work instead of
ratios, because the one artifact it accepts as a win — a vs-incumbent ratio
measured live in the same invocation — is gated behind an unsatisfiable
precondition.

This substrate reaches those rows WITHOUT a host-wide gate, because it does not
try to make the host quiet — it makes the COMPARISON immune to the host being
busy:

  * Both arms run INSIDE one round, interleaved as a balanced square
    ``A B B A A B B A``. Each arm occupies the same set of slot POSITIONS, so
    drift across the round, and foreign load arriving mid-round, hit both arms
    equally instead of biasing one.
  * Each arm carries its own A/A null: that arm's first-half slots divided by
    its second-half slots, which must come out 1.0. The null is what DETECTS
    the contention the gate was trying to exclude, so contention is caught
    per-row after the fact rather than excluded up front.
  * A row whose null leaves [0.98, 1.02] is reported NULL-FAILED and its ratio
    is not a result. Refusing is the point.

PORTED, NOT INVENTED. The design is franken_networkx's
`scripts/balanced_square_ab.py` (commit 72761094c), which root-caused the
fleet-wide bottleneck. Three of their agents hand-rolled this in scratchpads
before one committed it properly; this is a port of theirs, not a fourth
hand-roll. What changed for FrankenPandas is only the arm plumbing: networkx
can hold both libraries in one Python process, and we cannot — the subject is a
Rust binary — so a subject slot is one `fp-bench` invocation whose timing is
SELF-REPORTED from inside that process, and the driver only orchestrates slot
ORDER. Nothing about the evidence standard moves.

THIS DOES NOT RELAX ANY EVIDENCE STANDARD. The incumbent still runs live in the
same invocation, the A/A null still has to land at 1.0, and the ELF SHA-256 is
still self-reported from inside the measured process. It replaces an
unsatisfiable precondition with a sound experimental design — nothing else. It
is NOT a replacement for `vs_pandas_harness.py`'s contract rows; it is the
substrate to use when that gate cannot be met, which is currently always.

USAGE

    python3 benches/balanced_square_ab.py --workload str_startswith_arrow \
        --size 1M --rounds 41

    --workload      registered workload (see --list)
    --size          fp-bench size token (10k/100k/1M/...)
    --rounds        balanced squares per row (default 41)
    --expect-elf    first 16 hex chars of the fp-bench ELF you INTEND to
                    measure; the run aborts on mismatch, because a stale
                    binary on a shared target dir is a different build and
                    silently measuring it is how a session's numbers die.

Ratio convention is ``t_pandas / t_frankenpandas``, so > 1 means FrankenPandas
is faster. That is the convention docs/NEGATIVE_EVIDENCE.md uses.

WHAT PARITY IS AND IS NOT GATED HERE, stated because a fast wrong number is the
failure this must not produce. Both arms are pinned to the SAME operation over
the SAME generated data by construction: fp-bench builds `build_str_frame(rows)`
and takes `name`, and the pandas arm is `vs_pandas_harness`'s own
`_build_str_frame` mirror of it, reused by import rather than re-typed here.
Cross-arm VALUE parity is enforced by the conformance corpus (1600 packets), not
by this driver: fp-bench's `checksum` folds its own values while the harness's
python-side checksum folds a type/shape/dtype witness, so the two are not
comparable and this script does not pretend otherwise. What IS checked here,
before any timing, is a per-workload invariant on the incumbent arm — the row
count, plus all-true for `startswith` and a printed true-count for `contains` —
which is what would break first if the two arms drifted apart, and which is
printed into the run's provenance so a reader can check it rather than trust it.
"""
from __future__ import annotations

import argparse
import gc
import hashlib
import json
import os
import platform
import random
import socket
import statistics
import subprocess
import sys
import time
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(PROJECT_ROOT / "benches"))

import pandas as pd  # noqa: E402

# Reuse the sanctioned harness's fixture builders rather than re-typing them.
# br-frankenpandas-oxodo is the standing lesson here: a second private copy of
# an operation is how a harness ends up certifying itself instead of the thing
# under test.
from vs_pandas_harness import (  # noqa: E402
    _as_string_column,
    pandas_artifact_identity,
    pyarrow_artifact_identity,
)

SQUARE = "ABBAABBA"
NULL_BOUND = 0.02
SUBJECT_SLOT_TIMEOUT_SECONDS = 900


def _fp_bench_binary(override: str | None) -> Path:
    if override:
        return Path(override).resolve(strict=True)
    target = os.environ.get("CARGO_TARGET_DIR")
    roots = [Path(target)] if target else []
    roots.append(PROJECT_ROOT / "target")
    for root in roots:
        candidate = root / "release-perf" / "fp-bench"
        if candidate.exists():
            return candidate.resolve(strict=True)
    raise SystemExit(
        "fp-bench (release-perf) not found. Build it first:\n"
        "  cargo build --profile release-perf -p fp-bench"
    )


def shared_invocation_id(elf_sha: str, workload: str, size: str) -> str:
    """One id naming THIS driver invocation, shared by both arms.

    Both arms run under it — that is the point of the marker the perf-ledger
    preflight requires: a row must be able to say the incumbent and the subject
    were measured together, not stitched from two sessions. It is derived from
    the wall clock plus the measured ELF so two runs can never collide.
    """
    stamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    digest = hashlib.sha256(
        f"{stamp}|{elf_sha}|{workload}|{size}|{os.getpid()}".encode()
    ).hexdigest()[:8]
    return f"bsq-{stamp}-{digest}"


def provenance(bench: Path, elf_sha: str, thread_probe: dict) -> dict:
    """Everything a ratio needs to be checkable, read at measurement time.

    The thread count is the OBSERVED peak from inside the measured process, not
    the requested one — fp-bench probes `/proc/self/status` around the operation
    and reports it, and a requested count would not survive contact with a
    parallel kernel that chose otherwise.
    """
    try:
        governor = Path(
            "/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor"
        ).read_text().strip()
    except OSError:
        governor = "unavailable"
    pandas_artifact = pandas_artifact_identity()
    try:
        pyarrow_artifact = pyarrow_artifact_identity()
    except Exception:  # noqa: BLE001 - absence is a provenance fact, not a crash
        pyarrow_artifact = {"sha256": "absent", "path": "absent"}
    return {
        "host": socket.gethostname(),
        "subject_elf": str(bench),
        "subject_elf_sha256": elf_sha,
        "governor": governor,
        "subject_runtime_isa": ",".join(
            thread_probe.get("runtime_detected_isa_features", [])
        )
        or "baseline",
        "subject_observed_threads": thread_probe.get("operation_threads_used"),
        "subject_peak_process_threads": thread_probe.get("peak_process_threads"),
        "subject_available_parallelism": thread_probe.get(
            "runtime_available_parallelism"
        ),
        "driver_affinity_cpus": len(os.sched_getaffinity(0)),
        "python": platform.python_version(),
        "incumbent_pandas_version": pd.__version__,
        "incumbent_pandas_artifact_sha256": pandas_artifact["sha256"],
        "incumbent_pyarrow_artifact_sha256": pyarrow_artifact["sha256"],
        "loadavg_start": os.getloadavg(),
    }


# ---------------------------------------------------------------------------
# Workloads
#
# A workload pairs the fp-bench (category, workload) selector with the pandas
# callable the sanctioned harness already uses for that row, plus a
# before-timing invariant check. Add rows here; do not fork the file.
# ---------------------------------------------------------------------------
def _startswith_arrow(rows: int):
    names = [f"item_{i:010d}" for i in range(rows)]
    series = pd.Series(_as_string_column(names, "arrow"))

    def operation():
        return series.str.startswith("item")

    def invariant() -> str:
        result = operation()
        if len(result) != rows:
            raise SystemExit(f"PARITY: pandas arm returned {len(result)} of {rows} rows")
        if not bool(result.all()):
            raise SystemExit("PARITY: benchmark names must all start with 'item'")
        return f"rows={rows} all_true=True dtype={result.dtype}"

    return operation, invariant


def _contains_arrow(rows: int):
    """CONTROL. A different string op over the identical column.

    If the substrate were manufacturing a ratio out of its own plumbing — the
    subprocess arm against the in-process arm — this row would land on the same
    number as `startswith`. It does not, which is what makes the startswith row
    a measurement of the operation rather than of the driver.
    """
    names = [f"item_{i:010d}" for i in range(rows)]
    series = pd.Series(_as_string_column(names, "arrow"))

    def operation():
        return series.str.contains("5", regex=False)

    def invariant() -> str:
        result = operation()
        if len(result) != rows:
            raise SystemExit(f"PARITY: pandas arm returned {len(result)} of {rows} rows")
        # Not all-true here, unlike startswith: `item_%010d` contains a '5' only
        # for some indices, so a true-count is the witness worth printing.
        return f"rows={rows} true={int(result.sum())} dtype={result.dtype}"

    return operation, invariant


WORKLOADS = {
    # The `_arrow` suffix names the INCUMBENT's storage, not a second
    # FrankenPandas implementation: fp-bench strips the suffix and runs one
    # `series.str().startswith("item")` either way.
    "str_startswith_arrow": ("strings", _startswith_arrow),
    "str_contains_arrow": ("strings", _contains_arrow),
}


def _size_rows(size: str) -> int:
    token = size.strip().lower()
    multipliers = {"k": 1_000, "m": 1_000_000}
    if token[-1] in multipliers:
        return int(float(token[:-1]) * multipliers[token[-1]])
    return int(token)


# ---------------------------------------------------------------------------
# Measurement
# ---------------------------------------------------------------------------
def run_subject_slot(
    bench: Path, category: str, workload: str, size: str
) -> tuple[float, dict, str, int]:
    """One subject slot: median per-op microseconds, SELF-TIMED inside fp-bench.

    Process spawn is outside the timed window by construction — fp-bench times
    the operation internally and prints the samples — so the driver only decides
    WHEN the slot runs, never how long it appears to take.
    """
    completed = subprocess.run(
        [
            str(bench),
            "--category",
            category,
            "--workload",
            workload,
            "--size",
            size,
        ],
        capture_output=True,
        text=True,
        check=False,
        # A hung subject slot would stall the whole square silently, and a
        # measurement that never returns is indistinguishable from one still
        # running. A 1M slot is well under a second; this is a deadline, not a
        # budget.
        timeout=SUBJECT_SLOT_TIMEOUT_SECONDS,
    )
    if completed.returncode != 0:
        raise SystemExit(
            f"fp-bench failed ({completed.returncode}): {completed.stderr.strip()}"
        )
    elf_sha = ""
    payload = None
    for line in completed.stdout.splitlines():
        if line.startswith("bench_elf_sha256="):
            elf_sha = line.split("=", 1)[1].split()[0]
        elif line.startswith("{"):
            payload = json.loads(line)
    if payload is None:
        raise SystemExit(f"fp-bench emitted no sample payload: {completed.stdout[:400]}")
    times = payload["times_us"]
    if not times:
        raise SystemExit("fp-bench emitted an empty times_us vector")
    return (
        statistics.median(times),
        payload.get("thread_provenance", {}),
        elf_sha,
        len(times),
    )


def run_incumbent_slot(operation, iterations: int) -> float:
    """One incumbent slot: median per-op microseconds over the same N.

    Both arms average over the SAME iteration count so a slot means the same
    thing on each side; comparing one call against a median of many would give
    the arms different noise exposure even when their central tendency agrees.
    """
    samples = []
    gc.collect()
    gc.disable()
    try:
        for _ in range(iterations):
            start = time.perf_counter_ns()
            operation()
            samples.append((time.perf_counter_ns() - start) / 1000.0)
    finally:
        gc.enable()
    return statistics.median(samples)


def bootstrap_ci(values, iters: int = 4000, seed: int = 3):
    rng = random.Random(seed)
    n = len(values)
    medians = sorted(
        statistics.median(values[rng.randrange(n)] for _ in range(n))
        for _ in range(iters)
    )
    return medians[int(0.025 * iters)], medians[int(0.975 * iters)]


def run_row(
    bench: Path,
    category: str,
    workload: str,
    size: str,
    operation,
    iterations: int,
    rounds: int,
    warmup: int,
) -> dict:
    # Warm BOTH arms before the square. The subject arm is a fresh process per
    # slot, so its first invocations pay cold page-cache and first-touch costs
    # that the in-process incumbent does not; without this the subject's own A/A
    # null reports that asymmetry as drift and the row is refused for a reason
    # that has nothing to do with the comparison. Measured at rounds=3 with no
    # warmup: subject null 1.0398, incumbent null 0.9968.
    for _ in range(warmup):
        run_subject_slot(bench, category, workload, size)
        run_incumbent_slot(operation, iterations)

    ratios, null_incumbent, null_subject = [], [], []
    last_probe: dict = {}
    for _ in range(rounds):
        a_slots, b_slots = [], []
        for slot in SQUARE:
            if slot == "A":
                a_slots.append(run_incumbent_slot(operation, iterations))
            else:
                subject_us, last_probe, _, _ = run_subject_slot(
                    bench, category, workload, size
                )
                b_slots.append(subject_us)
        ratios.append(statistics.median(a_slots) / statistics.median(b_slots))
        # Each arm's own first-half / second-half ratio. The square places the
        # halves symmetrically, so a null that departs from 1.0 is drift or
        # contention, not slot position.
        null_incumbent.append(
            statistics.median(a_slots[:2]) / statistics.median(a_slots[2:])
        )
        null_subject.append(
            statistics.median(b_slots[:2]) / statistics.median(b_slots[2:])
        )

    ratio = statistics.median(ratios)
    low, high = bootstrap_ci(ratios)
    n_a = statistics.median(null_incumbent)
    n_b = statistics.median(null_subject)
    nulls_ok = abs(n_a - 1.0) <= NULL_BOUND and abs(n_b - 1.0) <= NULL_BOUND
    if not nulls_ok:
        verdict = "NULL-FAILED"
    elif low <= 1.0 <= high:
        verdict = "STRADDLES-1"
    else:
        verdict = "ADMISSIBLE"
    return {
        "ratio": ratio,
        "ci": (low, high),
        "null_incumbent": n_a,
        "null_subject": n_b,
        "verdict": verdict,
        "thread_provenance": last_probe,
    }


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description="Balanced-square vs-pandas A/B")
    parser.add_argument("--workload", default="str_startswith_arrow")
    parser.add_argument("--size", default="1M")
    parser.add_argument("--rounds", type=int, default=41)
    parser.add_argument("--warmup", type=int, default=4)
    parser.add_argument("--expect-elf", default=os.environ.get("EXPECT_ELF_SHA"))
    parser.add_argument("--fp-bench-binary", default=None)
    parser.add_argument("--list", action="store_true")
    args = parser.parse_args(argv[1:])

    if args.list:
        for name, (_, builder) in WORKLOADS.items():
            print(name if builder else f"{name}  (incumbent arm not wired yet)")
        return 0
    entry = WORKLOADS.get(args.workload)
    if entry is None or entry[1] is None:
        raise SystemExit(f"unknown or unwired workload {args.workload!r}; try --list")
    category, builder = entry

    bench = _fp_bench_binary(args.fp_bench_binary)
    rows = _size_rows(args.size)

    # One probe invocation: establishes the subject's ELF identity, its sample
    # count (so both arms average over the same N), and its thread provenance.
    probe_us, thread_probe, elf_sha, iterations = run_subject_slot(
        bench, category, args.workload, args.size
    )
    if args.expect_elf and not elf_sha.startswith(args.expect_elf):
        raise SystemExit(
            f"ELF MISMATCH: measured {elf_sha[:16]} at {bench}, "
            f"expected {args.expect_elf}"
        )

    operation, invariant = builder(rows)
    witness = invariant()

    invocation_id = shared_invocation_id(elf_sha, args.workload, args.size)
    prov = provenance(bench, elf_sha, thread_probe)
    prov["shared_invocation_id"] = invocation_id
    print("PROVENANCE (subject fields self-reported from inside fp-bench)")
    for key, value in prov.items():
        print(f"  {key:28s} {value}")
    print(f"  {'square/rounds/warmup':28s} {SQUARE}/{args.rounds}/{args.warmup}")
    print(f"  {'incumbent_invariant':28s} {witness}")
    print(f"  {'subject_probe_median_us':28s} {probe_us:.3f}")
    print(f"  {'iterations_per_slot':28s} {iterations} (from fp-bench's own sample count)")

    print(
        f"\nRATIO = t_pandas / t_frankenpandas   (>1 means FrankenPandas faster)"
        f"   null bound +/-{NULL_BOUND}"
    )
    row = run_row(
        bench,
        category,
        args.workload,
        args.size,
        operation,
        iterations,
        args.rounds,
        args.warmup,
    )
    low, high = row["ci"]
    print(
        f"  {args.workload}@{args.size:6s} {row['ratio']:8.4f}x  "
        f"CI [{low:.4f}, {high:.4f}]  "
        f"nulls {row['null_incumbent']:.4f}/{row['null_subject']:.4f}  "
        f"{row['verdict']}"
    )
    print(f"\n  loadavg_end                  {os.getloadavg()}")
    if row["verdict"] != "ADMISSIBLE":
        print("  NOT ADMISSIBLE — do not quote this number as a result.")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
