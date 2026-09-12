# Dependency Upgrade Log

**Date:** 2026-09-11
**Project:** frankenpandas
**Language:** Rust
**Manifest:** Cargo.toml

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
