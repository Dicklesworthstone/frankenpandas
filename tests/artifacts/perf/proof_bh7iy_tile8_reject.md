# br-frankenpandas-bh7iy tile-8 Gram rejection

## Target

`df_corr` / `df_cov` complete-column Gram path remains profile-backed and hot
on current `main`:

```text
df_corr 1000000 1: 917.9 ms +/- 53.9 ms
df_cov  1000000 1: 962.9 ms +/- 27.4 ms
```

The current implementation is already the row-partitioned
communication-avoiding Gram path. This pass therefore tested a different
single lever inside that path: increase the shared `gram_partial_rows` tile
from 4x4 to 8x8 to expose more independent floating-point accumulator chains
without changing row-band order, per-cell row order, or using `mul_add`.

## Behavior proof

The candidate preserved every captured golden exactly:

```text
df_corr_5000 cmp=0
df_cov_5000 cmp=0
df_dot_16 cmp=0
df_spearman_5000 cmp=0
```

Candidate SHA-256:

```text
d7d4c83538939ca2d83ecc13d0f0376d90cd4f9176eaa9009a126e058231cb07  tests/artifacts/perf/bh7iy_after_tile8_golden_df_corr_5000.txt
f3766dadd4b2d36dda67aa9690a0fc3a17708fa8001fcee0c9ccb1895a452e89  tests/artifacts/perf/bh7iy_after_tile8_golden_df_cov_5000.txt
df012f6912112355591ece3b629096ade884bdfad37d5c6f8c2d18ceb766c3e4  tests/artifacts/perf/bh7iy_after_tile8_golden_df_dot_16.txt
dc37f75e1ee2e23e28ed89e0f178a0268cb65cc420bc82f97cb16d219eb7dd1e  tests/artifacts/perf/bh7iy_after_tile8_golden_df_spearman_5000.txt
```

Isomorphism:

- Each cell still folds rows in ascending order inside each fixed row band.
- Band partials still merge in ascending band order.
- Symmetry/finalization, `min_periods`, NaN fallback, and marginal swap rules
  are unchanged.
- No floating-point reassociation, `mul_add`, RNG, ordering, or tie behavior
  was introduced by the candidate.

## Paired benchmark

`hyperfine --warmup 1 --runs 5`, 1,000,000 rows, 64 columns:

| Scenario | Before mean | Candidate mean | Result |
| --- | ---: | ---: | ---: |
| `df_corr` | 978.5 ms +/- 44.2 ms | 994.5 ms +/- 32.6 ms | baseline 1.02x faster |
| `df_cov` | 943.9 ms +/- 25.5 ms | 940.1 ms +/- 50.3 ms | neutral, 1.00x |

Score < 2.0. Runtime source changes were removed; this commit keeps only the
baseline/candidate evidence and rejection proof.

## Route

Do not repeat Gram tile-width, row-partition traffic-reduction, or
rank-1/repack variants. The next profile-backed attack should switch to a
different primitive, preferably the large-output join fanout/materialization
hotspot already recorded adjacent to this bead (`outer_join` and `left_join`
on 500k-row fanout inputs), with a run-length/fanout output plan and exact
ordering/null-introduction proof.
