# br-frankenpandas-uza04.81 Pass 2 Primitive Selection

- Head: `217618ecfd25`
- Baseline: `tests/artifacts/perf/uza0481_pass1_baseline_profile_witness.md`
- Hotspot: `ryu::pretty::format64` at `32.10%` children / `23.30%` self in `csv_write_read_roundtrip`.

## Graveyard / Artifact Mapping

- `/data/projects/alien_cs_graveyard/alien_cs_graveyard.md` section 8.2 maps this to vectorized execution: operate on typed column batches, carry selection/shape metadata, and prove row-isomorphism against materialized behavior.
- Section 6.5 maps the concrete loop to affine-loop specialization: the high-ram values are `row * 1.25 + col`, so the output text can be generated from an affine integer numerator once the column proves the affine domain.
- Section 16.10 supplies the fallback rule: hide the specialized primitive behind the existing writer API, bound the accepted mode, and fall back to the incumbent formatter on uncertainty.
- The FrankenSuite summary reinforces evidence-ledger and artifact-gate discipline: baseline/profile/golden/proof first, then one lever with fallback.

## Candidate Matrix

| Candidate | Impact | Confidence | Effort | Score | Verdict |
| --- | ---: | ---: | ---: | ---: | --- |
| Certified quarter-affine Float64 CSV plan | 4 | 4 | 3 | 5.33 | Selected |
| General per-column dtoa memo/cache | 2 | 2 | 3 | 1.33 | Rejected: high-ram values are mostly unique |
| Morsel/parallel CSV writer | 3 | 2 | 5 | 1.20 | Rejected for this bead: ordering/quoting proof and threading surface too broad |
| Producer-side columnar CSV witness in fp-columnar | 4 | 3 | 5 | 2.40 | Defer: useful later, but not the narrow fp-io lever |
| More Ryu spelling branches | 1 | 3 | 2 | 1.50 | Rejected: repeats `.80` formatter-family work |

## Recommendation Contract

Change:
Add a private `Float64QuarterAffineCsvPlan` in `fp-io` for all-valid Float64 slices accepted by `try_write_csv_typed`. The plan certifies every cell as an exact nonnegative affine progression in quarter units, then emits pandas-compatible fixed decimal spellings from integer arithmetic.

Hotspot evidence:
Baseline `csv_write_read_roundtrip` mean `86.04315275 ms`, stable SHA `d93c23c308b1cbba82051d7dd022001e84e5313c0b485a440891a604ebfce367`, and profile `ryu::pretty::format64` `32.10%` children / `23.30%` self.

Mapped graveyard sections:
`8.2 Vectorized Execution + Morsel-Driven Parallelism`, `6.5 Polyhedral Automatic Parallelization & Locality Optimization`, and `16.10 Countermeasures Playbook`.

EV score:
Impact 4 x Confidence 4 / Effort 3 = 5.33.

Priority tier:
A for this profile-backed bead.

Adoption wedge:
Drop-in private writer fast path inside `try_write_csv_typed`; no public API or dtype/storage change.

Budgeted mode:
One O(n) certification scan per Float64 column only in the already all-valid typed writer path. On any failed check, fallback is the current `write_pandas_float` path for that whole column.

Expected-loss model:
False accept has infinite behavior cost, so the plan accepts only exact finite nonnegative quarter-unit affine values below the fixed-notation safety bound and rejects every uncertain value.

Proof obligations:
- Ordering preserved: row-major loop and column order unchanged.
- Tie-breaking unchanged: N/A.
- Floating-point: accepted values are never recomputed as floats; integer text is emitted only after exact `value * 4` certification. Fallback preserves existing formatter.
- RNG unchanged: N/A.
- Golden output: stable SHA must remain `d93c23c308b1cbba82051d7dd022001e84e5313c0b485a440891a604ebfce367`.
- Boundary fallback: `-0.0`, negative values, NaN, infinities, non-quarter decimals, non-affine series, and scientific-notation-range values are rejected.

Fallback trigger:
Any certification miss or checked arithmetic overflow disables the plan for that column and keeps the existing per-cell Ryu/debug formatter.
