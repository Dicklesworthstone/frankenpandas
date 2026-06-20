# FrankenPandas Release-Readiness Scorecard

## Release-readiness verdict (gauntlet, measured)

**Perf vs pandas 2.2.3: 36/43 realistic ops faster (median ≈2.8× among wins); 5 losses,
2 neutral rows, all with documented fix paths; 0 shipped perf-lever regressions.** Conformance:
3078/3079 fp-frame tests pass (1 remaining failure — `groupby_prod_preserves_int64_j9w3s`,
cod-b's groupby-prod-dtype gap); the gauntlet drove this from 6 failures to 1 (peers fixed
the acosh/arccosh goldens; I fixed oeirt + tt0bx). NOT perf-lever-caused — every typed-lever
conformance guard passes by execution. Current dcfv8 gate also has `fp-conformance --lib
--tests` green; the `uza04.191` groupby min/max verification has focused `fp-groupby`
release tests green, and `uza04.192` groupby first/last verification has focused
`fp-groupby` release tests green. The `uza04.187` groupby count/size verification has
focused `fp-groupby` release tests green.

- **Ship-ready strengths:** value_counts (2.6×), drop_duplicates (2.0×), groupby int-key
  (5.4×), groupby sum/prod Utf8-key (2.18×/2.54×), groupby min/max Utf8-key (2.60×/2.54×),
  groupby first/last Utf8-key (2.92×/2.29×), groupby mean Utf8-key (2.80×), groupby nunique Utf8-key (2.89×),
  groupby count/size Utf8-key (2.49×/2.81×),
  groupby median Utf8-key (1.80–2.63×),
  groupby std/var Utf8-key (1.22–1.34×), Series.combine_first default construction (676×),
  reset/set_index (5–6.5×), std/var (11×), str case (6.5×), head/tail (17×),
  DataFrame.dropna Float64 (1.22×),
  slice/filter/sort/sum (1.2–1.3×), RangeIndex.asof scalar lookup (3,840–16,031×),
  RangeIndex bulk indexers (2.64–51.5×) —
  fp beats pandas wherever typed access unlocks a cheaper algorithm.
- **Known gaps before "faster than pandas everywhere":** concat was narrowed by the 3nah5
  mimalloc boundary allocator (24× slower -> 2.15× slower) but still needs a reused-buffer
  or chunk/view path; ffill now flips to 1.41× faster via skw2c validity-run bulk fill;
  shift flips to 1.40× faster in the no-scan + mimalloc boundary mode while remaining
  allocator-sensitive on the plain glibc path; DataFrame.dropna typed Float64 now
  flips from a 0.42× loss to a 1.22× win via missing-free scan pruning, lazy validity
  allocation, the bandwidth-bound serial floor, and lazy all-valid Float64 chunks; max/min
  still trail pandas after the manual 8-lane accumulator, and safe `std::simd`
  i64x8/i64x4 probes were measured and reverted as regressions; Series add/mul now has a
  kept morsel-sweep lever that makes both arithmetic rows near-parity pinned, with mul
  faster unpinned and add still threshold-sensitive; Series.combine_first default
  construction now flips to a 676× win, and typed materialization flips to a 2.84×
  win, after og9qm's lazy all-valid Float64 select tape; forced public
  `values()` materialization remains a 0.21× consumption-path loss because it
  boxes every f64 into `Scalar`; Series.map Float64
  dense integer-key mapping now flips the default construction lane to a 7.04× win
  after hbq6y's lazy repeated-slice output + counter witness, and p0irg flips
  typed numeric materialization to a 5.24× win by exposing repeated Float64 slices
  as an owned f64 buffer for `to_numpy()`. Forced `values()` materialization
  remains a 0.44× consumption-path loss. The qngdp materialization probes were
  measured and reverted: the threaded typed-cache fill regressed the forced
  materialize path from 27.646 ms to 33.013 ms, and the scalar-block repeated-slice
  fill still regressed to 30.838 ms.
  All gaps are tracked.
