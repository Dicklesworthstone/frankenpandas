"""`groupby(k).resample(f)` must survive normalization, and its group key must mean one thing.

br-frankenpandas-3826s. `normalize_groupby_resample_frame` called
`reset_index(level=group_levels)` on a frame that already carried the group key as a COLUMN, so
every one of the five `dataframe_groupby_resample_*` operations died with

    ValueError: cannot insert grp, already exists

⚠️ THOSE FIVE LIVE TESTS DID NOT MERELY SKIP — THEY COULD NEVER HAVE PASSED. Unlike the other
operations on br-frankenpandas-live-oracle-passes-by-skip-l7r1p, whose coverage no-ops when the
oracle is unavailable, these errored even with pandas present. Attribution was measured, not
assumed: plain pandas computes `df.groupby('grp').resample('ME').count()` fine, and only the
adapter's `reset_index()` raises.

WHY THE ROLLING TWIN NEEDS NO SUCH GUARD, which is the reason to fix one function and not both:

    groupby('grp').resample('ME').count()   columns ['grp', 'val']
    groupby('grp').rolling(2).count()        columns ['val']

resample aggregates the group key column as well and keeps the result; rolling excludes it. Only
resample has a column and an index level both wanting the name `grp`.
"""

from __future__ import annotations

import numpy as np
import pandas as pd
import pytest


@pytest.fixture
def frame():
    index = pd.to_datetime(['2024-01-01', '2024-01-15', '2024-02-10', '2024-03-05',
                            '2024-01-20', '2024-02-02', '2024-02-25'])
    return pd.DataFrame({'grp': ['a', 'a', 'a', 'a', 'b', 'b', 'b'],
                         'val': [10.0, 2.0, np.nan, 8.0, 7.0, 9.0, 4.0]}, index=index)


@pytest.mark.parametrize("agg", ["count", "first", "last", "max", "min"])
def test_every_resample_agg_normalizes_without_colliding_3826s(oracle, frame, agg):
    """All five, because the collision is not agg-specific — sum fails as readily as count."""
    aggregated = getattr(frame.groupby('grp').resample('ME'), agg)()
    out = oracle.normalize_groupby_resample_frame(aggregated, ['grp'], 'ME')
    assert 'grp' in out.columns


@pytest.mark.parametrize("agg", ["count", "first", "last", "max", "min"])
def test_the_group_key_column_is_the_LABEL_for_every_agg_3826s(oracle, frame, agg):
    """The substance of the fix, not just the absence of an exception.

    Before it, the `grp` column held an AGGREGATE OF THE KEY OVER ITSELF — measured:
        count -> [2, 1, 1, 1, 2]           the bucket size
        first/last/max/min -> ['a', ...]   the label, since aggregating a constant returns it
    So `grp` meant something different for count than for the other four. Dropping the
    aggregate and restoring the label from the index makes it mean one thing everywhere, and
    changes nothing at all for four of the five.
    """
    aggregated = getattr(frame.groupby('grp').resample('ME'), agg)()
    out = oracle.normalize_groupby_resample_frame(aggregated, ['grp'], 'ME')
    assert set(out['grp']) <= {'a', 'b'}, (
        f"the grp column must carry group LABELS, not {agg}(grp). A numeric value here means "
        "the aggregated copy of the key survived normalization"
    )


def test_the_rolling_twin_is_deliberately_not_guarded_3826s(oracle, frame):
    """Pins WHY only one function changed, so nobody 'fixes' the other for symmetry.

    If pandas ever starts including the group key in rolling output, this fails and says so —
    which is the moment the rolling normalizer would genuinely need the same guard.
    """
    rolled = frame.groupby('grp').rolling(2).count()
    assert 'grp' not in rolled.columns, (
        "pandas excludes the group key from groupby().rolling() output; if that changed, "
        "normalize_groupby_rolling_frame now needs the same collision guard as the resample one"
    )
    out = oracle.normalize_groupby_rolling_frame(rolled, ['grp'])
    assert 'grp' in out.columns
