# br-frankenpandas-0mtyz open-address residual rejection

Date: 2026-06-17
Agent: LavenderStone

## Change Evaluated

Residual safe-Rust khash-class/open-address Float64 `value_counts_nan50`
tally variants after the already-landed nullable Float64 open-address slice
(`14382121`) and all-valid unique/sort slice (`f8d082af`).

No source hunk is retained for this pass.

## Profile-Backed Target

- Bead: `br-frankenpandas-0mtyz`
- Workload: `perf_profile value_counts_nan50 1000000 10`
- Baseline artifact:
  `tests/artifacts/perf/lavender_0mtyz_base_hyperfine_value_counts_nan50_1000000x10.txt`
- Baseline: `648.2 ms +/- 22.4 ms`

## Golden Proof

- Baseline golden:
  `tests/artifacts/perf/lavender_0mtyz_base_golden_value_counts_nan50_5000.txt`
- Candidate golden:
  `tests/artifacts/perf/lavender_0mtyz_candidate4_golden_value_counts_nan50_5000.txt`
- SHA256: `2e0567f681bdacae8d0b5f59c542582b185b64186ada5bf4352741272dd0afb6`
- `cmp` status: `0`
- Candidate diff artifact:
  `tests/artifacts/perf/lavender_0mtyz_candidate4_golden_diff.txt` is empty.

## Bench Gate

Candidate 4 paired forward:

- Baseline: `615.1 ms +/- 5.4 ms`
- Candidate: `590.3 ms +/- 22.2 ms`
- Ratio: `1.04x +/- 0.04`

Candidate 4 paired reversed:

- Candidate: `597.0 ms +/- 26.5 ms`
- Baseline: `602.3 ms +/- 23.9 ms`
- Ratio: `1.01x +/- 0.06`

Earlier variants also failed the gate:

- Candidate 2 regressed: baseline was `1.03x +/- 0.07` faster.
- Candidate 3 was forward-only `1.09x +/- 0.05`, without a confirmed reversed
  keep ratio.

## Isomorphism Proof

- Ordering preserved: yes. The candidate outputs match the baseline golden
  byte-for-byte; first-seen ordering for tied counts is unchanged.
- Tie-breaking unchanged: yes. Stable count ordering is not altered in the
  retained source.
- Floating-point: identical output bytes for Float64 labels; `+0.0`/`-0.0`
  canonicalization and NaN/missing skipping follow the existing committed path.
- RNG: N/A.
- Golden output: SHA256 verified as unchanged.

## Score And Decision

Score is below the keep gate: Impact `1`, Confidence `3`, Effort `2` gives
`1.5`, and the reversed ratio is effectively neutral.

Decision: reject this residual 0mtyz pass, keep no new source hunk, and route
future work away from more nullable Float64 hash-table micro-variants unless a
fresh profile shows a different shared hash-table surface with a larger gap.
