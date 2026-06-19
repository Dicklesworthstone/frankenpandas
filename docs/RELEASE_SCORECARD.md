# FrankenPandas Release-Readiness Scorecard

Head-to-head vs **pandas 2.2.3** on realistic single-thread workloads. Numbers are
measured (release binary run locally; see `docs/NEGATIVE_EVIDENCE.md` for method).
ratio = pandas / fp (>1 ⇒ fp faster).

## Perf vs pandas (measured this gauntlet)

| op | workload | ratio vs pandas | status |
|---|---|---:|:--:|
| head / tail | 2M, k=5 | ~17× | 🟢 |
| value_counts | 500k, 5k distinct | 2.59× | 🟢 |
| drop_duplicates | 1M, card 1000 | 2.03× | 🟢 |
| filter `s[mask]` | 2M, 50% | 1.29× | 🟢 |
| sort_values | 1M shuffled | 1.20× | 🟢 |
| std / var | 2M int64 | 11.3× | 🟢 |
| sum | 2M int64 | 1.27× | 🟢 |
| max / min | 2M int64 | 0.19× / 0.20× | 🔴 lose to numpy SIMD |
| reset_index | 1M int64-indexed | 5.1× | 🟢 |
| str.lower/upper | 1M strings | 6.5× | 🟢 |
| concat | 8×125k Int64 | 0.041× | 🔴 24× slower (structural) |
| shift | 2M, p=1 | 0.082× | 🔴 12× slower (structural) |

**Score: 9/13 measured ops faster than pandas; 4 losses (max, min, concat, shift);
0 regressions; 2 reverted ~0-gain attempts.**

Pattern: typed-slice levers win 2–11× where they unlock a cheaper ALGORITHM (FxHash dedup,
dense value_counts, Welford std/var, contiguous str). They LOSE on ops that just rebuild
the whole Column (concat 24×, shift 12×) — fp's column-rebuild construction is heavier than
numpy's in-place memmove/concatenate; and on max/min (~5×) which need SIMD. All 4 losses are
kernel/structural, not code-first.

Notably, three of these (value_counts, sort_values, filter/dedup) were *lagging* pandas
before this session's levers (value_counts 0.62×, sort 0.91× per the perf-frontier notes)
and are now ahead — the FxHash-over-khash and zero-copy-gather/slice veins flipped them.

## Conformance

- All shipped levers were designed bit-transparent and are covered by no-mock conformance
  guards (`crates/fp-frame/tests/*_conformance.rs`) asserting typed-path == Scalar-path /
  cross-dtype equality. Conformance/main green throughout.

## Pending measurement

Levers shipped but not yet head-to-head benched (no dedicated bench example yet, or
covered by the Series-level typed paths): reductions (sum/max/min/prod typed Int64),
numeric_moments/numeric_values/cov_components (var/std/sem/skew/kurt/corr typed Int64),
concat (typed buffer), reset_index/set_index (typed Index↔Column). These are guarded for
correctness; perf benches to follow.

## Method / infra

- Build: `rch exec -- cargo build --release -p fp-frame --examples` (remote ovh-b,
  artifacts transferred to `/data/projects/.rch-targets/frankenpandas-cc`).
- Run: release example binaries executed **locally** (rch does not relay remote program
  stdout); pandas baselines via `python3` + `time.perf_counter` best-of-N.
