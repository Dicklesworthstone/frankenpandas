# fp-io csv_read_options profile ledger (br-frankenpandas-4ejqm)

## Scenario

- Workload: 100000 rows x 10 dense numeric CSV columns.
- Command shape: `perf_profile <scenario> 100000 20`.
- Metric: hyperfine wall time per 20 reads, lower is better.
- Build: `cargo build -p fp-conformance --profile release-perf --example perf_profile`.
- Target dir: `/data/tmp/cargo-target-fpio-csvread`.
- Host: Linux `thinkstation1` 6.17.0-29-generic, AMD Ryzen Threadripper PRO 5975WX 32-Cores, 64 logical CPUs.
- Toolchain: rustc 1.97.0-nightly (9ec5d5f32 2026-04-21), cargo 1.97.0-nightly (06ac0e7c0 2026-04-21).

## Baseline Table

| Rank | Location | Metric | Value | Category | Evidence |
|------|----------|--------|-------|----------|----------|
| 1 | `fp_io::parse_scalar_with_options` default numeric path | `csv_read_options` mean | 1.946492 s / 20 reads | CPU/alloc | `fp_csv_read_options_profile_before.json` |
| 2 | same parser with `na_filter=false` | mean | 1.849578 s / 20 reads | CPU/alloc | `fp_csv_read_options_profile_before.json` |
| 3 | simple `read_csv_str` parser | mean | 1.605946 s / 20 reads | baseline comparator | `fp_csv_read_options_profile_before.json` |

## Hypothesis Ledger

- NA membership scan dominates: rejects. `na_filter=false` still took 1.849578 s, only a partial recovery and still slower than `read_csv_str`.
- Options parser allocation dominates: supports. `parse_scalar_with_options` allocated a `String` for every numeric cell through `trimmed.to_owned()` when `thousands=None`, then cloned it again for `decimal='.'`. Replacing those with borrowed `Cow<str>` values reduced `csv_read_options` to 1.557213 s.
- CSV reader crate overhead dominates: rejects for this bead. The same csv crate and DataFrame materialization path are used before and after; only per-cell parser string ownership changed.

## After

| Scenario | Before mean | After mean | Delta | Evidence |
|----------|-------------|------------|-------|----------|
| `csv_read_options 100000 20` | 1.946492 s | 1.557213 s | 20.00% faster | `fp_csv_read_options_after_4ejqm.json` |

## Isomorphism

- Ordering and row/column labels are unchanged.
- Numeric parse order is unchanged: integer parse first, then float parse, then true/false and UTF-8 fallback.
- NA semantics are unchanged: `na_filter`, `keep_default_na`, and custom NA values are checked before numeric parsing as before.
- `thousands == decimal` still disables thousands stripping.
- Decimal replacement still happens only when `decimal != '.'`; the optimized path only avoids allocation when replacement would be a no-op.
- Floating-point strings are not rounded or transformed differently.
- RNG is not used.
- Golden output sha256 stayed `17f95925cec162c66933d5e935566b6f6fec5f8ef56872b98b00fb97ebf59fff`.
