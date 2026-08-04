# br-frankenpandas-uza04.124 - df_dot worker-private Float64 chunks

LavenderStone, 2026-06-14.

## Profile-backed target

`DataFrame::dot` for `df_dot 100000 6` remained on the row-band
materialization/output boundary after the prior dot sequence:

- `proof_uza0498_comm_avoiding_dot_gemm.md`: eliminated the A transpose and
  switched to row-banded GEMM.
- `proof_uza04100_dot_parallel_assembly.md`: removed the zero-filled final
  output buffer and parallelized per-column materialization.
- `proof_uza04116_direct_dot_output_reject.md`: direct shared-column row-slice
  writes regressed; private row-band buffers were locality-positive.
- `proof_lavender_df_dot_worker_chunks.md`: shared row-band tape plus lazy
  repeated-slice columns regressed to about `2.7s`; next route was
  worker-private final-column chunk adoption, not another lazy wrapper.

Mapped graveyard primitive: vectorized/morsel execution and
communication-avoiding dense kernels. This lever keeps the existing private
row-band GEMM and moves the output boundary to zero-copy all-valid Float64
chunk adoption.

## Baseline

- Build: `CARGO_TARGET_DIR=.rch-target-lavenderstone-panel-base rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile`
- Baseline hyperfine, `df_dot 100000 6`: `1.060 s +/- 0.090 s`.
- Paired forward baseline: `959.5 ms +/- 18.1 ms`.
- Paired reversed baseline: `964.8 ms +/- 17.4 ms`.
- Golden SHA:
  - `df_dot 2000`: `ddbde1c39c4856c19700fe90a29f6acce2a742a98a298585f896b0b02cdbb535`
  - `df_dot 5000`: `04af7c2bb0e772d23ed69b3733da0778c3693ba1e67557c0126fcbd4458fdb3d`

## Rejected sub-lever

Before the chunked route, I tested a narrower per-band NaN witness that kept the
final `Vec<f64>` copy but skipped `Column::from_f64_values`' no-NaN scan for
all-valid dot columns.

- Golden SHA matched exactly.
- Paired forward: baseline `949.3 ms +/- 12.9 ms`, candidate `948.4 ms +/- 17.3 ms`.
- Paired reversed: candidate `963.3 ms +/- 29.3 ms`, baseline `952.2 ms +/- 21.6 ms`.
- Verdict: reject. It was behavior-clean but effectively flat/noisy and below
  Score 2.0. Source was reverted before the kept lever.

## Kept lever

Add `Column::from_f64_all_valid_chunks`, backed by private
`ScalarValues::LazyAllValidFloat64Chunks`, and change `DataFrame::dot` output
assembly so each all-valid output column adopts Arc-backed views into the
existing private row-band buffers:

- Each worker still computes the same row band with the same 4x4 micro-kernel.
- Each cell still folds `l = 0..k` in ascending order with `acc += av * bp[dj]`.
- Band values remain column-major within band: `band[j * bw + local_row]`.
- Output columns store chunks in ascending `i0` band order, preserving row order.
- A per-band per-column NaN witness keeps null semantics exact. If any output
  chunk for a column contains NaN, that column falls back to the old
  concat-then-`Column::from_f64_values` path.

## Isomorphism proof

- Ordering preserved: yes. Band sort by `i0` is unchanged; chunk order is the
  old concat order.
- Tie-breaking unchanged: N/A. `dot` does not sort, hash, or break ties.
- Floating-point preserved: yes. The arithmetic kernel and accumulation order
  are unchanged; only the storage boundary changes after each accumulator is
  complete.
- RNG unchanged: N/A.
- Golden outputs: `sha256sum -c lavender_df_dot_panel_chunks_golden.sha256`
  passed, and `cmp -s` passed against both baseline golden files.

## Benchmark gate

- Paired forward:
  - baseline: `959.5 ms +/- 18.1 ms`
  - chunked: `749.7 ms +/- 7.7 ms`
  - speedup: `1.28x +/- 0.03`
- Paired reversed:
  - chunked: `740.8 ms +/- 15.3 ms`
  - baseline: `964.8 ms +/- 17.4 ms`
  - speedup: `1.30x +/- 0.04`
- Score: Impact 4 x Confidence 4 / Effort 3 = 5.33. Keep.

## Validation

- `rch exec -- cargo test -p fp-columnar from_f64_all_valid_chunks_materializes_in_chunk_order --lib`: passed.
- `rch exec -- cargo test -p fp-frame dot --lib -- --nocapture`: 12 passed.
- `rch exec -- cargo check -p fp-columnar --all-targets`: passed.
- `rch exec -- cargo check -p fp-frame --lib`: passed.
- `rch exec -- cargo clippy -p fp-columnar --lib --tests -- -D warnings`: passed.
- `rch exec -- cargo clippy -p fp-frame --lib -- -D warnings`: passed.
- `cargo fmt -p fp-columnar -- --check`: passed.
- `cargo fmt -p fp-frame -- --check`: failed on pre-existing reindex formatting
  drift outside this diff; the drift hunk was intentionally not included.
- `ubs crates/fp-columnar/src/lib.rs crates/fp-frame/src/lib.rs`: timed out and
  was terminated; status recorded as `124`.

## Residual / next route

The kept lever cuts system time sharply by avoiding final per-column output
copies for all-valid dot output. The residual is now the row-band GEMM compute
plus chunk metadata/scalar materialization for downstream consumers. Next
profile-backed route: either add a cached contiguous expansion for chunked
Float64 columns when a downstream kernel requires `as_f64_slice()`, or attack
the 4x4 FMA-free micro-kernel with a larger safe-Rust register-blocked panel
only if the profile/timing still points at compute.
