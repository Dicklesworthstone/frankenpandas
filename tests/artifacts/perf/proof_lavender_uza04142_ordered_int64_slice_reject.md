# br-frankenpandas-uza04.142 - ordered Int64 slice validation rejected

## Target

Profile-backed route after `br-frankenpandas-uza04.140` rejected shared
Utf8 optional-position gather plans:

- Prior routing matrix: `str_outer_join 100000x5 = 39.726 ms/iter`,
  `str_left_join 100000x5 = 32.255 ms/iter`.
- The failed `.140` proof showed sharing nullable Utf8 gather plans was only
  noise-level, so this pass tested a deeper ordered-unique key primitive:
  validate ordered Int64 join keys through typed `&[i64]` slices instead of
  materializing `Scalar` views.

## Candidate

Temporary source hunk in `crates/fp-join/src/lib.rs` changed:

- `strictly_increasing_int64_key_values -> strictly_increasing_int64_key_slice`
- ordered-unique Int64 inner/left/right/outer position builders consumed
  `&[i64]` directly.

The hunk was removed after the exact target did not clear the keep gate. No
runtime source change is retained.

## Behavior proof

Golden dumps from current baseline and candidate matched exactly:

```text
ffcfd982367e807e2a0f7edcf3c66c1a3517c1d2c759eec1d15bae19bc3ff752  ordered_unique_left_5000
3da878cd8a0b40903116f5bc9d7e8687cf47af3d31d749f384d224582f9d9bce  ordered_unique_outer_5000
```

Diff checks:

```text
left_diff_exit=0
outer_diff_exit=0
```

This preserves row order, tie behavior, suffix/column order, null placement,
and deterministic values. The code path is RNG-free and only changes key
validation/materialization, not equality or ordering rules.

## Bench evidence

Focused ordered-unique proxy, local release-perf `fp-join` binary:

- Internal left: `0.684 -> 0.300 ms/iter`.
- Internal outer: `6.593 -> 4.962 ms/iter`.
- Hyperfine left: `228.6 ms +/- 9.4 -> 197.4 ms +/- 15.0`.
- Hyperfine outer: `661.1 ms +/- 17.2 -> 437.4 ms +/- 11.7`.

Exact original string-join target, same-host local release-perf `perf_profile`:

- `str_left_join 100000x5`: `33.692 -> 32.747 ms/iter`.
- `str_outer_join 100000x5`: `42.955 -> 43.581 ms/iter`.

The proxy showed the intended key-validation win, but the original target is
dominated by other join assembly costs and did not clear the Score >= 2.0 gate.

Score: Impact 1 x Confidence 3 / Effort 2 = 1.5. Rejected.

## Validation

- `cargo fmt -p fp-join -- --check`: pass.
- `rch exec -- cargo check -p fp-join --lib --bins`: pass, fell open locally
  (`no admissible workers`).
- `rch exec -- cargo clippy -p fp-join --lib --bins -- -D warnings`: pass, fell
  open locally.
- `rch exec -- cargo test -p fp-join ordered_unique_int64_subset_matches_generic_validated_route --lib`:
  pass, 4 tests.
- `ubs crates/fp-join/src/lib.rs`: nonzero due pre-existing broad inventory;
  fmt/clippy/check/test sub-gates were clean. Reported criticals were the
  existing dtype/key equality false positives outside this hunk.

RCH note: release-perf builds on `vmi1153651` went stale and were canceled, so
the final exact proof used isolated local crate-scoped builds.

## Next route

Do not retry scalar-view key validation or shared optional Utf8 gather plans.
The next primitive should attack the actual string join residual: a fused
shifted-unique output lane for 50%-overlap left/outer joins that emits nullable
Utf8/int payloads from overlap segments without per-column optional-position
materialization.
