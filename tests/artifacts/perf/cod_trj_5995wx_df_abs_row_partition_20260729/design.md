# Candidate design — portable two-dimensional `df_abs`

## Entry guard

The candidate is a narrow `DataFrame::abs()` fast path. It may run only when:

- every frame column is an all-valid contiguous `Float64` column;
- the row count clears the measured crossover threshold;
- `available_parallelism()` exceeds the column count.

Every other dtype/validity/shape retains the current
`apply_per_column_min(131_072, Column::abs)` path. This keeps mixed, nullable,
Bool, Int64, Datetime-like pass-through, index, and error behavior outside the
new scheduling path.

## Work decomposition

Collect the ten immutable input slices in column order. Divide the available
worker budget across columns, then divide each column into disjoint contiguous
row chunks. One `std::thread::scope` owns every chunk task, so the total spawned
worker count is the requested budget rather than ten nested pools. After a
single shared start barrier, each task fills one exact-length `Arc<[f64]>`
directly through the standard library's exact-size iterator specialization:

```text
output_chunk = input_chunk.map(abs).collect::<Arc<[f64]>>()
```

No task shares output state. There is no pre-zero pass, no intermediate
`Vec<f64>`, and no `Vec`-to-`Arc` element copy. Reassembly retains chunk and
column order. The implementation is safe Rust with no x86 intrinsics; it
therefore keeps the scalar/codegen fallback usable on Apple Silicon.

## Column boundary

Keep the finiteness proof inside `fp-columnar`. A hidden helper accepts source
`Column` references, rejects anything other than all-valid Float64, performs
the row-chunk map, and constructs each result through a private
finiteness-witness-preserving chunk constructor. This avoids exposing a generic
"trust me" constructor to `fp-frame` and avoids a second NaN/finiteness scan.
Every absolute value is computed before `DataFrame::abs()` returns; only
optional consolidation for a later contiguous-slice observer remains lazy, as
it does for existing chunk-backed columns. Exact-bit tests cross that boundary.

Do not pre-zero a contiguous output and overwrite it: at 10M×10 that adds an
800 MB write-only pass to a bandwidth-bound operation. Do not use nested
per-column thread scopes: at cap 128 that would create ten pools and obscure
the actual worker budget.

## Negative-evidence boundary

This is not a retry of the historical `[[f64-arc-copy-on-produce]]` lever.
Current `Column::abs` already moves its output `Vec<f64>` into
`LazyAllValidFloat64Vec`; there is no incumbent `Vec`-to-`Arc<[f64]>` data
copy left to remove. The candidate needs independently owned worker chunks,
so it collects each slice-map iterator directly into `Arc<[f64]>`. The pinned
Rust 1.97 nightly implements `FromIterator` for an iterator satisfying
`TrustedLen` through `Arc::from_iter_exact`, which allocates the Arc slice once
and writes iterator elements directly into it; slice `Iter` and `Map` carry
that trusted-length contract. Re-open only if the pinned toolchain or iterator
shape changes and an allocation/copy count disproves this mechanism.

## Required proof before timing

- For `-0.0`, finite signs, infinities, and tail chunks, every result value has
  the same `to_bits()` as the current `Column::abs`.
- Dtype, length, validity, finiteness witness, index, and column order match.
- Nullable and mixed frames demonstrably take the existing fallback.
- The untimed operation probe observes more than ten workers at caps above ten.
- The named-frame baseline profile exposes enough removable self-time to make
  the 13.780 ms 10M incumbent budget feasible.

## Exclusive-host protocol

The trj booking is part of the evidence, not an informal scheduling detail.
Before any profile or timed arm:

1. Re-read Agent Mail thread `trj-booking` and require a recorded
   `[trj] RELEASE` from the preceding holder.
2. Post `[trj] CLAIM frankenpandas` and retain that message ID in the result
   artifact.
3. Require the harness's host-wide quiescence preflight to pass over every
   online CPU, including CPUs outside the benchmark's affinity mask. Repeat
   that check immediately before and after every engine arm.
4. Invalidate the whole affected invocation if another process makes the
   quiescence check fail; do not preserve an apparently favorable arm.
