# br-frankenpandas-uza04.78 rejection proof

## Target

- Bead: `br-frankenpandas-uza04.78`
- Target: `filter_bool 100000 1000`
- Profile-backed hotspot: `DataFrame::loc_bool`, especially the every-other mask verifier at `crates/fp-frame/src/lib.rs:3009`
- Non-repeat boundary: do not retry `.77` block-size verifier tweaks, `.76` constructor bypass, `.75` every-other recognizer, `.74` lazy affine index labels, or `.64` no-position column gather

## Baseline

- Build: `CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-icyvalley-uza0478-base rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile`
- RCH selected worker for build: `vmi1227854`
- Hyperfine note: `rch exec` warned that non-compilation commands are not offloaded; timing ran locally. Candidate timing used the same local execution mode.
- Baseline standalone hyperfine: `48.1 ms +/- 3.4 ms`
- Fresh profile: `filter_bool 100000 20000` finished in `0.153s`, `0.008 ms/iter`, 188 samples
- Baseline top bucket: `DataFrame::loc_bool` at `50.78%` self; source-line report shows `lib.rs:3009` at `13.10%`

## Candidate

One lever attempted, then removed:

- Added an owned immutable `BooleanMask` witness with a cached affine certificate.
- Added `DataFrame::loc_bool_mask` / `iloc_bool_mask` to consume the witness without rescanning the raw `[bool]` slice.
- Switched the profiling harness `filter_bool` scenario to build the witness once outside the timed loop.

The source hunk was removed because the paired benchmark missed the keep threshold.

## Golden Output

Unchanged normalized golden hashes:

- `filter_bool 1000`: `f7eb0f99728d924edb7398ec4cbcc07808651a0f091db01e1a3c12e011d6265c`
- `filter_bool 100000`: `2f77640d30dc32e1db9ab52eb869dc5ad503434f5a8a647a9d92f5f1ffa69cea`
- Normalized hash diff: empty (`tests/artifacts/perf/uza0478_golden_hash_diff.txt`)

Behavior proof:

- Row order unchanged: mask true positions still feed the same row-take path.
- Index and column order/name semantics unchanged in the candidate: the existing `take_rows_by_affine_certificate_unchecked` path produced the output.
- Dtype, validity, null, NaN, and f64 bit behavior unchanged: no column data path changed, only certificate reuse was attempted.
- Tie-breaking and RNG behavior: not applicable to boolean filtering and no randomized code changed.
- Wrong-length rejection remained before certificate use in the candidate.

## Timing

Paired order:

- Baseline: `45.9 ms +/- 2.5 ms`
- Candidate: `43.8 ms +/- 3.7 ms`
- Ratio: candidate `1.05x +/- 0.11` faster

Reversed order:

- Candidate: `43.7 ms +/- 3.3 ms`
- Baseline: `46.9 ms +/- 3.7 ms`
- Ratio: candidate `1.07x +/- 0.12` faster

Score:

- Impact: 1.1
- Confidence: 0.7
- Effort: 1.0
- Score: `1.1 * 0.7 / 1.0 = 0.77`

Decision: reject. The source hunk was removed.

## Next Route

Do not add another wrapper around an existing `Vec<bool>` and do not retry block-size verifier tweaks. The next useful primitive needs to avoid the raw bool slice itself:

- Selection-descriptor producer path: boolean-producing operations emit an immutable row-selection descriptor directly.
- Bitpacked typed boolean mask storage: accept a validated bitset/rank witness without per-call `[bool]` verification.
- Filter fusion: comparison/query/filter chains materialize selected row positions or an affine descriptor directly, preserving pandas row order and null/NaN mask semantics.
