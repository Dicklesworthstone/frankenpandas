# br-frankenpandas-uza04.74 Pass 2 Primitive Selection

Worktree: `/data/projects/.scratch/frankenpandas-orangepeak-uza04-47-20260611`
Head observed in Pass 1: `328d6228016d788d615c52ec1352666250f5fead`
Mode: planning/proof artifact only. No source or bead edits.

## Inputs Read

- Pass 1 witness: `tests/artifacts/perf/uza0474_pass1_20260611T010909Z_witness.md`
- Bead status: `br-frankenpandas-uza04.74` is `in_progress`, assigned to `OrangePeak`, labels include `perf`, `filter-bool`, `fp-frame`, `no-gaps`.
- Hot code:
  - `crates/fp-frame/src/lib.rs:2861-2902`: `AffineSelectionBuilder::push` and `finish`.
  - `crates/fp-frame/src/lib.rs:2906-2948`: `boolean_mask_affine_certificate` scans the bool mask and proves affine selected positions.
  - `crates/fp-frame/src/lib.rs:32715-32772`: `take_rows_by_affine_certificate_unchecked`.
  - `crates/fp-frame/src/lib.rs:32739-32746`: non-unit unit-range labels are eagerly pushed into `Vec<i64>`.
  - `crates/fp-frame/src/lib.rs:32754-32761`: typed Int64 labels selected by affine positions are eagerly pushed into `Vec<i64>`.
  - `crates/fp-frame/src/lib.rs:35612-35613`: `DataFrame::loc_bool` enters the affine fast path.
  - `crates/fp-index/src/lib.rs:364-418`: `IndexLabels` has lazy unit-range, typed Int64 `Arc<Vec<i64>>`, and contiguous Utf8 backings.
  - `crates/fp-index/src/lib.rs:431-520`: materialization and typed Int64 view currently allocate when only unit-range or materialized/typed Vec forms exist.
  - `crates/fp-index/src/lib.rs:705-745`: public constructors cover lazy unit range and eager typed Vec, but not lazy non-unit affine/strided Int64 labels.
- Graveyard references:
  - `alien_cs_graveyard.md` Section 8.2: vectorized execution/morsel-driven contracts require profile-backed operator hotspots, semantics proof, and conservative fallback.
  - `alien_cs_graveyard.md` Section 7.1: succinct integer sequence notes include compressed/lazy integer access forms and O(1) access for sorted integer sequences. Section 7.5 ART was read but does not match this sequential affine-label projection hotspot.
  - `high_level_summary...md`: vectorized execution is low-risk and proven for columnar/OLAP-style operators; apply the idea as cache-sized descriptorized batches rather than pointer-heavy per-label materialization.
  - `alien-artifact-coding` rewrite obligations: accepted performance rewrites need explicit domain assumptions, equivalence evidence, replay/golden fixtures, and a counterexample archive.

## Recommendation Card

Change:

Add a lazy affine/strided Int64 index-label descriptor and use it in `take_rows_by_affine_certificate_unchecked` for affine boolean filters. The descriptor should represent output labels as:

- arithmetic progression: `label_i = start + i * step`, for projected unit-range indexes with `step != 1`;
- strided typed view: `label_i = source_values[start + i * position_step]`, for existing all-Int64 label backings.

The key implementation boundary is `fp-index`: add an `IndexLabels` backing that can answer `len`, `labels()`, `int64_label_values()`, and cache queries from the descriptor, materializing only on demand. Then replace the eager `Vec<i64>` loops at `fp-frame/src/lib.rs:32739-32746` and `32754-32761` with constructor calls.

Hotspot evidence:

- Pass 1 baseline for `filter_bool 100000 1000`: `152.8 ms +/- 3.3 ms`, range `147.2 ms ... 158.9 ms`.
- Pass 1 profile: `95.25%` self time in `<fp_frame::DataFrame>::loc_bool`.
- Inline evidence points at `boolean_mask_affine_certificate`, `AffineSelectionBuilder::push`, and affine Int64 label materialization (`labels.push(values[pos])`) under `take_rows_by_affine_certificate_unchecked`.
- Column affine gather is no longer the dominant residual: `fp_columnar::Column::take_affine_all_valid_float64_positions` was only about `0.17%` children / `0.07%` self in the Pass 1 inline report.

Mapped graveyard sections:

