# br-frankenpandas-uza04.192 clone-free generic groupby first/last

## Attempt

- Agent: cod-a via Agent Mail identity GrayStone
- Date: 2026-06-18
- Bead: br-frankenpandas-uza04.192
- Lever: route generic-key `groupby_first` and `groupby_last` through a streaming scalar-slot map before the per-group value-vector fallback.
- Baseline comparator: prior `groupby_agg(First|Last)` generic path for non-Int64 keys, which hashes the same `GroupKeyRef` but clones every non-missing `Scalar` into per-group `Vec<Scalar>` before selecting the first or last element.
- Graveyard mapping: vectorized execution and cache-aware layout principle; keep only the scalar state the reduction needs instead of building per-group row-object vectors.

## Negative-Evidence Ledger

| Candidate | Source | Verdict | Retry Predicate |
|---|---|---|---|
| Float64 value_counts open-address residual | `.skill-loop-progress.md` br-frankenpandas-0mtyz | Rejected: forward-only movement did not survive reversed paired hyperfine; no source retained. | Retry only with a different storage/probe family and fresh profile proof naming nullable Float64 value_counts as the top residual. |
| Wide bool-mask verifier | memory June 11 filter_bool ledger | Rejected: 64-bool raw-slice verifier preserved behavior but lost on paired/reversed hyperfine. | Retry only as a producer-carried witness or bitpacked mask primitive, not another raw block-size verifier. |
| Borrowed join output metadata shortcut | memory June 9 ordered UTF8 join ledger | Rejected: setup-free proof kept semantics but the pushed result was rejection evidence; next route was deeper output assembly. | Retry only if a new split gate isolates output assembly rather than metadata borrowing. |
| Clone-free generic count/size counters | br-frankenpandas-uza04.187 | Pending batch benchmark: code-first campaign allowed `cargo check -p fp-groupby` only. | Keep only if same-host `--agg count/size --key-kind utf8` beats the vector-cloning path without conformance regressions. |
| Clone-free generic mean counters | br-frankenpandas-uza04.189 | Pending batch benchmark: code-first campaign allowed `cargo check -p fp-groupby` only. | Keep only if same-host `--agg mean --key-kind utf8` beats the vector-cloning path without conformance regressions. |
| Clone-free generic min/max scalar slots | br-frankenpandas-uza04.191 | Pending batch benchmark: code-first campaign allowed `cargo check -p fp-groupby` only. | Keep only if paired `groupby-bench --agg min/max --key-kind utf8` beats the pre-change vector-cloning path without conformance regressions. |
| Clone-free generic first/last scalar slots | This bead | Pending batch benchmark: code-first campaign allowed `cargo check -p fp-groupby` only. | Keep only if paired `groupby-bench --agg first/last --key-kind utf8` beats the pre-change vector-cloning path without conformance regressions. |

## Isomorphism

- Group identity: unchanged. The helper uses the same `GroupKeyRef::from_scalar` keys as the generic fallback.
- Ordering: unchanged. First-seen order comes from the same `ordering` vector; sorted order uses the same `sort_group_ordering_by` comparator.
- Output labels: unchanged. Labels are reconstructed from the first source row, matching the fallback.
- Null handling: unchanged. Missing values are skipped; all-missing groups emit `Null(NaN)`.
- First semantics: unchanged. The first non-missing value installs the slot once, matching `vals[0]`.
- Last semantics: unchanged. Each non-missing value overwrites the slot, matching `vals[vals.len() - 1]`.
- Dtype preservation: unchanged. The selected source `Scalar` is cloned directly before `Column::from_values` inference, just as the vector fallback did.
- RNG: N/A.

## Bench Guard

`crates/fp-groupby/src/bin/groupby-bench.rs` exposes the realistic string-key route:

```bash
CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a cargo run -p fp-groupby --bin groupby-bench -- --agg first --key-kind utf8 --rows 500000 --key-cardinality 512 --iters 25
CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a cargo run -p fp-groupby --bin groupby-bench -- --agg last --key-kind utf8 --rows 500000 --key-cardinality 512 --iters 25
```

Batch-test status: pending by instruction; only `cargo check -p fp-groupby` was run in this code-first commit.
