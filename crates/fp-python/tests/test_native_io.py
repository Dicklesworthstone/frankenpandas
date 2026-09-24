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


@pytest.mark.xfail(
    strict=True,
    reason="br-frankenpandas-rc0923-epic-rust-parity-bugs-4qg5w.19: fp-io guesses the "
    "unnamed first column is an index and drops it; pandas keeps it as 'Unnamed: 0'",
)
def test_excel_written_index_reads_back_as_unnamed_column(tmp_path):
    # pandas 2.2.3: DataFrame(DATA).to_excel(p); list(read_excel(p).columns)
    # -> ["Unnamed: 0", "i", "f", "s", "b"]
    path = tmp_path / "with_index.xlsx"
    _frame().to_excel(str(path))
    assert list(fpd.read_excel(str(path)).columns) == ["Unnamed: 0", "i", "f", "s", "b"]


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
