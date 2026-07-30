# Float64 telemetry display strings, 1M and 10M rows

**Bead:** `br-frankenpandas-vhp7d`
**Result class:** REALISTIC WORKLOAD WIN
**Decision:** KEEP the bounded telemetry-string benchmark surface and its
evidence. No production kernel changed in this bead.

## Result

One canonical same-worker invocation compared FrankenPandas with the fastest
task-equivalent pandas/NumPy route found by a nine-route screen. Both engines
formatted every value, materialized complete string Series, preserved the
global `RangeIndex` and Series name, observed total cardinality and endpoints,
and destroyed each result before advancing to the next 250,000-row batch.

| rows | FrankenPandas p50 / p95 / p99 | pandas+NumPy p50 / p95 / p99 | p50 ratio | absolute p50 saving | actual threads FP / incumbent |
|---:|---:|---:|---:|---:|---:|
| 1M | 49.975 / 55.757 / 60.546 ms | 205.205 / 241.198 / 258.769 ms | **4.106x** | 155.230 ms | 8 / 1 |
| 10M | 510.755 / 677.284 / 994.424 ms | 1,952.351 / 2,160.928 / 2,265.398 ms | **3.822x** | **1,441.596 ms** | 8 / 1 |

The two-row geomean is 3.961x. The ratio narrows from 1M to 10M; it does
not widen. The useful large-N effect is the absolute gap: it grows 9.287x,
from 155.230 ms to 1.442 s.

Canonical invocation:
`vs-pandas-20260730T034525.887023Z-pid1518869`.
Raw schema-v4 artifact:
`proud_lane_m_astype_str_f64_realistic_1m_10m_20260729.json`,
SHA-256
`257ef3dc7015976ac291cf2179e219405a7ba41cc573abe36b186f49c8b33b10`.

## Exact workload

- Rows: 1,000,000 and 10,000,000.
- Values: all-valid finite `Float64`, `value[i] = i * 1.5`.
- Labels: global `RangeIndex(0, rows)`.
- Series name: `s`.
- Sink: ordered, complete string Series consumed in 250,000-row batches.
- Observations per batch: cardinality, first string, last string, then drop.
- Expected whole-stream endpoints: `0.0` and `1499998.5` at 1M;
  `0.0` and `14999998.5` at 10M.
- Timed work per engine and size: 25 alternating A/A pairs, or 50 complete
  stream conversions. The untimed thread probe and three warmups execute the
  same stream contract in both engines.

The values are exact binary integers or half-integers. For this fixture only,
`"{:.1f}".format` is exactly equal to pandas' shortest Float64 spelling. That
equivalence does not hold for arbitrary Float64 data.

## Incumbent isolation proof

**Subject:** FrankenPandas `Series::astype(DType::Utf8)` over each prebuilt
250,000-row Float64 Series. Each output's first and last values and length are
observed, then the complete output is dropped inside the timed closure.

**Incumbent:** pandas 2.2.3 Series constructed from NumPy 2.4.3
`np.frompyfunc("{:.1f}".format)` output, with the same batch index and name.
Each complete object-dtype output has the same observations and is deleted
inside the timed operation.

The final worker screen ran seven interleaved samples per route at 1M rows.
All nine routes were exactly equal to direct `Series.astype(str)` in every
value, object dtype, index, name, cardinality, and endpoints.

| task-equivalent route | median |
|---|---:|
| `np.frompyfunc("{:.1f}".format)` + Series | **168.554 ms** |
| `np.frompyfunc(str)` + Series | 190.744 ms |
| `Series.transform(str)` | 219.202 ms |
| `Series.map(str)` | 220.342 ms |
| `Series.apply(str)` | 237.962 ms |
| NumPy `astype(str)` + Series | 390.377 ms |
| direct `Series.astype(str)` | 460.534 ms |
| `np.char.mod("%.1f", ...)` + Series | 477.775 ms |
| Arrow string then object | 596.663 ms |

Winner samples in milliseconds:
`[179.961154, 165.392709, 168.553728, 163.601903, 164.238512,
185.928349, 206.935869]`.

Direct pandas samples in milliseconds:
`[460.534289, 416.506116, 430.578754, 511.002007, 440.812734,
545.251896, 502.522869]`.

Direct `Series.astype(str)` is therefore a diagnostic secondary comparator,
not the incumbent headline. The fixed-format NumPy ufunc route is 2.732x
faster on the exact screened task and is the named incumbent.

## Median-CI adjudication

CV is recorded only as provenance and had no vote.

| rows | FP A/A median; bootstrap 95% CI | incumbent A/A median; bootstrap 95% CI | claim log effect | required 2x-null effect | decision |
|---:|---:|---:|---:|---:|---|
| 1M | 0.996454; `[0.878649, 1.073213]` | 0.983506; `[0.946530, 1.028709]` | 1.41248836 | 0.25873895 | decidable FASTER |
| 10M | 0.996087; `[0.958732, 1.015645]` | 1.016231; `[0.988441, 1.042336]` | 1.34089870 | 0.08428759 | decidable FASTER |

FP/incumbent CVs were 11.01%/10.64% at 1M and 20.61%/6.23% at 10M.
The bootstrap median-CI gate, not CV, decides both rows.

## Lifecycle and exclusivity

The first valid lifecycle design used a 250,000-row bounded buffer so no
monolithic 1M/10M object Series survives the operation. The linked mimalloc v2
normally delays page purges. The admitted FrankenPandas arm sets
`MIMALLOC_PURGE_DELAY=0`, which makes purging immediate when the timed drop
makes a page unused. This charges the cleanup at the semantic free boundary;
it does not add a post-arm grace period. The immediate post-arm gate remains
unchanged.