5. Post `[trj] RELEASE frankenpandas` immediately after the final artifact is
   copied and verified, or after any failure that ends the run.

Only the active claim holder may connect to trj. Compilation, source review,
exact-bit tests, and artifact preparation remain off-host.

## Prepared identities

- Exact signed-current baseline ELF from commit `7d6630b28`:
  `6b37a4d1a613953f1a3d15a6459029a1784424522c4af5f21bddefc9391eaada`
  (73,715,112 bytes), strict-remote-built on RCH worker `vmi1153651`
  and retained at
  `/data/projects/frankenpandas-prepared-dfabs-20260729/baseline-7d6630b28/release-perf/fp-bench`.
- Row×column candidate ELF, strict-remote build on RCH worker `vmi1153651`:
  `067bcf3bd6122ff916cfeed507c96c09f6b42e4a993cf34b7462d8d0623a6262`
  (73,880,632 bytes). External `sha256sum` and line-one process self-report
  agree exactly. A fresh read-back from that worker also matches the current
  local build inputs byte-for-byte:
  `fp-columnar/src/lib.rs=acaf39eaebb7e6628c1d7ceba73ea4e47c26071cca19064efaaf212e7c146ea0`,
  `fp-frame/src/lib.rs=922d6dd1af49fd9a6d39450a23e2f707eea66b406bc366859a8ac4fed117ee90`,
  and
  `Cargo.lock=e570e80ba6820cd01673456aeea0d0a71b02d8148504cdc95bcb217cabe7335a`.
- Arm-bracketing harness:
  `b7bf3fb488d0539711dcf6c7e051372f52523a100b22599f81e910371d8533c7`
  (97,714 bytes), with the mechanically self-checking bracket helper landed
  in signed commit `7d6630b28`.

The candidate ELF is not a result until it is deployed only after the
recorded trj claim, re-hashed on trj, and admitted by the baseline profile.

## Current-ELF profile admission

After Agent Mail RELEASE 6169 and FrankenPandas CLAIM 6189, the exact
`7d6630b28` baseline ELF ran the 10M `df_abs` fixture under affinity
`0-127`. Independent all-online-CPU samples immediately before and after the
profile were clear. The operation probe observed ten workers, and `perf
record -e cycles:u -F 997 --call-graph dwarf` attached after all twelve
process threads (main, monitor, ten operation workers) were live. It retained
8,779 samples with zero lost samples.

`<fp_columnar::Column>::abs` carries 96.51% self-time. Its impossible
full-removal Amdahl ceiling is `1 / (1 - 0.9651) = 28.65x`; replacing ten
equal workers with 128 equal workers predicts a 9.07x FP-side ceiling under
the same decomposition assumptions. The previous best 10M incumbent ratio
was 3.537x, so a 5x incumbent result requires only a 1.414x FP-side
improvement. The named-frame gate therefore admits the candidate. Raw
process identity, attach observation, quiescence samples, and self/children
reports are retained under `profile/`.

## Invalidated admission attempt

The first post-profile current-ELF invocation began at
`2026-07-29T22:00:05Z`, just before the host's scheduled
`git-prune-broken-refs` service started a repository-wide `git fsck` sweep.
The 1M row completed, but the all-online-CPU post-check immediately after the
10M pandas arm observed CPUs 38 and 63 above the 20% busy ceiling and exited
2 before the 10M FrankenPandas arm. The entire invocation is invalid and no
1M value is salvaged. Its retained fail-closed log is
`invalid_full_current_t001.log` (SHA-256
`749f881b4d36482c758dea65200c4043e7bfe8c9f2b793ce2d78798794f79454`).
Retry requires the maintenance process to finish naturally followed by three
consecutive clear all-CPU admission samples; every retry uses a new filename.

## Off-host correctness gate

Before any trj timing, strict-remote RCH worker `ovh-a` ran:

```text
cargo test -p fp-frame abs -- --nocapture
```

All 18 selected tests passed. This includes the existing DataFrame and Series
absolute-value contract tests, exact-bit equivalence with seven row×column
workers over three columns, and the mixed/nullable fallback test. The command
ran under `RCH_REQUIRE_REMOTE=1` with local `CARGO_TARGET_DIR` removed; it is
correctness evidence only and carries no scaling claim.

