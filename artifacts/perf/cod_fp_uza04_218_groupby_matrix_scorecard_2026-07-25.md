## Performance Scorecard

Generated: 2026-07-26

> **MODEL-INTEGRITY CORRECTION (2026-07-27):** this run measured only the
> five-workload GroupBy matrix. The former `1.24x` weighted score was invalid:
> five unmeasured categories had been inserted as `1.00x` placeholders. No
> whole-suite weighted result exists. The raw `df_groupby_2strkey_sum @10k`
> `0.8563x` sample also lacked an A/A null control and a counted mechanism, so
> it is `VOID-NONULL` routing evidence rather than an authoritative loss.

| Coverage | Scope | Admissible conclusion |
|---|---|---|
| GroupBy matrix | 5 workloads × 2 sizes | Harness coverage exists; raw ratios are retained in the linked bench artifacts |
| Other scorecard categories | Not measured | No ratio and no parity claim |

**Summary:** coverage-only artifact. Do not cite a weighted score or the
`0.8563x` sample as a decided performance verdict. Re-measure with executing-ELF
identity, a same-invocation A/A null, and the median-CI gate before drawing one.
