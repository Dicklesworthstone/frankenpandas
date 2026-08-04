# br-frankenpandas-uza04.120 rejection proof: df_dot shared output tape

## Target

- Hotspot: `DataFrame::dot` for `df_dot 100000 6`, building `A(100000 x 256) * B(256 x 256)`.
- Prior evidence:
  - `proof_uza0498_comm_avoiding_dot_gemm.md`: row-banded GEMM replaced the older transposed-A route.
  - `proof_uza04100_dot_parallel_assembly.md`: output assembly remained a large residual after row-banded compute.
  - `proof_uza04116_direct_dot_output_reject.md`: direct final-column writes regressed; the next candidate was a worker-private/shared output primitive.
- Candidate primitive: flatten row-band outputs into one shared Float64 tape and build each output column as lazy repeated slices over that tape.

## Candidate Isomorphism

- Ordering: preserved. Columns materialized by sorted row-band order, then each band's column-j slice in ascending local row order.
- Tie-breaking: not applicable.
- Floating point: preserved. GEMM computation and per-cell accumulation order were unchanged; only the storage boundary changed.
- RNG: not applicable.
- Golden output:
  - Base `df_dot 2000`: `ddbde1c39c4856c19700fe90a29f6acce2a742a98a298585f896b0b02cdbb535`
  - After `df_dot 2000`: `ddbde1c39c4856c19700fe90a29f6acce2a742a98a298585f896b0b02cdbb535`
  - Base `df_dot 5000`: `04af7c2bb0e772d23ed69b3733da0778c3693ba1e67557c0126fcbd4458fdb3d`
  - After `df_dot 5000`: `04af7c2bb0e772d23ed69b3733da0778c3693ba1e67557c0126fcbd4458fdb3d`
  - `cmp -s` passed for both baseline-vs-after golden files.

## Benchmark Gate

- Initial baseline artifact: `lavender_df_dot_base_hyperfine_100000x6.txt`
  - `df_dot 100000 6`: `1.317 s +/- 0.243 s`
- Paired forward artifact: `lavender_df_dot_pair_hyperfine_forward.txt`
  - baseline: `985.2 ms +/- 20.3 ms`
  - candidate: `2.715 s +/- 0.035 s`
  - baseline ran `2.76x` faster.
- Paired reversed artifact: `lavender_df_dot_pair_hyperfine_reversed.txt`
  - candidate: `2.684 s +/- 0.028 s`
  - baseline: `990.5 ms +/- 17.2 ms`
  - baseline ran `2.71x` faster.
- Verdict: reject. Score is below 2.0 because impact is negative.

## Validation

- `rch exec -- cargo check -p fp-frame --lib`: passed after fixing the candidate-only dead variant warning.
- Candidate release-perf build ran under the RCH wrapper, but RCH failed open locally once due saturated/no-admissible workers; timing proof uses paired local hyperfine binaries and records that limitation.
- No source code from the candidate was retained.

## Next Route

Do not repeat lazy per-column repeated-slice wrappers for `df_dot`. The regression shows that moving the boundary into lazy column materialization loses against the current parallel eager output assembly. The next route should attack a deeper primitive:

- eliminate or fuse the row-band tape copy itself,
- compute directly into column-major worker-private chunks without shared row-slice wrappers,
- or replace the fixed row-band GEMM with a blocked/recursive column-panel kernel that emits final column chunks in output order.
