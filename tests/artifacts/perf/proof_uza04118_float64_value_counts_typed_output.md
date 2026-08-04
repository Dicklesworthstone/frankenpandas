# br-frankenpandas-uza04.118 proof: Float64 value_counts typed output

## Change

Default `Series::value_counts()` for all-valid `Float64` columns now carries a
typed `Float64ValueCount { canonical_bits, value, count }` record from the raw
bit tally through output materialization. The path still uses the same
first-seen raw-bit FxHashMap tally and stable descending count sort, but it now
constructs `IndexLabel::Float64(OrderedF64(value))` and `Scalar::Int64(count)`
directly instead of round-tripping each distinct bucket through
`(Scalar::Float64, usize)` and the generic scalar-to-label mapper.

All non-Float64, categorical, nullable/NaN, and optionful
`value_counts_with_options` paths still use the existing generic materializer.

## Hotspot evidence

Prior `br-frankenpandas-uza04.103` and `br-frankenpandas-8sxez` evidence left
Float64 `value_counts 100k` in the typed tally plus output materialization path.
`br-frankenpandas-uza04.117` rejected the all-tie sort guard, leaving this
typed-output boundary as the next non-duplicate residual.

`perf stat` remained blocked by kernel policy:

```text
perf_event_paranoid setting is 4
```

## Mapped primitive

- Alien graveyard: Swiss-table/cache-local hash payload separation and
  vectorized columnar execution guidance. This change does not replace the
  hash table; it removes a generic AoS `Scalar` payload boundary after the
  measured typed tally.
- No-gaps directive: frankenpandas should keep data in contiguous typed
  columnar forms rather than falling back to `Vec<Scalar>` for hot paths.

## Isomorphism proof

- Ordering preserved: yes. The generic path and typed path both sort by
  descending count with Rust stable sort; equal counts retain first-seen row
  order.
- Tie-breaking unchanged: yes. The typed path records the first distinct bucket
  at the same row as the previous `Scalar::Float64` push.
- Floating point unchanged: yes. The hash key still canonicalizes `-0.0` and
  `+0.0` to key bits `0`, and the output label keeps the first-seen `f64`
  payload bits. NaN is unreachable through `as_f64_slice()` and remains on the
  existing generic/null path.
- RNG unchanged: N/A.
- Null/NaN behavior unchanged: yes. The new path is gated by all-valid
  `as_f64_slice()`. Missing Float64 values keep the existing fallback.

## Golden verification

```text
5c55febe9b1f6dd9a7f828800fae9e3389b056f30ae3bd116468806f32faf512  tests/artifacts/perf/uza04118_base_vc_float_golden.txt
5c55febe9b1f6dd9a7f828800fae9e3389b056f30ae3bd116468806f32faf512  tests/artifacts/perf/uza04118_after_vc_float_golden.txt
sha256sum -c tests/artifacts/perf/uza04118_base_vc_float_golden.sha256: OK
```

## Benchmark delta

`fp-bench dataframe_ops/value_counts 100k float64`, 25 internal measured
iterations:

| Metric | Before | After | Speedup |
| --- | ---: | ---: | ---: |
| mean | 12906.656 us | 10140.209 us | 1.27x |
| p50 | 12842.687 us | 10007.629 us | 1.28x |
| p95 | 17471.081 us | 12666.288 us | 1.38x |
| p99 | 19777.177 us | 13715.507 us | 1.44x |

Full-process hyperfine, 10 runs:

| Metric | Before | After | Speedup |
| --- | ---: | ---: | ---: |
| mean | 297.708 ms | 254.378 ms | 1.17x |
| min | 283.488 ms | 242.175 ms | 1.17x |

Score: Impact 3 x Confidence 4 / Effort 1 = 12.0. Keep.

## Validation

- `rch exec -- cargo test -p fp-frame series_value_counts_float_typed_materializer_preserves_zero_and_ties_uza04118 -- --nocapture`: passed.
- `rch exec -- cargo test -p fp-frame series_value_counts -- --nocapture`: 20 passed on `vmi1227854`.
- `rch exec -- cargo check -p fp-frame --all-targets`: passed on `vmi1227854`.
- `rch exec -- cargo clippy -p fp-frame --all-targets -- -D warnings`: passed on `vmi1227854`.
- `cargo fmt -p fp-frame -- --check`: failed on pre-existing unrelated formatting drift in fp-frame examples and unrelated `lib.rs` regions; no new value-counts hunk appeared in rustfmt output.
- `git diff --check`: passed.
- `timeout 180 ubs crates/fp-frame/src/lib.rs`: timed out with exit 124 before emitting findings.

## Rollback

Revert the commit carrying this proof and source hunk. The change is isolated to
the default all-valid Float64 `Series::value_counts()` output materializer and a
focused unit test.

## Next residual

Reprofile/timing residual after this keep points back to the raw-bit FxHashMap
tally/cache-miss phase, not more scalar output materialization. Next candidate
should attack typed Float64 tally layout or bucket reuse, not the rejected
all-tie sort guard.
