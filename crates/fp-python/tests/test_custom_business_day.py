"""Live pandas 2.2.3 contracts for native custom business calendars (fntq4)."""

from __future__ import annotations

import copy
import datetime as dt
import pickle
import warnings

import numpy as np
import pandas as pd
import pytest

import frankenpandas as fp


CALENDARS = [
    {},
    {"holidays": ["2024-07-04", "2024-07-05", "2024-07-04", "2024-07-06"]},
    {"weekmask": "Mon Wed Fri", "holidays": ["2024-07-05"]},
    {"weekmask": "SunMonTueWedThu", "holidays": ["2024-07-04"]},
    {"weekmask": [True, False, True, False, True, False, False]},
    {"calendar": np.busdaycalendar(weekmask="Mon Tue Thu Fri", holidays=["2024-07-04"])},
]


def assert_stamp_equal(actual, expected):
    assert actual.value == expected.value
    assert str(actual) == str(expected)
    assert str(actual.tz) == str(expected.tz)


def assert_offset_equal(actual, expected):
    assert isinstance(actual, fp.DateOffset)
    assert isinstance(actual, fp.offsets.CustomBusinessDay)
    assert type(actual).__name__ == type(expected).__name__
    assert actual.n == expected.n
    assert actual.normalize == expected.normalize
    assert actual.freqstr == expected.freqstr
    assert actual.name == expected.name
    assert actual.rule_code == expected.rule_code
    assert repr(actual) == repr(expected)
    assert actual.weekmask == expected.weekmask
    assert [str(day) for day in actual.holidays] == [str(day) for day in expected.holidays]
    assert actual.offset == expected.offset
    np.testing.assert_array_equal(actual.calendar.weekmask, expected.calendar.weekmask)
    np.testing.assert_array_equal(actual.calendar.holidays, expected.calendar.holidays)


def assert_index_equal(actual, expected):
    assert type(actual).__name__ == type(expected).__name__
    assert str(actual.dtype) == str(expected.dtype)
    assert str(actual.tz) == str(expected.tz)
    assert actual.name == expected.name
    assert actual.freqstr == expected.freqstr
    np.testing.assert_array_equal(actual.asi8, expected.asi8)
    assert [str(stamp) for stamp in actual] == [str(stamp) for stamp in expected]
    if expected.freq is not None:
        assert_offset_equal(actual.freq, expected.freq)


@pytest.mark.parametrize("calendar", CALENDARS)
@pytest.mark.parametrize("normalize", [False, True])
@pytest.mark.parametrize("n", [-3, -1, 0, 1, 2])
def test_custom_business_scalar_arithmetic(calendar, normalize, n):
    actual = fp.offsets.CustomBusinessDay(n, normalize=normalize, **calendar)
    expected = pd.offsets.CustomBusinessDay(n, normalize=normalize, **calendar)
    assert_offset_equal(actual, expected)
    for text in (
        "2024-07-03 15:22:33.123456789",
        "2024-07-04 15:22:33.123456789",
        "2024-07-06 15:22:33.123456789",
        "1969-12-31 23:59:59.123456789",
    ):
        for zone in (None, "America/New_York"):
            left = fp.Timestamp(text, tz=zone)
            right = pd.Timestamp(text, tz=zone)
            assert_stamp_equal(left + actual, right + expected)
            assert_stamp_equal(actual + left, expected + right)
            assert_stamp_equal(left - actual, right - expected)


