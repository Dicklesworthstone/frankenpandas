# br-frankenpandas-uza04.189 clone-free generic groupby mean

## Attempt

- Agent: cod-a via Agent Mail identity GrayStone
- Date: 2026-06-18
- Bead: br-frankenpandas-uza04.189
- Lever: route generic-key `groupby_mean` through a streaming numeric sum/count map before the per-group value-vector fallback.
- Baseline comparator: prior `groupby_agg(Mean)` generic path for non-Int64 keys, which hashes the same `GroupKeyRef` but clones every non-missing `Scalar` into per-group `Vec<Scalar>` before `nanmean`.
- Graveyard mapping: Swiss-table/hash hot path remains unchanged; this applies the lower-risk vectorized-execution principle of reducing the per-row payload to a cache-local accumulator instead of materializing row objects.

## Negative-Evidence Ledger

| Candidate | Source | Verdict | Retry Predicate |
|---|---|---|---|
| Float64 value_counts open-address residual | `.skill-loop-progress.md` br-frankenpandas-0mtyz | Rejected: forward-only movement did not survive reversed paired hyperfine; no source retained. | Retry only with a different storage/probe family and fresh profile proof naming nullable Float64 value_counts as the top residual. |
| Wide bool-mask verifier | memory June 11 filter_bool ledger | Rejected: 64-bool raw-slice verifier preserved behavior but lost on paired/reversed hyperfine. | Retry only as a producer-carried witness or bitpacked mask primitive, not another raw block-size verifier. |
| Borrowed join output metadata shortcut | memory June 9 ordered UTF8 join ledger | Rejected: setup-free proof kept semantics but the pushed result was rejection evidence; next route was deeper output assembly. | Retry only if a new split gate isolates output assembly rather than metadata borrowing. |
| Clone-free generic count/size counters | br-frankenpandas-uza04.187 | Pending batch benchmark: code-first campaign allowed `cargo check -p fp-groupby` only. | Keep only if same-host `--agg count/size --key-kind utf8` beats the vector-cloning path without conformance regressions. |
| Clone-free generic mean counters | This bead | Pending batch benchmark: code-first campaign allowed `cargo check -p fp-groupby` only. | Keep only if `groupby-bench --agg mean --key-kind utf8` beats the pre-change vector-cloning path under paired same-host runs without conformance regressions. |

## Isomorphism

- Group identity: unchanged. The helper uses the same `GroupKeyRef::from_scalar` keys as the generic fallback.
- Ordering: unchanged. First-seen order comes from the same `ordering` vector; sorted order uses the same `sort_group_ordering_by` comparator.
- Output labels: unchanged. Labels are reconstructed from the first source row, matching the fallback.
- Numeric mean semantics: unchanged for accepted inputs. The helper performs the same left-to-right `to_f64()` sum and count as `nanmean`.
- Null handling: unchanged. Missing values are skipped for the numerator and denominator; all-missing groups emit `Null(NaN)`.
- Timedelta and non-numeric values: unchanged by fallback. Any non-missing value outside the numeric `to_f64()` fold returns `None` and routes to the existing vector path.
- Floating point: unchanged for accepted inputs; same values, same order, same `+`, same final division.
- RNG: N/A.

## Bench Guard

`crates/fp-groupby/src/bin/groupby-bench.rs` already exposes the realistic string-key route:

```bash
CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a cargo run -p fp-groupby --bin groupby-bench -- --agg mean --key-kind utf8 --rows 500000 --key-cardinality 512 --iters 25
```

Batch-test status: pending by instruction; only `cargo check -p fp-groupby` was run in this code-first commit.
