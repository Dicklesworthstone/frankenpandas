"""Oracle-drift tripwire for the DISC-018 temporal overflow table.

Per br-frankenpandas-fyr1z.1. DISC-018 records what pandas does when
Timedelta/Timestamp arithmetic leaves the representable i64 nanosecond range,
and br-frankenpandas-fyr1z decided the strict/hardened mode split **per
operation against that table**. The table is therefore load-bearing: fyr1z.3
implements raise arms only for the rows that raise, and fyr1z.4 is a sign-off
on the two rows that wrap. If a pandas upgrade moves a row and nobody notices,
those beads are built on a stale spec.

This file pins the ORACLE half. It deliberately does NOT compare against
FrankenPandas: on most of these rows FrankenPandas diverges on purpose
(uniform NaT), so a parity assertion would fail by design and tell us nothing.
The FrankenPandas half is pinned separately in fp-types by
`overflow_divergence_surface_is_only_where_pandas_refuses_fyr1z` and
`arithmetic_overflow_never_fabricates_a_finite_value_lgyy8`. A row moving here
means DISC-018 and both of those tests need re-adjudication.

Two traps this file exists to stop, both hit while measuring the table:

1. **Input sensitivity.** `.std()` does NOT overflow on ``[max, max]`` -- it
   returns 0 -- so probing it with that input reads as "pandas doesn't check
   here" and would have wrongly retracted a correct DISC-018 row. It needs
   ``[min, max]``. Conversely `.sum()` on ``[0, max]`` returns NaT even though
   the exact answer IS representable, because pandas' f64 detour rounds
   2**63 - 1 up to 2**63 and the cast back to int64 lands on the sentinel.
   Every case below therefore carries its exact input.

2. **Sentinel collision.** ``Timedelta.min.value`` is ``i64::MIN + 1``, so one
   step below it lands exactly on ``i64::MIN``, which pandas also reserves for
   NaT. `min - 1ns` is NaT and `min - 2ns` raises. Pinning only the 1 ns case
   proves nothing -- it passes for a checked and an unchecked implementation
   alike. Both depths are asserted, adjacent, in
   `test_underflow_is_nat_at_one_step_and_raises_at_two`.
"""
from __future__ import annotations

import numpy as np
import pytest


NS = 1  # one nanosecond, as an int64 count

I64_MIN = np.iinfo(np.int64).min
I64_MAX = np.iinfo(np.int64).max


def _td(pd, *nanos):
    return pd.Series(list(nanos), dtype="timedelta64[ns]")


def _dt(pd, *nanos):
    return pd.Series(list(nanos), dtype="datetime64[ns]")


# ---------------------------------------------------------------------------
# The premise the whole table rests on
# ---------------------------------------------------------------------------


def test_representable_range_is_symmetric_and_min_is_one_above_the_sentinel(pd):
    """i64::MIN is spent on NaT, so the value range is symmetric about zero.

    Everything else in this file follows from these four identities. If a
    pandas release changes the sentinel or widens the range, this fails first
    and the rest of the failures below are downstream noise.
    """
    assert pd.NaT.value == I64_MIN
    assert pd.Timedelta.min.value == I64_MIN + 1
    assert pd.Timedelta.max.value == I64_MAX
    assert pd.Timedelta.min.value == -pd.Timedelta.max.value
    assert pd.Timestamp.min.value == I64_MIN + 1
    assert pd.Timestamp.max.value == I64_MAX


# ---------------------------------------------------------------------------
# RAISES -- the only rows fyr1z.3 may implement a strict raise arm for
# ---------------------------------------------------------------------------


def test_scalar_additive_and_multiplicative_overflow_raise(pd):
    with pytest.raises(OverflowError):
        pd.Timedelta.max + pd.Timedelta(NS, "ns")
    with pytest.raises(OverflowError):
        pd.Timedelta.max + pd.Timedelta.max
    with pytest.raises(OverflowError):
        pd.Timedelta.min + pd.Timedelta.min
    with pytest.raises(OverflowError):
        pd.Timedelta.max - pd.Timedelta.min
    with pytest.raises(OverflowError):
        pd.Timedelta.max * 2
    with pytest.raises(OverflowError):
        pd.Timedelta.max * 1.5
    with pytest.raises(OverflowError):
        pd.Timedelta.max / 0.5


def test_scalar_timestamp_overflow_raises_out_of_bounds_datetime(pd):
    """Timestamp uses a different class than Timedelta for the same failure."""
    from pandas.errors import OutOfBoundsDatetime

    with pytest.raises(OutOfBoundsDatetime):
        pd.Timestamp.max + pd.Timedelta(NS, "ns")
    with pytest.raises(OutOfBoundsDatetime):
        pd.Timestamp.min - pd.Timestamp.max
    with pytest.raises(OutOfBoundsDatetime):
        pd.Timestamp.max - pd.Timestamp.min
    with pytest.raises(OutOfBoundsDatetime):
        pd.Timestamp.max + pd.Timedelta.max
    # ...and it subclasses ValueError, which is what makes a blanket
    # `except OverflowError` in a strict arm wrong for the Timestamp family.
    assert issubclass(OutOfBoundsDatetime, ValueError)


