"""Native IO behaviour of the frankenpandas binding, runnable WITHOUT pandas.

br-frankenpandas-rc0923-epic-python-honest-dropin-fvsao.1: the readers used to
call pandas (or openpyxl/pyarrow) and return an EMPTY DataFrame on any failure,
and the writers returned None without creating a file. Every reader here must
read natively or raise; every writer must write natively or raise.

This module must not import pandas, pyarrow or openpyxl: it is the test that
runs in a venv where none of them is installed. Expected values are literals
recorded from pandas 2.2.3 reading the file pandas itself wrote from the same
frame (see each test).
"""

from __future__ import annotations

import math
import re
import subprocess
import sys
import textwrap
from pathlib import Path

import pytest

fpd = pytest.importorskip("frankenpandas")

BINDING_SOURCE = Path(__file__).resolve().parents[1] / "src" / "lib.rs"

# pandas 2.2.3: DataFrame(DATA).to_X(p); read_X(p) -> these values (nulls as None).
DATA = {"i": [1, 2, 3], "f": [1.5, None, 3.25], "s": ["a", "b", None], "b": [True, False, True]}
PANDAS_ROUND_TRIP = {"i": [1, 2, 3], "f": [1.5, None, 3.25], "s": ["a", "b", None], "b": [True, False, True]}
# Stata has no bool or null string: pandas writes the index, bool as int8, None as "".
PANDAS_STATA_ROUND_TRIP = {
    "index": [0, 1, 2],
    "i": [1, 2, 3],
    "f": [1.5, None, 3.25],
    "s": ["a", "b", ""],
    "b": [1, 0, 1],
}


def _cell(value):
    if value is None or (isinstance(value, float) and math.isnan(value)):
        return None
    return value


def _as_lists(frame) -> dict:
    return {str(col): [_cell(v) for v in frame[col].tolist()] for col in frame.columns}


def _frame():
    return fpd.DataFrame(DATA)


# ── the binding never reaches for the incumbent ──────────────────────────────

FORBIDDEN_IMPORT = re.compile(r'import\("(pandas|pyarrow|openpyxl|matplotlib|pandas_gbq)\b')


def test_binding_source_never_imports_pandas_or_its_io_stack():
    hits = [
        f"{n}: {line.strip()}"
        for n, line in enumerate(BINDING_SOURCE.read_text().splitlines(), start=1)
        if FORBIDDEN_IMPORT.search(line)
    ]
    assert hits == [], "binding imports the incumbent:\n" + "\n".join(hits)


def test_io_round_trip_does_not_import_pandas(tmp_path):
    # A subprocess, because another test module in the same session may import
    # pandas itself; here only frankenpandas runs.
    script = textwrap.dedent(
        f"""
        import sys
        import frankenpandas as fpd
        df = fpd.DataFrame({DATA!r})
        root = {str(tmp_path)!r}
        df.to_excel(root + "/x.xlsx", index=False)
        df.to_parquet(root + "/x.parquet")
        df.to_feather(root + "/x.feather")
        df.to_stata(root + "/x.dta")
        fpd.read_excel(root + "/x.xlsx")
        fpd.read_parquet(root + "/x.parquet")
        fpd.read_feather(root + "/x.feather")
        fpd.read_stata(root + "/x.dta")
        leaked = sorted(m for m in ("pandas", "pyarrow", "openpyxl", "matplotlib") if m in sys.modules)
        print(",".join(leaked))
        """
    )
    out = subprocess.run([sys.executable, "-c", script], capture_output=True, text=True, check=True)
    assert out.stdout.strip() == ""


# ── readers: a missing file is FileNotFoundError, never an empty frame ────────

MISSING_PATH_READERS = [
    "read_excel",
    "read_parquet",
    "read_feather",
    "read_stata",
    "read_sas",
    "read_orc",
    "read_fwf",
    "read_xml",
    "read_html",
    "read_csv",
    "read_json",
    "read_pickle",
]


