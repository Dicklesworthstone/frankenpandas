# Error-Path Conformance Tracker

Per `br-frankenpandas-niwb` and `br-frankenpandas-rc0923-epic-full-surface-conformance-j0lg5.7`:
this document tracks pandas exception-category and exception-class parity across
the FrankenPandas error surface.

## Categories & Pandas Exception Hierarchy (`pandas 2.2.3`)

The comparator in `crates/fp-conformance/src/lib.rs` (`ExpectedErrorCategory`)
groups pandas exceptions into six stable buckets and pairs each error with its
specific Python exception class name:

| Category (`ExpectedErrorCategory`) | Pandas / Python Exception Classes (`pandas 2.2.3` MRO) | FrankenPandas Error Sources |
| --- | --- | --- |
| `Type` | `TypeError`, `AttributeError` | `FrameError::Column(ColumnError::UnsupportedCast \| InvalidMaskType \| DTypeMismatch \| Type(IncompatibleDtypes \| InvalidCast(non-Utf8) \| InvalidBoolInt \| InvalidBoolFloat \| NonNumericValue))`, `IndexError::TypeError \| TimeZoneMismatch`, tz-localize/tz-convert/unordered-categorical/incomparable-dtype `CompatibilityRejected` |
| `Value` | `ValueError`, `MergeError` (`ValueError`), `DuplicateLabelError` (`ValueError`), `ParserError` (`ValueError`), `EmptyDataError` (`ValueError`), `OutOfBoundsDatetime` (`ValueError`), `OutOfBoundsTimedelta` (`ValueError`), `IntCastingNaNError` (`ValueError`), `NullFrequencyError` (`ValueError`), `DateParseError` (`ValueError`), `OverflowError` | `FrameError::LengthMismatch \| AssertionFailed`, `ColumnError::IntCastingNaN \| Type(ValueNotParseable \| InvalidCast(from: Utf8) \| LossyFloatToInt(!finite))`, `JoinError` (`MergeError` / `ValueError`), `IoError::Csv \| Json \| EmptyInput \| FormatError`, `DuplicateLabelError` / invalid-frequency / empty-reduction `CompatibilityRejected` |
| `Key` | `KeyError`, `UnsortedIndexError` (`KeyError`), `UnknownTimeZoneError` (`KeyError`) | `IndexError::LabelNotFound \| KeyError \| TimeZone(Unknown)`, missing-column / missing-label / missing-group `FrameError::CompatibilityRejected` |
| `Index` | `IndexError`, `IndexingError` (`Exception`), `InvalidIndexError` (`Exception`) | `IndexError::OutOfBounds`, positional OOB / wrong-length boolean mask / unalignable boolean Series (`IndexingError`) / duplicate-index `concat(axis=1)` (`InvalidIndexError`) `FrameError::CompatibilityRejected` |
| `Io` | `OSError`, `IOError`, `FileNotFoundError` | `fp_io::IoError::Io` (`ErrorKind::NotFound` -> `FileNotFoundError`, other OS errors -> `OSError`) |
| `Unsupported` | `NotImplementedError` | `FrameError::CompatibilityRejected` for pandas `NotImplementedError` paths (e.g. `iloc` with integer-indexed boolean `Series`, unsupported `pivot_table` aggfunc) |

## Corpus Error Fixture Categorization (2026-10-19 — `j0lg5.7`)

Out of **1,382** active (non-retired) packet fixtures in `crates/fp-conformance/fixtures/packets/*.json`:

- Active fixtures with non-null `expected_error_contains`: **81 / 81 (100.0%)** deterministically categorized by `(ExpectedErrorCategory, PythonExceptionClass)`.
- `PacketFixture` carries `expected_error_category: Option<ExpectedErrorCategory>` (wired into `fixture_expected`, `resolve_expected_oracle_error`, `compare_expected_error_category`, and `capture_live_oracle_expected`).
- Machine-verified in `crates/fp-conformance/src/tests/live_oracle_harness_availability.rs::live_oracle_error_exception_parity_campaign_j0lg5_7`.

| Category | Active Corpus Error Fixtures | Share |
| --- | ---: | ---: |
| `Value` (`ValueError`, `MergeError`, `ParserError`, `IntCastingNaNError`, `DuplicateLabelError`) | 37 | 45.7% |
| `Key` (`KeyError`) | 20 | 24.7% |
| `Index` (`IndexError`, `InvalidIndexError`, `IndexingError`) | 13 | 16.0% |
| `Type` (`TypeError`) | 10 | 12.3% |
| `Unsupported` (`NotImplementedError`) | 1 | 1.2% |
| **Total Categorized Active Error Fixtures** | **81 / 81** | **100.0%** |

## Live-Oracle Exception Parity Campaign (`j0lg5.7` — 9 Families, 61 Cases)

`live_oracle_error_exception_parity_campaign_j0lg5_7` runs **61** error-inducing operations live against `.venv-oracle/bin/python` (`pandas 2.2.3`) and verifies that FrankenPandas's error classifier produces the exact `(ExpectedErrorCategory, PythonExceptionClass)` raised by pandas:

