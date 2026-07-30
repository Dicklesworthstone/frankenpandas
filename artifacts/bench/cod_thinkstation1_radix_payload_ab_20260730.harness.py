#!/usr/bin/env python3
"""Interleaved baseline-vs-candidate A/B for the 10M sort, fleet-standard provenance.

Both arms are fp-bench ELFs differing ONLY in radix_argsort_u64, run alternately on
the same host with the same inputs, so the ratio is robust to background load in a
way an absolute level is not. Gate: bootstrap median-CI on the ratio, excluded from a
null band 2x the observed A/A null margin. CV is recorded as provenance and never
gates.
"""
import hashlib
import json
import os
import re
import statistics
import subprocess
import sys
import random

ROUNDS = int(os.environ.get("AB_ROUNDS", "6"))
SIZE = os.environ.get("AB_SIZE", "10M")
BOOT = 20000
SEED = 0xC0DFEED


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest(), os.path.getsize(path)


def host_provenance():
    lscpu = subprocess.run(["lscpu"], capture_output=True, text=True).stdout
    def field(name):
        m = re.search(rf"^{re.escape(name)}:\s+(.*)$", lscpu, re.M)
        return m.group(1).strip() if m else None
    with open("/proc/meminfo") as fh:
        mem_kb = int(re.search(r"MemTotal:\s+(\d+)", fh.read()).group(1))
    try:
        gov = open("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor").read().strip()
    except OSError:
        gov = "unknown"
    sockets = int(field("Socket(s)") or 1)
    cores_per_socket = int(field("Core(s) per socket") or 1)
    return {
        "hostname": os.uname().nodename,
        "model_name": field("Model name"),
        "sockets": sockets,
        "physical_cores": sockets * cores_per_socket,
        "logical_threads": os.cpu_count(),
        "threads_per_core": int(field("Thread(s) per core") or 1),
        "ram_bytes": mem_kb * 1024,
        "numa_nodes": int(field("NUMA node(s)") or 1),
        "kernel": os.uname().release,
        "governor": gov,
        "affinity_mask": sorted(os.sched_getaffinity(0)),
        "affinity_cpu_count": len(os.sched_getaffinity(0)),
    }


def run_arm(binary):
    out = subprocess.run(
        [binary, "--category", "dataframe_ops", "--workload", "sort_values_single",
         "--size", SIZE, "--dtype", "float64", "--json"],
        capture_output=True, text=True, check=True,
    ).stdout
    payload = json.loads([l for l in out.splitlines() if l.startswith("{")][0])
    self_sha = re.search(r"bench_elf_sha256=([0-9a-f]{64})", out).group(1)
    return payload, self_sha


def cv(xs):
    return statistics.stdev(xs) / statistics.mean(xs) if len(xs) > 1 else 0.0


def boot_median_ci(xs, rng):
    meds = []
    for _ in range(BOOT):
        meds.append(statistics.median([xs[rng.randrange(len(xs))] for _ in range(len(xs))]))
    meds.sort()
    return meds[int(0.025 * BOOT)], meds[int(0.975 * BOOT)]


def boot_ratio_ci(a, b, rng):
    """CI for median(a)/median(b) — >1 means arm b is FASTER."""
    ratios = []
    for _ in range(BOOT):
        ra = statistics.median([a[rng.randrange(len(a))] for _ in range(len(a))])
        rb = statistics.median([b[rng.randrange(len(b))] for _ in range(len(b))])
        ratios.append(ra / rb)
    ratios.sort()
    return ratios[int(0.025 * BOOT)], ratios[int(0.975 * BOOT)]


