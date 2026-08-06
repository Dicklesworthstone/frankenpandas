# strings vs pandas — decidable rows banked 2026-08-06 (SilverDune)

Campaign-level ledger for the `strings` category on `thinkstation1`, measured
during the cod-pane wall. Every row below was produced by the **unmodified**
host-wide exclusivity gate: live pandas 2.2.3 incumbent in the SAME invocation,
pre/post measurement guards bracketing each arm, A/A nulls on both engines,
executing-ELF SHA-256 self-reported from inside the process. **No gate,
threshold or verdict was relaxed to obtain any of them.**

## Rows

| workload | ratio | verdict | FP p50 / CV / threads | pandas p50 / CV / threads | FP null | pandas null |
|---|---|---|---|---|---|---|
| `str_startswith_arrow` @1M | **4.814x FASTER** | FASTER | 809.80 us / 13.69% / 64 | 3898.63 us / 12.29% / 1 | 0.997542 | 0.996420 |
| `str_groupby_sum_arrow` @1M | **2.984x FASTER** | FASTER | 5759.56 us / 4.84% / 1 | 17185.59 us / 12.37% / 1 | 1.000309 | 0.989519 |
| `str_sort` @1M (run 1) | **10.512x FASTER** | FASTER | 29778.96 us / 5.88% / 10 | 313049.98 us / 1.77% / 1 | 0.988444 | 1.005648 |
| `str_sort` @1M (run 2) | **10.225x FASTER** | FASTER | 30300.16 us / 7.14% / 10 | 309831.67 us / 1.91% / 1 | 1.003655 | 0.998491 |

All four rows report `contract_valid: true` with an empty `contract_errors`, and
`null_undecidable_workloads: 0`.

**`str_sort` was measured twice independently and agrees to within 2.8%**
(10.512x vs 10.225x), with all four of its nulls inside +/-1.2% of unity. Two
replicates from separate invocations is stronger evidence than either row alone,
so both artifacts are banked rather than the "better" one being selected.

Raw artifacts:
- `artifacts/bench/i7znp_str_startswith_arrow_1M_thinkstation1_2026-08-06.json`
- `artifacts/bench/str_groupby_sum_arrow_1M_thinkstation1_2026-08-06.json`
- `artifacts/bench/str_sort_1M_thinkstation1_2026-08-06_run1.json`
- `artifacts/bench/str_sort_1M_thinkstation1_2026-08-06_run2.json`

## Read the thread counts with the ratios

`str_startswith_arrow` (FP 64 threads vs pandas 1) and `str_sort` (FP 10 vs
pandas 1) are **whole-job comparisons of each engine as shipped**, not per-core
claims. pandas is single-threaded on both paths; FP parallelises. Both counts are
ACTUAL OBSERVED values from `thread_provenance`, not requested ones. Quote them
alongside the ratio.

`str_groupby_sum_arrow` is the exception and the evidentially cleanest row:
**both arms single-threaded**, so it is apples-to-apples per core with no
parallelism caveat at all.

## The `str_sort` rows exist only because of br-frankenpandas-ooivn

Both were banked from invocations that **failed closed** (`exit 2`): run 1 was
rejected at `pre_measurement:frankenpandas-candidate:.../str_sort_object/...`
and run 2 at `post_measurement:pandas:.../str_sort_object/...` — in both cases on
a LATER workload, after `str_sort` had already passed every one of its own
guards. Before commit `89526b65c` the harness discarded `--output` on any
rejection, so both of these rows would have been destroyed.

That is this ledger's most reusable finding: **the fix paid for itself on its
first production use.** Their artifacts carry a non-null `invocation_rejection`
field, which is exactly right — the invocation was rejected, the individual rows
were not, and the field makes that distinction visible rather than inferred.
An artifact with a non-null `invocation_rejection` must never be read as a
complete run.

## Yield, reported honestly

Ten full-category passes produced decidable rows on **two** of them, and only for
`str_sort`. Five passes produced no artifact at all (refused at
`invocation_preflight`, before anything was measured — correct behaviour, nothing
had been blessed). `str_contains_arrow`, `str_sort_arrow`,
`str_value_counts_arrow`, `str_len`, `str_value_counts` and the `_object`
variants remain **UNMEASURED**; an earlier per-workload sweep with 12 retries
each landed 1 of 7.

The binding constraint is not the harness and not the gate — it is other tenants
on this shared VPS. Do not read the gaps above as evidence about those workloads
in either direction.
