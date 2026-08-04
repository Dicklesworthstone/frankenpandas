# br-frankenpandas-jbyuc.1.1.1.1.1.1.1.1.1.1.1.1 single-key setup rejection

## Target

- Worktree head: `fc692c566dcb351cc1db7f11f5bdc826ba93cf33`
- Workload: `perf_profile str_inner_join 1000000 1000000`
- Baseline binary: `/data/projects/.scratch/cargo-target-orangepeak-jbyuc111111111111-fc692c56-base/release-perf/examples/perf_profile`
- After binary: `/data/projects/.scratch/cargo-target-orangepeak-jbyuc111111111111-fc692c56-after/release-perf/examples/perf_profile`

## Baseline

- Build: `CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-jbyuc111111111111-fc692c56-base RUSTFLAGS='-C force-frame-pointers=yes' rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile`
- RCH status: failed open locally after worker preflight failures; command remained crate-scoped in a scratch target directory.
- Hyperfine baseline: `656.9 ms +/- 11.4 ms`
- Golden SHA: `76d2f388645ed3f3578017c5e2d919fa809e4230793a86d54d5ca93a6d0bc10e`

## Profile Evidence

Base profile artifact: `tests/artifacts/perf/perf_report_base_jbyuc111111111111_fc692c56_str_inner_join_1000000x1000000.txt`

- `fp_join::build_single_key_inner_merge_output_with_selections`: `9.78%` self / `36.55%` children
- `HashMap<&str, ()>::insert`: `2.14%` self / `18.83%` children
- `BTreeMap<String, Column>::insert`: `5.38%` self
- `Column::take_contiguous_range`: `4.35%` self
- `collect_join_key_columns`: `2.24%` self

## Lever Attempted

Guard the duplicate-key `HashSet` construction in `merge_dataframes_on_with_options` with `left_on.len() > 1` and `right_on.len() > 1`, because after the existing nonempty and equal-length guards, a single-element key slice cannot contain an intra-side duplicate key name.

The hunk was removed after measurement rejected it. No source change is kept.

## Isomorphism Proof

- Ordering preserved: yes. The attempted hunk only skipped duplicate-name validation table construction for single-key requests and did not alter row selection, sort mode, suffixing, or output assembly.
- Tie-breaking unchanged: yes. Probe order, cartesian duplicate behavior, and output column ordering were untouched.
- Floating-point: identical. The Float64 payload path was not modified.
- RNG seeds: unchanged/N/A. No RNG is used by the merge workload; the attempted hunk would have removed `RandomState` construction for the single-key validation path only.
- Error behavior: unchanged for multi-key duplicates; single-key duplicates are impossible within one side by slice cardinality. Empty and unequal key-list errors are still checked first.
- Golden output: after SHA matched baseline exactly: `76d2f388645ed3f3578017c5e2d919fa809e4230793a86d54d5ca93a6d0bc10e`; `golden_compare_jbyuc111111111111_single_key_setup.txt` recorded `golden_cmp=0`.

## Score Gate

Same-command paired hyperfine:

- Pair 1, base then after:
  - baseline: `689.8 ms +/- 28.9 ms`
  - after: `875.0 ms +/- 87.2 ms`
  - result: baseline ran `1.27x +/- 0.14` faster than after
- Pair 2, after then base:
  - after: `972.6 ms +/- 36.6 ms`
  - baseline: `1.073 s +/- 0.220 s`
  - result: noisy and not a reliable win

Measured score: Impact 0 x Confidence 4 / Effort 1 = `0.0`.

## Verdict

Rejected. Do not retry the duplicate-key guard family for this workload. The next target should be a deeper output/setup primitive such as ordered UTF8 setup certificate reuse (`br-frankenpandas-3wo1d`) or a separate output-map materialization replacement with its own baseline and proof.
