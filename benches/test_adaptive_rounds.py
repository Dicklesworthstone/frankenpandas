"""Round scaling for FAST workloads — br-frankenpandas-4kig1.

`adaptive_balanced_square_rounds` exists because the A/A null gate is measurably
harder for a short operation than a long one at the same 2% limit. Measured over
226 rows in `artifacts/bench/`: sub-1ms rows pass both arms 33-41% of the time
against 54% above 5ms, because nine ABBAABBA rounds of a 186us call is under 7ms of
actual timed work.

The tests below pin the property that makes this a strengthening of the instrument
rather than a weakening of the gate, which is the whole reason it is shaped this
way: it can only ever ADD rounds.
"""

from __future__ import annotations

import importlib.util
import math
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


def test_a_slow_workload_keeps_the_shipped_round_count():
    """>5ms ops already pass at 54%; they must not be made more expensive."""
    m = _load()
    for slot_us in (5_000.0, 10_000.0, 11_425.0, 1e6):
        assert m.adaptive_balanced_square_rounds(slot_us) == m.BALANCED_SQUARE_ROUNDS


def test_the_floor_case_gets_enough_rounds_to_reach_the_target():
    """The 186us incumbent that would not certify in six attempts.

    Not a round-number assertion: the requirement is that the resulting timed
    region actually reaches the target, which is the thing that shrinks the null.
    """
    m = _load()
    slot_us = 186.0
    rounds = m.adaptive_balanced_square_rounds(slot_us)
    slots = m.BALANCED_SQUARE.count("A")
    assert rounds > m.BALANCED_SQUARE_ROUNDS
    assert rounds * slots * slot_us >= m.ADAPTIVE_TARGET_TIMED_US
    # ...and it stays bounded rather than running for hours.
    assert rounds <= m.ADAPTIVE_MAX_ROUNDS


def test_it_can_only_ever_ADD_rounds_never_remove_them():
    """THE ANTI-GATE-WEAKENING INVARIANT.

    A version that returned fewer rounds for some input would be measuring some
    workload LESS carefully than the shipped default — silently loosening the
    instrument while appearing to tune it. Swept across seven orders of magnitude
    including the degenerate inputs.
    """
    m = _load()
    probes = [10.0 ** e for e in range(-2, 8)]
    probes += [0.0, -1.0, -1e9, float("inf"), float("-inf"), float("nan"), 1e-12]
    for slot_us in probes:
        rounds = m.adaptive_balanced_square_rounds(slot_us)
        assert rounds >= m.BALANCED_SQUARE_ROUNDS, f"fewer rounds at slot_us={slot_us}"
        assert rounds <= m.ADAPTIVE_MAX_ROUNDS, f"unbounded at slot_us={slot_us}"
        assert isinstance(rounds, int)


def test_a_degenerate_measurement_falls_back_instead_of_guessing():
    """0, negative, NaN and inf are 'I do not know', not 'run forever'."""
    m = _load()
    for slot_us in (0.0, -0.0, -5.0, float("nan"), float("inf")):
        assert m.adaptive_balanced_square_rounds(slot_us) == m.BALANCED_SQUARE_ROUNDS


def test_faster_workloads_never_get_fewer_rounds_than_slower_ones():
    """Monotone non-increasing in slot duration — no inversions in the middle."""
    m = _load()
    slots = sorted([1.0, 10.0, 50.0, 186.0, 500.0, 1_000.0, 3_000.0, 9_000.0, 50_000.0])
    rounds = [m.adaptive_balanced_square_rounds(s) for s in slots]
    assert rounds == sorted(rounds, reverse=True), list(zip(slots, rounds))


def test_the_target_is_reached_whenever_the_cap_is_not_binding():
    """If it returned less than max_rounds, it owes us the full timed region."""
    m = _load()
    slots = m.BALANCED_SQUARE.count("A")
    for slot_us in (12.0, 50.0, 186.0, 400.0, 900.0, 1_111.0):
        rounds = m.adaptive_balanced_square_rounds(slot_us)
        if rounds < m.ADAPTIVE_MAX_ROUNDS:
            assert rounds * slots * slot_us >= m.ADAPTIVE_TARGET_TIMED_US
