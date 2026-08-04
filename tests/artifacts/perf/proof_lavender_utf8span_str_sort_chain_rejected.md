# br-frankenpandas-uza04.130 rejection proof

Target: `str_sort_chain 100000 10`

Baseline binary: `.rch-target-lavenderstone-current/release-perf/examples/perf_profile`
Candidate binary: `.rch-target-lavenderstone-utf8span/release-perf/examples/perf_profile`

Candidate lever: route all-valid Utf8 sorting through borrowed byte spans and
`utf8_msd_argsort_bytes` instead of rebuilding `&str` views before dispatching
to the existing stable MSD byte-radix sorter.

Behavior proof:
- `perf_profile golden str_sort_chain 2000` matched byte-for-byte.
- `perf_profile golden str_sort_chain 5000` matched byte-for-byte.
- Golden sha256:
  - `2000`: `9113cb95ca166c8e6143b113dfc9f495519722cb91320f4c4170c2c59ca97231`
  - `5000`: `637c2c8b0680315b5af8f21c7a55b907f686fe8300a77059adbe5e3f924bbc23`
- Ordering/tie semantics stayed delegated to the same stable MSD byte-radix
  implementation; no floating-point arithmetic, RNG, null, or NaN behavior was
  touched.

Bench gate:
- Baseline internal: `14.245 ms/iter`.
- Candidate internal: `15.392 ms/iter`.
- Paired hyperfine baseline: `142.3 ms +/- 10.7 ms` for 10 iterations.
- Paired hyperfine candidate: `161.7 ms +/- 26.7 ms` for 10 iterations.
- Hyperfine summary: baseline ran `1.14x +/- 0.21x` faster.

Verdict: reject. Source hunk removed. Do not retry byte-span bridge routing for
this residual without a deeper sort primitive.
