# br-frankenpandas-uza04.63 Proof: Mask Affine Certificate

Artifact note: this proof was captured during the local loop originally named
`br-frankenpandas-uza04.46`. Current `origin/main` had already consumed
`br-frankenpandas-uza04.46/.47` for the ordered-UTF8 campaign, so the integrated
Beads closeout uses `br-frankenpandas-uza04.63` while retaining the
`uza04_46` artifact filenames. During final rebase, current `origin/main` had
also consumed `br-frankenpandas-uza04.62` for the dense-outer source-provenance
campaign, so the final integrated filter-bool closeout is `.63` and its
follow-up residual is `.64`.

Timestamp: 2026-06-09T17:31:00-04:00

## Scope

Current candidate under test:

- `fp-frame` derives an affine selection certificate while scanning `boolean_mask_positions`.
- `DataFrame::loc_bool` passes that certificate to per-column gathering.
- `fp-columnar` accepts the certificate in `take_positions_with_affine_certificate`.
- The accepted all-valid Float64 path constructs `LazyStridedFloat64` from `{start, step, len}` without rescanning `positions`.

No source, bead, or `.skill-loop-progress.md` edits were made in this pass. New/updated evidence is limited to `tests/artifacts/perf/*uza04_46*`.

## Build

Command:

```bash
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza04-46-after \
  rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile \
  > tests/artifacts/perf/build_after_uza04_46.txt 2>&1
```

Result: PASS.

RCH mode: local fail-open. The log starts with `[RCH] local (all workers failed preflight checks)`.

After binary:

```text
/data/projects/.scratch/cargo-target-orangepeak-uza04-46-after/release-perf/examples/perf_profile
```

## Focused Verification

All requested crate-scoped checks passed except the explicitly allowed caveats below:

```text
PASS  rch exec -- cargo test -p fp-columnar take_positions_with_affine_certificate_uses_lazy_strided_float64 --lib
PASS  rch exec -- cargo test -p fp-frame boolean_mask_positions_tracks_affine_certificate --lib
PASS  rch exec -- cargo check -p fp-columnar --all-targets
PASS  rch exec -- cargo check -p fp-frame --lib
PASS  rch exec -- cargo clippy -p fp-columnar --all-targets -- -D warnings
PASS  rch exec -- cargo clippy -p fp-frame --lib -- -D warnings
PASS  git diff --check -- crates/fp-frame/src/lib.rs crates/fp-columnar/src/lib.rs
PASS  cargo fmt -p fp-columnar --check
FAIL  cargo fmt -p fp-frame --check
TIMEOUT  timeout 180s ubs crates/fp-columnar/src/lib.rs crates/fp-frame/src/lib.rs
PASS  timeout 180s ubs crates/fp-columnar/src/lib.rs
TIMEOUT  timeout 420s ubs crates/fp-frame/src/lib.rs
```

`cargo fmt -p fp-frame --check` failed on pre-existing unrelated formatting drift. The captured affected source ranges are examples plus `crates/fp-frame/src/lib.rs` around lines 23323, 23346, 45184, 50014, 54058, 54108, and 62133. The affine mask/certificate regions around `boolean_mask_positions`, `take_rows_by_positions_with_affine_certificate_unchecked`, and the focused test are not reported.

The first combined `ubs` run reached the 180s timeout after starting the Rust
scan; output is captured in `tests/artifacts/perf/ubs_uza04_46.txt`. A follow-up
split run completed for `fp-columnar` with exit 0 and broad pre-existing
file-wide warning inventory only. The `fp-frame` split run hit the 420s timeout
without emitting findings.

## Golden Verification

Commands:

```bash
/data/projects/.scratch/cargo-target-orangepeak-uza04-46-after/release-perf/examples/perf_profile \
  golden filter_bool 1000 \
  > tests/artifacts/perf/golden_after_filter_bool_1000_uza04_46.txt

/data/projects/.scratch/cargo-target-orangepeak-uza04-46-after/release-perf/examples/perf_profile \
  golden filter_bool 100000 \
  > tests/artifacts/perf/golden_after_filter_bool_100000_uza04_46.txt

sha256sum -c tests/artifacts/perf/golden_pair_uza04_46.sha256 \
  > tests/artifacts/perf/golden_pair_uza04_46.verify.txt
```

Result: PASS.

```text
tests/artifacts/perf/golden_uza04_46_pass1_filter_bool_1000.txt: OK
tests/artifacts/perf/golden_uza04_46_pass1_filter_bool_100000.txt: OK
tests/artifacts/perf/golden_after_filter_bool_1000_uza04_46.txt: OK
tests/artifacts/perf/golden_after_filter_bool_100000_uza04_46.txt: OK
```

After SHA values match the pass1 baselines:

