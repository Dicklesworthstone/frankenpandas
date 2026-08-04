# br-frankenpandas-uza04.73 rejection proof

Agent: OrangePeak
Date: 2026-06-10

## Profile-backed target

Baseline head: `89c6fd1d`

Fresh `outer_join 500000x2000` profile after the `.72` keep still showed
`build_dense_cycle_outer_run_tape` as the largest join-kernel residual:

- `build_dense_cycle_outer_run_tape`: 16.54% self
- visible residuals: `key_runs` push at 3.98%, `offset_count_for_key`
  checked arithmetic at 2.71%, invalid-range append at 1.05%

## Lever attempted

Single lever: replace the per-bucket dense outer run-tape walk for the
certified dense-cycle outer-join shape with a parametric interval schedule.
The candidate computed output length and sparse validity ranges from
left-only / overlap / right-only intervals, carried the shared key as a lazy
witness-backed Int64 lane, and bypassed eager `key_runs` construction on the
accepted path.

Alien primitive: vectorized/cache-shaped execution from §8.2 plus the
join-specialized cache-shaped route used in the §8 analytics-query recipe.

## Isomorphism proof

The candidate was behaviorally isomorphic on the accepted shape:

- Ordering: interval segments were sorted by ascending key and the overlap
  segment preserved the dense-cycle bucket order used by the original walk.
- Tie-breaking: duplicate joins remained left-major within each key bucket;
  the candidate changed only shape descriptors, not source value lane
  materialization.
- Null placement: left-only and right-only invalid ranges were emitted at the
  same output offsets as the original bucket walk.
- Dtype behavior: shared key stayed all-valid Int64; left/right promoted lanes
  continued using the existing sparse-validity constructors from prior keeps.
- Floating point: no arithmetic changed; promoted `i64 as f64` materialization
  stayed in the existing columnar paths.
- RNG: none.
- Fallback: non-certified or non-matching dense-cycle shapes fell back to the
  original `build_dense_cycle_outer_run_tape` implementation.

Focused parity passed during the attempt:

```text
cargo test -p fp-join merge_outer_dense_int64_duplicates_matches_generic_validated_route --lib
```

Golden output was byte-identical:

```text
outer_join 20000 sha256 = 453b318a10d828fcf65cef61898a965ea4e1a402e041cb45b0a7a4001b65b750
golden_cmp=0
sha256sum -c: OK
```

## Benchmark gate

Baseline artifacts:

- `hyperfine_base_uza0473_outer_join_500000x20.txt`: 75.9 ms +/- 6.5
- `hyperfine_base_uza0473_outer_join_500000x200.txt`: 78.8 ms +/- 7.3

Captured direct timings:

- `outer_join 500000x20`: baseline 3.347 ms/iter, candidate 3.917 ms/iter
- `outer_join 500000x200`: baseline 0.343 ms/iter, candidate 0.342 ms/iter

Paired hyperfine:

- `outer_join 500000x20`: base 77.6 ms +/- 4.6, candidate 79.1 ms +/- 5.7;
  base ran 1.02x +/- 0.09 faster.
- `outer_join 500000x200`: base 77.7 ms +/- 4.6, candidate 76.8 ms +/- 5.0;
  candidate ran 1.01x +/- 0.09 faster.

Score: Impact 1 x Confidence 4 / Effort 3 = 1.33. Rejected because it is
below the required Score >= 2.0 and has no real same-gate win.

## Closeout

The candidate source hunk was removed before closeout. This bead leaves only
the baseline, golden, paired benchmark, profile, and rejection proof artifacts.

Next route: do not repeat the dense-cycle key-run/shape-schedule micro-family.
Move to the next ready profile-backed perf bead, `br-frankenpandas-uza04.64`,
or to a larger vectorized execution primitive that changes in-loop work enough
to move the paired hyperfine gate.
