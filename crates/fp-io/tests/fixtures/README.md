# `fp-io` SAS Binary Test Fixtures (`br-frankenpandas-rc0923-epic-full-surface-conformance-j0lg5.8`)

## Provenance & License

- `people_nonascii.sas7bdat` (5,120 bytes):
  - **Source:** `sas7bdat` Rust crate (`0.9.1`, `tests/fixtures/people_nonascii.sas7bdat`), derived from the public synthetic teaching dataset `people.sas7bdat` (5 rows, 6 columns: `ID`, `SBP`, `DBP`, `GENDER`, `AGE`, `WT`, Windows-1252 encoding with row 0 `GENDER` byte `0xE9` `'é'`).
  - **License:** MIT OR Apache-2.0 (matching the `sas7bdat` crate and workspace allowlist in `deny.toml`).
  - **Clean-room compliance:** Not copied from the `pandas-dev/pandas` repository.
  - **Usage:** Used by `crates/fp-io/src/lib.rs` unit and differential oracle tests (`read_sas_real_files_match_pandas_oracle_j0lg5_8` and `read_sas_truncated_files_return_typed_error_j0lg5_8`) as the canonical binary `.sas7bdat` fixture and template for derived test variants (ASCII, missing values, signed/fractional values, all-missing numeric, and truncated-file negative probes).