```text
f7eb0f99728d924edb7398ec4cbcc07808651a0f091db01e1a3c12e011d6265c  tests/artifacts/perf/golden_after_filter_bool_1000_uza04_46.txt
2f77640d30dc32e1db9ab52eb869dc5ad503434f5a8a647a9d92f5f1ffa69cea  tests/artifacts/perf/golden_after_filter_bool_100000_uza04_46.txt
```

## Isomorphism Proof

- Ordering preserved: yes. `boolean_mask_positions` scans the mask from low to high index and pushes positions in the same order as the previous gather path. The certificate is metadata only; it does not reorder positions.
- Tie-breaking unchanged: yes / N/A. Boolean row filtering has no comparator ties. Index projection still iterates `positions` in order and preserves duplicate selected rows.
- Floating-point bits: identical by golden SHA for `filter_bool 1000` and `filter_bool 100000`. The all-valid Float64 fast path creates a lazy strided view over the same backing data; it does not recompute values.
- Null and validity behavior: preserved. The certificate fast path is accepted only for `DType::Float64` with `validity.all()`. All nullable or non-Float64 cases fall back to `take_positions`.
- Bounds and fallback: preserved. The all-valid Float64 path validates `last = start + step * (len - 1)` with checked arithmetic and rejects out-of-bounds certificates by falling back to `take_positions`. The `loc_bool` caller derives the certificate from the same mask scan that produced `positions`.
- RNG: unchanged / N/A.
- Panic behavior: preserved for fallback gathering. The certificate route is only used after mask-derived in-bounds positions in `loc_bool`; rejected certificates use the existing gather path.

## Benchmark

Paired command:

```bash
hyperfine --warmup 3 --runs 10 \
  --export-json tests/artifacts/perf/hyperfine_pair_uza04_46_filter_bool_100000_1000.json \
  '/data/projects/.scratch/cargo-target-orangepeak-uza04-46-pass1-worker/release-perf/examples/perf_profile filter_bool 100000 1000' \
  '/data/projects/.scratch/cargo-target-orangepeak-uza04-46-after/release-perf/examples/perf_profile filter_bool 100000 1000' \
  > tests/artifacts/perf/hyperfine_pair_uza04_46_filter_bool_100000_1000.txt 2>&1
```

Result:

```text
Before: 430.1 ms +/- 15.7 ms, range 408.8..462.0 ms, 10 runs
After:  173.0 ms +/-  6.8 ms, range 162.0..187.7 ms, 10 runs
Ratio:  2.49x faster, 59.77% lower mean time
```

This is directly comparable to the pass1 before binary on the same host session. The pass1 baseline supplied by the user was 434.5 ms +/- 9.3 ms; the paired before rerun measured 430.1 ms +/- 15.7 ms.

## After Profile

Commands:

```bash
perf record -F 997 -g --call-graph dwarf \
  -o tests/artifacts/perf/perf_after_uza04_46_filter_bool_100000_1000.data \
  -- /data/projects/.scratch/cargo-target-orangepeak-uza04-46-after/release-perf/examples/perf_profile \
  filter_bool 100000 1000 \
  > tests/artifacts/perf/perf_record_after_uza04_46_filter_bool_100000_1000.txt 2>&1

perf report --stdio --children --sort comm,dso,symbol \
  -i tests/artifacts/perf/perf_after_uza04_46_filter_bool_100000_1000.data \
  > tests/artifacts/perf/perf_report_after_uza04_46_filter_bool_100000_1000.txt 2>&1
```

Result: PASS, with restricted kernel symbol warnings only. `perf record` captured 208 samples.

Top after-profile rows:

```text
77.25% children / 0.00% self   loc_bool (inlined)
77.24% children / 76.00% self  <fp_frame::DataFrame>::loc_bool
47.03% children / 0.00% self   boolean_mask_positions (inlined)
25.60% children / 0.00% self   take_rows_by_positions_with_affine_certificate_unchecked (inlined)
14.92% children / 0.00% self   push<usize, alloc::alloc::Global> (inlined)
13.86% children / 0.00% self   push (inlined)
11.09% children / 0.00% self   index<i64, usize, alloc::alloc::Global> (inlined)
8.84% children / 0.00% self    build_numeric_frame (inlined)
```

The previous residual `bounded_arithmetic_progression_positions` / `take_strided_all_valid_float64_positions` does not appear in the after report search results. The bottleneck has shifted into mask position construction and row/index gathering inside `loc_bool`.

## Score

Impact: 5
Confidence: 5
Effort: 2

Score = Impact x Confidence / Effort = 5 x 5 / 2 = 12.5.

Verdict: KEEP. The lever clears the >=2.0 threshold, preserves goldens exactly, passes focused crate-scoped checks, and produces a paired 2.49x speedup for `filter_bool 100000 1000`.
