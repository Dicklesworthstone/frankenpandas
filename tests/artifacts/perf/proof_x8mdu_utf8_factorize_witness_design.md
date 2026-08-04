# br-frankenpandas-x8mdu: contiguous UTF-8 factorize witness design

## Scope

Mission: baseline and witness design only. No runtime source edits were made in
this pass.

Bead: `br-frankenpandas-x8mdu`

Current checked-out runtime source already contains an uncommitted x8mdu-shaped
candidate in:

- `crates/fp-columnar/src/lib.rs`
- `crates/fp-frame/src/lib.rs`

This artifact records the minimum safe shape and proof gate for that candidate.
It does not claim the source hunk as authored in this pass.

## Baseline Evidence

RCH-built baseline artifact:

- `tests/artifacts/perf/lavender_vcstr_current_cardinality_x50.json`
- sha256: `8a108f4a8814dc580898b272acc8dbf79b4f5aaf044810238bc1736f3e502743`

Build artifact:

- `tests/artifacts/perf/lavender_vcstr_current_build_perf_profile.txt`
- worker: `vmi1227854`
- command: `cargo build -p fp-conformance --profile release-perf --example perf_profile`

Current-head timing matrix from the artifact:

| Scenario | Command | Mean | Stddev |
|---|---:|---:|---:|
| `str_factorize` | `perf_profile str_factorize 500000 50` | 398.8 ms | 22.6 ms |
| `str_duplicated` | `perf_profile str_duplicated 500000 50` | 380.9 ms | 18.1 ms |
| `str_value_counts` | `perf_profile str_value_counts 500000 50` | 321.7 ms | 9.7 ms |
| `str_unique` | `perf_profile str_unique 500000 50` | 282.9 ms | 9.0 ms |

Profiler status:

- `perf` and `samply` are blocked by `perf_event_paranoid=4`.
- `tests/artifacts/perf/lavender_vcstr_current_strace_str_factorize_x5.txt`
  shows only 110 syscalls and 0.001324 s total syscall time, so the residual is
  not syscall-bound.

Prior rejection:

- `tests/artifacts/perf/proof_a3y5o_utf8_factorize_byte_span_reject.md`
- sha256: `584d802d17b4462f5649d4a270ac91dd58fa82c83d2191ad193e838b5d4aa9eb`
- `a3y5o` default factorize goldens matched before/after:
  - `str_factorize 1000`: `cd726ef905573f7e869818443fb3320d49ecf7fbc0556ca2d2d6794743491fc4`
  - `str_factorize 500000`: `da2da85584bdca84af3055c9104c531d5bdf760b191420e8aa5f8366cfd3fe11`
- x50 paired gate rejected the per-call byte-span hunk:
  - forward: baseline 372.3 ms +/- 13.6 -> candidate 398.3 ms +/- 28.5
  - reversed: candidate 381.1 ms +/- 28.7 -> baseline 367.2 ms +/- 18.7

Pre-existing x8mdu check logs observed in the allowed artifact namespace:

- `tests/artifacts/perf/x8mdu_check_fp_columnar_lib.txt`
  - sha256: `f98206c9ec68fc3f09478c451bcd832aa8251ff8075d89056a09c1e8103d9d7d`
  - contains RCH `cargo check -p fp-columnar --lib`
  - finished successfully on worker `vmi1227854`
- `tests/artifacts/perf/x8mdu_check_fp_frame_lib.txt`
  - sha256: `bd736d54922a23585f6044f001679d000384098b0c514b5670bbd6edfaefa351`
  - starts RCH `cargo check -p fp-frame --lib`
  - does not contain a final success line, so it is not proof of pass

## Files And Functions Inspected

Issue and coordination:

- `br show br-frankenpandas-x8mdu --json`
- `br list --status in_progress --json`
- Agent Mail reservation for `tests/artifacts/perf/x8mdu*` and
  `tests/artifacts/perf/proof_x8mdu*`

Benchmark harness and artifacts:

- `crates/fp-conformance/examples/perf_profile.rs`
  - `build_str_vc_series`
  - `str_value_counts`
  - `str_unique`
  - `str_factorize`
  - `str_duplicated`
- `tests/artifacts/perf/lavender_vcstr_current_cardinality_x50.json`
- `tests/artifacts/perf/lavender_vcstr_current_build_perf_profile.txt`
- `tests/artifacts/perf/lavender_vcstr_current_perf_stat_str_factorize_x50.txt`
- `tests/artifacts/perf/lavender_vcstr_current_strace_str_factorize_x5.txt`
- `tests/artifacts/perf/proof_a3y5o_utf8_factorize_byte_span_reject.md`
- `tests/artifacts/perf/a3y5o_base_golden_sha256.txt`
- `tests/artifacts/perf/a3y5o_after_golden_sha256.txt`

