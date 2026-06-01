# FrankenPandas — Profiling Pass Hotspot Table

> **Measurement only.** This artifact is the hand-off to the optimizer agents
> (extreme-software-optimization). No code was optimized in this pass.
> Owner: RubyGoose (claude-opus-4-8). Run date: 2026-06-01.

## Scenario + Success Metric

| Scenario | Operation | Data shape (apples-to-apples w/ `vs_pandas` bench) | Success metric |
|----------|-----------|----------------------------------------------------|----------------|
| `filter_bool`     | `DataFrame::iloc_bool(mask)` | 100k rows × 10 float cols, mask = every other row | p50 latency vs pandas |
| `sort_single`     | `DataFrame::sort_values("c0", true)` | 100k rows × 4 float cols | p50 latency vs pandas |
| `drop_duplicates` | `DataFrame::drop_duplicates(None, First, false)` | 100k rows × 2 cols (Int64 key 100 groups, unique Float64 val) | p50 latency vs pandas |

**Target:** README claims "performance must EXCEED pandas." Success = FP p50 ≤ pandas p50 (currently 4–18× over).

## Environment Fingerprint

| Field | Value |
|-------|-------|
| CPU | AMD Ryzen Threadripper PRO 5975WX (32C/64T) |
| RAM / swap | 226 GB / 72 GB |
| Storage / FS | ext4 on dm-linear |
| Kernel | 6.17.0-29-generic |
| Toolchain | rustc nightly-2026-04-22 |
| Build profile | `release-perf` (opt-level=3, lto=thin, codegen-units=1, debug=line-tables-only, **strip=false**) |
| Build flags | `RUSTFLAGS=-C force-frame-pointers=yes` |
| Isolation | bare (same host), `perf_event_paranoid=1` for sampling |
| Samples | samply (frame-pointer call stacks), symbolicated via `addr2line` against the pie binary |

Full machine JSON: `fingerprint.json`. Raw profiles: `samply_{filter,sort_single,drop_duplicates}.json`. Hyperfine JSON: `fp_baseline_*.json`. Pandas baseline: `pandas_baseline_fresh.json`.

## Baseline (100k rows, ≥20 hyperfine runs; per-iter = mean_process / internal_iters)

| Scenario | FP p50/iter | FP σ | pandas p50 | pandas p95 | **Ratio (FP/pandas p50)** | CPU split (User/Sys) |
|----------|-------------|------|------------|------------|---------------------------|----------------------|
| `filter_bool`     | **6.81 ms** | ±0.30 | 0.38 ms | 0.40 ms | **17.9×** 🔴 | 710 / 36 ms → CPU-bound |
| `sort_single`     | **12.43 ms**| ±0.56 | 3.19 ms | 5.03 ms | **3.9×** 🟠 | 432 / 435 ms → alloc-heavy |
| `drop_duplicates` | **21.74 ms**| ±0.94 | 5.66 ms | 7.68 ms | **3.8×** 🟠 | 599 / 270 ms → alloc-heavy |

> ⚠️ **Stale-report correction:** the 2026-05-25 `artifacts/bench/PROFILING_REPORT.md` listed
> `drop_duplicates` as **229× / >7000ms timeout (CRITICAL)**. That regression has since been
> **fixed** — it is now 21.7ms @ 100k (3.8×). Optimizers should NOT chase the old 229× number.
> Variance is within envelope (σ < 5% of mean on all three).

## Ranked Hotspot Table (self-time, symbolicated)

