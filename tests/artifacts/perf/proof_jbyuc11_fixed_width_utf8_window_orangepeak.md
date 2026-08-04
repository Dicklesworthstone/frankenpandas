# br-frankenpandas-jbyuc.1.1 proof

## Target

`str_inner_join 1000000` after accepted baseline `3aa830b5`
(`chore(perf): reject jbyuc1 range output route`).

Fresh baseline profile for `str_inner_join 1000000 1000` ranked:

- `Column::take_positions`: 41.24% children / 30.80% self
- `merge_single_key_inner_unsorted`: 28.79% children / 10.90% self
- `__memcmp_avx2_movbe`: 9.88% children / 6.50% self
- `__memmove_avx_unaligned_erms`: 6.82% children / 3.38% self

The rejected jbyuc.1 range-output route proved that changing how the same
contiguous output copies are performed is not enough. This lever instead
targets the ordered UTF8 comparison wall before output materialization.

## Change

One lever: cache a fixed-width witness for all-valid strict
`LazyContiguousUtf8` columns, then let `ordered_unique_utf8_inner_positions`
attempt one bulk equal-window emission:

- Inputs must already satisfy the strict all-valid contiguous UTF8 witness.
- Left and right key rows must have the same cached fixed byte width.
- After lower-bound alignment, the candidate byte windows must compare exactly
  equal as one contiguous byte slice.
- If the window is equal, emit the matching left/right position ranges in bulk.
- If the window is not fully equal, or keys are variable-width, nullable,
  duplicate, unsorted, or non-contiguous, the existing scalar merge/fallback
  path runs unchanged.

Opportunity score: Impact 3 x Confidence 4 / Effort 2 = 6.0.

## Isomorphism

- Ordering preserved: yes. The bulk path extends `left_idx..left_idx+run_len`
  and `right_idx..right_idx+run_len`, identical to repeated per-row equal
  comparisons in the sorted unique merge.
- Tie-breaking unchanged: yes. The route is only for strict unique ordered keys,
  so each emitted key has one left row and one right row; duplicate/unsorted
  cases keep the existing hash fallback.
- Null semantics unchanged: yes. The fixed-width witness is only available for
  all-valid contiguous UTF8. Nullable keys do not enter this route.
- Suffix and column order unchanged: yes. Output builder and suffix resolution
  are untouched.
- Floating-point unchanged: yes. Float payload columns are gathered exactly as
  before; this lever only changes string-key position generation.
- RNG unchanged: N/A. The workload is deterministic and no RNG code changed.

Golden output:

```text
before sha256: 76d2f388645ed3f3578017c5e2d919fa809e4230793a86d54d5ca93a6d0bc10e
after  sha256: 76d2f388645ed3f3578017c5e2d919fa809e4230793a86d54d5ca93a6d0bc10e
cmp: pass
```

## Benchmarks

Crate-scoped before binary:

`/data/projects/.scratch/cargo-target-orangepeak-jbyuc11-base-20260608T1006/release-perf/examples/perf_profile`

Crate-scoped after binary:

`/data/projects/.scratch/cargo-target-orangepeak-jbyuc11-after-20260608T1015/release-perf/examples/perf_profile`

RCH note: both release-perf builds were invoked through `rch exec`, but RCH
failed open locally because no admissible worker slots were available.
`hyperfine` was also invoked through `rch exec`; RCH warned that hyperfine is a
non-compilation command, so timings ran locally against the paired binaries.

Total `str_inner_join 1000000 10`:

```text
before-only baseline: 103.9 ms +/- 5.4 ms
paired before:        106.0 ms +/- 3.6 ms
paired after:         103.5 ms +/- 4.9 ms
speedup:              1.02x +/- 0.06
```

Join-loop comparison (`str_inner_join 1000000 1000`):

```text
before: 1.310 s +/- 0.045 s
after:  867.7 ms +/- 13.3 ms
speedup: 1.51x +/- 0.06
```

## Validation

- `cargo fmt --check -p fp-columnar -p fp-join`: pass
- `rch exec -- cargo test -p fp-join ordered_unique_utf8 --lib`: pass on
  `vmi1156319`
- `rch exec -- cargo check -p fp-columnar -p fp-join --all-targets`: pass on
  `vmi1156319`
- `rch exec -- cargo clippy -p fp-columnar -p fp-join --all-targets --no-deps -- -D warnings`:
  pass on `vmi1156319`
- Broader `cargo clippy -p fp-columnar -p fp-join --all-targets -- -D warnings`
  is blocked by pre-existing dependency lint `clippy::question_mark` in
  `crates/fp-frame/src/lib.rs:48488`.
- `ubs crates/fp-columnar/src/lib.rs crates/fp-join/src/lib.rs`: exit 1 due
  broad pre-existing heuristic findings; UBS-internal formatting, clippy,
  cargo-check, and test-build sections are clean.

## Re-profile

After-profile (`str_inner_join 1000000 1000`) ranks:

- `Column::take_positions`: 64.72% children / 49.52% self
- `__memmove_avx_unaligned_erms`: 9.56% children / 4.94% self
- `Vec<usize>::extend_trusted<Range<usize>>`: 3.23%
- `__memcmp_avx2_movbe`: 3.11%

The next profile-backed target is output materialization itself: avoid or
delay `Column::take_positions` for ordered unique UTF8 inner-join contiguous
overlaps, without repeating the rejected direct range-copy route.
