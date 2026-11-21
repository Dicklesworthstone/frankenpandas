"""Live pandas contracts for multiplied Period frequencies (tus8r).

Ordinals count base periods; the multiplier controls the span and each shift.
These are deliberately separate assertions, since snapping an ordinal to an
n-period grid or extending the displayed label can look plausible but is wrong.
"""

from __future__ import annotations

import warnings

import numpy as np
import pandas as pd
import pytest

import frankenpandas as fp


FREQUENCIES = [
    "D", "2D", "3h", "15min", "2W-FRI", "2M", "3Q-JAN", "2Y-JUN",
    "5ms", "4us", "7ns", "2B",
]


def assert_period_equal(actual, expected):
    assert actual.ordinal == expected.ordinal
    assert actual.freqstr == expected.freqstr
    assert str(actual) == str(expected)
    assert repr(actual) == repr(expected)
    assert actual.start_time.value == expected.start_time.value
    assert actual.end_time.value == expected.end_time.value
    for field in (
        "year", "month", "day", "hour", "minute", "second", "quarter", "qyear",
        "dayofweek", "dayofyear", "week", "days_in_month", "is_leap_year",
    ):
        assert getattr(actual, field) == getattr(expected, field), field


def assert_period_index_equal(actual, expected):
    assert type(actual).__name__ == type(expected).__name__
    assert str(actual.dtype) == str(expected.dtype)
    assert actual.freqstr == expected.freqstr
    assert actual.name == expected.name
    np.testing.assert_array_equal(actual.asi8, expected.asi8)
    assert [str(value) for value in actual] == [str(value) for value in expected]


@pytest.mark.parametrize("freq", FREQUENCIES)
@pytest.mark.parametrize("text", ["1969-12-31 23:59:59.123456789", "2024-02-28 12:34:56.123456789"])
def test_period_spans_and_fields_use_different_units(freq, text):
    with warnings.catch_warnings():
        warnings.simplefilter("ignore", FutureWarning)
        expected = pd.Period(text, freq=freq)
        actual = fp.Period(text, freq=freq)
        assert_period_equal(actual, expected)
        assert actual.strftime("%Y-%m-%d %H:%M:%S %q %F %f") == expected.strftime(
            "%Y-%m-%d %H:%M:%S %q %F %f"
        )
        for step in (-3, -1, 0, 1, 2):
            assert_period_equal(actual + step, expected + step)
            assert_period_equal(actual - step, expected - step)
            assert ((actual + step) - actual).n == ((expected + step) - expected).n


@pytest.mark.parametrize("freq", FREQUENCIES)
@pytest.mark.parametrize("how", ["start", "end"])
@pytest.mark.parametrize("target", ["D", "2D", "3h", "2M", "2W-FRI"])
def test_period_frequency_conversion_uses_the_source_span(freq, how, target):
    with warnings.catch_warnings():
        warnings.simplefilter("ignore", FutureWarning)
        text = "2024-01-05 12:34:56.123456789"
        assert_period_equal(
            fp.Period(text, freq=freq).asfreq(target, how=how),
            pd.Period(text, freq=freq).asfreq(target, how=how),
        )


@pytest.mark.parametrize("freq", FREQUENCIES)
def test_asfreq_to_the_same_multiple_is_not_an_identity(freq):
    with warnings.catch_warnings():
        warnings.simplefilter("ignore", FutureWarning)
        text = "2024-01-05 12:34:56.123456789"
        assert_period_equal(
            fp.Period(text, freq=freq).asfreq(freq),
            pd.Period(text, freq=freq).asfreq(freq),
        )


