# br-frankenpandas-uza04.95 - row-pair sign-tensor design rejection

Status: rejected before source edit. No runtime source changed.

## Target

`br-frankenpandas-uza04.95` asked for an exact all-pairs Kendall primitive based
on row-pair sign tensors / divide-and-conquer accumulation over packed
column-order signatures.

The intended algebra:

- For each unordered row pair `(a, b)`, define a bit signature `S(a,b)` where
  bit `c` is `1` when column `c` ranks `a` before `b`.
- For each column pair `(i, j)`, the pair is discordant iff
  `S(a,b)[i] xor S(a,b)[j]`.
- The full Kendall matrix can therefore be written as a sum over row-pair
  signatures.

This is exact but dangerous: the row-pair domain has
`n * (n - 1) / 2` elements.

## Fresh Baseline

Baseline binary:

`/data/projects/.scratch/cargo-target-lavenderstone-uza0495-base/release-perf/examples/perf_profile`

RCH build:

`CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-lavenderstone-uza0495-base RUSTFLAGS="-C force-frame-pointers=yes" rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile`

RCH worker: `vmi1153651`.

Golden-output SHA256:

- `df_kendall 2000`: `acf366c266b66f8497fb55734ed1b4ec40952a7c014f43db262c7c1e625e15e1`
- `df_kendall 5000`: `031978ba431260b942dd36d9be055064a7453118a7c00888c35747644d33d99e`
- `df_kendall 20000`: `f164fa86300fbea93e46aa99a5e0f7413fa8c0baf2ee49c6a689ed92652a4c3b`

Baseline hyperfine:

- `df_kendall 50000x1`: `117.8 ms +/- 7.1 ms`
- `df_kendall 200000x1`: `477.2 ms +/- 14.9 ms`

Artifacts:

- `tests/artifacts/perf/uza0495_baseline_df_kendall_50000.json`
- `tests/artifacts/perf/uza0495_baseline_df_kendall_200000.json`

`perf stat` profile probe was blocked by host policy:

`perf_event_paranoid setting is 4`.

The current profile-backed hotspot remains the same Kendall path identified in
the parent lane: `kendall_no_tie_fast_with_ordered_ranks` /
`count_ordered_rank_inversions_word_blocks` inside
`complete_kendall_no_tie_parallel_matrix`.

## Design Proof

The sign-tensor formulation only helps if the implementation can aggregate
row-pair signatures without materializing or visiting the row-pair surface.

For exact Kendall, each unordered row pair can carry an independent
`m`-bit ordering signature across `m` columns. The desired discordance matrix is
the xor co-occurrence sum over those signatures. A compressed accumulator must
therefore distinguish enough row-pair signature mass to update every affected
column-pair counter exactly.

The concrete routes available from the graveyard primitives reduce to rejected
or invalid forms:

- Explicit signature enumeration: exact, but `O(n^2)` row pairs. At
  `n=200000`, this is about `19,999,900,000` signatures before any column-pair
  update, far beyond the current `O(m^2 * n)` rank-counter surface.
- Succinct rank/select or wavelet rows: exact for one fixed ordering query, but
  arbitrary Kendall prefixes still require dynamic membership by another
  column's order. That is the `.91` and `.94` rejection family.
- Divide-and-conquer crossing partitions: exact only after carrying target
  membership through each split. That collapses back into the `.93` CDQ
  repartition/copy family or into pairwise target counters.
- Packed bitset/SWAR row-pair signatures: reduces per-signature update cost, but
  does not remove the `O(n^2)` number of signatures.

Therefore this bead has no proof-clean one-lever implementation that satisfies
both constraints:

1. exact integer discordance counts for every column pair, and
2. avoiding explicit row-pair enumeration or a previously rejected per-left /
   per-target dominance formulation.

## Decision

Reject before source edit. This is a route rejection, not a behavior change.

No `fp-frame` or harness source was edited for this bead. The only retained
artifacts are the fresh baseline and this proof.

## Next Route

Move off the Kendall rejected-family streak and attack a different
profile-backed primitive. The next target is the open safe-Rust numerical core
lane `br-frankenpandas-fbav3`: cache-blocked / communication-avoiding
`df.corr()` and `df.cov()` Gram kernels with exact fold-order proof and a target
ratio of at least `2.0x` over the current baseline.
