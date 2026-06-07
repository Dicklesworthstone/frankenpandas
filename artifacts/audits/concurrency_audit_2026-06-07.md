# Concurrency Audit — 2026-06-07

Scope: the week's new parallel/lazy surface (cc-pandas + cod commits):
six fused dense-join builders (`std::thread::scope` + `split_at_mut`
bundles), parallel CSV chunk merge, `OnceLock` lazy materialization
(fp-columnar `ScalarValues` lazy variants incl. `LazyRepeatRunsInt64`
and cod's `LazyRepeatedSlicesInt64`, both of whose init closures spawn
scoped threads; fp-index `IndexLabels` dual backings), the global
`INDEX_LABEL_EQUALITY_CACHE` mutex, the CSV parse cache, and the
spearman rank worker pool. Method: /deadlock-finder-and-fixer static
audit with the interleaving-proof standard (every finding must name a
concrete thread schedule).

## Verdict: no deadlock paths found

| Surface | Analysis |
|---------|----------|
| `OnceLock::get_or_init` ×17 (fp-index, fp-columnar) | Every init closure is pure over captured slices or performs only non-blocking `get()` on a DIFFERENT OnceLock of the same struct; no closure can reach its own cell. The two expansion closures that spawn `thread::scope` workers (`LazyRepeatRunsInt64::repeat_runs_i64_data`, `expand_repeated_slices_i64`) hand workers only `&[i64]`/`&[(_,_)]` slices and disjoint `split_at_mut` chunks — no worker can touch any Column/OnceLock. |
| Nested `thread::scope` (e.g. spearman pool worker triggering an RLE expansion) | Safe by construction (inner scope joins before outer continues); worst case is a transient 8×16-thread burst on 64 cores — noted, not a bug. |
| `INDEX_LABEL_EQUALITY_CACHE` (global `Mutex<FxHashMap>`) | Equality (which can materialize lazy labels — heavy) is computed BETWEEN two short lock scopes, never under the lock; bounded at 4096 with clear-on-full; double-insert race is idempotent. CLEAN. |
| Fused-builder scopes (fp-join ×5, fp-io merge, fp-frame spearman) | All `as_i64_slice()` extraction (which may trigger expansion) happens on the spawning thread BEFORE the scope; workers receive raw slices. Work distribution via `AtomicUsize::fetch_add(Relaxed)` is correct (each index claimed once; Relaxed suffices for a claim ticket). |
| Mutex poisoning | CSV cache degrades to cache-off on poison (`.ok()?`/`let Ok else return`); equality cache uses `expect` but its critical sections contain only HashMap ops. Acceptable. |

## Finding (fixed in this commit): CSV parse cache cloned DataFrames under the global lock

`csv_parse_cache_lookup`/`_store` performed the O(data) `DataFrame`
deep-clone while holding the global cache mutex — serializing EVERY
concurrent `read_csv` (including cache misses, which merely want to
probe) behind one reader's multi-ms clone. Concrete interleaving: T1
hits on a large cached frame and clones under lock; T2..Tn block on the
mutex before they can even miss. No cycle back into the lock (clone
touches no fp-io state) → contention, not deadlock. Fix: entries hold
`Arc<DataFrame>`; the critical section is now Arc-bump + LRU splice
only, with deep clones outside the lock on both paths. Likely also the
amplifier behind the order-sensitive
`csv_parse_cache_keeps_default_and_no_na_modes_separate` flake observed
2026-06-06.

## Efficiency note (not filed as a bug)

Both repeat-lane variants materialize their Scalar view independently
of their i64 view (double expansion if a consumer reads both). Bounded
waste, no hazard; worth folding the Scalar view onto the expanded i64
cache if a profile ever shows it.

## Re-audit guidance

New `get_or_init` closures must stay pure over captured data (no
`self` method calls that can block); new scope workers must receive
pre-extracted slices, never `&Column`/`&Index`. Re-run the grep set in
this file's history on the next parallel-code batch.
