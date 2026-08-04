# br-frankenpandas-uza04.100 sorted dense string-groupby order without gid remap

## Target

The prior `br-frankenpandas-uza04.96` rejection showed that the realistic
string-groupby lane needed the benchmark shape corrected before product work.
Commit `50779feb` fixed the timed `str_groupby_*` scenarios to use repeated
string keys, and `900e5184` added short fixed-width key interning while this
lever was being rebased. This proof compares against that current `origin/main`
baseline and targets the remaining sorted dense contiguous-Utf8 path in
`DataFrameGroupBy::aggregate_str_dense`.

## One lever

In the `self.sort && !near_all_distinct` branch, grouping already assigns
`gid_per_row` in first-seen dense-gid space and sorts only the distinct key
spans. The old code then:

- allocated `seen_to_sorted`,
- remapped every row in `gid_per_row`, and
- allocated `reps_sorted`.

This pass removes that O(n) remap. Accumulators stay indexed by first-seen
group id, and only final output iteration uses the sorted distinct-key
permutation.

## Isomorphism proof

- Group membership is unchanged: the short packed-key and borrowed-span
  interning branches still map the exact same UTF-8 byte spans to the same
  first-seen dense gids.
- Output ordering is unchanged: `order` is the same `utf8_msd_argsort_bytes`
  permutation of distinct key spans that previously populated
  `seen_to_sorted`.
- Tie-breaking is unchanged: equal keys share one dense gid; distinct output
  keys are ordered by the same byte comparator as before.
- Floating-point behavior is unchanged: sum/mean/var/std scans the original
  rows in the same order for each first-seen gid. Only final result collection
  order changes through `order`.
- First/last are unchanged: row-order scans still write to the same first-seen
  gid for each group, and final collection reads that gid in sorted output
  order.
- RNG and benchmark input generation are unchanged in this product pass.

## Golden-output SHA256

`tests/artifacts/perf/uza04100_order_rebased_base_golden_hash_rows.txt` and
`tests/artifacts/perf/uza04100_order_rebased_candidate_golden_hash_rows.txt`
compare byte-identically (`cmp` exit 0) for 20 rows:

```text
77a2baa488f3b39462a15e4cc2223a884afb0d48e088b2ed14b0c2138f8a1894  str_groupby_count 1000
d5cea8c5844bb962c8958f0a94d53a9e7d6b452a55ccaa51c5addb6000c0c46e  str_groupby_count 200000
eb1a74a5e85a12a37abe8d9890bdac88a38cb014ac8f6e3e3af330bc3e83eb9c  str_groupby_mean 1000
f71ecdabae351a6f948a263320ac0175026aed4e5fc07c770e0a6779ab951e18  str_groupby_mean 200000
33cf06ed6e0f6604c0fb7fb7c91f029e53cf80570c5a987ad4f4382d2e561343  str_groupby_std 1000
72e4b0378ad8409249711f58264e5c4cd64109f5c6f9d684d5e2c419da22a5f5  str_groupby_std 200000
a50867ca5082eda67016cc30fafb090f0ae56714aaae79d25e1555cb2d04ef2a  str_groupby_min 1000
ed44e2fb150da156c08154fb9ba42aef344e86eb1284ebc7fa0446b2cf5f8953  str_groupby_min 200000
dbf2d6fbfcdc5a6baa83dee36fb3263eeebc5ce61b8710ca938dd86654e4f1ff  str_groupby_max 1000
2098a1f62f78efa36a7935f797442347482f5eba340e0f8edc46b87ce65c796d  str_groupby_max 200000
897b2cb2c8da5cf47a549f63c95737a38a6cb2772417defd0443e8e8f4255a1d  str_groupby_first 1000
897b2cb2c8da5cf47a549f63c95737a38a6cb2772417defd0443e8e8f4255a1d  str_groupby_first 200000
6e9816976c37f8cadc3fa3e9e272945f9857eff8342c788c4f4000c91575fd5d  str_groupby_last 1000
c6ae8bdfb1b1fc113d6de9b5e88963efc7aef3173b14ebfe5af92fedcd9e4465  str_groupby_last 200000
a67aa6d00ae06b5aa85fda4bd91603591c6b6d40ebb7d7739dbc6fa686dcabde  str_groupby_prod 1000
7c799d5e62d7f1bf3d56159f97b941e2e0b4648f6ba1b59ae404d2628747f579  str_groupby_prod 200000
e5b2695b0ec58907aa9fee5a01c8e89039c21376895e240f6141b8c9f52183ae  str_groupby_median 1000
48bd9456876f60620753b341c2ea374eeafbebc352cafa4cf8fbbca8f5491d26  str_groupby_median 200000
a53b6ca20edea8eafabefe76ee5c12bd98d116d02d6372a93707fdc258f1c80d  str_groupby_sum 1000
a28f5534bd1a01e792695b5f80aa877724803caff9bbed0b6fc86807e20b5412  str_groupby_sum 200000
```