@pytest.mark.parametrize("freq", FREQUENCIES)
@pytest.mark.parametrize("endpoint", ["start", "end"])
@pytest.mark.parametrize("count", [0, 1, 4])
def test_period_ranges_step_by_the_multiplier(freq, endpoint, count):
    kwargs = {endpoint: "2024-01-05 12:34:56.123456789", "periods": count, "freq": freq, "name": "span"}
    with warnings.catch_warnings():
        warnings.simplefilter("ignore", FutureWarning)
        expected = pd.period_range(**kwargs)
        actual = fp.period_range(**kwargs)
        assert_period_index_equal(actual, expected)
        assert_period_index_equal(actual.shift(2), expected.shift(2))
        np.testing.assert_array_equal(actual.start_time.asi8, expected.start_time.asi8)
        np.testing.assert_array_equal(actual.end_time.asi8, expected.end_time.asi8)


@pytest.mark.parametrize("freq", ["2D", "3h", "15min", "2W-FRI", "2M", "3Q-JAN", "2Y-JUN"])
def test_period_ranges_count_base_ordinals_between_endpoints(freq):
    for module in (pd, fp):
        first = module.Period("2024-01-01", freq=freq)
        last = first + 3
        result = module.period_range(start=str(first), end=str(last), freq=freq)
        assert len(result) == 4
    assert_period_index_equal(
        fp.period_range("2024-01-01", "2024-08-01", freq=freq),
        pd.period_range("2024-01-01", "2024-08-01", freq=freq),
    )


@pytest.mark.parametrize("freq", FREQUENCIES)
def test_period_index_keeps_multiplier_through_missing_values_and_conversion(freq):
    texts = ["2024-01-05", None, "2024-02-29", "2024-01-05"]
    with warnings.catch_warnings():
        warnings.simplefilter("ignore", FutureWarning)
        actual = fp.PeriodIndex(texts, freq=freq, name="span")
        expected = pd.PeriodIndex(texts, freq=freq, name="span")
        assert_period_index_equal(actual, expected)
        for target in (freq, "2D", "3M"):
            for how in ("start", "end"):
                assert_period_index_equal(actual.asfreq(target, how=how), expected.asfreq(target, how=how))
        assert_period_index_equal(actual.shift(-2), expected.shift(-2))


@pytest.mark.parametrize("freq", FREQUENCIES)
def test_datetime_index_and_series_convert_without_grid_snapping(freq):
    texts = ["2024-01-01 00:01:02", "2024-01-02 01:02:03", None, "2024-01-05 12:34:56"]
    with warnings.catch_warnings():
        warnings.simplefilter("ignore", FutureWarning)
        expected_index = pd.to_datetime(texts).to_period(freq)
        actual_index = fp.to_datetime(texts).to_period(freq)
        assert_period_index_equal(actual_index, expected_index)
        expected = pd.Series(pd.to_datetime(texts), name="times").dt.to_period(freq)
        actual = fp.Series(fp.to_datetime(texts), name="times").dt.to_period(freq)
        assert str(actual.dtype) == str(expected.dtype)
        assert actual.name == expected.name
        assert [str(value) for value in actual] == [str(value) for value in expected]


@pytest.mark.parametrize("freq", ["2D", "3h", "15min", "2W-FRI"])
def test_inferred_datetime_frequency_reaches_period_multipliers(freq):
    actual = fp.Series(fp.date_range("2024-01-01", periods=4, freq=freq)).dt.to_period()
    expected = pd.Series(pd.date_range("2024-01-01", periods=4, freq=freq)).dt.to_period()
    assert str(actual.dtype) == str(expected.dtype)
    assert [str(value) for value in actual] == [str(value) for value in expected]


@pytest.mark.parametrize("texts", [
    ["2024-01-01", "2024-01-03", "2024-01-05"],
    ["2024-01-01 00:00:00", "2024-01-01 03:00:00", "2024-01-01 06:00:00"],
    ["2024-01-05", "2024-01-19", "2024-02-02"],
])
def test_datetime_index_infers_a_multiple_without_stored_frequency(texts):
    actual = fp.DatetimeIndex(texts, name="dates")
    expected = pd.DatetimeIndex(texts, name="dates")
    assert actual.freq is None
    assert expected.freq is None
    assert_period_index_equal(actual.to_period(), expected.to_period())


