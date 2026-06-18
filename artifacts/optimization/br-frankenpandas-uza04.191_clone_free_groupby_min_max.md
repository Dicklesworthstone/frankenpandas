# br-frankenpandas-uza04.191 clone-free generic groupby min/max

## Attempt

- Agent: cod-a via Agent Mail identity GrayStone
- Date: 2026-06-18
- Bead: br-frankenpandas-uza04.191
- Lever: route generic-key `groupby_min` and `groupby_max` through a streaming scalar-slot map before the per-group value-vector fallback.
- Baseline comparator: prior `groupby_agg(Min|Max)` generic path for non-Int64 keys, which hashes the same `GroupKeyRef` but clones every non-missing `Scalar` into per-group `Vec<Scalar>` before `nanmin`/`nanmax`.
- Graveyard mapping: Swiss-table/hash hot path remains unchanged; this applies vectorized-execution and cache-local accumulator pressure reduction by keeping one current extremum per group instead of materializing row objects.

## Negative-Evidence Ledger

| Candidate | Source | Verdict | Retry Predicate |
|---|---|---|---|
| Float64 value_counts open-address residual | `.skill-loop-progress.md` br-frankenpandas-0mtyz | Rejected: forward-only movement did not survive reversed paired hyperfine; no source retained. | Retry only with a different storage/probe family and fresh profile proof naming nullable Float64 value_counts as the top residual. |
| Wide bool-mask verifier | memory June 11 filter_bool ledger | Rejected: 64-bool raw-slice verifier preserved behavior but lost on paired/reversed hyperfine. | Retry only as a producer-carried witness or bitpacked mask primitive, not another raw block-size verifier. |
| Borrowed join output metadata shortcut | memory June 9 ordered UTF8 join ledger | Rejected: setup-free proof kept semantics but the pushed result was rejection evidence; next route was deeper output assembly. | Retry only if a new split gate isolates output assembly rather than metadata borrowing. |
| Clone-free generic count/size counters | br-frankenpandas-uza04.187 | Pending batch benchmark: code-first campaign allowed `cargo check -p fp-groupby` only. | Keep only if same-host `--agg count/size --key-kind utf8` beats the vector-cloning path without conformance regressions. |
| Clone-free generic mean counters | br-frankenpandas-uza04.189 | Pending batch benchmark: code-first campaign allowed `cargo check -p fp-groupby` only. | Keep only if same-host `--agg mean --key-kind utf8` beats the vector-cloning path without conformance regressions. |
| Clone-free generic min/max scalar slots | This bead | Pending batch benchmark: code-first campaign allowed `cargo check -p fp-groupby` only. | Keep only if paired `groupby-bench --agg min/max --key-kind utf8` beats the pre-change vector-cloning path without conformance regressions. |

## Isomorphism

- Group identity: unchanged. The helper uses the same `GroupKeyRef::from_scalar` keys as the generic fallback.
- Ordering: unchanged. First-seen order comes from the same `ordering` vector; sorted order uses the same `sort_group_ordering_by` comparator.
- Output labels: unchanged. Labels are reconstructed from the first source row, matching the fallback.
- Null handling: unchanged. Missing values are skipped; all-missing groups emit `Null(NaN)`.
- Same-type extrema: unchanged. Int64, Float64, Utf8, Bool, and Timedelta64 values use the same native comparisons as `fp_types::nanmin`/`nanmax`.
- Mixed numeric extrema: unchanged. Mixed comparable scalars use the same pairwise `to_f64()` comparison, selecting the same source scalar when it wins.
- Incomparable mixed values: unchanged. The first incomparable pair records the group as invalid and emits `Null(NaN)`, matching `nanmin`/`nanmax` early return.
- RNG: N/A.

## Bench Guard

`crates/fp-groupby/src/bin/groupby-bench.rs` exposes the realistic string-key route:

```bash
CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a cargo run -p fp-groupby --bin groupby-bench -- --agg min --key-kind utf8 --rows 500000 --key-cardinality 512 --iters 25
CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a cargo run -p fp-groupby --bin groupby-bench -- --agg max --key-kind utf8 --rows 500000 --key-cardinality 512 --iters 25
```

Batch-test status: pending by instruction; only `cargo check -p fp-groupby` was run in this code-first commit.