def main():
    base_bin, cand_bin = sys.argv[1], sys.argv[2]
    prov = host_provenance()
    base_sha, base_len = sha256(base_bin)
    cand_sha, cand_len = sha256(cand_bin)
    if base_sha == cand_sha:
        sys.exit("ERROR: both arms are the same ELF; nothing to compare")

    base_t, cand_t, null_ratios, threads = [], [], [], set()
    per_round = []
    isa = None
    checksums = set()
    for r in range(ROUNDS):
        round_p50 = {}
        for label, binary, sink in (("baseline", base_bin, base_t),
                                    ("candidate", cand_bin, cand_t)):
            payload, self_sha = run_arm(binary)
            expect = base_sha if label == "baseline" else cand_sha
            if self_sha != expect:
                sys.exit(f"ERROR: {label} self-reported {self_sha}, expected {expect}")
            sink.extend(payload["times_us"])
            null_ratios.extend(payload["null_control"]["ratios"])
            tp = payload["thread_provenance"]
            threads.add(tp["operation_threads_used"])
            isa = tp["runtime_detected_isa_features"]
            checksums.add(payload["checksum"])
            round_p50[label] = statistics.median(payload["times_us"])
            print(f"  round {r+1} {label:9s} p50={statistics.median(payload['times_us'])/1000:8.2f} ms "
                  f"threads={tp['operation_threads_used']} checksum={payload['checksum']}",
                  file=sys.stderr)
        per_round.append(round_p50["baseline"] / round_p50["candidate"])

    rng = random.Random(SEED)
    m_base = statistics.median(base_t)
    m_cand = statistics.median(cand_t)
    ratio = m_base / m_cand
    lo, hi = boot_ratio_ci(base_t, cand_t, rng)
    # Null margin from the A/A control of the SAME binary: how far from 1.0 does an
    # identical-vs-identical comparison drift on this host right now.
    #
    # GATE AUDIT (fleet primitive transfer, frankenlibc): this gate deliberately has
    # NO confidence interval on the null and NO "null CI must include 1.0" straddle
    # clause, so the reported defect — where a TIGHTER null vetoes its own row — cannot
    # arise here. `null_eps` is a worst-case max|x-1| over raw A/A ratios, which is
    # strictly conservative: it widens the reject band and can only ever suppress a
    # result, never manufacture one. The corrected three-clause rule is ALSO evaluated
    # below, including its null-MEDIAN-within-2% clause, which this gate never checked.
    null_eps = max(abs(x - 1.0) for x in null_ratios)
    band_lo, band_hi = 1.0 - 2 * null_eps, 1.0 + 2 * null_eps
    decided = (lo > band_hi) or (hi < band_lo)
    verdict = ("FASTER" if ratio > 1 else "SLOWER") if decided else "NULL_UNDECIDABLE"

    # --- corrected fleet rule, evaluated independently ---
    null_med = statistics.median(null_ratios)
    nlo, nhi = boot_median_ci(null_ratios, rng)
    null_half_width = (nhi - nlo) / 2.0
    c1_effect_ci_excludes_one = not (lo <= 1.0 <= hi)
    c2_effect_exceeds_2x_null_hw = abs(ratio - 1.0) > 2 * null_half_width
    c3_null_median_within_2pct = abs(null_med - 1.0) <= 0.02
    corrected_decidable = (c1_effect_ci_excludes_one and c2_effect_exceeds_2x_null_hw
                           and c3_null_median_within_2pct)

    result = {
        "workload": f"dataframe_ops/sort_values_single {SIZE} float64",
        "host_provenance": prov,
        "observed_operation_threads": sorted(threads),
        "runtime_detected_isa_features": isa,
        "output_checksums": sorted(checksums),
        "checksums_agree": len(checksums) == 1,
        "arms": {
            "baseline": {"path": base_bin, "elf_sha256": base_sha, "bytes": base_len,
                         "n": len(base_t), "p50_ms": m_base / 1000, "cv_provenance_only": cv(base_t)},
            "candidate": {"path": cand_bin, "elf_sha256": cand_sha, "bytes": cand_len,
                          "n": len(cand_t), "p50_ms": m_cand / 1000, "cv_provenance_only": cv(cand_t)},
        },
        "per_round_ratios": per_round,
        "null_telemetry": {
            "n": len(null_ratios),
            "median": null_med,
            "bootstrap_ci95": [nlo, nhi],
            "half_width": null_half_width,
            "max_abs_dev_from_one": null_eps,
        },
        "corrected_fleet_rule": {
            "c1_effect_ci_excludes_1.0": c1_effect_ci_excludes_one,
            "c2_effect_dev_exceeds_2x_null_half_width": c2_effect_exceeds_2x_null_hw,
            "c2_detail": {"effect_dev": abs(ratio - 1.0), "2x_null_half_width": 2 * null_half_width},
            "c3_null_median_within_2pct_of_1.0": c3_null_median_within_2pct,
            "c3_detail": {"null_median_dev": abs(null_med - 1.0)},
            "decidable": corrected_decidable,
            "agrees_with_local_gate": corrected_decidable == decided,
        },
        "gate": {
            "ratio_baseline_over_candidate": ratio,
            "bootstrap_ci95": [lo, hi],
            "bootstrap_resamples": BOOT,
            "null_margin_eps": null_eps,
            "null_band_2x": [band_lo, band_hi],
            "decided": decided,
            "verdict": verdict,
        },
    }
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