@pytest.mark.parametrize("freq", ["2ME", "2QE-JAN", "2YE-JUN"])
def test_stored_calendar_end_offset_keeps_its_multiplier(freq):
    actual = fp.date_range("2024-01-01", periods=4, freq=freq, name="dates")
    expected = pd.date_range("2024-01-01", periods=4, freq=freq, name="dates")
    assert_period_index_equal(actual.to_period(), expected.to_period())


@pytest.mark.parametrize("freq", ["2ME", "2QE-JAN", "2YE-JUN"])
def test_inferred_calendar_end_offset_keeps_pandas_refusal(freq):
    # pandas maps a stored offset to its Period alias, but its inference
    # path rejects the multiplied offset alias instead of normalizing it.
    for module in (pd, fp):
        dates = module.date_range("2024-01-01", periods=4, freq=freq)
        with pytest.raises(ValueError):
            module.DatetimeIndex(list(dates)).to_period()
        with pytest.raises(ValueError):
            module.Series(dates).dt.to_period()


@pytest.mark.parametrize("freq", ["2D", "15min", "2M"])
@pytest.mark.parametrize("route", ["empty_constructor", "empty_datetimes", "empty_range", "missing_only"])
def test_empty_and_all_missing_period_indexes_keep_the_declared_multiple(freq, route):
    def make(module):
        if route == "empty_constructor":
            return module.PeriodIndex([], freq=freq, name="span")
        if route == "empty_datetimes":
            return module.DatetimeIndex([], name="span").to_period(freq)
        if route == "empty_range":
            return module.date_range("2024-01-01", periods=0, freq="2D", name="span").to_period(freq)
        return module.PeriodIndex([None, module.NaT, np.nan, "NaT"], freq=freq, name="span")

    actual, expected = make(fp), make(pd)
    assert_period_index_equal(actual, expected)
    assert_period_index_equal(actual.shift(2), expected.shift(2))
    assert_period_index_equal(actual.asfreq("3D"), expected.asfreq("3D"))
    np.testing.assert_array_equal(actual.start_time.asi8, expected.start_time.asi8)
    np.testing.assert_array_equal(actual.end_time.asi8, expected.end_time.asi8)


@pytest.mark.parametrize("missing_only", [False, True])
def test_period_index_cannot_infer_a_frequency_without_present_values(missing_only):
    for module in (pd, fp):
        values = [None, module.NaT] if missing_only else []
        with pytest.raises(ValueError):
            module.PeriodIndex(values)


@pytest.mark.parametrize("freq", [None, "2D"])
def test_period_index_requires_a_collection_even_with_a_frequency(freq):
    for module in (pd, fp):
        with pytest.raises(TypeError):
            module.PeriodIndex(None, freq=freq)


def test_period_index_infers_the_multiple_after_a_missing_first_value():
    def make(module):
        return module.PeriodIndex(
            [None, module.Period("2024-01-01", freq="2D"), module.NaT], name="span",
        )

    assert_period_index_equal(make(fp), make(pd))


@pytest.mark.parametrize("freq", ["2D", "3h", "15min", "2M", "2Y-JUN", "3Q-JAN"])
def test_period_keyword_fields_keep_the_multiplier(freq):
    kwargs = dict(year=2024, month=2, day=3, hour=4, minute=5, second=6, freq=freq)
    if "Q" in freq:
        kwargs["quarter"] = 1
    assert_period_equal(fp.Period(**kwargs), pd.Period(**kwargs))


@pytest.mark.parametrize("freq", ["0D", "-2D", "2MS", "2unknown", "9223372036854775808D"])
def test_invalid_period_multipliers_are_rejected(freq):
    for module in (pd, fp):
        with pytest.raises((ValueError, OverflowError)):
            module.Period("2024-01-01", freq=freq)
        with pytest.raises((ValueError, OverflowError)):
            module.PeriodIndex(["2024-01-01"], freq=freq)