def test_constructor_cast_overflow_raises_out_of_bounds_timedelta(pd):
    """A third class again, for the from-unit constructor path.

    NaN is the odd one out here and is NOT an overflow: it is the missing
    value, so it becomes NaT rather than raising. That asymmetry is the whole
    subject of br-frankenpandas-8v92m.
    """
    from pandas.errors import OutOfBoundsTimedelta

    assert pd.Timedelta(float("nan"), "s") is pd.NaT
    assert pd.to_timedelta(float("nan"), "s") is pd.NaT
    assert pd.Timestamp(float("nan")) is pd.NaT

    for bad in (float("inf"), float("-inf"), 1e30):
        with pytest.raises(OutOfBoundsTimedelta):
            pd.Timedelta(bad, "s")
    with pytest.raises(OutOfBoundsTimedelta):
        pd.Timedelta(2**63, "ns")
    assert issubclass(OutOfBoundsTimedelta, ValueError)


def test_vectorized_int64_addition_overflow_raises_across_all_containers(pd):
    """Series, Index and DataFrame all route to the same checked add."""
    td_max, td_min = _td(pd, I64_MAX), _td(pd, I64_MIN + 1)
    dt_max, dt_min = _dt(pd, I64_MAX), _dt(pd, I64_MIN + 1)
    one = pd.Timedelta(NS, "ns")

    for label, call in (
        ("Series[td64] + 1ns", lambda: td_max + one),
        ("Series[td64] + Series[td64]", lambda: td_max + td_max),
        ("Series[td64] - Series[td64]", lambda: td_max - td_min),
        ("Series[dt64] + 1ns", lambda: dt_max + one),
        ("Series[dt64] - Series[dt64]", lambda: dt_min - dt_max),
        ("Series[dt64] + Series[td64]", lambda: dt_max + td_max),
        ("Series[dt64].diff()", lambda: _dt(pd, I64_MIN + 1, I64_MAX).diff()),
        ("TimedeltaIndex + 1ns", lambda: pd.TimedeltaIndex([pd.Timedelta.max]) + one),
        ("DatetimeIndex + 1ns", lambda: pd.DatetimeIndex([pd.Timestamp.max]) + one),
        ("DataFrame[td64] + 1ns", lambda: pd.DataFrame({"a": td_max}) + one),
    ):
        with pytest.raises(OverflowError, match="Overflow in int64 addition"):
            call()
        # `pytest.raises` above already failed the test if nothing raised;
        # the label is here so the report names the row that moved.
        assert label


def test_sum_and_std_reductions_raise_value_error_on_the_inputs_that_overflow(pd):
    """A fourth class -- and the inputs are not interchangeable.

    `.sum()` overflows on [max, max] but `.std()` does not (it returns 0,
    because the dispersion of two equal values is zero). `.std()` needs
    [min, max]. Probing either with the other's input reads as "no check".
    """
    with pytest.raises(ValueError, match="overflow in timedelta operation"):
        _td(pd, I64_MAX, I64_MAX).sum()
    with pytest.raises(ValueError, match="overflow in timedelta operation"):
        _td(pd, I64_MIN + 1, I64_MAX).std()

    # The negative controls for exactly that trap.
    assert _td(pd, I64_MAX, I64_MAX).std() == pd.Timedelta(0)
    assert _td(pd, I64_MIN + 1, I64_MAX).sum() == pd.Timedelta(0)


def test_ptp_expression_raises(pd):
    """`.max() - .min()` is a scalar subtraction, so it takes the scalar class."""
    span = _td(pd, I64_MIN + 1, I64_MAX)
    with pytest.raises(OverflowError):
        span.max() - span.min()


def test_scalar_division_by_zero_raises_zero_division_error(pd):
    """SCALAR only. The vectorized division family returns NaT -- see below.

    fyr1z.3 must not let a strict arm generalize this to the Series path.
    """
    with pytest.raises(ZeroDivisionError):
        pd.Timedelta.max // 0
    with pytest.raises(ZeroDivisionError):
        pd.Timedelta(NS, "ns") / 0


# ---------------------------------------------------------------------------
# WRAPS SILENTLY -- the rows fyr1z.4 must sign off before anything reproduces
# ---------------------------------------------------------------------------


def test_vectorized_integer_multiply_wraps_two_complement(pd):
    """No exception, no NaT: a positive duration becomes a negative one.

    This is fail-open behavior of exactly the class br-frankenpandas-lgyy8
    removed from FrankenPandas, which is why fyr1z.4 exists rather than
    fyr1z.3 quietly reproducing it.
    """
    # i64::MAX * 2 wraps to -2 in two's complement; asserted as the literal
    # rather than by recomputing it with numpy, which would only re-emit the
    # same overflow RuntimeWarning inside the test.
    assert (_td(pd, I64_MAX) * 2).astype("int64").iloc[0] == -2
    assert (_td(pd, I64_MAX) * 3).astype("int64").iloc[0] == I64_MAX - 2
    assert (_td(pd, I64_MAX) * -2).astype("int64").iloc[0] == 2


