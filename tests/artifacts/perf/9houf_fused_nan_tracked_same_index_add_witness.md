# br-frankenpandas-9houf — single-pass NaN-tracked same-index Float64 arithmetic

## Root cause
FP's same-index Float64 fast path (`Column::aligned_binary_f64_same_positions`)
paid FOUR memory passes for a fully-valid add — two input NaN pre-scans
(`!lsrc.any(is_nan)`, `!rsrc.any(is_nan)`), the add (`apply_f64_slices`), and the
`from_f64_values` output re-scan — where numpy/pandas pay one. The two input
pre-scans are NOT droppable (input-NaN must materialize `Scalar::Null(NaN)`,
not `Float64(NaN)`, and `NaN**0 == 1.0` for Pow diverges; see the d3mfh
rejection).

## Lever (one, structural — fusion, not deferral)
New monomorphized kernel `apply_f64_slices_nan_tracked(op, a, b) -> (data,
input_nan, output_nan)` computes `data` and the two NaN-presence flags in a
SINGLE sweep (plain `bool` OR-accumulators alongside a contiguous store;
Add/Sub/Mul/Div autovectorize). The fast path then branches on the flags,
reproducing the prior outcomes byte-for-byte:

- `validity.all() && validity.all() && !input_nan` is EXACTLY the old gate
  (`input_nan == lsrc.any(nan) || rsrc.any(nan)`), then:
  - `!output_nan`  → `from_f64_values_all_valid_unchecked(data)` (stamp all-valid
    mask, no re-scan; `data` proven NaN-free).
  - `output_nan`   → `from_f64_values(data)` (generated NaN e.g. inf-inf marked
    invalid — identical to the old fast path).
- any input NaN with all-valid bits → falls through to the unchanged general
  Scalar path (so `Scalar::Null(NaN)` and `NaN**0 == 1.0` stay correct).

The now-unused `apply_f64_slices` is removed; its per-element parity test is
repointed to `apply_f64_slices_nan_tracked(...).0`.

## Isomorphism proof
- Branch selection identical: `input_nan` reproduces the old two-scan disjunction
  exactly; `output_nan` only splits the old `from_f64_values(data)` into a
  no-rescan stamp vs the identical re-scan, both producing the same column.
- `data` element-for-element identical to `binary_f64_apply(op)` in order (same
  per-op closures, same fold order).
- Null/NaN/Float64(NaN) materialization contract, Pow `NaN**0`, validity,
  ordering, f64 bits, fallback: all preserved.
- No unsafe (`forbid(unsafe_code)` intact); `from_f64_values_all_valid_unchecked`
  only reached when `data` is proven NaN-free (`!output_nan`).

## Golden (byte-identical before==after)
`bench_series_add_same` digests f64 bits + per-row validity for all 7 ops,
n=100000 (FNV-1a64), identical before and after:
add `ce1f207031ac262e`, sub `a4d7b3136b8d1285`, mul `06348b3433a34fe8`,
div `72b9e6b279eecef3`, mod `a4d7af5e77c2a75f`, pow `830a3101d8cc86a4`,
floordiv `23f86c13ec6d2f98`.

## Benchmark (n=100000, op-only loop)
- min-of-5 internal per-iter (add): before ~0.159 ms → after ~0.050 ms = **3.2×**.
- mul 0.159→0.051 ms (3.1×), div 0.217→0.078 ms (2.8×).
- hyperfine -N paired (800 iters, process incl. one-time frame build):
  forward **2.50× ± 0.17**, reversed **2.42× ± 0.14** faster. Ordering-independent.

Score ≫ 2.0 (Impact 5 × Confidence 5 / Effort 2).

## Gates
- fp-columnar full test suite (incl. the locked `same_positions == general`
  NaN/inf parity test and the repointed kernel parity test): green.
- fp-conformance arithmetic suite: green.
- clippy -D warnings: clean; fmt: clean.
