# Lane M datetime `dt_strftime` live-incumbent gate — 2026-07-28

## Outcome

`Series.dt.strftime("%Y-%m-%d")` is a decisive FrankenPandas incumbent win at
both requested scales under the schema-v4 contract.

**Campaign result class:** `incumbent-win`.

| size | FP p50 | pandas 2.2.3 p50 | ratio | log effect / required | verdict |
|---:|---:|---:|---:|---:|---|
| 1M | 76.239 ms | 991.039 ms | **12.999x** | 2.56487794 / 0.12830703 | FASTER |
| 10M | 1,256.212 ms | 11,130.928 ms | **8.861x** | 2.18162684 / 1.08638672 | FASTER |

The two-row geomean is **10.732x**. Absolute median time saved grows from
914.800 ms at 1M to 9.875 seconds at 10M, but the ratio contracts by 31.8%.
This is a strong live-incumbent win, not a ratio-amplifying Class-1 result on
this ELF.

## Exact comparison boundary

Both engines receive the identical `datetime64[ns]` sequence:

```text
base = 946684800000000000  # 2000-01-01T00:00:00
value[i] = base + i * 600000000000  # 600 seconds
```

The final values are `1546684200000000000` at 1M rows
(`2019-01-05T10:30:00`) and `6946684200000000000` at 10M rows
(`2190-02-17T10:30:00`), both inside pandas' nanosecond range. Setup is
outside the timed region. The timed operations are:

- FrankenPandas: `series.dt().strftime("%Y-%m-%d")`
- pandas 2.2.3: `series.dt.strftime("%Y-%m-%d")`

Both produce one `%Y-%m-%d` string per row. The same-named pandas `.dt.date`
and `.dt.time` methods remain excluded because they produce Python
`date`/`time` object arrays rather than the FrankenPandas Utf8 result.

## Harness contract

- Worker: `vmi1264463`
- Invocation:
  `vs-pandas-20260729T003944.972139Z-pid3276902`
- Paired rounds: 25 per engine and row, after three warmups
- Gate: bootstrap 95% median CI with 2x combined-null margin
- CV: provenance only; it had no vote
- FrankenPandas in-process ELF SHA-256:
  `ed445e842c6ef4fffc19dc2e6ae047cc9c7984ef49a546df27c2f676ccff62d4`
  (70,294,592 bytes)
- Python executable SHA-256:
  `efb29ce53d36ebaeee80e3aa44fd6c7f9d71bbded5fe1665240b2ed8ecaeee0e`
- pandas 2.2.3 artifact SHA-256:
  `051be80fe43b4e0be4e04af314c42db966950eb877b0634d482099f42535e9bb`
- Harness source SHA-256:
  `f3e5dc5ca59d974654b39d32faccb22bd4b1d5c565f750db3a4b4e19e61bba17`

The 1M FrankenPandas/pandas A/A median CIs were
`[0.956368,1.052630]` / `[0.937861,1.025801]`; their combined 2x interval was
`[0.879583,1.136902]`. The 10M CIs were
`[0.580890,1.660819]` / `[0.922031,1.023460]`; their combined 2x interval was
`[0.337434,2.963547]`. Both median-CI effects clear their own numeric
thresholds even
though 10M FrankenPandas CV was 58.77%.

Strict-remote command:

```text
RCH_WORKER=vmi1264463 RCH_REQUIRE_REMOTE=1 env -u CARGO_TARGET_DIR \
  rch exec -- cargo run --locked --profile release-perf -p fp-bench -- \
  --remote-python-harness --category datetime --sizes 1M,10M \
  --dtypes float64 --workloads dt_strftime \
  --output artifacts/bench/proud_lane_m_dt_strftime_1m_10m_20260728.json \
  --json-stdout
```

## Validation

- Raw schema-v4 contract self-check: PASS
- Python input endpoint identity at 1M and 10M: PASS
- `cargo check --workspace --all-targets`: PASS, strict remote
- `cargo clippy -p fp-bench --all-targets --no-deps -- -D warnings`: PASS,
  strict remote
- `cargo test -p fp-frame dt_strftime --lib`: PASS, 2/2, strict remote
- `rustfmt --check --edition 2024 crates/fp-bench/src/main.rs`: PASS
- Python harness AST parse: PASS

The mandated workspace formatter reports unrelated pre-existing drift. The
mandated workspace clippy reaches `fp-columnar` first and fails on 25
pre-existing warnings; the focused owned-crate clippy above is clean.

Raw artifact:
`artifacts/bench/proud_lane_m_dt_strftime_1m_10m_20260728.json`, SHA-256
`6f3c6ac279221851d345ecf159e48e24377f8c3ad3fdd867a669cd1abd4c4aa3`.

## Concrete retry predicates

Keep the incumbent-win classification until the workload format, datetime
sequence, FrankenPandas implementation, harness source, pandas artifact,
allocator, compiler, worker ISA, or executing ELF changes. Re-open a
ratio-growth claim only when the same self-identified ELF runs 1M and 10M on
one worker in one invocation and two clean repeats place the 10M ratio above
the 1M ratio outside the combined A/A intervals. Do not use `.dt.date` or
`.dt.time` as the incumbent unless both engines first expose the same logical
and physical output contract.