@pytest.mark.parametrize("reader", MISSING_PATH_READERS)
def test_reader_on_missing_path_raises_file_not_found(reader, tmp_path):
    with pytest.raises(FileNotFoundError):
        getattr(fpd, reader)(str(tmp_path / f"missing.{reader}"))


@pytest.mark.parametrize(
    "reader, payload",
    [
        ("read_excel", b"PK\x03\x04 this is not a workbook"),
        ("read_parquet", b"PAR1 not really parquet PAR1"),
        ("read_feather", b"ARROW1 garbage"),
        ("read_stata", b"<stata_dta> garbage"),
    ],
)
def test_reader_on_corrupt_file_raises_instead_of_returning_empty(reader, payload, tmp_path):
    path = tmp_path / "corrupt.bin"
    path.write_bytes(payload)
    with pytest.raises(ValueError):
        getattr(fpd, reader)(str(path))


@pytest.mark.parametrize(
    "call",
    [
        lambda p: fpd.read_hdf(str(p / "x.h5"), "k"),
        lambda p: fpd.read_spss(str(p / "x.sav")),
        lambda p: fpd.HDFStore(str(p / "x.h5")),
        lambda p: fpd.ExcelWriter(str(p / "x.xlsx")),
        lambda p: _frame().to_hdf(str(p / "x.h5"), key="k"),
        lambda p: _frame().to_orc(str(p / "x.orc")),
        lambda p: _frame().to_gbq("dataset.table"),
        lambda p: _frame().to_clipboard(),
        lambda p: fpd.Series([1, 2]).to_clipboard(),
        lambda p: fpd.Series([1, 2]).to_hdf(str(p / "x.h5"), key="k"),
    ],
    ids=[
        "read_hdf",
        "read_spss",
        "HDFStore",
        "ExcelWriter",
        "to_hdf",
        "to_orc",
        "to_gbq",
        "to_clipboard",
        "Series.to_clipboard",
        "Series.to_hdf",
    ],
)
def test_surfaces_without_a_backend_raise_not_implemented(call, tmp_path):
    with pytest.raises(NotImplementedError):
        call(tmp_path)
    assert list(tmp_path.iterdir()) == []


def test_unsupported_keyword_raises_instead_of_being_ignored(tmp_path):
    with pytest.raises(NotImplementedError, match="bogus"):
        _frame().to_excel(str(tmp_path / "x.xlsx"), bogus=1)
    assert not (tmp_path / "x.xlsx").exists()


# ── writers write real files that read back as pandas reads them ─────────────


@pytest.mark.parametrize(
    "writer, reader, suffix, write_kwargs, expected",
    [
        ("to_excel", "read_excel", "xlsx", {"index": False}, PANDAS_ROUND_TRIP),
        ("to_parquet", "read_parquet", "parquet", {}, PANDAS_ROUND_TRIP),
        ("to_feather", "read_feather", "feather", {}, PANDAS_ROUND_TRIP),
        ("to_stata", "read_stata", "dta", {}, PANDAS_STATA_ROUND_TRIP),
    ],
)
def test_writer_round_trip_matches_pandas(writer, reader, suffix, write_kwargs, expected, tmp_path):
    path = tmp_path / f"out.{suffix}"
    assert getattr(_frame(), writer)(str(path), **write_kwargs) is None
    assert path.stat().st_size > 0
    assert _as_lists(getattr(fpd, reader)(str(path))) == expected


def test_series_to_excel_writes_a_named_column(tmp_path):
    # pandas 2.2.3: Series([1, 2], name="v").to_excel(p, index=False);
    # read_excel(p) -> {"v": [1, 2]}. (With the index, see the xfail below.)
    path = tmp_path / "s.xlsx"
    fpd.Series([1, 2], name="v").to_excel(str(path), index=False)
    assert _as_lists(fpd.read_excel(str(path))) == {"v": [1, 2]}


