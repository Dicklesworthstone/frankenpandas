# br-frankenpandas-8sxez proof: Float64 value_counts numeric index

## Target and Profile Evidence

- Bead: `br-frankenpandas-8sxez`
- Workload: `fp-bench --category dataframe_ops --workload value_counts --size 100k --dtype float64 --json`
- Profile-backed hotspot from bead: 96k distinct Float64 values spent about 8.0 ms in output materialization, dominated by `format!("{v:?}")` string allocation for every distinct Float64 label. Tally remained about 5.8 ms and column build about 1.6 ms.
- Baseline comparator: prior default `Series::value_counts()` emitted `IndexLabel::Utf8(format!("{v:?}"))` for Float64 values through the shared `scalar_to_value_counts_index_label` helper.

## Change

`Series::value_counts()` now uses a private value-counts-only label mapper for its default output materializer:

- `Scalar::Float64(v)` becomes `IndexLabel::Float64(fp_index::OrderedF64(v))`.
- Every other scalar still routes through `scalar_to_value_counts_index_label`.
- The shared helper remains unchanged, so pivot/crosstab/mode/categorical/value_counts_with_options call sites keep their prior label behavior.
- Tallying, stable count-desc sort, first-seen tie order, counts, null dropping, and result/index names are unchanged.

## Alien/optimization contract

- Graveyard primitive: replace string materialization on a hot non-DoS internal path with typed/hash-friendly representation; the canonical FrankenSuite quick-fix table calls out default string/hash overheads as hot-path targets.
- Artifact family: certified rewrite with a typed output witness.
- EV score: Impact 3 * Confidence 4 / Effort 1 = 12.0.
- Fallback/rollback: `git revert <commit>` restores Utf8 Float64 labels and previous benchmark behavior.

## Behavior Proof

- Ordering preserved: yes. The change happens after `counts.sort_by(...)`; label conversion preserves the existing sorted count order and stable first-seen tie order.
- Tie-breaking unchanged: yes. No change to tally keys, first-seen storage, or stable sort.
- Floating-point arithmetic unchanged: yes. The exact stored `f64` value is moved into `OrderedF64`; no rounding, arithmetic, parsing, or formatting participates in ordering or counts.
- Missing/NaN behavior unchanged for default `value_counts`: `as_f64_slice()` still excludes NaN-bearing columns; generic missing handling and null dropping are unchanged.
- RNG unchanged: no RNG.
- Intended parity change: Float64 value_counts labels become numeric. Pandas oracle artifact `8sxez_pandas_oracle_value_counts_float_index.txt` shows `pd.Series([1.0, 2.0, 2.0], name="vals").value_counts()` has `index_dtype=float64`, values `[2.0, 1.0]`, result name `count`, and index name `vals`.
- Focused contract test: `series_value_counts_float_labels_are_numeric_8sxez`.
- Golden checksums: existing non-Float64 value_counts goldens are unchanged and verified with `sha256sum -c tests/artifacts/perf/8sxez_after_value_counts_goldens.sha256`.

## Benchmark Evidence

Baseline source: `bdc396be` (`edi9i` landed locally before this lever).

Build baseline:

- `CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-lavenderstone-8sxez-base rch exec -- cargo build -p fp-bench --profile release-perf --bin fp-bench`
- RCH worker: `vmi1227854`
- Artifact: `8sxez_base_build_fp_bench.txt`

Direct `fp-bench` internal timing:

- Baseline mean: 12.485 ms, p50: 12.194 ms
- After mean: 9.298 ms, p50: 8.856 ms
- Mean speedup: 1.34x
- p50 speedup: 1.38x
- Delta artifact: `8sxez_fp_bench_value_counts_delta.json`

Hyperfine full-process timing:

- Baseline: 379.6 ms +/- 11.6 ms
- After: 311.6 ms +/- 14.4 ms
- Speedup: 1.22x
- Delta artifact: `8sxez_hyperfine_value_counts_delta.json`

## Validation

- `rch exec -- cargo test -p fp-frame value_counts --lib -- --nocapture`
  - Passed: 33 passed, 0 failed, 1 ignored.
  - Artifact: `8sxez_after_test_fp_frame_value_counts.txt`
- `rch exec -- cargo check -p fp-frame --lib`
  - Passed.
  - Artifact: `8sxez_after_check_fp_frame_lib.txt`
- `rch exec -- cargo clippy -p fp-frame --lib -- -D warnings`
  - Passed.
  - Artifact: `8sxez_after_clippy_fp_frame_lib.txt`
- `git diff --check`
  - Passed.
  - Artifact: `8sxez_after_git_diff_check.txt`
- `cargo fmt -p fp-frame -- --check`
  - Reported pre-existing unrelated rustfmt drift in older sort/take/reindex hunks.
  - The new value_counts helper and test hunk do not appear in the rustfmt diff.
  - Artifact: `8sxez_after_fmt_fp_frame_check.txt`
- `ubs crates/fp-frame/src/lib.rs tests/artifacts/perf/proof_8sxez_value_counts_float_index.md .skill-loop-progress.md`
  - Inconclusive: scanner timed out with `UBS_STATUS=124` while still at `Scanning rust...`.
  - Artifacts: `8sxez_after_ubs_changed_files.txt`, `8sxez_after_ubs_changed_files_status.txt`

## Residual

The remaining direct mean is about 9.30 ms. The bead's profile says the residual tally phase is FxHashMap/cache-miss-bound for high-cardinality Float64 values; the next pass should target the tally data structure/layout rather than output label formatting.