Note: the legacy `str_groupby_sum` golden helper still uses the older
broad-cardinality fixture, while timed `str_groupby_sum` uses the corrected
repeated-key fixture from `50779feb`. Count/mean/std/min/max/first/last/prod
and median goldens directly exercise the corrected repeated-key path.

## Benchmark

Baseline build:

```text
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-lavenderstone-uza04100-rebased-base
RUSTFLAGS='-C force-frame-pointers=yes'
rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
```

RCH had no admissible worker for the rebased baseline build and failed open
locally, but the build remained crate-scoped. Candidate build:

```text
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-lavenderstone-uza04100-rebased-order
RUSTFLAGS='-C force-frame-pointers=yes'
rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
```

Candidate warning-clean build passed on `vmi1153651`.

Paired high-iteration hyperfine on the same local runner, after rebasing over
`900e5184` short-key interning:

```text
str_groupby_count 200000x100: 177.5 ms +/- 5.0 -> 148.1 ms +/- 5.0 (1.20x)
str_groupby_sum   200000x100: 184.5 ms +/- 9.7 -> 163.0 ms +/- 8.0 (1.13x)
str_groupby_std   200000x100: 192.9 ms +/- 8.6 -> 176.6 ms +/- 8.8 (1.09x)
```

Short-iteration routing run:

```text
str_groupby_count 200000x6:        43.3 ms +/- 10.5 -> 32.7 ms +/- 3.0
str_groupby_sum   200000x6:        41.8 ms +/-  7.3 -> 33.1 ms +/- 2.0
str_groupby_std   200000x6:        38.0 ms +/-  7.6 -> 33.5 ms +/- 1.6
str_groupby_count_lowcard 200000x20: 81.4 ms +/- 13.5 -> 59.4 ms +/- 2.9
str_groupby_sum_lowcard   200000x20: 85.4 ms +/-  8.1 -> 61.8 ms +/- 2.9
```

The rebased paired high-iteration gate is the keep evidence. The older
short-iteration and pre-rebase high-iteration runs remain as historical routing
context only.

Score: Impact 2.5 x Confidence 4 / Effort 1.5 = 6.7, KEEP.

## Validation

- Post-rebase `rch exec -- cargo check -p fp-frame --lib`: passed on
  `vmi1153651`.
- Post-rebase `rch exec -- cargo clippy -p fp-frame --lib -- -D warnings`:
  passed on `vmi1227854`.
- Post-rebase `rch exec -- cargo test -p fp-frame groupby_sum_string_column_concatenates_6lnll --lib`:
  RCH had no admissible worker and failed open locally; focused test passed.
- Pre-rebase focused `dataframe_groupby_dense_float_agg_matches_reference`:
  passed.
- `cargo fmt -p fp-frame --check`: reports pre-existing formatting drift at
  `crates/fp-frame/src/lib.rs:41731`, outside this hunk. No unrelated
  formatting was changed.
- `git diff --check`: passed before proof artifact creation.
- `ubs crates/fp-frame/src/lib.rs`: attempted with a 600s cap and recorded
  `UBS_EXIT=124`; the scan timed out after startup output and produced no
  finding summary.
- `perf stat`: blocked by `perf_event_paranoid=4`; status captured in
  `tests/artifacts/perf/uza04100_product_order_perf_stat_status.txt`.

## Verdict

Productive keep. The corrected low-cardinality string groupby branch now avoids
one full per-row gid remap and one extra representative-vector rewrite while
preserving output order, row-order floating-point accumulation, first/last
semantics, and golden hashes.