The invocation recorded ten clear all-online-CPU observations: invocation
preflight/postflight plus pre/post checks for each engine at both sizes. The
threshold was 20% busy per CPU over a one-second sample. The maximum observed
busy fraction in the admitted invocation was 8.081%.

Earlier attempts or superseded evidence were excluded in full:

| invocation | contract | exclusion reason |
|---|---|---|
| `vs-pandas-20260730T023236.381568Z-pid1404329` | monolithic | 10M pandas post-arm |
| `vs-pandas-20260730T024000.100995Z-pid3594041` | monolithic | 1M pandas pre-arm |
| `vs-pandas-20260730T024401.406650Z-pid3598467` | monolithic | invocation preflight |
| `vs-pandas-20260730T024438.679278Z-pid3599598` | monolithic | 1M pandas post-arm |
| `vs-pandas-20260730T030529.205768Z-pid3622002` | 250k batches | invocation preflight |
| `vs-pandas-20260730T030629.331233Z-pid3623395` | 250k batches | invocation preflight |
| `vs-pandas-20260730T031414.181002Z-pid1443764` | 250k batches | 1M pandas post-arm |
| `vs-pandas-20260730T031905.172482Z-pid1449566` | 250k batches | 1M FrankenPandas post-arm |
| `vs-pandas-20260730T032136.646456Z-pid1453951` | 250k batches | settle-duration labels reversed; superseded |
| `vs-pandas-20260730T033802.841283Z-pid1490126` | 250k batches | valid gates, but raw ELF record lacked build worker; superseded |
| `vs-pandas-20260730T034349.618358Z-pid1515104` | 250k batches | 1M pandas pre-arm |
| `vs-pandas-20260730T034430.773283Z-pid1516009` | 250k batches | 1M FrankenPandas post-arm |

No ratio or partial row from those invocations is used. The first four exposed
monolithic object teardown and irrelevant setup-frame activity. The next
three overlapped provenance hashing or a fleet capability sweep. The eighth,
with no external process, exposed delayed mimalloc purge work escaping the
timer. The ninth passed every quiescence gate, but audit found that its JSON
labels reversed the actual provenance-hash and fixture-setup settle durations.
The tenth had corrected labels and passed every gate, but was superseded when
the builder identity moved into the raw executable record. The next two
builder-aware attempts failed closed. The final invocation recorded the true
settle call sites, builder identity, and clear checks at every checkpoint.

## Hardware, ISA, and executable identity

- Build worker: `vmi1152480`.
- Execution worker: `vmi1149989`.
- CPU: AMD EPYC Processor (with IBPB), 10 physical / 10 logical cores,
  SMT inactive, affinity CPUs 0-9.
- RAM: 63,196,901,376 bytes; one NUMA node.
- Kernel: `6.17.0-40-generic`.
- Runtime-present host ISA: SSE2, AVX, AVX2, FMA, BMI1, BMI2, AES.
- Runtime-absent host ISA: VAES, AVX-512F.
- FrankenPandas process ISA: scalar, SSE2, AVX2, FMA, BMI2.
- FrankenPandas ELF:
  `c585d7fa4e7df9bb880317158609e56644402d6a6a6fa2a3b9d3db480869e0b4`
  (73,472,496 bytes).
- Python 3.13 ELF:
  `efb29ce53d36ebaeee80e3aa44fd6c7f9d71bbded5fe1665240b2ed8ecaeee0e`
  (6,894,448 bytes).
- pandas artifact:
  `f8760652a7b02d2f3f03be104a86e68bcf259353c2b4fd626548c542ff9df9cf`.
- PyArrow artifact:
  `db8bc3d038a12075e7685dc9a860fc93ec6e8fedd1b63cfe17177fb73ce7b82c`.
- Harness source:
  `eea8716f3b0a3815ed6feddb58e2a1af395c40ea783341534edfaebe0a4589cf`.
- Rust benchmark source:
  `dca845060b208a857387caf669aa98033b4dcc24c9568f90a8df991d01b5b809`.

The worker checkout reported stale Git metadata
`7d6630b28aaccf2eb5b7bbfd1ed5d1df47cc9c2f`, so that field is not used as
source attribution. A local `git diff` proved that
`7d6630b28..6774e9a37` has no differences anywhere in the compiled closure
(`Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, and the fp-types,
fp-index, fp-columnar, fp-frame, and fp-bench crates). The changed benchmark
source and harness were overlaid byte-for-byte and matched the hashes above.

## Chooser statement

Choose FrankenPandas for this measured workload shape only: formatting 1M or
10M all-valid finite Float64 telemetry values `value[i] = i * 1.5` into
complete ordered string Series, preserving the global `RangeIndex` and name
`s`, then consuming and destroying results in 250,000-row batches on the
recorded 10-core EPYC/AVX2 host. The measured default runtimes used eight
FrankenPandas operation threads and one pandas operation thread.

Do not generalize this result to arbitrary Float64 spelling; null, NaN, or
infinite values; scientific notation; locale or precision controls; a
retained monolithic output; Python bindings; Arrow-native output; other batch
sizes; thread-normalized comparisons; other hardware; or other pandas APIs.

## Retry predicate

Do not rerun this exact 1M/10M, 250,000-row, finite-half-integer shape merely
to seek a larger ratio. Reopen only for a materially different chooser
question: arbitrary Float64 semantics, nullable data, retained monolithic
output, another batch size, a thread-normalized incumbent, or another
hardware class. Such a run must re-screen task-equivalent incumbents on the
final worker, retain complete output parity and executable/ISA provenance,
give both engines same-invocation A/A controls, and clear the unchanged
median-CI and all-CPU gates.
