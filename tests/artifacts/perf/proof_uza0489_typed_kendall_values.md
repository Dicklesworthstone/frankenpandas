# br-frankenpandas-uza04.89 proof: typed Float64 Kendall value extraction

## Lever

`Series::kendall_values_if_complete_finite` now consumes an all-valid typed
`Float64` slice directly with `as_f64_slice()` before falling back to the
existing `Scalar` materialization path. This removes the typed column ->
`Scalar` view -> `to_f64` walk before the complete/no-tie Kendall rank orders
are built.

This is one source lever. The separate `corr_parity_dump` edit is a clippy-only
example cleanup required for `fp-frame --all-targets`.

## Profile-backed target

Fresh baseline profile:
`tests/artifacts/perf/uza0489_base_perf_report_df_kendall_200000x1.txt`.

- `complete_kendall_no_tie_parallel_matrix` worker closure: 91.23% self/inline
  samples.
- `Scalar::to_f64`: 1.45% visible samples, with typed-scalar materialization
  also appearing through `ScalarValues::as_slice`/`OnceLock`.

The post-change `perf record` and `perf stat` retries were blocked by the
kernel changing to `perf_event_paranoid=4`; see
`uza0489_after_perf_report_df_kendall_200000x1.txt` and
`uza0489_after_perf_stat_df_kendall_200000x1.txt`. Hyperfine A/B remains the
accepted timing proof; residual routing stays on the all-pairs inversion wall.

## Builds

- Baseline build: `RUSTFLAGS="-C force-frame-pointers=yes"`
  `CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0489-base`
  `rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile`
  ran remotely on `vmi1153651`.
- After build: same command with
  `CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0489-after`
  ran remotely on `vmi1153651`.

## Golden SHA and equality

Baseline and after deterministic golden outputs are byte-identical:

```text
acf366c266b66f8497fb55734ed1b4ec40952a7c014f43db262c7c1e625e15e1  df_kendall_2000
031978ba431260b942dd36d9be055064a7453118a7c00888c35747644d33d99e  df_kendall_5000
f164fa86300fbea93e46aa99a5e0f7413fa8c0baf2ee49c6a689ed92652a4c3b  df_kendall_20000
cmp_2000=0
cmp_5000=0
cmp_20000=0
```

Artifacts:

- `uza0489_base_golden_generation.txt`
- `uza0489_after_golden_generation.txt`

## Hyperfine

`rch exec -- hyperfine` records non-compilation warnings and executes locally,
so these are same-host paired runs:

| Shape | Baseline | After | Ratio |
| --- | ---: | ---: | ---: |
| `df_kendall 50000 1` forward | 157.4 ms +/- 4.2 | 120.8 ms +/- 4.7 | 1.30x +/- 0.06 |
| `df_kendall 200000 1` forward | 648.0 ms +/- 17.3 | 509.9 ms +/- 13.8 | 1.27x +/- 0.05 |
| `df_kendall 200000 1` reversed | 644.6 ms +/- 23.2 | 516.6 ms +/- 15.0 | 1.25x +/- 0.06 |

Artifacts:

- `uza0489_pair_forward_hyperfine_df_kendall_50000x1.json`
- `uza0489_pair_forward_hyperfine_df_kendall_200000x1.json`
- `uza0489_pair_reversed_hyperfine_df_kendall_200000x1.json`
- `uza0489_pair_hyperfine.txt`

Score: Impact 3 * Confidence 4 / Effort 1 = 12.0. Keep.

## Isomorphism

- Ordering: unchanged. The extracted `Vec<f64>` preserves the typed slice order.
- Tie-breaking: unchanged. The same `kendall_no_tie_order` sort and epsilon tie
  gate consume the returned values.
- Floating point: unchanged. Values are copied from the same stored `f64` bits;
  no arithmetic, reassociation, or `mul_add` is introduced.
- NaN/inf/null: unchanged for accepted inputs. Typed slices are accepted only
  when length >= 2 and every value is finite; otherwise the function returns
  `None`, matching the old complete-finite gate. Nullable/missing/non-Float64
  columns still use the existing `Scalar` fallback.
- Output matrix: unchanged. Diagonal, symmetry, upper-triangle fill order, and
  fallback paths in `pairwise_rank_corr` are untouched.
- RNG: not used.

Focused unit proof:
`uza0489_after_focused_kendall_tests.txt` ran 6 Kendall tests remotely on
`vmi1153651`, including
`kendall_values_typed_f64_matches_scalar_path_and_rejects_non_finite`.

## Gates

- `rch exec -- cargo check -p fp-frame --all-targets`: pass on rerun
  (`uza0489_cargo_check_fp_frame_rerun.txt`).
- `rch exec -- cargo clippy -p fp-frame --all-targets -- -D warnings`: pass
  (`uza0489_cargo_clippy_fp_frame.txt`).
- `cargo fmt --all --check`: fails on unrelated pre-existing formatting drift
  in `fp-conformance` examples (`uza0489_cargo_fmt_check.txt`).
- `rustfmt --check crates/fp-frame/src/lib.rs crates/fp-frame/examples/corr_parity_dump.rs`:
  fails on unrelated pre-existing formatting regions in the giant `lib.rs`; the
  touched Kendall hunk and example hunk are not in the rustfmt diff
  (`uza0489_rustfmt_touched_files.txt`).
- `ubs crates/fp-frame/src/lib.rs crates/fp-frame/examples/corr_parity_dump.rs`:
  attempted twice; clean rerun timed out with `UBS_STATUS=124`
  (`uza0489_ubs_changed_files_rerun.txt`).

## Residual route

The after timing still leaves `complete_kendall_no_tie_parallel_matrix` as the
dominant wall. Do not repeat the rejected row-major multi-Fenwick batching,
morsel-size scheduling, merge-sort, sqrt/block counter, per-pair validation
removal, or buffer/cache-layout families. Next primitive should be a genuinely
different exact all-pairs Kendall algorithm: divide-and-conquer/offline
dominance, wavelet/rank-select static witnesses, or a proof-carrying
rank-signature primitive that shares work across the 32 columns.
