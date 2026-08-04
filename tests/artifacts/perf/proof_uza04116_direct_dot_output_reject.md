# br-frankenpandas-uza04.116 - direct df_dot output assembly - REJECTED

LavenderStone, 2026-06-13.

## Profile-backed target

Existing kept artifacts identify `df_dot` as the residual after the safe-Rust
GEMM work:

- `proof_uza0498_comm_avoiding_dot_gemm.md`: removed the A transpose and kept
  `df_dot 100000` `572.056 -> 224.882 ms/iter` on RCH worker `vmi1227854`.
- `proof_uza04100_dot_parallel_assembly.md`: phase timers showed
  `extract ~0.4ms`, `compute ~55ms`, `assemble ~95ms`, `result_cols ~110ms`,
  and kept `231.576 -> 134.998 ms/iter`.
- Residual called out there: row-band intermediate plus output allocation churn,
  roughly one extra 205MB row-band buffer for the `100000 x 256` output.

## Baseline

- RCH build: `rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile`
  completed remotely on `vmi1227854` in `272.8s`.
- RCH cargo-run timing fell open locally due
  `no admissible workers: insufficient_slots=2,hard_preflight=10`; retained as
  a same-machine timing baseline.
- Local hyperfine, binary `/data/tmp/cargo-target/release-perf/examples/perf_profile`,
  command `df_dot 100000 6`: `927.5 ms +/- 24.0 ms` for 6 iterations
  (`154.6 ms/iter`).
- Golden sha256:
  - `df_dot 2000`: `ddbde1c39c4856c19700fe90a29f6acce2a742a98a298585f896b0b02cdbb535`
  - `df_dot 5000`: `04af7c2bb0e772d23ed69b3733da0778c3693ba1e67557c0126fcbd4458fdb3d`

## Lever Tested

Replace the per-worker row-band `Vec<f64>` intermediate with direct writes into
disjoint mutable row slices of the final output columns.

Isomorphism argument:

- Each worker still owns the same global row band.
- Each `C[i][j]` still starts at `0.0` and folds `l = 0..k` in ascending order
  with the same separate `acc += a * b` operation.
- Output row order and `other.column_order` are unchanged.
- No RNG, hashing, sorting, tie-breaking, or error behavior is involved.

## Verification

- `cargo fmt -p fp-frame --check` after reverting source still reports
  pre-existing committed rustfmt drift in `fp-frame/src/lib.rs`; there is no
  remaining source diff for this rejected lever.
- RCH check for the candidate source before reverting:
  `rch exec -- cargo check -p fp-frame --lib`, remote `vmi1153651`, passed.
- Candidate goldens matched baseline byte-for-byte:
  - `df_dot 2000`: `ddbde1c39c4856c19700fe90a29f6acce2a742a98a298585f896b0b02cdbb535`
  - `df_dot 5000`: `04af7c2bb0e772d23ed69b3733da0778c3693ba1e67557c0126fcbd4458fdb3d`

## Result

- Remote candidate smoke timing on `vmi1227854`, `df_dot 100000 1`:
  `516.198 ms/iter` (not used as before/after proof because the baseline
  timing path fell open locally).
- Same-machine local hyperfine after, `df_dot 100000 6`:
  `991.8 ms +/- 20.0 ms`.
- Local before/after: `927.5 -> 991.8 ms` for 6 iterations, `0.94x`.

Score: Impact `0` (regression) x Confidence `0.9` / Effort `1.0` = `0`.
Decision: reject and revert source. Evidence artifacts retained.

## Next Primitive

Do not retry direct shared-column row-slice writes. The regression strongly
suggests the private row-band buffer was buying locality while the direct path
paid main-thread zero-fill / first-touch and shared-column write costs.

Next attack: worker-private output chunk adoption, not another loop tweak. Make
the output `Column` able to adopt worker-private Float64 row chunks (or an
equivalent reusable output arena) so the GEMM keeps private row-band locality
without the final concat/copy and without main-thread first-touching the whole
final output.
