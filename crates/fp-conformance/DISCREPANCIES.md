# Known Conformance Divergences

> Every intentional divergence from pandas behavior is documented here.
> Format: DISC-NNN, status (ACCEPTED/INVESTIGATING/WILL-FIX), affected tests.

## Active Divergences

### DISC-001: Integer division by zero promotes to Float64 with NaN/inf
- **Reference:** pandas `int64 // int64` with zero divisor returns `float64` with `inf`
- **Our impl:** Same behavior - promotes to Float64, returns `inf` for floor division, `nan` for mod
- **Impact:** Dtype promotion matches, values match
- **Resolution:** ACCEPTED - exact pandas parity achieved
- **Tests affected:** `int64_mod_floordiv_with_zero_promotes_to_float`
- **Review date:** 2026-04-15

### DISC-002: Unicode width tables version
- **Reference:** pandas uses system's ICU or Python's unicodedata (varies by install)
- **Our impl:** Uses `unicode-width` crate (Unicode 15.1 tables)
- **Impact:** Some emoji/CJK width calculations may differ by 1 column
- **Resolution:** ACCEPTED - newer Unicode tables are more correct
- **Tests affected:** None currently - string display width not yet tested
- **Review date:** 2026-04-15

### DISC-003: Error message text differs
- **Reference:** pandas error messages vary by version and locale
- **Our impl:** Custom error messages with consistent format
- **Impact:** Error semantics match, exact text differs
- **Resolution:** ACCEPTED - tests check error category, not message text
- **Tests affected:** All error-expecting tests use `expected_error_contains`
- **Review date:** 2026-04-15

### DISC-004: CSV NA value handling default differs from pandas 1.x
- **Reference:** pandas 2.x treats "None" as NA by default; pandas 1.x did not
- **Our impl:** Follows pandas 2.x behavior with `keep_default_na=true`
- **Impact:** Users migrating from pandas 1.x may see different behavior
- **Resolution:** ACCEPTED - aligning with current pandas 2.x
- **Tests affected:** `csv_none_is_default_na`
- **Review date:** 2026-04-15

### DISC-005: Mixed string/numeric constructors need pandas object-dtype parity
- **Reference:** `pd.Series(["x", 1])` and `pd.concat([pd.Series(["x", 1])], axis=1)` preserve heterogeneous values under pandas `object` dtype
- **Our impl:** `Series::from_values` / `DataFrame::from_series` currently reject mixed string/numeric payloads via `common_dtype`
- **Impact:** Constructor parity is incomplete for heterogeneous object-backed columns
- **Resolution:** WILL-FIX - conformance harness now captures the upstream object contract and the current structured drift
- **Tests affected:** `live_oracle_series_constructor_mixed_utf8_numeric_reports_object_values`, `live_oracle_dataframe_from_series_mixed_utf8_numeric_reports_object_gap`, `series_constructor_utf8_numeric_error_strict`, `dataframe_from_series_utf8_numeric_error_strict`
- **Review date:** 2026-04-15

## Resolved Divergences

_(None yet - newly established tracking)_

## Rules

1. Every divergence gets a sequential ID (DISC-NNN)
2. Must state whether ACCEPTED, INVESTIGATING, or WILL-FIX
3. Must list affected test cases
4. Must include review date
5. Tests for ACCEPTED divergences use XFAIL markers where applicable
