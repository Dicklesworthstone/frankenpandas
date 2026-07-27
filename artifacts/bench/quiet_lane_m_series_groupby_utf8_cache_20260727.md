# Lane M SeriesGroupBy contiguous-Utf8 factorization-cache `maintenance-self-speedup` — 2026-07-27

This artifact records ProudChapel's profile-first execution of
`br-frankenpandas-f11op`. The lever reuses the immutable key `Column`'s existing
default-factorize witness across separately constructed `SeriesGroupBy`
objects. It does not add or replace a string hash table.

**Campaign result class:** `maintenance-self-speedup`.

This is an fp-before/fp-after measurement with no pandas incumbent arm. It is
not campaign output or a competitive claim.

## Admission profile

- Workload: 1,000,000 Float64 values grouped into 1,000 first-seen groups by
  one all-valid contiguous-Utf8 key column.
- Route: each loop constructs a fresh `SeriesGroupBy` over the same key column
  and runs `sum`.
- Worker: RCH alias `ovh-a`; reported host `fixmydocuments`.
- Strict-remote command: `RCH_REQUIRE_REMOTE=1 RCH_WORKER=ovh-a
  env -u CARGO_TARGET_DIR rch exec -- cargo test --profile release-perf
  -p fp-frame ...`.
- Executing profile ELF:
  `bench_elf_sha256=9fdfd9facb8a63eca562294e74640cf660ffebf3832c7a1483aeb794157774af
  (108075184 bytes)
  /data/projects/frankenpandas/.rch-target-ovh-a-pool-bc445989bdf88102bcbc62abd4347d69/release-perf/deps/fp_frame-4db73fde98141b5a`.

The unchanged path attributed **10.46% flat self-time** to
`SeriesGroupBy::compute_dense_group_ids`, clearing the bead's predeclared
`>5%` named-frame gate. A test-only `inline(never)` attribute preserved the
symbol without changing production codegen. `HashMap<&[u8], usize>::rustc_entry`
accounted for a further 50.90%, `__memcmp_avx2_movbe` 14.13%,
`SeriesGroupBy::sum` 6.68%, and `dense_group_labels` 4.69%. The workload
therefore reached the intended
SeriesGroupBy factorization route rather than the unrelated DataFrame
multi-key path.

## One-binary A/B contract

The release-perf test binary contained the production candidate plus a
`cfg(test)` switch that forced the original contiguous-Utf8 factorization
branch. Each block ran:

1. original factorization,
2. the identical original factorization as the A/A null,
3. cached-witness candidate,

with order alternated across 25 blocks.

**Executing ELF SHA-256 (self-reported by process):**
`bench_elf_sha256=b34345a187f398884d1cb00a40abafe1c19ab2a1f17535f3dd4bc996faa7b3a2
(108075152 bytes)
/data/projects/frankenpandas/.rch-target-ovh-a-pool-bc445989bdf88102bcbc62abd4347d69/release-perf/deps/fp_frame-4db73fde98141b5a`

**A/A null control (same invocation):** 25 alternating original/original pairs;
the 95% bootstrap median CIs were [0.979670, 1.008254] for reused-column and
[0.964642, 1.003810] for first-use.

**Median-CI decision:** the 3.999235x reused-column effect cleared its
0.04107857 required log effect, and the 1.091185x first-use effect cleared its
0.07199659 requirement; both thresholds are twice the paired A/A bootstrap
median-CI log half-width.

**CV role:** provenance only; CV had no vote.

| workload | original median | candidate median | paired median ratio | A/A 95% median CI | required log effect | verdict |
|---|---:|---:|---:|---:|---:|---|
| fresh groupby objects, reused key column, 1M rows | 10.319186 ms | 2.646363 ms | **3.999235x** | [0.979670, 1.008254] | 0.04107857 | **`maintenance-self-speedup` KEEP** |
| fresh key column per arm, 250k rows | 2.750980 ms | 2.572976 ms | **1.091185x** | [0.964642, 1.003810] | 0.07199659 | **`maintenance-self-speedup` KEEP** |

Both fp-before/fp-after effects clear their respective A/A median-CI floors.
The reused-column row measures witness reuse. The fresh-column control shows
that the column's canonical default factorizer plus gid conversion is also
faster than the old groupby-local hash path on first use.

## Mechanism and behavior proof

On a second or later fresh groupby object over the same immutable key column,
the original path performs one million string hash-table entry operations
again. The candidate instead clones the column's cached factorize witness and
converts its one million non-negative first-seen `i64` codes to the `usize`
gid vector expected by existing groupby consumers. On first use it builds the
same canonical witness once and performs the same conversion; that path also
cleared its null floor. Nullable, scalar-backed, and non-Utf8 keys
cannot produce this witness and retain their old paths.

The A/B fixture produced observably equal Series and identical group-label
order. A non-ignored regression also covers empty input, variable-width keys,
an empty string, repeated values, and non-ASCII `é`; original, first cached fresh
groupby, and second cached fresh groupby agree exactly, with first-seen labels
`["beta", "", "alpha", "é", "z"]` and sums `[5, 8, 3, 5, 7]`.

## Concrete follow-up predicate

Do not reopen this as another string hash-table attempt. Extend cache reuse to
nullable or scalar-backed keys only if a current repeated-object profile puts
that exact factorization frame above 5% self-time and a canonical immutable
witness proves identical missing/dropna and first-seen semantics. The same
one-binary executing-ELF/A/A/median-CI contract must show a decisive
fp-before/fp-after `maintenance-self-speedup`. It remains maintenance unless an
actual pandas incumbent arm runs side-by-side in the same invocation.