@pytest.mark.parametrize("calendar", CALENDARS)
@pytest.mark.parametrize("normalize", [False, True])
def test_custom_business_rolling_checks_and_alias(calendar, normalize):
    assert fp.offsets.CustomBusinessDay is fp.offsets.CDay
    actual = fp.offsets.CDay(3, normalize=normalize, **calendar)
    expected = pd.offsets.CDay(3, normalize=normalize, **calendar)
    for text in ("2024-07-03", "2024-07-04 17:30", "2024-07-05", "2024-07-06 17:30"):
        for zone in (None, "America/New_York"):
            left, right = fp.Timestamp(text, tz=zone), pd.Timestamp(text, tz=zone)
            assert actual.is_on_offset(left) == expected.is_on_offset(right)
            assert_stamp_equal(actual.rollforward(left), expected.rollforward(right))
            assert_stamp_equal(actual.rollback(left), expected.rollback(right))
    for factor in (-2, 0, 1, 3):
        assert_offset_equal(actual * factor, expected * factor)
        assert_offset_equal(factor * actual, factor * expected)
    assert_offset_equal(-actual, -expected)
    assert_offset_equal(actual.base, expected.base)
    assert_offset_equal(actual.copy(), expected.copy())


@pytest.mark.parametrize("n", [-2, -1, 0, 1, 3])
@pytest.mark.parametrize("normalize", [False, True])
@pytest.mark.parametrize("delta", [dt.timedelta(hours=2), dt.timedelta(days=-1, seconds=1)])
def test_custom_business_time_offset_is_added_once(n, normalize, delta):
    kwargs = {"n": n, "normalize": normalize, "offset": delta, "holidays": ["2024-07-04"]}
    actual, expected = fp.offsets.CDay(**kwargs), pd.offsets.CDay(**kwargs)
    assert_offset_equal(actual, expected)
    for text in ("2024-07-03 21:30", "2024-07-06 21:30"):
        left, right = fp.Timestamp(text), pd.Timestamp(text)
        assert_stamp_equal(left + actual, right + expected)
        assert_stamp_equal(left + 3 * actual, right + 3 * expected)
        assert_stamp_equal(left - actual, right - expected)
        assert_stamp_equal(actual.rollback(left), expected.rollback(right))
        assert_stamp_equal(actual.rollforward(left), expected.rollforward(right))


RANGE_INPUTS = [
    {"start": "2024-07-06 14:00", "periods": 5},
    {"end": "2024-07-06 14:00", "periods": 5},
    {"start": "2024-07-02 14:00", "end": "2024-07-12 15:00"},
    {"start": "2024-07-12 15:00", "end": "2024-07-02 14:00"},
    {"start": "2024-07-06 14:00", "periods": 0},
]


@pytest.mark.parametrize("n", [-2, 0, 1, 3])
@pytest.mark.parametrize("normalize", [False, True])
@pytest.mark.parametrize("arguments", RANGE_INPUTS)
def test_custom_business_ranges_use_the_calendar_in_both_directions(n, normalize, arguments):
    kwargs = {"n": n, "normalize": normalize, "weekmask": "Mon Tue Thu Fri", "holidays": ["2024-07-04"]}
    actual, expected = fp.offsets.CDay(**kwargs), pd.offsets.CDay(**kwargs)
    options = {**arguments, "name": "business_dates"}
    try:
        wanted = pd.date_range(**options, freq=expected)
    except ValueError as error:
        with pytest.raises(type(error)):
            fp.date_range(**options, freq=actual)
    else:
        assert_index_equal(fp.date_range(**options, freq=actual), wanted)


@pytest.mark.parametrize("inclusive", ["both", "left", "right", "neither"])
@pytest.mark.parametrize("zone", [None, "America/New_York", "Europe/Berlin"])
def test_ranges_and_vector_arithmetic_preserve_local_time_through_dst(inclusive, zone):
    kwargs = {"holidays": ["2024-03-11"], "weekmask": "Mon Tue Wed Thu Fri"}
    actual, expected = fp.offsets.CDay(**kwargs), pd.offsets.CDay(**kwargs)
    options = {
        "start": "2024-03-08 12:34:56.123456789", "end": "2024-04-03 12:34:56.123456789",
        "tz": zone, "inclusive": inclusive, "name": "trading_days",
    }
    left = fp.date_range(**options, freq=actual)
    right = pd.date_range(**options, freq=expected)
    assert_index_equal(left, right)
    with warnings.catch_warnings():
        warnings.simplefilter("ignore")
        for count in (-2, 0, 3):
            got, want = left + count * actual, right + count * expected
            np.testing.assert_array_equal(got.asi8, want.asi8)
            assert str(got.tz) == str(want.tz)
            got_series = fp.Series(left) + count * actual
            want_series = pd.Series(right) + count * expected
            assert str(got_series.dtype) == str(want_series.dtype)
            assert [str(value) for value in got_series.tolist()] == [str(value) for value in want_series.tolist()]