- Section 8.2 Vectorized Execution + Morsel-Driven Parallelism: treat row-label projection as a vectorized operator with a compact descriptor/witness, not as per-row scalar pushes on the hot path.
- Section 7.1 Succinct Data Structures / Elias-Fano family: the useful idea here is not Elias-Fano encoding itself, because the sequence is exactly affine or a strided typed view; the applicable primitive is succinct/lazy integer-sequence access with O(1) element reconstruction and deferred materialization.
- Section 7.5 Adaptive Radix Trees: explicitly not selected. ART optimizes lookup/range search in key indexes, while this profile is sequential label projection during boolean filter output construction.
- Alien-artifact certified rewrite pipeline: this is a representation-preserving rewrite with explicit equivalence obligations, golden fixtures, and fallback to eager materialization.

EV score:

- Alien EV: `(Impact 4 * Confidence 4 * Reuse 3) / (Effort 2 * AdoptionFriction 1) = 24.0`.
- Extreme optimization score: `(Impact 4 * Confidence 4) / Effort 2 = 8.0`.
- Selected Score for implementation gate: `8.0`, above the required `>= 2.0`.

Priority tier:

- Tier A: implement in the next pass. It is directly profile-backed and isolated, but still needs careful index-cache semantics.

Fallback trigger:

- Do not keep the source change if same-worker paired hyperfine does not improve `filter_bool 100000 1000` by at least `8%` versus the `152.8 ms` baseline, or if after-profile still shows affine Int64 label materialization loops above `1%` self time.
- Also reject if any golden SHA changes, if duplicate/sort cache behavior changes for non-unit projections, or if any path panics where the current eager fallback returns `None`/fallback result.

Isomorphism proof plan:

- Domain: `certificate.start + certificate.step * (len - 1)` is checked in bounds before constructing the descriptor; `certificate.step > 0` follows from `AffineSelectionBuilder` rejecting zero step.
- Ordering preserved: descriptor iteration order is exactly `i = 0..len`, same as the current eager loops.
- Tie-breaking unchanged: N/A for row filtering; repeated labels in strided typed views must remain in first-seen positional order.
- Floating-point unchanged: no column value arithmetic changes; `f64` buffers and validity maps are untouched.
- RNG unchanged: no RNG.
- Index names/order unchanged: call the same `rename_index(self.index.name())` path after constructing `out_index`.
- Error/fallback behavior unchanged: keep existing overflow and bounds checks before descriptor construction; fallback to current materialized path if a descriptor cannot be constructed.
- Golden verification: capture new implementation outputs for `filter_bool 1000` and `filter_bool 100000`, compute their sha256 values, and compare the hash fields against `tests/artifacts/perf/uza0474_pass1_20260611T010909Z_golden_sha256.txt`.
- Additional equivalence fixtures: add small tests for empty, singleton, unit-step, non-unit step, non-zero start, overflow boundary, duplicate typed labels, descending/non-monotonic typed labels, and forced `labels()` materialization after filtering.

Before/after target:

- Baseline comparator: Pass 1 `rch exec` build plus hyperfine on `filter_bool 100000 1000`, mean `152.8 ms +/- 3.3 ms`.
- Keep threshold: mean <= `140.5 ms` on the same worker/run mode (`>= 8%` win).
- Target: mean <= `132 ms` (`~14%` win); aspirational if both eager Int64 loops disappear and no new materialization is triggered by the harness: <= `125 ms`.
- After-profile target: no `labels.push(value)` / `labels.push(values[pos])` line visible above `1%` self time; next residual should be certificate scanning or DataFrame output construction.

Primary risk + countermeasure:

- Risk: cache semantics drift in `Index` if the lazy strided view incorrectly pre-seeds uniqueness or ascending sort metadata.
- Countermeasure: for arithmetic progression with positive step, it is safe to pre-seed duplicate false and ascending Int64. For strided view into arbitrary typed values, do not pre-seed duplicate or sort caches unless the source index has a proven cache state that can be transported; let existing lazy detectors materialize or compute when requested. Add tests that cover duplicate/non-monotonic source labels.

Artifact pack:

- Baseline witness: `uza0474_pass1_20260611T010909Z_witness.md`
- Build log: `uza0474_pass1_20260611T010909Z_build.log`
- Golden outputs and checksums: `uza0474_pass1_20260611T010909Z_golden_*`
- Baseline hyperfine txt/json: `uza0474_pass1_20260611T010909Z_hyperfine_filter_bool_100000x1000.*`
- Profile data/reports/annotation: `uza0474_pass1_20260611T010909Z_perf_*`
- This selection/proof plan: `uza0474_pass2_20260611T011834Z_primitive_selection.md`
- Required implementation-pass additions: after-build log, after hyperfine txt/json, after perf report/annotation, golden `sha256sum` comparison log, and an isomorphism note referencing the exact commit.

Rollback:

- One lever only. If implementation fails the threshold or any proof gate, revert the single commit that adds the lazy affine/strided Int64 index descriptor and restores the two eager `Vec<i64>` construction paths.

