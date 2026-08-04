# br-frankenpandas-uza04.48 proof: rejected lazy lower-hex range take

## Profile-backed target

Post-`uza04.47` full ordered-UTF8 join-loop profiling
(`/data/projects/.scratch/perf_uza0448_full_ordered_utf8.data`, summarized in
`tests/artifacts/perf/perf_report_before_uza0448_full_ordered_utf8.txt`) showed
`Column::take_contiguous_range -> utf8_arc_view_source ->
lower_hex_sequence_utf8_contiguous` under
`fp_join::build_single_key_inner_merge_output_with_selections`.

The tested lever preserved `LazyLowerHexSequenceUtf8` across contiguous range
takes by returning a shifted lazy lower-hex sequence instead of materializing
UTF8 bytes.

## Isomorphism proof

- Source row `range_start + i` and output row `i` both describe
  `prefix || fixed_width_lower_hex(start + range_start + i)`.
- Prefix bytes, hex width, all-valid null layout, row order, and tie behavior
  are unchanged.
- The lever does not touch floating-point values or RNG state.
- Normal and empty ordered-UTF8 golden outputs were byte-identical:
  - normal SHA before/after:
    `2ac49173153820d4b3878817c44be31979faa18b2ae034167f7977adee83b02e`
  - empty SHA before/after:
    `fc03a4635d1fe035e39a6f625acc9a3093dae0e9c61429a5a5c9742b146d0129`
  - `cmp -s` passed for both normal and empty artifacts.

## Benchmarks

All build commands used `rch exec --` with crate-scoped `fp-join` builds and
isolated target dirs. RCH workers failed preflight and the wrapper fell open
locally.

Direct internal join timer, 5M/5M ordered UTF8, 200 measured merge iterations:

- before: `mean_ms=4.411`, `p50_ms=4.152`, `p95_ms=6.163`, `p99_ms=7.569`
- after:  `mean_ms=4.551`, `p50_ms=4.296`, `p95_ms=6.576`, `p99_ms=7.563`

Paired hyperfine, same command:

- before: `1.540 s +/- 0.042` (user `0.659`, system `0.879`)
- after:  `1.606 s +/- 0.086` (user `0.687`, system `0.915`)
- result: before ran `1.04x +/- 0.06` faster than after.

Score: Impact 0.0 x Confidence 0.90 / Effort 1.0 = 0.0. Reject.

## Outcome

The code lever was removed before commit. The retained artifacts document the
profile-backed target, behavior proof, and benchmark rejection. The next pass
should avoid this range-take micro-family and attack a deeper primitive.
