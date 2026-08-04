# br-frankenpandas-uza04.134 df_dot worker-cap rejection proof

Agent: LavenderStone
Date: 2026-06-14

## Target

`df_dot 100000x5` remained the largest measured residual after the accepted
parse-dates fast path and after `br-frankenpandas-uza04.133` rejected simple
packed-panel widening.

## Profiling Attempt

Focused `perf record` was attempted:

`perf record -F 99 -g -o tests/artifacts/perf/lavender_uza04134_df_dot_perf.data -- /data/tmp/cargo-target/release-perf/examples/perf_profile df_dot 100000 3`

The host rejected perf events:

- `perf_event_paranoid setting is 4`
- access requires lower paranoid level or CAP_PERFMON/CAP_SYS_PTRACE/CAP_SYS_ADMIN

`samply record --save-only` was also attempted and failed for the same reason:

- `/proc/sys/kernel/perf_event_paranoid` must be `1` or lower for non-root samply.

## Candidate Lever

Changed only the row-band worker cap:

- Before: `min(64)` workers
- Candidate: `min(24)` workers

Isomorphism argument:

- Row bands remain disjoint and are sorted by starting row before output assembly.
- Each output cell keeps the same `l = 0..k` left fold and the same operands.
- Column order, row index, NaN witness behavior, and all-valid chunk assembly are unchanged.

## Golden Proof

Baseline sha256:

- `df_dot 2000`: `ddbde1c39c4856c19700fe90a29f6acce2a742a98a298585f896b0b02cdbb535`
- `df_dot 5000`: `04af7c2bb0e772d23ed69b3733da0778c3693ba1e67557c0126fcbd4458fdb3d`

Candidate sha256:

- `df_dot 2000`: `ddbde1c39c4856c19700fe90a29f6acce2a742a98a298585f896b0b02cdbb535`
- `df_dot 5000`: `04af7c2bb0e772d23ed69b3733da0778c3693ba1e67557c0126fcbd4458fdb3d`

## Timing

Internal timing:

- Baseline: `131.904 ms/iter`
- Candidate: `130.674 ms/iter`

Hyperfine, 7 runs, command `/data/tmp/cargo-target/release-perf/examples/perf_profile df_dot 100000 5`:

- Baseline: `701.7 ms +/- 14.3 ms`
- Candidate: `707.3 ms +/- 17.6 ms`

Result: rejected. The tiny internal difference is noise-level and hyperfine
does not show a real win. Source hunk removed before commit.

Next df_dot route should skip worker-cap and panel-width tuning and target a
different primitive, such as changing the output computation layout or exposing
a more specialized benchmark/kernel for row-major packed A bands.
