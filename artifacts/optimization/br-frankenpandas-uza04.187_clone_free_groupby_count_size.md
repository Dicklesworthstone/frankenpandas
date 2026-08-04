# br-frankenpandas-uza04.187 clone-free generic groupby count/size

## Attempt

- Agent: cod-a via Agent Mail identity GrayStone
- Date: 2026-06-18
- Bead: br-frankenpandas-uza04.187
- Lever: route generic `groupby_count` / `groupby_size` through a counter-only map before the value-vector fallback.
- Baseline comparator: prior `groupby_agg` generic path for non-Int64 keys, which hashes the same `GroupKeyRef` but also clones every non-missing `Scalar` into per-group `Vec<Scalar>` before count/size.

## Negative-Evidence Ledger

| Candidate | Source | Verdict | Retry Predicate |
|---|---|---|---|
| Float64 value_counts open-address residual | `.skill-loop-progress.md` br-frankenpandas-0mtyz | Rejected: forward-only movement did not survive reversed paired hyperfine; no source retained. | Retry only if a later profile again shows nullable Float64 value_counts as the top residual and the design changes the probe/storage family, not another small open-address variant. |
| Dense right-lane serial replay for inner join | `artifacts/optimization/skill-loop-progress-orangepeak-uza04-38.md` | Rejected: preserved goldens but regressed dense inner join. | Retry only if join output assembly profile changes away from right-lane materialization/memmove and a new proof isolates a different primitive. |
| Clone-free generic count/size counters | This bead | Pending batch benchmark: code-first campaign allowed `cargo check -p fp-groupby` only. | Keep only if `groupby-bench --agg count --key-kind utf8` and `--agg size --key-kind utf8` beat the pre-change vector-cloning path under paired same-host runs without conformance regressions. |

## Isomorphism

- Group identity: unchanged. The helper uses the same `GroupKeyRef::from_scalar` keys as the generic fallback.
- Ordering: unchanged. First-seen order comes from the same `ordering` vector; sorted order uses the same `sort_group_ordering_by` comparator.
- Output labels: unchanged. Labels are reconstructed from the first source row, matching the generic fallback.
- Count semantics: unchanged. `count` increments only for non-missing values; `size` increments for every retained row.
- Null keys: unchanged. `dropna=true` skips missing keys before insertion, exactly like the fallback.
- Floating point: N/A; output values are integer counters.
- RNG: N/A.

## Bench Guard

`crates/fp-groupby/src/bin/groupby-bench.rs` now accepts `--key-kind utf8`, so the realistic string-key count/size path can be measured directly:

```bash
CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a cargo run -p fp-groupby --bin groupby-bench -- --agg count --key-kind utf8 --rows 500000 --key-cardinality 512 --iters 25
CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a cargo run -p fp-groupby --bin groupby-bench -- --agg size --key-kind utf8 --rows 500000 --key-cardinality 512 --iters 25
```

Batch-test status: pending by instruction; only `cargo check -p fp-groupby` was run in this code-first commit.
