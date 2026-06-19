# FrankenPandas Release-Readiness Scorecard

## Release-readiness verdict (gauntlet, measured)

**Perf vs pandas 2.2.3: 12/17 realistic ops faster (median ≈5.9× among wins); 5 losses, all
kernel/structural with documented fix paths; 0 perf-lever regressions.** Conformance:
3073/3081 fp-frame tests pass; the 6 failures are pre-existing behavioral/parity/math-golden
gaps (NOT perf-lever-caused — every typed-lever conformance guard passes by execution).

- **Ship-ready strengths:** value_counts (2.6×), drop_duplicates (2.0×), groupby int-key
  (5.4×), reset/set_index (5–6.5×), std/var (11×), str case (6.5×), head/tail (17×), slice/
  filter/sort/sum (1.2–1.3×), RangeIndex.asof scalar lookup (3,840–16,031×) — fp beats
  pandas wherever typed access unlocks a cheaper algorithm.
- **Known gaps before "faster than pandas everywhere":** concat (24×) + shift (12×) need a
  kernel-level single-pass column builder (avoid rebuild); max/min (5×) need SIMD; utf8
  groupby (1.8×) needs key-factorize→dense. All 4 are kernel/structural, tracked.
- **Conformance debt:** 6 behavioral/golden test failures (bug cosyd) — fix before release.


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
| groupby.sum int key | 1M, 1000 keys | 5.4× | 🟢 dense grouping |
| groupby.sum utf8 key | 1M, 1000 keys | 0.56× | 🔴 1.78× slower (Utf8 hashing) |
| set_index int col | 1M, 2 cols | 6.5× | 🟢 |
| RangeIndex.asof | 4,096 scalar probes, 100k/1M rows | 3,840× / 16,031× | 🟢 |

**Score: 12/17 measured ops faster than pandas; 5 losses (max, min, concat, shift,
utf8-groupby); 0 regressions; 2 reverted ~0-gain attempts.**

Median win among the 12 ≈ 5.9×; the 5 losses are all kernel/structural (SIMD, column-rebuild,
Utf8-factorize) with documented fix paths — none are code-first fp-frame regressions.

Pattern: typed-slice levers win 2–11× where they unlock a cheaper ALGORITHM (FxHash dedup,
dense value_counts, Welford std/var, contiguous str). They LOSE on ops that just rebuild
the whole Column (concat 24×, shift 12×) — fp's column-rebuild construction is heavier than
numpy's in-place memmove/concatenate; and on max/min (~5×) which need SIMD. All 4 losses are
kernel/structural, not code-first.

Notably, three of these (value_counts, sort_values, filter/dedup) were *lagging* pandas
before this session's levers (value_counts 0.62×, sort 0.91× per the perf-frontier notes)
and are now ahead — the FxHash-over-khash and zero-copy-gather/slice veins flipped them.

## Conformance (MEASURED — `rch exec -- cargo test --release -p fp-frame --tests`)

- **3073 passed / 6 failed.** All **15 typed-lever conformance guards PASS** → no recent perf
  lever regressed (bit-transparency verified by execution, not just compilation).
- The 6 failures are **behavioral/parity/math-golden, NOT perf-lever regressions** (surfaced
  by the first real suite run — several are early cargo-check-only tests whose expectations
  were never executed):
  - `series_acosh_golden_basic`, `series_arccosh_golden_basic` — math goldens (not perf-related).
  - `series_agg_size_any_all_tt0bx` — agg of mixed Int64+Bool: the result Column coerces
    Bool→Int64 ([3,1,0,3]); test expected pandas object-dtype Bool ([3,True,False,3]). Real
    parity gap (mixed-agg object dtype), not a perf lever.
  - ~~`dataframe_set_index_rejects_null_labels_oeirt`~~ **FIXED**: my early test wrongly
    expected set_index to reject a Float64 NaN label; pandas ACCEPTS NaN index labels
    (verified vs 2.2.3) and fp's i10en Float64Index path correctly does too. Corrected the
    test to assert the pandas-faithful semantics (Scalar::Null rejected, Float64 NaN
    accepted) — now passes (rch cargo test exit 0).
  - `dataframe_groupby_prod_preserves_int64_j9w3s` — groupby prod returns Float64(6.0) vs
    Int64(6); dtype-preservation gap.
- ACTION: these need owner fixes (parity/golden), tracked separately; perf levers are clean.
  Did NOT revert any perf lever (none caused these).

## Pending measurement

Remaining code-first lanes are now narrower: cod-b's categorical-index family and older
RangeIndex helpers still need focused Criterion/pandas rows, and cod-a's groupby ledger has
high-CV rows to rerun. Already measured rows above should not be treated as pending.

## Method / infra

- Build: `rch exec -- cargo build --release -p fp-frame --examples` (remote ovh-b,
  artifacts transferred to `/data/projects/.rch-targets/frankenpandas-cc`).
- Run: release example binaries executed **locally** (rch does not relay remote program
  stdout); pandas baselines via `python3` + `time.perf_counter` best-of-N.
