# Lane M DataFrame melt live-incumbent gate — 2026-07-28

## Outcome

At `cfb2397fc`, `df_melt` was run side-by-side with pandas 2.2.3 in one
invocation at 1M and 10M input rows. Both median-CI gates were decisive
FrankenPandas wins:

| size | long output rows | FP p50 | pandas p50 | ratio | combined 2x A/A interval | effect / required | verdict |
|---:|---:|---:|---:|---:|---|---:|---|
| 1M | 10M | 46.759 ms | 327.968 ms | **7.014x** | [0.800988, 1.248458] | 1.94792031 / 0.22190937 | FASTER |
| 10M | 100M | 477.967 ms | 3,101.565 ms | **6.489x** | [0.723188, 1.382767] | 1.87011998 / 0.32408655 | FASTER |

The absolute median advantage grows from 281.210 ms to 2.624 seconds, but
the ratio decreases by 7.5% as N grows. Melt is a strong incumbent win, not a
ratio-amplifying Class-1 result on this ELF.

## Harness contract

- Worker: `vmi1264463`
- Invocation:
  `vs-pandas-20260729T001542.350934Z-pid3244074`
- FrankenPandas ELF:
  `95bcac44a908ea7db67c069a319ef0b37886892892c9c55b581cde82c4b8d37a`
  (70,294,080 bytes)
- Harness source:
  `6109d2e720303ddb5ef2e13dd9aa2f4bf8b3f03444aab94129eed4d75974aff7`
  (63,405 bytes)
- pandas 2.2.3 artifact:
  `051be80fe43b4e0be4e04af314c42db966950eb877b0634d482099f42535e9bb`
- Python executable:
  `efb29ce53d36ebaeee80e3aa44fd6c7f9d71bbded5fe1665240b2ed8ecaeee0e`
  (6,894,448 bytes)
- 25 alternating A/A pairs per engine and row; 10,000 bootstrap resamples;
  95% median CI; 2x decidability margin.
- CV was recorded as provenance only and had no vote.

The FP A/A median CIs were [0.942553, 1.009586] at 1M and
[0.939184, 1.175911] at 10M. The pandas A/A median CIs were
[0.894979, 1.032962] at 1M and [0.984187, 1.050304] at 10M.

## Semantic boundary

Both engines start with ten all-valid Float64 columns named `col_0` through
`col_9`, with fixture construction outside the timed region.

- FrankenPandas runs `df.melt(&[], &[], None, None)`.
- pandas runs `df.melt()`.
- Empty id vars and value vars select every input column as a value column.
- Both use the default output column names `variable` and `value`.
- Each input row contributes one output row per input column, yielding 10M
  and 100M output rows.

The existing `dataframe_melt_auto_value_vars` and default-name unit coverage
locks the FP route; `live_oracle_dataframe_melt_basic` locks value ordering
and pandas parity. The new Python arm passed AST and workload-map validation,
then completed both full benchmark rows.

## Raw evidence

`artifacts/bench/proud_lane_m_melt_1m_10m_20260728.json`

SHA-256:
`6fd1aeba21a743549c902db456e02e51c0d547895bd44f60853964ced06cdb30`

## Validation

- The new Python arm passed AST and workload-map validation and executed all
  100 timed samples plus both engines' A/A controls at 1M and 10M.
- `python3 scripts/perf_candidate_preflight.py --self-test`: PASS (53 cases).
- `python3 scripts/perf_candidate_preflight.py --check-new-rows --base HEAD`:
  policy contract satisfied.
- UBS scanned the changed Python file. Its nonzero result was limited to
  pre-existing findings outside the touched lines: static-argv subprocess
  dispatch at line 1289, JSON parsing at line 1340, and Ruff findings at
  lines 804 and 1656. The added arm is confined to lines 493–499 and 1118.

## Retry predicates

- Keep the incumbent-win classification until the workload boundary, column
  count/dtype, FrankenPandas implementation, harness source, pandas artifact,
  allocator, compiler, or worker ISA changes.
- Re-open a ratio-growth claim only when the same self-identified ELF runs
  both sizes on the same worker in one invocation and two clean repeats show
  a larger 10M ratio outside the combined A/A intervals.
- Do not propose a melt source lever without a current profile naming a
  non-zero-self frame and a computed Amdahl ceiling.