## Threshold rule

Do not choose the threshold from the old 1M/10M endpoints. Use three separate
series so worker granularity and placement are not silently conflated:

1. Run the current ELF at 1M/2M/4M/6M/8M/10M under compact and L3-spread
   ten-CPU masks. It must report ten operation workers in every row; this
   isolates the old curve's placement effect.
2. Alternate the current and candidate ELFs over the same sizes at affinity
   cap 13. The current path must report ten workers and the candidate must
   report thirteen in every row. Holding each arm's worker cardinality fixed
   makes this the minimum-extra-parallelism crossover rather than an adaptive
   worker-count sweep.
3. Separately run the production candidate at requested caps
   1/2/4/8/16/32/64/128 for 1M and 10M. This series measures the adaptive
   policy, including the deliberate small-N worker limit, and is the only
   series used for the full-machine scaling claim.

Select the smallest row count whose fixed-cardinality candidate improvement
clears twice the same-invocation A/A log half-width in consecutive measured
sizes. If no monotone crossover exists, reject the automatic gate rather than
hard-coding a convenient endpoint.

If the measured crossover is above the candidate's current 1M activation,
introduce a separate minimum-total-values activation guard. Do not inflate the
per-worker target to suppress the 1M row: that would also reduce the useful
worker count below 128 at 10M and silently recreate a different ceiling.
Rebuild from the guarded source, re-establish the process-self ELF identity,
and repeat the production sweep before making a decision.

The maximum worker policy is decided separately from the row-count crossover.
At 10M, compare the 64-physical-thread and 128-SMT rows under their own null
intervals. Ship 128 only if its improvement over 64 clears the two-times-null
median-CI margin; an indistinguishable result is not evidence that SMT removed
work, so choose the smaller cap. If 128 is tied or worse, cap this streaming
path at the smallest worker count in the measured best envelope, rebuild, and
repeat the production sweep. Requested affinity is never substituted for the
operation probe's observed worker count.

## Final adjudication

The exclusive run completed under FrankenPandas claim 6189 and was released
in Agent Mail message 6264. The final admitted harness SHA-256 was
`17994e77586614a4c54fc0a2edb2bcef140ff690a37344fdaa747899691cb209`.
Commit `ec06548826eee926f8f84cb477307ee5eeca81c7` retains that exact measured
source. A follow-up replaced `lru_cache(maxsize=None)` with the equivalent
Python 3.13 `cache` spelling only after the run, so it does not rewrite the
artifact's executable-source identity.
It placed the invocation preflight after provenance hashing, cached executable
identity, allowed setup-only activity to drain, and used a one-second all-CPU
sample while retaining the 20% busy ceiling. Eighteen canonical JSON files
cover 44 rows; all 65 copied raw files matched the `trj` source tree
recursively by SHA-256 before release.

The implementation achieved its mechanical goal but failed its performance
goal. At 10M, actual candidate workers rose to 16/32/64/128 at the corresponding
requested caps. The best raised-cap candidate was 24.173 ms at cap 64, versus
20.336 ms for the current scheduler at that cap and 20.022 ms for current at
cap 32. Candidate cap 128 was slightly slower again at 24.527 ms. The best
candidate same-invocation live-pandas ratio was 2.812x, below both current and
the 5x campaign target.

Each canonical row records 50 timed calls per engine, equal fixture
cardinality, stable checksums, host-wide exclusivity, actual worker counts,
process-self ELF identity, and independent A/A nulls. This rules out the
iteration-count mismatch highlighted by other fleet campaigns. The candidate
instead increases coordination and allocation work: up to 128 scoped threads
and independently allocated output chunks replace ten column tasks and ten
owned output buffers while the elementwise work is unchanged.

**Decision: REJECT.** Do not tune this design's row threshold or maximum
worker cap. Re-open only for a materially different scheduler that removes
per-call thread creation and per-worker output allocation, backed first by an
allocation/named-frame profile predicting at most 13.780 ms at 10M. Any future
positive result must put live pandas, current FrankenPandas, and the new
candidate in the same invocation, give every arm its own A/A null, record the
same complete hardware/ELF/thread provenance, and clear the median-CI margin.
The production source was restored after the proof corpus was banked.
