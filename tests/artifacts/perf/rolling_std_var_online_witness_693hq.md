# br-frankenpandas-693hq rolling std/var online witness

## Target

- Bead: `br-frankenpandas-693hq`
- Crate: `fp-frame`
- Hot path: `Rolling::std` / `Rolling::var`
- Lever: replace per-window allocation plus two-pass variance with a monotonic-window online variance state using pandas-style `add_var` / `remove_var`, separate add/remove compensation, and exact-zero output for all-identical windows.

## Baseline

Command:

```text
rch exec -- cargo bench -p fp-conformance --bench vs_pandas -- rolling/std_w50
```

Worker: `vmi1227854`

```text
rolling/std_w50/rows/10000  time: [3.5803 ms 3.6612 ms 3.7444 ms]
rolling/std_w50/rows/100000 time: [36.882 ms 37.830 ms 38.788 ms]
```

## After

Command:

```text
ssh -i ~/.ssh/contabo_vps_ed25519 ubuntu@109.123.245.77 \
  'cd /data/projects/frankenpandas && cargo bench -p fp-conformance --bench vs_pandas -- rolling/std_w50'
```

Worker: `vmi1227854`

RCH worker affinity env hints (`RCH_WORKER=vmi1227854`, `RCH_WORKERS=vmi1227854`) were ignored by the installed `rch` binary. One selected worker (`vmi1152480`) lacks HDF5 headers, and a later affinity attempt selected a different worker before being stopped. The direct SSH run uses the same RCH worker and checkout path as the successful baseline.

```text
rolling/std_w50/rows/10000  time: [262.35 us 267.15 us 272.07 us]
rolling/std_w50/rows/100000 time: [2.7632 ms 2.8082 ms 2.8548 ms]
```

Delta:

```text
rows/10000:  3.6612 ms / 0.26715 ms = 13.70x faster
rows/100000: 37.830 ms / 2.8082 ms  = 13.47x faster
```

Score: Impact 5 x Confidence 5 / Effort 2 = 12.5, keep.

## Isomorphism Proof

- Window ordering and bounds: unchanged; the online state advances over existing `Rolling::window_bounds`, whose start/end are monotonic for trailing and centered windows.
- Missing semantics: unchanged; values enter the state only through the historical `is_missing()` then `to_f64().ok()` filter.
- Output surface: unchanged for `min_periods` vs `ddof`; below `min_periods` emits `Null(NaN)`, while a valid-but-ddof-insufficient window emits `Float64(NaN)`.
- Floating behavior: `var` follows pandas rolling variance with add/remove compensation; `std` applies pandas-style `zsqrt` negative clamp before square root. The previous `series_rolling_var_basic` golden was stale by one ULP; live pandas 2.2.3 returns `6.333333333333334`.
- RNG/tie-breaking: no RNG, sorting, or tie-ranking behavior is touched.

Golden SHA bundle:

```text
18c3ae3d892d709fb3291ca8d506de936996b1b11a30f6b974792da6a013006a  crates/fp-frame/tests/goldens/series_rolling_std_basic.txt
0f0da478e02043bda3490a6a30a355d6350685289742c8ea35135da3114b843f  crates/fp-frame/tests/goldens/series_rolling_var_basic.txt
729989241ae8774f8476f78eaddcb645fbd941077109afec7ec0cbce5c723969  crates/fp-frame/tests/goldens/dataframe_rolling_std_basic.txt
66c614eb841ada0c461b0b5a6c0cfd303065796b555b930cf8d599c22d62e810  crates/fp-frame/tests/goldens/dataframe_rolling_var_basic.txt
18c3ae3d892d709fb3291ca8d506de936996b1b11a30f6b974792da6a013006a  crates/fp-frame/tests/goldens/rolling_std_basic.txt
```

## Validation

Passed:

```text
rch exec -- cargo test -p fp-frame rolling_std_var_online_matches_naive_reference --lib -- --nocapture
rch exec -- cargo test -p fp-frame rolling_ --lib -- --nocapture
rch exec -- cargo check -p fp-frame --lib --tests
rch exec -- cargo clippy -p fp-frame --lib --tests -- -D warnings
git diff --check -- crates/fp-frame/src/lib.rs crates/fp-frame/tests/goldens/series_rolling_var_basic.txt
```

Known pre-existing blockers outside this lever:

```text
cargo check -p fp-frame --all-targets
error[E0599]: no method named `column_order` found in crates/fp-frame/examples/concat_bench.rs
```

```text
cargo fmt -p fp-frame -- --check
fails on pre-existing formatting drift in fp-frame examples and older unrelated src/lib.rs sections.
```

UBS note: `ubs crates/fp-frame/src/lib.rs crates/fp-frame/tests/goldens/series_rolling_var_basic.txt` produced no finding before stalling for more than two minutes in `ast-grep`; the scan was terminated and no child UBS processes remained.
