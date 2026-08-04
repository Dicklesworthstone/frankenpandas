# br-frankenpandas-uza04.81 Pass 1 Baseline/Profile Witness

- Head: `217618ecfd25`
- Bead: `br-frankenpandas-uza04.81`
- Status check: `in_progress`, assigned to `OrangePeak`
- Scope: baseline/profile only for shifted CSV Float64 residual after `.80`; no source edits.

## Commands

Build:

```bash
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0481-base RUSTFLAGS='-C force-frame-pointers=yes' rch exec -- cargo build -p fp-conformance --profile release-perf --bin high_ram_perf_baseline
```

Baseline JSON:

```bash
/data/projects/.scratch/cargo-target-orangepeak-uza0481-base/release-perf/high_ram_perf_baseline --profile uza0481-base --rows 100000 --iters 20 --warmup 3 --frame-cols 10 --key-cardinality 100000
```

Stable witness:

```bash
jq -S '.workloads[] | select(.name=="csv_write_read_roundtrip") | {checksum, io_payload_bytes, name, rows_out}' tests/artifacts/perf/uza0481_base_high_ram_keycard100000_100000x20.json
sha256sum tests/artifacts/perf/uza0481_base_csv_write_read_roundtrip_stable.json
sha256sum -c tests/artifacts/perf/uza0481_base_csv_write_read_roundtrip_stable.sha256
```

Hyperfine:

```bash
hyperfine --warmup 2 --runs 10 --export-json tests/artifacts/perf/uza0481_base_hyperfine_highram_keycard100000_100000x20.json '/data/projects/.scratch/cargo-target-orangepeak-uza0481-base/release-perf/high_ram_perf_baseline --profile uza0481-base-hyperfine --rows 100000 --iters 20 --warmup 3 --frame-cols 10 --key-cardinality 100000'
```

Perf:

```bash
perf record -F 999 --call-graph fp -o tests/artifacts/perf/uza0481_base_perf_highram_keycard100000_100000x20.data -- /data/projects/.scratch/cargo-target-orangepeak-uza0481-base/release-perf/high_ram_perf_baseline --profile uza0481-base-perf --rows 100000 --iters 20 --warmup 3 --frame-cols 10 --key-cardinality 100000
perf report --stdio --no-children -i tests/artifacts/perf/uza0481_base_perf_highram_keycard100000_100000x20.data
perf report --stdio --children -i tests/artifacts/perf/uza0481_base_perf_highram_keycard100000_100000x20.data
perf report --stdio --children --call-graph=graph,0.5,caller -i tests/artifacts/perf/uza0481_base_perf_highram_keycard100000_100000x20.data
```

## RCH/Host Caveat

- Build offloaded through RCH to remote worker `vmi1227854`; RCH reported `[RCH] remote vmi1227854 (237.4s)`.
- The retrieved release-perf binary was benchmarked and profiled locally from `/data/projects/.scratch/cargo-target-orangepeak-uza0481-base/release-perf/high_ram_perf_baseline`.
- `perf report` warned that kernel address maps were restricted, so kernel samples are unresolved; user-space Rust symbols for the CSV residual are present.

## csv_write_read_roundtrip

- mean: `86.04315275 ms`
- p50: `86.642249 ms`
- p95: `92.795965 ms`
- p99: `94.571176 ms`
- throughput rows/sec: `1162207.5296398294`
- throughput bytes/sec: `100081990.54514538`
- io payload bytes: `8611370`
- rows_out: `100000`
- checksum: `62503875000.0`
- stable SHA256: `d93c23c308b1cbba82051d7dd022001e84e5313c0b485a440891a604ebfce367`
- SHA check: `tests/artifacts/perf/uza0481_base_csv_write_read_roundtrip_stable.json: OK`

Stable witness JSON fields:

```json
{
  "checksum": 62503875000.0,
  "io_payload_bytes": 8611370,
  "name": "csv_write_read_roundtrip",
  "rows_out": 100000
}
```

## Hyperfine

- command mean: `3.73404179634 s`
- stddev: `0.21611290229476543 s`
- min: `3.4801639470400003 s`
- max: `4.2702302760399995 s`
- runs: `10`
- warmup: `2`

## Profile Rows

Children report:

- `ryu::pretty::format64`: `32.10%` children / `23.30%` self.
- `fp_io::write_pandas_float`: `8.32%` children / `5.36%` self.
- `fp_io::write_csv_string_with_options`: `3.85%` children / `1.63%` self.
- `<fp_frame::DataFrameGroupBy>::aggregate_named_func`: `15.93%` children / `1.24%` self.

No-children report:

- `ryu::pretty::format64`: `23.30%` self.
- `fp_io::write_pandas_float`: `5.36%` self.
- `<core::hash::sip::Hasher<core::hash::sip::Sip13Rounds> as core::hash::Hasher>::write`: `11.07%` self, mostly under groupby.
- `malloc_consolidate`: `5.73%` self, under groupby allocator cleanup.

## Verdict

PRODUCTIVE baseline. The `.80` shift is reproduced on the fresh `.81` baseline: `ryu::pretty::format64` remains the dominant CSV Float64 residual, with `fp_io::write_pandas_float` still visible beneath it. Pass 2 should select and score a deeper columnar/arithmetic CSV Float64 formatting primitive against this profile, without repeating the generic Ryu swap or touching groupby allocator work in this bead.
