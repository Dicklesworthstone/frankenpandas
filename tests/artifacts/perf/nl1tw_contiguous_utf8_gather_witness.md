# br-frankenpandas-nl1tw Witness

## Target

- Bead: `br-frankenpandas-nl1tw`
- Lever: add an all-valid `LazyContiguousUtf8` gather path inside
  `Column::take_positions`, copying selected byte spans into one contiguous
  buffer plus offsets instead of materializing one `String`/`Scalar` per row.
- Profile-backed hotspot: `str_filter` on a text-heavy frame spent cycles in
  `Column::take_positions`, `_int_malloc`, `malloc_consolidate`, and per-row
  `ScalarValues::as_slice` materialization.

## Build Provenance

- Baseline worktree:
  `/data/projects/.scratch/frankenpandas-nl1tw-baseline-20260608T000718Z`
  at `3cda38efe4e9353af4db413d50ada152e281b457`, with only the
  `str_filter` benchmark/golden harness applied.
- Baseline build:
  `rch exec -- cargo build -p fp-conformance --example perf_profile --profile release-perf`
  with `CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-nl1tw-baseline-20260608T000718Z`.
- After build:
  `rch exec -- cargo build -p fp-conformance --example perf_profile --profile release-perf`
  with `CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-nl1tw-after-20260608T000718Z`.
- Final comparable builds both used `RCH remote vmi1156319`:
  baseline `build_perf_profile_baseline_nl1tw_remote_rebuild.txt`, after
  `build_perf_profile_after_nl1tw_rebuild.txt`.

## Golden SHA

Command shape:

```text
perf_profile golden str_filter 20000 | sha256sum
```

Result:

```text
MATCH str_filter 6a6e40a20adc24875545e5e1d8f4456298b36620f6dc7f73d7d904f4dfbbf9d9
```

Artifacts:

- `golden_nl1tw_baseline_str_filter.sha256`
- `golden_nl1tw_after_str_filter.sha256`
- `golden_compare_nl1tw.txt`

## Paired Hyperfine

Command shape:

```text
hyperfine --warmup 1 --runs 7 \
  baseline: perf_profile str_filter 200000 25 \
  after:    perf_profile str_filter 200000 25
```

Results:

| case | mean | sigma | user | system |
| --- | ---: | ---: | ---: | ---: |
| baseline | 407.3 ms | 12.5 ms | 338.4 ms | 68.2 ms |
| after | 141.8 ms | 8.7 ms | 110.2 ms | 30.8 ms |

Win: after ran `2.87x +/- 0.20x` faster than baseline.

Artifacts:

- `hyperfine_nl1tw_pair.txt`
- `fp_nl1tw_pair.json`

## Perf Stat

Command shape:

```text
perf stat -r 5 -- perf_profile str_filter 200000 25
```

| metric | baseline | after | ratio |
| --- | ---: | ---: | ---: |
| elapsed | 0.43087 s | 0.14182 s | 3.04x faster |
| task-clock | 420.67 ms | 138.93 ms | 3.03x lower |
| instructions | 5.521 B | 1.438 B | 3.84x lower |
| cycles | 1.707 B | 0.559 B | 3.05x lower |
| page-faults | 32762 | 15713 | 2.09x lower |

## Profile Shift

Baseline `perf report --stdio --no-children`:

- `Column::take_positions`: 28.85%
- `_int_malloc`: 27.77%
- `malloc_consolidate`: 10.35%
- `__memmove_avx_unaligned_erms`: 4.18%
- `ScalarValues::as_slice`: 3.63%

After:

- `Column::take_positions`: 94.48%
- `__memmove_avx_unaligned_erms` under `take_positions`: 74.87%

Interpretation: the lever removed allocator-heavy per-row string materialization
and shifted the residual to unavoidable raw byte copying for selected UTF-8
spans. The next deeper primitive should attack gather bandwidth directly:
selection-run coalescing, mask-to-run compression, or byte-count prefix planning
for contiguous string filters instead of more per-row scalar tuning.

## Isomorphism Proof

- Ordering: `positions` is traversed in the same order as the fallback path.
  Each output row appends exactly the source span at `positions[i]`.
- Tie-breaking: no compare or dedup semantics are introduced; duplicate string
  values remain duplicated at the same output positions.
- Floating point: numeric columns and numeric values are untouched by this
  lever. The benchmark output includes only string columns, and the public
  filter path's row count/index order is unchanged by the golden SHA.
- RNG: no RNG is introduced.
- Hashing: no hash map or hash seed participates in the new path.
- Nulls: the path is inside `validity.all()` and uses
  `as_utf8_contiguous()`, so it only applies to all-valid contiguous UTF-8.
  Nullable or non-contiguous UTF-8 continues through the existing fallback.
- Dtypes/storage: output dtype remains `Utf8`, validity remains all-valid for
  exactly `positions.len()` rows, and materialization yields the same
  `Scalar::Utf8` strings as the fallback. The storage representation is lazy
  contiguous UTF-8 rather than eager `Vec<Scalar>`, which is not public
  behavior.
- Bounds: the previous path indexed `self.values[p]`; the new path indexes
  `offsets[p]..offsets[p + 1]` under the same caller-supplied valid-position
  contract.

## Validation

- `rustfmt --edition 2024 --check` on changed Rust files: pass.
- `rch exec -- cargo test -p fp-columnar --lib`: pass.
- `rch exec -- cargo test -p fp-frame iloc -- --nocapture`: pass.
- `rch exec -- cargo check -p fp-columnar -p fp-frame -p fp-conformance --all-targets`: pass on `vmi1156319`.
- `rch exec -- cargo clippy -p fp-columnar -p fp-frame -p fp-conformance --all-targets -- -D warnings`: pass.
- `ubs` on six changed Rust files: exit 1 due broad existing inventory; no
  new finding was identified in the `Column::take_positions` hunk. The only
  direct nearby hit is the benchmark helper's `c as u32` cast.

## Score

- Impact: 5
- Confidence: 4
- Effort: 2
- Score: `5 * 4 / 2 = 10.0`
- Decision: keep.
