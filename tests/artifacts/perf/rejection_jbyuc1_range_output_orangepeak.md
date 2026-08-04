# br-frankenpandas-jbyuc.1 Rejection Note

## Target

`str_inner_join 1000000 10` after `br-frankenpandas-jbyuc`.

Profile-backed hotspot: post-jbyuc profile showed output copying/materialization under
`fp_join::build_single_key_inner_merge_output` / `merge_single_key_inner_unsorted`
with `__memmove_avx_unaligned_erms` at 16.63%.

## Lever Tested

Contiguous ordered-UTF8 overlap detection plus direct contiguous-range output
materialization for inner joins.

This was one lever: replace the ordered UTF8 positions vector path with a
contiguous range certificate and build output columns from source ranges.

## Behavior Proof

- Ordering preserved: intended yes, output rows were still left/right ascending overlap ranges.
- Tie-breaking unchanged: intended yes, strict unique ordered UTF8 keys have one match per key.
- Floating-point: unchanged/N/A for the benchmark key path; payload copies preserve bits.
- RNG: unchanged/N/A.
- Golden output: before and after SHA both
  `76d2f388645ed3f3578017c5e2d919fa809e4230793a86d54d5ca93a6d0bc10e`;
  `cmp -s golden_before_str_inner_join_jbyuc1_orangepeak.txt golden_after_str_inner_join_jbyuc1_orangepeak.txt`
  passed.

## Benchmark Gate

Baseline hyperfine:

- `fp_before_str_inner_join_jbyuc1_orangepeak.json`
- `105.2 ms +/- 5.1`

After-only current candidate:

- `fp_after_str_inner_join_jbyuc1_orangepeak.json`
- `109.5 ms +/- 5.3`

Paired A/B, committed jbyuc vs block-range candidate:

- `fp_ab_str_inner_join_jbyuc1_blockrange_orangepeak.json`
- before: `108.0 ms +/- 4.4`
- after: `110.6 ms +/- 6.3`
- summary: committed jbyuc ran `1.02x +/- 0.07` faster than the candidate.

## Verdict

Rejected. Score is below 2.0 because measured impact is negative. The code
candidate was manually reverted and was not committed.

Next primitive: a deeper run-container / lazy selection-vector output
representation for join results, so contiguous overlap can avoid eager
per-column materialization instead of just changing how the same copies are
performed.