def test_excel_written_index_reads_back_as_unnamed_column(tmp_path):
    # pandas 2.2.3: DataFrame(DATA).to_excel(p); read_excel(p) ->
    # columns ["Unnamed: 0", "i", "f", "s", "b"], the old index as DATA.
    # fp-io used to guess the unnamed 0..n column was an index and drop it.
    # (br-frankenpandas-rc0923-epic-rust-parity-bugs-4qg5w.19)
    path = tmp_path / "with_index.xlsx"
    _frame().to_excel(str(path))
    back = fpd.read_excel(str(path))
    assert list(back.columns) == ["Unnamed: 0", "i", "f", "s", "b"]
    assert back["Unnamed: 0"].tolist() == [0, 1, 2]


def test_csv_blank_index_header_reads_as_unnamed(tmp_path):
    # pandas 2.2.3: DataFrame({'i': [1, 2], 'f': [1.5, None]}).to_csv() is
    # ',i,f\n0,1,1.5\n1,2,\n' and read_csv of it -> ['Unnamed: 0', 'i', 'f'].
    path = tmp_path / "pandas_default.csv"
    path.write_text(",i,f\n0,1,1.5\n1,2,\n")
    assert list(fpd.read_csv(str(path)).columns) == ["Unnamed: 0", "i", "f"]


def test_json_defaults_match_pandas_both_ways(tmp_path):
    # pandas 2.2.3 defaults: DataFrame.to_json() orient 'columns', Series
    # 'index'; read_json accepts both its default shape and records.
    frame = fpd.DataFrame({"i": [1, 2], "f": [1.5, None]})
    assert frame.to_json() == '{"i":{"0":1,"1":2},"f":{"0":1.5,"1":null}}'
    assert fpd.Series([1.5, 2.0], name="x").to_json() == '{"0":1.5,"1":2.0}'
    expected = {"i": [1, 2], "f": [1.5, None]}
    for name, text in [
        ("columns.json", '{"i":{"0":1,"1":2},"f":{"0":1.5,"1":null}}'),
        ("records.json", '[{"i":1,"f":1.5},{"i":2,"f":null}]'),
    ]:
        path = tmp_path / name
        path.write_text(text)
        assert _as_lists(fpd.read_json(str(path))) == expected, name


def _temporal_frame():
    return fpd.DataFrame(
        {
            "t": fpd.to_datetime(fpd.Series(["2024-01-02 03:04:05", None, "2024-12-31 23:59:59"])),
            "d": fpd.to_timedelta(fpd.Series(["1D", None, "2h"])),
        }
    )


# pandas 2.2.3: the same frame through to_X / read_X; str() of each value.
PANDAS_TEMPORAL_ROUND_TRIP = {
    "t": ["2024-01-02 03:04:05", "NaT", "2024-12-31 23:59:59"],
    "d": ["1 days 00:00:00", "NaT", "0 days 02:00:00"],
}


@pytest.mark.parametrize("writer, reader, suffix", [
    ("to_parquet", "read_parquet", "parquet"),
    ("to_feather", "read_feather", "feather"),
])
def test_datetime_and_timedelta_survive_arrow_formats(writer, reader, suffix, tmp_path):
    # br-frankenpandas-rc0923-epic-rust-parity-bugs-4qg5w.20: fp-io wrote these
    # as plain int64 nanoseconds and read Arrow timestamps back as strings.
    path = tmp_path / f"t.{suffix}"
    getattr(_temporal_frame(), writer)(str(path))
    back = getattr(fpd, reader)(str(path))
    assert {c: [str(v) for v in back[c].tolist()] for c in back.columns} == PANDAS_TEMPORAL_ROUND_TRIP