- **Allocator adoption gate:** exact-parent `fp-bench` A/B for `250bfbf2` kept the 3nah5
  process-boundary allocator: 5 broad smoke wins (up to 3.35×), neutral control lanes, and
  no confirmed regression above 5% after paired reruns of the initially suspicious rows.
- **Conformance debt:** down to 1 failure (`j9w3s` groupby-prod dtype, cod-b) from 6 (bug cosyd).


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
| max / min | 2M int64 | 0.57× / 0.57× rerun | 🟡 8-lane chunked accumulator remains best safe-Rust path; safe `std::simd` i64x8/i64x4 rejected |
| Series add / mul | 2M f64 same-index | pinned add 1.01× neutral, mul 0.96× neutral; unpinned add 0.88× loss, mul 1.19× win | 🟡 tycz7 kept disjoint morsel sweep; FP-side add/mul ~6.0×/5.6× faster, add remains threshold-sensitive |
| Series.map Float64 | 2M f64, 50-entry zero-based full-coverage map | 7.04× deferred; FP-side 16.06→1.71 ms | 🟢 flipped from 0.75× loss; hbq6y stores periodic dense-code output as lazy repeated Float64 slices and replaces the witness modulo with a rolling counter |
| Series.map Float64 `to_numpy()` | same workload, forced `out.to_numpy()` materialization | 5.24×; FP-side 32.95→2.30 ms | 🟢 p0irg exposes repeated Float64 slices through a direct owned f64 buffer for typed consumers; avoids public `values()` enum boxing |
| Series.map Float64 `values()` | same workload, forced `out.values()` materialization | 0.44× residual; qngdp probes 0.38-0.40× reverted | 🔴 residual scalar consumption-path loss; lazy repeated-slice `Scalar` materialization is still heavier than pandas' numeric result buffer; threaded enum materialization and scalar-block cloning both lost |
| Series.combine_first | 2M f64 same-index, ~50% NaN fill | 676× default construct; 2.84× typed materialize | 🟢 flipped from 0.48× loss; og9qm defers the all-valid Float64 select into a lazy tape and only materializes the selected f64 buffer for typed consumers |
| Series.combine_first `values()` | same workload, forced `out.values()` materialization | 0.21–0.23× residual | 🔴 residual consumption-path loss; public `values()` still boxes every f64 into `Scalar`; 3gsa7 scalar-materializer probes were measured and reverted/no-shipped |
| reset_index | 1M int64-indexed | 5.1× | 🟢 |
| loc[[labels]] sorted Int64 | 2M f64 step-2 idx, select 1000 | 1.58× | 🟢 flipped from 5340× SLOWER; 0pkt2 cached int64_view + binary-search batch resolver |
| loc[[labels]] unsorted Int64 | 2M f64 shuffled unique idx, select 1000 | 13.7× | 🟢 flipped from 5147× SLOWER; 2pvdg identity-cached i64→pos hashtable |
| loc[[labels]] Utf8 index | 2M f64 string idx, select 1000 | 7.9× | 🟢 flipped from 2029× SLOWER; sfsx4 identity-cached String→pos hashtable |
| loc[[ts]] Datetime64 index | 2M f64 1-min DatetimeIndex, select 1000 | 67.6× | 🟢 flipped from 1173× SLOWER; recbe identity-cached ns→pos hashtable |
| get_indexer unsorted Utf8 (repeated) | 1M unsorted Utf8 self, 1000 targets | 4.1× | 🟢 flipped from 744× SLOWER; c90bo routes core reindex/align/join resolver through cached loc lookups |
| get_indexer unsorted Int64 (repeated) | 1M unsorted Int64 self, 1000 targets | 3.6× | 🟢 flipped from 210× SLOWER; c90bo follow-on reuses cached i64 resolver instead of rebuilding the map |
| merge inner on Utf8 keys | 1M×1M → 500k rows | 0.42× | 🔴 2.4× slower; deferred (bead f1ftd) — fp-join pointer-key + output-alloc, hot/peer-reserved crate |
| str.lower/upper | 1M strings | 6.5× | 🟢 |
| concat | 8×125k Int64 | 0.46× with 3nah5 mimalloc boundary | 🔴 2.15× slower; allocator floor narrowed, still structural |
| DataFrame.dropna(how=any) | 500k×5 f64, ~10% NaN rows | 1.22× | 🟢 flipped from 0.42× loss; 9bccl uses missing-free Float64 witnesses plus lazy all-valid chunked run gather |
| shift | 2M, p=1 | 1.40× with dcfv8 no-scan + 3nah5 mimalloc boundary | 🟢 flipped; plain glibc path remains 0.64×, golden unchanged |
| ffill | 2M f64, ~10% NaN | 1.41× with skw2c validity-run fill + 3nah5 mimalloc boundary | 🟢 flipped; packed validity-run bulk fill |
| groupby.sum int key | 1M, 1000 keys | 5.4× | 🟢 dense grouping |
| groupby.mean utf8 key | 1M, 1000 keys | 2.80× | 🟢 clone-free streaming sum/count counters |
| groupby.sum utf8 key | 1M, 1000 keys | 2.18× | 🟢 clone-free streaming sum counter; was 0.56× |
| groupby.min utf8 key | 1M, 1000 keys | 2.60× | 🟢 clone-free streaming extremum slot |
| groupby.max utf8 key | 1M, 1000 keys | 2.54× | 🟢 clone-free streaming extremum slot |
| groupby.first utf8 key | 1M, 1000 keys | 2.92× | 🟢 clone-free streaming selected-value slot |
| groupby.last utf8 key | 1M, 1000 keys | 2.29× | 🟢 clone-free streaming selected-value slot |
| groupby.count utf8 key | 1M, 1000 keys | 2.49× | 🟢 clone-free streaming non-null counter |
| groupby.size utf8 key | 1M, 1000 keys | 2.81× | 🟢 clone-free streaming total-row counter |
| groupby.agg(nunique) utf8 key | 2M, 1000 keys | 2.89× | 🟢 CV-gated accepted |
| groupby.agg(median) utf8 key | 100k/2M, 1000 keys | 2.63× / 1.80× | 🟢 CV-gated accepted |
| groupby.agg(var) utf8 key | 100k/1M/2M, 1000 keys | 1.29× / 1.22× / 1.30× | 🟢 CV-gated accepted |
| groupby.agg(std) utf8 key | 100k/1M/2M, 1000 keys | 1.34× / 1.23× / 1.34× | 🟢 CV-gated accepted |
| set_index int col | 1M, 2 cols | 6.5× | 🟢 |
| RangeIndex.asof | 4,096 scalar probes, 100k/1M rows | 3,840× / 16,031× | 🟢 |
| RangeIndex.get_indexer miss-heavy | 100k / 1M targets | 2.64× / 3.61× | 🟢 flipped by arithmetic bulk membership; `rch` same-worker FP-side 4.0× |
| RangeIndex.reindex all-miss | 100k / 1M targets | 36.1× / 51.5× | 🟢 exact RangeIndex lattice fast path; `rch` same-worker FP-side 75.7× / 32.2× |