def test_cumsum_wraps_while_sum_raises_on_the_same_input(pd):
    """The pair is the point: same values, same accumulation, opposite policy."""
    values = _td(pd, I64_MAX, I64_MAX)
    assert values.cumsum().astype("int64").tolist() == [I64_MAX, -2]
    with pytest.raises(ValueError, match="overflow in timedelta operation"):
        values.sum()


# ---------------------------------------------------------------------------
# RETURNS NaT -- FrankenPandas already agrees here; a blanket strict raise
# would turn each of these rows INTO a divergence
# ---------------------------------------------------------------------------


def test_vectorized_division_family_returns_nat(pd):
    td_max = _td(pd, I64_MAX)
    for label, result in (
        ("/ 0", td_max / 0),
        ("// 0", td_max // 0),
        ("/ 0.0", td_max / 0.0),
        ("/ 0.5 (overflowing)", td_max / 0.5),
        ("// 0.5 (overflowing)", td_max // 0.5),
    ):
        assert result.isna().all(), f"expected NaT for Series[td64] {label}"


def test_float_multiplier_returns_nat_while_int_multiplier_wraps(pd):
    """Same operator, same operand magnitude -- the multiplier dtype decides."""
    td_max = _td(pd, I64_MAX)
    assert (td_max * 2.0).isna().all()
    assert not (td_max * 2).isna().any()


def test_averaging_reductions_return_nat(pd):
    values = _td(pd, I64_MAX, I64_MAX)
    assert values.mean() is pd.NaT
    assert values.median() is pd.NaT
    assert values.quantile(0.5) is pd.NaT


def test_sum_returns_nat_when_the_exact_answer_is_representable(pd):
    """pandas' own f64 detour loses a value it could have returned.

    0 + Timedelta.max is exactly Timedelta.max, but the f64 round-trip rounds
    2**63 - 1 up to 2**63, and the cast back to int64 lands on the NaT
    sentinel. Recorded so nobody "fixes" FrankenPandas to reproduce it.
    """
    assert _td(pd, 0, I64_MAX).sum() is pd.NaT


# ---------------------------------------------------------------------------
# AGREEMENT BY SENTINEL COLLISION, and the rows that are not overflow at all
# ---------------------------------------------------------------------------


def test_underflow_is_nat_at_one_step_and_raises_at_two(pd):
    """The single most misleading row in the table.

    pandas and FrankenPandas agree at one step below min -- but only because
    the result lands on the shared NaT sentinel, not because either chose a
    policy. Two steps below, they part company. Asserting the pair adjacently
    is what keeps a future reader from generalizing the first line.
    """
    one, two = pd.Timedelta(NS, "ns"), pd.Timedelta(2, "ns")

    # One step: NaT, scalar and vectorized, Timedelta and Timestamp.
    assert pd.Timedelta.min - one is pd.NaT
    assert pd.Timestamp.min - one is pd.NaT
    assert _td(pd, I64_MIN + 1).sub(one).isna().all()
    assert _dt(pd, I64_MIN + 1).sub(one).isna().all()

    # Two steps: refused.
    with pytest.raises(OverflowError):
        pd.Timedelta.min - two
    from pandas.errors import OutOfBoundsDatetime

    with pytest.raises(OutOfBoundsDatetime):
        pd.Timestamp.min - two
    with pytest.raises(OverflowError, match="Overflow in int64 addition"):
        _td(pd, I64_MIN + 1) - two
    with pytest.raises(OverflowError, match="Overflow in int64 addition"):
        _dt(pd, I64_MIN + 1) - two


def test_negation_and_abs_of_min_are_not_overflow(pd):
    """`-Timedelta.min` is `Timedelta.max`, NOT NaT.

    `Timedelta::neg` in fp-types documented the opposite until fyr1z, and its
    only tests were at +/-5 and 0, so the false claim was reachable without
    failing anything. This is the oracle half of that fix.
    """
    assert -pd.Timedelta.min == pd.Timedelta.max
    assert abs(pd.Timedelta.min) == pd.Timedelta.max
    assert pd.Timedelta.min * -1 == pd.Timedelta.max
    assert (-_td(pd, I64_MIN + 1)).astype("int64").iloc[0] == I64_MAX
    assert _td(pd, I64_MIN + 1).abs().astype("int64").iloc[0] == I64_MAX


def test_nat_operands_still_propagate_rather_than_raising(pd):
    """The control: NaT in, NaT out, with no exception anywhere near it."""
    assert pd.NaT + pd.Timedelta.max is pd.NaT
    assert pd.Timedelta("nat") + pd.Timedelta(NS, "ns") is pd.NaT
    assert _td(pd, I64_MIN).add(pd.Timedelta(NS, "ns")).isna().all()
