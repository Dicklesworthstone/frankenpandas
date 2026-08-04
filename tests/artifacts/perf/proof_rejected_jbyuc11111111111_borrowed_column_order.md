# Rejected Lever: Borrowed Column Order Metadata

Bead: `br-frankenpandas-jbyuc.1.1.1.1.1.1.1.1.1.1.1`

Timestamp: `2026-06-09T05:41:43Z`

## Target

Post arithmetic lower-hex ordered UTF8 join profiling showed output materialization
and metadata insertion as the next visible residual:

- `fp_join::build_single_key_inner_merge_output_with_selections`: 9.15% self in
  the fresh baseline profile.
- `BTreeMap<String, Column>::insert`: 4.43% self.
- allocator and metadata-adjacent samples remained visible.

The tested one-lever candidate replaced per-merge `DataFrame::column_names()`
temporary vectors in the single-key output builders with a borrowed
`column_order()` accessor.

## Behavior Proof

The candidate did not change observable output:

- Baseline golden SHA:
  `76d2f388645ed3f3578017c5e2d919fa809e4230793a86d54d5ca93a6d0bc10e`
- Candidate golden SHA:
  `76d2f388645ed3f3578017c5e2d919fa809e4230793a86d54d5ca93a6d0bc10e`
- `cmp` between baseline and candidate golden files returned success.

Isomorphism obligations:

- Column iteration order remained the stored DataFrame column order.
- Key omission and suffix/tie behavior stayed on the existing output-builder
  branches.
- Null, NaN, floating-point bits, and RNG behavior were untouched.
- The candidate was metadata-only and did not alter row selection or column data.

## Benchmark Gate

Command:

```text
perf_profile str_inner_join 1000000 1000000
```

Same-command A/B:

- Baseline: `647.3 ms +/- 27.4 ms`
- Candidate: `699.3 ms +/- 27.6 ms`
- Ratio: `0.93x` candidate versus baseline

Score: `0.0`. Measured impact was negative, so this lever fails the
Score >= 2.0 keep gate.

## Decision

Rejected. The source hunk was removed and no code from this candidate is
retained. The evidence is kept because it rules out another metadata
micro-lever and points the next pass at a deeper structural primitive:
compact output column-map assembly or reduced column movement, not borrowed
column-name vector avoidance.
