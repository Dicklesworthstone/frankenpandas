# br-frankenpandas-uza04.146 proof: finite Float64 witness for lazy df_dot

## Target

- Profile source: `tests/artifacts/perf/lavender_next_profile_matrix_20260616_pass6.txt`
- Hotspot: `perf_profile df_dot 100000 1`
- Baseline matrix timing: `250.880 ms/iter`, highest scored current profile entry.
- Lever: carry an all-finite Float64 witness from typed Float64 column construction and let `DataFrame::dot` consume that witness instead of rescanning every all-valid Float64 source column for finiteness.

## Isomorphism

- Row order: unchanged. `DataFrame::dot` still builds result columns in `other.column_order` and keeps `self.index`.
- Column order: unchanged. The fast path only replaces the finiteness proof, not result insertion order.
- Floating-point arithmetic: unchanged. `Column::from_f64_all_valid_dot_product` still materializes each cell with the same `l = 0..k` left fold and the same operands.
- Null/NaN/inf behavior: unchanged. NaN-bearing Float64 columns are not all-valid and do not expose the finite Arc view; infinity remains valid Float64 data but rejects the finite dot fast path and falls back to the existing eager path.
- RNG/tie-breaking: not applicable.

## Golden

- Baseline golden: `04af7c2bb0e772d23ed69b3733da0778c3693ba1e67557c0126fcbd4458fdb3d`
- Candidate golden: `04af7c2bb0e772d23ed69b3733da0778c3693ba1e67557c0126fcbd4458fdb3d`
- Artifact: `tests/artifacts/perf/lavender_uza04146_candidate_golden_df_dot_5000.sha256`

## Benchmarks

Baseline standalone:

- `df_dot 100000 1`: `231.3 ms +/- 8.7 ms`

Paired `df_dot 100000 1`:

- Forward: baseline `251.9 ms +/- 22.9 ms`, candidate `231.8 ms +/- 10.5 ms`, `1.09x`
- Reversed: candidate `230.8 ms +/- 10.7 ms`, baseline `242.3 ms +/- 10.4 ms`, `1.05x`

Paired repeated-dot `df_dot 100000 5`:

- Forward: baseline `305.4 ms +/- 8.0 ms`, candidate `237.7 ms +/- 9.7 ms`, `1.28x`
- Reversed: candidate `238.3 ms +/- 3.6 ms`, baseline `297.8 ms +/- 8.5 ms`, `1.25x`

Score: keep. Impact `3` (1.25x repeated-dot win), confidence `0.85` (golden exact, paired forward/reverse), effort `1`, score `2.55`.

## Gates

- `cargo test -p fp-columnar finite_float64_arc_view_uses_constructor_witness --lib`: pass.
- `cargo check -p fp-frame --all-targets`: pass.
- `cargo clippy -p fp-frame --lib -- -D warnings`: pass.
- `cargo clippy -p fp-frame --all-targets -- -D warnings`: blocked by unrelated existing `crates/fp-frame/examples/golden_value_counts_f64.rs` `unnecessary_sort_by` lint.
- `cargo fmt --check`: workspace has unrelated pre-existing formatting drift; touched files pass `rustfmt --edition 2024 --check crates/fp-columnar/src/lib.rs crates/fp-frame/src/lib.rs`.
- `ubs crates/fp-columnar/src/lib.rs crates/fp-frame/src/lib.rs`: interrupted after several minutes with no findings emitted.
- `perf record`: blocked by `perf_event_paranoid=4`.