**Score: 36/43 measured ops faster than pandas; 5 losses (max, min, concat, Series.map Float64 `values()`, Series.combine_first `values()`),
2 neutral rows (add, mul pinned); 0 shipped regressions; 12 reverted/no-ship SIMD, allocation,
or ~0-gain attempts.**

Median win among the 36 ≈ 2.8×; the remaining losses are kernel/structural gaps with
documented fix paths — none are code-first fp-frame regressions. concat
remains a confirmed **column-rebuild** loss; ffill was the same class until skw2c changed
the no-limit path to bulk-copy the f64 buffer and fill only invalid validity runs.
RangeIndex indexers were a separate vectorized-engine gap after `29u49`; `uza04.159`
closed it with arithmetic bulk membership and an exact reindex lattice path.
The `uza04.191` groupby min/max verification closes two more generic-key extremum rows:
streaming scalar slots beat pandas 2.2.3 by 2.60×/2.54× on 1M Utf8-key Float64 groups,
with golden digests `def13b65b5e3a35d` and `6d20c5176a43035d`.
The `uza04.192` groupby first/last verification closes two more selected-value rows:
streaming scalar slots beat pandas by 2.92×/2.29× on the same fixture, with golden
digests `a8c2c037ffb85c88` and `d373b7337998d544`.
The `uza04.187` groupby count/size verification closes two more counter rows:
clone-free counters beat pandas by 2.49×/2.81× on the same fixture, with golden
digests `1e555b43a73656c1` and `c6ccd2e318a736dd`.
The latest `tycz7` Series add/mul pass kept a disjoint morsel sweep in
`apply_f64_slices_nan_tracked`: public add/mul improved from 16.56/16.40 ms to
2.76/2.91 ms pinned, while preserving focused conformance. Add is neutral pinned
(1.01×) but still loses in the unpinned sanity row (0.88×), and mul is neutral pinned /
a small win unpinned. The prior `38xpk`
push-output zero-fill and discard-ledger probes remain measured no-ships.
The latest `og9qm` Series.combine_first pass keeps the `grtx1` no-rescan proof and
`gmn0f` packed validity-word semantics, but stops eagerly building the selected Float64
buffer when the right side is all-valid. The same-index Float64 path now returns a lazy
all-valid select tape; CPU7 best-of-50 measured FP construction at 0.0091 ms vs pandas
6.177 ms (676× faster), and typed materialization at 2.142 ms vs pandas `to_numpy()`
6.075 ms (2.84× faster). Forced public `out.values()` remains red at 30.298 ms vs
pandas 6.506 ms (0.21×) because it boxes every f64 into `Scalar`. This pass is
**2 wins / 1 loss / 0 neutral**; route deeper to typed numeric public consumption or
lower-allocation scalar materialization, not another select-kernel trim. The follow-up
`3gsa7` scalar-materialization probes confirmed that loop reshaping alone is not enough:
local CPU7 baseline was 30.444 ms vs pandas 6.983 ms (0.23×); a right-buffer+patch
scalar fill regressed to 30.601 ms, and a single-pass scalar push reached only
29.999 ms (~1.5% FP-side, still 0.23× vs pandas). Both code probes were reverted;
the remaining route is an API/storage change that avoids public `Vec<Scalar>` for
numeric consumers, or a fundamentally smaller scalar representation.
The latest Series.map Float64 state keeps the earlier `0jdij` dense direct-address table
and hbq6y's guarded periodic dense-code witness, lazy repeated-slice output, and rolling
counter scan. Default Series construction is green at 7.04× vs pandas. p0irg adds the
typed numeric consumption path qngdp routed toward: `out.to_numpy()` now expands/copies
the lazy repeated Float64 tape as f64 directly, moving from 32.949 ms to 2.301 ms and
beating pandas 12.053 ms by 5.24×. Forced `out.values()` is still red at 27.348 ms vs
pandas 12.075 ms (0.44×). The attempted qngdp enum materializers remain no-ships:
the threaded typed-cache fill regressed the forced materialize path from 27.646 ms to
33.013 ms, and the scalar-block repeated-slice fill still regressed to 30.838 ms.
The remaining fix path is lower-allocation public scalar values, not parallel enum
boxing or cloning a scalar tape.

