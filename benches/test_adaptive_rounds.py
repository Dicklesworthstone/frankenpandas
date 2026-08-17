"""Round scaling for noisy workloads — br-frankenpandas-flicz.

`adaptive_balanced_square_rounds` gives a noisy op the same null-median precision as
a quiet one. The A/A null is a median over per-round ratios, so its standard error
goes as `cv / sqrt(rounds)`; equalising that means `rounds` proportional to `cv**2`,
and the `cv` per duration bucket is MEASURED over 463 arm-rows rather than assumed.

Two earlier justifications for this function were wrong and are recorded in
docs/NEGATIVE_EVIDENCE.md: "a slot is short for a fast op" (false — fp-bench does 50
timed calls per slot at every size) and fixed per-slot cost (false — absolute spread
scales 222x across a 170x p50 range). The tests below pin the behaviour, not either
dead story.
"""

from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
HARNESS = REPO / "benches" / "vs_pandas_harness.py"


def _load():
    spec = importlib.util.spec_from_file_location("vs_pandas_harness", HARNESS)
    module = importlib.util.module_from_spec(spec)
    sys.modules["vs_pandas_harness"] = module
    spec.loader.exec_module(module)
    return module


def test_it_can_only_ever_ADD_rounds_never_remove_them():
    """THE ANTI-GATE-WEAKENING INVARIANT, swept over the whole input domain.

    A version returning fewer rounds for some input would measure that workload LESS
    carefully than the shipped default — quietly loosening the instrument while
    looking like tuning. This is the assertion that forbids it.
    """
    m = _load()
    probes = [10.0**e for e in range(-3, 9)]
    probes += [0.0, -0.0, -1.0, -1e9, float("inf"), float("-inf"), float("nan")]
    probes += [299.9, 300.0, 300.1, 999.9, 1000.0, 2999.9, 3000.0, 19999.9, 20000.0]
    for p50 in probes:
        r = m.adaptive_balanced_square_rounds(p50)
        assert isinstance(r, int)
        assert r >= m.BALANCED_SQUARE_ROUNDS, f"fewer rounds at p50={p50}"
        assert r <= m.ADAPTIVE_MAX_ROUNDS, f"unbounded at p50={p50}"


def test_a_degenerate_measurement_falls_back_instead_of_guessing():
    m = _load()
    for p50 in (0.0, -0.0, -5.0, float("nan"), float("inf")):
        assert m.adaptive_balanced_square_rounds(p50) == m.BALANCED_SQUARE_ROUNDS


def test_the_quietest_bucket_is_the_yardstick_and_keeps_the_shipped_count():
    """3-20ms is the measured-quietest bucket (cv 6.07%) and defines the reference."""
    m = _load()
    for p50 in (3_000.0, 10_473.7, 19_999.0):
        assert m.adaptive_balanced_square_rounds(p50) == m.BALANCED_SQUARE_ROUNDS