def test_excel_writes_datetime_cells_and_timedelta_days(tmp_path):
    # pandas 2.2.3 writes datetime64 as date cells and timedelta64 as float
    # days, so read_excel returns datetimes and FLOATS:
    #   t -> ['2024-01-02 03:04:05', 'NaT', '2024-12-31 23:59:59']
    #   d -> [1.0, nan, 0.08333333333333333]
    path = tmp_path / "t.xlsx"
    _temporal_frame().to_excel(str(path), index=False)
    back = fpd.read_excel(str(path))
    assert [str(v) for v in back["t"].tolist()] == PANDAS_TEMPORAL_ROUND_TRIP["t"]
    assert [_cell(v) for v in back["d"].tolist()] == [1.0, None, 0.08333333333333333]


def test_datetime_survives_stata_and_timedelta_is_refused(tmp_path):
    # pandas 2.2.3 writes datetime64 as %tc (ms since 1960) and read_stata
    # converts it back; timedelta64 raises NotImplementedError("Data type
    # timedelta64[ns] not supported.").
    path = tmp_path / "t.dta"
    fpd.DataFrame({"t": _temporal_frame()["t"]}).to_stata(str(path), write_index=False)
    back = fpd.read_stata(str(path))
    assert [str(v) for v in back["t"].tolist()] == PANDAS_TEMPORAL_ROUND_TRIP["t"]
    with pytest.raises(NotImplementedError, match="timedelta64"):
        fpd.DataFrame({"d": _temporal_frame()["d"]}).to_stata(str(tmp_path / "d.dta"))


def test_to_parquet_without_a_path_returns_parquet_bytes():
    payload = _frame().to_parquet()
    assert isinstance(payload, bytes)
    assert payload[:4] == b"PAR1" and payload[-4:] == b"PAR1"


def test_arrow_writers_refuse_to_drop_a_label_index(tmp_path):
    frame = fpd.DataFrame({"k": ["x", "y"], "v": [1, 2]}).set_index("k")
    for writer in ("to_parquet", "to_feather"):
        with pytest.raises(NotImplementedError, match="non-default index"):
            getattr(frame, writer)(str(tmp_path / f"idx.{writer}"))
    # index=False is an explicit request to drop it, as in pandas.
    frame.to_parquet(str(tmp_path / "no_index.parquet"), index=False)
    assert _as_lists(fpd.read_parquet(str(tmp_path / "no_index.parquet"))) == {"v": [1, 2]}


# ── to_latex is LaTeX, formatted as pandas formats it ────────────────────────


@pytest.mark.parametrize(
    "build, kwargs, expected",
    [
        (
            lambda: fpd.DataFrame({"x": [1, 2], "y": ["p", "q"]}),
            {},
            "\\begin{tabular}{lrl}\n\\toprule\n & x & y \\\\\n\\midrule\n"
            "0 & 1 & p \\\\\n1 & 2 & q \\\\\n\\bottomrule\n\\end{tabular}\n",
        ),
        (
            lambda: fpd.DataFrame({"x": [1, 2], "y": ["p", "q"]}),
            {"index": False},
            "\\begin{tabular}{rl}\n\\toprule\nx & y \\\\\n\\midrule\n"
            "1 & p \\\\\n2 & q \\\\\n\\bottomrule\n\\end{tabular}\n",
        ),
        (
            lambda: fpd.Series([1.5, None], name="v"),
            {},
            "\\begin{tabular}{lr}\n\\toprule\n & v \\\\\n\\midrule\n"
            "0 & 1.500000 \\\\\n1 & NaN \\\\\n\\bottomrule\n\\end{tabular}\n",
        ),
    ],
    ids=["frame", "frame-no-index", "series"],
)
def test_to_latex_matches_pandas(build, kwargs, expected, tmp_path):
    # Expected strings are pandas 2.2.3's to_latex output for the same object.
    obj = build()
    assert obj.to_latex(**kwargs) == expected
    path = tmp_path / "t.tex"
    assert obj.to_latex(str(path), **kwargs) is None
    assert path.read_text() == expected