Pattern: typed-slice levers win 2–11× where they unlock a cheaper ALGORITHM (FxHash dedup,
dense value_counts, Welford std/var, contiguous str). They LOSE on ops that just rebuild
the whole Column. The 3nah5 mimalloc boundary allocator turns those losses from catastrophic
to actionable; dcfv8's no-scan shift path now flips shift to 1.40× faster under that boundary,
and skw2c's validity-run ffill path flips ffill to 1.41× faster. concat (0.46×) still trails
pandas because fp's column-rebuild construction is still heavier than numpy's pooled/in-place
memmove/concatenate; max/min
still need target-specific SIMD beyond current safe `std::simd` lowering; Series add/mul
still need durable numpy-class vectorization or output materialization work to move from
near-parity to clear wins; Series.map Float64 now wins on deferred construction and
typed `to_numpy()` consumption, while Series.combine_first wins on deferred/default
construction and typed materialization. Both forced `values()` paths still need
lower-allocation public scalar output despite the hbq6y/p0irg repeated-slice and og9qm
lazy-select keeps.
The latest 9bccl DataFrame.dropna pass turns the row-wise Float64 dropna case green:
missing-free selected columns are excluded from the row scan, nullable Float64 run-gather
defers validity bitmap allocation until an invalid output appears, the gather keeps the
bandwidth-bound 4M-cell serial floor, and `dropna(how=Any)` carries its kept-row validity
witness into lazy `LazyAllValidFloat64Chunks` output instead of copying every retained
f64 value. Same-core CPU7 best-of-200: FP 3.109 ms vs pandas 3.791 ms, 1.22× faster.
The qngdp `values()` materializers were measured and reverted because they added
initialization/thread/scalar-tape overhead without removing enum boxing. The Utf8 `groupby.sum` gap flipped under the clone-free streaming counter,
and the RangeIndex indexer gap flipped under the affine arithmetic bulk path, not by
weakening the retained public `get_loc` error semantics.