def test_the_noisiest_bucket_gets_the_cv_squared_multiple():
    """<300us measures cv 11.27% against the 6.07% yardstick: (11.27/6.07)^2 = 3.45."""
    m = _load()
    rounds = m.adaptive_balanced_square_rounds(177.7)  # the floor @1M incumbent
    expected = (0.1127 / 0.0607) ** 2 * m.BALANCED_SQUARE_ROUNDS
    assert rounds == int(-(-expected // 1)), (rounds, expected)
    assert rounds > 3 * m.BALANCED_SQUARE_ROUNDS


def test_rounds_track_measured_dispersion_not_duration():
    """THE NEGATIVE CASE against both dead mechanisms.

    A duration-based rule (either "short slots need more rounds" or "fixed cost
    dominates") is MONOTONE in p50: the faster the op, the more rounds, always. The
    measured dispersion is NOT monotone — >20ms sits at 8.20%, noisier than the
    3-20ms bucket's 6.07% — so a correct implementation must give a 30ms op MORE
    rounds than a 10ms one. A monotone-in-duration implementation gives it fewer or
    equal, and fails here.
    """
    m = _load()
    quiet = m.adaptive_balanced_square_rounds(10_473.7)   # 3-20ms, cv 6.07%
    slowest = m.adaptive_balanced_square_rounds(30_268.4)  # >20ms,  cv 8.20%
    assert slowest > quiet, (
        "a >20ms op is measurably NOISIER than a 3-20ms one and must get more "
        f"rounds; got {slowest} vs {quiet} — this is duration-based, not "
        "dispersion-based"
    )


def test_the_bucket_lookup_covers_every_duration():
    m = _load()
    for p50 in (0.001, 1.0, 299.999, 300.0, 5_000.0, 1e12):
        d = m.adaptive_dispersion_for_p50(p50)
        assert 0.0 < d < 1.0


# ---------------------------------------------------------------------------
# Wiring: rounds_after_first_round is what run_balanced_square_cell calls at the
# end of round 0. It decides from slots ALREADY measured, so no pilot slot is paid.
# ---------------------------------------------------------------------------


def test_disabled_is_a_no_op_so_the_default_path_is_untouched():
    """--adaptive-rounds defaults OFF; every banked row's methodology must stand."""
    m = _load()
    for p50s in ([177.7] * 4, [10_473.0] * 4, [], [0.0], [float("nan")]):
        assert m.rounds_after_first_round(p50s, 9, enabled=False) == 9


def test_a_useless_first_round_leaves_the_bound_alone():
    """Round 0 giving nothing usable means 'I do not know', not 'run forever'."""
    m = _load()
    for p50s in ([], [0.0, -1.0], [float("nan"), float("inf")], [None]):  # type: ignore[list-item]
        assert m.rounds_after_first_round(p50s, 9, enabled=True) == 9


def test_the_bound_can_only_grow():
    """THE LOOP-SAFETY INVARIANT.

    run_balanced_square_cell now runs `while round_index < rounds` and recomputes
    `rounds` mid-loop. If this could ever RETURN LESS than the current bound, a run
    already past that point would exit early and silently produce a row built from
    fewer samples than requested — a shorter measurement wearing the requested
    round count. It must never shrink.
    """
    m = _load()
    for current in (9, 15, 32, 120, 500):
        for p50s in ([177.7] * 4, [598.0] * 4, [10_473.0] * 4, [30_268.0] * 4):
            assert m.rounds_after_first_round(p50s, current, enabled=True) >= current


def test_a_fast_incumbent_grows_the_bound_and_a_quiet_one_does_not():
    m = _load()
    fast = m.rounds_after_first_round([177.7] * 4, m.BALANCED_SQUARE_ROUNDS, enabled=True)
    quiet = m.rounds_after_first_round([10_473.0] * 4, m.BALANCED_SQUARE_ROUNDS, enabled=True)
    assert fast > m.BALANCED_SQUARE_ROUNDS
    assert quiet == m.BALANCED_SQUARE_ROUNDS
    assert fast == m.adaptive_balanced_square_rounds(177.7)


def test_it_uses_the_median_slot_not_the_first_or_the_mean():
    """One slow outlier slot in round 0 must not drag the decision.

    Three sub-300us slots and one 40ms straggler: the median is still sub-300us, so
    the op is still classified noisy. Taking the mean (10.6ms) or slots[0] would
    misclassify it, and a straggler in round 0 is exactly what a loaded host
    produces.
    """
    m = _load()
    with_straggler = m.rounds_after_first_round(
        [180.0, 185.0, 40_000.0, 179.0], m.BALANCED_SQUARE_ROUNDS, enabled=True
    )
    clean = m.rounds_after_first_round([180.0] * 4, m.BALANCED_SQUARE_ROUNDS, enabled=True)
    assert with_straggler == clean


# ---------------------------------------------------------------------------
# The dispersion table is a hardcoded measurement. Measurements go stale.
# ---------------------------------------------------------------------------


def _measured_dispersion_by_bucket():
    """Recompute relative between-slot dispersion from the banked corpus."""
    import glob
    import json
    import statistics

    buckets: dict[float, list[float]] = {}
    m = _load()
    edges = [upper for upper, _ in m.ADAPTIVE_DISPERSION_BY_P50_US]
    for path in glob.glob(str(REPO / "artifacts" / "bench" / "*.json")):
        try:
            doc = json.load(open(path))
        except Exception:
            continue
        if not isinstance(doc, dict):
            continue
        for row in doc.get("results") or []:
            if not isinstance(row, dict):
                continue
            for arm in ("frankenpandas", "pandas"):
                a = row.get(arm)
                if not isinstance(a, dict):
                    continue
                samples = a.get("samples_us")
                if not isinstance(samples, list) or len(samples) < 8:
                    continue
                vals = [v for v in samples if isinstance(v, (int, float)) and v > 0]
                if len(vals) < 8:
                    continue
                med = statistics.median(vals)
                rel = statistics.pstdev(vals) / med
                for upper in edges:
                    if med < upper:
                        buckets.setdefault(upper, []).append(rel)
                        break
    return {k: statistics.median(v) for k, v in buckets.items() if len(v) >= 10}


def test_the_hardcoded_dispersion_table_still_matches_the_corpus():
    """REGRESSION GUARD on a constant that was measured, not chosen.

    `ADAPTIVE_DISPERSION_BY_P50_US` decides how many rounds a workload gets. It was
    derived from 463 arm-rows on one day; the corpus grows, the kernels change, and
    a table that has drifted would silently mis-scale every adaptive run while
    looking authoritative. This recomputes it and fails when it no longer describes
    reality.

    Deletion condition: when the table is computed at runtime instead of pinned.

    ⚠ The `>= 3 buckets` assertion is deliberate. If the artifacts move or the
    schema changes, every bucket goes empty, the comparison loop body never runs,
    and a naive version of this test PASSES while checking nothing — the
    absence-of-work-as-success failure this repo keeps re-finding.
    """
    m = _load()
    measured = _measured_dispersion_by_bucket()
    assert len(measured) >= 3, (
        f"only {len(measured)} buckets had >=10 rows — the corpus scan found almost "
        "nothing, so this test is not checking the table. Fix the scan, do not "
        "relax the bound."
    )
    drifted = []
    for upper, pinned in m.ADAPTIVE_DISPERSION_BY_P50_US:
        if upper not in measured:
            continue
        now = measured[upper]
        if not (0.6 * now <= pinned <= 1.6 * now):
            drifted.append((upper, pinned, round(now, 4)))
    assert not drifted, (
        "ADAPTIVE_DISPERSION_BY_P50_US has drifted from the corpus it was measured "
        f"from (bucket_upper_us, pinned, measured_now): {drifted}. Re-derive it and "
        "say so in the ledger; do not widen this tolerance to make it pass."
    )


def test_the_reference_bucket_is_still_the_quietest_one():
    """The yardstick must remain the quietest measured bucket.

    `ADAPTIVE_REFERENCE_DISPERSION` is what every other bucket is scaled up to. If
    some other bucket became quieter, the reference would be scaling ops DOWN toward
    a noisier baseline — the multiplier would shrink and noisy ops would get fewer
    rounds, which is the one direction this whole mechanism must never move.
    """
    m = _load()
    measured = _measured_dispersion_by_bucket()
    assert len(measured) >= 3
    quietest = min(measured.values())
    assert m.ADAPTIVE_REFERENCE_DISPERSION <= 1.35 * quietest, (
        f"reference {m.ADAPTIVE_REFERENCE_DISPERSION} is no longer near the quietest "
        f"measured bucket ({quietest:.4f}); re-derive it"
    )
