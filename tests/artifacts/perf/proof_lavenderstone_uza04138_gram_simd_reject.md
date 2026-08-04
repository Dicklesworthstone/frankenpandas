# br-frankenpandas-uza04.138 SIMD Gram lane rejection

## Target

Fresh current routing after `2a12c533` showed:

- `df_cov 100000 5`: `60.997 ms/iter`
- `df_corr 100000 5`: `56.562 ms/iter`
- `perf record` blocked by kernel policy (`perf_event_paranoid=4`)

Selected primitive: apply safe `std::simd::Simd<f64, 4>` lane accumulation to
the existing full 4x4 `gram_partial_rows` tile. This follows the graveyard
vectorized-execution guidance: SIMD kernels over typed column batches with a
row-isomorphism proof. No tile-width retune was attempted.

## Baseline

Baseline binary:

`/data/projects/.scratch/cargo-target-lavenderstone-agent-route/release-perf/examples/perf_profile`

Baseline hyperfine:

- `df_cov 100000 5`: `309.4 ms +/- 22.8 ms`
- `df_corr 100000 5`: `304.3 ms +/- 15.1 ms`

Golden SHA256:

- `df_cov 2000`: `4db99b7be959e7a9fe2b2d5518e980c910968496ded930ebb80646699fd769ec`
- `df_cov 5000`: `c72cd8ebff48785c7620736d4e1b11d6a7c10dbb6a705e565429bd0787008eaa`
- `df_corr 2000`: `a0d0c26cb07cfca33d5105991cd528bbef7765d57ed17bb440768ea54dffe221`
- `df_corr 5000`: `ed9392f10bb6c553347a99e21ff5bdf8c140758fd9d8f86da61d1fd1addf58f2`

## Candidate

Candidate binary:

`/data/projects/.scratch/cargo-target-lavenderstone-uza04138-candidate/release-perf/examples/perf_profile`

Build note: first candidate build completed through RCH local fallback but did
not leave an unambiguous binary path; reran with explicit `CARGO_TARGET_DIR`.
The second build completed successfully.

Candidate golden SHA256 matched the baseline exactly for all four files above;
`tests/artifacts/perf/uza04138_candidate_golden_diff.txt` is empty.

## Isomorphism proof

- Ordering preserved: yes. The row-band partition, upper-triangle walk, column
  label order, and output assembly were unchanged.
- Tie-breaking unchanged: yes. There is no ordering tie-break in the Gram cell
  computation; output cells are assigned to the same `(i, j)` coordinates.
- Floating-point contract: candidate kept each cell's row-ascending update order
  and used separate multiply/add through `Simd` lane operations. Goldens were
  byte-identical on the focused fixtures.
- RNG seeds: N/A.
- Fallback behavior: tails (`bi < 4` or `bj < 4`) stayed on the existing scalar
  path. NaN/null behavior and pairwise filtering were untouched.

## Bench gate

Paired hyperfine:

- Forward `df_cov`: baseline `286.4 ms +/- 8.7 ms`, candidate
  `283.5 ms +/- 8.0 ms`; candidate `1.01x +/- 0.04`.
- Reversed `df_cov`: candidate `283.8 ms +/- 4.3 ms`, baseline
  `286.7 ms +/- 6.3 ms`; candidate `1.01x +/- 0.03`.
- Forward `df_corr`: baseline `283.3 ms +/- 5.3 ms`, candidate
  `287.9 ms +/- 5.6 ms`; baseline `1.02x +/- 0.03`.
- Reversed `df_corr`: candidate `287.9 ms +/- 6.9 ms`, baseline
  `287.6 ms +/- 11.9 ms`; baseline `1.00x +/- 0.05`.

Score: Impact 1 * Confidence 4 / Effort 2 = `2.0` by formula, but the measured
gain is noise/flat and does not clear the campaign keep standard. Source hunk
removed.

## Decision

Rejected. Do not retry the same 4-lane `gram_partial_rows` SIMD family. The
next Gram/corr route needs a structurally different primitive, such as reducing
partial-combine/output construction overhead or changing the row-band/column
layout with a fresh profile-backed target.
