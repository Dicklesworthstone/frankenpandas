# br-frankenpandas-uza04.84 — quarter-affine CSV integer digit emitter

## Lever (one)
`append_quarter_scaled_pandas_float` (crates/fp-io/src/lib.rs) previously emitted
the quarter-affine *whole* part via `write!(out, "{whole}")`, i.e. the
`core::fmt::write` + i64 `Display` formatting machinery the uza04.81 after-profile
flagged as the residual CSV-writer hot path (`core::fmt::write` 15.84% children /
3.27% self, i64 Display 10.56% children / 2.61% self under the quarter-scaled
emitter). Replaced with a manual decimal digit emitter that writes the whole
digits (right-to-left) and the fixed fractional suffix (`.0`/`.25`/`.5`/`.75`)
into a single 23-byte stack buffer, then emits one `push_str`. No fmt dispatch,
no extra per-cell `push_str` for the fractional tail.

## Byte-identity (golden SHA, before == after)
Workload = `build_numeric_frame("io_col", rows, cols)` from
`high_ram_perf_baseline.rs` (`value = row * 1.25 + col`, every column
quarter-affine). Reproduced by `cargo run -p fp-io --example bench_csv_quarter`.

- rows=100000 cols=10 (bead workload; io_payload_bytes=8611370 matches the bead
  exactly): SHA256 `9d8c04e680d25cdea733fc5dc0d07143fe80d4fd9e2f60737f51cd05145c9b73`
  before == after.
- rows=100000 cols=20: SHA256
  `0858b5d13288de333cd6d941946cb1efd64c94b5c600ca21f7a761a5512f0a70` before == after.

## Isomorphism proof
- Ordering preserved: row/column iteration untouched; only the per-cell whole-part
  formatting changed.
- Output bytes: identical — manual emitter reproduces decimal `Display` for any
  non-negative `u64` (`whole = scaled / 4 >= 0`; `scaled` non-negative by the
  quarter-plan invariants), same fractional suffix table.
- Float / NaN / negative-zero: untouched (fast path only entered for the all-valid
  quarter-affine plan; NaN/non-quarter values still fall through to
  `write_pandas_float`).
- Null / index / columns / fallback surfaces: unchanged.
- Safety: no unsafe (`#![forbid(unsafe_code)]` intact); buffer indexing bounded
  (≤20 u64 digits + ≤3-byte suffix in 23 bytes, `start` cannot underflow).

## Benchmark (hyperfine -N, warmup 3, paired + reversed)
Write-only internal per-iter (bench prints `per_iter`), rows=100000 cols=10:
before ~24.7 ms → after ~19.5–20.5 ms (≈1.23×).

Whole-process paired hyperfine, rows=100000 cols=10, 30 writes/run:
- forward: after 637.8 ms vs before 845.1 ms → **1.33× ± 0.06 faster**
- reversed: after 632.7 ms vs before 820.5 ms → **1.30× ± 0.06 faster**
- cols=20 forward: **1.47× ± 0.30 faster**

Ordering-independent, reproducible. Score ≫ 2.0 (Impact 4 × Confidence 5 /
Effort 2 = 10).

## Gates
- `cargo test -p fp-io --release csv`: 124 passed, 0 failed.
- `cargo clippy -p fp-io --release --all-targets -- -D warnings`: clean.
- `cargo fmt -p fp-io --check`: clean.
- UBS findings are whole-file aggregates (pre-existing); the change adds no new
  unsafe and no new findings.

Note: the release example is `strip = true`, so a symbolized `perf` after-profile
of the example is unavailable; the write-only timing delta (−4.7 ms/iter ≈ 19% of
write time) matches removing the ~15–16% `core::fmt::write` residual the bead
documented.