| Family | Scope | Live-Oracle Cases | Pandas Exception Classes Verified | Status |
| --- | --- | ---: | --- | --- |
| 1. Missing column / label | `df[col]`, `df.drop`, `s.drop`, `s.loc`, `df.loc`, `s.xs`, `df.xs`, `df.set_index`, `df.sort_values`, `df.drop_duplicates(subset=...)`, `df.reindex(copy=False)` (strict), `groupby.get_group`, `df.pivot`, `Index.get_loc` | 14 | `KeyError` | 14 / 14 PASS |
| 2. Bad dtype cast & type mismatches | `Series.astype(int64/float64)`, `Series.astype_width(Int32)` on `NaN`, `to_datetime(errors='raise')`, `to_timedelta(errors='raise')`, `to_numeric(errors='raise')`, `Series.between` incomparable dtypes, `dt.tz_convert` on tz-naive, `dt.tz_localize` on tz-aware, `Series.median/var/std` on `Utf8`, `bool` Series `.sub()`, unordered `Categorical.min()` | 15 | `ValueError`, `TypeError`, `IntCastingNaNError` (`ValueError`) | 15 / 15 PASS |
| 3. Positional & boolean indexing | `Series.iloc` / `DataFrame.iloc` positive & negative OOB, `Series.take` / `DataFrame.take` OOB, `iloc_bool` wrong mask length, unalignable boolean `Series` mask (`IndexingError`), `iloc` with integer-indexed boolean `Series` (`NotImplementedError`) | 10 | `IndexError`, `IndexingError`, `NotImplementedError` | 10 / 10 PASS |
| 4. Merge validate & join errors | `validate='one_to_one'` (dup left / dup right), `validate='one_to_many'` (dup left), `validate='many_to_one'` (dup right), overlapping non-key columns with empty suffixes `(None, None)` | 5 | `MergeError` (`ValueError`), `ValueError` | 5 / 5 PASS |
| 5. Invalid frequency strings | `calendar_date_range`, `date_range`, `timedelta_range`, `Series.to_period`, `Series.resample`, `DataFrame.resample`, `dt.floor`, `dt.ceil`, `dt.round` | 9 | `ValueError` | 9 / 9 PASS |
| 6. Duplicate labels & uniqueness | `Series.set_flags(allows_duplicate_labels=False)`, `DataFrame.set_flags(allows_duplicate_labels=False)`, `Series.reindex` / `DataFrame.reindex` on duplicate source index, `DataFrame.pivot` duplicate entries, `DataFrame.set_index(verify_integrity=True)`, `DataFrame.insert` duplicate column | 7 | `DuplicateLabelError` (`ValueError`), `ValueError` | 7 / 7 PASS |
| 7. Empty-input reductions & concatenations | Empty `Series.idxmin/idxmax/argmin/argmax`, empty `DataFrame.idxmin/idxmax`, `concat` empty/mismatched keys, `Series.agg([])` | 8 | `ValueError` | 8 / 8 PASS |
| 8. `read_csv` / IO malformed input & missing files | `read_csv_str` extra fields, `read_csv_str` unclosed quote, `IoError::Io(NotFound)`, `read_json_str` malformed JSON | 4 | `ParserError` (`ValueError`), `FileNotFoundError` (`OSError`), `ValueError` | 4 / 4 PASS |
| 9. `SettingWithCopy`-free semantics | Derived filtered/sliced `DataFrame` mutation succeeds without `SettingWithCopyError` and leaves parent `DataFrame` unmodified (CoW / value semantics) | 1 | Value-semantics invariant | 1 / 1 PASS |

## Discovered Error-Path Parity Bugs Filed During `j0lg5.7`

Every genuine error-path divergence uncovered while probing the 9 families against live `pandas 2.2.3` was filed as a dedicated P2 bug bead:

| Bug Bead | Subsystem | Summary |
| --- | --- | --- |
| `br-frankenpandas-747ax` | `fp-frame` | `Series::filter` and `DataFrame::filter_rows` silently drop rows whose labels are missing from a boolean `Series` mask instead of raising `IndexingError: Unalignable boolean Series provided as indexer` |
| `br-frankenpandas-qs8b6` | `fp-frame` | `Series::iloc_bool_series` and `DataFrame::iloc_bool_series` succeed instead of raising `NotImplementedError` (integer-indexed boolean `Series`) or `ValueError` (non-integer-indexed boolean `Series`) like `pandas` `iloc[bool_series]` |
| `br-frankenpandas-pv3ll` | `fp-frame` | `Series::agg(&[])` and `DataFrame::agg(&HashMap::new())` return empty output instead of raising `ValueError("No objects to concatenate")` |
| `br-frankenpandas-xw5ax` | `fp-frame` | `concat_series(&[])` and `concat_dataframes(&[])` return empty output instead of raising `ValueError("No objects to concatenate")` |
| `br-frankenpandas-hvt7o` | `fp-columnar` / `fp-frame` | `Column::astype(DType::Int64)` on a `Float64` column with `NaN` preserves missing values in non-nullable `DType::Int64` instead of raising `ColumnError::IntCastingNaN` (unlike `astype_width(NumericWidth::Int32, false)`) |
| `br-frankenpandas-3j10c` | `fp-frame` | `ToDatetimeOptions::default()` (and bare `fp_frame::to_datetime`) defaults to `DatetimeErrors::Coerce` instead of `DatetimeErrors::Raise` (unlike `ToNumericOptions` and `ToTimedeltaOptions`) |
| `br-frankenpandas-cc4m7` | `fp-frame` | `Series::mean()` on a `Utf8` Series returns `Ok(Scalar::Float64(NaN))` after `self.sum()` string concatenation instead of raising `TypeError` (unlike `Series::var()`, `Series::std()`, and `Series::median()`) |

