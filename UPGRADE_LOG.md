# Dependency Upgrade Log

**Date:** 2026-09-11
**Project:** frankenpandas
**Language:** Rust
**Manifest:** Cargo.toml

## Release-wave audit: 2026-10-03 (in progress)

The historical September audit below is retained. Its test checkmarks do not
qualify the current checkout or the following updates.

Final 0.4.0 qualification remains incomplete. The current source passed the
focused 15 I/O tests and 12 merge/object-key tests, and its formatting gate.
The final pinned-toolchain workspace/all-targets Clippy gate passed with
warnings denied. The final default Linux Rust workspace suite then passed
9,060 tests with zero failures and 154 ignored across 121 target summaries,
including all 37 Python-binding Rust tests with ordinary libpython linkage.
The actual current 0.4.0 Linux raw extension also passed 62 unchanged stock
pytest cases, all 54 new recursion/merge cases, the existing 15 interoperability
checks and the stock smoke; both pytest groups had no skips or failures. This
does not qualify wheel/PyPI packaging, all platforms or the full Python API. Genuine
published v0.3.0 Arrow writers produced nine retained files using the authentic
published package and its original lockfile. Current-reader compatibility
qualification is still in progress. The separate Git-source producer never
executed and remains unrecoverable through the supported CLI
([RCH #88](https://github.com/Dicklesworthstone/remote_compilation_helper/issues/88)).
Earlier full-workspace results below predate those fixes and do not qualify the
final candidate; no 0.4.0 publication has occurred.

The current all-target workspace check also passed. The complete stock G6 CLI
returned 1: its conformance step passed, but supplemental sidecar verification
found 460 missing `parity_report.json` inputs. Exact v0.3.0 and current tracked
corpora both contain zero such files; generation and retention are tracked in
[#41](https://github.com/Dicklesworthstone/frankenpandas/issues/41). Generated
source-root reports could not be retrieved afterward, so exact positive live
counts are not claimed from that CLI run. The independent static review found
no ship-blocking issue in the 168 changed-line findings; the full-file UBS scan
remains nonzero, with all findings retained. Neither result implies global
Python API parity or a green whole G6 pipeline.

The final stock `cargo bench --workspace` also passed through ordinary RCH
against the unchanged final compiler inputs. All three Criterion targets ran,
with 76 distinct measurement sets retained. These are current stock benchmark
measurements, not a matched previous-release performance comparison or a new
speedup claim.

- Audited 44 direct dependencies against current stable registry metadata and
  the resolved lockfile. Several manifest minimums are older than the versions
  already resolved; those do not require unrelated lockfile updates.
- **PyO3 0.29.2 -> 0.29.3:** researched the
  [exact stable patch comparison](https://github.com/PyO3/pyo3/compare/v0.29.2...v0.29.3).
  No removed FFI names are used here. Preserve ordinary libpython linkage and
  keep `extension-module` opt-in for wheel packaging. Updated exactly five
  PyO3 lockfile entries. The explicit Linux extension build and hash-verified
  import passed, as did 62 unchanged stock differential tests without skips,
  the stock version/CSV/groupby/merge smoke and 15 bytes/Arrow compatibility
  cases against actual pandas 2.2.3 and pyarrow 25.0.1. The full default Rust
  workspace gate then passed: 9,045 tests, zero failures, 154 ignored, including
  all 37 PyO3 tests with ordinary libpython linkage. The private embedded Python
  environment required its verified library and NumPy package paths; earlier
  loader/import failures are retained separately from application results.
- **mysql 28.0.2 -> 28.0.3:**
  [stable patch comparison](https://github.com/blackbeam/rust-mysql-simple/compare/v28.0.2...v28.0.3)
  reviewed; applied after PyO3 qualification completed. The patch
  requires `lru` >=0.18.2 and `pem` 4; the precise update resolved `lru` 0.18.5,
  `pem` 4.0.0 and the already-resolved `base64` 0.23.1. Reviewed the
  [exact LRU 0.18.2-to-0.18.5 comparison](https://github.com/jeromefroe/lru-rs/compare/0.18.2...0.18.5):
  additive sparse/retain APIs and allocation lint fixes; MSRV 1.85 is unchanged.
  MySQL disables LRU's default hashbrown feature. A separately compiled,
  hash-verified adapter probe passed 38 checks against an isolated official
  MySQL 8.4.11 server on Linux: prepared-cache reuse and eviction, bound Unicode
  and nulls, transactions and rollback, dataframe writes and independent fresh
  reads. The server stopped cleanly and the source and executable hashes stayed
  unchanged. This qualifies that opt-in runtime lane; TLS and other platforms
  remain outside this evidence.
- **hdf5-metno 0.15.0 evaluated, update withdrawn:** exact published source
  tags reviewed; both old 0.12.6 and candidate 0.15.0 producers built, and each
  passed all five unchanged default+HDF5 stock tests on native HDF5 1.14.6.
  The stricter bidirectional snapshot check failed on exact nullable dtypes
  while reading an old file with the candidate. A separate read-only check
  reproduced the identical failure with both genuine readers on that same
  file. The six relevant HDF/Pickle/JSON helper bodies are unchanged from
  v0.3.0; a v0.3.0 binary was not executed. This is the existing typed-snapshot
  limitation tracked in
  [#40](https://github.com/Dicklesworthstone/frankenpandas/issues/40).
  Preserve the original manifest floor 0.12.4, resolved 0.12.6 and sys 0.12.3.
  The 40-check/30-data-check compatibility gate remains failed and incomplete;
  later value/null/index checks and the opposite read/write leg did not run.
  No complete snapshot or PyTables interoperability claim follows from the
  five passing stock tests.
- **fallible-iterator 0.2.0:** preserve the major implemented by
  `postgres-protocol` 0.6.12. Independently updating to 0.3 changes the trait and
  previously broke the opt-in PostgreSQL adapter.

The earlier formatting, Clippy, focused Rust/Python and full-workspace results
predated the additional Arrow correction and MySQL update. They are superseded
by the final unchanged-source gates above. The Python runtime evidence remains
raw-extension Linux qualification, not wheel packaging or every platform.

The initial fresh cargo-audit database (updated 2026-10-02) reported zero vulnerability
list entries and one unsoundness warning,
[RUSTSEC-2026-0253](https://rustsec.org/advisories/RUSTSEC-2026-0253.html),
on optional MySQL's `lru` 0.16.4. That initial result was not a clean security
verdict. After the MySQL patch resolved LRU to 0.18.5, a fresh fetch and audit of
all 454 resolved dependencies passed with `--deny warnings`: zero vulnerabilities
and zero warnings. No advisory was ignored and no stale database or target
filter was used. The isolated MySQL adapter runtime subsequently passed as
described above.

---

## Summary

| Metric | Count |
|--------|-------|
| **Total dependencies evaluated** | 32 |
| **Updated** | 14 |
| **Skipped** | 2 |
| **Failed (rolled back)** | 0 |
| **Requires attention** | 0 |

---

## Successfully Updated

### arrow: 59.0.0 → 59.3.0
- **Breaking changes:** None
- **Notable changes:** Bug fixes and performance optimizations in Arrow array kernels.
- **Tests:** ✓ Passed

### bytes: 1.11.1 → 1.12.1
- **Breaking changes:** None
- **Tests:** ✓ Passed

### fast-float2: 0.2.3 → 0.2.4
- **Breaking changes:** None
- **Tests:** ✓ Passed

### parquet: 59.0.0 → 59.3.0
- **Breaking changes:** None
- **Notable changes:** Parquet reader/writer bug fixes and synchronization with arrow 59.3.0.
- **Tests:** ✓ Passed

### mysql: 28.0 → 28.0.2
- **Breaking changes:** None
- **Tests:** ✓ Passed

### memchr: 2.8.0 → 2.8.3
- **Breaking changes:** None
- **Tests:** ✓ Passed

### mimalloc: 0.1 → 0.1.52
- **Breaking changes:** None
- **Tests:** ✓ Passed

### pyo3: 0.29 → 0.29.2
- **Breaking changes:** None
- **Notable changes:** Python 3.13 stability fixes.
- **Tests:** ✓ Passed

### regex: 1.12.3 → 1.13.1
- **Breaking changes:** None
- **Tests:** ✓ Passed

### rusqlite: 0.40.1 → 0.40.2
- **Breaking changes:** None
- **Tests:** ✓ Passed

### thiserror: 2.0.19 → 2.0.20
- **Breaking changes:** None
- **Tests:** ✓ Passed

### zmij: 1.0.21 → 1.0.23
- **Breaking changes:** None
- **Tests:** ✓ Passed

### tracing: 0.1.41 → 0.1.44
- **Breaking changes:** None
- **Tests:** ✓ Passed

### asupersync: 0.4.9 → 0.4.11
- **Breaking changes:** None
- **Tests:** ✓ Passed

---

## Skipped

### hdf5-metno: 0.12.4
**Reason:** 0.14.1 contains breaking API redesign in 0.x series; preserved at current stable 0.12.x.

### sas7bdat: 0.2.0
**Reason:** 0.9.1 contains breaking changes in 0.x series; preserved at current stable 0.2.x.

---

## Post-Upgrade Checklist

- [x] All tests passing
- [x] No clippy warnings (`cargo clippy --workspace --all-targets -- -D warnings`)
- [x] Code formatting clean (`cargo fmt --check`)
