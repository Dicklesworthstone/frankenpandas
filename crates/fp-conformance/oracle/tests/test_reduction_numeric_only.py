"""A reduction fixture must be able to SAY which numeric_only path it means.

br-frankenpandas-reductions-numeric-only-default-zx21n, scope 4. Thirteen
fixtures are stuck because the oracle never passed `numeric_only` to the
DataFrame reduction family, so every one of them exercised pandas 2.x's default
whatever the fixture intended:

  * Three (sum / min / max) pin the numeric_only=TRUE result and AGREE with
    FrankenPandas, concealing the fact that FP's default is pandas 1.x's.
  * Ten are named `..._skips_nonnumeric_...`, pin the same numeric_only=TRUE
    path, and the oracle DIES on the object column instead — they are the ten
    oracle errors in p6srr's untriaged bucket.

Neither group can be corrected while the option is unsayable. Regenerating the
ten would convert them into error fixtures and delete the only coverage the
numeric_only=True path has.

The measurements these tests encode were taken on live pandas 2.2.3 over one
frame with a float `b`, an object `label` and an int `a` — the shape all the
affected fixtures use.
"""
from __future__ import annotations

import pytest


FRAME = {
    "index": [
        {"kind": "int64", "value": 0},
        {"kind": "int64", "value": 1},
        {"kind": "int64", "value": 2},
    ],
    "column_order": ["b", "label", "a"],
    "columns": {
        "b": [
            {"kind": "float64", "value": 1.5},
            {"kind": "float64", "value": 2.0},
            {"kind": "float64", "value": -7.5},
        ],
        "label": [
            {"kind": "utf8", "value": "a"},
            {"kind": "utf8", "value": "z"},
            {"kind": "utf8", "value": "m"},
        ],
        "a": [
            {"kind": "int64", "value": 1},
            {"kind": "int64", "value": -1},
            {"kind": "int64", "value": 5},
        ],
    },
}

# The eight reductions that RAISE on an object column under pandas 2.x's
# default, and therefore cannot be pinned at all without this option.
RAISING_OPS = [
    ("dataframe_mean", "mean_numeric_only"),
    ("dataframe_prod", "prod_numeric_only"),
    ("dataframe_median", "median_numeric_only"),
    ("dataframe_std", "std_numeric_only"),
    ("dataframe_var", "var_numeric_only"),
    ("dataframe_sem", "sem_numeric_only"),
    ("dataframe_skew", "skew_numeric_only"),
    ("dataframe_kurtosis", "kurtosis_numeric_only"),
]

# The reductions that INCLUDE the object column under the default.
INCLUDING_OPS = [
    ("dataframe_sum", "sum_numeric_only"),
    ("dataframe_min", "min_numeric_only"),
    ("dataframe_max", "max_numeric_only"),
    ("dataframe_count", "count_numeric_only"),
]


def _payload(operation, **options):
    return {
        "packet_id": "FP-TEST",
        "case_id": "numeric_only",
        "mode": "strict",
        "operation": operation,
        "frame": FRAME,
        **options,
    }


def _columns(oracle, pd, operation, **options):
    """Run the op and return {column -> (kind, value)}; fails if it raised."""
    response = oracle.dispatch(pd, _payload(operation, **options))
    series = response.get("expected_series")
    assert series is not None, f"expected a series, got {response}"
    return {
        label["value"]: (value["kind"], value["value"])
        for label, value in zip(series["index"], series["values"])
    }


def _refusal(oracle, pd, operation, **options):
    """Run the op expecting an OracleError; return (message, origin)."""
    with pytest.raises(oracle.OracleError) as excinfo:
        oracle.dispatch(pd, _payload(operation, **options))
    return str(excinfo.value), oracle.oracle_error_origin(excinfo.value)


@pytest.mark.parametrize("operation,key", INCLUDING_OPS)
def test_default_includes_the_object_column(oracle, pd, operation, key):
    """No key supplied -> pandas' own default, which keeps `label`."""
    columns = _columns(oracle, pd, operation)
    assert "label" in columns, (
        f"{operation} default is numeric_only=False in pandas 2.x and must "
        f"include the object column; got {columns}"
    )


@pytest.mark.parametrize("operation,key", INCLUDING_OPS)
def test_numeric_only_true_drops_the_object_column(oracle, pd, operation, key):
    columns = _columns(oracle, pd, operation, **{key: True})
    assert "label" not in columns, f"{operation} numeric_only=True must drop it"
    assert {"b", "a"} <= set(columns)


@pytest.mark.parametrize("operation,key", RAISING_OPS)
def test_default_raises_and_the_option_rescues_it(oracle, pd, operation, key):
    """The negative control FuchsiaBass pointed out.

    A wrong choice on sum/min/max still yields a plausible number; on these
    eight it yields an exception. That the SAME payload succeeds once the option
    is supplied is what proves the option is actually reaching pandas rather
    than being read and dropped.
    """
    message, origin = _refusal(oracle, pd, operation)
    assert origin == oracle.ERROR_ORIGIN_PANDAS, message

    columns = _columns(oracle, pd, operation, **{key: True})
    assert "label" not in columns
    assert {"b", "a"} <= set(columns)


def test_numeric_only_true_promotes_the_int_column_to_float64(oracle, pd):
    """The dtype clincher recorded on the bead, now enforced.

    Under numeric_only=True pandas returns the int column as float64; under the
    default it stays int64. The 13 fixtures pin the float64, which is what
    proves they are the numeric_only=True path specifically and not merely "the
    numeric columns". Any implementation must reproduce the promotion too.
    """
    default = _columns(oracle, pd, "dataframe_sum")
    assert default["a"][0] == "int64", default
    assert default["label"] == ("utf8", "azm"), "sum CONCATENATES an object column"

    numeric_only = _columns(oracle, pd, "dataframe_sum", sum_numeric_only=True)
    assert numeric_only["a"][0] == "float64", numeric_only


def test_min_max_compare_an_object_column_lexicographically(oracle, pd):
    assert _columns(oracle, pd, "dataframe_min")["label"] == ("utf8", "a")
    assert _columns(oracle, pd, "dataframe_max")["label"] == ("utf8", "z")


def test_nunique_refuses_the_option_because_pandas_has_none(oracle, pd):
    """nunique is the one member of the family with no numeric_only at all.

    Measured: `df.nunique(numeric_only=True)` raises
    `TypeError: DataFrame.nunique() got an unexpected keyword argument`.
    Accepting a key we would have to ignore is exactly how a fixture ends up
    believing it pinned an option that never applied — the corr_numeric_only
    defect 9s0c4 found. So the adapter refuses it.
    """
    message, origin = _refusal(oracle, pd, "dataframe_nunique", nunique_numeric_only=True)
    assert "no numeric_only option" in message
    assert origin == oracle.ERROR_ORIGIN_ADAPTER

    # Without the key it still counts every column, object included.
    assert "label" in _columns(oracle, pd, "dataframe_nunique")


def test_a_non_bool_option_is_an_adapter_refusal(oracle, pd):
    message, origin = _refusal(oracle, pd, "dataframe_sum", sum_numeric_only="yes")
    assert "must be a bool" in message
    assert origin == oracle.ERROR_ORIGIN_ADAPTER