@pytest.mark.parametrize("calendar", CALENDARS)
def test_calendar_metadata_survives_offset_and_index_copy_selection_pickle(calendar):
    actual, expected = fp.offsets.CDay(2, **calendar), pd.offsets.CDay(2, **calendar)
    for clone in (copy.copy, copy.deepcopy, lambda obj: pickle.loads(pickle.dumps(obj))):
        assert_offset_equal(clone(actual), clone(expected))
    left = fp.date_range("2024-07-01", periods=8, freq=actual, name="day")
    right = pd.date_range("2024-07-01", periods=8, freq=expected, name="day")
    for selector in (slice(None), slice(1, 6), slice(None, None, 2), slice(None, None, -1)):
        assert_index_equal(left[selector], right[selector])
    assert_index_equal(pickle.loads(pickle.dumps(left)), pickle.loads(pickle.dumps(right)))
    # Validate a fresh sequence: pandas 2.2.3 tries to hash list-valued
    # weekmasks when comparing the frequency of an existing index.
    assert_index_equal(
        fp.DatetimeIndex(list(left), freq=actual, name="day"),
        pd.DatetimeIndex(list(right), freq=expected, name="day"),
    )
    assert_index_equal(fp.Series(range(len(left)), index=left).index, pd.Series(range(len(right)), index=right).index)


@pytest.mark.parametrize("count", [-10000, -501, -13, -1, 0, 1, 13, 501, 10000])
def test_week_skipping_handles_long_and_adjacent_holiday_runs(count):
    holidays = pd.date_range("2023-12-18", "2024-02-16").strftime("%Y-%m-%d").tolist()
    kwargs = {"n": count, "weekmask": "Mon Wed Fri", "holidays": holidays}
    actual, expected = fp.offsets.CDay(**kwargs), pd.offsets.CDay(**kwargs)
    assert_stamp_equal(fp.Timestamp("2024-01-03") + actual, pd.Timestamp("2024-01-03") + expected)


@pytest.mark.parametrize("freq", ["C", "2C", "-3C"])
def test_custom_business_frequency_strings(freq):
    actual = fp.tseries.frequencies.to_offset(freq)
    expected = pd.tseries.frequencies.to_offset(freq)
    assert_offset_equal(actual, expected)
    assert_index_equal(fp.date_range("2024-07-06", periods=5, freq=freq), pd.date_range("2024-07-06", periods=5, freq=freq))


@pytest.mark.parametrize("weekmask", ["", "0000000", "mon tue", "Monday", [1, 2, 0, 1, 1, 0, 0], [True, False]])
def test_invalid_custom_business_masks_raise_like_pandas(weekmask):
    with pytest.raises(ValueError):
        pd.offsets.CDay(weekmask=weekmask)
    with pytest.raises(ValueError):
        fp.offsets.CDay(weekmask=weekmask)


def test_different_holiday_calendars_do_not_compare_as_the_same_c_frequency():
    plain = fp.offsets.CDay()
    holiday = fp.offsets.CDay(holidays=["2024-07-04"])
    assert plain == "C"
    assert holiday != "C"
    assert holiday != plain
    assert fp.tseries.frequencies.to_offset(holiday) is holiday


def test_standard_business_day_behavior_is_unchanged():
    for n in (-3, 0, 1, 4):
        for text in ("2024-07-04", "2024-07-06"):
            assert_stamp_equal(
                fp.Timestamp(text) + fp.offsets.BDay(n),
                pd.Timestamp(text) + pd.offsets.BDay(n),
            )
