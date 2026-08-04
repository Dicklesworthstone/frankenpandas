# br-frankenpandas-uza04.140 shared Utf8 join gather plans - rejected

## Target

- Profile-backed route:
  `tests/artifacts/perf/lavenderstone_agent_route_current_matrix.txt` after
  `2a12c533` measured `str_outer_join 100000x5 = 39.726 ms/iter` and
  `str_left_join 100000x5 = 32.255 ms/iter`.
- Candidate lever: share one optional-position Utf8 gather plan and validity
  mask across all-valid eager Utf8 payload columns during wide string joins.

## Semantic proof

- Baseline build:
  `tests/artifacts/perf/lavender_uza04140_base_build_perf_profile.txt`
  (`rch`, worker `vmi1227854`, release-perf `fp-conformance` example).
- Candidate build:
  `tests/artifacts/perf/lavender_uza04140_candidate_build_perf_profile_final.txt`
  (`rch`, worker `vmi1153651`, release-perf `fp-conformance` example).
- Golden SHA256 values matched baseline exactly:
  - `str_outer_join 5000`:
    `72546ef6396c90dd2a9a33b8da723079a5db22d449a7607ded73287b9818741c`
  - `str_left_join 5000`:
    `995b9ca366dacc4d808be3f5b99d027fd0b7d8f6adffd2ff9f91a43e92628006`
- Diff artifacts are empty:
  - `tests/artifacts/perf/lavender_uza04140_golden_str_outer_join_diff.txt`
  - `tests/artifacts/perf/lavender_uza04140_golden_str_left_join_diff.txt`

The candidate preserved row ordering, key tie behavior, suffix/column order,
missing Utf8 placement, dtype/validity shape, and RNG-free deterministic output.

## Bench gate

Binary-only internal timing:

- `str_outer_join 100000x5`: baseline `38.930 ms/iter`, candidate
  `38.547 ms/iter`.
- `str_left_join 100000x5`: baseline `33.688 ms/iter`, candidate
  `33.424 ms/iter`.

Paired hyperfine:

- Outer forward:
  `base 219.8 ms +/- 5.5`, `candidate 215.8 ms +/- 9.4`,
  candidate `1.02x +/- 0.05`.
- Outer reversed:
  `candidate 222.7 ms +/- 11.0`, `base 221.1 ms +/- 6.3`,
  base `1.01x +/- 0.06`.
- Left forward:
  `base 180.2 ms +/- 7.5`, `candidate 176.7 ms +/- 8.2`,
  candidate `1.02x +/- 0.06`.
- Left reversed:
  `candidate 176.8 ms +/- 5.9`, `base 184.2 ms +/- 8.4`,
  candidate `1.04x +/- 0.06`.

## Decision

Rejected. The semantic proof passed, but the broad shared-plan route did not
clear the Score >= 2.0 keep threshold: outer join was noise/negative under
order reversal, and left join was only low-single-digit improvement inside
variance. The candidate source hunk was removed. Next route should target a
deeper structural primitive in the join assembly path instead of reusing this
same optional-plan micro-lever.
