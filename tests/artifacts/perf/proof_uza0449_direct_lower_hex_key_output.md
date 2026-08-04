# br-frankenpandas-uza04.49 proof: direct lower-hex key output

## Profile-backed target

Baseline was current `origin/main` (`a1b2726f`) in the clean worktree
`/data/projects/.scratch/frankenpandas-orangepeak-3wo1d-current-20260609`.

The 5M/5M ordered-UTF8 join profile showed repeated output materialization
inside the contiguous no-overlap merge-output path:

- `__memmove_avx_unaligned_erms`: 26.18% self
- `Column::take_contiguous_range`: 53.18% children under
  `fp_join::build_single_key_inner_merge_output_with_selections`
- `Column::as_utf8_contiguous` / lower-hex materialization: 10.54% children
- `join_bench::add_numeric_checksum`: 9.02% self

The previous generic lower-hex range-take lever (`uza04.48`) was rejected, so
this pass did not change `Column::take_contiguous_range`. Instead it attacked
the merge-output builder directly: when the ordered unique UTF8 plan already
proves a contiguous shared-key overlap and the left key column carries a
lower-hex sequence witness, synthesize the output key column as a new
`LazyLowerHexSequenceUtf8` at the shifted start value.

## Isomorphism proof

- Source row `left_start + i` and output row `i` both describe
  `prefix || fixed_width_lower_hex(start + left_start + i)`.
- Prefix bytes, hex width, all-valid null layout, row order, column order, and
  suffix/error fallthrough are unchanged.
- The lever is gated to the existing no-overlap contiguous-output fast path,
  so overlapping non-key columns still use the generic suffix/error route.
- Floating-point value columns are not transformed by the new branch; existing
  `take_contiguous_range` handling still determines their representation.
- No RNG state is used by this benchmark or merge path.
- Normal and empty ordered-UTF8 golden outputs were byte-identical:
  - normal SHA before/after:
    `2ac49173153820d4b3878817c44be31979faa18b2ae034167f7977adee83b02e`
  - empty SHA before/after:
    `fc03a4635d1fe035e39a6f625acc9a3093dae0e9c61429a5a5c9742b146d0129`
  - `cmp -s` passed for both artifacts.
- Unit coverage now asserts the no-overlap ordered UTF8 output key retains the
  lower-hex sequence witness and still materializes to the same string values.

## Benchmarks

Builds used `rch exec --` with crate-scoped `fp-join` commands and isolated
target dirs. RCH worker preflight failed and the wrapper fell open locally.
`rch exec -- hyperfine` warns that hyperfine is not a compilation command, so
the benchmark itself ran locally after the crate-scoped `rch` builds.

Direct internal merge timer, 5M/5M ordered UTF8, 200 measured merge iterations:

- before: `mean_ms=6.402`, `p50_ms=5.974`, `p95_ms=11.498`, `p99_ms=12.901`
- after:  `mean_ms=0.001`, `p50_ms=0.001`, `p95_ms=0.004`, `p99_ms=0.009`

Paired hyperfine, same command:

- before: `1.715 s +/- 0.078` (user `0.740`, system `0.974`)
- after:  `296.4 ms +/- 6.1` (user `216.8 ms`, system `80.6 ms`)
- result: after ran `5.79x +/- 0.29` faster than before.

Score: Impact 5.0 x Confidence 5.0 / Effort 2.0 = 12.5. Keep.

## Validation

- `rustfmt --check crates/fp-join/src/lib.rs`
- `cargo check -p fp-join --all-targets`
- `cargo test -p fp-join`
- `cargo test -p fp-join ordered_utf8_contiguous_no_overlap_output_fast_path_jbyuc111111111`
- `cargo clippy -p fp-join --all-targets -- -D warnings`
- `ubs crates/fp-join/src/lib.rs` completed with pre-existing file-wide
  findings; sampled critical findings are dtype/sentinel equality false
  positives outside this hunk, and no finding targets the new helper or output
  branch.

`cargo fmt --check` still reports unrelated formatting drift in other existing
files, so the changed-file `rustfmt --check` result is the formatting gate for
this one-lever commit.

## Shifted residual

After-profile on the same 5M/5M command:

- `join_bench::add_numeric_checksum`: 51.71% self
- `__memmove_avx_unaligned_erms`: 13.00% self, now under
  `join_bench::build_ordered_utf8_frame`
- `join_bench::build_ordered_utf8_frame`: 2.61% self
- `Column::from_f64_values`: 2.04% self

Next pass should avoid the merge-output key materialization family and attack
the new top residual: benchmark checksum/setup masking, reusable ordered-UTF8
fixture/value-column construction, or a production reduction primitive if a
real user-facing profile confirms the checksum path.