| Rank | Scenario | Location | Self % | Category | Evidence |
|------|----------|----------|--------|----------|----------|
| 1 | filter_bool | `fp_frame::DataFrame::take_rows_by_positions_unchecked` (lib.rs:26427) | 34.5% | CPU/alloc | samply_filter.json → 0x40f86 |
| 2 | filter_bool | `fp_columnar::Column::new` (lib.rs:1149) | ~11% | CPU | samply_filter.json → 0x2f38c/0x2f267 |
| 3 | filter_bool | `drop_in_place::<BTreeMap<String,Column>>` | 6.0% | alloc | samply_filter.json → 0x548ae |
| 4 | filter_bool | `fp_columnar::ValidityMask::from_values` | 3.1% | CPU | samply_filter.json → 0x2efd7 |
| 5 | sort_single | `fp_frame::DataFrame::sort_values_na` (lib.rs:28484) | ~50% | CPU | samply_sort_single.json → 0x3f08d/0x3f070/0x3f081 |
| 6 | sort_single | (libc memcpy/element move, unsymbolized) | 8.4% | mem | samply_sort_single.json → 0x12ac5b |
| 7 | sort_single | `fp_index::Index::take` | 3.4% | alloc | samply_sort_single.json → 0x49188 |
| 8 | drop_duplicates | `fp_frame::DataFrame::take_rows_by_positions_unchecked` | ~10% | CPU/alloc | samply_drop_duplicates.json → 0x40f99/0x40f86 |
| 9 | drop_duplicates | `hashbrown HashMap<&[u64],usize>::{insert,get}` | ~10% | CPU | samply_drop_duplicates.json → 0x38ae9/0x2fa33 |
| 10 | drop_duplicates | `Vec<fp_index::IndexLabel>::clone` | ~8% | alloc | samply_drop_duplicates.json → 0x49eb6/0x49eb1/0x49ead |
| 11 | drop_duplicates | `fp_frame::DataFrame::duplicated` (lib.rs:28020) | 2.7% | CPU | samply_drop_duplicates.json → 0x3a5b1 |

## Hypothesis Ledger

```
H1  Column::new revalidation dominates materialization  : SUPPORTS — take_rows_by_positions_unchecked
    (cross-cutting, highest leverage)                      (34.5% filter, 10% drop_dup self) clones each Scalar
                                                           per position then calls Column::new, which makes ~5
                                                           O(n) passes (utf8-bucket x2, needs_coercion, normalize
                                                           map, ValidityMask::from_values) over an ALREADY-valid
                                                           source column. Fix: a gather/`from_validated_parts`
                                                           fast path that copies values + validity bitmap in one
                                                           pass, skipping coercion/revalidation. Affects filter,
                                                           sort, drop_duplicates, take, head, tail, reindex,
                                                           groupby row-selection — all funnel through here.

H2  filter materializes row-by-row, not vectorized      : SUPPORTS — filter is CPU-bound (User 710 / Sys 36);
                                                           34.5% self in take_rows_by_positions_unchecked +
                                                           ~14% in Column::new/ValidityMask. pandas does a
                                                           contiguous numpy mask copy at 0.38ms (17.9x gap).

H3  sort uses generic boxed-Scalar comparison sort      : SUPPORTS — ~50% self in sort_values_na, whose
                                                           order.sort_by(|l,r| compare_scalars_with_na_position(
                                                           values[l], values[r], ..)) does per-comparison enum
                                                           dispatch + bounds-checked Vec<Scalar> indexing
                                                           (~1.7M cmps for 100k). Fix: typed-column fast path
                                                           (extract f64/i64 slice, sort indices with native
                                                           comparator / pdqsort on the typed buffer).

H4  drop_duplicates key-building allocates per row      : SUPPORTS — duplicated() builds a Vec<u64> PER ROW
                                                           (n heap allocs) + uses HashMap<&[u64],usize> (slice
                                                           hashing/probing ~10% self) + per-cell BTreeMap<String>
                                                           column lookup. Sys time 270ms confirms alloc churn.
                                                           Fix: single flat key buffer / FxHash on packed key,
                                                           hoist column refs out of the row loop.

H5  drop_in_place BTreeMap<String,Column> is real cost  : SUPPORTS (minor) — 6% self in filter is just freeing
                                                           the intermediate frame; reducing intermediate
                                                           allocations (H1) removes it for free.

H6  drop_duplicates is still O(n^2) (per stale report)  : REJECTS — scaling 10k→100k is ~linear (2.3ms→21.7ms,
                                                           ~9.5x for 10x data) and no quadratic frame in profile;
                                                           the O(n^2) `keep=None` nested loop is not on the
                                                           default `keep=First` path. The 229x regression is gone.
```

## Hand-off

Top 5 hotspots ranked with evidence. **Highest-leverage single fix = H1** (`Column::new` fast-path in
`take_rows_by_positions_unchecked`), which improves filter, sort, and drop_duplicates simultaneously.
Ready for extreme-software-optimization to score (Impact × Confidence / Effort ≥ 2.0) and apply one lever
per run, re-baselining against this table. Reproduce any profile with:

```
RUSTFLAGS="-C force-frame-pointers=yes" cargo build -p fp-conformance --profile release-perf --example perf_profile
sudo env "PATH=$PATH" samply record --save-only -o p.json -- \
  ./<target>/release-perf/examples/perf_profile <scenario> 100000 300
```
