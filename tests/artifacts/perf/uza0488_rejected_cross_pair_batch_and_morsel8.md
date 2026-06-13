# br-frankenpandas-uza04.88 rejection evidence

Target: `DataFrame.corr(method="kendall")` no-tie complete-matrix path.

Baseline binary:
`/data/projects/.scratch/cargo-target-orangepeak-uza0488-base/release-perf/examples/perf_profile`

Baseline golden SHA256:
`031978ba431260b942dd36d9be055064a7453118a7c00888c35747644d33d99e`

## Lever 1: row-major rank slab plus 4-lane cross-pair Fenwick batching

Behavior proof:
- Focused matrix parity test passed:
  `cargo test -p fp-frame complete_kendall_parallel_matrix_matches_serial_ordered_ranks --lib`
- Golden output SHA256 matched baseline exactly:
  `031978ba431260b942dd36d9be055064a7453118a7c00888c35747644d33d99e`
- Ordering and tie semantics were unchanged: the candidate still used each column's
  existing no-tie order/rank witnesses, wrote the same symmetric `(i, j)` matrix
  cells, and computed each tau from the same integer discordant-pair count.

Benchmark result:
- `df_kendall 50000 3`: base 484.5 ms, candidate 539.9 ms. Rejected.
- `df_kendall 200000 1`: base 748.9 ms, candidate 994.7 ms. Rejected.

The batched Fenwick layout reduced outer `x_order` traversals but increased active
Fenwick working set and user CPU enough to regress.

## Lever 2: flat-pair morsel size 4 to 8

Behavior proof:
- Golden output SHA256 matched baseline exactly:
  `031978ba431260b942dd36d9be055064a7453118a7c00888c35747644d33d99e`
- Ordering and floating-point behavior were unchanged: only worker scheduling
  granularity changed; each pair still used the original ordered-rank Fenwick
  inversion kernel and wrote by deterministic matrix indices.

Benchmark result:
- `df_kendall 50000 3` forward: base 468.9 ms, morsel8 439.1 ms.
- `df_kendall 50000 3` reversed: morsel8 430.4 ms, base 433.7 ms.
- `df_kendall 200000 1`: base 702.1 ms, morsel8 692.6 ms.

The apparent 50k win did not survive reversed-order confirmation and the 200k
case was neutral. Score < 2.0, so the source change was removed.

Next target: a fundamentally different all-pairs Kendall primitive, not another
Fenwick scheduling/cache-layout micro-lever.