Frame-level consumers:

- `crates/fp-frame/src/lib.rs`
  - `Series`
  - `Series::value_counts`
  - `Series::value_counts_with_options`
  - `Series::unique`
  - `Series::factorize_with_options`
  - `Series::factorize`
  - `Series::duplicated`
  - `Series::duplicated_keep`
  - `Series::drop_duplicates`
  - `Series::drop_duplicates_keep`

Columnar backing and constructors:

- `crates/fp-columnar/src/lib.rs`
  - `Utf8FactorizeDefaultWitness` (currently uncommitted source hunk)
  - `build_utf8_factorize_default_witness` (currently uncommitted source hunk)
  - `ScalarValues::LazyContiguousUtf8`
  - `ScalarValues::lazy_all_valid_int64_arc`
  - `ScalarValues::lazy_contiguous_utf8_arc`
  - `Column::as_utf8_contiguous`
  - `Column::utf8_default_factorize_columns` (currently uncommitted source hunk)
  - `Column::take_positions`
  - `Column::factorize`
  - `Column::factorize_with_options`
  - `Column::unique`
  - `Column::value_counts_with_options`

Directive and primitive mapping:

- `/data/projects/.scratch/no_gaps_directive.txt`
- `/data/projects/alien_cs_graveyard/alien_cs_graveyard.md`
- `/data/projects/alien_cs_graveyard/high_level_summary_of_frankensuite_planned_and_implemented_features_and_concepts.md`

## Highest-EV Witness Shape

Implement, or keep if the current uncommitted hunk is validated, this shape on
`ScalarValues::LazyContiguousUtf8`:

```rust
struct Utf8FactorizeDefaultWitness {
    codes: Arc<[i64]>,
    unique_bytes: Arc<[u8]>,
    unique_offsets: Arc<[usize]>,
}
```

Attach it as:

```rust
factorize_default: OnceLock<Utf8FactorizeDefaultWitness>
```

Build rule:

1. Walk source rows once in row order.
2. Key each row by exact byte span `&bytes[offsets[i]..offsets[i + 1]]`.
3. Use a local `FxHashMap<&[u8], i64>` only during witness construction.
4. On first sight, assign the next code, append the span to
   `unique_bytes`, and append the new byte length to `unique_offsets`.
5. On every row, push the code to `codes`.
6. Drop the map after construction. Do not cache borrowed span keys.

Return rule:

1. Codes column is `DType::Int64` with
   `ScalarValues::lazy_all_valid_int64_arc(Arc::clone(&witness.codes))`.
2. Uniques column is `DType::Utf8` with
   `ScalarValues::lazy_contiguous_utf8_arc(Arc::clone(&witness.unique_bytes),
   Arc::clone(&witness.unique_offsets))`.
3. `Series::factorize()` wraps those columns with the same generated
   `0..len` code labels and `0..nuniques` unique labels as the current path.

Why this is the highest-EV shape:

- It directly targets the x50 residual: repeated `Series::factorize()` on one
  immutable all-valid contiguous UTF-8 Series.
- The first call pays the same necessary dictionary discovery pass as the
  byte-span path; later calls avoid the whole O(n) hash pass and avoid rebuilding
  distinct `Scalar::Utf8` values.
- It returns Arc-backed typed columns, so each repeated call clones immutable
  buffers instead of rebuilding the 500000-row code vector.
- It is narrower than a general dictionary cache: no nullable/object/sorted
  semantics are changed.
- It follows the project-local witness pattern already used by
  `strictly_increasing`, `fixed_width`, `lower_hex_sequence`, dense-cycle, and
  affine-selection witnesses.

Do not add counts to the first ship unless the next profile proves
`str_value_counts` or `duplicated_keep(None)` is the top residual. Counts can be
derived from `codes` with a direct-indexed `Vec<usize>` in a later one-lever
commit, without expanding this factorize proof surface now.

EV score:

- Impact: 4 (expected to remove repeated O(n) span hashing and large code Vec
  rebuild from `str_factorize 500000x50`)
- Confidence: 4 (current profile shape and a3y5o rejection both isolate repeated
  per-call work)
- Effort: 2 (small local witness plus narrow factorize hooks)
- Score: `(4 * 4) / 2 = 8.0`

## Fallback Trigger

Use the witness only when all gates are true:

1. Series has no categorical metadata.
2. `sort == false`.
3. `use_na_sentinel == true`.
4. `Column::dtype() == DType::Utf8`.
5. `Column::validity().all()`.
6. Backing is exactly `ScalarValues::LazyContiguousUtf8`.

Fallback to the existing byte-span or generic paths when any gate fails:

- categorical Series
- nullable UTF-8
- object/eager `Vec<Scalar>` UTF-8
- `LazyNullableUtf8`
- `LazyGatherUtf8`
- `LazyUtf8Slice`
- `LazyLowerHexSequenceUtf8`
- `sort=true`
- `use_na_sentinel=false`
- any non-UTF8 dtype

The `LazyLowerHexSequenceUtf8` fallback is intentional for this pass. It has a
different certificate and should only get a separate arithmetic/code witness if
a later profile makes it the dominant residual.

## Isomorphism Obligations

Ordering:

- Preserve row-order code assignment.
- Unique output order is first-seen order because `unique_bytes` is appended
  only at first encounter while scanning rows from 0 to n-1.
- Code Series labels remain `0..len`.
- Unique Series labels remain `0..nuniques`.

Tie-breaking:

- No sort is introduced.
- Equal strings are equal iff the source byte spans are exactly equal, matching
  `Scalar::Utf8(s).as_str()` equality for valid UTF-8.
- Later value-count reuse, if implemented, must sort stably by count so equal
  counts retain code/first-seen order.

Missing/null semantics:

- Witness is all-valid only.
- Default factorize therefore emits no `-1` codes.
- Nullable, object, `sort=true`, and `use_na_sentinel=false` paths must keep
  the existing fallback semantics.

Floating point:

- N/A for this UTF-8 witness. No f64 operations or f64 labels are touched.

RNG:

- N/A. No random behavior or seeds are touched.

Golden SHA gate:

- Before and after candidate:
  - `perf_profile str_factorize 1000 1`
  - `perf_profile str_factorize 500000 1`
- Required hashes must match current goldens:
  - `str_factorize 1000`: `cd726ef905573f7e869818443fb3320d49ecf7fbc0556ca2d2d6794743491fc4`
  - `str_factorize 500000`: `da2da85584bdca84af3055c9104c531d5bdf760b191420e8aa5f8366cfd3fe11`
- Also run a targeted differential unit that compares:
  - current generic fallback on an eager UTF-8 Series
  - witness path on a contiguous UTF-8 Series with the same values
  - cases: empty, all unique, all same, mixed repeated, empty string distinct
    from non-empty string, high-cardinality repeated keys.

Benchmark gate:

- Rebuild with RCH:
  - `rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile`
- Paired/reversed hyperfine at `500000x50`:
  - baseline command before candidate
  - candidate command after candidate
  - candidate-first reversed run
- Keep only if Score >= 2.0 and the x50 paired/reversed gate shows a real
  speedup, not only x5 noise.
- Neutrality check:
  - `str_unique 500000 50`
  - `str_duplicated 500000 50`
  - `str_value_counts 500000 50`
  These may improve later via the witness, but this pass must not regress them.

Quality gate for implementation pass:

- `rch exec -- cargo test -p fp-columnar factorize --lib`
- `rch exec -- cargo test -p fp-frame factorize --lib`
- `rch exec -- cargo clippy -p fp-columnar --all-targets -- -D warnings`
- `rch exec -- cargo clippy -p fp-frame --all-targets -- -D warnings`
- `cargo fmt --check`
- `ubs crates/fp-columnar/src/lib.rs crates/fp-frame/src/lib.rs`

## What Not To Retry

Do not retry the a3y5o per-call byte-span factorize hunk. It preserved behavior
but failed the x50 repeated-call gate because the steady-state workload still
rebuilt the dictionary every call.

Do not optimize around syscalls. The strace artifact shows the workload is not
syscall-bound.

Do not rely on `perf` or `samply` until `perf_event_paranoid` changes. Treat
their block as environment evidence, not missing proof.

Do not cache `FxHashMap<&[u8], i64>` or any borrowed span map inside the column.
The safe reusable artifact must own `Arc` buffers or primitive vectors only.

Do not broaden the first implementation to nullable/object/sorted factorize.
Those paths include missing bucket and sorting semantics that are outside this
default all-valid proof.

Do not touch `br-frankenpandas-arr72` lazy RangeIndex work.

Do not co-land unrelated Float64 `value_counts` materializer work with x8mdu.
It is visible in the current diff, but it is a separate bead surface and needs
its own proof/bench closeout.