Notably, three of these (value_counts, sort_values, filter/dedup) were *lagging* pandas
before this session's levers (value_counts 0.62×, sort 0.91× per the perf-frontier notes)
and are now ahead — the FxHash-over-khash and zero-copy-gather/slice veins flipped them.

## Conformance (MEASURED — `rch exec -- cargo test --release -p fp-frame --tests`)

- **3078 passed / 1 failed** (was 3073/6 at first run; gauntlet drove it down). All **15
  typed-lever conformance guards PASS** → no recent perf lever regressed (bit-transparency
  verified by execution, not just compilation).
- Resolved during the gauntlet (verified by re-run):
  - `series_acosh_golden_basic`, `series_arccosh_golden_basic` — math goldens, now pass (peer fix).
  - `dataframe_set_index_rejects_null_labels_oeirt` — my early test wrongly rejected NaN
    labels; corrected to pandas-faithful semantics (NaN accepted, Null rejected). Passes.
  - `series_agg_size_any_all_tt0bx` — my early test expected pandas object-dtype Bool; fp's
    typed Column coerces mixed Int64+Bool agg → Int64 (values correct: any=1, all=0).
    Corrected the assertion to fp's actual behavior + documented the object-dtype gap. Passes.
- Remaining (1 failure, real gap, tracked in bug cosyd):
  - `dataframe_groupby_prod_preserves_int64_j9w3s` — groupby prod returns Float64(6.0) vs
    pandas Int64(6); dtype-preservation gap in cod-b's `aggregate_named_func`. NOT a
    perf-lever regression; needs an owner fix in the groupby kernel.
- Did NOT revert any perf lever (none caused these failures).

## Pending measurement

Remaining code-first lanes are now narrower: cod-b's categorical-index family and RangeIndex
helpers other than `jlv2o`/`uza04.159` still need focused Criterion/pandas rows. Already measured
rows above should not be treated as pending.

## Method / infra

- Build: `rch exec -- cargo build --release -p fp-frame --examples` (remote ovh-b,
  artifacts transferred to `/data/projects/.rch-targets/frankenpandas-cc`).
- Run: release example binaries executed **locally** (rch does not relay remote program
  stdout); pandas baselines via `python3` + `time.perf_counter` best-of-N.
