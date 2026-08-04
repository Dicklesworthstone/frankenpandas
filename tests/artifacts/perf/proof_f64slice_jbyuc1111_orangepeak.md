# br-frankenpandas-jbyuc.1.1.1.1 proof - Float64 contiguous slice view

Target:
- `str_inner_join 1000000` after `LazyUtf8Slice`.
- Post-view profile source: `tests/artifacts/perf/post_utf8slice_reprofile_jbyuc111.txt`.
- Residual: `Column::take_positions` 40.57% self in ordered UTF8 inner-join output.

Worker and commands:
- Worker: `vmi1227854` (`ubuntu@109.123.245.77`).
- Build: `rch exec -j -- cargo build -p fp-conformance --profile release-perf --example perf_profile`.
- Benchmark binary on worker:
  `/data/projects/.scratch/frankenpandas-orangepeak-20260608-f64slice/.rch-target-vmi1227854-job-29879359665864990-1780955020919971158-0/release-perf/examples/perf_profile`.
- Hyperfine command:
  `hyperfine --warmup 3 --runs 10 '<bin> str_inner_join 1000000 500'`.

Golden output:
- Before SHA: `76d2f388645ed3f3578017c5e2d919fa809e4230793a86d54d5ca93a6d0bc10e`.
- After SHA: `76d2f388645ed3f3578017c5e2d919fa809e4230793a86d54d5ca93a6d0bc10e`.
- `cmp` before vs after exit code: 0.

Timing:
- Direct harness before: `1000 iters in 0.495s (0.495 ms/iter)`.
- Direct harness after: `1000 iters in 0.281s (0.281 ms/iter)`.
- Hyperfine before: mean `258.918971 ms`, stddev `20.121937 ms`, min `234.294511 ms`.
- Hyperfine after: mean `195.209532 ms`, stddev `27.248300 ms`, min `160.162568 ms`.
- Hyperfine mean speedup: `1.33x`.
- Direct harness speedup: `1.76x`.

Isomorphism proof:
- Ordering: the new branch only fires when `positions[i] == positions[0] + i`; output row `i` reads the same source row as the copied gather.
- Tie and suffix behavior: no join planning, column ordering, key matching, or suffix/name code changed; only the internal representation returned by `Column::take_positions` changes.
- Null and NaN behavior: branch requires `self.validity.all()` and `DType::Float64`. `Column::from_f64_values` marks NaN invalid, so `as_f64_slice` excludes NaN-bearing columns. Nullable columns keep the existing gather path.
- Floating-point bits: the view borrows the exact `Arc<[f64]>` slots; no arithmetic, cast, normalization, or copy rewrite occurs. Tests assert `to_bits()` for `-0.0`, infinity, and a subnormal value.
- RNG: none.

Score:
- Impact 3 x Confidence 4 / Effort 2 = 6.0.
- Keep threshold `>= 2.0` cleared.
