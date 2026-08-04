# br-frankenpandas-uza04.90 - word-blocked Kendall rank bitset proof

## Change

One lever was kept in `crates/fp-frame/src/lib.rs`: complete no-tie Kendall inversion counting now tries an exact word-blocked dynamic rank bitset before falling back to the existing Fenwick implementation.

The new path stores one `u64` bit per observed `y` rank and a Fenwick tree over 64-rank word counts. For each row in `x_order`, it computes the number of already-seen ranks `<= y_rank` as:

- prefix count of full lower 64-rank words
- plus `popcount` of the current word masked through the queried bit

The discordance contribution remains `seen - rank_or_lower`, identical to the previous ordered-rank inversion contract.

## Baseline/profile

Baseline was rebuilt before editing with:

```bash
RUSTFLAGS='-C force-frame-pointers=yes' CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0490-base rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
```

RCH worker: `vmi1227854`, exit 0, artifact `tests/artifacts/perf/uza0490_base_build_perf_profile.txt`.

Baseline artifacts:

- `tests/artifacts/perf/proof_uza0490_pass1_baseline_profile_witness.md`
- `tests/artifacts/perf/uza0490_base_hyperfine_df_kendall_50000x1.json`
- `tests/artifacts/perf/uza0490_base_hyperfine_df_kendall_200000x1.json`
- `tests/artifacts/perf/uza0490_base_perf_record_df_kendall_200000x1.txt`
- `tests/artifacts/perf/uza0490_base_perf_stat_df_kendall_200000x1.txt`
- `tests/artifacts/perf/uza0490_base_time_v_df_kendall_200000x1.txt`

`perf record` and `perf stat` were blocked by `perf_event_paranoid=4`; the block transcripts were retained and `/usr/bin/time -v` confirmed the residual remains CPU-bound in the Kendall path.

## Golden outputs

Baseline and after binaries emitted byte-identical `df_kendall` outputs.

| Scenario | Golden SHA256 |
| --- | --- |
| `df_kendall 2000` | `acf366c266b66f8497fb55734ed1b4ec40952a7c014f43db262c7c1e625e15e1` |
| `df_kendall 5000` | `031978ba431260b942dd36d9be055064a7453118a7c00888c35747644d33d99e` |
| `df_kendall 20000` | `f164fa86300fbea93e46aa99a5e0f7413fa8c0baf2ee49c6a689ed92652a4c3b` |

Comparison artifact: `tests/artifacts/perf/uza0490_after_golden_cmp.txt`; all `cmp` statuses were `0`.

## Benchmarks

After binary was rebuilt with:

```bash
RUSTFLAGS='-C force-frame-pointers=yes' CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0490-after rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
```

RCH worker: `vmi1227854`, exit 0, artifact `tests/artifacts/perf/uza0490_after_build_perf_profile.txt`.

Paired forward order:

| Scenario | Baseline mean | After mean | Ratio |
| --- | ---: | ---: | ---: |
| `df_kendall 50000 1` | `123.868 ms` | `118.004 ms` | `1.05x` |
| `df_kendall 200000 1` | `531.936 ms` | `501.401 ms` | `1.06x` |

Paired reversed order:

| Scenario | Baseline mean | After mean | Ratio |
| --- | ---: | ---: | ---: |
| `df_kendall 50000 1` | `124.552 ms` | `119.076 ms` | `1.05x` |
| `df_kendall 200000 1` | `538.402 ms` | `492.466 ms` | `1.09x` |

Benchmark artifacts:

- `tests/artifacts/perf/uza0490_pair_forward_hyperfine_df_kendall_50000x1.json`
- `tests/artifacts/perf/uza0490_pair_forward_hyperfine_df_kendall_200000x1.json`
- `tests/artifacts/perf/uza0490_pair_reversed_hyperfine_df_kendall_50000x1.json`
- `tests/artifacts/perf/uza0490_pair_reversed_hyperfine_df_kendall_200000x1.json`

The original pass-2 target was more aggressive, but both command orders and both input sizes showed a consistent positive wall-time delta with unchanged goldens.

## Isomorphism proof

- Ordering preserved: yes. The caller still iterates the same `x_order` and uses the same `y_rank_by_row` witness.
- Tie-breaking unchanged: yes. The complete no-tie fast path is still gated by the same rank witnesses; duplicate bit detection returns `None`, which falls back to the previous Fenwick implementation.
- Floating-point unchanged: yes. The change only computes the integer discordance count. The tau formula and `f64` write path are unchanged. Golden output bytes are identical.
- RNG unchanged: N/A. The runtime path has no RNG. The test-only shuffled boundary cases are deterministic.
- NaN/null behavior unchanged: yes. Complete-finite admission and fallback behavior are outside this helper and were not changed.
- Diagonal/symmetry unchanged: yes. Matrix assembly remains unchanged; only the per-pair inversion counter changed.

## Validation

- Focused new boundary test: `tests/artifacts/perf/uza0490_after_focused_word_block_test.txt`, passed.
- Focused Kendall test set: `tests/artifacts/perf/uza0490_after_focused_kendall_tests.txt`, 7 passed.
- Crate-scoped check: `tests/artifacts/perf/uza0490_after_cargo_check_fp_frame.txt`, passed.
- Crate-scoped clippy: `tests/artifacts/perf/uza0490_after_cargo_clippy_fp_frame.txt`, passed.
- `rustfmt --edition 2024 --check crates/fp-frame/src/lib.rs`: still fails due broad pre-existing formatting drift outside this Kendall hunk; rerun artifact `tests/artifacts/perf/uza0490_rustfmt_fp_frame_check_rerun.txt`.
- `ubs crates/fp-frame/src/lib.rs`: timed out before findings on the very large file; startup artifact `tests/artifacts/perf/uza0490_ubs_fp_frame.txt`.

## Score

Impact `2` x Confidence `5` / Effort `2` = `5.0`.

Verdict: keep. The lever is proof-clean, algorithmically different from the rejected families, and consistently improves the residual complete no-tie Kendall path. Residual work remains in the all-pairs Kendall primitive and should move to a deeper cross-column rank-sharing design rather than more single-pair counter micro-tuning.
