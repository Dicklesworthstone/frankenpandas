# br-frankenpandas-uza04.78 Pass 2 Primitive Selection

Timestamp: 2026-06-11T14:15:43Z
Worktree: `/data/projects/.scratch/frankenpandas-codex-opt-20260611`
Mode: analysis/artifact only; no source, Cargo, beads, or benchmark changes.

## Live Bead Check

`br show br-frankenpandas-uza04.78 --json` reports:

- Status: `in_progress`
- Assignee: `Codex`
- Parent: `br-frankenpandas-uza04`
- Labels: `filter-bool`, `fp-frame`, `no-gaps`, `perf`
- Scope: carry a bool-mask witness into `loc_bool`; do not repeat `.77` block-size verifier tweaks, `.76` constructor bypass, `.75` every-other recognizer, `.74` lazy affine index labels, or `.64` no-position gather.

## Fresh Profile Basis

Pass 1 artifacts read:

- `tests/artifacts/perf/uza0478_pass1_baseline_profile_witness.md`
- `tests/artifacts/perf/uza0478_base_perf_report_filter_bool_100000x20000.txt`
- `tests/artifacts/perf/uza0478_base_perf_report_children_filter_bool_100000x20000.txt`
- `tests/artifacts/perf/uza0478_base_perf_annotate_loc_bool_filter_bool_100000x20000.txt`
- `tests/artifacts/perf/uza0478_base_hyperfine_filter_bool_100000x1000.txt`
- `tests/artifacts/perf/uza0478_base_rch_hyperfine_filter_bool_100000x1000.txt`
- `tests/artifacts/perf/uza0478_base_golden_sha256.txt`
- `tests/artifacts/perf/uza0478_base_golden_verify.txt`

Baseline:

- `filter_bool 100000 1000`: `49.1 ms +/- 4.3 ms`; direct corroboration `49.2 ms +/- 3.6 ms`.
- Golden SHA `filter_bool 1000`: `f7eb0f99728d924edb7398ec4cbcc07808651a0f091db01e1a3c12e011d6265c`.
- Golden SHA `filter_bool 100000`: `2f77640d30dc32e1db9ab52eb869dc5ad503434f5a8a647a9d92f5f1ffa69cea`.

CPU profile:

- `<fp_frame::DataFrame>::loc_bool`: `46.70%` self, `71.27%` children under `perf_profile::main`.
- `<fp_frame::DataFrame>::new_with_axes`: `9.57%` children under `loc_bool`.
- `BTreeMap<String, Column>::insert`: `8.84%` children under `loc_bool`.
- `__memmove_avx_unaligned_erms`: `2.76%` children under `loc_bool`.
- `__memcmp_avx2_movbe`: `2.71%` children under `loc_bool`.
- Local annotation concentrates samples in `boolean_mask_matches_repeated_octet`, especially the repeated `test/add/cmp/lea/je` loop over 8-byte bool chunks.

Source read confirms the benchmark constructs a stable every-other `Vec<bool>` once and repeatedly calls `frame.iloc_bool(&mask)`, which delegates to `loc_bool(&[bool])`. Current `loc_bool` first rescans the raw bool slice to rediscover an affine certificate before calling the affine row projection path.

## Graveyard/Artifact Mapping

- Vectorized execution selection-vector discipline applies directly: carry a selection descriptor/certificate between producer and consumer instead of re-materializing or re-deriving row positions.
- Proof-carrying artifact discipline applies as an in-process immutable witness: runtime may consume the witness only when it was minted by a trusted local producer, its length/bounds are checked, and fallback remains the existing slice scanner.
- Succinct rank/select bitvectors are relevant only for a future compact arbitrary-mask representation; this profile is dominated by rediscovering a simple generated mask, not rank/select queries.

## Candidate Scores

Score = Impact x Confidence / Effort.

| Candidate | Impact | Confidence | Effort | Score | Assessment |
| --- | ---: | ---: | ---: | ---: | --- |
| Producer-carried immutable mask witness | 4 | 4 | 3 | 5.33 | Directly targets the dominant `loc_bool` verifier self-time by removing hot-loop rediscovery for masks whose producer already knows the certificate. This is not another recognizer: `loc_bool` consumes a private immutable witness or falls back to the current scanner. |
| Typed/Series-backed boolean mask descriptor | 1 | 3 | 3 | 1.00 | Architecturally useful for `df.loc[bool_series]`, but the measured `filter_bool` command uses `iloc_bool(&[bool])`, so this is not the current profile's dominant path. |
| Bitpacked/succinct bool mask certificate | 2 | 2 | 5 | 0.80 | Rank/select bitvectors are mathematically clean, but this workload already has a simple affine mask and no arbitrary-mask rank/select hot path in the fresh profile. Representation churn is too large for this bead. |
| Output-assembly map/vector-layout primitive | 3 | 3 | 5 | 1.80 | `new_with_axes` and `BTreeMap::insert` are real secondary child costs, but changing DataFrame output layout is broader than this bead and risks repeating `.76`-style constructor/output plumbing before the dominant mask-witness cost is removed. |

## Selected Primitive

Selected: producer-carried immutable bool-mask witness.

Concrete shape for the next pass:

- Introduce a narrow safe-Rust witness/descriptor path with private construction, e.g. an immutable mask reference plus `AffineSelectionCertificate { start, step, len }`.
- Allow only trusted producers to mint it, such as a deterministic every-other mask builder in the benchmark/conformance path or future typed comparison producers.
- Add a `loc_bool` sibling/internal helper that consumes the witness and jumps directly to `take_rows_by_affine_certificate_unchecked` after cheap length/bounds checks.
- Keep public `loc_bool(&[bool])` behavior unchanged: absent witness, stale witness, row-multiindex, unsupported source index/columns, or bound mismatch falls back to the existing scanner/path.

This differs from rejected or closed families:

- Not `.77`: no larger or different block-size verifier.
- Not `.75`: no new every-other pattern recognizer in `loc_bool`.
- Not `.76`: no constructor validation bypass as the primary lever.
- Not `.74`: no new lazy index-label backing.
- Not `.64`: no new no-position gather primitive; it reuses the already-kept affine projection only after avoiding redundant mask rediscovery.

## Isomorphism Obligations

- Witness expansion must equal the true mask positions: `pos_i = start + i * step`, `i in 0..len`.
- `mask.len() == self.len()` and final selected position must be in bounds before taking the fast path.
- Public raw-slice and error behavior must remain unchanged when no trusted witness is present.
- Row order, index labels/name, row multiindex handling, column order/names, and column multiindex handling must match the existing materialized/scanned path.
- Dtype, validity, null/NaN behavior, f64 bits, and fallback/panic behavior must remain byte-for-byte golden-equivalent.
- Witness fields must not be publicly forgeable; safe Rust only, no unchecked trust of external data.
- Verify unchanged goldens for `filter_bool 1000` and `filter_bool 100000`, then paired and reversed hyperfine. Keep only if measured Score remains at least 2.0.

## Recommendation

Proceed to Pass 3 implementation of the producer-carried immutable mask witness. Forecast Score is `4 x 4 / 3 = 5.33`, and the lever is profile-backed by the `46.70%` `loc_bool` self-time concentrated in mask rediscovery.