Baseline comparator:

- Current eager non-unit affine label materialization in `take_rows_by_affine_certificate_unchecked`, measured by Pass 1 on current head with `filter_bool 100000 1000`.

## Selected Primitive

Selected: lazy affine/strided Int64 index descriptor.

This is the best one-lever implementation because the current profile no longer points at column gather or mask-position allocation. It points at row-label projection under `DataFrame::loc_bool`, and the code already has the proof object needed to describe selected positions. The missing primitive is a non-unit affine/strided label backing in `fp-index`.

This is not a repeat of prior levers:

- Not `.64`: it does not change no-position mask scanning or column gather.
- Not `.63`: it does not derive a new mask affine certificate.
- Not `.46`: it does not thread an existing certificate through more call sites.
- Not old bounded arithmetic progression positions: it does not materialize row positions or retune bounds checks; it changes the label backing so the proven projection is represented lazily.

## Implementation Surface For Later Pass

Files to edit:

- `crates/fp-index/src/lib.rs`
  - Add an `Int64AffineLabels`/`Int64StridedLabels` descriptor type or one enum that covers arithmetic progression and source-Arc strided view.
  - Extend `IndexLabels` storage, `as_slice`, `len`, `int64_view`, and `cached_int64_view` to understand the descriptor.
  - Add constructors such as `Index::new_known_unique_int64_affine_range(start, step, len)` and `Index::from_i64_strided_values(values, start, step, len)`.
  - Keep descriptor construction fallible where overflow or source bounds cannot be proven.
- `crates/fp-frame/src/lib.rs`
  - Replace eager loops at `32739-32746` and `32754-32761` with the new index constructors.
  - Preserve the existing bounds/overflow gates and `rename_index` behavior.

Focused tests to run:

- Unit tests in `fp-index` for descriptor len, `labels()`, `int64_label_values()`, cached view behavior, duplicate detection, sort detection, clone sharing, and materialization idempotence.
- Existing frame tests around DataFrame/Series boolean filtering:
  - `dataframe_loc_bool_selects_true_rows`
  - `dataframe_loc_bool_wrong_length_rejected`
  - `dataframe_loc_bool_all_false_returns_empty`
  - `dataframe_iloc_bool_delegates_to_loc_bool`
  - `dataframe_loc_bool_golden_basic`
  - `dataframe_iloc_bool_golden_basic`
- Add targeted regression tests for:
  - default/unit-range index filtered by every-other mask;
  - non-zero starting Int64 index filtered by every-third mask;
  - all-Int64 duplicate labels filtered by affine mask;
  - all-Int64 non-monotonic labels filtered by affine mask;
  - forcing `out.index().labels()` after filtering to prove materialized labels match the old eager vector.

Crate-scoped validation commands for implementation pass:

```bash
rch exec -- cargo test -p fp-index
rch exec -- cargo test -p fp-frame dataframe_loc_bool --lib
rch exec -- cargo test -p fp-frame dataframe_iloc_bool --lib
rch exec -- cargo test -p fp-conformance --example perf_profile
rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
<release-perf perf_profile> golden filter_bool 1000 > tests/artifacts/perf/uza0474_after_<timestamp>_golden_filter_bool_1000.txt
<release-perf perf_profile> golden filter_bool 100000 > tests/artifacts/perf/uza0474_after_<timestamp>_golden_filter_bool_100000.txt
sha256sum tests/artifacts/perf/uza0474_after_<timestamp>_golden_filter_bool_1000.txt tests/artifacts/perf/uza0474_after_<timestamp>_golden_filter_bool_100000.txt > tests/artifacts/perf/uza0474_after_<timestamp>_golden_sha256.txt
cut -d' ' -f1 tests/artifacts/perf/uza0474_pass1_20260611T010909Z_golden_sha256.txt > tests/artifacts/perf/uza0474_after_<timestamp>_expected_hashes.txt
cut -d' ' -f1 tests/artifacts/perf/uza0474_after_<timestamp>_golden_sha256.txt > tests/artifacts/perf/uza0474_after_<timestamp>_actual_hashes.txt
diff -u tests/artifacts/perf/uza0474_after_<timestamp>_expected_hashes.txt tests/artifacts/perf/uza0474_after_<timestamp>_actual_hashes.txt
hyperfine --warmup 3 --runs 10 '<release-perf perf_profile> filter_bool 100000 1000'
perf record -F 999 -g --call-graph fp -o tests/artifacts/perf/uza0474_after_<timestamp>_perf_filter_bool_100000x20000.data -- '<release-perf perf_profile>' filter_bool 100000 20000
```
